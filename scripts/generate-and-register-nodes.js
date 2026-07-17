#!/usr/bin/env node

/**
 * Generate N oracle nodes (Solana owner + EVM secp256k1), register on-chain via
 * `register_node`, and write a JSON manifest with keys and registry metadata.
 *
 * Prerequisites:
 *   - `anchor build` (target/idl/molpha.json)
 *   - Protocol initialized (`yarn molpha init`)
 *   - Admin wallet funded (ANCHOR_WALLET): SOL + USDC for node deposits
 *
 * Usage:
 *   node scripts/generate-and-register-nodes.js [options]
 *
 * Options:
 *   --cluster <name>      localnet | devnet | mainnet (default: Anchor.toml provider or localnet)
 *   --rpc <url>           RPC URL (overrides cluster default)
 *   --wallet <path>       Payer/admin keypair (default: ANCHOR_WALLET or ~/.config/solana/id.json)
 *   --count <n>           Number of nodes when generating (default: 5)
 *   --output <path>       Output JSON path (default: ./generated-nodes.json)
 *   --generate-new        Generate fresh keys and overwrite the output file
 *   --finalize            Call finalize_registry_transition after last node
 *   --security-deposit <n> Override deposit (default: protocol minimum)
 *   --skip-register       Only generate keys; do not send transactions
 *
 * By default, if the output file already exists, keys are loaded from it and only
 * on-chain registration runs (nodes that already have a registration are skipped).
 */

const fs = require("fs");
const path = require("path");
const os = require("os");
const anchor = require("@coral-xyz/anchor");
const { BN, Program } = require("@coral-xyz/anchor");
const {
  Keypair,
  PublicKey,
  SystemProgram,
  ComputeBudgetProgram,
  LAMPORTS_PER_SOL,
  Transaction,
  sendAndConfirmTransaction,
} = require("@solana/web3.js");
const {
  getAssociatedTokenAddressSync,
  createAssociatedTokenAccountInstruction,
  createTransferInstruction,
  TOKEN_PROGRAM_ID,
  ASSOCIATED_TOKEN_PROGRAM_ID,
} = require("@solana/spl-token");
const { keccak_256 } = require("@noble/hashes/sha3");
const {
  generateKeypair: generateEvmKeypair,
} = require("./generate-evm-keypair");
const { findSecp256k1KeyClaimPda, signNodePop } = require("./lib/node-pop");

const NODE_ID_PREFIX = Buffer.from("MOLPHA_NODE_ID_V1");
const PROTOCOL_CONFIG_SEED = Buffer.from("molpha_config");
const REGISTRY_STATE_SEED = Buffer.from("molpha_registry");
const NODE_SEED = Buffer.from("molpha_node");
const NODE_OPERATOR_SEED = Buffer.from("molpha_node_operator");
const DEFAULT_COUNT = 5;
const DEFAULT_OUTPUT = path.resolve(process.cwd(), "generated-nodes.json");
const OWNER_SOL_LAMPORTS = 0.05 * LAMPORTS_PER_SOL;
const DEPOSIT_BUFFER_BPS = 11000; // 110% of minimum deposit
const ANCHOR_TOML_PATH = path.resolve(__dirname, "../Anchor.toml");

const VALID_CLUSTERS = ["localnet", "devnet", "mainnet"];
const DEFAULT_RPCS = {
  localnet: "http://127.0.0.1:8899",
  devnet: "https://api.devnet.solana.com",
  mainnet: "https://api.mainnet-beta.solana.com",
};

function parseArgs(argv) {
  const opts = {
    cluster: null,
    rpc: null,
    wallet: null,
    count: DEFAULT_COUNT,
    countExplicit: false,
    output: DEFAULT_OUTPUT,
    generateNew: false,
    finalize: false,
    securityDeposit: null,
    skipRegister: false,
  };

  for (let i = 2; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--cluster" || arg === "-c") {
      opts.cluster = argv[++i];
    } else if (arg === "--rpc") {
      opts.rpc = argv[++i];
    } else if (arg === "--wallet" || arg === "-w") {
      opts.wallet = argv[++i];
    } else if (arg === "--count" || arg === "-n") {
      opts.count = parseInt(argv[++i], 10);
      opts.countExplicit = true;
    } else if (arg === "--output" || arg === "-o") {
      opts.output = path.resolve(argv[++i]);
    } else if (arg === "--generate-new") {
      opts.generateNew = true;
    } else if (arg === "--finalize") {
      opts.finalize = true;
    } else if (arg === "--security-deposit") {
      opts.securityDeposit = argv[++i];
    } else if (arg === "--skip-register") {
      opts.skipRegister = true;
    } else if (arg === "--help" || arg === "-h") {
      console.log(
        fs.readFileSync(__filename, "utf8").split("\n").slice(0, 28).join("\n")
      );
      process.exit(0);
    } else {
      console.error(`Unknown argument: ${arg}`);
      process.exit(1);
    }
  }

  if (opts.countExplicit && (!Number.isFinite(opts.count) || opts.count < 1)) {
    console.error("--count must be a positive integer");
    process.exit(1);
  }

  if (opts.cluster && !VALID_CLUSTERS.includes(opts.cluster)) {
    console.error(`--cluster must be one of: ${VALID_CLUSTERS.join(", ")}`);
    process.exit(1);
  }

  opts.useExisting = !opts.generateNew && fs.existsSync(opts.output);

  return opts;
}

function inferClusterFromRpc(rpc) {
  if (!rpc) {
    return null;
  }
  if (rpc.includes("devnet")) {
    return "devnet";
  }
  if (rpc.includes("mainnet")) {
    return "mainnet";
  }
  return "localnet";
}

function readAnchorTomlProviderCluster() {
  if (!fs.existsSync(ANCHOR_TOML_PATH)) {
    return null;
  }
  const source = fs.readFileSync(ANCHOR_TOML_PATH, "utf8");
  let inProvider = false;
  for (const rawLine of source.split(/\r?\n/)) {
    const line = rawLine.trim();
    if (line === "[provider]") {
      inProvider = true;
      continue;
    }
    if (inProvider && line.match(/^\[/)) {
      break;
    }
    if (!inProvider) {
      continue;
    }
    const match = line.match(/^cluster\s*=\s*"([^"]+)"$/);
    if (match && VALID_CLUSTERS.includes(match[1])) {
      return match[1];
    }
  }
  return null;
}

function readProgramIdFromAnchorToml(cluster) {
  if (!fs.existsSync(ANCHOR_TOML_PATH)) {
    return null;
  }
  const source = fs.readFileSync(ANCHOR_TOML_PATH, "utf8");
  const lines = source.split(/\r?\n/);
  let inTargetSection = false;

  for (const rawLine of lines) {
    const line = rawLine.trim();
    if (!line || line.startsWith("#")) {
      continue;
    }
    const sectionMatch = line.match(/^\[(.+)\]$/);
    if (sectionMatch) {
      inTargetSection = sectionMatch[1] === `programs.${cluster}`;
      continue;
    }
    if (!inTargetSection) {
      continue;
    }
    const programMatch = line.match(/^molpha\s*=\s*"([^"]+)"$/);
    if (programMatch) {
      return programMatch[1];
    }
  }
  return null;
}

function resolveConnectionOptions(opts) {
  const envRpc = process.env.ANCHOR_PROVIDER_URL;
  const cluster =
    opts.cluster ??
    inferClusterFromRpc(opts.rpc ?? envRpc) ??
    readAnchorTomlProviderCluster() ??
    "localnet";

  const rpcUrl =
    opts.rpc ??
    (opts.cluster ? DEFAULT_RPCS[cluster] : null) ??
    envRpc ??
    DEFAULT_RPCS[cluster];

  const programIdFromToml = readProgramIdFromAnchorToml(cluster);
  if (!programIdFromToml && !opts.skipRegister) {
    console.warn(
      `⚠️  No molpha program id in Anchor.toml [programs.${cluster}]; using IDL address`
    );
  }

  const walletPath =
    opts.wallet ??
    process.env.ANCHOR_WALLET ??
    path.join(os.homedir(), ".config/solana/id.json");

  process.env.ANCHOR_PROVIDER_URL = rpcUrl;
  if (opts.wallet) {
    process.env.ANCHOR_WALLET = path.resolve(opts.wallet);
  } else if (!process.env.ANCHOR_WALLET && fs.existsSync(walletPath)) {
    process.env.ANCHOR_WALLET = walletPath;
  }

  return {
    cluster,
    rpcUrl,
    walletPath: process.env.ANCHOR_WALLET ?? walletPath,
    programId: programIdFromToml,
  };
}

function loadProgram(provider, programIdOverride) {
  const idlPath = path.resolve(__dirname, "../target/idl/molpha.json");
  if (!fs.existsSync(idlPath)) {
    throw new Error(`Missing IDL at ${idlPath}. Run: anchor build -p molpha`);
  }
  const idl = JSON.parse(fs.readFileSync(idlPath, "utf8"));
  const programId = new PublicKey(programIdOverride ?? idl.address);
  idl.address = programId.toBase58();
  return { program: new Program(idl, provider), programId, idlPath };
}

function deriveNodeId(owner, secpCompressed, registrationSalt) {
  return Buffer.from(
    keccak_256(
      Buffer.concat([
        NODE_ID_PREFIX,
        owner.toBuffer(),
        Buffer.from(secpCompressed),
        Buffer.from(registrationSalt),
      ])
    )
  );
}

function findNodeOperatorPda(programId, nodeOwner) {
  return PublicKey.findProgramAddressSync(
    [NODE_OPERATOR_SEED, nodeOwner.toBuffer()],
    programId
  )[0];
}

function findNodePda(programId, index) {
  const indexBytes = Buffer.alloc(4);
  indexBytes.writeUInt32LE(index);
  return PublicKey.findProgramAddressSync(
    [NODE_SEED, indexBytes],
    programId
  )[0];
}

function buildRegisterNodeArgs({
  newIndex,
  registrationSalt,
  secpCompressed,
  popSigR,
  popSigS,
  securityDeposit,
}) {
  return {
    newIndex,
    registrationSalt: Array.from(registrationSalt),
    ip: [127, 0, 0, 1],
    port: 8080 + newIndex,
    secp256K1PubkeyCompressed: Array.from(secpCompressed),
    secp256K1KeyFingerprint: Array.from(keccak_256(secpCompressed)),
    popSigR: Array.from(popSigR),
    popSigS: Array.from(popSigS),
    securityDeposit,
  };
}

function solanaKeypairRecord(kp) {
  return {
    publicKey: kp.publicKey.toBase58(),
    secretKey: Array.from(kp.secretKey),
    secretKeyBase58: anchor.utils.bytes.bs58.encode(kp.secretKey),
  };
}

function evmKeypairRecord(evm) {
  const x = evm.publicKeyUncompressed.slice(1, 33);
  const y = evm.publicKeyUncompressed.slice(33, 65);
  return {
    privateKey: evm.privateKey.toString("hex"),
    privateKey0x: `0x${evm.privateKey.toString("hex")}`,
    publicKeyCompressed: evm.publicKeyCompressed.toString("hex"),
    publicKeyCompressed0x: `0x${evm.publicKeyCompressed.toString("hex")}`,
    publicKeyUncompressed: evm.publicKeyUncompressed.toString("hex"),
    publicKeyUncompressed0x: `0x${evm.publicKeyUncompressed.toString("hex")}`,
    pubKeyX: x.toString("hex"),
    pubKeyY: y.toString("hex"),
    address: `0x${evm.address.toString("hex")}`,
  };
}

async function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function waitForRegistryGrace(connection, registryStatePda, program) {
  const registry = await program.account.registryState.fetch(registryStatePda);
  const expiresAt = Number(registry.previousExpiresAt);
  if (expiresAt <= 0) {
    return;
  }

  while (true) {
    const slot = await connection.getSlot("confirmed");
    const blockTime = await connection.getBlockTime(slot);
    const now = blockTime ?? Math.floor(Date.now() / 1000);
    if (now > expiresAt) {
      return;
    }
    const waitSecs = Math.min(Math.max(expiresAt - now + 1, 1), 120);
    console.log(
      `   ⏳ Registry grace active until ${expiresAt} (now ${now}); sleeping ${waitSecs}s...`
    );
    await sleep(waitSecs * 1000);
  }
}

async function ensureOwnerFunded(
  connection,
  payer,
  owner,
  usdcMint,
  tokenAmount
) {
  const balance = await connection.getBalance(owner.publicKey);
  if (balance < OWNER_SOL_LAMPORTS) {
    const needed = OWNER_SOL_LAMPORTS - balance;
    console.log(`Airdropping ${needed} SOL to ${owner.publicKey.toBase58()}`);
    const sig = await connection.requestAirdrop(owner.publicKey, needed);
    await connection.confirmTransaction(sig, "confirmed");
  }

  const ownerAta = getAssociatedTokenAddressSync(usdcMint, owner.publicKey);
  const payerAta = getAssociatedTokenAddressSync(usdcMint, payer.publicKey);

  const ataInfo = await connection.getAccountInfo(ownerAta);
  const tx = new Transaction();
  if (!ataInfo) {
    tx.add(
      createAssociatedTokenAccountInstruction(
        payer.publicKey,
        ownerAta,
        owner.publicKey,
        usdcMint
      )
    );
  }

  const payerAtaInfo = await connection
    .getTokenAccountBalance(payerAta)
    .catch(() => null);
  if (!payerAtaInfo) {
    throw new Error(
      `Admin USDC ATA ${payerAta.toBase58()} missing. Fund ${payer.publicKey.toBase58()} with USDC first.`
    );
  }

  const ownerAtaInfo = await connection
    .getTokenAccountBalance(ownerAta)
    .catch(() => null);
  const current = ownerAtaInfo ? BigInt(ownerAtaInfo.value.amount) : 0n;
  if (current < tokenAmount) {
    tx.add(
      createTransferInstruction(
        payerAta,
        ownerAta,
        payer.publicKey,
        tokenAmount - current
      )
    );
  }

  if (tx.instructions.length > 0) {
    tx.feePayer = payer.publicKey;
    tx.recentBlockhash = (await connection.getLatestBlockhash()).blockhash;
    await sendAndConfirmTransaction(connection, tx, [payer]);
  }

  return ownerAta;
}

function writeOutput(filepath, payload) {
  const dir = path.dirname(filepath);
  if (!fs.existsSync(dir)) {
    fs.mkdirSync(dir, { recursive: true });
  }
  fs.writeFileSync(filepath, JSON.stringify(payload, null, 2), "utf8");
  try {
    fs.chmodSync(filepath, 0o600);
  } catch {
    // ignore on unsupported platforms
  }
}

function loadManifestFromFile(filepath) {
  let raw;
  try {
    raw = JSON.parse(fs.readFileSync(filepath, "utf8"));
  } catch (e) {
    throw new Error(`Failed to read ${filepath}: ${e.message}`);
  }
  if (!Array.isArray(raw.nodes) || raw.nodes.length === 0) {
    throw new Error(
      `Invalid manifest at ${filepath}: expected non-empty "nodes" array`
    );
  }
  return raw;
}

function keypairFromNodeRecord(node) {
  if (!node.solana?.secretKey?.length) {
    throw new Error(
      `Node ${node.index ?? "?"} missing solana.secretKey in manifest`
    );
  }
  return Keypair.fromSecretKey(Uint8Array.from(node.solana.secretKey));
}

function secpCompressedFromNodeRecord(node) {
  const hex =
    node.evm?.publicKeyCompressed0x ?? node.evm?.publicKeyCompressed ?? null;
  if (!hex) {
    throw new Error(
      `Node ${node.index ?? "?"} missing EVM compressed public key`
    );
  }
  const clean = hex.startsWith("0x") ? hex.slice(2) : hex;
  const buf = Buffer.from(clean, "hex");
  if (buf.length !== 33) {
    throw new Error(
      `Node ${node.index ?? "?"}: expected 33-byte compressed pubkey, got ${
        buf.length
      }`
    );
  }
  return buf;
}

function secpPrivateKeyFromNodeRecord(node) {
  const hex = node.evm?.privateKey0x ?? node.evm?.privateKey ?? null;
  if (!hex)
    throw new Error(`Node ${node.index ?? "?"} missing EVM private key`);
  const key = Buffer.from(hex.replace(/^0x/, ""), "hex");
  if (key.length !== 32) throw new Error("EVM private key must be 32 bytes");
  return key;
}

function registrationSaltFromNodeRecord(node) {
  const hex = (node.registrationSalt ?? "").replace(/^0x/, "");
  if (hex.length !== 64) {
    throw new Error(
      `Node ${node.index ?? "?"}: registrationSalt must be 32 bytes hex`
    );
  }
  return Buffer.from(hex, "hex");
}

function nodeIdFromRecord(node) {
  if (node.nodeId) {
    return Buffer.from(node.nodeId.replace(/^0x/, ""), "hex");
  }
  const owner = new PublicKey(node.solana.publicKey);
  const salt = registrationSaltFromNodeRecord(node);
  const secp = secpCompressedFromNodeRecord(node);
  return deriveNodeId(owner, secp, salt);
}

function prepareLoadedNodes(manifest, programId, limit) {
  const slice = limit ? manifest.nodes.slice(0, limit) : manifest.nodes;
  return slice.map((record) => {
    const solanaOwner = keypairFromNodeRecord(record);
    const secpCompressed = secpCompressedFromNodeRecord(record);
    const secpPrivateKey = secpPrivateKeyFromNodeRecord(record);
    const registrationSalt = registrationSaltFromNodeRecord(record);
    const nodeId = nodeIdFromRecord(record);
    const nodePda = findNodeOperatorPda(programId, solanaOwner.publicKey);
    return {
      record: { ...record },
      solanaOwner,
      secpCompressed,
      secpPrivateKey,
      registrationSalt,
      nodeId,
      nodePda,
    };
  });
}

function generateFreshNodes(programId, startIndex, count) {
  const items = [];
  for (let i = 0; i < count; i++) {
    const index = startIndex + i;
    const solanaOwner = Keypair.generate();
    const evm = generateEvmKeypair();
    const secpCompressed = evm.publicKeyCompressed;
    const secpPrivateKey = evm.privateKey;
    const registrationSalt = Buffer.alloc(32, index + 1);
    const nodeId = deriveNodeId(
      solanaOwner.publicKey,
      secpCompressed,
      registrationSalt
    );
    const nodePda = findNodeOperatorPda(programId, solanaOwner.publicKey);

    items.push({
      record: {
        index,
        nodeId: `0x${nodeId.toString("hex")}`,
        registrationSalt: `0x${registrationSalt.toString("hex")}`,
        solana: solanaKeypairRecord(solanaOwner),
        evm: evmKeypairRecord(evm),
        onChain: {
          nodePda: nodePda.toBase58(),
          nodeOwner: solanaOwner.publicKey.toBase58(),
        },
        registration: null,
      },
      solanaOwner,
      secpCompressed,
      secpPrivateKey,
      registrationSalt,
      nodeId,
      nodePda,
    });
  }
  return items;
}

async function isNodeRegisteredOnChain(connection, nodePda) {
  const info = await connection.getAccountInfo(nodePda);
  return info !== null;
}

async function main() {
  const opts = parseArgs(process.argv);
  const connection = resolveConnectionOptions(opts);

  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);
  const { program, programId } = loadProgram(provider, connection.programId);
  const admin = provider.wallet.payer;

  const [protocolConfigPda] = PublicKey.findProgramAddressSync(
    [PROTOCOL_CONFIG_SEED],
    programId
  );
  const [registryStatePda] = PublicKey.findProgramAddressSync(
    [REGISTRY_STATE_SEED],
    programId
  );

  let protocolConfig = null;
  let registryState = null;
  let startIndex = 0;
  let depositPerNode =
    opts.securityDeposit !== null ? BigInt(opts.securityDeposit) : null;
  let securityDepositBn = null;
  let usdcMint = null;
  let treasury = null;

  if (!opts.skipRegister) {
    try {
      protocolConfig = await program.account.protocolConfig.fetch(
        protocolConfigPda
      );
      registryState = await program.account.registryState.fetch(
        registryStatePda
      );
    } catch (e) {
      throw new Error(
        `Protocol not initialized. Run init first. (${e.message})`
      );
    }

    if (admin.publicKey.toString() !== protocolConfig.authority.toString()) {
      throw new Error(
        `ANCHOR_WALLET ${admin.publicKey.toBase58()} is not registry admin ${protocolConfig.authority.toBase58()}`
      );
    }

    startIndex = Number(registryState.currentNodeCount);

    const minDeposit = BigInt(protocolConfig.minimumNodeDeposit.toString());
    depositPerNode =
      opts.securityDeposit !== null
        ? BigInt(opts.securityDeposit)
        : (minDeposit * BigInt(DEPOSIT_BUFFER_BPS)) / 10000n;

    if (depositPerNode < minDeposit) {
      throw new Error(
        `security deposit ${depositPerNode} is below minimum ${minDeposit}`
      );
    }

    usdcMint = protocolConfig.usdcMint;
    treasury = protocolConfig.treasury;
    securityDepositBn = new BN(depositPerNode.toString());
  } else {
    startIndex = 0;
    depositPerNode =
      opts.securityDeposit !== null ? BigInt(opts.securityDeposit) : null;
  }

  let manifest;
  let nodeItems;
  let loadedFromFile = false;

  if (opts.useExisting) {
    manifest = loadManifestFromFile(opts.output);
    loadedFromFile = true;
    const limit = opts.countExplicit ? opts.count : manifest.nodes.length;
    nodeItems = prepareLoadedNodes(manifest, programId, limit);
    console.log(`Mode: loaded ${nodeItems.length} node(s) from ${opts.output}`);
  } else {
    if (opts.generateNew && fs.existsSync(opts.output)) {
      console.log(`Mode: generate new (will overwrite ${opts.output})`);
    } else {
      console.log("Mode: generate new keys");
    }
    const generateCount = opts.countExplicit ? opts.count : DEFAULT_COUNT;
    const maxNodes = Number(protocolConfig?.maxNodeCount ?? 0);
    if (!opts.skipRegister && startIndex + generateCount > maxNodes) {
      throw new Error(
        `Cannot register ${generateCount} nodes: registry has ${startIndex}/${maxNodes} slots used`
      );
    }
    nodeItems = generateFreshNodes(programId, startIndex, generateCount);
  }

  console.log(`Cluster: ${connection.cluster}`);
  console.log(`RPC: ${connection.rpcUrl}`);
  console.log(`Program: ${programId.toBase58()}`);
  if (!opts.skipRegister) {
    console.log(`Admin: ${admin.publicKey.toBase58()}`);
    console.log(`Registry node count (on-chain): ${startIndex}`);
    console.log(`Security deposit per node: ${depositPerNode.toString()}`);
  }
  console.log(`Nodes: ${nodeItems.length}`);
  console.log(`Output: ${opts.output}`);
  if (opts.skipRegister) {
    console.log("Registration: skipped (--skip-register)");
  }
  console.log("");

  if (!loadedFromFile) {
    manifest = {
      generatedAt: new Date().toISOString(),
      cluster: connection.cluster,
      programId: programId.toBase58(),
      rpcUrl: connection.rpcUrl,
      wallet: connection.walletPath,
      registryState: registryStatePda.toBase58(),
      protocolConfig: protocolConfigPda.toBase58(),
      usdcMint: usdcMint?.toBase58() ?? null,
      treasury: treasury?.toBase58() ?? null,
      startIndex,
      count: nodeItems.length,
      securityDepositPerNode: depositPerNode?.toString() ?? null,
      skipRegister: opts.skipRegister,
      loadedFromFile: false,
      nodes: [],
    };
  } else {
    manifest.generatedAt = manifest.generatedAt ?? new Date().toISOString();
    manifest.cluster = connection.cluster;
    manifest.programId = programId.toBase58();
    manifest.rpcUrl = connection.rpcUrl;
    manifest.wallet = connection.walletPath;
    manifest.registryState = registryStatePda.toBase58();
    manifest.protocolConfig = protocolConfigPda.toBase58();
    manifest.loadedFromFile = true;
    manifest.count = nodeItems.length;
    if (!opts.skipRegister) {
      manifest.usdcMint = usdcMint?.toBase58() ?? manifest.usdcMint;
      manifest.treasury = treasury?.toBase58() ?? manifest.treasury;
      manifest.securityDepositPerNode =
        depositPerNode?.toString() ?? manifest.securityDepositPerNode;
      manifest.skipRegister = false;
    }
  }

  let registeredThisRun = 0;
  let graceWaitedThisRun = false;

  for (let i = 0; i < nodeItems.length; i++) {
    const item = nodeItems[i];
    const nodeRecord = item.record;

    console.log(
      `Node ${nodeRecord.index}: Solana ${nodeRecord.solana.publicKey} | EVM ${nodeRecord.evm.address}`
    );

    if (!opts.skipRegister) {
      const alreadyOnChain = await isNodeRegisteredOnChain(
        provider.connection,
        item.nodePda
      );
      if (alreadyOnChain) {
        console.log("   ⏭️  Already registered on-chain, skipping");
        continue;
      }

      registryState = await program.account.registryState.fetch(
        registryStatePda
      );
      const chainIndex = Number(registryState.currentNodeCount);
      const maxNodes = Number(protocolConfig.maxNodeCount);
      if (chainIndex >= maxNodes) {
        throw new Error(
          `Registry full (${chainIndex}/${maxNodes}); cannot register more nodes`
        );
      }

      if (chainIndex > 0 || graceWaitedThisRun) {
        await waitForRegistryGrace(
          provider.connection,
          registryStatePda,
          program
        );
      }
      graceWaitedThisRun = true;

      const nodeOperatorPda = findNodeOperatorPda(
        programId,
        item.solanaOwner.publicKey
      );
      const nodePda = findNodePda(programId, chainIndex);
      const keyClaimPda = findSecp256k1KeyClaimPda(
        programId,
        item.secpCompressed
      );
      nodeRecord.index = chainIndex;
      nodeRecord.onChain = {
        ...nodeRecord.onChain,
        nodePda: nodePda.toBase58(),
        nodeOperatorPda: nodeOperatorPda.toBase58(),
        secp256k1KeyClaimPda: keyClaimPda.toBase58(),
        nodeOwner: item.solanaOwner.publicKey.toBase58(),
      };

      const ownerUsdc = await ensureOwnerFunded(
        provider.connection,
        admin,
        item.solanaOwner,
        usdcMint,
        depositPerNode
      );

      const pop = signNodePop({
        programId,
        nodeId: item.nodeId,
        secp256k1PrivateKey: item.secpPrivateKey,
      });
      const registerNodeArgs = buildRegisterNodeArgs({
        newIndex: chainIndex,
        registrationSalt: item.registrationSalt,
        secpCompressed: item.secpCompressed,
        popSigR: pop.popSigR,
        popSigS: pop.popSigS,
        securityDeposit: securityDepositBn,
      });

      const signature = await program.methods
        .registerNode(registerNodeArgs, Array.from(item.nodeId))
        .accountsPartial({
          adminAuthority: admin.publicKey,
          nodeOwner: item.solanaOwner.publicKey,
          protocolConfig: protocolConfigPda,
          registryState: registryStatePda,
          nodeOperator: nodeOperatorPda,
          node: nodePda,
          secp256K1KeyClaim: keyClaimPda,
          usdcMint,
          ownerUsdc,
          treasury,
          systemProgram: SystemProgram.programId,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .preInstructions([
          ComputeBudgetProgram.setComputeUnitLimit({ units: 400_000 }),
        ])
        .signers([item.solanaOwner])
        .rpc();

      nodeRecord.registration = {
        signature,
        registeredAt: new Date().toISOString(),
      };
      registeredThisRun += 1;
      console.log(`   ✅ Registered at index ${chainIndex}: ${signature}`);
    }
  }

  manifest.nodes = nodeItems.map((item) => item.record);

  if (!opts.skipRegister && opts.finalize) {
    await waitForRegistryGrace(provider.connection, registryStatePda, program);
    const finalizeSig = await program.methods
      .finalizeRegistryTransition()
      .accountsPartial({
        registryState: registryStatePda,
        virtualRegistryIndex: null,
        rentReceiver: admin.publicKey,
      })
      .rpc();
    manifest.finalizeRegistryTransition = { signature: finalizeSig };
    console.log(`\n✅ Finalized registry transition: ${finalizeSig}`);
  }

  writeOutput(opts.output, manifest);
  console.log(
    `\n💾 Wrote ${manifest.nodes.length} node(s) to ${opts.output} (mode 600)`
  );
  if (!opts.skipRegister) {
    console.log(`   Registered ${registeredThisRun} node(s) this run`);
  }
  console.log("⚠️  Keep this file secret — it contains private keys.");
}

main().catch((err) => {
  console.error("❌", err.message || err);
  if (err.logs) {
    console.error(err.logs.join("\n"));
  }
  process.exit(1);
});
