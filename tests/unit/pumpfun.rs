use sol_trade_sdk::instruction::utils::pumpfun::accounts::PUMPFUN;
use sol_trade_sdk::instruction::utils::pumpfun::global_constants::{
    CREATOR_FEE, FEE_BASIS_POINTS, FEE_RECIPIENT, INITIAL_VIRTUAL_SOL_RESERVES,
    INITIAL_VIRTUAL_TOKEN_RESERVES,
};
use sol_trade_sdk::instruction::utils::pumpfun::{BUY_DISCRIMINATOR, SELL_DISCRIMINATOR};

// ── Program ID ──

#[test]
fn program_id_verified() {
    let expected = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P".parse().unwrap();
    assert_eq!(PUMPFUN, expected);
}

// ── Fee Constants ──

#[test]
fn fee_basis_points_matches_on_chain() {
    assert_eq!(FEE_BASIS_POINTS, 100);
}

#[test]
fn creator_fee_is_correct() {
    assert_eq!(CREATOR_FEE, 30);
}

#[test]
fn total_fee_with_creator() {
    assert_eq!(FEE_BASIS_POINTS + CREATOR_FEE, 130);
}

#[test]
fn total_fee_without_creator() {
    assert_eq!(FEE_BASIS_POINTS, 100);
}

// ── Reserve Constants ──

#[test]
fn initial_virtual_token_reserves_verified() {
    assert_eq!(INITIAL_VIRTUAL_TOKEN_RESERVES, 1_073_000_000_000_000);
}

#[test]
fn initial_virtual_sol_reserves_verified() {
    assert_eq!(INITIAL_VIRTUAL_SOL_RESERVES, 30_000_000_000);
}

#[test]
fn fee_recipient_is_non_default() {
    assert_ne!(FEE_RECIPIENT, solana_sdk::pubkey::Pubkey::default());
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

// ── PDA Derivation ──

#[test]
fn bonding_curve_pda_is_deterministic() {
    use sol_trade_sdk::instruction::utils::pumpfun::get_bonding_curve_pda;
    let mint: solana_sdk::pubkey::Pubkey =
        "So11111111111111111111111111111111111111111".parse().unwrap();
    assert!(get_bonding_curve_pda(&mint).is_some());
}

#[test]
fn bonding_curve_v2_pda_is_deterministic() {
    use sol_trade_sdk::instruction::utils::pumpfun::get_bonding_curve_v2_pda;
    let mint: solana_sdk::pubkey::Pubkey =
        "So11111111111111111111111111111111111111111".parse().unwrap();
    assert!(get_bonding_curve_v2_pda(&mint).is_some());
}
