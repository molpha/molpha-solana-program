"use strict";

/**
 * Shared node-manifest helpers for generate-nodes.js and register-nodes.js.
 */
const fs = require("fs");
const path = require("path");
const anchor = require("@coral-xyz/anchor");
const {
  Keypair,
  PublicKey,
  SystemProgram,
  LAMPORTS_PER_SOL,
  Transaction,
  sendAndConfirmTransaction,
} = require("@solana/web3.js");
const {
  getAssociatedTokenAddressSync,
  createAssociatedTokenAccountInstruction,
  createTransferInstruction,
} = require("@solana/spl-token");
const { keccak_256 } = require("@noble/hashes/sha3.js");
const {
  generateKeypair: generateEvmKeypair,
} = require("../generate-evm-keypair");

const NODE_ID_PREFIX = Buffer.from("MOLPHA_NODE_ID_V1");
const PROTOCOL_CONFIG_SEED = Buffer.from("molpha_config");
const REGISTRY_STATE_SEED = Buffer.from("molpha_registry");
const REGISTRY_SEED = Buffer.from("molpha_registry");
const NODE_SEED = Buffer.from("molpha_node");
const OWNER_SOL_LAMPORTS = LAMPORTS_PER_SOL;
const DEPOSIT_BUFFER_BPS = 11000;
const MAX_NODES = 256;

function deriveNodeId(owner, secpCompressed, registrationSalt) {
  return Buffer.from(
    keccak_256(
      Buffer.concat([
        NODE_ID_PREFIX,
        owner.toBuffer(),
        Buffer.from(secpCompressed),
        Buffer.from(registrationSalt),
      ]),
    ),
  );
}

function findProtocolConfigPda(programId) {
  return PublicKey.findProgramAddressSync([PROTOCOL_CONFIG_SEED], programId)[0];
}

function findNodePda(programId, nodeOwner) {
  return PublicKey.findProgramAddressSync(
    [NODE_SEED, nodeOwner.toBuffer()],
    programId,
  )[0];
}

function findRegistryStatePda(programId) {
  return PublicKey.findProgramAddressSync([REGISTRY_STATE_SEED], programId)[0];
}

function findRegistryPda(programId, version) {
  const versionBytes = Buffer.alloc(4);
  versionBytes.writeUInt32LE(version);
  return PublicKey.findProgramAddressSync(
    [REGISTRY_SEED, versionBytes],
    programId,
  )[0];
}

function findNodeDepositVault(usdcMint, registryStatePda) {
  return getAssociatedTokenAddressSync(usdcMint, registryStatePda, true);
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
  } catch (error) {
    throw new Error(`Failed to read ${filepath}: ${error.message}`);
  }
  if (!Array.isArray(raw.nodes) || raw.nodes.length === 0) {
    throw new Error(
      `Invalid manifest at ${filepath}: expected non-empty "nodes" array`,
    );
  }
  return raw;
}

function keypairFromNodeRecord(node) {
  if (!node.solana?.secretKey?.length) {
    throw new Error(
      `Node ${node.index ?? "?"} missing solana.secretKey in manifest`,
    );
  }
  return Keypair.fromSecretKey(Uint8Array.from(node.solana.secretKey));
}

function secpCompressedFromNodeRecord(node) {
  const hex =
    node.evm?.publicKeyCompressed0x ?? node.evm?.publicKeyCompressed ?? null;
  if (!hex) {
    throw new Error(
      `Node ${node.index ?? "?"} missing EVM compressed public key`,
    );
  }
  const clean = hex.startsWith("0x") ? hex.slice(2) : hex;
  const buf = Buffer.from(clean, "hex");
  if (buf.length !== 33) {
    throw new Error(
      `Node ${node.index ?? "?"}: expected 33-byte compressed pubkey, got ${
        buf.length
      }`,
    );
  }
  return buf;
}

function secpPrivateKeyFromNodeRecord(node) {
  const hex = node.evm?.privateKey0x ?? node.evm?.privateKey ?? null;
  if (!hex) {
    throw new Error(`Node ${node.index ?? "?"} missing EVM private key`);
  }
  const key = Buffer.from(hex.replace(/^0x/, ""), "hex");
  if (key.length !== 32) throw new Error("EVM private key must be 32 bytes");
  return key;
}

function registrationSaltFromNodeRecord(node) {
  const hex = (node.registrationSalt ?? "").replace(/^0x/, "");
  if (hex.length !== 64) {
    throw new Error(
      `Node ${node.index ?? "?"}: registrationSalt must be 32 bytes hex`,
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

function parseIpv4(value, label = "ip") {
  const parts = String(value)
    .split(".")
    .map((part) => Number(part));
  if (
    parts.length !== 4 ||
    parts.some((part) => !Number.isInteger(part) || part < 0 || part > 255)
  ) {
    throw new Error(`Invalid ${label}: ${value}`);
  }
  return parts;
}

function requireNodeEndpoint(node) {
  const label = `Node ${node.index ?? "?"}`;
  if (
    node.ip == null ||
    node.ip === "" ||
    node.port == null ||
    node.port === ""
  ) {
    throw new Error(
      `${label} is missing ip and port. Set both on the node in the manifest before registering.`,
    );
  }
  const ip = parseIpv4(node.ip, `${label} ip`);
  const port = Number(node.port);
  if (!Number.isInteger(port) || port < 1 || port > 65535) {
    throw new Error(
      `${label} has invalid port ${node.port} (expected 1..65535)`,
    );
  }
  if (ip.every((octet) => octet === 0)) {
    throw new Error(`${label} ip must not be 0.0.0.0`);
  }
  return { ip, port };
}

function prepareLoadedNodes(manifest, programId, limit) {
  const slice = limit ? manifest.nodes.slice(0, limit) : manifest.nodes;
  return slice.map((record) => {
    const solanaOwner = keypairFromNodeRecord(record);
    const secpCompressed = secpCompressedFromNodeRecord(record);
    const secpPrivateKey = secpPrivateKeyFromNodeRecord(record);
    const registrationSalt = registrationSaltFromNodeRecord(record);
    const nodeId = nodeIdFromRecord(record);
    const nodePda = findNodePda(programId, solanaOwner.publicKey);
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
      registrationSalt,
    );
    const nodePda = findNodePda(programId, solanaOwner.publicKey);

    items.push({
      record: {
        index,
        nodeId: `0x${nodeId.toString("hex")}`,
        registrationSalt: `0x${registrationSalt.toString("hex")}`,
        ip: null,
        port: null,
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

async function ensureOwnerFunded(
  connection,
  payer,
  owner,
  usdcMint,
  tokenAmount,
) {
  const balance = await connection.getBalance(owner.publicKey);
  const ownerAta = getAssociatedTokenAddressSync(usdcMint, owner.publicKey);
  const payerAta = getAssociatedTokenAddressSync(usdcMint, payer.publicKey);

  const ataInfo = await connection.getAccountInfo(ownerAta);
  const tx = new Transaction();
  if (balance < OWNER_SOL_LAMPORTS) {
    const needed = OWNER_SOL_LAMPORTS - balance;
    console.log(
      `Transferring ${
        needed / LAMPORTS_PER_SOL
      } SOL from ${payer.publicKey.toBase58()} to ${owner.publicKey.toBase58()}`,
    );
    tx.add(
      SystemProgram.transfer({
        fromPubkey: payer.publicKey,
        toPubkey: owner.publicKey,
        lamports: needed,
      }),
    );
  }
  if (!ataInfo) {
    tx.add(
      createAssociatedTokenAccountInstruction(
        payer.publicKey,
        ownerAta,
        owner.publicKey,
        usdcMint,
      ),
    );
  }

  const payerAtaInfo = await connection
    .getTokenAccountBalance(payerAta)
    .catch(() => null);
  if (!payerAtaInfo) {
    throw new Error(
      `Admin USDC ATA ${payerAta.toBase58()} missing. Fund ${payer.publicKey.toBase58()} with USDC first.`,
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
        tokenAmount - current,
      ),
    );
  }

  if (tx.instructions.length > 0) {
    tx.feePayer = payer.publicKey;
    tx.recentBlockhash = (await connection.getLatestBlockhash()).blockhash;
    await sendAndConfirmTransaction(connection, tx, [payer]);
  }

  return ownerAta;
}

function printUsage(filename, lineCount) {
  console.log(
    fs
      .readFileSync(filename, "utf8")
      .split("\n")
      .slice(0, lineCount)
      .join("\n"),
  );
}

module.exports = {
  MAX_NODES,
  DEPOSIT_BUFFER_BPS,
  deriveNodeId,
  findProtocolConfigPda,
  findNodePda,
  findRegistryStatePda,
  findRegistryPda,
  findNodeDepositVault,
  writeOutput,
  loadManifestFromFile,
  prepareLoadedNodes,
  generateFreshNodes,
  requireNodeEndpoint,
  isNodeRegisteredOnChain,
  ensureOwnerFunded,
  printUsage,
};
