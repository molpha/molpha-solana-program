/**
 * MVP happy path: register → subscribe → create_job → submit_data_update →
 * verify_data_update on the current registry, with return-data checks.
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
import { generateE2eScenario } from "../helpers/e2e-fixture-export";
import {
  createJobViaInstruction,
  executeVerifyDataUpdateReturnData,
  expectedVerifyDataUpdateReturnBytes,
  generateE2eSignerMaterialFromSeed,
  getProtocolTreasury,
  registerNodesForE2e,
  signE2eDataUpdate,
  signerRemainingAccounts,
  VERIFY_DATA_UPDATE_RETURN_LEN,
} from "../helpers/evm-submit-data-update";

const PROTOCOL_CONFIG_SEED = Buffer.from("molpha_config");
const REGISTRY_STATE_SEED = Buffer.from("molpha_registry");
const PLAN_SEED = Buffer.from("molpha_plan");
const SUBSCRIPTION_SEED = Buffer.from("molpha_subscription");

const SIGNER_COUNT = 8;
const PLAN_ID = 1;
const E2E_SEED = "mvp-submit-then-verify-v1";

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

describe("submit_data_update then verify_data_update (current registry)", () => {
  it("submits an update then verifies return data matches value, timestamp, and round_id", async () => {
    const scenario = generateE2eScenario(E2E_SEED, SIGNER_COUNT);
    const ctx = await setupTestContext();
    const program = ctx.molphaProgram as anchor.Program;

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
        maxNodeCount: SIGNER_COUNT,
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

    const signerMaterial = generateE2eSignerMaterialFromSeed(E2E_SEED, SIGNER_COUNT);
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
      signerMaterial.compressedPubkeys,
      advanceClock
    );

    const registry = await program.account.registryState.fetch(registryStatePda);
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
      scenario.configHash
    );

    const signed = signE2eDataUpdate(
      registry.currentVersion,
      signerMaterial,
      SIGNER_COUNT,
      jobId,
      {
        configHash: scenario.configHash,
        value: scenario.value,
        canonicalTimestamp: scenario.canonicalTimestamp,
      }
    );

    const remaining = signerRemainingAccounts(
      program.programId,
      signed.submitArgs.signersBitmap
    );

    await program.methods
      .submitDataUpdate(signed.submitArgs)
      .accountsPartial({
        submitter: ctx.authority.publicKey,
        protocolConfig: protocolConfigPda,
        registryState: registryStatePda,
        transitionAccount: null,
        job: jobPda,
        feed: feedPda,
      })
      .remainingAccounts(remaining)
      .preInstructions([
        anchor.web3.ComputeBudgetProgram.setComputeUnitLimit({
          units: 1_400_000,
        }),
      ])
      .rpc();

    const feed = await program.account.feed.fetch(feedPda);
    assert.deepEqual(Array.from(feed.value), Array.from(scenario.value));
    assert.equal(feed.registryVersion, registry.currentVersion);
    assert.equal(
      feed.canonicalTimestamp.toString(),
      String(scenario.canonicalTimestamp)
    );

    const returnBytes = await executeVerifyDataUpdateReturnData(
      ctx,
      program,
      signed,
      { registryStatePda, jobPda }
    );

    assert.equal(returnBytes.length, VERIFY_DATA_UPDATE_RETURN_LEN);

    const expected = expectedVerifyDataUpdateReturnBytes(
      scenario.value,
      scenario.canonicalTimestamp,
      new Uint8Array(signed.submitArgs.roundId)
    );
    assert.deepEqual(Array.from(returnBytes), Array.from(expected));

    const recoveredTs = new DataView(returnBytes.buffer).getBigUint64(32, false);
    assert.equal(recoveredTs, BigInt(scenario.canonicalTimestamp));
    assert.deepEqual(
      Array.from(returnBytes.slice(0, 32)),
      Array.from(scenario.value)
    );
    assert.ok(returnBytes.slice(40, 72).every((b) => b === 0));
  });
});
