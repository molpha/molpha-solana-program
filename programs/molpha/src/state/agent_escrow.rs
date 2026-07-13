use anchor_lang::prelude::*;

/// Agent lifecycle status.
///
/// `Paused` is owner-controlled; `Frozen` is a protocol-level safety control
/// (`set_agent_frozen`, gated on `ProtocolConfig.authority`). A frozen escrow cannot be
/// paused/resumed, updated, deposited into, closed, or settled against.
#[derive(
    AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, InitSpace, Default, Debug,
)]
pub enum AgentStatus {
    #[default]
    Active,
    Paused,
    Frozen,
}

/// Funding source consulted by `settle_agent_round` (spec v1 §5).
///
/// Rail consumers (node crediting, finalize, dispute) never see the rail — it only decides
/// where the round price is debited from at settlement.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum PaymentRail {
    /// Prepaid on-chain escrow; settlement debits `AgentEscrow.balance`.
    UsdcEscrow,
    /// Paid off-chain (HTTP 402) to the gateway; settlement debits
    /// `Gateway.settlement_balance` and records `payment_ref` for audit.
    X402,
}

/// Agent entitlement + funding + policy, one account (spec v1 §4.1, minimal Phase-1 shape).
///
/// `owner` (cold) controls funds and policy; `authority` (hot runtime key) only signs
/// `AgentRequestAuth` messages. Deposited USDC is custodied in the protocol treasury —
/// `balance` is internal accounting, exactly like subscription revenue and gateway bonds.
///
/// The on-chain caps (`max_signers`, `epoch_cap`, `expires_at`) are the authoritative
/// backstop bounding a compromised hot key; per-request rate limiting and template/scope
/// policy are enforced off-chain by the gateway (the "no on-chain preflight" contract).
#[account]
#[derive(InitSpace)]
pub struct AgentEscrow {
    /// Cold key: policy + funds control.
    pub owner: Pubkey,
    /// Hot key: signs `AgentRequestAuth` only.
    pub authority: Pubkey,
    /// Seed component; an owner may hold many escrows.
    pub agent_index: u64,
    pub status: AgentStatus,
    /// Internal USDC accounting; principal custodied in the protocol treasury.
    pub balance: u64,
    pub spent_total: u64,
    /// Settlement rejects any request with `signatures_required` above this.
    pub max_signers: u8,
    /// Max settled spend per epoch (`agent_epoch_len_secs`).
    pub epoch_cap: u64,
    pub epoch_spent: u64,
    /// `unix_timestamp / agent_epoch_len_secs`, lazily rolled at settlement.
    pub epoch_id: i64,
    /// 0 = never expires.
    pub expires_at: i64,
    /// Replay guard: settled `request_nonce` values must be strictly increasing.
    pub last_request_nonce: u64,
    pub bump: u8,
}

impl AgentEscrow {
    pub const SEED_PREFIX: &'static [u8] = b"molpha_agent";
    pub const SPACE: usize = 8 + Self::INIT_SPACE;

    pub fn is_expired(&self, now: i64) -> bool {
        self.expires_at != 0 && now >= self.expires_at
    }

    /// Reset the epoch spend window if `now` has crossed into a new epoch.
    pub fn roll_epoch(&mut self, now: i64, epoch_len_secs: i64) {
        let epoch_id = now / epoch_len_secs;
        if epoch_id != self.epoch_id {
            self.epoch_id = epoch_id;
            self.epoch_spent = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_escrow() -> AgentEscrow {
        AgentEscrow {
            owner: Pubkey::new_from_array([1; 32]),
            authority: Pubkey::new_from_array([2; 32]),
            agent_index: 0,
            status: AgentStatus::Active,
            balance: 1_000_000,
            spent_total: 0,
            max_signers: 8,
            epoch_cap: 100_000,
            epoch_spent: 40_000,
            epoch_id: 20_000,
            expires_at: 0,
            last_request_nonce: 0,
            bump: 255,
        }
    }

    #[test]
    fn roll_epoch_resets_spend_only_on_boundary() {
        let mut escrow = fixture_escrow();
        // Same epoch: no reset.
        escrow.roll_epoch(20_000 * 86_400 + 10, 86_400);
        assert_eq!(escrow.epoch_id, 20_000);
        assert_eq!(escrow.epoch_spent, 40_000);
        // Next epoch: reset.
        escrow.roll_epoch(20_001 * 86_400 + 10, 86_400);
        assert_eq!(escrow.epoch_id, 20_001);
        assert_eq!(escrow.epoch_spent, 0);
    }

    #[test]
    fn expiry_semantics() {
        let mut escrow = fixture_escrow();
        assert!(!escrow.is_expired(i64::MAX)); // 0 = never
        escrow.expires_at = 100;
        assert!(!escrow.is_expired(99));
        assert!(escrow.is_expired(100));
    }
}
