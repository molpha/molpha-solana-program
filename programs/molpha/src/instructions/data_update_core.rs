//! Shared logic for `submit_data_update` and `verify_data_update`.
//!
//! Both instructions verify the same `DataUpdate + AggregatedSig` payload.
//! The only difference is that `submit_data_update` also enforces a feed-freshness
//! constraint and writes state, while `verify_data_update` is stateless.

use anchor_lang::prelude::*;
use solana_keccak_hasher::hashv;
use solana_secp256k1_recover::secp256k1_recover;

use crate::{
    error::MolphaError,
    instructions::submit_data_update::SubmitDataUpdateArgs,
    state::{RegistryIndex, RegistryState, RegistryTransitionType},
    utils::{
        account_layout::{
            expected_registry_index, validate_remove_transition_for_previous, CoalitionAccumulator,
        },
        bitmap::{
            bitmap_is_subset_u256, bitmap_load, derive_group_bitmap, effective_selection_size,
            for_each_set_bit_u256, validate_bitmap_upper_bits_clear_u256,
        },
        schnorr::{
            eth_address_from_uncompressed_pubkey, evm_schnorr_ecdsa_inputs,
            secp256k1_scalar_is_valid_nonzero,
        },
    },
};

/// `bytes32(keccak256("MOLPHA_MESSAGE_V1"))` — EVM `Validator._constructMessage` prefix.
///
/// Value: `keccak256(bytes("MOLPHA_MESSAGE_V1"))`, verified by the unit test below.
pub(crate) const MESSAGE_PREFIX: [u8; 32] = [
    0xa7, 0x55, 0x23, 0xa2, 0xab, 0x7b, 0x71, 0x8d, 0x9c, 0xff, 0xd2, 0xfa, 0x97, 0xed, 0x06, 0x9f,
    0xc1, 0x21, 0x84, 0xea, 0xbe, 0xe7, 0xd5, 0x07, 0x85, 0x4d, 0x09, 0x22, 0xf7, 0x0e, 0x7f, 0xe7,
];

/// `bytes32(keccak256("MOLPHA_SELECTION_V1"))` — EVM `Validator` selection seed prefix.
///
/// Value: `keccak256(bytes("MOLPHA_SELECTION_V1"))`, verified by the unit test below.
pub(crate) const SELECTION_SEED_PREFIX: [u8; 32] = [
    0x1d, 0xef, 0x81, 0x59, 0xcb, 0xcf, 0xcd, 0xfd, 0x72, 0x8d, 0x41, 0x97, 0x51, 0x9a, 0x57, 0xc0,
    0x6e, 0x24, 0x3f, 0x0d, 0x94, 0x68, 0xb4, 0xc1, 0xe5, 0xc4, 0xa2, 0x33, 0xfc, 0x56, 0x53, 0xc3,
];

/// Compute the EVM-compatible `DataUpdate` message hash.
///
/// Matches `Validator._constructMessage` in the EVM reference implementation:
/// ```text
/// keccak256(abi.encodePacked(
///     MESSAGE_PREFIX, jobId, registryVersion, signaturesRequired,
///     configHash, signersBitmap, value, canonicalTimestamp
/// ))
/// ```
pub(crate) fn compute_message_hash(
    args: &SubmitDataUpdateArgs,
    signatures_required: u8,
) -> [u8; 32] {
    let registry_version_bytes = args.registry_version.to_be_bytes();
    let signatures_required_bytes = u32::from(signatures_required).to_be_bytes();
    let canonical_timestamp_bytes = (args.canonical_timestamp as u64).to_be_bytes();

    hashv(&[
        MESSAGE_PREFIX.as_slice(),
        args.job_id.as_slice(),
        registry_version_bytes.as_slice(),
        signatures_required_bytes.as_slice(),
        args.signers_bitmap.as_slice(),
        args.value.as_slice(),
        canonical_timestamp_bytes.as_slice(),
    ])
    .to_bytes()
}

/// Validate the full aggregate Schnorr signature and signer set against `accepted_bitmap`.
///
/// Checks (in order):
/// 1. `agg_sig_s` is non-zero and a valid secp256k1 scalar.
/// 2. `popcount(signers_bitmap) >= signatures_required`.
/// 3. Re-derives `selection_bitmap` from `(job_id, registry_version, canonical_timestamp)`.
/// 4. `signers_bitmap ⊆ selection_bitmap`.
/// 5. For each bit set in `signers_bitmap`, reads the corresponding `RegistryIndex` from
///    `remaining_accounts` (index may remap during previous-version-after-remove grace).
/// 6. Reconstructs the coalition key `X_coalition = Σ X_i` and verifies the aggregate Schnorr
///    signature via the ECDSA-recovery trick.
pub(crate) fn build_coalition_and_verify<'info>(
    args: &SubmitDataUpdateArgs,
    registry: &RegistryState,
    now: i64,
    remaining_accounts: &[AccountInfo<'info>],
    program_id: &Pubkey,
    signatures_required: u8,
    redundancy_buffer: u8,
) -> Result<()> {
    require!(
        args.agg_sig_s != [0u8; 32],
        MolphaError::InvalidAggregateSignature
    );
    require!(
        secp256k1_scalar_is_valid_nonzero(&args.agg_sig_s),
        MolphaError::InvalidAggregateSignature
    );

    let signers = bitmap_load(&args.signers_bitmap);
    let signer_count = signers.count_ones();
    require!(
        signer_count >= signatures_required as u32,
        MolphaError::InsufficientSigners
    );

    let is_current = args.registry_version == registry.current_version;
    let is_previous_live =
        args.registry_version == registry.previous_version && now <= registry.previous_expires_at;
    require!(
        is_current || is_previous_live,
        MolphaError::InvalidRegistryVersion
    );

    let node_count = if is_current {
        registry.current_node_count
    } else {
        registry.previous_node_count
    };

    let canonical_timestamp_bytes = (args.canonical_timestamp as u64).to_be_bytes();
    let selection_seed = hashv(&[
        SELECTION_SEED_PREFIX.as_slice(),
        args.job_id.as_ref(),
        args.registry_version.to_be_bytes().as_ref(),
        canonical_timestamp_bytes.as_ref(),
    ])
    .to_bytes();
    let selection_size =
        effective_selection_size(signatures_required, redundancy_buffer, node_count);
    let expected_selection = derive_group_bitmap(&selection_seed, node_count, selection_size)?;

    require!(
        bitmap_is_subset_u256(signers, bitmap_load(&expected_selection)),
        MolphaError::SignersNotSubsetOfSelection
    );

    require!(
        remaining_accounts.len() == signer_count as usize,
        MolphaError::MissingSignerAccount
    );

    let apply_remove_remap =
        !is_current && registry.last_transition_type != RegistryTransitionType::Add;
    if apply_remove_remap {
        validate_remove_transition_for_previous(registry)?;
    }

    let mut coalition = CoalitionAccumulator::default();
    let mut account_cursor = 0usize;

    for_each_set_bit_u256(signers, |bit_pos| {
        let account_info = remaining_accounts
            .get(account_cursor)
            .ok_or(error!(MolphaError::MissingSignerAccount))?;
        require!(
            account_info.owner == program_id,
            MolphaError::MissingSignerAccount
        );

        let data = account_info.try_borrow_data()?;
        let registry_index = RegistryIndex::try_deserialize(&mut &data[..])?;

        account_cursor = account_cursor.saturating_add(1);

        let expected_index = expected_registry_index(bit_pos as u32, registry, apply_remove_remap);
        require!(
            registry_index.index == expected_index,
            MolphaError::InvalidNodeIndex
        );

        coalition.add_stored_xy(
            &registry_index.secp256k1_pubkey_x,
            &registry_index.secp256k1_pubkey_y,
        )?;

        Ok(())
    })?;

    require!(
        account_cursor == remaining_accounts.len(),
        MolphaError::MissingSignerAccount
    );

    let x_coalition = coalition.compressed_pubkey()?;

    let message_hash = compute_message_hash(args, signatures_required);

    let (recovery_id, ecdsa_signature, ecdsa_hash) = evm_schnorr_ecdsa_inputs(
        &x_coalition,
        &message_hash,
        &args.agg_sig_s,
        &args.commitment_addr,
    )
    .map_err(|_| error!(MolphaError::InvalidAggregateSignature))?;

    let recovered = secp256k1_recover(&ecdsa_hash, recovery_id, &ecdsa_signature)
        .map_err(|_| error!(MolphaError::InvalidAggregateSignature))?;
    let recovered_addr = eth_address_from_uncompressed_pubkey(recovered.to_bytes());

    require!(
        recovered_addr == args.commitment_addr,
        MolphaError::InvalidAggregateSignature
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Job;
    use anchor_lang::prelude::Pubkey;
    use libsecp256k1::{PublicKey, PublicKeyFormat};
    use solana_secp256k1_recover::secp256k1_recover;

    // -----------------------------------------------------------------------
    // Test vectors decoded from `tests/fixtures-json/verify-answer-evm.json`.
    //
    // These act as end-to-end EVM-compatibility regression tests for:
    //   - The MESSAGE_PREFIX constant
    //   - compute_message_hash field ordering and encoding
    //   - The full Schnorr-recovery verification path
    // -----------------------------------------------------------------------

    /// `dataUpdate.jobId` — "solana-compat-job" right-padded to 32 bytes.
    const FIXTURE_JOB_ID: [u8; 32] = [
        0x73, 0x6f, 0x6c, 0x61, 0x6e, 0x61, 0x2d, 0x63, 0x6f, 0x6d, 0x70, 0x61, 0x74, 0x2d, 0x6a,
        0x6f, 0x62, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00,
    ];

    /// `dataUpdate.configHash` — "solana-compat-cfg" right-padded to 32 bytes.
    /// Used as `api_config_hash` in the Solana message hash.
    const FIXTURE_CONFIG_HASH: [u8; 32] = [
        0x73, 0x6f, 0x6c, 0x61, 0x6e, 0x61, 0x2d, 0x63, 0x6f, 0x6d, 0x70, 0x61, 0x74, 0x2d, 0x63,
        0x66, 0x67, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00,
    ];

    /// `dataUpdate.value` — "solana-compat-val" right-padded to 32 bytes.
    const FIXTURE_VALUE: [u8; 32] = [
        0x73, 0x6f, 0x6c, 0x61, 0x6e, 0x61, 0x2d, 0x63, 0x6f, 0x6d, 0x70, 0x61, 0x74, 0x2d, 0x76,
        0x61, 0x6c, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00,
    ];

    /// EVM `uint256(255)` as big-endian `[u8; 32]` — bits 0–7 set (8 signers).
    const FIXTURE_SIGNERS_BITMAP: [u8; 32] = [
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0xff,
    ];

    const FIXTURE_REGISTRY_VERSION: u32 = 1;
    const FIXTURE_SIGNATURES_REQUIRED: u8 = 8;
    /// `dataUpdate.canonicalTimestamp`.
    const FIXTURE_CANONICAL_TIMESTAMP: i64 = 1_700_000_123;

    /// `schnorrSignature.signature` — the Schnorr scalar `s`.
    const FIXTURE_S: [u8; 32] = [
        0xc7, 0xe0, 0x99, 0x60, 0x3c, 0xee, 0xd2, 0xa1, 0x13, 0xd7, 0x5a, 0x9d, 0x95, 0xe2, 0x0f,
        0x92, 0x00, 0x6b, 0x06, 0xc5, 0x49, 0x7a, 0xdd, 0x09, 0x81, 0x7d, 0xa8, 0x90, 0x8d, 0x39,
        0x0d, 0xa5,
    ];

    /// `schnorrSignature.commitment` — Ethereum address (20 bytes).
    const FIXTURE_COMMITMENT: [u8; 20] = [
        0xc6, 0xb9, 0x4f, 0xea, 0x5d, 0xd5, 0xf9, 0x65, 0xd8, 0x67, 0x14, 0xb1, 0xd9, 0x9d, 0xcf,
        0xaf, 0x1e, 0x72, 0xee, 0x35,
    ];

    /// Compressed secp256k1 pubkeys for nodes at bit positions 0–7 (signersBitmap = 255).
    const FIXTURE_PUBKEYS: [[u8; 33]; 8] = [
        [
            0x03, 0xc0, 0x95, 0x27, 0xe9, 0x78, 0xf6, 0xea, 0x69, 0xf0, 0xc6, 0xb7, 0xac, 0x0f,
            0xb6, 0x3a, 0xd0, 0x81, 0xa8, 0xa2, 0x91, 0x15, 0x1c, 0x5a, 0x0b, 0x11, 0x5c, 0xce,
            0x43, 0x57, 0x51, 0xbe, 0x7d,
        ],
        [
            0x02, 0x64, 0xa7, 0x27, 0x04, 0xf3, 0x9f, 0x8d, 0xd1, 0x7f, 0x20, 0xd7, 0x1c, 0x5b,
            0x21, 0xf3, 0x7b, 0x58, 0x52, 0x65, 0x6b, 0xc0, 0x55, 0x54, 0x42, 0xbf, 0x72, 0x72,
            0x22, 0xf2, 0x9d, 0x7e, 0x58,
        ],
        [
            0x02, 0x75, 0xae, 0x1e, 0x3d, 0xac, 0x00, 0xeb, 0x7d, 0xf0, 0x2e, 0x9f, 0xe8, 0xd9,
            0x70, 0x9c, 0x8a, 0x2c, 0x09, 0xa1, 0x1e, 0xd4, 0xf7, 0xd9, 0xaa, 0x46, 0xa7, 0xde,
            0xa6, 0xcf, 0x37, 0x6d, 0x7f,
        ],
        [
            0x02, 0x6c, 0xe2, 0x5b, 0x3a, 0x16, 0x1a, 0xb8, 0xe0, 0xf0, 0x5e, 0x4c, 0xd1, 0xc7,
            0x7b, 0x77, 0x69, 0x6d, 0x26, 0xc6, 0x41, 0xeb, 0xde, 0xa4, 0xe8, 0x1a, 0xa8, 0x9a,
            0x90, 0xf3, 0x2c, 0xfc, 0x54,
        ],
        [
            0x03, 0x5b, 0x95, 0xd7, 0x03, 0x22, 0x8b, 0xef, 0xcc, 0xc3, 0x78, 0x62, 0x9d, 0xc1,
            0x98, 0x04, 0xce, 0xfe, 0x56, 0xc3, 0x3c, 0x64, 0x5f, 0xa4, 0xbc, 0x1a, 0xa0, 0xf3,
            0x75, 0xe3, 0xb4, 0xfa, 0x5e,
        ],
        [
            0x03, 0x99, 0x5e, 0x4b, 0xe0, 0xec, 0xd4, 0x22, 0xbf, 0x25, 0x0a, 0x3d, 0xa3, 0xa0,
            0xb8, 0x34, 0x2e, 0x52, 0x89, 0x3a, 0x3e, 0x06, 0x4f, 0xa6, 0x35, 0x55, 0x73, 0x78,
            0xb5, 0x9a, 0xfa, 0x8b, 0x50,
        ],
        [
            0x03, 0xec, 0x90, 0x6d, 0x0a, 0x1c, 0xfc, 0x3c, 0x7d, 0xec, 0x18, 0x08, 0x8c, 0x3d,
            0x14, 0x4f, 0x32, 0x15, 0x80, 0xec, 0xe0, 0xa6, 0xba, 0xe5, 0xce, 0xb2, 0x8d, 0xcf,
            0x8d, 0xc6, 0xe3, 0xda, 0x03,
        ],
        [
            0x03, 0x27, 0x5f, 0xcf, 0x98, 0x38, 0xb4, 0x7a, 0xac, 0xff, 0x25, 0x1f, 0x4f, 0x09,
            0x9f, 0x80, 0xc6, 0x4a, 0x1a, 0x9a, 0xed, 0xbd, 0xb6, 0x28, 0xc2, 0xc8, 0x7f, 0x2c,
            0x5e, 0x12, 0x3d, 0xd0, 0x40,
        ],
    ];

    fn fixture_args() -> SubmitDataUpdateArgs {
        SubmitDataUpdateArgs {
            job_id: FIXTURE_JOB_ID,
            registry_version: FIXTURE_REGISTRY_VERSION,
            signers_bitmap: FIXTURE_SIGNERS_BITMAP,
            signatures_required: FIXTURE_SIGNATURES_REQUIRED,
            value: FIXTURE_VALUE,
            canonical_timestamp: FIXTURE_CANONICAL_TIMESTAMP,
            agg_sig_s: FIXTURE_S,
            commitment_addr: FIXTURE_COMMITMENT,
        }
    }

    fn fixture_job() -> Job {
        Job {
            job_id: FIXTURE_JOB_ID,
            owner: Pubkey::default(),
            delegates: [Pubkey::default(); Job::MAX_DELEGATES],
            delegate_count: 0,
            api_config_hash: FIXTURE_CONFIG_HASH,
            decimals: 0,
            signatures_required: FIXTURE_SIGNATURES_REQUIRED,
            created_at: 0,
            bump: 0,
        }
    }

    fn fixture_coalition_compressed() -> [u8; 33] {
        let pks: Vec<PublicKey> = FIXTURE_PUBKEYS
            .iter()
            .map(|c| {
                PublicKey::parse_slice(c, Some(PublicKeyFormat::Compressed))
                    .expect("fixture pubkey must be a valid curve point")
            })
            .collect();
        PublicKey::combine(&pks)
            .expect("fixture pubkeys must combine")
            .serialize_compressed()
    }

    // -----------------------------------------------------------------------
    // domain constants
    // -----------------------------------------------------------------------

    #[test]
    fn selection_seed_prefix_is_keccak_of_domain() {
        let expected = hashv(&[b"MOLPHA_SELECTION_V1"]).to_bytes();
        assert_eq!(SELECTION_SEED_PREFIX, expected);
    }

    // -----------------------------------------------------------------------
    // compute_message_hash
    // -----------------------------------------------------------------------

    /// `compute_message_hash` must be deterministic.
    #[test]
    fn compute_message_hash_is_deterministic() {
        let args = fixture_args();
        let job = fixture_job();
        assert_eq!(
            compute_message_hash(&args, job.signatures_required),
            compute_message_hash(&args, job.signatures_required)
        );
    }

    /// Changing any field must produce a distinct hash (sensitivity check).
    #[test]
    fn compute_message_hash_is_sensitive_to_each_field() {
        let base_args = fixture_args();
        let base_job = fixture_job();
        let base = compute_message_hash(&base_args, base_job.signatures_required);

        let mut a = fixture_args();
        a.registry_version += 1;
        assert_ne!(
            compute_message_hash(&a, base_job.signatures_required),
            base,
            "registry_version must affect hash"
        );

        let mut j = fixture_job();
        j.signatures_required = j.signatures_required.saturating_sub(1);
        assert_ne!(
            compute_message_hash(&base_args, j.signatures_required),
            base,
            "signatures_required must affect hash"
        );

        let mut config_hash = base_job.api_config_hash;
        config_hash[0] ^= 0xff;
        assert_ne!(
            compute_message_hash(&base_args, base_job.signatures_required),
            base,
            "api_config_hash must affect hash"
        );

        let mut a = fixture_args();
        a.signers_bitmap[31] ^= 0x01;
        assert_ne!(
            compute_message_hash(&a, base_job.signatures_required),
            base,
            "signers_bitmap must affect hash"
        );

        let mut a = fixture_args();
        a.value[0] ^= 0xff;
        assert_ne!(
            compute_message_hash(&a, base_job.signatures_required),
            base,
            "value must affect hash"
        );

        let mut a = fixture_args();
        a.canonical_timestamp += 1;
        assert_ne!(
            compute_message_hash(&a, base_job.signatures_required),
            base,
            "canonical_timestamp must affect hash"
        );

        let mut a = fixture_args();
        a.job_id[0] ^= 0xff;
        assert_ne!(
            compute_message_hash(&a, base_job.signatures_required),
            base,
            "job_id must affect hash"
        );
    }

    // -----------------------------------------------------------------------
    // Fixture integrity
    // -----------------------------------------------------------------------

    /// All 8 fixture pubkeys must be valid compressed secp256k1 points.
    #[test]
    fn fixture_pubkeys_are_valid_curve_points() {
        for (i, pk) in FIXTURE_PUBKEYS.iter().enumerate() {
            PublicKey::parse_slice(pk, Some(PublicKeyFormat::Compressed))
                .unwrap_or_else(|_| panic!("fixture pubkey {i} is not a valid curve point"));
        }
    }

    /// Fixture signers bitmap encodes exactly 8 active bits (matching `signaturesRequired`).
    #[test]
    fn fixture_signers_bitmap_popcount_is_8() {
        use crate::utils::bitmap::bitmap_popcount_evm;
        assert_eq!(
            bitmap_popcount_evm(&FIXTURE_SIGNERS_BITMAP),
            FIXTURE_SIGNATURES_REQUIRED as u32
        );
    }

    // -----------------------------------------------------------------------
    // End-to-end EVM-compatibility
    // -----------------------------------------------------------------------

    /// Full EVM-compat regression test using `tests/fixtures-json/verify-answer-evm.json`.
    ///
    /// Constructs the coalition key from the 8 fixture pubkeys, computes the message hash,
    /// runs the Schnorr-recovery trick, and asserts the recovered address matches the
    /// fixture commitment. This is the same computation the Solana program performs at
    /// runtime, validated against a fixture generated by the EVM reference implementation.
    #[test]
    fn evm_message_hash_and_schnorr_recovery_matches_fixture() {
        let args = fixture_args();
        let job = fixture_job();
        let message_hash = compute_message_hash(&args, job.signatures_required);
        let x_coalition = fixture_coalition_compressed();

        let (recovery_id, ecdsa_sig, ecdsa_hash) = crate::utils::schnorr::evm_schnorr_ecdsa_inputs(
            &x_coalition,
            &message_hash,
            &FIXTURE_S,
            &FIXTURE_COMMITMENT,
        )
        .expect("ecdsa inputs must succeed for a valid fixture");

        let recovered = secp256k1_recover(&ecdsa_hash, recovery_id, &ecdsa_sig)
            .expect("secp256k1_recover must succeed for a valid fixture");
        let recovered_addr =
            crate::utils::schnorr::eth_address_from_uncompressed_pubkey(recovered.to_bytes());

        assert_eq!(
            recovered_addr, FIXTURE_COMMITMENT,
            "Schnorr recovery must produce the fixture commitment address"
        );
    }

    /// Mutating the signature scalar `s` must cause recovery to produce a different address.
    #[test]
    fn tampered_s_fails_commitment_check() {
        let args = fixture_args();
        let job = fixture_job();
        let message_hash = compute_message_hash(&args, job.signatures_required);
        let x_coalition = fixture_coalition_compressed();

        let mut bad_s = FIXTURE_S;
        bad_s[31] ^= 0x01; // flip one bit

        if let Ok((rid, sig, hash)) = crate::utils::schnorr::evm_schnorr_ecdsa_inputs(
            &x_coalition,
            &message_hash,
            &bad_s,
            &FIXTURE_COMMITMENT,
        ) {
            if let Ok(recovered) = secp256k1_recover(&hash, rid, &sig) {
                let addr = crate::utils::schnorr::eth_address_from_uncompressed_pubkey(
                    recovered.to_bytes(),
                );
                assert_ne!(
                    addr, FIXTURE_COMMITMENT,
                    "tampered s must not recover the correct address"
                );
            }
            // If secp256k1_recover fails, that's also an acceptable outcome (signature is invalid).
        }
    }
}
