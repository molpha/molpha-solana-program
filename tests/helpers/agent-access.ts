/**
 * Shared harness for the agent pay-per-request tests (agent-access spec v1).
 *
 * Wraps protocol initialization, gateway bonding, escrow lifecycle, and
 * `settle_agent_round` transaction construction (including the Ed25519
 * `AgentRequestAuth` attestation instruction packed into the same tx).
 */
import * as anchor from "@coral-xyz/anchor";
import { BN, Idl, Program } from "@coral-xyz/anchor";
import {
  Ed25519Program,
  Keypair,
  PublicKey,
  SystemProgram,
  SYSVAR_INSTRUCTIONS_PUBKEY,
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
} from "./evm-submit-data-update";
import molphaIdl from "../../target/idl/molpha.json";

export const LOCALNET_PROGRAM_ID = new PublicKey(
  "MoLFeTRpDZgckPjjbLwW1wB9n85bQiqboPnvw9RwoG8"
);

export const PROTOCOL_CONFIG_SEED = Buffer.from("molpha_config");
export const REGISTRY_STATE_SEED = Buffer.from("molpha_registry");
export const GATEWAY_SEED = Buffer.from("molpha_gateway");
export const AGENT_ESCROW_SEED = Buffer.from("molpha_agent");
export const AGENT_ROUND_SEED = Buffer.from("molpha_agent_round");
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
  maxSigners: 8,
  epochCap: 1_000_000,
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

export function sha256(parts: Buffer[]): Buffer {
  const h = crypto.createHash("sha256");
  for (const part of parts) h.update(part);
  return h.digest();
}

/** `H(AgentRequestAuth)` — must match `state/receipt.rs::hash_agent_request_auth`. */
export function hashAgentRequestAuth(params: {
  agent: PublicKey;
  requestNonce: number | bigint;
  feedId: Uint8Array;
  maxPrice: number | bigint;
}): Buffer {
  const body = Buffer.concat([
    params.agent.toBuffer(),
    u64le(params.requestNonce),
    Buffer.from(params.feedId),
    u64le(params.maxPrice),
  ]);
  return sha256([DOMAIN_AGENT_REQAUTH, body]);
}

/** Escrow PDA — seeds ["molpha_agent", owner, authority]. */
export function findAgentEscrowPda(
  programId: PublicKey,
  owner: PublicKey,
  authority: PublicKey
): PublicKey {
  return PublicKey.findProgramAddressSync(
    [AGENT_ESCROW_SEED, owner.toBuffer(), authority.toBuffer()],
    programId
  )[0];
}

/** x402-provisioned escrow PDA — owner == authority == payer. */
export function findX402EscrowPda(
  programId: PublicKey,
  payer: PublicKey
): PublicKey {
  return findAgentEscrowPda(programId, payer, payer);
}

/** The escrow's own USDC ATA (real custody). The escrow PDA is off-curve. */
export function findEscrowAta(
  usdcMint: PublicKey,
  escrowPda: PublicKey
): PublicKey {
  return getAssociatedTokenAddressSync(usdcMint, escrowPda, true);
}

/**
 * Directly write a funded USDC token account at `ata` owned by `owner` (may be a PDA).
 * Mirrors `fundNodeOwnerInBankrun`, but for an arbitrary ATA — used to simulate an x402
 * `exact` payment landing in the escrow ATA before settlement.
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

export function findAgentRoundPda(
  programId: PublicKey,
  agent: PublicKey,
  requestNonce: number | bigint
): PublicKey {
  return PublicKey.findProgramAddressSync(
    [AGENT_ROUND_SEED, agent.toBuffer(), u64le(requestNonce)],
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

  await program.methods
    .initialize({
      registryCooldownSecs: new BN(1),
      previousGraceSecs: new BN(1),
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
      treasury: proto.treasury,
      systemProgram: SystemProgram.programId,
      tokenProgram: TOKEN_PROGRAM_ID,
    })
    .signers([gatewayAuthority])
    .rpc();

  return { gatewayAuthority, gatewayPda, authorityUsdc };
}

export interface AgentEscrowSetup {
  owner: Keypair;
  hotKey: Keypair;
  escrowPda: PublicKey;
  escrowAta: PublicKey;
  ownerUsdc: PublicKey;
}

export async function createFundedEscrow(
  ctx: TestContext,
  proto: AgentProtocolContext,
  opts?: {
    maxSigners?: number;
    epochCap?: number;
    expiresAt?: number;
    depositAmount?: number;
  }
): Promise<AgentEscrowSetup> {
  const owner = Keypair.generate();
  const hotKey = Keypair.generate();
  // Fresh owner + hot key per escrow — the [owner, authority] seed pair is the identity;
  // owner != authority marks it owner-provisioned.
  const ownerUsdc = await fundNodeOwnerInBankrun(
    ctx,
    owner,
    ctx.underlyingTokenMint,
    BigInt(1_000_000_000)
  );
  const escrowPda = findAgentEscrowPda(
    proto.program.programId,
    owner.publicKey,
    hotKey.publicKey
  );
  const escrowAta = findEscrowAta(ctx.underlyingTokenMint, escrowPda);

  await proto.program.methods
    .createAgentEscrow({
      authority: hotKey.publicKey,
      maxSigners: opts?.maxSigners ?? INIT.maxSigners,
      epochCap: new BN(opts?.epochCap ?? INIT.epochCap),
      expiresAt: new BN(opts?.expiresAt ?? 0),
    })
    .accountsPartial({
      owner: owner.publicKey,
      protocolConfig: proto.protocolConfigPda,
      agentEscrow: escrowPda,
      usdcMint: ctx.underlyingTokenMint,
      escrowAta,
      tokenProgram: TOKEN_PROGRAM_ID,
      associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
    })
    .signers([owner])
    .rpc();

  const depositAmount = opts?.depositAmount ?? 1_000_000;
  if (depositAmount > 0) {
    await proto.program.methods
      .depositAgentCredits(new BN(depositAmount))
      .accountsPartial({
        depositor: owner.publicKey,
        protocolConfig: proto.protocolConfigPda,
        agentEscrow: escrowPda,
        usdcMint: ctx.underlyingTokenMint,
        depositorUsdc: ownerUsdc,
        escrowAta,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([owner])
      .rpc();
  }

  return { owner, hotKey, escrowPda, escrowAta, ownerUsdc };
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
  /**
   * Owner-provisioned escrow (+ hot key that signs the `AgentRequestAuth`). Omit for the
   * x402 path and pass `payer` instead — then owner == authority == payer and the escrow is
   * lazily created by settle.
   */
  escrow?: AgentEscrowSetup;
  /** x402 payer identity (used when `escrow` is omitted). */
  payer?: PublicKey;
  requestNonce: number;
  maxPrice: number;
  feedId: Uint8Array;
  timestamp: number;
  signaturesRequired: number;
  registryVersion: number;
  aggSig?: ReturnType<typeof zeroedAggSig>;
  value?: Uint8Array;
  /** Node PDAs for the credited bitmap, ascending bit order. */
  creditedNodePdas?: PublicKey[];
  /** Override the Ed25519 attestation (e.g. wrong key / missing sig tests). */
  ed25519Signer?: Keypair | null;
  /** Override the (owner, authority) args passed on-chain (mismatch tests). */
  ownerAuthorityOverride?: { owner: PublicKey; authority: PublicKey };
}

export async function settleAgentRound(
  params: SettleAgentRoundParams
): Promise<void> {
  const programId = params.proto.program.programId;
  const ownerMode = !!params.escrow;

  const owner =
    params.ownerAuthorityOverride?.owner ??
    (ownerMode ? params.escrow!.owner.publicKey : params.payer!);
  const authority =
    params.ownerAuthorityOverride?.authority ??
    (ownerMode ? params.escrow!.hotKey.publicKey : params.payer!);

  const escrowPda = findAgentEscrowPda(programId, owner, authority);
  const escrowAta = findEscrowAta(params.proto.usdcMint, escrowPda);

  const roundRecordPda = findAgentRoundPda(
    programId,
    escrowPda,
    params.requestNonce
  );

  // Owner mode carries the off-chain hot-key attestation; x402 does not (the client's
  // on-chain payment into the escrow ATA is the authorization).
  const preInstructions = [];
  if (ownerMode && params.ed25519Signer !== null) {
    const signer = params.ed25519Signer ?? params.escrow!.hotKey;
    const authHash = hashAgentRequestAuth({
      agent: escrowPda,
      requestNonce: params.requestNonce,
      feedId: params.feedId,
      maxPrice: params.maxPrice,
    });
    preInstructions.push(
      Ed25519Program.createInstructionWithPrivateKey({
        privateKey: signer.secretKey,
        message: authHash,
      })
    );
  }

  const builder = params.proto.program.methods
    .settleAgentRound({
      owner,
      authority,
      requestNonce: new BN(params.requestNonce),
      maxPrice: new BN(params.maxPrice),
      feedId: Array.from(params.feedId),
      timestamp: new BN(params.timestamp),
      signaturesRequired: params.signaturesRequired,
      registryVersion: params.registryVersion,
      aggSig: params.aggSig ?? zeroedAggSig(),
      value: Array.from(params.value ?? new Uint8Array(32)),
    })
    .accountsPartial({
      gatewayAuthority: params.gateway.gatewayAuthority.publicKey,
      protocolConfig: params.proto.protocolConfigPda,
      registryState: params.proto.registryStatePda,
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
        isWritable: true,
        isSigner: false,
      }))
    )
    .preInstructions(preInstructions)
    .signers([params.gateway.gatewayAuthority]);

  await builder.rpc();
}

export function creditedNodePdasForBits(
  programId: PublicKey,
  bits: number[]
): PublicKey[] {
  return [...bits]
    .sort((a, b) => a - b)
    .map((bit) => findNodePda(programId, bit));
}

export { findNodePda };
