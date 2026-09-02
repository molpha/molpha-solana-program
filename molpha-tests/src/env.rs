//! LiteSVM harness: one fresh VM per test, plus the assertions the tests share.

use std::path::PathBuf;
use std::sync::OnceLock;

use anchor_lang::prelude::Pubkey;
use anchor_lang::solana_program::instruction::{AccountMeta, Instruction};
use anchor_lang::{AccountDeserialize, AnchorDeserialize, Discriminator};
use litesvm::types::{FailedTransactionMetadata, TransactionMetadata, TransactionResult};
use litesvm::LiteSVM;
use solana_clock::Clock;
use solana_instruction::error::InstructionError;
use solana_keypair::Keypair;
use solana_message::Message;
use solana_signer::Signer;
use solana_transaction::Transaction;
use solana_transaction_error::TransactionError;

use crate::pda;
use crate::token;

pub const COMPUTE_BUDGET_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("ComputeBudget111111111111111111111111111111");

/// Every test transaction runs at the max per-transaction budget: CU tuning is not what these
/// tests are for, and a silent `ComputeBudgetExceeded` reads exactly like a logic failure.
pub const DEFAULT_CU_LIMIT: u32 = 1_400_000;

pub const SOL: u64 = 1_000_000_000;

/// Maximum serialized transaction size the Solana network accepts.
pub const PACKET_DATA_SIZE: usize = 1_232;

/// Where every test's wall clock starts. LiteSVM defaults `unix_timestamp` to 0, which the
/// freshness window would read as "every duty is ~1.7e9 seconds in the future". Sits above
/// every hardcoded `canonical_timestamp` literal in the suite so they all read as recent past.
pub const GENESIS_UNIX_TIMESTAMP: i64 = 1_700_200_000;

fn program_bytes() -> &'static [u8] {
    static BYTES: OnceLock<Vec<u8>> = OnceLock::new();
    BYTES.get_or_init(|| {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/deploy/molpha.so");
        std::fs::read(&path).unwrap_or_else(|e| {
            panic!(
                "cannot read {}: {e}\nBuild the program first: `anchor build -p molpha`",
                path.display()
            )
        })
    })
}

pub fn set_compute_unit_limit(units: u32) -> Instruction {
    let mut data = Vec::with_capacity(5);
    data.push(2u8);
    data.extend_from_slice(&units.to_le_bytes());
    Instruction {
        program_id: COMPUTE_BUDGET_PROGRAM_ID,
        accounts: vec![],
        data,
    }
}

/// `RequestHeapFrame` — raises the per-invocation heap above the 32 KiB default.
///
/// Large punishment filings deserialize many `Node` accounts in one instruction; production
/// callers pair this with a v0 transaction when the signer list exceeds the legacy wire limit.
pub fn request_heap_frame(bytes: u32) -> Instruction {
    let mut data = Vec::with_capacity(5);
    data.push(1u8);
    data.extend_from_slice(&bytes.to_le_bytes());
    Instruction {
        program_id: COMPUTE_BUDGET_PROGRAM_ID,
        accounts: vec![],
        data,
    }
}

/// Largest heap frame the compute-budget program accepts (256 KiB).
pub const MAX_HEAP_FRAME_BYTES: u32 = 256 * 1024;

/// `SetComputeUnitPrice` — a harmless second instruction, used by tests that need to wedge
/// something between the Ed25519 authorization and the settlement it authorizes.
pub fn set_compute_unit_price(micro_lamports: u64) -> Instruction {
    let mut data = Vec::with_capacity(9);
    data.push(3u8);
    data.extend_from_slice(&micro_lamports.to_le_bytes());
    Instruction {
        program_id: COMPUTE_BUDGET_PROGRAM_ID,
        accounts: vec![],
        data,
    }
}

/// A LiteSVM instance with the Molpha program loaded and a funded fee payer.
pub struct TestEnv {
    pub svm: LiteSVM,
    pub payer: Keypair,
}

impl Default for TestEnv {
    fn default() -> Self {
        Self::new()
    }
}

impl TestEnv {
    pub fn new() -> Self {
        let mut svm = LiteSVM::new();
        svm.add_program(molpha::ID, program_bytes())
            .expect("load molpha program");
        let mut clock = svm.get_sysvar::<Clock>();
        clock.unix_timestamp = GENESIS_UNIX_TIMESTAMP;
        svm.set_sysvar(&clock);
        let payer = Keypair::new();
        svm.airdrop(&payer.pubkey(), 10_000 * SOL)
            .expect("fund payer");
        Self { svm, payer }
    }

    pub fn payer_pubkey(&self) -> Pubkey {
        self.payer.pubkey()
    }

    /// A funded keypair. Every actor in a test gets one of these rather than sharing the payer,
    /// so authorization failures cannot pass by accident.
    pub fn keypair(&mut self) -> Keypair {
        let kp = Keypair::new();
        self.svm
            .airdrop(&kp.pubkey(), 1_000 * SOL)
            .expect("airdrop");
        kp
    }

    // -----------------------------------------------------------------------
    // transactions
    // -----------------------------------------------------------------------

    /// Send `ixs` signed by the payer plus `signers`, at the default compute budget.
    pub fn send(&mut self, ixs: &[Instruction], signers: &[&Keypair]) -> TransactionResult {
        self.send_with_cu(ixs, signers, DEFAULT_CU_LIMIT)
    }

    /// Like [`send`](Self::send), but also requests the maximum heap frame.
    pub fn send_with_max_heap(
        &mut self,
        ixs: &[Instruction],
        signers: &[&Keypair],
    ) -> TransactionResult {
        self.send_with_cu_and_heap(ixs, signers, DEFAULT_CU_LIMIT, MAX_HEAP_FRAME_BYTES)
    }

    pub fn send_with_cu(
        &mut self,
        ixs: &[Instruction],
        signers: &[&Keypair],
        cu_limit: u32,
    ) -> TransactionResult {
        let tx = self.build_transaction(ixs, signers, cu_limit);
        let result = self.svm.send_transaction(tx);
        self.svm.expire_blockhash();
        result
    }

    pub fn send_with_cu_and_heap(
        &mut self,
        ixs: &[Instruction],
        signers: &[&Keypair],
        cu_limit: u32,
        heap_bytes: u32,
    ) -> TransactionResult {
        let mut all = Vec::with_capacity(ixs.len() + 2);
        all.push(set_compute_unit_limit(cu_limit));
        if heap_bytes > 0 {
            all.push(request_heap_frame(heap_bytes));
        }
        all.extend_from_slice(ixs);
        let tx = self.build_transaction_prepared(&all, signers);
        let result = self.svm.send_transaction(tx);
        // Two otherwise-identical transactions in one test would collide on signature;
        // rotating the blockhash keeps callers from having to think about it.
        self.svm.expire_blockhash();
        result
    }

    /// Serialized wire size of the transaction `send` would build for `ixs`.
    ///
    /// A transaction larger than [`PACKET_DATA_SIZE`] is rejected by the network before it ever
    /// reaches a validator; LiteSVM does not model that, so tests that care must measure.
    pub fn transaction_size(&self, ixs: &[Instruction], signers: &[&Keypair]) -> usize {
        let tx = self.build_transaction(ixs, signers, DEFAULT_CU_LIMIT);
        bincode::serialize(&tx)
            .expect("transaction serializes")
            .len()
    }

    /// Wire size of `ixs` as `payer` would actually broadcast them.
    ///
    /// [`transaction_size`](Self::transaction_size) measures the harness's own transaction, which
    /// always adds the funding payer as an extra signer — 96 bytes a real caller does not pay.
    /// Instructions whose account list is near the packet limit must be measured this way.
    pub fn transaction_size_as(&self, payer: &Pubkey, ixs: &[Instruction]) -> usize {
        let mut all = Vec::with_capacity(ixs.len() + 1);
        all.push(set_compute_unit_limit(DEFAULT_CU_LIMIT));
        all.extend_from_slice(ixs);
        let message = Message::new(&all, Some(payer));
        let signatures = usize::from(message.header.num_required_signatures);
        // Legacy wire format: a short-vec of 64-byte signatures, then the serialized message.
        1 + 64 * signatures + message.serialize().len()
    }

    /// The compute-budget prefix, message and signatures every test transaction shares.
    ///
    /// Signs with exactly the keys the message requires: callers pass whoever is conceptually
    /// acting, and an account that turned out not to be a signer (or a duplicate) is dropped here
    /// rather than blowing up inside `Transaction::sign`.
    fn build_transaction(
        &self,
        ixs: &[Instruction],
        signers: &[&Keypair],
        cu_limit: u32,
    ) -> Transaction {
        let mut all = Vec::with_capacity(ixs.len() + 1);
        all.push(set_compute_unit_limit(cu_limit));
        all.extend_from_slice(ixs);
        self.build_transaction_prepared(&all, signers)
    }

    /// Build a transaction from a fully prepared instruction list (compute budget already included).
    fn build_transaction_prepared(
        &self,
        ixs: &[Instruction],
        signers: &[&Keypair],
    ) -> Transaction {
        let payer_pubkey = self.payer.pubkey();
        let message = Message::new(ixs, Some(&payer_pubkey));
        let available: Vec<&Keypair> = std::iter::once(&self.payer)
            .chain(signers.iter().copied())
            .collect();
        let required = &message.account_keys[..message.header.num_required_signatures as usize];
        let signing: Vec<&Keypair> = required
            .iter()
            .map(|key| {
                *available
                    .iter()
                    .find(|kp| kp.pubkey() == *key)
                    .unwrap_or_else(|| {
                        panic!(
                            "transaction requires a signature from {key}, which was not supplied"
                        )
                    })
            })
            .collect();

        let mut tx = Transaction::new_unsigned(message);
        tx.sign(&signing, self.svm.latest_blockhash());
        tx
    }

    /// Send and require success, surfacing program logs on failure.
    pub fn send_ok(&mut self, ixs: &[Instruction], signers: &[&Keypair]) -> TransactionMetadata {
        match self.send(ixs, signers) {
            Ok(meta) => meta,
            Err(failed) => panic!(
                "transaction failed: {:?}\n--- logs ---\n{}",
                failed.err,
                failed.meta.logs.join("\n")
            ),
        }
    }

    /// Like [`send_ok`](Self::send_ok), but requests the maximum heap frame first.
    pub fn send_ok_with_max_heap(
        &mut self,
        ixs: &[Instruction],
        signers: &[&Keypair],
    ) -> TransactionMetadata {
        match self.send_with_max_heap(ixs, signers) {
            Ok(meta) => meta,
            Err(failed) => panic!(
                "transaction failed: {:?}\n--- logs ---\n{}",
                failed.err,
                failed.meta.logs.join("\n")
            ),
        }
    }

    /// Send and require failure.
    pub fn send_err(
        &mut self,
        ixs: &[Instruction],
        signers: &[&Keypair],
    ) -> FailedTransactionMetadata {
        match self.send(ixs, signers) {
            Ok(meta) => panic!(
                "transaction unexpectedly succeeded\n--- logs ---\n{}",
                meta.logs.join("\n")
            ),
            Err(failed) => failed,
        }
    }

    // -----------------------------------------------------------------------
    // account access
    // -----------------------------------------------------------------------

    pub fn account_data(&self, key: &Pubkey) -> Vec<u8> {
        self.svm
            .get_account(key)
            .unwrap_or_else(|| panic!("account {key} does not exist"))
            .data
    }

    /// Deserialize a Borsh Anchor account (discriminator checked).
    pub fn get<T: AccountDeserialize>(&self, key: &Pubkey) -> T {
        let data = self.account_data(key);
        T::try_deserialize(&mut data.as_slice())
            .unwrap_or_else(|e| panic!("account {key} failed to deserialize: {e}"))
    }

    /// `true` if the account holds live data (a `close = ..` account reads as gone).
    pub fn account_exists(&self, key: &Pubkey) -> bool {
        self.svm
            .get_account(key)
            .map(|a| a.lamports > 0 && !a.data.is_empty())
            .unwrap_or(false)
    }

    pub fn lamports(&self, key: &Pubkey) -> u64 {
        self.svm.get_account(key).map(|a| a.lamports).unwrap_or(0)
    }

    pub fn token_balance(&self, token_account: &Pubkey) -> u64 {
        self.token_account(token_account).amount
    }

    /// Mint, owner and amount of an SPL token account.
    pub fn token_account(&self, token_account: &Pubkey) -> token::TokenAccountState {
        token::unpack_token_account(&self.account_data(token_account))
    }

    /// `0` when the account has not been created yet — lets tests read a balance before and
    /// after an `init_if_needed` destination without branching.
    pub fn token_balance_or_zero(&self, token_account: &Pubkey) -> u64 {
        self.svm
            .get_account(token_account)
            .filter(|a| a.data.len() >= token::TOKEN_ACCOUNT_LEN)
            .map(|a| token::unpack_token_account(&a.data).amount)
            .unwrap_or(0)
    }

    /// Zero-copy `Registry` snapshot, parsed field by field (no bytemuck alignment concerns).
    pub fn registry(&self, version: u32) -> RegistrySnapshot {
        RegistrySnapshot::parse(&self.account_data(&pda::registry(version)))
    }

    // -----------------------------------------------------------------------
    // clock
    // -----------------------------------------------------------------------

    pub fn clock(&self) -> Clock {
        self.svm.get_sysvar::<Clock>()
    }

    pub fn unix_timestamp(&self) -> i64 {
        self.clock().unix_timestamp
    }

    pub fn slot(&self) -> u64 {
        self.clock().slot
    }

    pub fn set_clock(&mut self, clock: Clock) {
        self.svm.set_sysvar(&clock);
    }

    pub fn advance_seconds(&mut self, seconds: i64) {
        let mut clock = self.clock();
        clock.unix_timestamp += seconds;
        clock.slot += 1;
        self.set_clock(clock);
    }

    pub fn advance_slots(&mut self, slots: u64) {
        let mut clock = self.clock();
        clock.slot += slots;
        // Keep wall-clock roughly consistent with slot time so time-based and slot-based
        // guards cannot disagree about ordering.
        clock.unix_timestamp += (slots as i64) * 400 / 1000;
        self.set_clock(clock);
    }

    // -----------------------------------------------------------------------
    // SPL token setup
    // -----------------------------------------------------------------------

    /// Create a real SPL mint through the token program and return `(mint, mint_authority)`.
    pub fn create_mint(&mut self, decimals: u8) -> (Pubkey, Keypair) {
        let mint = Keypair::new();
        let authority = self.keypair();
        let rent = self.svm.minimum_balance_for_rent_exemption(token::MINT_LEN);
        let create = anchor_lang::solana_program::system_instruction::create_account(
            &self.payer.pubkey(),
            &mint.pubkey(),
            rent,
            token::MINT_LEN as u64,
            &pda::TOKEN_PROGRAM_ID,
        );
        let init = token::initialize_mint2(&mint.pubkey(), &authority.pubkey(), decimals);
        self.send_ok(&[create, init], &[&mint]);
        (mint.pubkey(), authority)
    }

    /// Create `owner`'s ATA if needed and mint `amount` into it.
    pub fn mint_to(
        &mut self,
        mint: &Pubkey,
        mint_authority: &Keypair,
        owner: &Pubkey,
        amount: u64,
    ) -> Pubkey {
        let ata = pda::ata(owner, mint);
        let payer = self.payer.pubkey();
        let mut ixs = vec![token::create_ata_idempotent(&payer, owner, mint)];
        if amount > 0 {
            ixs.push(token::mint_to(mint, &ata, &mint_authority.pubkey(), amount));
        }
        self.send_ok(&ixs, &[mint_authority]);
        ata
    }

    /// Plant a mint owned by the Token-2022 program. Used to prove the program rejects it;
    /// nothing ever CPIs into it, so raw account data is enough.
    pub fn plant_token_2022_mint(&mut self, decimals: u8) -> Pubkey {
        let mint = Pubkey::new_unique();
        let authority = self.payer.pubkey();
        let data = token::pack_mint(&authority, 0, decimals);
        let lamports = self.svm.minimum_balance_for_rent_exemption(data.len());
        self.svm
            .set_account(
                mint,
                solana_account::Account {
                    lamports,
                    data,
                    owner: pda::TOKEN_2022_PROGRAM_ID,
                    executable: false,
                    rent_epoch: 0,
                },
            )
            .expect("plant token-2022 mint");
        mint
    }

    /// Overwrite an existing program-owned account with `data` (discriminator included).
    ///
    /// The escape hatch for states the instruction set cannot currently reach — today only
    /// `NodeStatus::Frozen`, which no shipped penalty produces but `reinstate_node` handles.
    pub fn overwrite_account(&mut self, key: &Pubkey, data: Vec<u8>) {
        let existing = self
            .svm
            .get_account(key)
            .unwrap_or_else(|| panic!("account {key} does not exist"));
        self.svm
            .set_account(
                *key,
                solana_account::Account {
                    lamports: existing.lamports,
                    data,
                    owner: existing.owner,
                    executable: existing.executable,
                    rent_epoch: existing.rent_epoch,
                },
            )
            .expect("overwrite account");
    }
}

/// Parsed `Registry` zero-copy snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegistrySnapshot {
    pub version: u32,
    pub node_count: u16,
    pub redundancy_buffer: u8,
    pub bump: u8,
    /// Only the populated prefix, `nodes[..node_count]`.
    pub nodes: Vec<Pubkey>,
}

impl RegistrySnapshot {
    pub fn parse(data: &[u8]) -> Self {
        assert_eq!(
            data.len(),
            molpha::state::Registry::SPACE,
            "unexpected Registry account size"
        );
        let body = &data[8..];
        let version = u32::from_le_bytes(body[0..4].try_into().unwrap());
        let node_count = u16::from_le_bytes(body[4..6].try_into().unwrap());
        let redundancy_buffer = body[6];
        let bump = body[7];
        let nodes = (0..node_count as usize)
            .map(|i| {
                let start = 8 + i * 32;
                Pubkey::try_from(&body[start..start + 32]).unwrap()
            })
            .collect();
        Self {
            version,
            node_count,
            redundancy_buffer,
            bump,
            nodes,
        }
    }

    pub fn index_of(&self, node: &Pubkey) -> Option<u16> {
        self.nodes.iter().position(|n| n == node).map(|i| i as u16)
    }
}

// ---------------------------------------------------------------------------
// error assertions
// ---------------------------------------------------------------------------

pub fn instruction_error(failed: &FailedTransactionMetadata) -> InstructionError {
    match &failed.err {
        TransactionError::InstructionError(_, ix_err) => ix_err.clone(),
        other => panic!(
            "expected an instruction error, got {other:?}\n--- logs ---\n{}",
            failed.meta.logs.join("\n")
        ),
    }
}

pub fn custom_error_code(failed: &FailedTransactionMetadata) -> u32 {
    match instruction_error(failed) {
        InstructionError::Custom(code) => code,
        other => panic!(
            "expected a custom program error, got {other:?}\n--- logs ---\n{}",
            failed.meta.logs.join("\n")
        ),
    }
}

/// Assert a transaction failed with a specific `MolphaError`.
#[track_caller]
pub fn assert_molpha_error(
    failed: &FailedTransactionMetadata,
    expected: molpha::error::MolphaError,
) {
    let expected_code = u32::from(expected);
    let actual = custom_error_code(failed);
    assert_eq!(
        actual,
        expected_code,
        "expected {expected:?} ({expected_code}), got custom error {actual}\n--- logs ---\n{}",
        failed.meta.logs.join("\n")
    );
}

/// Assert a transaction failed with one of Anchor's built-in constraint errors.
#[track_caller]
pub fn assert_anchor_error(
    failed: &FailedTransactionMetadata,
    expected: anchor_lang::error::ErrorCode,
) {
    let expected_code = u32::from(expected);
    let actual = custom_error_code(failed);
    assert_eq!(
        actual,
        expected_code,
        "expected {expected:?} ({expected_code}), got custom error {actual}\n--- logs ---\n{}",
        failed.meta.logs.join("\n")
    );
}

/// Assert a transaction failed with a non-custom runtime instruction error.
#[track_caller]
pub fn assert_instruction_error(failed: &FailedTransactionMetadata, expected: InstructionError) {
    let actual = instruction_error(failed);
    assert_eq!(
        actual,
        expected,
        "unexpected instruction error\n--- logs ---\n{}",
        failed.meta.logs.join("\n")
    );
}

// ---------------------------------------------------------------------------
// events
// ---------------------------------------------------------------------------

/// Decode every `emit!`-ed event of type `E` from a transaction's logs.
pub fn events<E: Discriminator + AnchorDeserialize>(meta: &TransactionMetadata) -> Vec<E> {
    use base64::Engine;
    meta.logs
        .iter()
        .filter_map(|line| line.strip_prefix("Program data: "))
        .filter_map(|payload| {
            base64::engine::general_purpose::STANDARD
                .decode(payload)
                .ok()
        })
        .filter(|bytes| bytes.len() >= 8 && bytes[..8] == *E::DISCRIMINATOR)
        .map(|bytes| E::deserialize(&mut &bytes[8..]).expect("event body should deserialize"))
        .collect()
}

/// The single `E` a transaction emitted.
#[track_caller]
pub fn only_event<E: Discriminator + AnchorDeserialize>(meta: &TransactionMetadata) -> E {
    let mut found = events::<E>(meta);
    assert_eq!(
        found.len(),
        1,
        "expected exactly one event, found {}",
        found.len()
    );
    found.remove(0)
}

/// `AccountMeta` list for read-only, non-signer accounts — how every `remaining_accounts`
/// slice in this program is passed.
pub fn readonly_metas(keys: impl IntoIterator<Item = Pubkey>) -> Vec<AccountMeta> {
    keys.into_iter()
        .map(|k| AccountMeta::new_readonly(k, false))
        .collect()
}

/// `remaining_accounts` for instructions that mutate the node accounts they credit or slash.
pub fn writable_metas(keys: impl IntoIterator<Item = Pubkey>) -> Vec<AccountMeta> {
    keys.into_iter()
        .map(|k| AccountMeta::new(k, false))
        .collect()
}
