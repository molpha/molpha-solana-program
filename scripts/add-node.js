#!/usr/bin/env node

const anchor = require("@coral-xyz/anchor");
const {
  getAssociatedTokenAddressSync,
  TOKEN_PROGRAM_ID,
  ASSOCIATED_TOKEN_PROGRAM_ID,
} = require("@solana/spl-token");
const { SYSVAR_CLOCK_PUBKEY, SYSVAR_RENT_PUBKEY } = require("@solana/web3.js");

async function main() {
  if (!process.argv[2]) {
    console.error("❌ Error: Node public key is required");
    console.log("Usage: node add-node.js <NODE_PUBKEY> [EVM_PUBKEY_HEX] [URL]");
    console.log(
      "Example: node add-node.js 11111111111111111111111111111112 0x0404... https://node.example.com"
    );
    process.exit(1);
  }

  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.Molpha;

  // Derive the protocol config PDA
  const [protocolConfigPDA] = anchor.web3.PublicKey.findProgramAddressSync(
    [Buffer.from("config")],
    program.programId
  );

  // Helper function to derive Wormhole accounts
  function getWormholeAccounts(protocolConfig, protocolConfigPDA) {
    const wormholeBridgeProgram = protocolConfig.wormholeBridgeProgram;
    const isWormholeConfigured =
      wormholeBridgeProgram &&
      !wormholeBridgeProgram.equals(anchor.web3.PublicKey.default);

    // Use configured bridge program or default to SystemProgram for mock accounts
    const bridgeProgram = isWormholeConfigured
      ? wormholeBridgeProgram
      : anchor.web3.SystemProgram.programId;

    // Derive Wormhole bridge config PDA
    const [wormholeBridge] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("Bridge")],
      bridgeProgram
    );

    // Derive Wormhole sequence PDA (emitter is protocol config PDA)
    const [wormholeSequence] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("Sequence"), protocolConfigPDA.toBuffer()],
      bridgeProgram
    );

    // Derive Wormhole fee collector PDA
    const [wormholeFeeCollector] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("fee_collector")],
      bridgeProgram
    );

    // Generate a fresh keypair for the message account (signer)
    const wormholeMessage = anchor.web3.Keypair.generate();

    return {
      wormholeBridgeProgram: bridgeProgram,
      wormholeBridge,
      wormholeMessage,
      wormholeEmitter: protocolConfigPDA, // Protocol config is the emitter
      wormholeSequence,
      wormholeFeeCollector,
      clock: SYSVAR_CLOCK_PUBKEY,
      rent: SYSVAR_RENT_PUBKEY,
      isWormholeConfigured,
    };
  }

  try {
    const nodePubkey = new anchor.web3.PublicKey(process.argv[2]);

    // Parse EVM pubkey (compressed secp256k1 - 33 bytes) or use default
    let evmPubkey;
    if (process.argv[3]) {
      const hexString = process.argv[3].startsWith("0x")
        ? process.argv[3].slice(2)
        : process.argv[3];
      evmPubkey = Buffer.from(hexString, "hex");
      if (evmPubkey.length !== 33) {
        console.error(
          "❌ Error: EVM pubkey must be 33 bytes (compressed secp256k1)"
        );
        process.exit(1);
      }
    } else {
      // Default EVM pubkey (all zeros) if not provided
      evmPubkey = Buffer.alloc(33, 0);
      console.warn(
        "⚠️  Warning: No EVM pubkey provided, using default (all zeros)"
      );
    }

    // Get URL or use default
    const url =
      process.argv[4] ||
      `https://node-${nodePubkey.toString().slice(0, 8)}.example.com`;

    // Derive the node PDA
    const [nodePDA] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("node"), nodePubkey.toBuffer()],
      program.programId
    );

    // Get protocol config to check security deposit and underlying token
    const protocolConfig = await program.account.protocolConfig.fetch(
      protocolConfigPDA
    );
    const underlyingToken = protocolConfig.underlyingToken;
    const securityDeposit = protocolConfig.nodeSecurityDeposit;

    // Get Wormhole accounts
    const wormholeAccounts = getWormholeAccounts(
      protocolConfig,
      protocolConfigPDA
    );

    // Get operator's token account
    const nodeOperatorTokenAccount = getAssociatedTokenAddressSync(
      underlyingToken,
      provider.wallet.publicKey
    );

    // Get node's security deposit account
    const nodeSecurityDepositAccount = getAssociatedTokenAddressSync(
      underlyingToken,
      nodePDA,
      true // allowOwnerOffCurve
    );

    // Get SOL balance
    const solBalance = await provider.connection.getBalance(
      provider.wallet.publicKey
    );
    const solBalanceSOL = solBalance / anchor.web3.LAMPORTS_PER_SOL;

    // Get token balance
    let tokenBalance = 0;
    try {
      const tokenAccountInfo = await provider.connection.getTokenAccountBalance(
        nodeOperatorTokenAccount
      );
      tokenBalance = parseInt(tokenAccountInfo.value.amount);
    } catch (e) {
      // Token account doesn't exist or other error
      tokenBalance = 0;
    }

    console.log("🔍 Registering node with the following details:");
    console.log("Node pubkey:", nodePubkey.toString());
    console.log("Node PDA:", nodePDA.toString());
    console.log("Protocol Config PDA:", protocolConfigPDA.toString());
    console.log("Authority:", provider.wallet.publicKey.toString());
    console.log("Underlying Token:", underlyingToken.toString());
    console.log("Security Deposit:", securityDeposit.toString(), "tokens");
    console.log("EVM Pubkey:", evmPubkey.toString("hex"));
    console.log("Node URL:", url);
    if (wormholeAccounts.isWormholeConfigured) {
      console.log("🌉 Wormhole: Enabled");
      console.log(
        "   Bridge Program:",
        wormholeAccounts.wormholeBridgeProgram.toString()
      );
      console.log("   Target Chains:", protocolConfig.targetChainIds || []);
    } else {
      console.log("🌉 Wormhole: Disabled (no cross-chain messaging)");
    }
    console.log("");
    console.log("💰 Balance Information:");
    console.log("SOL Balance:", solBalanceSOL.toFixed(6), "SOL");
    console.log("Token Balance:", tokenBalance.toString(), "tokens");
    console.log(
      "Required Security Deposit:",
      securityDeposit.toString(),
      "tokens"
    );
    console.log("Token Account:", nodeOperatorTokenAccount.toString());
    console.log("");

    // Check if node already exists
    try {
      await program.account.node.fetch(nodePDA);
      console.error("❌ Error: Node already exists");
      process.exit(1);
    } catch (e) {
      // Node doesn't exist, which is good for registering
    }

    // Check if you have enough tokens for security deposit
    if (tokenBalance < securityDeposit) {
      console.error(
        "❌ Error: Insufficient token balance for security deposit"
      );
      console.error(`Required: ${securityDeposit.toString()} tokens`);
      console.error(`Available: ${tokenBalance.toString()} tokens`);
      console.error(
        `Missing: ${(securityDeposit - tokenBalance).toString()} tokens`
      );
      console.error("");
      console.error("💡 To get tokens:");
      console.error("1. Use the faucet: node scripts/faucet.js");
      console.error("2. Or transfer tokens to your account");
      process.exit(1);
    }

    // Check operator token account balance
    try {
      const tokenAccountInfo = await provider.connection.getAccountInfo(
        nodeOperatorTokenAccount
      );
      if (!tokenAccountInfo) {
        console.error(
          "❌ Error: Operator token account not found. Please fund your account with tokens first."
        );
        process.exit(1);
      }
    } catch (e) {
      console.error(
        "❌ Error: Could not check operator token account:",
        e.message
      );
      process.exit(1);
    }

    const txSignature = await program.methods
      .registerNode(nodePubkey, evmPubkey, url)
      .accounts({
        node: nodePDA,
        protocolConfig: protocolConfigPDA,
        underlyingToken: underlyingToken,
        nodeOperatorTokenAccount: nodeOperatorTokenAccount,
        nodeSecurityDepositAccount: nodeSecurityDepositAccount,
        authority: provider.wallet.publicKey,
        systemProgram: anchor.web3.SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        // // Wormhole accounts (required even if Wormhole is disabled)
        // wormholeBridgeProgram: wormholeAccounts.wormholeBridgeProgram,
        // wormholeBridge: wormholeAccounts.wormholeBridge,
        // wormholeMessage: wormholeAccounts.wormholeMessage.publicKey,
        // wormholeEmitter: wormholeAccounts.wormholeEmitter,
        // wormholeSequence: wormholeAccounts.wormholeSequence,
        // wormholeFeeCollector: wormholeAccounts.wormholeFeeCollector,
        // clock: wormholeAccounts.clock,
        // rent: wormholeAccounts.rent,
      })
      // .signers([wormholeAccounts.wormholeMessage])
      .rpc();

    console.log("✅ Node registered successfully!");
    console.log("Node pubkey:", nodePubkey.toString());
    console.log("Node PDA:", nodePDA.toString());
    console.log("Transaction signature:", txSignature);

    // Verify the node was added
    try {
      const nodeAccount = await program.account.node.fetch(nodePDA);

      console.log("");
      console.log("📊 Node Details:");
      console.log("- Authority:", nodeAccount.authority.toString());
      console.log("- Node Pubkey:", nodeAccount.nodePubkey.toString());
      console.log(
        "- EVM Pubkey:",
        Buffer.from(nodeAccount.nodePubEvm).toString("hex")
      );
      console.log("- Status:", JSON.stringify(nodeAccount.status));
      console.log(
        "- Security Deposit:",
        nodeAccount.securityDeposit.toString()
      );
    } catch (e) {
      console.warn(
        "⚠️  Warning: Could not fetch node details for verification:",
        e.message
      );
    }
  } catch (e) {
    console.error("❌ Error registering node:", e.message);
    if (e.logs) {
      console.error("Transaction logs:", e.logs);
    }
    process.exit(1);
  }
}

main().catch(console.error);
