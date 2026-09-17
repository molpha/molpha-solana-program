#![allow(deprecated)]

use anchor_lang::prelude::*;

pub mod allocator;
pub mod error;
pub mod events;
pub mod instructions;
pub mod state;
pub mod utils;

use instructions::*;
use molpha_verifier::Attestation;
use state::PlanType;

declare_id!("MoLFnEbuMS5gWnXNfUMLAYSqRM3eQZKWRzjeMQfqbT3");

#[program]
pub mod molpha {
    use super::*;

    pub fn initialize(ctx: Context<Initialize>, args: InitializeArgs) -> Result<()> {
        instructions::initialize(ctx, args)
    }

    pub fn update_config(ctx: Context<UpdateConfig>, args: UpdateConfigArgs) -> Result<()> {
        instructions::update_config(ctx, args)
    }

    pub fn register_node(ctx: Context<RegisterNode>, args: RegisterNodeArgs) -> Result<()> {
        instructions::register_node(ctx, args)
    }

    pub fn deactivate_node(ctx: Context<DeactivateNode>, node_owner: Pubkey) -> Result<()> {
        instructions::deactivate_node(ctx, node_owner)
    }

    pub fn freeze_unresponsive_node(
        ctx: Context<FreezeUnresponsiveNode>,
        node_owner: Pubkey,
    ) -> Result<()> {
        instructions::freeze_unresponsive_node(ctx, node_owner)
    }

    pub fn update_node_endpoint(
        ctx: Context<UpdateNodeEndpoint>,
        args: UpdateNodeEndpointArgs,
    ) -> Result<()> {
        instructions::update_node_endpoint(ctx, args)
    }

    pub fn update_registry_policy(
        ctx: Context<UpdateRegistryPolicy>,
        redundancy_buffer: u8,
    ) -> Result<()> {
        instructions::update_registry_policy(ctx, redundancy_buffer)
    }

    pub fn add_plan(ctx: Context<AddPlan>, args: AddPlanArgs) -> Result<()> {
        instructions::add_plan(ctx, args)
    }

    pub fn update_plan(ctx: Context<UpdatePlan>, args: UpdatePlanArgs) -> Result<()> {
        instructions::update_plan(ctx, args)
    }

    pub fn subscribe(ctx: Context<Subscribe>, plan_type: PlanType) -> Result<()> {
        instructions::subscribe(ctx, plan_type)
    }

    pub fn extend_subscription(ctx: Context<ExtendSubscription>) -> Result<()> {
        instructions::extend_subscription(ctx)
    }

    pub fn add_delegate(ctx: Context<AddDelegate>, args: AddDelegateArgs) -> Result<()> {
        instructions::add_delegate(ctx, args)
    }

    pub fn remove_delegate(ctx: Context<RemoveDelegate>, args: RemoveDelegateArgs) -> Result<()> {
        instructions::remove_delegate(ctx, args)
    }

    pub fn submit_attestation(
        ctx: Context<SubmitAttestation>,
        args: SubmitAttestationArgs,
    ) -> Result<()> {
        instructions::submit_attestation(ctx, args)
    }

    pub fn register_gateway(
        ctx: Context<RegisterGateway>,
        args: RegisterGatewayArgs,
    ) -> Result<()> {
        instructions::register_gateway(ctx, args)
    }

    pub fn settle_subscription_round<'info>(
        ctx: Context<SettleSubscriptionRound<'info>>,
        attestation: Attestation,
        consumer_authority: Pubkey,
    ) -> Result<()> {
        instructions::settle_subscription_round(ctx, attestation, consumer_authority)
    }

    pub fn finalize_round<'info>(ctx: Context<'info, FinalizeRound<'info>>) -> Result<()> {
        instructions::finalize_round(ctx)
    }

    pub fn validate_round<'info>(ctx: Context<'info, ValidateRound<'info>>) -> Result<()> {
        instructions::validate_round(ctx)
    }

    pub fn claim_node_rewards(ctx: Context<ClaimNodeRewards>) -> Result<()> {
        instructions::claim_node_rewards(ctx)
    }

    pub fn withdraw_node_deposit(ctx: Context<WithdrawNodeDeposit>) -> Result<()> {
        instructions::withdraw_node_deposit(ctx)
    }

    pub fn deactivate_gateway(ctx: Context<DeactivateGateway>) -> Result<()> {
        instructions::deactivate_gateway(ctx)
    }

    pub fn withdraw_gateway_bond(ctx: Context<WithdrawGatewayBond>) -> Result<()> {
        instructions::withdraw_gateway_bond(ctx)
    }

    pub fn settle_x402_round<'info>(
        ctx: Context<SettleX402Round<'info>>,
        attestation: Attestation,
        args: SettleX402RoundArgs,
    ) -> Result<()> {
        instructions::settle_x402_round(ctx, attestation, args)
    }

    pub fn punish_equivocation<'info>(
        ctx: Context<'info, PunishEquivocation<'info>>,
        args: PunishEquivocationArgs,
    ) -> Result<()> {
        instructions::punish_equivocation(ctx, args)
    }

    pub fn punish_unsolicited_signing<'info>(
        ctx: Context<'info, PunishUnsolicitedSigning<'info>>,
        args: PunishUnsolicitedSigningArgs,
    ) -> Result<()> {
        instructions::punish_unsolicited_signing(ctx, args)
    }

    pub fn punish_commitment_reuse<'info>(
        ctx: Context<'info, PunishCommitmentReuse<'info>>,
        args: PunishCommitmentReuseArgs,
    ) -> Result<()> {
        instructions::punish_commitment_reuse(ctx, args)
    }

    pub fn punish_nonce_reuse<'info>(
        ctx: Context<'info, PunishNonceReuse<'info>>,
        args: PunishNonceReuseArgs,
    ) -> Result<()> {
        instructions::punish_nonce_reuse(ctx, args)
    }

    pub fn reinstate_node(ctx: Context<ReinstateNode>) -> Result<()> {
        instructions::reinstate_node(ctx)
    }
}
