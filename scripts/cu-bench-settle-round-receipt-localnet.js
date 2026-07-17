#!/usr/bin/env node
/* eslint-disable no-console */
/**
 * Measure CU usage of `settle_round_receipt` on localnet.
 *
 * This script bootstraps a fresh protocol state, then simulates
 * `settle_round_receipt` with valid Ed25519 attestations and reports:
 * - total transaction CU (`unitsConsumed`)
 * - CU consumed by the Molpha program invocation itself (parsed from logs)
 *
 * Prerequisites:
 *   - `anchor build -p molpha`
 *   - local validator running with program deployed
 *   - fresh validator state (`solana-test-validator --reset`) recommended
 *
 * Usage:
 *   node scripts/cu-bench-settle-round-receipt-localnet.js
 *   node scripts/cu-bench-settle-round-receipt-localnet.js --signers 1,3,8
 *   node scripts/cu-bench-settle-round-receipt-localnet.js --output settle-cu.json
 */
const crypto = require("crypto");
const anchor = require("@coral-xyz/anchor");
const {
  AddressLookupTableProgram,
  ComputeBudgetProgram,
  Ed25519Program,
  Keypair,
  PublicKey,
  SystemProgram,
  SYSVAR_INSTRUCTIONS_PUBKEY,
  Transaction,
  TransactionMessage,
  VersionedTransaction,
} = require("@solana/web3.js");
const {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  TOKEN_PROGRAM_ID,
  createMint,
  getOrCreateAssociatedTokenAccount,
  mintTo,
} = require("@solana/spl-token");
const secp256k1 = require("@noble/secp256k1");
const { Point, utils: secpUtils } = secp256k1;
const { keccak_256 } = require("@noble/hashes/sha3");
const fs = require("fs");
const path = require("path");
const os = require("os");
const { createProvider } = require("./lib/molpha-provider");
const { findSecp256k1KeyClaimPda, signNodePop } = require("./lib/node-pop");

const DOMAIN_REQAUTH = Buffer.from("MOLPHA_REQAUTH_V1", "utf8");
const DOMAIN_GWRCPT = Buffer.from("MOLPHA_GWRCPT_V1", "utf8");
const MESSAGE_PREFIX = Buffer.from(
  keccak_256(Buffer.from("MOLPHA_MESSAGE_V1", "utf8"))
);
const FEED_ID_PREFIX = Buffer.from("MOLPHA_JOB_V1", "utf8");
const NODE_ID_PREFIX = Buffer.from("MOLPHA_NODE_ID_V1", "utf8");

const PROTOCOL_CONFIG_SEED = Buffer.from("molpha_config");
const REGISTRY_STATE_SEED = Buffer.from("molpha_registry");
const PLAN_SEED = Buffer.from("molpha_plan");
const SUBSCRIPTION_SEED = Buffer.from("molpha_subscription");
const JOB_SEED = Buffer.from("molpha_job");
const FEED_SEED = Buffer.from("molpha_feed");
const GATEWAY_SEED = Buffer.from("molpha_gateway");
const NODE_SEED = Buffer.from("molpha_node");
const NODE_OPERATOR_SEED = Buffer.from("molpha_node_operator");
const ROUND_RECEIPT_SEED = Buffer.from("molpha_round");

const DEFAULT_SIGNER_COUNTS = [1, 3, 8];
const DEFAULT_COMPUTE_UNIT_LIMIT = 1_400_000;
const DEFAULT_OUTPUT = path.join(
  process.cwd(),
  "settle-round-receipt-cu-localnet.json"
);

function parseArgs(argv) {
  const opts = {
    cluster: "localnet",
    rpc: null,
    wallet: null,
    signers: DEFAULT_SIGNER_COUNTS,
    computeUnitLimit: DEFAULT_COMPUTE_UNIT_LIMIT,
    output: DEFAULT_OUTPUT,
  };

  for (let i = 2; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--cluster") opts.cluster = argv[++i];
    else if (arg === "--rpc") opts.rpc = argv[++i];
    else if (arg === "--wallet") opts.wallet = argv[++i];
    else if (arg === "--signers") {
      opts.signers = argv[++i]
        .split(",")
        .map((v) => parseInt(v.trim(), 10))
        .filter((v) => Number.isFinite(v));
    } else if (arg === "--compute-unit-limit") {
      opts.computeUnitLimit = parseInt(argv[++i], 10);
    } else if (arg === "--output") {
      opts.output = path.resolve(argv[++i]);
    } else if (arg === "--help" || arg === "-h") {
      console.log(
        "Usage: node scripts/cu-bench-settle-round-receipt-localnet.js [--signers 1,3,8] [--output file.json]"
      );
      process.exit(0);
    } else {
      throw new Error(`Unknown argument: ${arg}`);
    }
  }

  if (!Array.isArray(opts.signers) || opts.signers.length === 0) {
    throw new Error("--signers must include at least one positive integer");
  }
  for (const n of opts.signers) {
    if (!Number.isInteger(n) || n < 1 || n > 256) {
      throw new Error(`Invalid signer count: ${n}`);
    }
  }
  if (
    !Number.isInteger(opts.computeUnitLimit) ||
    opts.computeUnitLimit < 200_000
  ) {
    throw new Error("--compute-unit-limit must be an integer >= 200000");
  }

  return opts;
}

function u32le(value) {
  const out = Buffer.alloc(4);
  out.writeUInt32LE(value >>> 0, 0);
  return out;
}

function u64le(value) {
  const out = Buffer.alloc(8);
  out.writeBigUInt64LE(BigInt(value), 0);
  return out;
}

function u32be(value) {
  const out = Buffer.alloc(4);
  out.writeUInt32BE(value >>> 0, 0);
  return out;
}

function i64AsU64be(value) {
  const out = Buffer.alloc(8);
  out.writeBigUInt64BE(BigInt.asUintN(64, BigInt(value)), 0);
  return out;
}

function sha256(parts) {
  const h = crypto.createHash("sha256");
  for (const p of parts) {
    h.update(p);
  }
  return h.digest();
}

function deriveFeedId(owner, apiConfigHash, signaturesRequired) {
  return Buffer.from(
    keccak_256(
      Buffer.concat([
        FEED_ID_PREFIX,
        owner.toBuffer(),
        apiConfigHash,
        Buffer.from([signaturesRequired]),
      ])
    )
  );
}

function deriveNodeId(owner, secp256k1PubkeyCompressed, registrationSalt) {
  return Buffer.from(
    keccak_256(
      Buffer.concat([
        NODE_ID_PREFIX,
        owner.toBuffer(),
        Buffer.from(secp256k1PubkeyCompressed),
        Buffer.from(registrationSalt),
      ])
    )
  );
}

function bitmapForFirstN(n) {
  const bm = Buffer.alloc(32, 0);
  for (let i = 0; i < n; i++) {
    const byteIndex = 31 - (i >> 3);
    const bitMask = 1 << (i & 7);
    bm[byteIndex] |= bitMask;
  }
  return bm;
}

function bitmapBitSet(bitmap, bit) {
  const byteIndex = 31 - (bit >> 3);
  const bitMask = 1 << (bit & 7);
  return (bitmap[byteIndex] & bitMask) !== 0;
}

function hashRequestAuth(feedId, timestamp) {
  const body = Buffer.concat([Buffer.from(feedId), u64le(timestamp)]);
  return sha256([DOMAIN_REQAUTH, body]);
}

function hashAggregatedSig(aggSig) {
  const body = Buffer.concat([
    Buffer.from(aggSig.s),
    Buffer.from(aggSig.rx),
    Buffer.from([aggSig.ryParity]),
    Buffer.from(aggSig.commitmentAddr),
    Buffer.from(aggSig.signersBitmap),
  ]);
  return sha256([body]);
}

function computeDataUpdateMessageHash({
  feedId,
  registryVersion,
  signaturesRequired,
  signersBitmap,
  value,
  timestamp,
}) {
  return Buffer.from(
    keccak_256(
      Buffer.concat([
        MESSAGE_PREFIX,
        Buffer.from(feedId),
        u32be(registryVersion),
        u32be(signaturesRequired),
        Buffer.from(signersBitmap),
        Buffer.from(value),
        i64AsU64be(timestamp),
      ])
    )
  );
}

function hashGatewayReceipt(
  gatewayReceipt,
  requestAuthHash,
  aggSigHash,
  dataUpdateMessageHash
) {
  const body = Buffer.concat([
    u32le(gatewayReceipt.registryVersion),
    Buffer.from(gatewayReceipt.selectionBitmap),
    u32le(gatewayReceipt.gatewayIndex),
  ]);
  return sha256([
    DOMAIN_GWRCPT,
    body,
    requestAuthHash,
    aggSigHash,
    dataUpdateMessageHash,
  ]);
}

function walletKeypairFromPath(walletPath) {
  const secret = Uint8Array.from(
    JSON.parse(fs.readFileSync(walletPath, "utf8"))
  );
  return Keypair.fromSecretKey(secret);
}

async function airdropIfNeeded(connection, pubkey, minimumSol = 5) {
  const minLamports = minimumSol * 1_000_000_000;
  const current = await connection.getBalance(pubkey, "processed");
  if (current >= minLamports) {
    return;
  }
  const sig = await connection.requestAirdrop(pubkey, minLamports - current);
  await connection.confirmTransaction(sig, "processed");
}

function parseProgramConsumedUnits(logs, programId) {
  if (!Array.isArray(logs)) return null;
  const prefix = `Program ${programId.toBase58()} consumed `;
  for (const line of logs) {
    if (!line.startsWith(prefix)) continue;
    const match = line.match(/consumed (\d+) of \d+ compute units/);
    if (match) return Number(match[1]);
  }
  return null;
}

function writeOutputJson(filepath, data) {
  fs.mkdirSync(path.dirname(filepath), { recursive: true });
  fs.writeFileSync(filepath, JSON.stringify(data, null, 2), "utf8");
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function sendLegacyInstructions(connection, payer, instructions) {
  const { blockhash } = await connection.getLatestBlockhash("processed");
  const tx = new Transaction().add(...instructions);
  tx.feePayer = payer.publicKey;
  tx.recentBlockhash = blockhash;
  tx.sign(payer);
  const sig = await connection.sendRawTransaction(tx.serialize(), {
    skipPreflight: true,
  });
  await connection.confirmTransaction(sig, "processed");
  return sig;
}

async function createLookupTableForAddresses(connection, payer, addresses) {
  if (!addresses.length) {
    return null;
  }

  const recentSlot = await connection.getSlot("processed");
  const [createIx, lutAddress] = AddressLookupTableProgram.createLookupTable({
    authority: payer.publicKey,
    payer: payer.publicKey,
    recentSlot,
  });

  await sendLegacyInstructions(connection, payer, [createIx]);

  const chunkSize = 20;
  for (let i = 0; i < addresses.length; i += chunkSize) {
    const chunk = addresses.slice(i, i + chunkSize);
    const extendIx = AddressLookupTableProgram.extendLookupTable({
      authority: payer.publicKey,
      payer: payer.publicKey,
      lookupTable: lutAddress,
      addresses: chunk,
    });
    await sendLegacyInstructions(connection, payer, [extendIx]);
  }

  let lutResp = await connection.getAddressLookupTable(lutAddress);
  if (!lutResp.value) {
    throw new Error("Failed to fetch created lookup table");
  }
  const lastExtendedSlot = lutResp.value.state.lastExtendedSlot;

  // Newly extended LUT addresses become fully available starting the next slot.
  for (let i = 0; i < 100; i++) {
    const slot = await connection.getSlot("processed");
    if (slot > lastExtendedSlot) {
      break;
    }
    await sleep(150);
  }

  lutResp = await connection.getAddressLookupTable(lutAddress);
  if (!lutResp.value) {
    throw new Error("Lookup table disappeared after extension");
  }
  return lutResp.value;
}

async function bootstrapState({
  program,
  provider,
  programId,
  payer,
  signerCounts,
}) {
  const maxSigners = Math.max(...signerCounts);
  const connection = provider.connection;

  const [protocolConfigPda] = PublicKey.findProgramAddressSync(
    [PROTOCOL_CONFIG_SEED],
    programId
  );
  const [registryStatePda] = PublicKey.findProgramAddressSync(
    [REGISTRY_STATE_SEED],
    programId
  );
  const [planPda] = PublicKey.findProgramAddressSync(
    [PLAN_SEED, Buffer.from([1])],
    programId
  );
  const [subscriptionPda] = PublicKey.findProgramAddressSync(
    [SUBSCRIPTION_SEED, payer.publicKey.toBuffer()],
    programId
  );

  const existingProtocol = await connection.getAccountInfo(
    protocolConfigPda,
    "processed"
  );
  if (existingProtocol) {
    throw new Error(
      `Protocol already initialized at ${protocolConfigPda.toBase58()}. ` +
        "Use a fresh validator (`solana-test-validator --reset`) for deterministic CU bench."
    );
  }

  await airdropIfNeeded(connection, payer.publicKey, 20);

  const usdcMint = await createMint(
    connection,
    payer,
    payer.publicKey,
    null,
    6
  );

  // Initialize protocol with a negative grace window so we can register nodes back-to-back.
  await program.methods
    .initialize({
      registryCooldownSecs: new anchor.BN(0),
      previousGraceSecs: new anchor.BN(-1),
      withdrawalCooldownSecs: new anchor.BN(0),
      maxNodeCount: maxSigners + 4,
      redundancyBuffer: 0,
      minimumNodeDeposit: new anchor.BN(1),
      rewardPerSignature: new anchor.BN(10),
      gatewayFee: new anchor.BN(20),
      receiptSubmitterBonus: new anchor.BN(5),
      protocolFeeBps: 100,
      maxReceiptDelaySlots: new anchor.BN(512),
      protocolFee: new anchor.BN(0),
      disputeWindowSlots: new anchor.BN(512),
      gatewayBondMin: new anchor.BN(500_000),
      challengerBountyBps: 500,
      usdcMint,
    })
    .accountsPartial({
      protocolConfig: protocolConfigPda,
      registryState: registryStatePda,
      usdcMint,
      authority: payer.publicKey,
      tokenProgram: TOKEN_PROGRAM_ID,
      associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
    })
    .rpc({ commitment: "processed" });

  const protocolConfig = await program.account.protocolConfig.fetch(
    protocolConfigPda
  );
  const treasury = new PublicKey(protocolConfig.treasury);

  // Create and fund payer USDC ATA.
  const payerUsdcAta = await getOrCreateAssociatedTokenAccount(
    connection,
    payer,
    usdcMint,
    payer.publicKey
  );
  await mintTo(
    connection,
    payer,
    usdcMint,
    payerUsdcAta.address,
    payer.publicKey,
    BigInt(5_000_000_000)
  );

  // Add Standard plan and subscribe payer (job owner).
  await program.methods
    .addPlan({
      planType: { standard: {} },
      subscriptionPrice: new anchor.BN(100_000),
      maxJobs: 8,
      maxSigners: maxSigners,
      maxDelegates: 2,
      privateApiEnabled: false,
      isActive: true,
    })
    .accountsPartial({
      protocolConfig: protocolConfigPda,
      plan: planPda,
      authority: payer.publicKey,
      systemProgram: SystemProgram.programId,
    })
    .rpc({ commitment: "processed" });

  await program.methods
    .subscribe({ standard: {} })
    .accountsPartial({
      owner: payer.publicKey,
      protocolConfig: protocolConfigPda,
      plan: planPda,
      subscription: subscriptionPda,
      usdcMint,
      ownerUsdc: payerUsdcAta.address,
      treasury,
      systemProgram: SystemProgram.programId,
      tokenProgram: TOKEN_PROGRAM_ID,
    })
    .rpc({ commitment: "processed" });

  // Create a job with signatures_required = 1 so we can test multiple credited counts.
  const apiConfigHash = crypto.randomBytes(32);
  const feedId = deriveFeedId(payer.publicKey, apiConfigHash, 1);
  const [jobPda] = PublicKey.findProgramAddressSync(
    [JOB_SEED, feedId],
    programId
  );
  const [feedPda] = PublicKey.findProgramAddressSync(
    [FEED_SEED, feedId],
    programId
  );

  await program.methods
    .createJob(
      {
        apiConfigHash: Array.from(apiConfigHash),
        signaturesRequired: 1,
        decimals: 8,
      },
      Array.from(feedId)
    )
    .accountsPartial({
      creator: payer.publicKey,
      jobOwner: payer.publicKey,
      protocolConfig: protocolConfigPda,
      plan: planPda,
      job: jobPda,
      subscription: subscriptionPda,
      feed: feedPda,
      delegate: null,
      systemProgram: SystemProgram.programId,
    })
    .rpc({ commitment: "processed" });

  // Register gateway.
  const gatewayAuthority = Keypair.generate();
  await airdropIfNeeded(connection, gatewayAuthority.publicKey, 2);

  const gatewayUsdcAta = await getOrCreateAssociatedTokenAccount(
    connection,
    payer,
    usdcMint,
    gatewayAuthority.publicKey
  );
  await mintTo(
    connection,
    payer,
    usdcMint,
    gatewayUsdcAta.address,
    payer.publicKey,
    BigInt(2_000_000)
  );

  const [gatewayPda] = PublicKey.findProgramAddressSync(
    [GATEWAY_SEED, gatewayAuthority.publicKey.toBuffer()],
    programId
  );
  await program.methods
    .registerGateway({
      ip: [10, 0, 0, 9],
      port: 8080,
      bondAmount: new anchor.BN(500_000),
    })
    .accountsPartial({
      gatewayAuthority: gatewayAuthority.publicKey,
      protocolConfig: protocolConfigPda,
      gateway: gatewayPda,
      usdcMint,
      authorityUsdc: gatewayUsdcAta.address,
      treasury,
      systemProgram: SystemProgram.programId,
      tokenProgram: TOKEN_PROGRAM_ID,
    })
    .signers([gatewayAuthority])
    .rpc({ commitment: "processed" });

  const gateway = await program.account.gateway.fetch(gatewayPda);

  // Register nodes and keep PDA maps by index.
  const nodePdasByIndex = new Map();
  for (let index = 0; index < maxSigners; index++) {
    const nodeOwner = Keypair.generate();
    await airdropIfNeeded(connection, nodeOwner.publicKey, 2);

    const ownerUsdc = await getOrCreateAssociatedTokenAccount(
      connection,
      payer,
      usdcMint,
      nodeOwner.publicKey
    );
    await mintTo(
      connection,
      payer,
      usdcMint,
      ownerUsdc.address,
      payer.publicKey,
      BigInt(1_000_000)
    );

    const [nodeOperatorPda] = PublicKey.findProgramAddressSync(
      [NODE_OPERATOR_SEED, nodeOwner.publicKey.toBuffer()],
      programId
    );
    const indexBytes = Buffer.alloc(4);
    indexBytes.writeUInt32LE(index, 0);
    const [nodePda] = PublicKey.findProgramAddressSync(
      [NODE_SEED, indexBytes],
      programId
    );

    const secpPrivate = secpUtils.randomPrivateKey();
    const secpCompressed = Buffer.from(
      secp256k1.getPublicKey(secpPrivate, true)
    );
    const registrationSalt = crypto.randomBytes(32);
    const nodeId = deriveNodeId(
      nodeOwner.publicKey,
      secpCompressed,
      registrationSalt
    );
    const pop = signNodePop({
      programId,
      nodeId,
      secp256k1PrivateKey: secpPrivate,
    });
    const keyClaimPda = findSecp256k1KeyClaimPda(programId, secpCompressed);

    await program.methods
      .registerNode({
        newIndex: index,
        registrationSalt: Array.from(registrationSalt),
        ip: [127, 0, 0, 1],
        port: 9000 + index,
        secp256K1PubkeyCompressed: Array.from(secpCompressed),
        secp256K1KeyFingerprint: Array.from(keccak_256(secpCompressed)),
        popSigR: Array.from(pop.popSigR),
        popSigS: Array.from(pop.popSigS),
        securityDeposit: new anchor.BN(1),
      })
      .accountsPartial({
        authority: payer.publicKey,
        nodeOwner: nodeOwner.publicKey,
        protocolConfig: protocolConfigPda,
        registryState: registryStatePda,
        nodeOperator: nodeOperatorPda,
        node: nodePda,
        secp256K1KeyClaim: keyClaimPda,
        usdcMint,
        ownerUsdc: ownerUsdc.address,
        treasury,
        systemProgram: SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .preInstructions([
        ComputeBudgetProgram.setComputeUnitLimit({ units: 400_000 }),
      ])
      .signers([nodeOwner])
      .rpc({ commitment: "processed" });

    nodePdasByIndex.set(index, nodePda);
  }

  const registryState = await program.account.registryState.fetch(
    registryStatePda
  );
  const lookupAddresses = [];
  for (let index = 0; index < maxSigners; index++) {
    lookupAddresses.push(nodePdasByIndex.get(index));
  }
  const lookupTableAccount = await createLookupTableForAddresses(
    connection,
    payer,
    lookupAddresses
  );

  return {
    protocolConfigPda,
    registryStatePda,
    subscriptionPda,
    gatewayPda,
    gatewayAuthority,
    gatewayIndex: Number(gateway.index),
    registryVersion: Number(registryState.currentVersion),
    signaturesRequired: 1,
    apiConfigHash,
    feedId,
    nodePdasByIndex,
    lookupTableAccount,
  };
}

function remainingAccountsForBitmap(bitmap, nodePdasByIndex) {
  const remaining = [];
  for (let bit = 0; bit < 256; bit++) {
    if (!bitmapBitSet(bitmap, bit)) continue;
    const nodePda = nodePdasByIndex.get(bit);
    if (!nodePda) {
      throw new Error(`Missing registered node for bit ${bit}`);
    }
    remaining.push({
      pubkey: nodePda,
      isSigner: false,
      isWritable: true,
    });
  }
  return remaining;
}

async function simulateSettleRound({
  program,
  provider,
  payer,
  setupState,
  signerCount,
  computeUnitLimit,
}) {
  const bitmap = bitmapForFirstN(signerCount);
  const timestamp = Math.floor(Date.now() / 1000) + signerCount;
  const value = crypto.randomBytes(32);

  const aggSig = {
    s: crypto.randomBytes(32),
    rx: crypto.randomBytes(32),
    ryParity: 1,
    commitmentAddr: crypto.randomBytes(20),
    signersBitmap: bitmap,
  };

  const feedId = deriveFeedId(
    payer.publicKey,
    setupState.apiConfigHash,
    setupState.signaturesRequired
  );
  const requestAuthHash = hashRequestAuth(feedId, timestamp);

  const consumerSigIx = Ed25519Program.createInstructionWithPrivateKey({
    privateKey: payer.secretKey,
    message: requestAuthHash,
  });

  const [roundRecordPda] = PublicKey.findProgramAddressSync(
    [
      ROUND_RECEIPT_SEED,
      payer.publicKey.toBuffer(),
      setupState.apiConfigHash,
      Buffer.from([setupState.signaturesRequired]),
      u64le(timestamp),
    ],
    program.programId
  );

  const settleArgs = {
    timestamp: new anchor.BN(timestamp),
    apiConfigHash: Array.from(setupState.apiConfigHash),
    signaturesRequired: setupState.signaturesRequired,
    registryVersion: setupState.registryVersion,
    aggSig: {
      s: Array.from(aggSig.s),
      rx: Array.from(aggSig.rx),
      ryParity: aggSig.ryParity,
      commitmentAddr: Array.from(aggSig.commitmentAddr),
      signersBitmap: Array.from(aggSig.signersBitmap),
    },
    value: Array.from(value),
    consumerAuthority: payer.publicKey,
  };

  const settleIx = await program.methods
    .settleRoundReceipt(settleArgs)
    .accountsPartial({
      payer: payer.publicKey,
      protocolConfig: setupState.protocolConfigPda,
      registryState: setupState.registryStatePda,
      subscription: setupState.subscriptionPda,
      consumerDelegate: null,
      gateway: setupState.gatewayPda,
      roundRecord: roundRecordPda,
      instructionsSysvar: SYSVAR_INSTRUCTIONS_PUBKEY,
      systemProgram: SystemProgram.programId,
    })
    .remainingAccounts(
      remainingAccountsForBitmap(bitmap, setupState.nodePdasByIndex)
    )
    .instruction();

  const { blockhash } = await provider.connection.getLatestBlockhash(
    "processed"
  );
  const v0Message = new TransactionMessage({
    payerKey: payer.publicKey,
    recentBlockhash: blockhash,
    instructions: [
      ComputeBudgetProgram.setComputeUnitLimit({ units: computeUnitLimit }),
      consumerSigIx,
      settleIx,
    ],
  }).compileToV0Message(
    setupState.lookupTableAccount ? [setupState.lookupTableAccount] : []
  );

  const vtx = new VersionedTransaction(v0Message);
  vtx.sign([payer]);

  const sim = await provider.connection.simulateTransaction(vtx, {
    sigVerify: false,
    commitment: "processed",
  });
  if (sim.value.err) {
    throw new Error(
      `Simulation failed for signerCount=${signerCount}: ${JSON.stringify(
        sim.value.err
      )}\n${(sim.value.logs || []).join("\n")}`
    );
  }

  const totalUnitsConsumed = sim.value.unitsConsumed;
  if (typeof totalUnitsConsumed !== "number") {
    throw new Error("simulateTransaction did not return unitsConsumed");
  }
  const settleProgramUnitsConsumed = parseProgramConsumedUnits(
    sim.value.logs,
    program.programId
  );

  return {
    signerCount,
    timestamp,
    signersBitmapHex: `0x${bitmap.toString("hex")}`,
    totalUnitsConsumed,
    settleProgramUnitsConsumed,
    roundRecord: roundRecordPda.toBase58(),
  };
}

async function main() {
  const opts = parseArgs(process.argv);

  const walletPath =
    opts.wallet ||
    process.env.ANCHOR_WALLET ||
    path.join(os.homedir(), ".config", "solana", "id.json");
  if (!fs.existsSync(walletPath)) {
    throw new Error(`Wallet not found at ${walletPath}`);
  }
  const payer = walletKeypairFromPath(walletPath);

  const { program, programId } = createProvider({
    cluster: opts.cluster,
    rpc: opts.rpc,
    wallet: walletPath,
  });
  const provider = anchor.getProvider();

  if (!program?.methods?.settleRoundReceipt) {
    throw new Error(
      "IDL does not contain settleRoundReceipt. Rebuild first: `anchor build -p molpha`."
    );
  }

  const programInfo = await provider.connection.getAccountInfo(
    programId,
    "processed"
  );
  if (!programInfo || !programInfo.executable) {
    throw new Error(
      `Program ${programId.toBase58()} is not deployed on ${
        provider.connection.rpcEndpoint
      }.`
    );
  }

  console.log(`Program: ${programId.toBase58()}`);
  console.log(`RPC: ${provider.connection.rpcEndpoint}`);
  console.log(`Payer: ${payer.publicKey.toBase58()}`);
  console.log(`Signer counts: ${opts.signers.join(", ")}`);

  const setupState = await bootstrapState({
    program,
    provider,
    programId,
    payer,
    signerCounts: opts.signers,
  });

  const measurements = [];
  for (const signerCount of opts.signers) {
    const result = await simulateSettleRound({
      program,
      provider,
      payer,
      setupState,
      signerCount,
      computeUnitLimit: opts.computeUnitLimit,
    });
    measurements.push(result);
    console.log(
      `measured signerCount=${signerCount} total=${result.totalUnitsConsumed} settle_program=${result.settleProgramUnitsConsumed}`
    );
  }

  const output = {
    measuredAt: new Date().toISOString(),
    programId: programId.toBase58(),
    rpc: provider.connection.rpcEndpoint,
    signerCounts: opts.signers,
    computeUnitLimit: opts.computeUnitLimit,
    results: measurements,
  };

  writeOutputJson(opts.output, output);
  console.log("");
  console.log(JSON.stringify(output, null, 2));
  console.log(`\nWrote CU report to ${opts.output}`);
}

main().catch((err) => {
  console.error("CU bench failed:", err.message || err);
  if (err?.stack) {
    console.error(err.stack);
  }
  if (err.logs) {
    console.error(err.logs.join("\n"));
  }
  process.exit(1);
});
