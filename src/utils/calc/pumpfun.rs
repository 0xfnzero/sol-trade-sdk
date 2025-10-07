use solana_sdk::{native_token::sol_str_to_lamports, pubkey::Pubkey};

use crate::{
    instruction::utils::pumpfun::global_constants::{CREATOR_FEE, FEE_BASIS_POINTS},
    utils::calc::common::compute_fee,
};

/// Calculates the amount of tokens that can be purchased with a given SOL amount
/// using the bonding curve formula.
///
/// # Arguments
/// * `virtual_token_reserves` - Virtual token reserves in the bonding curve
/// * `virtual_sol_reserves` - Virtual SOL reserves in the bonding curve
/// * `real_token_reserves` - Actual token reserves available for purchase
/// * `creator` - Creator's public key (affects fee calculation)
/// * `amount` - SOL amount to spend (in lamports)
///
/// # Returns
/// The amount of tokens that will be received (in token's smallest unit)
#[inline]
pub fn get_buy_token_amount_from_sol_amount(
    virtual_token_reserves: u128,
    virtual_sol_reserves: u128,
    real_token_reserves: u128,
    creator: Pubkey,
    amount: u64,
) -> u64 {
    if amount == 0 {
        return 0;
    }

    if virtual_token_reserves == 0 {
        return 0;
    }

    let total_fee_basis_points =
        FEE_BASIS_POINTS + if creator != Pubkey::default() { CREATOR_FEE } else { 0 };

    // Convert to u128 to prevent overflow
    let amount_128 = amount as u128;
    let total_fee_basis_points_128 = total_fee_basis_points as u128;

    let input_amount = amount_128
        .checked_mul(10_000)
        .unwrap()
        .checked_div(total_fee_basis_points_128 + 10_000)
        .unwrap();

    let denominator = virtual_sol_reserves + input_amount;

    let mut tokens_received =
        input_amount.checked_mul(virtual_token_reserves).unwrap().checked_div(denominator).unwrap();

    tokens_received = tokens_received.min(real_token_reserves);

    if tokens_received <= 100 * 1_000_000_u128 {
        tokens_received = if amount > sol_str_to_lamports("0.01").unwrap_or(0) {
            25547619 * 1_000_000_u128
        } else {
            255476 * 1_000_000_u128
        };
    }

    tokens_received as u64
}

/// Calculates the amount of SOL that will be received when selling a given token amount
/// using the bonding curve formula with transaction fees deducted.
///
/// # Arguments
/// * `virtual_token_reserves` - Virtual token reserves in the bonding curve
/// * `virtual_sol_reserves` - Virtual SOL reserves in the bonding curve
/// * `creator` - Creator's public key (affects fee calculation)
/// * `amount` - Token amount to sell (in token's smallest unit)
///
/// # Returns
/// The amount of SOL that will be received after fees (in lamports)
#[inline]
pub fn get_sell_sol_amount_from_token_amount(
    virtual_token_reserves: u128,
    virtual_sol_reserves: u128,
    creator: Pubkey,
    amount: u64,
) -> u64 {
    if amount == 0 {
        return 0;
    }

    // migrated bonding curve
    if virtual_token_reserves == 0 {
        return 0;
    }

    let amount_128 = amount as u128;

    // Calculate SOL amount received from selling tokens using constant product formula
    let numerator = amount_128.checked_mul(virtual_sol_reserves).unwrap_or(0);
    let denominator = virtual_token_reserves.checked_add(amount_128).unwrap_or(1);

    let sol_cost = numerator.checked_div(denominator).unwrap_or(0);

    let total_fee_basis_points =
        FEE_BASIS_POINTS + if creator != Pubkey::default() { CREATOR_FEE } else { 0 };
    let total_fee_basis_points_128 = total_fee_basis_points as u128;

    // Calculate transaction fee
    let fee = compute_fee(sol_cost, total_fee_basis_points_128);

    sol_cost.saturating_sub(fee) as u64
}
