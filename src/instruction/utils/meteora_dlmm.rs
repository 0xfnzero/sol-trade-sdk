use anyhow::{anyhow, Result};
use solana_sdk::{pubkey, pubkey::Pubkey};

use crate::common::SolanaRpcClient;
use crate::trading::core::params::HopSpot;

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
    pub base_factor: u16,
    pub variable_fee_control: u32,
    pub base_fee_power_factor: u8,
    pub volatility_accumulator: u32,
    pub active_id: i32,
    pub bin_step: u16,
    pub token_x_mint: Pubkey,
    pub token_y_mint: Pubkey,
    pub reserve_x: Pubkey,
    pub reserve_y: Pubkey,
    pub oracle: Pubkey,
}

/// Fee precision of `LbPairState::total_fee_rate`.
pub const FEE_PRECISION: u128 = 1_000_000_000;
/// The program caps the total fee at 10%.
pub const MAX_FEE_RATE: u128 = 100_000_000;

impl LbPairState {
    /// Base plus variable fee, in `FEE_PRECISION` units, as the program
    /// charges it at the pair's last volatility.
    pub fn total_fee_rate(&self) -> u128 {
        let bin_step = u128::from(self.bin_step);
        let base = u128::from(self.base_factor)
            .saturating_mul(bin_step)
            .saturating_mul(10)
            .saturating_mul(10u128.saturating_pow(u32::from(self.base_fee_power_factor)));
        let variable = if self.variable_fee_control > 0 {
            let volatility = u128::from(self.volatility_accumulator).saturating_mul(bin_step);
            u128::from(self.variable_fee_control)
                .saturating_mul(volatility.saturating_mul(volatility))
                .saturating_add(99_999_999_999)
                / 100_000_000_000
        } else {
            0
        };
        base.saturating_add(variable).min(MAX_FEE_RATE)
    }

    /// Spot price (Y per X) of the active bin and the total fee.
    pub fn spot(&self) -> HopSpot {
        HopSpot::from_bin(
            self.active_id,
            self.bin_step,
            self.total_fee_rate() as f64 / FEE_PRECISION as f64,
        )
    }
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
    if data.len() < TOKEN_X_MINT_OFFSET + 32 * 4 + 32 {
        return Err(anyhow!("Meteora DLMM LbPair account too short"));
    }
    if data[..8] != LB_PAIR_DISC {
        return Err(anyhow!("Meteora DLMM LbPair discriminator mismatch"));
    }
    // StaticParameters (8..40) and VariableParameters (40..72) lead the account.
    let base_factor = u16::from_le_bytes(data[8..10].try_into().unwrap());
    let variable_fee_control = u32::from_le_bytes(data[16..20].try_into().unwrap());
    let base_fee_power_factor = data[34];
    let volatility_accumulator = u32::from_le_bytes(data[40..44].try_into().unwrap());
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
        base_factor,
        variable_fee_control,
        base_fee_power_factor,
        volatility_accumulator,
        active_id,
        bin_step,
        token_x_mint,
        token_y_mint,
        reserve_x,
        reserve_y,
        oracle,
    })
}

/// Bin-array PDAs from the active one on, in the swap's direction (selling X
/// for Y moves the active bin down, to lower indices).
pub fn bin_array_candidates(lb_pair: &Pubkey, active_id: i32, swap_for_y: bool) -> Vec<Pubkey> {
    let base = bin_id_to_bin_array_index(active_id);
    (0..5)
        .map(|i| if swap_for_y { base - i } else { base + i })
        .map(|index| bin_array_pda(lb_pair, index))
        .collect()
}

/// Up to three of `candidates` that exist, in order.
pub fn initialized_bin_arrays(
    candidates: Vec<Pubkey>,
    accounts: &[Option<solana_sdk::account::Account>],
) -> Result<Vec<Pubkey>> {
    let out: Vec<Pubkey> = candidates
        .into_iter()
        .zip(accounts)
        .filter(|(_, account)| account.is_some())
        .map(|(pda, _)| pda)
        .take(3)
        .collect();
    if out.is_empty() {
        return Err(anyhow!("no initialized Meteora DLMM bin arrays near active_id"));
    }
    Ok(out)
}

/// Resolve bin arrays around `active_id` for a swap that crosses bins.
pub async fn resolve_bin_arrays_for_swap(
    rpc: &SolanaRpcClient,
    lb_pair: &Pubkey,
    active_id: i32,
    swap_for_y: bool,
) -> Result<Vec<Pubkey>> {
    let candidates = bin_array_candidates(lb_pair, active_id, swap_for_y);
    let accounts = rpc.get_multiple_accounts(&candidates).await?;
    initialized_bin_arrays(candidates, &accounts)
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
    }
}
