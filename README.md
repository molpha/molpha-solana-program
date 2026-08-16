# Molpha Oracle Solana Program

A decentralized oracle network on Solana with an on-chain node registry, subscription-based feeds, and Schnorr-verified data updates aligned with Molpha’s EVM contracts.

## Overview

Molpha on Solana provides:

- **Node registry** — Permanent owner-keyed nodes and immutable, versioned membership snapshots
- **Plans & subscriptions** — USDC-paid plans that cap jobs and define signing thresholds
- **Feeds** — PDAs keyed by a 32-byte `source_id`, quorum (`signatures_required`), and submitter; updates store a payload (raw ≤32 bytes, or keccak digest), coalition bitmap, and aggregate Schnorr signature
- **Data updates** — `submit_data_update` writes feed state; `verify_data_update` runs the same checks without writing (for CPI / historical-registry consumers)
- **Rounds & settlement** — Gateways settle subscription and agent rounds against `source_id`; request-auth hashes bind `source_id` + `signatures_required` + timestamp
- **Delegates** — Subscription owners can authorize delegates (including owner-provisioned agents), capped by their plan's `max_delegates`
- **Protocol governance** — Admin config for fees, snapshot policy, gateway allowlists, and treasury

For what is implemented today versus the long-term P2P / MuSig2 design, see [`WP.md`](WP.md) and the operator-focused [**Node Operator Guide**](NODE_OPERATOR_GUIDE.md).

## Features

### Node registry

- `register_node` / `deactivate_node` atomically create immutable `Registry` version `N+1`
- permanent owner-keyed `Node` identities remain available for historical verification and rewards
- canonical compressed secp256k1 keys are validated on registration, stored as affine coordinates,
  and permanently reserved by a key-claim PDA
- registration requires a domain-separated Schnorr proof of possession bound to the program and
  derived node identity; verifier hot paths rely on this registration invariant
- `update_registry_policy` snapshots redundancy-policy changes without changing membership

### Plans, subscriptions, and feeds

- Admin-defined **plans** (`add_plan`, `update_plan`) with per-round pricing and signer limits
- **Subscribe** / **extend_subscription** in USDC
- Feeds are identified by a caller-provided 32-byte **`source_id`** (not a keccak-derived `feed_id`). `submit_data_update` lazy-inits the feed PDA on first submit. There is no `Job` account or `create_job` instruction.

### Data publication

- Coalition **Schnorr** verification with on-chain bitmap re-derivation (matches EVM `NodeGroupBitmapLib`); selection seed and message hash bind `source_id`
- Optional **verify-only** path (`verify_data_update`) against any retained historical registry
- Off-chain signing helpers in the CLI (`yarn molpha data sign` / `submit`) and `scripts/lib/cu-bench-data-update-common.js`

### Rounds

- `settle_round_receipt` / `settle_agent_round` take `source_id` (replacing `api_config_hash` / derived `feed_id`)
- `RequestAuth` is `H(source_id, signatures_required, timestamp)`; `AgentRequestAuth` includes the same pair plus agent, gateway, timestamp, and amount

### Tooling

- TypeScript **CLI** (`yarn molpha`) for init, registry, plans, delegates, feeds, rounds, and data flows
- Scripts for node generation, CU benchmarks, and end-to-end feed submission

## Program structure

```
programs/
├── molpha/                         # Main oracle program
│   └── src/
│       ├── instructions/
│       │   ├── initialize.rs / update_config.rs
│       │   ├── register_node.rs / deactivate_node.rs / update_node_endpoint.rs
│       │   ├── update_registry_policy.rs
│       │   ├── register_gateway.rs
│       │   ├── add_plan.rs / update_plan.rs
│       │   ├── subscribe.rs / extend_subscription.rs
│       │   ├── add_delegate.rs / remove_delegate.rs / set_delegate_paused.rs
│       │   ├── submit_data_update.rs / verify_data_update.rs
│       │   ├── settle_round_receipt.rs / settle_agent_round.rs
│       │   ├── finalize_round.rs / dispute_round.rs
│       │   └── claim_node_rewards.rs / claim_gateway_rewards.rs
│       ├── state/
│       │   ├── protocol_config.rs
│       │   ├── registry_state.rs / node.rs / secp256k1_key_claim.rs
│       │   ├── plan.rs / subscription.rs / delegate.rs
│       │   ├── feed.rs
│       │   ├── gateway.rs / agent_escrow.rs
│       │   └── round_receipt.rs / receipt.rs
│       ├── utils/
│       │   ├── bitmap.rs          # Group selection (EVM parity)
│       │   ├── schnorr.rs
│       │   └── registry.rs
│       ├── events.rs
│       ├── error.rs
│       └── lib.rs
```

```
cli/                                # TypeScript operator CLI
scripts/                            # Benchmarks, batch registration, e2e flows
tests/                              # Jest + bankrun instruction tests
```

## Getting started

### For integrators and feed consumers

| Goal                     | Document                                     |
| ------------------------ | -------------------------------------------- |
| Quick integration        | [QUICK_START.md](QUICK_START.md)             |
| Full product usage       | [MOLPHA_USER_GUIDE.md](MOLPHA_USER_GUIDE.md) |
| Step-by-step integration | [INTEGRATION_GUIDE.md](INTEGRATION_GUIDE.md) |
| API & account reference  | [API_REFERENCE.md](API_REFERENCE.md)         |

### For oracle node operators

👉 **[Node Operator Guide](NODE_OPERATOR_GUIDE.md)** — registry lifecycle, signing math, submit/verify flows, scripts, and CU limits

### For developers (build & test)

**Prerequisites:** Rust 1.70+, Solana CLI 1.17+, Anchor **0.32.1**, Node.js 18+, Yarn 1.22+

```bash
# Build program + copy .so for bankrun tests
yarn build:molpha

# Full test suite (build + jest/bankrun)
yarn test

# Anchor integration tests (validator)
anchor test

# CU regression: submit/verify @ 3/8/18 signers
yarn cu:data-update:bankrun
```

**TypeScript CLI:**

```bash
yarn build:cli
yarn molpha --help
```

See [`cli/README.md`](cli/README.md) for command groups and an end-to-end example (`init` → plan → subscribe → submit). Feeds and rounds take `--source-id` (not `--feed-id` / `--api-config-hash`).

**Deploy (devnet):**

```bash
anchor deploy --provider.cluster devnet
```

Program IDs are defined in [`Anchor.toml`](Anchor.toml) (see below).

### Compute budgets (`submit_data_update` / `verify_data_update`)

Bankrun/localnet measurements (May 2026 — details in [NODE_OPERATOR_GUIDE.md §10](NODE_OPERATOR_GUIDE.md#10-compute-budgets-and-limits)):

| Signers | `verify_data_update` | `submit_data_update` |
| ------- | -------------------- | -------------------- |
| 3       | ~296K                | ~300K                |
| 8       | ~384K                | ~387K                |
| 18      | ~529K                | ~532K                |

Use `ComputeBudgetProgram.setComputeUnitLimit` ≥ **550_000** for large coalitions. CI enforces ceilings via `yarn cu:data-update:bankrun`.

### Example: read feed via CLI

```bash
yarn molpha account feed --source-id <SOURCE_ID_HEX> --signatures-required 1
```

The feed PDA is `[molpha_feed, source_id, signatures_required, submitter]` (`--submitter` defaults to the wallet).

Off-chain signing and submission:

```bash
yarn molpha data sign --source-id <hex> --registry-version 1 --value <hex> ...
yarn molpha data submit --source-id <hex> --registry-version 1 --value <hex> ...
```

## Program IDs

From [`Anchor.toml`](Anchor.toml) (devnet / localnet):

| Program    | ID                                            |
| ---------- | --------------------------------------------- |
| **Molpha** | `MoLFGDpFoVnQgwbkTNScKPohCxhbfd61JjFrnotuwzh` |

## Documentation

### User & integration

- [Documentation Index](DOCUMENTATION_INDEX.md)
- [Quick Start](QUICK_START.md)
- [User Guide](MOLPHA_USER_GUIDE.md)
- [Integration Guide](INTEGRATION_GUIDE.md)
- [API Reference](API_REFERENCE.md)

### Operators & protocol engineers

- [**Node Operator Guide**](NODE_OPERATOR_GUIDE.md) — canonical on-chain behavior and runbooks
- [Oracle Interfaces](ORACLE_INTERFACES.md) — adapter and encoding notes
- [Remaining Accounts Limits](REMAINING_ACCOUNTS_LIMITS.md)
- [Feed Consumer Implementation](FEED_CONSUMER_IMPLEMENTATION.md)

### Deployment & project notes

- [Devnet Deployment](DEVNET_DEPLOYMENT.md)
- [Migration Guide](MIGRATION_GUIDE.md)
- [Scripts README](scripts/README.md)

## Scripts & yarn commands

| Command                       | Purpose                                        |
| ----------------------------- | ---------------------------------------------- |
| `yarn molpha`                 | Operator CLI (after `yarn build:cli`)          |
| `yarn register-nodes`         | `scripts/generate-and-register-nodes.js`       |
| `yarn cu:data-update:bankrun` | CU benchmark (bankrun)                         |

Key scripts under `scripts/`: `generate-and-register-nodes.js`, `oracle-client.ts`, and CU bench utilities. Use `yarn molpha` for all on-chain operations.

## Architecture

### Data flow (current on-chain path)

```
Subscriber / agent authorizes a source_id
        ↓
Off-chain coalition signs (secp256k1 / Schnorr; message binds source_id)
        ↓
submit_data_update → init feed if needed → verify bitmap + signature → update Feed
        ↓
Consumers read Feed account OR CPI verify_data_update (no write)
        ↓
Gateway settle_round_receipt / settle_agent_round (source_id + signatures_required)
```

### PDA seeds

| Account             | Seed prefix            | Additional seeds                                              |
| ------------------- | ---------------------- | ------------------------------------------------------------- |
| Protocol config     | `molpha_config`        | —                                                             |
| Registry state      | `molpha_registry`      | —                                                             |
| Registry            | `molpha_registry`      | version (`u32` LE)                                            |
| Node                | `molpha_node`          | node owner                                                    |
| Secp key claim      | `molpha_secp_key`      | keccak256(compressed pubkey)                                  |
| Plan                | `molpha_plan`          | plan type byte                                                |
| Subscription        | `molpha_subscription`  | subscriber owner                                              |
| Delegate            | `molpha_delegate`      | owner, delegate                                               |
| Feed                | `molpha_feed`          | `source_id` (32), `signatures_required` (u8), submitter       |
| Gateway             | `molpha_gateway`       | gateway authority                                             |
| Agent escrow        | `molpha_agent`         | authority, gateway                                            |
| Subscription round  | `molpha_round`         | consumer, `source_id`, `signatures_required` (u8), ts (`u64` LE) |
| Agent round         | `molpha_agent_round`   | authority, gateway, `source_id`, `signatures_required` (u8), ts (`u64` LE) |

Exact derivations are documented in [NODE_OPERATOR_GUIDE.md §3](NODE_OPERATOR_GUIDE.md#3-on-chain-accounts-and-pdas).

## Security

- Coalition Schnorr verification with on-chain group bitmap derivation
- Registry version checks on data updates
- PDA-scoped accounts and authority checks per instruction
- USDC deposits and protocol fee configuration for node registration and subscriptions

## Contributing

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Run `yarn test` and/or `anchor test`
5. Open a pull request

## License

ISC — see [package.json](package.json).

## Support

- GitHub Issues: [Molpha/molpha-solana-program](https://github.com/Molpha/molpha-solana-program/issues)
- Operator & protocol details: [NODE_OPERATOR_GUIDE.md](NODE_OPERATOR_GUIDE.md)
- Documentation hub: [DOCUMENTATION_INDEX.md](DOCUMENTATION_INDEX.md)
