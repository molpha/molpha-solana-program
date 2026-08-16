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
| `registry` | `register` \| `update-endpoint` \| `deactivate` \| `set-buffer` |
| `plan` | `add` \| `update` \| `show` |
| `subscription` | `subscribe` \| `extend` |
| `delegate` | `add` \| `remove` \| `pause` |
| `gateway` | `register` \| `claim` |
| `round` | `settle-subscription` \| `settle-agent` \| `finalize` \| `dispute` |
| `rewards` | `claim-node` |
| `data` | `sign` \| `submit` \| `verify` (historical, read-only) |
| `admin` | `update-config` |
| `account` | `protocol-config` \| `registry` \| `registry-version` \| `node` \| `plan` \| `subscription` \| `feed` \| `delegate` \| `gateway` \| `agent-escrow` \| `round` \| `secp-key` |

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
  --max-receipt-delay-slots 256 \
  --protocol-fee 0 \
  --dispute-window-slots 256 \
  --gateway-bond-min 500000000 \
  --challenger-bounty-bps 500 \
  --agent-round-base 1000 \
  --agent-round-per-sig 15 \
  --agent-epoch-len-secs 86400 \
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

yarn molpha subscription subscribe --plan-id 1 --owner-usdc <OWNER_USDC_ATA>
```

Register a node and submit data. The first submit lazy-inits the feed:

```bash
yarn molpha registry register \
  --secp-pubkey <COMPRESSED_SECP256K1_PUBKEY> \
  --registration-salt <32_BYTE_HEX> \
  --pop-sig-r <32_BYTE_EVEN_NONCE_X_HEX> \
  --pop-sig-s <32_BYTE_RESPONSE_SCALAR_HEX> \
  --owner-usdc <OWNER_USDC_ATA>

yarn molpha data submit \
  --feed-id <FEED_ID_HEX> \
  --registry-version 1 \
  --config-hash <32_BYTE_HEX> \
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
yarn molpha account feed --feed-id <FEED_ID_HEX>
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
  --api-config-hash <32_BYTE_HEX> \
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
yarn molpha gateway claim
```

## Notes

- Seeds match the current program: `molpha_config`, `molpha_registry`, `molpha_feed`,
  `molpha_gateway`, `molpha_round`, `molpha_agent`, `molpha_agent_round`, etc.
- There is no `Job` account or `create_job` / `verify_data_update` instruction.
  Feeds are created via `init_if_needed` inside `submit_data_update`.
- `data verify` only rebuilds/prints submit args off chain; verification happens on submit.
- Subscription payment uses monthly `subscribe` / `extend` (not per-round top-ups).
- Settlement requires an Ed25519 RequestAuth / AgentRequestAuth pre-instruction; provide
  `--consumer-wallet` / `--payer-wallet` so the CLI can sign the auth message.
