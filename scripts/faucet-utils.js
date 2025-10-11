const anchor = require("@coral-xyz/anchor");
const {
  PublicKey,
  SystemProgram,
  Keypair,
  SYSVAR_RENT_PUBKEY,
} = require("@solana/web3.js");
const {
  getAssociatedTokenAddressSync,
  getAccount,
  TOKEN_PROGRAM_ID,
  ASSOCIATED_TOKEN_PROGRAM_ID,
} = require("@solana/spl-token");

// Load the faucet program
function loadFaucetProgram() {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const faucetProgram = anchor.workspace.Faucet;
  return { provider, faucetProgram };
}

// Initialize a faucet with a new token mint
async function initializeFaucet(
  authority,
  amountPerRequest = 1000,
  cooldownSeconds = 300,
  tokenDecimals = 6,
  tokenName = "Test Token",
  tokenSymbol = "TEST"
) {
  const { faucetProgram } = loadFaucetProgram();

  // Generate a new token mint keypair
  const tokenMint = Keypair.generate();

  const [faucetConfigPDA] = PublicKey.findProgramAddressSync(
    [Buffer.from("faucet_config"), tokenMint.publicKey.toBuffer()],
    faucetProgram.programId
  );

  const amountBN = new anchor.BN(
    amountPerRequest * Math.pow(10, tokenDecimals)
  );
  const cooldownBN = new anchor.BN(cooldownSeconds);

  const tx = await faucetProgram.methods
    .initialize(amountBN, cooldownBN, tokenDecimals, tokenName, tokenSymbol)
    .accountsPartial({
      faucetConfig: faucetConfigPDA,
      tokenMint: tokenMint.publicKey,
      authority: authority.publicKey,
      systemProgram: SystemProgram.programId,
      tokenProgram: TOKEN_PROGRAM_ID,
      rent: SYSVAR_RENT_PUBKEY,
    })
    .signers([authority, tokenMint])
    .rpc();

  console.log("✅ Initialized faucet:");
  console.log("  Token Mint:", tokenMint.publicKey.toString());
  console.log("  Faucet Config:", faucetConfigPDA.toString());
  console.log("  Amount per request:", amountPerRequest, tokenSymbol);
  console.log("  Cooldown:", cooldownSeconds, "seconds");
  console.log("  Transaction:", tx);

  return {
    tokenMint: tokenMint.publicKey,
    faucetConfigPDA,
    tokenMintKeypair: tokenMint, // Return keypair for saving to file
  };
}

// Request tokens from faucet
async function requestTokens(tokenMint, user) {
  const { provider, faucetProgram } = loadFaucetProgram();

  const [faucetConfigPDA] = PublicKey.findProgramAddressSync(
    [Buffer.from("faucet_config"), tokenMint.toBuffer()],
    faucetProgram.programId
  );

  const userTokenAccount = getAssociatedTokenAddressSync(
    tokenMint,
    user.publicKey
  );

  const [userCooldownPDA] = PublicKey.findProgramAddressSync(
    [
      Buffer.from("user_cooldown"),
      user.publicKey.toBuffer(),
      tokenMint.toBuffer(),
    ],
    faucetProgram.programId
  );

  try {
    const tx = await faucetProgram.methods
      .requestTokens()
      .accountsPartial({
        faucetConfig: faucetConfigPDA,
        userCooldown: userCooldownPDA,
        tokenMint: tokenMint,
        userTokenAccount: userTokenAccount,
        user: user.publicKey,
        systemProgram: SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
      })
      .signers([user])
      .rpc();

    console.log("✅ Tokens requested successfully:");
    console.log("  User:", user.publicKey.toString());
    console.log("  Transaction:", tx);

    // Check user's token balance
    try {
      const userAccount = await getAccount(
        provider.connection,
        userTokenAccount
      );
      console.log("  New token balance:", userAccount.amount.toString());
    } catch (err) {
      console.log("  Token account will be created on first request");
    }

    return tx;
  } catch (error) {
    if (error.message.includes("CooldownActive")) {
      console.log(
        "❌ Cooldown period is still active. Please wait before requesting again."
      );

      // Get cooldown info
      try {
        const cooldownAccount = await faucetProgram.account.userCooldown.fetch(
          userCooldownPDA
        );
        const faucetConfig = await faucetProgram.account.faucetConfig.fetch(
          faucetConfigPDA
        );
        const currentTime = Math.floor(Date.now() / 1000);
        const timeRemaining =
          cooldownAccount.lastRequest.toNumber() +
          faucetConfig.cooldownSeconds.toNumber() -
          currentTime;
        console.log("  Time remaining:", Math.max(0, timeRemaining), "seconds");
      } catch (e) {
        console.log("  Could not fetch cooldown details");
      }
    } else {
      console.error("❌ Error requesting tokens:", error.message);
    }
    throw error;
  }
}

// Get faucet info
async function getFaucetInfo(tokenMint) {
  const { provider, faucetProgram } = loadFaucetProgram();

  const [faucetConfigPDA] = PublicKey.findProgramAddressSync(
    [Buffer.from("faucet_config"), tokenMint.toBuffer()],
    faucetProgram.programId
  );

  try {
    const faucetConfig = await faucetProgram.account.faucetConfig.fetch(
      faucetConfigPDA
    );

    console.log("📊 Faucet Information:");
    console.log("  Token Mint:", faucetConfig.tokenMint.toString());
    console.log("  Authority:", faucetConfig.authority.toString());
    console.log(
      "  Amount per request:",
      faucetConfig.amountPerRequest.toString()
    );
    console.log("  Cooldown seconds:", faucetConfig.cooldownSeconds.toString());
    console.log("  Token decimals:", faucetConfig.tokenDecimals);
    console.log("  Is active:", faucetConfig.isActive);
    console.log("  Faucet Config PDA:", faucetConfigPDA.toString());

    return faucetConfig;
  } catch (error) {
    console.error("❌ Faucet not found or error fetching info:", error.message);
    throw error;
  }
}

// Get user cooldown info
async function getUserCooldown(tokenMint, userPublicKey) {
  const { faucetProgram } = loadFaucetProgram();

  const [faucetConfigPDA] = PublicKey.findProgramAddressSync(
    [Buffer.from("faucet_config"), tokenMint.toBuffer()],
    faucetProgram.programId
  );

  const [userCooldownPDA] = PublicKey.findProgramAddressSync(
    [
      Buffer.from("user_cooldown"),
      userPublicKey.toBuffer(),
      tokenMint.toBuffer(),
    ],
    faucetProgram.programId
  );

  try {
    const cooldownAccount = await faucetProgram.account.userCooldown.fetch(
      userCooldownPDA
    );
    const faucetConfig = await faucetProgram.account.faucetConfig.fetch(
      faucetConfigPDA
    );

    const currentTime = Math.floor(Date.now() / 1000);
    const lastRequest = cooldownAccount.lastRequest.toNumber();
    const cooldownSeconds = faucetConfig.cooldownSeconds.toNumber();
    const timeRemaining = Math.max(
      0,
      lastRequest + cooldownSeconds - currentTime
    );
    const canRequest = timeRemaining === 0;

    console.log("⏰ User Cooldown Status:");
    console.log("  User:", userPublicKey.toString());
    console.log("  Last request:", new Date(lastRequest * 1000).toISOString());
    console.log("  Can request now:", canRequest);
    console.log("  Time remaining:", timeRemaining, "seconds");

    return {
      lastRequest,
      canRequest,
      timeRemaining,
      cooldownSeconds,
    };
  } catch (error) {
    console.log("ℹ️  User has not requested tokens yet");
    return {
      lastRequest: 0,
      canRequest: true,
      timeRemaining: 0,
      cooldownSeconds: 0,
    };
  }
}

// Get user token balance
async function getUserTokenBalance(tokenMint, userPublicKey) {
  const { provider } = loadFaucetProgram();

  const userTokenAccount = getAssociatedTokenAddressSync(
    tokenMint,
    userPublicKey
  );

  try {
    const account = await getAccount(provider.connection, userTokenAccount);
    const balance = account.amount.toString();

    console.log("💰 Token Balance:");
    console.log("  User:", userPublicKey.toString());
    console.log("  Token Account:", userTokenAccount.toString());
    console.log("  Balance:", balance);

    return balance;
  } catch (error) {
    console.log("ℹ️  User token account does not exist yet");
    return "0";
  }
}

// Save token mint keypair to file
function saveTokenMintKeypair(tokenMintKeypair, filename) {
  const fs = require("fs");
  const keypairArray = Array.from(tokenMintKeypair.secretKey);
  fs.writeFileSync(filename, JSON.stringify(keypairArray));
  console.log("💾 Token mint keypair saved to:", filename);
}

// Load token mint keypair from file
function loadTokenMintKeypair(filename) {
  const fs = require("fs");
  try {
    const keypairArray = JSON.parse(fs.readFileSync(filename, "utf8"));
    const keypair = Keypair.fromSecretKey(new Uint8Array(keypairArray));
    console.log("📂 Token mint keypair loaded from:", filename);
    console.log("  Public Key:", keypair.publicKey.toString());
    return keypair;
  } catch (error) {
    console.error("❌ Error loading token mint keypair:", error.message);
    throw error;
  }
}

module.exports = {
  loadFaucetProgram,
  initializeFaucet,
  requestTokens,
  getFaucetInfo,
  getUserCooldown,
  getUserTokenBalance,
  saveTokenMintKeypair,
  loadTokenMintKeypair,
};
