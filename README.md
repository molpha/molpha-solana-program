# Molpha Oracle Solana Program

A decentralized oracle network on Solana with an on-chain node registry, subscription-based feeds, and Schnorr-verified attestations aligned with Molpha’s EVM contracts.

## Overview

Molpha on Solana provides:

- **Node registry** — Permanent owner-keyed nodes and immutable, versioned membership snapshots
- **Plans & subscriptions** — USDC-paid plans that cap jobs and define signing thresholds
- **Feeds** — PDAs keyed by a 32-byte `source_id`, quorum (`signatures_required`), and submitter; updates store a payload (raw ≤32 bytes, or keccak digest), coalition bitmap, and aggregate Schnorr signature
- **Data updates** — `submit_attestation` writes feed state; `verify_attestation` runs the same checks without writing (for CPI / historical-registry consumers)
- **Rounds & settlement** — Gateways settle subscription-funded and x402 gateway-funded rounds against `source_id`; request-auth hashes bind `source_id` + `signatures_required` + timestamp
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
- Feeds are identified by a caller-provided 32-byte **`source_id`** (not a keccak-derived `feed_id`). `submit_attestation` lazy-inits the feed PDA on first submit. There is no `Job` account or `create_job` instruction.

### Data publication

- Coalition **Schnorr** verification with on-chain bitmap re-derivation (matches EVM `NodeGroupBitmapLib`); selection seed and message hash bind `source_id`
- Optional **verify-only** path (`verify_attestation`) against any retained historical registry
- Off-chain signing helpers in the CLI (`yarn molpha data sign` / `submit`) and `scripts/lib/cu-bench-attestation-common.js`

### Rounds

- Two settlement instructions, discriminated by funding source: `settle_subscription_round`
  (draws down a subscription's round quota) and `settle_x402_round` (a bonded gateway pays the
  protocol-quoted price from its own USDC float — any payment rail, e.g. Solana or Base x402 —
  and records `payment_rail` / `payment_ref` as attested, gateway-claimed metadata the program
  never verifies). Both take `source_id` directly (not a derived `feed_id` / `api_config_hash`).
- `RequestAuth` is `H(source_id, signatures_required, timestamp)`, verified on-chain for the
  subscription path. The x402 path has no on-chain payer authentication — the gateway self-attests
  and is economically deterred by paying real USDC per round; demand authenticity is a bilateral,
  rail-level concern.

### Tooling

- TypeScript **CLI** (`yarn molpha`) for init, registry, plans, delegates, feeds, rounds, and data flows
- Scripts for node generation, CU benchmarks, and end-to-end feed submission

## Program structure

```
programs/
├── molpha/                         # Main oracle program
│   └── src/
│       ├── instructions/
│       │   ├── admin/              # initialize.rs, update_config.rs, add_plan.rs, ...
│       │   ├── node/                # register_node.rs, deactivate_node.rs, update_node_endpoint.rs, ...
│       │   ├── delegate/            # add_delegate.rs, remove_delegate.rs, set_delegate_paused.rs
│       │   ├── gateway/             # register_gateway.rs, claim_gateway_rewards.rs
│       │   ├── subscriptions/       # subscribe.rs, extend_subscription.rs
│       │   ├── round_settlement/    # settle_subscription_round.rs, settle_x402_round.rs, finalize_round.rs
│       │   ├── disputing/           # dispute_round.rs, punish_*.rs
│       │   ├── submit_attestation.rs / verify_attestation.rs
│       │   └── bench_registry.rs   # feature = "cu-bench"
│       ├── state/
│       │   ├── protocol_config.rs
│       │   ├── registry_state.rs / node.rs / secp256k1_key_claim.rs
│       │   ├── plan.rs / subscription.rs / delegate.rs
│       │   ├── feed.rs
│       │   ├── gateway.rs
│       │   └── round_receipt.rs / receipt.rs
│       ├── utils/
│       │   ├── bitmap.rs          # Group selection (EVM parity)
│       │   ├── ed25519.rs
│       │   └── registry.rs / punish.rs
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
yarn cu:attestation:bankrun
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

### Compute budgets (`submit_attestation` / `verify_attestation`)

Bankrun/localnet measurements (May 2026 — details in [NODE_OPERATOR_GUIDE.md §10](NODE_OPERATOR_GUIDE.md#10-compute-budgets-and-limits)):

| Signers | `verify_attestation` | `submit_attestation` |
| ------- | -------------------- | -------------------- |
| 3       | ~296K                | ~300K                |
| 8       | ~384K                | ~387K                |
| 18      | ~529K                | ~532K                |

Use `ComputeBudgetProgram.setComputeUnitLimit` ≥ **550_000** for large coalitions. CI enforces ceilings via `yarn cu:attestation:bankrun`.

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
| `yarn cu:attestation:bankrun` | CU benchmark (bankrun)                         |

Key scripts under `scripts/`: `generate-and-register-nodes.js`, `oracle-client.ts`, and CU bench utilities. Use `yarn molpha` for all on-chain operations.

## Architecture

### Data flow (current on-chain path)

```
Subscriber (or an x402 payer, off-chain) requests a source_id round
        ↓
Off-chain coalition signs (secp256k1 / Schnorr; message binds source_id)
        ↓
submit_attestation → init feed if needed → verify bitmap + signature → update Feed
        ↓
Consumers read Feed account OR CPI verify_attestation (no write)
        ↓
Gateway settle_subscription_round (subscription draw) OR
Gateway settle_x402_round (pays price from its own USDC float, any rail)
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
| Subscription round  | `molpha_round`         | subscription owner, `source_id`, `signatures_required` (u8), ts (`u64` LE) |
| x402 round          | `molpha_x402_round`    | `payer_id` (32), gateway, `source_id`, `signatures_required` (u8), ts (`u64` LE) |

Exact derivations are documented in [NODE_OPERATOR_GUIDE.md §3](NODE_OPERATOR_GUIDE.md#3-on-chain-accounts-and-pdas).

## Security

- Coalition Schnorr verification with on-chain group bitmap derivation
- Registry version checks on attestations
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
