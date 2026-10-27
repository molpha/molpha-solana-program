#![allow(deprecated)]

use anchor_lang::prelude::*;

pub mod allocator;
pub mod error;
pub mod events;
pub mod instructions;
pub mod state;
pub mod utils;

use instructions::*;
use molpha_verifier::{Attestation, CoalitionKey, KeyRotationProof};
use state::{PlanConfig, PlanType};

declare_id!("dzemhcTeUnUEhDcyUssWP8GfcMCYwKezVg4U1hvdEJe");

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

    pub fn extend_registry(ctx: Context<ExtendRegistry>) -> Result<()> {
        instructions::extend_registry(ctx)
    }

    pub fn rotate_node_key(ctx: Context<RotateNodeKey>, proof: KeyRotationProof) -> Result<()> {
        instructions::rotate_node_key(ctx, proof)
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

    pub fn add_plan(ctx: Context<AddPlan>, config: PlanConfig) -> Result<()> {
        instructions::add_plan(ctx, config)
    }

    pub fn update_plan(ctx: Context<UpdatePlan>, config: PlanConfig) -> Result<()> {
        instructions::update_plan(ctx, config)
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

    pub fn claim_node_rewards(ctx: Context<ClaimNodeRewards>) -> Result<()> {
        instructions::claim_node_rewards(ctx)
    }

    pub fn withdraw_node_deposit(ctx: Context<WithdrawNodeDeposit>) -> Result<()> {
        instructions::withdraw_node_deposit(ctx)
    }

    pub fn deactivate_gateway(ctx: Context<DeactivateGateway>) -> Result<()> {
        instructions::deactivate_gateway(ctx)
    }

    pub fn sweep_treasury_revenue(
        ctx: Context<SweepTreasuryRevenue>,
        open_epoch: u64,
    ) -> Result<()> {
        instructions::sweep_treasury_revenue(ctx, open_epoch)
    }

    pub fn withdraw_protocol_reserve(
        ctx: Context<WithdrawProtocolReserve>,
        amount: u64,
    ) -> Result<()> {
        instructions::withdraw_protocol_reserve(ctx, amount)
    }

    pub fn submit_ticket(
        ctx: Context<SubmitTicket>,
        epoch: u64,
        attestation: Attestation,
        roster_slots: Vec<u16>,
        coalition_key: CoalitionKey,
    ) -> Result<()> {
        instructions::submit_ticket(ctx, epoch, attestation, roster_slots, coalition_key)
    }

    pub fn close_ticket_marker(ctx: Context<CloseTicketMarker>) -> Result<()> {
        instructions::close_ticket_marker(ctx)
    }

    pub fn finalize_epoch(ctx: Context<FinalizeEpoch>, epoch: u64) -> Result<()> {
        instructions::finalize_epoch(ctx, epoch)
    }

    pub fn credit_epoch_rewards<'info>(
        ctx: Context<'info, CreditEpochRewards<'info>>,
        epoch: u64,
        slots: Vec<u16>,
    ) -> Result<()> {
        instructions::credit_epoch_rewards(ctx, epoch, slots)
    }

    pub fn close_epoch(ctx: Context<CloseEpoch>, epoch: u64, open_epoch: u64) -> Result<()> {
        instructions::close_epoch(ctx, epoch, open_epoch)
    }

    pub fn slash_offence<'info>(
        ctx: Context<'info, SlashOffence<'info>>,
        offence: state::Offence,
    ) -> Result<()> {
        instructions::slash_offence(ctx, offence)
    }

    pub fn reinstate_node(ctx: Context<ReinstateNode>) -> Result<()> {
        instructions::reinstate_node(ctx)
    }
}
