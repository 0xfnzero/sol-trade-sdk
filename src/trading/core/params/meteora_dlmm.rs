use anyhow::{anyhow, Result};
use meteora_dlmm::{array_index_of, PoolState as DlmmPool, QuoteError};
use solana_sdk::pubkey::Pubkey;

use super::{token_transfer_fee_for_epoch, HopSpot, TokenTransferFee, CLOCK_SYSVAR};
use crate::common::SolanaRpcClient;
use crate::instruction::utils::meteora_dlmm::{
    bin_array_pda, bin_id_to_bin_array_index, bitmap_extension_pda, decode_bin_array_index,
    decode_lb_pair, BinArrayBitmapExtension, LbPairState, PROGRAM_ID,
};

/// Bin arrays holding liquidity a quote reads ahead of the active bin, each
/// way, by default.
pub const QUOTE_BIN_ARRAYS: usize = 3;

/// Meteora DLMM `swap2` parameters (exact-in).
#[derive(Clone, Debug)]
pub struct MeteoraDlmmParams {
    pub lb_pair: Pubkey,
    pub bitmap_extension: Option<Pubkey>,
    pub reserve_x: Pubkey,
    pub reserve_y: Pubkey,
    pub token_x_mint: Pubkey,
    pub token_y_mint: Pubkey,
    pub oracle: Pubkey,
    pub token_x_program: Pubkey,
    pub token_y_program: Pubkey,
    pub bin_arrays: Vec<Pubkey>,
    /// Spot price and fee when loaded; quotes a swap through the pool.
    pub spot: Option<HopSpot>,
    /// The pair's accounts, decoded, when loaded for exact quotes.
    pub quote_state: Option<Box<DlmmQuoteState>>,
}

/// What an exact quote through the pair reads: its accounts, decoded, and the
/// cluster time and slot they were read at.
#[derive(Clone, Debug)]
pub struct DlmmQuoteState {
    pub pair: LbPairState,
    pub bitmap_extension: Option<BinArrayBitmapExtension>,
    /// The pair as the swap math sees it, with the bins of every array read.
    pub pool: DlmmPool,
    pub token_x_transfer_fee: TokenTransferFee,
    pub token_y_transfer_fee: TokenTransferFee,
    /// Cluster unix time of the read; the variable fee decays with it.
    pub unix_timestamp: i64,
    /// Slot of the read; a permissioned pair may open at one.
    pub slot: u64,
}

/// An exact quote of a swap through the pair.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DlmmHopQuote {
    /// What the receiver gets, after any transfer fee of the output mint.
    pub amount_out: u64,
    /// The bin arrays the swap walks through, in order, then the next one
    /// holding liquidity when it was read, for a price that moves before the
    /// swap lands: the accounts `swap2` needs.
    pub bin_arrays: Vec<Pubkey>,
    /// Bins the swap takes liquidity from; its compute use grows with them.
    pub bins: u32,
}

/// Raw accounts a quote reads; `quote_account_keys` lists their addresses.
pub struct DlmmQuoteAccounts<'a> {
    pub lb_pair: &'a [u8],
    pub bitmap_extension: Option<&'a [u8]>,
    /// The pair's bin arrays, any order.
    pub bin_arrays: Vec<&'a [u8]>,
    /// Owner (token program) and data of each mint.
    pub token_x_mint: (Pubkey, &'a [u8]),
    pub token_y_mint: (Pubkey, &'a [u8]),
    pub clock: &'a [u8],
}

impl DlmmQuoteState {
    /// The bin arrays holding liquidity a swap from array `start` walks
    /// through, in its order, as far as their accounts were read.
    fn arrays_read(&self, start: i64, swap_for_y: bool) -> Vec<i64> {
        let read = &self.pool.loaded_arrays;
        self.pair
            .liquid_bin_arrays(self.bitmap_extension.as_ref(), start, swap_for_y, read.len() + 1)
            .into_iter()
            .take_while(|index| read.contains(index))
            .collect()
    }

    /// The swap math's view of a walk from array `start` through `arrays`:
    /// their bins, and every array between, which holds none, taken as read.
    fn pool_through(&self, start: i64, arrays: &[i64]) -> DlmmPool {
        let last = arrays.last().copied().unwrap_or(start);
        let mut pool = self.pool.clone();
        pool.bins.retain(|bin_id, _| arrays.contains(&array_index_of(*bin_id)));
        pool.loaded_arrays = (start.min(last)..=start.max(last)).collect();
        pool.exhaustive = false;
        pool
    }
}

impl MeteoraDlmmParams {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        lb_pair: Pubkey,
        reserve_x: Pubkey,
        reserve_y: Pubkey,
        token_x_mint: Pubkey,
        token_y_mint: Pubkey,
        oracle: Pubkey,
        token_x_program: Pubkey,
        token_y_program: Pubkey,
        bin_arrays: Vec<Pubkey>,
    ) -> Self {
        Self {
            lb_pair,
            bitmap_extension: None,
            reserve_x,
            reserve_y,
            token_x_mint,
            token_y_mint,
            oracle,
            token_x_program,
            token_y_program,
            bin_arrays,
            spot: None,
            quote_state: None,
        }
    }

    pub fn with_spot(mut self, spot: HopSpot) -> Self {
        self.spot = Some(spot);
        self
    }

    pub fn with_bitmap_extension(mut self, ext: Pubkey) -> Self {
        self.bitmap_extension = Some(ext);
        self
    }

    /// Exact quote of `amount_in` of `input_mint` through the pair, as `swap2`
    /// fills it: all of it, or an error.
    pub fn quote_exact_in(&self, input_mint: &Pubkey, amount_in: u64) -> Result<DlmmHopQuote> {
        let state = self.quote_state.as_ref().ok_or_else(|| {
            anyhow!("DLMM pair {} was not loaded with the state to quote it", self.lb_pair)
        })?;
        let swap_for_y = if *input_mint == self.token_x_mint {
            true
        } else if *input_mint == self.token_y_mint {
            false
        } else {
            return Err(anyhow!("{input_mint} is not a mint of DLMM pair {}", self.lb_pair));
        };
        let pair = &state.pair;
        let limit_orders = pair
            .ensure_swappable(state.unix_timestamp, state.slot)
            .and_then(|()| pair.support_limit_order())
            .map_err(|err| anyhow!("DLMM pair {}: {err}", self.lb_pair))?;
        let (input_fee, output_fee) = if swap_for_y {
            (state.token_x_transfer_fee, state.token_y_transfer_fee)
        } else {
            (state.token_y_transfer_fee, state.token_x_transfer_fee)
        };
        let pool_amount_in = u128::from(amount_in - input_fee.calculate(amount_in));
        let lacks_liquidity =
            || anyhow!("DLMM pair {} lacks the liquidity to swap {amount_in}", self.lb_pair);
        let start = bin_id_to_bin_array_index(pair.active_id);
        let arrays = state.arrays_read(start, swap_for_y);
        // The fewest arrays that fill the swap are the ones it walks through.
        for walked in 1..=arrays.len() {
            let pool = state.pool_through(start, &arrays[..walked]);
            match meteora_dlmm::quote(
                &pool,
                pool_amount_in,
                swap_for_y,
                state.unix_timestamp,
                limit_orders,
                true,
                None,
                None,
            ) {
                Ok(quote) if quote.remaining_in == 0 => {
                    let amount_out = u64::try_from(quote.amount_out)
                        .map_err(|_| anyhow!("DLMM pair {} pays out too much", self.lb_pair))?;
                    let amount_out = amount_out - output_fee.calculate(amount_out);
                    // The program refuses a swap that pays nothing (InsufficientOutAmount).
                    if amount_out == 0 {
                        return Err(anyhow!(
                            "DLMM pair {} pays nothing for {amount_in}",
                            self.lb_pair
                        ));
                    }
                    let ahead = (walked + 1).min(arrays.len());
                    return Ok(DlmmHopQuote {
                        amount_out,
                        bin_arrays: arrays[..ahead]
                            .iter()
                            .map(|index| bin_array_pda(&self.lb_pair, *index))
                            .collect(),
                        bins: quote.bins_crossed,
                    });
                }
                // The walk left the bin id range with input left over.
                Ok(_) => return Err(lacks_liquidity()),
                Err(QuoteError::InsufficientBinArrays { .. }) => {}
            }
        }
        // The arrays read do not fill it.
        let step = if swap_for_y { -1 } else { 1 };
        let after = arrays.last().map_or(start, |last| last + step);
        match pair.liquid_bin_arrays(state.bitmap_extension.as_ref(), after, swap_for_y, 1).first()
        {
            Some(next) => Err(anyhow!(
                "DLMM pair {} quote needs bin array {} (index {next})",
                self.lb_pair,
                bin_array_pda(&self.lb_pair, *next)
            )),
            None => Err(lacks_liquidity()),
        }
    }

    /// Addresses of the accounts a quote of `lb_pair` reads, in the order
    /// `DlmmQuoteAccounts` takes them: the pair, its bitmap extension, token X
    /// and token Y mints, the clock, then the bin arrays holding liquidity
    /// ahead of the active bin each way, per `state` (the pair as last seen).
    pub fn quote_account_keys(
        lb_pair: &Pubkey,
        state: &LbPairState,
        bitmap_extension: Option<&BinArrayBitmapExtension>,
        bin_arrays_each_way: usize,
    ) -> Vec<Pubkey> {
        let mut keys = vec![
            *lb_pair,
            bitmap_extension_pda(lb_pair),
            state.token_x_mint,
            state.token_y_mint,
            CLOCK_SYSVAR,
        ];
        let start = bin_id_to_bin_array_index(state.active_id);
        for swap_for_y in [true, false] {
            for index in
                state.liquid_bin_arrays(bitmap_extension, start, swap_for_y, bin_arrays_each_way)
            {
                let key = bin_array_pda(lb_pair, index);
                if !keys.contains(&key) {
                    keys.push(key);
                }
            }
        }
        keys
    }

    /// Params and quote state for `lb_pair` from accounts read together. A swap
    /// takes the bin arrays its quote names; until then `bin_arrays` lists
    /// those read, by index.
    pub fn from_quote_accounts(lb_pair: Pubkey, accounts: &DlmmQuoteAccounts) -> Result<Self> {
        let pair = decode_lb_pair(accounts.lb_pair)
            .map_err(|err| anyhow!("{lb_pair} is not a Meteora DLMM pair: {err}"))?;
        let bitmap_extension = accounts
            .bitmap_extension
            .map(BinArrayBitmapExtension::decode)
            .transpose()
            .map_err(|err| anyhow!("DLMM bitmap extension of {lb_pair}: {err}"))?;
        if bitmap_extension.as_ref().is_some_and(|extension| extension.lb_pair != lb_pair) {
            return Err(anyhow!(
                "The bitmap extension given for {lb_pair} belongs to another pair"
            ));
        }
        let mut indexes = Vec::with_capacity(accounts.bin_arrays.len());
        let mut arrays = Vec::with_capacity(accounts.bin_arrays.len());
        for data in &accounts.bin_arrays {
            indexes.push(
                decode_bin_array_index(data, &lb_pair)
                    .map_err(|err| anyhow!("DLMM bin array of {lb_pair}: {err}"))?,
            );
            arrays.push(data.to_vec());
        }
        indexes.sort_unstable();
        let decimals = |mint: &[u8]| mint.get(44).copied().unwrap_or(0);
        let (token_x_program, token_x_data) = accounts.token_x_mint;
        let (token_y_program, token_y_data) = accounts.token_y_mint;
        let pool = DlmmPool::from_accounts(
            accounts.lb_pair,
            &arrays,
            decimals(token_x_data),
            decimals(token_y_data),
            Some(&lb_pair.to_bytes()),
            false,
        )
        .map_err(|err| anyhow!("DLMM pair {lb_pair}: {err}"))?;
        if accounts.clock.len() < 40 {
            return Err(anyhow!("Clock sysvar data is too short"));
        }
        let slot = u64::from_le_bytes(accounts.clock[0..8].try_into().unwrap());
        let epoch = u64::from_le_bytes(accounts.clock[16..24].try_into().unwrap());
        let unix_timestamp = i64::from_le_bytes(accounts.clock[32..40].try_into().unwrap());
        let token_x_transfer_fee =
            token_transfer_fee_for_epoch(token_x_data, token_x_program, epoch)?;
        let token_y_transfer_fee =
            token_transfer_fee_for_epoch(token_y_data, token_y_program, epoch)?;
        Ok(Self {
            lb_pair,
            bitmap_extension: bitmap_extension.as_ref().map(|_| bitmap_extension_pda(&lb_pair)),
            reserve_x: pair.reserve_x,
            reserve_y: pair.reserve_y,
            token_x_mint: pair.token_x_mint,
            token_y_mint: pair.token_y_mint,
            oracle: pair.oracle,
            token_x_program,
            token_y_program,
            bin_arrays: indexes.iter().map(|index| bin_array_pda(&lb_pair, *index)).collect(),
            spot: Some(pair.spot()),
            quote_state: Some(Box::new(DlmmQuoteState {
                pair,
                bitmap_extension,
                pool,
                token_x_transfer_fee,
                token_y_transfer_fee,
                unix_timestamp,
                slot,
            })),
        })
    }

    /// Pair accounts, the state for exact quotes and the bin arrays of an
    /// `input_mint → output_mint` swap ahead of the price, in two RPC round
    /// trips: the pair, then what a quote reads per `quote_account_keys`.
    pub async fn from_pool_address_by_rpc(
        rpc: &SolanaRpcClient,
        lb_pair: &Pubkey,
        input_mint: &Pubkey,
        output_mint: &Pubkey,
    ) -> Result<Self> {
        let bitmap = bitmap_extension_pda(lb_pair);
        let first = rpc.get_multiple_accounts(&[*lb_pair, bitmap]).await?;
        let pair_account = first[0]
            .as_ref()
            .filter(|account| account.owner == PROGRAM_ID)
            .ok_or_else(|| anyhow!("{lb_pair} is not a Meteora DLMM pair"))?;
        let state = decode_lb_pair(&pair_account.data)?;
        let swap_for_y = if input_mint == &state.token_x_mint && output_mint == &state.token_y_mint
        {
            true
        } else if input_mint == &state.token_y_mint && output_mint == &state.token_x_mint {
            false
        } else {
            anyhow::bail!("DLMM swap mints do not match pool");
        };
        let extension = first[1]
            .as_ref()
            .map(|account| BinArrayBitmapExtension::decode(&account.data))
            .transpose()
            .map_err(|err| anyhow!("DLMM bitmap extension of {lb_pair}: {err}"))?;
        let keys = Self::quote_account_keys(lb_pair, &state, extension.as_ref(), QUOTE_BIN_ARRAYS);
        let accounts = rpc.get_multiple_accounts(&keys).await?;
        let data = |index: usize, what: &str| {
            accounts[index]
                .as_ref()
                .map(|account| account.data.as_slice())
                .ok_or_else(|| anyhow!("DLMM {what} of {lb_pair} missing"))
        };
        let mint = |index: usize, what: &str| {
            accounts[index]
                .as_ref()
                .map(|account| (account.owner, account.data.as_slice()))
                .ok_or_else(|| anyhow!("DLMM {what} of {lb_pair} missing"))
        };
        let mut params = Self::from_quote_accounts(
            *lb_pair,
            &DlmmQuoteAccounts {
                lb_pair: data(0, "pair")?,
                bitmap_extension: accounts[1].as_ref().map(|account| account.data.as_slice()),
                token_x_mint: mint(2, "token X mint")?,
                token_y_mint: mint(3, "token Y mint")?,
                clock: data(4, "clock")?,
                bin_arrays: accounts[5..]
                    .iter()
                    .filter_map(|account| account.as_ref().map(|a| a.data.as_slice()))
                    .collect(),
            },
        )?;
        let quote_state = params.quote_state.as_ref().expect("loaded with its quote state");
        let start = bin_id_to_bin_array_index(quote_state.pair.active_id);
        params.bin_arrays = quote_state
            .arrays_read(start, swap_for_y)
            .iter()
            .map(|index| bin_array_pda(lb_pair, *index))
            .collect();
        if params.bin_arrays.is_empty() {
            anyhow::bail!("DLMM pair {lb_pair} holds no liquidity for the swap");
        }
        Ok(params)
    }
}

/// A pair captured in `tests/fixtures/meteora_dlmm`, the swap's amount in and
/// the output the program credited.
#[cfg(test)]
pub(crate) fn fixture_pair(name: &str) -> (MeteoraDlmmParams, u64, u64) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/meteora_dlmm")
        .join(name);
    let (case, params) = fixture_tests::load(&path);
    (params, case["amount_in"].as_u64().unwrap(), case["amount_out"].as_u64().unwrap())
}

/// Quotes of real mainnet swaps against the output `swap2` credited in
/// `simulateTransaction`, captured by `examples/verify_hop_quotes` with
/// `DUMP_DIR`: fees off the input and in token Y, limit orders filled,
/// Token-2022 mints, swaps across three bin arrays, and pairs beyond their own
/// bitmap.
#[cfg(test)]
mod fixture_tests {
    use super::*;
    use base64::Engine;
    use std::str::FromStr;

    fn bytes(value: &serde_json::Value) -> Vec<u8> {
        base64::engine::general_purpose::STANDARD.decode(value.as_str().unwrap()).unwrap()
    }

    fn mint(value: &serde_json::Value) -> (Pubkey, Vec<u8>) {
        (Pubkey::from_str(value["owner"].as_str().unwrap()).unwrap(), bytes(&value["data"]))
    }

    pub(super) fn load(path: &std::path::Path) -> (serde_json::Value, MeteoraDlmmParams) {
        let case: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let accounts = &case["accounts"];
        let pool = Pubkey::from_str(case["pool"].as_str().unwrap()).unwrap();
        let lb_pair = bytes(&accounts["lb_pair"]);
        let bitmap =
            accounts["bitmap_extension"].as_str().map(|_| bytes(&accounts["bitmap_extension"]));
        let (x, y) = (mint(&accounts["token_x_mint"]), mint(&accounts["token_y_mint"]));
        let arrays: Vec<Vec<u8>> =
            accounts["bin_arrays"].as_array().unwrap().iter().map(bytes).collect();
        let mut clock = vec![0u8; 40];
        clock[0..8].copy_from_slice(&case["slot"].as_u64().unwrap().to_le_bytes());
        clock[16..24].copy_from_slice(&case["epoch"].as_u64().unwrap().to_le_bytes());
        clock[32..40].copy_from_slice(&case["unix_timestamp"].as_i64().unwrap().to_le_bytes());
        let params = MeteoraDlmmParams::from_quote_accounts(
            pool,
            &DlmmQuoteAccounts {
                lb_pair: &lb_pair,
                bitmap_extension: bitmap.as_deref(),
                bin_arrays: arrays.iter().map(Vec::as_slice).collect(),
                token_x_mint: (x.0, &x.1),
                token_y_mint: (y.0, &y.1),
                clock: &clock,
            },
        )
        .unwrap();
        (case, params)
    }

    fn input(case: &serde_json::Value, params: &MeteoraDlmmParams) -> Pubkey {
        if case["swap_for_y"].as_bool().unwrap() {
            params.token_x_mint
        } else {
            params.token_y_mint
        }
    }

    #[test]
    fn quotes_match_what_the_program_credited() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/meteora_dlmm");
        let (mut checked, mut with_limit_orders) = (0, 0);
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let (case, params) = load(&path);
            let input = input(&case, &params);
            let amount_in = case["amount_in"].as_u64().unwrap();
            let quote = params.quote_exact_in(&input, amount_in).unwrap();
            assert_eq!(
                quote.amount_out,
                case["amount_out"].as_u64().unwrap(),
                "{}",
                path.display()
            );
            // The fixture holds exactly the bin arrays the quote named.
            let used: Vec<Pubkey> = case["bin_arrays_used"]
                .as_array()
                .unwrap()
                .iter()
                .map(|key| Pubkey::from_str(key.as_str().unwrap()).unwrap())
                .collect();
            assert_eq!(quote.bin_arrays, used, "{}", path.display());

            // Left out, the bins' limit orders change what some swaps pay.
            let mut mining = params.clone();
            mining.quote_state.as_mut().unwrap().pair.function_type = 1;
            let without = mining.quote_exact_in(&input, amount_in).map(|q| q.amount_out).ok();
            if without != Some(quote.amount_out) {
                with_limit_orders += 1;
            }
            checked += 1;
        }
        assert_eq!(checked, 10);
        assert!(with_limit_orders >= 3, "{with_limit_orders} swaps fill limit orders");
    }

    #[test]
    fn a_quote_needs_the_bin_arrays_its_swap_reaches() {
        let path = std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/meteora_dlmm/",
            "3C5YE97HADPDxZehYq9Cis8AXr9aNyrUsczKzE1nDbW9-x2y-116635016372.json"
        ));
        let (case, full) = load(path);
        let amount_in = case["amount_in"].as_u64().unwrap();
        // The swap walks three arrays; with only the first read it cannot be quoted.
        let mut first = full.clone();
        let state = first.quote_state.as_mut().unwrap();
        let start = bin_id_to_bin_array_index(state.pair.active_id);
        state.pool.bins.retain(|bin_id, _| array_index_of(*bin_id) == start);
        state.pool.loaded_arrays.retain(|index| *index == start);
        let err = first.quote_exact_in(&first.token_x_mint, amount_in).unwrap_err();
        assert!(err.to_string().contains("needs bin array"), "{err}");
        // A small swap stays in it.
        assert!(first.quote_exact_in(&first.token_x_mint, 1_000).is_ok());
        assert!(full.quote_exact_in(&Pubkey::new_unique(), 1_000).is_err());
    }
}
