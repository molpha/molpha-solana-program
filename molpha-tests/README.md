# molpha-tests

Rust + [LiteSVM](https://github.com/LiteSVM/litesvm) test suite for the Molpha Solana program.

```bash
# LiteSVM loads target/deploy/molpha.so — rebuild after every program change.
cargo build-sbf --manifest-path programs/molpha/Cargo.toml
cp target/sbpfv3-solana-solana/release/molpha.so target/deploy/molpha.so
# or: anchor build -p molpha  (requires Docker in Anchor 1.2)
cargo test --workspace
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
| `src/ed25519.rs`    | the Ed25519 precompile instruction `settle_subscription_round` parses   |
| `src/pda.rs`        | PDA derivations, built from the program's own `SEED_PREFIX` constants   |
| `src/token.rs`      | hand-encoded SPL Token / ATA instructions and account readers           |
| `fixtures/`         | the cross-chain golden vector shared with the EVM validator             |

One test file per instruction group: `protocol_config`, `plans_and_subscriptions`, `delegates`,
`node_registry`, `reinstate_node`, `gateway`, `attestation`, `settle_subscription`, `settle_x402`,
`rounds`, `slash_equivocation`, `slash_unsolicited_signing`, `slash_commitment_reuse`,
`slash_nonce_reuse`, plus `evm_compatibility` and `limits`.

Four files hold the program's pure unit tests — no VM, no `.so`, they exercise the `molpha` crate
as a library. They live here rather than in `#[cfg(test)]` modules so the whole suite is in one
place:

| File                    | What it pins                                                      |
| ----------------------- | ----------------------------------------------------------------- |
| `encoding.rs`           | argument wire formats, `Attestation` mirror, `Node`/`Feed` layouts |
| `hashing.rs`            | request-auth and punishment evidence hashes, domain separation     |
| `registry_snapshot.rs`  | version succession, snapshot copy / `swap_remove`, `evict_nodes`   |
| `penalties.rs`          | the fixed penalty table and `apply_penalty` arithmetic             |

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
