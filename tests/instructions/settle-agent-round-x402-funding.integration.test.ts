import { assert } from "chai";
import { Keypair, PublicKey } from "@solana/web3.js";
import { setupTestContext, TestContext } from "../setup";
import {
  generateE2eSignerMaterial,
  registerNodesForE2e,
} from "../helpers/evm-submit-data-update";
import {
  advanceClock,
  AgentProtocolContext,
  BondedGateway,
  bitmapWithBits,
  creditedNodePdasForBits,
  findAgentEscrowPda,
  findAgentRoundPda,
  findEscrowAta,
  findNodePda,
  freshRoundPrice,
  fundAtaInBankrun,
  INIT,
  initializeProtocolForAgents,
  readTokenAmount,
  registerBondedGateway,
  settleAgentRound,
  zeroedAggSig,
} from "../helpers/agent-access";

const NODE_COUNT = 4;
const SIGS_REQUIRED = 2;
const FEED_ID = Uint8Array.from(Buffer.alloc(32, 0x24));

/**
 * x402 / self-funded pay-per-request: the client pays plain `exact` USDC to the
 * deterministic escrow ATA `ATA(PDA(["molpha_agent", payer]), USDC)`. There is no
 * pre-existing escrow data account and no request signature — the on-chain payment is the
 * authorization, and `settle_agent_round` lazily creates the escrow. Settlement moves
 * `price` from that ATA into the treasury (the escrow PDA signs) and credits nodes +
 * gateway exactly like the subscription path. This is the ONLY use of `settle_agent_round`;
 * owner-provisioned agents are `Delegate`s settled via `settle_round_receipt`.
 */
describe("settle_agent_round (x402 / self-funded rail)", () => {
  let ctx: TestContext;
  let proto: AgentProtocolContext;
  let gateway: BondedGateway;
  let registryVersion: number;
  let ts = 1_700_100_000;
  let nonce = 0;

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
      material.compressedPubkeys,
      advanceClock
    );
    const registry = await proto.program.account.registryState.fetch(
      proto.registryStatePda
    );
    registryVersion = registry.currentVersion;
    gateway = await registerBondedGateway(ctx, proto);
  }, 180_000);

  // The x402 payer is just an identity; no on-chain account is required for it.
  const payer = Keypair.generate().publicKey;
  const escrowPda = () => findAgentEscrowPda(proto.program.programId, payer);
  const escrowAta = () => findEscrowAta(proto.usdcMint, escrowPda());

  const baseParams = (over: Record<string, any> = {}) => ({
    proto,
    gateway,
    payer,
    requestNonce: ++nonce,
    maxPrice: freshRoundPrice(SIGS_REQUIRED),
    feedId: FEED_ID,
    timestamp: ++ts,
    signaturesRequired: SIGS_REQUIRED,
    registryVersion,
    aggSig: zeroedAggSig(bitmapWithBits([0, 1])),
    creditedNodePdas: creditedNodePdasForBits(proto.program.programId, [0, 1]),
    ...over,
  });

  it("settles an x402-funded FreshRound: moves price ATA -> treasury, credits nodes + gateway", async () => {
    const price = freshRoundPrice(SIGS_REQUIRED);
    // Simulate the client's x402 `exact` payment landing in the escrow ATA.
    fundAtaInBankrun(ctx, escrowAta(), proto.usdcMint, escrowPda(), 1_000_000);

    // The escrow data account does not exist yet — settle lazily creates it.
    assert.isNull(
      await ctx.provider.context.banksClient.getAccount(escrowPda())
    );

    const ataBefore = await readTokenAmount(ctx, escrowAta());
    const treasuryBefore = await readTokenAmount(ctx, proto.treasury);
    const gwBefore = await proto.program.account.gateway.fetch(
      gateway.gatewayPda
    );

    const params = baseParams();
    await settleAgentRound(params);

    const ataAfter = await readTokenAmount(ctx, escrowAta());
    const treasuryAfter = await readTokenAmount(ctx, proto.treasury);
    assert.equal(ataBefore - ataAfter, BigInt(price));
    assert.equal(treasuryAfter - treasuryBefore, BigInt(price));

    const gwAfter = await proto.program.account.gateway.fetch(
      gateway.gatewayPda
    );
    assert.equal(
      gwAfter.pendingRewards.toNumber() - gwBefore.pendingRewards.toNumber(),
      INIT.gatewayFee
    );

    // Settle lazily created the x402 escrow keyed by the payer identity.
    const escrow = await proto.program.account.agentEscrow.fetch(escrowPda());
    assert.equal(escrow.authority.toBase58(), payer.toBase58());
    assert.deepEqual(escrow.status, { active: {} });
    assert.equal(escrow.spentTotal.toNumber(), price);

    const record = await proto.program.account.roundReceiptRecord.fetch(
      findAgentRoundPda(
        proto.program.programId,
        escrowPda(),
        params.requestNonce
      )
    );
    assert.equal(record.price.toNumber(), price);
  });

  it("blocks replay of the same (payer, request_nonce) via the record PDA", async () => {
    const params = baseParams();
    await settleAgentRound(params);
    try {
      await settleAgentRound({ ...params, timestamp: params.timestamp + 1 });
      assert.fail("expected duplicate record PDA failure");
    } catch (e: any) {
      assert.ok(e);
    }
  });

  it("rejects settlement when the escrow ATA is underfunded", async () => {
    const poorPayer = Keypair.generate().publicKey;
    const poorEscrow = findAgentEscrowPda(proto.program.programId, poorPayer);
    const poorAta = findEscrowAta(proto.usdcMint, poorEscrow);
    // Fund below the FreshRound price.
    fundAtaInBankrun(ctx, poorAta, proto.usdcMint, poorEscrow, 500);
    try {
      await settleAgentRound(
        baseParams({ payer: poorPayer, requestNonce: 900 })
      );
      assert.fail("expected InsufficientAgentCredits");
    } catch (e: any) {
      assert.include(e.toString(), "InsufficientAgentCredits");
    }
  });

  it("rejects when the computed price exceeds the payer's max_price", async () => {
    try {
      await settleAgentRound(
        baseParams({ maxPrice: freshRoundPrice(SIGS_REQUIRED) - 1 })
      );
      assert.fail("expected MaxPriceExceeded");
    } catch (e: any) {
      assert.include(e.toString(), "MaxPriceExceeded");
    }
  });

  it("enforces quorum shape: threshold, inflation bound, node account list", async () => {
    // Below threshold.
    try {
      await settleAgentRound(
        baseParams({
          aggSig: zeroedAggSig(bitmapWithBits([0])),
          creditedNodePdas: creditedNodePdasForBits(proto.program.programId, [
            0,
          ]),
        })
      );
      assert.fail("expected QuorumBelowThreshold");
    } catch (e: any) {
      assert.include(e.toString(), "QuorumBelowThreshold");
    }

    // Above signatures_required + redundancy_buffer.
    try {
      await settleAgentRound(
        baseParams({
          signaturesRequired: 1,
          maxPrice: freshRoundPrice(1),
          aggSig: zeroedAggSig(bitmapWithBits([0, 1, 2, 3])),
          creditedNodePdas: creditedNodePdasForBits(
            proto.program.programId,
            [0, 1, 2, 3]
          ),
        })
      );
      assert.fail("expected CreditedExceedsSelection");
    } catch (e: any) {
      assert.include(e.toString(), "CreditedExceedsSelection");
    }

    // Wrong remaining-accounts count.
    try {
      await settleAgentRound(
        baseParams({
          creditedNodePdas: creditedNodePdasForBits(proto.program.programId, [
            0,
          ]),
        })
      );
      assert.fail("expected InvalidCreditNodeAccount");
    } catch (e: any) {
      assert.include(e.toString(), "InvalidCreditNodeAccount");
    }

    // Wrong node order (index mismatch with bit position).
    try {
      await settleAgentRound(
        baseParams({
          creditedNodePdas: [
            findNodePda(proto.program.programId, 1),
            findNodePda(proto.program.programId, 0),
          ],
        })
      );
      assert.fail("expected InvalidNodeIndex");
    } catch (e: any) {
      assert.include(e.toString(), "InvalidNodeIndex");
    }
  });

  it("rejects a stale registry version", async () => {
    try {
      await settleAgentRound(
        baseParams({ registryVersion: registryVersion + 1 })
      );
      assert.fail("expected InvalidRegistryVersion");
    } catch (e: any) {
      assert.include(e.toString(), "InvalidRegistryVersion");
    }
  });
});
