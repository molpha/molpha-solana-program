//! Composable setup: an initialized protocol, registered nodes, gateways, subscriptions, and
//! signed attestations.
//!
//! Every builder returns owned state, so a test can take a scenario as far as it needs without
//! reaching into a shared fixture.

use anchor_lang::prelude::Pubkey;
use anchor_lang::solana_program::instruction::Instruction;
use anchor_lang::{Discriminator, InstructionData, ToAccountMetas};
use litesvm::types::TransactionMetadata;
use molpha::instructions::{
    AddDelegateArgs, InitializeArgs, MemberEndpoint, RegisterGatewayArgs, RegisterNodeArgs,
    RemoveDelegateArgs, SubmitAttestationArgs, UpdateConfigArgs,
};
use molpha::state::{
    CommitmentReuseEvidence, EpochState, EquivocationEvidence, InactivityEvidence,
    NonceReuseEvidence, Offence, PlanConfig, PlanType, ProtocolConfig, Provider, RegistryState,
    UnsolicitedSigningEvidence, MAX_REGISTRY_NODES,
};
use molpha_verifier::{
    Attestation, AttestationPayload, CoalitionKey, KeyRotationProof, SchnorrSignature,
};
use solana_keypair::Keypair;
use solana_signer::Signer;

use crate::crypto::{self, AggregateSignature, NodeKey};
use crate::env::TestEnv;
use crate::pda;

/// `register_node` args for `key` held by `owner`, with the node proof of possession over the
/// node id. A node and a provider prove possession the same way.
pub fn member_args(
    owner: &Pubkey,
    key: &NodeKey,
    salt: [u8; 32],
    endpoint: MemberEndpoint,
) -> RegisterNodeArgs {
    let node_id = crypto::derive_node_id(owner, &key.compressed, &salt);
    let nonce = solana_keccak_hasher::hashv(&[b"molpha-test-pop-nonce".as_ref(), node_id.as_ref()])
        .to_bytes();
    let (pop_sig_r, pop_sig_s) = crypto::sign_pop(&molpha::ID, &node_id, key, &nonce);
    RegisterNodeArgs {
        registration_salt: salt,
        endpoint,
        secp256k1_pubkey_compressed: key.compressed,
        pop_sig_r,
        pop_sig_s,
    }
}

/// Placeholder key for signer sets without a sum; every verifying instruction rejects it.
pub const ZERO_COALITION_KEY: CoalitionKey = CoalitionKey {
    x: [0; 32],
    y: [0; 32],
};

/// A node that completed `register_node`.
pub struct RegisteredNode {
    pub owner: Keypair,
    pub key: NodeKey,
    pub pda: Pubkey,
    pub salt: [u8; 32],
    /// Slot in the registry snapshot created by its own registration: the lowest hole, or the
    /// slot after the last. It stays the node's slot in every later version.
    pub index: u16,
    /// Keys this node signed with before `key`, oldest first. Older registry versions still name
    /// them, so signing under those versions needs their secrets.
    pub retired_keys: Vec<NodeKey>,
}

impl RegisteredNode {
    /// The key (current or retired) whose public half is `xy`.
    pub fn key_for(&self, xy: &([u8; 32], [u8; 32])) -> Option<&NodeKey> {
        std::iter::once(&self.key)
            .chain(&self.retired_keys)
            .find(|key| (key.x, key.y) == *xy)
    }
}

/// A provider signer that completed `register_provider`.
pub struct RegisteredProvider {
    pub authority: Keypair,
    pub key: NodeKey,
    /// The ed25519 libp2p identity it registered.
    pub p2p_key: [u8; 32],
    pub id: u32,
    pub pda: Pubkey,
    /// Provider slot `p` (bit `64 + p`) once activated.
    pub slot: Option<usize>,
    /// Keys rotated away from, oldest first.
    pub retired_keys: Vec<NodeKey>,
}

/// A provider registration built by [`Protocol::prepare_provider`] and not yet sent.
pub struct PendingProvider {
    pub authority: Keypair,
    pub key: NodeKey,
    pub p2p_key: [u8; 32],
    pub id: u32,
    /// `register_node` with the provider account.
    pub ix: Instruction,
}

impl RegisteredProvider {
    /// The key (current or retired) whose public half is `xy`.
    pub fn key_for(&self, xy: &([u8; 32], [u8; 32])) -> Option<&NodeKey> {
        std::iter::once(&self.key)
            .chain(&self.retired_keys)
            .find(|key| (key.x, key.y) == *xy)
    }

    /// The bit this provider signs with, once activated.
    pub fn bit(&self) -> usize {
        molpha_verifier::PROVIDER_BIT_BASE + self.slot.expect("provider is active")
    }
}

/// The duty a round attests to — `(source_id, version, timestamp, quorum)` plus the value.
///
/// `timestamp` is unix MILLISECONDS, as on the wire. `at` takes milliseconds,
/// `at_secs` whole seconds, and `secs` reads it back in seconds (the unit of the cluster clock).
#[derive(Clone, Copy, Debug)]
pub struct Duty {
    pub source_id: [u8; 32],
    pub registry_version: u32,
    pub timestamp: u64,
    pub signatures_required: u8,
    pub value: [u8; 32],
}

impl Duty {
    pub fn new(registry_version: u32, signatures_required: u8) -> Self {
        Self {
            source_id: [0x11; 32],
            registry_version,
            timestamp: 1_700_000_000_000,
            signatures_required,
            value: [0x42; 32],
        }
    }

    pub fn source_id(mut self, source_id: [u8; 32]) -> Self {
        self.source_id = source_id;
        self
    }

    /// Stamp the duty at `timestamp` unix milliseconds.
    pub fn at(mut self, timestamp: u64) -> Self {
        self.timestamp = timestamp;
        self
    }

    /// Stamp the duty at the start of unix second `seconds`.
    pub fn at_secs(self, seconds: u64) -> Self {
        self.at(seconds * 1_000)
    }

    /// The timestamp in whole unix seconds.
    pub fn secs(&self) -> u64 {
        self.timestamp / 1_000
    }

    pub fn value(mut self, value: [u8; 32]) -> Self {
        self.value = value;
        self
    }

    pub fn payload(&self) -> AttestationPayload {
        AttestationPayload {
            value: self.value,
            source_id: self.source_id,
            registry_version: self.registry_version,
            signatures_required: self.signatures_required,
            timestamp: self.timestamp,
        }
    }

    /// The message a coalition signs for this duty.
    pub fn message_hash(&self, signers_bitmap: [u8; 32]) -> [u8; 32] {
        molpha_verifier::compute_message_hash(&self.payload(), signers_bitmap)
    }
}

/// A duty plus a coalition signature over it.
#[derive(Clone, Debug)]
pub struct SignedAttestation {
    pub duty: Duty,
    pub signers_bitmap: [u8; 32],
    pub message_hash: [u8; 32],
    pub signature: AggregateSignature,
    /// Node PDAs for the set bits, in ascending bit order. Verification reads signer keys from
    /// the registry snapshot, so these are not passed as accounts.
    pub signer_pdas: Vec<Pubkey>,
    /// Affine sum of the signers' keys, as every verifying instruction takes it (zero if
    /// undefined, e.g. an empty set).
    pub coalition_key: CoalitionKey,
}

impl SignedAttestation {
    /// The instruction-argument form: payload plus aggregate signature.
    pub fn attestation(&self) -> Attestation {
        Attestation {
            payload: self.duty.payload(),
            signature: SchnorrSignature {
                agg_sig_s: self.signature.agg_sig_s,
                commitment: self.signature.commitment,
                signers_bitmap: self.signers_bitmap,
            },
        }
    }

    pub fn submit_args(&self) -> SubmitAttestationArgs {
        SubmitAttestationArgs {
            attestation: self.attestation(),
            raw_value: None,
            coalition_key: self.coalition_key,
        }
    }

    /// A copy whose signature no longer verifies, with the committed shape left intact.
    pub fn with_tampered_signature(&self) -> Self {
        let mut tampered = self.clone();
        tampered.signature.agg_sig_s[31] ^= 0x01;
        tampered
    }
}

/// An environment with a funded authority and a USDC mint, but no `initialize` yet.
///
/// The starting point for tests about `initialize` itself; [`ProtocolBuilder::start`] builds on it.
pub struct Bare {
    pub env: TestEnv,
    /// The program's upgrade authority, which is the only key `initialize` accepts.
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
    env.set_upgrade_authority(Some(authority.pubkey()));
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
        max_settlement_delay_seconds: 604_800,
        challenger_bounty_bps: 500,
        usdc_mint,
        x402_round_base: 1_000_000,
        min_signers: 3,
        epoch_len_seconds: 86_400,
        ticket_grace_seconds: 21_600,
        target_tickets_per_epoch: 2_000,
        min_availability_bps: 8_000,
        min_availability_samples: 50,
        protocol_fee_bps: 500,
        // Sample every round: most tests want each served round to be a ticket.
        initial_tau: u64::MAX,
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
            node_deposit_vault: pda::ata(&pda::registry_state(), usdc_mint_account),
            epoch0: pda::epoch(0),
            epoch1: pda::epoch(1),
            authority: *authority,
            program: molpha::ID,
            program_data: pda::program_data(),
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

    pub fn reward_per_signature(mut self, value: u64) -> Self {
        self.args.reward_per_signature = value;
        self
    }

    pub fn challenger_bounty_bps(mut self, value: u16) -> Self {
        self.args.challenger_bounty_bps = value;
        self
    }

    pub fn epoch_len_seconds(mut self, value: u64) -> Self {
        self.args.epoch_len_seconds = value;
        self
    }

    pub fn ticket_grace_seconds(mut self, value: u64) -> Self {
        self.args.ticket_grace_seconds = value;
        self
    }

    pub fn target_tickets_per_epoch(mut self, value: u32) -> Self {
        self.args.target_tickets_per_epoch = value;
        self
    }

    pub fn min_availability_bps(mut self, value: u16) -> Self {
        self.args.min_availability_bps = value;
        self
    }

    pub fn min_availability_samples(mut self, value: u16) -> Self {
        self.args.min_availability_samples = value;
        self
    }

    pub fn initial_tau(mut self, value: u64) -> Self {
        self.args.initial_tau = value;
        self
    }

    pub fn protocol_fee_bps(mut self, value: u16) -> Self {
        self.args.protocol_fee_bps = value;
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

    pub fn max_settlement_delay_seconds(mut self, value: u64) -> Self {
        self.args.max_settlement_delay_seconds = value;
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
            providers: Vec::new(),
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
    pub providers: Vec<RegisteredProvider>,
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
        pda::ata(&pda::registry_state(), &self.usdc_mint)
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

    pub fn config(&self) -> ProtocolConfig {
        self.env.get::<ProtocolConfig>(&pda::protocol_config())
    }

    /// `Σ Node.claimable_rewards` as the protocol tracks it.
    pub fn reward_liability(&self) -> u64 {
        self.config().reward_liability
    }

    /// Epoch accounts that exist (created and not yet closed), ascending.
    pub fn live_epochs(&self) -> Vec<u64> {
        let mut epochs: Vec<_> = self
            .env
            .svm
            .get_program_accounts(&molpha::ID)
            .into_iter()
            .filter(|(_, account)| {
                account.lamports > 0
                    && account.data.len() == EpochState::SPACE
                    && account.data.starts_with(EpochState::DISCRIMINATOR)
            })
            .map(|(_, account)| u64::from_le_bytes(account.data[8..16].try_into().unwrap()))
            .collect();
        epochs.sort_unstable();
        epochs
    }

    /// Assert the custody invariants the treasury separation exists to guarantee.
    ///
    /// Cheap enough to call at the end of any test that moves money, and the fastest way to
    /// notice a payout path that broke principal custody, reward accounting or pool accounting.
    pub fn assert_solvent(&self) {
        // Enumerate the VM, not the fixture's actor list: raw instruction builders can create
        // nodes without adding a RegisteredNode, and long campaigns pass epoch 64.
        let nodes: Vec<molpha::state::Node> = self
            .env
            .svm
            .get_program_accounts(&molpha::ID)
            .into_iter()
            .filter(|(_, account)| {
                account.lamports > 0
                    && account.data.len() == molpha::state::Node::SPACE
                    && account.data.starts_with(molpha::state::Node::DISCRIMINATOR)
            })
            .map(|(key, _)| self.env.get(&key))
            .collect();
        let locked: u128 = nodes
            .iter()
            .map(|node| u128::from(node.locked_amount))
            .sum();
        assert!(
            u128::from(self.node_deposit_balance()) >= locked,
            "deposit vault {} cannot cover the {} it holds for nodes",
            self.node_deposit_balance(),
            locked
        );

        let owed_rewards: u128 = nodes
            .iter()
            .map(|node| u128::from(node.claimable_rewards))
            .sum();
        assert_eq!(
            u128::from(self.reward_liability()),
            owed_rewards,
            "reward_liability is not the sum of every claimable_rewards"
        );
        let config = self.config();
        let live_pools: u128 = self
            .live_epochs()
            .into_iter()
            .map(|epoch| {
                let state = self.env.epoch(epoch);
                assert!(
                    state.paid <= state.pool,
                    "epoch {epoch} paid more than its pool"
                );
                let len = usize::from(state.roster_len);
                assert!(len <= molpha::state::MAX_REGISTRY_NODES, "roster overflow");
                let selected: u32 = state.selected[..len].iter().map(|n| u32::from(*n)).sum();
                let signed: u32 = state.signed[..len].iter().map(|n| u32::from(*n)).sum();
                assert_eq!(selected, state.sum_selected, "epoch {epoch} selection sum");
                assert_eq!(signed, state.sum_signed, "epoch {epoch} signature sum");
                for slot in 0..len {
                    assert!(
                        state.signed[slot] <= state.selected[slot],
                        "signed without selection"
                    );
                    assert!(
                        u32::from(state.selected[slot]) <= state.tickets,
                        "duplicate selection"
                    );
                }
                u128::from(state.pool - state.paid)
            })
            .sum();
        assert_eq!(
            u128::from(config.pool_liability),
            live_pools,
            "pool_liability is not the sum of every live epoch's pool - paid"
        );
        let committed = config.committed().expect("committed fits u64");
        assert!(
            self.treasury_balance() >= committed,
            "treasury {} cannot back the {} it owes (rewards + pools + reserve)",
            self.treasury_balance(),
            committed
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
                node: Some(pda::node(owner)),
                provider: None,
                key_x_claim: pda::key_x_claim_for(&args.secp256k1_pubkey_compressed),
                owner_usdc: Some(pda::ata(owner, &self.usdc_mint)),
                node_deposit_vault: Some(self.node_deposit_vault()),
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
        member_args(
            owner,
            key,
            salt,
            MemberEndpoint::Node {
                ip: [127, 0, 0, 1],
                port,
            },
        )
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
        let members = self.current_members() + 1;
        let mut ixs = self.registry_extend_ixs(&owner.pubkey(), members);
        ixs.push(self.register_node_ix(&owner.pubkey(), &args, &authority.pubkey()));
        self.env.send_ok(&ixs, &[&authority, &owner]);

        let pda = pda::node(&owner.pubkey());
        let slot = self
            .env
            .registry(self.current_version())
            .index_of(&pda)
            .expect("a registered node holds a slot");
        self.nodes.push(RegisteredNode {
            index: slot,
            owner,
            key,
            pda,
            salt,
            retired_keys: Vec::new(),
        });
    }

    pub fn deactivate_node_ix(&self, index: usize, caller: &Pubkey) -> Instruction {
        let node = &self.nodes[index];
        self.deactivate_node_raw_ix(caller, &node.owner.pubkey())
    }

    /// `deactivate_node` with a caller-chosen owner — for repeat and failure cases.
    pub fn deactivate_node_raw_ix(&self, caller: &Pubkey, node_owner: &Pubkey) -> Instruction {
        let current_version = self.current_version();
        Instruction {
            program_id: molpha::ID,
            accounts: molpha::accounts::DeactivateNode {
                authority: *caller,
                protocol_config: pda::protocol_config(),
                registry_state: pda::registry_state(),
                current_registry: pda::registry(current_version),
                new_registry: pda::registry(current_version + 1),
                node: Some(pda::node(node_owner)),
                provider: None,
                system_program: anchor_lang::solana_program::system_program::ID,
            }
            .to_account_metas(None),
            data: molpha::instruction::DeactivateNode {
                node_owner: *node_owner,
            }
            .data(),
        }
    }

    /// Records of the current registry version, node and provider slots, holes included.
    pub fn current_members(&self) -> usize {
        self.env.registry(self.current_version()).records()
    }

    /// `extend_registry`: grow the next registry version's account by up to 10 KiB, toward room
    /// for one record more than the current version has.
    pub fn extend_registry_ix(&self, payer: &Pubkey) -> Instruction {
        self.extend_registry_for_ix(payer, 0)
    }

    /// `extend_registry` toward room for `members` records (or one more than the current
    /// version has, if that is larger).
    pub fn extend_registry_for_ix(&self, payer: &Pubkey, members: usize) -> Instruction {
        let current_version = self.current_version();
        Instruction {
            program_id: molpha::ID,
            accounts: molpha::accounts::ExtendRegistry {
                payer: *payer,
                registry_state: pda::registry_state(),
                current_registry: pda::registry(current_version),
                new_registry: pda::registry(current_version + 1),
                system_program: anchor_lang::solana_program::system_program::ID,
            }
            .to_account_metas(None),
            data: molpha::instruction::ExtendRegistry {
                members: u16::try_from(members).unwrap(),
            }
            .data(),
        }
    }

    /// How many `extend_registry` calls must precede an instruction that publishes a version
    /// needing room for `members` members.
    ///
    /// That instruction adds up to 10 KiB itself, so a snapshot of up to 106 members needs none.
    /// Whatever is already allocated for the next version counts.
    pub fn registry_extends_needed(&self, members: usize) -> usize {
        let next = pda::registry(self.current_version() + 1);
        let allocated = if self.env.account_exists(&next) {
            self.env.account_data(&next).len()
        } else {
            0
        };
        let short = molpha::state::Registry::space(members).saturating_sub(allocated);
        short.saturating_sub(1) / molpha::utils::registry_account::MAX_GROWTH_PER_INSTRUCTION
    }

    /// The `extend_registry` instructions to put in front of one that publishes a version with
    /// room for `members` members. Empty for a registry of up to 106.
    pub fn registry_extend_ixs(&self, payer: &Pubkey, members: usize) -> Vec<Instruction> {
        (0..self.registry_extends_needed(members))
            .map(|_| self.extend_registry_for_ix(payer, members))
            .collect()
    }

    /// The proof that rotates node `index` to `new_key`, signed with the key it holds now.
    pub fn key_rotation_proof(&self, index: usize, new_key: &NodeKey) -> KeyRotationProof {
        let node = &self.nodes[index];
        crypto::key_rotation_proof(&molpha::ID, &node.pda, &node.key, new_key)
    }

    pub fn rotate_node_key_ix(&self, index: usize, proof: KeyRotationProof) -> Instruction {
        self.rotate_node_key_raw_ix(
            &self.nodes[index].owner.pubkey(),
            proof,
            self.current_version() + 1,
        )
    }

    /// `rotate_node_key` with a caller-chosen owner and target version — for failure cases.
    pub fn rotate_node_key_raw_ix(
        &self,
        owner: &Pubkey,
        proof: KeyRotationProof,
        new_registry_version: u32,
    ) -> Instruction {
        let current_version = self.current_version();
        Instruction {
            program_id: molpha::ID,
            accounts: molpha::accounts::RotateNodeKey {
                node_owner: *owner,
                protocol_config: pda::protocol_config(),
                registry_state: pda::registry_state(),
                current_registry: pda::registry(current_version),
                new_registry: pda::registry(new_registry_version),
                node: Some(pda::node(owner)),
                provider: None,
                key_x_claim: pda::key_x_claim_for(&proof.new_public_key_compressed),
                system_program: anchor_lang::solana_program::system_program::ID,
            }
            .to_account_metas(None),
            data: molpha::instruction::RotateNodeKey { proof }.data(),
        }
    }

    /// Rotate node `index` to `new_key`, signed by its owner and its current key. The node's
    /// previous key is kept so attestations under older registry versions can still be signed.
    pub fn rotate_node_key(&mut self, index: usize, new_key: NodeKey) -> TransactionMetadata {
        let proof = self.key_rotation_proof(index, &new_key);
        let owner = self.nodes[index].owner.insecure_clone();
        let mut ixs = self.registry_extend_ixs(&owner.pubkey(), self.current_members());
        ixs.push(self.rotate_node_key_ix(index, proof));
        let meta = self.env.send_ok(&ixs, &[&owner]);
        let node = &mut self.nodes[index];
        let retired = std::mem::replace(&mut node.key, new_key);
        node.retired_keys.push(retired);
        meta
    }

    // -----------------------------------------------------------------------
    // plans, subscriptions, delegates
    // -----------------------------------------------------------------------

    pub fn add_plan(&mut self, config: PlanConfig) -> TransactionMetadata {
        let authority = self.authority.insecure_clone();
        let ix = self.add_plan_ix(&config, &authority.pubkey());
        self.env.send_ok(&[ix], &[&authority])
    }

    pub fn add_plan_ix(&self, config: &PlanConfig, authority: &Pubkey) -> Instruction {
        Instruction {
            program_id: molpha::ID,
            accounts: molpha::accounts::AddPlan {
                protocol_config: pda::protocol_config(),
                plan: pda::plan(config.plan_type),
                authority: *authority,
                system_program: anchor_lang::solana_program::system_program::ID,
            }
            .to_account_metas(None),
            data: molpha::instruction::AddPlan {
                config: config.clone(),
            }
            .data(),
        }
    }

    /// A plan with generous limits, for tests that are not about plan limits.
    pub fn add_default_plan(&mut self, plan_type: PlanType) -> PlanConfig {
        let config = PlanConfig {
            plan_type,
            subscription_price: 50_000_000,
            max_signers: 16,
            max_delegates: 4,
            max_rounds: 100,
            private_api_enabled: true,
            is_active: true,
        };
        self.add_plan(config.clone());
        config
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

    pub fn add_delegate(&mut self, owner: &Keypair, delegate: &Pubkey, max_data_requests: u64) {
        let args = AddDelegateArgs {
            delegate: *delegate,
            max_data_requests,
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
        gateway_authority: &Pubkey,
        args: &RegisterGatewayArgs,
    ) -> Instruction {
        Instruction {
            program_id: molpha::ID,
            accounts: molpha::accounts::RegisterGateway {
                authority: self.authority.pubkey(),
                protocol_config: pda::protocol_config(),
                gateway: pda::gateway(gateway_authority),
                system_program: anchor_lang::solana_program::system_program::ID,
            }
            .to_account_metas(None),
            data: molpha::instruction::RegisterGateway { args: args.clone() }.data(),
        }
    }

    /// A registered gateway. Its authority signs round dispatches.
    pub fn register_gateway(&mut self) -> Keypair {
        let gateway_authority = self.env.keypair();
        let args = RegisterGatewayArgs {
            authority: gateway_authority.pubkey(),
            ip: [10, 0, 0, 7],
            port: 9_000,
        };
        let ix = self.register_gateway_ix(&gateway_authority.pubkey(), &args);
        let protocol_authority = self.authority.insecure_clone();
        self.env.send_ok(&[ix], &[&protocol_authority]);
        gateway_authority
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

    // -----------------------------------------------------------------------
    // attestations
    // -----------------------------------------------------------------------

    /// The deterministic selection set for `duty` against the registry it names.
    pub fn selection_bits(&self, duty: &Duty) -> Vec<usize> {
        let registry = self.env.registry(duty.registry_version);
        let bitmap = molpha_verifier::derive_selection_bitmap(
            &duty.source_id,
            duty.timestamp,
            duty.signatures_required,
            &registry.selection_view(),
        )
        .expect("selection derives");
        crypto::bitmap_bits(&bitmap.to_bytes())
    }

    /// The selection set of a **provider round** of `duty`: the committee of
    /// `signatures_required - 1` nodes plus the buffer. The draw is not nested in the committee
    /// round's, so a provider round is judged by this one.
    pub fn provider_selection_bits(&self, duty: &Duty) -> Vec<usize> {
        let registry = self.env.registry(duty.registry_version);
        let bitmap = molpha_verifier::derive_selection_bitmap(
            &duty.source_id,
            duty.timestamp,
            duty.signatures_required - 1,
            &registry.selection_view(),
        )
        .expect("selection derives");
        crypto::bitmap_bits(&bitmap.to_bytes())
    }

    /// The selection set for the round that produced `signers_bitmap`: a provider round (a
    /// provider bit set) is drawn by its node quorum, any other by `signatures_required`.
    pub fn selection_bits_for(&self, duty: &Duty, signers_bitmap: &[u8; 32]) -> Vec<usize> {
        if molpha_verifier::Bitmap::load(signers_bitmap).provider_bits()
            == molpha_verifier::Bitmap::EMPTY
        {
            self.selection_bits(duty)
        } else {
            self.provider_selection_bits(duty)
        }
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
        // The key a node signs with is the one the snapshot names for it, which differs from its
        // current key once it has rotated.
        let signer_keys: Vec<&NodeKey> = bits
            .iter()
            .map(|bit| {
                let account = registry.nodes[*bit];
                self.nodes
                    .iter()
                    .find(|n| n.pda == account)
                    .and_then(|n| n.key_for(&registry.keys[*bit]))
                    .or_else(|| {
                        // A provider bit: the key the version names, current or retired.
                        self.providers
                            .iter()
                            .find(|p| p.pda == account)
                            .and_then(|p| p.key_for(&registry.keys[*bit]))
                    })
                    .unwrap_or_else(|| panic!("no key material for registry bit {bit}"))
            })
            .collect();
        let signer_pdas = bits.iter().map(|bit| registry.nodes[*bit]).collect();

        let signers_bitmap = crypto::bitmap_from_bits(bits);
        let message_hash = duty.message_hash(signers_bitmap);
        let signature = crypto::sign_aggregate(&signer_keys, &message_hash, nonce);

        let signer_xy: Vec<_> = signer_keys.iter().map(|key| (key.x, key.y)).collect();
        let coalition_key =
            molpha_verifier::coalition_key(&signer_xy).unwrap_or(ZERO_COALITION_KEY);

        SignedAttestation {
            duty,
            signers_bitmap,
            message_hash,
            signature,
            signer_pdas,
            coalition_key,
        }
    }

    /// The coalition key a client derives for `attestation`: the sum of its signers' keys, read
    /// from the registry snapshot the attestation names. Zero when a signer bit is outside the
    /// snapshot (malformed-bitmap tests), so those still fail on the check they target.
    pub fn coalition_key_for(&self, attestation: &Attestation) -> CoalitionKey {
        let registry = self.env.registry(attestation.payload.registry_version);
        let bits = crypto::bitmap_bits(&attestation.signature.signers_bitmap);
        if bits.iter().any(|bit| *bit >= registry.keys.len()) {
            return ZERO_COALITION_KEY;
        }
        let signer_xy: Vec<_> = bits.iter().map(|bit| registry.keys[*bit]).collect();
        molpha_verifier::coalition_key(&signer_xy).unwrap_or(ZERO_COALITION_KEY)
    }

    pub fn submit_attestation_ix(
        &self,
        submitter: &Pubkey,
        attestation: &SignedAttestation,
    ) -> Instruction {
        self.submit_attestation_ix_with_args(submitter, attestation.submit_args())
    }

    /// `submit_attestation` with caller-built args — for raw values and tampered payloads.
    pub fn submit_attestation_ix_with_args(
        &self,
        submitter: &Pubkey,
        args: SubmitAttestationArgs,
    ) -> Instruction {
        let payload = &args.attestation.payload;
        let accounts = molpha::accounts::SubmitAttestation {
            submitter: *submitter,
            registry: pda::registry(payload.registry_version),
            feed: pda::feed(&payload.source_id, payload.signatures_required, submitter),
            protocol_config: pda::protocol_config(),
            system_program: anchor_lang::solana_program::system_program::ID,
        }
        .to_account_metas(None);
        Instruction {
            program_id: molpha::ID,
            accounts,
            data: molpha::instruction::SubmitAttestation { args }.data(),
        }
    }

    // -----------------------------------------------------------------------
    // node rewards
    // -----------------------------------------------------------------------

    pub fn claim_node_rewards_ix(&self, owner: &Pubkey) -> Instruction {
        Instruction {
            program_id: molpha::ID,
            accounts: molpha::accounts::ClaimNodeRewards {
                authority: *owner,
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
                authority: *owner,
                protocol_config: pda::protocol_config(),
                node: pda::node(owner),
                registry_state: pda::registry_state(),
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
    pub fn update_plan_ix(&self, authority: &Pubkey, config: &PlanConfig) -> Instruction {
        Instruction {
            program_id: molpha::ID,
            accounts: molpha::accounts::UpdatePlan {
                authority: *authority,
                protocol_config: pda::protocol_config(),
                plan: pda::plan(config.plan_type),
            }
            .to_account_metas(None),
            data: molpha::instruction::UpdatePlan {
                config: config.clone(),
            }
            .data(),
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

    pub fn update_node_endpoint_ix(
        &self,
        node_owner: &Pubkey,
        ip: [u8; 4],
        port: u16,
    ) -> Instruction {
        self.update_endpoint_raw_ix(
            node_owner,
            Some(pda::node(node_owner)),
            None,
            MemberEndpoint::Node { ip, port },
        )
    }

    /// `update_node_endpoint` with caller-chosen accounts and endpoint: for a node, a provider,
    /// and the mismatch cases.
    pub fn update_endpoint_raw_ix(
        &self,
        signer: &Pubkey,
        node: Option<Pubkey>,
        provider: Option<Pubkey>,
        endpoint: MemberEndpoint,
    ) -> Instruction {
        Instruction {
            program_id: molpha::ID,
            accounts: molpha::accounts::UpdateNodeEndpoint {
                node_owner: *signer,
                node,
                provider,
            }
            .to_account_metas(None),
            data: molpha::instruction::UpdateNodeEndpoint { endpoint }.data(),
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

    pub fn slash_offence_ix(&self, reporter: &Pubkey, offence: Offence) -> Instruction {
        self.slash_offence_ix_with_registry(reporter, offence, self.current_version() + 1)
    }

    pub fn slash_offence_ix_with_registry(
        &self,
        reporter: &Pubkey,
        offence: Offence,
        new_registry_version: u32,
    ) -> Instruction {
        // The accounts are the guilty nodes: the signers both attestations share, or the
        // signers outside the selection. Their keys come from the evidence registry.
        let bitmap = match &offence {
            // Guilty sets are nodes only: a provider bit both attestations share is not punished.
            Offence::Equivocation(e) => node_part(&intersect_bitmap(
                &e.attestation_a.signature.signers_bitmap,
                &e.attestation_b.signature.signers_bitmap,
            )),
            Offence::CommitmentReuse(e) => node_part(&intersect_bitmap(
                &e.attestation_a.signature.signers_bitmap,
                &e.attestation_b.signature.signers_bitmap,
            )),
            Offence::UnsolicitedSigning(e) => unsolicited_guilty_bitmap(self, &e.attestation),
            Offence::NonceReuse(e) => crypto::bitmap_from_bits(&[e.node_index as usize]),
            Offence::Inactivity(e) => crypto::bitmap_from_bits(&[e.node_index as usize]),
        };
        let inactivity_epoch = match &offence {
            Offence::Inactivity(e) => Some(pda::epoch(e.epoch)),
            _ => None,
        };
        let mut accounts = molpha::accounts::SlashOffence {
            reporter: *reporter,
            protocol_config: pda::protocol_config(),
            registry_state: pda::registry_state(),
            evidence_registry: pda::registry(offence.registry_version()),
            current_registry: pda::registry(self.current_version()),
            new_registry: pda::registry(new_registry_version),
            punishment_case: pda::punishment_case(&offence.hash()),
            node_deposit_vault: self.node_deposit_vault(),
            treasury: self.treasury(),
            reporter_usdc: pda::ata(reporter, &self.usdc_mint),
            token_program: pda::TOKEN_PROGRAM_ID,
            system_program: anchor_lang::solana_program::system_program::ID,
        }
        .to_account_metas(None);
        accounts.extend(crate::env::writable_metas(
            self.node_pdas_for_bitmap(offence.registry_version(), &bitmap),
        ));
        accounts.extend(crate::env::readonly_metas(inactivity_epoch));
        Instruction {
            program_id: molpha::ID,
            accounts,
            data: molpha::instruction::SlashOffence { offence }.data(),
        }
    }

    /// Inactivity evidence for `node_pda` in finalized `epoch`, located in the current registry.
    pub fn inactivity_evidence(&self, epoch: u64, node_pda: &Pubkey) -> InactivityEvidence {
        let version = self.current_version();
        InactivityEvidence {
            epoch,
            roster_slot: self
                .roster_slot_of(epoch, node_pda)
                .expect("node is on the epoch roster"),
            registry_version: version,
            node_index: u32::from(
                self.env
                    .registry(version)
                    .index_of(node_pda)
                    .expect("node is in the current registry"),
            ),
        }
    }

    pub fn slash_inactivity_ix(
        &self,
        reporter: &Pubkey,
        epoch: u64,
        node_pda: &Pubkey,
    ) -> Instruction {
        self.slash_offence_ix(
            reporter,
            Offence::Inactivity(self.inactivity_evidence(epoch, node_pda)),
        )
    }

    pub fn slash_equivocation_ix(
        &self,
        reporter: &Pubkey,
        evidence: EquivocationEvidence,
    ) -> Instruction {
        self.slash_offence_ix(reporter, Offence::Equivocation(evidence))
    }

    pub fn slash_equivocation_ix_with_registry(
        &self,
        reporter: &Pubkey,
        evidence: EquivocationEvidence,
        new_registry_version: u32,
    ) -> Instruction {
        self.slash_offence_ix_with_registry(
            reporter,
            Offence::Equivocation(evidence),
            new_registry_version,
        )
    }

    pub fn slash_unsolicited_signing_ix(
        &self,
        reporter: &Pubkey,
        evidence: UnsolicitedSigningEvidence,
    ) -> Instruction {
        self.slash_offence_ix(reporter, Offence::UnsolicitedSigning(evidence))
    }

    pub fn slash_unsolicited_signing_ix_with_registry(
        &self,
        reporter: &Pubkey,
        evidence: UnsolicitedSigningEvidence,
        new_registry_version: u32,
    ) -> Instruction {
        self.slash_offence_ix_with_registry(
            reporter,
            Offence::UnsolicitedSigning(evidence),
            new_registry_version,
        )
    }

    pub fn slash_commitment_reuse_ix(
        &self,
        reporter: &Pubkey,
        evidence: CommitmentReuseEvidence,
    ) -> Instruction {
        self.slash_offence_ix(reporter, Offence::CommitmentReuse(evidence))
    }

    pub fn slash_commitment_reuse_ix_with_registry(
        &self,
        reporter: &Pubkey,
        evidence: CommitmentReuseEvidence,
        new_registry_version: u32,
    ) -> Instruction {
        self.slash_offence_ix_with_registry(
            reporter,
            Offence::CommitmentReuse(evidence),
            new_registry_version,
        )
    }

    pub fn slash_nonce_reuse_ix(
        &self,
        reporter: &Pubkey,
        evidence: NonceReuseEvidence,
    ) -> Instruction {
        self.slash_offence_ix(reporter, Offence::NonceReuse(evidence))
    }

    pub fn slash_nonce_reuse_ix_with_registry(
        &self,
        reporter: &Pubkey,
        evidence: NonceReuseEvidence,
        new_registry_version: u32,
    ) -> Instruction {
        self.slash_offence_ix_with_registry(
            reporter,
            Offence::NonceReuse(evidence),
            new_registry_version,
        )
    }
}

/// The guilty set of an unsolicited-signing filing: signers outside the derived selection.
pub fn unsolicited_guilty_bitmap(p: &Protocol, attestation: &Attestation) -> [u8; 32] {
    let payload = &attestation.payload;
    let registry = p.env.registry(payload.registry_version);
    let signers = molpha_verifier::Bitmap::load(&attestation.signature.signers_bitmap);
    let selection = molpha_verifier::attestation_selection(
        &payload.source_id,
        payload.timestamp,
        payload.signatures_required,
        &signers,
        &registry.selection_view(),
    )
    .expect("selection derives");
    signers.node_bits().subtract(&selection).to_bytes()
}

/// The node bits of a signer bitmap (`bitmap & NODE_MASK`): provider bits dropped.
pub fn node_part(bitmap: &[u8; 32]) -> [u8; 32] {
    molpha_verifier::Bitmap::load(bitmap).node_bits().to_bytes()
}

/// Bitwise union of two 256-bit signer bitmaps.
pub fn union_bitmap(a: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = a[i] | b[i];
    }
    out
}

/// Bits present in `a` but not `b`.
pub fn bitmap_difference(a: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = a[i] & !b[i];
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
    /// Use this only to create focused test states that would otherwise require many rounds, such
    /// as a frozen node.
    ///
    /// Writing `locked_amount` or `claimable_rewards` here desynchronises the node from its
    /// deposit vault and from `ProtocolConfig.reward_liability`, which the program maintains but
    /// cannot repair. Fund the vault to match if you move the deposit fields.
    /// [`assert_solvent`](Self::assert_solvent) will then fail, and a later claim or punishment
    /// can underflow the liability. Move money through the instructions instead.
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
}

// ---------------------------------------------------------------------------
// epochs, tickets and revenue
// ---------------------------------------------------------------------------

impl Protocol {
    /// A duty stamped at the current cluster time against the current registry, the way a live
    /// round looks, so it falls in the epoch open now.
    pub fn live_duty(&self, signatures_required: u8) -> Duty {
        Duty::new(self.current_version(), signatures_required)
            .at_secs(self.env.unix_timestamp() as u64)
    }

    /// Sign `duty` with the first `k` selected nodes: the canonical set a live round produces.
    pub fn sign_canonical(&self, duty: Duty) -> SignedAttestation {
        let bits = self.selection_bits(&duty);
        let k = usize::from(duty.signatures_required);
        self.sign_duty_with_bits(duty, &bits[..k.min(bits.len())], None)
    }

    /// The epoch whose window contains `unix_timestamp`, among existing epoch accounts.
    pub fn epoch_containing(&self, unix_timestamp: i64) -> u64 {
        self.live_epochs()
            .into_iter()
            .find(|epoch| {
                let state = self.env.epoch(*epoch);
                state.start <= unix_timestamp && unix_timestamp < state.end
            })
            .unwrap_or_else(|| panic!("no epoch account covers {unix_timestamp}"))
    }

    /// The epoch open at the current cluster time.
    pub fn open_epoch(&self) -> u64 {
        self.epoch_containing(self.env.unix_timestamp())
    }

    pub fn roster_slot_of(&self, epoch: u64, node_pda: &Pubkey) -> Option<u16> {
        let state = self.env.epoch(epoch);
        state.roster[..usize::from(state.roster_len)]
            .iter()
            .position(|key| key == &node_pda.to_bytes())
            .map(|slot| slot as u16)
    }

    /// Roster slot hints for `duty`'s selection against `epoch`'s current roster: existing slots
    /// for known nodes, appended slots for new ones.
    pub fn roster_slots(&self, epoch: u64, duty: &Duty) -> Vec<u16> {
        self.roster_slots_of(epoch, duty, self.selection_bits(duty))
    }

    /// [`Protocol::roster_slots`] for the round that produced `signers_bitmap`: a provider round
    /// is drawn by its node quorum.
    pub fn roster_slots_for(&self, epoch: u64, duty: &Duty, signers_bitmap: &[u8; 32]) -> Vec<u16> {
        self.roster_slots_of(epoch, duty, self.selection_bits_for(duty, signers_bitmap))
    }

    fn roster_slots_of(&self, epoch: u64, duty: &Duty, selection: Vec<usize>) -> Vec<u16> {
        let state = self.env.epoch(epoch);
        let registry = self.env.registry(duty.registry_version);
        let mut roster: Vec<[u8; 32]> = state.roster[..usize::from(state.roster_len)].to_vec();
        selection
            .into_iter()
            .map(|bit| {
                let node = registry.nodes[bit].to_bytes();
                match roster.iter().position(|key| *key == node) {
                    Some(slot) => slot as u16,
                    None if roster.len() == MAX_REGISTRY_NODES => MAX_REGISTRY_NODES as u16,
                    None => {
                        roster.push(node);
                        (roster.len() - 1) as u16
                    }
                }
            })
            .collect()
    }

    /// The gateway's dispatch signature over `duty`, as the Ed25519 instruction that must sit
    /// immediately before `submit_ticket`.
    pub fn dispatch_ix(&self, gateway_authority: &Keypair, duty: &Duty) -> Instruction {
        crate::ed25519::sign_ed25519(
            gateway_authority,
            &molpha::state::dispatch_message(&molpha::ID, &duty.payload()),
        )
    }

    pub fn submit_ticket_ix(
        &self,
        submitter: &Pubkey,
        gateway_authority: &Pubkey,
        epoch: u64,
        signed: &SignedAttestation,
    ) -> Instruction {
        self.submit_ticket_raw_ix(
            submitter,
            gateway_authority,
            epoch,
            signed.attestation(),
            self.roster_slots_for(epoch, &signed.duty, &signed.signers_bitmap),
            signed.coalition_key,
        )
    }

    /// `submit_ticket` with caller-built arguments — for tampered hints, keys and payloads.
    #[allow(clippy::too_many_arguments)]
    pub fn submit_ticket_raw_ix(
        &self,
        submitter: &Pubkey,
        gateway_authority: &Pubkey,
        epoch: u64,
        attestation: Attestation,
        roster_slots: Vec<u16>,
        coalition_key: CoalitionKey,
    ) -> Instruction {
        let payload = &attestation.payload;
        let accounts = molpha::accounts::SubmitTicket {
            submitter: *submitter,
            protocol_config: pda::protocol_config(),
            registry: pda::registry(payload.registry_version),
            epoch_state: pda::epoch(epoch),
            gateway: pda::gateway(gateway_authority),
            ticket_marker: pda::ticket_marker(&molpha::state::ticket_id(payload)),
            instructions_sysvar: pda::INSTRUCTIONS_SYSVAR_ID,
            system_program: anchor_lang::solana_program::system_program::ID,
        }
        .to_account_metas(None);
        Instruction {
            program_id: molpha::ID,
            accounts,
            data: molpha::instruction::SubmitTicket {
                epoch,
                attestation,
                roster_slots,
                coalition_key,
            }
            .data(),
        }
    }

    /// `[dispatch, submit_ticket]` for `signed`, in the epoch containing its timestamp.
    pub fn ticket_ixs(
        &self,
        submitter: &Pubkey,
        gateway_authority: &Keypair,
        signed: &SignedAttestation,
    ) -> Vec<Instruction> {
        let epoch = self.epoch_containing(signed.duty.secs() as i64);
        vec![
            self.dispatch_ix(gateway_authority, &signed.duty),
            self.submit_ticket_ix(submitter, &gateway_authority.pubkey(), epoch, signed),
        ]
    }

    pub fn submit_ticket(
        &mut self,
        submitter: &Keypair,
        gateway_authority: &Keypair,
        signed: &SignedAttestation,
    ) -> TransactionMetadata {
        let ixs = self.ticket_ixs(&submitter.pubkey(), gateway_authority, signed);
        self.env.send_ok(&ixs, &[submitter])
    }

    pub fn close_ticket_marker_ix(&self, marker: &Pubkey, submitter: &Pubkey) -> Instruction {
        Instruction {
            program_id: molpha::ID,
            accounts: molpha::accounts::CloseTicketMarker {
                ticket_marker: *marker,
                submitter: *submitter,
            }
            .to_account_metas(None),
            data: molpha::instruction::CloseTicketMarker {}.data(),
        }
    }

    pub fn finalize_epoch_ix(&self, payer: &Pubkey, epoch: u64) -> Instruction {
        Instruction {
            program_id: molpha::ID,
            accounts: molpha::accounts::FinalizeEpoch {
                payer: *payer,
                protocol_config: pda::protocol_config(),
                target: pda::epoch(epoch),
                next: pda::epoch(epoch + 1),
                successor: pda::epoch(epoch + 2),
                system_program: anchor_lang::solana_program::system_program::ID,
            }
            .to_account_metas(None),
            data: molpha::instruction::FinalizeEpoch { epoch }.data(),
        }
    }

    /// Warp to `epoch`'s ticket deadline and finalize it, paid by the fee payer.
    pub fn finalize_epoch(&mut self, epoch: u64) -> TransactionMetadata {
        let state = self.env.epoch(epoch);
        let deadline = state.end + state.ticket_grace;
        if self.env.unix_timestamp() < deadline {
            self.env.warp_to(deadline);
        }
        let ix = self.finalize_epoch_ix(&self.env.payer_pubkey(), epoch);
        self.env.send_ok(&[ix], &[])
    }

    pub fn credit_epoch_rewards_ix(&self, epoch: u64, slots: &[u16]) -> Instruction {
        let state = self.env.epoch(epoch);
        let mut accounts = molpha::accounts::CreditEpochRewards {
            protocol_config: pda::protocol_config(),
            epoch_state: pda::epoch(epoch),
        }
        .to_account_metas(None);
        accounts.extend(crate::env::writable_metas(
            slots
                .iter()
                .map(|slot| Pubkey::new_from_array(state.roster[usize::from(*slot)])),
        ));
        Instruction {
            program_id: molpha::ID,
            accounts,
            data: molpha::instruction::CreditEpochRewards {
                epoch,
                slots: slots.to_vec(),
            }
            .data(),
        }
    }

    /// Roster slots of a finalized epoch that carry weight and are not yet credited.
    pub fn creditable_slots(&self, epoch: u64) -> Vec<u16> {
        let state = self.env.epoch(epoch);
        (0..state.roster_len)
            .filter(|slot| {
                let slot = usize::from(*slot);
                state.weight(slot) > 0 && !state.is_credited(slot)
            })
            .collect()
    }

    /// Credit every weighted slot of a finalized epoch.
    pub fn credit_all(&mut self, epoch: u64) {
        let slots = self.creditable_slots(epoch);
        for chunk in slots.chunks(16) {
            let ix = self.credit_epoch_rewards_ix(epoch, chunk);
            self.env.send_ok(&[ix], &[]);
        }
    }

    pub fn close_epoch_ix(&self, epoch: u64, open_epoch: u64) -> Instruction {
        let state = self.env.epoch(epoch);
        Instruction {
            program_id: molpha::ID,
            accounts: molpha::accounts::CloseEpoch {
                epoch_state: pda::epoch(epoch),
                open_epoch_state: pda::epoch(open_epoch),
                rent_payer: Pubkey::new_from_array(state.rent_payer),
            }
            .to_account_metas(None),
            data: molpha::instruction::CloseEpoch { epoch, open_epoch }.data(),
        }
    }

    /// The instant `epoch` becomes closable: one epoch length after its ticket deadline.
    pub fn close_deadline(state: &EpochState) -> i64 {
        state.end + state.ticket_grace + (state.end - state.start)
    }

    pub fn sweep_treasury_revenue_ix(&self, open_epoch: u64) -> Instruction {
        Instruction {
            program_id: molpha::ID,
            accounts: molpha::accounts::SweepTreasuryRevenue {
                protocol_config: pda::protocol_config(),
                treasury: self.treasury(),
                epoch_state: pda::epoch(open_epoch),
                token_program: pda::TOKEN_PROGRAM_ID,
            }
            .to_account_metas(None),
            data: molpha::instruction::SweepTreasuryRevenue { open_epoch }.data(),
        }
    }

    pub fn withdraw_protocol_reserve_ix(
        &self,
        authority: &Pubkey,
        destination: &Pubkey,
        amount: u64,
    ) -> Instruction {
        Instruction {
            program_id: molpha::ID,
            accounts: molpha::accounts::WithdrawProtocolReserve {
                authority: *authority,
                protocol_config: pda::protocol_config(),
                treasury: self.treasury(),
                destination: *destination,
                token_program: pda::TOKEN_PROGRAM_ID,
            }
            .to_account_metas(None),
            data: molpha::instruction::WithdrawProtocolReserve { amount }.data(),
        }
    }

    /// An x402 payment of `income` into the treasury, swept into the open epoch.
    pub fn earn_income(&mut self, income: u64) {
        let epoch = self.open_epoch();
        self.fund_treasury(income);
        let ix = self.sweep_treasury_revenue_ix(epoch);
        self.env.send_ok(&[ix], &[]);
    }

    /// One full reward cycle for the open epoch: `income` swept from the treasury, `rounds` canonical
    /// tickets dispatched by `gateway` on distinct sources, then finalize and credit. Returns the epoch index.
    pub fn run_reward_epoch(
        &mut self,
        gateway_authority: &Keypair,
        income: u64,
        rounds: u8,
    ) -> u64 {
        let epoch = self.open_epoch();
        if income > 0 {
            self.earn_income(income);
        }
        let submitter = self.env.keypair();
        for round in 0..rounds {
            let duty = self
                .live_duty(self.args.min_signers)
                .source_id([round.wrapping_add(1); 32]);
            let signed = self.sign_canonical(duty);
            self.submit_ticket(&submitter, gateway_authority, &signed);
        }
        self.finalize_epoch(epoch);
        self.credit_all(epoch);
        epoch
    }
}

impl Protocol {
    /// Sign `duty` with the first `k` selected nodes other than `excluded`, so the excluded
    /// nodes count as selected-but-missing in a ticket.
    pub fn sign_excluding(&self, duty: Duty, excluded: &[Pubkey]) -> SignedAttestation {
        let registry = self.env.registry(duty.registry_version);
        let k = usize::from(duty.signatures_required);
        let bits: Vec<usize> = self
            .selection_bits(&duty)
            .into_iter()
            .filter(|bit| !excluded.contains(&registry.nodes[*bit]))
            .take(k)
            .collect();
        assert_eq!(
            bits.len(),
            k,
            "not enough selected signers outside the excluded set"
        );
        self.sign_duty_with_bits(duty, &bits, None)
    }

    /// Live duties (distinct sources) whose selection includes `node_pda`.
    pub fn duties_selecting(&self, node_pda: &Pubkey, count: usize, salt: u8) -> Vec<Duty> {
        let version = self.current_version();
        let index = self
            .env
            .registry(version)
            .index_of(node_pda)
            .expect("node is in the current registry") as usize;
        (0u16..=u16::MAX)
            .map(|i| {
                let mut source = [salt; 32];
                source[..2].copy_from_slice(&i.to_be_bytes());
                self.live_duty(self.args.min_signers).source_id(source)
            })
            .filter(|duty| self.selection_bits(duty).contains(&index))
            .take(count)
            .collect()
    }

    /// Put `node_pda` below the availability floor of the open epoch with `rounds` tickets that
    /// select it but leave it out, then finalize that epoch. Returns the epoch index.
    pub fn run_epoch_missing(
        &mut self,
        gateway_authority: &Keypair,
        node_pda: &Pubkey,
        rounds: usize,
    ) -> u64 {
        let epoch = self.open_epoch();
        let submitter = self.env.keypair();
        for duty in self.duties_selecting(node_pda, rounds, 0x5a) {
            let signed = self.sign_excluding(duty, &[*node_pda]);
            self.submit_ticket(&submitter, gateway_authority, &signed);
        }
        self.finalize_epoch(epoch);
        epoch
    }

    /// Freeze `node_pda` through a genuine inactivity filing on an epoch it missed.
    pub fn freeze_via_inactivity(&mut self, gateway_authority: &Keypair, node_pda: &Pubkey) -> u64 {
        let rounds = usize::from(self.args.min_availability_samples);
        let epoch = self.run_epoch_missing(gateway_authority, node_pda, rounds);
        let reporter = self.reporter();
        let ix = self.slash_inactivity_ix(&reporter.pubkey(), epoch, node_pda);
        self.env.send_ok(&[ix], &[&reporter]);
        epoch
    }
}

// ---------------------------------------------------------------------------
// provider signers
// ---------------------------------------------------------------------------

impl Protocol {
    pub fn provider_account(&self, id: u32) -> Provider {
        self.env.get::<Provider>(&pda::provider(id))
    }

    /// The id the next `register_provider` is assigned.
    pub fn next_provider_id(&self) -> u32 {
        self.env
            .get::<RegistryState>(&pda::registry_state())
            .next_provider_id
    }

    /// Well-formed `register_node` args registering a provider holding `key`, signed by its
    /// `authority`, with a valid node proof of possession.
    pub fn register_provider_args(
        &self,
        authority: &Pubkey,
        key: &NodeKey,
        p2p_key: [u8; 32],
    ) -> RegisterNodeArgs {
        let mut salt = [0u8; 32];
        salt[0] = 2;
        salt[1..5].copy_from_slice(&self.next_provider_id().to_be_bytes());
        member_args(authority, key, salt, MemberEndpoint::Provider { p2p_key })
    }

    /// `register_node` for a provider at the next id, co-signed by the protocol authority.
    pub fn register_provider_ix(&self, authority: &Pubkey, args: &RegisterNodeArgs) -> Instruction {
        self.register_provider_ix_for_id(authority, args, self.next_provider_id())
    }

    /// `register_node` for a provider naming the provider PDA of `id`, which must be the next id.
    pub fn register_provider_ix_for_id(
        &self,
        authority: &Pubkey,
        args: &RegisterNodeArgs,
        id: u32,
    ) -> Instruction {
        let current_version = self.current_version();
        Instruction {
            program_id: molpha::ID,
            accounts: molpha::accounts::RegisterNode {
                authority: self.authority.pubkey(),
                node_owner: *authority,
                protocol_config: pda::protocol_config(),
                registry_state: pda::registry_state(),
                current_registry: pda::registry(current_version),
                new_registry: pda::registry(current_version + 1),
                node: None,
                provider: Some(pda::provider(id)),
                key_x_claim: pda::key_x_claim_for(&args.secp256k1_pubkey_compressed),
                owner_usdc: None,
                node_deposit_vault: None,
                system_program: anchor_lang::solana_program::system_program::ID,
                token_program: pda::TOKEN_PROGRAM_ID,
            }
            .to_account_metas(None),
            data: molpha::instruction::RegisterNode { args: args.clone() }.data(),
        }
    }

    /// The `extend_registry` instructions a provider registration needs in front of it: at most
    /// two records more than the current version (the first provider brings reserved slot 0).
    pub fn provider_extend_ixs(&self, payer: &Pubkey) -> Vec<Instruction> {
        self.registry_extend_ixs(payer, self.current_members() + 2)
    }

    /// `deactivate_node` for a provider: the provider authority leaves the registry.
    pub fn eject_provider_ix(&self, authority: &Pubkey, id: u32) -> Instruction {
        let current_version = self.current_version();
        Instruction {
            program_id: molpha::ID,
            accounts: molpha::accounts::DeactivateNode {
                authority: *authority,
                protocol_config: pda::protocol_config(),
                registry_state: pda::registry_state(),
                current_registry: pda::registry(current_version),
                new_registry: pda::registry(current_version + 1),
                node: None,
                provider: Some(pda::provider(id)),
                system_program: anchor_lang::solana_program::system_program::ID,
            }
            .to_account_metas(None),
            // `node_owner` is unused on the provider path.
            data: molpha::instruction::DeactivateNode {
                node_owner: *authority,
            }
            .data(),
        }
    }

    /// The proof that rotates provider `index` to `new_key`, signed with the key it holds now.
    pub fn provider_key_rotation_proof(&self, index: usize, new_key: &NodeKey) -> KeyRotationProof {
        let provider = &self.providers[index];
        crypto::key_rotation_proof(&molpha::ID, &provider.pda, &provider.key, new_key)
    }

    pub fn rotate_provider_key_ix(&self, index: usize, proof: KeyRotationProof) -> Instruction {
        self.rotate_provider_key_raw_ix(
            &self.providers[index].authority.pubkey(),
            self.providers[index].id,
            proof,
        )
    }

    /// `rotate_node_key` for a provider with a caller-chosen signer and id — for failure cases.
    pub fn rotate_provider_key_raw_ix(
        &self,
        signer: &Pubkey,
        id: u32,
        proof: KeyRotationProof,
    ) -> Instruction {
        let current_version = self.current_version();
        Instruction {
            program_id: molpha::ID,
            accounts: molpha::accounts::RotateNodeKey {
                node_owner: *signer,
                protocol_config: pda::protocol_config(),
                registry_state: pda::registry_state(),
                current_registry: pda::registry(current_version),
                new_registry: pda::registry(current_version + 1),
                node: None,
                provider: Some(pda::provider(id)),
                key_x_claim: pda::key_x_claim_for(&proof.new_public_key_compressed),
                system_program: anchor_lang::solana_program::system_program::ID,
            }
            .to_account_metas(None),
            data: molpha::instruction::RotateNodeKey { proof }.data(),
        }
    }

    /// `update_node_endpoint` for a provider: its authority moves its libp2p identity.
    pub fn update_provider_p2p_key_ix(
        &self,
        authority: &Pubkey,
        id: u32,
        new_p2p_key: [u8; 32],
    ) -> Instruction {
        self.update_endpoint_raw_ix(
            authority,
            None,
            Some(pda::provider(id)),
            MemberEndpoint::Provider {
                p2p_key: new_p2p_key,
            },
        )
    }

    /// A fresh ed25519 identity for a provider's p2p key.
    pub fn p2p_key(&mut self) -> [u8; 32] {
        self.env.keypair().pubkey().to_bytes()
    }

    /// Register a provider holding `key` through `register_node`, co-signed by the protocol
    /// authority: it is in the registry at once. Returns its index in `providers`.
    pub fn register_provider(&mut self, key: NodeKey) -> usize {
        let pending = self.prepare_provider(key);
        let mut ixs = self.provider_extend_ixs(&pending.authority.pubkey());
        ixs.push(pending.ix.clone());
        let protocol_authority = self.authority.insecure_clone();
        self.env
            .send_ok(&ixs, &[&protocol_authority, &pending.authority]);
        self.record_provider(pending)
    }

    /// A provider registration for `key` at the next id, built but not sent: for tests that
    /// choose what goes in front of it, or expect it to fail. Sign it with the protocol authority
    /// and `authority`, then [`Protocol::record_provider`] once it lands.
    pub fn prepare_provider(&mut self, key: NodeKey) -> PendingProvider {
        let authority = self.env.keypair();
        let p2p_key = self.p2p_key();
        let args = self.register_provider_args(&authority.pubkey(), &key, p2p_key);
        let id = self.next_provider_id();
        let ix = self.register_provider_ix(&authority.pubkey(), &args);
        PendingProvider {
            authority,
            key,
            p2p_key,
            id,
            ix,
        }
    }

    /// Track a provider whose registration landed. Returns its index in `providers`.
    pub fn record_provider(&mut self, pending: PendingProvider) -> usize {
        let slot = usize::from(self.provider_account(pending.id).slot);
        self.providers.push(RegisteredProvider {
            authority: pending.authority,
            key: pending.key,
            p2p_key: pending.p2p_key,
            id: pending.id,
            pda: pda::provider(pending.id),
            slot: Some(slot),
            retired_keys: Vec::new(),
        });
        self.providers.len() - 1
    }

    /// [`Protocol::register_provider`]: a registered provider is active.
    pub fn add_active_provider(&mut self, key: NodeKey) -> usize {
        self.register_provider(key)
    }

    /// Rotate provider `index` to `new_key`, signed by its authority and its current key. The
    /// previous key is kept so attestations under older registry versions can still be signed.
    pub fn rotate_provider_key(&mut self, index: usize, new_key: NodeKey) -> TransactionMetadata {
        let proof = self.provider_key_rotation_proof(index, &new_key);
        let authority = self.providers[index].authority.insecure_clone();
        let mut ixs = self.registry_extend_ixs(&authority.pubkey(), self.current_members());
        ixs.push(self.rotate_provider_key_ix(index, proof));
        let meta = self.env.send_ok(&ixs, &[&authority]);
        let provider = &mut self.providers[index];
        let retired = std::mem::replace(&mut provider.key, new_key);
        provider.retired_keys.push(retired);
        meta
    }

    /// The provider authority ejects provider `index`.
    pub fn eject_provider(&mut self, index: usize) {
        let authority = self.providers[index].authority.insecure_clone();
        let id = self.providers[index].id;
        let ix = self.eject_provider_ix(&authority.pubkey(), id);
        self.env.send_ok(&[ix], &[&authority]);
    }
}

impl Protocol {
    /// Register `count` providers with deterministic keys derived from `domain`. Returns their
    /// indices in `providers`.
    pub fn add_active_providers(&mut self, domain: &str, count: usize) -> Vec<usize> {
        crypto::node_keys(domain, count)
            .into_iter()
            .map(|key| self.register_provider(key))
            .collect()
    }

    /// The bits of the providers at `indices`, for [`Protocol::sign_duty_with_bits`].
    pub fn provider_bits(&self, indices: &[usize]) -> Vec<usize> {
        indices.iter().map(|i| self.providers[*i].bit()).collect()
    }

    /// Sign `duty` with exactly the nodes at `node_bits` and the providers at `providers`
    /// (indices in `self.providers`), whose bits join the aggregate beside the nodes'.
    pub fn sign_duty_with_providers(
        &self,
        duty: Duty,
        node_bits: &[usize],
        providers: &[usize],
    ) -> SignedAttestation {
        let mut bits = node_bits.to_vec();
        bits.extend(self.provider_bits(providers));
        self.sign_duty_with_bits(duty, &bits, None)
    }
}
