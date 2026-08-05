/* eslint-disable no-console */
/**
 * CU gates for immutable historical registries.
 *
 * Build first with `yarn build:molpha:cu-bench`, then run this script. It measures:
 * - cloning a 255-member fixed snapshot and appending member 256;
 * - production account-backed historical resolution for 18, 32, and 64 signers
 *   selected from a 256-member snapshot.
 */
const anchor = require("@coral-xyz/anchor");
const {
  AddressLookupTableAccount,
  AddressLookupTableProgram,
  ComputeBudgetProgram,
  Keypair,
  PublicKey,
  SystemProgram,
  Transaction,
  TransactionInstruction,
  TransactionMessage,
  VersionedTransaction,
} = require("@solana/web3.js");
const { BankrunProvider, startAnchor } = require("anchor-bankrun");
const crypto = require("crypto");
const fs = require("fs");
const path = require("path");

const COMPUTE_UNIT_LIMIT = 1_400_000;
const U64_MAX = 0xffffffffffffffffn;
const REGISTRY_SPACE = 8_208;
const MAX_NODES = 256;
const HISTORICAL_VERSION = 77;
const BENCH_VERSION = 91;
const REGISTRY_SEED = Buffer.from("molpha_registry");
const NODE_SEED = Buffer.from("molpha_node");
const BENCH_REGISTRY_SEED = Buffer.from("bench_full_registry");

function rootPath() {
  return path.join(__dirname, "..");
}

function loadIdl() {
  return JSON.parse(
    fs.readFileSync(
      path.join(rootPath(), "target", "idl", "molpha.json"),
      "utf8"
    )
  );
}

function u32le(value) {
  const out = Buffer.alloc(4);
  out.writeUInt32LE(value >>> 0);
  return out;
}

function discriminator(namespace, name) {
  return crypto
    .createHash("sha256")
    .update(`${namespace}:${name}`)
    .digest()
    .subarray(0, 8);
}

function deterministicKeypair(index) {
  const seed = crypto
    .createHash("sha256")
    .update(`molpha-versioned-registry-cu:${index}`)
    .digest()
    .subarray(0, 32);
  return Keypair.fromSeed(seed);
}

function bitmapFirstN(count) {
  const bitmap = Buffer.alloc(32);
  for (let index = 0; index < count; index++) {
    bitmap[31 - (index >> 3)] |= 1 << (index & 7);
  }
  return bitmap;
}

function systemAccount(lamports = 1_000_000_000_000) {
  return {
    lamports,
    owner: SystemProgram.programId,
    executable: false,
    data: Buffer.alloc(0),
  };
}

function setRawAccount(context, address, owner, data, lamports = 100_000_000) {
  context.setAccount(address, {
    lamports,
    owner,
    executable: false,
    data: Buffer.from(data),
  });
}

function registryData(
  accountName,
  version,
  members,
  bump,
  redundancyBuffer = 2
) {
  if (members.length > MAX_NODES) throw new Error("registry exceeds 256 nodes");
  const data = Buffer.alloc(REGISTRY_SPACE);
  discriminator("account", accountName).copy(data, 0);
  data.writeUInt32LE(version >>> 0, 8);
  data.writeUInt16LE(members.length, 12);
  data.writeUInt8(redundancyBuffer, 14);
  data.writeUInt8(bump, 15);
  members.forEach((member, index) =>
    member.toBuffer().copy(data, 16 + index * 32)
  );
  return data;
}

async function seedNode(context, program, ownerKeypair, index) {
  const [node, bump] = PublicKey.findProgramAddressSync(
    [NODE_SEED, ownerKeypair.publicKey.toBuffer()],
    program.programId
  );
  const data = await program.coder.accounts.encode("node", {
    owner: ownerKeypair.publicKey,
    secp256K1PubkeyX: Array.from(Buffer.alloc(32, (index % 251) + 1)),
    secp256K1PubkeyY: Array.from(Buffer.alloc(32, ((index + 1) % 251) + 1)),
    status: { active: {} },
    ip: [127, 0, 0, 1],
    port: 9_000 + index,
    lockedAmount: new anchor.BN(100_000_000),
    claimableRewards: new anchor.BN(0),
    registeredAt: new anchor.BN(0),
    deactivatedAt: new anchor.BN(0),
    withdrawableAt: new anchor.BN(0),
    bump,
  });
  setRawAccount(context, node, program.programId, data);
  return node;
}

function lookupTableData(addresses) {
  const data = Buffer.alloc(56 + addresses.length * 32);
  data.writeUInt32LE(1, 0);
  data.writeBigUInt64LE(U64_MAX, 4);
  data.writeBigUInt64LE(0n, 12);
  addresses.forEach((address, index) =>
    address.toBuffer().copy(data, 56 + index * 32)
  );
  return data;
}

function resultOf(simulation) {
  const logs = simulation.meta?.logMessages ?? [];
  return {
    ok: simulation.result === null,
    computeUnits: simulation.meta
      ? Number(simulation.meta.computeUnitsConsumed)
      : null,
    error: simulation.result === null ? null : String(simulation.result),
    diagnosticLogs: simulation.result === null ? null : logs.slice(-8),
  };
}

async function simulateLegacy(provider, payer, instruction) {
  const [blockhash] = await provider.context.banksClient.getLatestBlockhash();
  const transaction = new Transaction().add(
    ComputeBudgetProgram.setComputeUnitLimit({ units: COMPUTE_UNIT_LIMIT }),
    instruction
  );
  transaction.feePayer = payer.publicKey;
  transaction.recentBlockhash = blockhash;
  transaction.sign(payer);
  return provider.context.banksClient.simulateTransaction(transaction);
}

async function simulateV0(provider, payer, instruction, lookupTable) {
  const [blockhash] = await provider.context.banksClient.getLatestBlockhash();
  const message = new TransactionMessage({
    payerKey: payer.publicKey,
    recentBlockhash: blockhash,
    instructions: [
      ComputeBudgetProgram.setComputeUnitLimit({ units: COMPUTE_UNIT_LIMIT }),
      instruction,
    ],
  }).compileToV0Message([lookupTable]);
  const transaction = new VersionedTransaction(message);
  transaction.sign([payer]);
  return provider.context.banksClient.simulateTransaction(transaction);
}

async function measureSnapshotCreation(
  context,
  provider,
  program,
  payer,
  members
) {
  const [oldRegistry, oldBump] = PublicKey.findProgramAddressSync(
    [BENCH_REGISTRY_SEED, u32le(BENCH_VERSION)],
    program.programId
  );
  const [newRegistry] = PublicKey.findProgramAddressSync(
    [BENCH_REGISTRY_SEED, u32le(BENCH_VERSION + 1)],
    program.programId
  );
  setRawAccount(
    context,
    oldRegistry,
    program.programId,
    registryData(
      "BenchFullRegistry",
      BENCH_VERSION,
      members.slice(0, 255),
      oldBump
    )
  );

  // Feature-gated ix may be absent from the production IDL; encode manually.
  const data = Buffer.alloc(8 + 4 + 4 + 1 + 32 + 2);
  discriminator("global", "bench_full_registry_transition").copy(data, 0);
  data.writeUInt32LE(BENCH_VERSION, 8);
  data.writeUInt32LE(BENCH_VERSION + 1, 12);
  data.writeUInt8(0, 16); // append mutation
  members[255].toBuffer().copy(data, 17);
  data.writeUInt16LE(0, 49);

  const instruction = new TransactionInstruction({
    programId: program.programId,
    keys: [
      { pubkey: payer.publicKey, isSigner: true, isWritable: true },
      { pubkey: oldRegistry, isSigner: false, isWritable: false },
      { pubkey: newRegistry, isSigner: false, isWritable: true },
      { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
    ],
    data,
  });
  return resultOf(await simulateLegacy(provider, payer, instruction));
}

async function measureHistoricalResolution(
  context,
  provider,
  program,
  payer,
  registry,
  nodes,
  signerCount
) {
  const selected = nodes.slice(0, signerCount);
  const instruction = new TransactionInstruction({
    programId: program.programId,
    keys: [
      { pubkey: registry, isSigner: false, isWritable: false },
      ...selected.map((pubkey) => ({
        pubkey,
        isSigner: false,
        isWritable: false,
      })),
    ],
    data: Buffer.concat([
      discriminator("global", "bench_resolve_accounts"),
      bitmapFirstN(signerCount),
    ]),
  });

  const addresses = [registry, ...selected];
  const lookupKey = PublicKey.unique();
  const lookupTable = new AddressLookupTableAccount({
    key: lookupKey,
    state: {
      deactivationSlot: U64_MAX,
      lastExtendedSlot: 0,
      lastExtendedSlotStartIndex: 0,
      authority: undefined,
      addresses,
    },
  });
  setRawAccount(
    context,
    lookupKey,
    AddressLookupTableProgram.programId,
    lookupTableData(addresses)
  );
  return resultOf(await simulateV0(provider, payer, instruction, lookupTable));
}

async function main() {
  const soPath = path.join(rootPath(), "target", "deploy", "molpha.so");
  if (!fs.existsSync(soPath)) {
    throw new Error(
      "missing program build; run yarn build:molpha:cu-bench first"
    );
  }
  const idl = loadIdl();
  const programId = new PublicKey(idl.address);
  const payer = Keypair.generate();
  const context = await startAnchor(
    ".",
    [{ name: "molpha", programId }],
    [{ address: payer.publicKey, info: systemAccount() }],
    BigInt(COMPUTE_UNIT_LIMIT)
  );
  const provider = new BankrunProvider(context);
  provider.wallet = new anchor.Wallet(payer);
  anchor.setProvider(provider);
  const program = new anchor.Program(idl, provider);

  const nodes = [];
  for (let index = 0; index < MAX_NODES; index++) {
    nodes.push(
      await seedNode(context, program, deterministicKeypair(index), index)
    );
  }
  const [registry, registryBump] = PublicKey.findProgramAddressSync(
    [REGISTRY_SEED, u32le(HISTORICAL_VERSION)],
    program.programId
  );
  setRawAccount(
    context,
    registry,
    program.programId,
    registryData("Registry", HISTORICAL_VERSION, nodes, registryBump)
  );

  const snapshotCreation256 = await measureSnapshotCreation(
    context,
    provider,
    program,
    payer,
    nodes
  );
  console.log("snapshot-creation-256", snapshotCreation256);

  const historicalResolution = [];
  for (const signerCount of [18, 32, 64]) {
    const measurement = await measureHistoricalResolution(
      context,
      provider,
      program,
      payer,
      registry,
      nodes,
      signerCount
    );
    const row = { registryNodeCount: MAX_NODES, signerCount, ...measurement };
    historicalResolution.push(row);
    console.log("historical-resolution", row);
  }

  const results = {
    generatedAt: new Date().toISOString(),
    runtime: "solana-bankrun / cu-bench feature",
    computeUnitLimit: COMPUTE_UNIT_LIMIT,
    registrySpaceBytes: REGISTRY_SPACE,
    snapshotCreation256,
    historicalResolution,
  };
  const outputPath = path.join(
    rootPath(),
    "reports",
    "node-registry-cu-results.raw.json"
  );
  fs.mkdirSync(path.dirname(outputPath), { recursive: true });
  fs.writeFileSync(outputPath, `${JSON.stringify(results, null, 2)}\n`);
  console.log(`wrote ${outputPath}`);
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
