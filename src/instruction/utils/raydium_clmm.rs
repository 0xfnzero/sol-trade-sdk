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
    if data.len() < 8 + 265 {
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

/// Read the official default or extension bitmap bit for an array start.
fn initialized_array(pool_data: &[u8], extension: Option<&[u8]>, index: i32) -> Result<bool> {
    let (data, offset, bit) = if (-512..512).contains(&index) {
        (pool_data, 904usize, (index + 512) as usize)
    } else {
        let data = extension.ok_or_else(|| anyhow!("CLMM bitmap extension required"))?;
        let (offset, bit) = if index >= 512 {
            (40usize, index - 512)
        } else {
            let distance = -index - 513;
            (936usize, distance / 512 * 512 + 511 - distance % 512)
        };
        if !(0..7168).contains(&bit) {
            return Err(anyhow!("CLMM bitmap index outside range"));
        }
        (data, offset, bit as usize)
    };
    let byte = data.get(offset + bit / 8).ok_or_else(|| anyhow!("CLMM bitmap data truncated"))?;
    Ok(byte & (1 << (bit % 8)) != 0)
}

/// Select initialized tick-array PDAs using Raydium's bitmaps, not account existence.
/// Existing arrays with no initialized ticks must not be the first swap account.
pub async fn resolve_tick_arrays_for_swap(
    rpc: &SolanaRpcClient,
    pool: &Pubkey,
    _tick_current: i32,
    _tick_spacing: u16,
    zero_for_one: bool,
) -> Result<Vec<Pubkey>> {
    // Refresh the state and bitmap together; caller hints may already be stale.
    let account = rpc.get_account(pool).await?;
    if account.owner != PROGRAM_ID {
        return Err(anyhow!("CLMM pool owner mismatch"));
    }
    let state = decode_pool_state(&account.data)?;
    if state.tick_spacing == 0 || !(-443636..=443636).contains(&state.tick_current) {
        return Err(anyhow!("CLMM tick spacing or current tick is invalid"));
    }
    if account.data.len() < 1032 {
        return Err(anyhow!("CLMM default bitmap truncated"));
    }
    let step = tick_count(state.tick_spacing);
    let mut start = get_array_start_index(state.tick_current, state.tick_spacing);
    let mut extension = None;
    let mut starts = Vec::with_capacity(3);
    while start <= 443636 && start + step > -443636 {
        let index = start / step;
        if !(-512..512).contains(&index) && extension.is_none() {
            let value = rpc.get_account(&tick_array_bitmap_extension(pool)).await?;
            if value.owner != PROGRAM_ID
                || value.data.len() < 1832
                || value.data[..8] != [60, 150, 36, 219, 97, 128, 139, 153]
                || value.data[8..40] != pool.to_bytes()
            {
                return Err(anyhow!("CLMM bitmap extension owner/layout/pool mismatch"));
            }
            extension = Some(value.data);
        }
        if initialized_array(&account.data, extension.as_deref(), index)? {
            starts.push(start);
            if starts.len() == 3 {
                break;
            }
        }
        start += if zero_for_one { -step } else { step };
    }
    if starts.is_empty() {
        return Err(anyhow!("no initialized CLMM tick arrays in swap direction"));
    }
    let pdas: Vec<_> = starts.iter().map(|start| tick_array_pda(pool, *start)).collect();
    let accounts = rpc.get_multiple_accounts(&pdas).await?;
    if accounts.len() != pdas.len() {
        return Err(anyhow!("CLMM tick array response length mismatch"));
    }
    for (start, value) in starts.iter().zip(accounts) {
        let value = value.ok_or_else(|| anyhow!("bitmap-selected CLMM tick array missing"))?;
        if value.owner != PROGRAM_ID
            || value.data.len() < 10240
            || value.data[..8] != [192, 155, 85, 205, 49, 249, 129, 42]
            || value.data[8..40] != pool.to_bytes()
            || i32::from_le_bytes(value.data[40..44].try_into().unwrap()) != *start
        {
            return Err(anyhow!("CLMM tick array owner/layout/pool/start mismatch"));
        }
    }
    Ok(pdas)
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
        let mut data = vec![0; 1032];
        // An allocated current array may be empty. The next initialized one can
        // be more than five arrays away (the old existence scan stopped early).
        data[904 + (512 + 11) / 8] |= 1 << ((512 + 11) % 8);
        assert!(!initialized_array(&data, None, 0).unwrap());
        assert!(initialized_array(&data, None, 11).unwrap());
        assert!(initialized_array(&data[..1000], None, 511).is_err());
        let mut extension = vec![0; 1832];
        for (index, offset, bit) in
            [(512, 40, 0), (1023, 40, 511), (-513, 936, 511), (-1024, 936, 0), (-1025, 936, 1023)]
        {
            extension[offset + bit / 8] |= 1 << (bit % 8);
            assert!(initialized_array(&data, Some(&extension), index).unwrap());
            assert!(initialized_array(&data, None, index).is_err());
        }
    }
}
