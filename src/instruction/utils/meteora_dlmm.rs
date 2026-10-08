use anyhow::{anyhow, Result};
use solana_sdk::{account::Account, pubkey, pubkey::Pubkey};

use crate::common::SolanaRpcClient;

pub const PROGRAM_ID: Pubkey = pubkey!("LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo");
pub const MEMO_PROGRAM: Pubkey = pubkey!("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr");
pub const EVENT_AUTHORITY: Pubkey = pubkey!("D1ZN9Wj1fRSUQfCjhvnu1hqDMT7hzjzBBpi12nVniYD6");
pub const SWAP2_DISCRIMINATOR: [u8; 8] = [65, 75, 63, 76, 235, 91, 91, 136];

const LB_PAIR_DISC: [u8; 8] = [33, 11, 49, 98, 181, 101, 177, 13];
pub const MAX_BIN_PER_ARRAY: i32 = 70;
/// `token_x_mint` offset after 8-byte discriminator (StaticParameters 32 + VariableParameters 32 + header 16).
pub const TOKEN_X_MINT_OFFSET: usize = 8 + 32 + 32 + 16;

#[derive(Clone, Debug)]
pub struct LbPairState {
    pub active_id: i32,
    pub bin_step: u16,
    pub token_x_mint: Pubkey,
    pub token_y_mint: Pubkey,
    pub reserve_x: Pubkey,
    pub reserve_y: Pubkey,
    pub oracle: Pubkey,
}

#[inline]
pub fn bin_id_to_bin_array_index(bin_id: i32) -> i64 {
    i64::from(bin_id.div_euclid(MAX_BIN_PER_ARRAY))
}

#[inline]
pub fn bin_array_pda(lb_pair: &Pubkey, index: i64) -> Pubkey {
    Pubkey::find_program_address(
        &[b"bin_array", lb_pair.as_ref(), &index.to_le_bytes()],
        &PROGRAM_ID,
    )
    .0
}

#[inline]
pub fn bitmap_extension_pda(lb_pair: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[b"bitmap", lb_pair.as_ref()], &PROGRAM_ID).0
}

pub fn decode_lb_pair(data: &[u8]) -> Result<LbPairState> {
    // Layout verified against IDL + memcmp on live SOL/USDC pairs (mint at offset 88).
    if data.len() < 584 {
        return Err(anyhow!("Meteora DLMM LbPair account too short"));
    }
    if data[..8] != LB_PAIR_DISC {
        return Err(anyhow!("Meteora DLMM LbPair discriminator mismatch"));
    }
    // active_id / bin_step sit just before token_x_mint.
    let active_id = i32::from_le_bytes(data[76..80].try_into().unwrap());
    let bin_step = u16::from_le_bytes(data[80..82].try_into().unwrap());
    let token_x_mint = Pubkey::new_from_array(data[88..120].try_into().unwrap());
    let token_y_mint = Pubkey::new_from_array(data[120..152].try_into().unwrap());
    let reserve_x = Pubkey::new_from_array(data[152..184].try_into().unwrap());
    let reserve_y = Pubkey::new_from_array(data[184..216].try_into().unwrap());
    // protocol_fee(16) + padding_1(32) + reward_infos(2 * 144?); oracle follows rewards.
    // Empirically oracle is at offset 552 on current mainnet LbPair accounts (see probe).
    // Prefer scanning: after reserves comes ProtocolFee { amount_x u64, amount_y u64 } = 16,
    // _padding_1 = 32, RewardInfo[2]. Each RewardInfo is typically 144 bytes in bytemuck layout
    // → 16+32+288 = 336; 216+336 = 552.
    let oracle = Pubkey::new_from_array(data[552..584].try_into().unwrap());
    Ok(LbPairState {
        active_id,
        bin_step,
        token_x_mint,
        token_y_mint,
        reserve_x,
        reserve_y,
        oracle,
    })
}

// DLMM 1.9.14 IDL: LbPair bitmap follows oracle at 584; extension
// contains 12 positive and 12 negative 512-bit chunks after its pool key.
const BITMAP_OFFSET: usize = 584;
const EXTENSION_BYTES: usize = 1576;
const EXTENSION_DISC: [u8; 8] = [80, 111, 124, 113, 55, 237, 18, 5];
const ARRAY_DISC: [u8; 8] = [92, 142, 92, 220, 5, 148, 70, 181];

fn extension_data<'a>(account: Option<&'a Account>, pair: &Pubkey) -> Result<Option<&'a [u8]>> {
    let Some(account) = account else {
        return Ok(None);
    };
    if account.owner == Pubkey::default() && account.data.is_empty() {
        return Ok(None);
    }
    if account.owner != PROGRAM_ID
        || account.data.len() < EXTENSION_BYTES
        || account.data[..8] != EXTENSION_DISC
        || account.data[8..40] != pair.to_bytes()
    {
        return Err(anyhow!("invalid DLMM bitmap extension"));
    }
    Ok(Some(&account.data))
}

fn liquid_array_indices(
    pair: &[u8],
    extension: Option<&[u8]>,
    active_id: i32,
    down: bool,
) -> Result<Vec<i64>> {
    let bitmap = pair
        .get(BITMAP_OFFSET..BITMAP_OFFSET + 128)
        .ok_or_else(|| anyhow!("truncated DLMM liquidity bitmap"))?;
    if extension.is_some_and(|data| data.len() < EXTENSION_BYTES) {
        return Err(anyhow!("truncated DLMM bitmap extension"));
    }
    let start = bin_id_to_bin_array_index(active_id);
    if !(-512..=511).contains(&start) && extension.is_none() {
        return Err(anyhow!("DLMM active bin requires bitmap extension"));
    }
    let mut out = Vec::with_capacity(4);
    let mut index = start;
    while (-6656..=6655).contains(&index) {
        let (data, bit) = if (-512..=511).contains(&index) {
            (Some(bitmap), (index + 512) as usize)
        } else if index >= 512 {
            (extension.map(|data| &data[40..808]), (index - 512) as usize)
        } else {
            (extension.map(|data| &data[808..1576]), (-index - 513) as usize)
        };
        if data.is_some_and(|data| data[bit / 8] & (1 << (bit % 8)) != 0) {
            out.push(index);
            if out.len() == 4 {
                break;
            }
        }
        index += if down { -1 } else { 1 };
    }
    if out.is_empty() {
        return Err(anyhow!("no DLMM liquidity in swap direction"));
    }
    Ok(out)
}

fn validate_bin_array(account: &Account, pair: &Pubkey, index: i64) -> Result<()> {
    // BinArray header = 56 bytes; 70 bins of 144 bytes, including current
    // limit-order fields. Pool/index bindings precede version-independent bins.
    if account.owner != PROGRAM_ID
        || account.data.len() < 10136
        || account.data[..8] != ARRAY_DISC
        || account.data[8..16] != index.to_le_bytes()
        || account.data[24..56] != pair.to_bytes()
    {
        return Err(anyhow!("invalid DLMM bin array for index {index}"));
    }
    Ok(())
}

/// Refresh active bin and both liquidity bitmaps together, then load the first
/// four liquid arrays in swap direction, matching official getBinArrayForSwap.
pub async fn resolve_swap_snapshot(
    rpc: &SolanaRpcClient,
    lb_pair: &Pubkey,
    swap_for_y: bool,
) -> Result<(LbPairState, Option<Pubkey>, Vec<Pubkey>)> {
    let extension_key = bitmap_extension_pda(lb_pair);
    let snapshot = rpc.get_multiple_accounts(&[*lb_pair, extension_key]).await?;
    let pair =
        snapshot.first().and_then(Option::as_ref).ok_or_else(|| anyhow!("missing DLMM pair"))?;
    if pair.owner != PROGRAM_ID {
        return Err(anyhow!("invalid DLMM pair owner"));
    }
    let state = decode_lb_pair(&pair.data)?;
    let extension = extension_data(snapshot.get(1).and_then(Option::as_ref), lb_pair)?;
    let indices = liquid_array_indices(&pair.data, extension, state.active_id, swap_for_y)?;
    let keys: Vec<_> = indices.iter().map(|index| bin_array_pda(lb_pair, *index)).collect();
    let arrays = rpc.get_multiple_accounts(&keys).await?;
    for (i, index) in indices.iter().enumerate() {
        let account = arrays
            .get(i)
            .and_then(Option::as_ref)
            .ok_or_else(|| anyhow!("DLMM bitmap-selected array {index} missing"))?;
        validate_bin_array(account, lb_pair, *index)?;
    }
    Ok((state, extension.map(|_| extension_key), keys))
}

/// The supplied active_id is a hint retained for API compatibility. A coherent
/// current pair/bitmap snapshot determines selection, never account existence.
pub async fn resolve_bin_arrays_for_swap(
    rpc: &SolanaRpcClient,
    lb_pair: &Pubkey,
    _active_id: i32,
    swap_for_y: bool,
) -> Result<Vec<Pubkey>> {
    Ok(resolve_swap_snapshot(rpc, lb_pair, swap_for_y).await?.2)
}

pub async fn fetch_lb_pair(rpc: &SolanaRpcClient, key: &Pubkey) -> Result<LbPairState> {
    let account = rpc.get_account(key).await?;
    if account.owner != PROGRAM_ID {
        return Err(anyhow!("account is not owned by Meteora DLMM"));
    }
    decode_lb_pair(&account.data)
}

pub async fn maybe_bitmap_extension(rpc: &SolanaRpcClient, lb_pair: &Pubkey) -> Option<Pubkey> {
    let key = bitmap_extension_pda(lb_pair);
    match rpc.get_account(&key).await {
        Ok(_) => Some(key),
        Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bin_array_index_euclid() {
        assert_eq!(bin_id_to_bin_array_index(0), 0);
        assert_eq!(bin_id_to_bin_array_index(69), 0);
        assert_eq!(bin_id_to_bin_array_index(70), 1);
        assert_eq!(bin_id_to_bin_array_index(-1), -1);
        assert_eq!(bin_id_to_bin_array_index(-70), -1);
        assert_eq!(bin_id_to_bin_array_index(-71), -2);
        let mut pair = vec![0u8; 712];
        let mut extension = vec![0u8; EXTENSION_BYTES];
        let set = |data: &mut [u8], byte: usize, bit: usize| data[byte + bit / 8] |= 1 << (bit % 8);
        // Skip allocated but non-liquid current arrays and gaps beyond five PDAs.
        for index in [7i64, 100, 511, -10, -511, -512] {
            set(&mut pair, BITMAP_OFFSET, (index + 512) as usize);
        }
        for index in [512i64, 1023, 1024, 6655] {
            set(&mut extension, 40, (index - 512) as usize);
        }
        for index in [-513i64, -1024, -1025, -6656] {
            set(&mut extension, 808, (-index - 513) as usize);
        }
        assert_eq!(liquid_array_indices(&pair, None, 0, false).unwrap(), [7, 100, 511]);
        let oracle: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/dlmm_bitmap_selection.json"
        ))
        .unwrap();
        for case in oracle["cases"].as_array().unwrap() {
            let expected: Vec<_> =
                case["indices"].as_array().unwrap().iter().map(|v| v.as_i64().unwrap()).collect();
            assert_eq!(
                liquid_array_indices(
                    &pair,
                    if case["extension"].as_bool().unwrap() { Some(&extension) } else { None },
                    case["active_id"].as_i64().unwrap() as i32,
                    case["down"].as_bool().unwrap(),
                )
                .unwrap(),
                expected
            );
        }
        assert!(liquid_array_indices(&pair, None, 512 * 70, false).is_err());
        assert!(liquid_array_indices(&pair[..711], None, 0, false).is_err());
        assert!(liquid_array_indices(&pair, Some(&extension[..1575]), 0, false).is_err());
        assert!(liquid_array_indices(&vec![0; 712], None, 0, false).is_err());
        let pool = Pubkey::new_unique();
        let mut account = Account { owner: PROGRAM_ID, data: vec![0; 10136], ..Default::default() };
        account.data[..8].copy_from_slice(&ARRAY_DISC);
        account.data[8..16].copy_from_slice(&7i64.to_le_bytes());
        account.data[24..56].copy_from_slice(pool.as_ref());
        validate_bin_array(&account, &pool, 7).unwrap();
        assert!(validate_bin_array(&account, &pool, 8).is_err());
        assert!(validate_bin_array(&account, &Pubkey::new_unique(), 7).is_err());
        account.owner = Pubkey::default();
        assert!(validate_bin_array(&account, &pool, 7).is_err());
        account.owner = PROGRAM_ID;
        account.data[0] ^= 1;
        assert!(validate_bin_array(&account, &pool, 7).is_err());
        account.data[..8].copy_from_slice(&ARRAY_DISC);
        account.data.truncate(10135);
        assert!(validate_bin_array(&account, &pool, 7).is_err());
        let mut ext = Account { owner: PROGRAM_ID, data: extension, ..Default::default() };
        ext.data[..8].copy_from_slice(&EXTENSION_DISC);
        ext.data[8..40].copy_from_slice(pool.as_ref());
        assert!(extension_data(Some(&ext), &pool).unwrap().is_some());
        assert!(extension_data(Some(&ext), &Pubkey::new_unique()).is_err());
        ext.owner = Pubkey::default();
        assert!(extension_data(Some(&ext), &pool).is_err());
        assert!(extension_data(None, &pool).unwrap().is_none());
    }
}
