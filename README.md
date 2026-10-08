# Molpha Oracle Solana Program

A decentralized oracle network on Solana with an on-chain node registry, subscription-based feeds, and Schnorr-verified attestations aligned with Molpha’s EVM contracts.

## Overview

Molpha on Solana provides:

- **Node registry** — Permanent owner-keyed nodes and immutable, versioned membership snapshots
- **Plans & subscriptions** — USDC-paid plans that cap jobs and define signing thresholds
- **Feeds** — PDAs keyed by a 32-byte `source_id`, quorum (`signatures_required`), and submitter; updates store the signed 32-byte value (the oracle value itself, or `keccak256(raw_value)` for a preimage of up to 256 bytes, as recorded by `value_kind`), coalition bitmap, timestamp, and registry version
- **Data updates** — `submit_attestation` writes feed state; `verify_attestation` runs the same checks without writing (for CPI / historical-registry consumers)
- **Epoch rewards** — Income lands in program-owned vaults and is swept into a per-epoch pool; node participation is measured from a sample of fully verified rounds (tickets); each node's share is credited after the epoch. No transaction per round.
- **Delegates** — Subscription owners can authorize delegates (including owner-provisioned agents), capped by their plan's `max_delegates`
- **Protocol governance** — Admin config for fees, snapshot policy, gateway allowlists, and treasury

## Features

### Node registry

- `register_node` / `deactivate_node` atomically create immutable `Registry` version `N+1`
- permanent owner-keyed `Node` identities remain available for historical verification and rewards
- canonical compressed secp256k1 keys are validated on registration, stored as affine coordinates,
  and permanently reserved by a key-claim PDA
- registration requires a domain-separated Schnorr proof of possession bound to the program and
  derived node identity; verifier hot paths rely on this registration invariant
- `update_registry_policy` snapshots redundancy-policy changes without changing membership
- **Epoch availability** — a node's inclusion rate in a finalized epoch (`signed / selected`,
  relative to the network rate) sets its payout floor; `slash_offence` with `Offence::Inactivity`
  freezes a node below that floor (one case per node per epoch), and `reinstate_node` restores it
  after cooldown
- Node security deposits are held in a **shared USDC vault** (the canonical ATA of `registry_state`),
  separate from the protocol treasury; per-node `locked_amount` tracks each node's share

### Plans, subscriptions, and feeds

- Admin-defined **plans** (`add_plan`, `update_plan`) with per-round pricing and signer limits
- **Subscribe** / **extend_subscription** in USDC
- Feeds are identified by a caller-provided 32-byte **`source_id`**. `submit_attestation` lazy-inits the feed PDA on first submit.

### Data publication

- Coalition **Schnorr** verification with on-chain bitmap re-derivation (matches EVM `NodeGroupBitmapLib`); selection seed and message hash bind `source_id`
- Optional **verify-only** path (`verify_attestation`) against any retained historical registry
- Off-chain signing helpers in the CLI (`yarn molpha attestation sign` / `submit`)

### Node slashing

- One permissionless instruction, `slash_offence`, takes an `Offence` enum: equivocation,
  unsolicited signing, commitment reuse, nonce reuse, or inactivity. Proven evidence writes a
  `PunishmentCase` PDA (seeded by the evidence hash, so the same filing cannot replay), slashes
  the guilty set, and evicts them into a successor registry. Inactivity freezes without taking
  principal; the other offences tombstone.

### Epoch rewards

Per-round settlement (`settle_subscription_round`, `settle_x402_round`, `close_round_replay`) is
gone. Three flows replace it, and none of them writes an account per round:

- **Income.** All income lands in the **treasury** (the USDC ATA of the `protocol_config` PDA):
  x402 `payTo` is the treasury, and subscriptions pay it too. Gateways hold no funds, take no fee
  and earn no rewards. Permissionless `sweep_treasury_revenue` books unswept income into the pool
  of the epoch open at cluster time, keeping `protocol_fee_bps` as protocol reserve.
  `withdraw_protocol_reserve` lets the authority withdraw fees and slashing proceeds, never pools
  or rewards. The treasury ledger is `reward_liability` (Σ claimable) + `pool_liability`
  (Σ live pools) + `protocol_reserved`; anything above it is unswept income.
- **Measurement.** A round is a **ticket** when the big-endian `u64` prefix of
  `keccak("MOLPHA_TICKET_V1" ‖ m ‖ s ‖ commitment)` is below its epoch's threshold `tau`.
  Anyone may `submit_ticket`: the round must fall in the epoch window, arrive before
  `end + ticket_grace`, use a registry version created no later than its
  `timestamp` (seconds) and still live then, carry
  the dispatching gateway's Ed25519 signature over
  `keccak("MOLPHA_DISPATCH_V1" ‖ program_id ‖ source_id ‖ k ‖ version ‖ ts)` (`ts` is the round's millisecond `timestamp`) in the immediately
  preceding instruction, and verify in full. Its whole selection is re-derived and counted in the
  epoch's `EpochState` (selected / signed per node, keyed by an epoch roster of `Node` PDAs because
  registry positions move between versions). A `TicketMarker` keyed by round identity
  `(source_id, k, ts)` stops any aggregate of the same round counting twice; it is closable once
  the epoch stops accepting tickets. The epoch also pins `min_signers` when it is created;
  later config updates do not invalidate its pending tickets.
- **Payout.** After the ticket deadline anyone may `finalize_epoch`: weights freeze and the epoch
  two ahead is created with `tau` rescaled toward `target_tickets_per_epoch` (at most 8x per step,
  from the ticket count of the epoch being finalized, so a change reaches an epoch two later).
  The two genesis epochs carry `initialize`'s `initial_tau` (at least `TAU_MIN`, 2^32, a sampling
  probability of 2^-32; `u64::MAX` samples every round). Size it as
  `2^64 · target_tickets_per_epoch / expected_rounds_per_epoch`: an unsized start makes every
  round of the first epochs a ticket transaction (live, 4,003 tickets from 4,003 rounds), while a
  sized one sampled 998 of 4,984 rounds against a 20% target. `submit_attestation` on an existing
  feed rejects a stale or duplicate round (same source and quorum, timestamp not newer, registry
  not older) before verifying the signature, about 17-20K CU instead of 120-210K.
  A node's weight is its sampled signature count, or zero when it has at least
  `min_availability_samples` selections and an inclusion rate below `min_availability_bps` of the
  network rate. `credit_epoch_rewards` adds `floor(pool · w / W)` to `Node.claimable_rewards`
  (tombstoned nodes forfeit); `claim_node_rewards` withdraws as before. `close_epoch` waits one
  more epoch length (inactivity evidence window), then rolls the residual into the open epoch.

Subscription round quotas and delegate request limits are enforced by the gateway at admission;
`reward_per_signature` and `x402_round_base` remain as published x402 pricing inputs only.

Every instruction that verifies an aggregate (`submit_attestation`, `submit_ticket`, and the
equivocation / commitment-reuse / unsolicited-signing evidence) also takes the coalition key
`CoalitionKey { x, y }`: the affine sum of the signers' secp256k1 keys, which any library computes
from their `Node` accounts in any order. It is unsigned and outside the evidence hash. The verifier
checks it against its own sum instead of inverting that sum (~138K CU), and a wrong key fails with
`InvalidCoalitionKey`.

Known beta posture (see the epoch reward spec): the gateway dispatch signature is the only thing
binding a ticket to an admitted round, and a coalition that forms a round's whole signer set can
re-sign that round to re-roll its lottery or exclude a co-signer. Both assume the permissioned
operator set and the Molpha-operated gateway.

### Tooling

- TypeScript **CLI** (`yarn molpha`) for init, registry, plans, delegates, feeds, rounds, slashing, and data flows
- Scripts for node generation and end-to-end feed submission

## Program structure

```
programs/
├── molpha/                         # Main oracle program
│   └── src/
│       ├── instructions/
│       │   ├── admin/              # initialize.rs, update_config.rs, add_plan.rs, ...
│       │   ├── node/                # register_node.rs, deactivate_node.rs, update_node_endpoint.rs, ...
│       │   ├── delegate/            # add_delegate.rs, remove_delegate.rs, set_delegate_paused.rs
│       │   ├── gateway/             # register_gateway.rs, deactivate_gateway.rs
│       │   ├── subscriptions/       # subscribe.rs, extend_subscription.rs
│       │   ├── epoch/               # submit_ticket, finalize_epoch, credit_epoch_rewards, close_epoch, close_ticket_marker
│       │   ├── revenue/             # sweep_treasury_revenue, withdraw_protocol_reserve
│       │   ├── slashing/            # slash_offence.rs
│       │   ├── submit_attestation.rs / verify_attestation.rs
│       ├── state/
│       │   ├── attestation.rs       # Attestation / AttestationPayload / SchnorrSignature (instruction args)
│       │   ├── protocol_config.rs
│       │   ├── registry_state.rs / node.rs / secp256k1_key_claim.rs
│       │   ├── plan.rs / subscription.rs / delegate.rs
│       │   ├── feed.rs
│       │   ├── gateway.rs
│       │   ├── offence/             # Offence enum, evidence, penalties
│       │   ├── epoch.rs / ticket.rs  # EpochState, TicketMarker, ticket / dispatch hashes
│       │   └── punishment_case.rs
│       ├── utils/
│       │   ├── ed25519.rs
│       │   ├── attestation.rs     # Attestation checks shared by submit_attestation and submit_ticket
│       │   └── registry.rs / slash.rs / time.rs
│       ├── events.rs
│       ├── error.rs
│       └── lib.rs
```

```
cli/                                # TypeScript operator CLI
scripts/                            # Benchmarks, batch registration, e2e flows
molpha-tests/                       # Rust + LiteSVM test suite
│   ├── src/                        # harness: env, PDAs, secp256k1 signing, scenarios
│   ├── tests/                      # one file per instruction group
│   └── fixtures/                   # cross-chain golden vector
```

## Getting started

### For integrators and feed consumers

| Goal                     | Document                                     |
| ------------------------ | -------------------------------------------- |
| Step-by-step integration | [Integration guide](guides/INTEGRATION.md) |
| API & account reference  | [API reference](guides/API_REFERENCE.md)   |

### For developers (build & test)

**Prerequisites:** Rust 1.70+, Solana CLI 1.17+, Anchor **1.1.2**, Node.js 18+, Yarn 1.22+
(Node and Yarn are needed only for the CLI; the test suite is pure Rust.)

```bash
# Build the program — the test suite loads target/deploy/molpha.so
anchor build -p molpha

# Full test suite: unit tests + LiteSVM integration tests, all in molpha-tests
cargo test --workspace

# One area at a time (all files are modules of the single `suite` binary)
cargo test -p molpha-tests --test suite tickets
cargo test -p molpha-tests --test suite encoding          # pure unit tests, no .so needed
cargo test -p molpha-tests --test suite limits -- --nocapture   # prints CU and transaction sizes
```

Tests run against [LiteSVM](https://github.com/LiteSVM/litesvm): no validator, no ledger, no
network. Each test builds its own VM, mint and keypairs, so they are order-independent and safe
to run in parallel. Rebuild the program after changing it — a stale `.so` silently tests the old
binary.

**TypeScript CLI:**

```bash
yarn build:cli
yarn molpha --help
```

See [`cli/README.md`](cli/README.md) for command groups and an end-to-end example (`init` → plan → subscribe → submit). Feeds and rounds take `--source-id`.

**Deploy (devnet):**

```bash
anchor deploy --provider.cluster devnet
```

Program IDs are defined in [`Anchor.toml`](Anchor.toml) (see below).

### Ticket limits (`submit_ticket`)

A ticket transaction carries the compute-budget instruction, the gateway's Ed25519 dispatch
instruction and the ticket, so it fits fewer signers than `submit_attestation`. Measured by
`cargo test -p molpha-tests --test suite limits compute_bench -- --nocapture`:

| Signers | Compute units (whole tx) | Fits                                  |
| ------- | ------------------------ | ------------------------------------- |
| 8       | ~132K                    | legacy                                |
| 10      | —                        | legacy ceiling                        |
| 12      | ~166K                    | v0 + lookup table                     |
| 18      | ~223K                    | v0 + lookup table                     |
| 20      | ~249K                    | v0 + lookup table (991 bytes)         |

Put the signer `Node` accounts in the lookup table; the marker, epoch and payer stay static. Every
ticket write-locks its epoch's `EpochState`, which is fine at the sampled rate.

### Coalition size limits (`submit_attestation`)

Measured by `cargo test -p molpha-tests --test suite limits -- --nocapture`, which asserts these
numbers rather than reporting them:

| Signers | Compute units | Transaction bytes |
| ------- | ------------- | ----------------- |
| 5       | ~91K          | 742               |
| 10      | ~142K         | 907               |
| 15      | ~183K         | 1,072             |
| 19      | ~223K         | 1,204             |
| 20      | —             | 1,237 ✗           |

**19 signers** is the ceiling for a legacy transaction: each extra signer costs 33 bytes, the
64-byte coalition key costs about two of them, and the packet limit is 1,232. The compute budget is
not the binding constraint — a 19-signer round uses about a sixth of the 1.4M maximum. Set
`ComputeBudgetProgram.setComputeUnitLimit` ≥ **400_000** for large coalitions; go past 19 signers
only with address lookup tables.

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

## Scripts & yarn commands

| Command                 | Purpose                                  |
| ----------------------- | ---------------------------------------- |
| `yarn molpha`           | Operator CLI (after `yarn build:cli`)    |
| `yarn generate-nodes`   | `scripts/generate-nodes.js`              |
| `yarn register-nodes`   | `scripts/register-nodes.js`              |

Key scripts under `scripts/`: `generate-nodes.js`, `register-nodes.js`. Use `yarn molpha` for all on-chain operations.

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
Sampled rounds → submit_ticket (dispatch sig + full verify) → EpochState counters
        ↓
x402 payments and subscriptions land in the treasury → sweep_treasury_revenue → open epoch pool
        ↓
finalize_epoch → credit_epoch_rewards → claim_node_rewards
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
| Epoch state         | `molpha_epoch`         | epoch index (`u64` LE)                                        |
| Ticket marker       | `molpha_ticket`        | `keccak("MOLPHA_TICKET_ID_V1" ‖ source_id ‖ k ‖ ts BE)`        |

**Derived token accounts** (not PDAs, but program-enforced):

| Account             | Owner / authority      | Notes                                                         |
| ------------------- | ---------------------- | ------------------------------------------------------------- |
| Protocol treasury   | `protocol_config` PDA  | USDC ATA: x402 `payTo`, subscriptions, pools, rewards, reserve |
| Node deposit vault  | `registry_state` PDA   | Shared USDC vault for all node security deposits              |

Exact derivations are documented in [NODE_OPERATOR_GUIDE.md §3](NODE_OPERATOR_GUIDE.md#3-on-chain-accounts-and-pdas).

## Security

- Coalition Schnorr verification with on-chain group bitmap derivation
- Registry version checks on attestations
- PDA-scoped accounts and authority checks per instruction
- USDC deposits and protocol fee configuration for node registration and subscriptions
- Node deposits are custodied in a shared vault (`registry_state` ATA), isolated from treasury revenue
- Gateway registration and deactivation are controlled by the protocol authority

### Withdrawals

- `registry withdraw-deposit` releases a deactivated node's deposit after `withdrawable_at`, or
  the surviving balance of a tombstoned node after its punishment cooldown. Rewards remain on the
  separate `claim_node_rewards` path.
- `gateway deactivate` immediately stops the gateway's dispatch signatures from admitting tickets
  and records deactivation on the permanent Gateway account. Gateway registration is bondless,
  and a deactivated identity cannot re-register.

## Contributing

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Run `anchor build -p molpha && cargo test --workspace`
5. Open a pull request

## License

Apache-2.0 — see [LICENSE](LICENSE)

## Support

- GitHub Issues: [Molpha/molpha-solana-program](https://github.com/Molpha/molpha-solana-program/issues)
