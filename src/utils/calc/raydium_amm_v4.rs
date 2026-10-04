use crate::instruction::utils::raydium_amm_v4::accounts::{
    SWAP_FEE_DENOMINATOR, SWAP_FEE_NUMERATOR, TRADE_FEE_DENOMINATOR, TRADE_FEE_NUMERATOR,
};
use crate::trading::core::params::RaydiumAmmV4Params;

use super::common::calculate_min_amount_out;

/// Computes trading fee using ceiling division.
///
/// # Arguments
/// * `amount` - The amount to calculate fee for
/// * `fee_rate` - The fee rate to apply
///
/// # Returns
/// The calculated trading fee
fn compute_trading_fee(amount: u64, fee_rate: u64, fee_denominator: u64) -> u64 {
    let numerator = (amount as u128) * (fee_rate as u128);
    ((numerator + fee_denominator as u128 - 1) / fee_denominator as u128) as u64
}

/// Computes protocol or fund fee using floor division.
///
/// # Arguments
/// * `amount` - The amount to calculate fee for
/// * `fee_rate` - The fee rate to apply
///
/// # Returns
/// The calculated protocol or fund fee
fn compute_protocol_fund_fee(amount: u64, fee_rate: u64, fee_denominator: u64) -> u64 {
    let numerator = (amount as u128) * (fee_rate as u128);
    (numerator / fee_denominator as u128) as u64
}

/// Parameters for computing swap amounts and fees.
#[derive(Debug, Clone)]
pub struct ComputeSwapParams {
    /// Whether the entire input amount is traded
    pub all_trade: bool,
    /// The input amount for the swap
    pub amount_in: u64,
    /// The expected output amount from the swap
    pub amount_out: u64,
    /// The minimum acceptable output amount (considering slippage_basis_points)
    pub min_amount_out: u64,
    /// The trading fee amount
    pub fee: u64,
}

/// Result of a swap calculation containing all relevant amounts and fees.
#[derive(Debug, Clone)]
pub struct SwapResult {
    /// The new amount in the input vault after the swap
    pub new_input_vault_amount: u64,
    /// The new amount in the output vault after the swap
    pub new_output_vault_amount: u64,
    /// The actual input amount used in the swap
    pub input_amount: u64,
    /// The actual output amount received from the swap
    pub output_amount: u64,
    /// The trading fee charged
    pub trade_fee: u64,
    /// The swap fee charged
    pub swap_fee: u64,
}

/// Performs a swap calculation based on input amount.
///
/// Calculates the output amount and all associated fees when swapping a specific input amount.
///
/// # Arguments
/// * `input_amount` - The amount of input tokens to swap
/// * `input_vault_amount` - Current amount in the input token vault
/// * `output_vault_amount` - Current amount in the output token vault
/// * `trade_fee_rate` - The trading fee rate
/// * `swap_fee_rate` - The swap fee rate
///
/// # Returns
/// A `SwapResult` containing all swap calculations and fees
fn swap_base_input(
    input_amount: u64,
    input_vault_amount: u64,
    output_vault_amount: u64,
    trade_fee_rate: u64,
    swap_fee_rate: u64,
) -> SwapResult {
    let trade_fee = compute_trading_fee(input_amount, trade_fee_rate, TRADE_FEE_DENOMINATOR);

    let input_amount_less_fees = input_amount.saturating_sub(trade_fee);

    let swap_fee = compute_protocol_fund_fee(trade_fee, swap_fee_rate, SWAP_FEE_DENOMINATOR);

    let output_amount_swapped = ((output_vault_amount as u128)
        .saturating_mul(input_amount_less_fees as u128)
        / (input_vault_amount as u128).saturating_add(input_amount_less_fees as u128))
        as u64;

    // Official Raydium AMM V4: trade fee is taken from input; do not subtract
    // input-denominated swap_fee from output units (matches router quote.rs).
    let output_amount = output_amount_swapped;

    SwapResult {
        new_input_vault_amount: input_vault_amount.saturating_add(input_amount_less_fees),
        new_output_vault_amount: output_vault_amount.saturating_sub(output_amount_swapped),
        input_amount,
        output_amount,
        trade_fee,
        swap_fee,
    }
}

/// Computes swap parameters including amounts, fees, and slippage protection.
///
/// This function calculates the expected output amount, minimum output amount (with slippage),
/// and trading fees for a given input amount in a Raydium AMM V4 pool.
///
/// # Arguments
/// * `base_reserve` - The current reserve amount of the base token in the pool
/// * `quote_reserve` - The current reserve amount of the quote token in the pool  
/// * `is_base_in` - Whether the input token is the base token (true) or quote token (false)
/// * `amount_in` - The amount of input tokens to swap
/// * `slippage_basis_points` - The acceptable slippage in basis points (e.g., 100 for 1%)
///
/// # Returns
/// A `ComputeSwapParams` struct containing all computed swap parameters
pub fn compute_swap_amount(
    base_reserve: u64,
    quote_reserve: u64,
    is_base_in: bool,
    amount_in: u64,
    slippage_basis_points: u64,
) -> ComputeSwapParams {
    let (input_reserve, output_reserve) =
        if is_base_in { (base_reserve, quote_reserve) } else { (quote_reserve, base_reserve) };

    let swap_result = swap_base_input(
        amount_in,
        input_reserve,
        output_reserve,
        TRADE_FEE_NUMERATOR,
        SWAP_FEE_NUMERATOR,
    );

    let min_amount_out = calculate_min_amount_out(swap_result.output_amount, slippage_basis_points);

    let all_trade = swap_result.input_amount == amount_in;

    ComputeSwapParams {
        all_trade,
        amount_in,
        amount_out: swap_result.output_amount,
        min_amount_out,
        fee: swap_result.trade_fee,
    }
}

/// Output of the program's `swap_base_in`: the swap fee
/// (`swap_fee_numerator / swap_fee_denominator` of the input, rounded up) comes off
/// the input, then constant product on the reserves. `None` when the pool cannot
/// fill it (zero denominator, or a fee that eats the input).
pub fn swap_base_in_amount_out(
    amount_in: u64,
    input_reserve: u64,
    output_reserve: u64,
    swap_fee_numerator: u64,
    swap_fee_denominator: u64,
) -> Option<u64> {
    if swap_fee_denominator == 0 {
        return None;
    }
    let fee = (u128::from(amount_in) * u128::from(swap_fee_numerator))
        .div_ceil(u128::from(swap_fee_denominator));
    let amount_in_after_fee = u128::from(amount_in).checked_sub(fee)?;
    let out = u128::from(output_reserve) * amount_in_after_fee
        / (u128::from(input_reserve) + amount_in_after_fee);
    u64::try_from(out).ok().filter(|out| *out > 0)
}

/// Swap parameters for `amount_in` through `pool` (the v2, no-orderbook swap), at
/// the pool's own swap fee and the reserves it holds (vaults net of
/// `need_take_pnl` when loaded by RPC or from a cache).
pub fn compute_swap_amount_for_pool(
    pool: &RaydiumAmmV4Params,
    is_coin_in: bool,
    amount_in: u64,
    slippage_basis_points: u64,
) -> Result<ComputeSwapParams, anyhow::Error> {
    anyhow::ensure!(
        pool.swap_fee_denominator > 0 && pool.swap_fee_numerator < pool.swap_fee_denominator,
        "Invalid AMM v4 swap fee"
    );
    let (input_reserve, output_reserve) = if is_coin_in {
        (pool.coin_reserve, pool.pc_reserve)
    } else {
        (pool.pc_reserve, pool.coin_reserve)
    };
    anyhow::ensure!(input_reserve > 0 && output_reserve > 0, "AMM v4 reserves are empty");
    let amount_out = swap_base_in_amount_out(
        amount_in,
        input_reserve,
        output_reserve,
        pool.swap_fee_numerator,
        pool.swap_fee_denominator,
    )
    .ok_or_else(|| anyhow::anyhow!("AMM v4 pool {} cannot fill {amount_in}", pool.amm))?;
    let fee = (u128::from(amount_in) * u128::from(pool.swap_fee_numerator))
        .div_ceil(u128::from(pool.swap_fee_denominator)) as u64;
    Ok(ComputeSwapParams {
        all_trade: true,
        amount_in,
        amount_out,
        min_amount_out: calculate_min_amount_out(amount_out, slippage_basis_points),
        fee,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::calc::common::{calculate_min_amount_out, MAX_SLIPPAGE_BASIS_POINTS};
    use solana_sdk::pubkey::Pubkey;

    #[test]
    fn swap_base_in_takes_the_fee_off_the_input_rounded_up() {
        // fee = ceil(1_000_001 * 25 / 10_000) = 2_501, so 997_500 is swapped.
        let out = swap_base_in_amount_out(1_000_001, 50_000_000_000, 7_000_000_000, 25, 10_000);
        let expected = 7_000_000_000u128 * 997_500 / (50_000_000_000u128 + 997_500);
        assert_eq!(out, Some(expected as u64));
    }

    #[test]
    fn swap_base_in_charges_the_pools_own_fee() {
        let standard = swap_base_in_amount_out(1_000_000, 10_000_000, 10_000_000, 25, 10_000);
        let dearer = swap_base_in_amount_out(1_000_000, 10_000_000, 10_000_000, 100, 10_000);
        assert!(dearer < standard);
        assert_eq!(swap_base_in_amount_out(1, 10_000, 10_000, 25, 0), None);
    }

    #[test]
    fn pool_quotes_follow_the_direction_and_the_pool_fee() {
        let pool = RaydiumAmmV4Params::new(
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            4_000_000,
            9_000_000,
        )
        .with_swap_fee(30, 10_000);
        let coin_in = compute_swap_amount_for_pool(&pool, true, 10_000, 100).unwrap();
        assert_eq!(
            Some(coin_in.amount_out),
            swap_base_in_amount_out(10_000, 4_000_000, 9_000_000, 30, 10_000)
        );
        assert_eq!(coin_in.min_amount_out, calculate_min_amount_out(coin_in.amount_out, 100));
        let pc_in = compute_swap_amount_for_pool(&pool, false, 10_000, 0).unwrap();
        assert_eq!(
            Some(pc_in.amount_out),
            swap_base_in_amount_out(10_000, 9_000_000, 4_000_000, 30, 10_000)
        );
    }

    #[test]
    fn min_amount_out_uses_exact_integer_slippage() {
        let no_slippage = compute_swap_amount(u64::MAX, u64::MAX, true, u64::MAX, 0);
        let one_percent = compute_swap_amount(u64::MAX, u64::MAX, true, u64::MAX, 100);

        assert_eq!(one_percent.amount_out, no_slippage.amount_out);
        assert_eq!(
            one_percent.min_amount_out,
            calculate_min_amount_out(one_percent.amount_out, 100)
        );
    }

    #[test]
    fn min_amount_out_rounds_down_after_applying_slippage() {
        assert_eq!(calculate_min_amount_out(101, 100), 99);
    }

    #[test]
    fn excessive_slippage_is_clamped_without_underflow() {
        let excessive = compute_swap_amount(1_000_000, 2_000_000, true, 100_000, u64::MAX);
        let clamped =
            compute_swap_amount(1_000_000, 2_000_000, true, 100_000, MAX_SLIPPAGE_BASIS_POINTS);

        assert_eq!(excessive.min_amount_out, clamped.min_amount_out);
        assert_eq!(
            excessive.min_amount_out,
            calculate_min_amount_out(excessive.amount_out, MAX_SLIPPAGE_BASIS_POINTS)
        );
    }
}
