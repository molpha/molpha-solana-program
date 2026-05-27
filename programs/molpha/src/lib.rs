#![allow(deprecated)]

use anchor_lang::prelude::*;

pub mod error;
pub mod events;
pub mod instructions;
pub mod state;
pub mod utils;

use instructions::*;
use state::PlanType;

declare_id!("MLPHd2Pv9D4WhGUyh3FJYGvd4rR9SMProkSNcXm7zh9");

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

    pub fn create_job(
        ctx: Context<CreateJob>,
        args: CreateJobArgs,
        job_id: [u8; 32],
    ) -> Result<()> {
        instructions::create_job(ctx, args, job_id)
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

    pub fn verify_data_update(
        ctx: Context<VerifyDataUpdate>,
        args: SubmitDataUpdateArgs,
    ) -> Result<()> {
        instructions::verify_data_update(ctx, args)
    }
}
