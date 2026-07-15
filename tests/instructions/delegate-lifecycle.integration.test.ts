import { BN } from "@coral-xyz/anchor";
import { assert } from "chai";
import { Keypair, PublicKey, SystemProgram } from "@solana/web3.js";
import { TOKEN_PROGRAM_ID } from "@solana/spl-token";
import { setupTestContext, TestContext } from "../setup";
import {
  AgentProtocolContext,
  initializeProtocolForAgents,
} from "../helpers/agent-access";

const PLAN_SEED = Buffer.from("molpha_plan");
const SUBSCRIPTION_SEED = Buffer.from("molpha_subscription");
const DELEGATE_SEED = Buffer.from("molpha_delegate");

const PLAN_ID = 1; // PlanType::Standard
const PLAN_MAX_SIGNERS = 4;
const PLAN_MAX_DELEGATES = 2;

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

/**
 * Delegate lifecycle: subscription-gated provisioning, plan-clamped `max_signers`,
 * the `plan.max_delegates` cap, owner pause / protocol freeze, and removal.
 * Owner-provisioned agents are these same `Delegate` accounts — there is no
 * separate escrow apparatus for them.
 */
describe("delegate lifecycle", () => {
  let ctx: TestContext;
  let proto: AgentProtocolContext;
  let planPda: PublicKey;
  let subscriptionPda: PublicKey;

  const addDelegate = (
    delegate: PublicKey,
    over: Partial<{
      maxJobsCreated: number;
      maxDataRequests: number;
      maxSigners: number;
    }> = {}
  ) =>
    proto.program.methods
      .addDelegate({
        delegate,
        maxJobsCreated: over.maxJobsCreated ?? 2,
        maxDataRequests: new BN(over.maxDataRequests ?? 10),
        maxSigners: over.maxSigners ?? 3,
      })
      .accountsPartial({
        owner: ctx.authority.publicKey,
        subscription: subscriptionPda,
        plan: planPda,
        delegateAccount: findDelegatePda(
          proto.program.programId,
          ctx.authority.publicKey,
          delegate
        ),
        systemProgram: SystemProgram.programId,
      })
      .rpc();

  beforeAll(async () => {
    ctx = await setupTestContext();
    proto = await initializeProtocolForAgents(ctx);

    [planPda] = PublicKey.findProgramAddressSync(
      [PLAN_SEED, Buffer.from([PLAN_ID])],
      proto.program.programId
    );
    [subscriptionPda] = PublicKey.findProgramAddressSync(
      [SUBSCRIPTION_SEED, ctx.authority.publicKey.toBuffer()],
      proto.program.programId
    );

    await proto.program.methods
      .addPlan({
        planType: { standard: {} },
        subscriptionPrice: new BN(2_000_000),
        maxJobs: 3,
        maxSigners: PLAN_MAX_SIGNERS,
        maxDelegates: PLAN_MAX_DELEGATES,
        privateApiEnabled: false,
        isActive: true,
      })
      .accountsPartial({
        protocolConfig: proto.protocolConfigPda,
        plan: planPda,
        authority: ctx.authority.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    await proto.program.methods
      .subscribe({ standard: {} })
      .accountsPartial({
        owner: ctx.authority.publicKey,
        protocolConfig: proto.protocolConfigPda,
        plan: planPda,
        subscription: subscriptionPda,
        usdcMint: ctx.underlyingTokenMint,
        ownerUsdc: ctx.userTokenAccount,
        treasury: proto.treasury,
        systemProgram: SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .rpc();
  }, 180_000);

  const delegateKey = Keypair.generate().publicKey;
  const delegatePda = () =>
    findDelegatePda(
      proto.program.programId,
      ctx.authority.publicKey,
      delegateKey
    );

  it("adds a delegate against an active subscription", async () => {
    await addDelegate(delegateKey);

    const delegate = await proto.program.account.delegate.fetch(delegatePda());
    assert.equal(delegate.owner.toBase58(), ctx.authority.publicKey.toBase58());
    assert.equal(delegate.delegate.toBase58(), delegateKey.toBase58());
    assert.deepEqual(delegate.status, { active: {} });
    assert.equal(delegate.maxJobsCreated, 2);
    assert.equal(delegate.maxDataRequests.toString(), "10");
    assert.equal(delegate.maxSigners, 3);
    // Dormant credit-economy fields start zeroed.
    assert.equal(delegate.epochCap.toNumber(), 0);
    assert.equal(delegate.epochSpent.toNumber(), 0);
    assert.equal(delegate.epochId.toNumber(), 0);

    const subscription = await proto.program.account.subscription.fetch(
      subscriptionPda
    );
    assert.equal(subscription.delegateCount, 1);
  });

  it("clamps max_signers to the plan ceiling and rejects zero", async () => {
    const other = Keypair.generate().publicKey;
    try {
      await addDelegate(other, { maxSigners: 0 });
      assert.fail("expected SignaturesExceedSubscriptionMax");
    } catch (e: any) {
      assert.include(e.toString(), "SignaturesExceedSubscriptionMax");
    }
    try {
      await addDelegate(other, { maxSigners: PLAN_MAX_SIGNERS + 1 });
      assert.fail("expected SignaturesExceedSubscriptionMax");
    } catch (e: any) {
      assert.include(e.toString(), "SignaturesExceedSubscriptionMax");
    }
  });

  it("enforces plan.max_delegates via subscription.delegate_count", async () => {
    const second = Keypair.generate().publicKey;
    await addDelegate(second);
    let subscription = await proto.program.account.subscription.fetch(
      subscriptionPda
    );
    assert.equal(subscription.delegateCount, PLAN_MAX_DELEGATES);

    try {
      await addDelegate(Keypair.generate().publicKey);
      assert.fail("expected DelegateCapReached");
    } catch (e: any) {
      assert.include(e.toString(), "DelegateCapReached");
    }

    // Removing one frees a slot again.
    await proto.program.methods
      .removeDelegate({ delegate: second })
      .accountsPartial({
        owner: ctx.authority.publicKey,
        subscription: subscriptionPda,
        delegateAccount: findDelegatePda(
          proto.program.programId,
          ctx.authority.publicKey,
          second
        ),
      })
      .rpc();
    subscription = await proto.program.account.subscription.fetch(
      subscriptionPda
    );
    assert.equal(subscription.delegateCount, 1);
  });

  it("removes a delegate: closes the PDA and decrements the counter", async () => {
    const before = await proto.program.account.subscription.fetch(
      subscriptionPda
    );
    await proto.program.methods
      .removeDelegate({ delegate: delegateKey })
      .accountsPartial({
        owner: ctx.authority.publicKey,
        subscription: subscriptionPda,
        delegateAccount: delegatePda(),
      })
      .rpc();

    const after = await proto.program.account.subscription.fetch(
      subscriptionPda
    );
    assert.equal(after.delegateCount, before.delegateCount - 1);
    const closed = await ctx.provider.context.banksClient.getAccount(
      delegatePda()
    );
    assert.isTrue(closed === null || closed.data.length === 0);
  });

  it("rejects provisioning without a subscription", async () => {
    const stranger = Keypair.generate();
    ctx.provider.context.setAccount(stranger.publicKey, {
      lamports: 1_000_000_000,
      owner: SystemProgram.programId,
      executable: false,
      data: Buffer.alloc(0),
    });
    const [strangerSubscription] = PublicKey.findProgramAddressSync(
      [SUBSCRIPTION_SEED, stranger.publicKey.toBuffer()],
      proto.program.programId
    );
    const delegate = Keypair.generate().publicKey;
    try {
      await proto.program.methods
        .addDelegate({
          delegate,
          maxJobsCreated: 1,
          maxDataRequests: new BN(1),
          maxSigners: 1,
        })
        .accountsPartial({
          owner: stranger.publicKey,
          subscription: strangerSubscription,
          plan: planPda,
          delegateAccount: findDelegatePda(
            proto.program.programId,
            stranger.publicKey,
            delegate
          ),
          systemProgram: SystemProgram.programId,
        })
        .signers([stranger])
        .rpc();
      assert.fail("expected missing subscription account failure");
    } catch (e: any) {
      assert.include(e.toString(), "AccountNotInitialized");
    }
  });
});
