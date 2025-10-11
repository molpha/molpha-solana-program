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
    remainingHours: Math.max(0, Math.floor(remainingTime % 86400 / 3600)),
    remainingMinutes: Math.max(0, Math.floor(remainingTime % 3600 / 60))
  };
}

function formatTokenAmount(amount, decimals = 6) {
  return (amount / Math.pow(10, decimals)).toFixed(decimals);
}

async function readAllSubscriptions(program) {
  console.log("🔍 Reading all feed subscriptions...");
  
  try {
    // Get all feed accounts
    const feeds = await program.account.feed.all();
    
    if (feeds.length === 0) {
      console.log("❌ No feeds found");
      return;
    }

    const currentTime = Math.floor(Date.now() / 1000);
    
    // Separate active and expired subscriptions
    const activeSubscriptions = [];
    const expiredSubscriptions = [];
    
    feeds.forEach(feedAccount => {
      const feed = feedAccount.account;
      const subscription = formatSubscriptionStatus(feed, currentTime);
      
      const subscriptionInfo = {
        feedAddress: feedAccount.publicKey.toString(),
        feed: feed,
        subscription: subscription
      };
      
      if (subscription.isActive) {
        activeSubscriptions.push(subscriptionInfo);
      } else {
        expiredSubscriptions.push(subscriptionInfo);
      }
    });

    console.log(`✅ Found ${feeds.length} feed subscription(s):`);
    console.log(`   Active: ${activeSubscriptions.length}`);
    console.log(`   Expired: ${expiredSubscriptions.length}`);
    console.log("");

    // Display active subscriptions
    if (activeSubscriptions.length > 0) {
      console.log("🟢 Active Subscriptions:");
      activeSubscriptions.forEach((sub, index) => {
        const { feed, subscription } = sub;
        console.log(`   ${index + 1}. ${feed.name} (${formatFeedType(feed.feed_type)})`);
        console.log(`      Feed: ${sub.feedAddress}`);
        console.log(`      Authority: ${feed.authority.toString()}`);
        console.log(`      Remaining: ${subscription.remainingDays}d ${subscription.remainingHours}h ${subscription.remainingMinutes}m`);
        console.log(`      Due: ${subscription.dueTime}`);
        console.log(`      Price per Second: ${formatTokenAmount(feed.price_per_second_scaled / 1000000)} tokens`);
        console.log("");
      });
    }

    // Display expired subscriptions
    if (expiredSubscriptions.length > 0) {
      console.log("🔴 Expired Subscriptions:");
      expiredSubscriptions.forEach((sub, index) => {
        const { feed, subscription } = sub;
        const expiredDays = isNaN(feed.subscription_due_time) ? 0 : Math.floor((currentTime - feed.subscription_due_time) / 86400);
        console.log(`   ${index + 1}. ${feed.name} (${formatFeedType(feed.feed_type)})`);
        console.log(`      Feed: ${sub.feedAddress}`);
        console.log(`      Authority: ${feed.authority.toString()}`);
        console.log(`      Expired: ${expiredDays} days ago`);
        console.log(`      Was due: ${subscription.dueTime}`);
        console.log(`      Price per Second: ${feed.price_per_second_scaled ? formatTokenAmount(feed.price_per_second_scaled / 1000000) : 'Not set'} tokens`);
        console.log("");
      });
    }

    // Summary statistics
    console.log("📊 Subscription Summary:");
    const totalRevenue = feeds.reduce((sum, feedAccount) => {
      const feed = feedAccount.account;
      // Calculate total cost based on subscription duration
      if (feed.subscription_due_time && feed.created_at && feed.price_per_second_scaled) {
        const subscriptionDuration = feed.subscription_due_time - feed.created_at;
        const totalCost = (feed.price_per_second_scaled * subscriptionDuration) / 1000000;
        return sum + totalCost;
      }
      return sum;
    }, 0);
    
    console.log(`   Total Revenue: ${isNaN(totalRevenue) ? 'Not available' : formatTokenAmount(totalRevenue)} tokens`);
    
    const validPriceFeeds = feeds.filter(f => f.account.price_per_second_scaled);
    const avgPrice = validPriceFeeds.length > 0 
      ? validPriceFeeds.reduce((sum, f) => sum + f.account.price_per_second_scaled, 0) / validPriceFeeds.length / 1000000
      : 0;
    console.log(`   Average Price per Second: ${isNaN(avgPrice) ? 'Not available' : formatTokenAmount(avgPrice)} tokens`);
    
    const personalFeeds = feeds.filter(f => f.account.feed_type && f.account.feed_type.personal !== undefined);
    const publicFeeds = feeds.filter(f => f.account.feed_type && f.account.feed_type.public !== undefined);
    
    console.log(`   Personal Feeds: ${personalFeeds.length}`);
    console.log(`   Public Feeds: ${publicFeeds.length}`);

  } catch (error) {
    console.error("❌ Error reading subscriptions:", error.message);
    throw error;
  }
}

async function readSubscriptionsByAuthority(program, authorityAddress) {
  console.log(`🔍 Reading subscriptions for authority: ${authorityAddress}`);
  
  try {
    const authorityPublicKey = new PublicKey(authorityAddress);
    const feeds = await program.account.feed.all();
    
    const authorityFeeds = feeds.filter(
      feedAccount => feedAccount.account.authority.equals(authorityPublicKey)
    );
    
    if (authorityFeeds.length === 0) {
      console.log(`❌ No subscriptions found for authority: ${authorityAddress}`);
      return;
    }

    const currentTime = Math.floor(Date.now() / 1000);
    
    console.log(`✅ Found ${authorityFeeds.length} subscription(s) for authority ${authorityAddress}:`);
    console.log("");

    authorityFeeds.forEach((feedAccount, index) => {
      const feed = feedAccount.account;
      const subscription = formatSubscriptionStatus(feed, currentTime);
      
      console.log(`📊 Subscription #${index + 1}`);
      console.log(`   Feed: ${feedAccount.publicKey.toString()}`);
      console.log(`   Name: ${feed.name}`);
      console.log(`   Type: ${formatFeedType(feed.feed_type)}`);
      console.log(`   Status: ${subscription.isActive ? '🟢 Active' : '🔴 Expired'}`);
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
      console.log(`   Due: ${subscription.dueTime}`);
      
      if (subscription.isActive) {
        console.log(`   Remaining: ${subscription.remainingDays}d ${subscription.remainingHours}h ${subscription.remainingMinutes}m`);
      } else {
        const expiredDays = isNaN(feed.subscription_due_time) ? 0 : Math.floor((currentTime - feed.subscription_due_time) / 86400);
        console.log(`   Expired: ${expiredDays} days ago`);
      }
      
      console.log(`   Price per Second: ${feed.price_per_second_scaled ? formatTokenAmount(feed.price_per_second_scaled / 1000000) : 'Not set'} tokens`);
      console.log("");
    });

  } catch (error) {
    console.error("❌ Error reading subscriptions by authority:", error.message);
    throw error;
  }
}

async function readExpiringSubscriptions(program, hoursThreshold = 24) {
  console.log(`🔍 Reading subscriptions expiring within ${hoursThreshold} hours...`);
  
  try {
    const feeds = await program.account.feed.all();
    const currentTime = Math.floor(Date.now() / 1000);
    const thresholdSeconds = hoursThreshold * 3600;
    
    const expiringFeeds = feeds.filter(feedAccount => {
      const feed = feedAccount.account;
      const timeUntilExpiry = feed.subscription_due_time - currentTime;
      return timeUntilExpiry > 0 && timeUntilExpiry <= thresholdSeconds;
    });
    
    if (expiringFeeds.length === 0) {
      console.log(`✅ No subscriptions expiring within ${hoursThreshold} hours`);
      return;
    }

    console.log(`⚠️  Found ${expiringFeeds.length} subscription(s) expiring within ${hoursThreshold} hours:`);
    console.log("");

    expiringFeeds.forEach((feedAccount, index) => {
      const feed = feedAccount.account;
      const subscription = formatSubscriptionStatus(feed, currentTime);
      
      console.log(`📊 Subscription #${index + 1}`);
      console.log(`   Feed: ${feedAccount.publicKey.toString()}`);
      console.log(`   Name: ${feed.name}`);
      console.log(`   Authority: ${feed.authority.toString()}`);
      console.log(`   Type: ${formatFeedType(feed.feed_type)}`);
      console.log(`   Remaining: ${subscription.remainingDays}d ${subscription.remainingHours}h ${subscription.remainingMinutes}m`);
      console.log(`   Due: ${subscription.dueTime}`);
      console.log(`   Price per Second: ${feed.price_per_second_scaled ? formatTokenAmount(feed.price_per_second_scaled / 1000000) : 'Not set'} tokens`);
      console.log("");
    });

  } catch (error) {
    console.error("❌ Error reading expiring subscriptions:", error.message);
    throw error;
  }
}

async function main() {
  setupEnvironment();

  const command = process.argv[2];
  const param = process.argv[3];

  try {
    const provider = anchor.AnchorProvider.env();
    anchor.setProvider(provider);

    const program = anchor.workspace.Molpha;
    console.log("Program ID:", program.programId.toString());
    console.log("RPC URL:", provider.connection.rpcEndpoint);
    console.log("");

    if (command === 'authority' && param) {
      await readSubscriptionsByAuthority(program, param);
    } else if (command === 'expiring' && param) {
      await readExpiringSubscriptions(program, parseInt(param));
    } else if (command === 'expiring') {
      await readExpiringSubscriptions(program, 24); // Default 24 hours
    } else if (!command) {
      await readAllSubscriptions(program);
    } else {
      console.error("❌ Invalid command or missing parameter");
      console.log("Usage:");
      console.log("  node read-subscriptions.js                    # Read all subscriptions");
      console.log("  node read-subscriptions.js authority <ADDRESS> # Read by authority");
      console.log("  node read-subscriptions.js expiring [HOURS]   # Read expiring subscriptions");
      process.exit(1);
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
if (process.argv.includes('--help') || process.argv.includes('-h')) {
  console.log("📖 Molpha Subscription Reader");
  console.log("");
  console.log("Usage:");
  console.log("  node read-subscriptions.js                        # Read all subscriptions");
  console.log("  node read-subscriptions.js authority <ADDRESS>     # Read by authority");
  console.log("  node read-subscriptions.js expiring [HOURS]        # Read expiring subscriptions");
  console.log("  node read-subscriptions.js --help                  # Show this help");
  console.log("");
  console.log("This script displays:");
  console.log("  - All feed subscriptions (active and expired)");
  console.log("  - Subscription status and remaining time");
  console.log("  - Revenue statistics");
  console.log("  - Expiring subscriptions for renewal alerts");
  console.log("");
  console.log("Examples:");
  console.log("  node read-subscriptions.js");
  console.log("  node read-subscriptions.js authority 9K9FknHzW7j8a88yKTrzxKfDrxnV2QLqSR58ETAVdc8P");
  console.log("  node read-subscriptions.js expiring 48");
  process.exit(0);
}

main().catch(console.error);
