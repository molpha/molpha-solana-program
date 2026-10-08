# Scripts

Batch helpers for node operators, plus the secret scanner that CI runs. Everything else on chain
goes through the CLI; see [`cli/README.md`](../cli/README.md).

## Node setup

| Command                      | Script                     | What it does                                        |
| ---------------------------- | -------------------------- | --------------------------------------------------- |
| `yarn generate-nodes`        | `generate-nodes.js`        | Writes a manifest of Solana and secp256k1 keypairs  |
| `yarn register-nodes`        | `register-nodes.js`        | Registers the manifest's nodes with `register_node` |
| `yarn update-node-endpoints` | `update-node-endpoints.js` | Syncs each node's IP and port from the manifest     |

```bash
yarn generate-nodes --cluster localnet --count 5 --output generated-nodes.json
# set nodes[].ip and nodes[].port in the manifest, then:
yarn register-nodes --input generated-nodes.json
```

`register-nodes` needs a built IDL (`anchor build -p molpha`), an initialized protocol
(`yarn molpha init`) and an authority wallet funded with SOL and USDC. It refuses to send anything
until every unregistered node has `ip` and `port` set. Each script documents its options in its
header.

Without `--cluster`, the scripts use the provider cluster in `Anchor.toml`, which is `localnet`.

**The manifest contains private keys.** It is written with mode `0600` and `generated-nodes*.json`
is ignored by Git. Keep operational manifests outside the checkout and never commit one.

## Secret scanner

`scan-secrets.py` fails when it finds a Solana keypair file, a JSON field holding a signing secret,
a labelled plain-text key, a private-key PEM block, a common provider token or a populated `.env`
file. It prints paths and categories, never values.

```bash
python3 scripts/scan-secrets.py             # tracked and untracked, non-ignored files
python3 scripts/scan-secrets.py --staged    # the index, including force-added ignored files
python3 scripts/scan-secrets.py --history   # every blob reachable from branches, remotes and tags
python3 -m unittest discover -s scripts/tests -p 'test_*.py'
```

`test-key-allowlist.json` holds SHA-256 digests of the test fixtures and historical test keys that
are allowed to match. It contains no key material. An entry binds exact file contents, so an
edited file must be reviewed again. Treat any change to it as security-sensitive; see
[SECURITY.md](../SECURITY.md#test-keys-in-this-repository).
