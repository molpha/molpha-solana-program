/**
 * Shared harness for the self-funded / x402 agent pay-per-request tests.
 *
 * Wraps protocol initialization, gateway bonding, escrow funding, and
 * `settle_agent_round` transaction construction. Escrows are lazily created by
 * settlement (seeded by payer/authority); funding is a direct payment into the
 * escrow ATA. Owner-provisioned agents are `Delegate` accounts settled through
 * `settle_round_receipt` and are covered by the delegate test suites.
 */
import * as anchor from "@coral-xyz/anchor";
import { BN, Idl, Program } from "@coral-xyz/anchor";
import {
  Keypair,
  PublicKey,
  SystemProgram,
  SYSVAR_INSTRUCTIONS_PUBKEY,
  Ed25519Program,
} from "@solana/web3.js";
import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  TOKEN_PROGRAM_ID,
  getAssociatedTokenAddressSync,
} from "@solana/spl-token";
import { Clock } from "solana-bankrun";
import * as crypto from "crypto";
import { TestContext } from "../setup";
import {
  fundNodeOwnerInBankrun,
  getProtocolTreasury,
  findNodePda,
  findRegistryPda,
  registeredNodePda,
} from "./submit-data-update";
import molphaIdl from "../../target/idl/molpha.json";

/** Prefer the built IDL address (matches `declare_id!` / Anchor.toml localnet). */
export const LOCALNET_PROGRAM_ID = new PublicKey(
  (molphaIdl as { address: string }).address
);

export const PROTOCOL_CONFIG_SEED = Buffer.from("molpha_config");
export const REGISTRY_STATE_SEED = Buffer.from("molpha_registry");
export const GATEWAY_SEED = Buffer.from("molpha_gateway");
export const AGENT_ESCROW_SEED = Buffer.from("molpha_agent");
export const AGENT_ROUND_SEED = Buffer.from("molpha_agent_round");
export const ROUND_SEED = Buffer.from("molpha_round");
export const DOMAIN_AGENT_REQAUTH = Buffer.from(
  "MOLPHA_AGENT_REQAUTH_V1",
  "utf8"
);

// Mirrors the `initialize` args used across the integration suite.
export const INIT = {
  rewardPerSignature: 10,
  gatewayFee: 20,
  disputeWindowSlots: 256,
  gatewayBondMin: 500_000_000,
  redundancyBuffer: 2,
  challengerBountyBps: 500,
  agentRoundBase: 1_000,
  agentRoundPerSig: 15,
  agentEpochLenSecs: 86_400,
};

export const REQUEST_KIND_FRESH = 0;

export function freshRoundPrice(signaturesRequired: number): number {
  return INIT.agentRoundBase + signaturesRequired * INIT.agentRoundPerSig;
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
 * `H(AgentRequestAuth)` — mirrors `hash_agent_request_auth_fields` in Rust.
 * Borsh body order: agent, gateway, source_id, signatures_required, canonical_timestamp, amount.
 */
export function hashAgentRequestAuthFields(
  agent: PublicKey,
  gateway: PublicKey,
  sourceId: Uint8Array | Buffer,
  signaturesRequired: number,
  canonicalTimestamp: number | bigint,
  amount: number | bigint
): Buffer {
  const body = Buffer.concat([
    agent.toBuffer(),
    gateway.toBuffer(),
    Buffer.from(sourceId),
    u8(signaturesRequired),
    u64le(canonicalTimestamp),
    u64le(amount),
  ]);
  return sha256([DOMAIN_AGENT_REQAUTH, body]);
}

/** Escrow PDA — seeds ["molpha_agent", authority, gateway]. */
export function findAgentEscrowPda(
  programId: PublicKey,
  authority: PublicKey,
  gateway: PublicKey
): PublicKey {
  return PublicKey.findProgramAddressSync(
    [AGENT_ESCROW_SEED, authority.toBuffer(), gateway.toBuffer()],
    programId
  )[0];
}

/** The escrow's own USDC ATA (real custody). The escrow PDA is off-curve. */
export function findEscrowAta(
  usdcMint: PublicKey,
  escrowPda: PublicKey
): PublicKey {
  return getAssociatedTokenAddressSync(usdcMint, escrowPda, true);
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
 * Mirrors `fundNodeOwnerInBankrun`, but for an arbitrary ATA — used to simulate an
 * x402 payment landing in the escrow ATA before settlement.
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
 * Agent round-record PDA:
 * `["molpha_agent_round", authority, gateway, source_id, signatures_required, canonical_timestamp_le]`
 */
export function findAgentRoundPda(
  programId: PublicKey,
  authority: PublicKey,
  gateway: PublicKey,
  sourceId: Uint8Array | Buffer,
  signaturesRequired: number,
  canonicalTimestamp: number | bigint
): PublicKey {
  return PublicKey.findProgramAddressSync(
    [
      AGENT_ROUND_SEED,
      authority.toBuffer(),
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
 * `["molpha_round", consumer_authority, source_id, signatures_required, timestamp_le]`
 */
export function findSubscriptionRoundPda(
  programId: PublicKey,
  consumerAuthority: PublicKey,
  sourceId: Uint8Array | Buffer,
  signaturesRequired: number,
  timestamp: number | bigint
): PublicKey {
  return PublicKey.findProgramAddressSync(
    [
      ROUND_SEED,
      consumerAuthority.toBuffer(),
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
      agentRoundBase: new BN(INIT.agentRoundBase),
      agentRoundPerSig: new BN(INIT.agentRoundPerSig),
      agentEpochLenSecs: new BN(INIT.agentEpochLenSecs),
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
  // The gateway pays rent for every agent round record. Keep enough SOL available
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

export interface X402EscrowSetup {
  /** The paying identity (escrow PDA seed). */
  payer: Keypair;
  /** The gateway this escrow is scoped to (second escrow PDA seed). */
  gatewayPda: PublicKey;
  escrowPda: PublicKey;
  escrowAta: PublicKey;
}

/**
 * Set up an x402-style funded escrow: derive PDA(["molpha_agent", payer, gateway]) and
 * write USDC directly into its ATA — no program call. The escrow data account itself is
 * lazily created by the first `settle_agent_round`. Escrows are scoped per (payer,
 * gateway) pair, so funding is not fungible across gateways — reuse the same `payer` with
 * a different `gateway` to get an isolated escrow for that counterparty.
 */
export function createX402FundedEscrow(
  ctx: TestContext,
  proto: AgentProtocolContext,
  gateway: BondedGateway | PublicKey,
  opts?: { payer?: Keypair; fundAmount?: number | bigint }
): X402EscrowSetup {
  const gatewayPda =
    gateway instanceof PublicKey ? gateway : gateway.gatewayPda;
  const payer = opts?.payer ?? Keypair.generate();
  const escrowPda = findAgentEscrowPda(
    proto.program.programId,
    payer.publicKey,
    gatewayPda
  );
  const escrowAta = findEscrowAta(ctx.underlyingTokenMint, escrowPda);

  const fundAmount = opts?.fundAmount ?? 1_000_000;
  if (BigInt(fundAmount) > 0n) {
    fundAtaInBankrun(
      ctx,
      escrowAta,
      ctx.underlyingTokenMint,
      escrowPda,
      fundAmount
    );
  }

  return { payer, gatewayPda, escrowPda, escrowAta };
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

export interface SettleAgentRoundParams {
  proto: AgentProtocolContext;
  gateway: BondedGateway;
  /** x402 payer keypair — seeds the escrow PDA as `authority` and signs AgentRequestAuth. */
  payer: Keypair;
  amount: number;
  sourceId: Uint8Array;
  canonicalTimestamp: number;
  signaturesRequired: number;
  registryVersion: number;
  aggSig?: ReturnType<typeof zeroedAggSig>;
  value?: Uint8Array;
  /** Node PDAs for the credited bitmap, ascending bit order. */
  creditedNodePdas?: PublicKey[];
  /** When false, omit the Ed25519 AgentRequestAuth pre-instruction (negative tests). */
  signAgentRequest?: boolean;
  /** Force a specific `agent_escrow` account (e.g. another gateway's escrow PDA) to
   *  exercise the seeds constraint in negative tests. Defaults to the correctly
   *  derived `(payer, gateway)` escrow. */
  agentEscrowOverride?: PublicKey;
}

export async function settleAgentRound(
  params: SettleAgentRoundParams
): Promise<PublicKey> {
  const programId = params.proto.program.programId;
  const authority = params.payer.publicKey;

  const escrowPda =
    params.agentEscrowOverride ??
    findAgentEscrowPda(programId, authority, params.gateway.gatewayPda);
  const escrowAta = findEscrowAta(params.proto.usdcMint, escrowPda);

  const roundRecordPda = findAgentRoundPda(
    programId,
    authority,
    params.gateway.gatewayPda,
    params.sourceId,
    params.signaturesRequired,
    params.canonicalTimestamp
  );

  const preInstructions = [];
  if (params.signAgentRequest !== false) {
    const authHash = hashAgentRequestAuthFields(
      escrowPda,
      params.gateway.gatewayPda,
      params.sourceId,
      params.signaturesRequired,
      params.canonicalTimestamp,
      params.amount
    );
    preInstructions.push(
      Ed25519Program.createInstructionWithPrivateKey({
        privateKey: params.payer.secretKey,
        message: authHash,
      })
    );
  }

  await params.proto.program.methods
    .settleAgentRound({
      authority,
      amount: new BN(params.amount),
      sourceId: Array.from(params.sourceId),
      canonicalTimestamp: new BN(params.canonicalTimestamp),
      signaturesRequired: params.signaturesRequired,
      registryVersion: params.registryVersion,
      aggSig: params.aggSig ?? zeroedAggSig(),
      value: Buffer.from(params.value ?? new Uint8Array(32)),
    })
    .accountsPartial({
      gatewayAuthority: params.gateway.gatewayAuthority.publicKey,
      protocolConfig: params.proto.protocolConfigPda,
      registryState: params.proto.registryStatePda,
      registry: findRegistryPda(programId, params.registryVersion),
      gateway: params.gateway.gatewayPda,
      agentEscrow: escrowPda,
      usdcMint: params.proto.usdcMint,
      escrowAta,
      treasury: params.proto.treasury,
      tokenProgram: TOKEN_PROGRAM_ID,
      roundRecord: roundRecordPda,
      instructionsSysvar: SYSVAR_INSTRUCTIONS_PUBKEY,
      systemProgram: SystemProgram.programId,
    })
    .remainingAccounts(
      (params.creditedNodePdas ?? []).map((pubkey) => ({
        pubkey,
        isWritable: false,
        isSigner: false,
      }))
    )
    .preInstructions(preInstructions)
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
