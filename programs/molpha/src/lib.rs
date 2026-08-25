#![allow(deprecated)]

use anchor_lang::prelude::*;

pub mod error;
pub mod events;
pub mod instructions;
pub mod state;
pub mod utils;

use instructions::*;
use state::{AggregatedSig, PlanType};

declare_id!("MoLFGDpFoVnQgwbkTNScKPohCxhbfd61JjFrnotuwzh");

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

    pub fn deactivate_node(ctx: Context<DeactivateNode>, args: DeactivateNodeArgs) -> Result<()> {
        instructions::deactivate_node(ctx, args)
    }

    #[cfg(feature = "cu-bench")]
    pub fn bench_resolve_synthetic(ctx: Context<BenchRegistry>, node_count: u16) -> Result<()> {
        instructions::bench_resolve_synthetic(ctx, node_count)
    }

    #[cfg(feature = "cu-bench")]
    pub fn bench_resolve_accounts(
        ctx: Context<BenchRegistry>,
        signers_bitmap: [u8; 32],
    ) -> Result<()> {
        instructions::bench_resolve_accounts(ctx, signers_bitmap)
    }

    #[cfg(feature = "cu-bench")]
    pub fn bench_full_registry_transition(
        ctx: Context<BenchFullRegistryTransition>,
        old_version: u32,
        next_version: u32,
        mutation: u8,
        node: Pubkey,
        remove_index: u16,
    ) -> Result<()> {
        instructions::bench_full_registry_transition(
            ctx,
            old_version,
            next_version,
            mutation,
            node,
            remove_index,
        )
    }

    #[cfg(feature = "cu-bench")]
    pub fn bench_resolve_full_registry(
        ctx: Context<BenchResolveFullRegistry>,
        version: u32,
        signers_bitmap: [u8; 32],
    ) -> Result<()> {
        instructions::bench_resolve_full_registry(ctx, version, signers_bitmap)
    }

    pub fn update_node_endpoint(
        ctx: Context<UpdateNodeEndpoint>,
        args: UpdateNodeEndpointArgs,
    ) -> Result<()> {
        instructions::update_node_endpoint(ctx, args)
    }

    pub fn update_registry_policy(
        ctx: Context<UpdateRegistryPolicy>,
        new_registry_version: u32,
        redundancy_buffer: u8,
    ) -> Result<()> {
        instructions::update_registry_policy(ctx, new_registry_version, redundancy_buffer)
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

    pub fn set_delegate_paused(ctx: Context<SetDelegatePaused>, paused: bool) -> Result<()> {
        instructions::set_delegate_paused(ctx, paused)
    }

    pub fn submit_attestation(
        ctx: Context<SubmitAttestation>,
        args: SubmitAttestationArgs,
    ) -> Result<()> {
        instructions::submit_attestation(ctx, args)
    }

    pub fn verify_attestation(
        ctx: Context<VerifyAttestation>,
        args: SubmitAttestationArgs,
    ) -> Result<()> {
        instructions::verify_attestation(ctx, args)
    }

    pub fn register_gateway(
        ctx: Context<RegisterGateway>,
        args: RegisterGatewayArgs,
    ) -> Result<()> {
        instructions::register_gateway(ctx, args)
    }

    pub fn settle_subscription_round<'info>(
        ctx: Context<SettleSubscriptionRound<'info>>,
        args: SettleRoundArgs,
    ) -> Result<()> {
        instructions::settle_subscription_round(ctx, args)
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

    // ---- Unified x402 settlement (gateway-funded, all payment rails). ----

    pub fn settle_x402_round<'info>(
        ctx: Context<SettleX402Round<'info>>,
        args: SettleX402RoundArgs,
    ) -> Result<()> {
        instructions::settle_x402_round(ctx, args)
    }

    // ---- Node punishment. ----

    pub fn punish_equivocation<'info>(
        ctx: Context<PunishEquivocation<'info>>,
        args: PunishEquivocationArgs,
    ) -> Result<()> {
        instructions::punish_equivocation(ctx, args)
    }

    pub fn punish_unsolicited_signing<'info>(
        ctx: Context<PunishUnsolicitedSigning<'info>>,
        args: PunishUnsolicitedSigningArgs,
    ) -> Result<()> {
        instructions::punish_unsolicited_signing(ctx, args)
    }

    pub fn punish_nonce_reuse<'info>(
        ctx: Context<PunishNonceReuse<'info>>,
        args: PunishNonceReuseArgs,
    ) -> Result<()> {
        instructions::punish_nonce_reuse(ctx, args)
    }

    pub fn reinstate_node(ctx: Context<ReinstateNode>, args: ReinstateNodeArgs) -> Result<()> {
        instructions::reinstate_node(ctx, args)
    }
}
