use anchor_lang::prelude::*;
use solana_keccak_hasher::hashv;

use crate::error::MolphaError;

/// Records that `I(S)` was derived from `S`, so the committee of an `I(S)` attestation is drawn under
/// `S`.
///
/// A source with a policy (node-client `docs/policy.md`) signs an INCONCLUSIVE result under the derived
/// ID `I(S) = keccak256("MOLPHA_INCONCLUSIVE_V1" || S)`, in the same round, by the committee selected
/// for `S`. The program draws a round's committee from the signed `source_id` in `submit_ticket` and in
/// the `UnsolicitedSigning` offence, and `I(S)` cannot be reversed to `S`. This account carries `S` at
/// the address derived from `I(S)`: both instructions take the account at that address, always, so an
/// accuser can neither omit nor replace it. Forging a link for an ordinary source would need a keccak
/// preimage, so a link cannot shield anyone from slashing.
///
/// It is a selection hint, not an identity registry: `S` and `I(S)` stay pure derivations, and anyone
/// may create the link (`register_inconclusive_id`).
#[account]
#[derive(InitSpace)]
pub struct InconclusiveLink {
    /// `S`: the source whose INCONCLUSIVE results are signed under this account's `I(S)`.
    pub source_id: [u8; 32],
    pub bump: u8,
}

impl InconclusiveLink {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_inconclusive";
    /// The raw ASCII domain tag of `I(S)`, followed by the 32 bytes of `S`.
    pub const TAG: &'static [u8] = b"MOLPHA_INCONCLUSIVE_V1";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;
}

/// `I(S) = keccak256("MOLPHA_INCONCLUSIVE_V1" || S)`.
pub fn inconclusive_id(source_id: &[u8; 32]) -> [u8; 32] {
    hashv(&[InconclusiveLink::TAG, source_id]).to_bytes()
}

/// The source ID an attestation's committee is drawn under: `S` when `link` (the account at the
/// `InconclusiveLink` address derived from `attested`) holds a link, `attested` otherwise.
///
/// The caller has checked `link`'s address. An account there that the program owns must be a link
/// for `attested`; anything else at the address (nothing, or a system account someone funded) is no
/// link.
pub fn selection_source_id(link: &AccountInfo, attested: &[u8; 32]) -> Result<[u8; 32]> {
    if link.owner != &crate::ID || link.data_is_empty() {
        return Ok(*attested);
    }
    let data = link.try_borrow_data()?;
    let record = InconclusiveLink::try_deserialize(&mut &data[..])
        .map_err(|_| error!(MolphaError::InvalidInconclusiveLink))?;
    require!(
        inconclusive_id(&record.source_id) == *attested,
        MolphaError::InvalidInconclusiveLink
    );
    Ok(record.source_id)
}
