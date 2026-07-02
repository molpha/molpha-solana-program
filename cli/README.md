# Molpha CLI

`molpha` is the in-repo TypeScript CLI for operating the Molpha Solana program.

## Prerequisites

- `anchor build` or `yarn build:molpha`
- `yarn build:cli`
- A funded wallet (`ANCHOR_WALLET` or `--wallet`)
- A reachable RPC (`--rpc` or `ANCHOR_PROVIDER_URL`)

By default the CLI reads:

- program id from `Anchor.toml`
- wallet from `ANCHOR_WALLET` or `~/.config/solana/id.json`
- IDL from `target/idl/molpha.json`

## Build

```bash
yarn build:cli
```

Run the built CLI:

```bash
yarn molpha --help
```

Or use the TypeScript entrypoint during development:

```bash
yarn molpha:dev --help
```

## Command Groups

- `molpha init`
- `molpha registry register|deactivate|finalize-transition`
- `molpha plan add|update`
- `molpha subscription subscribe|top-up`
- `molpha job create`
- `molpha delegate add|remove`
- `molpha data sign|submit|verify`
- `molpha account protocol-config|registry|node|plan|subscription|job|feed|registry-index|registry-transition`

## Example Flow

Initialize protocol state:

```bash
yarn molpha --idl "/Users/vitaliikoval/dev/molpha-solana-program/target/idl/molpha.json" init \
  --registry-cooldown-secs 1 \
  --previous-grace-secs 1 \
  --withdrawal-cooldown-secs 1 \
  --max-node-count 8 \
  --minimum-node-deposit 1000000 \
  --reward-per-signature 10 \
  --gateway-fee 20 \
  --receipt-submitter-bonus 5 \
  --protocol-fee-bps 100 \
  --max-receipt-delay-slots 256 \
  --usdc-mint <MINT>
```

`init` now derives and creates the protocol treasury ATA automatically from
`protocol_config` + `usdc_mint`. The command output includes the treasury
address.

Create a plan and subscribe:

```bash
yarn molpha plan add \
  --plan-id 1 \
  --plan-type standard \
  --subscription-price-per-round 1000 \
  --max-jobs 4 \
  --group-size 1 \
  --signatures-required 1 \
  --max-signers 1

yarn molpha subscription subscribe --plan-id 1 --rounds 5 --owner-usdc <OWNER_USDC_ATA>
```

Register a node and submit data. The first submit initializes the job/feed if needed:

```bash
yarn molpha registry register \
  --secp-pubkey <COMPRESSED_SECP256K1_PUBKEY> \
  --registration-salt <32_BYTE_HEX> \
  --owner-usdc <OWNER_USDC_ATA> \
  --security-deposit 1000000

yarn molpha data submit \
  --job-id <JOB_ID_HEX> \
  --registry-version 1 \
  --config-hash <32_BYTE_HEX> \
  --decimals 8 \
  --value <32_BYTE_HEX> \
  --canonical-timestamp 1700000123 \
  --signer-count 1 \
  --signer-seed molpha-cli-e2e-signers
```

Verify and inspect:

```bash
yarn molpha data verify \
  --job-id <JOB_ID_HEX> \
  --registry-version 1 \
  --config-hash <32_BYTE_HEX> \
  --value <32_BYTE_HEX> \
  --canonical-timestamp 1700000123 \
  --signer-count 1 \
  --signer-seed molpha-cli-e2e-signers

yarn molpha account feed --job-id <JOB_ID_HEX>
```

## Notes

- The CLI uses the current v0.3 seeds (`molpha_config`, `molpha_registry`, `molpha_job`, `molpha_feed`, etc.).
- Subscription top-ups, node deposits, and admin withdrawals all use the same protocol treasury ATA.
- Use `yarn molpha` for all on-chain operations (init, registry, plans, subscriptions, jobs, account reads).
- `data sign`, `data submit`, and `data verify` can also consume fixtures with `--fixture`.
