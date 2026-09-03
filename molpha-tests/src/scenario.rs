//! Composable setup: an initialized protocol, registered nodes, gateways, subscriptions, and
//! signed attestations.
//!
//! Every builder returns owned state, so a test can take a scenario as far as it needs without
//! reaching into a shared fixture.

use anchor_lang::prelude::Pubkey;
use anchor_lang::solana_program::instruction::Instruction;
use anchor_lang::{InstructionData, ToAccountMetas};
use litesvm::types::TransactionMetadata;
use molpha::instructions::{
    AddDelegateArgs, AddPlanArgs, DeactivateNodeArgs, InitializeArgs, PunishEquivocationArgs,
    PunishNonceReuseArgs, PunishUnsolicitedSigningArgs, RegisterGatewayArgs, RegisterNodeArgs,
    RemoveDelegateArgs, SettleRoundArgs, SettleX402RoundArgs, SubmitAttestationArgs,
    UpdateConfigArgs, UpdateNodeEndpointArgs, UpdatePlanArgs,
};
use molpha::state::{AggregatedSig, PlanType, RegistryState};
use solana_keypair::Keypair;
use solana_signer::Signer;

use crate::crypto::{self, AggregateSignature, NodeKey};
use crate::env::TestEnv;
use crate::pda;

/// A node that completed `register_node`.
pub struct RegisteredNode {
    pub owner: Keypair,
    pub key: NodeKey,
    pub pda: Pubkey,
    pub salt: [u8; 32],
    /// Index in the registry snapshot created by its own registration.
    pub index: u16,
}

/// The duty a round attests to — `(source_id, version, timestamp, quorum)` plus the value.
#[derive(Clone, Copy, Debug)]
pub struct Duty {
    pub source_id: [u8; 32],
    pub registry_version: u32,
    pub canonical_timestamp: u64,
    pub signatures_required: u8,
    pub value: [u8; 32],
}

impl Duty {
    pub fn new(registry_version: u32, signatures_required: u8) -> Self {
        Self {
            source_id: [0x11; 32],
            registry_version,
            canonical_timestamp: 1_700_000_000,
            signatures_required,
            value: [0x42; 32],
        }
    }

    pub fn source_id(mut self, source_id: [u8; 32]) -> Self {
        self.source_id = source_id;
        self
    }

    pub fn at(mut self, canonical_timestamp: u64) -> Self {
        self.canonical_timestamp = canonical_timestamp;
        self
    }

    pub fn value(mut self, value: [u8; 32]) -> Self {
        self.value = value;
        self
    }

    pub fn payload(&self, signers_bitmap: [u8; 32]) -> [u8; 32] {
        molpha_verifier::compute_message_hash(
            &molpha_verifier::AttestationPayload {
                source_id: self.source_id,
                registry_version: self.registry_version,
                value: self.value,
                canonical_timestamp: self.canonical_timestamp,
                signatures_required: self.signatures_required,
            },
            signers_bitmap,
        )
    }
}

/// A duty plus a coalition signature over it.
#[derive(Clone, Debug)]
pub struct SignedAttestation {
    pub duty: Duty,
    pub signers_bitmap: [u8; 32],
    pub message_hash: [u8; 32],
    pub signature: AggregateSignature,
    /// Node PDAs for the set bits, in ascending bit order — the `remaining_accounts` order.
    pub signer_pdas: Vec<Pubkey>,
}

impl SignedAttestation {
    pub fn submit_args(&self) -> SubmitAttestationArgs {
        SubmitAttestationArgs {
            source_id: self.duty.source_id,
            registry_version: self.duty.registry_version,
            value: self.duty.value,
            canonical_timestamp: self.duty.canonical_timestamp,
            signatures_required: self.duty.signatures_required,
            agg_sig_s: self.signature.agg_sig_s,
            commitment: self.signature.commitment,
            signers_bitmap: self.signers_bitmap,
        }
    }

    pub fn aggregated_sig(&self) -> AggregatedSig {
        AggregatedSig {
            agg_sig_s: self.signature.agg_sig_s,
            commitment: self.signature.commitment,
            signers_bitmap: self.signers_bitmap,
        }
    }

    /// A copy whose signature no longer verifies, with the committed shape left intact.
    pub fn with_tampered_signature(&self) -> Self {
        let mut tampered = self.clone();
        tampered.signature.agg_sig_s[31] ^= 0x01;
        tampered
    }

    pub fn statement(&self) -> molpha::state::SignedStatement {
        molpha::state::SignedStatement {
            value: self.duty.value,
            agg_sig_s: self.signature.agg_sig_s,
            commitment_addr: self.signature.commitment,
            signers_bitmap: self.signers_bitmap,
        }
    }

    pub fn signing_context(&self) -> molpha::state::SigningContext {
        molpha::state::SigningContext {
            source_id: self.duty.source_id,
            signatures_required: self.duty.signatures_required,
            canonical_timestamp: self.duty.canonical_timestamp,
        }
    }
}

/// An environment with a funded authority and a USDC mint, but no `initialize` yet.
///
/// The starting point for tests about `initialize` itself; [`ProtocolBuilder::start`] builds on it.
pub struct Bare {
    pub env: TestEnv,
    pub authority: Keypair,
    pub usdc_mint: Pubkey,
    pub mint_authority: Keypair,
}

pub fn bare() -> Bare {
    bare_with(TestEnv::new())
}

/// [`bare`] against a caller-supplied VM.
pub fn bare_with(env: TestEnv) -> Bare {
    let mut env = env;
    let authority = env.keypair();
    let (usdc_mint, mint_authority) = env.create_mint(6);
    Bare {
        env,
        authority,
        usdc_mint,
        mint_authority,
    }
}

/// The `InitializeArgs` every scenario starts from.
pub fn default_initialize_args(usdc_mint: Pubkey) -> InitializeArgs {
    InitializeArgs {
        withdrawal_cooldown_slots: 432_000,
        redundancy_buffer: 2,
        minimum_node_deposit: 100_000_000,
        reward_per_signature: 10_000,
        // Wide enough that every hardcoded duty timestamp stays inside the window even after
        // a cooldown-length clock advance; freshness tests tighten it via the builder.
        max_receipt_delay_seconds: 604_800,
        dispute_window_slots: 100,
        gateway_bond_min: 500_000_000,
        challenger_bounty_bps: 500,
        usdc_mint,
        x402_round_base: 1_000_000,
        min_signers: 3,
    }
}

/// `usdc_mint_account` is passed separately from `args.usdc_mint` so a test can disagree with
/// itself on purpose.
pub fn initialize_ix(
    authority: &Pubkey,
    usdc_mint_account: &Pubkey,
    args: &InitializeArgs,
) -> Instruction {
    Instruction {
        program_id: molpha::ID,
        accounts: molpha::accounts::Initialize {
            protocol_config: pda::protocol_config(),
            registry_state: pda::registry_state(),
            registry: pda::registry(0),
            usdc_mint: *usdc_mint_account,
            treasury: pda::ata(&pda::protocol_config(), usdc_mint_account),
            node_deposits: pda::node_deposits(),
            node_deposit_vault: pda::ata(&pda::node_deposits(), usdc_mint_account),
            authority: *authority,
            token_program: pda::TOKEN_PROGRAM_ID,
            associated_token_program: pda::ASSOCIATED_TOKEN_PROGRAM_ID,
            system_program: anchor_lang::solana_program::system_program::ID,
        }
        .to_account_metas(None),
        data: molpha::instruction::Initialize { args: args.clone() }.data(),
    }
}

pub fn update_config_ix(authority: &Pubkey, args: &UpdateConfigArgs) -> Instruction {
    Instruction {
        program_id: molpha::ID,
        accounts: molpha::accounts::UpdateConfig {
            authority: *authority,
            protocol_config: pda::protocol_config(),
        }
        .to_account_metas(None),
        data: molpha::instruction::UpdateConfig { args: args.clone() }.data(),
    }
}

/// Builder for [`Protocol`]: everything a test might want to vary about `InitializeArgs`.
pub struct ProtocolBuilder {
    args: InitializeArgs,
    env: Option<TestEnv>,
}

impl Default for ProtocolBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ProtocolBuilder {
    pub fn new() -> Self {
        Self {
            // The mint is replaced with the real one in `start`.
            args: default_initialize_args(Pubkey::default()),
            env: None,
        }
    }

    pub fn redundancy_buffer(mut self, value: u8) -> Self {
        self.args.redundancy_buffer = value;
        self
    }

    pub fn min_signers(mut self, value: u8) -> Self {
        self.args.min_signers = value;
        self
    }

    pub fn dispute_window_slots(mut self, value: u64) -> Self {
        self.args.dispute_window_slots = value;
        self
    }

    pub fn reward_per_signature(mut self, value: u64) -> Self {
        self.args.reward_per_signature = value;
        self
    }

    pub fn challenger_bounty_bps(mut self, value: u16) -> Self {
        self.args.challenger_bounty_bps = value;
        self
    }

    pub fn gateway_bond_min(mut self, value: u64) -> Self {
        self.args.gateway_bond_min = value;
        self
    }

    pub fn minimum_node_deposit(mut self, value: u64) -> Self {
        self.args.minimum_node_deposit = value;
        self
    }

    pub fn withdrawal_cooldown_slots(mut self, value: u64) -> Self {
        self.args.withdrawal_cooldown_slots = value;
        self
    }

    pub fn max_receipt_delay_seconds(mut self, value: u64) -> Self {
        self.args.max_receipt_delay_seconds = value;
        self
    }

    /// Build the environment and run `initialize`.
    /// Run against a caller-supplied VM, for tests that need a non-default feature set.
    pub fn with_env(mut self, env: TestEnv) -> Self {
        self.env = Some(env);
        self
    }

    pub fn start(self) -> Protocol {
        let Bare {
            mut env,
            authority,
            usdc_mint,
            mint_authority,
        } = match self.env {
            Some(env) => bare_with(env),
            None => bare(),
        };
        let args = InitializeArgs {
            usdc_mint,
            ..self.args
        };
        let ix = initialize_ix(&authority.pubkey(), &usdc_mint, &args);
        env.send_ok(&[ix], &[&authority]);

        Protocol {
            env,
            authority,
            usdc_mint,
            mint_authority,
            args,
            nodes: Vec::new(),
        }
    }
}

/// An initialized protocol plus whatever the test has built on top of it.
pub struct Protocol {
    pub env: TestEnv,
    pub authority: Keypair,
    pub usdc_mint: Pubkey,
    pub mint_authority: Keypair,
    pub args: InitializeArgs,
    pub nodes: Vec<RegisteredNode>,
}

impl Protocol {
    pub fn builder() -> ProtocolBuilder {
        ProtocolBuilder::new()
    }

    pub fn start() -> Self {
        ProtocolBuilder::new().start()
    }

    pub fn treasury(&self) -> Pubkey {
        pda::ata(&pda::protocol_config(), &self.usdc_mint)
    }

    pub fn treasury_balance(&self) -> u64 {
        self.env.token_balance(&self.treasury())
    }

    /// The one vault holding every node's security deposit.
    pub fn node_deposit_vault(&self) -> Pubkey {
        pda::ata(&pda::node_deposits(), &self.usdc_mint)
    }

    pub fn node_deposit_balance(&self) -> u64 {
        self.env.token_balance_or_zero(&self.node_deposit_vault())
    }

    /// Sum the principal owed to every node registered through this fixture.
    pub fn total_locked_deposits(&self) -> u64 {
        self.node_pdas()
            .iter()
            .map(|node| self.env.get::<molpha::state::Node>(node).locked_amount)
            .sum()
    }

    /// `Σ Node.claimable_rewards` as the protocol tracks it.
    pub fn reward_liability(&self) -> u64 {
        self.env
            .get::<molpha::state::ProtocolConfig>(&pda::protocol_config())
            .reward_liability
    }

    /// Assert the custody invariants the treasury separation exists to guarantee.
    ///
    /// Cheap enough to call at the end of any test that moves money, and the fastest way to
    /// notice a payout path that broke principal custody or reward accounting.
    pub fn assert_solvent(&self) {
        assert!(
            self.node_deposit_balance() >= self.total_locked_deposits(),
            "deposit vault {} cannot cover the {} it holds for nodes",
            self.node_deposit_balance(),
            self.total_locked_deposits()
        );

        let owed_rewards: u64 = self
            .node_pdas()
            .iter()
            .map(|node| self.env.get::<molpha::state::Node>(node).claimable_rewards)
            .sum();
        assert_eq!(
            self.reward_liability(),
            owed_rewards,
            "reward_liability is not the sum of every claimable_rewards"
        );
        assert!(
            self.treasury_balance() >= self.reward_liability(),
            "treasury {} cannot back the {} it owes nodes",
            self.treasury_balance(),
            self.reward_liability()
        );
    }

    /// Donate USDC straight into the treasury ATA — the plain SPL top-up that funds rewards
    /// beyond collected revenue.
    pub fn fund_treasury(&mut self, amount: u64) -> Pubkey {
        let config = pda::protocol_config();
        self.fund_usdc(&config, amount)
    }

    pub fn current_version(&self) -> u32 {
        self.env
            .get::<RegistryState>(&pda::registry_state())
            .current_version
    }

    pub fn node_pdas(&self) -> Vec<Pubkey> {
        self.nodes.iter().map(|n| n.pda).collect()
    }

    /// A funded actor who can dispute a round.
    ///
    /// `validate_round` pays the challenger bounty into an account it does not create, so a
    /// challenger needs a USDC account before it can dispute.
    pub fn challenger(&mut self) -> Keypair {
        let challenger = self.env.keypair();
        self.fund_usdc(&challenger.pubkey(), 0);
        challenger
    }

    /// A funded actor who can file a punishment.
    ///
    /// The bounty is paid by the filing itself into an account it does not create, so a reporter
    /// needs a USDC account before they can file.
    pub fn reporter(&mut self) -> Keypair {
        let reporter = self.env.keypair();
        self.fund_usdc(&reporter.pubkey(), 0);
        reporter
    }

    /// Fund `owner`'s USDC ATA.
    pub fn fund_usdc(&mut self, owner: &Pubkey, amount: u64) -> Pubkey {
        let mint = self.usdc_mint;
        let mint_authority = self.mint_authority.insecure_clone();
        self.env.mint_to(&mint, &mint_authority, owner, amount)
    }

    // -----------------------------------------------------------------------
    // node registry
    // -----------------------------------------------------------------------

    pub fn register_node_ix(
        &self,
        owner: &Pubkey,
        args: &RegisterNodeArgs,
        authority: &Pubkey,
    ) -> Instruction {
        self.register_node_ix_with_registry(owner, args, authority, self.current_version() + 1)
    }

    pub fn register_node_ix_with_registry(
        &self,
        owner: &Pubkey,
        args: &RegisterNodeArgs,
        authority: &Pubkey,
        new_registry_version: u32,
    ) -> Instruction {
        let current_version = self.current_version();
        Instruction {
            program_id: molpha::ID,
            accounts: molpha::accounts::RegisterNode {
                authority: *authority,
                node_owner: *owner,
                protocol_config: pda::protocol_config(),
                registry_state: pda::registry_state(),
                current_registry: pda::registry(current_version),
                new_registry: pda::registry(new_registry_version),
                node: pda::node(owner),
                secp256k1_key_claim: pda::secp256k1_key_claim(&args.secp256k1_pubkey_compressed),
                owner_usdc: pda::ata(owner, &self.usdc_mint),
                node_deposits: pda::node_deposits(),
                node_deposit_vault: self.node_deposit_vault(),
                system_program: anchor_lang::solana_program::system_program::ID,
                token_program: pda::TOKEN_PROGRAM_ID,
            }
            .to_account_metas(None),
            data: molpha::instruction::RegisterNode { args: args.clone() }.data(),
        }
    }

    /// Well-formed `register_node` args for `key`, with a valid proof of possession.
    pub fn register_node_args(
        &self,
        owner: &Pubkey,
        key: &NodeKey,
        salt: [u8; 32],
        port: u16,
    ) -> RegisterNodeArgs {
        let node_id = crypto::derive_node_id(owner, &key.compressed, &salt);
        let nonce = solana_keccak_hasher::hashv(&[b"molpha-test-pop-nonce", &node_id]).to_bytes();
        let (pop_sig_r, pop_sig_s) = crypto::sign_pop(&molpha::ID, &node_id, key, &nonce);
        RegisterNodeArgs {
            registration_salt: salt,
            ip: [127, 0, 0, 1],
            port,
            secp256k1_pubkey_compressed: key.compressed,
            pop_sig_r,
            pop_sig_s,
        }
    }

    /// Register `count` nodes with deterministic keys derived from `domain`.
    pub fn register_nodes(&mut self, domain: &str, count: usize) {
        let keys = crypto::node_keys(domain, count);
        for (i, key) in keys.into_iter().enumerate() {
            self.register_node(key, i);
        }
    }

    /// Register one node with caller-supplied key material, appending it to the registry.
    pub fn register_node_with_key(&mut self, key: NodeKey) {
        let sequence = self.nodes.len();
        self.register_node(key, sequence);
    }

    fn register_node(&mut self, key: NodeKey, sequence: usize) {
        let owner = self.env.keypair();
        let deposit = self.args.minimum_node_deposit;
        self.fund_usdc(&owner.pubkey(), deposit * 4);

        let mut salt = [0u8; 32];
        salt[0] = 1;
        salt[1..5].copy_from_slice(&(sequence as u32).to_be_bytes());

        let args = self.register_node_args(
            &owner.pubkey(),
            &key,
            salt,
            8_000u16.wrapping_add(sequence as u16),
        );
        let authority = self.authority.insecure_clone();
        let ix = self.register_node_ix(&owner.pubkey(), &args, &authority.pubkey());
        self.env.send_ok(&[ix], &[&authority, &owner]);

        let pda = pda::node(&owner.pubkey());
        self.nodes.push(RegisteredNode {
            index: (self.nodes.len()) as u16,
            owner,
            key,
            pda,
            salt,
        });
    }

    pub fn deactivate_node_ix(&self, index: usize, caller: &Pubkey) -> Instruction {
        let node = &self.nodes[index];
        let current_version = self.current_version();
        let registry = self.env.registry(current_version);
        let remove_index = registry
            .index_of(&node.pda)
            .expect("node is in the current registry");
        self.deactivate_node_raw_ix(
            caller,
            DeactivateNodeArgs {
                node_owner: node.owner.pubkey(),
                remove_index,
            },
        )
    }

    /// `deactivate_node` with caller-chosen args — for index and version failure cases.
    pub fn deactivate_node_raw_ix(&self, caller: &Pubkey, args: DeactivateNodeArgs) -> Instruction {
        let current_version = self.current_version();
        Instruction {
            program_id: molpha::ID,
            accounts: molpha::accounts::DeactivateNode {
                authority: *caller,
                protocol_config: pda::protocol_config(),
                registry_state: pda::registry_state(),
                current_registry: pda::registry(current_version),
                new_registry: pda::registry(current_version + 1),
                node: pda::node(&args.node_owner),
                system_program: anchor_lang::solana_program::system_program::ID,
            }
            .to_account_metas(None),
            data: molpha::instruction::DeactivateNode { args }.data(),
        }
    }

    // -----------------------------------------------------------------------
    // plans, subscriptions, delegates
    // -----------------------------------------------------------------------

    pub fn add_plan(&mut self, args: AddPlanArgs) -> TransactionMetadata {
        let authority = self.authority.insecure_clone();
        let ix = self.add_plan_ix(&args, &authority.pubkey());
        self.env.send_ok(&[ix], &[&authority])
    }

    pub fn add_plan_ix(&self, args: &AddPlanArgs, authority: &Pubkey) -> Instruction {
        Instruction {
            program_id: molpha::ID,
            accounts: molpha::accounts::AddPlan {
                protocol_config: pda::protocol_config(),
                plan: pda::plan(args.plan_type),
                authority: *authority,
                system_program: anchor_lang::solana_program::system_program::ID,
            }
            .to_account_metas(None),
            data: molpha::instruction::AddPlan { args: args.clone() }.data(),
        }
    }

    /// A plan with generous limits, for tests that are not about plan limits.
    pub fn add_default_plan(&mut self, plan_type: PlanType) -> AddPlanArgs {
        let args = AddPlanArgs {
            plan_type,
            subscription_price: 50_000_000,
            max_signers: 16,
            max_delegates: 4,
            max_rounds: 100,
            private_api_enabled: true,
            is_active: true,
        };
        self.add_plan(args.clone());
        args
    }

    pub fn subscribe_ix(&self, owner: &Pubkey, plan_type: PlanType) -> Instruction {
        Instruction {
            program_id: molpha::ID,
            accounts: molpha::accounts::Subscribe {
                owner: *owner,
                protocol_config: pda::protocol_config(),
                plan: pda::plan(plan_type),
                subscription: pda::subscription(owner),
                owner_usdc: pda::ata(owner, &self.usdc_mint),
                treasury: self.treasury(),
                system_program: anchor_lang::solana_program::system_program::ID,
                token_program: pda::TOKEN_PROGRAM_ID,
            }
            .to_account_metas(None),
            data: molpha::instruction::Subscribe { plan_type }.data(),
        }
    }

    /// A funded subscriber holding an active subscription to `plan_type`.
    pub fn subscribe(&mut self, plan_type: PlanType) -> Keypair {
        let owner = self.env.keypair();
        self.fund_usdc(&owner.pubkey(), 1_000_000_000);
        let ix = self.subscribe_ix(&owner.pubkey(), plan_type);
        self.env.send_ok(&[ix], &[&owner]);
        owner
    }

    pub fn add_delegate_ix(&self, owner: &Pubkey, args: &AddDelegateArgs) -> Instruction {
        Instruction {
            program_id: molpha::ID,
            accounts: molpha::accounts::AddDelegate {
                owner: *owner,
                subscription: pda::subscription(owner),
                delegate_account: pda::delegate(owner, &args.delegate),
                system_program: anchor_lang::solana_program::system_program::ID,
            }
            .to_account_metas(None),
            data: molpha::instruction::AddDelegate { args: args.clone() }.data(),
        }
    }

    pub fn add_delegate(
        &mut self,
        owner: &Keypair,
        delegate: &Pubkey,
        max_data_requests: u64,
        max_signers: u8,
    ) {
        let args = AddDelegateArgs {
            delegate: *delegate,
            max_data_requests,
            max_signers,
        };
        let ix = self.add_delegate_ix(&owner.pubkey(), &args);
        let owner = owner.insecure_clone();
        self.env.send_ok(&[ix], &[&owner]);
    }

    // -----------------------------------------------------------------------
    // gateway
    // -----------------------------------------------------------------------

    pub fn register_gateway_ix(
        &self,
        authority: &Pubkey,
        args: &RegisterGatewayArgs,
    ) -> Instruction {
        Instruction {
            program_id: molpha::ID,
            accounts: molpha::accounts::RegisterGateway {
                gateway_authority: *authority,
                protocol_config: pda::protocol_config(),
                gateway: pda::gateway(authority),
                usdc_mint: self.usdc_mint,
                authority_usdc: pda::ata(authority, &self.usdc_mint),
                gateway_bond: pda::ata(&pda::gateway(authority), &self.usdc_mint),
                system_program: anchor_lang::solana_program::system_program::ID,
                token_program: pda::TOKEN_PROGRAM_ID,
                associated_token_program: pda::ASSOCIATED_TOKEN_PROGRAM_ID,
            }
            .to_account_metas(None),
            data: molpha::instruction::RegisterGateway { args: args.clone() }.data(),
        }
    }

    /// A bonded gateway whose authority also holds a USDC float for x402 settlement.
    pub fn register_gateway(&mut self, float: u64) -> Keypair {
        let authority = self.env.keypair();
        let bond = self.args.gateway_bond_min;
        self.fund_usdc(&authority.pubkey(), bond + float);
        let args = RegisterGatewayArgs {
            ip: [10, 0, 0, 7],
            port: 9_000,
            bond_amount: bond,
        };
        let ix = self.register_gateway_ix(&authority.pubkey(), &args);
        self.env.send_ok(&[ix], &[&authority]);
        authority
    }

    pub fn deactivate_gateway_ix(
        &self,
        protocol_authority: &Pubkey,
        gateway_authority: &Pubkey,
    ) -> Instruction {
        Instruction {
            program_id: molpha::ID,
            accounts: molpha::accounts::DeactivateGateway {
                authority: *protocol_authority,
                protocol_config: pda::protocol_config(),
                gateway: pda::gateway(gateway_authority),
                system_program: anchor_lang::solana_program::system_program::ID,
            }
            .to_account_metas(None),
            data: molpha::instruction::DeactivateGateway {}.data(),
        }
    }

    pub fn withdraw_gateway_bond_ix(&self, authority: &Pubkey) -> Instruction {
        let gateway = pda::gateway(authority);
        Instruction {
            program_id: molpha::ID,
            accounts: molpha::accounts::WithdrawGatewayBond {
                authority: *authority,
                protocol_config: pda::protocol_config(),
                gateway,
                gateway_bond: pda::ata(&gateway, &self.usdc_mint),
                destination: pda::ata(authority, &self.usdc_mint),
                token_program: pda::TOKEN_PROGRAM_ID,
                associated_token_program: pda::ASSOCIATED_TOKEN_PROGRAM_ID,
                system_program: anchor_lang::solana_program::system_program::ID,
            }
            .to_account_metas(None),
            data: molpha::instruction::WithdrawGatewayBond {}.data(),
        }
    }

    // -----------------------------------------------------------------------
    // attestations
    // -----------------------------------------------------------------------

    /// The deterministic selection set for `duty` against the registry it names.
    pub fn selection_bits(&self, duty: &Duty) -> Vec<usize> {
        let registry = self.env.registry(duty.registry_version);
        let bitmap = molpha_verifier::derive_selection_bitmap(
            &duty.source_id,
            duty.registry_version,
            duty.canonical_timestamp,
            u32::from(registry.node_count),
            duty.signatures_required,
            registry.redundancy_buffer,
        )
        .expect("selection derives");
        crypto::bitmap_bits(&bitmap)
    }

    /// Sign `duty` with the whole derived selection set.
    pub fn sign_duty(&self, duty: Duty) -> SignedAttestation {
        let bits = self.selection_bits(&duty);
        self.sign_duty_with_bits(duty, &bits, None)
    }

    /// Sign `duty` with exactly `bits` — the lever for quorum, subset and guilt-set tests.
    pub fn sign_duty_with_bits(
        &self,
        duty: Duty,
        bits: &[usize],
        nonce: Option<[u8; 32]>,
    ) -> SignedAttestation {
        let registry = self.env.registry(duty.registry_version);
        let signer_keys: Vec<&NodeKey> = bits
            .iter()
            .map(|bit| {
                let node_pda = registry.nodes[*bit];
                &self
                    .nodes
                    .iter()
                    .find(|n| n.pda == node_pda)
                    .unwrap_or_else(|| panic!("no key material for registry bit {bit}"))
                    .key
            })
            .collect();
        let signer_pdas = bits.iter().map(|bit| registry.nodes[*bit]).collect();

        let signers_bitmap = crypto::bitmap_from_bits(bits);
        let message_hash = duty.payload(signers_bitmap);
        let signature = crypto::sign_aggregate(&signer_keys, &message_hash, nonce);

        SignedAttestation {
            duty,
            signers_bitmap,
            message_hash,
            signature,
            signer_pdas,
        }
    }

    pub fn submit_attestation_ix(
        &self,
        submitter: &Pubkey,
        attestation: &SignedAttestation,
    ) -> Instruction {
        let args = attestation.submit_args();
        let mut accounts = molpha::accounts::SubmitAttestation {
            submitter: *submitter,
            registry: pda::registry(args.registry_version),
            feed: pda::feed(&args.source_id, args.signatures_required, submitter),
            protocol_config: pda::protocol_config(),
            system_program: anchor_lang::solana_program::system_program::ID,
        }
        .to_account_metas(None);
        accounts.extend(crate::env::readonly_metas(
            attestation.signer_pdas.iter().copied(),
        ));
        Instruction {
            program_id: molpha::ID,
            accounts,
            data: molpha::instruction::SubmitAttestation { args }.data(),
        }
    }

    // -----------------------------------------------------------------------
    // settlement
    // -----------------------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    pub fn settle_subscription_round_ix(
        &self,
        gateway_authority: &Pubkey,
        subscription_owner: &Pubkey,
        consumer_authority: &Pubkey,
        consumer_delegate: Option<Pubkey>,
        attestation: &SignedAttestation,
    ) -> Instruction {
        let duty = attestation.duty;
        let args = SettleRoundArgs {
            canonical_timestamp: duty.canonical_timestamp,
            source_id: duty.source_id,
            signatures_required: duty.signatures_required,
            registry_version: duty.registry_version,
            agg_sig: attestation.aggregated_sig(),
            value: duty.value,
            consumer_authority: *consumer_authority,
        };
        let gateway = pda::gateway(gateway_authority);
        Instruction {
            program_id: molpha::ID,
            accounts: molpha::accounts::SettleSubscriptionRound {
                authority: *gateway_authority,
                protocol_config: pda::protocol_config(),
                registry_state: pda::registry_state(),
                registry: pda::registry(duty.registry_version),
                subscription: pda::subscription(subscription_owner),
                consumer_delegate,
                gateway,
                round_record: pda::subscription_round(
                    subscription_owner,
                    &gateway,
                    &duty.source_id,
                    duty.signatures_required,
                    duty.canonical_timestamp,
                ),
                instructions_sysvar: pda::INSTRUCTIONS_SYSVAR_ID,
                system_program: anchor_lang::solana_program::system_program::ID,
            }
            .to_account_metas(None),
            data: molpha::instruction::SettleSubscriptionRound { args }.data(),
        }
    }

    /// The Ed25519 instruction that must sit immediately before a subscription settlement.
    ///
    /// `gateway_authority` names the gateway the authorization is scoped to; the preimage commits
    /// to that gateway's PDA, so no other gateway can redeem the signature.
    pub fn consumer_authorization_ix(
        &self,
        consumer: &Keypair,
        gateway_authority: &Pubkey,
        duty: &Duty,
    ) -> Instruction {
        let request_auth_hash = molpha::state::hash_request_auth_fields(
            &pda::gateway(gateway_authority),
            &duty.source_id,
            duty.signatures_required,
            duty.canonical_timestamp,
        );
        crate::ed25519::sign_ed25519(consumer, &request_auth_hash)
    }

    pub fn settle_x402_round_ix(
        &self,
        gateway_authority: &Pubkey,
        args: SettleX402RoundArgs,
    ) -> Instruction {
        let gateway = pda::gateway(gateway_authority);
        Instruction {
            program_id: molpha::ID,
            accounts: molpha::accounts::SettleX402Round {
                authority: *gateway_authority,
                protocol_config: pda::protocol_config(),
                registry_state: pda::registry_state(),
                registry: pda::registry(args.registry_version),
                gateway,
                gateway_usdc: pda::ata(gateway_authority, &self.usdc_mint),
                treasury: self.treasury(),
                token_program: pda::TOKEN_PROGRAM_ID,
                round_record: pda::x402_round(
                    &args.payer_id,
                    &gateway,
                    &args.source_id,
                    args.signatures_required,
                    args.canonical_timestamp,
                ),
                system_program: anchor_lang::solana_program::system_program::ID,
            }
            .to_account_metas(None),
            data: molpha::instruction::SettleX402Round { args }.data(),
        }
    }

    /// Well-formed x402 args at the protocol's own price.
    pub fn x402_args(
        &self,
        attestation: &SignedAttestation,
        payer_id: [u8; 32],
    ) -> SettleX402RoundArgs {
        let duty = attestation.duty;
        let price = self.args.x402_round_base
            + u64::from(duty.signatures_required + self.args.redundancy_buffer)
                * self.args.reward_per_signature;
        SettleX402RoundArgs {
            payer_id,
            amount: price,
            source_id: duty.source_id,
            canonical_timestamp: duty.canonical_timestamp,
            signatures_required: duty.signatures_required,
            registry_version: duty.registry_version,
            agg_sig: attestation.aggregated_sig(),
            value: duty.value,
            payment_rail: 1,
            payment_ref: b"test-payment-ref".to_vec(),
        }
    }

    pub fn finalize_round_ix(
        &self,
        caller: &Pubkey,
        round_record: &Pubkey,
        credited_nodes: &[Pubkey],
    ) -> Instruction {
        let record = self
            .env
            .get::<molpha::state::RoundReceiptRecord>(round_record);
        let mut accounts = molpha::accounts::FinalizeRound {
            caller: *caller,
            protocol_config: pda::protocol_config(),
            treasury: self.treasury(),
            registry: pda::registry(record.registry_version),
            round_record: *round_record,
            gateway: record.gateway,
        }
        .to_account_metas(None);
        accounts.extend(crate::env::writable_metas(credited_nodes.iter().copied()));
        Instruction {
            program_id: molpha::ID,
            accounts,
            data: molpha::instruction::FinalizeRound {}.data(),
        }
    }

    pub fn validate_round_ix(
        &self,
        challenger: &Pubkey,
        round_record: &Pubkey,
        subscription: Option<Pubkey>,
        signer_nodes: &[Pubkey],
    ) -> Instruction {
        let record = self
            .env
            .get::<molpha::state::RoundReceiptRecord>(round_record);
        let mut accounts = molpha::accounts::ValidateRound {
            challenger: *challenger,
            protocol_config: pda::protocol_config(),
            registry: pda::registry(record.registry_version),
            round_record: *round_record,
            gateway: record.gateway,
            usdc_mint: self.usdc_mint,
            gateway_bond: pda::ata(&record.gateway, &self.usdc_mint),
            treasury: self.treasury(),
            challenger_usdc: pda::ata(challenger, &self.usdc_mint),
            token_program: pda::TOKEN_PROGRAM_ID,
            associated_token_program: pda::ASSOCIATED_TOKEN_PROGRAM_ID,
            system_program: anchor_lang::solana_program::system_program::ID,
            subscription,
        }
        .to_account_metas(None);
        accounts.extend(crate::env::writable_metas(signer_nodes.iter().copied()));
        Instruction {
            program_id: molpha::ID,
            accounts,
            data: molpha::instruction::ValidateRound {}.data(),
        }
    }

    pub fn claim_node_rewards_ix(&self, owner: &Pubkey) -> Instruction {
        Instruction {
            program_id: molpha::ID,
            accounts: molpha::accounts::ClaimNodeRewards {
                owner: *owner,
                protocol_config: pda::protocol_config(),
                node: pda::node(owner),
                treasury: self.treasury(),
                destination: pda::ata(owner, &self.usdc_mint),
                token_program: pda::TOKEN_PROGRAM_ID,
                associated_token_program: pda::ASSOCIATED_TOKEN_PROGRAM_ID,
                system_program: anchor_lang::solana_program::system_program::ID,
            }
            .to_account_metas(None),
            data: molpha::instruction::ClaimNodeRewards {}.data(),
        }
    }

    pub fn withdraw_node_deposit_ix(&self, owner: &Pubkey) -> Instruction {
        Instruction {
            program_id: molpha::ID,
            accounts: molpha::accounts::WithdrawNodeDeposit {
                owner: *owner,
                protocol_config: pda::protocol_config(),
                node: pda::node(owner),
                node_deposits: pda::node_deposits(),
                node_deposit_vault: self.node_deposit_vault(),
                destination: pda::ata(owner, &self.usdc_mint),
                token_program: pda::TOKEN_PROGRAM_ID,
                associated_token_program: pda::ASSOCIATED_TOKEN_PROGRAM_ID,
                system_program: anchor_lang::solana_program::system_program::ID,
            }
            .to_account_metas(None),
            data: molpha::instruction::WithdrawNodeDeposit {}.data(),
        }
    }
}

// ---------------------------------------------------------------------------
// remaining instruction builders
// ---------------------------------------------------------------------------

impl Protocol {
    pub fn update_plan_ix(&self, authority: &Pubkey, args: &UpdatePlanArgs) -> Instruction {
        Instruction {
            program_id: molpha::ID,
            accounts: molpha::accounts::UpdatePlan {
                authority: *authority,
                protocol_config: pda::protocol_config(),
                plan: pda::plan(args.plan_type),
            }
            .to_account_metas(None),
            data: molpha::instruction::UpdatePlan { args: args.clone() }.data(),
        }
    }

    pub fn extend_subscription_ix(&self, owner: &Pubkey, plan_type: PlanType) -> Instruction {
        Instruction {
            program_id: molpha::ID,
            accounts: molpha::accounts::ExtendSubscription {
                owner: *owner,
                protocol_config: pda::protocol_config(),
                subscription: pda::subscription(owner),
                plan: pda::plan(plan_type),
                owner_usdc: pda::ata(owner, &self.usdc_mint),
                treasury: self.treasury(),
                token_program: pda::TOKEN_PROGRAM_ID,
            }
            .to_account_metas(None),
            data: molpha::instruction::ExtendSubscription {}.data(),
        }
    }

    pub fn remove_delegate_ix(&self, owner: &Pubkey, delegate: &Pubkey) -> Instruction {
        Instruction {
            program_id: molpha::ID,
            accounts: molpha::accounts::RemoveDelegate {
                owner: *owner,
                subscription: pda::subscription(owner),
                delegate_account: pda::delegate(owner, delegate),
            }
            .to_account_metas(None),
            data: molpha::instruction::RemoveDelegate {
                args: RemoveDelegateArgs {
                    delegate: *delegate,
                },
            }
            .data(),
        }
    }

    pub fn set_delegate_paused_ix(
        &self,
        owner: &Pubkey,
        delegate: &Pubkey,
        paused: bool,
    ) -> Instruction {
        Instruction {
            program_id: molpha::ID,
            accounts: molpha::accounts::SetDelegatePaused {
                owner: *owner,
                delegate_account: pda::delegate(owner, delegate),
            }
            .to_account_metas(None),
            data: molpha::instruction::SetDelegatePaused { paused }.data(),
        }
    }

    pub fn update_node_endpoint_ix(
        &self,
        node_owner: &Pubkey,
        ip: [u8; 4],
        port: u16,
    ) -> Instruction {
        Instruction {
            program_id: molpha::ID,
            accounts: molpha::accounts::UpdateNodeEndpoint {
                node_owner: *node_owner,
                node: pda::node(node_owner),
            }
            .to_account_metas(None),
            data: molpha::instruction::UpdateNodeEndpoint {
                args: UpdateNodeEndpointArgs { ip, port },
            }
            .data(),
        }
    }

    pub fn update_registry_policy_ix(
        &self,
        authority: &Pubkey,
        new_registry_version: u32,
        redundancy_buffer: u8,
    ) -> Instruction {
        let current_version = self.current_version();
        Instruction {
            program_id: molpha::ID,
            accounts: molpha::accounts::UpdateRegistryPolicy {
                authority: *authority,
                protocol_config: pda::protocol_config(),
                registry_state: pda::registry_state(),
                current_registry: pda::registry(current_version),
                new_registry: pda::registry(new_registry_version),
                system_program: anchor_lang::solana_program::system_program::ID,
            }
            .to_account_metas(None),
            data: molpha::instruction::UpdateRegistryPolicy { redundancy_buffer }.data(),
        }
    }

    pub fn reinstate_node_ix(&self, caller: &Pubkey, node_owner: &Pubkey) -> Instruction {
        self.reinstate_node_raw_ix(caller, node_owner, self.current_version() + 1)
    }

    /// `reinstate_node` with a caller-chosen target version — for version failure cases.
    pub fn reinstate_node_raw_ix(
        &self,
        caller: &Pubkey,
        node_owner: &Pubkey,
        new_registry_version: u32,
    ) -> Instruction {
        let current_version = self.current_version();
        Instruction {
            program_id: molpha::ID,
            accounts: molpha::accounts::ReinstateNode {
                caller: *caller,
                protocol_config: pda::protocol_config(),
                registry_state: pda::registry_state(),
                current_registry: pda::registry(current_version),
                new_registry: pda::registry(new_registry_version),
                node: pda::node(node_owner),
                system_program: anchor_lang::solana_program::system_program::ID,
            }
            .to_account_metas(None),
            data: molpha::instruction::ReinstateNode.data(),
        }
    }

    /// Node PDAs for the set bits of `bitmap`, in ascending bit order, against `version`.
    pub fn node_pdas_for_bitmap(&self, version: u32, bitmap: &[u8; 32]) -> Vec<Pubkey> {
        let registry = self.env.registry(version);
        crypto::bitmap_bits(bitmap)
            .into_iter()
            .map(|bit| registry.nodes[bit])
            .collect()
    }

    pub fn punish_equivocation_ix(
        &self,
        reporter: &Pubkey,
        args: PunishEquivocationArgs,
    ) -> Instruction {
        self.punish_equivocation_ix_with_registry(reporter, args, self.current_version() + 1)
    }

    pub fn punish_equivocation_ix_with_registry(
        &self,
        reporter: &Pubkey,
        args: PunishEquivocationArgs,
        new_registry_version: u32,
    ) -> Instruction {
        let union = union_bitmap(
            &args.statement_a.signers_bitmap,
            &args.statement_b.signers_bitmap,
        );
        let accused = self.node_pdas_for_bitmap(args.registry_version, &union);
        let mut accounts = molpha::accounts::PunishEquivocation {
            reporter: *reporter,
            protocol_config: pda::protocol_config(),
            registry_state: pda::registry_state(),
            evidence_registry: pda::registry(args.registry_version),
            current_registry: pda::registry(self.current_version()),
            new_registry: pda::registry(new_registry_version),
            punishment_case: pda::punishment_case(&args.evidence_hash),
            node_deposits: pda::node_deposits(),
            node_deposit_vault: self.node_deposit_vault(),
            treasury: self.treasury(),
            reporter_usdc: pda::ata(reporter, &self.usdc_mint),
            token_program: pda::TOKEN_PROGRAM_ID,
            system_program: anchor_lang::solana_program::system_program::ID,
        }
        .to_account_metas(None);
        accounts.extend(crate::env::writable_metas(accused));
        Instruction {
            program_id: molpha::ID,
            accounts,
            data: molpha::instruction::PunishEquivocation { args }.data(),
        }
    }

    pub fn punish_unsolicited_signing_ix(
        &self,
        reporter: &Pubkey,
        args: PunishUnsolicitedSigningArgs,
    ) -> Instruction {
        self.punish_unsolicited_signing_ix_with_registry(reporter, args, self.current_version() + 1)
    }

    pub fn punish_unsolicited_signing_ix_with_registry(
        &self,
        reporter: &Pubkey,
        args: PunishUnsolicitedSigningArgs,
        new_registry_version: u32,
    ) -> Instruction {
        let accused =
            self.node_pdas_for_bitmap(args.registry_version, &args.statement.signers_bitmap);
        let mut accounts = molpha::accounts::PunishUnsolicitedSigning {
            reporter: *reporter,
            protocol_config: pda::protocol_config(),
            registry_state: pda::registry_state(),
            evidence_registry: pda::registry(args.registry_version),
            current_registry: pda::registry(self.current_version()),
            new_registry: pda::registry(new_registry_version),
            punishment_case: pda::punishment_case(&args.evidence_hash),
            node_deposits: pda::node_deposits(),
            node_deposit_vault: self.node_deposit_vault(),
            treasury: self.treasury(),
            reporter_usdc: pda::ata(reporter, &self.usdc_mint),
            token_program: pda::TOKEN_PROGRAM_ID,
            system_program: anchor_lang::solana_program::system_program::ID,
        }
        .to_account_metas(None);
        accounts.extend(crate::env::writable_metas(accused));
        Instruction {
            program_id: molpha::ID,
            accounts,
            data: molpha::instruction::PunishUnsolicitedSigning { args }.data(),
        }
    }

    pub fn punish_nonce_reuse_ix(
        &self,
        reporter: &Pubkey,
        args: PunishNonceReuseArgs,
    ) -> Instruction {
        self.punish_nonce_reuse_ix_with_registry(reporter, args, self.current_version() + 1)
    }

    pub fn punish_nonce_reuse_ix_with_registry(
        &self,
        reporter: &Pubkey,
        args: PunishNonceReuseArgs,
        new_registry_version: u32,
    ) -> Instruction {
        let union = union_bitmap(
            &args.statement_a.signers_bitmap,
            &args.statement_b.signers_bitmap,
        );
        let accused = self.node_pdas_for_bitmap(args.registry_version, &union);
        let mut accounts = molpha::accounts::PunishNonceReuse {
            reporter: *reporter,
            protocol_config: pda::protocol_config(),
            registry_state: pda::registry_state(),
            evidence_registry: pda::registry(args.registry_version),
            current_registry: pda::registry(self.current_version()),
            new_registry: pda::registry(new_registry_version),
            punishment_case: pda::punishment_case(&args.evidence_hash),
            node_deposits: pda::node_deposits(),
            node_deposit_vault: self.node_deposit_vault(),
            treasury: self.treasury(),
            reporter_usdc: pda::ata(reporter, &self.usdc_mint),
            token_program: pda::TOKEN_PROGRAM_ID,
            system_program: anchor_lang::solana_program::system_program::ID,
        }
        .to_account_metas(None);
        accounts.extend(crate::env::writable_metas(accused));
        Instruction {
            program_id: molpha::ID,
            accounts,
            data: molpha::instruction::PunishNonceReuse { args }.data(),
        }
    }
}

/// The guilty set of an unsolicited-signing filing: signers outside the derived selection.
pub fn unsolicited_guilty_bitmap(p: &Protocol, args: &PunishUnsolicitedSigningArgs) -> [u8; 32] {
    let registry = p.env.registry(args.registry_version);
    let selection = molpha_verifier::derive_selection_bitmap(
        &args.source_id,
        args.registry_version,
        args.canonical_timestamp,
        u32::from(registry.node_count),
        args.signatures_required,
        registry.redundancy_buffer,
    )
    .expect("selection derives");
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = args.statement.signers_bitmap[i] & !selection[i];
    }
    out
}

/// Bitwise union of two 256-bit signer bitmaps.
pub fn union_bitmap(a: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = a[i] | b[i];
    }
    out
}

/// Bitwise intersection — the guilty set for equivocation and nonce reuse.
pub fn intersect_bitmap(a: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = a[i] & b[i];
    }
    out
}

impl Protocol {
    /// Rewrite a `Node` account in place.
    ///
    /// The only supported use is producing `NodeStatus::Frozen`, which `reinstate_node` handles
    /// but no shipped penalty currently produces (every offence tombstones). Without this the
    /// freeze branch would be untestable, and untested, until the first penalty table change.
    ///
    /// Writing `locked_amount`, `claimable_rewards` or `pending_seizure` here desynchronises the
    /// node from its deposit vault and from `ProtocolConfig.reward_liability`, which the program
    /// maintains but cannot repair. Fund the vault to match if you move the deposit fields. [`assert_solvent`](Self::assert_solvent) will then fail, and a later claim
    /// or punishment can underflow the liability. Move money through the instructions instead.
    pub fn overwrite_node<F: FnOnce(&mut molpha::state::Node)>(
        &mut self,
        node_pda: &Pubkey,
        edit: F,
    ) {
        use anchor_lang::{AccountSerialize, Discriminator};

        let mut node: molpha::state::Node = self.env.get(node_pda);
        edit(&mut node);

        let mut data = Vec::with_capacity(molpha::state::Node::SPACE);
        node.try_serialize(&mut data).expect("serialize node");
        assert_eq!(&data[..8], molpha::state::Node::DISCRIMINATOR);
        data.resize(molpha::state::Node::SPACE, 0);
        self.env.overwrite_account(node_pda, data);
    }

    /// Rewrite `ProtocolConfig` in place, for setting up states no instruction produces.
    pub fn overwrite_protocol_config<F: FnOnce(&mut molpha::state::ProtocolConfig)>(
        &mut self,
        edit: F,
    ) {
        use anchor_lang::{AccountSerialize, Discriminator};

        let key = pda::protocol_config();
        let mut config: molpha::state::ProtocolConfig = self.env.get(&key);
        edit(&mut config);

        let mut data = Vec::with_capacity(molpha::state::ProtocolConfig::SPACE);
        config.try_serialize(&mut data).expect("serialize config");
        assert_eq!(&data[..8], molpha::state::ProtocolConfig::DISCRIMINATOR);
        data.resize(molpha::state::ProtocolConfig::SPACE, 0);
        self.env.overwrite_account(&key, data);
    }

    /// Take a node out of the registry and freeze it until `frozen_until`.
    pub fn freeze_node(&mut self, index: usize, frozen_until: i64) {
        let owner = self.nodes[index].owner.insecure_clone();
        let ix = self.deactivate_node_ix(index, &owner.pubkey());
        self.env.send_ok(&[ix], &[&owner]);

        let node_pda = self.nodes[index].pda;
        self.overwrite_node(&node_pda, |node| {
            node.status = molpha::state::NodeStatus::Frozen;
            node.frozen_until = frozen_until;
        });
    }
}
