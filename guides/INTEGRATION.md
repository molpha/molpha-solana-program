# Integration guide

How to read a Molpha feed from a consumer. There is no packaged client SDK in this repository:
use the IDL and, for inspection, the CLI, both built from the same commit.

## Build the IDL and CLI

```bash
yarn install --frozen-lockfile
anchor build -p molpha
yarn build:cli
```

This writes `target/idl/molpha.json` and `target/deploy/molpha.so`. Neither is tracked. If you
copy the IDL into another project, record the commit it came from, and check that the program ID
you connect to is the one you expect.

## Read a feed

A feed's address is the PDA `[molpha_feed, source_id, signatures_required, submitter]` under the
program ID. Derive it from values you chose in advance, not from values an untrusted party hands
you.

With the CLI, against a local validator where the program is deployed and initialized:

```bash
yarn molpha --cluster localnet --rpc http://127.0.0.1:8899 \
  --wallet /path/to/disposable-wallet.json \
  account feed --source-id <SOURCE_ID_HEX> --signatures-required 3 --submitter <SUBMITTER>
```

`account feed --address <pubkey>` reads a known address directly. The CLI needs a wallet file to
build its provider even though a read sends no transaction; use a disposable one.

## What a consumer must check

1. **Ownership.** The account is owned by the Molpha program and has the `Feed` discriminator.
2. **Identity.** `source_id`, `signatures_required` and `submitter` are the ones you expect. A
   feed with a lower quorum or another submitter is a different feed.
3. **Freshness.** `timestamp` is in unix milliseconds. Compare `timestamp / 1000` with the cluster
   clock and reject anything older than your application tolerates.
4. **Encoding.** `value` is 32 bytes whose meaning is agreed per source. When `value_kind` is
   `Hash`, obtain the preimage and check `keccak256(preimage) == value`.
5. **Registry.** `registry_version` tells you which node set signed.

A valid signature shows that a quorum of registered nodes signed the value. It does not make the
value fresh or correct for your use; those checks are yours.

## Local testing

```bash
cargo test --workspace --locked
yarn test:cli
```

The LiteSVM tests create their own keys and send nothing to a network.
`molpha-tests/tests/attestation.rs` shows publication and reads end to end, and
`molpha-tests/tests/evm_compatibility.rs` checks the cross-chain signature vector.

See the [API reference](API_REFERENCE.md) for the account layout and
[Architecture](ARCHITECTURE.md) for the trust model.
