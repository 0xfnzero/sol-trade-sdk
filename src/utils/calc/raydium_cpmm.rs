use crate::instruction::utils::raydium_cpmm::accounts::{
    CREATOR_FEE_RATE, FEE_RATE_DENOMINATOR_VALUE, FUND_FEE_RATE, PROTOCOL_FEE_RATE, TRADE_FEE_RATE,
};

/// Computes trading fee using ceiling division.
///
/// # Arguments
/// * `amount` - The amount to calculate fee for
/// * `fee_rate` - The fee rate to apply
///
/// # Returns
/// The calculated trading fee
#[inline(always)]
fn compute_trading_fee(amount: u64, fee_rate: u64) -> u64 {
    let numerator = (amount as u128) * (fee_rate as u128);
    ((numerator + FEE_RATE_DENOMINATOR_VALUE - 1) / FEE_RATE_DENOMINATOR_VALUE) as u64
}

/// Computes protocol or fund fee using floor division.
///
/// # Arguments
/// * `amount` - The amount to calculate fee for
/// * `fee_rate` - The fee rate to apply
///
/// # Returns
/// The calculated protocol or fund fee
#[inline(always)]
fn compute_protocol_fund_fee(amount: u64, fee_rate: u64) -> u64 {
    let numerator = (amount as u128) * (fee_rate as u128);
    (numerator / FEE_RATE_DENOMINATOR_VALUE) as u64
}

/// Computes creator fee using ceiling division.
///
/// # Arguments
/// * `amount` - The amount to calculate fee for
/// * `fee_rate` - The fee rate to apply
///
/// # Returns
/// The calculated creator fee
#[inline(always)]
fn compute_creator_fee_new(amount: u64, fee_rate: u64) -> u64 {
    let numerator = (amount as u128) * (fee_rate as u128);
    ((numerator + FEE_RATE_DENOMINATOR_VALUE - 1) / FEE_RATE_DENOMINATOR_VALUE) as u64
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
    /// The protocol fee charged
    pub protocol_fee: u64,
    /// The fund fee charged
    pub fund_fee: u64,
    /// The creator fee charged
    pub creator_fee: u64,
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
/// * `creator_fee_rate` - The creator fee rate
/// * `protocol_fee_rate` - The protocol fee rate
/// * `fund_fee_rate` - The fund fee rate
/// * `is_creator_fee_on_input` - Whether creator fee is charged on input tokens
///
/// # Returns
/// A `SwapResult` containing all swap calculations and fees
#[inline]
fn swap_base_input(
    input_amount: u64,
    input_vault_amount: u64,
    output_vault_amount: u64,
    trade_fee_rate: u64,
    creator_fee_rate: u64,
    protocol_fee_rate: u64,
    fund_fee_rate: u64,
    is_creator_fee_on_input: bool,
) -> SwapResult {
    let mut creator_fee = 0u64;

    let trade_fee = compute_trading_fee(input_amount, trade_fee_rate);

    let input_amount_less_fees = if is_creator_fee_on_input {
        creator_fee = compute_creator_fee_new(input_amount, creator_fee_rate);
        input_amount.saturating_sub(trade_fee).saturating_sub(creator_fee)
    } else {
        input_amount.saturating_sub(trade_fee)
    };

    let protocol_fee = compute_protocol_fund_fee(trade_fee, protocol_fee_rate);
    let fund_fee = compute_protocol_fund_fee(trade_fee, fund_fee_rate);

    let output_amount_swapped = ((output_vault_amount as u128)
        .saturating_mul(input_amount_less_fees as u128)
        / (input_vault_amount as u128).saturating_add(input_amount_less_fees as u128))
        as u64;

    let output_amount = if is_creator_fee_on_input {
        output_amount_swapped
    } else {
        creator_fee = compute_creator_fee_new(output_amount_swapped, creator_fee_rate);
        output_amount_swapped.saturating_sub(creator_fee)
    };

    SwapResult {
        new_input_vault_amount: input_vault_amount.saturating_add(input_amount_less_fees),
        new_output_vault_amount: output_vault_amount.saturating_sub(output_amount_swapped),
        input_amount,
        output_amount,
        trade_fee,
        protocol_fee,
        fund_fee,
        creator_fee,
    }
}

/// Computes swap parameters including amounts, fees, and slippage protection.
///
/// This function calculates the expected output amount, minimum output amount (with slippage),
/// and trading fees for a given input amount in a CPMM (Constant Product Market Maker) pool.
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
#[inline]
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
        TRADE_FEE_RATE,
        CREATOR_FEE_RATE,
        PROTOCOL_FEE_RATE,
        FUND_FEE_RATE,
        true,
    );

    let min_amount_out = ((swap_result.output_amount as f64)
        * (1.0 - (slippage_basis_points as f64) / 10000.0)) as u64;

    let all_trade = swap_result.input_amount == amount_in;

    ComputeSwapParams {
        all_trade,
        amount_in,
        amount_out: swap_result.output_amount,
        min_amount_out,
        fee: swap_result.trade_fee,
    }
}
