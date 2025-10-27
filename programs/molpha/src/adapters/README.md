# Oracle Adapters

This module provides standard oracle interfaces for Molpha feeds while keeping the core data structure flexible.

## Architecture

```
┌─────────────────────────────────────────────────┐
│         Core Feed (Flexible)                    │
│         value: [u8; 32]                         │
└──────────────┬──────────────────────────────────┘
               │
               ├──► Pyth Adapter
               │    - PythPrice struct
               │    - ToPythPrice trait
               │    - PythCompatible trait
               │
               ├──► Switchboard Adapter
               │    - SwitchboardResult struct
               │    - ToSwitchboardResult trait
               │    - SwitchboardCompatible trait
               │
               └──► Custom Adapters (Future)
                    - Chainlink, DIA, etc.
```

## Modules

### `pyth.rs`
Provides Pyth Network compatible interface for price feeds.

**Key Types:**
- `PythPrice` - Pyth-style price with confidence and exponent
- `PriceStatus` - Trading, Halted, Unknown
- `ToPythPrice` trait - Convert Answer to Pyth price
- `PythCompatible` trait - Get Pyth price from Feed

**Data Layout:**
```
Bytes 0-7:   price (i64)      - Integer price value
Bytes 8-15:  conf (u64)       - Confidence interval
Bytes 16-19: expo (i32)       - Price exponent
Bytes 20-31: reserved (12)    - Future use
```

**Usage:**
```rust
use molpha::adapters::pyth::PythCompatible;

let pyth_price = feed.get_pyth_price()?;
let fresh_price = feed.get_pyth_price_no_older_than(60)?;
```

### `switchboard.rs`
Provides Switchboard compatible interface for aggregated data.

**Key Types:**
- `SwitchboardResult` - Aggregator result with statistics
- `ToSwitchboardResult` trait - Convert Answer to Switchboard result
- `SwitchboardCompatible` trait - Get Switchboard result from Feed

**Data Layout:**
```
Bytes 0-7:   value (i64)      - Main value
Bytes 8-15:  std_dev (i64)    - Standard deviation
Bytes 16-23: min (i64)        - Minimum response
Bytes 24-31: max (i64)        - Maximum response
```

**Usage:**
```rust
use molpha::adapters::switchboard::SwitchboardCompatible;

let sb_result = feed.get_switchboard_result()?;
```

## Benefits

1. **Separation of Concerns**: Oracle interfaces are separate from core logic
2. **Flexibility**: Core structure supports any 32-byte data format
3. **Compatibility**: Drop-in replacement for Pyth/Switchboard consumers
4. **Extensibility**: Easy to add new oracle adapters
5. **Zero Overhead**: Only pay for what you use

## Examples

### Publishing Price Data

```rust
use molpha::adapters::pyth::encoding::encode_price_data;
use molpha::state::Answer;

// Encode BTC/USD = $50,000.00 with $10 confidence
let answer_value = encode_price_data(
    50_000_00000000,  // price with 8 decimals
    10_00000000,      // confidence
    -8                // exponent
);

let answer = Answer {
    value: answer_value,
    timestamp: Clock::get()?.unix_timestamp,
};

// Publish via publish_answer instruction
```

### Consuming Price Data (Rust)

```rust
use molpha::adapters::pyth::PythCompatible;
use molpha::state::Feed;

#[derive(Accounts)]
pub struct MyInstruction<'info> {
    pub molpha_feed: Account<'info, Feed>,
}

pub fn my_instruction(ctx: Context<MyInstruction>) -> Result<()> {
    let feed = &ctx.accounts.molpha_feed;
    
    // Get price with staleness check
    let price = feed.get_pyth_price_no_older_than(60)?;
    
    // Use price in your logic
    let actual_price = price.price as f64 * 10f64.powi(price.expo);
    
    msg!("Current price: ${}", actual_price);
    
    Ok(())
}
```

### Consuming Price Data (TypeScript)

```typescript
import { MolphaOracleClient } from '../scripts/oracle-client';

const client = new MolphaOracleClient(program, connection);

// Simple way - get decimal price
const price = await client.getPrice(feedPubkey);
console.log(`Price: $${price.value}`);

// Pyth way - get raw price data
const pythPrice = await client.getPythPrice(feedPubkey);
const actualPrice = Number(pythPrice.price) * Math.pow(10, pythPrice.expo);
console.log(`Price: $${actualPrice}`);

// With staleness check
const freshPrice = await client.getPriceNoOlderThan(feedPubkey, 60);
```

## Adding Custom Adapters

To add support for another oracle standard:

1. Create new file: `src/adapters/your_oracle.rs`
2. Define structs matching the oracle's format
3. Implement conversion traits for `Answer` and `Feed`
4. Add encoding/decoding helpers
5. Export from `mod.rs`

Example structure:
```rust
pub struct YourOraclePrice {
    // Fields matching your oracle
}

pub trait ToYourOraclePrice {
    fn to_your_oracle_price(&self) -> Option<YourOraclePrice>;
}

impl ToYourOraclePrice for Answer {
    fn to_your_oracle_price(&self) -> Option<YourOraclePrice> {
        // Convert Answer.value to your format
    }
}
```

## Testing

Run adapter tests:
```bash
anchor test -- --features test-adapters
```

Test encoding/decoding:
```bash
cargo test -p molpha adapters::
```

## References

- [Pyth Network Documentation](https://docs.pyth.network/)
- [Switchboard Documentation](https://docs.switchboard.xyz/)
- [Main Oracle Interfaces Guide](../../ORACLE_INTERFACES.md)

