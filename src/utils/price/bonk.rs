use crate::constants::decimals::{DEFAULT_TOKEN_DECIMALS, SOL_DECIMALS};

/// Calculate the price of token in WSOL
///
/// # Arguments
/// * `virtual_base` - Virtual base reserves
/// * `virtual_quote` - Virtual quote reserves
/// * `real_base` - Real base reserves
/// * `real_quote` - Real quote reserves
///
/// # Returns
/// The price of token in WSOL
pub fn price_token_in_wsol(
    virtual_base: u64,
    virtual_quote: u64,
    real_base: u64,
    real_quote: u64,
) -> f64 {
    price_base_in_quote(
        virtual_base,
        virtual_quote,
        real_base,
        real_quote,
        DEFAULT_TOKEN_DECIMALS,
        SOL_DECIMALS,
    )
}

/// Calculate the price of base in quote
///
/// # Arguments
/// * `virtual_base` - Virtual base reserves
/// * `virtual_quote` - Virtual quote reserves
/// * `real_base` - Real base reserves
/// * `real_quote` - Real quote reserves
/// * `base_decimals` - Base decimals
/// * `quote_decimals` - Quote decimals
///
/// # Returns
/// The price of base in quote
pub fn price_base_in_quote(
    virtual_base: u64,
    virtual_quote: u64,
    real_base: u64,
    real_quote: u64,
    base_decimals: u8,
    quote_decimals: u8,
) -> f64 {
    // Calculate decimal places difference
    let decimal_diff = quote_decimals as i32 - base_decimals as i32;
    let decimal_factor = if decimal_diff >= 0 {
        10_f64.powi(decimal_diff)
    } else {
        1.0 / 10_f64.powi(-decimal_diff)
    };
    // Calculate reserves state before price calculation
    let quote_reserves = virtual_quote.checked_add(real_quote).unwrap_or(0);
    let base_reserves = virtual_base.checked_sub(real_base).unwrap_or(0);

    if base_reserves == 0 {
        return 0.0;
    }

    if decimal_factor == 0.0 {
        return 0.0;
    }

    // Use floating point calculation to avoid precision loss from integer division
    let price = (quote_reserves as f64) / (base_reserves as f64) / decimal_factor;

    price
}
