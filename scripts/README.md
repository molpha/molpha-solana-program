# Scripts

Utility scripts for benchmarks, batch node setup, E2E flows, and crypto helpers.

On-chain interaction (init, registry, plans, subscriptions, jobs, account reads) is handled by the TypeScript CLI — see [`cli/README.md`](../cli/README.md).

## Yarn shortcuts

| Command                                 | Script                                                |
| --------------------------------------- | ----------------------------------------------------- |
| `yarn register-nodes`                   | `generate-and-register-nodes.js`                      |
| `yarn cu:attestation:bankrun`           | `cu-bench-attestation-bankrun.js`                     |
| `yarn cu:attestation:localnet`          | `cu-bench-attestation-localnet.js`                    |
| `yarn cu:settle-subscription-round:localnet` | `cu-bench-settle-subscription-round-localnet.js` |
| `yarn cu:settle_round:bankrun`          | Subscription + x402 settle-round CU gates (bankrun)   |
| `yarn cu:node-registry:bankrun`         | Versioned registry snapshot/resolution CU gates       |
| `yarn cu:verify-answer-alt:localnet`    | `cu-bench-verify-answer-alt.js`                       |

## Node generation & registration

`generate-and-register-nodes.js` — generate Solana + EVM keypairs, register nodes on-chain, write `generated-nodes.json`.

```bash
yarn register-nodes --count 5 --generate-new
```

Prerequisites: `anchor build`, `yarn molpha init`, funded admin wallet (SOL + USDC).

## CU benchmarks

- `cu-bench-attestation-bankrun.js` — bankrun regression for verify/submit (CI)
- `cu-bench-attestation-localnet.js` — localnet verify/submit measurements
- `cu-bench-attestation-breakdown.js` — per-threshold verify CU sweep + linear fit
- `cu-bench-settle-subscription-round-localnet.js` — settle-subscription-round CU (localnet)
- `cu-bench-settle-round-bankrun.js` — subscription + x402 settle-round CU gates (bankrun); writes `reports/settle-round-cu-results.raw.json`
- `cu-bench-verify-answer-alt.js` — alias for attestation localnet bench
- `cu-bench-node-registry-bankrun.js` — gates 256-node snapshot creation and 18/32/64-signer historical resolution; writes `reports/node-registry-cu-results.raw.json`

The node-registry sweep uses feature-gated measurement instructions. Build and run it with
`NO_DNA=1 yarn build:molpha:cu-bench` followed by `NO_DNA=1 yarn cu:node-registry:bankrun`, then
restore a production artifact with `NO_DNA=1 cargo build-sbf --manifest-path programs/molpha/Cargo.toml --sbf-out-dir target/deploy`
(or `NO_DNA=1 anchor build -p molpha --ignore-keys`).

If `anchor build` reports a program-id keypair mismatch, the yarn cu-bench script already passes
`--ignore-keys` and forces an SBF rebuild with `--features cu-bench`.

Shared helpers: `lib/cu-bench-attestation-common.js`, `lib/cu-bench-settle-round-common.js`, `lib/molpha-provider.js`.

Attestation benches seed a versioned `Registry` snapshot plus owner-keyed `Node` PDAs
(remaining accounts), matching `submit_attestation` / `verify_attestation`.

## Crypto utilities

| Script                            | Purpose                                     |
| --------------------------------- | ------------------------------------------- |
| `generate-evm-keypair.js`         | Generate secp256k1 keypair for node signing |
| `compress-evm-pubkey.js`          | Compress uncompressed pubkey hex            |
| `compressed-pubkey-to-address.js` | Derive EVM address from compressed pubkey   |
| `verify-evm-point.js`             | Verify secp256k1 point / compression        |

## Oracle client (TypeScript)

- `oracle-client.ts` — off-chain feed reader client
- `example-oracle-usage.ts` — usage examples
