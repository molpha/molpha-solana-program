/**
 * Shared harness for the unified settlement tests (subscription draw + x402 gateway-float).
 *
 * Wraps protocol initialization, gateway bonding, gateway-float funding, and
 * `settle_x402_round` / `settle_subscription_round` transaction construction. Owner-provisioned
 * agents are `Delegate` accounts settled through `settle_subscription_round` and are covered by
 * the delegate test suites.
 */
import { BN, Idl, Program } from "@coral-xyz/anchor";
import { Keypair, PublicKey, SystemProgram } from "@solana/web3.js";
import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  TOKEN_PROGRAM_ID,
  getAssociatedTokenAddressSync,
} from "@solana/spl-token";
import { Clock } from "solana-bankrun";
import * as crypto from "crypto";
import { keccak_256 } from "@noble/hashes/sha3";
import { TestContext } from "../setup";
import {
  fundNodeOwnerInBankrun,
  getProtocolTreasury,
  findNodePda,
  findRegistryPda,
  registeredNodePda,
} from "./submit-attestation";
import molphaIdl from "../../target/idl/molpha.json";

/** Prefer the built IDL address (matches `declare_id!` / Anchor.toml localnet). */
export const LOCALNET_PROGRAM_ID = new PublicKey(
  (molphaIdl as { address: string }).address
);

export const PROTOCOL_CONFIG_SEED = Buffer.from("molpha_config");
export const REGISTRY_STATE_SEED = Buffer.from("molpha_registry");
export const GATEWAY_SEED = Buffer.from("molpha_gateway");
export const X402_ROUND_SEED = Buffer.from("molpha_x402_round");
export const ROUND_SEED = Buffer.from("molpha_round");

// Mirrors the `initialize` args used across the integration suite.
export const INIT = {
  rewardPerSignature: 10,
  gatewayFee: 20,
  disputeWindowSlots: 256,
  gatewayBondMin: 500_000_000,
  redundancyBuffer: 2,
  challengerBountyBps: 500,
  x402RoundBase: 1_000,
  x402RoundPerSig: 15,
};

export function x402Price(signaturesRequired: number): number {
  return INIT.x402RoundBase + signaturesRequired * INIT.x402RoundPerSig;
}

export function u64le(value: number | bigint): Buffer {
  const buf = Buffer.alloc(8);
  buf.writeBigUInt64LE(BigInt(value));
  return buf;
}

export function u8(value: number): Buffer {
  return Buffer.from([value & 0xff]);
}

export function sha256(parts: Buffer[]): Buffer {
  const h = crypto.createHash("sha256");
  for (const part of parts) h.update(part);
  return h.digest();
}

/**
 * `H(RequestAuth)` — mirrors `hash_request_auth_fields` in Rust.
 * Borsh body order: source_id, signatures_required, timestamp.
 */
export function hashRequestAuthFields(
  sourceId: Uint8Array | Buffer,
  signaturesRequired: number,
  timestamp: number | bigint
): Buffer {
  const body = Buffer.concat([
    Buffer.from(sourceId),
    u8(signaturesRequired),
    u64le(timestamp),
  ]);
  return sha256([Buffer.from("MOLPHA_REQAUTH_V1", "utf8"), body]);
}

/**
 * Rail-scoped payer identity — mirrors the spec's
 * `payer_id = keccak256(rail_id_be ‖ payer_address_bytes)`.
 * `railId` is a u16 registry id (0 = none/subscription, 1 = solana, 2 = base).
 */
export function derivePayerId(
  railId: number,
  addressBytes: Uint8Array | Buffer
): Uint8Array {
  const railIdBe = Buffer.alloc(2);
  railIdBe.writeUInt16BE(railId, 0);
  return keccak_256(Buffer.concat([railIdBe, Buffer.from(addressBytes)]));
}

/** Gateway-owned USDC ATA that holds the slashable bond. */
export function findGatewayBondAta(
  usdcMint: PublicKey,
  gatewayPda: PublicKey
): PublicKey {
  return getAssociatedTokenAddressSync(usdcMint, gatewayPda, true);
}

/**
 * Directly write a funded USDC token account at `ata` owned by `owner` (may be a PDA).
 * Mirrors `fundNodeOwnerInBankrun`, but for an arbitrary ATA — used to fund a gateway's
 * operating float, or any other ad hoc account, without a program call.
 */
export function fundAtaInBankrun(
  ctx: TestContext,
  ata: PublicKey,
  usdcMint: PublicKey,
  owner: PublicKey,
  amount: number | bigint
): void {
  const tokenData = Buffer.alloc(165);
  usdcMint.toBuffer().copy(tokenData, 0);
  owner.toBuffer().copy(tokenData, 32);
  tokenData.writeBigUInt64LE(BigInt(amount), 64);
  tokenData.writeUInt32LE(0, 72);
  tokenData.writeUInt8(1, 108);
  ctx.provider.context.setAccount(ata, {
    lamports: 2_039_280,
    owner: TOKEN_PROGRAM_ID,
    executable: false,
    data: tokenData,
  });
}

/** Read an SPL token account's `amount` (bytes 64..72) from bankrun. */
export async function readTokenAmount(
  ctx: TestContext,
  ata: PublicKey
): Promise<bigint> {
  const acct = await ctx.provider.context.banksClient.getAccount(ata);
  if (!acct) return 0n;
  return Buffer.from(acct.data).readBigUInt64LE(64);
}

/**
 * x402 round-record PDA:
 * `["molpha_x402_round", payer_id, gateway, source_id, signatures_required, canonical_timestamp_le]`
 */
export function findX402RoundPda(
  programId: PublicKey,
  payerId: Uint8Array | Buffer,
  gateway: PublicKey,
  sourceId: Uint8Array | Buffer,
  signaturesRequired: number,
  canonicalTimestamp: number | bigint
): PublicKey {
  return PublicKey.findProgramAddressSync(
    [
      X402_ROUND_SEED,
      Buffer.from(payerId),
      gateway.toBuffer(),
      Buffer.from(sourceId),
      u8(signaturesRequired),
      u64le(canonicalTimestamp),
    ],
    programId
  )[0];
}

/**
 * Subscription / delegate round-record PDA:
 * `["molpha_round", subscription.owner, source_id, signatures_required, timestamp_le]`
 */
export function findSubscriptionRoundPda(
  programId: PublicKey,
  subscriptionOwner: PublicKey,
  sourceId: Uint8Array | Buffer,
  signaturesRequired: number,
  timestamp: number | bigint
): PublicKey {
  return PublicKey.findProgramAddressSync(
    [
      ROUND_SEED,
      subscriptionOwner.toBuffer(),
      Buffer.from(sourceId),
      u8(signaturesRequired),
      u64le(timestamp),
    ],
    programId
  )[0];
}

export function findGatewayPda(
  programId: PublicKey,
  authority: PublicKey
): PublicKey {
  return PublicKey.findProgramAddressSync(
    [GATEWAY_SEED, authority.toBuffer()],
    programId
  )[0];
}

export async function advanceClock(
  ctx: TestContext,
  seconds: number
): Promise<void> {
  const currentClock = await ctx.provider.context.banksClient.getClock();
  const newClock = new Clock(
    currentClock.slot + BigInt(seconds),
    currentClock.epochStartTimestamp,
    currentClock.epoch,
    currentClock.leaderScheduleEpoch,
    currentClock.unixTimestamp + BigInt(seconds)
  );
  ctx.provider.context.setClock(newClock);
}

export async function currentUnixTs(ctx: TestContext): Promise<number> {
  const clock = await ctx.provider.context.banksClient.getClock();
  return Number(clock.unixTimestamp);
}

export interface AgentProtocolContext {
  program: any;
  protocolConfigPda: PublicKey;
  registryStatePda: PublicKey;
  treasury: PublicKey;
  usdcMint: PublicKey;
}

export async function initializeProtocolForAgents(
  ctx: TestContext
): Promise<AgentProtocolContext> {
  const program = new Program(
    { ...molphaIdl, address: LOCALNET_PROGRAM_ID.toBase58() } as Idl,
    ctx.provider
  ) as any;

  const [protocolConfigPda] = PublicKey.findProgramAddressSync(
    [PROTOCOL_CONFIG_SEED],
    program.programId
  );
  const [registryStatePda] = PublicKey.findProgramAddressSync(
    [REGISTRY_STATE_SEED],
    program.programId
  );
  const treasury = getProtocolTreasury(
    protocolConfigPda,
    ctx.underlyingTokenMint
  );
  const registry = findRegistryPda(program.programId, 0);

  await program.methods
    .initialize({
      withdrawalCooldownSecs: new BN(5),
      maxNodeCount: 8,
      redundancyBuffer: INIT.redundancyBuffer,
      minimumNodeDeposit: new BN(100_000_000),
      rewardPerSignature: new BN(INIT.rewardPerSignature),
      gatewayFee: new BN(INIT.gatewayFee),
      receiptSubmitterBonus: new BN(5),
      protocolFeeBps: 100,
      maxReceiptDelaySlots: new BN(256),
      protocolFee: new BN(0),
      disputeWindowSlots: new BN(INIT.disputeWindowSlots),
      gatewayBondMin: new BN(INIT.gatewayBondMin),
      challengerBountyBps: INIT.challengerBountyBps,
      usdcMint: ctx.underlyingTokenMint,
      x402RoundBase: new BN(INIT.x402RoundBase),
      x402RoundPerSig: new BN(INIT.x402RoundPerSig),
    })
    .accountsPartial({
      protocolConfig: protocolConfigPda,
      registryState: registryStatePda,
      registry,
      usdcMint: ctx.underlyingTokenMint,
      treasury,
      authority: ctx.authority.publicKey,
      tokenProgram: TOKEN_PROGRAM_ID,
      associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
    })
    .rpc();

  return {
    program,
    protocolConfigPda,
    registryStatePda,
    treasury,
    usdcMint: ctx.underlyingTokenMint,
  };
}

export interface BondedGateway {
  gatewayAuthority: Keypair;
  gatewayPda: PublicKey;
  /** The gateway authority's own USDC ATA — doubles as its x402 operating float. */
  authorityUsdc: PublicKey;
  gatewayBondAta: PublicKey;
}

export async function registerBondedGateway(
  ctx: TestContext,
  proto: AgentProtocolContext
): Promise<BondedGateway> {
  const gatewayAuthority = Keypair.generate();
  const authorityUsdc = await fundNodeOwnerInBankrun(
    ctx,
    gatewayAuthority,
    ctx.underlyingTokenMint,
    BigInt(10_000_000_000)
  );
  // The gateway pays rent for every settled round record. Keep enough SOL available
  // for suites that intentionally create many successful and failing settlements.
  ctx.provider.context.setAccount(gatewayAuthority.publicKey, {
    lamports: 1_000_000_000,
    owner: SystemProgram.programId,
    executable: false,
    data: Buffer.alloc(0),
  });
  const gatewayPda = findGatewayPda(
    proto.program.programId,
    gatewayAuthority.publicKey
  );
  const gatewayBondAta = findGatewayBondAta(
    ctx.underlyingTokenMint,
    gatewayPda
  );

  await proto.program.methods
    .registerGateway({
      ip: [10, 0, 0, 1],
      port: 8080,
      bondAmount: new BN(INIT.gatewayBondMin),
    })
    .accountsPartial({
      gatewayAuthority: gatewayAuthority.publicKey,
      protocolConfig: proto.protocolConfigPda,
      gateway: gatewayPda,
      usdcMint: ctx.underlyingTokenMint,
      authorityUsdc,
      gatewayBond: gatewayBondAta,
      systemProgram: SystemProgram.programId,
      tokenProgram: TOKEN_PROGRAM_ID,
      associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
    })
    .signers([gatewayAuthority])
    .rpc();

  return { gatewayAuthority, gatewayPda, authorityUsdc, gatewayBondAta };
}

/** Top up a gateway's own USDC ATA (its x402 float) directly, no program call. */
export function fundGatewayFloat(
  ctx: TestContext,
  proto: AgentProtocolContext,
  gateway: BondedGateway,
  amount: number | bigint
): void {
  fundAtaInBankrun(
    ctx,
    gateway.authorityUsdc,
    proto.usdcMint,
    gateway.gatewayAuthority.publicKey,
    amount
  );
}

/** Bitmap (EVM u256 layout) with the given bit positions set. */
export function bitmapWithBits(bits: number[]): Uint8Array {
  const bitmap = new Uint8Array(32);
  for (const bit of bits) {
    bitmap[31 - (bit >> 3)] |= 1 << (bit & 7);
  }
  return bitmap;
}

export function zeroedAggSig(signersBitmap?: Uint8Array) {
  return {
    s: Array.from(new Uint8Array(32)),
    rx: Array.from(new Uint8Array(32)),
    ryParity: 0,
    commitmentAddr: Array.from(new Uint8Array(20)),
    signersBitmap: Array.from(signersBitmap ?? new Uint8Array(32)),
  };
}

export interface SettleX402RoundParams {
  proto: AgentProtocolContext;
  gateway: BondedGateway;
  /** Rail-scoped payer identity — 32 raw bytes, see `derivePayerId`. */
  payerId: Uint8Array | Buffer;
  amount: number;
  sourceId: Uint8Array;
  canonicalTimestamp: number;
  signaturesRequired: number;
  registryVersion: number;
  aggSig?: ReturnType<typeof zeroedAggSig>;
  value?: Uint8Array;
  paymentRail?: number;
  paymentRef?: Uint8Array | Buffer;
  /** Node PDAs for the credited bitmap, ascending bit order. */
  creditedNodePdas?: PublicKey[];
  /** Force a specific `gateway_usdc` account (e.g. the bond ATA) to exercise the
   *  owner constraint in negative tests. Defaults to the gateway authority's own ATA. */
  gatewayUsdcOverride?: PublicKey;
}

export async function settleX402Round(
  params: SettleX402RoundParams
): Promise<PublicKey> {
  const programId = params.proto.program.programId;
  const payerId = Buffer.from(params.payerId);

  const roundRecordPda = findX402RoundPda(
    programId,
    payerId,
    params.gateway.gatewayPda,
    params.sourceId,
    params.signaturesRequired,
    params.canonicalTimestamp
  );

  await params.proto.program.methods
    .settleX402Round({
      payerId: Array.from(payerId),
      amount: new BN(params.amount),
      sourceId: Array.from(params.sourceId),
      canonicalTimestamp: new BN(params.canonicalTimestamp),
      signaturesRequired: params.signaturesRequired,
      registryVersion: params.registryVersion,
      aggSig: params.aggSig ?? zeroedAggSig(),
      value: Array.from(params.value ?? new Uint8Array(32)),
      paymentRail: params.paymentRail ?? 1,
      paymentRef: Buffer.from(params.paymentRef ?? new Uint8Array(0)),
    })
    .accountsPartial({
      gatewayAuthority: params.gateway.gatewayAuthority.publicKey,
      protocolConfig: params.proto.protocolConfigPda,
      registryState: params.proto.registryStatePda,
      registry: findRegistryPda(programId, params.registryVersion),
      gateway: params.gateway.gatewayPda,
      usdcMint: params.proto.usdcMint,
      gatewayUsdc: params.gatewayUsdcOverride ?? params.gateway.authorityUsdc,
      treasury: params.proto.treasury,
      tokenProgram: TOKEN_PROGRAM_ID,
      roundRecord: roundRecordPda,
      systemProgram: SystemProgram.programId,
    })
    .remainingAccounts(
      (params.creditedNodePdas ?? []).map((pubkey) => ({
        pubkey,
        isWritable: false,
        isSigner: false,
      }))
    )
    .signers([params.gateway.gatewayAuthority])
    .rpc();

  return roundRecordPda;
}

export function creditedNodePdasForBits(
  programId: PublicKey,
  bits: number[],
  owners?: PublicKey[]
): PublicKey[] {
  return [...bits]
    .sort((a, b) => a - b)
    .map((bit) =>
      owners?.[bit]
        ? findNodePda(programId, owners[bit])
        : registeredNodePda(programId, bit)
    );
}

export { findNodePda };
