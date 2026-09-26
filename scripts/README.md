# Scripts

Utility scripts for batch node setup, E2E flows, and crypto helpers.

On-chain interaction (init, registry, plans, subscriptions, jobs, account reads) is handled by the TypeScript CLI — see [`cli/README.md`](../cli/README.md).

## Yarn shortcuts

| Command               | Script                 |
| --------------------- | ---------------------- |
| `yarn generate-nodes` | `generate-nodes.js`    |
| `yarn register-nodes` | `register-nodes.js`    |

## Node generation & registration

`generate-nodes.js` writes a manifest of Solana + EVM keypairs. Each node is created with `ip` and `port` set to `null`.

`register-nodes.js` registers those nodes on-chain. It refuses to send transactions unless every unregistered node has `ip` and `port` set in the manifest.

```bash
yarn generate-nodes --cluster devnet --count 5 --output generated-nodes-dev.json
# set nodes[].ip and nodes[].port, then:
yarn register-nodes --input generated-nodes-dev.json
```

Prerequisites for registration: `anchor build`, `yarn molpha init`, funded admin wallet (SOL + USDC).

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
