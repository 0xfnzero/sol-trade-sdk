use crate::instruction::utils::pumpfun::global_constants::{LAMPORTS_PER_SOL, SCALE};

/// Calculate the token price in SOL based on virtual reserves
///
/// # Arguments
/// * `virtual_sol_reserves` - Virtual SOL reserves in the bonding curve
/// * `virtual_token_reserves` - Virtual token reserves in the bonding curve
///
/// # Returns
/// Token price in SOL as f64
pub fn price_token_in_sol(virtual_sol_reserves: u64, virtual_token_reserves: u64) -> f64 {
    let v_sol = virtual_sol_reserves as f64 / LAMPORTS_PER_SOL as f64;
    let v_tokens = virtual_token_reserves as f64 / SCALE as f64;
    if v_tokens == 0.0 {
        return 0.0;
    }
    v_sol / v_tokens
}
