use anchor_lang::prelude::*;
use anchor_lang::solana_program::{ed25519_program, sysvar};

use crate::{
    error::FeedError,
    events::AnswerPublished,
    state::{Answer, Feed, Node, MAX_HISTORY},
    utils::{parse_ed25519_instruction, pricing::*},
};

const MAX_NODES_PER_PUBLISH: usize = 25;

pub fn publish_answer(ctx: Context<PublishAnswer>, answer: Answer) -> Result<()> {
    let feed = &mut ctx.accounts.feed;
    let clock = Clock::get()?;

    // Check if subscription is active
    require!(
        feed.is_subscription_active(clock.unix_timestamp),
        FeedError::SubscriptionExpired
    );

    require!(
        answer.timestamp > feed.latest_answer.timestamp,
        FeedError::PastTimestamp
    );
    require!(
        answer.timestamp <= clock.unix_timestamp,
        FeedError::FutureTimestamp
    );

    // Build a set of registered node pubkeys from remaining accounts
    let mut registered_nodes = Vec::new();
    let remaining = ctx.remaining_accounts;

    // Enforce maximum nodes limit to prevent exceeding transaction constraints
    require!(
        remaining.len() <= MAX_NODES_PER_PUBLISH,
        FeedError::TooManyNodes
    );

    for i in 0..remaining.len() {
        let account_info = &remaining[i];
        // Try to deserialize as Node account
        if account_info.owner == ctx.program_id {
            if let Ok(data) = account_info.try_borrow_data() {
                // Skip the 8-byte discriminator
                if data.len() > 8 {
                    if let Ok(node) = Node::try_deserialize(&mut &data[..]) {
                        // Verify the PDA derivation
                        let (expected_pda, _bump) = Pubkey::find_program_address(
                            &[Node::SEED_PREFIX, node.node_pubkey.as_ref()],
                            ctx.program_id,
                        );

                        if account_info.key == &expected_pda {
                            registered_nodes.push(node.node_pubkey);
                        }
                    }
                }
            }
        }
    }

    // Calculate actual priority fee paid (use provided nodes count)
    let estimated_compute_units = estimate_compute_units(
        registered_nodes.len() as u32,
        feed.answer_history.len() as u32,
    );

    let priority_fee = calculate_priority_fee_from_instructions(
        &ctx.accounts.instructions,
        estimated_compute_units,
    )?;

    // Validate signatures from registered nodes
    let instructions_sysvar = &ctx.accounts.instructions;
    let current_instruction_index =
        sysvar::instructions::load_current_index_checked(instructions_sysvar)?;

    let mut unique_valid_signers = Vec::new();
    let message = answer.value.to_vec();

    for i in (0..current_instruction_index).rev() {
        let instruction =
            sysvar::instructions::load_instruction_at_checked(i as usize, instructions_sysvar)?;

        if instruction.program_id == ed25519_program::ID {
            if let Ok((signer_pubkey, signed_message)) = parse_ed25519_instruction(&instruction) {
                if signed_message == message
                    && registered_nodes.contains(&signer_pubkey)
                    && !unique_valid_signers.contains(&signer_pubkey)
                {
                    unique_valid_signers.push(signer_pubkey);
                }
            }
        }
    }

    require!(
        unique_valid_signers.len() >= feed.min_signatures_threshold as usize,
        FeedError::NotEnoughSignatures
    );

    feed.latest_answer = answer;

    // Use a ring buffer for history
    if feed.answer_history.len() < MAX_HISTORY {
        feed.answer_history.push(answer);
        feed.history_idx = feed.answer_history.len() as u64;
    } else {
        let history_idx = feed.history_idx as usize;
        feed.answer_history[history_idx] = answer;
        feed.history_idx = (history_idx as u64 + 1) % MAX_HISTORY as u64;
    }

    msg!(
        "Successfully published answer with {} valid signatures. Priority fee: {}",
        unique_valid_signers.len(),
        priority_fee
    );

    // Emit event
    emit!(AnswerPublished {
        feed: ctx.accounts.feed.key(),
        answer,
        signatures_count: unique_valid_signers.len() as u8,
        published_at: clock.unix_timestamp,
    });

    Ok(())
}

#[derive(Accounts)]
pub struct PublishAnswer<'info> {
    #[account(
        mut,
        constraint = feed.is_subscription_active(Clock::get()?.unix_timestamp) @ FeedError::SubscriptionExpired,
        seeds = [
            Feed::SEED_PREFIX,
            feed.authority.as_ref(),
            &feed.name_seed(),
            feed.feed_type.to_seed().as_ref(),
            feed.min_signatures_threshold.to_le_bytes().as_ref(),
            feed.frequency.to_le_bytes().as_ref(),
            feed.job_id.as_ref()
        ],
        bump = feed.bump
    )]
    pub feed: Account<'info, Feed>,

    /// CHECK: This is safe. We only read the instructions sysvar for validation.
    #[account(address = sysvar::instructions::ID)]
    pub instructions: UncheckedAccount<'info>,
    // Note: Node PDAs should be passed as remaining_accounts
    // Each node PDA should have seeds = [Node::SEED_PREFIX, node_pubkey]
}

