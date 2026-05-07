use anchor_lang::prelude::*;
use borsh::{BorshDeserialize, BorshSerialize};
use libsecp256k1::{PublicKey, PublicKeyFormat};
use solana_keccak_hasher::hashv;
use solana_secp256k1_recover::secp256k1_recover;

use crate::{
    error::VerifyError,
    state::{Feed, Node, ProtocolConfig},
    utils::{
        bitmap_is_subset, bitmap_popcount_evm, derive_group_bitmap, evm_bitmap_bit_set,
        evm_bitmap_set_bit, eth_address_from_uncompressed_pubkey, evm_schnorr_ecdsa_inputs,
        secp256k1_scalar_is_valid_nonzero,
        validate_bitmap_upper_bits_clear,
    },
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq)]
pub struct VerifyAnswerArgs {
    pub job_id: [u8; 32],
    pub value: i64,
    /// EVM-compatible `bytes32 value` (Solidity `DataUpdate.value`).
    /// This is used for signature verification preimage compatibility with `ref/src/Validator.sol`.
    pub value_bytes32: [u8; 32],
    pub timestamp: u64,
    pub round: u32,
    pub signers_bitmap: [u8; 32],
    /// Schnorr scalar `s` (EVM `SchnorrSignature.s`).
    pub s: [u8; 32],
    /// EVM Schnorr commitment address `address(R)`, derived as:
    /// `keccak256(R.x || R.y)[12..32]`.
    pub commitment_addr: [u8; 20],
    /// EVM-compatible `uint32 registryVersion` (Solidity `DataUpdate.registryVersion`).
    pub registry_version: u32,
    /// EVM-compatible `bytes32 configHash` (Solidity `DataUpdate.configHash`).
    pub config_hash: [u8; 32],
    pub max_staleness: u32,
}

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq)]
pub struct VerifyAnswerReturn {
    pub value: i64,
    pub timestamp: u64,
}

/// Solidity `bytes32(keccak256("MOLPHA_PULL_ROUND_V1"))`.
const EVM_ROUND_ID_PREFIX: [u8; 32] = [
    0x05, 0xcc, 0x9a, 0xbf, 0xa8, 0x3c, 0xdc, 0x2d, 0xff, 0xa2, 0x8a, 0xe0, 0xd4, 0x68,
    0x59, 0x5e, 0xce, 0xd0, 0x56, 0x4a, 0x16, 0xf6, 0xed, 0x9a, 0x3e, 0x2c, 0x5b, 0x4e,
    0x65, 0x2d, 0x98, 0x26,
];

pub fn verify_answer(ctx: Context<VerifyAnswer>, params: VerifyAnswerArgs) -> Result<()> {
    let protocol_config = &ctx.accounts.protocol_config;
    let feed = &ctx.accounts.feed;

    // Staleness is enforced in `publish_answer`. `verify_answer` is a pure
    // cross-chain verifier and intentionally does not gate on local clock.

    require!(params.s != [0u8; 32], VerifyError::InvalidSignature);
    require!(
        secp256k1_scalar_is_valid_nonzero(&params.s),
        VerifyError::InvalidSignature
    );

    validate_bitmap_upper_bits_clear(&params.signers_bitmap, protocol_config.node_count)?;

    let signatures_required = feed.min_signatures_threshold;

    let signer_count = bitmap_popcount_evm(&params.signers_bitmap);
    require!(
        signer_count >= signatures_required as u32,
        VerifyError::InsufficientSigners
    );

    let group_size = (signatures_required as u32)
        .checked_add(protocol_config.redundancy_buffer)
        .ok_or(VerifyError::GroupBitmapDerivationFailed)?;

    let group_bm = derive_group_bitmap(
        &feed.seed,
        params.round,
        protocol_config.node_count,
        group_size,
    )?;

    require!(
        bitmap_is_subset(&params.signers_bitmap, &group_bm),
        VerifyError::SignerNotInGroup
    );

    require!(
        ctx.remaining_accounts.len() == signer_count as usize,
        VerifyError::MissingSignerAccount
    );

    // Signer pubkeys are combined in ascending bitmap bit order (EVM `Validator.verify`).
    // Collect per-(node_index) keys, then sort — small `k` (signature threshold), cheap vs 256-slot map.
    let mut signer_entries: Vec<(u32, PublicKey)> = Vec::with_capacity(signer_count as usize);
    let mut covered = [0u8; 32];

    for account_info in ctx.remaining_accounts.iter() {
        require!(
            account_info.owner == ctx.program_id,
            VerifyError::MissingSignerAccount
        );

        let data = account_info.try_borrow_data()?;
        let node = Node::try_deserialize(&mut &data[..])
            .map_err(|_| error!(VerifyError::MissingSignerAccount))?;

        // Prefer stored bump (one SHA256 via syscall); only hit the expensive
        // canonical derivation when the bump is missing or wrong (legacy data).
        let bump_seed = [node.bump];
        match Pubkey::create_program_address(
            &[
                Node::SEED_PREFIX,
                node.node_pubkey.as_ref(),
                bump_seed.as_slice(),
            ],
            ctx.program_id,
        ) {
            Ok(pda) if pda == account_info.key() => {}
            _ => {
                let (canonical, _) = Pubkey::find_program_address(
                    &[Node::SEED_PREFIX, node.node_pubkey.as_ref()],
                    ctx.program_id,
                );
                require_keys_eq!(canonical, account_info.key());
            }
        }

        require!(node.node_index >= 1, VerifyError::MissingSignerAccount);
        let pos = node.node_index.saturating_sub(1) as usize;
        require!(pos < 256, VerifyError::InvalidSignersBitmap);

        require!(
            evm_bitmap_bit_set(&params.signers_bitmap, pos),
            VerifyError::SignerNotInGroup
        );
        require!(
            !evm_bitmap_bit_set(&covered, pos),
            VerifyError::MissingSignerAccount
        );
        evm_bitmap_set_bit(&mut covered, pos);

        require!(
            node.node_pub_evm_x != [0u8; 32] && node.node_pub_evm_y != [0u8; 32],
            VerifyError::InvalidSignature
        );
        let mut uncompressed = [0u8; 65];
        uncompressed[0] = 0x04;
        uncompressed[1..33].copy_from_slice(&node.node_pub_evm_x);
        uncompressed[33..65].copy_from_slice(&node.node_pub_evm_y);
        let pk = PublicKey::parse_slice(&uncompressed, Some(PublicKeyFormat::Full))
            .map_err(|_| error!(VerifyError::InvalidSignature))?;
        signer_entries.push((node.node_index, pk));
    }

    require!(
        covered == params.signers_bitmap,
        VerifyError::MissingSignerAccount
    );

    signer_entries.sort_unstable_by_key(|e| e.0);

    let coalition_pks: Vec<PublicKey> = signer_entries.iter().map(|(_, pk)| *pk).collect();
    let coalition_key =
        PublicKey::combine(&coalition_pks).map_err(|_| error!(VerifyError::InvalidSignature))?;
    let x_coalition = coalition_key.serialize_compressed();

    let timestamp_bytes = params.timestamp.to_be_bytes();
    let registry_version_bytes = params.registry_version.to_be_bytes();
    // EVM `DataUpdate.signaturesRequired` is `uint32` and is carried on-feed as `min_signatures_threshold`.
    let signatures_required_u32 = u32::from(signatures_required);
    let signatures_required_bytes = signatures_required_u32.to_be_bytes();

    // EVM `Validator._constructMessage` compatibility:
    // keccak256(abi.encodePacked(
    //   ROUND_ID_PREFIX, jobId, registryVersion, signaturesRequired, configHash,
    //   signersBitmap, value(bytes32), canonicalTimestamp(uint64)
    // ))
    let message_hash = hashv(&[
        EVM_ROUND_ID_PREFIX.as_slice(),
        params.job_id.as_slice(),
        registry_version_bytes.as_slice(),
        signatures_required_bytes.as_slice(),
        params.config_hash.as_slice(),
        params.signers_bitmap.as_slice(),
        params.value_bytes32.as_slice(),
        timestamp_bytes.as_slice(),
    ])
    .to_bytes();

    // --- EVM-compatible Schnorr verification via `ecrecover` syscall ---
    // EVM derives `commitment` as `address(R)` and verifies via the ECDSA
    // recovery trick (see `ref/src/libs/LibSchnorr.sol`). We accept the
    // derived address directly to avoid on-chain R decompression.
    let commitment_addr = params.commitment_addr;

    let (recovery_id, ecdsa_signature, ecdsa_hash) = evm_schnorr_ecdsa_inputs(
        &x_coalition,
        &message_hash,
        &params.s,
        &commitment_addr,
    )
    .map_err(|_| error!(VerifyError::InvalidSignature))?;

    let recovered = secp256k1_recover(&ecdsa_hash, recovery_id, &ecdsa_signature)
        .map_err(|_| error!(VerifyError::InvalidSignature))?;
    let recovered_addr = eth_address_from_uncompressed_pubkey(recovered.to_bytes());

    require!(recovered_addr == commitment_addr, VerifyError::InvalidSignature);

    let return_data = VerifyAnswerReturn {
        value: params.value,
        timestamp: params.timestamp,
    };
    anchor_lang::solana_program::program::set_return_data(&return_data.try_to_vec()?);

    Ok(())
}

#[derive(Accounts)]
#[instruction(params: VerifyAnswerArgs)]
pub struct VerifyAnswer<'info> {
    #[account(
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump = protocol_config.bump
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    #[account(
        seeds = [
            Feed::SEED_PREFIX,
            feed.authority.as_ref(),
            feed.name_hash.as_ref(),
            feed.feed_type.to_seed().as_ref(),
        ],
        bump = feed.bump,
        constraint = feed.job_id == params.job_id @ VerifyError::InvalidFeedJobId
    )]
    pub feed: Account<'info, Feed>,
}
