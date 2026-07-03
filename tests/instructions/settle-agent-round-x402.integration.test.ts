import { BN } from "@coral-xyz/anchor";
import { assert } from "chai";
import { Keypair } from "@solana/web3.js";
import { TOKEN_PROGRAM_ID } from "@solana/spl-token";
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
  createFundedEscrow,
  creditedNodePdasForBits,
  findAgentRoundPda,
  freshRoundPrice,
  INIT,
  initializeProtocolForAgents,
  registerBondedGateway,
  settleAgentRound,
  sha256,
  zeroedAggSig,
} from "../helpers/agent-access";

const NODE_COUNT = 4;
const SIGS_REQUIRED = 2;
const JOB_ID = Uint8Array.from(Buffer.alloc(32, 0x24));
const PAYMENT_REF = Uint8Array.from(sha256([Buffer.from("x402-settlement-1")]));

describe("settle_agent_round (X402 rail) + gateway settlement balance", () => {
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

  // X402 rounds carry a gateway-assigned agent identity; no escrow account exists.
  const x402Agent = Keypair.generate().publicKey;

  const baseParams = (over: Record<string, any> = {}) => ({
    proto,
    gateway,
    agent: x402Agent,
    requestNonce: ++nonce,
    requestKind: "freshRound" as const,
    rail: "x402" as const,
    maxPrice: freshRoundPrice(SIGS_REQUIRED),
    paymentRef: PAYMENT_REF,
    jobId: JOB_ID,
    timestamp: ++ts,
    signaturesRequired: SIGS_REQUIRED,
    registryVersion,
    aggSig: zeroedAggSig(bitmapWithBits([0, 1])),
    creditedNodePdas: creditedNodePdasForBits(proto.program.programId, [0, 1]),
    ...over,
  });

  it("rejects X402 settlement while the gateway settlement balance is unfunded", async () => {
    try {
      await settleAgentRound(baseParams());
      assert.fail("expected GatewaySettlementUnderfunded");
    } catch (e: any) {
      assert.include(e.toString(), "GatewaySettlementUnderfunded");
    }
  });

  it("funds the settlement balance (gateway-authority-gated)", async () => {
    await proto.program.methods
      .depositGatewaySettlement(new BN(10_000))
      .accountsPartial({
        gatewayAuthority: gateway.gatewayAuthority.publicKey,
        protocolConfig: proto.protocolConfigPda,
        gateway: gateway.gatewayPda,
        usdcMint: ctx.underlyingTokenMint,
        authorityUsdc: gateway.authorityUsdc,
        treasury: proto.treasury,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([gateway.gatewayAuthority])
      .rpc();

    const gw = await proto.program.account.gateway.fetch(gateway.gatewayPda);
    assert.equal(gw.settlementBalance.toNumber(), 10_000);
    // Bond untouched — settlement working capital is not slashing collateral.
    assert.equal(gw.bond.toNumber(), INIT.gatewayBondMin);
  });

  it("settles an X402 FreshRound from the gateway settlement balance", async () => {
    const price = freshRoundPrice(SIGS_REQUIRED);
    const before = await proto.program.account.gateway.fetch(gateway.gatewayPda);

    const params = baseParams();
    await settleAgentRound(params);

    const after = await proto.program.account.gateway.fetch(gateway.gatewayPda);
    assert.equal(
      before.settlementBalance.toNumber() - after.settlementBalance.toNumber(),
      price
    );
    // Gateway still accrues its fee through the normal pending path.
    assert.equal(
      after.pendingRewards.toNumber() - before.pendingRewards.toNumber(),
      INIT.gatewayFee
    );

    const record = await proto.program.account.roundReceiptRecord.fetch(
      findAgentRoundPda(proto.program.programId, x402Agent, params.requestNonce)
    );
    assert.equal(record.rail, 1); // X402
    assert.equal(record.kind, 1); // RECORD_KIND_AGENT_FRESH
    assert.equal(record.price.toNumber(), price);
    assert.deepEqual(
      Uint8Array.from(record.paymentRef),
      PAYMENT_REF
    );
  });

  it("settles an X402 CachedRead", async () => {
    const before = await proto.program.account.gateway.fetch(gateway.gatewayPda);
    const params = baseParams({
      requestKind: "cachedRead",
      maxPrice: INIT.agentCachedReadPrice,
      aggSig: zeroedAggSig(),
      creditedNodePdas: [],
    });
    await settleAgentRound(params);

    const after = await proto.program.account.gateway.fetch(gateway.gatewayPda);
    assert.equal(
      before.settlementBalance.toNumber() - after.settlementBalance.toNumber(),
      INIT.agentCachedReadPrice
    );
    assert.equal(
      after.claimableRewards.toNumber() - before.claimableRewards.toNumber(),
      INIT.gatewayFee
    );

    const record = await proto.program.account.roundReceiptRecord.fetch(
      findAgentRoundPda(proto.program.programId, x402Agent, params.requestNonce)
    );
    assert.isTrue(record.finalized);
    assert.equal(record.kind, 2); // RECORD_KIND_AGENT_CACHED
  });

  it("rejects a zero payment_ref on the X402 rail", async () => {
    try {
      await settleAgentRound(baseParams({ paymentRef: new Uint8Array(32) }));
      assert.fail("expected MissingPaymentRef");
    } catch (e: any) {
      assert.include(e.toString(), "MissingPaymentRef");
    }
  });

  it("rejects an escrow account supplied on the X402 rail", async () => {
    const escrow = await createFundedEscrow(ctx, proto, {
      agentIndex: 50,
      depositAmount: 10_000,
    });
    try {
      await settleAgentRound(
        baseParams({ escrow, ed25519Signer: null })
      );
      assert.fail("expected UnexpectedAgentEscrow");
    } catch (e: any) {
      assert.include(e.toString(), "UnexpectedAgentEscrow");
    }
  });

  it("rejects deposit_gateway_settlement from a non-authority signer", async () => {
    const impostor = Keypair.generate();
    const { fundNodeOwnerInBankrun } = await import(
      "../helpers/evm-submit-data-update"
    );
    const impostorUsdc = await fundNodeOwnerInBankrun(
      ctx,
      impostor,
      ctx.underlyingTokenMint
    );
    try {
      await proto.program.methods
        .depositGatewaySettlement(new BN(1_000))
        .accountsPartial({
          gatewayAuthority: impostor.publicKey,
          protocolConfig: proto.protocolConfigPda,
          gateway: gateway.gatewayPda,
          usdcMint: ctx.underlyingTokenMint,
          authorityUsdc: impostorUsdc,
          treasury: proto.treasury,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .signers([impostor])
        .rpc();
      assert.fail("expected seeds/authority constraint failure");
    } catch (e: any) {
      assert.ok(e);
    }
  });
});
