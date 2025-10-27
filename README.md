# Molpha Oracle Solana Program

A decentralized oracle network on Solana that provides reliable, multi-source data feeds with signature verification and oracle adapter support.

## Overview

Molpha is a comprehensive oracle solution that combines:
- **Oracle Node Management**: Register and manage oracle nodes with EVM key support
- **Data Feed Management**: Create and configure flexible data feeds (public/personal)
- **Data Source Management**: Define and manage multiple data sources
- **Subscription System**: Time-based subscriptions for personal feeds with automatic expiration
- **Oracle Adapters**: Compatible with Pyth and Switchboard interfaces
- **Signature Verification**: Ed25519 signature verification for oracle nodes
- **Protocol Governance**: Configurable pricing and subscription parameters

## Features

### 🔐 Node Management
- Register oracle nodes with Solana and EVM public keys
- Signature verification using Ed25519
- Node-based access control for data publication

### 📊 Feed Management
- **Public Feeds**: Open data accessible by anyone
- **Personal Feeds**: Subscription-based, time-limited access
- Configurable update frequency and signature thresholds
- Support for any 32-byte data format

### 🔌 Oracle Adapters
Compatible with major oracle standards:
- **Pyth Network**: Price feeds with confidence intervals
- **Switchboard**: Aggregated results with statistics
- Extensible for custom oracle formats

See [Adapters README](programs/molpha/src/adapters/README.md) for details.

### 💎 Data Source Management
- Create multiple data sources with metadata
- Link data sources to specific feeds
- Update data source configurations
- Support for various data source types

### 💰 Subscription System
- Time-based subscriptions for personal feeds
- Extend existing subscriptions
- Automatic expiration handling
- Configurable pricing per second

## Program Structure

```
programs/
├── molpha/                    # Main oracle program
│   └── src/
│       ├── adapters/          # Oracle compatibility layer
│       │   ├── pyth.rs        # Pyth Network adapter
│       │   ├── switchboard.rs # Switchboard adapter
│       │   └── README.md      # Adapter documentation
│       ├── instructions/      # Instruction handlers
│       │   ├── initialize.rs           # Initialize protocol
│       │   ├── register_node.rs        # Register oracle nodes
│       │   ├── create_feed.rs          # Create data feeds
│       │   ├── update_feed_config.rs   # Update feed settings
│       │   ├── publish_answer.rs       # Publish oracle data
│       │   ├── extend_subscription.rs  # Extend subscriptions
│       │   ├── create_data_source.rs   # Create data sources
│       │   ├── update_data_source.rs   # Update data sources
│       │   └── update_protocol_config.rs # Update protocol config
│       ├── state/             # Account definitions
│       │   ├── protocol_config.rs # Protocol configuration
│       │   ├── node.rs        # Oracle node state
│       │   ├── feed.rs        # Feed account state
│       │   ├── feed_types.rs  # Feed type enums
│       │   ├── answer.rs      # Answer/response state
│       │   └── data_source.rs # Data source state
│       ├── utils/             # Utility functions
│       │   ├── ed25519.rs     # Signature verification
│       │   ├── hash.rs        # Hashing utilities
│       │   └── pricing.rs     # Pricing calculations
│       ├── events.rs          # Event definitions
│       ├── error.rs           # Error types
│       └── lib.rs             # Program entry point
└── faucet/                    # Test token faucet
    └── src/
        ├── instructions/
        │   ├── initialize.rs
        │   └── request_tokens.rs
        └── state/
            └── faucet_config.rs
```

## Quick Start

### Prerequisites

- Rust 1.70+
- Solana CLI 1.17+
- Anchor 0.30+
- Node.js 18+

### Build

```bash
anchor build
```

### Test

```bash
anchor test
```

### Deploy

```bash
# Deploy to devnet
anchor deploy --provider.cluster devnet

# Deploy to mainnet
anchor deploy --provider.cluster mainnet
```

## Usage Examples

### Creating a Feed

```bash
node scripts/create-feed.js \
  --name "BTC/USD" \
  --type personal \
  --frequency 60 \
  --threshold 2
```

### Publishing Data

```bash
node scripts/publish-answer.js \
  --feed <FEED_PUBKEY> \
  --value <ANSWER_VALUE> \
  --signature <NODE_SIGNATURE>
```

### Consuming Feed Data

See [example-oracle-usage.ts](scripts/example-oracle-usage.ts) for TypeScript examples.

## Program IDs

- **Molpha Oracle**: `moLfNsTqLqej38QPHu4xq7uMBVK14MRQCQWhBhz8X3k`
- **Faucet** (devnet): Check `target/deploy/faucet-keypair.json`

## Documentation

- [Adapter Guide](programs/molpha/src/adapters/README.md) - Oracle compatibility layer
- [Complete Summary](COMPLETE_SUMMARY.md) - Recent implementation details
- [Oracle Interfaces](ORACLE_INTERFACES.md) - Interface specifications
- [Migration Guide](MIGRATION_GUIDE.md) - Upgrade instructions
- [Faucet README](FAUCET_README.md) - Test token faucet

## Scripts

The `scripts/` directory contains utility scripts:

- `initialize.js` - Initialize protocol configuration
- `create-feed.js` - Create new data feeds
- `create-data-source.js` - Create data sources
- `register-node.js` - Register oracle nodes
- `read-feeds.js` - Read feed data
- `subscribe.js` - Subscribe to personal feeds
- `oracle-client.ts` - TypeScript client library

## Architecture

### Data Flow

```
Oracle Node → Sign Data → Publish Answer → Feed Account
                                ↓
                        Verify Signature
                                ↓
                        Update Feed State
                                ↓
                    Consumer Reads via Adapter
                    (Pyth/Switchboard/Custom)
```

### PDA Seeds

- **Protocol Config**: `["protocol_config"]`
- **Node**: `["node", authority, node_pubkey]`
- **Feed**: `["feed", authority, hash(name), feed_type, threshold, frequency, job_id]`
- **Data Source**: `["data_source", authority, hash(name), type]`

## Security

- Ed25519 signature verification for all oracle submissions
- PDA-based access control
- Time-based subscription expiration
- Authority-based feed management

## Contributing

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Run tests: `anchor test`
5. Submit a pull request

## License

[Add your license here]

## Support

For questions and support:
- GitHub Issues: [Create an issue](https://github.com/your-repo/issues)
- Documentation: See docs in this repository