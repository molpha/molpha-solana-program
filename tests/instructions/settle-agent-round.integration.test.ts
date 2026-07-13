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
  AgentEscrowSetup,
  BondedGateway,
  bitmapWithBits,
  createFundedEscrow,
  creditedNodePdasForBits,
  currentUnixTs,
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
const FEED_ID = Uint8Array.from(Buffer.alloc(32, 0x42));

describe("settle_agent_round (UsdcEscrow rail)", () => {
  let ctx: TestContext;
  let proto: AgentProtocolContext;
  let gateway: BondedGateway;
  let escrow: AgentEscrowSetup;
  let registryVersion: number;
  let ts = 1_700_000_000;
  let nonce = 0;

  const nextRequest = () => ({ nonce: ++nonce, ts: ++ts });

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
    escrow = await createFundedEscrow(ctx, proto, {
      maxSigners: 5,
      depositAmount: 50_000,
    });
  }, 180_000);

  const baseParams = (
    over: Partial<Parameters<typeof settleAgentRound>[0]> = {}
  ) => {
    const { nonce: n, ts: t } = nextRequest();
    return {
      proto,
      gateway,
      escrow,
      requestNonce: n,
      maxPrice: freshRoundPrice(SIGS_REQUIRED),
      feedId: FEED_ID,
      timestamp: t,
      signaturesRequired: SIGS_REQUIRED,
      registryVersion,
      aggSig: zeroedAggSig(bitmapWithBits([0, 1, 2])),
      creditedNodePdas: creditedNodePdasForBits(
        proto.program.programId,
        [0, 1, 2]
      ),
      ...over,
    };
  };

  it("settles a FreshRound: debits escrow, credits nodes + gateway, writes the record", async () => {
    const price = freshRoundPrice(SIGS_REQUIRED);
    const escrowBefore = await proto.program.account.agentEscrow.fetch(
      escrow.escrowPda
    );
    const escrowAtaBefore = await readTokenAmount(ctx, escrow.escrowAta);
    const treasuryBefore = await readTokenAmount(ctx, proto.treasury);
    const gatewayBefore = await proto.program.account.gateway.fetch(
      gateway.gatewayPda
    );
    const nodeBefore = await proto.program.account.node.fetch(
      findNodePda(proto.program.programId, 0)
    );

    const params = baseParams();
    await settleAgentRound(params);

    const escrowAfter = await proto.program.account.agentEscrow.fetch(
      escrow.escrowPda
    );
    // Real USDC moved escrow ATA -> treasury.
    const escrowAtaAfter = await readTokenAmount(ctx, escrow.escrowAta);
    const treasuryAfter = await readTokenAmount(ctx, proto.treasury);
    assert.equal(escrowAtaBefore - escrowAtaAfter, BigInt(price));
    assert.equal(treasuryAfter - treasuryBefore, BigInt(price));
    assert.equal(
      escrowAfter.spentTotal.toNumber() - escrowBefore.spentTotal.toNumber(),
      price
    );
    assert.equal(escrowAfter.lastRequestNonce.toNumber(), params.requestNonce);

    const nodeAfter = await proto.program.account.node.fetch(
      findNodePda(proto.program.programId, 0)
    );
    assert.equal(
      nodeAfter.pendingRewards.toNumber() -
        nodeBefore.pendingRewards.toNumber(),
      INIT.rewardPerSignature
    );

    const gatewayAfter = await proto.program.account.gateway.fetch(
      gateway.gatewayPda
    );
    assert.equal(
      gatewayAfter.pendingRewards.toNumber() -
        gatewayBefore.pendingRewards.toNumber(),
      INIT.gatewayFee
    );

    const record = await proto.program.account.roundReceiptRecord.fetch(
      findAgentRoundPda(
        proto.program.programId,
        escrow.escrowPda,
        params.requestNonce
      )
    );
    assert.equal(record.kind, 1); // RECORD_KIND_AGENT_FRESH
    assert.equal(record.price.toNumber(), price);
    assert.equal(record.agent.toBase58(), escrow.escrowPda.toBase58());
    assert.equal(record.requestNonce.toNumber(), params.requestNonce);
    assert.isFalse(record.finalized);
    assert.isFalse(record.voided);
    assert.equal(record.gateway.toBase58(), gateway.gatewayPda.toBase58());
  });

  it("rejects replayed and non-increasing nonces", async () => {
    const params = baseParams();
    await settleAgentRound(params);

    // Same nonce again: the record PDA already exists.
    try {
      await settleAgentRound({ ...params, timestamp: params.timestamp + 1 });
      assert.fail("expected duplicate record PDA failure");
    } catch (e: any) {
      assert.ok(e);
    }

    // Lower, unused nonce: rejected by the strictly-increasing guard. Zero is
    // intentionally unused; earlier positive nonces already have record PDAs.
    try {
      await settleAgentRound(baseParams({ requestNonce: 0 }));
      assert.fail("expected AgentNonceNotIncreasing");
    } catch (e: any) {
      assert.include(e.toString(), "AgentNonceNotIncreasing");
    }
  });

  it("rejects a bad or missing agent signature", async () => {
    try {
      await settleAgentRound(baseParams({ ed25519Signer: Keypair.generate() }));
      assert.fail("expected AgentSigInvalid");
    } catch (e: any) {
      assert.include(e.toString(), "AgentSigInvalid");
    }
    try {
      await settleAgentRound(baseParams({ ed25519Signer: null }));
      assert.fail("expected AgentSigInvalid");
    } catch (e: any) {
      assert.include(e.toString(), "AgentSigInvalid");
    }
  });

  it("enforces the agent-signed max_price and the owner max_signers", async () => {
    try {
      await settleAgentRound(
        baseParams({ maxPrice: freshRoundPrice(SIGS_REQUIRED) - 1 })
      );
      assert.fail("expected MaxPriceExceeded");
    } catch (e: any) {
      assert.include(e.toString(), "MaxPriceExceeded");
    }

    const cappedEscrow = await createFundedEscrow(ctx, proto, {
      maxSigners: SIGS_REQUIRED - 1,
      depositAmount: 50_000,
    });
    try {
      await settleAgentRound(baseParams({ escrow: cappedEscrow }));
      assert.fail("expected SignaturesExceedAgentMax");
    } catch (e: any) {
      assert.include(e.toString(), "SignaturesExceedAgentMax");
    }
  });

  it("tracks epoch spend without capping and lazily rolls the epoch window", async () => {
    const price = freshRoundPrice(SIGS_REQUIRED);
    const epochEscrow = await createFundedEscrow(ctx, proto, {
      maxSigners: 5,
      depositAmount: 50_000,
    });

    let n = 100;
    await settleAgentRound(
      baseParams({ escrow: epochEscrow, requestNonce: ++n })
    );
    await settleAgentRound(
      baseParams({ escrow: epochEscrow, requestNonce: ++n })
    );
    let after = await proto.program.account.agentEscrow.fetch(
      epochEscrow.escrowPda
    );
    assert.equal(after.epochSpent.toNumber(), price * 2);

    // Cross the epoch boundary: spend window resets lazily at settlement.
    await advanceClock(ctx, INIT.agentEpochLenSecs + 1);
    await settleAgentRound(
      baseParams({ escrow: epochEscrow, requestNonce: ++n })
    );
    after = await proto.program.account.agentEscrow.fetch(
      epochEscrow.escrowPda
    );
    assert.equal(after.epochSpent.toNumber(), price);
  });

  it("rejects settlement against paused, frozen, and expired escrows", async () => {
    // Paused.
    await proto.program.methods
      .setAgentPaused(true)
      .accountsPartial({
        owner: escrow.owner.publicKey,
        agentEscrow: escrow.escrowPda,
      })
      .signers([escrow.owner])
      .rpc();
    try {
      await settleAgentRound(baseParams());
      assert.fail("expected AgentNotActive");
    } catch (e: any) {
      assert.include(e.toString(), "AgentNotActive");
    }
    await proto.program.methods
      .setAgentPaused(false)
      .accountsPartial({
        owner: escrow.owner.publicKey,
        agentEscrow: escrow.escrowPda,
      })
      .signers([escrow.owner])
      .rpc();

    // Expired.
    const now = await currentUnixTs(ctx);
    const expiring = await createFundedEscrow(ctx, proto, {
      expiresAt: now + 5,
      depositAmount: 50_000,
    });
    await advanceClock(ctx, 6);
    try {
      await settleAgentRound(
        baseParams({ escrow: expiring, requestNonce: 200 })
      );
      assert.fail("expected AgentExpired");
    } catch (e: any) {
      assert.include(e.toString(), "AgentExpired");
    }
  });

  it("rejects insufficient escrow balance", async () => {
    const poor = await createFundedEscrow(ctx, proto, {
      depositAmount: 500, // below the 1030 FreshRound price
    });
    try {
      await settleAgentRound(
        baseParams({ escrow: poor, requestNonce: 300 })
      );
      assert.fail("expected InsufficientAgentCredits");
    } catch (e: any) {
      assert.include(e.toString(), "InsufficientAgentCredits");
    }
  });

  it("gateway cannot lazily create a privileged owner-provisioned escrow (owner != authority)", async () => {
    // Fund an ATA at PDA[owner, authority] with owner != authority but with NO data account,
    // then try to settle: lazy-init fires (owner == default) and must reject because only
    // x402 escrows (owner == authority) may be created at settle.
    const owner = Keypair.generate().publicKey;
    const authority = Keypair.generate().publicKey;
    const escrowPda = findAgentEscrowPda(
      proto.program.programId,
      owner,
      authority
    );
    fundAtaInBankrun(
      ctx,
      findEscrowAta(proto.usdcMint, escrowPda),
      proto.usdcMint,
      escrowPda,
      1_000_000
    );
    try {
      await settleAgentRound(
        baseParams({
          ownerAuthorityOverride: { owner, authority },
          ed25519Signer: null,
          requestNonce: 400,
        })
      );
      assert.fail("expected InvalidAgentCaps");
    } catch (e: any) {
      assert.include(e.toString(), "InvalidAgentCaps");
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
          aggSig: zeroedAggSig(bitmapWithBits([0, 1])),
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

  it("rejects a stale registry version for FreshRound", async () => {
    try {
      await settleAgentRound(
        baseParams({ registryVersion: registryVersion + 1 })
      );
      assert.fail("expected InvalidRegistryVersion");
    } catch (e: any) {
      assert.include(e.toString(), "InvalidRegistryVersion");
    }
  });

  it("rejects a non-gateway signer", async () => {
    const impostor = Keypair.generate();
    ctx.provider.context.setAccount(impostor.publicKey, {
      lamports: 1_000_000_000,
      owner: new PublicKey("11111111111111111111111111111111"),
      executable: false,
      data: Buffer.alloc(0),
    });
    try {
      await settleAgentRound(
        baseParams({
          gateway: {
            ...gateway,
            gatewayAuthority: impostor,
          },
        })
      );
      assert.fail("expected gateway constraint failure");
    } catch (e: any) {
      // Seeds are derived from the signer, so the supplied gateway PDA mismatches.
      assert.ok(e);
    }
  });
});
