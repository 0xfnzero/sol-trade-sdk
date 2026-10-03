//! Meteora DAMM v2 swap quotes.
//!
//! A pool holds one liquidity over its price range, so a swap moves the
//! square-root price (Q64.64) along a single segment: the arithmetic of a
//! Meteora DBC curve segment. The fee is taken from the output, or, in a pool
//! that collects fees in token B only, from the input of a B to A swap.
//!
//! The tests replay swaps of a mainnet pool.

use anyhow::{anyhow, Result};

use super::meteora_dbc::{
    delta_base, delta_quote, excluded_fee_amount, next_sqrt_price_from_base_in,
    next_sqrt_price_from_quote_in, to_u64, FEE_DENOMINATOR, MAX_FEE_NUMERATOR,
};
use crate::instruction::utils::meteora_damm_v2_types::Pool;

/// `Pool::collect_fee_mode`: fees are taken in the token a swap pays out.
pub const COLLECT_FEE_BOTH_TOKEN: u8 = 0;

/// What a swap's output is quoted from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DammV2QuoteState {
    pub sqrt_price: u128,
    pub liquidity: u128,
    pub sqrt_min_price: u128,
    pub sqrt_max_price: u128,
    pub fee_numerator: u64,
    pub collect_fee_mode: u8,
}

impl DammV2QuoteState {
    /// The state of `pool` at a fee of `fee_numerator`.
    pub fn from_pool(pool: &Pool, fee_numerator: u64) -> Self {
        Self {
            sqrt_price: pool.sqrt_price,
            liquidity: pool.liquidity,
            sqrt_min_price: pool.sqrt_min_price,
            sqrt_max_price: pool.sqrt_max_price,
            fee_numerator,
            collect_fee_mode: pool.collect_fee_mode,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DammV2Quote {
    /// What the payer receives, after the fee.
    pub amount_out: u64,
    pub fee: u64,
    pub next_sqrt_price: u128,
}

/// Quotes `amount_in` of token A (`a_to_b`) or token B; an error when the
/// swap would leave the pool's price range.
pub fn quote_exact_in(
    state: &DammV2QuoteState,
    a_to_b: bool,
    amount_in: u64,
) -> Result<DammV2Quote> {
    quote(state, a_to_b, amount_in)
        .ok_or_else(|| anyhow!("Meteora DAMM v2 swap of {amount_in} leaves the pool's price range"))
}

fn quote(state: &DammV2QuoteState, a_to_b: bool, amount_in: u64) -> Option<DammV2Quote> {
    let fee_on_input = !a_to_b && state.collect_fee_mode != COLLECT_FEE_BOTH_TOKEN;
    let (actual_in, mut fee) = if fee_on_input {
        excluded_fee_amount(state.fee_numerator, amount_in)?
    } else {
        (amount_in, 0)
    };
    let (next_sqrt_price, amount_out) = if a_to_b {
        let next = next_sqrt_price_from_base_in(state.sqrt_price, state.liquidity, actual_in)?;
        if next < state.sqrt_min_price {
            return None;
        }
        (next, to_u64(delta_quote(next, state.sqrt_price, state.liquidity, false)?)?)
    } else {
        let next = next_sqrt_price_from_quote_in(state.sqrt_price, state.liquidity, actual_in)?;
        if next > state.sqrt_max_price {
            return None;
        }
        (next, to_u64(delta_base(state.sqrt_price, next, state.liquidity, false)?)?)
    };
    let amount_out = if fee_on_input {
        amount_out
    } else {
        let (amount_out, output_fee) = excluded_fee_amount(state.fee_numerator, amount_out)?;
        fee = output_fee;
        amount_out
    };
    Some(DammV2Quote { amount_out, fee, next_sqrt_price })
}

/// A pool's fee numerator at `current_point`, a slot or a unix timestamp by
/// the pool's activation type: its scheduled base fee plus its dynamic fee.
pub fn fee_numerator(pool: &Pool, current_point: u64) -> u64 {
    let base = &pool.pool_fees.base_fee;
    // No schedule when the period length is zero.
    let periods = current_point
        .saturating_sub(pool.activation_point)
        .checked_div(base.period_frequency)
        .unwrap_or(0)
        .min(u64::from(base.number_of_period));
    let base_fee = if periods == 0 {
        base.cliff_fee_numerator
    } else if base.fee_scheduler_mode == 0 {
        base.cliff_fee_numerator.saturating_sub(base.reduction_factor.saturating_mul(periods))
    } else {
        // cliff * (1 - reduction / 10_000) ^ periods
        let factor = 1.0 - base.reduction_factor as f64 / 10_000.0;
        (base.cliff_fee_numerator as f64 * factor.powi(periods as i32)) as u64
    };
    let dynamic = &pool.pool_fees.dynamic_fee;
    let dynamic_fee = if dynamic.initialized == 0 {
        0
    } else {
        dynamic
            .volatility_accumulator
            .checked_mul(u128::from(dynamic.bin_step))
            .and_then(|value| value.checked_pow(2))
            .and_then(|value| value.checked_mul(u128::from(dynamic.variable_fee_control)))
            .map_or(u128::from(MAX_FEE_NUMERATOR), |value| value.div_ceil(100_000_000_000))
    };
    u64::try_from(u128::from(base_fee) + dynamic_fee).unwrap_or(u64::MAX).min(MAX_FEE_NUMERATOR)
}

/// The fee numerator a swap paid: its fees over the amount they were taken
/// from.
pub fn fee_numerator_paid(fee: u64, fee_base: u64) -> u64 {
    if fee_base == 0 {
        return 0;
    }
    let numerator = u128::from(fee) * u128::from(FEE_DENOMINATOR) / u128::from(fee_base);
    u64::try_from(numerator).unwrap_or(u64::MAX).min(MAX_FEE_NUMERATOR)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mainnet pool EPy3Rnwz9G1eg1wx6a9wCoEsnSFCwb3r4keFFzxauLLX, which a DBC
    /// curve migrated to: full range, 2% fee taken in token B (USDC).
    fn pool_at(sqrt_price: u128) -> DammV2QuoteState {
        DammV2QuoteState {
            sqrt_price,
            liquidity: 582_338_177_451_872_515_801_881_719_705,
            sqrt_min_price: 4_295_048_016,
            sqrt_max_price: 79_226_673_521_066_979_257_578_248_091,
            fee_numerator: 20_000_000,
            collect_fee_mode: 1,
        }
    }

    // Each swap starts from the price the pool's swap before it reported.

    #[test]
    fn sale_of_token_a_reproduces_a_mainnet_swap() {
        // Transaction QdiNzUmM2qBgMGtD…
        let quote =
            quote_exact_in(&pool_at(2_904_275_383_804_432_252), true, 9_629_976_451).unwrap();
        assert_eq!(
            quote,
            DammV2Quote {
                amount_out: 223_210_626,
                fee: 4_555_319,
                next_sqrt_price: 2_771_183_070_634_004_317,
            }
        );
    }

    #[test]
    fn buy_with_token_b_reproduces_a_mainnet_swap() {
        let quote =
            quote_exact_in(&pool_at(2_711_494_429_538_635_068), false, 990_000_000).unwrap();
        assert_eq!(
            quote,
            DammV2Quote {
                amount_out: 37_138_769_539,
                fee: 19_800_000,
                next_sqrt_price: 3_278_419_225_421_486_422,
            }
        );
    }

    #[test]
    fn output_fee_mode_takes_a_buys_fee_from_token_a() {
        let mut state = pool_at(2_711_494_429_538_635_068);
        state.collect_fee_mode = COLLECT_FEE_BOTH_TOKEN;
        let quote = quote_exact_in(&state, false, 990_000_000).unwrap();
        let gross = quote.amount_out + quote.fee;
        assert_eq!(quote.fee, (u128::from(gross) * 20_000_000).div_ceil(1_000_000_000) as u64);
        // The whole input moved the price.
        assert!(quote.next_sqrt_price > 3_278_419_225_421_486_422);
    }

    #[test]
    fn swap_out_of_the_price_range_is_an_error() {
        let mut state = pool_at(2_711_494_429_538_635_068);
        state.sqrt_min_price = state.sqrt_price - 1;
        assert!(quote_exact_in(&state, true, 9_629_976_451).is_err());
        state.sqrt_max_price = state.sqrt_price + 1;
        assert!(quote_exact_in(&state, false, 990_000_000).is_err());
    }

    #[test]
    fn pool_fee_follows_its_schedule_and_volatility() {
        let mut pool = Pool::default();
        pool.pool_fees.base_fee.cliff_fee_numerator = 20_000_000;
        assert_eq!(fee_numerator(&pool, 1_000), 20_000_000);

        pool.activation_point = 100;
        pool.pool_fees.base_fee.period_frequency = 10;
        pool.pool_fees.base_fee.number_of_period = 5;
        pool.pool_fees.base_fee.reduction_factor = 2_000_000;
        assert_eq!(fee_numerator(&pool, 105), 20_000_000);
        assert_eq!(fee_numerator(&pool, 120), 16_000_000);
        assert_eq!(fee_numerator(&pool, 10_000), 10_000_000);

        pool.pool_fees.dynamic_fee.initialized = 1;
        pool.pool_fees.dynamic_fee.bin_step = 1;
        pool.pool_fees.dynamic_fee.variable_fee_control = 2_000_000;
        pool.pool_fees.dynamic_fee.volatility_accumulator = 100_000;
        assert_eq!(fee_numerator(&pool, 10_000), 10_200_000);
        pool.pool_fees.dynamic_fee.volatility_accumulator = u128::MAX;
        assert_eq!(fee_numerator(&pool, 10_000), MAX_FEE_NUMERATOR);
    }

    #[test]
    fn fee_paid_recovers_the_numerator() {
        // The sale above: 4_555_319 of fees on 227_765_945 of output.
        assert_eq!(fee_numerator_paid(4_555_319, 223_210_626 + 4_555_319), 20_000_000);
    }
}
