# Contributing

Issues and pull requests are welcome. For a security problem, follow [SECURITY.md](SECURITY.md)
instead of opening an issue.

## Setup

Install the toolchain listed in the [README](README.md#build-and-test), then:

```bash
yarn install --frozen-lockfile
anchor build -p molpha
```

## Before opening a pull request

Run what CI runs:

```bash
cargo fmt --all -- --check
yarn lint
cargo clippy --workspace --all-targets --locked -- -D warnings
anchor build -p molpha
cargo test --workspace --locked
yarn test:cli
python3 scripts/scan-secrets.py
```

The Rust tests load `target/deploy/molpha.so`. Rebuild the program after every change to it, or
the tests silently exercise the old binary.

## What we look for

- **Tests with the change.** A new instruction or constraint comes with LiteSVM tests that assert
  the specific error on failure and the resulting state on success. See
  [molpha-tests/README.md](molpha-tests/README.md) for the conventions.
- **Protocol changes move together.** The signed message layout, the node selection algorithm,
  PDA seeds and account layouts are shared with the CLI, the gateway, the nodes and the EVM
  contracts. A change to any of them must update the program, the CLI encoders in `cli/src/lib`
  and the docs in the same pull request, and should say what else needs to follow.
- **No secrets.** Never commit a keypair, a node manifest or an `.env` file. The secret scan
  fails the build if one appears.
- **Focused changes.** Keep refactors and dependency upgrades separate from behaviour changes.

By contributing you agree that your contribution is licensed under the
[Apache License 2.0](LICENSE).
