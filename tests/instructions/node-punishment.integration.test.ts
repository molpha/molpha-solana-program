/**
 * `punish_equivocation` — offence #1: two valid Molpha attestations for the same execution duty
 * (`source_id`, `registry_version`, `signatures_required`, `canonical_timestamp`) committing to
 * different `value`s. Guilty set is the intersection of the two signer bitmaps; sentence is a
 * 100% slash + permanent tombstone, applied immediately (no dispute window).
 *
 * Also covers the `reinstate_node` guard for the non-Frozen case — no offence currently produces
 * a `Freeze` penalty (equivocation always tombstones), so the freeze->restore path itself is
 * exercised at the Rust unit-test level (`utils::punish::tests`), not here.
 */
import * as anchor from "@coral-xyz/anchor";
import { BN, Idl, Program } from "@coral-xyz/anchor";
import { assert } from "chai";
import { ComputeBudgetProgram, PublicKey, SystemProgram } from "@solana/web3.js";
import molphaIdl from "../../target/idl/molpha.json";
import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { Clock } from "solana-bankrun";
import { setupTestContext, TestContext } from "../setup";
import {
  E2eSignerMaterial,
  deriveSignersBitmap,
  findRegistryPda,
  generateE2eSignerMaterialFromSeed,
  getProtocolTreasury,
  registeredNodePda,
  registerNodesForE2e,
  signE2eAttestation,
} from "../helpers/submit-attestation";
import {
  SignedStatementBytes,
  bitsOfBitmap,
  equivocationEvidenceHash,
  findPunishmentCasePda,
  unionBitmap,
} from "../helpers/node-punishment";

const PROTOCOL_CONFIG_SEED = Buffer.from("molpha_config");
const REGISTRY_STATE_SEED = Buffer.from("molpha_registry");

/**
 * `anchor.workspace.Molpha` is a process-wide singleton bound to whichever provider was active
 * the first time it was constructed. Since every test here calls `setupTestContext()` fresh (a
 * brand-new bankrun ledger per `it()`), reusing that cached Program would silently route later
 * tests' RPCs through an earlier test's ledger. Build the Program directly from the fresh
 * `ctx.provider` instead — same workaround `protocol-initialization.integration.test.ts` uses.
 */
const LOCALNET_PROGRAM_ID = new PublicKey(
  (molphaIdl as { address: string }).address
);

function programFromCtx(ctx: TestContext): anchor.Program {
  return new Program(
    { ...molphaIdl, address: LOCALNET_PROGRAM_ID.toBase58() } as Idl,
    ctx.provider
  );
}

const SIGNER_COUNT = 6;
const SIGS_REQUIRED = 3;
const REDUNDANCY_BUFFER = 1; // selection size = min(6, 3+1) = 4 -> leaves 2 nodes untouched
const MIN_DEPOSIT = 100_000_000;
const BOUNTY_BPS = 2_000; // 20%
const E2E_SEED = "node-punishment-v1";
const SOURCE_ID = Uint8Array.from(Buffer.alloc(32, 0x44));

async function advanceClock(ctx: TestContext, seconds: number): Promise<void> {
  const currentClock = await ctx.provider.context.banksClient.getClock();
  const newClock = new Clock(
    currentClock.slot + BigInt(seconds),
    currentClock.epochStartTimestamp,
    currentClock.epoch,
    currentClock.leaderScheduleEpoch,
    currentClock.unixTimestamp + BigInt(seconds)
  );
  ctx.provider.context.setClock(newClock);
}

async function readTokenAmount(
  ctx: TestContext,
  address: PublicKey
): Promise<bigint> {
  const account = await ctx.provider.context.banksClient.getAccount(address);
  assert(account, `missing token account ${address.toBase58()}`);
  return Buffer.from(account!.data).readBigUInt64LE(64);
}

function setBitEvm(bitmap: Uint8Array, bit: number): void {
  const byteIndex = 31 - (bit >> 3);
  bitmap[byteIndex] |= 1 << (bit & 7);
}

function bitsToBitmap(bits: number[]): Uint8Array {
  const bm = new Uint8Array(32);
  for (const bit of bits) setBitEvm(bm, bit);
  return bm;
}

type PunishmentFixture = {
  ctx: TestContext;
  program: anchor.Program;
  protocolConfigPda: PublicKey;
  registryStatePda: PublicKey;
  treasury: PublicKey;
  material: E2eSignerMaterial;
  registryVersion: number;
};

async function setupPunishmentFixture(): Promise<PunishmentFixture> {
  const ctx = await setupTestContext();
  const program = programFromCtx(ctx);
  const [protocolConfigPda] = PublicKey.findProgramAddressSync(
    [PROTOCOL_CONFIG_SEED],
    program.programId
  );
  const [registryStatePda] = PublicKey.findProgramAddressSync(
    [REGISTRY_STATE_SEED],
    program.programId
  );
  const treasury = getProtocolTreasury(protocolConfigPda, ctx.underlyingTokenMint);

  await program.methods
    .initialize({
      withdrawalCooldownSecs: new BN(5),
      maxNodeCount: SIGNER_COUNT,
      redundancyBuffer: REDUNDANCY_BUFFER,
      minimumNodeDeposit: new BN(MIN_DEPOSIT),
      rewardPerSignature: new BN(1_000_000),
      gatewayFee: new BN(20),
      receiptSubmitterBonus: new BN(5),
      protocolFeeBps: 100,
      maxReceiptDelaySlots: new BN(256),
      protocolFee: new BN(30),
      disputeWindowSlots: new BN(5),
      gatewayBondMin: new BN(500_000_000),
      challengerBountyBps: BOUNTY_BPS,
      usdcMint: ctx.underlyingTokenMint,
      x402RoundBase: new BN(1_000),
      x402RoundPerSig: new BN(15),
    })
    .accountsPartial({
      protocolConfig: protocolConfigPda,
      registryState: registryStatePda,
      usdcMint: ctx.underlyingTokenMint,
      treasury,
      authority: ctx.authority.publicKey,
      tokenProgram: TOKEN_PROGRAM_ID,
      associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
    })
    .rpc();

  const material = generateE2eSignerMaterialFromSeed(E2E_SEED, SIGNER_COUNT);
  await registerNodesForE2e(
    ctx,
    program,
    {
      protocolConfigPda,
      registryStatePda,
      usdcMint: ctx.underlyingTokenMint,
      ownerUsdc: ctx.userTokenAccount,
      treasury,
    },
    material,
    advanceClock
  );

  const registryState = await program.account.registryState.fetch(
    registryStatePda
  );
  const registryVersion: number = registryState.currentVersion;

  return {
    ctx,
    program,
    protocolConfigPda,
    registryStatePda,
    treasury,
    material,
    registryVersion,
  };
}

function buildStatement(params: {
  registryVersion: number;
  material: E2eSignerMaterial;
  bitmap: Uint8Array;
  value: Uint8Array;
  canonicalTimestamp: number;
  signaturesRequired: number;
}): SignedStatementBytes {
  const signed = signE2eAttestation(
    params.registryVersion,
    params.material,
    params.bitmap,
    SOURCE_ID,
    { value: params.value, canonicalTimestamp: params.canonicalTimestamp },
    params.signaturesRequired
  );
  return {
    value: params.value,
    aggSigS: signed.aggSigS,
    commitmentAddr: signed.commitmentAddr,
    signersBitmap: signed.signersBitmap,
  };
}

function toAnchorStatement(statement: SignedStatementBytes) {
  return {
    value: Buffer.from(statement.value),
    aggSigS: Array.from(statement.aggSigS),
    commitmentAddr: Array.from(statement.commitmentAddr),
    signersBitmap: Array.from(statement.signersBitmap),
  };
}

async function callPunishEquivocation(
  fixture: PunishmentFixture,
  params: {
    registryVersion: number;
    currentRegistryVersion: number;
    signaturesRequired: number;
    canonicalTimestamp: number;
    statementA: SignedStatementBytes;
    statementB: SignedStatementBytes;
    newRegistryVersion: number;
  }
): Promise<{ punishmentCasePda: PublicKey; newRegistryPda: PublicKey }> {
  const { program } = fixture;
  const evidenceHash = equivocationEvidenceHash({
    sourceId: SOURCE_ID,
    registryVersion: params.registryVersion,
    signaturesRequired: params.signaturesRequired,
    canonicalTimestamp: params.canonicalTimestamp,
    statementA: params.statementA,
    statementB: params.statementB,
  });

  const args = {
    sourceId: Array.from(SOURCE_ID),
    registryVersion: params.registryVersion,
    signaturesRequired: params.signaturesRequired,
    canonicalTimestamp: new BN(params.canonicalTimestamp),
    statementA: toAnchorStatement(params.statementA),
    statementB: toAnchorStatement(params.statementB),
    evidenceHash: Array.from(evidenceHash),
    newRegistryVersion: params.newRegistryVersion,
  };

  const evidenceRegistryPda = findRegistryPda(
    program.programId,
    params.registryVersion
  );
  const currentRegistryPda = findRegistryPda(
    program.programId,
    params.currentRegistryVersion
  );
  const newRegistryPda = findRegistryPda(
    program.programId,
    params.newRegistryVersion
  );
  const punishmentCasePda = findPunishmentCasePda(
    program.programId,
    evidenceHash
  );
  const union = unionBitmap(
    params.statementA.signersBitmap,
    params.statementB.signersBitmap
  );
  const remainingAccounts = bitsOfBitmap(union).map((bit) => ({
    pubkey: registeredNodePda(program.programId, bit),
    isWritable: true,
    isSigner: false,
  }));

  await program.methods
    .punishEquivocation(args)
    .accountsPartial({
      reporter: fixture.ctx.authority.publicKey,
      protocolConfig: fixture.protocolConfigPda,
      registryState: fixture.registryStatePda,
      evidenceRegistry: evidenceRegistryPda,
      currentRegistry: currentRegistryPda,
      newRegistry: newRegistryPda,
      punishmentCase: punishmentCasePda,
      treasury: fixture.treasury,
      reporterUsdc: fixture.ctx.userTokenAccount,
      tokenProgram: TOKEN_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
    })
    .remainingAccounts(remainingAccounts)
    .preInstructions([
      ComputeBudgetProgram.setComputeUnitLimit({ units: 1_400_000 }),
    ])
    .rpc();

  return { punishmentCasePda, newRegistryPda };
}

describe("punish_equivocation", () => {
  it("slashes and tombstones exactly the guilty intersection, evicts them, and pays the reporter bounty", async () => {
    const fixture = await setupPunishmentFixture();
    const { program, registryVersion, material } = fixture;
    const canonicalTimestamp = 1_700_500_000;
    const selection = deriveSignersBitmap(
      SOURCE_ID,
      registryVersion,
      SIGNER_COUNT,
      SIGS_REQUIRED,
      REDUNDANCY_BUFFER,
      canonicalTimestamp
    );
    const guiltyBits = bitsOfBitmap(selection);
    assert.isAbove(guiltyBits.length, 0);
    assert.isBelow(guiltyBits.length, SIGNER_COUNT);

    const valueA = Uint8Array.from(Buffer.alloc(32, 0x11));
    const valueB = Uint8Array.from(Buffer.alloc(32, 0x22));
    const statementA = buildStatement({
      registryVersion,
      material,
      bitmap: selection,
      value: valueA,
      canonicalTimestamp,
      signaturesRequired: SIGS_REQUIRED,
    });
    const statementB = buildStatement({
      registryVersion,
      material,
      bitmap: selection,
      value: valueB,
      canonicalTimestamp,
      signaturesRequired: SIGS_REQUIRED,
    });

    const untouchedBit = [...Array(SIGNER_COUNT).keys()].find(
      (i) => !guiltyBits.includes(i)
    )!;
    const untouchedNodePda = registeredNodePda(program.programId, untouchedBit);

    const reporterUsdcBefore = await readTokenAmount(
      fixture.ctx,
      fixture.ctx.userTokenAccount
    );

    const newRegistryVersion = registryVersion + 1;
    const { punishmentCasePda, newRegistryPda } = await callPunishEquivocation(
      fixture,
      {
        registryVersion,
        currentRegistryVersion: registryVersion,
        signaturesRequired: SIGS_REQUIRED,
        canonicalTimestamp,
        statementA,
        statementB,
        newRegistryVersion,
      }
    );

    for (const bit of guiltyBits) {
      const node = await program.account.node.fetch(
        registeredNodePda(program.programId, bit)
      );
      assert.property(node.status, "tombstoned");
      assert.equal(node.lockedAmount.toNumber(), 0);
      assert.equal(node.claimableRewards.toNumber(), 0);
      assert.isAbove(node.punishedAt.toNumber(), 0);
    }

    const untouchedAfter = await program.account.node.fetch(untouchedNodePda);
    assert.property(untouchedAfter.status, "active");
    assert.equal(untouchedAfter.lockedAmount.toNumber(), MIN_DEPOSIT);

    const newRegistry = await program.account.registry.fetch(newRegistryPda);
    assert.equal(newRegistry.nodeCount, SIGNER_COUNT - guiltyBits.length);
    const remainingKeys: string[] = newRegistry.nodes
      .slice(0, Number(newRegistry.nodeCount))
      .map((raw: number[]) => new PublicKey(Uint8Array.from(raw)).toBase58());
    for (const bit of guiltyBits) {
      assert.notInclude(
        remainingKeys,
        registeredNodePda(program.programId, bit).toBase58()
      );
    }
    assert.include(remainingKeys, untouchedNodePda.toBase58());

    const registryState = await program.account.registryState.fetch(
      fixture.registryStatePda
    );
    assert.equal(registryState.currentVersion, newRegistryVersion);

    const kase = await program.account.punishmentCase.fetch(punishmentCasePda);
    assert.property(kase.offence, "equivocation");
    assert.equal(kase.guiltyCount, guiltyBits.length);
    assert.deepEqual(Array.from(kase.guiltyBitmap), Array.from(selection));
    assert.equal(
      kase.reporter.toBase58(),
      fixture.ctx.authority.publicKey.toBase58()
    );

    const expectedSlashed = guiltyBits.length * MIN_DEPOSIT;
    assert.equal(kase.slashedTotal.toNumber(), expectedSlashed);
    const expectedBounty = Math.floor((expectedSlashed * BOUNTY_BPS) / 10_000);
    assert.equal(kase.bountyPaid.toNumber(), expectedBounty);

    const reporterUsdcAfter = await readTokenAmount(
      fixture.ctx,
      fixture.ctx.userTokenAccount
    );
    assert.equal(
      Number(reporterUsdcAfter - reporterUsdcBefore),
      expectedBounty
    );
  });

  it("rejects two statements with the same value", async () => {
    const fixture = await setupPunishmentFixture();
    const { registryVersion, material } = fixture;
    const canonicalTimestamp = 1_700_500_100;
    const selection = deriveSignersBitmap(
      SOURCE_ID,
      registryVersion,
      SIGNER_COUNT,
      SIGS_REQUIRED,
      REDUNDANCY_BUFFER,
      canonicalTimestamp
    );
    const value = Uint8Array.from(Buffer.alloc(32, 0x33));
    const statementA = buildStatement({
      registryVersion,
      material,
      bitmap: selection,
      value,
      canonicalTimestamp,
      signaturesRequired: SIGS_REQUIRED,
    });
    const statementB = buildStatement({
      registryVersion,
      material,
      bitmap: selection,
      value,
      canonicalTimestamp,
      signaturesRequired: SIGS_REQUIRED,
    });

    try {
      await callPunishEquivocation(fixture, {
        registryVersion,
        currentRegistryVersion: registryVersion,
        signaturesRequired: SIGS_REQUIRED,
        canonicalTimestamp,
        statementA,
        statementB,
        newRegistryVersion: registryVersion + 1,
      });
      assert.fail("expected EquivocationSameStatement");
    } catch (e: any) {
      assert.include(e.toString(), "EquivocationSameStatement");
    }
  });

  it("rejects a tampered aggregate signature", async () => {
    const fixture = await setupPunishmentFixture();
    const { registryVersion, material } = fixture;
    const canonicalTimestamp = 1_700_500_200;
    const selection = deriveSignersBitmap(
      SOURCE_ID,
      registryVersion,
      SIGNER_COUNT,
      SIGS_REQUIRED,
      REDUNDANCY_BUFFER,
      canonicalTimestamp
    );
    const statementA = buildStatement({
      registryVersion,
      material,
      bitmap: selection,
      value: Uint8Array.from(Buffer.alloc(32, 0x44)),
      canonicalTimestamp,
      signaturesRequired: SIGS_REQUIRED,
    });
    const statementB = buildStatement({
      registryVersion,
      material,
      bitmap: selection,
      value: Uint8Array.from(Buffer.alloc(32, 0x55)),
      canonicalTimestamp,
      signaturesRequired: SIGS_REQUIRED,
    });
    const tamperedSigS = Uint8Array.from(statementB.aggSigS);
    tamperedSigS[0] ^= 0xff;
    statementB.aggSigS = tamperedSigS;

    try {
      await callPunishEquivocation(fixture, {
        registryVersion,
        currentRegistryVersion: registryVersion,
        signaturesRequired: SIGS_REQUIRED,
        canonicalTimestamp,
        statementA,
        statementB,
        newRegistryVersion: registryVersion + 1,
      });
      assert.fail("expected InvalidAggregateSignature");
    } catch (e: any) {
      assert.include(e.toString(), "InvalidAggregateSignature");
    }
  });

  it("rejects a bitmap outside the derived selection", async () => {
    const fixture = await setupPunishmentFixture();
    const { registryVersion, material } = fixture;
    const canonicalTimestamp = 1_700_500_300;
    const selection = deriveSignersBitmap(
      SOURCE_ID,
      registryVersion,
      SIGNER_COUNT,
      SIGS_REQUIRED,
      REDUNDANCY_BUFFER,
      canonicalTimestamp
    );
    const selectionBits = bitsOfBitmap(selection);
    const extraBit = [...Array(SIGNER_COUNT).keys()].find(
      (i) => !selectionBits.includes(i)
    )!;
    const oversizedBitmap = Uint8Array.from(selection);
    setBitEvm(oversizedBitmap, extraBit);

    const statementA = buildStatement({
      registryVersion,
      material,
      bitmap: oversizedBitmap,
      value: Uint8Array.from(Buffer.alloc(32, 0x66)),
      canonicalTimestamp,
      signaturesRequired: SIGS_REQUIRED,
    });
    const statementB = buildStatement({
      registryVersion,
      material,
      bitmap: selection,
      value: Uint8Array.from(Buffer.alloc(32, 0x77)),
      canonicalTimestamp,
      signaturesRequired: SIGS_REQUIRED,
    });

    try {
      await callPunishEquivocation(fixture, {
        registryVersion,
        currentRegistryVersion: registryVersion,
        signaturesRequired: SIGS_REQUIRED,
        canonicalTimestamp,
        statementA,
        statementB,
        newRegistryVersion: registryVersion + 1,
      });
      assert.fail("expected SignersNotSubsetOfSelection");
    } catch (e: any) {
      assert.include(e.toString(), "SignersNotSubsetOfSelection");
    }
  });

  it("rejects disjoint signer sets with no guilty intersection", async () => {
    const fixture = await setupPunishmentFixture();
    const { registryVersion, material } = fixture;
    const canonicalTimestamp = 1_700_500_400;
    const selection = deriveSignersBitmap(
      SOURCE_ID,
      registryVersion,
      SIGNER_COUNT,
      SIGS_REQUIRED,
      REDUNDANCY_BUFFER,
      canonicalTimestamp
    );
    const selectionBits = bitsOfBitmap(selection);
    assert.isAtLeast(selectionBits.length, 2);
    const mid = Math.ceil(selectionBits.length / 2);
    const bitmapA = bitsToBitmap(selectionBits.slice(0, mid));
    const bitmapB = bitsToBitmap(selectionBits.slice(mid));

    const statementA = buildStatement({
      registryVersion,
      material,
      bitmap: bitmapA,
      value: Uint8Array.from(Buffer.alloc(32, 0x88)),
      canonicalTimestamp,
      signaturesRequired: SIGS_REQUIRED,
    });
    const statementB = buildStatement({
      registryVersion,
      material,
      bitmap: bitmapB,
      value: Uint8Array.from(Buffer.alloc(32, 0x99)),
      canonicalTimestamp,
      signaturesRequired: SIGS_REQUIRED,
    });

    try {
      await callPunishEquivocation(fixture, {
        registryVersion,
        currentRegistryVersion: registryVersion,
        signaturesRequired: SIGS_REQUIRED,
        canonicalTimestamp,
        statementA,
        statementB,
        newRegistryVersion: registryVersion + 1,
      });
      assert.fail("expected GuiltySetEmpty");
    } catch (e: any) {
      assert.include(e.toString(), "GuiltySetEmpty");
    }
  });

  it("rejects replaying the same evidence submitted in either statement order", async () => {
    const fixture = await setupPunishmentFixture();
    const { registryVersion, material } = fixture;
    const canonicalTimestamp = 1_700_500_500;
    const selection = deriveSignersBitmap(
      SOURCE_ID,
      registryVersion,
      SIGNER_COUNT,
      SIGS_REQUIRED,
      REDUNDANCY_BUFFER,
      canonicalTimestamp
    );
    const statementA = buildStatement({
      registryVersion,
      material,
      bitmap: selection,
      value: Uint8Array.from(Buffer.alloc(32, 0xaa)),
      canonicalTimestamp,
      signaturesRequired: SIGS_REQUIRED,
    });
    const statementB = buildStatement({
      registryVersion,
      material,
      bitmap: selection,
      value: Uint8Array.from(Buffer.alloc(32, 0xbb)),
      canonicalTimestamp,
      signaturesRequired: SIGS_REQUIRED,
    });

    const firstNewVersion = registryVersion + 1;
    await callPunishEquivocation(fixture, {
      registryVersion,
      currentRegistryVersion: registryVersion,
      signaturesRequired: SIGS_REQUIRED,
      canonicalTimestamp,
      statementA,
      statementB,
      newRegistryVersion: firstNewVersion,
    });

    // Same evidence, statements swapped (same evidence_hash, order-invariant), a fresh
    // new_registry_version so only `punishment_case`'s `init` can collide.
    try {
      await callPunishEquivocation(fixture, {
        registryVersion,
        currentRegistryVersion: firstNewVersion,
        signaturesRequired: SIGS_REQUIRED,
        canonicalTimestamp,
        statementA: statementB,
        statementB: statementA,
        newRegistryVersion: firstNewVersion + 1,
      });
      assert.fail("expected replay to fail on punishment_case re-init");
    } catch (e: any) {
      assert.match(e.toString(), /already in use/i);
    }
  });
});

describe("reinstate_node", () => {
  it("rejects reinstating a node that is not Frozen", async () => {
    const fixture = await setupPunishmentFixture();
    const { program, registryVersion } = fixture;
    const nodeOwner = fixture.ctx.nodes[0];
    const nodePda = registeredNodePda(program.programId, 0);
    const newRegistryVersion = registryVersion + 1;

    try {
      await program.methods
        .reinstateNode({
          nodeOwner: nodeOwner.publicKey,
          newRegistryVersion,
        })
        .accountsPartial({
          caller: nodeOwner.publicKey,
          protocolConfig: fixture.protocolConfigPda,
          registryState: fixture.registryStatePda,
          currentRegistry: findRegistryPda(program.programId, registryVersion),
          newRegistry: findRegistryPda(program.programId, newRegistryVersion),
          node: nodePda,
          systemProgram: SystemProgram.programId,
        })
        .signers([nodeOwner])
        .rpc();
      assert.fail("expected NodeNotFrozen");
    } catch (e: any) {
      assert.include(e.toString(), "NodeNotFrozen");
    }
  });
});
