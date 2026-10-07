//! Pump v3 quote math. Supply current resolved curve fee rates and actual vault balance.
use anyhow::{ensure, Result};
#[derive(Debug, Clone, Copy, Default)]
pub struct PumpV3QuoteState {
    pub virtual_base: u64,
    pub virtual_quote: u64,
    pub remaining_base: u64,
    pub real_quote: u64,
    pub curve_base_balance: u64,
    pub migration_fee: u64,
    pub protocol_bps: u64,
    pub creator_bps: u64,
    pub complete: bool,
    pub mayhem: bool,
}
#[derive(Debug, Clone, Copy, Default)]
pub struct PumpV3Quote {
    pub curve_base: u64,
    pub curve_quote: u64,
    pub pool_base: u64,
    pub pool_quote: u64,
    pub protocol_fee: u64,
    pub creator_fee: u64,
    pub base_out: u64,
    pub quote_in: u64,
}
fn validate(s: &PumpV3QuoteState) -> Result<()> {
    ensure!(!s.complete, "BondingCurveComplete");
    ensure!(
        s.virtual_base > s.remaining_base
            && s.virtual_quote > 0
            && (s.protocol_bps as u128 + s.creator_bps as u128) <= 10000,
        "invalid curve reserves or fee rates"
    );
    Ok(())
}
fn fee(n: u128, bps: u64) -> u128 {
    (n * bps as u128 + 9999) / 10000
}
fn net(b: u64, s: &PumpV3QuoteState) -> u128 {
    let b = b as u128;
    let n = b * 10000 / (10000 + s.protocol_bps as u128 + s.creator_bps as u128);
    let c = n + fee(n, s.protocol_bps) + fee(n, s.creator_bps);
    n - c.saturating_sub(b)
}
fn curve(n: u64, s: &PumpV3QuoteState) -> Result<u64> {
    if n == 0 {
        return Ok(0);
    }
    Ok(u64::try_from(n as u128 * s.virtual_quote as u128 / (s.virtual_base - n) as u128 + 1)?)
}
fn pool(s: &PumpV3QuoteState, cq: u64) -> Result<(u128, u128)> {
    let b = (s.curve_base_balance as u128).checked_sub(s.remaining_base as u128).unwrap_or(0);
    let q = (s.real_quote as u128 + cq as u128).checked_sub(s.migration_fee as u128).unwrap_or(0);
    ensure!(b > 0 && q > 0, "empty pool-to-be; fetch actual curve base vault balance");
    Ok((b, q))
}
fn result(cb: u64, cq: u64, pb: u64, pq: u64, s: &PumpV3QuoteState) -> Result<PumpV3Quote> {
    let pf = fee(cq as u128, s.protocol_bps) + fee(pq as u128, s.protocol_bps);
    let cf = fee(cq as u128, s.creator_bps) + fee(pq as u128, s.creator_bps);
    Ok(PumpV3Quote {
        curve_base: cb,
        curve_quote: cq,
        pool_base: pb,
        pool_quote: pq,
        protocol_fee: u64::try_from(pf)?,
        creator_fee: u64::try_from(cf)?,
        base_out: u64::try_from(cb as u128 + pb as u128)?,
        quote_in: u64::try_from(cq as u128 + pq as u128 + pf + cf)?,
    })
}
pub fn quote_pump_buy_v3_exact_out(
    s: &PumpV3QuoteState,
    amount: u64,
    partial_fill: bool,
) -> Result<PumpV3Quote> {
    validate(s)?;
    let cb = amount.min(s.remaining_base);
    let cq = curve(cb, s)?;
    if amount <= s.remaining_base {
        return result(cb, cq, 0, 0, s);
    }
    if s.mayhem {
        ensure!(partial_fill, "NotEnoughTokensToBuy");
        return result(cb, cq, 0, 0, s);
    }
    let (b, q) = pool(s, cq)?;
    let pb = amount - cb;
    ensure!((pb as u128) < b, "NotEnoughTokensToBuy");
    let den = b - pb as u128;
    let num = q
        .checked_mul(pb as u128)
        .and_then(|n| n.checked_add(den - 1))
        .ok_or_else(|| anyhow::anyhow!("quote overflow"))?;
    result(cb, cq, pb, u64::try_from(num / den)?, s)
}
pub fn quote_pump_buy_v3_exact_in(s: &PumpV3QuoteState, budget: u64) -> Result<PumpV3Quote> {
    validate(s)?;
    let n = net(budget, s);
    if n <= 1 {
        return Ok(PumpV3Quote::default());
    }
    let tokens =
        u64::try_from((n - 1) * s.virtual_base as u128 / (s.virtual_quote as u128 + n - 1))?;
    let cb = tokens.min(s.remaining_base);
    let cq = curve(cb, s)?;
    if tokens <= s.remaining_base || s.mayhem {
        return result(cb, cq, 0, 0, s);
    }
    let spent = cq as u128 + fee(cq as u128, s.protocol_bps) + fee(cq as u128, s.creator_bps);
    if spent >= budget as u128 {
        return result(cb, cq, 0, 0, s);
    }
    let leg = net(u64::try_from(budget as u128 - spent)?, s);
    if leg <= 1 {
        return result(cb, cq, 0, 0, s);
    }
    let (b, q) = pool(s, cq)?;
    let pb = u64::try_from((leg - 1) * b / (q + leg - 1))?;
    if pb == 0 {
        return result(cb, cq, 0, 0, s);
    }
    result(cb, cq, pb, u64::try_from(leg)?, s)
}

/// For migrated Q, effective_quote is actual quote vault + signed virtual reserves.
/// Deeper Q uses its stored initial quote seed. Caller verifies quote eligibility.
pub fn pump_coin_initial_quote_reserves(
    seed: u64,
    quote_base: u64,
    effective_quote: i128,
    initial_base: u64,
    initial_real: u64,
    quote_supply: u64,
    depth: u8,
    max_depth: u8,
) -> Result<u64> {
    ensure!(
        effective_quote > 0 && initial_base > initial_real && depth < max_depth,
        "invalid Pump quote state or CurveDepthExceeded"
    );
    let reserves = seed as u128 * quote_base as u128 / effective_quote as u128;
    let reserves = u64::try_from(reserves)?;
    ensure!(
        reserves > 0
            && reserves as u128 * initial_real as u128 / (initial_base - initial_real) as u128
                <= quote_supply as u128,
        "QuoteReservesOutOfRange"
    );
    Ok(reserves)
}
