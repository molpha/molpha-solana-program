//! Independent security assertions and deterministic input generation. No signing shortcuts.

use std::collections::BTreeMap;

use anchor_lang::prelude::Pubkey;
use anchor_lang::solana_program::instruction::{AccountMeta, Instruction};
use solana_account::Account;

use crate::{env::TestEnv, pda};

/// Snapshot every account the transaction can name, plus related accounts supplied by the test.
/// The fee payer's balance is excluded; failed transactions still pay transaction fees.
#[derive(Debug)]
pub struct AccountSnapshot {
    accounts: BTreeMap<Pubkey, Option<Account>>,
    payer: Pubkey,
}

fn live_account(env: &TestEnv, key: &Pubkey) -> Option<Account> {
    env.svm
        .get_account(key)
        .filter(|a| a.lamports != 0 || !a.data.is_empty())
}

impl AccountSnapshot {
    pub fn capture(env: &TestEnv, ixs: &[Instruction], extra: &[Pubkey]) -> Self {
        let keys = ixs
            .iter()
            .flat_map(|ix| ix.accounts.iter().map(|a| a.pubkey))
            .chain(extra.iter().copied());
        let accounts = keys
            .filter(|key| *key != pda::INSTRUCTIONS_SYSVAR_ID)
            .map(|key| (key, live_account(env, &key)))
            .collect();
        Self {
            accounts,
            payer: env.payer_pubkey(),
        }
    }

    #[track_caller]
    pub fn assert_unchanged(&self, env: &TestEnv) {
        for (key, before) in &self.accounts {
            let after = live_account(env, key);
            if *key == self.payer {
                let before = before.as_ref().expect("payer exists");
                let after = after.as_ref().expect("payer remains");
                assert_eq!(before.data, after.data, "fee payer data changed");
                assert_eq!(before.owner, after.owner, "fee payer owner changed");
                continue;
            }
            assert_eq!(*before, after, "failed transaction changed account {key}");
        }
    }
}

/// Change the account identified by address, avoiding positional assumptions in attack tests.
pub fn account_meta_mut<'a>(ix: &'a mut Instruction, key: &Pubkey) -> &'a mut AccountMeta {
    ix.accounts
        .iter_mut()
        .find(|meta| meta.pubkey == *key)
        .expect("instruction names account")
}

/// SplitMix64 is an input generator, never a production cryptographic RNG.
#[derive(Clone, Debug)]
pub struct InputRng(pub u64);

impl InputRng {
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    }

    pub fn seed(&mut self) -> [u8; 32] {
        let mut seed = [0; 32];
        for chunk in seed.chunks_exact_mut(8) {
            chunk.copy_from_slice(&self.next_u64().to_le_bytes());
        }
        seed
    }
}

/// Independent integer formulas shared with the fuzz targets, not calls back into the program.
pub fn fee_split(gross: u64, bps: u16) -> (u64, u64) {
    assert!(bps <= 10_000);
    let fee = (u128::from(gross) * u128::from(bps) / 10_000) as u64;
    (fee, gross - fee)
}

pub fn payout(pool: u64, weight: u64, total: u64) -> u64 {
    assert!(weight <= total);
    if total == 0 {
        0
    } else {
        (u128::from(pool) * u128::from(weight) / u128::from(total)) as u64
    }
}
