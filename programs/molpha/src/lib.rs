#![allow(deprecated)]

use anchor_lang::prelude::*;

pub mod error;
pub mod events;
pub mod instructions;
pub mod state;
pub mod utils;

use instructions::*;
use state::{AggregatedSig, PlanType};

declare_id!("MoLFeTRpDZgckPjjbLwW1wB9n85bQiqboPnvw9RwoG8");

#[program]
pub mod molpha {
    use super::*;

    pub fn initialize(ctx: Context<Initialize>, args: InitializeArgs) -> Result<()> {
        instructions::initialize(ctx, args)
    }

    pub fn register_node(
        ctx: Context<RegisterNode>,
        args: RegisterNodeArgs,
    ) -> Result<()> {
        instructions::register_node(ctx, args)
    }

    pub fn deactivate_node(ctx: Context<DeactivateNode>) -> Result<()> {
        instructions::deactivate_node(ctx)
    }

    pub fn finalize_registry_transition(ctx: Context<FinalizeRegistryTransition>) -> Result<()> {
        instructions::finalize_registry_transition(ctx)
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

    pub fn admin_withdraw_subscription_vault(
        ctx: Context<AdminWithdrawSubscriptionVault>,
    ) -> Result<()> {
        instructions::admin_withdraw_subscription_vault(ctx)
    }

    pub fn add_delegate(ctx: Context<AddDelegate>, args: AddDelegateArgs) -> Result<()> {
        instructions::add_delegate(ctx, args)
    }

    pub fn remove_delegate(ctx: Context<RemoveDelegate>, args: RemoveDelegateArgs) -> Result<()> {
        instructions::remove_delegate(ctx, args)
    }

    pub fn set_delegate_paused(ctx: Context<SetDelegatePaused>, paused: bool) -> Result<()> {
        instructions::set_delegate_paused(ctx, paused)
    }

    pub fn submit_data_update(
        ctx: Context<SubmitDataUpdate>,
        args: SubmitDataUpdateArgs,
    ) -> Result<()> {
        instructions::submit_data_update(ctx, args)
    }

    pub fn register_gateway(
        ctx: Context<RegisterGateway>,
        args: RegisterGatewayArgs,
    ) -> Result<()> {
        instructions::register_gateway(ctx, args)
    }

    pub fn settle_round_receipt<'info>(
        ctx: Context<SettleRoundReceipt<'info>>,
        args: SettleRoundArgs,
    ) -> Result<()> {
        instructions::settle_round_receipt(ctx, args)
    }

    pub fn finalize_round<'info>(ctx: Context<FinalizeRound<'info>>) -> Result<()> {
        instructions::finalize_round(ctx)
    }

    pub fn dispute_round<'info>(
        ctx: Context<DisputeRound<'info>>,
        agg_sig: AggregatedSig,
    ) -> Result<()> {
        instructions::dispute_round(ctx, agg_sig)
    }

    pub fn claim_node_rewards(ctx: Context<ClaimNodeRewards>) -> Result<()> {
        instructions::claim_node_rewards(ctx)
    }

    pub fn claim_gateway_rewards(ctx: Context<ClaimGatewayRewards>) -> Result<()> {
        instructions::claim_gateway_rewards(ctx)
    }

    // ---- Self-funded / x402 agent pay-per-request access. ----

    pub fn settle_agent_round<'info>(
        ctx: Context<SettleAgentRound<'info>>,
        args: SettleAgentRoundArgs,
    ) -> Result<()> {
        instructions::settle_agent_round(ctx, args)
    }
}
