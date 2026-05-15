use crate::{
    common::SolanaRpcClient,
    instruction::utils::meteora_damm_v2_types::{pool_decode, Pool},
};
use anyhow::anyhow;
use solana_sdk::pubkey::Pubkey;

/// Constants used as seeds for deriving PDAs (Program Derived Addresses)
pub mod seeds {
    pub const EVENT_AUTHORITY_SEED: &[u8] = b"__event_authority";
}

/// Constants related to program accounts and authorities
pub mod accounts {
    use solana_sdk::{pubkey, pubkey::Pubkey};

    pub const AUTHORITY: Pubkey = pubkey!("HLnpSz9h2S4hiLQ43rnSD9XkcUThA7B8hQMKmDaiTLcC");
    pub const METEORA_DAMM_V2: Pubkey = pubkey!("cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG");
    pub const SYSVAR_INSTRUCTIONS: Pubkey = pubkey!("Sysvar1nstructions1111111111111111111111111");

    // META

    pub const METEORA_DAMM_V2_META: solana_sdk::instruction::AccountMeta =
        solana_sdk::instruction::AccountMeta {
            pubkey: METEORA_DAMM_V2,
            is_signer: false,
            is_writable: false,
        };

    pub const AUTHORITY_META: solana_sdk::instruction::AccountMeta =
        solana_sdk::instruction::AccountMeta {
            pubkey: AUTHORITY,
            is_signer: false,
            is_writable: false,
        };

    pub const SYSVAR_INSTRUCTIONS_META: solana_sdk::instruction::AccountMeta =
        solana_sdk::instruction::AccountMeta {
            pubkey: SYSVAR_INSTRUCTIONS,
            is_signer: false,
            is_writable: false,
        };
}

pub const SWAP_DISCRIMINATOR: &[u8] = &[248, 198, 158, 145, 225, 117, 135, 200];
pub const SWAP2_DISCRIMINATOR: &[u8] = &[65, 75, 63, 76, 235, 91, 91, 136];
pub const SWAP_MODE_EXACT_IN: u8 = 0;
pub const SWAP_MODE_PARTIAL_FILL: u8 = 1;
pub const SWAP_MODE_EXACT_OUT: u8 = 2;

pub async fn fetch_pool(
    rpc: &SolanaRpcClient,
    pool_address: &Pubkey,
) -> Result<Pool, anyhow::Error> {
    let account = rpc.get_account(pool_address).await?;
    if account.owner != accounts::METEORA_DAMM_V2 {
        return Err(anyhow!("Account is not owned by Meteora Damm V2 program"));
    }
    let pool = pool_decode(&account.data[8..]).ok_or_else(|| anyhow!("Failed to decode pool"))?;
    Ok(pool)
}

#[inline]
pub fn get_event_authority_pda() -> Pubkey {
    Pubkey::find_program_address(&[seeds::EVENT_AUTHORITY_SEED], &accounts::METEORA_DAMM_V2).0
}
