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
| `registry` | `register` \| `update-endpoint` \| `deactivate` \| `withdraw-deposit` \| `set-buffer` \| `reinstate` |
| `plan` | `add` \| `update` \| `show` |
| `subscription` | `subscribe` \| `extend` |
| `delegate` | `add` \| `remove` \| `pause` |
| `gateway` | `register` \| `deactivate` |
| `round` | `settle-subscription` \| `settle-x402` |
| `rewards` | `claim-node` |
| `attestation` | `sign` \| `submit` |
| `slash` | `inactivity` \| `offence` |
| `admin` | `update-config` |
| `account` | `protocol-config` \| `registry` \| `registry-version` \| `node` \| `plan` \| `subscription` \| `feed` \| `delegate` \| `gateway` \| `round-replay` \| `secp-key` \| `punishment-case` |

## Example Flow

Initialize protocol state:

```bash
yarn molpha init \
  --withdrawal-cooldown-slots 432000 \
  --redundancy-buffer 2 \
  --minimum-node-deposit 1000000 \
  --reward-per-signature 10 \
  --max-settlement-delay-seconds 256 \
  --challenger-bounty-bps 500 \
  --liveliness-freeze-threshold 5000 \
  --x402-round-base 1000 \
  --min-signers 3 \
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

`--value` is the 32-byte signed value and the feed stores it as `value_kind = Value`. For a longer
value pass `--raw-value <hex>` (<= 256 bytes) instead: the CLI signs `keccak256(raw)` as the value,
submits the preimage as `raw_value`, and the feed stores the hash as `value_kind = Hash`.
`attestation submit --args <path>` takes the `SubmitAttestationArgs` JSON that `attestation sign`
prints under `submitArgs` (`{ attestation: { payload, signature }, rawValue, coalitionKey }`).
`coalitionKey` is the affine sum of the signers' secp256k1 keys. When it is missing, `submit` sums it
from the signers' `Node` accounts. The settlement commands always derive it that way, and
`slash offence` does too unless the offence JSON already carries `coalitionKeyA` / `coalitionKeyB`
(pair offences) or `coalitionKey` (unsolicited signing).

Node PoP is generated off chain over
`keccak256("MOLPHA_NODE_POP_V1" || program_id || node_id)`. The CLI accepts only the
resulting signature — never the secp256k1 private key.

Inspect accounts:

```bash
yarn molpha account feed --source-id <SOURCE_ID_HEX> --signatures-required 1
yarn molpha account gateway --authority <GATEWAY_AUTHORITY>
yarn molpha account round-replay --address <ROUND_REPLAY_MARKER>
```

Update protocol config (authority only; omitted fields keep current on-chain values):

```bash
yarn molpha admin update-config --liveliness-penalty-per-miss 750 --liveliness-recovery-per-sign 200
```

Gateway + settlement:

```bash
yarn molpha gateway register --gateway-authority <GATEWAY_AUTHORITY> --ip 10.0.0.1 --port 8080

yarn molpha round settle-subscription \
  --source-id <SOURCE_ID_HEX> \
  --timestamp 1700000123 \
  --signatures-required 3 \
  --registry-version 1 \
  --gateway-authority <GATEWAY_AUTHORITY> \
  --agg-sig ./signature.json \
  --consumer-wallet <CONSUMER_KEYPAIR>

yarn molpha rewards claim-node
```

Node deposits retain their cooldown exit path:

```bash
# Run after registry deactivate and the node's cooldown.
yarn molpha registry withdraw-deposit

# Permissionless freeze of a node whose liveliness score is below the threshold.
yarn molpha slash inactivity --node-index 3
# or: yarn molpha slash inactivity --node-owner <NODE_OWNER>

# File any offence from IDL-shaped JSON (`{ equivocation | unsolicitedSigning | commitmentReuse | nonceReuse | inactivity }`).
# The CLI derives the PunishmentCase PDA from the evidence hash; pass --evidence-hash to assert it.
yarn molpha slash offence --args ./offence.json
yarn molpha account punishment-case --evidence-hash <32_BYTE_HEX>

# Restore a frozen node after its freeze window (anyone), or earlier as protocol authority.
yarn molpha registry reinstate --node-owner <NODE_OWNER>

# Protocol authority: deactivate the target gateway immediately.
yarn molpha gateway deactivate --gateway-authority <GATEWAY_AUTHORITY>
```

Deactivation stops both settlement paths immediately. Gateway registration no longer transfers or
tracks a bond.

A gateway-funded x402 round (any payment rail; the gateway pays from its canonical USDC ATA and
records how it was paid as attested metadata — see the program docs for `settle_x402_round`):

```bash
yarn molpha round settle-x402 \
  --source-id <SOURCE_ID_HEX> \
  --canonical-timestamp 1700000123 \
  --signatures-required 3 \
  --registry-version 1 \
  --amount <PRICE> \
  --payer-id <32_BYTE_HEX> \
  --payment-rail 1 \
  --gateway-wallet <GATEWAY_KEYPAIR> \
  --agg-sig ./signature.json
```

## Notes

- Seeds match the current program: `molpha_config`, `molpha_registry`, `molpha_feed`,
  `molpha_gateway`, `molpha_round`, `molpha_x402_round`, etc.
- Feed PDA is `[molpha_feed, source_id, signatures_required, submitter]`. Settlement creates a
  minimal consumer-scoped round replay marker; it stores no attestation or pending state.
- There is no `Job` account or `create_job` instruction. Feeds are created via
  `init_if_needed` inside `submit_attestation`. `source_id` is a caller-provided
  32-byte identifier.
- USDC accounts are always canonical ATAs derived by the CLI. Reward ATAs are created by the
  program when missing; funded source ATAs must already exist. `slash` creates the reporter USDC
  ATA as a pre-instruction if needed (the program itself does not).
- Subscription payment uses monthly `subscribe` / `extend` (not per-round top-ups).
- `round settle-subscription` requires an Ed25519 RequestAuth pre-instruction; provide
  `--consumer-wallet` so the CLI can sign the auth message. The message is bound to the program id
  and to the `--gateway-authority` gateway PDA, so it is only redeemable by that gateway. `round settle-x402` has no
  on-chain payer authentication — the gateway self-attests and pays from its own float.

Protocol initialization accepts `--liveliness-penalty-per-miss` (default 10) and
`--liveliness-recovery-per-sign` (default 90), a 10% break-even inclusion rate. JSON config uses
`livelinessPenaltyPerMiss` and `livelinessRecoveryPerSign`. Updates preserve omitted values.
These settings take effect during settlement.

Both admin instructions require positive withdrawal cooldown, settlement delay, and node deposit.
Liveliness threshold, penalty, and recovery must
be 1–10,000; challenger bounty is 0–10,000; minimum signers is 3–255; initialization
redundancy is 1–8. Zero reward and base price are allowed. Slot/timestamp deadlines and
the base price plus rewards for 256 signers must not overflow. The CLI checks static
bounds before sending; the program also checks deadlines against its current clock.

The removed config fields change the account layout and instruction encoding. Rebuild the
program and IDL before using this CLI; existing config accounts require migration or
reinitialization in a fresh deployment.

Run the CLI config tests with `yarn test:cli`.
