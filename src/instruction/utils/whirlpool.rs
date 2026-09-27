use anyhow::{anyhow, Result};
use solana_sdk::{pubkey, pubkey::Pubkey};

use crate::common::SolanaRpcClient;

pub const PROGRAM_ID: Pubkey = pubkey!("whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc");
pub const MEMO_PROGRAM: Pubkey = pubkey!("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr");
pub const SWAP_V2_DISCRIMINATOR: [u8; 8] = [43, 4, 237, 11, 26, 201, 30, 98];

/// Full-range sqrt-price bounds (Q64.64) from Orca Whirlpool `tick_math`.
///
/// On-chain accepts `sqrt_price_limit == 0` as `NO_EXPLICIT_SQRT_PRICE_LIMIT`
/// and remaps to these by direction. Do **not** reuse Raydium CLMM's MAX —
/// it is larger and triggers `SqrtPriceOutOfBounds` on Whirlpool.
pub const MIN_SQRT_PRICE: u128 = 4_295_048_016;
pub const MAX_SQRT_PRICE: u128 = 79_226_673_515_401_279_992_447_579_055;

const WHIRLPOOL_DISC: [u8; 8] = [63, 149, 209, 12, 225, 128, 99, 9];
pub const TICK_ARRAY_SIZE: i32 = 88;

#[derive(Clone, Debug)]
pub struct WhirlpoolState {
    pub tick_spacing: u16,
    pub tick_current_index: i32,
    pub token_mint_a: Pubkey,
    pub token_vault_a: Pubkey,
    pub token_mint_b: Pubkey,
    pub token_vault_b: Pubkey,
}

#[inline]
pub fn default_sqrt_price_limit(a_to_b: bool) -> u128 {
    if a_to_b {
        MIN_SQRT_PRICE
    } else {
        MAX_SQRT_PRICE
    }
}

#[inline]
pub fn oracle(whirlpool: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[b"oracle", whirlpool.as_ref()], &PROGRAM_ID).0
}

/// Orca PDA uses the ASCII decimal of `start_tick`, not LE/BE bytes.
#[inline]
pub fn tick_array_pda(whirlpool: &Pubkey, start_tick: i32) -> Pubkey {
    Pubkey::find_program_address(
        &[b"tick_array", whirlpool.as_ref(), start_tick.to_string().as_bytes()],
        &PROGRAM_ID,
    )
    .0
}

#[inline]
pub fn get_start_tick_index(tick_index: i32, tick_spacing: u16, offset: i32) -> i32 {
    let ticks_in_array = i32::from(tick_spacing) * TICK_ARRAY_SIZE;
    let mut real_index = tick_index / ticks_in_array;
    if tick_index < 0 && tick_index % ticks_in_array != 0 {
        real_index -= 1;
    }
    (real_index + offset) * ticks_in_array
}

pub fn decode_whirlpool(data: &[u8]) -> Result<WhirlpoolState> {
    // disc(8) + config(32) + bump(1) + spacing(2) + seed(2) + fee(2) + pfee(2)
    // + liq(16) + sqrt(16) + tick(4) + proto_a(8) + proto_b(8)
    // + mint_a(32) + vault_a(32) + fee_a(16) + mint_b(32) + vault_b(32)
    // Verified against mainnet HJPjoW… (SOL/USDC): mint_a @ body+93, tick @ body+73.
    if data.len() < 8 + 237 {
        return Err(anyhow!("Whirlpool account too short"));
    }
    if data[..8] != WHIRLPOOL_DISC {
        return Err(anyhow!("Whirlpool discriminator mismatch"));
    }
    let body = &data[8..];
    let tick_spacing = u16::from_le_bytes(body[33..35].try_into().unwrap());
    let tick_current_index = i32::from_le_bytes(body[73..77].try_into().unwrap());
    let token_mint_a = Pubkey::new_from_array(body[93..125].try_into().unwrap());
    let token_vault_a = Pubkey::new_from_array(body[125..157].try_into().unwrap());
    let token_mint_b = Pubkey::new_from_array(body[173..205].try_into().unwrap());
    let token_vault_b = Pubkey::new_from_array(body[205..237].try_into().unwrap());
    Ok(WhirlpoolState {
        tick_spacing,
        tick_current_index,
        token_mint_a,
        token_vault_a,
        token_mint_b,
        token_vault_b,
    })
}

/// Resolve 3 tick arrays in swap direction (`a_to_b` → decreasing start indices).
pub async fn resolve_tick_arrays_for_swap(
    rpc: &SolanaRpcClient,
    whirlpool: &Pubkey,
    tick_current: i32,
    tick_spacing: u16,
    a_to_b: bool,
) -> Result<Vec<Pubkey>> {
    let offsets: [i32; 5] = if a_to_b {
        [0, -1, -2, -3, -4]
    } else {
        [0, 1, 2, 3, 4]
    };
    let pdas: Vec<Pubkey> = offsets
        .iter()
        .map(|o| tick_array_pda(whirlpool, get_start_tick_index(tick_current, tick_spacing, *o)))
        .collect();
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
    if out.len() < 3 {
        // Pad with derived PDAs so the ix still has 3 slots; on-chain may reject
        // uninitialized ones, but most liquid pools have neighbors initialized.
        while out.len() < 3 {
            let o = offsets[out.len()];
            out.push(tick_array_pda(
                whirlpool,
                get_start_tick_index(tick_current, tick_spacing, o),
            ));
        }
    }
    Ok(out)
}

pub async fn fetch_whirlpool(rpc: &SolanaRpcClient, key: &Pubkey) -> Result<WhirlpoolState> {
    let account = rpc.get_account(key).await?;
    if account.owner != PROGRAM_ID {
        return Err(anyhow!("account is not owned by Orca Whirlpool"));
    }
    decode_whirlpool(&account.data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whirlpool_max_sqrt_matches_orca_not_raydium_clmm() {
        const RAYDIUM_CLMM_MAX: u128 = 79_226_673_521_066_979_257_578_248_091;
        assert_eq!(MAX_SQRT_PRICE, 79_226_673_515_401_279_992_447_579_055);
        assert!(MAX_SQRT_PRICE < RAYDIUM_CLMM_MAX);
        assert_eq!(default_sqrt_price_limit(true), MIN_SQRT_PRICE);
        assert_eq!(default_sqrt_price_limit(false), MAX_SQRT_PRICE);
    }

    #[test]
    fn start_tick_index_negative_floor() {
        assert_eq!(get_start_tick_index(100, 64, 0), 0);
        assert_eq!(get_start_tick_index(-1, 64, 0), -5632);
        assert_eq!(get_start_tick_index(-5632, 64, 0), -5632);
    }
}
