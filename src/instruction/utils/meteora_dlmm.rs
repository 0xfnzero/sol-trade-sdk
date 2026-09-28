use anyhow::{anyhow, Result};
use solana_sdk::{pubkey, pubkey::Pubkey};

use crate::common::SolanaRpcClient;
use crate::trading::core::params::HopSpot;

pub const PROGRAM_ID: Pubkey = pubkey!("LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo");
pub const MEMO_PROGRAM: Pubkey = pubkey!("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr");
pub const EVENT_AUTHORITY: Pubkey = pubkey!("D1ZN9Wj1fRSUQfCjhvnu1hqDMT7hzjzBBpi12nVniYD6");
pub const SWAP2_DISCRIMINATOR: [u8; 8] = [65, 75, 63, 76, 235, 91, 91, 136];

/// Anchor discriminator of `LbPair` accounts.
pub const LB_PAIR_DISCRIMINATOR: [u8; 8] = [33, 11, 49, 98, 181, 101, 177, 13];
const BIN_ARRAY_DISC: [u8; 8] = [92, 142, 92, 220, 5, 148, 70, 181];
const BITMAP_EXTENSION_DISC: [u8; 8] = [80, 111, 124, 113, 55, 237, 18, 5];
pub const MAX_BIN_PER_ARRAY: i32 = 70;
/// `token_x_mint` offset after 8-byte discriminator (StaticParameters 32 + VariableParameters 32 + header 16).
pub const TOKEN_X_MINT_OFFSET: usize = 8 + 32 + 32 + 16;
/// LbPair bytes `decode_lb_pair` reads: up to `activation_point`.
const LB_PAIR_READ_LEN: usize = 824;
/// The pair's own bitmap covers bin arrays -512..=511.
pub const BIN_ARRAY_BITMAP_SIZE: i64 = 512;
/// Rows of 512 bin arrays the bitmap extension covers beyond them, each side.
pub const EXTENSION_BITMAP_ROWS: usize = 12;

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
    /// `PairType`: 0 permissionless, 1 permission, 2 customizable
    /// permissionless, 3 permissionless v2.
    pub pair_type: u8,
    /// `PairStatus`: 0 enabled, 1 disabled.
    pub status: u8,
    /// `ActivationType`: 0 slot, 1 unix timestamp.
    pub activation_type: u8,
    pub activation_point: u64,
    /// `FunctionType`: 0 undetermined, 1 liquidity mining, 2 limit order.
    pub function_type: u8,
    /// `CollectFeeMode`: 0 fee off the input, 1 fee in token Y.
    pub collect_fee_mode: u8,
    pub reward_mints: [Pubkey; 2],
    /// Which bin arrays -512..=511 hold liquidity: bit `index + 512`.
    pub bin_array_bitmap: [u64; 16],
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

    /// Whether the program lets anyone swap through the pair at `unix_timestamp`
    /// and `slot`: it is enabled, and a permissioned pair has reached its
    /// activation point.
    pub fn ensure_swappable(&self, unix_timestamp: i64, slot: u64) -> Result<()> {
        if self.status != 0 {
            return Err(anyhow!("Meteora DLMM pair is disabled"));
        }
        // Permission and customizable permissionless pairs open at a point.
        if matches!(self.pair_type, 1 | 2) {
            let now = match self.activation_type {
                0 => slot,
                1 => unix_timestamp.max(0) as u64,
                other => return Err(anyhow!("Meteora DLMM activation type {other} is unknown")),
            };
            if now < self.activation_point {
                return Err(anyhow!("Meteora DLMM pair opens at {}", self.activation_point));
            }
        }
        Ok(())
    }

    /// Whether swaps fill the bins' limit orders as well as their liquidity: a
    /// pair of undetermined function type does unless it pays farming rewards.
    pub fn support_limit_order(&self) -> Result<bool> {
        match self.function_type {
            0 => Ok(self.reward_mints.iter().all(|mint| *mint == Pubkey::default())),
            1 => Ok(false),
            2 => Ok(true),
            other => Err(anyhow!("Meteora DLMM function type {other} is unknown")),
        }
    }

    /// Whether bin array `index` holds liquidity, per the pair's bitmap and,
    /// beyond its range, the extension's; without one, arrays there hold none.
    pub fn bin_array_has_liquidity(
        &self,
        extension: Option<&BinArrayBitmapExtension>,
        index: i64,
    ) -> bool {
        if (-BIN_ARRAY_BITMAP_SIZE..BIN_ARRAY_BITMAP_SIZE).contains(&index) {
            let bit = (index + BIN_ARRAY_BITMAP_SIZE) as usize;
            self.bin_array_bitmap[bit / 64] >> (bit % 64) & 1 == 1
        } else {
            extension.is_some_and(|extension| extension.has_liquidity(index))
        }
    }

    /// The first `count` bin arrays holding liquidity that a swap reaches from
    /// array `start` on, `start` included, in its order: down for `swap_for_y`,
    /// up otherwise. The program skips the arrays between them.
    pub fn liquid_bin_arrays(
        &self,
        extension: Option<&BinArrayBitmapExtension>,
        start: i64,
        swap_for_y: bool,
        count: usize,
    ) -> Vec<i64> {
        let rows = if extension.is_some() { EXTENSION_BITMAP_ROWS as i64 + 1 } else { 1 };
        let reach = BIN_ARRAY_BITMAP_SIZE * rows;
        let step = if swap_for_y { -1 } else { 1 };
        let mut arrays = Vec::with_capacity(count);
        let mut index = start;
        while arrays.len() < count && (-reach..reach).contains(&index) {
            if self.bin_array_has_liquidity(extension, index) {
                arrays.push(index);
            }
            index += step;
        }
        arrays
    }
}

/// A pair's bitmap of the bin arrays beyond -512..=511 holding liquidity:
/// `EXTENSION_BITMAP_ROWS` rows of 512 arrays each side.
#[derive(Clone, Debug)]
pub struct BinArrayBitmapExtension {
    pub lb_pair: Pubkey,
    /// Row `r`, bit `b`: array `(r + 1) * 512 + b`.
    pub positive: [[u64; 8]; EXTENSION_BITMAP_ROWS],
    /// Row `r`, bit `b`: array `-((r + 1) * 512 + b) - 1`.
    pub negative: [[u64; 8]; EXTENSION_BITMAP_ROWS],
}

impl BinArrayBitmapExtension {
    pub fn decode(data: &[u8]) -> Result<Self> {
        const ROW_BYTES: usize = 64;
        const LEN: usize = 8 + 32 + 2 * EXTENSION_BITMAP_ROWS * ROW_BYTES;
        if data.len() < LEN {
            return Err(anyhow!("Meteora DLMM bitmap extension too short"));
        }
        if data[..8] != BITMAP_EXTENSION_DISC {
            return Err(anyhow!("Meteora DLMM bitmap extension discriminator mismatch"));
        }
        let rows = |from: usize| {
            let mut rows = [[0u64; 8]; EXTENSION_BITMAP_ROWS];
            for (r, row) in rows.iter_mut().enumerate() {
                for (w, word) in row.iter_mut().enumerate() {
                    let at = from + r * ROW_BYTES + w * 8;
                    *word = u64::from_le_bytes(data[at..at + 8].try_into().unwrap());
                }
            }
            rows
        };
        Ok(Self {
            lb_pair: Pubkey::new_from_array(data[8..40].try_into().unwrap()),
            positive: rows(40),
            negative: rows(40 + EXTENSION_BITMAP_ROWS * ROW_BYTES),
        })
    }

    /// Whether bin array `index`, outside the pair's own bitmap, holds liquidity.
    pub fn has_liquidity(&self, index: i64) -> bool {
        let (rows, offset) = if index >= BIN_ARRAY_BITMAP_SIZE {
            (&self.positive, index)
        } else if index < -BIN_ARRAY_BITMAP_SIZE {
            (&self.negative, -(index + 1))
        } else {
            return false;
        };
        let bit = (offset % BIN_ARRAY_BITMAP_SIZE) as usize;
        rows.get((offset / BIN_ARRAY_BITMAP_SIZE - 1) as usize)
            .is_some_and(|row| row[bit / 64] >> (bit % 64) & 1 == 1)
    }
}

/// The index of `lb_pair`'s bin array held in `data`.
pub fn decode_bin_array_index(data: &[u8], lb_pair: &Pubkey) -> Result<i64> {
    if data.len() < 56 || data[..8] != BIN_ARRAY_DISC {
        return Err(anyhow!("not a Meteora DLMM bin array"));
    }
    if data[24..56] != lb_pair.to_bytes() {
        return Err(anyhow!("the bin array belongs to another pair"));
    }
    Ok(i64::from_le_bytes(data[8..16].try_into().unwrap()))
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
    if data.len() < LB_PAIR_READ_LEN {
        return Err(anyhow!("Meteora DLMM LbPair account too short"));
    }
    if data[..8] != LB_PAIR_DISCRIMINATOR {
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
    // IDL 0.12: function_type and collect_fee_mode follow base_fee_power_factor;
    // the reward infos (144 bytes each) start with their mints.
    let pubkey_at = |at: usize| Pubkey::new_from_array(data[at..at + 32].try_into().unwrap());
    let mut bin_array_bitmap = [0u64; 16];
    for (i, word) in bin_array_bitmap.iter_mut().enumerate() {
        let at = 584 + i * 8;
        *word = u64::from_le_bytes(data[at..at + 8].try_into().unwrap());
    }
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
        pair_type: data[75],
        status: data[82],
        activation_type: data[86],
        activation_point: u64::from_le_bytes(data[816..824].try_into().unwrap()),
        function_type: data[35],
        collect_fee_mode: data[36],
        reward_mints: [pubkey_at(264), pubkey_at(408)],
        bin_array_bitmap,
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

    fn pair() -> LbPairState {
        LbPairState {
            base_factor: 0,
            variable_fee_control: 0,
            base_fee_power_factor: 0,
            volatility_accumulator: 0,
            active_id: 0,
            bin_step: 10,
            token_x_mint: Pubkey::new_unique(),
            token_y_mint: Pubkey::new_unique(),
            reserve_x: Pubkey::new_unique(),
            reserve_y: Pubkey::new_unique(),
            oracle: Pubkey::new_unique(),
            pair_type: 0,
            status: 0,
            activation_type: 0,
            activation_point: 0,
            function_type: 0,
            collect_fee_mode: 0,
            reward_mints: [Pubkey::default(); 2],
            bin_array_bitmap: [0; 16],
        }
    }

    fn set_internal(pair: &mut LbPairState, index: i64) {
        let bit = (index + 512) as usize;
        pair.bin_array_bitmap[bit / 64] |= 1 << (bit % 64);
    }

    fn extension() -> BinArrayBitmapExtension {
        BinArrayBitmapExtension {
            lb_pair: Pubkey::new_unique(),
            positive: [[0; 8]; EXTENSION_BITMAP_ROWS],
            negative: [[0; 8]; EXTENSION_BITMAP_ROWS],
        }
    }

    #[test]
    fn the_pair_bitmap_holds_arrays_minus_512_to_511() {
        let mut pair = pair();
        for index in [-512, -1, 0, 511] {
            set_internal(&mut pair, index);
        }
        for index in [-512, -1, 0, 511] {
            assert!(pair.bin_array_has_liquidity(None, index), "{index}");
        }
        for index in [-513, -511, -2, 1, 510, 512] {
            assert!(!pair.bin_array_has_liquidity(None, index), "{index}");
        }
    }

    #[test]
    fn the_extension_holds_rows_of_512_arrays_beyond() {
        let mut ext = extension();
        // Row 0 bit 0 each side, the last bits of row 11, and row 1 bit 0.
        ext.positive[0][0] |= 1;
        ext.positive[11][7] |= 1 << 63;
        ext.negative[0][0] |= 1;
        ext.negative[11][7] |= 1 << 63;
        ext.negative[1][0] |= 1;
        for index in [512, 6655, -513, -6656, -1025] {
            assert!(ext.has_liquidity(index), "{index}");
        }
        for index in [513, 6654, 6656, -514, -1024, -1026, -6657, 0, 511, -512] {
            assert!(!ext.has_liquidity(index), "{index}");
        }
    }

    #[test]
    fn a_walk_skips_empty_arrays_and_crosses_into_the_extension() {
        let mut pair = pair();
        set_internal(&mut pair, -510);
        set_internal(&mut pair, 3);
        let mut ext = extension();
        ext.negative[0][0] |= 1 << 5; // array -518
        ext.positive[0][0] |= 1 << 2; // array 514
        assert_eq!(pair.liquid_bin_arrays(Some(&ext), 0, true, 5), vec![-510, -518]);
        assert_eq!(pair.liquid_bin_arrays(Some(&ext), 0, false, 5), vec![3, 514]);
        assert_eq!(pair.liquid_bin_arrays(Some(&ext), 3, false, 1), vec![3]);
        // Without the extension nothing lies beyond the pair's own bitmap.
        assert_eq!(pair.liquid_bin_arrays(None, 0, true, 5), vec![-510]);
        assert_eq!(pair.liquid_bin_arrays(None, 600, false, 5), Vec::<i64>::new());
    }

    #[test]
    fn a_permissioned_pair_swaps_from_its_activation_point() {
        let mut pair = pair();
        pair.activation_point = 100;
        assert!(pair.ensure_swappable(0, 0).is_ok(), "permissionless pairs ignore the point");
        pair.pair_type = 1;
        assert!(pair.ensure_swappable(0, 99).is_err());
        assert!(pair.ensure_swappable(0, 100).is_ok());
        pair.activation_type = 1;
        assert!(pair.ensure_swappable(99, 1_000).is_err());
        assert!(pair.ensure_swappable(100, 0).is_ok());
        pair.status = 1;
        assert!(pair.ensure_swappable(1_000, 1_000).is_err());
    }

    #[test]
    fn limit_orders_follow_the_function_type() {
        let mut pair = pair();
        assert!(pair.support_limit_order().unwrap());
        pair.reward_mints[1] = Pubkey::new_unique();
        assert!(!pair.support_limit_order().unwrap(), "undetermined with rewards: mining");
        pair.function_type = 2;
        assert!(pair.support_limit_order().unwrap());
        pair.function_type = 1;
        assert!(!pair.support_limit_order().unwrap());
        pair.function_type = 3;
        assert!(pair.support_limit_order().is_err());
    }
}
