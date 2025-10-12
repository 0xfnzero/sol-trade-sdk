use solana_sdk::{pubkey, pubkey::Pubkey};

pub const SYSTEM_PROGRAM: Pubkey = pubkey!("11111111111111111111111111111111");
pub const SYSTEM_PROGRAM_META: solana_sdk::instruction::AccountMeta =
    solana_sdk::instruction::AccountMeta {
        pubkey: SYSTEM_PROGRAM,
        is_signer: false,
        is_writable: false,
    };

pub const TOKEN_PROGRAM: Pubkey = pubkey!("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
pub const TOKEN_PROGRAM_META: solana_sdk::instruction::AccountMeta =
    solana_sdk::instruction::AccountMeta {
        pubkey: TOKEN_PROGRAM,
        is_signer: false,
        is_writable: false,
    };

pub const TOKEN_PROGRAM_2022: Pubkey = pubkey!("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");
pub const TOKEN_PROGRAM_2022_META: solana_sdk::instruction::AccountMeta =
    solana_sdk::instruction::AccountMeta {
        pubkey: TOKEN_PROGRAM_2022,
        is_signer: false,
        is_writable: false,
    };

pub const SOL_TOKEN_ACCOUNT: Pubkey = pubkey!("So11111111111111111111111111111111111111111");

pub const WSOL_TOKEN_ACCOUNT: Pubkey = pubkey!("So11111111111111111111111111111111111111112");
pub const WSOL_TOKEN_ACCOUNT_META: solana_sdk::instruction::AccountMeta =
    solana_sdk::instruction::AccountMeta {
        pubkey: WSOL_TOKEN_ACCOUNT,
        is_signer: false,
        is_writable: false,
    };

pub const USD1_TOKEN_ACCOUNT: Pubkey = pubkey!("USD1ttGY1N17NEEHLmELoaybftRBUSErhqYiQzvEmuB");
pub const USD1_TOKEN_ACCOUNT_META: solana_sdk::instruction::AccountMeta =
    solana_sdk::instruction::AccountMeta {
        pubkey: USD1_TOKEN_ACCOUNT,
        is_signer: false,
        is_writable: false,
    };

// USDC (mainnet) mint and meta
pub const USDC_TOKEN_ACCOUNT: Pubkey =
    pubkey!("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v");
pub const USDC_TOKEN_ACCOUNT_META: solana_sdk::instruction::AccountMeta =
    solana_sdk::instruction::AccountMeta {
        pubkey: USDC_TOKEN_ACCOUNT,
        is_signer: false,
        is_writable: false,
    };

pub const RENT: Pubkey = solana_sdk::sysvar::rent::id();
pub const RENT_META: solana_sdk::instruction::AccountMeta =
    solana_sdk::instruction::AccountMeta { pubkey: RENT, is_signer: false, is_writable: false };

pub const ASSOCIATED_TOKEN_PROGRAM_ID: Pubkey =
    pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");
