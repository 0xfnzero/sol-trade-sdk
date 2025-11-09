pub fn price_token0_in_token1(
    price: f64,
    decimals_token0: u8,
    decimals_token1: u8,
) -> f64 {
    let scale = 10f64.powi((decimals_token0 as i32) - (decimals_token1 as i32));
    price * scale
}