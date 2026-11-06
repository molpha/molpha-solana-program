use anchor_lang::prelude::*;

/// A provider signer: a data provider whose secp256k1 key co-signs the node committee's aggregate
/// at bit `64 + slot` of the signers bitmap, proving the provider signed that exact attestation.
/// Its bit counts toward `signatures_required` like a node's (at most one provider bit per
/// attestation), so a provider round is `signatures_required - 1` nodes plus the provider.
///
/// Admitted like a node: `register_node` with the `provider` account, co-signed by the protocol
/// authority, puts it in the registry at once. It leaves through `deactivate_node` and rotates
/// its key through `rotate_node_key`. It locks no deposit: nothing slashes a provider.
///
/// PDA `["molpha_provider", id.to_le_bytes()]`, which is also the `account` of the provider's
/// record in the registry's provider table (provider slot `slot`). `id` is stable: assigned from
/// `RegistryState.next_provider_id`, never reused, so consumers bind a source to an id rather than
/// to a slot, and a slot handed to another provider later transfers no trust.
///
/// Byte layout: `docs/provider-accounts.md`.
#[account]
#[derive(InitSpace)]
pub struct Provider {
    pub id: u32,
    /// Registers the provider (with the protocol authority), ejects it, rotates its keys. Never
    /// signs rounds.
    pub authority: Pubkey,
    /// The signing key, affine and big-endian. It is the registry record's key.
    pub x: [u8; 32],
    pub y: [u8; 32],
    /// ed25519 libp2p identity the gateway dials. Not the signing key.
    pub p2p_key: [u8; 32],
    /// Provider slot `p` (bit `64 + p`). A `Removed` provider keeps the slot it last held, for
    /// reference only.
    pub slot: u8,
    pub status: ProviderStatus,
    pub bump: u8,
    /// When the signing key was last rotated; `0` if it never was.
    pub key_rotated_at: i64,
}

#[derive(
    AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, InitSpace, Default, Debug,
)]
pub enum ProviderStatus {
    /// In the registry at provider slot `slot` (bit `64 + slot`).
    #[default]
    Active,
    /// Ejected by its authority. Terminal: the id is not reused.
    Removed,
}

impl Provider {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_provider";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;

    pub fn init(
        &mut self,
        id: u32,
        authority: Pubkey,
        (x, y): ([u8; 32], [u8; 32]),
        p2p_key: [u8; 32],
        slot: u8,
        bump: u8,
    ) {
        self.id = id;
        self.authority = authority;
        self.x = x;
        self.y = y;
        self.p2p_key = p2p_key;
        self.slot = slot;
        self.status = ProviderStatus::Active;
        self.bump = bump;
        self.key_rotated_at = 0;
    }

    /// Switch to a new signing key. The caller publishes the registry version that names it.
    pub fn set_key(&mut self, (x, y): ([u8; 32], [u8; 32]), rotated_at: i64) {
        self.x = x;
        self.y = y;
        self.key_rotated_at = rotated_at;
    }

    pub fn xy(&self) -> ([u8; 32], [u8; 32]) {
        (self.x, self.y)
    }

    pub fn find_address(id: u32) -> (Pubkey, u8) {
        Pubkey::find_program_address(&[Self::SEED_PREFIX, id.to_le_bytes().as_ref()], &crate::ID)
    }
}

const _: () = assert!(Provider::SPACE == 151);
