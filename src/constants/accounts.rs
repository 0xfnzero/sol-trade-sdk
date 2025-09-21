use solana_sdk::{pubkey, pubkey::Pubkey};

pub const SYSTEM_PROGRAM: Pubkey = solana_sdk::system_program::ID;
pub const SYSTEM_PROGRAM_META: solana_sdk::instruction::AccountMeta =
    solana_sdk::instruction::AccountMeta {
        pubkey: SYSTEM_PROGRAM,
        is_signer: false,
        is_writable: false,
    };

pub const TOKEN_PROGRAM: Pubkey = spl_token::ID;
pub const TOKEN_PROGRAM_META: solana_sdk::instruction::AccountMeta =
    solana_sdk::instruction::AccountMeta {
        pubkey: TOKEN_PROGRAM,
        is_signer: false,
        is_writable: false,
    };

pub const TOKEN_PROGRAM_2022: Pubkey = spl_token_2022::ID;
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

pub const RENT: Pubkey = solana_sdk::sysvar::rent::id();
pub const RENT_META: solana_sdk::instruction::AccountMeta =
    solana_sdk::instruction::AccountMeta { pubkey: RENT, is_signer: false, is_writable: false };
