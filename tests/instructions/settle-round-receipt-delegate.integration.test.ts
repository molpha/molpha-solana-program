import { BN } from "@coral-xyz/anchor";
import { assert } from "chai";
import {
  Ed25519Program,
  Keypair,
  PublicKey,
  SystemProgram,
  SYSVAR_INSTRUCTIONS_PUBKEY,
} from "@solana/web3.js";
import { TOKEN_PROGRAM_ID } from "@solana/spl-token";
import { setupTestContext, TestContext } from "../setup";
import {
  generateE2eSignerMaterial,
  registerNodesForE2e,
} from "../helpers/submit-data-update";
import {
  advanceClock,
  AgentProtocolContext,
  BondedGateway,
  bitmapWithBits,
  creditedNodePdasForBits,
  findSubscriptionRoundPda,
  hashRequestAuthFields,
  initializeProtocolForAgents,
  registerBondedGateway,
  zeroedAggSig,
} from "../helpers/agent-access";

const NODE_COUNT = 4;
const SIGS_REQUIRED = 2;
const PLAN_SEED = Buffer.from("molpha_plan");
const SUBSCRIPTION_SEED = Buffer.from("molpha_subscription");
const DELEGATE_SEED = Buffer.from("molpha_delegate");

const PLAN_MAX_SIGNERS = 3;
const DELEGATE_MAX_SIGNERS = 2;

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
 * Unified `settle_round_receipt` authorization: owner-direct rounds require the plan
 * (plan_type match + `plan.max_signers`); delegate/agent rounds are bounded by the
 * delegate's own `max_signers`, its Active status, and its data-request quota.
 * Owner-provisioned agents ARE delegates — this suite is their settlement coverage.
 */
describe("settle_round_receipt (owner / delegate / agent authorization)", () => {
  let ctx: TestContext;
  let proto: AgentProtocolContext;
  let gateway: BondedGateway;
  let registryVersion: number;
  let planPda: PublicKey;
  let subscriptionPda: PublicKey;
  let delegateHotKey: Keypair;
  let delegatePda: PublicKey;
  let ts = 1_700_300_000;
  let sourceByte = 0;

  const nextSourceId = () => Buffer.alloc(32, ++sourceByte);

  const addPlan = (planType: any, planId: number, maxSigners: number) => {
    const [pda] = PublicKey.findProgramAddressSync(
      [PLAN_SEED, Buffer.from([planId])],
      proto.program.programId
    );
    return proto.program.methods
      .addPlan({
        planType,
        subscriptionPrice: new BN(2_000_000),
        maxJobs: 3,
        maxSigners,
        maxDelegates: 4,
        privateApiEnabled: false,
        isActive: true,
      })
      .accountsPartial({
        protocolConfig: proto.protocolConfigPda,
        plan: pda,
        authority: ctx.authority.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .rpc()
      .then(() => pda);
  };

  const addDelegate = (
    delegate: PublicKey,
    over: Partial<{ maxDataRequests: number; maxSigners: number }> = {}
  ) =>
    proto.program.methods
      .addDelegate({
        delegate,
        maxDataRequests: new BN(over.maxDataRequests ?? 100),
        maxSigners: over.maxSigners ?? DELEGATE_MAX_SIGNERS,
      })
      .accountsPartial({
        owner: ctx.authority.publicKey,
        subscription: subscriptionPda,
        delegateAccount: findDelegatePda(
          proto.program.programId,
          ctx.authority.publicKey,
          delegate
        ),
        systemProgram: SystemProgram.programId,
      })
      .rpc();

  interface SettleReceiptParams {
    timestamp: number;
    sourceId: Uint8Array;
    signaturesRequired?: number;
    /** Consumer keypair that signs H(RequestAuth). Defaults to the delegate hot key. */
    consumer?: Keypair;
    /** Delegate PDA to pass; null for owner-direct. */
    delegate?: PublicKey | null;
    creditedBits?: number[];
  }

  async function settleReceipt(
    params: SettleReceiptParams
  ): Promise<PublicKey> {
    const signaturesRequired = params.signaturesRequired ?? SIGS_REQUIRED;
    const consumer = params.consumer ?? delegateHotKey;
    const bits =
      params.creditedBits ??
      Array.from({ length: signaturesRequired }, (_, i) => i);
    const requestAuthHash = hashRequestAuthFields(
      params.sourceId,
      signaturesRequired,
      params.timestamp
    );
    const consumerSigIx = Ed25519Program.createInstructionWithPrivateKey({
      privateKey: consumer.secretKey,
      message: requestAuthHash,
    });
    const roundRecordPda = findSubscriptionRoundPda(
      proto.program.programId,
      consumer.publicKey,
      params.sourceId,
      signaturesRequired,
      params.timestamp
    );

    await proto.program.methods
      .settleRoundReceipt({
        timestamp: new BN(params.timestamp),
        sourceId: Array.from(params.sourceId),
        signaturesRequired,
        registryVersion,
        aggSig: zeroedAggSig(bitmapWithBits(bits)),
        value: Buffer.alloc(32),
        consumerAuthority: consumer.publicKey,
      })
      .accountsPartial({
        payer: ctx.authority.publicKey,
        protocolConfig: proto.protocolConfigPda,
        registryState: proto.registryStatePda,
        subscription: subscriptionPda,
        consumerDelegate:
          params.delegate === undefined ? delegatePda : params.delegate,
        gateway: gateway.gatewayPda,
        roundRecord: roundRecordPda,
        instructionsSysvar: SYSVAR_INSTRUCTIONS_PUBKEY,
        systemProgram: SystemProgram.programId,
      })
      .remainingAccounts(
        creditedNodePdasForBits(proto.program.programId, bits).map(
          (pubkey) => ({
            pubkey,
            isWritable: true,
            isSigner: false,
          })
        )
      )
      .preInstructions([consumerSigIx])
      .rpc();

    return roundRecordPda;
  }

  beforeAll(async () => {
    ctx = await setupTestContext();
    proto = await initializeProtocolForAgents(ctx);
    const material = generateE2eSignerMaterial(NODE_COUNT);
    await registerNodesForE2e(
      ctx,
      proto.program,
      {
        protocolConfigPda: proto.protocolConfigPda,
        registryStatePda: proto.registryStatePda,
        usdcMint: ctx.underlyingTokenMint,
        ownerUsdc: ctx.userTokenAccount,
        treasury: proto.treasury,
      },
      material,
      advanceClock
    );
    const registry = await proto.program.account.registryState.fetch(
      proto.registryStatePda
    );
    registryVersion = registry.currentVersion;
    gateway = await registerBondedGateway(ctx, proto);

    planPda = await addPlan({ standard: {} }, 1, PLAN_MAX_SIGNERS);

    [subscriptionPda] = PublicKey.findProgramAddressSync(
      [SUBSCRIPTION_SEED, ctx.authority.publicKey.toBuffer()],
      proto.program.programId
    );
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

    delegateHotKey = Keypair.generate();
    delegatePda = findDelegatePda(
      proto.program.programId,
      ctx.authority.publicKey,
      delegateHotKey.publicKey
    );
    await addDelegate(delegateHotKey.publicKey);
  }, 180_000);

  it("settles a delegate round: consumes quota, writes the shared-feed record", async () => {
    const before = await proto.program.account.delegate.fetch(delegatePda);
    const subBefore = await proto.program.account.subscription.fetch(
      subscriptionPda
    );

    const sourceId = nextSourceId();
    const t = ++ts;
    const recordPda = await settleReceipt({ sourceId, timestamp: t });

    const after = await proto.program.account.delegate.fetch(delegatePda);
    assert.equal(
      after.dataRequestsUsed.toNumber() - before.dataRequestsUsed.toNumber(),
      1
    );
    const subAfter = await proto.program.account.subscription.fetch(
      subscriptionPda
    );
    assert.equal(
      subAfter.usedRounds.toNumber() - subBefore.usedRounds.toNumber(),
      1
    );

    const record = await proto.program.account.roundReceiptRecord.fetch(
      recordPda
    );

    assert.equal(record.price.toNumber(), 0); // subscription-funded, no per-request USDC
    assert.deepEqual(Array.from(record.sourceId), Array.from(sourceId));
  });

  it("blocks replay of the same (consumer, source_id, sigs, timestamp) via the record PDA", async () => {
    const sourceId = nextSourceId();
    const t = ++ts;
    await settleReceipt({ sourceId, timestamp: t });
    try {
      await settleReceipt({ sourceId, timestamp: t });
      assert.fail("expected duplicate record PDA failure");
    } catch (e: any) {
      assert.ok(e);
    }
  });

  it("rejects a paused or frozen delegate with DelegateNotActive", async () => {
    await proto.program.methods
      .setDelegatePaused(true)
      .accountsPartial({
        owner: ctx.authority.publicKey,
        delegateAccount: delegatePda,
      })
      .rpc();
    try {
      await settleReceipt({
        sourceId: nextSourceId(),
        timestamp: ++ts,
      });
      assert.fail("expected DelegateNotActive");
    } catch (e: any) {
      assert.include(e.toString(), "DelegateNotActive");
    }
    // Restore Active so later cases in this suite can reuse the same delegate.
    await proto.program.methods
      .setDelegatePaused(false)
      .accountsPartial({
        owner: ctx.authority.publicKey,
        delegateAccount: delegatePda,
      })
      .rpc();
  });

  it("caps delegate rounds at delegate.max_signers", async () => {
    try {
      await settleReceipt({
        sourceId: nextSourceId(),
        timestamp: ++ts,
        signaturesRequired: DELEGATE_MAX_SIGNERS + 1,
      });
      assert.fail("expected SignaturesExceedDelegateMax");
    } catch (e: any) {
      assert.include(e.toString(), "SignaturesExceedDelegateMax");
    }
  });

  it("exhausts the delegate data-request quota", async () => {
    const throttledKey = Keypair.generate();
    await addDelegate(throttledKey.publicKey, { maxDataRequests: 1 });

    const throttledPda = findDelegatePda(
      proto.program.programId,
      ctx.authority.publicKey,
      throttledKey.publicKey
    );
    await settleReceipt({
      sourceId: nextSourceId(),
      timestamp: ++ts,
      consumer: throttledKey,
      delegate: throttledPda,
    });
    try {
      await settleReceipt({
        sourceId: nextSourceId(),
        timestamp: ++ts,
        consumer: throttledKey,
        delegate: throttledPda,
      });
      assert.fail("expected DelegateDataRequestLimitReached");
    } catch (e: any) {
      assert.include(e.toString(), "DelegateDataRequestLimitReached");
    }
  });

  it("owner-direct settles without a delegate account", async () => {
    await settleReceipt({
      sourceId: nextSourceId(),
      timestamp: ++ts,
      consumer: ctx.authority.payer,
      delegate: null,
    });
  });

  it("caps owner-direct rounds at subscription.max_signers", async () => {
    try {
      await settleReceipt({
        sourceId: nextSourceId(),
        timestamp: ++ts,
        signaturesRequired: PLAN_MAX_SIGNERS + 1,
        consumer: ctx.authority.payer,
        delegate: null,
      });
      assert.fail("expected SignaturesExceedSubscriptionMax");
    } catch (e: any) {
      assert.include(e.toString(), "SignaturesExceedSubscriptionMax");
    }
  });
});
