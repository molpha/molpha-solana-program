# Scripts

Utility scripts for batch node setup, E2E flows, and crypto helpers.

On-chain interaction (init, registry, plans, subscriptions, jobs, account reads) is handled by the TypeScript CLI — see [`cli/README.md`](../cli/README.md).

## Yarn shortcuts

| Command               | Script                           |
| --------------------- | -------------------------------- |
| `yarn register-nodes` | `generate-and-register-nodes.js` |

## Node generation & registration

`generate-and-register-nodes.js` — generate Solana + EVM keypairs, register nodes on-chain, write `generated-nodes.json`.

```bash
yarn register-nodes --count 5 --generate-new
```

Prerequisites: `anchor build`, `yarn molpha init`, funded admin wallet (SOL + USDC).

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
