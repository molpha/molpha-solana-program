//! Error type for DataUpdate verification.
//!
//! Pure verification errors are mapped by downstream programs at the call boundary. The
//! on-chain resolver additionally preserves Anchor account-borrow/deserialization errors so
//! their runtime codes are not changed by crossing the crate boundary.

#[derive(Debug, PartialEq, Eq)]
pub enum DataUpdateError {
    /// Aggregate Schnorr signature failed verification (recovered address mismatch,
    /// invalid coalition key, or invalid scalar `s`).
    InvalidAggregateSignature,
    /// A signature component was malformed (e.g. non-canonical scalar during the
    /// Schnorr→ECDSA conversion).
    InvalidSignature,
    /// `popcount(signers_bitmap) < signatures_required`.
    InsufficientSigners,
    /// `signers_bitmap` is not a subset of the deterministically derived selection bitmap.
    SignersNotSubsetOfSelection,
    /// `signers_bitmap` has bits set outside `[0, node_count)`, or is otherwise invalid.
    InvalidSignersBitmap,
    /// Selection-group bitmap derivation failed (bad node count / group size, or the
    /// bounded sampling loop did not converge).
    GroupBitmapDerivationFailed,
    /// `ordered_signers.len()` does not equal `popcount(signers_bitmap)`.
    SignerCountMismatch,
    /// The requested registry version is neither current nor a live previous version.
    InvalidRegistryVersion,
    /// A signer account is missing, extra, or owned by another program.
    MissingSignerAccount,
    /// A `RegistryIndex` account does not match its expected bitmap index.
    InvalidNodeIndex,
    /// Previous-version remove remapping was requested without valid transition metadata.
    InvalidTransitionAccount,
    /// Preserve an Anchor account error (for example a discriminator mismatch) unchanged.
    AnchorError(anchor_lang::error::Error),
}

impl From<anchor_lang::error::Error> for DataUpdateError {
    fn from(error: anchor_lang::error::Error) -> Self {
        Self::AnchorError(error)
    }
}
