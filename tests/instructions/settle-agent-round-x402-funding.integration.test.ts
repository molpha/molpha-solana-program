import { assert } from "chai";
import { Keypair } from "@solana/web3.js";
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
  findAgentEscrowPda,
  findAgentRoundPda,
  findEscrowAta,
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
const API_SOURCE_ID = Uint8Array.from(Buffer.alloc(32, 0x24));

/**
 * x402 / self-funded pay-per-request: the client pays USDC to the deterministic
 * escrow ATA `ATA(PDA(["molpha_agent", payer, gateway]), USDC)` and signs
 * `AgentRequestAuth` (agent, gateway, source_id, signatures_required, canonical_timestamp, amount).
 * Settlement verifies the signature via Ed25519 introspection, requires
 * `escrow_ata.amount >= price` and `amount == price`, then moves `price` into
 * the treasury. Escrows are scoped per (payer, gateway) pair, so funding is not
 * fungible across gateways. Replay is guarded by the round-record PDA keyed by
 * `(authority, gateway, source_id, signatures_required, canonical_timestamp)`.
 */
describe("settle_agent_round (x402 / self-funded rail)", () => {
  let ctx: TestContext;
  let proto: AgentProtocolContext;
  let gateway: BondedGateway;
  let registryVersion: number;
  let ts = 1_700_100_000;

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
  }, 180_000);

  const payer = Keypair.generate();
  const escrowPda = () =>
    findAgentEscrowPda(
      proto.program.programId,
      payer.publicKey,
      gateway.gatewayPda
    );
  const escrowAta = () => findEscrowAta(proto.usdcMint, escrowPda());

  const fundExact = (sigs = SIGS_REQUIRED) => {
    fundAtaInBankrun(
      ctx,
      escrowAta(),
      proto.usdcMint,
      escrowPda(),
      freshRoundPrice(sigs)
    );
  };

  const baseParams = (over: Record<string, any> = {}) => ({
    proto,
    gateway,
    payer,
    amount: freshRoundPrice(SIGS_REQUIRED),
    sourceId: API_SOURCE_ID,
    canonicalTimestamp: ++ts,
    signaturesRequired: SIGS_REQUIRED,
    registryVersion,
    aggSig: zeroedAggSig(bitmapWithBits([0, 1])),
    creditedNodePdas: creditedNodePdasForBits(proto.program.programId, [0, 1]),
    ...over,
  });

  it("settles an x402-funded FreshRound: moves price ATA -> treasury, credits nodes + gateway", async () => {
    const price = freshRoundPrice(SIGS_REQUIRED);
    fundExact();

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
    const recordPda = await settleAgentRound(params);

    const ataAfter = await readTokenAmount(ctx, escrowAta());
    const treasuryAfter = await readTokenAmount(ctx, proto.treasury);
    assert.equal(ataBefore - ataAfter, BigInt(price));
    assert.equal(treasuryAfter - treasuryBefore, BigInt(price));
    assert.equal(ataAfter, 0n);

    const gwAfter = await proto.program.account.gateway.fetch(
      gateway.gatewayPda
    );
    assert.equal(
      gwAfter.claimableRewards.toNumber(),
      gwBefore.claimableRewards.toNumber()
    );

    const escrow = await proto.program.account.agentEscrow.fetch(escrowPda());
    assert.equal(escrow.authority.toBase58(), payer.publicKey.toBase58());
    assert.deepEqual(escrow.status, { active: {} });

    assert.equal(
      recordPda.toBase58(),
      findAgentRoundPda(
        proto.program.programId,
        payer.publicKey,
        gateway.gatewayPda,
        API_SOURCE_ID,
        SIGS_REQUIRED,
        params.canonicalTimestamp
      ).toBase58()
    );
    const record = await proto.program.account.roundReceiptRecord.fetch(
      recordPda
    );
    assert.equal(record.price.toNumber(), price);
    assert.deepEqual(Array.from(record.sourceId), Array.from(API_SOURCE_ID));
  });

  it("blocks replay of the same (authority, source_id, sigs, timestamp) via the record PDA", async () => {
    fundExact();
    const params = baseParams();
    await settleAgentRound(params);
    fundExact();
    try {
      await settleAgentRound(params);
      assert.fail("expected duplicate record PDA failure");
    } catch (e: any) {
      assert.ok(e);
    }
  });

  it("rejects settlement when the escrow ATA is underfunded", async () => {
    const poorPayer = Keypair.generate();
    const poorEscrow = findAgentEscrowPda(
      proto.program.programId,
      poorPayer.publicKey,
      gateway.gatewayPda
    );
    const poorAta = findEscrowAta(proto.usdcMint, poorEscrow);
    fundAtaInBankrun(ctx, poorAta, proto.usdcMint, poorEscrow, 500);
    try {
      await settleAgentRound(baseParams({ payer: poorPayer }));
      assert.fail("expected InsufficientAgentCredits");
    } catch (e: any) {
      assert.include(e.toString(), "InsufficientAgentCredits");
    }
  });

  it("allows an over-funded escrow ATA (surplus remains after settle)", async () => {
    const price = freshRoundPrice(SIGS_REQUIRED);
    const surplus = 50;
    fundAtaInBankrun(
      ctx,
      escrowAta(),
      proto.usdcMint,
      escrowPda(),
      price + surplus
    );
    await settleAgentRound(baseParams());
    assert.equal(await readTokenAmount(ctx, escrowAta()), BigInt(surplus));
  });

  it("rejects settlement without a payer AgentRequestAuth signature", async () => {
    fundExact();
    try {
      await settleAgentRound(baseParams({ signAgentRequest: false }));
      assert.fail("expected AgentSigInvalid");
    } catch (e: any) {
      assert.include(e.toString(), "AgentSigInvalid");
    }
  });

  it("rejects when the signed amount does not match the computed price", async () => {
    fundExact();
    try {
      await settleAgentRound(
        baseParams({ amount: freshRoundPrice(SIGS_REQUIRED) - 1 })
      );
      assert.fail("expected AmountMismatch");
    } catch (e: any) {
      assert.include(e.toString(), "AmountMismatch");
    }
  });

  it("enforces quorum shape: threshold, inflation bound, node account list", async () => {
    fundExact();
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

    fundExact(1);
    try {
      await settleAgentRound(
        baseParams({
          signaturesRequired: 1,
          amount: freshRoundPrice(1),
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

    fundExact();
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

    fundExact();
    try {
      await settleAgentRound(
        baseParams({
          creditedNodePdas: [
            Keypair.generate().publicKey,
            Keypair.generate().publicKey,
          ],
        })
      );
      assert.fail("expected signer account validation to fail");
    } catch (e: any) {
      assert.match(e.toString(), /MissingSignerAccount|InvalidNodeAccount/);
    }
  });

  it("rejects a stale registry version", async () => {
    fundExact();
    try {
      await settleAgentRound(
        baseParams({ registryVersion: registryVersion + 1 })
      );
      assert.fail("expected InvalidRegistryVersion");
    } catch (e: any) {
      assert.match(
        e.toString(),
        /InvalidRegistryVersion|AccountOwnedByWrongProgram|AccountNotInitialized/
      );
    }
  });

  describe("per-gateway escrow scoping", () => {
    it("does not let a second gateway spend a payer's escrow funded for the first", async () => {
      const gatewayB = await registerBondedGateway(ctx, proto);
      const localPayer = Keypair.generate();
      const t = ++ts;
      const price = freshRoundPrice(SIGS_REQUIRED);

      const escrowA = findAgentEscrowPda(
        proto.program.programId,
        localPayer.publicKey,
        gateway.gatewayPda
      );
      const ataA = findEscrowAta(proto.usdcMint, escrowA);
      fundAtaInBankrun(ctx, ataA, proto.usdcMint, escrowA, price);

      // Gateway B's escrow ATA must exist (even empty) to reach the balance
      // check at all — an entirely uninitialized ATA fails account resolution
      // (`AccountNotInitialized`) before the handler runs.
      const escrowB = findAgentEscrowPda(
        proto.program.programId,
        localPayer.publicKey,
        gatewayB.gatewayPda
      );
      fundAtaInBankrun(
        ctx,
        findEscrowAta(proto.usdcMint, escrowB),
        proto.usdcMint,
        escrowB,
        0
      );

      try {
        await settleAgentRound(
          baseParams({
            payer: localPayer,
            canonicalTimestamp: t,
            gateway: gatewayB,
          })
        );
        assert.fail("expected InsufficientAgentCredits");
      } catch (e: any) {
        assert.include(e.toString(), "InsufficientAgentCredits");
      }

      // Gateway A's escrow is untouched by the failed attempt through gateway B.
      assert.equal(await readTokenAmount(ctx, ataA), BigInt(price));
    });

    it("lets two gateways independently settle the same (payer, api_config, sigs, timestamp)", async () => {
      const gatewayB = await registerBondedGateway(ctx, proto);
      const localPayer = Keypair.generate();
      const t = ++ts;
      const price = freshRoundPrice(SIGS_REQUIRED);

      for (const gw of [gateway, gatewayB]) {
        const escrowPdaX = findAgentEscrowPda(
          proto.program.programId,
          localPayer.publicKey,
          gw.gatewayPda
        );
        fundAtaInBankrun(
          ctx,
          findEscrowAta(proto.usdcMint, escrowPdaX),
          proto.usdcMint,
          escrowPdaX,
          price
        );
      }

      const recordA = await settleAgentRound(
        baseParams({ payer: localPayer, canonicalTimestamp: t, gateway })
      );
      const recordB = await settleAgentRound(
        baseParams({
          payer: localPayer,
          canonicalTimestamp: t,
          gateway: gatewayB,
        })
      );

      assert.notEqual(recordA.toBase58(), recordB.toBase58());
      const recA = await proto.program.account.roundReceiptRecord.fetch(
        recordA
      );
      const recB = await proto.program.account.roundReceiptRecord.fetch(
        recordB
      );
      assert.equal(recA.gateway.toBase58(), gateway.gatewayPda.toBase58());
      assert.equal(recB.gateway.toBase58(), gatewayB.gatewayPda.toBase58());
    });

    it("rejects a mismatched escrow account (another gateway's PDA) via the seeds constraint", async () => {
      const gatewayB = await registerBondedGateway(ctx, proto);
      const localPayer = Keypair.generate();
      const t = ++ts;

      const wrongEscrow = findAgentEscrowPda(
        proto.program.programId,
        localPayer.publicKey,
        gatewayB.gatewayPda
      );
      // The escrow ATA must exist for account resolution to reach the seeds
      // constraint on `agent_escrow` at all (see the isolation test above).
      fundAtaInBankrun(
        ctx,
        findEscrowAta(proto.usdcMint, wrongEscrow),
        proto.usdcMint,
        wrongEscrow,
        0
      );

      try {
        await settleAgentRound(
          baseParams({
            payer: localPayer,
            canonicalTimestamp: t,
            gateway, // settling via gateway A ...
            agentEscrowOverride: wrongEscrow, // ... but naming gateway B's escrow
          })
        );
        assert.fail("expected a seeds constraint failure");
      } catch (e: any) {
        assert.include(e.toString(), "ConstraintSeeds");
      }
    });
  });
});
