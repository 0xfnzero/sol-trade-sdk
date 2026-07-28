use sol_trade_sdk::instruction::utils::pumpswap::accounts::AMM_PROGRAM;
use sol_trade_sdk::instruction::utils::pumpswap::accounts::{
    COIN_CREATOR_FEE_BASIS_POINTS, LP_FEE_BASIS_POINTS, PROTOCOL_FEE_BASIS_POINTS,
};
use sol_trade_sdk::instruction::utils::pumpswap::{BUY_DISCRIMINATOR, SELL_DISCRIMINATOR};

// ── Program ID ──

#[test]
fn program_id_verified() {
    let expected = "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA".parse().unwrap();
    assert_eq!(AMM_PROGRAM, expected);
}

// ── Fee Constants ──

#[test]
fn lp_fee_basis_points_default() {
    assert_eq!(LP_FEE_BASIS_POINTS, 25);
}

#[test]
fn protocol_fee_basis_points_default() {
    assert_eq!(PROTOCOL_FEE_BASIS_POINTS, 5);
}

#[test]
fn coin_creator_fee_basis_points_default() {
    assert_eq!(COIN_CREATOR_FEE_BASIS_POINTS, 5);
}

// ── Discriminators ──

#[test]
fn buy_discriminator_length() {
    assert_eq!(BUY_DISCRIMINATOR.len(), 8);
}

#[test]
fn buy_discriminator_values() {
    assert_eq!(BUY_DISCRIMINATOR, [102, 6, 61, 18, 1, 218, 235, 234]);
}

#[test]
fn sell_discriminator_length() {
    assert_eq!(SELL_DISCRIMINATOR.len(), 8);
}

#[test]
fn sell_discriminator_values() {
    assert_eq!(SELL_DISCRIMINATOR, [51, 230, 133, 164, 1, 127, 131, 173]);
}

#[test]
fn buy_and_sell_discriminators_differ() {
    assert_ne!(BUY_DISCRIMINATOR, SELL_DISCRIMINATOR);
}
