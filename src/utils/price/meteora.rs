//! Prices of Meteora DBC and DAMM v2 pools, from their Q64.64 square-root
//! price: the raw price is quote per base (token B per token A).

/// Price of a DBC pool's base token in its quote, in UI units.
pub fn dbc_price_base_in_quote(sqrt_price: u128, base_decimals: u8, quote_decimals: u8) -> f64 {
    price_from_sqrt_price(sqrt_price, base_decimals, quote_decimals)
}

/// Price of a DAMM v2 pool's token A in token B, in UI units.
pub fn damm_v2_price_a_in_b(sqrt_price: u128, a_decimals: u8, b_decimals: u8) -> f64 {
    price_from_sqrt_price(sqrt_price, a_decimals, b_decimals)
}

/// Price of a DAMM v2 pool's token B in token A, in UI units; 0 at a zero price.
pub fn damm_v2_price_b_in_a(sqrt_price: u128, a_decimals: u8, b_decimals: u8) -> f64 {
    let price = price_from_sqrt_price(sqrt_price, a_decimals, b_decimals);
    if price == 0.0 {
        0.0
    } else {
        1.0 / price
    }
}

fn price_from_sqrt_price(sqrt_price: u128, decimals: u8, quote_decimals: u8) -> f64 {
    let sqrt = sqrt_price as f64 / (1u128 << 64) as f64;
    sqrt * sqrt * 10f64.powi(i32::from(decimals) - i32::from(quote_decimals))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dbc_price_matches_what_a_mainnet_buy_paid() {
        // After transaction ysZEH25dfiiZMm94…: 186.931912 USDC had bought
        // 128.399686024 tokens (9 decimals), 1.4558 USDC each on average, and
        // left the pool a little above that.
        let price = dbc_price_base_in_quote(0x09eb_9d3b_f0a7_1eb7, 9, 6);
        assert!((1.45..1.52).contains(&price), "{price}");
    }

    #[test]
    fn damm_prices_are_reciprocal() {
        let sqrt_price = 2_771_183_070_634_004_317;
        let a_in_b = damm_v2_price_a_in_b(sqrt_price, 9, 6);
        // About 22.57 USDC a token after the mainnet sale quoted in the calc tests.
        assert!((22.0..23.0).contains(&a_in_b), "{a_in_b}");
        assert!((damm_v2_price_b_in_a(sqrt_price, 9, 6) * a_in_b - 1.0).abs() < 1e-12);
        assert_eq!(damm_v2_price_b_in_a(0, 9, 6), 0.0);
    }
}
