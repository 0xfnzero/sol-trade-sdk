// Ported from https://github.com/raydium-io/raydium-clmm at ed7c84a54ced59c55981780546adb0b4583dcf85
// (programs/amm/src/instructions/swap.rs `SwapState`, `swap_internal`; swap_v2.rs
// `exact_internal_v2`), Apache-2.0: see LICENSE in this directory.
// Changes: runs over decoded accounts without writing them (no fee growth,
// reward, observation or pool updates); tick arrays are looked up by start index
// instead of popped from the account list, and the ones crossed are returned;
// only exact input without a price limit, as `swap_v2` with
// `sqrt_price_limit_x64 == 0`.

//! Exact-input swap quotes for a Raydium CLMM pool.

use super::config::AmmConfig;
use super::error::{ErrorCode, Result};
use super::full_math::MulDiv;
use super::state::{
    tick_spacing_index_from_tick, DynamicFeeInfo, PoolState, PoolStatusBitIndex,
    TickArrayBitmapExtension, TickArrayState, DYNAMIC_FEE_CONTROL_DENOMINATOR,
    MAX_FEE_RATE_NUMERATOR, VOLATILITY_ACCUMULATOR_SCALE,
};
use super::swap_math::{self, SwapComputationResult};
use super::{liquidity_math, tick_math, U128};
use std::ops::Neg;

/// What an exact-input swap through the pool does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwapQuote {
    /// Input the pool takes: all of the amount quoted.
    pub amount_in: u64,
    /// Output the pool sends, before any transfer fee of the output mint.
    pub amount_out: u64,
    /// Start indexes of the tick arrays the swap reads, in the order it reads
    /// them: the accounts `swap_v2` needs.
    pub tick_array_start_indexes: Vec<i32>,
    /// Pool price and tick after the swap.
    pub sqrt_price_x64: u128,
    pub tick: i32,
    /// Ticks whose limit orders the swap filled.
    pub limit_order_ticks: usize,
    /// Swap steps the program takes (one per tick, or per tick spacing group
    /// with a dynamic fee): what its compute use grows with.
    pub steps: usize,
}

/// Why a quote could not be made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuoteError {
    /// The swap reads a tick array that was not provided; fetch it and quote again.
    MissingTickArray(i32),
    /// The program would reject the swap.
    Swap(ErrorCode),
}

impl From<ErrorCode> for QuoteError {
    fn from(code: ErrorCode) -> Self {
        QuoteError::Swap(code)
    }
}

impl std::fmt::Display for QuoteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QuoteError::MissingTickArray(start) => {
                write!(f, "Raydium CLMM quote needs the tick array starting at {start}")
            }
            QuoteError::Swap(code) => write!(f, "{code}"),
        }
    }
}

impl std::error::Error for QuoteError {}

// the top level state of the swap, the results of which are recorded in storage at the end
#[derive(Debug)]
struct SwapState {
    // the amount remaining to be swapped in/out of the input/output asset
    amount_specified_remaining: u64,
    // the amount already swapped out/in of the output/input asset
    amount_calculated: u64,
    // The latest sqrt price of the pool
    sqrt_price_x64: u128,
    // the tick associated with the current price
    tick: i32,
    // the current liquidity in range
    liquidity: u128,
    // the sqrt price for the next tick
    sqrt_price_next_x64: u128,
    // the next tick to swap to from the current tick in the swap direction
    tick_next: i32,
    // The tick spacing of the pool, used to group ticks for dynamic fee calculation
    tick_spacing: u16,
    // The base fee rate (static component) of the pool
    base_fee_rate: u32,
    // The current tick spacing index, representing which tick group the current price belongs to.
    tick_spacing_index: i32,
    // Dynamic fee configuration and state; None if dynamic fee is not enabled for this pool.
    dynamic_fee_info: Option<DynamicFeeInfo>,
}

impl SwapState {
    fn new(
        pool_state: &PoolState,
        amount_specified: u64,
        base_fee_rate: u32,
        block_timestamp: u64,
    ) -> Result<Self> {
        let mut state = Self {
            amount_specified_remaining: amount_specified,
            amount_calculated: 0,
            sqrt_price_x64: pool_state.sqrt_price_x64,
            tick: pool_state.tick_current,
            liquidity: pool_state.liquidity,
            sqrt_price_next_x64: 0,
            tick_next: 0,
            base_fee_rate,
            tick_spacing: pool_state.tick_spacing,
            tick_spacing_index: 0,
            dynamic_fee_info: pool_state.get_dynamic_fee_info(),
        };
        if let Some(dynamic_fee_info) = &mut state.dynamic_fee_info {
            state.tick_spacing_index = tick_spacing_index_from_tick(state.tick, state.tick_spacing);
            dynamic_fee_info.update_reference(state.tick_spacing_index, block_timestamp)?;
        }
        Ok(state)
    }

    /// Apply swap step result by updating remaining and calculated amounts (the
    /// fee split between lp, protocol and fund does not change them).
    fn apply_swap_amounts(
        &mut self,
        amount_in: u64,
        amount_out: u64,
        fee_amount: u64,
        is_base_input: bool,
        is_fee_on_input: bool,
    ) -> Result<()> {
        // If fee is from input, user pays amount_in + fee_amount; otherwise just amount_in
        let amount_in_consumed = if is_fee_on_input {
            amount_in.checked_add(fee_amount).ok_or(ErrorCode::CalculateOverflow)?
        } else {
            amount_in
        };

        if is_base_input {
            self.amount_specified_remaining = self
                .amount_specified_remaining
                .checked_sub(amount_in_consumed)
                .ok_or(ErrorCode::CalculateOverflow)?;
            self.amount_calculated = self
                .amount_calculated
                .checked_add(amount_out)
                .ok_or(ErrorCode::CalculateOverflow)?;
        } else {
            self.amount_specified_remaining = self
                .amount_specified_remaining
                .checked_sub(amount_out)
                .ok_or(ErrorCode::CalculateOverflow)?;
            self.amount_calculated = self
                .amount_calculated
                .checked_add(amount_in_consumed)
                .ok_or(ErrorCode::CalculateOverflow)?;
        }
        Ok(())
    }

    fn get_target_price_based_on_next_tick(
        &mut self,
        tick_next: i32,
        zero_for_one: bool,
        sqrt_price_limit_x64: u128,
    ) -> Result<u128> {
        // Clamp tick_next to valid range
        self.tick_next = tick_next.clamp(tick_math::MIN_TICK, tick_math::MAX_TICK);

        // Calculate sqrt_price for the next tick
        self.sqrt_price_next_x64 = tick_math::get_sqrt_price_at_tick(self.tick_next)?;

        // Determine target price: either the next tick price or the limit price
        let target_price = if (zero_for_one && self.sqrt_price_next_x64 < sqrt_price_limit_x64)
            || (!zero_for_one && self.sqrt_price_next_x64 > sqrt_price_limit_x64)
        {
            sqrt_price_limit_x64
        } else {
            self.sqrt_price_next_x64
        };

        // Validate swap direction
        if zero_for_one {
            require_gte!(self.tick, self.tick_next);
            require_gte!(self.sqrt_price_x64, self.sqrt_price_next_x64);
            require_gte!(self.sqrt_price_x64, target_price);
        } else {
            require_gt!(self.tick_next, self.tick);
            require_gte!(self.sqrt_price_next_x64, self.sqrt_price_x64);
            require_gte!(target_price, self.sqrt_price_x64);
        }

        Ok(target_price)
    }

    fn update_volatility_accumulator(&mut self) -> Result<()> {
        if let Some(dynamic_fee_info) = &mut self.dynamic_fee_info {
            dynamic_fee_info.update_volatility_accumulator(self.tick_spacing_index)?;
        }
        Ok(())
    }

    fn update_dynamic_fee_index(
        &mut self,
        zero_for_one: bool,
        is_skipped_tick_spacing: bool,
    ) -> Result<()> {
        if let Some(dynamic_fee_info) = &self.dynamic_fee_info {
            if is_skipped_tick_spacing {
                let tick_index = if self.sqrt_price_x64 == self.sqrt_price_next_x64 {
                    self.tick_next
                } else {
                    self.tick
                };
                let mut tick_spacing_index =
                    tick_spacing_index_from_tick(tick_index, self.tick_spacing);
                if !zero_for_one && tick_index % (self.tick_spacing as i32) == 0 {
                    tick_spacing_index -= 1;
                }
                self.tick_spacing_index = tick_spacing_index;

                if dynamic_fee_info.volatility_accumulator
                    != dynamic_fee_info.max_volatility_accumulator
                {
                    self.update_volatility_accumulator()?;
                }
            }
            self.tick_spacing_index += if zero_for_one { -1 } else { 1 };
        }
        Ok(())
    }

    fn get_spacing_bounded_price(
        &self,
        target_price: u128,
        zero_for_one: bool,
    ) -> Result<(bool, u128, Option<i32>)> {
        if let Some(dynamic_fee_info) = &self.dynamic_fee_info {
            if self.liquidity == 0
                || dynamic_fee_info.volatility_accumulator
                    == dynamic_fee_info.max_volatility_accumulator
            {
                return Ok((true, target_price, None));
            }

            let tick_spacing_i32 = i32::from(self.tick_spacing);
            let bounded_tick = if zero_for_one {
                self.tick_spacing_index.saturating_mul(tick_spacing_i32)
            } else {
                self.tick_spacing_index.saturating_add(1).saturating_mul(tick_spacing_i32)
            };
            let bounded_tick_clamped = bounded_tick.clamp(tick_math::MIN_TICK, tick_math::MAX_TICK);
            let bounded_sqrt_price = tick_math::get_sqrt_price_at_tick(bounded_tick_clamped)?;

            if zero_for_one {
                if target_price > bounded_sqrt_price {
                    Ok((false, target_price, None))
                } else {
                    Ok((false, bounded_sqrt_price, Some(bounded_tick_clamped)))
                }
            } else if target_price < bounded_sqrt_price {
                Ok((false, target_price, None))
            } else {
                Ok((false, bounded_sqrt_price, Some(bounded_tick_clamped)))
            }
        } else {
            Ok((true, target_price, None))
        }
    }

    fn get_total_fee_rate(&self) -> Result<u32> {
        // Use base + dynamic fee if dynamic fee is enabled
        if let Some(dynamic_fee_info) = &self.dynamic_fee_info {
            let dynamic_fee_rate =
                Self::compute_dynamic_fee_rate(dynamic_fee_info, self.tick_spacing)?;
            let total_fee_rate = self.base_fee_rate + dynamic_fee_rate;
            return Ok(total_fee_rate.min(MAX_FEE_RATE_NUMERATOR));
        }
        Ok(self.base_fee_rate)
    }

    /// Computes the dynamic fee rate based on volatility accumulator (quadratic in it).
    fn compute_dynamic_fee_rate(
        dynamic_fee_info: &DynamicFeeInfo,
        tick_spacing: u16,
    ) -> Result<u32> {
        let crossed = dynamic_fee_info.volatility_accumulator * tick_spacing as u32;

        // Square the crossed value to create quadratic fee scaling
        let squared = u64::from(crossed) * u64::from(crossed);

        let denominator = U128::from(DYNAMIC_FEE_CONTROL_DENOMINATOR)
            * U128::from(VOLATILITY_ACCUMULATOR_SCALE)
            * U128::from(VOLATILITY_ACCUMULATOR_SCALE);

        // Compute fee rate using ceiling division to ensure minimum fee protection
        let fee_rate = U128::from(dynamic_fee_info.dynamic_fee_control)
            .mul_div_ceil(U128::from(squared), denominator)
            .ok_or(ErrorCode::CalculateOverflow)?
            .as_u128();
        // bound the fee rate to the maximum fee rate
        if fee_rate > MAX_FEE_RATE_NUMERATOR as u128 {
            Ok(MAX_FEE_RATE_NUMERATOR)
        } else {
            Ok(fee_rate as u32)
        }
    }
}

/// Quote `amount_in` (what the pool receives, net of any input transfer fee)
/// swapped through the pool, as `swap_v2` with no price limit would run it at
/// `block_timestamp`. `tick_arrays` are the decoded tick arrays of the pool the
/// swap may read; a missing one is reported by its start index.
pub fn quote_exact_in(
    pool_state: &PoolState,
    amm_config: &AmmConfig,
    tick_arrays: &[TickArrayState],
    tickarray_bitmap_extension: Option<&TickArrayBitmapExtension>,
    amount_in: u64,
    zero_for_one: bool,
    block_timestamp: u64,
) -> core::result::Result<SwapQuote, QuoteError> {
    let is_base_input = true;
    let amount_specified = amount_in;
    // `swap_v2` without a limit swaps to the edge of the price range
    let sqrt_price_limit_x64 = if zero_for_one {
        tick_math::MIN_SQRT_PRICE_X64 + 1
    } else {
        tick_math::MAX_SQRT_PRICE_X64 - 1
    };
    if block_timestamp <= pool_state.open_time {
        return Err(ErrorCode::PoolNotOpen.into());
    }
    if amount_specified == 0 {
        return Err(ErrorCode::ZeroAmountSpecified.into());
    }
    if !pool_state.get_status_by_bit(PoolStatusBitIndex::Swap) {
        return Err(ErrorCode::NotApproved.into());
    }
    let limit_ok = if zero_for_one {
        sqrt_price_limit_x64 < pool_state.sqrt_price_x64
            && sqrt_price_limit_x64 > tick_math::MIN_SQRT_PRICE_X64
    } else {
        sqrt_price_limit_x64 > pool_state.sqrt_price_x64
            && sqrt_price_limit_x64 < tick_math::MAX_SQRT_PRICE_X64
    };
    if !limit_ok {
        return Err(ErrorCode::RequireViolated.into());
    }

    let lookup = |start: i32| -> core::result::Result<TickArrayState, QuoteError> {
        tick_arrays
            .iter()
            .find(|array| array.start_tick_index == start)
            .cloned()
            .ok_or(QuoteError::MissingTickArray(start))
    };

    let (mut first_tick_array_contains_pool_tick, first_valid_tick_array_start_index) =
        pool_state.get_first_initialized_tick_array(tickarray_bitmap_extension, zero_for_one)?;
    let mut current_valid_tick_array_start_index = first_valid_tick_array_start_index;
    let mut tick_array_current = lookup(current_valid_tick_array_start_index)?;
    let mut tick_array_start_indexes = vec![current_valid_tick_array_start_index];
    let mut limit_order_ticks = 0;
    let mut steps = 0;

    // Determine if fee should be collected from input token (only need to calculate once)
    let is_fee_on_input = pool_state.is_fee_on_input(zero_for_one);
    let mut state =
        SwapState::new(pool_state, amount_specified, amm_config.trade_fee_rate, block_timestamp)?;
    // Main swap loop: continue swapping until we've consumed all input/output or reached the price limit
    while state.amount_specified_remaining != 0 && state.sqrt_price_x64 != sqrt_price_limit_x64 {
        let tick_index = if let Some(index) = tick_array_current.next_initialized_tick(
            state.tick,
            pool_state.tick_spacing,
            zero_for_one,
        ) {
            index
        }
        // If not found and the first tick array doesn't contain pool's current tick,
        // use the first initialized tick in current array (only happens once in the first iteration)
        else if !first_tick_array_contains_pool_tick {
            first_tick_array_contains_pool_tick = true;
            tick_array_current.first_initialized_tick(zero_for_one)?
        }
        // Otherwise, need to move to next tick array
        else {
            let next_tick_array_index = pool_state
                .next_initialized_tick_array_start_index(
                    tickarray_bitmap_extension,
                    current_valid_tick_array_start_index,
                    zero_for_one,
                )?
                .ok_or(ErrorCode::LiquidityInsufficient)?;
            tick_array_current = lookup(next_tick_array_index)?;
            tick_array_start_indexes.push(next_tick_array_index);
            current_valid_tick_array_start_index = next_tick_array_index;
            tick_array_current.first_initialized_tick(zero_for_one)?
        };
        let mut next_initialized_tick = tick_array_current.ticks[tick_index];
        if !next_initialized_tick.is_initialized() {
            return Err(ErrorCode::RequireViolated.into());
        }

        let target_price = state.get_target_price_based_on_next_tick(
            next_initialized_tick.tick,
            zero_for_one,
            sqrt_price_limit_x64,
        )?;

        let mut liquidity_next = state.liquidity;
        loop {
            steps += 1;
            state.update_volatility_accumulator()?;
            let total_fee_rate = state.get_total_fee_rate()?;
            let (is_skipped_tick_spacing, bounded_price, bounded_tick_opt) =
                state.get_spacing_bounded_price(target_price, zero_for_one)?;

            let is_price_change = state.sqrt_price_x64 != bounded_price;
            let swap_computed_result = if is_price_change {
                let swap_computed_result = swap_math::compute_swap(
                    state.sqrt_price_x64,
                    bounded_price,
                    state.liquidity,
                    state.amount_specified_remaining,
                    total_fee_rate,
                    is_base_input,
                    zero_for_one,
                    is_fee_on_input,
                )?;
                state.apply_swap_amounts(
                    swap_computed_result.amount_in,
                    swap_computed_result.amount_out,
                    swap_computed_result.fee_amount,
                    is_base_input,
                    is_fee_on_input,
                )?;
                swap_computed_result
            } else {
                SwapComputationResult::new(bounded_price)
            };
            let limit_order_unfilled_amount_before =
                next_initialized_tick.limit_order_unfilled_amount()?;
            if state.sqrt_price_next_x64 == swap_computed_result.sqrt_price_next_x64 {
                // Match limit orders on this boundary tick at the pre-advance `total_fee_rate`
                let limit_order_result = next_initialized_tick.match_limit_order_with_sqrt_price(
                    state.amount_specified_remaining,
                    zero_for_one,
                    is_base_input,
                    total_fee_rate,
                    is_fee_on_input,
                    state.sqrt_price_next_x64,
                )?;
                if limit_order_result.amount_in != 0
                    || limit_order_result.amount_out != 0
                    || limit_order_result.amm_fee_amount != 0
                {
                    state.apply_swap_amounts(
                        limit_order_result.amount_in,
                        limit_order_result.amount_out,
                        limit_order_result.amm_fee_amount,
                        is_base_input,
                        is_fee_on_input,
                    )?;
                    limit_order_ticks += 1;
                }
                if !next_initialized_tick.is_initialized() {
                    tick_array_current.initialized_tick_count = tick_array_current
                        .initialized_tick_count
                        .checked_sub(1)
                        .ok_or(ErrorCode::CalculateOverflow)?;
                }

                if next_initialized_tick.has_liquidity()
                    && !next_initialized_tick.has_limit_orders()
                {
                    // `cross` returns the tick's liquidity_net
                    let mut liquidity_net = next_initialized_tick.liquidity_net;
                    if zero_for_one {
                        liquidity_net = liquidity_net.neg();
                    }
                    liquidity_next = liquidity_math::add_delta(state.liquidity, liquidity_net)?;
                }

                tick_array_current.ticks[tick_index] = next_initialized_tick;

                // Update tick based on limit order status and swap direction
                state.tick = if (zero_for_one && !next_initialized_tick.has_limit_orders())
                    || (!zero_for_one && next_initialized_tick.has_limit_orders())
                {
                    state.tick_next - 1
                } else {
                    state.tick_next
                };
            } else if state.sqrt_price_x64 != swap_computed_result.sqrt_price_next_x64 {
                // recompute unless we're on a lower tick boundary (i.e. already transitioned ticks), and haven't moved
                state.tick = match bounded_tick_opt {
                    Some(t) if swap_computed_result.sqrt_price_next_x64 == bounded_price => t,
                    _ => {
                        tick_math::get_tick_at_sqrt_price(swap_computed_result.sqrt_price_next_x64)?
                    }
                };
            }
            state.sqrt_price_x64 = swap_computed_result.sqrt_price_next_x64;
            state.update_dynamic_fee_index(zero_for_one, is_skipped_tick_spacing)?;
            if state.amount_specified_remaining == 0 || state.sqrt_price_x64 == target_price {
                let limit_order_unfilled_amount_after =
                    next_initialized_tick.limit_order_unfilled_amount()?;
                // If a limit order has been executed and the active swap amount is not zero, then the remaining amount of the limit order must be zero
                if state.amount_specified_remaining != 0
                    && limit_order_unfilled_amount_after != limit_order_unfilled_amount_before
                    && limit_order_unfilled_amount_after != 0
                {
                    return Err(ErrorCode::RequireViolated.into());
                }
                break;
            }
        }
        state.liquidity = liquidity_next;
    }

    // `swap_v2` without a price limit does not allow a partial fill
    if state.amount_specified_remaining != 0 {
        return Err(ErrorCode::PartialFill.into());
    }
    if state.amount_calculated == 0 {
        return Err(ErrorCode::TooSmallInputOrOutputAmount.into());
    }
    Ok(SwapQuote {
        amount_in: amount_specified,
        amount_out: state.amount_calculated,
        tick_array_start_indexes,
        sqrt_price_x64: state.sqrt_price_x64,
        tick: state.tick,
        limit_order_ticks,
        steps,
    })
}
