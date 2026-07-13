import { assert } from "chai";
import { Keypair } from "@solana/web3.js";
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
  findAgentRoundPda,
  findEscrowAta,
  findX402EscrowPda,
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
 * x402 as a *funding mechanism* (not a parallel rail): the client pays plain `exact` USDC to
 * the deterministic escrow ATA `ATA(PDA(["molpha_agent", payer]), USDC)`. There is no escrow
 * data account and no `AgentRequestAuth` signature — the on-chain payment is the
 * authorization. Settlement moves `price` from that ATA into the treasury (the escrow PDA
 * signs) and credits nodes + gateway exactly like the owner-provisioned path.
 */
describe("settle_agent_round (x402 funding of AgentEscrow)", () => {
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
  const escrowPda = () => findX402EscrowPda(proto.program.programId, payer);
  const escrowAta = () => findEscrowAta(proto.usdcMint, escrowPda());

  const baseParams = (over: Record<string, any> = {}) => ({
    proto,
    gateway,
    // No `escrow` -> x402 mode; owner == authority == payer, escrow lazily created by settle.
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
    const gwBefore = await proto.program.account.gateway.fetch(gateway.gatewayPda);

    const params = baseParams();
    await settleAgentRound(params);

    const ataAfter = await readTokenAmount(ctx, escrowAta());
    const treasuryAfter = await readTokenAmount(ctx, proto.treasury);
    assert.equal(ataBefore - ataAfter, BigInt(price));
    assert.equal(treasuryAfter - treasuryBefore, BigInt(price));

    const gwAfter = await proto.program.account.gateway.fetch(gateway.gatewayPda);
    assert.equal(
      gwAfter.pendingRewards.toNumber() - gwBefore.pendingRewards.toNumber(),
      INIT.gatewayFee
    );

    // Settle lazily created the x402 escrow: owner == authority == payer, caps 0.
    const escrow = await proto.program.account.agentEscrow.fetch(escrowPda());
    assert.equal(escrow.owner.toBase58(), payer.toBase58());
    assert.equal(escrow.authority.toBase58(), payer.toBase58());
    assert.equal(escrow.maxSigners, 0);
    assert.equal(escrow.epochCap.toNumber(), 0);

    const record = await proto.program.account.roundReceiptRecord.fetch(
      findAgentRoundPda(proto.program.programId, escrowPda(), params.requestNonce)
    );
    assert.equal(record.kind, 1); // RECORD_KIND_AGENT_FRESH
    assert.equal(record.price.toNumber(), price);
    assert.equal(record.agent.toBase58(), escrowPda().toBase58());
    assert.equal(record.requestNonce.toNumber(), params.requestNonce);
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
    const poorEscrow = findX402EscrowPda(proto.program.programId, poorPayer);
    const poorAta = findEscrowAta(proto.usdcMint, poorEscrow);
    // Fund below the FreshRound price.
    fundAtaInBankrun(ctx, poorAta, proto.usdcMint, poorEscrow, 500);
    try {
      await settleAgentRound(baseParams({ payer: poorPayer, requestNonce: 900 }));
      assert.fail("expected InsufficientAgentCredits");
    } catch (e: any) {
      assert.include(e.toString(), "InsufficientAgentCredits");
    }
  });
});
