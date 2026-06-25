# Molpha Oracle Solana Program

A decentralized oracle network on Solana with an on-chain node registry, subscription-based jobs, and Schnorr-verified data updates aligned with Molpha’s EVM contracts.

## Overview

Molpha on Solana provides:

- **Node registry** — Register oracle nodes with USDC deposits, sequential indices, and registry version transitions
- **Plans & subscriptions** — USDC-paid plans that cap jobs and define signing thresholds
- **Jobs & feeds** — Each job owns a feed PDA; updates carry a 32-byte value, coalition bitmap, and aggregate Schnorr signature
- **Data updates** — `submit_data_update` writes feed state; `verify_data_update` runs the same checks without writing (72-byte return data for CPI consumers)
- **Delegates** — Job owners can authorize up to five delegates to submit on their behalf
- **Protocol governance** — Admin config for fees, registry cooldowns, gateway allowlists, and treasury

For what is implemented today versus the long-term P2P / MuSig2 design, see [`WP.md`](WP.md) and the operator-focused [**Node Operator Guide**](NODE_OPERATOR_GUIDE.md).

## Features

### Node registry

- `register_node` / `deactivate_node` with versioned `RegistryState` and `RegistryIndex` entries
- secp256k1 signing keys stored in the registry index (EVM-parity message hashing)
- `finalize_registry_transition` after batch registry changes

### Jobs, plans, and subscriptions

- Admin-defined **plans** (`add_plan`, `update_plan`) with per-round pricing and signer limits
- **Subscribe** / **extend_subscription** in USDC
- Jobs are bound to a `job_id`, feed, and API config hash; `submit_data_update` initializes the job/feed on first submit, while `create_job` remains available for explicit pre-creation.

### Data publication

- Coalition **Schnorr** verification with on-chain bitmap re-derivation (matches EVM `NodeGroupBitmapLib`)
- Optional **verify-only** path for pull oracles that read return data in the same transaction
- Off-chain signing helpers in the CLI and `scripts/lib/cu-bench-data-update-common.js`

### Tooling

- TypeScript **CLI** (`yarn molpha`) for init, registry, plans, jobs, delegates, and data flows
- Scripts for local setup, node generation, CU benchmarks, and end-to-end job submission

## Program structure

```
programs/
├── molpha/                         # Main oracle program
│   └── src/
│       ├── instructions/
│       │   ├── initialize.rs
│       │   ├── register_node.rs / deactivate_node.rs
│       │   ├── finalize_registry_transition.rs
│       │   ├── add_gateway.rs / remove_gateway.rs
│       │   ├── add_plan.rs / update_plan.rs
│       │   ├── subscribe.rs / extend_subscription.rs
│       │   ├── create_job.rs
│       │   ├── add_delegate.rs / remove_delegate.rs
│       │   ├── submit_data_update.rs / verify_data_update.rs
│       │   └── data_update_core.rs
│       ├── state/
│       │   ├── protocol_config.rs
│       │   ├── registry_state.rs / registry_index.rs
│       │   ├── node.rs / gateway.rs
│       │   ├── plan.rs / subscription.rs
│       │   ├── job.rs / feed.rs
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
scripts/                            # Setup, registration, benchmarks, e2e flows
tests/                              # Jest + bankrun instruction tests
```

## Getting started

### For integrators and feed consumers

| Goal | Document |
|------|----------|
| Quick integration | [QUICK_START.md](QUICK_START.md) |
| Full product usage | [MOLPHA_USER_GUIDE.md](MOLPHA_USER_GUIDE.md) |
| Step-by-step integration | [INTEGRATION_GUIDE.md](INTEGRATION_GUIDE.md) |
| API & account reference | [API_REFERENCE.md](API_REFERENCE.md) |

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

See [`cli/README.md`](cli/README.md) for command groups and an end-to-end example (`init` → plan → subscribe → submit).

**Deploy (devnet):**

```bash
anchor deploy --provider.cluster devnet
```

Program IDs are defined in [`Anchor.toml`](Anchor.toml) (see below).

### Compute budgets (`submit_data_update` / `verify_data_update`)

Bankrun/localnet measurements (May 2026 — details in [NODE_OPERATOR_GUIDE.md §10](NODE_OPERATOR_GUIDE.md#10-compute-budgets-and-limits)):

| Signers | `verify_data_update` | `submit_data_update` |
|---------|----------------------|----------------------|
| 3 | ~296K | ~300K |
| 8 | ~384K | ~387K |
| 18 | ~529K | ~532K |

Use `ComputeBudgetProgram.setComputeUnitLimit` ≥ **550_000** for large coalitions. CI enforces ceilings via `yarn cu:data-update:bankrun`.

### Example: read feed via CLI

```bash
yarn molpha account feed --job-id <64_hex_job_id>
```

Off-chain signing and submission:

```bash
yarn molpha data sign --job-id <hex> --registry-version 1 --value <hex> ...
yarn molpha data submit ...
```

See [`scripts/subscribe-create-job-submit.js`](scripts/subscribe-create-job-submit.js) (`yarn job-flow`) for a full devnet/localnet path.

## Program IDs

From [`Anchor.toml`](Anchor.toml) (devnet / localnet):

| Program | ID |
|---------|-----|
| **Molpha** | `MoLFeTRpDZgckPjjbLwW1wB9n85bQiqboPnvw9RwoG8` |

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

| Command | Purpose |
|---------|---------|
| `yarn molpha` | Operator CLI (after `yarn build:cli`) |
| `yarn register-nodes` | `scripts/generate-and-register-nodes.js` |
| `yarn job-flow` | Subscribe → submit update (lazy job/feed init) |
| `yarn cu:data-update:bankrun` | CU benchmark (bankrun) |

Key scripts under `scripts/`: `initialize.js`, `generate-and-register-nodes.js`, `subscribe-create-job-submit.js`, `oracle-client.ts`, `pda-helpers.js`, and CU bench utilities.

## Architecture

### Data flow (current on-chain path)

```
Job owner subscribes
        ↓
Off-chain coalition signs (secp256k1 / Schnorr, EVM-compatible message)
        ↓
submit_data_update → init job/feed if needed → verify bitmap + signature → update Feed
        ↓
Consumers read Feed account OR CPI verify_data_update (no write, return data)
```

### PDA seeds

| Account | Seed prefix | Additional seeds |
|---------|-------------|------------------|
| Protocol config | `molpha_config` | — |
| Registry state | `molpha_registry` | — |
| Registry index | `molpha_registry_index` | version, index |
| Node | `molpha_node` | node owner |
| Plan | `molpha_plan` | plan type byte |
| Subscription | `molpha_subscription` | subscriber owner |
| Job | `molpha_job` | `job_id` (32 bytes) |
| Feed | `molpha_feed` | `job_id` (32 bytes) |
| Gateway | `molpha_gateway` | IP + port |

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
