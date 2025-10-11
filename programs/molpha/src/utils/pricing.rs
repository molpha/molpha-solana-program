use crate::state::{Feed, ProtocolConfig};
use anchor_lang::prelude::*;

pub fn calculate_price_per_second_scaled(feed: &Feed, config: &ProtocolConfig) -> Result<u64> {
    let frequency = feed.frequency;
    let signatures_required = feed.min_signatures_threshold as u64;

    // Calculate updates per day for frequency scaling
    let updates_per_day = 86400 / frequency; // 86400 seconds in a day

    // Calculate frequency factor: updatesPerDay^(frequencyCoefficient/10000)
    let frequency_factor = precise_pow(
        updates_per_day,
        config.frequency_coefficient,
        10000,
        ProtocolConfig::SCALAR,
    )?;

    // Calculate signers factor: signaturesRequired^(signersCoefficient/10000)
    let signers_factor = precise_pow(
        signatures_required,
        config.signers_coefficient,
        10000,
        ProtocolConfig::SCALAR,
    )?;

    // Calculate base price with priority fee buffer
    let base_price = config
        .base_price_per_second_scaled
        .checked_mul(frequency_factor)
        .and_then(|x| x.checked_mul(signers_factor))
        .and_then(|x| x.checked_div(ProtocolConfig::SCALAR))
        .and_then(|x| x.checked_div(ProtocolConfig::SCALAR))
        .ok_or(error!(ErrorCode::ArithmeticError))?;

    Ok(base_price)
}

pub fn calculate_priority_fee_from_instructions(
    instructions_sysvar: &UncheckedAccount,
    estimated_compute_units: u32,
) -> Result<u64> {
    let instructions_account = instructions_sysvar.to_account_info();
    let current_index =
        anchor_lang::solana_program::sysvar::instructions::load_current_index_checked(
            &instructions_account,
        )?;

    for i in 0..current_index {
        let instruction =
            anchor_lang::solana_program::sysvar::instructions::load_instruction_at_checked(
                i as usize,
                &instructions_account,
            )?;

        // Check for ComputeBudgetProgram ID (ComputeBudgetProgram11111111111111111111111111)
        let compute_budget_program_id =
            anchor_lang::solana_program::pubkey!("ComputeBudget111111111111111111111111111111");
        if instruction.program_id == compute_budget_program_id {
            if let Ok(microlamports_per_cu) = parse_compute_unit_price(&instruction.data) {
                let priority_fee =
                    (microlamports_per_cu as u64 * estimated_compute_units as u64) / 1_000_000;
                return Ok(priority_fee);
            }
        }
    }

    Ok(0) // No priority fee instruction found
}

pub fn estimate_compute_units(node_count: u32, history_length: u32) -> u32 {
    let base_units = 5000;
    let signature_units = node_count * 1000;
    let history_units = if (history_length as usize) < crate::state::feed::MAX_HISTORY {
        500
    } else {
        200
    };

    base_units + signature_units + history_units
}

// Simplified power function for pricing calculations
fn precise_pow(base: u64, numerator: u64, denominator: u64, scalar: u64) -> Result<u64> {
    if numerator == denominator {
        return Ok(base);
    }

    // Simplified approximation for small exponents
    // For production, you'd want a more sophisticated implementation
    let exponent_scaled = (numerator * scalar) / denominator;

    if exponent_scaled <= scalar {
        // Exponent <= 1, use linear approximation
        Ok(base)
    } else {
        // Exponent > 1, use quadratic approximation
        let factor = exponent_scaled / scalar;
        base.checked_mul(factor)
            .ok_or(error!(ErrorCode::ArithmeticError))
    }
}

fn parse_compute_unit_price(data: &[u8]) -> Result<u32> {
    // Parse SetComputeUnitPrice instruction
    if data.len() >= 9 && data[0] == 3 {
        // SetComputeUnitPrice discriminator
        let microlamports = u64::from_le_bytes([
            data[1], data[2], data[3], data[4], data[5], data[6], data[7], data[8],
        ]);
        Ok(microlamports as u32)
    } else {
        Err(error!(ErrorCode::InvalidInstruction))
    }
}

#[error_code]
pub enum ErrorCode {
    #[msg("Arithmetic error occurred")]
    ArithmeticError,
    #[msg("Invalid instruction format")]
    InvalidInstruction,
}
