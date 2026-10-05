use crate::instruction::utils::raydium_cpmm::accounts::{
    CREATOR_FEE_RATE, FEE_RATE_DENOMINATOR_VALUE, FUND_FEE_RATE, PROTOCOL_FEE_RATE, TRADE_FEE_RATE,
};

use super::common::calculate_min_amount_out;
use crate::trading::core::params::RaydiumCpmmParams;

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
    let trade_fee: u64;

    let input_amount_less_fees = if is_creator_fee_on_input {
        let total_fee_rate = trade_fee_rate.saturating_add(creator_fee_rate);
        let total_fee = compute_trading_fee(input_amount, total_fee_rate);
        creator_fee = if total_fee_rate == 0 {
            0
        } else {
            ((total_fee as u128) * (creator_fee_rate as u128) / (total_fee_rate as u128)) as u64
        };
        trade_fee = total_fee.saturating_sub(creator_fee);
        input_amount.saturating_sub(total_fee)
    } else {
        trade_fee = compute_trading_fee(input_amount, trade_fee_rate);
        input_amount.saturating_sub(trade_fee)
    };

    let protocol_fee = compute_protocol_fund_fee(trade_fee, protocol_fee_rate);
    let fund_fee = compute_protocol_fund_fee(trade_fee, fund_fee_rate);

    // The legacy infallible API represents an empty pool as a zero quote;
    // the validated pool API rejects it before reaching this calculation.
    let denominator = u128::from(input_vault_amount) + u128::from(input_amount_less_fees);
    let output_amount_swapped = (u128::from(output_vault_amount)
        * u128::from(input_amount_less_fees))
    .checked_div(denominator)
    .unwrap_or(0) as u64;

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

/// Computes an exact-input quote using the current on-chain CPMM config,
/// accrued-fee-adjusted reserves, creator fee mode, and Token-2022 fees.
pub fn compute_swap_amount_for_pool(
    protocol_params: &RaydiumCpmmParams,
    is_base_in: bool,
    amount_in: u64,
    slippage_basis_points: u64,
) -> Result<ComputeSwapParams, anyhow::Error> {
    if amount_in == 0 || protocol_params.base_reserve == 0 || protocol_params.quote_reserve == 0 {
        return Err(anyhow::anyhow!("Raydium CPMM amount and pool reserves must be nonzero"));
    }
    let creator_fee_rate =
        if protocol_params.enable_creator_fee { protocol_params.creator_fee_rate } else { 0 };
    let total_input_fee_rate = protocol_params
        .trade_fee_rate
        .checked_add(creator_fee_rate)
        .ok_or_else(|| anyhow::anyhow!("Raydium CPMM fee rate overflow"))?;
    if protocol_params.trade_fee_rate > FEE_RATE_DENOMINATOR_VALUE as u64
        || creator_fee_rate > FEE_RATE_DENOMINATOR_VALUE as u64
        || total_input_fee_rate >= FEE_RATE_DENOMINATOR_VALUE as u64
        || protocol_params.protocol_fee_rate > FEE_RATE_DENOMINATOR_VALUE as u64
        || protocol_params.fund_fee_rate > FEE_RATE_DENOMINATOR_VALUE as u64
        || !protocol_params
            .protocol_fee_rate
            .checked_add(protocol_params.fund_fee_rate)
            .is_some_and(|rate| rate <= FEE_RATE_DENOMINATOR_VALUE as u64)
        || protocol_params.base_transfer_fee.basis_points > 10_000
        || protocol_params.quote_transfer_fee.basis_points > 10_000
    {
        return Err(anyhow::anyhow!("Invalid Raydium CPMM fee configuration"));
    }
    let (input_reserve, output_reserve, input_transfer_fee, output_transfer_fee) = if is_base_in {
        (
            protocol_params.base_reserve,
            protocol_params.quote_reserve,
            protocol_params.base_transfer_fee,
            protocol_params.quote_transfer_fee,
        )
    } else {
        (
            protocol_params.quote_reserve,
            protocol_params.base_reserve,
            protocol_params.quote_transfer_fee,
            protocol_params.base_transfer_fee,
        )
    };
    let is_creator_fee_on_input = match protocol_params.creator_fee_on {
        0 => true,
        1 => is_base_in,
        2 => !is_base_in,
        value => return Err(anyhow::anyhow!("Invalid Raydium CPMM creator fee mode: {}", value)),
    };
    let actual_amount_in = amount_in
        .checked_sub(input_transfer_fee.calculate(amount_in))
        .filter(|amount| *amount > 0)
        .ok_or_else(|| anyhow::anyhow!("Raydium CPMM transfer fee consumes the input"))?;
    // The curve uses u128, but SPL token account balances are u64. Reserves
    // exclude accrued fees and are only a lower bound on the vault balance:
    // overflow here necessarily makes the incoming transfer impossible. Use
    // the received amount, before swap fees, rather than the gross transfer.
    input_reserve
        .checked_add(actual_amount_in)
        .ok_or_else(|| anyhow::anyhow!("Raydium CPMM input vault balance overflow"))?;
    let swap_result = swap_base_input(
        actual_amount_in,
        input_reserve,
        output_reserve,
        protocol_params.trade_fee_rate,
        creator_fee_rate,
        protocol_params.protocol_fee_rate,
        protocol_params.fund_fee_rate,
        is_creator_fee_on_input,
    );
    let received_amount = swap_result
        .output_amount
        .saturating_sub(output_transfer_fee.calculate(swap_result.output_amount));

    if received_amount == 0 {
        return Err(anyhow::anyhow!("Raydium CPMM swap produces zero output"));
    }
    Ok(ComputeSwapParams {
        all_trade: true,
        amount_in,
        amount_out: received_amount,
        min_amount_out: calculate_min_amount_out(received_amount, slippage_basis_points),
        fee: swap_result.trade_fee,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::calc::common::{calculate_min_amount_out, MAX_SLIPPAGE_BASIS_POINTS};

    fn pool_params() -> RaydiumCpmmParams {
        use solana_sdk::pubkey::Pubkey;
        RaydiumCpmmParams::from_trade(
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            crate::constants::TOKEN_PROGRAM,
            crate::constants::TOKEN_PROGRAM,
            Pubkey::new_unique(),
            1_000_000,
            2_000_000,
        )
    }

    #[test]
    fn empty_legacy_quote_does_not_panic_and_validated_quotes_reject_empty_inputs() {
        assert_eq!(compute_swap_amount(0, 0, true, 0, 0).amount_out, 0);
        assert_eq!(compute_swap_amount(0, 100, true, 1, 0).amount_out, 0);
        for base_in in [false, true] {
            for (base, quote, amount) in [(0, 100, 100), (100, 0, 100), (100, 100, 0)] {
                let mut params = pool_params();
                params.base_reserve = base;
                params.quote_reserve = quote;
                assert!(compute_swap_amount_for_pool(&params, base_in, amount, 0).is_err());
            }
            assert!(compute_swap_amount_for_pool(&pool_params(), base_in, 1, 0).is_err());
        }
    }

    #[test]
    fn validated_quotes_reject_invalid_fee_rates_and_fully_taxed_input() {
        use crate::trading::core::params::TokenTransferFee;
        let mut params = pool_params();
        params.trade_fee_rate = 1_000_000;
        assert!(compute_swap_amount_for_pool(&params, true, 100, 0).is_err());
        params = pool_params();
        params.protocol_fee_rate = 600_000;
        params.fund_fee_rate = 500_000;
        assert!(compute_swap_amount_for_pool(&params, true, 100, 0).is_err());
        params = pool_params();
        params.base_transfer_fee = TokenTransferFee { basis_points: 10_000, maximum_fee: 100 };
        assert!(compute_swap_amount_for_pool(&params, true, 100, 0).is_err());
        // A capped 100% transfer fee can still leave a valid positive input.
        assert!(compute_swap_amount_for_pool(&params, true, 1000, 0).is_ok());
        params.quote_transfer_fee.basis_points = 10_001;
        assert!(compute_swap_amount_for_pool(&params, true, 1000, 0).is_err());
        let quote = compute_swap_amount_for_pool(&pool_params(), true, 1000, 10_000).unwrap();
        assert!(quote.amount_out > 0);
        assert_eq!(quote.min_amount_out, 0);
    }

    #[test]
    fn validated_quote_checks_received_input_balance_capacity_in_both_directions() {
        use crate::trading::core::params::TokenTransferFee;
        for base_in in [false, true] {
            let mut params = pool_params();
            params.base_reserve = u64::MAX;
            params.quote_reserve = u64::MAX;
            let error = compute_swap_amount_for_pool(&params, base_in, 1_000, 0).unwrap_err();
            assert!(error.to_string().contains("input vault balance overflow"));
            // Even when the curve's post-swap-fee input fits, the token vault
            // receives the full post-transfer-fee input before booking fees.
            if base_in {
                params.base_reserve = u64::MAX - 999;
            } else {
                params.quote_reserve = u64::MAX - 999;
            }
            assert!(compute_swap_amount_for_pool(&params, base_in, 1_000, 0).is_err());
            let fee = TokenTransferFee { basis_points: 1_000, maximum_fee: 100 };
            if base_in {
                params.base_reserve = u64::MAX - 900;
                params.base_transfer_fee = fee;
            } else {
                params.quote_reserve = u64::MAX - 900;
                params.quote_transfer_fee = fee;
            }
            // Gross input overflows reserve + input, but the net 900 fits exactly.
            assert!(compute_swap_amount_for_pool(&params, base_in, 1_000, 0).is_ok());
            if base_in {
                params.base_reserve += 1;
            } else {
                params.quote_reserve += 1;
            }
            assert!(compute_swap_amount_for_pool(&params, base_in, 1_000, 0).is_err());
        }
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

    #[test]
    fn creator_fee_on_input_matches_official_combined_fee_rounding() {
        let result = swap_base_input(101, 1_000_000, 2_000_000, 2_500, 10_000, 0, 0, true);

        // ceil(101 * 12_500 / 1_000_000) is 2. Splitting that combined fee
        // yields creator=1 and trade=1; separately ceiling each fee would charge 3.
        assert_eq!(result.creator_fee, 1);
        assert_eq!(result.trade_fee, 1);
        assert_eq!(result.new_input_vault_amount, 1_000_099);
    }
}
