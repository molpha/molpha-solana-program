# Security policy

## Reporting a vulnerability

Report vulnerabilities privately through GitHub: open the repository's **Security** tab and choose
**Report a vulnerability**. Please do not open a public issue or pull request for a security
problem.

Include what you can of:

- the affected instruction, account or file, and the commit you tested;
- a description of the impact;
- steps or a test that reproduces it. A LiteSVM test in `molpha-tests` is ideal.

We aim to acknowledge a report within five working days and will keep you informed while we
investigate.
Please give us a reasonable chance to fix the issue before disclosing it.

## Scope

In scope: the program in `programs/molpha`, the CLI in `cli`, and the scripts in `scripts`.

Signature verification lives in [`molpha-verifier`](https://github.com/molpha/molpha-verifier);
report issues in that code to that repository.

Known limitations that are already documented are listed in
[guides/ARCHITECTURE.md](guides/ARCHITECTURE.md#trust-model-and-known-limitations). Reports that
show a concrete exploit beyond what is described there are welcome.

## Status

The program is in beta, deployed on devnet only, and has not been audited. Only the latest commit
on the default branch is supported.

## Test keys in this repository

The Git history and the test fixtures contain signing keys that were generated for tests:
deterministic secp256k1 vectors for the cross-chain compatibility tests, and a few early
development keypairs. They are public. Never fund them, give them any authority, or use them to
sign on a public network.

CI scans every push for key material. The exceptions for those test keys are bound to exact file
contents in `scripts/test-key-allowlist.json`; see [scripts/README.md](scripts/README.md#secret-scanner).
Never add an operational key to that list.
