#!/usr/bin/env node

const anchor = require("@coral-xyz/anchor");
const {
  getAssociatedTokenAddressSync,
  TOKEN_PROGRAM_ID,
  ASSOCIATED_TOKEN_PROGRAM_ID,
} = require("@solana/spl-token");

async function main() {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.Molpha;

  // Derive the protocol config PDA
  const [protocolConfigPDA] = anchor.web3.PublicKey.findProgramAddressSync(
    [Buffer.from("config")],
    program.programId
  );

  try {
    // Get protocol config to check security deposit and underlying token
    const protocolConfig = await program.account.protocolConfig.fetch(
      protocolConfigPDA
    );
    const underlyingToken = protocolConfig.underlyingToken;
    const securityDeposit = protocolConfig.nodeSecurityDeposit;

    // Get operator's token account
    const nodeOperatorTokenAccount = getAssociatedTokenAddressSync(
      underlyingToken,
      provider.wallet.publicKey
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

    console.log("💰 Balance Information:");
    console.log("=====================================");
    console.log("Wallet Address:", provider.wallet.publicKey.toString());
    console.log("SOL Balance:", solBalanceSOL.toFixed(6), "SOL");
    console.log("");
    console.log("Token Information:");
    console.log("Underlying Token:", underlyingToken.toString());
    console.log("Token Account:", nodeOperatorTokenAccount.toString());
    console.log("Token Balance:", tokenBalance.toString(), "tokens");
    console.log(
      "Required Security Deposit:",
      securityDeposit.toString(),
      "tokens"
    );
    console.log("");

    if (tokenBalance >= securityDeposit) {
      console.log("✅ You have enough tokens for node registration!");
      console.log(`   Available: ${tokenBalance.toString()} tokens`);
      console.log(`   Required: ${securityDeposit.toString()} tokens`);
      console.log(
        `   Surplus: ${(tokenBalance - securityDeposit).toString()} tokens`
      );
    } else {
      console.log("❌ Insufficient tokens for node registration");
      console.log(`   Available: ${tokenBalance.toString()} tokens`);
      console.log(`   Required: ${securityDeposit.toString()} tokens`);
      console.log(
        `   Missing: ${(securityDeposit - tokenBalance).toString()} tokens`
      );
      console.log("");
      console.log("💡 To get tokens: transfer tokens to your account");
    }
  } catch (e) {
    console.error("❌ Error checking balance:", e.message);
    if (e.logs) {
      console.error("Transaction logs:", e.logs);
    }
    process.exit(1);
  }
}

main().catch(console.error);
