use sol_trade_sdk::instruction::utils::raydium_cpmm::accounts::RAYDIUM_CPMM;

#[test]
fn program_id_verified() {
    let expected = "CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C".parse().unwrap();
    assert_eq!(RAYDIUM_CPMM, expected);
}

#[test]
fn swap_in_discriminator() {
    use sol_trade_sdk::instruction::utils::raydium_cpmm::SWAP_BASE_IN_DISCRIMINATOR;
    assert_eq!(SWAP_BASE_IN_DISCRIMINATOR.len(), 8);
    assert_eq!(SWAP_BASE_IN_DISCRIMINATOR, [143, 190, 90, 218, 196, 30, 51, 222]);
}

#[test]
fn swap_out_discriminator() {
    use sol_trade_sdk::instruction::utils::raydium_cpmm::SWAP_BASE_OUT_DISCRIMINATOR;
    assert_eq!(SWAP_BASE_OUT_DISCRIMINATOR.len(), 8);
    assert_eq!(SWAP_BASE_OUT_DISCRIMINATOR, [55, 217, 98, 86, 163, 74, 180, 173]);
}

#[test]
fn swap_in_and_out_differ() {
    use sol_trade_sdk::instruction::utils::raydium_cpmm::{
        SWAP_BASE_IN_DISCRIMINATOR, SWAP_BASE_OUT_DISCRIMINATOR,
    };
    assert_ne!(SWAP_BASE_IN_DISCRIMINATOR, SWAP_BASE_OUT_DISCRIMINATOR);
}

// ── PDA Derivation ──

#[test]
fn pool_pda_is_deterministic() {
    use sol_trade_sdk::instruction::utils::raydium_cpmm::get_pool_pda;
    let amm_config: solana_sdk::pubkey::Pubkey =
        "GpMZbSM2GgvTKHJirzeGfMFoaZ8UR2X7F4v8vHTvxFbL".parse().unwrap();
    let mint1: solana_sdk::pubkey::Pubkey =
        "So11111111111111111111111111111111111111111".parse().unwrap();
    let mint2: solana_sdk::pubkey::Pubkey =
        "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".parse().unwrap();
    let pda = get_pool_pda(&amm_config, &mint1, &mint2);
    assert!(pda.is_some(), "pool PDA should derive for valid mints");
}

// ── Fee Rate Constants ──

#[test]
fn trade_fee_rate_non_zero() {
    use sol_trade_sdk::instruction::utils::raydium_cpmm::accounts::TRADE_FEE_RATE;
    assert!(TRADE_FEE_RATE > 0);
}

#[test]
fn fee_rate_denominator_correct() {
    use sol_trade_sdk::instruction::utils::raydium_cpmm::accounts::FEE_RATE_DENOMINATOR_VALUE;
    assert_eq!(FEE_RATE_DENOMINATOR_VALUE, 1_000_000);
}
