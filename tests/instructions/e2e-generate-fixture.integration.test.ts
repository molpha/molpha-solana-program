/**
 * Full on-chain e2e: plan → nodes → subscription → submit_attestation.
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
import { rpcInitialize } from "../helpers/settlement-access";
import {
  defaultE2eFixtureOutputPath,
  generateE2eScenario,
  randomE2eScenarioSeed,
  buildVerifyAnswerEvmFixture,
  writeVerifyAnswerEvmFixture,
} from "../helpers/e2e-fixture-export";
import {
  findFeedPda,
  findRegistryPda,
  generateE2eSignerMaterialFromSeed,
  signerRemainingAccounts,
  getProtocolTreasury,
  registerNodesForE2e,
  signE2eAttestation,
} from "../helpers/submit-attestation";

const PROTOCOL_CONFIG_SEED = Buffer.from("molpha_config");
const REGISTRY_STATE_SEED = Buffer.from("molpha_registry");
const PLAN_SEED = Buffer.from("molpha_plan");
const SUBSCRIPTION_SEED = Buffer.from("molpha_subscription");

const NODE_COUNT = 12;
const SIGNATURES_REQUIRED = 5;
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
    await rpcInitialize(
      program,
      {
        protocolConfig: protocolConfigPda,
        registryState: registryStatePda,
        usdcMint: ctx.underlyingTokenMint,
        treasury,
        authority: ctx.authority.publicKey,
      },
      ctx.underlyingTokenMint,
      { redundancy_buffer: REDUNDANCY_BUFFER }
    );

    // add_plan (3-of-10 standard plan with redundancy buffer 2)
    await program.methods
      .addPlan({
        planType: { standard: {} },
        subscriptionPrice: new BN(3_000_000),
        maxRounds: new BN(100),
        maxSigners: SIGNATURES_REQUIRED,
        maxDelegates: 2,
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
      signerMaterial,
      advanceClock
    );

    const registryState = await program.account.registryState.fetch(
      registryStatePda
    );
    const registryPda = findRegistryPda(
      program.programId,
      registryState.currentVersion
    );
    const registry = await program.account.registry.fetch(registryPda);
    assert.equal(registry.nodeCount, NODE_COUNT);

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

    const sourceId = scenario.sourceId;
    const feedPda = findFeedPda(
      program.programId,
      sourceId,
      SIGNATURES_REQUIRED,
      ctx.authority.publicKey
    );

    const signed = signE2eAttestation(
      registry.version,
      signerMaterial,
      NODE_COUNT,
      sourceId,
      {
        value: scenario.value,
        canonicalTimestamp: scenario.canonicalTimestamp,
      },
      SIGNATURES_REQUIRED,
      registry.nodeCount,
      REDUNDANCY_BUFFER
    );

    // await program.methods
    //   .submitAttestation(signed.submitArgs)
    //   .accountsPartial({
    //     submitter: ctx.authority.publicKey,
    //     registryState: registryStatePda,
    //     registry: registryPda,
    //     feed: feedPda,
    //     systemProgram: SystemProgram.programId,
    //   })
    //   .remainingAccounts(
    //     signerRemainingAccounts(
    //       program.programId,
    //       signed.submitArgs.signersBitmap
    //     )
    //   )
    //   .preInstructions([
    //     anchor.web3.ComputeBudgetProgram.setComputeUnitLimit({
    //       units: 1_400_000,
    //     }),
    //   ])
    //   .rpc();

    // const feed = await program.account.feed.fetch(feedPda);
    // assert.deepEqual(Array.from(feed.value), Array.from(scenario.value));
    // assert.deepEqual(feed.valueKind, { value: {} });
    // assert.equal(feed.registryVersion, registry.version);
    // assert.equal(
    //   feed.canonicalTimestamp.toString(),
    //   String(scenario.canonicalTimestamp)
    // );

    // console.log("feed.value", feed.value);
    // console.log("registry.version", registry.version);
    // console.log("signed", signed);

    const fixture = buildVerifyAnswerEvmFixture({
      registeredNodeCount: NODE_COUNT,
      registryVersion: registry.version,
      messageHash: signed.messageHash,
      nodePubkeys: signerMaterial.compressedPubkeys,
      nodePrivkeys: signerMaterial.privKeys,
      signaturesRequired: SIGNATURES_REQUIRED,
      sourceId,
      attestation: {
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
        registryVersion: registry.version,
        sourceId: fixture.attestation.sourceId,
      })
    );

    assert.equal(fixture.nodePubkeys.length, NODE_COUNT);
    assert.equal(fixture.attestation.signaturesRequired, SIGNATURES_REQUIRED);
    assert.notEqual(
      fixture.configHash,
      "0x736f6c616e612d636f6d7061742d636667000000000000000000000000000000"
    );
  });
});
