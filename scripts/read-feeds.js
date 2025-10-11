#!/usr/bin/env node

const anchor = require("@coral-xyz/anchor");
const { PublicKey } = require("@solana/web3.js");
const fs = require("fs");

// Setup environment variables if not set
function setupEnvironment() {
  if (!process.env.ANCHOR_PROVIDER_URL) {
    process.env.ANCHOR_PROVIDER_URL = "http://127.0.0.1:8899";
  }

  if (!process.env.ANCHOR_WALLET) {
    const defaultWallet = require("os").homedir() + "/.config/solana/id.json";
    if (fs.existsSync(defaultWallet)) {
      process.env.ANCHOR_WALLET = defaultWallet;
    }
  }
}

function formatAnswer(answer) {
  if (!answer || answer.value.every((byte) => byte === 0)) {
    return "No data";
  }

  // Convert bytes to hex string
  const hexValue = Buffer.from(answer.value).toString("hex");

  // Handle timestamp safely
  let timestamp;
  if (answer.timestamp && answer.timestamp > 0) {
    try {
      timestamp = new Date(answer.timestamp * 1000).toISOString();
    } catch (e) {
      timestamp = "Invalid timestamp";
    }
  } else {
    timestamp = "No timestamp";
  }

  return {
    value: `0x${hexValue}`,
    timestamp: timestamp,
    rawValue: answer.value,
  };
}

function formatFeedType(feedType) {
  if (!feedType) return "Unknown";
  if (feedType.public !== undefined) return "Public";
  if (feedType.personal !== undefined) return "Personal";
  return "Unknown";
}

function formatSubscriptionStatus(feed, currentTime) {
  const isActive = feed.subscription_due_time > currentTime;
  const remainingTime = feed.subscription_due_time - currentTime;

  // Handle subscription due time safely
  let dueTime;
  if (feed.subscription_due_time && feed.subscription_due_time > 0) {
    try {
      dueTime = new Date(feed.subscription_due_time * 1000).toISOString();
    } catch (e) {
      dueTime = "Invalid due time";
    }
  } else {
    dueTime = "No due time set";
  }

  return {
    isActive,
    dueTime: dueTime,
    remainingSeconds: Math.max(0, remainingTime),
    remainingDays: Math.max(0, Math.floor(remainingTime / 86400)),
  };
}

async function readAllFeeds(program) {
  console.log("🔍 Searching for all feeds...");

  try {
    // Get all feed accounts
    const feeds = await program.account.feed.all();

    if (feeds.length === 0) {
      console.log("❌ No feeds found");
      return;
    }

    console.log(`✅ Found ${feeds.length} feed(s):`);
    console.log("");

    const currentTime = Math.floor(Date.now() / 1000);

    feeds.forEach((feedAccount, index) => {
      const feed = feedAccount.account;
      const subscription = formatSubscriptionStatus(feed, currentTime);
      const latestAnswer = formatAnswer(feed.latest_answer);

      console.log(`📊 Feed #${index + 1}`);
      console.log(`   Address: ${feedAccount.publicKey.toString()}`);
      console.log(`   Name: ${feed.name}`);
      console.log(`   Authority: ${feed.authority.toString()}`);
      console.log(`   Type: ${formatFeedType(feed.feedType)}`);
      console.log(
        `   Job ID: ${
          feed.jobId ? Buffer.from(feed.jobId).toString("hex") : "No job ID"
        }`
      );
      console.log(
        `   Data Source: ${
          feed.dataSource ? feed.dataSource.toString() : "No data source"
        }`
      );
      console.log(
        `   Min Signatures: ${feed.minSignaturesThreshold || "Not set"}`
      );
      console.log(`   Frequency: ${feed.frequency || "Not set"} seconds`);
      console.log(`   IPFS CID: ${feed.ipfsCid || "No IPFS CID"}`);
      console.log(
        `   Price per Second: ${
          feed.pricePerSecondScaled || "Not set"
        } (scaled)`
      );
      console.log(
        `   Subscription Due Time: ${feed.subscriptionDueTime || "Not set"}`
      );
      // Handle created_at safely
      let createdAt;
      if (feed.created_at && feed.created_at > 0) {
        try {
          createdAt = new Date(feed.created_at * 1000).toISOString();
        } catch (e) {
          createdAt = "Invalid creation time";
        }
      } else {
        createdAt = "No creation time";
      }

      console.log(`   Created: ${createdAt}`);
      console.log(`   Bump: ${feed.bump}`);

      console.log(`   📈 Latest Answer:`);
      if (latestAnswer === "No data") {
        console.log(`      ${latestAnswer}`);
      } else {
        console.log(`      Value: ${latestAnswer.value}`);
        console.log(`      Timestamp: ${latestAnswer.timestamp}`);
      }

      console.log(`   💰 Subscription:`);
      console.log(
        `      Active: ${subscription.isActive ? "✅ Yes" : "❌ No"}`
      );
      console.log(`      Due Time: ${subscription.dueTime}`);
      const remainingHours = isNaN(subscription.remainingSeconds)
        ? 0
        : Math.floor((subscription.remainingSeconds % 86400) / 3600);
      console.log(
        `      Remaining: ${
          subscription.remainingDays || 0
        } days, ${remainingHours} hours`
      );
      console.log(
        `      Price per Second: ${
          feed.pricePerSecondScaled || "Not set"
        } (scaled)`
      );

      console.log(
        `   📚 History: ${
          feed.answer_history ? feed.answer_history.length : 0
        }/${feed.historyIdx || 0} entries`
      );

      if (feed.answer_history && feed.answer_history.length > 0) {
        console.log(`   📋 Recent Answers:`);
        feed.answer_history.slice(-3).forEach((answer, i) => {
          const formatted = formatAnswer(answer);
          console.log(
            `      ${i + 1}. ${formatted.value} @ ${formatted.timestamp}`
          );
        });
      }

      console.log("");
    });
  } catch (error) {
    console.error("❌ Error reading feeds:", error.message);
    throw error;
  }
}

async function readSpecificFeed(program, feedAddress) {
  console.log(`🔍 Reading feed: ${feedAddress}`);

  try {
    const feedPublicKey = new PublicKey(feedAddress);
    const feed = await program.account.feed.fetch(feedPublicKey);

    const currentTime = Math.floor(Date.now() / 1000);
    const subscription = formatSubscriptionStatus(feed, currentTime);
    const latestAnswer = formatAnswer(feed.latestAnswer);

    console.log("✅ Feed Details:");
    console.log(`   Address: ${feedPublicKey.toString()}`);
    console.log(`   Name: ${feed.name}`);
    console.log(`   Authority: ${feed.authority.toString()}`);
    console.log(`   Type: ${formatFeedType(feed.feedType)}`);
    console.log(
      `   Job ID: ${
        feed.jobId ? Buffer.from(feed.jobId).toString("hex") : "No job ID"
      }`
    );
    console.log(
      `   Data Source: ${
        feed.dataSource ? feed.dataSource.toString() : "No data source"
      }`
    );
    console.log(
      `   Min Signatures: ${feed.minSignaturesThreshold || "Not set"}`
    );
    console.log(`   Frequency: ${feed.frequency || "Not set"} seconds`);
    console.log(`   IPFS CID: ${feed.ipfsCid || "No IPFS CID"}`);
    // Handle created_at safely
    let createdAt;
    if (feed.createdAt && feed.createdAt > 0) {
      try {
        createdAt = new Date(feed.createdAt * 1000).toISOString();
      } catch (e) {
        createdAt = "Invalid creation time";
      }
    } else {
      createdAt = "No creation time";
    }

    console.log(`   Created: ${createdAt}`);
    console.log(`   Bump: ${feed.bump}`);

    console.log(`   📈 Latest Answer:`);
    if (latestAnswer === "No data") {
      console.log(`      ${latestAnswer}`);
    } else {
      console.log(`      Value: ${latestAnswer.value}`);
      console.log(`      Timestamp: ${latestAnswer.timestamp}`);
    }

    console.log(`   💰 Subscription:`);
    console.log(`      Active: ${subscription.isActive ? "✅ Yes" : "❌ No"}`);
    console.log(`      Due Time: ${subscription.dueTime}`);
    const remainingHours = isNaN(subscription.remainingSeconds)
      ? 0
      : Math.floor((subscription.remainingSeconds % 86400) / 3600);
    console.log(
      `      Remaining: ${
        subscription.remainingDays || 0
      } days, ${remainingHours} hours`
    );
    console.log(
      `      Price per Second: ${
        feed.pricePerSecondScaled || "Not set"
      } (scaled)`
    );

    console.log(
      `   📚 History: ${feed.answerHistory ? feed.answerHistory.length : 0}/${
        feed.historyIdx || 0
      } entries`
    );

    if (feed.answerHistory && feed.answerHistory.length > 0) {
      console.log(`   📋 All Answers:`);
      feed.answer_history.forEach((answer, i) => {
        const formatted = formatAnswer(answer);
        console.log(
          `      ${i + 1}. ${formatted.value} @ ${formatted.timestamp}`
        );
      });
    }
  } catch (error) {
    if (error.message.includes("Account does not exist")) {
      console.error(`❌ Feed not found: ${feedAddress}`);
    } else {
      console.error("❌ Error reading feed:", error.message);
    }
    throw error;
  }
}

async function main() {
  setupEnvironment();

  const feedAddress = process.argv[2];

  try {
    const provider = anchor.AnchorProvider.env();
    anchor.setProvider(provider);

    const program = anchor.workspace.Molpha;
    console.log("Program ID:", program.programId.toString());
    console.log("RPC URL:", provider.connection.rpcEndpoint);
    console.log("");

    if (feedAddress) {
      await readSpecificFeed(program, feedAddress);
    } else {
      await readAllFeeds(program);
    }
  } catch (error) {
    console.error("❌ Error:", error.message);
    if (error.logs) {
      console.error("Transaction logs:", error.logs);
    }
    process.exit(1);
  }
}

// Show usage if help requested
if (process.argv.includes("--help") || process.argv.includes("-h")) {
  console.log("📖 Molpha Feed Reader");
  console.log("");
  console.log("Usage:");
  console.log("  node read-feeds.js                    # Read all feeds");
  console.log("  node read-feeds.js <FEED_ADDRESS>     # Read specific feed");
  console.log("  node read-feeds.js --help             # Show this help");
  console.log("");
  console.log("Examples:");
  console.log("  node read-feeds.js");
  console.log(
    "  node read-feeds.js 7eb72Es6Z88LBzjTcypvi9uJXjy3FzNGc97X3DfhGZ7F"
  );
  process.exit(0);
}

main().catch(console.error);
