# Disposable test keys and release hygiene

The repository owner confirmed on 8 October 2026 that all exposed keys identified
in the public-release review are disposable test keys with no balance or active
authority. This includes historical Solana keypair files, generated node manifests
and EVM signing fixtures. Operational key rotation is therefore not required for
this release. Git history is retained on that basis; it has not been rewritten.

Treat every identity represented by those historical files as publicly known.
Never fund it, give it upgrade/configuration/dispatch authority, or use it for node
signing on a public network. Generate separate disposable identities for local
tests and keep operational credentials outside this repository.

`scripts/test-key-allowlist.json` records SHA-256 digests of the exact reviewed
historical file contents, their paths and detection categories. It contains no
key values. Historical exceptions are content-bound: editing a file creates a new
digest that must be reviewed. Only the existing EVM golden fixture is allowed in
the current source tree. Root keypairs and operator manifests remain local and
ignored; adding them with `git add -f` will fail the staged-file scanner.

The EVM fixture in `molpha-tests/fixtures/verify-answer-evm.json` intentionally
contains deterministic signing secrets for cross-chain compatibility tests.
Removing them would discard reproducible verification vectors. Rust test helpers
also generate ephemeral keys or use fixed test-only scalars.

Run the gates from the repository root:

```bash
python3 scripts/scan-secrets.py
python3 scripts/scan-secrets.py --staged
python3 scripts/scan-secrets.py --history
python3 -m unittest discover -s scripts/tests -p 'test_*.py'
```

The default scan includes tracked working-tree files and non-ignored untracked
files. The staged scan reads index blobs, including forced additions of ignored
files. The history scan reads blobs reachable from all local refs. CI fetches full
history before scanning. Findings print paths and categories only, never matching
values. The scanner covers Solana JSON keypairs, structured signing secrets,
private-key PEM blocks, common provider tokens and populated `.env` files. It is
not an exhaustive detector for arbitrary credentials or a check of remote balances.

Review every allowlist change as a security-sensitive change. Do not add exceptions
for operational credentials, or disable the CI gate to publish a new key.

Local drafts and operator experiments are excluded from the public source package
by explicit `.gitignore` entries. They remain on disk. The maintained API and
integration guides are in this directory. Review new release files explicitly;
never bulk-add an operator checkout.
