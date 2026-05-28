/**
 * Full bankrun e2e for the frozen `solana-compat-v10` fixture.
 *
 * The fixture's Schnorr signature is valid for the EVM literal `dataUpdate.jobId`
 * and `registryVersion: 10` with signersBitmap 255 (nodes 1–8). On-chain
 * `create_job` uses `deriveJobId(owner, configHash)`; `verify_data_update`
 * also requires `signers_bitmap ⊆ derive_group_bitmap(...)`.
 */
import * as anchor from "@coral-xyz/anchor";
import { BN } from "@coral-xyz/anchor";
import { assert } from "chai";
import { PublicKey, SystemProgram } from "@solana/web3.js";
import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { Clock } from "solana-bankrun";
import { setupTestContext, TestContext } from "../setup";
import fixture from "../fixtures-json/solana-compat-v10.json";
import {
  assertE2eFixtureIntegrity,
  buildSubmitArgsFromE2eFixture,
  createJobViaInstruction,
  deriveSignersBitmapForJob,
  evmUint256BitmapForFirstN,
  getProtocolTreasury,
  hexToBytes,
  LoadedE2eFixture,
  registerNodesForE2e,
  signE2eDataUpdate,
  signerMaterialFromFixture,
  signerRemainingAccounts,
  verifyFixtureSchnorrSignature,
} from "../helpers/evm-submit-data-update";

const PROTOCOL_CONFIG_SEED = Buffer.from("molpha_config");
const REGISTRY_STATE_SEED = Buffer.from("molpha_registry");
const PLAN_SEED = Buffer.from("molpha_plan");
const SUBSCRIPTION_SEED = Buffer.from("molpha_subscription");

const FIXTURE = fixture as LoadedE2eFixture;
const SIGNER_COUNT = FIXTURE.dataUpdate.signaturesRequired;
const PLAN_ID = 1;

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

describe("solana-compat-v10 fixture e2e", () => {
  before(() => {
    assertE2eFixtureIntegrity(FIXTURE);
  });

  it("fixture has required fields for coalition keys and compat payload", () => {
    assert.equal(FIXTURE.registeredNodeCount, 10);
    assert.equal(FIXTURE.registryVersion, 10);
    assert.equal(SIGNER_COUNT, 8);
    assert.equal(FIXTURE.secretKeys?.length, 8);
    assert.equal(FIXTURE.nodePubkeys.length, 10);
    assert.equal(FIXTURE.schnorrSignature.signersBitmap, 255);
  });

  it("frozen Schnorr signature recovers the fixture commitment (off-chain)", () => {
    verifyFixtureSchnorrSignature(FIXTURE);
  });

  it("on-chain: happy path, subset failure, and job_id seeds failure", async () => {
    const ctx = await setupTestContext();
    const program = ctx.molphaProgram as anchor.Program;
    const configHash = hexToBytes(FIXTURE.configHash);
    const value = hexToBytes(FIXTURE.dataUpdate.value);
    const coalitionPubkeys = FIXTURE.nodePubkeys
      .slice(0, SIGNER_COUNT)
      .map((h) => hexToBytes(h));
    const extraPubkeys = FIXTURE.nodePubkeys
      .slice(SIGNER_COUNT)
      .map((h) => hexToBytes(h));

    const [protocolConfigPda] = PublicKey.findProgramAddressSync(
      [PROTOCOL_CONFIG_SEED],
      program.programId
    );
    const [registryStatePda] = PublicKey.findProgramAddressSync(
      [REGISTRY_STATE_SEED],
      program.programId
    );
    const [planPda] = PublicKey.findProgramAddressSync(
      [PLAN_SEED, Buffer.from([PLAN_ID])],
      program.programId
    );
    const [subscriptionPda] = PublicKey.findProgramAddressSync(
      [SUBSCRIPTION_SEED, ctx.authority.publicKey.toBuffer()],
      program.programId
    );
    const treasury = getProtocolTreasury(protocolConfigPda, ctx.underlyingTokenMint);

    await program.methods
      .initialize({
        registryCooldownSecs: new BN(1),
        previousGraceSecs: new BN(1),
        withdrawalCooldownSecs: new BN(5),
        maxNodeCount: FIXTURE.registeredNodeCount,
        redundancyBuffer: 2,
        minimumNodeDeposit: new BN(100_000_000),
        rewardPerSignature: new BN(10),
        gatewayFee: new BN(20),
        receiptSubmitterBonus: new BN(5),
        protocolFeeBps: 100,
        maxReceiptDelaySlots: new BN(256),
        usdcMint: ctx.underlyingTokenMint,
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

    await program.methods
      .addPlan({
        planType: { standard: {} },
        subscriptionPrice: new BN(3_000_000),
        maxJobs: 4,
        signaturesRequired: SIGNER_COUNT,
        maxSigners: SIGNER_COUNT,
        privateApiEnabled: true,
        isActive: true,
      })
      .accountsPartial({
        protocolConfig: protocolConfigPda,
        plan: planPda,
        authority: ctx.authority.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    await advanceClock(ctx, 5);

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
      coalitionPubkeys,
      advanceClock
    );

    let registry = await program.account.registryState.fetch(registryStatePda);
    assert.equal(registry.currentNodeCount, SIGNER_COUNT);
    assert.equal(registry.currentVersion, SIGNER_COUNT);

    await program.methods
      .subscribe({ standard: {} })
      .accountsPartial({
        owner: ctx.authority.publicKey,
        protocolConfig: protocolConfigPda,
        plan: planPda,
        subscription: subscriptionPda,
        usdcMint: ctx.underlyingTokenMint,
        ownerUsdc: ctx.userTokenAccount,
        treasury,
        systemProgram: SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .rpc();

    const { jobPda, feedPda, jobId } = await createJobViaInstruction(
      ctx,
      program,
      { protocolConfigPda, planPda, subscriptionPda },
      configHash
    );

    const signersBitmap = deriveSignersBitmapForJob(
      jobId,
      registry.currentVersion,
      registry.currentNodeCount,
      SIGNER_COUNT,
      2,
      FIXTURE.dataUpdate.canonicalTimestamp
    );
    assert.deepEqual(
      Array.from(signersBitmap),
      Array.from(evmUint256BitmapForFirstN(SIGNER_COUNT))
    );

    const signerMaterial = signerMaterialFromFixture(FIXTURE);
    const signed = signE2eDataUpdate(
      registry.currentVersion,
      signerMaterial,
      signersBitmap,
      jobId,
      { configHash, value, canonicalTimestamp: FIXTURE.dataUpdate.canonicalTimestamp }
    );

    const remaining8 = signerRemainingAccounts(
      program.programId,
      signed.submitArgs.signersBitmap
    );

    await program.methods
      .verifyDataUpdate(signed.submitArgs)
      .accountsPartial({
        registryState: registryStatePda,
        job: jobPda,
      })
      .remainingAccounts(remaining8)
      .preInstructions([
        anchor.web3.ComputeBudgetProgram.setComputeUnitLimit({ units: 1_400_000 }),
      ])
      .rpc();

    await program.methods
      .submitDataUpdate(signed.submitArgs)
      .accountsPartial({
        submitter: ctx.authority.publicKey,
        registryState: registryStatePda,
        job: jobPda,
        feed: feedPda,
      })
      .remainingAccounts(remaining8)
      .preInstructions([
        anchor.web3.ComputeBudgetProgram.setComputeUnitLimit({ units: 1_400_000 }),
      ])
      .rpc();

    const feed = await program.account.feed.fetch(feedPda);
    assert.deepEqual(Array.from(feed.value), Array.from(value));
    assert.equal(feed.registryVersion, registry.currentVersion);

    // Register two more nodes → registry version 10 (matches fixture metadata).
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
      extraPubkeys,
      advanceClock,
      { startIndex: SIGNER_COUNT }
    );

    registry = await program.account.registryState.fetch(registryStatePda);
    assert.equal(registry.currentNodeCount, FIXTURE.registeredNodeCount);
    assert.equal(registry.currentVersion, FIXTURE.registryVersion);

    const frozenBitmap = evmUint256BitmapForFirstN(SIGNER_COUNT);
    const signedDense = signE2eDataUpdate(
      FIXTURE.registryVersion,
      signerMaterial,
      frozenBitmap,
      jobId,
      {
        configHash,
        value,
        canonicalTimestamp: FIXTURE.dataUpdate.canonicalTimestamp,
      }
    );
    const remainingDense = signerRemainingAccounts(
      program.programId,
      signedDense.submitArgs.signersBitmap
    );

    try {
      await program.methods
        .verifyDataUpdate(signedDense.submitArgs)
        .accountsPartial({
          registryState: registryStatePda,
          job: jobPda,
        })
        .remainingAccounts(remainingDense)
        .preInstructions([
          anchor.web3.ComputeBudgetProgram.setComputeUnitLimit({ units: 1_400_000 }),
        ])
        .rpc();
      assert.fail("expected SignersNotSubsetOfSelection for bitmap 255 with 10 nodes");
    } catch (error: unknown) {
      const message = String(error);
      assert.ok(
        message.includes("SignersNotSubsetOfSelection") || message.includes("6042"),
        `unexpected error: ${message}`
      );
    }

    const frozenArgs = buildSubmitArgsFromE2eFixture(FIXTURE);
    const remainingFrozen = signerRemainingAccounts(
      program.programId,
      frozenArgs.signersBitmap
    );

    try {
      await program.methods
        .verifyDataUpdate(frozenArgs)
        .accountsPartial({
          registryState: registryStatePda,
          job: jobPda,
        })
        .remainingAccounts(remainingFrozen)
        .rpc();
      assert.fail("expected seeds constraint for literal fixture job_id");
    } catch (error: unknown) {
      const message = String(error);
      assert.ok(
        message.includes("ConstraintSeeds") ||
          message.includes("Error Number: 2006") ||
          message.includes("A seeds constraint was violated"),
        `unexpected error: ${message}`
      );
    }
  });
});
