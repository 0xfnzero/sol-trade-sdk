use anyhow::{anyhow, Result};
use solana_sdk::{pubkey, pubkey::Pubkey};

use crate::common::SolanaRpcClient;

pub const PROGRAM_ID: Pubkey = pubkey!("CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK");
pub const MEMO_PROGRAM: Pubkey = pubkey!("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr");
pub const SWAP_V2_DISCRIMINATOR: [u8; 8] = [43, 4, 237, 11, 26, 201, 30, 98];
pub const MIN_SQRT_PRICE_X64: u128 = 4_295_048_016;
pub const MAX_SQRT_PRICE_X64: u128 = 79_226_673_521_066_979_257_578_248_091;

const POOL_DISC: [u8; 8] = [247, 237, 227, 245, 215, 195, 222, 70];
pub const TICK_ARRAY_SIZE: i32 = 60;

/// Compact PoolState fields needed to build `swap_v2`.
#[derive(Clone, Debug)]
pub struct ClmmPoolState {
    pub amm_config: Pubkey,
    pub token_mint_0: Pubkey,
    pub token_mint_1: Pubkey,
    pub token_vault_0: Pubkey,
    pub token_vault_1: Pubkey,
    pub observation_key: Pubkey,
    pub tick_spacing: u16,
    pub tick_current: i32,
}

/// PDA: `["pool_tick_array_bitmap_extension", pool_state]` under CLMM program.
#[inline]
pub fn tick_array_bitmap_extension(pool_state: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[b"pool_tick_array_bitmap_extension", pool_state.as_ref()],
        &PROGRAM_ID,
    )
    .0
}

#[inline]
pub fn tick_count(tick_spacing: u16) -> i32 {
    TICK_ARRAY_SIZE * i32::from(tick_spacing)
}

/// Start index of the tick array that contains `tick_index`.
#[inline]
pub fn get_array_start_index(tick_index: i32, tick_spacing: u16) -> i32 {
    let ticks_in_array = tick_count(tick_spacing);
    let mut start = tick_index / ticks_in_array;
    if tick_index < 0 && tick_index % ticks_in_array != 0 {
        start -= 1;
    }
    start * ticks_in_array
}

#[inline]
pub fn tick_array_pda(pool: &Pubkey, start_index: i32) -> Pubkey {
    Pubkey::find_program_address(
        &[b"tick_array", pool.as_ref(), &start_index.to_be_bytes()],
        &PROGRAM_ID,
    )
    .0
}

/// Decode Raydium CLMM PoolState (8-byte Anchor discriminator + body).
pub fn decode_pool_state(data: &[u8]) -> Result<ClmmPoolState> {
    if data.len() < 8 + 235 {
        return Err(anyhow!("Raydium CLMM pool account too short"));
    }
    if data[..8] != POOL_DISC {
        return Err(anyhow!("Raydium CLMM pool discriminator mismatch"));
    }
    let body = &data[8..];
    // bump(1) + amm_config(32) + owner(32) + mint0(32) + mint1(32) + vault0(32) + vault1(32)
    // + observation(32) + dec0(1) + dec1(1) + tick_spacing(2) + liquidity(16) + sqrt(16) + tick(4)
    let amm_config = Pubkey::new_from_array(body[1..33].try_into().unwrap());
    let token_mint_0 = Pubkey::new_from_array(body[65..97].try_into().unwrap());
    let token_mint_1 = Pubkey::new_from_array(body[97..129].try_into().unwrap());
    let token_vault_0 = Pubkey::new_from_array(body[129..161].try_into().unwrap());
    let token_vault_1 = Pubkey::new_from_array(body[161..193].try_into().unwrap());
    let observation_key = Pubkey::new_from_array(body[193..225].try_into().unwrap());
    let tick_spacing = u16::from_le_bytes(body[227..229].try_into().unwrap());
    let tick_current = i32::from_le_bytes(body[261..265].try_into().unwrap());
    Ok(ClmmPoolState {
        amm_config,
        token_mint_0,
        token_mint_1,
        token_vault_0,
        token_vault_1,
        observation_key,
        tick_spacing,
        tick_current,
    })
}

/// Derive consecutive initialized tick-array PDAs for a `zero_for_one` swap.
pub async fn resolve_tick_arrays_for_swap(
    rpc: &SolanaRpcClient,
    pool: &Pubkey,
    tick_current: i32,
    tick_spacing: u16,
    zero_for_one: bool,
) -> Result<Vec<Pubkey>> {
    let start = get_array_start_index(tick_current, tick_spacing);
    let step = tick_count(tick_spacing);
    // Walk current + next arrays in swap direction (price down → lower indices).
    let candidates: Vec<i32> = if zero_for_one {
        (0..5).map(|i| start - i * step).collect()
    } else {
        (0..5).map(|i| start + i * step).collect()
    };
    let pdas: Vec<Pubkey> = candidates.iter().map(|s| tick_array_pda(pool, *s)).collect();
    let accounts = rpc.get_multiple_accounts(&pdas).await?;
    let mut out = Vec::new();
    for (pda, acc) in pdas.into_iter().zip(accounts) {
        if acc.is_some() {
            out.push(pda);
        }
        if out.len() >= 3 {
            break;
        }
    }
    if out.is_empty() {
        return Err(anyhow!("no initialized Raydium CLMM tick arrays near current tick"));
    }
    Ok(out)
}

pub async fn fetch_pool(rpc: &SolanaRpcClient, pool: &Pubkey) -> Result<ClmmPoolState> {
    let account = rpc.get_account(pool).await?;
    if account.owner != PROGRAM_ID {
        return Err(anyhow!("account is not owned by Raydium CLMM"));
    }
    decode_pool_state(&account.data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn array_start_index_matches_raydium_formula() {
        assert_eq!(get_array_start_index(100, 1), 60);
        assert_eq!(get_array_start_index(60, 1), 60);
        assert_eq!(get_array_start_index(-1, 1), -60);
        assert_eq!(get_array_start_index(-60, 1), -60);
        assert_eq!(get_array_start_index(-61, 1), -120);
    }
}
