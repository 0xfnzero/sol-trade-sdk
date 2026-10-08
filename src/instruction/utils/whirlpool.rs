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

/// Derive consecutive sparse-swap arrays, preserving uninitialized PDAs.
/// B→A starts at current tick + spacing, as in Orca's SwapUtils.
fn swap_tick_array_keys(
    whirlpool: &Pubkey,
    tick_current: i32,
    tick_spacing: u16,
    a_to_b: bool,
) -> Result<Vec<Pubkey>> {
    const MIN_TICK: i32 = -443_636;
    const MAX_TICK: i32 = 443_636;
    if tick_spacing == 0 || !(MIN_TICK..=MAX_TICK).contains(&tick_current) {
        return Err(anyhow!("invalid Whirlpool tick or spacing"));
    }
    let shifted = tick_current + if a_to_b { 0 } else { i32::from(tick_spacing) };
    let min_start = get_start_tick_index(MIN_TICK, tick_spacing, 0);
    let mut out = Vec::with_capacity(3);
    for i in 0..3 {
        let start = get_start_tick_index(shifted, tick_spacing, if a_to_b { -i } else { i });
        if start < min_start || start > MAX_TICK {
            break;
        }
        out.push(tick_array_pda(whirlpool, start));
    }
    let last = *out.last().ok_or_else(|| anyhow!("no Whirlpool tick arrays in swap direction"))?;
    // swap_v2 has three fixed slots. At a global tick bound, repeat the last
    // valid PDA; the program deduplicates slots before building its sequence.
    out.resize(3, last);
    Ok(out)
}

/// Resolve the three fixed swap_v2 array slots. Missing accounts must remain in
/// the sequence: the current program treats their canonical PDAs as empty arrays.
/// The RPC argument is retained for API compatibility; derivation needs no I/O.
pub async fn resolve_tick_arrays_for_swap(
    _rpc: &SolanaRpcClient,
    whirlpool: &Pubkey,
    tick_current: i32,
    tick_spacing: u16,
    a_to_b: bool,
) -> Result<Vec<Pubkey>> {
    swap_tick_array_keys(whirlpool, tick_current, tick_spacing, a_to_b)
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
        let pool = Pubkey::new_unique();
        let keys = |tick, spacing, down| swap_tick_array_keys(&pool, tick, spacing, down).unwrap();
        let pdas = |starts: [i32; 3]| starts.map(|start| tick_array_pda(&pool, start)).to_vec();
        // Official SwapUtils shifts B→A by spacing, including negative ticks.
        assert_eq!(keys(5631, 64, false), pdas([5632, 11264, 16896]));
        assert_eq!(keys(-1, 64, false), pdas([0, 5632, 11264]));
        assert_eq!(keys(5631, 64, true), pdas([0, -5632, -11264]));
        // Sparse PDAs are retained without filtering by account existence.
        assert_eq!(keys(0, 64, false), pdas([0, 5632, 11264]));
        assert_eq!(keys(-443636, 1, true), pdas([-443696; 3]));
        assert_eq!(keys(443635, 1, false), pdas([443608; 3]));
        assert!(swap_tick_array_keys(&pool, 0, 0, true).is_err());
        assert!(swap_tick_array_keys(&pool, 443637, 1, false).is_err());
    }
}
