# Solana program API reference

This reference covers the checked-in program and CLI. The generated IDL from
`anchor build -p molpha` is the machine-readable instruction/account interface.
Its configured address is `chivcFQgxzwkpLvW41PV431HQ4dYpaW3povQH3AdpQt`;
this is source configuration, not a certification of a deployed instance.

## Instructions

| Area | Entrypoints in `programs/molpha/src/lib.rs` |
| --- | --- |
| Configuration | `initialize`, `update_config`, `update_registry_policy` |
| Nodes | `register_node`, `deactivate_node`, `update_node_endpoint`, `reinstate_node` |
| Plans and subscriptions | `add_plan`, `update_plan`, `subscribe`, `extend_subscription` |
| Delegates | `add_delegate`, `remove_delegate` |
| Gateways | `register_gateway`, `deactivate_gateway` |
| Publication | `submit_attestation` |
| Epochs | `submit_ticket`, `close_ticket_marker`, `finalize_epoch`, `credit_epoch_rewards`, `close_epoch` |
| Treasury and withdrawals | `sweep_treasury_revenue`, `withdraw_protocol_reserve`, `claim_node_rewards`, `withdraw_node_deposit` |
| Evidence | `slash_offence` |

There is no `verify_attestation` entrypoint in this program. Signature primitives
are supplied by the separately versioned `molpha-verifier` dependency. This
repository does not ship a `MolphaOracleClient`, price conversion API or Pyth adapter.

## Feed accounts

`programs/molpha/src/state/feed.rs` defines the Anchor `Feed` account. The PDA
seeds are `molpha_feed`, the 32-byte source ID, one-byte quorum, and submitter
public key, under the configured program ID.

| Field | Meaning |
| --- | --- |
| `source_id` | Application source identity, 32 bytes |
| `value` | Signed 32-byte value or Keccak-256 hash |
| `value_kind` | `Value` or `Hash`; hashed preimages are supplied in the submission instruction |
| `submitter` | Submitter public key; part of the PDA identity |
| `timestamp` | Unix milliseconds, strictly increasing on updates |
| `signatures_required` | Quorum, `u8` |
| `signers_bitmap` | 32-byte signer bitmap |
| `registry_version` | `u32` snapshot version |
| `bump` | Canonical PDA bump |

Consumers must check owner/discriminator, expected source, submitter, quorum,
freshness and value encoding. The program does not assign price decimals or a
confidence interval to the 32-byte value. Epochs and chain clocks use seconds;
convert a feed timestamp from milliseconds before comparing it with those clocks.

For instruction arguments, account constraints and remaining-account ordering,
use the matching IDL and the corresponding instruction source. See
[`cli/README.md`](../cli/README.md) for CLI command groups and
[`molpha-tests/README.md`](../molpha-tests/README.md) for executable examples.
