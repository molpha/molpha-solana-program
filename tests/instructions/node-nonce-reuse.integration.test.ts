/**
 * `punish_nonce_reuse` — offence #3: a node contributes a partial signature under one aggregate
 * nonce commitment (`SignedStatement.commitmentAddr`) for two different signed messages. Guilty
 * set is the intersection of the two signer bitmaps, exactly as with `punish_equivocation`;
 * sentence is a 50% slash + permanent tombstone (`OffenceKind::NonceReuse`) — the first
 * partial-slash offence, half of the 100% the other two node-fault offences carry.
 *
 * Mirrors `node-punishment.integration.test.ts` / `node-unsolicited-signing.integration.test.ts`'s
 * fixture/helper patterns (same bankrun-per-test setup, same 6-node / sigsRequired=3 /
 * redundancyBuffer=1 registry). The one new fixture technique is pinning the Schnorr nonce `k`
 * across two `signE2eAttestation` calls so both aggregates land on the same `commitmentAddr` —
 * see `schnorrSignForVerifier`'s `nonce` parameter.
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
  SigningContextFields,
  bitsOfBitmap,
  findPunishmentCasePda,
  intersectionBitmap,
  nonceReuseEvidenceHash,
  unionBitmap,
} from "../helpers/node-punishment";

const PROTOCOL_CONFIG_SEED = Buffer.from("molpha_config");
const REGISTRY_STATE_SEED = Buffer.from("molpha_registry");

/**
 * Same workaround as the other punishment test files: `anchor.workspace.Molpha` is a process-wide
 * singleton bound to whichever provider was active first, so a fresh bankrun ledger per `it()`
 * needs a Program rebuilt from the fresh `ctx.provider`.
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
const REDUNDANCY_BUFFER = 1; // selection size = min(6, 3+1) = 4 -> leaves 2 nodes unselected
const MIN_DEPOSIT = 100_000_000;
const HALF_SLASH = MIN_DEPOSIT / 2;
const BOUNTY_BPS = 2_000; // 20%
const E2E_SEED = "node-nonce-reuse-v1";
const SOURCE_ID = Uint8Array.from(Buffer.alloc(32, 0x66));
const SOURCE_ID_ALT = Uint8Array.from(Buffer.alloc(32, 0x67));

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

function clearBitEvm(bitmap: Uint8Array, bit: number): Uint8Array {
  const out = Uint8Array.from(bitmap);
  const byteIndex = 31 - (bit >> 3);
  out[byteIndex] &= ~(1 << (bit & 7));
  return out;
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
  sourceId: Uint8Array;
  bitmap: Uint8Array;
  value: Uint8Array;
  canonicalTimestamp: number;
  signaturesRequired: number;
  /** Pin the Schnorr nonce so this statement shares `commitmentAddr` with another. */
  nonce?: bigint;
}): SignedStatementBytes & { nonce: bigint } {
  const signed = signE2eAttestation(
    params.registryVersion,
    params.material,
    params.bitmap,
    params.sourceId,
    { value: params.value, canonicalTimestamp: params.canonicalTimestamp },
    params.signaturesRequired,
    undefined,
    2,
    params.nonce
  );
  return {
    value: params.value,
    aggSigS: signed.aggSigS,
    commitmentAddr: signed.commitmentAddr,
    signersBitmap: signed.signersBitmap,
    nonce: signed.nonce,
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

function toAnchorContext(context: SigningContextFields) {
  return {
    sourceId: Array.from(context.sourceId),
    signaturesRequired: context.signaturesRequired,
    canonicalTimestamp: new BN(context.canonicalTimestamp),
  };
}

async function callPunishNonceReuse(
  fixture: PunishmentFixture,
  params: {
    registryVersion: number;
    currentRegistryVersion: number;
    contextA: SigningContextFields;
    statementA: SignedStatementBytes;
    contextB: SigningContextFields;
    statementB: SignedStatementBytes;
    newRegistryVersion: number;
    /** Override the computed evidence hash, for the mismatch test. */
    evidenceHashOverride?: Uint8Array;
  }
): Promise<{ punishmentCasePda: PublicKey; newRegistryPda: PublicKey }> {
  const { program } = fixture;
  const evidenceHash =
    params.evidenceHashOverride ??
    nonceReuseEvidenceHash({
      registryVersion: params.registryVersion,
      contextA: params.contextA,
      statementA: params.statementA,
      contextB: params.contextB,
      statementB: params.statementB,
    });

  const args = {
    registryVersion: params.registryVersion,
    contextA: toAnchorContext(params.contextA),
    statementA: toAnchorStatement(params.statementA),
    contextB: toAnchorContext(params.contextB),
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
    .punishNonceReuse(args)
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

describe("punish_nonce_reuse", () => {
  it("slashes 50% and tombstones exactly the cross-context intersection, evicts it, and pays the reporter bounty", async () => {
    const fixture = await setupPunishmentFixture();
    const { program, registryVersion, material } = fixture;
    const tsA = 1_700_700_000;
    const tsB = 1_700_700_100;

    const selectionA = deriveSignersBitmap(
      SOURCE_ID,
      registryVersion,
      SIGNER_COUNT,
      SIGS_REQUIRED,
      REDUNDANCY_BUFFER,
      tsA
    );
    const selectionB = deriveSignersBitmap(
      SOURCE_ID,
      registryVersion,
      SIGNER_COUNT,
      SIGS_REQUIRED,
      REDUNDANCY_BUFFER,
      tsB
    );
    const bitsA = bitsOfBitmap(selectionA);
    const bitsB = bitsOfBitmap(selectionB);
    // Two size-4 selections of 6 nodes intersect in >= 2 by pigeonhole (4 + 4 - 6 = 2) — not
    // fixture-fragile, but assert it so the reason is on record.
    const intersectionBits = bitsA.filter((b) => bitsB.includes(b));
    assert.isAtLeast(intersectionBits.length, 2);
    const onlyABits = bitsA.filter((b) => !bitsB.includes(b));
    const onlyBBits = bitsB.filter((b) => !bitsA.includes(b));

    const contextA: SigningContextFields = {
      sourceId: SOURCE_ID,
      signaturesRequired: SIGS_REQUIRED,
      canonicalTimestamp: tsA,
    };
    const contextB: SigningContextFields = {
      sourceId: SOURCE_ID,
      signaturesRequired: SIGS_REQUIRED,
      canonicalTimestamp: tsB,
    };

    const statementA = buildStatement({
      registryVersion,
      material,
      sourceId: SOURCE_ID,
      bitmap: selectionA,
      value: Uint8Array.from(Buffer.alloc(32, 0xaa)),
      canonicalTimestamp: tsA,
      signaturesRequired: SIGS_REQUIRED,
    });
    const statementB = buildStatement({
      registryVersion,
      material,
      sourceId: SOURCE_ID,
      bitmap: selectionB,
      value: Uint8Array.from(Buffer.alloc(32, 0xbb)),
      canonicalTimestamp: tsB,
      signaturesRequired: SIGS_REQUIRED,
      nonce: statementA.nonce,
    });
    assert.deepEqual(
      Array.from(statementA.commitmentAddr),
      Array.from(statementB.commitmentAddr)
    );

    const reporterUsdcBefore = await readTokenAmount(
      fixture.ctx,
      fixture.ctx.userTokenAccount
    );

    const newRegistryVersion = registryVersion + 1;
    const { punishmentCasePda, newRegistryPda } = await callPunishNonceReuse(
      fixture,
      {
        registryVersion,
        currentRegistryVersion: registryVersion,
        contextA,
        statementA,
        contextB,
        statementB,
        newRegistryVersion,
      }
    );

    for (const bit of intersectionBits) {
      const node = await program.account.node.fetch(
        registeredNodePda(program.programId, bit)
      );
      assert.property(node.status, "tombstoned");
      assert.equal(node.lockedAmount.toNumber(), HALF_SLASH);
      assert.equal(node.claimableRewards.toNumber(), 0);
      assert.isAbove(node.punishedAt.toNumber(), 0);
    }

    for (const bit of [...onlyABits, ...onlyBBits]) {
      const node = await program.account.node.fetch(
        registeredNodePda(program.programId, bit)
      );
      assert.property(node.status, "active");
      assert.equal(node.lockedAmount.toNumber(), MIN_DEPOSIT);
    }

    const newRegistry = await program.account.registry.fetch(newRegistryPda);
    assert.equal(newRegistry.nodeCount, SIGNER_COUNT - intersectionBits.length);
    const remainingKeys: string[] = newRegistry.nodes
      .slice(0, Number(newRegistry.nodeCount))
      .map((raw: number[]) => new PublicKey(Uint8Array.from(raw)).toBase58());
    for (const bit of intersectionBits) {
      assert.notInclude(
        remainingKeys,
        registeredNodePda(program.programId, bit).toBase58()
      );
    }
    for (const bit of [...onlyABits, ...onlyBBits]) {
      assert.include(
        remainingKeys,
        registeredNodePda(program.programId, bit).toBase58()
      );
    }

    const registryState = await program.account.registryState.fetch(
      fixture.registryStatePda
    );
    assert.equal(registryState.currentVersion, newRegistryVersion);

    const kase = await program.account.punishmentCase.fetch(punishmentCasePda);
    assert.property(kase.offence, "nonceReuse");
    assert.equal(kase.guiltyCount, intersectionBits.length);
    assert.deepEqual(
      Array.from(kase.guiltyBitmap),
      Array.from(intersectionBitmap(selectionA, selectionB))
    );
    assert.equal(
      kase.reporter.toBase58(),
      fixture.ctx.authority.publicKey.toBase58()
    );

    const expectedSlashed = intersectionBits.length * HALF_SLASH;
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

  it("treats same duty tuple + same value + different bitmaps as nonce reuse, not equivocation", async () => {
    const fixture = await setupPunishmentFixture();
    const { program, registryVersion, material } = fixture;
    const ts = 1_700_700_200;
    const context: SigningContextFields = {
      sourceId: SOURCE_ID,
      signaturesRequired: SIGS_REQUIRED,
      canonicalTimestamp: ts,
    };
    const selection = deriveSignersBitmap(
      SOURCE_ID,
      registryVersion,
      SIGNER_COUNT,
      SIGS_REQUIRED,
      REDUNDANCY_BUFFER,
      ts
    );
    const selectionBits = bitsOfBitmap(selection);
    const droppedBit = selectionBits[0];
    const bitmapB = clearBitEvm(selection, droppedBit);
    const guiltyBits = selectionBits.filter((b) => b !== droppedBit);

    const value = Uint8Array.from(Buffer.alloc(32, 0xcc));
    const statementA = buildStatement({
      registryVersion,
      material,
      sourceId: SOURCE_ID,
      bitmap: selection,
      value,
      canonicalTimestamp: ts,
      signaturesRequired: SIGS_REQUIRED,
    });
    const statementB = buildStatement({
      registryVersion,
      material,
      sourceId: SOURCE_ID,
      bitmap: bitmapB,
      value,
      canonicalTimestamp: ts,
      signaturesRequired: SIGS_REQUIRED,
      nonce: statementA.nonce,
    });

    const { punishmentCasePda } = await callPunishNonceReuse(fixture, {
      registryVersion,
      currentRegistryVersion: registryVersion,
      contextA: context,
      statementA,
      contextB: context,
      statementB,
      newRegistryVersion: registryVersion + 1,
    });

    for (const bit of guiltyBits) {
      const node = await program.account.node.fetch(
        registeredNodePda(program.programId, bit)
      );
      assert.property(node.status, "tombstoned");
      assert.equal(node.lockedAmount.toNumber(), HALF_SLASH);
    }
    const untouched = await program.account.node.fetch(
      registeredNodePda(program.programId, droppedBit)
    );
    assert.property(untouched.status, "active");
    assert.equal(untouched.lockedAmount.toNumber(), MIN_DEPOSIT);

    const kase = await program.account.punishmentCase.fetch(punishmentCasePda);
    assert.property(kase.offence, "nonceReuse");
    assert.equal(kase.guiltyCount, guiltyBits.length);
  });

  it("supports two different source_ids under one registry version", async () => {
    const fixture = await setupPunishmentFixture();
    const { program, registryVersion, material } = fixture;
    const ts = 1_700_700_300;

    const selectionA = deriveSignersBitmap(
      SOURCE_ID,
      registryVersion,
      SIGNER_COUNT,
      SIGS_REQUIRED,
      REDUNDANCY_BUFFER,
      ts
    );
    const selectionB = deriveSignersBitmap(
      SOURCE_ID_ALT,
      registryVersion,
      SIGNER_COUNT,
      SIGS_REQUIRED,
      REDUNDANCY_BUFFER,
      ts
    );
    const bitsA = bitsOfBitmap(selectionA);
    const bitsB = bitsOfBitmap(selectionB);
    const intersectionBits = bitsA.filter((b) => bitsB.includes(b));
    assert.isAtLeast(intersectionBits.length, 2);

    const contextA: SigningContextFields = {
      sourceId: SOURCE_ID,
      signaturesRequired: SIGS_REQUIRED,
      canonicalTimestamp: ts,
    };
    const contextB: SigningContextFields = {
      sourceId: SOURCE_ID_ALT,
      signaturesRequired: SIGS_REQUIRED,
      canonicalTimestamp: ts,
    };

    const statementA = buildStatement({
      registryVersion,
      material,
      sourceId: SOURCE_ID,
      bitmap: selectionA,
      value: Uint8Array.from(Buffer.alloc(32, 0xdd)),
      canonicalTimestamp: ts,
      signaturesRequired: SIGS_REQUIRED,
    });
    const statementB = buildStatement({
      registryVersion,
      material,
      sourceId: SOURCE_ID_ALT,
      bitmap: selectionB,
      value: Uint8Array.from(Buffer.alloc(32, 0xdd)),
      canonicalTimestamp: ts,
      signaturesRequired: SIGS_REQUIRED,
      nonce: statementA.nonce,
    });

    const { punishmentCasePda } = await callPunishNonceReuse(fixture, {
      registryVersion,
      currentRegistryVersion: registryVersion,
      contextA,
      statementA,
      contextB,
      statementB,
      newRegistryVersion: registryVersion + 1,
    });

    const kase = await program.account.punishmentCase.fetch(punishmentCasePda);
    assert.property(kase.offence, "nonceReuse");
    assert.equal(kase.guiltyCount, intersectionBits.length);
    for (const bit of intersectionBits) {
      const node = await program.account.node.fetch(
        registeredNodePda(program.programId, bit)
      );
      assert.property(node.status, "tombstoned");
      assert.equal(node.lockedAmount.toNumber(), HALF_SLASH);
    }
  });

  it("rejects statements signed under independent nonces", async () => {
    const fixture = await setupPunishmentFixture();
    const { registryVersion, material } = fixture;
    const tsA = 1_700_700_400;
    const tsB = 1_700_700_500;
    const selectionA = deriveSignersBitmap(
      SOURCE_ID,
      registryVersion,
      SIGNER_COUNT,
      SIGS_REQUIRED,
      REDUNDANCY_BUFFER,
      tsA
    );
    const selectionB = deriveSignersBitmap(
      SOURCE_ID,
      registryVersion,
      SIGNER_COUNT,
      SIGS_REQUIRED,
      REDUNDANCY_BUFFER,
      tsB
    );
    const contextA: SigningContextFields = {
      sourceId: SOURCE_ID,
      signaturesRequired: SIGS_REQUIRED,
      canonicalTimestamp: tsA,
    };
    const contextB: SigningContextFields = {
      sourceId: SOURCE_ID,
      signaturesRequired: SIGS_REQUIRED,
      canonicalTimestamp: tsB,
    };

    const statementA = buildStatement({
      registryVersion,
      material,
      sourceId: SOURCE_ID,
      bitmap: selectionA,
      value: Uint8Array.from(Buffer.alloc(32, 0x01)),
      canonicalTimestamp: tsA,
      signaturesRequired: SIGS_REQUIRED,
    });
    // No `nonce` feedback — an independent, freshly-sampled nonce, so `commitmentAddr` differs.
    const statementB = buildStatement({
      registryVersion,
      material,
      sourceId: SOURCE_ID,
      bitmap: selectionB,
      value: Uint8Array.from(Buffer.alloc(32, 0x02)),
      canonicalTimestamp: tsB,
      signaturesRequired: SIGS_REQUIRED,
    });

    try {
      await callPunishNonceReuse(fixture, {
        registryVersion,
        currentRegistryVersion: registryVersion,
        contextA,
        statementA,
        contextB,
        statementB,
        newRegistryVersion: registryVersion + 1,
      });
      assert.fail("expected NonceCommitmentMismatch");
    } catch (e: any) {
      assert.include(e.toString(), "NonceCommitmentMismatch");
    }
  });

  it("rejects two byte-identical statements (same commitment, same message)", async () => {
    const fixture = await setupPunishmentFixture();
    const { registryVersion, material } = fixture;
    const ts = 1_700_700_600;
    const context: SigningContextFields = {
      sourceId: SOURCE_ID,
      signaturesRequired: SIGS_REQUIRED,
      canonicalTimestamp: ts,
    };
    const selection = deriveSignersBitmap(
      SOURCE_ID,
      registryVersion,
      SIGNER_COUNT,
      SIGS_REQUIRED,
      REDUNDANCY_BUFFER,
      ts
    );
    const value = Uint8Array.from(Buffer.alloc(32, 0x03));

    const statementA = buildStatement({
      registryVersion,
      material,
      sourceId: SOURCE_ID,
      bitmap: selection,
      value,
      canonicalTimestamp: ts,
      signaturesRequired: SIGS_REQUIRED,
    });
    // Identical context, bitmap, and value, signed with the same pinned nonce -> byte-identical
    // to statementA (deterministic Schnorr signing given a fixed k).
    const statementB = buildStatement({
      registryVersion,
      material,
      sourceId: SOURCE_ID,
      bitmap: selection,
      value,
      canonicalTimestamp: ts,
      signaturesRequired: SIGS_REQUIRED,
      nonce: statementA.nonce,
    });
    assert.deepEqual(
      Array.from(statementA.aggSigS),
      Array.from(statementB.aggSigS)
    );

    try {
      await callPunishNonceReuse(fixture, {
        registryVersion,
        currentRegistryVersion: registryVersion,
        contextA: context,
        statementA,
        contextB: context,
        statementB,
        newRegistryVersion: registryVersion + 1,
      });
      assert.fail("expected NonceReuseSameMessage");
    } catch (e: any) {
      assert.include(e.toString(), "NonceReuseSameMessage");
    }
  });

  it("rejects same duty tuple with different values as nonce reuse - it is equivocation", async () => {
    const fixture = await setupPunishmentFixture();
    const { registryVersion, material } = fixture;
    const ts = 1_700_700_700;
    const context: SigningContextFields = {
      sourceId: SOURCE_ID,
      signaturesRequired: SIGS_REQUIRED,
      canonicalTimestamp: ts,
    };
    const selection = deriveSignersBitmap(
      SOURCE_ID,
      registryVersion,
      SIGNER_COUNT,
      SIGS_REQUIRED,
      REDUNDANCY_BUFFER,
      ts
    );

    const statementA = buildStatement({
      registryVersion,
      material,
      sourceId: SOURCE_ID,
      bitmap: selection,
      value: Uint8Array.from(Buffer.alloc(32, 0x04)),
      canonicalTimestamp: ts,
      signaturesRequired: SIGS_REQUIRED,
    });
    const statementB = buildStatement({
      registryVersion,
      material,
      sourceId: SOURCE_ID,
      bitmap: selection,
      value: Uint8Array.from(Buffer.alloc(32, 0x05)),
      canonicalTimestamp: ts,
      signaturesRequired: SIGS_REQUIRED,
      nonce: statementA.nonce,
    });

    try {
      await callPunishNonceReuse(fixture, {
        registryVersion,
        currentRegistryVersion: registryVersion,
        contextA: context,
        statementA,
        contextB: context,
        statementB,
        newRegistryVersion: registryVersion + 1,
      });
      assert.fail("expected PreferEquivocation");
    } catch (e: any) {
      assert.include(e.toString(), "PreferEquivocation");
    }
  });

  it("rejects a tampered aggregate signature", async () => {
    const fixture = await setupPunishmentFixture();
    const { registryVersion, material } = fixture;
    const tsA = 1_700_700_800;
    const tsB = 1_700_700_900;
    const selectionA = deriveSignersBitmap(
      SOURCE_ID,
      registryVersion,
      SIGNER_COUNT,
      SIGS_REQUIRED,
      REDUNDANCY_BUFFER,
      tsA
    );
    const selectionB = deriveSignersBitmap(
      SOURCE_ID,
      registryVersion,
      SIGNER_COUNT,
      SIGS_REQUIRED,
      REDUNDANCY_BUFFER,
      tsB
    );
    const contextA: SigningContextFields = {
      sourceId: SOURCE_ID,
      signaturesRequired: SIGS_REQUIRED,
      canonicalTimestamp: tsA,
    };
    const contextB: SigningContextFields = {
      sourceId: SOURCE_ID,
      signaturesRequired: SIGS_REQUIRED,
      canonicalTimestamp: tsB,
    };

    const statementA = buildStatement({
      registryVersion,
      material,
      sourceId: SOURCE_ID,
      bitmap: selectionA,
      value: Uint8Array.from(Buffer.alloc(32, 0x06)),
      canonicalTimestamp: tsA,
      signaturesRequired: SIGS_REQUIRED,
    });
    const statementB = buildStatement({
      registryVersion,
      material,
      sourceId: SOURCE_ID,
      bitmap: selectionB,
      value: Uint8Array.from(Buffer.alloc(32, 0x07)),
      canonicalTimestamp: tsB,
      signaturesRequired: SIGS_REQUIRED,
      nonce: statementA.nonce,
    });
    // Tamper the signature only — leave `commitmentAddr` intact so the handler reaches step 7
    // (aggregate verification) rather than failing an earlier, cheaper guard.
    const tamperedSigS = Uint8Array.from(statementB.aggSigS);
    tamperedSigS[0] ^= 0xff;
    statementB.aggSigS = tamperedSigS;

    try {
      await callPunishNonceReuse(fixture, {
        registryVersion,
        currentRegistryVersion: registryVersion,
        contextA,
        statementA,
        contextB,
        statementB,
        newRegistryVersion: registryVersion + 1,
      });
      assert.fail("expected InvalidAggregateSignature");
    } catch (e: any) {
      assert.include(e.toString(), "InvalidAggregateSignature");
    }
  });

  it("rejects a signer outside its own statement's selection", async () => {
    const fixture = await setupPunishmentFixture();
    const { registryVersion, material } = fixture;
    const tsA = 1_700_701_000;
    const tsB = 1_700_701_100;
    const selectionA = deriveSignersBitmap(
      SOURCE_ID,
      registryVersion,
      SIGNER_COUNT,
      SIGS_REQUIRED,
      REDUNDANCY_BUFFER,
      tsA
    );
    const selectionB = deriveSignersBitmap(
      SOURCE_ID,
      registryVersion,
      SIGNER_COUNT,
      SIGS_REQUIRED,
      REDUNDANCY_BUFFER,
      tsB
    );
    const selectionBBits = bitsOfBitmap(selectionB);
    const unselectedForB = [...Array(SIGNER_COUNT).keys()].find(
      (i) => !selectionBBits.includes(i)
    )!;
    const bitmapBWithExtra = Uint8Array.from(selectionB);
    const byteIndex = 31 - (unselectedForB >> 3);
    bitmapBWithExtra[byteIndex] |= 1 << (unselectedForB & 7);

    const contextA: SigningContextFields = {
      sourceId: SOURCE_ID,
      signaturesRequired: SIGS_REQUIRED,
      canonicalTimestamp: tsA,
    };
    const contextB: SigningContextFields = {
      sourceId: SOURCE_ID,
      signaturesRequired: SIGS_REQUIRED,
      canonicalTimestamp: tsB,
    };

    const statementA = buildStatement({
      registryVersion,
      material,
      sourceId: SOURCE_ID,
      bitmap: selectionA,
      value: Uint8Array.from(Buffer.alloc(32, 0x08)),
      canonicalTimestamp: tsA,
      signaturesRequired: SIGS_REQUIRED,
    });
    const statementB = buildStatement({
      registryVersion,
      material,
      sourceId: SOURCE_ID,
      bitmap: bitmapBWithExtra,
      value: Uint8Array.from(Buffer.alloc(32, 0x09)),
      canonicalTimestamp: tsB,
      signaturesRequired: SIGS_REQUIRED,
      nonce: statementA.nonce,
    });

    try {
      await callPunishNonceReuse(fixture, {
        registryVersion,
        currentRegistryVersion: registryVersion,
        contextA,
        statementA,
        contextB,
        statementB,
        newRegistryVersion: registryVersion + 1,
      });
      assert.fail("expected SignersNotSubsetOfSelection");
    } catch (e: any) {
      assert.include(e.toString(), "SignersNotSubsetOfSelection");
    }
  });

  it("rejects disjoint signer sets", async () => {
    const fixture = await setupPunishmentFixture();
    const { registryVersion, material } = fixture;
    const ts = 1_700_701_200;
    const context: SigningContextFields = {
      sourceId: SOURCE_ID,
      signaturesRequired: SIGS_REQUIRED,
      canonicalTimestamp: ts,
    };
    const selection = deriveSignersBitmap(
      SOURCE_ID,
      registryVersion,
      SIGNER_COUNT,
      SIGS_REQUIRED,
      REDUNDANCY_BUFFER,
      ts
    );
    const selectionBits = bitsOfBitmap(selection);
    assert.isAtLeast(selectionBits.length, 4);

    let bitmapA = new Uint8Array(32);
    let byteIndex = 31 - (selectionBits[0] >> 3);
    bitmapA[byteIndex] |= 1 << (selectionBits[0] & 7);
    byteIndex = 31 - (selectionBits[1] >> 3);
    bitmapA[byteIndex] |= 1 << (selectionBits[1] & 7);

    let bitmapB = new Uint8Array(32);
    byteIndex = 31 - (selectionBits[2] >> 3);
    bitmapB[byteIndex] |= 1 << (selectionBits[2] & 7);
    byteIndex = 31 - (selectionBits[3] >> 3);
    bitmapB[byteIndex] |= 1 << (selectionBits[3] & 7);

    const value = Uint8Array.from(Buffer.alloc(32, 0x0a));
    const statementA = buildStatement({
      registryVersion,
      material,
      sourceId: SOURCE_ID,
      bitmap: bitmapA,
      value,
      canonicalTimestamp: ts,
      signaturesRequired: SIGS_REQUIRED,
    });
    const statementB = buildStatement({
      registryVersion,
      material,
      sourceId: SOURCE_ID,
      bitmap: bitmapB,
      value,
      canonicalTimestamp: ts,
      signaturesRequired: SIGS_REQUIRED,
      nonce: statementA.nonce,
    });

    try {
      await callPunishNonceReuse(fixture, {
        registryVersion,
        currentRegistryVersion: registryVersion,
        contextA: context,
        statementA,
        contextB: context,
        statementB,
        newRegistryVersion: registryVersion + 1,
      });
      assert.fail("expected GuiltySetEmpty");
    } catch (e: any) {
      assert.include(e.toString(), "GuiltySetEmpty");
    }
  });

  it("rejects an evidence hash that does not match the supplied fields", async () => {
    const fixture = await setupPunishmentFixture();
    const { registryVersion, material } = fixture;
    const tsA = 1_700_701_300;
    const tsB = 1_700_701_400;
    const selectionA = deriveSignersBitmap(
      SOURCE_ID,
      registryVersion,
      SIGNER_COUNT,
      SIGS_REQUIRED,
      REDUNDANCY_BUFFER,
      tsA
    );
    const selectionB = deriveSignersBitmap(
      SOURCE_ID,
      registryVersion,
      SIGNER_COUNT,
      SIGS_REQUIRED,
      REDUNDANCY_BUFFER,
      tsB
    );
    const contextA: SigningContextFields = {
      sourceId: SOURCE_ID,
      signaturesRequired: SIGS_REQUIRED,
      canonicalTimestamp: tsA,
    };
    const contextB: SigningContextFields = {
      sourceId: SOURCE_ID,
      signaturesRequired: SIGS_REQUIRED,
      canonicalTimestamp: tsB,
    };

    const statementA = buildStatement({
      registryVersion,
      material,
      sourceId: SOURCE_ID,
      bitmap: selectionA,
      value: Uint8Array.from(Buffer.alloc(32, 0x0b)),
      canonicalTimestamp: tsA,
      signaturesRequired: SIGS_REQUIRED,
    });
    const statementB = buildStatement({
      registryVersion,
      material,
      sourceId: SOURCE_ID,
      bitmap: selectionB,
      value: Uint8Array.from(Buffer.alloc(32, 0x0c)),
      canonicalTimestamp: tsB,
      signaturesRequired: SIGS_REQUIRED,
      nonce: statementA.nonce,
    });

    // Hash computed over a different contextB.canonicalTimestamp than the args carry.
    const wrongHash = nonceReuseEvidenceHash({
      registryVersion,
      contextA,
      statementA,
      contextB: { ...contextB, canonicalTimestamp: tsB + 1 },
      statementB,
    });

    try {
      await callPunishNonceReuse(fixture, {
        registryVersion,
        currentRegistryVersion: registryVersion,
        contextA,
        statementA,
        contextB,
        statementB,
        newRegistryVersion: registryVersion + 1,
        evidenceHashOverride: wrongHash,
      });
      assert.fail("expected EvidenceHashMismatch");
    } catch (e: any) {
      assert.include(e.toString(), "EvidenceHashMismatch");
    }
  });

  it("rejects replaying the same evidence with the pairs swapped", async () => {
    const fixture = await setupPunishmentFixture();
    const { registryVersion, material } = fixture;
    const tsA = 1_700_701_500;
    const tsB = 1_700_701_600;
    const selectionA = deriveSignersBitmap(
      SOURCE_ID,
      registryVersion,
      SIGNER_COUNT,
      SIGS_REQUIRED,
      REDUNDANCY_BUFFER,
      tsA
    );
    const selectionB = deriveSignersBitmap(
      SOURCE_ID,
      registryVersion,
      SIGNER_COUNT,
      SIGS_REQUIRED,
      REDUNDANCY_BUFFER,
      tsB
    );
    const contextA: SigningContextFields = {
      sourceId: SOURCE_ID,
      signaturesRequired: SIGS_REQUIRED,
      canonicalTimestamp: tsA,
    };
    const contextB: SigningContextFields = {
      sourceId: SOURCE_ID,
      signaturesRequired: SIGS_REQUIRED,
      canonicalTimestamp: tsB,
    };

    const statementA = buildStatement({
      registryVersion,
      material,
      sourceId: SOURCE_ID,
      bitmap: selectionA,
      value: Uint8Array.from(Buffer.alloc(32, 0x0d)),
      canonicalTimestamp: tsA,
      signaturesRequired: SIGS_REQUIRED,
    });
    const statementB = buildStatement({
      registryVersion,
      material,
      sourceId: SOURCE_ID,
      bitmap: selectionB,
      value: Uint8Array.from(Buffer.alloc(32, 0x0e)),
      canonicalTimestamp: tsB,
      signaturesRequired: SIGS_REQUIRED,
      nonce: statementA.nonce,
    });

    const firstNewVersion = registryVersion + 1;
    await callPunishNonceReuse(fixture, {
      registryVersion,
      currentRegistryVersion: registryVersion,
      contextA,
      statementA,
      contextB,
      statementB,
      newRegistryVersion: firstNewVersion,
    });

    // Same evidence with the two pairs swapped, a fresh new_registry_version so only
    // `punishment_case`'s `init` can collide — proves the evidence hash is pairwise
    // order-independent at the integration level too.
    try {
      await callPunishNonceReuse(fixture, {
        registryVersion,
        currentRegistryVersion: firstNewVersion,
        contextA: contextB,
        statementA: statementB,
        contextB: contextA,
        statementB: statementA,
        newRegistryVersion: firstNewVersion + 1,
      });
      assert.fail("expected replay to fail on punishment_case re-init");
    } catch (e: any) {
      assert.match(e.toString(), /already in use/i);
    }
  });

  it("does not compound the slash on a node already tombstoned by a prior nonce-reuse case", async () => {
    const fixture = await setupPunishmentFixture();
    const { program, registryVersion, material } = fixture;
    const ts = 1_700_701_700;
    const context: SigningContextFields = {
      sourceId: SOURCE_ID,
      signaturesRequired: SIGS_REQUIRED,
      canonicalTimestamp: ts,
    };
    const selection = deriveSignersBitmap(
      SOURCE_ID,
      registryVersion,
      SIGNER_COUNT,
      SIGS_REQUIRED,
      REDUNDANCY_BUFFER,
      ts
    );
    const selectionBits = bitsOfBitmap(selection);
    assert.equal(selectionBits.length, 4);
    const [s0, s1, s2, s3] = selectionBits;

    // Case #1: A = {s0,s1,s2}, B = {s1,s2,s3} -> guilty = {s1,s2}, both tombstoned at 50%.
    const bitmapA1 = clearBitEvm(selection, s3);
    const bitmapB1 = clearBitEvm(selection, s0);
    const value1 = Uint8Array.from(Buffer.alloc(32, 0x0f));
    const statementA1 = buildStatement({
      registryVersion,
      material,
      sourceId: SOURCE_ID,
      bitmap: bitmapA1,
      value: value1,
      canonicalTimestamp: ts,
      signaturesRequired: SIGS_REQUIRED,
    });
    const statementB1 = buildStatement({
      registryVersion,
      material,
      sourceId: SOURCE_ID,
      bitmap: bitmapB1,
      value: value1,
      canonicalTimestamp: ts,
      signaturesRequired: SIGS_REQUIRED,
      nonce: statementA1.nonce,
    });
    const versionAfterFirst = registryVersion + 1;
    await callPunishNonceReuse(fixture, {
      registryVersion,
      currentRegistryVersion: registryVersion,
      contextA: context,
      statementA: statementA1,
      contextB: context,
      statementB: statementB1,
      newRegistryVersion: versionAfterFirst,
    });

    const s1AfterFirst = await program.account.node.fetch(
      registeredNodePda(program.programId, s1)
    );
    assert.property(s1AfterFirst.status, "tombstoned");
    assert.equal(s1AfterFirst.lockedAmount.toNumber(), HALF_SLASH);
    const s2AfterFirst = await program.account.node.fetch(
      registeredNodePda(program.programId, s2)
    );
    assert.property(s2AfterFirst.status, "tombstoned");
    assert.equal(s2AfterFirst.lockedAmount.toNumber(), HALF_SLASH);

    // Case #2: A = {s0,s1,s3}, B = {s1,s3} -> guilty = {s1,s3}. s1 is already tombstoned (from
    // case #1) and must be skipped, not re-slashed; s3 is still live and drops to 50% for the
    // first time. `evidenceRegistry` stays pinned to the original `registryVersion` (Node
    // accounts are never closed, so an evicted node still resolves there); `currentRegistry`
    // advances to `versionAfterFirst`.
    const bitmapA2 = clearBitEvm(selection, s2);
    const bitmapB2 = clearBitEvm(bitmapA2, s0);
    const value2 = Uint8Array.from(Buffer.alloc(32, 0x10));
    const statementA2 = buildStatement({
      registryVersion,
      material,
      sourceId: SOURCE_ID,
      bitmap: bitmapA2,
      value: value2,
      canonicalTimestamp: ts,
      signaturesRequired: SIGS_REQUIRED,
    });
    const statementB2 = buildStatement({
      registryVersion,
      material,
      sourceId: SOURCE_ID,
      bitmap: bitmapB2,
      value: value2,
      canonicalTimestamp: ts,
      signaturesRequired: SIGS_REQUIRED,
      nonce: statementA2.nonce,
    });

    const { punishmentCasePda } = await callPunishNonceReuse(fixture, {
      registryVersion,
      currentRegistryVersion: versionAfterFirst,
      contextA: context,
      statementA: statementA2,
      contextB: context,
      statementB: statementB2,
      newRegistryVersion: versionAfterFirst + 1,
    });

    const kase = await program.account.punishmentCase.fetch(punishmentCasePda);
    // guilty_bitmap still reflects both bits (bitmap arithmetic doesn't know about prior
    // punishment), but slashed_total and the resulting node states show only the live one was
    // actually punished this time.
    assert.equal(kase.guiltyCount, 2);
    assert.equal(kase.slashedTotal.toNumber(), HALF_SLASH);

    const s1Again = await program.account.node.fetch(
      registeredNodePda(program.programId, s1)
    );
    assert.property(s1Again.status, "tombstoned");
    // Skipped, not compounded to 25%.
    assert.equal(s1Again.lockedAmount.toNumber(), HALF_SLASH);

    const s3After = await program.account.node.fetch(
      registeredNodePda(program.programId, s3)
    );
    assert.property(s3After.status, "tombstoned");
    assert.equal(s3After.lockedAmount.toNumber(), HALF_SLASH);
  });
});
