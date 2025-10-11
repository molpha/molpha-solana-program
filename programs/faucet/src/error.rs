use anchor_lang::prelude::*;

#[error_code]
pub enum FaucetError {
    #[msg("The faucet is currently inactive")]
    FaucetInactive,

    #[msg("Cooldown period is still active. Please wait before requesting again")]
    CooldownActive,

    #[msg("Unauthorized access. Only the faucet authority can perform this action")]
    UnauthorizedAccess,

    #[msg("Insufficient funds in the faucet")]
    InsufficientFunds,
}
