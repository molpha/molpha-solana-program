# molpha-tests

Rust + [LiteSVM](https://github.com/LiteSVM/litesvm) test suite for the Molpha Solana program.

```bash
# LiteSVM loads target/deploy/molpha.so, so rebuild after every program change.
anchor build -p molpha
cargo test --workspace --locked
```

No validator, no ledger, no network. Every test builds its own VM, USDC mint and keypairs, so
tests are order-independent and run in parallel. **Rebuild the program after changing it** — a
stale `.so` silently tests the old binary.

## Layout

| Path                | What it holds                                                          |
| ------------------- | ---------------------------------------------------------------------- |
| `src/env.rs`        | `TestEnv`: the VM, transaction sending, account/event/error assertions  |
| `src/scenario.rs`   | `Protocol`: initialized protocol, registered nodes, gateways, duties    |
| `src/crypto.rs`     | secp256k1 signer: node keys, proofs of possession, coalition Schnorr    |
| `src/ed25519.rs`    | the Ed25519 precompile instruction `submit_ticket` parses (dispatch sig) |
| `src/pda.rs`        | PDA derivations, built from the program's own `SEED_PREFIX` constants   |
| `src/token.rs`      | hand-encoded SPL Token / ATA instructions and account readers           |
| `fixtures/`         | the cross-chain golden vector shared with the EVM validator             |

The files are modules of one `suite` binary (`tests/suite.rs`), so `cargo test` links LiteSVM
once. Filter an area with `cargo test -p molpha-tests --test suite <file>`. One file per
instruction group: `protocol_config`, `plans_and_subscriptions`, `delegates`,
`node_registry`, `reinstate_node`, `gateway`, `attestation`, `revenue`, `tickets`, `epochs`,
`epoch_rewards`, `punish_equivocation`, `punish_unsolicited_signing`, `punish_commitment_reuse`,
`punish_nonce_reuse`, `slash_offence` (epoch inactivity), plus `evm_compatibility`, `limits` and
`compute_bench`.

Four files hold the program's pure unit tests — no VM, no `.so`, they exercise the `molpha` crate
as a library. They live here rather than in `#[cfg(test)]` modules so the whole suite is in one
place:

| File                    | What it pins                                                      |
| ----------------------- | ----------------------------------------------------------------- |
| `encoding.rs`           | argument wire formats, `Attestation` mirror, `Node`/`Feed` layouts |
| `hashing.rs`            | ticket / dispatch hashes and punishment evidence hashes, domain separation |
| `registry_snapshot.rs`  | version succession, snapshot copy / `swap_remove`, `evict_nodes`   |
| `penalties.rs`          | the fixed penalty table and `Penalty::apply` arithmetic            |

## Writing a test

```rust
let mut p = Protocol::start();          // initialize, with a real mint and treasury
p.register_nodes("my-test", 8);         // 8 nodes with real PoP, registry version 8
let signed = p.sign_duty(Duty::new(8, 5));   // sign with the derived selection set

let ix = p.submit_attestation_ix(&submitter.pubkey(), &signed);
p.env.send_ok(&[ix], &[&submitter]);    // panics with program logs on failure
```

`ProtocolBuilder` varies anything in `InitializeArgs` (`Protocol::builder().min_signers(5).start()`).
`sign_duty_with_bits` picks the coalition by hand, which is how quorum, selection-subset and
guilt-set cases are built. `submit_attestation_ix_with_args` takes caller-built
`SubmitAttestationArgs`, for `raw_value` and tampered-payload cases. Failures assert a specific
error:

```rust
let failed = p.env.send_err(&[ix], &[&submitter]);
assert_molpha_error(&failed, MolphaError::FeedNotNewer);       // program error
assert_anchor_error(&failed, ErrorCode::ConstraintHasOne);     // Anchor constraint
```

## Conventions

- **One VM per test.** Nothing is shared, so a failure never cascades and order never matters.
- **Deterministic keys.** `crypto::node_keys(domain, n)` and the Schnorr nonce derive from a seed,
  never from randomness, so a failure reproduces on re-run.
- **Assert the specific error**, not just that something failed — a test that accepts any failure
  passes for the wrong reason as soon as the setup drifts.
- **Assert state, not just success.** After a rejection, check the state the instruction would
  have written is still untouched.
