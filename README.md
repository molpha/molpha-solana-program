# Molpha Solana program

The on-chain half of the Molpha oracle network on Solana: a versioned node registry, USDC
subscriptions, feeds updated by threshold Schnorr attestations, and epoch-based node rewards. The
attestation format is shared with Molpha's EVM contracts, so one coalition signature verifies on
both.

## Status

**Beta. Deployed on devnet. Not audited.**

- Node operators and gateways are admitted by the protocol authority; registration is not
  permissionless.
- The devnet program is upgradeable and changes often. It may not match any given commit.
- Interfaces and account layouts can still change between releases.

See [Trust model and known limitations](guides/ARCHITECTURE.md#trust-model-and-known-limitations)
before relying on a feed.

## How it works

```
Subscriber (or an x402 payer, off chain) requests a round for a source_id
        ↓
Gateway dispatches the round; a selected coalition of nodes signs (secp256k1 Schnorr)
        ↓
submit_attestation: re-derive the selection, verify the signature, update the Feed account
        ↓
Consumers read the Feed account
        ↓
Sampled rounds → submit_ticket → per-epoch participation counters
        ↓
Payments land in the treasury → sweep_treasury_revenue → epoch pool
        ↓
finalize_epoch → credit_epoch_rewards → claim_node_rewards
```

- **Node registry.** Every membership change creates an immutable registry snapshot. Nodes
  register a secp256k1 key with a proof of possession and lock a USDC deposit.
- **Plans and subscriptions.** Admin-defined plans, paid monthly in USDC. Owners can authorize
  delegates up to the plan's limit.
- **Feeds.** One account per `(source_id, quorum, submitter)`, holding the signed 32-byte value
  (or the hash of a preimage of up to 256 bytes), signer bitmap, timestamp and registry version.
- **Epoch rewards.** Income is pooled per epoch. A sample of fully verified rounds measures each
  node's participation, and the pool is split by that measurement. No transaction per round.
- **Slashing.** Anyone can file evidence of equivocation, unsolicited signing, commitment reuse,
  nonce reuse or inactivity through one instruction, `slash_offence`.

[guides/ARCHITECTURE.md](guides/ARCHITECTURE.md) covers each of these in detail.

## Repository layout

| Path             | Contents                                                              |
| ---------------- | --------------------------------------------------------------------- |
| `programs/molpha` | The Anchor program                                                    |
| `molpha-tests`   | Rust + [LiteSVM](https://github.com/LiteSVM/litesvm) test suite        |
| `cli`            | `molpha`, the TypeScript operator CLI                                 |
| `scripts`        | Batch node generation and registration, and the CI secret scanner     |
| `guides`         | Architecture, API reference and integration guide                     |

Signature verification lives in the separately versioned
[`molpha-verifier`](https://github.com/molpha/molpha-verifier) crate.

## Build and test

Toolchain, as pinned in `rust-toolchain.toml`, `Anchor.toml` and CI:

| Tool       | Version |
| ---------- | ------- |
| Rust       | 1.97.1  |
| Solana CLI | 4.1.1   |
| Anchor     | 1.2.1   |
| Node.js    | 22      |
| Yarn       | 1.22    |

Node and Yarn are needed only for the CLI and scripts.

```bash
# Build the program and its IDL. The test suite loads target/deploy/molpha.so.
anchor build -p molpha

# Unit tests and LiteSVM integration tests
cargo test --workspace --locked

# One area at a time
cargo test -p molpha-tests --test suite tickets
cargo test -p molpha-tests --test suite limits -- --nocapture   # prints CU and transaction sizes
```

The tests need no validator and no network. Rebuild the program after changing it: a stale `.so`
silently tests the old binary. See [molpha-tests/README.md](molpha-tests/README.md).

CLI:

```bash
yarn install --frozen-lockfile
yarn test:cli        # builds the CLI and runs its tests
yarn molpha --help
```

The CLI reads the IDL from `target/idl/molpha.json`, so build the program first. See
[cli/README.md](cli/README.md) for the command groups and an end-to-end example.

## Program ID

| Cluster           | Program ID                                     |
| ----------------- | ---------------------------------------------- |
| Devnet, localnet  | `MoLF3pgVA9MENSuEujxNNSMaTo71aLyewaH5nrWfUZs`  |

This is the ID declared in `programs/molpha/src/lib.rs` and `Anchor.toml`. There is no mainnet
deployment.

## Documentation

| Document                                         | For                                              |
| ------------------------------------------------ | ------------------------------------------------ |
| [Architecture](guides/ARCHITECTURE.md)           | Accounts, PDAs, reward and slashing mechanics, limits |
| [API reference](guides/API_REFERENCE.md)         | Instructions and the `Feed` account              |
| [Integration guide](guides/INTEGRATION.md)       | Reading a feed from a consumer                   |
| [CLI](cli/README.md)                             | Operating the program                            |
| [Scripts](scripts/README.md)                     | Batch node setup                                 |
| [Tests](molpha-tests/README.md)                  | Test harness and conventions                     |

## Security

Report vulnerabilities privately; see [SECURITY.md](SECURITY.md). Do not open a public issue.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

Apache-2.0. See [LICENSE](LICENSE).
