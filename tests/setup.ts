import * as anchor from "@coral-xyz/anchor";
import { Program, BN, AnchorError, Wallet } from "@coral-xyz/anchor";
import { Molpha } from "../target/types/molpha";
import {
  Keypair,
  SystemProgram,
  PublicKey,
  ComputeBudgetProgram,
} from "@solana/web3.js";
import {
  getAssociatedTokenAddressSync,
  TOKEN_PROGRAM_ID,
  ASSOCIATED_TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { BankrunProvider } from "anchor-bankrun";
import * as toml from "toml";
import * as fs from "fs";
import * as crypto from "crypto";
import { AddedAccount, startAnchor } from "solana-bankrun";

export interface TestContext {
  molphaProgram: Program<Molpha>;
  nodeRegistryPDA: PublicKey;
  protocolConfigPDA: PublicKey;
  nodes: Keypair[];
  authority: anchor.Wallet;
  provider: BankrunProvider;
  // SPL Token related
  underlyingTokenMint: PublicKey;
  programTokenAccount: PublicKey;
  userTokenAccount: PublicKey;
}

function getProgramId(): PublicKey {
  const anchorToml = toml.parse(fs.readFileSync("./Anchor.toml", "utf-8"));

  return new PublicKey(anchorToml.programs.localnet.molpha);
}

export async function createAccounts() {
  // Load wallet
  console.log("Loading local wallet...");
  const wallet = loadWallet().payer;

  // Create Service Provider KeyPair
  console.log("Creating Service Provider KeyPair...");
  const user = Keypair.generate();

  const newAccounts = {
    wallet,
    user,
  };

  const wa = Object.values(newAccounts).map((acc) => ({
    address: acc.publicKey,
    info: {
      lamports: 1000_000_000_000,
      executable: false,
      owner: anchor.web3.SystemProgram.programId,
      data: Buffer.alloc(0),
    },
  }));

  // // Connection to mainnet for cloning accounts
  // const connection = new Connection('https://api.mainnet-beta.solana.com')

  // // Add Raydium config account
  // const raydiumConfig = await connection.getAccountInfo(RAYDIUM_CONFIG)
  // addedAccounts.push({
  //   address: RAYDIUM_CONFIG,
  //   info: raydiumConfig,
  // })

  // // Add Raydium pool fee receiver account
  // const raydiumPoolFeeReceiver = await connection.getAccountInfo(
  //   RAYDIUM_POOL_FEE_RECEIVER,
  // )
  // addedAccounts.push({
  //   address: RAYDIUM_POOL_FEE_RECEIVER,
  //   info: raydiumPoolFeeReceiver,
  // })

  return {
    ...newAccounts,
    addedAccounts: wa,
  };
}

export async function getProvider() {
  // In Bankrun, the default wallet is pre-funded with SOL
  // We can use it directly - no need to create a new wallet
  const wallet = loadWallet();

  // Create mock token accounts with fixed addresses for testing
  const mockTokenMint = new PublicKey("11111111111111111111111111111112");

  // Calculate the protocol config PDA to derive the correct ATA addresses
  const [protocolConfigPDA] = PublicKey.findProgramAddressSync(
    [Buffer.from("config")],
    new PublicKey("EwkRYuQXEAfiZ3aKs6Dfoi1r6FKch4voNQEKQkRvniW1") // Program ID
  );

  const userTokenAccount = getAssociatedTokenAddressSync(
    mockTokenMint,
    wallet.publicKey
  );
  const programTokenAccount = getAssociatedTokenAddressSync(
    mockTokenMint,
    protocolConfigPDA,
    true // allowOwnerOffCurve
  );

  const newAccounts = { wallet };
  // Set the balance of testAccount
  const wa = Object.values(newAccounts).map((acc) => ({
    address: acc.publicKey,
    info: {
      lamports: 1000_000_000_000,
      executable: false,
      owner: anchor.web3.SystemProgram.programId,
      data: Buffer.alloc(0),
    },
  }));

  // Create properly initialized SPL token mint account data
  const mintData = Buffer.alloc(82);
  mintData.writeUInt32LE(1, 0); // mint_authority_option (1 = Some)
  wallet.publicKey.toBuffer().copy(mintData, 4); // mint_authority
  mintData.writeBigUInt64LE(BigInt(1000000000000), 36); // supply (1M tokens with 6 decimals)
  mintData.writeUInt8(6, 44); // decimals
  mintData.writeUInt8(1, 45); // is_initialized
  mintData.writeUInt32LE(0, 46); // freeze_authority_option (0 = None)

  wa.push({
    address: mockTokenMint,
    info: {
      lamports: 1000_000_000,
      executable: false,
      owner: TOKEN_PROGRAM_ID,
      data: mintData,
    },
  });

  // Create properly initialized user token account data
  const userTokenData = Buffer.alloc(165);
  mockTokenMint.toBuffer().copy(userTokenData, 0); // mint
  wallet.publicKey.toBuffer().copy(userTokenData, 32); // owner
  userTokenData.writeBigUInt64LE(BigInt(500000000000), 64); // amount (500k tokens)
  userTokenData.writeUInt32LE(0, 72); // delegate_option (0 = None)
  userTokenData.writeUInt8(1, 108); // state (1 = initialized)
  userTokenData.writeUInt32LE(0, 109); // is_native_option (0 = None)
  userTokenData.writeBigUInt64LE(BigInt(0), 113); // delegated_amount
  userTokenData.writeUInt32LE(0, 121); // close_authority_option (0 = None)

  wa.push({
    address: userTokenAccount,
    info: {
      lamports: 2039280,
      executable: false,
      owner: TOKEN_PROGRAM_ID,
      data: userTokenData,
    },
  });

  // Create properly initialized program token account data (initially empty)
  const programTokenData = Buffer.alloc(165);
  mockTokenMint.toBuffer().copy(programTokenData, 0); // mint
  // Program token account authority is the protocol config PDA
  protocolConfigPDA.toBuffer().copy(programTokenData, 32); // owner
  programTokenData.writeBigUInt64LE(BigInt(0), 64); // amount (0 initially)
  programTokenData.writeUInt32LE(0, 72); // delegate_option (0 = None)
  programTokenData.writeUInt8(1, 108); // state (1 = initialized)
  programTokenData.writeUInt32LE(0, 109); // is_native_option (0 = None)
  programTokenData.writeBigUInt64LE(BigInt(0), 113); // delegated_amount
  programTokenData.writeUInt32LE(0, 121); // close_authority_option (0 = None)

  wa.push({
    address: programTokenAccount,
    info: {
      lamports: 2039280,
      executable: false,
      owner: TOKEN_PROGRAM_ID,
      data: programTokenData,
    },
  });

  // Debug: ensure bankrun picks up latest program artifact.
  try {
    const soPath = "./target/deploy/molpha.so";
    const bytes = fs.readFileSync(soPath);
    const sha = crypto.createHash("sha256").update(bytes).digest("hex");
    // eslint-disable-next-line no-console
    console.log("molpha.so sha256", sha, "bytes", bytes.length);
  } catch (e) {
    // eslint-disable-next-line no-console
    console.log("molpha.so missing/unreadable", e);
  }

  const context = await startAnchor(
    ".",
    [
      {
        name: "molpha",
        programId: getProgramId(),
      },
    ],
    wa
  );
  const provider = new BankrunProvider(context);
  provider.wallet = new Wallet(wallet.payer);

  return provider;
}

export function loadWallet(): anchor.Wallet {
  const walletPath = `${require("os").homedir()}/.config/solana/id.json`;
  const secretKeyString = fs.readFileSync(walletPath, "utf8");
  const secretKey = Uint8Array.from(JSON.parse(secretKeyString));
  const keypair = Keypair.fromSecretKey(secretKey);
  return new anchor.Wallet(keypair);
}

export async function setupTestContext(): Promise<TestContext> {
  const provider = await getProvider();
  anchor.setProvider(provider);

  const molphaProgram = anchor.workspace.Molpha as Program<Molpha>;
  const authority = provider.wallet as anchor.Wallet;

  const [nodeRegistryPDA] = PublicKey.findProgramAddressSync(
    [Buffer.from("node-registry")],
    molphaProgram.programId
  );

  const [protocolConfigPDA] = PublicKey.findProgramAddressSync(
    [Buffer.from("config")],
    molphaProgram.programId
  );

  const nodes: Keypair[] = [];
  const MAX_NODES = 32;

  for (let i = 0; i < MAX_NODES; i++) {
    nodes.push(Keypair.generate());
  }

  // For testing, we'll use fixed addresses that we created in getProvider
  // This avoids the complexity of creating real SPL token accounts in bankrun
  const underlyingTokenMint = new PublicKey("11111111111111111111111111111112"); // Mock mint
  const userTokenAccount = getAssociatedTokenAddressSync(
    underlyingTokenMint,
    authority.publicKey
  );
  const programTokenAccount = getAssociatedTokenAddressSync(
    underlyingTokenMint,
    protocolConfigPDA,
    true // allowOwnerOffCurve
  );

  return {
    molphaProgram,
    nodeRegistryPDA,
    protocolConfigPDA,
    nodes,
    authority,
    provider,
    underlyingTokenMint,
    programTokenAccount,
    userTokenAccount,
  };
}

export async function initializeProtocol(ctx: TestContext): Promise<void> {
  try {
    // Initialize protocol config
    await ctx.molphaProgram.methods
      .initialize()
      .accountsPartial({
        protocolConfig: ctx.protocolConfigPDA,
        underlyingToken: ctx.underlyingTokenMint,
        programTokenAccount: ctx.programTokenAccount,
        authority: ctx.authority.publicKey,
        systemProgram: SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
      })
      .rpc();

    // Set a default security deposit of 1000 tokens (1000 * 10^6 = 1000000000 with 6 decimals)
    await ctx.molphaProgram.methods
      .updateProtocolConfig({
        basePricePerSecondScaled: null,
        frequencyCoefficient: null,
        signersCoefficient: null,
        rewardPercentage: null,
        nodeSecurityDeposit: new BN(1000000000), // 1000 tokens
        registryXAgg: null,
        signaturesRequired: null,
        nodeCount: null,
        redundancyBuffer: null,
      })
      .accountsPartial({
        protocolConfig: ctx.protocolConfigPDA,
        authority: ctx.authority.publicKey,
      })
      .rpc();
  } catch (e) {
    // Ignore if already initialized
    console.log(
      "Protocol initialization error (may be already initialized):",
      e
    );
  }
}

// Helper function to create feed parameters
export function createFeedParams(
  sourceId: string,
  signaturesRequired: number,
  feedType: { public: {} } | { personal: {} }
) {
  // Convert hex string to byte array properly and pad to 32 bytes
  const hexString = sourceId.startsWith("0x") ? sourceId.slice(2) : sourceId;

  // Pad to 32 bytes (64 hex characters) if needed
  const paddedHexString = hexString.padStart(64, "0");
  const paddedSourceIdBytes = Array.from(Buffer.from(paddedHexString, "hex"));

  return {
    name: sourceId.slice(0, 32), // Truncate name to 32 characters for PDA generation
    sourceId: paddedSourceIdBytes,
    feedType: feedType,
    minSignaturesThreshold: 2,
    frequency: new anchor.BN(300), // 5 minutes as BN
    ipfsCid: "QmTestCID123456789",
  };
}

export function createTestDataSourceInfo(dataSourceType: number, name: string) {
  const metadataHash = new Uint8Array(32);
  metadataHash.fill(1);
  // Use proper discriminated union format for the enum
  const dataSourceTypeEnum =
    dataSourceType === 1 ? { personal: {} } : { public: {} };

  return {
    dataSourceType: dataSourceTypeEnum,
    metadataHash: Array.from(metadataHash),
    name: name,
  };
}

export function getDataSourcePda(
  programId: PublicKey,
  authority: PublicKey,
  name: string,
  dataSourceType: number
) {
  const { toSeed } = require("./helpers/seeds");
  return PublicKey.findProgramAddressSync(
    [
      Buffer.from("data_source"),
      authority.toBuffer(),
      toSeed(name),
      Buffer.from([dataSourceType]),
    ],
    programId
  );
}

export function getFeedPda(
  programId: PublicKey,
  authority: PublicKey,
  name: string,
  feedType: number
) {
  const { toSeed } = require("./helpers/seeds");
  return PublicKey.findProgramAddressSync(
    [
      Buffer.from("feed"),
      authority.toBuffer(),
      toSeed(name),
      Buffer.from([feedType]),
    ],
    programId
  );
}
