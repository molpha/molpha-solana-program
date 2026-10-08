# Molpha CLI

`molpha` is the TypeScript CLI for operating the Molpha Solana program.

## Setup

```bash
yarn install --frozen-lockfile
anchor build -p molpha    # the CLI reads the IDL this produces
yarn build:cli
yarn molpha --help
```

You also need a funded wallet and a reachable RPC. Global options, with their defaults:

| Option         | Default                                                              |
| -------------- | -------------------------------------------------------------------- |
| `--cluster`    | `localnet` (`localnet`, `devnet` or `mainnet`)                       |
| `--rpc`        | `ANCHOR_PROVIDER_URL`, else the cluster's public endpoint            |
| `--wallet`     | `ANCHOR_WALLET`, else `~/.config/solana/id.json`                     |
| `--program-id` | The cluster's entry in `Anchor.toml`                                 |
| `--idl`        | `target/idl/molpha.json`                                             |
| `--commitment` | `confirmed`                                                          |

## Command Groups

| Group | Commands |
|---|---|
| `init` | initialize protocol |
| `registry` | `register` \| `update-endpoint` \| `deactivate` \| `withdraw-deposit` \| `set-buffer` \| `reinstate` |
| `plan` | `add` \| `update` \| `show` |
| `subscription` | `subscribe` \| `extend` |
| `delegate` | `add` \| `remove` |
| `gateway` | `register` \| `deactivate` |
| `epoch` | `show` \| `finalize` \| `credit` \| `close` |
| `ticket` | `submit` \| `close` |
| `revenue` | `sweep-treasury` \| `withdraw-reserve` |
| `rewards` | `claim-node` |
| `attestation` | `sign` \| `submit` |
| `slash` | `inactivity` \| `offence` |
| `admin` | `update-config` |
| `account` | `protocol-config` \| `registry` \| `registry-version` \| `node` \| `plan` \| `subscription` \| `feed` \| `delegate` \| `gateway` \| `epoch` \| `ticket-marker` \| `secp-key` \| `punishment-case` |

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
  --x402-round-base 1000 \
  --min-signers 3 \
  --protocol-fee-bps 500 \
  --usdc-mint <MINT>
```

`init` derives and creates the protocol treasury ATA from `protocol_config` + `usdc_mint`, and the
genesis epochs 0 and 1. The fee flags have no defaults. Epoch knobs default to a 1-day epoch,
6-hour ticket grace, a 2,000-ticket target, an 80% relative availability floor and 50 samples
(`--epoch-len-seconds`, `--ticket-grace-seconds`, `--target-tickets-per-epoch`,
`--min-availability-bps`, `--min-availability-samples`).

The genesis epochs also carry the ticket threshold. Size it for the load you expect with
`--expected-rounds-per-epoch <N>` (`2^64 * target / N`, clamped to the program's floor of 2^32), or
set it directly with `--initial-tau <u64|max>`. Without either, every round of the first two epochs
is a ticket (a warning is printed): the controller moves the threshold at most 8x per epoch with a
two-epoch lag, so a high-volume deployment that starts unsized pays one ticket transaction per round
for a whole epoch or more.

Run the keeper (finalize, credit, close and sweep, all permissionless) as a daemon:

```bash
yarn molpha keeper --interval-seconds 60 \
  --rpc-fallback https://rpc-2.example --priority-fee-micro-lamports 1000
yarn molpha keeper --once   # one pass; exits 2 when the epoch lifecycle is unhealthy
```

It logs one JSON object per line. Health signals: `no-open-epoch` (no epoch covers the cluster
time: rounds cannot be ticketed and income cannot be swept until an overdue epoch is finalized),
`finalize-overdue` (an epoch is more than `--finalize-overdue-seconds`, default 900, past its
ticket deadline: the ticket threshold only adapts in `finalize_epoch`) and `epoch-runway-low`
(the newest epoch ends within `--runway-seconds`, default 7200). Each pass finalizes epochs past
their ticket deadline in order (each finalize creates the account the next one needs), credits
finalized epochs, closes them after their evidence window, and sweeps the open epoch at most every
`--sweep-interval-seconds` (default 3600). The last sweep time is stored in
`~/.config/molpha/keeper/<program>-<wallet>.json` (override with `--state-file`) so `--once` cron
jobs and daemon restarts respect the interval. A failing endpoint is retried on the next
`--rpc-fallback` URL; a program error is not. Catch-up after an outage is two finalizes per pass.
Ticket-marker rent is returned by whoever created the markers (the gateway does it for its own);
`ticket close` closes one by hand.

Create a plan and subscribe:

```bash
yarn molpha plan add \
  --plan-type standard \
  --subscription-price 1000 \
  --max-signers 3 \
  --max-delegates 100

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
  --timestamp 1700000123000 \
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
from the signers' `Node` accounts. `ticket submit` always derives it that way, and
`slash offence` does too unless the offence JSON already carries `coalitionKeyA` / `coalitionKeyB`
(pair offences) or `coalitionKey` (unsolicited signing).

Node PoP is generated off chain over
`keccak256("MOLPHA_NODE_POP_V1" || program_id || node_id)`. The CLI accepts only the
resulting signature, never the secp256k1 private key.

Inspect accounts:

```bash
yarn molpha account feed --source-id <SOURCE_ID_HEX> --signatures-required 1
yarn molpha account gateway --authority <GATEWAY_AUTHORITY>
yarn molpha account epoch --epoch 0
yarn molpha account ticket-marker --ticket-id <32_BYTE_HEX>
```

Update protocol config (authority only; omitted fields keep current on-chain values):

```bash
yarn molpha admin update-config --min-availability-bps 7500 --target-tickets-per-epoch 4000
```

Gateway, income and epoch rewards:

```bash
yarn molpha gateway register --gateway-authority <GATEWAY_AUTHORITY> --ip 10.0.0.1 --port 8080

# Permissionless: x402 payments and subscriptions land in the treasury (x402 payTo); book
# unswept income into the epoch open now (found automatically).
yarn molpha revenue sweep-treasury

# Submit a sampled round. The gateway's dispatch signature over the round is required; the CLI
# checks the lottery locally and derives roster hints and the coalition key.
yarn molpha ticket submit \
  --source-id <SOURCE_ID_HEX> \
  --timestamp 1700000123000 \
  --signatures-required 3 \
  --registry-version 1 \
  --value <32_BYTE_HEX> \
  --agg-sig ./signature.json \
  --epoch 0 \
  --gateway-authority <GATEWAY_AUTHORITY> \
  --dispatch-sig <64_BYTE_HEX>

# After end + ticket_grace: freeze weights, create epoch + 2, credit every weighted node.
yarn molpha epoch finalize --epoch 0
yarn molpha epoch credit --epoch 0
yarn molpha rewards claim-node

# One epoch length later: roll the residual forward and reclaim rent.
yarn molpha epoch close --epoch 0
yarn molpha ticket close --ticket-id <32_BYTE_HEX>

# Protocol authority: withdraw protocol fees and slashing proceeds.
yarn molpha revenue withdraw-reserve --amount <AMOUNT> --destination <USDC_ACCOUNT>
```

Node deposits retain their cooldown exit path:

```bash
# Run after registry deactivate and the node's cooldown.
yarn molpha registry withdraw-deposit

# Permissionless freeze of a node below the availability floor of a finalized epoch.
yarn molpha slash inactivity --epoch 0 --node-owner <NODE_OWNER>

# File any offence from IDL-shaped JSON (`{ equivocation | unsolicitedSigning | commitmentReuse | nonceReuse | inactivity }`).
# The CLI derives the PunishmentCase PDA from the evidence hash; pass --evidence-hash to assert it.
yarn molpha slash offence --args ./offence.json
yarn molpha account punishment-case --evidence-hash <32_BYTE_HEX>

# Restore a frozen node after its freeze window (anyone), or earlier as protocol authority.
yarn molpha registry reinstate --node-owner <NODE_OWNER>

# Protocol authority: deactivate the target gateway immediately.
yarn molpha gateway deactivate --gateway-authority <GATEWAY_AUTHORITY>
```

Deactivation stops the gateway's dispatch signatures from admitting tickets immediately.

## Notes

- Seeds match the current program: `molpha_config`, `molpha_registry`, `molpha_feed`,
  `molpha_gateway`, `molpha_epoch`, `molpha_ticket`, etc.
- Feed PDA is `[molpha_feed, source_id, signatures_required, submitter]`. A ticket creates a
  marker keyed by round identity `(source_id, signatures_required, timestamp)`.
- Feeds are created via `init_if_needed` inside `submit_attestation`. `source_id` is a caller-provided
  32-byte identifier.
- USDC accounts are always canonical ATAs derived by the CLI. Reward ATAs are created by the
  program when missing; funded source ATAs must already exist. `slash` creates the reporter USDC
  ATA as a pre-instruction if needed (the program itself does not).
- Subscription payment uses monthly `subscribe` / `extend`.
- Subscription round quotas and delegate request limits are enforced by the gateway at admission;
  nothing on chain consumes them.
- `ticket submit` needs the dispatch signature immediately before the ticket instruction. Pass
  `--dispatch-sig` (from the gateway), or `--gateway-wallet` to sign locally in tests.

Epoch parameters, including the ticket `min_signers` floor, apply to epochs created after an
update. Existing epochs keep their ticket rules; feed submission uses the current floor. Updates
preserve omitted values.

Both admin instructions require positive withdrawal cooldown, attestation delay, and node deposit.
Epoch length is 3,600 s to 30 days with a positive grace shorter than the epoch; the ticket target
is 1–8,191; the availability floor is 0–10,000 bps with at least one sample; gateway plus protocol
fee is at most 10,000 bps; challenger bounty is 0–10,000; minimum signers is 3–255;
initialization redundancy is 1–8. Zero reward and base price are allowed. Slot/timestamp deadlines and
the base price plus rewards for 256 signers must not overflow. The CLI checks static
bounds before sending; the program also checks deadlines against its current clock.

Run the CLI tests with `yarn test:cli`.
