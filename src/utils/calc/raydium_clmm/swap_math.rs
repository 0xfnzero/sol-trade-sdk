// Vendored from https://github.com/raydium-io/raydium-clmm at ed7c84a54ced59c55981780546adb0b4583dcf85
// (programs/amm/src/libraries/swap_math.rs), Apache-2.0: see LICENSE in this directory.
// Changes: tests removed; formatted with this repository's rustfmt; Anchor errors
// replaced by the local `ErrorCode`.

use super::config::FEE_RATE_DENOMINATOR_VALUE;
use super::error::ErrorCode;
use super::error::Result;
use super::full_math::MulDiv;
use super::liquidity_math;
use super::sqrt_price_math;

/// Result of a swap computation
/// Contains the computed price, amounts, and fees after executing a swap calculation
#[derive(Default, Debug)]
pub struct SwapComputationResult {
    /// The price after swapping the amount in/out, not to exceed the price target
    pub sqrt_price_next_x64: u128,
    pub amount_in: u64,
    pub amount_out: u64,
    pub fee_amount: u64,
}

impl SwapComputationResult {
    pub fn new(sqrt_price_next_x64: u128) -> Self {
        Self { sqrt_price_next_x64, amount_in: 0, amount_out: 0, fee_amount: 0 }
    }
}

/// Computes the result of swapping some amount in, or amount out, given the parameters of the swap
pub fn compute_swap(
    sqrt_price_current_x64: u128,
    sqrt_price_target_x64: u128,
    liquidity: u128,
    amount_remaining: u64,
    fee_rate: u32,
    is_base_input: bool,
    zero_for_one: bool,
    is_fee_on_input: bool,
) -> Result<SwapComputationResult> {
    let mut result = SwapComputationResult::default();

    // Gross amount that drives the price math: deduct fee for exact-input
    // fee-on-input; scale up for exact-output fee-on-output.
    let amount_for_price_calc = if is_base_input {
        if is_fee_on_input {
            amount_remaining
                .mul_div_floor(
                    (FEE_RATE_DENOMINATOR_VALUE - fee_rate).into(),
                    u64::from(FEE_RATE_DENOMINATOR_VALUE),
                )
                .ok_or(ErrorCode::CalculateOverflow)?
        } else {
            amount_remaining
        }
    } else {
        if is_fee_on_input {
            amount_remaining
        } else {
            amount_remaining
                .mul_div_ceil(
                    u64::from(FEE_RATE_DENOMINATOR_VALUE).into(),
                    (FEE_RATE_DENOMINATOR_VALUE - fee_rate).into(),
                )
                .ok_or(ErrorCode::CalculateOverflow)?
        }
    };

    // Both amounts at the target price. `MaxTokenOverflow` ⇒ target unreachable
    // at u64 precision → fall through to the not-reached branch.
    let max_reachable = match liquidity_math::get_delta_amounts_for_swap(
        sqrt_price_target_x64,
        sqrt_price_current_x64,
        liquidity,
        zero_for_one,
    ) {
        Ok((amount_in_at_target, amount_out_at_target)) => {
            let user_limit = if is_base_input { amount_in_at_target } else { amount_out_at_target };
            if amount_for_price_calc >= user_limit {
                Some((amount_in_at_target, amount_out_at_target))
            } else {
                None
            }
        }
        Err(e) if e == error!(ErrorCode::MaxTokenOverflow) => None,
        Err(e) => return Err(e),
    };

    if let Some((amount_in_at_target, amount_out_at_target)) = max_reachable {
        result.sqrt_price_next_x64 = sqrt_price_target_x64;
        result.amount_in = amount_in_at_target;
        result.amount_out = amount_out_at_target;
    } else {
        // Solve for the actual sqrt_next reachable with `amount_for_price_calc`.
        // Since sqrt_next is closer to current than target, both amounts at
        // sqrt_next fit u64 even when the target-side amounts didn't.
        let sqrt_next = if is_base_input {
            sqrt_price_math::get_next_sqrt_price_from_input(
                sqrt_price_current_x64,
                liquidity,
                amount_for_price_calc,
                zero_for_one,
            )?
        } else {
            sqrt_price_math::get_next_sqrt_price_from_output(
                sqrt_price_current_x64,
                liquidity,
                amount_for_price_calc,
                zero_for_one,
            )?
        };
        result.sqrt_price_next_x64 = sqrt_next;
        let (amount_in, amount_out) = liquidity_math::get_delta_amounts_for_swap(
            sqrt_next,
            sqrt_price_current_x64,
            liquidity,
            zero_for_one,
        )?;
        result.amount_in = amount_in;
        result.amount_out = amount_out;
    }

    if zero_for_one {
        require_gte!(result.sqrt_price_next_x64, sqrt_price_target_x64);
    } else {
        require_gte!(sqrt_price_target_x64, result.sqrt_price_next_x64);
    }

    if is_base_input {
        if is_fee_on_input {
            if result.sqrt_price_next_x64 != sqrt_price_target_x64 {
                result.fee_amount = amount_remaining
                    .checked_sub(result.amount_in)
                    .ok_or(ErrorCode::CalculateOverflow)?;
            } else {
                result.fee_amount = result
                    .amount_in
                    .mul_div_ceil(fee_rate.into(), (FEE_RATE_DENOMINATOR_VALUE - fee_rate).into())
                    .ok_or(ErrorCode::CalculateOverflow)?;
            }
        } else {
            // Fee from output: result.amount_out is gross output, fee is calculated from gross output
            // fee = gross_output * fee_rate / FEE_RATE_DENOMINATOR
            result.fee_amount = result
                .amount_out
                .mul_div_ceil(fee_rate.into(), FEE_RATE_DENOMINATOR_VALUE.into())
                .ok_or(ErrorCode::CalculateOverflow)?;
            // Deduct fee from output: user receives net output
            result.amount_out = result
                .amount_out
                .checked_sub(result.fee_amount)
                .ok_or(ErrorCode::CalculateOverflow)?;

            // Partial step: the price moved less than the exact input warrants (rounded toward the
            // pool — down for one_for_zero, up for zero_for_one), so amount_in recomputed from that
            // move can be below the available input, leaving an un-tradeable dust (< liquidity/Q64).
            // Fee-on-input folds it into the fee, fee-on-output cannot, so it would stall the loop.
            // Charge the full input (== amount_remaining here); the sub-unit excess goes to the pool.
            if result.sqrt_price_next_x64 != sqrt_price_target_x64 {
                result.amount_in = amount_remaining;
            }
        }
    } else {
        if is_fee_on_input {
            // Fee from input: amount_remaining is the desired gross output
            // Cap the gross output amount to the remaining amount
            result.amount_out = result.amount_out.min(amount_remaining);
            result.fee_amount = result
                .amount_in
                .mul_div_ceil(fee_rate.into(), (FEE_RATE_DENOMINATOR_VALUE - fee_rate).into())
                .ok_or(ErrorCode::CalculateOverflow)?;
        } else {
            result.fee_amount = result
                .amount_out
                .mul_div_ceil(fee_rate.into(), FEE_RATE_DENOMINATOR_VALUE.into())
                .ok_or(ErrorCode::CalculateOverflow)?;

            // Calculate net output
            let net_output = result
                .amount_out
                .checked_sub(result.fee_amount)
                .ok_or(ErrorCode::CalculateOverflow)?;

            // Cap net output to amount_remaining (user's desired net output)
            // If net output exceeds amount_remaining, adjust fee to cap it
            if net_output > amount_remaining {
                // Adjust fee so that net output = amount_remaining
                result.fee_amount = result
                    .amount_out
                    .checked_sub(amount_remaining)
                    .ok_or(ErrorCode::CalculateOverflow)?;
                result.amount_out = amount_remaining;
            } else {
                // Deduct fee from output: user receives net output
                result.amount_out = net_output;
            }
        }
    }

    Ok(result)
}
