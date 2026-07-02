# Scripts

Utility scripts for benchmarks, batch node setup, E2E flows, and crypto helpers.

On-chain interaction (init, registry, plans, subscriptions, jobs, account reads) is handled by the TypeScript CLI — see [`cli/README.md`](../cli/README.md).

## Yarn shortcuts

| Command | Script |
|---------|--------|
| `yarn register-nodes` | `generate-and-register-nodes.js` |
| `yarn job-flow` | `subscribe-create-job-submit.js` |
| `yarn cu:data-update:bankrun` | `cu-bench-data-update-bankrun.js` |
| `yarn cu:data-update:localnet` | `cu-bench-data-update-localnet.js` |
| `yarn cu:settle-round-receipt:localnet` | `cu-bench-settle-round-receipt-localnet.js` |
| `yarn cu:verify-answer-alt:localnet` | `cu-bench-verify-answer-alt.js` |

## Node generation & registration

`generate-and-register-nodes.js` — generate Solana + EVM keypairs, register nodes on-chain, write `generated-nodes.json`.

```bash
yarn register-nodes --count 5 --generate-new
yarn register-nodes --finalize
```

Prerequisites: `anchor build`, `yarn molpha init`, funded admin wallet (SOL + USDC).

## End-to-end job flow

`subscribe-create-job-submit.js` — subscribe, sign a coalition update, and submit (lazy job/feed init).

```bash
yarn job-flow --plan-id 1
```

Requires `generated-nodes.json` from `yarn register-nodes`.

## CU benchmarks

- `cu-bench-data-update-bankrun.js` — bankrun regression (CI)
- `cu-bench-data-update-localnet.js` — localnet measurements
- `cu-bench-data-update-breakdown.js` — per-instruction CU breakdown
- `cu-bench-settle-round-receipt-localnet.js` — settle-round receipt CU
- `cu-bench-verify-answer-alt.js` — verify answer ALT path
- `cu-bench-derive-group-bitmap.js` — group bitmap derivation bench

Shared helpers: `lib/cu-bench-data-update-common.js`, `lib/molpha-provider.js`.

## Crypto utilities

| Script | Purpose |
|--------|---------|
| `generate-evm-keypair.js` | Generate secp256k1 keypair for node signing |
| `compress-evm-pubkey.js` | Compress uncompressed pubkey hex |
| `compressed-pubkey-to-address.js` | Derive EVM address from compressed pubkey |
| `verify-evm-point.js` | Verify secp256k1 point / compression |

## Oracle client (TypeScript)

- `oracle-client.ts` — off-chain feed reader client
- `example-oracle-usage.ts` — usage examples
