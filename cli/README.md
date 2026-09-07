# Molpha CLI

`molpha` is the in-repo TypeScript CLI for operating the Molpha Solana program.

## Prerequisites

- `anchor build -p molpha --ignore-keys` (or `yarn build:molpha` after keys are synced)
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

```bash
yarn molpha --help
```

Development entrypoint:

```bash
yarn molpha:dev --help
```

## Command Groups

| Group | Commands |
|---|---|
| `init` | initialize protocol |
| `registry` | `register` \| `update-endpoint` \| `deactivate` \| `withdraw-deposit` \| `set-buffer` |
| `plan` | `add` \| `update` \| `show` |
| `subscription` | `subscribe` \| `extend` |
| `delegate` | `add` \| `remove` \| `pause` |
| `gateway` | `register` \| `deactivate` \| `withdrawal-status` \| `withdraw-bond` |
| `round` | `settle-subscription` \| `settle-x402` \| `finalize` \| `dispute` |
| `rewards` | `claim-node` |
| `attestation` | `sign` \| `submit` \| `verify` (historical, read-only) |
| `admin` | `update-config` |
| `account` | `protocol-config` \| `registry` \| `registry-version` \| `node` \| `plan` \| `subscription` \| `feed` \| `delegate` \| `gateway` \| `round` \| `secp-key` |

## Example Flow

Initialize protocol state:

```bash
yarn molpha init \
  --withdrawal-cooldown-secs 1 \
  --max-node-count 8 \
  --redundancy-buffer 2 \
  --minimum-node-deposit 1000000 \
  --reward-per-signature 10 \
  --gateway-fee 20 \
  --receipt-submitter-bonus 5 \
  --protocol-fee-bps 100 \
  --max-settlement-delay-seconds 256 \
  --protocol-fee 0 \
  --dispute-window-slots 256 \
  --gateway-bond-min 500000000 \
  --challenger-bounty-bps 500 \
  --x402-round-base 1000 \
  --x402-round-per-sig 15 \
  --usdc-mint <MINT>
```

`init` derives and creates the protocol treasury ATA from `protocol_config` + `usdc_mint`.

Create a plan and subscribe:

```bash
yarn molpha plan add \
  --plan-type standard \
  --subscription-price 1000 \
  --max-signers 3 \
  --max-delegates 100

yarn molpha plan add --plan-type basic --subscription-price 20000000 --max-signers 3 --max-delegates 10 --private-api-enabled --cluster devnet

yarn molpha subscription subscribe --plan-id 1
```

Register a node and submit an attestation. The first submit lazy-inits the feed:

```bash
yarn molpha registry register \
  --secp-pubkey <COMPRESSED_SECP256K1_PUBKEY> \
  --registration-salt <32_BYTE_HEX> \
  --pop-sig-r <32_BYTE_EVEN_NONCE_X_HEX> \
  --pop-sig-s <32_BYTE_RESPONSE_SCALAR_HEX>

yarn molpha attestation submit \
  --source-id <SOURCE_ID_HEX> \
  --registry-version 1 \
  --value <32_BYTE_HEX> \
  --canonical-timestamp 1700000123 \
  --signer-count 1 \
  --signatures-required 1 \
  --signer-seed molpha-cli-e2e-signers
```

Node PoP is generated off chain over
`keccak256("MOLPHA_NODE_POP_V1" || program_id || node_id)`. The CLI accepts only the
resulting signature — never the secp256k1 private key.

Inspect accounts:

```bash
yarn molpha account feed --source-id <SOURCE_ID_HEX> --signatures-required 1
yarn molpha account gateway --authority <GATEWAY_AUTHORITY>
yarn molpha account round --address <ROUND_RECORD>
```

Update protocol config (authority only; omitted fields keep current on-chain values):

```bash
yarn molpha admin update-config --gateway-bond-min 1000000000
```

Gateway + settlement:

```bash
yarn molpha gateway register --bond-amount 500000000 --ip 10.0.0.1 --port 8080

yarn molpha round settle-subscription \
  --source-id <SOURCE_ID_HEX> \
  --timestamp 1700000123 \
  --signatures-required 1 \
  --registry-version 1 \
  --gateway-authority <GATEWAY_AUTHORITY> \
  --credited-bits 0 \
  --consumer-wallet <CONSUMER_KEYPAIR>

yarn molpha round finalize \
  --round-record <ROUND_RECORD> \
  --gateway-authority <GATEWAY_AUTHORITY>

yarn molpha rewards claim-node
```

Node deposits and gateway bonds have separate exit paths:

```bash
# Run after registry deactivate and the node's cooldown.
yarn molpha registry withdraw-deposit

# Protocol authority: deactivate the target gateway and start its slot deadline.
yarn molpha gateway deactivate --gateway-authority <GATEWAY_AUTHORITY>
yarn molpha gateway withdrawal-status --gateway-authority <GATEWAY_AUTHORITY>

# Gateway authority: withdraw after the reported slot deadline.
yarn molpha gateway withdraw-bond --gateway-wallet <GATEWAY_KEYPAIR_PATH>
```

Deactivation writes `deactivated_at` and `withdrawable_slot` directly into the Gateway account.
Settlement stops immediately, while the bond remains slashable through
`deactivation_slot + dispute_window_slots`. Withdrawal opens at the first slot strictly beyond
that boundary. A successful dispute blocks withdrawal. A clean withdrawal closes only the bond
ATA; the deactivated Gateway account remains, so the same authority cannot register again.

A gateway-funded x402 round (any payment rail; the gateway pays from its canonical USDC ATA and
records how it was paid as attested metadata — see the program docs for `settle_x402_round`):

```bash
yarn molpha round settle-x402 \
  --source-id <SOURCE_ID_HEX> \
  --canonical-timestamp 1700000123 \
  --signatures-required 1 \
  --registry-version 1 \
  --amount <PRICE> \
  --payer-id <32_BYTE_HEX> \
  --payment-rail 1 \
  --gateway-wallet <GATEWAY_KEYPAIR> \
  --credited-bits 0
```

## Notes

- Seeds match the current program: `molpha_config`, `molpha_registry`, `molpha_feed`,
  `molpha_gateway`, `molpha_round`, `molpha_x402_round`, etc.
- Feed PDA is `[molpha_feed, source_id, signatures_required, submitter]`. Round PDAs
  also include `source_id` and `signatures_required` (not a derived `feed_id` or
  `api_config_hash`). `--submitter` defaults to the wallet.
- There is no `Job` account or `create_job` instruction. Feeds are created via
  `init_if_needed` inside `submit_attestation`. `source_id` is a caller-provided
  32-byte identifier.
- USDC accounts are always canonical ATAs derived by the CLI. Reward, challenger, and reporter
  ATAs are created by the program when missing; funded source ATAs must already exist.
- `attestation verify` only rebuilds/prints submit args off chain; verification happens on submit.
- Subscription payment uses monthly `subscribe` / `extend` (not per-round top-ups).
- `round settle-subscription` requires an Ed25519 RequestAuth pre-instruction; provide
  `--consumer-wallet` so the CLI can sign the auth message. The message is bound to the program id
  and to the `--gateway-authority` gateway PDA, so it is only redeemable by that gateway. `round settle-x402` has no
  on-chain payer authentication — the gateway self-attests and pays from its own float.
