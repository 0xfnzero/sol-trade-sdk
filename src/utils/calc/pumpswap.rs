use super::common::{
    calculate_with_slippage_buy, calculate_with_slippage_sell, ceil_div, compute_fee,
};
use crate::instruction::utils::pumpswap::accounts::{
    COIN_CREATOR_FEE_BASIS_POINTS, LP_FEE_BASIS_POINTS, PROTOCOL_FEE_BASIS_POINTS,
};
use crate::instruction::utils::pumpswap::PumpSwapFeeBasisPoints;
use solana_sdk::pubkey::Pubkey;

#[inline]
fn effective_quote_reserve(
    quote_reserve: u64,
    virtual_quote_reserves: i128,
) -> Result<u64, String> {
    crate::instruction::utils::pumpswap_types::effective_quote_reserves(
        quote_reserve,
        virtual_quote_reserves,
    )
    .filter(|reserve| *reserve != 0)
    .ok_or_else(|| {
        format!(
            "Invalid effective quote reserves: raw={quote_reserve}, virtual={virtual_quote_reserves}."
        )
    })
}

/// Creator-side fee bps: fixed coin-creator fee when a creator vault applies, plus optional
/// cashback fee bps for cashback-enabled coins (see Pump AMM / parser event field).
#[inline]
pub(crate) fn creator_side_fee_basis_points(
    coin_creator: &Pubkey,
    cashback_fee_basis_points: u64,
) -> u64 {
    let creator_bps =
        if *coin_creator == Pubkey::default() { 0 } else { COIN_CREATOR_FEE_BASIS_POINTS };
    creator_bps.saturating_add(cashback_fee_basis_points)
}

/// Result for buying base tokens with base amount input
#[derive(Clone, Debug)]
pub struct BuyBaseInputResult {
    /// Raw quote amount needed before fees
    pub internal_quote_amount: u64,
    /// Total quote amount including all fees
    pub ui_quote: u64,
    /// Maximum quote amount with slippage protection
    pub max_quote: u64,
}

/// Result for buying base tokens with quote amount input
#[derive(Clone, Debug)]
pub struct BuyQuoteInputResult {
    /// Amount of base tokens received
    pub base: u64,
    /// Effective quote amount after fee deduction
    pub internal_quote_without_fees: u64,
    /// Maximum quote amount with slippage protection
    pub max_quote: u64,
}

/// Result for selling base tokens with base amount input
#[derive(Clone, Debug)]
pub struct SellBaseInputResult {
    /// Final quote amount received after fees
    pub ui_quote: u64,
    /// Minimum quote amount with slippage protection
    pub min_quote: u64,
    /// Raw quote amount before fee deduction
    pub internal_quote_amount_out: u64,
}

/// Result for selling base tokens with quote amount input
#[derive(Clone, Debug)]
pub struct SellQuoteInputResult {
    /// Raw quote amount including fees
    pub internal_raw_quote: u64,
    /// Amount of base tokens needed to sell
    pub base: u64,
    /// Minimum quote amount with slippage protection
    pub min_quote: u64,
}

/// Calculate quote amount needed to buy a specific amount of base tokens
///
/// # Arguments
/// * `base` - Amount of base tokens to buy
/// * `slippage_basis_points` - Slippage tolerance in basis points (100 = 1%)
/// * `base_reserve` - Base token reserves in the pool
/// * `quote_reserve` - Raw quote-vault balance
/// * `virtual_quote_reserves` - Signed virtual quote reserves from the same pool snapshot
/// * `coin_creator` - Token creator address
/// * `cashback_fee_basis_points` - Extra fee bps for cashback coins (from on-chain / events); use `0` if unknown
///
/// # Returns
/// * `BuyBaseInputResult` containing quote amounts and slippage calculations
pub fn buy_base_input_internal(
    base: u64,
    slippage_basis_points: u64,
    base_reserve: u64,
    quote_reserve: u64,
    virtual_quote_reserves: i128,
    coin_creator: &Pubkey,
    cashback_fee_basis_points: u64,
) -> Result<BuyBaseInputResult, String> {
    buy_base_input_internal_with_fees(
        base,
        slippage_basis_points,
        base_reserve,
        quote_reserve,
        virtual_quote_reserves,
        &PumpSwapFeeBasisPoints::new(
            LP_FEE_BASIS_POINTS,
            PROTOCOL_FEE_BASIS_POINTS,
            creator_side_fee_basis_points(coin_creator, cashback_fee_basis_points),
        ),
    )
}

pub fn buy_base_input_internal_with_fees(
    base: u64,
    slippage_basis_points: u64,
    base_reserve: u64,
    quote_reserve: u64,
    virtual_quote_reserves: i128,
    fee_basis_points: &PumpSwapFeeBasisPoints,
) -> Result<BuyBaseInputResult, String> {
    if base_reserve == 0 || quote_reserve == 0 {
        return Err("Invalid input: 'baseReserve' or 'quoteReserve' cannot be zero.".to_string());
    }
    let effective_quote_reserve = effective_quote_reserve(quote_reserve, virtual_quote_reserves)?;
    if base > base_reserve {
        return Err("Cannot buy more base tokens than the pool reserves.".to_string());
    }

    // Calculate required quote amount using constant product formula
    let numerator = (effective_quote_reserve as u128) * (base as u128);
    let denominator = base_reserve - base;

    if denominator == 0 {
        return Err("Pool would be depleted; denominator is zero.".to_string());
    }

    let quote_amount_in = ceil_div(numerator, denominator as u128) as u64;

    // Calculate fees
    let lp_fee =
        compute_fee(quote_amount_in as u128, fee_basis_points.lp_fee_basis_points as u128) as u64;
    let protocol_fee =
        compute_fee(quote_amount_in as u128, fee_basis_points.protocol_fee_basis_points as u128)
            as u64;
    let coin_creator_fee = compute_fee(
        quote_amount_in as u128,
        fee_basis_points.coin_creator_fee_basis_points as u128,
    ) as u64;
    let total_quote = quote_amount_in + lp_fee + protocol_fee + coin_creator_fee;

    // Calculate max quote with slippage
    let max_quote = calculate_with_slippage_buy(total_quote, slippage_basis_points);

    Ok(BuyBaseInputResult {
        internal_quote_amount: quote_amount_in,
        ui_quote: total_quote,
        max_quote,
    })
}

/// Calculate base tokens received for a specific quote amount
///
/// # Arguments
/// * `quote` - Amount of quote tokens to spend
/// * `slippage_basis_points` - Slippage tolerance in basis points (100 = 1%)
/// * `base_reserve` - Base token reserves in the pool
/// * `quote_reserve` - Raw quote-vault balance
/// * `virtual_quote_reserves` - Signed virtual quote reserves from the same pool snapshot
/// * `coin_creator` - Token creator address
/// * `cashback_fee_basis_points` - Extra fee bps for cashback coins; use `0` if unknown
///
/// # Returns
/// * `BuyQuoteInputResult` containing base amount and slippage calculations
pub fn buy_quote_input_internal(
    quote: u64,
    slippage_basis_points: u64,
    base_reserve: u64,
    quote_reserve: u64,
    virtual_quote_reserves: i128,
    coin_creator: &Pubkey,
    cashback_fee_basis_points: u64,
) -> Result<BuyQuoteInputResult, String> {
    buy_quote_input_internal_with_fees(
        quote,
        slippage_basis_points,
        base_reserve,
        quote_reserve,
        virtual_quote_reserves,
        &PumpSwapFeeBasisPoints::new(
            LP_FEE_BASIS_POINTS,
            PROTOCOL_FEE_BASIS_POINTS,
            creator_side_fee_basis_points(coin_creator, cashback_fee_basis_points),
        ),
    )
}

pub fn buy_quote_input_internal_with_fees(
    quote: u64,
    slippage_basis_points: u64,
    base_reserve: u64,
    quote_reserve: u64,
    virtual_quote_reserves: i128,
    fee_basis_points: &PumpSwapFeeBasisPoints,
) -> Result<BuyQuoteInputResult, String> {
    if base_reserve == 0 || quote_reserve == 0 {
        return Err("Invalid input: 'baseReserve' or 'quoteReserve' cannot be zero.".to_string());
    }
    let effective_quote_reserve = effective_quote_reserve(quote_reserve, virtual_quote_reserves)?;

    // Calculate total fee basis points
    let total_fee_bps = fee_basis_points
        .lp_fee_basis_points
        .saturating_add(fee_basis_points.protocol_fee_basis_points)
        .saturating_add(fee_basis_points.coin_creator_fee_basis_points);
    let denominator = 10_000 + total_fee_bps;

    // Calculate effective quote amount after fees
    let mut effective_quote = (quote as u128 * 10_000) / denominator as u128;
    let lp_fee = compute_fee(effective_quote, fee_basis_points.lp_fee_basis_points as u128);
    let protocol_fee =
        compute_fee(effective_quote, fee_basis_points.protocol_fee_basis_points as u128);
    let coin_creator_fee =
        compute_fee(effective_quote, fee_basis_points.coin_creator_fee_basis_points as u128);
    let total_with_fees = effective_quote + lp_fee + protocol_fee + coin_creator_fee;
    if total_with_fees > quote as u128 {
        effective_quote = effective_quote.saturating_sub(total_with_fees - quote as u128);
    }
    let input_amount = effective_quote.saturating_sub(1);

    // Calculate base amount out using constant product formula
    let numerator = (base_reserve as u128) * input_amount;
    let denominator_effective = (effective_quote_reserve as u128) + input_amount;

    if denominator_effective == 0 {
        return Err("Pool would be depleted; denominator is zero.".to_string());
    }

    let base_amount_out = (numerator / denominator_effective) as u64;

    // Calculate max quote with slippage
    let max_quote = calculate_with_slippage_buy(quote, slippage_basis_points);

    Ok(BuyQuoteInputResult {
        base: base_amount_out,
        internal_quote_without_fees: effective_quote as u64,
        max_quote,
    })
}

/// Calculate quote tokens received for selling a specific amount of base tokens
///
/// # Arguments
/// * `base` - Amount of base tokens to sell
/// * `slippage_basis_points` - Slippage tolerance in basis points (100 = 1%)
/// * `base_reserve` - Base token reserves in the pool
/// * `quote_reserve` - Raw quote-vault balance
/// * `virtual_quote_reserves` - Signed virtual quote reserves from the same pool snapshot
/// * `coin_creator` - Token creator address
/// * `cashback_fee_basis_points` - Extra fee bps for cashback coins; use `0` if unknown
///
/// # Returns
/// * `SellBaseInputResult` containing quote amounts and slippage calculations
pub fn sell_base_input_internal(
    base: u64,
    slippage_basis_points: u64,
    base_reserve: u64,
    quote_reserve: u64,
    virtual_quote_reserves: i128,
    coin_creator: &Pubkey,
    cashback_fee_basis_points: u64,
) -> Result<SellBaseInputResult, String> {
    sell_base_input_internal_with_fees(
        base,
        slippage_basis_points,
        base_reserve,
        quote_reserve,
        virtual_quote_reserves,
        &PumpSwapFeeBasisPoints::new(
            LP_FEE_BASIS_POINTS,
            PROTOCOL_FEE_BASIS_POINTS,
            creator_side_fee_basis_points(coin_creator, cashback_fee_basis_points),
        ),
    )
}

pub fn sell_base_input_internal_with_fees(
    base: u64,
    slippage_basis_points: u64,
    base_reserve: u64,
    quote_reserve: u64,
    virtual_quote_reserves: i128,
    fee_basis_points: &PumpSwapFeeBasisPoints,
) -> Result<SellBaseInputResult, String> {
    if base_reserve == 0 || quote_reserve == 0 {
        return Err("Invalid input: 'baseReserve' or 'quoteReserve' cannot be zero.".to_string());
    }
    let effective_quote_reserve = effective_quote_reserve(quote_reserve, virtual_quote_reserves)?;

    // Calculate quote amount out using constant product formula
    let quote_amount_out = ((effective_quote_reserve as u128) * (base as u128)
        / ((base_reserve as u128) + (base as u128))) as u64;

    // Calculate fees
    let lp_fee =
        compute_fee(quote_amount_out as u128, fee_basis_points.lp_fee_basis_points as u128) as u64;
    let protocol_fee =
        compute_fee(quote_amount_out as u128, fee_basis_points.protocol_fee_basis_points as u128)
            as u64;
    let coin_creator_fee = compute_fee(
        quote_amount_out as u128,
        fee_basis_points.coin_creator_fee_basis_points as u128,
    ) as u64;

    // Calculate final quote after fees
    let total_fees = lp_fee + protocol_fee + coin_creator_fee;
    if total_fees > quote_amount_out {
        return Err("Fees exceed total output; final quote is negative.".to_string());
    }
    let quote_vault_outflow = quote_amount_out - lp_fee;
    if quote_vault_outflow > quote_reserve {
        return Err("Insufficient real quote reserves to cover the sell output.".to_string());
    }
    let final_quote = quote_amount_out - total_fees;

    // Calculate min quote with slippage
    let min_quote = calculate_with_slippage_sell(final_quote, slippage_basis_points);

    Ok(SellBaseInputResult {
        ui_quote: final_quote,
        min_quote,
        internal_quote_amount_out: quote_amount_out,
    })
}

const MAX_FEE_BASIS_POINTS: u64 = 10_000;

/// Calculate quote amount out including fees
fn calculate_quote_amount_out(
    user_quote_amount_out: u64,
    lp_fee_basis_points: u64,
    protocol_fee_basis_points: u64,
    coin_creator_fee_basis_points: u64,
) -> Result<u64, String> {
    let total_fee_basis_points = lp_fee_basis_points
        .checked_add(protocol_fee_basis_points)
        .and_then(|fees| fees.checked_add(coin_creator_fee_basis_points))
        .ok_or_else(|| "Fee basis points overflow.".to_string())?;
    let denominator = MAX_FEE_BASIS_POINTS
        .checked_sub(total_fee_basis_points)
        .ok_or_else(|| "Total fee basis points must be less than 10,000.".to_string())?;
    if denominator == 0 {
        return Err("Total fee basis points must be less than 10,000.".to_string());
    }
    let raw_quote = ceil_div(
        (user_quote_amount_out as u128) * (MAX_FEE_BASIS_POINTS as u128),
        denominator as u128,
    );
    u64::try_from(raw_quote).map_err(|_| "Calculated quote amount exceeds u64.".to_string())
}

/// Calculate base tokens needed to receive a specific amount of quote tokens
///
/// # Arguments
/// * `quote` - Desired amount of quote tokens to receive
/// * `slippage_basis_points` - Slippage tolerance in basis points (100 = 1%)
/// * `base_reserve` - Base token reserves in the pool
/// * `quote_reserve` - Raw quote-vault balance
/// * `virtual_quote_reserves` - Signed virtual quote reserves from the same pool snapshot
/// * `coin_creator` - Token creator address
/// * `cashback_fee_basis_points` - Extra fee bps for cashback coins; use `0` if unknown
///
/// # Returns
/// * `SellQuoteInputResult` containing base amount and slippage calculations
pub fn sell_quote_input_internal(
    quote: u64,
    slippage_basis_points: u64,
    base_reserve: u64,
    quote_reserve: u64,
    virtual_quote_reserves: i128,
    coin_creator: &Pubkey,
    cashback_fee_basis_points: u64,
) -> Result<SellQuoteInputResult, String> {
    sell_quote_input_internal_with_fees(
        quote,
        slippage_basis_points,
        base_reserve,
        quote_reserve,
        virtual_quote_reserves,
        &PumpSwapFeeBasisPoints::new(
            LP_FEE_BASIS_POINTS,
            PROTOCOL_FEE_BASIS_POINTS,
            creator_side_fee_basis_points(coin_creator, cashback_fee_basis_points),
        ),
    )
}

pub fn sell_quote_input_internal_with_fees(
    quote: u64,
    slippage_basis_points: u64,
    base_reserve: u64,
    quote_reserve: u64,
    virtual_quote_reserves: i128,
    fee_basis_points: &PumpSwapFeeBasisPoints,
) -> Result<SellQuoteInputResult, String> {
    if base_reserve == 0 || quote_reserve == 0 {
        return Err("Invalid input: 'baseReserve' or 'quoteReserve' cannot be zero.".to_string());
    }
    if quote > quote_reserve {
        return Err("Cannot receive more quote tokens than the pool quote reserves.".to_string());
    }
    let effective_quote_reserve = effective_quote_reserve(quote_reserve, virtual_quote_reserves)?;

    // Calculate raw quote amount including fees
    let raw_quote = calculate_quote_amount_out(
        quote,
        fee_basis_points.lp_fee_basis_points,
        fee_basis_points.protocol_fee_basis_points,
        fee_basis_points.coin_creator_fee_basis_points,
    )?;

    let lp_fee =
        compute_fee(raw_quote as u128, fee_basis_points.lp_fee_basis_points as u128) as u64;
    if raw_quote.saturating_sub(lp_fee) > quote_reserve {
        return Err("Insufficient real quote reserves to cover the sell output.".to_string());
    }

    // Calculate base amount needed using inverse constant product formula
    if raw_quote >= effective_quote_reserve {
        return Err("Invalid input: Desired quote amount exceeds available reserve.".to_string());
    }

    let base_amount_in = ceil_div(
        (base_reserve as u128) * (raw_quote as u128),
        (effective_quote_reserve - raw_quote) as u128,
    ) as u64;

    // Calculate min quote with slippage
    let min_quote = calculate_with_slippage_sell(quote, slippage_basis_points);

    Ok(SellQuoteInputResult { internal_raw_quote: raw_quote, base: base_amount_in, min_quote })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fees() -> PumpSwapFeeBasisPoints {
        PumpSwapFeeBasisPoints::new(20, 5, 0)
    }

    #[test]
    fn buy_uses_effective_quote_reserves() {
        let result =
            buy_quote_input_internal_with_fees(10_000, 100, 1_000_000, 1_000_000, 500_000, &fees())
                .unwrap();
        let without_virtual =
            buy_quote_input_internal_with_fees(10_000, 100, 1_000_000, 1_000_000, 0, &fees())
                .unwrap();

        assert!(result.base < without_virtual.base);
    }

    #[test]
    fn sell_rejects_output_not_covered_by_real_quote_vault() {
        let error = sell_base_input_internal_with_fees(
            1_000_000,
            100,
            1_000_000,
            1_000,
            1_000_000,
            &fees(),
        )
        .unwrap_err();

        assert_eq!(error, "Insufficient real quote reserves to cover the sell output.");
    }

    #[test]
    fn exact_quote_sell_uses_effective_reserve_for_denominator() {
        let result =
            sell_quote_input_internal_with_fees(500, 100, 1_000_000, 1_000, 1_000_000, &fees())
                .unwrap();

        assert!(result.base < 1_000);
    }

    #[test]
    fn exact_quote_sell_rejects_output_above_real_quote_vault() {
        let error =
            sell_quote_input_internal_with_fees(1_001, 100, 1_000_000, 1_000, 1_000_000, &fees())
                .unwrap_err();

        assert_eq!(error, "Cannot receive more quote tokens than the pool quote reserves.");
    }

    #[test]
    fn negative_virtual_reserves_are_applied() {
        let result = buy_quote_input_internal_with_fees(
            10_000,
            100,
            1_000_000,
            1_000_000,
            -500_000,
            &fees(),
        )
        .unwrap();
        let without_virtual =
            buy_quote_input_internal_with_fees(10_000, 100, 1_000_000, 1_000_000, 0, &fees())
                .unwrap();

        assert!(result.base > without_virtual.base);
    }

    #[test]
    fn zero_effective_quote_reserves_are_rejected() {
        let error = buy_quote_input_internal_with_fees(
            10_000,
            100,
            1_000_000,
            1_000_000,
            -1_000_000,
            &fees(),
        )
        .unwrap_err();

        assert_eq!(error, "Invalid effective quote reserves: raw=1000000, virtual=-1000000.");
    }
}
