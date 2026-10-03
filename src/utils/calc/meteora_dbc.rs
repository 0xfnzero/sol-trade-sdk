//! Meteora Dynamic Bonding Curve swap quotes.
//!
//! A curve is a row of constant-liquidity segments between square-root prices
//! (Q64.64), like a concentrated-liquidity pool with fixed positions: a swap
//! walks them from the pool's price, buys up to the migration price, sales
//! down to the start price. The fee is a share of the quote side in
//! quote-token fee mode, of the output otherwise.
//!
//! The quotes reproduce the program's arithmetic and rounding; the tests
//! replay swaps of mainnet pools.

use anyhow::{anyhow, Result};

use crate::instruction::utils::meteora_dbc_types::{
    DbcConfig, ACTIVATION_SLOT, BASE_FEE_RATE_LIMITER, BASE_FEE_SCHEDULER_EXPONENTIAL,
    BASE_FEE_SCHEDULER_LINEAR, COLLECT_FEE_QUOTE_TOKEN,
};
use crate::utils::calc::raydium_clmm::big_num::U256;

/// Fee numerators are parts of this.
pub const FEE_DENOMINATOR: u64 = 1_000_000_000;
/// The most a swap pays in fees: 99%.
pub const MAX_FEE_NUMERATOR: u64 = 990_000_000;
const BASIS_POINT_MAX: u128 = 10_000;
const ONE_Q64: u128 = 1 << 64;
/// Exponents the program's Q64.64 power takes.
const MAX_EXPONENTIAL: u32 = 0x80000;

pub(crate) fn to_u64(value: U256) -> Option<u64> {
    (value <= U256::from(u64::MAX)).then(|| value.low_u64())
}

fn to_u128(value: U256) -> Option<u128> {
    (value <= U256::from(u128::MAX)).then(|| value.low_u128())
}

fn ceil_div(numerator: U256, denominator: U256) -> Option<U256> {
    if denominator.is_zero() {
        return None;
    }
    let (quotient, remainder) = numerator.div_mod(denominator);
    if remainder.is_zero() {
        Some(quotient)
    } else {
        quotient.checked_add(U256::one())
    }
}

/// Base tokens a segment holds between two prices: `L * (upper - lower) /
/// (upper * lower)`.
pub(crate) fn delta_base(
    lower: u128,
    upper: u128,
    liquidity: u128,
    round_up: bool,
) -> Option<U256> {
    // Two factors below 2^128 cannot overflow 256 bits.
    let numerator = U256::from(liquidity) * U256::from(upper.checked_sub(lower)?);
    let denominator = U256::from(lower) * U256::from(upper);
    if round_up {
        ceil_div(numerator, denominator)
    } else if denominator.is_zero() {
        None
    } else {
        Some(numerator / denominator)
    }
}

/// Quote tokens a segment holds between two prices: `L * (upper - lower)`,
/// scaled down from Q128.
pub(crate) fn delta_quote(
    lower: u128,
    upper: u128,
    liquidity: u128,
    round_up: bool,
) -> Option<U256> {
    let product = U256::from(liquidity) * U256::from(upper.checked_sub(lower)?);
    let quotient = product >> 128;
    if round_up && product.low_u128() != 0 {
        quotient.checked_add(U256::one())
    } else {
        Some(quotient)
    }
}

/// The price after `amount` of quote goes in: `sqrt + amount / L`, rounded down.
pub(crate) fn next_sqrt_price_from_quote_in(
    sqrt_price: u128,
    liquidity: u128,
    amount: u64,
) -> Option<u128> {
    if liquidity == 0 {
        return None;
    }
    let quotient = (U256::from(amount) << 128) / U256::from(liquidity);
    to_u128(U256::from(sqrt_price).checked_add(quotient)?)
}

/// The price after `amount` of base goes in: `L * sqrt / (L + amount * sqrt)`,
/// rounded up.
pub(crate) fn next_sqrt_price_from_base_in(
    sqrt_price: u128,
    liquidity: u128,
    amount: u64,
) -> Option<u128> {
    if amount == 0 {
        return Some(sqrt_price);
    }
    let product = U256::from(amount) * U256::from(sqrt_price);
    let denominator = U256::from(liquidity).checked_add(product)?;
    to_u128(ceil_div(U256::from(liquidity) * U256::from(sqrt_price), denominator)?)
}

/// A walk along the curve, before fees.
struct CurveSwap {
    amount_out: u64,
    next_sqrt_price: u128,
    /// Input the curve did not take.
    amount_left: u64,
}

/// Quote in, base out, up to the migration price.
fn quote_to_base(config: &DbcConfig, sqrt_price: u128, amount_in: u64) -> Option<CurveSwap> {
    let stop = config.migration_sqrt_price;
    let mut amount_out = 0u64;
    let mut amount_left = amount_in;
    let mut current = sqrt_price;
    for point in &config.curve {
        let reference = stop.min(point.sqrt_price);
        if reference <= current {
            continue;
        }
        let max_amount_in = delta_quote(current, reference, point.liquidity, true)?;
        if U256::from(amount_left) < max_amount_in {
            let next = next_sqrt_price_from_quote_in(current, point.liquidity, amount_left)?;
            let output = to_u64(delta_base(current, next, point.liquidity, false)?)?;
            amount_out = amount_out.checked_add(output)?;
            current = next;
            amount_left = 0;
            break;
        }
        let output = to_u64(delta_base(current, reference, point.liquidity, false)?)?;
        amount_out = amount_out.checked_add(output)?;
        current = reference;
        amount_left = amount_left.checked_sub(to_u64(max_amount_in)?)?;
        if reference == stop {
            break;
        }
    }
    Some(CurveSwap { amount_out, next_sqrt_price: current, amount_left })
}

/// Base in, quote out, down to the start price.
fn base_to_quote(config: &DbcConfig, sqrt_price: u128, amount_in: u64) -> Option<CurveSwap> {
    let curve = &config.curve;
    let mut amount_out = 0u64;
    let mut amount_left = amount_in;
    let mut current = sqrt_price;
    // The segment above point `index` has the liquidity of the point after it.
    for index in (0..curve.len().saturating_sub(1)).rev() {
        if curve[index].sqrt_price >= current {
            continue;
        }
        let liquidity = curve[index + 1].liquidity;
        let max_amount_in = delta_base(curve[index].sqrt_price, current, liquidity, true)?;
        if U256::from(amount_left) < max_amount_in {
            let next = next_sqrt_price_from_base_in(current, liquidity, amount_left)?;
            let output = to_u64(delta_quote(next, current, liquidity, false)?)?;
            amount_out = amount_out.checked_add(output)?;
            current = next;
            amount_left = 0;
            break;
        }
        let next = curve[index].sqrt_price;
        let output = to_u64(delta_quote(next, current, liquidity, false)?)?;
        amount_out = amount_out.checked_add(output)?;
        current = next;
        amount_left = amount_left.checked_sub(to_u64(max_amount_in)?)?;
    }
    if amount_left != 0 {
        // The first segment, from its point down to the start price.
        let liquidity = curve.first()?.liquidity;
        let mut next = next_sqrt_price_from_base_in(current, liquidity, amount_left)?;
        if next < config.sqrt_start_price {
            next = config.sqrt_start_price;
            let taken = to_u64(delta_base(next, current, liquidity, true)?)?;
            amount_left = amount_left.checked_sub(taken)?;
        } else {
            amount_left = 0;
        }
        let output = to_u64(delta_quote(next, current, liquidity, false)?)?;
        amount_out = amount_out.checked_add(output)?;
        current = next;
    }
    Some(CurveSwap { amount_out, next_sqrt_price: current, amount_left })
}

/// `amount` less its fee, and the fee: `amount * numerator / FEE_DENOMINATOR`,
/// rounded up.
pub(crate) fn excluded_fee_amount(fee_numerator: u64, amount: u64) -> Option<(u64, u64)> {
    let fee =
        (u128::from(amount) * u128::from(fee_numerator)).div_ceil(u128::from(FEE_DENOMINATOR));
    let fee = u64::try_from(fee).ok()?;
    Some((amount.checked_sub(fee)?, fee))
}

/// The amount that leaves `amount` after its fee, rounded up.
fn included_fee_amount(fee_numerator: u64, amount: u64) -> Option<u64> {
    let denominator = FEE_DENOMINATOR.checked_sub(fee_numerator).filter(|value| *value != 0)?;
    let included =
        (u128::from(amount) * u128::from(FEE_DENOMINATOR)).div_ceil(u128::from(denominator));
    u64::try_from(included).ok()
}

/// A quote of an exact-in or partial-fill swap.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DbcQuote {
    /// What the swap takes from the payer, fee included. Less than the input
    /// when the curve cannot take it all: a partial fill gives the rest back.
    pub amount_in: u64,
    /// What the payer receives, after the fee.
    pub amount_out: u64,
    pub fee: u64,
    pub next_sqrt_price: u128,
    /// Input, less its fee, the curve did not take.
    pub amount_left: u64,
}

/// Quotes `amount_in` of quote (a buy) or base (a sale) at `sqrt_price`, as a
/// partial fill pays it; an exact-in swap pays the same or, when
/// `amount_left` is not zero, fails.
pub fn quote_exact_in(
    config: &DbcConfig,
    sqrt_price: u128,
    fee_numerator: u64,
    is_buy: bool,
    amount_in: u64,
) -> Result<DbcQuote> {
    quote(config, sqrt_price, fee_numerator, is_buy, amount_in)
        .ok_or_else(|| anyhow!("Meteora DBC swap of {amount_in} overflows the curve math"))
}

fn quote(
    config: &DbcConfig,
    sqrt_price: u128,
    fee_numerator: u64,
    is_buy: bool,
    amount_in: u64,
) -> Option<DbcQuote> {
    // Quote-token mode takes a buy's fee from its input; everything else pays
    // it from the output.
    let fee_on_input = is_buy && config.collect_fee_mode == COLLECT_FEE_QUOTE_TOKEN;
    let (mut actual_in, mut fee) =
        if fee_on_input { excluded_fee_amount(fee_numerator, amount_in)? } else { (amount_in, 0) };
    let swap = if is_buy {
        quote_to_base(config, sqrt_price, actual_in)?
    } else {
        base_to_quote(config, sqrt_price, actual_in)?
    };
    let mut included_in = amount_in;
    if swap.amount_left != 0 {
        actual_in = actual_in.checked_sub(swap.amount_left)?;
        if fee_on_input {
            included_in = included_fee_amount(fee_numerator, actual_in)?;
            fee = included_in.checked_sub(actual_in)?;
        } else {
            included_in = actual_in;
        }
    }
    let amount_out = if fee_on_input {
        swap.amount_out
    } else {
        let (amount_out, output_fee) = excluded_fee_amount(fee_numerator, swap.amount_out)?;
        fee = output_fee;
        amount_out
    };
    Some(DbcQuote {
        amount_in: included_in,
        amount_out,
        fee,
        next_sqrt_price: swap.next_sqrt_price,
        amount_left: swap.amount_left,
    })
}

/// The program's Q64.64 power by squaring.
fn pow_q64(base: u128, exponent: u32) -> Option<u128> {
    if exponent == 0 {
        return Some(ONE_Q64);
    }
    if exponent >= MAX_EXPONENTIAL {
        return None;
    }
    let mut invert = false;
    let mut squared_base = base;
    let mut result = ONE_Q64;
    if squared_base >= result {
        squared_base = u128::MAX.checked_div(squared_base)?;
        invert = true;
    }
    let mut bit = 1u32;
    while bit < MAX_EXPONENTIAL {
        if exponent & bit != 0 {
            result = result.checked_mul(squared_base)? >> 64;
        }
        squared_base = squared_base.checked_mul(squared_base)? >> 64;
        bit <<= 1;
    }
    if result == 0 {
        return None;
    }
    if invert {
        result = u128::MAX.checked_div(result)?;
    }
    Some(result)
}

/// A fee scheduler's numerator `periods` after activation.
fn scheduler_fee_numerator(config: &DbcConfig, periods: u64) -> Option<u64> {
    let fee = &config.base_fee;
    let periods = periods.min(u64::from(fee.first_factor));
    match fee.base_fee_mode {
        BASE_FEE_SCHEDULER_LINEAR => {
            fee.cliff_fee_numerator.checked_sub(fee.third_factor.checked_mul(periods)?)
        }
        BASE_FEE_SCHEDULER_EXPONENTIAL => {
            // cliff * (1 - reduction / 10_000) ^ periods
            let reduction = (u128::from(fee.third_factor) << 64) / BASIS_POINT_MAX;
            let base = ONE_Q64.checked_sub(reduction)?;
            let power = pow_q64(base, u32::try_from(periods).ok()?)?;
            u64::try_from(power.checked_mul(u128::from(fee.cliff_fee_numerator))? >> 64).ok()
        }
        _ => None,
    }
}

/// A rate limiter's numerator for a buy of `amount`, fee included: the cliff
/// fee up to the reference amount, then a step more for each further
/// reference amount, up to the maximum fee.
pub fn rate_limiter_fee_numerator(config: &DbcConfig, amount: u64) -> Option<u64> {
    let fee = &config.base_fee;
    let reference_amount = fee.third_factor;
    let increment = u128::from(fee.first_factor) * u128::from(FEE_DENOMINATOR) / BASIS_POINT_MAX;
    if amount <= reference_amount || reference_amount == 0 || increment == 0 {
        return Some(fee.cliff_fee_numerator);
    }
    let cliff = U256::from(fee.cliff_fee_numerator);
    let increment = U256::from(increment);
    let reference = U256::from(reference_amount);
    let steps = U256::from((amount - reference_amount) / reference_amount);
    let rest = U256::from((amount - reference_amount) % reference_amount);
    let max_steps = U256::from(MAX_FEE_NUMERATOR.checked_sub(fee.cliff_fee_numerator)?) / increment;
    let one = U256::one();
    let two = U256::from(2u8);
    let fee_numerator_sum = if steps < max_steps {
        let full = cliff + cliff * steps + increment * steps * (steps + one) / two;
        reference * full + rest * (cliff + increment * (steps + one))
    } else {
        let full = cliff + cliff * max_steps + increment * max_steps * (max_steps + one) / two;
        let beyond = (steps - max_steps) * reference + rest;
        reference * full + beyond * U256::from(MAX_FEE_NUMERATOR)
    };
    let trading_fee = ceil_div(fee_numerator_sum, U256::from(FEE_DENOMINATOR))?;
    to_u64(ceil_div(trading_fee * U256::from(FEE_DENOMINATOR), U256::from(amount))?)
}

/// Whether the rate limiter prices a swap at `current_point`: buys only, for
/// its duration after the pool's activation. Such a buy must pass the
/// instructions sysvar.
pub fn rate_limiter_applies(
    config: &DbcConfig,
    activation_point: u64,
    current_point: u64,
    is_buy: bool,
) -> bool {
    let fee = &config.base_fee;
    fee.base_fee_mode == BASE_FEE_RATE_LIMITER
        && is_buy
        && (fee.first_factor != 0 || fee.second_factor != 0 || fee.third_factor != 0)
        && u128::from(current_point) <= u128::from(activation_point) + u128::from(fee.second_factor)
}

/// The dynamic fee's numerator at a pool's volatility.
fn dynamic_fee_numerator(config: &DbcConfig, volatility_accumulator: u128) -> Option<u128> {
    let fee = &config.dynamic_fee;
    if !fee.initialized {
        return Some(0);
    }
    let square = volatility_accumulator.checked_mul(u128::from(fee.bin_step))?.checked_pow(2)?;
    let variable = square.checked_mul(u128::from(fee.variable_fee_control))?;
    Some(variable.checked_add(99_999_999_999)? / 100_000_000_000)
}

/// The fee numerator of a swap of `amount` (fee included) at `current_point`,
/// a slot or a unix timestamp by the config's activation type: the base fee
/// of the config's mode plus the dynamic fee, at most the maximum fee.
pub fn fee_numerator(
    config: &DbcConfig,
    activation_point: u64,
    volatility_accumulator: u128,
    current_point: u64,
    is_buy: bool,
    amount: u64,
) -> Result<u64> {
    let fee = &config.base_fee;
    let base = match fee.base_fee_mode {
        BASE_FEE_SCHEDULER_LINEAR | BASE_FEE_SCHEDULER_EXPONENTIAL => {
            // No schedule when the period length is zero.
            match current_point.saturating_sub(activation_point).checked_div(fee.second_factor) {
                None => Some(fee.cliff_fee_numerator),
                Some(periods) => scheduler_fee_numerator(config, periods),
            }
        }
        BASE_FEE_RATE_LIMITER => {
            if rate_limiter_applies(config, activation_point, current_point, is_buy) {
                rate_limiter_fee_numerator(config, amount)
            } else {
                Some(fee.cliff_fee_numerator)
            }
        }
        _ => None,
    }
    .ok_or_else(|| anyhow!("Meteora DBC base fee mode {} is not priced", fee.base_fee_mode))?;
    let dynamic = dynamic_fee_numerator(config, volatility_accumulator)
        .ok_or_else(|| anyhow!("Meteora DBC dynamic fee overflows"))?;
    Ok(u64::try_from(u128::from(base) + dynamic).unwrap_or(u64::MAX).min(MAX_FEE_NUMERATOR))
}

/// The point a config counts time in, of a slot and a unix timestamp.
pub fn current_point(config: &DbcConfig, slot: u64, unix_timestamp: u64) -> u64 {
    if config.activation_type == ACTIVATION_SLOT {
        slot
    } else {
        unix_timestamp
    }
}

/// The fee numerator a swap paid, from what it reports: its fees, and the
/// amount they were taken from. The next swap in the pool pays about the same.
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
    use crate::instruction::utils::meteora_dbc_types::{
        DbcBaseFee, DbcCurvePoint, DbcDynamicFee, ACTIVATION_TIMESTAMP,
    };

    /// The single-segment curve of mainnet pool
    /// 2Rz8zRLAqMtXKBGsxb8DwYN1Ed13TDwLtxNUrEUHtBJY (config
    /// CchPHVPXdshYVhUd3ExZDd9vJQgW3NK5jhesoeewLZp8): USDC quote, 2% fee.
    fn single_segment() -> DbcConfig {
        DbcConfig {
            base_fee: DbcBaseFee { cliff_fee_numerator: 20_000_000, ..Default::default() },
            collect_fee_mode: COLLECT_FEE_QUOTE_TOKEN,
            activation_type: ACTIVATION_TIMESTAMP,
            migration_quote_threshold: 10_000_000_000,
            migration_sqrt_price: 1_750_011_800_614_054_764,
            sqrt_start_price: 583_337_266_871_351_588,
            curve: vec![DbcCurvePoint {
                sqrt_price: 1_837_512_390_644_757_503,
                liquidity: 2_916_686_334_356_757_942_357_946_112_045,
            }],
            ..Default::default()
        }
    }

    /// The price the pool was at after the swap of transaction ysZEH25d…, the
    /// buy the DBC fixtures of sol-parser-sdk start from.
    const AFTER_SAMPLE_BUY: u128 = 0x09eb_9d3b_f0a7_1eb7;

    // The mainnet swaps below start from the price the pool's swap before
    // them reported, and must give what their own events report.

    #[test]
    fn buy_reproduces_a_mainnet_swap() {
        // Transaction 4V1rijYshdgHKQB8…: 56.387707 USDC in.
        let quote = quote_exact_in(
            &single_segment(),
            1_730_409_438_693_799_042,
            20_000_000,
            true,
            56_387_707,
        )
        .unwrap();
        assert_eq!(
            quote,
            DbcQuote {
                amount_in: 56_387_707,
                amount_out: 6_256_581_995,
                fee: 1_127_755,
                next_sqrt_price: 1_736_856_476_567_223_457,
                amount_left: 0,
            }
        );
    }

    #[test]
    fn partial_fill_stops_at_the_migration_price() {
        // Transaction 5BDEC1rK5Gvt74Nw…: 117.61941 USDC offered; the curve
        // took 53.981573 and completed.
        let config = single_segment();
        let quote =
            quote_exact_in(&config, 1_743_839_865_990_686_897, 20_000_000, true, 117_619_410)
                .unwrap();
        assert_eq!(
            quote,
            DbcQuote {
                amount_in: 53_981_573,
                amount_out: 5_898_797_192,
                fee: 1_079_632,
                next_sqrt_price: config.migration_sqrt_price,
                amount_left: 62_365_080,
            }
        );
    }

    /// The curve of mainnet pool HLfVDQpRWL9KEHVXbGiRcgE3epYMLckY4iJa1KKaQvvD
    /// (config 72hdX81iELb1VU8TAvsbdg5AoEiDzF5X16XLecyjBNcY): WSOL quote, 3%
    /// fee, a second segment beyond the migration price.
    fn sol_pool() -> DbcConfig {
        DbcConfig {
            base_fee: DbcBaseFee { cliff_fee_numerator: 30_000_000, ..Default::default() },
            collect_fee_mode: COLLECT_FEE_QUOTE_TOKEN,
            activation_type: ACTIVATION_SLOT,
            migration_quote_threshold: 74_958_614_076,
            migration_sqrt_price: 347_772_775_840_093_967,
            sqrt_start_price: 9_294_618_542_784_383,
            curve: vec![
                DbcCurvePoint {
                    sqrt_price: 347_772_775_840_093_967,
                    liquidity: 100_095_891_004_270_435_465_244_482_450_288,
                },
                DbcCurvePoint {
                    sqrt_price: 79_226_673_521_066_979_257_578_248_091,
                    liquidity: 3_388_032_290_707_789_754_186_664,
                },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn sol_pool_buy_and_sale_reproduce_mainnet_swaps() {
        let config = sol_pool();
        let buy = quote_exact_in(&config, 134_482_582_197_345_447, 30_000_000, true, 966_242_269)
            .unwrap();
        assert_eq!(
            buy,
            DbcQuote {
                amount_in: 966_242_269,
                amount_out: 17_226_439_266_910,
                fee: 28_987_269,
                next_sqrt_price: 137_668_840_360_479_011,
                amount_left: 0,
            }
        );
        let sale =
            quote_exact_in(&config, 137_668_840_360_479_011, 30_000_000, false, 14_439_370_483_139)
                .unwrap();
        assert_eq!(
            sale,
            DbcQuote {
                amount_in: 14_439_370_483_139,
                amount_out: 764_912_087,
                fee: 23_657_075,
                next_sqrt_price: 134_988_049_186_048_558,
                amount_left: 0,
            }
        );
    }

    #[test]
    fn sale_pays_its_fee_from_the_output() {
        let config = single_segment();
        let buy = quote_exact_in(&config, AFTER_SAMPLE_BUY, 20_000_000, true, 100_000_000).unwrap();
        // Selling what the buy bought returns the quote less two fees and
        // rounding.
        let sale = quote_exact_in(&config, buy.next_sqrt_price, 20_000_000, false, buy.amount_out)
            .unwrap();
        assert_eq!(sale.amount_in, buy.amount_out);
        assert_eq!(sale.amount_left, 0);
        assert!(sale.next_sqrt_price >= AFTER_SAMPLE_BUY);
        assert!(sale.next_sqrt_price - AFTER_SAMPLE_BUY < 1 << 40);
        let gross = sale.amount_out + sale.fee;
        assert!((97_999_990..=98_000_000).contains(&gross), "{gross}");
        assert_eq!(sale.fee, (u128::from(gross) * 20_000_000).div_ceil(1_000_000_000) as u64);
    }

    #[test]
    fn sale_below_the_start_price_gives_the_rest_back() {
        let config = single_segment();
        // A little above the start price: the curve holds almost no quote.
        let sqrt_price = config.sqrt_start_price + (1 << 30);
        let quote = quote_exact_in(&config, sqrt_price, 20_000_000, false, u64::MAX / 2).unwrap();
        assert_eq!(quote.next_sqrt_price, config.sqrt_start_price);
        assert!(quote.amount_left > 0);
        assert!(quote.amount_in < u64::MAX / 2);
    }

    /// Two segments, as most mainnet configs have.
    fn two_segments() -> DbcConfig {
        DbcConfig {
            base_fee: DbcBaseFee { cliff_fee_numerator: 30_000_000, ..Default::default() },
            collect_fee_mode: COLLECT_FEE_QUOTE_TOKEN,
            migration_sqrt_price: 400 << 56,
            sqrt_start_price: 100 << 56,
            curve: vec![
                DbcCurvePoint { sqrt_price: 200 << 56, liquidity: 1 << 100 },
                DbcCurvePoint { sqrt_price: 500 << 56, liquidity: 1 << 98 },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn swaps_cross_segments_both_ways() {
        let config = two_segments();
        let start = 150 << 56;
        // Enough quote to leave the first segment.
        let first_segment = to_u64(delta_quote(start, 200 << 56, 1 << 100, true).unwrap()).unwrap();
        let amount_in = included_fee_amount(30_000_000, first_segment * 2).unwrap();
        let buy = quote_exact_in(&config, start, 30_000_000, true, amount_in).unwrap();
        assert!(buy.next_sqrt_price > 200 << 56);
        assert_eq!(buy.amount_left, 0);

        // The sale walks back through both segments to where the buy started.
        let sale = quote_exact_in(&config, buy.next_sqrt_price, 30_000_000, false, buy.amount_out)
            .unwrap();
        assert!(sale.next_sqrt_price >= start);
        assert!(sale.next_sqrt_price - start < 1 << 40, "{}", sale.next_sqrt_price - start);
        let gross = sale.amount_out + sale.fee;
        assert!(first_segment * 2 - gross < 10, "{gross}");
    }

    #[test]
    fn output_token_mode_takes_a_buys_fee_from_the_tokens() {
        let mut config = single_segment();
        let quote_mode =
            quote_exact_in(&config, AFTER_SAMPLE_BUY, 20_000_000, true, 100_000_000).unwrap();
        config.collect_fee_mode = 1;
        let output_mode =
            quote_exact_in(&config, AFTER_SAMPLE_BUY, 20_000_000, true, 100_000_000).unwrap();
        // The whole input reaches the curve, and the fee is in base tokens.
        assert!(output_mode.next_sqrt_price > quote_mode.next_sqrt_price);
        assert_eq!(output_mode.amount_in, 100_000_000);
        let gross = output_mode.amount_out + output_mode.fee;
        assert_eq!(
            output_mode.fee,
            (u128::from(gross) * 20_000_000).div_ceil(1_000_000_000) as u64
        );
    }

    #[test]
    fn linear_scheduler_steps_down_and_stops() {
        let mut config = single_segment();
        config.base_fee = DbcBaseFee {
            cliff_fee_numerator: 500_000_000,
            first_factor: 10,
            second_factor: 60,
            third_factor: 49_000_000,
            base_fee_mode: BASE_FEE_SCHEDULER_LINEAR,
        };
        let at = |seconds: u64| fee_numerator(&config, 1_000, 0, 1_000 + seconds, true, 1).unwrap();
        assert_eq!(at(0), 500_000_000);
        assert_eq!(at(59), 500_000_000);
        assert_eq!(at(60), 451_000_000);
        assert_eq!(at(599), 59_000_000);
        assert_eq!(at(600), 10_000_000);
        assert_eq!(at(100_000), 10_000_000);
        // A clock behind the activation point quotes the cliff fee.
        assert_eq!(fee_numerator(&config, 1_000, 0, 900, true, 1).unwrap(), 500_000_000);
    }

    #[test]
    fn exponential_scheduler_decays_by_its_factor() {
        let mut config = single_segment();
        config.base_fee = DbcBaseFee {
            cliff_fee_numerator: 500_000_000,
            first_factor: 100,
            second_factor: 10,
            third_factor: 1_000, // 10% a period
            base_fee_mode: BASE_FEE_SCHEDULER_EXPONENTIAL,
        };
        let at = |periods: u64| fee_numerator(&config, 0, 0, periods * 10, true, 1).unwrap();
        assert_eq!(at(0), 500_000_000);
        for (periods, expected) in [(1u64, 450_000_000u64), (2, 405_000_000), (10, 174_339_220)] {
            let numerator = at(periods);
            assert!(expected.abs_diff(numerator) < 10, "{periods}: {numerator}");
        }
        assert_eq!(at(100), at(5_000));
    }

    #[test]
    fn rate_limiter_charges_more_for_larger_buys_while_it_lasts() {
        let mut config = single_segment();
        config.base_fee = DbcBaseFee {
            cliff_fee_numerator: 10_000_000,
            first_factor: 100, // +1% a reference amount
            second_factor: 300,
            third_factor: 1_000_000_000,
            base_fee_mode: BASE_FEE_RATE_LIMITER,
        };
        let buy = |amount, at| fee_numerator(&config, 1_000, 0, at, true, amount).unwrap();
        assert_eq!(buy(1_000_000_000, 1_100), 10_000_000);
        // Two reference amounts: 1% on the first, 2% on the second.
        assert_eq!(buy(2_000_000_000, 1_100), 15_000_000);
        assert!(buy(50_000_000_000, 1_100) > buy(5_000_000_000, 1_100));
        assert!(buy(u64::MAX, 1_100) <= MAX_FEE_NUMERATOR);
        // Past its duration, and for every sale, the cliff fee.
        assert_eq!(buy(50_000_000_000, 1_301), 10_000_000);
        assert_eq!(fee_numerator(&config, 1_000, 0, 1_100, false, u64::MAX).unwrap(), 10_000_000);
        assert!(rate_limiter_applies(&config, 1_000, 1_300, true));
        assert!(!rate_limiter_applies(&config, 1_000, 1_301, true));
        assert!(!rate_limiter_applies(&config, 1_000, 1_100, false));
    }

    #[test]
    fn dynamic_fee_adds_to_the_base_fee_up_to_the_maximum() {
        let mut config = single_segment();
        config.dynamic_fee = DbcDynamicFee {
            initialized: true,
            bin_step: 1,
            variable_fee_control: 2_000_000,
            ..Default::default()
        };
        assert_eq!(fee_numerator(&config, 0, 0, 0, true, 1).unwrap(), 20_000_000);
        // (100_000 * 1)^2 * 2_000_000 / 1e11 = 200_000
        assert_eq!(fee_numerator(&config, 0, 100_000, 0, true, 1).unwrap(), 20_200_000);
        assert_eq!(fee_numerator(&config, 0, u64::MAX as u128, 0, true, 1).ok(), None);
        assert_eq!(
            fee_numerator(&config, 0, 10_000_000_000, 0, true, 1).unwrap(),
            MAX_FEE_NUMERATOR
        );
    }

    #[test]
    fn fee_paid_recovers_the_numerator() {
        // 2% of 190_746_849, rounded up, as the sample buy paid.
        assert_eq!(fee_numerator_paid(3_814_937, 190_746_849), 20_000_000);
        assert_eq!(fee_numerator_paid(1, 0), 0);
        assert_eq!(fee_numerator_paid(u64::MAX, 1), MAX_FEE_NUMERATOR);
    }

    #[test]
    fn points_follow_the_activation_type() {
        let mut config = single_segment();
        assert_eq!(current_point(&config, 5, 9), 9);
        config.activation_type = ACTIVATION_SLOT;
        assert_eq!(current_point(&config, 5, 9), 5);
    }
}
