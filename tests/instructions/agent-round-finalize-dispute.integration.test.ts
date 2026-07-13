import { BN } from "@coral-xyz/anchor";
import { assert } from "chai";
import {
  ComputeBudgetProgram,
  Ed25519Program,
  PublicKey,
  SystemProgram,
  SYSVAR_INSTRUCTIONS_PUBKEY,
} from "@solana/web3.js";
import { TOKEN_PROGRAM_ID } from "@solana/spl-token";
import { setupTestContext, TestContext } from "../setup";
import {
  deriveSignersBitmapForJob,
  evmBitmapBitSet,
  fundNodeOwnerInBankrun,
  generateE2eSignerMaterial,
  registerNodesForE2e,
  signE2eDataUpdate,
} from "../helpers/evm-submit-data-update";
import {
  advanceClock,
  AgentProtocolContext,
  AgentEscrowSetup,
  BondedGateway,
  bitmapWithBits,
  createFundedEscrow,
  creditedNodePdasForBits,
  findAgentRoundPda,
  findNodePda,
  freshRoundPrice,
  INIT,
  initializeProtocolForAgents,
  registerBondedGateway,
  settleAgentRound,
  sha256,
  u64le,
  zeroedAggSig,
} from "../helpers/agent-access";

const NODE_COUNT = 4;
const SIGS_REQUIRED = 2;
const FEED_ID = Uint8Array.from(Buffer.alloc(32, 0x66));
const DOMAIN_REQAUTH = Buffer.from("MOLPHA_REQAUTH_V1", "utf8");
const PLAN_SEED = Buffer.from("molpha_plan");
const SUBSCRIPTION_SEED = Buffer.from("molpha_subscription");
const ROUND_SEED = Buffer.from("molpha_round");

function bitsOf(bitmap: Uint8Array): number[] {
  const bits: number[] = [];
  for (let bit = 0; bit < 256; bit++) {
    if (evmBitmapBitSet(bitmap, bit)) bits.push(bit);
  }
  return bits;
}

describe("agent rounds: finalize + dispute through the shared path", () => {
  let ctx: TestContext;
  let proto: AgentProtocolContext;
  let gateway: BondedGateway;
  let victimGateway: BondedGateway;
  let escrow: AgentEscrowSetup;
  let material: ReturnType<typeof generateE2eSignerMaterial>;
  let registryVersion: number;
  let ts = 1_700_200_000;
  let nonce = 0;

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
      material.compressedPubkeys,
      advanceClock
    );
    const registry = await proto.program.account.registryState.fetch(
      proto.registryStatePda
    );
    registryVersion = registry.currentVersion;
    gateway = await registerBondedGateway(ctx, proto);
    victimGateway = await registerBondedGateway(ctx, proto);
    escrow = await createFundedEscrow(ctx, proto, { depositAmount: 500_000 });
  }, 180_000);

  async function disputeRound(params: {
    roundRecord: PublicKey;
    gateway: BondedGateway;
    aggSig: ReturnType<typeof zeroedAggSig>;
    challengerUsdc: PublicKey;
    challenger: any;
    nodeBits: number[];
  }): Promise<void> {
    await proto.program.methods
      .disputeRound(params.aggSig)
      .accountsPartial({
        challenger: params.challenger.publicKey,
        protocolConfig: proto.protocolConfigPda,
        registryState: proto.registryStatePda,
        roundRecord: params.roundRecord,
        gateway: params.gateway.gatewayPda,
        treasury: proto.treasury,
        challengerUsdc: params.challengerUsdc,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .remainingAccounts(
        params.nodeBits.map((bit) => ({
          pubkey: findNodePda(proto.program.programId, bit),
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

  it("finalizes an agent FreshRound after the dispute window", async () => {
    const n = ++nonce;
    const t = ++ts;
    await settleAgentRound({
      proto,
      gateway,
      escrow,
      requestNonce: n,
      maxPrice: freshRoundPrice(SIGS_REQUIRED),
      feedId: FEED_ID,
      timestamp: t,
      signaturesRequired: SIGS_REQUIRED,
      registryVersion,
      aggSig: zeroedAggSig(bitmapWithBits([0, 1])),
      creditedNodePdas: creditedNodePdasForBits(
        proto.program.programId,
        [0, 1]
      ),
    });
    const recordPda = findAgentRoundPda(
      proto.program.programId,
      escrow.escrowPda,
      n
    );

    const finalize = (computeUnitLimit?: number) =>
      proto.program.methods
        .finalizeRound()
        .accountsPartial({
          caller: ctx.authority.publicKey,
          protocolConfig: proto.protocolConfigPda,
          registryState: proto.registryStatePda,
          roundRecord: recordPda,
          gateway: gateway.gatewayPda,
        })
        .remainingAccounts(
          [0, 1].map((bit) => ({
            pubkey: findNodePda(proto.program.programId, bit),
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

    await advanceClock(ctx, INIT.disputeWindowSlots + 10);

    const nodeBefore = await proto.program.account.node.fetch(
      findNodePda(proto.program.programId, 0)
    );
    const gwBefore = await proto.program.account.gateway.fetch(
      gateway.gatewayPda
    );
    // Bankrun does not rotate the blockhash when setClock advances time. A
    // harmless compute-budget instruction keeps this retry's signature unique.
    await finalize(200_001);

    const nodeAfter = await proto.program.account.node.fetch(
      findNodePda(proto.program.programId, 0)
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
      nodeBefore.pendingRewards.toNumber() -
        nodeAfter.pendingRewards.toNumber(),
      INIT.rewardPerSignature
    );
    assert.equal(
      gwAfter.claimableRewards.toNumber() -
        gwBefore.claimableRewards.toNumber(),
      INIT.gatewayFee
    );

    // round record is closed by the finalize round instruction
    try {
      await proto.program.account.roundReceiptRecord.fetch(recordPda);
      assert.fail("expected RoundReceiptRecordNotFound");
    } catch (e: any) {
      assert.include(e.toString(), "Could not find " + recordPda.toBase58());
    }
  });

  it("voids a fabricated agent round on dispute and slashes the gateway bond", async () => {
    const n = ++nonce;
    const t = ++ts;
    // The credited set must be the deterministically derived selection so the dispute
    // reaches Schnorr verification (which then fails on the garbage signature).
    const selection = deriveSignersBitmapForJob(
      FEED_ID,
      registryVersion,
      NODE_COUNT,
      SIGS_REQUIRED,
      INIT.redundancyBuffer,
      t
    );
    const bits = bitsOf(selection);
    const fabricatedSig = zeroedAggSig(selection);

    await settleAgentRound({
      proto,
      gateway: victimGateway,
      escrow,
      requestNonce: n,
      maxPrice: freshRoundPrice(SIGS_REQUIRED),
      feedId: FEED_ID,
      timestamp: t,
      signaturesRequired: SIGS_REQUIRED,
      registryVersion,
      aggSig: fabricatedSig,
      creditedNodePdas: creditedNodePdasForBits(proto.program.programId, bits),
    });
    const recordPda = findAgentRoundPda(
      proto.program.programId,
      escrow.escrowPda,
      n
    );

    const challenger = ctx.nodes[30];
    const challengerUsdc = await fundNodeOwnerInBankrun(
      ctx,
      challenger,
      ctx.underlyingTokenMint,
      BigInt(0)
    );

    await disputeRound({
      roundRecord: recordPda,
      gateway: victimGateway,
      aggSig: fabricatedSig,
      challenger,
      challengerUsdc,
      nodeBits: bits,
    });

    // round record is closed by the dispute round instruction
    try {
      await proto.program.account.roundReceiptRecord.fetch(recordPda);
      assert.fail("expected RoundReceiptRecordNotFound");
    } catch (e: any) {
      assert.include(e.toString(), "Could not find " + recordPda.toBase58());
    }

    const gw = await proto.program.account.gateway.fetch(
      victimGateway.gatewayPda
    );
    assert.isTrue(gw.slashed);
    assert.equal(gw.bond.toNumber(), 0);

    const challengerAccount = await ctx.provider.context.banksClient.getAccount(
      challengerUsdc
    );
    const bounty = Buffer.from(challengerAccount!.data).readBigUInt64LE(64);
    assert.equal(
      bounty,
      BigInt((INIT.gatewayBondMin * INIT.challengerBountyBps) / 10_000)
    );
  });

  it("fails to dispute a genuinely valid full-participation agent round", async () => {
    const n = ++nonce;
    const t = ++ts;
    const value = Uint8Array.from(Buffer.alloc(32, 0x99));
    // Full participation: signers == derived selection. Before the subset-check fix this
    // was falsely adjudicated as fraud; it must now fail with DisputeSignatureValid.
    const signed = signE2eDataUpdate(
      registryVersion,
      material,
      NODE_COUNT, // derive the selection bitmap for this job/timestamp
      FEED_ID,
      {
        configHash: new Uint8Array(32),
        value,
        canonicalTimestamp: t,
      },
      SIGS_REQUIRED,
      NODE_COUNT,
      INIT.redundancyBuffer
    );
    const bits = bitsOf(signed.signersBitmap);
    const validSig = {
      s: Array.from(signed.aggSigS),
      rx: Array.from(new Uint8Array(32)),
      ryParity: 0,
      commitmentAddr: Array.from(signed.commitmentAddr),
      signersBitmap: Array.from(signed.signersBitmap),
    };

    await settleAgentRound({
      proto,
      gateway,
      escrow,
      requestNonce: n,
      maxPrice: freshRoundPrice(SIGS_REQUIRED),
      feedId: FEED_ID,
      timestamp: t,
      signaturesRequired: SIGS_REQUIRED,
      registryVersion,
      aggSig: validSig,
      value,
      creditedNodePdas: creditedNodePdasForBits(proto.program.programId, bits),
    });
    const recordPda = findAgentRoundPda(
      proto.program.programId,
      escrow.escrowPda,
      n
    );

    try {
      await disputeRound({
        roundRecord: recordPda,
        gateway,
        aggSig: validSig,
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
  });

  it("still finalizes a subscription round record (seeds-constraint removal regression)", async () => {
    // Plan + subscription for the authority wallet.
    const planId = 1; // PlanType::Standard
    const [planPda] = PublicKey.findProgramAddressSync(
      [PLAN_SEED, Buffer.from([planId])],
      proto.program.programId
    );
    const [subscriptionPda] = PublicKey.findProgramAddressSync(
      [SUBSCRIPTION_SEED, ctx.authority.publicKey.toBuffer()],
      proto.program.programId
    );
    await proto.program.methods
      .addPlan({
        planType: { standard: {} },
        subscriptionPrice: new BN(2_000_000),
        maxJobs: 3,
        signaturesRequired: 1,
        maxSigners: 4,
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

    // Subscription settle: consumer Ed25519 attestation over H(RequestAuth).
    const t = ++ts;
    const subFeedId = Buffer.alloc(32, 0x77);
    const requestAuthHash = sha256([
      DOMAIN_REQAUTH,
      Buffer.concat([subFeedId, u64le(t)]),
    ]);
    const consumerSigIx = Ed25519Program.createInstructionWithPrivateKey({
      privateKey: ctx.authority.payer.secretKey,
      message: requestAuthHash,
    });
    const [roundRecordPda] = PublicKey.findProgramAddressSync(
      [ROUND_SEED, subFeedId, u64le(t)],
      proto.program.programId
    );

    await proto.program.methods
      .settleRoundReceipt({
        feedId: Array.from(subFeedId),
        timestamp: new BN(t),
        signaturesRequired: 1,
        registryVersion,
        aggSig: zeroedAggSig(bitmapWithBits([0])),
        value: Array.from(new Uint8Array(32)),
        consumerAuthority: ctx.authority.publicKey,
      })
      .accountsPartial({
        payer: ctx.authority.publicKey,
        protocolConfig: proto.protocolConfigPda,
        registryState: proto.registryStatePda,
        plan: null,
        subscription: subscriptionPda,
        consumerDelegate: null,
        gateway: gateway.gatewayPda,
        roundRecord: roundRecordPda,
        instructionsSysvar: SYSVAR_INSTRUCTIONS_PUBKEY,
        systemProgram: SystemProgram.programId,
      })
      .remainingAccounts([
        {
          pubkey: findNodePda(proto.program.programId, 0),
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
        registryState: proto.registryStatePda,
        roundRecord: roundRecordPda,
        gateway: gateway.gatewayPda,
      })
      .remainingAccounts([
        {
          pubkey: findNodePda(proto.program.programId, 0),
          isWritable: true,
          isSigner: false,
        },
      ])
      .rpc();

    // round record is closed by the finalize round instruction
    try {
      await proto.program.account.roundReceiptRecord.fetch(roundRecordPda);
      assert.fail("expected RoundReceiptRecordNotFound");
    } catch (e: any) {
      assert.include(
        e.toString(),
        "Could not find " + roundRecordPda.toBase58()
      );
    }
  });
});
