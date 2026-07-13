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

/// Agent entitlement + funding + policy, one account (spec v1 §4.1, minimal Phase-1 shape).
///
/// `owner` (cold) controls funds and policy; `authority` (hot runtime key) only signs
/// `AgentRequestAuth` messages. Funds are held as **real USDC in the escrow's own ATA**
/// (`ATA(escrow_pda, usdc_mint)`); the escrow PDA signs the transfer to treasury at
/// settlement. There is no internal balance counter — the ATA balance is authoritative.
///
/// The on-chain caps (`max_signers`, `epoch_cap`, `expires_at`) are the authoritative
/// backstop bounding a compromised hot key; per-request rate limiting and template/scope
/// policy are enforced off-chain by the gateway (the "no on-chain preflight" contract).
///
/// x402-provisioned escrows set `owner == authority == payer` with all caps `0`, and are
/// lazily created by `settle_agent_round` (the payer never signs a Molpha tx). The x402
/// payment into the escrow ATA is the authorization; replay is guarded by the round-receipt
/// PDA. `owner == authority` is the settlement-time discriminator that skips the
/// `AgentRequestAuth` check and all caps.
#[account]
#[derive(InitSpace)]
pub struct AgentEscrow {
    /// Cold key: policy + funds control. Also a seed component.
    pub owner: Pubkey,
    /// Hot key: signs `AgentRequestAuth` only. Also a seed component (so it cannot be rotated
    /// in place). For x402 escrows `authority == owner == payer`.
    pub authority: Pubkey,
    pub status: AgentStatus,
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
            status: AgentStatus::Active,
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
