//! A node signed two verifying partials against one nonce pair it had committed in a signed batch.
//!
//! The offence is defined at the equation level: two triples `(b, c, s)` differing in `(b, c)`
//! that each satisfy `s*G == R1 + b*R2 + c*X` against a single batch-committed `(R1, R2)`. The
//! program never recomputes `b` or `c` from a signing context, so commitment lists and message
//! transport stay off-chain entirely. Producing even one satisfying triple requires the node's
//! `(r1, r2, x)`, so a filer cannot fabricate a second triple from an observed one without solving
//! a discrete log — framing an honest node is DL-hard.
//!
//! Two transcripts prove the violation; a third would extract the share outright. The offence is
//! the reuse, not the extraction, so the penalty is a full slash and permanent ejection.
//!
//! Same-round value equivocation files here as well: the slot derives from the round, so signing
//! two values for one round is mechanically slot reuse.
//!
//! Accepted residual: a node that grinds `R1 = -(c*X)` against a challenge it predicts at
//! commitment time makes that one transcript unevaluable on-chain (the equation's right-hand side
//! lands on the point at infinity). It has to predict the challenge before committing, and the
//! filing needs two transcripts, so this buys a misbehaving node nothing beyond one wasted slot.

use anchor_lang::prelude::*;
use anchor_spl::token::{Token, TokenAccount};
use molpha_verifier::{bitmap::bitmap_set_bit, NonceEquationError};

use crate::{
    error::{map_attestation_err, MolphaError},
    events::NonceReuseProven,
    state::{
        commitment_batch_message_hash, commitment_leaf_hash, nonce_reuse_evidence_hash,
        resolve_registry_signers_indexed, verify_commitment_merkle_proof, CommittedNoncePair,
        OffenceKind, PartialTranscript, ProtocolConfig, PunishmentCase, Registry, RegistryState,
        MAX_COMMITMENT_PROOF_DEPTH, MAX_REGISTRY_NODES,
    },
    utils::slash::{slash_nodes, SlashAccounts, SlashInput},
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct PunishNonceReuseArgs {
    /// Registry version that `node_index` indexes into; also bound into the batch signature.
    pub registry_version: u32,
    pub node_index: u32,
    pub epoch: u64,
    /// Opaque batch header bytes, covered by the signature but not interpreted on-chain.
    pub batch_meta: [u8; 32],
    pub batch_root: [u8; 32],
    /// Single-key EVM-Schnorr signature over the batch message.
    pub batch_sig_s: [u8; 32],
    pub batch_commitment_addr: [u8; 20],
    pub slot: u64,
    pub nonces: CommittedNoncePair,
    pub merkle_proof: Vec<[u8; 32]>,
    pub transcript_a: PartialTranscript,
    pub transcript_b: PartialTranscript,
    /// Must match `nonce_reuse_evidence_hash(..)`; used as the `PunishmentCase` PDA seed.
    pub evidence_hash: [u8; 32],
}

pub fn punish_nonce_reuse<'info>(
    ctx: Context<'info, PunishNonceReuse<'info>>,
    args: PunishNonceReuseArgs,
) -> Result<()> {
    require!(
        args.merkle_proof.len() <= MAX_COMMITMENT_PROOF_DEPTH,
        MolphaError::MerkleProofTooDeep
    );

    // The equation fixes `s` given `(b, c)`, so differing shares alone prove nothing; the reuse is
    // two distinct challenge pairs consumed against one commitment.
    require!(
        (args.transcript_a.b, args.transcript_a.c) != (args.transcript_b.b, args.transcript_b.c),
        MolphaError::TranscriptContextsEqual
    );

    require!(
        nonce_reuse_evidence_hash(
            args.registry_version,
            args.node_index,
            args.epoch,
            &args.batch_meta,
            &args.batch_root,
            args.slot,
            &args.nonces,
            &args.transcript_a,
            &args.transcript_b,
        ) == args.evidence_hash,
        MolphaError::EvidenceHashMismatch
    );

    let node_index =
        usize::try_from(args.node_index).map_err(|_| error!(MolphaError::InvalidNodeIndex))?;
    require!(
        node_index < MAX_REGISTRY_NODES,
        MolphaError::InvalidNodeIndex
    );
    let mut guilty_bitmap = [0u8; 32];
    bitmap_set_bit(&mut guilty_bitmap, node_index);

    let resolved = {
        let evidence_registry = ctx.accounts.evidence_registry.load()?;
        resolve_registry_signers_indexed(
            ctx.remaining_accounts,
            ctx.program_id,
            &evidence_registry,
            &guilty_bitmap,
        )?
    };
    let (_, node_key) = *resolved
        .first()
        .ok_or(error!(MolphaError::MissingSignerAccount))?;

    let leaf = commitment_leaf_hash(args.slot, &args.nonces);
    require!(
        verify_commitment_merkle_proof(leaf, &args.merkle_proof, &args.batch_root),
        MolphaError::InvalidMerkleProof
    );

    let batch_message = commitment_batch_message_hash(
        ctx.program_id,
        args.registry_version,
        args.node_index,
        args.epoch,
        &args.batch_meta,
        &args.batch_root,
    );
    require!(
        molpha_verifier::verify_aggregate_over_hash(
            &[node_key],
            &args.batch_sig_s,
            &args.batch_commitment_addr,
            &batch_message,
        )
        .map_err(map_attestation_err)?,
        MolphaError::InvalidBatchSignature
    );

    let r1 = (args.nonces.r1_x, args.nonces.r1_y);
    let r2 = (args.nonces.r2_x, args.nonces.r2_y);
    for transcript in [&args.transcript_a, &args.transcript_b] {
        require!(
            molpha_verifier::verify_partial_signature_equation(
                &node_key,
                &r1,
                &r2,
                &transcript.b,
                &transcript.c,
                &transcript.s,
            )
            .map_err(map_nonce_equation_err)?,
            MolphaError::InvalidPartialSignature
        );
    }

    let reporter = ctx.accounts.reporter.key();
    let new_registry_version = ctx.accounts.registry_state.next_version;
    let grace_active_until = ctx.accounts.protocol_config.get_grace_active_until();
    let outcome = slash_nodes(
        SlashAccounts {
            registry_state: &mut ctx.accounts.registry_state,
            current_registry: &ctx.accounts.current_registry,
            new_registry: &ctx.accounts.new_registry,
            new_registry_bump: ctx.bumps.new_registry,
            protocol_config: &mut ctx.accounts.protocol_config,
            punishment_case: &mut ctx.accounts.punishment_case,
            punishment_case_bump: ctx.bumps.punishment_case,
            reporter,
            node_deposit_vault: &mut ctx.accounts.node_deposit_vault,
            treasury: &mut ctx.accounts.treasury,
            reporter_usdc: &ctx.accounts.reporter_usdc,
            token_program: ctx.accounts.token_program.key(),
        },
        SlashInput {
            signer_accounts: ctx.remaining_accounts,
            program_id: ctx.program_id,
            resolved: &resolved,
            guilty_bitmap,
            guilty_count: 1,
            offence: OffenceKind::NonceReuse,
            registry_version: args.registry_version,
            evidence_hash: args.evidence_hash,
            new_registry_version,
            grace_active_until,
        },
    )?;

    emit!(NonceReuseProven {
        registry_version: args.registry_version,
        node_index: args.node_index,
        epoch: args.epoch,
        batch_meta: args.batch_meta,
        batch_root: args.batch_root,
        slot: args.slot,
        r1_x: args.nonces.r1_x,
        r1_y: args.nonces.r1_y,
        r2_x: args.nonces.r2_x,
        r2_y: args.nonces.r2_y,
        guilty_bitmap,
        guilty_count: 1,
        evidence_hash: args.evidence_hash,
        reporter,
        slashed_total: outcome.slashed_total,
        bounty_paid: outcome.bounty_paid,
    });

    Ok(())
}

fn map_nonce_equation_err(error: NonceEquationError) -> anchor_lang::error::Error {
    match error {
        NonceEquationError::InvalidNoncePoint => error!(MolphaError::InvalidNoncePoint),
        NonceEquationError::NonCanonicalScalar => error!(MolphaError::NonCanonicalScalar),
    }
}

#[derive(Accounts)]
#[instruction(args: PunishNonceReuseArgs)]
pub struct PunishNonceReuse<'info> {
    #[account(mut)]
    pub reporter: Signer<'info>,

    #[account(
        mut,
        seeds = [ProtocolConfig::SEED_PREFIX],
        bump = protocol_config.bump,
    )]
    pub protocol_config: Box<Account<'info, ProtocolConfig>>,

    #[account(
        mut,
        seeds = [RegistryState::SEED_PREFIX],
        bump = registry_state.bump,
    )]
    pub registry_state: Box<Account<'info, RegistryState>>,

    #[account(
        seeds = [Registry::SEED_PREFIX, args.registry_version.to_le_bytes().as_ref()],
        bump,
    )]
    pub evidence_registry: AccountLoader<'info, Registry>,

    #[account(
        mut,
        seeds = [Registry::SEED_PREFIX, registry_state.current_version.to_le_bytes().as_ref()],
        bump,
    )]
    pub current_registry: AccountLoader<'info, Registry>,

    #[account(
        init,
        payer = reporter,
        space = Registry::SPACE,
        seeds = [Registry::SEED_PREFIX, registry_state.next_version.to_le_bytes().as_ref()],
        bump,
    )]
    pub new_registry: AccountLoader<'info, Registry>,

    #[account(
        init,
        payer = reporter,
        space = PunishmentCase::SPACE,
        seeds = [PunishmentCase::SEED_PREFIX, args.evidence_hash.as_ref()],
        bump,
    )]
    pub punishment_case: Box<Account<'info, PunishmentCase>>,

    #[account(
        mut,
        associated_token::mint = protocol_config.usdc_mint,
        associated_token::authority = registry_state,
        associated_token::token_program = token_program,
    )]
    pub node_deposit_vault: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        associated_token::mint = protocol_config.usdc_mint,
        associated_token::authority = protocol_config,
        associated_token::token_program = token_program,
    )]
    pub treasury: Box<Account<'info, TokenAccount>>,

    /// Must already exist. Creating it here would cost two more transaction slots, which the
    /// signer list needs more than a reporter needs the convenience.
    #[account(
        mut,
        associated_token::mint = protocol_config.usdc_mint,
        associated_token::authority = reporter,
        associated_token::token_program = token_program,
    )]
    pub reporter_usdc: Box<Account<'info, TokenAccount>>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}
