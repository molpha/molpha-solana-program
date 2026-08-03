/**
 * MVP happy path: register → subscribe → submit_data_update (lazy feed init).
 * Covers FeedValueKind::Value (≤32 bytes) and FeedValueKind::Hash (>32 bytes).
 */
import * as anchor from "@coral-xyz/anchor";
import { BN } from "@coral-xyz/anchor";
import { assert } from "chai";
import { PublicKey, SystemProgram } from "@solana/web3.js";
import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { keccak_256 } from "@noble/hashes/sha3";
import { Clock } from "solana-bankrun";
import { setupTestContext, TestContext } from "../setup";
import { generateE2eScenario } from "../helpers/e2e-fixture-export";
import {
  deriveFeedId,
  findFeedPda,
  generateE2eSignerMaterialFromSeed,
  getProtocolTreasury,
  registerNodesForE2e,
  signE2eDataUpdate,
  signerRemainingAccounts,
} from "../helpers/submit-data-update";

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

describe("submit_data_update (current registry)", () => {
  it("stores raw value (<=32) or keccak hash (>32) based on payload length", async () => {
    const scenario = generateE2eScenario(E2E_SEED, SIGNER_COUNT);
    const longValue = Buffer.alloc(64, 0xab);
    const expectedHash = Buffer.from(keccak_256(longValue));

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
    const treasury = getProtocolTreasury(
      protocolConfigPda,
      ctx.underlyingTokenMint
    );

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

    await program.methods
      .addPlan({
        planType: { standard: {} },
        subscriptionPrice: new BN(3_000_000),
        maxJobs: 4,
        maxSigners: SIGNER_COUNT,
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

    const signerMaterial = generateE2eSignerMaterialFromSeed(
      E2E_SEED,
      SIGNER_COUNT
    );
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

    const registry = await program.account.registryState.fetch(
      registryStatePda
    );
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

    // ---- ≤32 bytes → FeedValueKind::Value ----
    const feedId = deriveFeedId(
      ctx.authority.publicKey,
      scenario.configHash,
      SIGNER_COUNT
    );
    const feedPda = findFeedPda(program.programId, feedId);
    const signedValue = signE2eDataUpdate(
      registry.currentVersion,
      signerMaterial,
      SIGNER_COUNT,
      feedId,
      {
        configHash: scenario.configHash,
        value: scenario.value,
        canonicalTimestamp: scenario.canonicalTimestamp,
      }
    );

    await program.methods
      .submitDataUpdate(signedValue.submitArgs)
      .accountsPartial({
        submitter: ctx.authority.publicKey,
        registryState: registryStatePda,
        feed: feedPda,
        systemProgram: SystemProgram.programId,
      })
      .remainingAccounts(
        signerRemainingAccounts(
          program.programId,
          signedValue.submitArgs.signersBitmap
        )
      )
      .preInstructions([
        anchor.web3.ComputeBudgetProgram.setComputeUnitLimit({
          units: 1_400_000,
        }),
      ])
      .rpc();

    const feedValue = await program.account.feed.fetch(feedPda);
    assert.deepEqual(feedValue.valueKind, { value: {} });
    assert.deepEqual(Array.from(feedValue.value), Array.from(scenario.value));
    assert.equal(feedValue.registryVersion, registry.currentVersion);
    assert.equal(
      feedValue.canonicalTimestamp.toString(),
      String(scenario.canonicalTimestamp)
    );
    assert.equal(feedValue.signaturesRequired, SIGNER_COUNT);
    assert.deepEqual(
      Array.from(feedValue.signersBitmap),
      Array.from(signedValue.submitArgs.signersBitmap)
    );

    // ---- >32 bytes → FeedValueKind::Hash ----
    const longConfigHash = Buffer.from(keccak_256(Buffer.from("long-config")));
    const longFeedId = deriveFeedId(
      ctx.authority.publicKey,
      longConfigHash,
      SIGNER_COUNT
    );
    const longFeedPda = findFeedPda(program.programId, longFeedId);
    const signedHash = signE2eDataUpdate(
      registry.currentVersion,
      signerMaterial,
      SIGNER_COUNT,
      longFeedId,
      {
        configHash: longConfigHash,
        value: longValue,
        canonicalTimestamp: scenario.canonicalTimestamp + 1,
      }
    );

    await program.methods
      .submitDataUpdate(signedHash.submitArgs)
      .accountsPartial({
        submitter: ctx.authority.publicKey,
        registryState: registryStatePda,
        feed: longFeedPda,
        systemProgram: SystemProgram.programId,
      })
      .remainingAccounts(
        signerRemainingAccounts(
          program.programId,
          signedHash.submitArgs.signersBitmap
        )
      )
      .preInstructions([
        anchor.web3.ComputeBudgetProgram.setComputeUnitLimit({
          units: 1_400_000,
        }),
      ])
      .rpc();

    const feedHash = await program.account.feed.fetch(longFeedPda);
    assert.deepEqual(feedHash.valueKind, { hash: {} });
    assert.deepEqual(Array.from(feedHash.value), Array.from(expectedHash));
    assert.equal(feedHash.value.length, 32);
  });
});
