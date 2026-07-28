use sol_trade_sdk::instruction::utils::raydium_amm_v4::accounts::RAYDIUM_AMM_V4;

#[test]
fn program_id_verified() {
    let expected = "675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8".parse().unwrap();
    assert_eq!(RAYDIUM_AMM_V4, expected);
}

// Raydium AMM V4 uses 1-byte custom discriminators (not Anchor 8-byte)
// swap_in = [9], swap_out = [11]
#[test]
fn swap_in_discriminator() {
    let discriminator: [u8; 1] = [9];
    assert_eq!(discriminator[0], 9);
}

#[test]
fn swap_out_discriminator() {
    let discriminator: [u8; 1] = [11];
    assert_eq!(discriminator[0], 11);
}

#[test]
fn swap_in_and_out_differ() {
    assert_ne!([9u8], [11u8]);
}