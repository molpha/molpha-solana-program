import { BN } from "@coral-xyz/anchor";
import { assert } from "chai";
import {
  ComputeBudgetProgram,
  Ed25519Program,
  Keypair,
  PublicKey,
  SystemProgram,
  SYSVAR_INSTRUCTIONS_PUBKEY,
} from "@solana/web3.js";
import {
  getAssociatedTokenAddressSync,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { setupTestContext, TestContext } from "../setup";
import {
  deriveSignersBitmap,
  evmBitmapBitSet,
  findRegistryPda,
  fundNodeOwnerInBankrun,
  generateE2eSignerMaterial,
  registerNodesForE2e,
  registeredNodePda,
  signE2eAttestation,
} from "../helpers/submit-attestation";
import {
  advanceClock,
  AgentProtocolContext,
  BondedGateway,
  bitmapWithBits,
  creditedNodePdasForBits,
  derivePayerId,
  findSubscriptionRoundPda,
  findX402RoundPda,
  hashRequestAuthFields,
  INIT,
  readTokenAmount,
  initializeProtocolForAgents,
  registerBondedGateway,
  settleX402Round,
  x402Price,
  zeroedAggSig,
} from "../helpers/settlement-access";

const NODE_COUNT = 4;
const SIGS_REQUIRED = 2;
const SOURCE_ID = Uint8Array.from(Buffer.alloc(32, 0x66));
const PLAN_SEED = Buffer.from("molpha_plan");
const SUBSCRIPTION_SEED = Buffer.from("molpha_subscription");
const DELEGATE_SEED = Buffer.from("molpha_delegate");

function bitsOf(bitmap: Uint8Array): number[] {
  const bits: number[] = [];
  for (let bit = 0; bit < 256; bit++) {
    if (evmBitmapBitSet(bitmap, bit)) bits.push(bit);
  }
  return bits;
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

/**
 * `finalize_round` and `dispute_round` are shared by both funding paths, discriminated by
 * `record.kind`. x402 rounds are never refunded on a successful dispute — the settled `price`
 * came from the gateway's own float, and the gateway is the party being slashed; refunding it
 * would return the fraud proceeds to the fraudster. Subscription rounds still restore
 * `used_rounds`. See `dispute_round`'s module doc for the full rationale.
 */
describe("rounds: finalize + dispute through the shared path", () => {
  let ctx: TestContext;
  let proto: AgentProtocolContext;
  let gateway: BondedGateway;
  let victimGateway: BondedGateway;
  let payerId: Uint8Array;
  let material: ReturnType<typeof generateE2eSignerMaterial>;
  let registryVersion: number;
  let registryPda: PublicKey;
  let redundancyBuffer = INIT.redundancyBuffer;
  let ts = 1_700_200_000;

  beforeAll(async () => {
    ctx = await setupTestContext();
    proto = await initializeProtocolForAgents(ctx);
    material = generateE2eSignerMaterial(NODE_COUNT);
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
    registryPda = findRegistryPda(proto.program.programId, registryVersion);
    gateway = await registerBondedGateway(ctx, proto);
    victimGateway = await registerBondedGateway(ctx, proto);
    payerId = derivePayerId(1, Keypair.generate().publicKey.toBuffer());
  }, 180_000);

  async function advanceRegistryPolicy(nextBuffer: number): Promise<void> {
    const nextVersion = registryVersion + 1;
    const nextRegistry = findRegistryPda(proto.program.programId, nextVersion);
    await proto.program.methods
      .updateRegistryPolicy(nextVersion, nextBuffer)
      .accountsPartial({
        authority: ctx.authority.publicKey,
        protocolConfig: proto.protocolConfigPda,
        registryState: proto.registryStatePda,
        currentRegistry: registryPda,
        newRegistry: nextRegistry,
        systemProgram: SystemProgram.programId,
      })
      .rpc();
    registryVersion = nextVersion;
    registryPda = nextRegistry;
    redundancyBuffer = nextBuffer;
  }

  async function disputeRound(params: {
    roundRecord: PublicKey;
    gateway: BondedGateway;
    challengerUsdc: PublicKey;
    challenger: any;
    nodeBits: number[];
    registry?: PublicKey;
    subscription?: PublicKey;
  }): Promise<void> {
    await proto.program.methods
      .disputeRound()
      .accountsPartial({
        challenger: params.challenger.publicKey,
        protocolConfig: proto.protocolConfigPda,
        registry: params.registry ?? registryPda,
        roundRecord: params.roundRecord,
        gateway: params.gateway.gatewayPda,
        gatewayBond: params.gateway.gatewayBondAta,
        treasury: proto.treasury,
        challengerUsdc: params.challengerUsdc,
        tokenProgram: TOKEN_PROGRAM_ID,
        subscription: params.subscription ?? null,
      })
      .remainingAccounts(
        params.nodeBits.map((bit) => ({
          pubkey: registeredNodePda(proto.program.programId, bit),
          isWritable: false,
          isSigner: false,
        }))
      )
      .preInstructions([
        ComputeBudgetProgram.setComputeUnitLimit({ units: 1_400_000 }),
      ])
      .signers(params.challenger.secretKey ? [params.challenger] : [])
      .rpc();
  }

  async function ensureSubscription(): Promise<PublicKey> {
    const planId = 1; // PlanType::Standard
    const [planPda] = PublicKey.findProgramAddressSync(
      [PLAN_SEED, Buffer.from([planId])],
      proto.program.programId
    );
    const [subscriptionPda] = PublicKey.findProgramAddressSync(
      [SUBSCRIPTION_SEED, ctx.authority.publicKey.toBuffer()],
      proto.program.programId
    );
    try {
      await proto.program.methods
        .addPlan({
          planType: { standard: {} },
          subscriptionPrice: new BN(2_000_000),
          maxJobs: 3,
          maxSigners: 4,
          maxDelegates: 2,
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
    } catch {
      // Plan may already exist from a prior test in this suite.
    }
    try {
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
    } catch {
      // Owner may already be subscribed.
    }
    return subscriptionPda;
  }

  it("finalizes an x402 round after the dispute window", async () => {
    const t = ++ts;
    const roundRegistry = registryPda;
    const recordPda = await settleX402Round({
      proto,
      gateway,
      payerId,
      amount: x402Price(SIGS_REQUIRED),
      sourceId: SOURCE_ID,
      canonicalTimestamp: t,
      signaturesRequired: SIGS_REQUIRED,
      registryVersion,
      aggSig: zeroedAggSig(bitmapWithBits([0, 1])),
      creditedNodePdas: creditedNodePdasForBits(
        proto.program.programId,
        [0, 1]
      ),
    });
    assert.equal(
      recordPda.toBase58(),
      findX402RoundPda(
        proto.program.programId,
        payerId,
        gateway.gatewayPda,
        SOURCE_ID,
        SIGS_REQUIRED,
        t
      ).toBase58()
    );

    const finalize = (computeUnitLimit?: number) =>
      proto.program.methods
        .finalizeRound()
        .accountsPartial({
          caller: ctx.authority.publicKey,
          protocolConfig: proto.protocolConfigPda,
          registry: roundRegistry,
          roundRecord: recordPda,
          gateway: gateway.gatewayPda,
        })
        .remainingAccounts(
          [0, 1].map((bit) => ({
            pubkey: registeredNodePda(proto.program.programId, bit),
            isWritable: true,
            isSigner: false,
          }))
        )
        .preInstructions(
          computeUnitLimit === undefined
            ? []
            : [
                ComputeBudgetProgram.setComputeUnitLimit({
                  units: computeUnitLimit,
                }),
              ]
        )
        .rpc();

    // Inside the window: not finalizable yet.
    try {
      await finalize();
      assert.fail("expected RoundNotFinalizable");
    } catch (e: any) {
      assert.include(e.toString(), "RoundNotFinalizable");
    }

    await advanceRegistryPolicy(1);

    await advanceClock(ctx, INIT.disputeWindowSlots + 10);

    const nodeBefore = await proto.program.account.node.fetch(
      registeredNodePda(proto.program.programId, 0)
    );
    const gwBefore = await proto.program.account.gateway.fetch(
      gateway.gatewayPda
    );
    // Bankrun does not rotate the blockhash when setClock advances time. A
    // harmless compute-budget instruction keeps this retry's signature unique.
    await finalize(200_001);

    const nodeAfter = await proto.program.account.node.fetch(
      registeredNodePda(proto.program.programId, 0)
    );
    const gwAfter = await proto.program.account.gateway.fetch(
      gateway.gatewayPda
    );
    assert.equal(
      nodeAfter.claimableRewards.toNumber() -
        nodeBefore.claimableRewards.toNumber(),
      INIT.rewardPerSignature
    );
    assert.equal(
      gwAfter.claimableRewards.toNumber() -
        gwBefore.claimableRewards.toNumber(),
      INIT.gatewayFee
    );

    const record = await proto.program.account.roundReceiptRecord.fetch(
      recordPda
    );
    assert.isOk(record.status.finalized);
  });

  let voidedX402RecordPda: PublicKey;
  let voidedX402RecordRegistryPda: PublicKey;

  it("voids a fabricated x402 round on dispute, slashes the gateway bond, and does not refund the float", async () => {
    const t = ++ts;
    const selection = deriveSignersBitmap(
      SOURCE_ID,
      registryVersion,
      NODE_COUNT,
      SIGS_REQUIRED,
      redundancyBuffer,
      t
    );
    const bits = bitsOf(selection);
    const fabricatedSig = zeroedAggSig(selection);

    const recordPda = await settleX402Round({
      proto,
      gateway: victimGateway,
      payerId,
      amount: x402Price(SIGS_REQUIRED),
      sourceId: SOURCE_ID,
      canonicalTimestamp: t,
      signaturesRequired: SIGS_REQUIRED,
      registryVersion,
      aggSig: fabricatedSig,
      creditedNodePdas: creditedNodePdasForBits(proto.program.programId, bits),
    });

    const challenger = ctx.nodes[30];
    const challengerUsdc = await fundNodeOwnerInBankrun(
      ctx,
      challenger,
      ctx.underlyingTokenMint,
      BigInt(0)
    );

    const expectedBounty = BigInt(
      (INIT.gatewayBondMin * INIT.challengerBountyBps) / 10_000
    );
    const expectedRemainder = BigInt(INIT.gatewayBondMin) - expectedBounty;
    // Measured after settle: the gateway already paid `price` into the treasury from its
    // own float. A successful dispute must not move either of these again.
    const floatBefore = await readTokenAmount(ctx, victimGateway.authorityUsdc);
    const treasuryBefore = await readTokenAmount(ctx, proto.treasury);
    const bondBefore = await readTokenAmount(
      ctx,
      victimGateway.gatewayBondAta
    );
    assert.equal(bondBefore, BigInt(INIT.gatewayBondMin));

    const roundRegistry = registryPda;
    await advanceRegistryPolicy(2);
    await disputeRound({
      roundRecord: recordPda,
      gateway: victimGateway,
      challenger,
      challengerUsdc,
      nodeBits: bits,
      registry: roundRegistry,
    });

    const voided = await proto.program.account.roundReceiptRecord.fetch(
      recordPda
    );
    assert.isOk(voided.status.voided);
    voidedX402RecordPda = recordPda;
    voidedX402RecordRegistryPda = roundRegistry;

    const gw = await proto.program.account.gateway.fetch(
      victimGateway.gatewayPda
    );
    assert.isTrue(gw.slashed);
    assert.equal(gw.bond.toNumber(), 0);

    const challengerAccount = await ctx.provider.context.banksClient.getAccount(
      challengerUsdc
    );
    const bounty = Buffer.from(challengerAccount!.data).readBigUInt64LE(64);
    assert.equal(bounty, expectedBounty);

    // No refund: the gateway's float is exactly as the settle call left it.
    const floatAfter = await readTokenAmount(ctx, victimGateway.authorityUsdc);
    assert.equal(floatAfter, floatBefore);
    const treasuryAfter = await readTokenAmount(ctx, proto.treasury);
    assert.equal(treasuryAfter - treasuryBefore, expectedRemainder);
    assert.equal(
      await readTokenAmount(ctx, victimGateway.gatewayBondAta),
      0n
    );
  });

  it("blocks finalizing a voided x402 round", async () => {
    try {
      await proto.program.methods
        .finalizeRound()
        .accountsPartial({
          caller: ctx.authority.publicKey,
          protocolConfig: proto.protocolConfigPda,
          registry: voidedX402RecordRegistryPda,
          roundRecord: voidedX402RecordPda,
          gateway: victimGateway.gatewayPda,
        })
        .remainingAccounts([])
        .rpc();
      assert.fail("expected RoundNotPending");
    } catch (e: any) {
      assert.include(e.toString(), "RoundNotPending");
    }
  });

  it("rejects a dispute that names a subscription account for an x402 record", async () => {
    const subscriptionPda = await ensureSubscription();
    const t = ++ts;
    const selection = deriveSignersBitmap(
      SOURCE_ID,
      registryVersion,
      NODE_COUNT,
      SIGS_REQUIRED,
      redundancyBuffer,
      t
    );
    const bits = bitsOf(selection);
    const fabricatedSig = zeroedAggSig(selection);

    // `victimGateway` is already slashed by the earlier dispute test, so this uses the
    // still-active `gateway` instead — this test errors before any state change anyway
    // (the subscription check runs before slashing), so `gateway` comes out untouched.
    const recordPda = await settleX402Round({
      proto,
      gateway,
      payerId,
      amount: x402Price(SIGS_REQUIRED),
      sourceId: SOURCE_ID,
      canonicalTimestamp: t,
      signaturesRequired: SIGS_REQUIRED,
      registryVersion,
      aggSig: fabricatedSig,
      creditedNodePdas: creditedNodePdasForBits(proto.program.programId, bits),
    });

    const challenger = ctx.nodes[28];
    const challengerUsdc = await fundNodeOwnerInBankrun(
      ctx,
      challenger,
      ctx.underlyingTokenMint,
      BigInt(0)
    );

    try {
      await disputeRound({
        roundRecord: recordPda,
        gateway,
        challenger,
        challengerUsdc,
        nodeBits: bits,
        subscription: subscriptionPda,
      });
      assert.fail("expected DisputeRefundAccountsMissing");
    } catch (e: any) {
      assert.include(e.toString(), "DisputeRefundAccountsMissing");
    }

    const gw = await proto.program.account.gateway.fetch(gateway.gatewayPda);
    assert.isFalse(gw.slashed);
  });

  it("fails to dispute a genuinely valid full-participation x402 round", async () => {
    const t = ++ts;
    const value = Uint8Array.from(Buffer.alloc(32, 0x99));
    const signed = signE2eAttestation(
      registryVersion,
      material,
      NODE_COUNT,
      SOURCE_ID,
      {
        value,
        canonicalTimestamp: t,
      },
      SIGS_REQUIRED,
      NODE_COUNT,
      redundancyBuffer
    );
    const bits = bitsOf(signed.signersBitmap);
    const validSig = {
      aggSigS: Array.from(signed.aggSigS),
      commitment: Array.from(signed.commitment),
      signersBitmap: Array.from(signed.signersBitmap),
    };

    const recordPda = await settleX402Round({
      proto,
      gateway,
      payerId,
      amount: x402Price(SIGS_REQUIRED),
      sourceId: SOURCE_ID,
      canonicalTimestamp: t,
      signaturesRequired: SIGS_REQUIRED,
      registryVersion,
      aggSig: validSig,
      value,
      creditedNodePdas: creditedNodePdasForBits(proto.program.programId, bits),
    });

    try {
      await disputeRound({
        roundRecord: recordPda,
        gateway,
        challenger: ctx.authority.payer,
        challengerUsdc: ctx.userTokenAccount,
        nodeBits: bits,
      });
      assert.fail("expected DisputeSignatureValid");
    } catch (e: any) {
      assert.include(e.toString(), "DisputeSignatureValid");
    }

    const gw = await proto.program.account.gateway.fetch(gateway.gatewayPda);
    assert.isFalse(gw.slashed);
    assert.equal(
      await readTokenAmount(ctx, gateway.gatewayBondAta),
      BigInt(INIT.gatewayBondMin)
    );
  });

  it("rejects settling a new x402 round through a slashed gateway", async () => {
    try {
      await settleX402Round({
        proto,
        gateway: victimGateway,
        payerId,
        amount: x402Price(SIGS_REQUIRED),
        sourceId: SOURCE_ID,
        canonicalTimestamp: ++ts,
        signaturesRequired: SIGS_REQUIRED,
        registryVersion,
        aggSig: zeroedAggSig(bitmapWithBits([0, 1])),
        creditedNodePdas: creditedNodePdasForBits(
          proto.program.programId,
          [0, 1]
        ),
      });
      assert.fail("expected GatewayNotBonded");
    } catch (e: any) {
      assert.include(e.toString(), "GatewayNotBonded");
    }
  });

  it("voids a fabricated subscription round and restores used_rounds", async () => {
    const subscriptionPda = await ensureSubscription();
    const subGateway = await registerBondedGateway(ctx, proto);
    const subBefore = await proto.program.account.subscription.fetch(
      subscriptionPda
    );

    const t = ++ts;
    const sourceId = Buffer.alloc(32, 0x88);
    const sigsRequired = 1;
    const selection = deriveSignersBitmap(
      sourceId,
      registryVersion,
      NODE_COUNT,
      sigsRequired,
      redundancyBuffer,
      t
    );
    const bits = bitsOf(selection);
    const fabricatedSig = zeroedAggSig(selection);

    const requestAuthHash = hashRequestAuthFields(sourceId, sigsRequired, t);
    const consumerSigIx = Ed25519Program.createInstructionWithPrivateKey({
      privateKey: ctx.authority.payer.secretKey,
      message: requestAuthHash,
    });
    const roundRecordPda = findSubscriptionRoundPda(
      proto.program.programId,
      ctx.authority.publicKey,
      sourceId,
      sigsRequired,
      t
    );

    await proto.program.methods
      .settleSubscriptionRound({
        timestamp: new BN(t),
        sourceId: Array.from(sourceId),
        signaturesRequired: sigsRequired,
        registryVersion,
        aggSig: fabricatedSig,
        value: Buffer.alloc(32),
        consumerAuthority: ctx.authority.publicKey,
      })
      .accountsPartial({
        payer: ctx.authority.publicKey,
        protocolConfig: proto.protocolConfigPda,
        registryState: proto.registryStatePda,
        registry: registryPda,
        subscription: subscriptionPda,
        consumerDelegate: null,
        gateway: subGateway.gatewayPda,
        roundRecord: roundRecordPda,
        instructionsSysvar: SYSVAR_INSTRUCTIONS_PUBKEY,
        systemProgram: SystemProgram.programId,
      })
      .remainingAccounts(
        bits.map((bit) => ({
          pubkey: registeredNodePda(proto.program.programId, bit),
          isWritable: true,
          isSigner: false,
        }))
      )
      .preInstructions([consumerSigIx])
      .rpc();

    const subAfterSettle = await proto.program.account.subscription.fetch(
      subscriptionPda
    );
    assert.equal(
      subAfterSettle.usedRounds.toNumber() - subBefore.usedRounds.toNumber(),
      1
    );

    const challenger = ctx.nodes[31];
    const challengerUsdc = await fundNodeOwnerInBankrun(
      ctx,
      challenger,
      ctx.underlyingTokenMint,
      BigInt(0)
    );

    await disputeRound({
      roundRecord: roundRecordPda,
      gateway: subGateway,
      challenger,
      challengerUsdc,
      nodeBits: bits,
      subscription: subscriptionPda,
    });

    const voided = await proto.program.account.roundReceiptRecord.fetch(
      roundRecordPda
    );
    assert.isOk(voided.status.voided);

    const subAfterDispute = await proto.program.account.subscription.fetch(
      subscriptionPda
    );
    assert.equal(
      subAfterDispute.usedRounds.toNumber(),
      subBefore.usedRounds.toNumber()
    );
  });

  it("voids a fabricated delegate-settled subscription round and restores used_rounds", async () => {
    const subscriptionPda = await ensureSubscription();
    const subGateway = await registerBondedGateway(ctx, proto);
    const delegateHotKey = Keypair.generate();
    const delegatePda = findDelegatePda(
      proto.program.programId,
      ctx.authority.publicKey,
      delegateHotKey.publicKey
    );

    await proto.program.methods
      .addDelegate({
        delegate: delegateHotKey.publicKey,
        maxDataRequests: new BN(100),
        maxSigners: 2,
      })
      .accountsPartial({
        owner: ctx.authority.publicKey,
        subscription: subscriptionPda,
        delegateAccount: delegatePda,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    const subBefore = await proto.program.account.subscription.fetch(
      subscriptionPda
    );

    const t = ++ts;
    const sourceId = Buffer.alloc(32, 0x89);
    const sigsRequired = 1;
    const selection = deriveSignersBitmap(
      sourceId,
      registryVersion,
      NODE_COUNT,
      sigsRequired,
      redundancyBuffer,
      t
    );
    const bits = bitsOf(selection);
    const fabricatedSig = zeroedAggSig(selection);

    const requestAuthHash = hashRequestAuthFields(sourceId, sigsRequired, t);
    const consumerSigIx = Ed25519Program.createInstructionWithPrivateKey({
      privateKey: delegateHotKey.secretKey,
      message: requestAuthHash,
    });
    const roundRecordPda = findSubscriptionRoundPda(
      proto.program.programId,
      ctx.authority.publicKey,
      sourceId,
      sigsRequired,
      t
    );

    await proto.program.methods
      .settleSubscriptionRound({
        timestamp: new BN(t),
        sourceId: Array.from(sourceId),
        signaturesRequired: sigsRequired,
        registryVersion,
        aggSig: fabricatedSig,
        value: Buffer.alloc(32),
        consumerAuthority: delegateHotKey.publicKey,
      })
      .accountsPartial({
        payer: ctx.authority.publicKey,
        protocolConfig: proto.protocolConfigPda,
        registryState: proto.registryStatePda,
        registry: registryPda,
        subscription: subscriptionPda,
        consumerDelegate: delegatePda,
        gateway: subGateway.gatewayPda,
        roundRecord: roundRecordPda,
        instructionsSysvar: SYSVAR_INSTRUCTIONS_PUBKEY,
        systemProgram: SystemProgram.programId,
      })
      .remainingAccounts(
        bits.map((bit) => ({
          pubkey: registeredNodePda(proto.program.programId, bit),
          isWritable: true,
          isSigner: false,
        }))
      )
      .preInstructions([consumerSigIx])
      .rpc();

    const subAfterSettle = await proto.program.account.subscription.fetch(
      subscriptionPda
    );
    assert.equal(
      subAfterSettle.usedRounds.toNumber() - subBefore.usedRounds.toNumber(),
      1
    );

    const challenger = ctx.nodes[29];
    const challengerUsdc = await fundNodeOwnerInBankrun(
      ctx,
      challenger,
      ctx.underlyingTokenMint,
      BigInt(0)
    );
    const expectedBounty = BigInt(
      (INIT.gatewayBondMin * INIT.challengerBountyBps) / 10_000
    );
    const expectedRemainder = BigInt(INIT.gatewayBondMin) - expectedBounty;
    const treasuryBefore = await readTokenAmount(ctx, proto.treasury);
    const challengerBefore = await readTokenAmount(ctx, challengerUsdc);
    const bondBefore = await readTokenAmount(ctx, subGateway.gatewayBondAta);
    assert.equal(bondBefore, BigInt(INIT.gatewayBondMin));

    await disputeRound({
      roundRecord: roundRecordPda,
      gateway: subGateway,
      challenger,
      challengerUsdc,
      nodeBits: bits,
      subscription: subscriptionPda,
    });

    const voided = await proto.program.account.roundReceiptRecord.fetch(
      roundRecordPda
    );
    assert.isOk(voided.status.voided);

    const subAfterDispute = await proto.program.account.subscription.fetch(
      subscriptionPda
    );
    assert.equal(
      subAfterDispute.usedRounds.toNumber(),
      subBefore.usedRounds.toNumber()
    );

    const gw = await proto.program.account.gateway.fetch(subGateway.gatewayPda);
    assert.isTrue(gw.slashed);
    assert.equal(gw.bond.toNumber(), 0);
    assert.equal(
      await readTokenAmount(ctx, subGateway.gatewayBondAta),
      BigInt(0)
    );
    assert.equal(
      await readTokenAmount(ctx, challengerUsdc),
      challengerBefore + expectedBounty
    );
    assert.equal(
      await readTokenAmount(ctx, proto.treasury),
      treasuryBefore + expectedRemainder
    );
  });

  it("still finalizes a subscription round record (seeds-constraint removal regression)", async () => {
    const subscriptionPda = await ensureSubscription();

    const t = ++ts;
    const sourceId = Buffer.alloc(32, 0x77);
    const requestAuthHash = hashRequestAuthFields(sourceId, 1, t);
    const consumerSigIx = Ed25519Program.createInstructionWithPrivateKey({
      privateKey: ctx.authority.payer.secretKey,
      message: requestAuthHash,
    });
    const roundRecordPda = findSubscriptionRoundPda(
      proto.program.programId,
      ctx.authority.publicKey,
      sourceId,
      1,
      t
    );

    await proto.program.methods
      .settleSubscriptionRound({
        timestamp: new BN(t),
        sourceId: Array.from(sourceId),
        signaturesRequired: 1,
        registryVersion,
        aggSig: zeroedAggSig(bitmapWithBits([0])),
        value: Buffer.alloc(32),
        consumerAuthority: ctx.authority.publicKey,
      })
      .accountsPartial({
        payer: ctx.authority.publicKey,
        protocolConfig: proto.protocolConfigPda,
        registryState: proto.registryStatePda,
        registry: registryPda,
        subscription: subscriptionPda,
        consumerDelegate: null,
        gateway: gateway.gatewayPda,
        roundRecord: roundRecordPda,
        instructionsSysvar: SYSVAR_INSTRUCTIONS_PUBKEY,
        systemProgram: SystemProgram.programId,
      })
      .remainingAccounts([
        {
          pubkey: registeredNodePda(proto.program.programId, 0),
          isWritable: true,
          isSigner: false,
        },
      ])
      .preInstructions([consumerSigIx])
      .rpc();

    await advanceClock(ctx, INIT.disputeWindowSlots + 10);
    await proto.program.methods
      .finalizeRound()
      .accountsPartial({
        caller: ctx.authority.publicKey,
        protocolConfig: proto.protocolConfigPda,
        registry: registryPda,
        roundRecord: roundRecordPda,
        gateway: gateway.gatewayPda,
      })
      .remainingAccounts([
        {
          pubkey: registeredNodePda(proto.program.programId, 0),
          isWritable: true,
          isSigner: false,
        },
      ])
      .rpc();

    const record = await proto.program.account.roundReceiptRecord.fetch(
      roundRecordPda
    );
    assert.isOk(record.status.finalized);
  });

  it("allows a permanent node to claim historical rewards after deactivation", async () => {
    const nodeOwner = ctx.nodes[0];
    const node = registeredNodePda(proto.program.programId, 0);
    const current = await proto.program.account.registry.fetch(registryPda);
    const removeIndex = current.nodes
      .slice(0, Number(current.nodeCount))
      .findIndex((value: number[]) =>
        new PublicKey(Uint8Array.from(value)).equals(node)
      );
    assert.isAtLeast(removeIndex, 0);

    const nextVersion = registryVersion + 1;
    const nextRegistry = findRegistryPda(proto.program.programId, nextVersion);
    await proto.program.methods
      .deactivateNode({
        nodeOwner: nodeOwner.publicKey,
        removeIndex,
        newRegistryVersion: nextVersion,
      })
      .accountsPartial({
        authority: nodeOwner.publicKey,
        protocolConfig: proto.protocolConfigPda,
        registryState: proto.registryStatePda,
        currentRegistry: registryPda,
        newRegistry: nextRegistry,
        node,
        systemProgram: SystemProgram.programId,
      })
      .signers([nodeOwner])
      .rpc();

    const beforeClaim = await proto.program.account.node.fetch(node);
    assert.property(beforeClaim.status, "deactivated");
    assert.isAbove(beforeClaim.claimableRewards.toNumber(), 0);

    const destination = getAssociatedTokenAddressSync(
      proto.usdcMint,
      nodeOwner.publicKey
    );
    await proto.program.methods
      .claimNodeRewards()
      .accountsPartial({
        owner: nodeOwner.publicKey,
        protocolConfig: proto.protocolConfigPda,
        node,
        treasury: proto.treasury,
        destination,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([nodeOwner])
      .rpc();

    const afterClaim = await proto.program.account.node.fetch(node);
    assert.equal(afterClaim.claimableRewards.toNumber(), 0);
  });
});
