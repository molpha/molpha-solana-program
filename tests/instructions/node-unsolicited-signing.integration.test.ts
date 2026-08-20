/**
 * `punish_unsolicited_signing` — offence #2: a valid Molpha attestation whose signer bitmap
 * contains a bit outside the duty's deterministic selection set. Guilty set is
 * `signers_bitmap & !selection`; sentence is a 100% slash + permanent tombstone, applied
 * immediately (no dispute window), exactly as `punish_equivocation` sentences its intersection.
 *
 * Mirrors `node-punishment.integration.test.ts`'s fixture/helper patterns (same bankrun-per-test
 * setup, same 6-node / sigsRequired=3 / redundancyBuffer=1 registry).
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
  differenceBitmap,
  equivocationEvidenceHash,
  findPunishmentCasePda,
  unionBitmap,
  unsolicitedSigningEvidenceHash,
} from "../helpers/node-punishment";

const PROTOCOL_CONFIG_SEED = Buffer.from("molpha_config");
const REGISTRY_STATE_SEED = Buffer.from("molpha_registry");

/**
 * Same workaround as `node-punishment.integration.test.ts`: `anchor.workspace.Molpha` is a
 * process-wide singleton bound to whichever provider was active first, so a fresh bankrun ledger
 * per `it()` needs a Program rebuilt from the fresh `ctx.provider`.
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
const BOUNTY_BPS = 2_000; // 20%
const E2E_SEED = "node-unsolicited-signing-v1";
const SOURCE_ID = Uint8Array.from(Buffer.alloc(32, 0x55));

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
      agentRoundBase: new BN(1_000),
      agentRoundPerSig: new BN(15),
      agentEpochLenSecs: new BN(86_400),
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

async function callPunishUnsolicitedSigning(
  fixture: PunishmentFixture,
  params: {
    registryVersion: number;
    currentRegistryVersion: number;
    signaturesRequired: number;
    canonicalTimestamp: number;
    statement: SignedStatementBytes;
    newRegistryVersion: number;
    /** Override the computed evidence hash, for the mismatch test. */
    evidenceHashOverride?: Uint8Array;
  }
): Promise<{ punishmentCasePda: PublicKey; newRegistryPda: PublicKey }> {
  const { program } = fixture;
  const evidenceHash =
    params.evidenceHashOverride ??
    unsolicitedSigningEvidenceHash({
      sourceId: SOURCE_ID,
      registryVersion: params.registryVersion,
      signaturesRequired: params.signaturesRequired,
      canonicalTimestamp: params.canonicalTimestamp,
      statement: params.statement,
    });

  const args = {
    sourceId: Array.from(SOURCE_ID),
    registryVersion: params.registryVersion,
    signaturesRequired: params.signaturesRequired,
    canonicalTimestamp: new BN(params.canonicalTimestamp),
    statement: toAnchorStatement(params.statement),
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
  const remainingAccounts = bitsOfBitmap(params.statement.signersBitmap).map(
    (bit) => ({
      pubkey: registeredNodePda(program.programId, bit),
      isWritable: true,
      isSigner: false,
    })
  );

  await program.methods
    .punishUnsolicitedSigning(args)
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

/** Selection bitmap plus one extra, unselected bit — the shape every test here signs over. */
function selectionPlusOneUnselected(
  registryVersion: number,
  canonicalTimestamp: number
): { selection: Uint8Array; bitmap: Uint8Array; unselectedBit: number } {
  const selection = deriveSignersBitmap(
    SOURCE_ID,
    registryVersion,
    SIGNER_COUNT,
    SIGS_REQUIRED,
    REDUNDANCY_BUFFER,
    canonicalTimestamp
  );
  const selectionBits = bitsOfBitmap(selection);
  const unselectedBit = [...Array(SIGNER_COUNT).keys()].find(
    (i) => !selectionBits.includes(i)
  )!;
  const bitmap = Uint8Array.from(selection);
  setBitEvm(bitmap, unselectedBit);
  return { selection, bitmap, unselectedBit };
}

describe("punish_unsolicited_signing", () => {
  it("slashes and tombstones exactly the unselected signer, evicts it, and pays the reporter bounty", async () => {
    const fixture = await setupPunishmentFixture();
    const { program, registryVersion, material } = fixture;
    const canonicalTimestamp = 1_700_600_000;
    const { selection, bitmap, unselectedBit } = selectionPlusOneUnselected(
      registryVersion,
      canonicalTimestamp
    );
    const selectionBits = bitsOfBitmap(selection);

    const statement = buildStatement({
      registryVersion,
      material,
      bitmap,
      value: Uint8Array.from(Buffer.alloc(32, 0x11)),
      canonicalTimestamp,
      signaturesRequired: SIGS_REQUIRED,
    });

    const reporterUsdcBefore = await readTokenAmount(
      fixture.ctx,
      fixture.ctx.userTokenAccount
    );

    const newRegistryVersion = registryVersion + 1;
    const { punishmentCasePda, newRegistryPda } =
      await callPunishUnsolicitedSigning(fixture, {
        registryVersion,
        currentRegistryVersion: registryVersion,
        signaturesRequired: SIGS_REQUIRED,
        canonicalTimestamp,
        statement,
        newRegistryVersion,
      });

    const victimPda = registeredNodePda(program.programId, unselectedBit);
    const victim = await program.account.node.fetch(victimPda);
    assert.property(victim.status, "tombstoned");
    assert.equal(victim.lockedAmount.toNumber(), 0);
    assert.equal(victim.claimableRewards.toNumber(), 0);
    assert.isAbove(victim.punishedAt.toNumber(), 0);

    for (const bit of selectionBits) {
      const cosigner = await program.account.node.fetch(
        registeredNodePda(program.programId, bit)
      );
      assert.property(cosigner.status, "active");
      assert.equal(cosigner.lockedAmount.toNumber(), MIN_DEPOSIT);
    }

    const newRegistry = await program.account.registry.fetch(newRegistryPda);
    assert.equal(newRegistry.nodeCount, SIGNER_COUNT - 1);
    const remainingKeys: string[] = newRegistry.nodes
      .slice(0, Number(newRegistry.nodeCount))
      .map((raw: number[]) => new PublicKey(Uint8Array.from(raw)).toBase58());
    assert.notInclude(remainingKeys, victimPda.toBase58());
    for (const bit of selectionBits) {
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
    assert.property(kase.offence, "unsolicitedSigning");
    assert.equal(kase.guiltyCount, 1);
    assert.deepEqual(
      Array.from(kase.guiltyBitmap),
      Array.from(differenceBitmap(bitmap, selection))
    );
    assert.equal(
      kase.reporter.toBase58(),
      fixture.ctx.authority.publicKey.toBase58()
    );

    const expectedSlashed = MIN_DEPOSIT;
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

  it("rejects a fully-selected signer bitmap", async () => {
    const fixture = await setupPunishmentFixture();
    const { registryVersion, material } = fixture;
    const canonicalTimestamp = 1_700_600_100;
    const selection = deriveSignersBitmap(
      SOURCE_ID,
      registryVersion,
      SIGNER_COUNT,
      SIGS_REQUIRED,
      REDUNDANCY_BUFFER,
      canonicalTimestamp
    );

    const statement = buildStatement({
      registryVersion,
      material,
      bitmap: selection,
      value: Uint8Array.from(Buffer.alloc(32, 0x22)),
      canonicalTimestamp,
      signaturesRequired: SIGS_REQUIRED,
    });

    try {
      await callPunishUnsolicitedSigning(fixture, {
        registryVersion,
        currentRegistryVersion: registryVersion,
        signaturesRequired: SIGS_REQUIRED,
        canonicalTimestamp,
        statement,
        newRegistryVersion: registryVersion + 1,
      });
      assert.fail("expected SignersFullySelected");
    } catch (e: any) {
      assert.include(e.toString(), "SignersFullySelected");
    }
  });

  it("rejects a tampered aggregate signature", async () => {
    const fixture = await setupPunishmentFixture();
    const { registryVersion, material } = fixture;
    const canonicalTimestamp = 1_700_600_200;
    const { bitmap } = selectionPlusOneUnselected(
      registryVersion,
      canonicalTimestamp
    );

    const statement = buildStatement({
      registryVersion,
      material,
      bitmap,
      value: Uint8Array.from(Buffer.alloc(32, 0x33)),
      canonicalTimestamp,
      signaturesRequired: SIGS_REQUIRED,
    });
    const tamperedSigS = Uint8Array.from(statement.aggSigS);
    tamperedSigS[0] ^= 0xff;
    statement.aggSigS = tamperedSigS;

    try {
      await callPunishUnsolicitedSigning(fixture, {
        registryVersion,
        currentRegistryVersion: registryVersion,
        signaturesRequired: SIGS_REQUIRED,
        canonicalTimestamp,
        statement,
        newRegistryVersion: registryVersion + 1,
      });
      assert.fail("expected InvalidAggregateSignature");
    } catch (e: any) {
      assert.include(e.toString(), "InvalidAggregateSignature");
    }
  });

  it("rejects an evidence hash that does not match the supplied fields", async () => {
    const fixture = await setupPunishmentFixture();
    const { registryVersion, material } = fixture;
    const canonicalTimestamp = 1_700_600_300;
    const { bitmap } = selectionPlusOneUnselected(
      registryVersion,
      canonicalTimestamp
    );

    const statement = buildStatement({
      registryVersion,
      material,
      bitmap,
      value: Uint8Array.from(Buffer.alloc(32, 0x44)),
      canonicalTimestamp,
      signaturesRequired: SIGS_REQUIRED,
    });

    // Hash computed over a different canonical_timestamp than the args carry.
    const wrongHash = unsolicitedSigningEvidenceHash({
      sourceId: SOURCE_ID,
      registryVersion,
      signaturesRequired: SIGS_REQUIRED,
      canonicalTimestamp: canonicalTimestamp + 1,
      statement,
    });

    try {
      await callPunishUnsolicitedSigning(fixture, {
        registryVersion,
        currentRegistryVersion: registryVersion,
        signaturesRequired: SIGS_REQUIRED,
        canonicalTimestamp,
        statement,
        newRegistryVersion: registryVersion + 1,
        evidenceHashOverride: wrongHash,
      });
      assert.fail("expected EvidenceHashMismatch");
    } catch (e: any) {
      assert.include(e.toString(), "EvidenceHashMismatch");
    }
  });

  it("rejects replaying the same evidence", async () => {
    const fixture = await setupPunishmentFixture();
    const { registryVersion, material } = fixture;
    const canonicalTimestamp = 1_700_600_400;
    const { bitmap } = selectionPlusOneUnselected(
      registryVersion,
      canonicalTimestamp
    );

    const statement = buildStatement({
      registryVersion,
      material,
      bitmap,
      value: Uint8Array.from(Buffer.alloc(32, 0x55)),
      canonicalTimestamp,
      signaturesRequired: SIGS_REQUIRED,
    });

    const firstNewVersion = registryVersion + 1;
    await callPunishUnsolicitedSigning(fixture, {
      registryVersion,
      currentRegistryVersion: registryVersion,
      signaturesRequired: SIGS_REQUIRED,
      canonicalTimestamp,
      statement,
      newRegistryVersion: firstNewVersion,
    });

    // Same evidence, a fresh new_registry_version so only `punishment_case`'s `init` can collide.
    try {
      await callPunishUnsolicitedSigning(fixture, {
        registryVersion,
        currentRegistryVersion: firstNewVersion,
        signaturesRequired: SIGS_REQUIRED,
        canonicalTimestamp,
        statement,
        newRegistryVersion: firstNewVersion + 1,
      });
      assert.fail("expected replay to fail on punishment_case re-init");
    } catch (e: any) {
      assert.match(e.toString(), /already in use/i);
    }
  });

  it("skips an already-tombstoned guilty signer and still punishes the rest", async () => {
    const fixture = await setupPunishmentFixture();
    const { program, registryVersion, material } = fixture;

    // Duty #1 (earlier timestamp, different selection): tombstone `firstUnselected` on its own,
    // via this same offence, so it enters duty #2 already terminal. Picked so its own
    // lowest-unselected-bit choice also lands outside duty #2's independently-derived selection
    // (verified by the `notInclude` precondition below, not assumed).
    const earlierTimestamp = 1_700_600_442;
    const {
      bitmap: earlierBitmap,
      unselectedBit: firstUnselected,
    } = selectionPlusOneUnselected(registryVersion, earlierTimestamp);
    const earlierStatement = buildStatement({
      registryVersion,
      material,
      bitmap: earlierBitmap,
      value: Uint8Array.from(Buffer.alloc(32, 0x66)),
      canonicalTimestamp: earlierTimestamp,
      signaturesRequired: SIGS_REQUIRED,
    });
    const versionAfterFirst = registryVersion + 1;
    await callPunishUnsolicitedSigning(fixture, {
      registryVersion,
      currentRegistryVersion: registryVersion,
      signaturesRequired: SIGS_REQUIRED,
      canonicalTimestamp: earlierTimestamp,
      statement: earlierStatement,
      newRegistryVersion: versionAfterFirst,
    });
    const tombstoned = await program.account.node.fetch(
      registeredNodePda(program.programId, firstUnselected)
    );
    assert.property(tombstoned.status, "tombstoned");

    // Duty #2 (later timestamp): its own selection, plus both `firstUnselected` (already
    // tombstoned, but still resolvable by pubkey against the pinned evidence-version registry
    // since Node accounts are never closed) and a second, still-live unselected node.
    const canonicalTimestamp = 1_700_600_500;
    const selection = deriveSignersBitmap(
      SOURCE_ID,
      registryVersion,
      SIGNER_COUNT,
      SIGS_REQUIRED,
      REDUNDANCY_BUFFER,
      canonicalTimestamp
    );
    const selectionBits = bitsOfBitmap(selection);
    // Duty #2's selection is an independent deterministic draw (different timestamp seed), so
    // `firstUnselected` isn't guaranteed to fall outside it — assert it does, so this case
    // actually exercises "guilty and already tombstoned" rather than silently degrading into the
    // single-victim happy path.
    assert.notInclude(
      selectionBits,
      firstUnselected,
      "test fixture assumption broken: pick a different earlierTimestamp/canonicalTimestamp pair"
    );
    const secondUnselected = [...Array(SIGNER_COUNT).keys()].find(
      (i) => !selectionBits.includes(i) && i !== firstUnselected
    )!;
    const bothUnselectedBitmap = Uint8Array.from(selection);
    setBitEvm(bothUnselectedBitmap, firstUnselected);
    setBitEvm(bothUnselectedBitmap, secondUnselected);
    const secondStatement = buildStatement({
      registryVersion,
      material,
      bitmap: bothUnselectedBitmap,
      value: Uint8Array.from(Buffer.alloc(32, 0x77)),
      canonicalTimestamp,
      signaturesRequired: SIGS_REQUIRED,
    });

    const { punishmentCasePda } = await callPunishUnsolicitedSigning(fixture, {
      registryVersion,
      currentRegistryVersion: versionAfterFirst,
      signaturesRequired: SIGS_REQUIRED,
      canonicalTimestamp,
      statement: secondStatement,
      newRegistryVersion: versionAfterFirst + 1,
    });

    const kase = await program.account.punishmentCase.fetch(punishmentCasePda);
    // guilty_bitmap still reflects both bits (bitmap arithmetic doesn't know about prior
    // punishment), but slashed_total and the resulting node states show only the live one was
    // actually punished this time.
    assert.equal(kase.guiltyCount, 2);
    assert.equal(kase.slashedTotal.toNumber(), MIN_DEPOSIT);

    const secondVictim = await program.account.node.fetch(
      registeredNodePda(program.programId, secondUnselected)
    );
    assert.property(secondVictim.status, "tombstoned");
    assert.equal(secondVictim.lockedAmount.toNumber(), 0);

    // The already-tombstoned node's balances were already zero from the first punishment and are
    // untouched (not re-slashed, no error) by this second case.
    const firstVictimAgain = await program.account.node.fetch(
      registeredNodePda(program.programId, firstUnselected)
    );
    assert.property(firstVictimAgain.status, "tombstoned");
    assert.equal(firstVictimAgain.lockedAmount.toNumber(), 0);
  });
});
