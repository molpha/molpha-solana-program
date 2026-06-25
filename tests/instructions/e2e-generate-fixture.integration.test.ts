/**
 * Full on-chain e2e: plan → nodes → subscription → submit_data_update.
 * Uses a seeded random payload (not verify-answer-evm.json) and writes a
 * compatible JSON fixture for EVM / CU benches / offline verification.
 *
 * Env:
 *   E2E_FIXTURE_SEED      — optional hex seed (default: random per run)
 *   E2E_FIXTURE_OUTPUT    — optional output path (default: tests/fixtures-json/e2e-generated-<seed>.json)
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
import {
  defaultE2eFixtureOutputPath,
  generateE2eScenario,
  randomE2eScenarioSeed,
  buildVerifyAnswerEvmFixture,
  writeVerifyAnswerEvmFixture,
} from "../helpers/e2e-fixture-export";
import {
  deriveJobId,
  findFeedPda,
  findJobPda,
  generateE2eSignerMaterialFromSeed,
  signerRemainingAccounts,
  getProtocolTreasury,
  registerNodesForE2e,
  signE2eDataUpdate,
} from "../helpers/evm-submit-data-update";

const PROTOCOL_CONFIG_SEED = Buffer.from("molpha_config");
const REGISTRY_STATE_SEED = Buffer.from("molpha_registry");
const PLAN_SEED = Buffer.from("molpha_plan");
const SUBSCRIPTION_SEED = Buffer.from("molpha_subscription");

const NODE_COUNT = 10;
const SIGNATURES_REQUIRED = 3;
const REDUNDANCY_BUFFER = 2;
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

describe("e2e generate verify-answer-evm fixture", () => {
  it("runs full flow and writes JSON fixture", async () => {
    const seed = process.env.E2E_FIXTURE_SEED ?? randomE2eScenarioSeed();
    const scenario = generateE2eScenario(seed, SIGNATURES_REQUIRED);
    const outputPath = defaultE2eFixtureOutputPath(seed);

    const ctx = await setupTestContext();
    const program = ctx.molphaProgram as any;

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

    const treasury = getProtocolTreasury(
      protocolConfigPda,
      ctx.underlyingTokenMint
    );

    // initialize
    await program.methods
      .initialize({
        registryCooldownSecs: new BN(1),
        previousGraceSecs: new BN(1),
        withdrawalCooldownSecs: new BN(5),
        maxNodeCount: NODE_COUNT,
        redundancyBuffer: REDUNDANCY_BUFFER,
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

    // add_plan (3-of-10 standard plan with redundancy buffer 2)
    await program.methods
      .addPlan({
        planType: { standard: {} },
        subscriptionPrice: new BN(3_000_000),
        maxJobs: 4,
        maxSigners: SIGNATURES_REQUIRED,
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

    // register nodes
    const signerMaterial = generateE2eSignerMaterialFromSeed(seed, NODE_COUNT);
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

    const registry = await program.account.registryState.fetch(
      registryStatePda
    );
    assert.equal(registry.currentNodeCount, NODE_COUNT);

    // subscribe
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

    const jobId = deriveJobId(ctx.authority.publicKey, scenario.configHash);
    const jobPda = findJobPda(program.programId, jobId);
    const feedPda = findFeedPda(program.programId, jobId);

    const signed = signE2eDataUpdate(
      registry.currentVersion,
      signerMaterial,
      NODE_COUNT,
      jobId,
      {
        configHash: scenario.configHash,
        value: scenario.value,
        canonicalTimestamp: scenario.canonicalTimestamp,
      },
      SIGNATURES_REQUIRED,
      NODE_COUNT,
      REDUNDANCY_BUFFER
    );

    await program.methods
      .submitDataUpdate(signed.submitArgs, {
        apiConfigHash: Array.from(scenario.configHash),
        decimals: 8,
      })
      .accountsPartial({
        submitter: ctx.authority.publicKey,
        jobOwner: ctx.authority.publicKey,
        plan: planPda,
        subscription: subscriptionPda,
        delegate: null,
        registryState: registryStatePda,
        job: jobPda,
        feed: feedPda,
        systemProgram: SystemProgram.programId,
      })
      .remainingAccounts(
        signerRemainingAccounts(
          program.programId,
          signed.submitArgs.signersBitmap
        )
      )
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

    console.log("feed.value", feed.value);
    console.log("registry.currentVersion", registry.currentVersion);
    console.log("signed", signed);

    const fixture = buildVerifyAnswerEvmFixture({
      registeredNodeCount: NODE_COUNT,
      registryVersion: registry.currentVersion,
      configHash: scenario.configHash,
      nodePubkeys: signerMaterial.compressedPubkeys,
      nodePrivkeys: signerMaterial.privKeys,
      signaturesRequired: SIGNATURES_REQUIRED,
      jobId,
      dataUpdate: {
        configHash: scenario.configHash,
        value: scenario.value,
        canonicalTimestamp: scenario.canonicalTimestamp,
      },
      signed,
    });

    writeVerifyAnswerEvmFixture(outputPath, fixture);

    // eslint-disable-next-line no-console
    console.log(
      JSON.stringify({
        seed,
        outputPath,
        registryVersion: registry.currentVersion,
        jobId: fixture.dataUpdate.jobId,
      })
    );

    assert.equal(fixture.nodePubkeys.length, NODE_COUNT);
    assert.equal(fixture.dataUpdate.signaturesRequired, SIGNATURES_REQUIRED);
    assert.notEqual(
      fixture.configHash,
      "0x736f6c616e612d636f6d7061742d636667000000000000000000000000000000"
    );
  });
});
