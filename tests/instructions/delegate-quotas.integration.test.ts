import { BN } from "@coral-xyz/anchor";
import { assert } from "chai";
import { Keypair, PublicKey, SystemProgram } from "@solana/web3.js";
import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { keccak_256 } from "@noble/hashes/sha3";
import { setupTestContext, TestContext } from "../setup";
import { getProtocolTreasury } from "../helpers/evm-submit-data-update";

const PROTOCOL_CONFIG_SEED = Buffer.from("molpha_config");
const REGISTRY_STATE_SEED = Buffer.from("molpha_registry");
const PLAN_SEED = Buffer.from("molpha_plan");
const SUBSCRIPTION_SEED = Buffer.from("molpha_subscription");
const JOB_SEED = Buffer.from("molpha_job");
const FEED_SEED = Buffer.from("molpha_feed");
const DELEGATE_SEED = Buffer.from("molpha_delegate");
const JOB_ID_PREFIX = Buffer.from("MOLPHA_JOB_V1");

function deriveJobId(owner: PublicKey, apiConfigHash: Uint8Array): Uint8Array {
  return keccak_256(
    Buffer.concat([JOB_ID_PREFIX, owner.toBuffer(), Buffer.from(apiConfigHash)])
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

async function bootstrapOwnerSubscription(ctx: TestContext, program: any) {
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

  await program.methods
    .addPlan({
      planType: { standard: {} },
      subscriptionPrice: new BN(2_000_000),
      maxJobs: 10,
      signaturesRequired: 1,
      maxSigners: 4,
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

  return { protocolConfigPda, planPda, subscriptionPda };
}

describe("delegate quotas", () => {
  let ctx: TestContext;
  let program: any;
  let protocolConfigPda: PublicKey;
  let planPda: PublicKey;
  let subscriptionPda: PublicKey;
  let owner: PublicKey;

  beforeAll(async () => {
    ctx = await setupTestContext();
    program = ctx.molphaProgram;
    owner = ctx.authority.publicKey;
    ({ protocolConfigPda, planPda, subscriptionPda } =
      await bootstrapOwnerSubscription(ctx, program));
  });

  it("allows a delegate to create jobs until max_jobs_created is reached", async () => {
    const delegate = Keypair.generate();

    ctx.provider.context.setAccount(delegate.publicKey, {
      lamports: 10_000_000_000,
      owner: SystemProgram.programId,
      executable: false,
      data: Buffer.alloc(0),
    });

    const delegatePda = findDelegatePda(
      program.programId,
      owner,
      delegate.publicKey
    );

    await program.methods
      .addDelegate({
        delegate: delegate.publicKey,
        maxJobsCreated: 1,
        maxDataRequests: new BN(5),
      })
      .accountsPartial({
        owner,
        delegateAccount: delegatePda,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    const apiConfigHash = Uint8Array.from(Buffer.alloc(32, 9));
    const jobId = deriveJobId(owner, apiConfigHash);
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
        creator: delegate.publicKey,
        jobOwner: owner,
        protocolConfig: protocolConfigPda,
        plan: planPda,
        job: jobPda,
        subscription: subscriptionPda,
        delegate: delegatePda,
        feed: feedPda,
        systemProgram: SystemProgram.programId,
      })
      .signers([delegate])
      .rpc();

    const delegateAccount = await program.account.delegate.fetch(delegatePda);
    assert.equal(delegateAccount.jobsCreated, 1);

    const secondConfigHash = Uint8Array.from(Buffer.alloc(32, 10));
    const secondJobId = deriveJobId(owner, secondConfigHash);
    const [secondJobPda] = PublicKey.findProgramAddressSync(
      [JOB_SEED, Buffer.from(secondJobId)],
      program.programId
    );
    const [secondFeedPda] = PublicKey.findProgramAddressSync(
      [FEED_SEED, Buffer.from(secondJobId)],
      program.programId
    );

    try {
      await program.methods
        .createJob(
          {
            apiConfigHash: Array.from(secondConfigHash),
            signaturesRequired: 1,
            decimals: 8,
          },
          Array.from(secondJobId)
        )
        .accountsPartial({
          creator: delegate.publicKey,
          jobOwner: owner,
          protocolConfig: protocolConfigPda,
          plan: planPda,
          job: secondJobPda,
          subscription: subscriptionPda,
          delegate: delegatePda,
          feed: secondFeedPda,
          systemProgram: SystemProgram.programId,
        })
        .signers([delegate])
        .rpc();
      assert.fail("expected delegate job limit error");
    } catch (error: any) {
      assert.include(String(error), "DelegateJobLimitReached");
    }
  });

  it("requires delegate account when a non-owner creates a job", async () => {
    const delegate = Keypair.generate();

    ctx.provider.context.setAccount(delegate.publicKey, {
      lamports: 10_000_000_000,
      owner: SystemProgram.programId,
      executable: false,
      data: Buffer.alloc(0),
    });

    const apiConfigHash = Uint8Array.from(Buffer.alloc(32, 11));
    const jobId = deriveJobId(owner, apiConfigHash);
    const [jobPda] = PublicKey.findProgramAddressSync(
      [JOB_SEED, Buffer.from(jobId)],
      program.programId
    );
    const [feedPda] = PublicKey.findProgramAddressSync(
      [FEED_SEED, Buffer.from(jobId)],
      program.programId
    );

    try {
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
          creator: delegate.publicKey,
          jobOwner: owner,
          protocolConfig: protocolConfigPda,
          plan: planPda,
          job: jobPda,
          subscription: subscriptionPda,
          delegate: null,
          feed: feedPda,
          systemProgram: SystemProgram.programId,
        })
        .signers([delegate])
        .rpc();
      assert.fail("expected delegate account required error");
    } catch (error: any) {
      assert.include(String(error), "DelegateAccountRequired");
    }
  });
});
