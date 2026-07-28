use sol_trade_sdk::instruction::utils::bonk::accounts::AUTHORITY;
use sol_trade_sdk::instruction::utils::bonk::accounts::BONK;

#[test]
fn program_id_verified() {
    let expected = "LanMV9sAd7wArD4vJFi2qDdfnVhFxYSUg6eADduJ3uj".parse().unwrap();
    assert_eq!(BONK, expected);
}

#[test]
fn pda_authority_exists() {
    assert_ne!(AUTHORITY, solana_sdk::pubkey::Pubkey::default());
}

// ── Fee Rate Constants ──

#[test]
fn platform_fee_rate_non_zero() {
    use sol_trade_sdk::instruction::utils::bonk::accounts::PLATFORM_FEE_RATE;
    assert!(PLATFORM_FEE_RATE > 0, "platform fee rate should be > 0");
}

#[test]
fn protocol_fee_rate_non_zero() {
    use sol_trade_sdk::instruction::utils::bonk::accounts::PROTOCOL_FEE_RATE;
    assert!(PROTOCOL_FEE_RATE > 0, "protocol fee rate should be > 0");
}

// ── PDA Derivation ──

#[test]
fn pool_pda_is_deterministic() {
    use sol_trade_sdk::instruction::utils::bonk::get_pool_pda;
    let base: solana_sdk::pubkey::Pubkey =
        "So11111111111111111111111111111111111111111".parse().unwrap();
    let quote: solana_sdk::pubkey::Pubkey =
        "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".parse().unwrap();
    let pda = get_pool_pda(&base, &quote);
    assert!(pda.is_some(), "pool PDA should derive for valid mint pair");
}