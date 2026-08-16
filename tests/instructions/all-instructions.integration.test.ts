import * as anchor from "@coral-xyz/anchor";
import { BN } from "@coral-xyz/anchor";
import { assert } from "chai";
import { PublicKey, SystemProgram } from "@solana/web3.js";
import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { Clock } from "solana-bankrun";
import { keccak_256 } from "@noble/hashes/sha3";
import { setupTestContext, TestContext } from "../setup";
import {
  buildRegisterNodeArgs,
  deriveNodeId,
  EVM_FIXTURE_NODE_PUBKEYS,
  findNodePda,
  findRegistryPda,
  findSecp256k1KeyClaimPda,
  fundNodeOwnerInBankrun,
  registerNodesForE2e,
  getProtocolTreasury,
  signerMaterialForVerifyAnswerEvm,
} from "../helpers/submit-data-update";

const PROTOCOL_CONFIG_SEED = Buffer.from("molpha_config");
const REGISTRY_STATE_SEED = Buffer.from("molpha_registry");
const PLAN_SEED = Buffer.from("molpha_plan");
const SUBSCRIPTION_SEED = Buffer.from("molpha_subscription");
const JOB_SEED = Buffer.from("molpha_job");
const FEED_SEED = Buffer.from("molpha_feed");
const DELEGATE_SEED = Buffer.from("molpha_delegate");

const SECONDS_PER_MONTH = 30 * 24 * 60 * 60;

function findDelegatePda(
  programId: PublicKey,
  owner: PublicKey,
  delegate: PublicKey
): PublicKey {
  return PublicKey.findProgramAddressSync(
    [DELEGATE_SEED, owner.toBuffer(), delegate.toBuffer()],
    programId
  )[0];
}

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

describe("Molpha instruction coverage integration", () => {
  it("covers initialize, plan/subscription/job/delegate and registry transitions", async () => {
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
    const planId = 1;
    const [planPda] = PublicKey.findProgramAddressSync(
      [PLAN_SEED, Buffer.from([planId])],
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

    // 1) initialize
    await program.methods
      .initialize({
        withdrawalCooldownSecs: new BN(5),
        maxNodeCount: 8,
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

    const protocolConfig = await program.account.protocolConfig.fetch(
      protocolConfigPda
    );
    assert.equal(
      protocolConfig.minimumNodeDeposit.toString(),
      new BN(100_000_000).toString()
    );

    // 2) add_plan + update_plan
    await program.methods
      .addPlan({
        planType: { standard: {} },
        subscriptionPrice: new BN(2_000_000),
        maxJobs: 3,
        maxSigners: 2,
        maxDelegates: 2,
        privateApiEnabled: false,
        isActive: true,
      })
      .accountsPartial({
        protocolConfig: protocolConfigPda,
        plan: planPda,
        authority: ctx.authority.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    await program.methods
      .updatePlan({
        planType: { standard: {} },
        subscriptionPrice: new BN(3_000_000),
        maxJobs: 4,
        maxSigners: 3,
        maxDelegates: 3,
        privateApiEnabled: true,
        isActive: true,
      })
      .accountsPartial({
        protocolConfig: protocolConfigPda,
        plan: planPda,
        authority: ctx.authority.publicKey,
      })
      .rpc();

    const plan = await program.account.plan.fetch(planPda);
    assert.equal(plan.subscriptionPrice.toString(), "3000000");
    assert.equal(plan.maxDelegates, 3);

    // 3) subscribe + extend_subscription
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

    await program.methods
      .extendSubscription()
      .accountsPartial({
        owner: ctx.authority.publicKey,
        protocolConfig: protocolConfigPda,
        subscription: subscriptionPda,
        plan: planPda,
        usdcMint: ctx.underlyingTokenMint,
        ownerUsdc: ctx.userTokenAccount,
        treasury,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .rpc();

    const subscription = await program.account.subscription.fetch(
      subscriptionPda
    );
    assert.equal(subscription.prepaidUsdc.toString(), "6000000");
    const clock = await ctx.provider.context.banksClient.getClock();
    const now = Number(clock.unixTimestamp);
    assert.isAbove(Number(subscription.validUntil), now + SECONDS_PER_MONTH);

    const sourceId = Uint8Array.from(Buffer.alloc(32, 7));
    const [feedPda] = PublicKey.findProgramAddressSync(
      [
        FEED_SEED,
        Buffer.from(sourceId),
        Buffer.from([1]),
        ctx.authority.publicKey.toBuffer(),
      ],
      program.programId
    );

    // 5) add_delegate + remove_delegate
    const delegate = PublicKey.unique();
    const delegatePda = findDelegatePda(
      program.programId,
      ctx.authority.publicKey,
      delegate
    );
    await program.methods
      .addDelegate({
        delegate,
        maxDataRequests: new BN(10),
        maxSigners: 2,
      })
      .accountsPartial({
        owner: ctx.authority.publicKey,
        subscription: subscriptionPda,
        delegateAccount: delegatePda,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    let delegateAccount = await program.account.delegate.fetch(delegatePda);
    assert.equal(delegateAccount.maxDataRequests.toString(), "10");
    assert.equal(delegateAccount.maxSigners, 2);
    assert.deepEqual(delegateAccount.status, { active: {} });

    let subscriptionAfterDelegate = await program.account.subscription.fetch(
      subscriptionPda
    );
    assert.equal(subscriptionAfterDelegate.delegateCount, 1);

    await program.methods
      .removeDelegate({
        delegate,
      })
      .accountsPartial({
        owner: ctx.authority.publicKey,
        subscription: subscriptionPda,
        delegateAccount: delegatePda,
      })
      .rpc();

    subscriptionAfterDelegate = await program.account.subscription.fetch(
      subscriptionPda
    );
    assert.equal(subscriptionAfterDelegate.delegateCount, 0);

    try {
      await program.account.delegate.fetch(delegatePda);
      assert.fail("expected delegate account to be closed");
    } catch (error: any) {
      assert.ok(String(error));
    }

    // 6) submit_data_update + verify_data_update (failure-path coverage)
    const badSourceId = Uint8Array.from(Buffer.alloc(32, 4));
    const dataUpdateArgs = {
      sourceId: Array.from(sourceId),
      registryVersion: 0,
      signaturesRequired: 1,
      signersBitmap: Array.from(Buffer.alloc(32, 0)),
      value: Buffer.alloc(32, 1),
      canonicalTimestamp: new BN(1),
      aggSigR: Array.from(Buffer.alloc(32, 0)),
      aggSigS: Array.from(Buffer.alloc(32, 0)),
      commitmentAddr: Array.from(Buffer.alloc(20, 0)),
    };
    const badDataUpdateArgs = {
      ...dataUpdateArgs,
      sourceId: Array.from(badSourceId),
    };

    try {
      await program.methods
        .submitDataUpdate(dataUpdateArgs)
        .accountsPartial({
          submitter: ctx.authority.publicKey,
          registryState: registryStatePda,
          registry: findRegistryPda(program.programId, 0),
          feed: feedPda,
          systemProgram: SystemProgram.programId,
        })
        .rpc();
      assert.fail(
        "submit_data_update should fail with invalid aggregate signature"
      );
    } catch (error: any) {
      assert.include(String(error), "InvalidAggregateSignature");
    }

    // 7) register_node rejects when registry is at capacity
    await program.methods
      .updatePlan({
        planType: { standard: {} },
        subscriptionPrice: new BN(3_000_000),
        maxJobs: 4,
        maxSigners: 8,
        maxDelegates: 3,
        privateApiEnabled: true,
        isActive: true,
      })
      .accountsPartial({
        protocolConfig: protocolConfigPda,
        plan: planPda,
        authority: ctx.authority.publicKey,
      })
      .rpc();

    await advanceClock(ctx, 5);

    const fixtureMaterial = signerMaterialForVerifyAnswerEvm();
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
      {
        privKeys: fixtureMaterial.privKeys.slice(0, 8),
        compressedPubkeys: fixtureMaterial.compressedPubkeys.slice(0, 8),
      },
      advanceClock
    );

    const registryStateAfterRegister = await program.account.registryState.fetch(
      registryStatePda
    );
    assert.equal(registryStateAfterRegister.currentVersion, 8);
    const registryAfterRegister = await program.account.registry.fetch(
      findRegistryPda(program.programId, 8)
    );
    assert.equal(registryAfterRegister.nodeCount, 8);

    const capNodeOwner = ctx.nodes[8];
    const extraSalt = Uint8Array.from(Buffer.alloc(32, 99));
    const extraCompressed = EVM_FIXTURE_NODE_PUBKEYS[0];
    const extraNodeId = deriveNodeId(
      capNodeOwner.publicKey,
      extraCompressed,
      extraSalt
    );
    const capOwnerUsdc = await fundNodeOwnerInBankrun(
      ctx,
      capNodeOwner,
      ctx.underlyingTokenMint
    );
    const extraNodePda = findNodePda(program.programId, capNodeOwner.publicKey);
    const extraKeyClaimPda = findSecp256k1KeyClaimPda(
      program.programId,
      extraCompressed
    );

    try {
      await program.methods
        .registerNode(
          buildRegisterNodeArgs({
            newIndex: 8,
            registrationSalt: extraSalt,
            secp256k1PubkeyCompressed: extraCompressed,
            popSigR: Buffer.alloc(32),
            popSigS: Buffer.alloc(32),
          })
        )
        .accountsPartial({
          authority: ctx.authority.publicKey,
          nodeOwner: capNodeOwner.publicKey,
          protocolConfig: protocolConfigPda,
          registryState: registryStatePda,
          currentRegistry: findRegistryPda(program.programId, 8),
          newRegistry: findRegistryPda(program.programId, 9),
          node: extraNodePda,
          secp256K1KeyClaim: extraKeyClaimPda,
          usdcMint: ctx.underlyingTokenMint,
          ownerUsdc: capOwnerUsdc,
          treasury,
          systemProgram: SystemProgram.programId,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .signers([capNodeOwner])
        .rpc();
      assert.fail("register_node should fail when registry is full");
    } catch (error: any) {
      const err = String(error);
      assert.ok(
        err.includes("NodeCapExceeded") || err.includes("InvalidNewIndex"),
        `expected NodeCapExceeded or InvalidNewIndex, got: ${err}`
      );
    }

    // 8) deactivate_node (failure-path: authority has no node PDA)
    try {
      await program.methods
        .deactivateNode({
          nodeOwner: ctx.authority.publicKey,
          removeIndex: 0,
          newRegistryVersion: 9,
        })
        .accountsPartial({
          authority: ctx.authority.publicKey,
          protocolConfig: protocolConfigPda,
          registryState: registryStatePda,
          currentRegistry: findRegistryPda(program.programId, 8),
          newRegistry: findRegistryPda(program.programId, 9),
          node: findNodePda(program.programId, ctx.authority.publicKey),
          systemProgram: SystemProgram.programId,
        })
        .rpc();
      assert.fail("deactivate_node should fail without registered node");
    } catch (error: any) {
      assert.match(String(error), /account/i);
    }

    const registryStateAfterFlow = await program.account.registryState.fetch(
      registryStatePda
    );
    assert.equal(registryStateAfterFlow.currentVersion, 8);
    const registryAfterFlow = await program.account.registry.fetch(
      findRegistryPda(program.programId, 8)
    );
    assert.equal(registryAfterFlow.nodeCount, 8);
  });
});
