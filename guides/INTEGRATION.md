# Integrating with this source release

Use the program's generated IDL and the operator CLI from the same checkout.
The test suite supplies local instruction examples; there is no separately
packaged price-client implementation in this repository.

## Build and inspect

Use Rust 1.97.1, Anchor 1.2.0, Solana CLI 4.1.1, Node 22 and Yarn 1.22.22 as
configured in the repository/CI. Install dependencies and build:

```bash
yarn install --frozen-lockfile
NO_DNA=1 anchor build -p molpha
yarn build:cli
yarn molpha --help
yarn molpha account feed --help
```

The build creates `target/idl/molpha.json` and `target/deploy/molpha.so`. Neither
is tracked. Copy the generated IDL into a consumer project only together with
the source revision it represents. Check the selected deployment address before
connecting; `Anchor.toml` defaults the Anchor provider to devnet.

## Local verification

```bash
NO_DNA=1 cargo test --workspace --locked
yarn test:cli
```

LiteSVM tests create their own disposable keys and do not send remote transactions.
Rebuild the `.so` after program changes. See `molpha-tests/tests/attestation.rs`
for publication/reading behavior and `molpha-tests/tests/evm_compatibility.rs`
for signature fixtures. CLI account reads require a wallet file to construct
the Anchor provider even though the read does not send a transaction.

For a separately provisioned local validator and deployed/initialized program,
read a known feed with an explicit loopback RPC and a disposable local wallet:

```bash
yarn molpha --cluster localnet --rpc http://127.0.0.1:8899 \
  --wallet /absolute/path/to/disposable-local-wallet.json \
  account feed --address <feed-public-key>
```

This command reads state; it does not set up a validator or initialize the protocol.
Follow [`cli/README.md`](../cli/README.md) for operator setup, using explicit local
options rather than copying remote deployment configurations.

## Consumer acceptance

Choose the source ID, expected submitter and quorum explicitly when deriving the
feed PDA. Validate account ownership, discriminator and data before trusting it.
Enforce application freshness using the feed's millisecond timestamp. Interpret
`value` according to `value_kind` and your source's agreed encoding; hashed feeds
need the preimage and a hash check. Treat registries, signatures and source
encoding as one versioned protocol across gateway, nodes and consumers.

See the [API reference](API_REFERENCE.md) for the actual program interface and
the [test-key policy](TEST_KEYS.md) before handling any historical example key.
