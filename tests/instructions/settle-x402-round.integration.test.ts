import { assert } from "chai";
import { Keypair, SystemProgram } from "@solana/web3.js";
import { setupTestContext, TestContext } from "../setup";
import {
  generateE2eSignerMaterial,
  registerNodesForE2e,
} from "../helpers/submit-attestation";
import {
  advanceClock,
  AgentProtocolContext,
  BondedGateway,
  bitmapWithBits,
  creditedNodePdasForBits,
  derivePayerId,
  findGatewayPda,
  findX402RoundPda,
  fundAtaInBankrun,
  fundGatewayFloat,
  initializeProtocolForAgents,
  readTokenAmount,
  registerBondedGateway,
  settleX402Round,
  x402Price,
  zeroedAggSig,
} from "../helpers/settlement-access";

const NODE_COUNT = 4;
const SIGS_REQUIRED = 2;
const API_SOURCE_ID = Uint8Array.from(Buffer.alloc(32, 0x24));

/**
 * Unified x402 settlement: a bonded gateway pays the protocol-quoted price from its own USDC
 * "float" (any token account it controls) into the treasury, and records how it was paid
 * (`payment_rail`, `payment_ref`) as attested metadata the program stores and never verifies.
 * Bond and float are structurally separate — the bond ATA's SPL authority is the Gateway PDA,
 * not the gateway authority, so it can never be named as the float. Replay is guarded by the
 * round-record PDA keyed by `(payer_id, gateway, source_id, signatures_required,
 * canonical_timestamp)` — unlike escrow, `payer_id` is opaque rail-scoped metadata, not an
 * on-chain account, so two payer_ids for an identical round each get their own receipt and the
 * gateway pays (and the reward pool accrues) once per payer for one signing ceremony. This is
 * pre-existing shipped behaviour on the subscription path too (spec §11.3), unchanged here.
 */
describe("settle_x402_round (gateway-funded, all payment rails)", () => {
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

  const newPayerId = () =>
    derivePayerId(1, Keypair.generate().publicKey.toBuffer());

  const baseParams = (
    payerId: Uint8Array,
    over: Record<string, any> = {}
  ) => ({
    proto,
    gateway,
    payerId,
    amount: x402Price(SIGS_REQUIRED),
    sourceId: API_SOURCE_ID,
    canonicalTimestamp: ++ts,
    signaturesRequired: SIGS_REQUIRED,
    registryVersion,
    aggSig: zeroedAggSig(bitmapWithBits([0, 1])),
    creditedNodePdas: creditedNodePdasForBits(proto.program.programId, [0, 1]),
    ...over,
  });

  it("settles from the gateway float: debits float, credits treasury, leaves bond and rewards untouched", async () => {
    const price = x402Price(SIGS_REQUIRED);
    const floatBefore = await readTokenAmount(ctx, gateway.authorityUsdc);
    const treasuryBefore = await readTokenAmount(ctx, proto.treasury);
    const bondBefore = await readTokenAmount(ctx, gateway.gatewayBondAta);
    const gwBefore = await proto.program.account.gateway.fetch(
      gateway.gatewayPda
    );

    const payerId = newPayerId();
    const params = baseParams(payerId);
    const recordPda = await settleX402Round(params);

    const floatAfter = await readTokenAmount(ctx, gateway.authorityUsdc);
    const treasuryAfter = await readTokenAmount(ctx, proto.treasury);
    const bondAfter = await readTokenAmount(ctx, gateway.gatewayBondAta);
    assert.equal(floatBefore - floatAfter, BigInt(price));
    assert.equal(treasuryAfter - treasuryBefore, BigInt(price));
    assert.equal(bondAfter, bondBefore);

    // Crediting happens at finalize, not at settle.
    const gwAfter = await proto.program.account.gateway.fetch(
      gateway.gatewayPda
    );
    assert.equal(
      gwAfter.claimableRewards.toNumber(),
      gwBefore.claimableRewards.toNumber()
    );

    assert.equal(
      recordPda.toBase58(),
      findX402RoundPda(
        proto.program.programId,
        payerId,
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
    assert.deepEqual(record.kind, { x402: {} });
    assert.deepEqual(Array.from(record.sourceId), Array.from(API_SOURCE_ID));
  });

  it("round-trips rail metadata: payment_rail, payment_ref, payer_id", async () => {
    const payerId = newPayerId();
    const paymentRef = Buffer.from("0123456789abcdef01234567", "utf8"); // 25 bytes
    const recordPda = await settleX402Round(
      baseParams(payerId, { paymentRail: 2, paymentRef })
    );
    const record = await proto.program.account.roundReceiptRecord.fetch(
      recordPda
    );
    assert.equal(record.paymentRail, 2);
    assert.equal(record.paymentRefLen, paymentRef.length);
    assert.deepEqual(
      Array.from(record.paymentRef).slice(0, paymentRef.length),
      Array.from(paymentRef)
    );
    assert.deepEqual(
      Array.from(record.paymentRef).slice(paymentRef.length),
      Array.from(new Uint8Array(64 - paymentRef.length))
    );
    assert.deepEqual(Array.from(record.payerId), Array.from(payerId));
  });

  it("accepts payment_ref at the 0- and 64-byte boundaries", async () => {
    await settleX402Round(
      baseParams(newPayerId(), { paymentRef: new Uint8Array(0) })
    );
    await settleX402Round(
      baseParams(newPayerId(), { paymentRef: new Uint8Array(64).fill(7) })
    );
  });

  it("rejects a payment_ref over 64 bytes", async () => {
    try {
      await settleX402Round(
        baseParams(newPayerId(), { paymentRef: new Uint8Array(65) })
      );
      assert.fail("expected PaymentRefTooLong");
    } catch (e: any) {
      assert.include(e.toString(), "PaymentRefTooLong");
    }
  });

  it("blocks replay of the same (payer_id, gateway, source_id, sigs, timestamp) via the record PDA", async () => {
    const params = baseParams(newPayerId());
    await settleX402Round(params);
    try {
      await settleX402Round(params);
      assert.fail("expected duplicate record PDA failure");
    } catch (e: any) {
      assert.ok(e);
    }
  });

  it("scopes records per payer_id: two payers of the same round each get a receipt and pay in full", async () => {
    const t = ++ts;
    const payerA = newPayerId();
    const payerB = newPayerId();
    const price = x402Price(SIGS_REQUIRED);
    const floatBefore = await readTokenAmount(ctx, gateway.authorityUsdc);

    const recordA = await settleX402Round(
      baseParams(payerA, { canonicalTimestamp: t })
    );
    const recordB = await settleX402Round(
      baseParams(payerB, { canonicalTimestamp: t })
    );

    assert.notEqual(recordA.toBase58(), recordB.toBase58());
    const floatAfter = await readTokenAmount(ctx, gateway.authorityUsdc);
    assert.equal(floatBefore - floatAfter, BigInt(price) * 2n);
  });

  it("rejects when the signed amount does not match the computed price", async () => {
    try {
      await settleX402Round(
        baseParams(newPayerId(), { amount: x402Price(SIGS_REQUIRED) - 1 })
      );
      assert.fail("expected AmountMismatch");
    } catch (e: any) {
      assert.include(e.toString(), "AmountMismatch");
    }
  });

  it("rejects settlement when the gateway float is underfunded", async () => {
    const poorGateway = await registerBondedGateway(ctx, proto);
    fundGatewayFloat(ctx, proto, poorGateway, 500);
    try {
      await settleX402Round(
        baseParams(newPayerId(), { gateway: poorGateway })
      );
      assert.fail("expected InsufficientGatewayFloat");
    } catch (e: any) {
      assert.include(e.toString(), "InsufficientGatewayFloat");
    }
  });

  it("rejects the gateway bond ATA as the float — bond and float are structurally separate", async () => {
    const bondBefore = await readTokenAmount(ctx, gateway.gatewayBondAta);
    try {
      await settleX402Round(
        baseParams(newPayerId(), {
          gatewayUsdcOverride: gateway.gatewayBondAta,
        })
      );
      assert.fail("expected InvalidOwnerUsdc");
    } catch (e: any) {
      assert.include(e.toString(), "InvalidOwnerUsdc");
    }
    assert.equal(
      await readTokenAmount(ctx, gateway.gatewayBondAta),
      bondBefore
    );
  });

  it("rejects a third party's USDC account as the float", async () => {
    const stranger = Keypair.generate();
    const strangerAta = Keypair.generate().publicKey;
    fundAtaInBankrun(
      ctx,
      strangerAta,
      proto.usdcMint,
      stranger.publicKey,
      1_000_000
    );
    try {
      await settleX402Round(
        baseParams(newPayerId(), { gatewayUsdcOverride: strangerAta })
      );
      assert.fail("expected InvalidOwnerUsdc");
    } catch (e: any) {
      assert.include(e.toString(), "InvalidOwnerUsdc");
    }
  });

  it("enforces quorum shape: threshold, inflation bound, node account list", async () => {
    try {
      await settleX402Round(
        baseParams(newPayerId(), {
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

    try {
      await settleX402Round(
        baseParams(newPayerId(), {
          signaturesRequired: 1,
          amount: x402Price(1),
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

    try {
      await settleX402Round(
        baseParams(newPayerId(), {
          // Bitmap credits 2 signers but only 1 node account is supplied.
          creditedNodePdas: creditedNodePdasForBits(proto.program.programId, [
            0,
          ]),
        })
      );
      assert.fail("expected MissingSignerAccount");
    } catch (e: any) {
      assert.include(e.toString(), "MissingSignerAccount");
    }

    try {
      await settleX402Round(
        baseParams(newPayerId(), {
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
    try {
      await settleX402Round(
        baseParams(newPayerId(), { registryVersion: registryVersion + 1 })
      );
      assert.fail("expected InvalidRegistryVersion");
    } catch (e: any) {
      assert.match(
        e.toString(),
        /InvalidRegistryVersion|AccountOwnedByWrongProgram|AccountNotInitialized/
      );
    }
  });

  it("rejects settlement through an unregistered gateway", async () => {
    const impostor = Keypair.generate();
    ctx.provider.context.setAccount(impostor.publicKey, {
      lamports: 1_000_000_000,
      owner: SystemProgram.programId,
      executable: false,
      data: Buffer.alloc(0),
    });
    const unregisteredGateway: BondedGateway = {
      gatewayAuthority: impostor,
      gatewayPda: findGatewayPda(proto.program.programId, impostor.publicKey),
      authorityUsdc: gateway.authorityUsdc,
      gatewayBondAta: gateway.gatewayBondAta,
    };
    try {
      await settleX402Round(
        baseParams(newPayerId(), {
          gateway: unregisteredGateway,
          gatewayUsdcOverride: gateway.authorityUsdc,
        })
      );
      assert.fail("expected the unregistered gateway PDA to fail to load");
    } catch (e: any) {
      assert.match(e.toString(), /AccountNotInitialized|AccountOwnedByWrongProgram/);
    }
  });

  it("rejects a settle whose signer does not match the named gateway PDA", async () => {
    const impostor = Keypair.generate();
    ctx.provider.context.setAccount(impostor.publicKey, {
      lamports: 1_000_000_000,
      owner: SystemProgram.programId,
      executable: false,
      data: Buffer.alloc(0),
    });
    try {
      await settleX402Round(
        baseParams(newPayerId(), {
          gateway: { ...gateway, gatewayAuthority: impostor },
        })
      );
      assert.fail("expected a seeds constraint failure");
    } catch (e: any) {
      assert.match(e.toString(), /ConstraintSeeds/);
    }
  });
});
