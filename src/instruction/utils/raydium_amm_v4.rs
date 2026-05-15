use crate::{
    common::SolanaRpcClient,
    instruction::utils::raydium_amm_v4_types::{
        amm_info_decode, market_state_decode, AmmInfo, MarketState,
    },
};
use anyhow::anyhow;
use solana_sdk::pubkey::Pubkey;

/// Constants used as seeds for deriving PDAs (Program Derived Addresses)
pub mod seeds {
    pub const POOL_SEED: &[u8] = b"pool";
}

/// Constants related to program accounts and authorities
pub mod accounts {
    use solana_sdk::{pubkey, pubkey::Pubkey};
    pub const AUTHORITY: Pubkey = pubkey!("5Q544fKrFoe6tsEbD7S8EmxGTJYAKtTVhAW5Q5pge4j1");
    pub const RAYDIUM_AMM_V4: Pubkey = pubkey!("675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8");

    pub const TRADE_FEE_NUMERATOR: u64 = 25;
    pub const TRADE_FEE_DENOMINATOR: u64 = 10000;
    pub const SWAP_FEE_NUMERATOR: u64 = 25;
    pub const SWAP_FEE_DENOMINATOR: u64 = 10000;

    // META

    pub const AUTHORITY_META: solana_sdk::instruction::AccountMeta =
        solana_sdk::instruction::AccountMeta {
            pubkey: AUTHORITY,
            is_signer: false,
            is_writable: false,
        };
}

pub const SWAP_BASE_IN_DISCRIMINATOR: &[u8] = &[9];
pub const SWAP_BASE_OUT_DISCRIMINATOR: &[u8] = &[11];

pub async fn fetch_amm_info(rpc: &SolanaRpcClient, amm: Pubkey) -> Result<AmmInfo, anyhow::Error> {
    let amm_info = rpc.get_account_data(&amm).await?;
    let amm_info =
        amm_info_decode(&amm_info).ok_or_else(|| anyhow!("Failed to decode amm info"))?;
    Ok(amm_info)
}

pub async fn fetch_market_state(
    rpc: &SolanaRpcClient,
    market: Pubkey,
) -> Result<MarketState, anyhow::Error> {
    let market_data = rpc.get_account_data(&market).await?;
    market_state_decode(&market_data).ok_or_else(|| anyhow!("Failed to decode market state"))
}

pub fn derive_serum_vault_signer(
    serum_program: &Pubkey,
    serum_market: &Pubkey,
    vault_signer_nonce: u64,
) -> Result<Pubkey, anyhow::Error> {
    let nonce = vault_signer_nonce.to_le_bytes();
    Pubkey::create_program_address(&[serum_market.as_ref(), &nonce], serum_program)
        .or_else(|_| {
            let legacy_nonce = [vault_signer_nonce as u8];
            Pubkey::create_program_address(&[serum_market.as_ref(), &legacy_nonce], serum_program)
        })
        .map_err(|err| anyhow!("Failed to derive Serum vault signer: {}", err))
}
