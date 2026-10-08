# Architecture

How the program's accounts and instructions fit together. For the instruction list see the
[API reference](API_REFERENCE.md); for exact arguments and account constraints use the IDL built
from the same commit.

## Accounts and PDAs

| Account         | Seed prefix           | Additional seeds                                          |
| --------------- | --------------------- | --------------------------------------------------------- |
| Protocol config | `molpha_config`       | none                                                      |
| Registry state  | `molpha_registry`     | none                                                      |
| Registry        | `molpha_registry`     | version (`u32` LE)                                        |
| Node            | `molpha_node`         | node owner                                                |
| Secp key claim  | `molpha_secp_key`     | `keccak256(compressed pubkey)`                            |
| Plan            | `molpha_plan`         | plan type byte                                            |
| Subscription    | `molpha_subscription` | subscriber owner                                          |
| Delegate        | `molpha_delegate`     | owner, delegate                                           |
| Feed            | `molpha_feed`         | `source_id` (32), `signatures_required` (`u8`), submitter |
| Gateway         | `molpha_gateway`      | gateway authority                                         |
| Epoch state     | `molpha_epoch`        | epoch index (`u64` LE)                                    |
| Ticket marker   | `molpha_ticket`       | `keccak("MOLPHA_TICKET_ID_V1" ‖ source_id ‖ k ‖ ts BE)`   |
| Punishment case | `molpha_punishment`   | evidence hash                                             |

Two token accounts are associated token accounts rather than PDAs, with the program enforcing
their addresses:

| Account            | Authority             | Holds                                                    |
| ------------------ | --------------------- | -------------------------------------------------------- |
| Protocol treasury  | `protocol_config` PDA | Subscription and x402 income, epoch pools, rewards, reserve |
| Node deposit vault | `registry_state` PDA  | Every node's security deposit, apart from revenue        |

## Node registry

- `register_node` and `deactivate_node` each create an immutable `Registry` version `N+1`.
  `update_registry_policy` snapshots a redundancy-policy change without changing membership.
- `Node` accounts are keyed by owner and are permanent, so historical rounds and rewards can
  always be resolved.
- A node registers a compressed secp256k1 key with a domain-separated Schnorr proof of possession
  over `keccak256("MOLPHA_NODE_POP_V1" ‖ program_id ‖ node_id)`. The key is stored as affine
  coordinates and reserved forever by a key-claim PDA. The verifier's hot path relies on this
  registration check.
- Deposits sit in the shared node deposit vault; each node's `locked_amount` tracks its share.

## Plans, subscriptions and delegates

- The authority defines plans (`add_plan`, `update_plan`) with a price, a signer limit and a
  delegate limit.
- `subscribe` and `extend_subscription` pay monthly in USDC into the treasury.
- A subscription owner can add delegates up to the plan's `max_delegates`.
- Round quotas and delegate request limits are enforced by the gateway at admission. Nothing on
  chain consumes them.

## Publishing a round

`submit_attestation` creates the feed on first use and updates it afterwards. For each round it:

1. Rejects a stale or duplicate round for an existing feed (same source and quorum, timestamp not
   newer, registry not older) before any signature work, at about 17-20K CU.
2. Re-derives the selected node set from the registry snapshot and the round, matching the EVM
   `NodeGroupBitmapLib`.
3. Verifies the coalition Schnorr signature over a message that binds `source_id`, value,
   registry version, quorum, timestamp and signer bitmap.
4. Writes the value, bitmap, timestamp and registry version to the `Feed` account.

The signed value is 32 bytes. For a longer value (up to 256 bytes) the signers sign
`keccak256(raw_value)`, the submitter passes the preimage, and the feed records
`value_kind = Hash`.

### Coalition key

Every instruction that verifies an aggregate (`submit_attestation`, `submit_ticket`, and the
equivocation, commitment-reuse and unsolicited-signing evidence) also takes
`CoalitionKey { x, y }`: the affine sum of the signers' secp256k1 keys, which any library can
compute from their `Node` accounts in any order. It is unsigned and outside the evidence hash. The
verifier checks it against its own sum instead of inverting that sum, which would cost about
138K CU. A wrong key fails with `InvalidCoalitionKey`.

## Epoch rewards

No account is written per round. Three flows cover income, measurement and payout.

### Income

All income lands in the treasury: subscriptions pay it, and it is the x402 `payTo`. Gateways hold
no funds, take no fee and earn no rewards. The permissionless `sweep_treasury_revenue` books
unswept income into the pool of the epoch open at cluster time and keeps `protocol_fee_bps` as
protocol reserve. `withdraw_protocol_reserve` lets the authority withdraw fees and slashing
proceeds, never pools or rewards.

The treasury ledger is `reward_liability` (sum of claimable rewards) + `pool_liability` (sum of
live pools) + `protocol_reserved`. Anything above it is unswept income.

`reward_per_signature` and `x402_round_base` are published x402 pricing inputs only; they are not
read on chain.

### Measurement

A round is a **ticket** when the big-endian `u64` prefix of
`keccak("MOLPHA_TICKET_V1" ‖ m ‖ s ‖ commitment)` is below its epoch's threshold `tau`. Anyone
may `submit_ticket`. The round must:

- fall inside the epoch window and arrive before `end + ticket_grace`;
- use a registry version created no later than its timestamp and still live then;
- carry, in the immediately preceding instruction, the dispatching gateway's Ed25519 signature
  over `keccak("MOLPHA_DISPATCH_V1" ‖ program_id ‖ source_id ‖ k ‖ version ‖ ts)`;
- verify in full, like a published round.

The round's whole selection is re-derived and counted in the epoch's `EpochState` as selected and
signed per node. Counters are keyed by an epoch roster of `Node` PDAs, because registry positions
move between versions. A `TicketMarker` keyed by round identity `(source_id, k, ts)` stops the
same round counting twice; it can be closed once the epoch stops accepting tickets. An epoch pins
`min_signers` when it is created, so later config updates do not invalidate its pending tickets.

The round timestamp `ts` is in unix milliseconds. Epoch boundaries and chain clocks are in
seconds.

### Payout

After the ticket deadline anyone may `finalize_epoch`. Weights freeze, and the epoch two ahead is
created with `tau` rescaled toward `target_tickets_per_epoch` by at most 8x per step, from the
ticket count of the epoch being finalized.

- A node's weight is its sampled signature count, or zero when it has at least
  `min_availability_samples` selections and an inclusion rate below `min_availability_bps` of the
  network rate.
- `credit_epoch_rewards` adds `floor(pool · w / W)` to `Node.claimable_rewards`. Tombstoned nodes
  forfeit.
- `claim_node_rewards` withdraws.
- `close_epoch` waits one more epoch length, the inactivity evidence window, then rolls the
  residual into the open epoch.

### Sizing the first epochs

The two genesis epochs carry `initialize`'s `initial_tau`, which must be at least `TAU_MIN`
(2^32, a sampling probability of 2^-32). `u64::MAX` samples every round. Size it as
`2^64 · target_tickets_per_epoch / expected_rounds_per_epoch`. Because the controller moves the
threshold by at most 8x per epoch with a two-epoch lag, an unsized start makes every round of the
first epochs a ticket transaction.

## Slashing

`slash_offence` is permissionless and takes an `Offence`: equivocation, unsolicited signing,
commitment reuse, nonce reuse or inactivity. Proven evidence writes a `PunishmentCase` PDA seeded
by the evidence hash, so the same filing cannot replay.

- Equivocation, unsolicited signing, commitment reuse and nonce reuse slash the guilty set,
  tombstone it and evict it into a successor registry.
- Inactivity freezes a node whose inclusion rate in a finalized epoch is below the availability
  floor, without taking principal. There is one case per node per epoch. `reinstate_node` restores
  the node after its freeze window, or earlier when called by the authority.

## Withdrawals

- `withdraw_node_deposit` releases a deactivated node's deposit after `withdrawable_at`, or the
  surviving balance of a tombstoned node after its punishment cooldown. Rewards use the separate
  `claim_node_rewards` path.
- `deactivate_gateway` immediately stops the gateway's dispatch signatures from admitting tickets.
  Gateway registration is bondless, and a deactivated identity cannot re-register.

## Limits

Both tables come from `cargo test -p molpha-tests --test suite limits compute_bench -- --nocapture`.
The `limits` tests assert the `submit_attestation` numbers rather than only reporting them.

### `submit_attestation`

| Signers | Compute units | Transaction bytes |
| ------- | ------------- | ----------------- |
| 5       | ~91K          | 742               |
| 10      | ~142K         | 907               |
| 15      | ~183K         | 1,072             |
| 19      | ~223K         | 1,204             |
| 20      | n/a           | 1,237 (too large) |

19 signers is the ceiling for a legacy transaction: each extra signer costs 33 bytes, the 64-byte
coalition key costs about two signers' worth, and the packet limit is 1,232 bytes. Compute is not
the binding constraint. Set the compute unit limit to at least 400,000 for large coalitions, and
go past 19 signers only with address lookup tables.

### `submit_ticket`

A ticket transaction also carries the gateway's Ed25519 dispatch instruction, so it fits fewer
signers.

| Signers | Compute units (whole transaction) | Fits                          |
| ------- | --------------------------------- | ----------------------------- |
| 8       | ~132K                             | legacy                        |
| 10      | n/a                               | legacy ceiling                |
| 12      | ~166K                             | v0 + lookup table             |
| 18      | ~223K                             | v0 + lookup table             |
| 20      | ~249K                             | v0 + lookup table (991 bytes) |

Put the signer `Node` accounts in the lookup table; the marker, epoch and payer stay static. Every
ticket write-locks its epoch's `EpochState`, which is acceptable at the sampled rate.

## Trust model and known limitations

The program is in beta and assumes a permissioned operator set and a Molpha-operated gateway.

**What the protocol authority can do**

- Change protocol configuration, plans and the registry redundancy policy.
- Admit nodes and gateways: `register_node` and `register_gateway` both require its signature.
- Deactivate any node or gateway, and reinstate a frozen node early.
- Withdraw the protocol reserve (fees and slashing proceeds). It cannot withdraw epoch pools,
  credited rewards or node deposits.

On devnet the authority and the program's upgrade authority are single keys.

**Known limitations**

- `initialize` makes its signer the protocol authority. Whoever deploys a fresh instance must
  initialize it before anyone else does.
- The gateway's dispatch signature is the only thing binding a ticket to an admitted round.
- A coalition that forms a round's whole signer set can re-sign that round to re-roll its ticket
  lottery or to exclude a co-signer.
- Subscription quotas are enforced off chain by the gateway.

**What a consumer must still check**

A valid signature shows that a quorum of registered nodes signed a value. It does not replace
the consumer's own policy. Check the feed's owner and discriminator, the expected `source_id`,
submitter and quorum, the freshness of `timestamp`, and how `value` is encoded for your source.
See the [integration guide](INTEGRATION.md).
