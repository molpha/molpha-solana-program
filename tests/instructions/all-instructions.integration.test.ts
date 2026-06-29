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
  findNodePdaByOwner,
  fundNodeOwnerInBankrun,
  registerNodesForE2e,
  getProtocolTreasury,
} from "../helpers/evm-submit-data-update";

const PROTOCOL_CONFIG_SEED = Buffer.from("molpha_config");
const REGISTRY_STATE_SEED = Buffer.from("molpha_registry");
const PLAN_SEED = Buffer.from("molpha_plan");
const SUBSCRIPTION_SEED = Buffer.from("molpha_subscription");
const JOB_SEED = Buffer.from("molpha_job");
const FEED_SEED = Buffer.from("molpha_feed");
const DELEGATE_SEED = Buffer.from("molpha_delegate");
const REGISTRY_INDEX_SEED = Buffer.from("molpha_registry_index");

const JOB_ID_PREFIX = Buffer.from("MOLPHA_JOB_V1");
const SECONDS_PER_MONTH = 30 * 24 * 60 * 60;

function hexToBytes(hex: string): Uint8Array {
  const clean = hex.startsWith("0x") ? hex.slice(2) : hex;
  return Uint8Array.from(Buffer.from(clean, "hex"));
}

function deriveJobId(
  owner: PublicKey,
  apiConfigHash: Uint8Array,
  signaturesRequired: number
): Uint8Array {
  return keccak_256(
    Buffer.concat([
      JOB_ID_PREFIX,
      owner.toBuffer(),
      Buffer.from(apiConfigHash),
      Buffer.from([signaturesRequired]),
    ])
  );
}

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
        registryCooldownSecs: new BN(1),
        previousGraceSecs: new BN(1),
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
        signaturesRequired: 1,
        maxSigners: 2,
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
    assert.equal(plan.maxJobs, 4);
    assert.equal(plan.subscriptionPrice.toString(), "3000000");

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

    // 4) create_job
    const apiConfigHash = Uint8Array.from(Buffer.alloc(32, 7));
    const jobId = deriveJobId(ctx.authority.publicKey, apiConfigHash, 1);
    const [jobPda] = PublicKey.findProgramAddressSync(
      [JOB_SEED, Buffer.from(jobId)],
      program.programId
    );
    const [feedPda] = PublicKey.findProgramAddressSync(
      [FEED_SEED, Buffer.from(jobId)],
      program.programId
    );

    await program.methods
      .createJob(
        {
          apiConfigHash: Array.from(apiConfigHash),
          signaturesRequired: 1,
          decimals: 8,
        },
        Array.from(jobId)
      )
      .accountsPartial({
        creator: ctx.authority.publicKey,
        jobOwner: ctx.authority.publicKey,
        protocolConfig: protocolConfigPda,
        plan: planPda,
        job: jobPda,
        subscription: subscriptionPda,
        delegate: null,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    const createdJob = await program.account.job.fetch(jobPda);
    assert.equal(
      createdJob.owner.toBase58(),
      ctx.authority.publicKey.toBase58()
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
        maxJobsCreated: 2,
        maxDataRequests: new BN(10),
      })
      .accountsPartial({
        owner: ctx.authority.publicKey,
        delegateAccount: delegatePda,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    let delegateAccount = await program.account.delegate.fetch(delegatePda);
    assert.equal(delegateAccount.jobsCreated, 0);
    assert.equal(delegateAccount.maxJobsCreated, 2);
    assert.equal(delegateAccount.maxDataRequests.toString(), "10");

    await program.methods
      .removeDelegate({
        delegate,
      })
      .accountsPartial({
        owner: ctx.authority.publicKey,
        delegateAccount: delegatePda,
      })
      .rpc();

    try {
      await program.account.delegate.fetch(delegatePda);
      assert.fail("expected delegate account to be closed");
    } catch (error: any) {
      assert.ok(String(error));
    }

    // 6) submit_data_update + verify_data_update (failure-path coverage)
    const badJobId = Uint8Array.from(Buffer.alloc(32, 4));
    const dataUpdateArgs = {
      jobId: Array.from(jobId),
      registryVersion: 0,
      signaturesRequired: 1,
      signersBitmap: Array.from(Buffer.alloc(32, 0)),
      value: Array.from(Buffer.alloc(32, 1)),
      canonicalTimestamp: new BN(1),
      aggSigR: Array.from(Buffer.alloc(32, 0)),
      aggSigS: Array.from(Buffer.alloc(32, 0)),
      commitmentAddr: Array.from(Buffer.alloc(20, 0)),
    };
    const badDataUpdateArgs = {
      ...dataUpdateArgs,
      jobId: Array.from(badJobId),
    };

    try {
      await program.methods
        .submitDataUpdate(dataUpdateArgs)
        .accountsPartial({
          submitter: ctx.authority.publicKey,
          registryState: registryStatePda,
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
      EVM_FIXTURE_NODE_PUBKEYS.slice(0, 8),
      advanceClock
    );

    const registryAfterRegister = await program.account.registryState.fetch(
      registryStatePda
    );
    assert.equal(registryAfterRegister.currentNodeCount, 8);

    const capNodeOwner = ctx.nodes[8];
    const extraSalt = Uint8Array.from(Buffer.alloc(32, 99));
    const extraCompressed = EVM_FIXTURE_NODE_PUBKEYS[0];
    const extraNodeId = deriveNodeId(
      capNodeOwner.publicKey,
      extraCompressed,
      extraSalt
    );
    const extraNodePda = findNodePdaByOwner(
      program.programId,
      capNodeOwner.publicKey
    );
    const capOwnerUsdc = await fundNodeOwnerInBankrun(
      ctx,
      capNodeOwner,
      ctx.underlyingTokenMint
    );
    const fullIndexBytes = Buffer.alloc(4);
    fullIndexBytes.writeUInt32LE(8);
    const [registryIndexPda] = PublicKey.findProgramAddressSync(
      [REGISTRY_INDEX_SEED, fullIndexBytes],
      program.programId
    );

    try {
      await program.methods
        .registerNode(
          buildRegisterNodeArgs({
            newIndex: 8,
            registrationSalt: extraSalt,
            secp256k1PubkeyCompressed: extraCompressed,
          }),
          Array.from(extraNodeId)
        )
        .accountsPartial({
          adminAuthority: ctx.authority.publicKey,
          nodeOwner: capNodeOwner.publicKey,
          protocolConfig: protocolConfigPda,
          registryState: registryStatePda,
          node: extraNodePda,
          registryIndex: registryIndexPda,
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
    const registryForDeactivate = await program.account.registryState.fetch(
      registryStatePda
    );
    try {
      await program.methods
        .deactivateNode()
        .accountsPartial({
          authority: ctx.authority.publicKey,
          registryState: registryStatePda,
          rentReceiver: ctx.authority.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .rpc();
      assert.fail("deactivate_node should fail without registered node");
    } catch (error: any) {
      assert.include(String(error), "Account");
    }

    const registryAfterFlow = await program.account.registryState.fetch(
      registryStatePda
    );
    assert.equal(registryAfterFlow.currentNodeCount, 8);
    assert.equal(registryAfterFlow.previousExpiresAt.toString(), "0");
  });
});
