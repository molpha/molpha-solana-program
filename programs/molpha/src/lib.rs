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
        node_id: [u8; 32],
    ) -> Result<()> {
        instructions::register_node(ctx, args, node_id)
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
        ctx: Context<'_, '_, 'info, 'info, SettleRoundReceipt<'info>>,
        args: SettleRoundArgs,
    ) -> Result<()> {
        instructions::settle_round_receipt(ctx, args)
    }

    pub fn finalize_round<'info>(
        ctx: Context<'_, '_, 'info, 'info, FinalizeRound<'info>>,
    ) -> Result<()> {
        instructions::finalize_round(ctx)
    }

    pub fn dispute_round<'info>(
        ctx: Context<'_, '_, 'info, 'info, DisputeRound<'info>>,
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

    // ---- Agent pay-per-request access (agent-access spec v1). ----

    pub fn create_agent_escrow(
        ctx: Context<CreateAgentEscrow>,
        args: CreateAgentEscrowArgs,
    ) -> Result<()> {
        instructions::create_agent_escrow(ctx, args)
    }

    pub fn deposit_agent_credits(ctx: Context<DepositAgentCredits>, amount: u64) -> Result<()> {
        instructions::deposit_agent_credits(ctx, amount)
    }

    pub fn update_agent_policy(
        ctx: Context<UpdateAgentPolicy>,
        args: UpdateAgentPolicyArgs,
    ) -> Result<()> {
        instructions::update_agent_policy(ctx, args)
    }

    pub fn set_agent_paused(ctx: Context<SetAgentPaused>, paused: bool) -> Result<()> {
        instructions::set_agent_paused(ctx, paused)
    }

    pub fn set_agent_frozen(ctx: Context<SetAgentFrozen>, frozen: bool) -> Result<()> {
        instructions::set_agent_frozen(ctx, frozen)
    }

    pub fn close_agent_escrow(ctx: Context<CloseAgentEscrow>) -> Result<()> {
        instructions::close_agent_escrow(ctx)
    }

    pub fn deposit_gateway_settlement(
        ctx: Context<DepositGatewaySettlement>,
        amount: u64,
    ) -> Result<()> {
        instructions::deposit_gateway_settlement(ctx, amount)
    }

    pub fn settle_agent_round<'info>(
        ctx: Context<'_, '_, 'info, 'info, SettleAgentRound<'info>>,
        args: SettleAgentRoundArgs,
    ) -> Result<()> {
        instructions::settle_agent_round(ctx, args)
    }
}
