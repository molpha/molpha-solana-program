/**
 * Full on-chain e2e: plan → nodes → subscription → job → submit_data_update.
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
  createJobViaInstruction,
  evmRegistryIndexRemainingAccounts,
  generateE2eSignerMaterialFromSeed,
  getProtocolTreasury,
  registerNodesForE2e,
  signE2eDataUpdate,
} from "../helpers/evm-submit-data-update";

const PROTOCOL_CONFIG_SEED = Buffer.from("molpha_config");
const REGISTRY_STATE_SEED = Buffer.from("molpha_registry");
const PLAN_SEED = Buffer.from("molpha_plan");
const SUBSCRIPTION_SEED = Buffer.from("molpha_subscription");

const SIGNER_COUNT = 8;
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
    const scenario = generateE2eScenario(seed, SIGNER_COUNT);
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

    const treasury = getProtocolTreasury(protocolConfigPda, ctx.underlyingTokenMint);

    // initialize
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

    // add_plan (8-of-8 standard plan)
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

    // register nodes
    const signerMaterial = generateE2eSignerMaterialFromSeed(seed, SIGNER_COUNT);
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

    // create_job with random config hash
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
      .remainingAccounts(
        evmRegistryIndexRemainingAccounts(program.programId, SIGNER_COUNT)
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
    
    console.log("feed.roundId", feed.roundId);
    console.log("feed.value", feed.value);
    console.log("registry.currentVersion", registry.currentVersion);
    console.log("signed", signed);


    const fixture = buildVerifyAnswerEvmFixture({
      registeredNodeCount: SIGNER_COUNT,
      registryVersion: registry.currentVersion,
      configHash: scenario.configHash,
      nodePubkeys: signerMaterial.compressedPubkeys,
      nodePrivkeys: signerMaterial.privKeys,
      signerCount: SIGNER_COUNT,
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

    assert.equal(fixture.nodePubkeys.length, SIGNER_COUNT);
    assert.equal(fixture.dataUpdate.signaturesRequired, SIGNER_COUNT);
    assert.notEqual(
      fixture.configHash,
      "0x736f6c616e612d636f6d7061742d636667000000000000000000000000000000"
    );
  });
});
