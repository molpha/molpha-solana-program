#!/usr/bin/env node

const anchor = require("@coral-xyz/anchor");
const {
  PublicKey,
  SystemProgram,
  Keypair,
  SYSVAR_RENT_PUBKEY,
} = require("@solana/web3.js");
const {
  TOKEN_PROGRAM_ID,
  ASSOCIATED_TOKEN_PROGRAM_ID,
} = require("@solana/spl-token");

async function testFaucetOnDevnet() {
  console.log("🧪 Testing Faucet on Devnet\n");

  try {
    // Set up provider for devnet
    const connection = new anchor.web3.Connection(
      "https://api.devnet.solana.com",
      "confirmed"
    );
    const wallet = anchor.AnchorProvider.env().wallet;
    const provider = new anchor.AnchorProvider(connection, wallet, {
      commitment: "confirmed",
    });
    anchor.setProvider(provider);

    // Load the program
    const programId = new PublicKey(
      "EMSa6X33J25Ln6d5PkBpyZh69i4yCeF5xDTtyJWMBaSd"
    );
    const idl = await anchor.Program.fetchIdl(programId, provider);
    const program = new anchor.Program(idl, programId, provider);

    console.log("✅ Connected to devnet");
    console.log("📍 Program ID:", programId.toString());
    console.log("💰 Wallet:", wallet.publicKey.toString());
    console.log(
      "💸 Balance:",
      (await connection.getBalance(wallet.publicKey)) / 1e9,
      "SOL"
    );

    // Test parameters
    const amountPerRequest = new anchor.BN(1000 * 1e6); // 1000 tokens
    const cooldownSeconds = new anchor.BN(60); // 1 minute
    const tokenDecimals = 6;
    const tokenName = "Devnet Test Token";
    const tokenSymbol = "DTT";

    // Generate a new token mint keypair
    const tokenMint = Keypair.generate();

    const [faucetConfigPDA] = PublicKey.findProgramAddressSync(
      [Buffer.from("faucet_config"), tokenMint.publicKey.toBuffer()],
      programId
    );

    console.log("\n🏗️  Initializing faucet...");
    console.log("🪙 Token Mint:", tokenMint.publicKey.toString());
    console.log("⚙️  Faucet Config PDA:", faucetConfigPDA.toString());

    // Initialize the faucet
    const initTx = await program.methods
      .initialize(
        amountPerRequest,
        cooldownSeconds,
        tokenDecimals,
        tokenName,
        tokenSymbol
      )
      .accountsPartial({
        faucetConfig: faucetConfigPDA,
        tokenMint: tokenMint.publicKey,
        authority: wallet.publicKey,
        systemProgram: SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
        rent: SYSVAR_RENT_PUBKEY,
      })
      .signers([tokenMint])
      .rpc();

    console.log("✅ Faucet initialized!");
    console.log("📝 Transaction:", initTx);

    // Fetch and display faucet config
    const faucetConfig = await program.account.faucetConfig.fetch(
      faucetConfigPDA
    );
    console.log("\n📊 Faucet Configuration:");
    console.log("  Authority:", faucetConfig.authority.toString());
    console.log("  Token Mint:", faucetConfig.tokenMint.toString());
    console.log(
      "  Amount per request:",
      faucetConfig.amountPerRequest.toString()
    );
    console.log("  Cooldown seconds:", faucetConfig.cooldownSeconds.toString());
    console.log("  Token decimals:", faucetConfig.tokenDecimals);
    console.log("  Is active:", faucetConfig.isActive);

    console.log("\n🎉 Faucet test completed successfully on devnet!");
    console.log("\n📋 Summary:");
    console.log(`  Program ID: ${programId.toString()}`);
    console.log(`  Token Mint: ${tokenMint.publicKey.toString()}`);
    console.log(`  Faucet Config: ${faucetConfigPDA.toString()}`);
    console.log(`  You can now use the CLI to request tokens:`);
    console.log(
      `  ./scripts/faucet request -m ${tokenMint.publicKey.toString()} -u ~/.config/solana/id.json`
    );
  } catch (error) {
    console.error("❌ Test failed:", error.message);
    if (error.logs) {
      console.log("📋 Transaction logs:");
      error.logs.forEach((log) => console.log("  ", log));
    }
    process.exit(1);
  }
}

// Run if called directly
if (require.main === module) {
  testFaucetOnDevnet();
}

module.exports = { testFaucetOnDevnet };
