#!/usr/bin/env node

/**
 * Register a bonded gateway on-chain.
 *
 * Prerequisites:
 *   - anchor build -p molpha
 *   - Protocol initialized; wallet must hold enough USDC for the bond
 *
 * Usage:
 *   node scripts/add-gateway.js <IP> <PORT> [BOND_AMOUNT]
 *   node scripts/add-gateway.js <IP:PORT> [BOND_AMOUNT]
 *
 * Options:
 *   --cluster <name>   localnet | devnet | mainnet
 *   --rpc <url>
 *   --wallet <path>
 *
 * Examples:
 *   node scripts/add-gateway.js 10.0.0.1 8080
 *   node scripts/add-gateway.js 10.0.0.1:8080 500000000 --cluster devnet
 */

const { SystemProgram } = require("@solana/web3.js");
const {
  getAssociatedTokenAddressSync,
  TOKEN_PROGRAM_ID,
} = require("@solana/spl-token");
const { createProvider } = require("./lib/molpha-provider");

const PROTOCOL_CONFIG_SEED = Buffer.from("molpha_config");
const GATEWAY_SEED = Buffer.from("molpha_gateway");
const DEFAULT_BOND_AMOUNT = 500_000_000;

function parseArgs(argv) {
  const positional = [];
  const opts = { cluster: null, rpc: null, wallet: null };

  for (let i = 2; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--cluster") {
      opts.cluster = argv[++i];
    } else if (arg === "--rpc") {
      opts.rpc = argv[++i];
    } else if (arg === "--wallet") {
      opts.wallet = argv[++i];
    } else if (arg.startsWith("--")) {
      console.error(`Unknown option: ${arg}`);
      process.exit(1);
    } else {
      positional.push(arg);
    }
  }

  return { positional, opts };
}

function parseIpv4(ipStr) {
  const parts = ipStr.split(".").map((part) => parseInt(part, 10));
  if (
    parts.length !== 4 ||
    parts.some((n) => Number.isNaN(n) || n < 0 || n > 255)
  ) {
    throw new Error(`Invalid IPv4 address: ${ipStr}`);
  }
  return parts;
}

function parsePort(portStr) {
  const port = parseInt(portStr, 10);
  if (Number.isNaN(port) || port < 1 || port > 65535) {
    throw new Error(`Invalid port: ${portStr} (must be 1–65535)`);
  }
  return port;
}

function parseEndpoint(positional) {
  if (positional.length === 0) {
    return null;
  }

  if (positional.length === 1) {
    const endpoint = positional[0];
    const colon = endpoint.lastIndexOf(":");
    if (colon === -1) {
      throw new Error("Provide <IP> <PORT> or a single <IP:PORT> endpoint");
    }
    return {
      ip: parseIpv4(endpoint.slice(0, colon)),
      port: parsePort(endpoint.slice(colon + 1)),
      bondAmount: DEFAULT_BOND_AMOUNT,
    };
  }

  if (positional.length === 2) {
    const maybeBond = parseInt(positional[1], 10);
    if (!Number.isNaN(maybeBond) && !positional[1].includes(".")) {
      return {
        ip: parseIpv4(positional[0]),
        port: parsePort(positional[1]),
        bondAmount: DEFAULT_BOND_AMOUNT,
      };
    }
    return {
      ip: parseIpv4(positional[0]),
      port: parsePort(positional[1]),
      bondAmount: DEFAULT_BOND_AMOUNT,
    };
  }

  if (positional.length === 3) {
    return {
      ip: parseIpv4(positional[0]),
      port: parsePort(positional[1]),
      bondAmount: parseInt(positional[2], 10),
    };
  }

  throw new Error("Too many arguments; use <IP> <PORT> [BOND_AMOUNT] or <IP:PORT> [BOND_AMOUNT]");
}

function findGatewayPda(programId, authority) {
  return require("@solana/web3.js").PublicKey.findProgramAddressSync(
    [GATEWAY_SEED, authority.toBuffer()],
    programId
  );
}

function formatIp(ip) {
  return ip.join(".");
}

async function main() {
  const { positional, opts } = parseArgs(process.argv);

  let endpoint;
  try {
    endpoint = parseEndpoint(positional);
  } catch (e) {
    console.error(`❌ ${e.message}`);
    console.log("");
    console.log("Usage: node scripts/add-gateway.js <IP> <PORT> [BOND_AMOUNT]");
    console.log("       node scripts/add-gateway.js <IP:PORT> [BOND_AMOUNT]");
    process.exit(1);
  }

  if (!endpoint) {
    console.error("❌ Error: gateway IP and port are required");
    console.log("Usage: node scripts/add-gateway.js <IP> <PORT> [BOND_AMOUNT]");
    process.exit(1);
  }

  const { ip, port, bondAmount } = endpoint;
  if (ip.every((octet) => octet === 0) || port === 0) {
    console.error("❌ Error: IP cannot be 0.0.0.0 and port cannot be 0");
    process.exit(1);
  }

  const { program, programId } = createProvider(opts);
  const gatewayAuthority = program.provider.wallet.publicKey;

  const [protocolConfigPda] =
    require("@solana/web3.js").PublicKey.findProgramAddressSync(
      [PROTOCOL_CONFIG_SEED],
      programId
    );
  const [gatewayPda] = findGatewayPda(programId, gatewayAuthority);

  let protocolConfig;
  try {
    protocolConfig = await program.account.protocolConfig.fetch(
      protocolConfigPda
    );
  } catch (e) {
    console.error(
      "❌ Protocol config not found. Initialize the protocol first."
    );
    process.exit(1);
  }

  const treasury = new (require("@solana/web3.js").PublicKey)(
    protocolConfig.treasury
  );
  const usdcMint = new (require("@solana/web3.js").PublicKey)(
    protocolConfig.usdcMint
  );
  const authorityUsdc = getAssociatedTokenAddressSync(
    usdcMint,
    gatewayAuthority
  );

  console.log("🔍 Registering gateway:");
  console.log("  Program ID:", programId.toString());
  console.log("  Endpoint:", `${formatIp(ip)}:${port}`);
  console.log("  Bond amount:", bondAmount);
  console.log("  Protocol config PDA:", protocolConfigPda.toString());
  console.log("  Gateway PDA:", gatewayPda.toString());
  console.log("  Gateway authority:", gatewayAuthority.toString());
  console.log("");

  try {
    await program.account.gateway.fetch(gatewayPda);
    console.error("❌ Gateway already registered for this authority");
    process.exit(1);
  } catch (e) {
    // account does not exist — expected
  }

  const signature = await program.methods
    .registerGateway({ ip, port, bondAmount })
    .accountsPartial({
      gatewayAuthority,
      protocolConfig: protocolConfigPda,
      gateway: gatewayPda,
      usdcMint,
      authorityUsdc,
      treasury,
      systemProgram: SystemProgram.programId,
      tokenProgram: TOKEN_PROGRAM_ID,
    })
    .rpc();

  console.log("✅ Gateway registered");
  console.log("  Transaction:", signature);

  const gateway = await program.account.gateway.fetch(gatewayPda);
  console.log("");
  console.log("📊 Gateway account:");
  console.log("  Authority:", gateway.authority.toString());
  console.log("  Index:", gateway.index);
  console.log("  IP:", formatIp(gateway.ip));
  console.log("  Port:", gateway.port);
  console.log("  Bond:", gateway.bond.toString());
  console.log(
    "  Registered at:",
    new Date(Number(gateway.registeredAt) * 1000).toISOString()
  );
}

main().catch((e) => {
  console.error("❌ Error registering gateway:", e.message ?? e);
  if (e.logs) {
    console.error("Transaction logs:", e.logs);
  }
  process.exit(1);
});
