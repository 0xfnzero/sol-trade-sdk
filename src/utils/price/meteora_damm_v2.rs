/// Calculate the price of token0 in token1
///
/// # Arguments
/// * `sqrt_price_x64` - The sqrt price of the pool
/// * `decimals_token0` - The decimals of token0
/// * `decimals_token1` - The decimals of token1
///
/// # Returns
/// The price of token0 in token1
pub fn price_token0_in_token1(
    sqrt_price_x64: u128,
    decimals_token0: u8,
    decimals_token1: u8,
) -> f64 {
    let sqrt_price = sqrt_price_x64 as f64 / (1u128 << 64) as f64; // Q64.64 转浮点
    let price_raw = sqrt_price * sqrt_price; // 未调整小数位的价格
    let scale = 10f64.powi((decimals_token0 as i32) - (decimals_token1 as i32));
    price_raw * scale
}

/// Calculate the price of token1 in token0
///
/// # Arguments
/// * `sqrt_price_x64` - The sqrt price of the pool
/// * `decimals_token0` - The decimals of token0
/// * `decimals_token1` - The decimals of token1
///
/// # Returns
/// The price of token1 in token0
pub fn price_token1_in_token0(
    sqrt_price_x64: u128,
    decimals_token0: u8,
    decimals_token1: u8,
) -> f64 {
    1.0 / price_token0_in_token1(sqrt_price_x64, decimals_token0, decimals_token1)
}
