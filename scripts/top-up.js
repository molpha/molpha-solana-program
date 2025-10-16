#!/usr/bin/env node

// ⚠️ DEPRECATED: This script is for an older version of the program.
// Subscriptions are now integrated into feeds and are extended via the
// extend_subscription instruction. Use subscribe.js instead.
// See: scripts/subscribe.js

const anchor = require("@coral-xyz/anchor");
const { SystemProgram } = require("@solana/web3.js");

async function main() {
  if (!process.argv[2]) {
    console.error("❌ Error: Feed ID is required");
    console.log(
      "Usage: node top-up.js <FEED_ID> [CONSUMER_PUBKEY_OR_AMOUNT] [AMOUNT]"
    );
    console.log(
      "  CONSUMER_PUBKEY_OR_AMOUNT: consumer public key OR amount in lamports"
    );
    console.log(
      "  AMOUNT: amount in lamports (used when CONSUMER_PUBKEY is specified)"
    );
    console.log("  Examples:");
    console.log(
      '    node top-up.js "ETH/USD" 5000                     # Top up 5000 lamports for wallet'
    );
    console.log(
      '    node top-up.js "ETH/USD" <CONSUMER_PUBKEY> 5000   # Top up 5000 lamports for consumer'
    );
    process.exit(1);
  }

  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.MolphaFeed;

  // Parse command line arguments
  const feedId = process.argv[2];

  // Smart parameter parsing: if arg3 is a number, treat it as amount, otherwise as consumer
  let consumer = provider.wallet.publicKey;
  let amount = 1000;

  if (process.argv[3]) {
    const arg3 = process.argv[3].trim();
    if (/^\d+$/.test(arg3)) {
      // arg3 is a number, so it's the amount
      amount = parseInt(arg3);
    } else if (arg3 !== "") {
      // arg3 is not a number and not empty, so it's the consumer pubkey
      consumer = new anchor.web3.PublicKey(arg3);
      amount = parseInt(process.argv[4]) || 1000;
    }
  }

  const [feedAccountPDA] = anchor.web3.PublicKey.findProgramAddressSync(
    [
      Buffer.from("feed"),
      provider.wallet.publicKey.toBuffer(),
      Buffer.from(feedId),
    ],
    program.programId
  );

  const [subscriptionPDA] = anchor.web3.PublicKey.findProgramAddressSync(
    [
      Buffer.from("subscription"),
      consumer.toBuffer(),
      feedAccountPDA.toBuffer(),
    ],
    program.programId
  );

  try {
    const txSignature = await program.methods
      .topUp(new anchor.BN(amount))
      .accounts({
        subscriptionAccount: subscriptionPDA,
        owner: provider.wallet.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    console.log("✅ Subscription topped up successfully!");
    console.log("Feed ID:", feedId);
    console.log("Consumer:", consumer.toString());
    console.log("Amount:", amount, "lamports");
    console.log("Subscription PDA:", subscriptionPDA.toString());
    console.log("Transaction signature:", txSignature);
  } catch (e) {
    console.error("❌ Error topping up subscription:", e.message);
    process.exit(1);
  }
}

main().catch(console.error);
