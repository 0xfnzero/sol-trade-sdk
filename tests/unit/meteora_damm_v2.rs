use sol_trade_sdk::instruction::utils::meteora_damm_v2::accounts::METEORA_DAMM_V2;
use sol_trade_sdk::instruction::utils::meteora_damm_v2::SWAP_DISCRIMINATOR;

#[test]
fn program_id_verified() {
    let expected = "cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG".parse().unwrap();
    assert_eq!(METEORA_DAMM_V2, expected);
}

#[test]
fn swap_discriminator_length() {
    assert_eq!(SWAP_DISCRIMINATOR.len(), 8);
}

#[test]
fn swap_discriminator_values() {
    // Anchor SHA256("global:swap")[..8]
    assert_eq!(SWAP_DISCRIMINATOR, [248, 198, 158, 145, 225, 117, 135, 200]);
}

#[test]
fn event_authority_pda_is_deterministic() {
    use sol_trade_sdk::instruction::utils::meteora_damm_v2::get_event_authority_pda;
    let pda = get_event_authority_pda();
    assert_ne!(pda, solana_sdk::pubkey::Pubkey::default());
}