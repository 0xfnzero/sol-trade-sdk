use crate::*;
use anyhow::{ensure, Result};
use core::result::Result::Ok;
use solana_sdk::{account::Account, clock::Clock};
use std::collections::HashMap;
use solana_streamer::streaming::event_parser::protocols::meteora_dlmm::types::LbPair;

#[derive(Debug)]
pub struct SwapExactInQuote {
    pub amount_out: u64,
    pub fee: u64,
}

#[derive(Debug)]
pub struct SwapExactOutQuote {
    pub amount_in: u64,
    pub fee: u64,
}

fn shift_active_bin_if_empty_gap(
    lb_pair: &mut LbPair,
    active_bin_array: &BinArray,
    swap_for_y: bool,
) -> Result<()> {
    let lb_pair_bin_array_index = BinArray::bin_id_to_bin_array_index(lb_pair.active_id)?;

    if i64::from(lb_pair_bin_array_index) != active_bin_array.index {
        if swap_for_y {
            let (_, upper_bin_id) =
                BinArray::get_bin_array_lower_upper_bin_id(active_bin_array.index as i32)?;
            lb_pair.active_id = upper_bin_id;
        } else {
            let (lower_bin_id, _) =
                BinArray::get_bin_array_lower_upper_bin_id(active_bin_array.index as i32)?;
            lb_pair.active_id = lower_bin_id;
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn quote_exact_out(
    lb_pair_pubkey: Pubkey,
    lb_pair: &LbPair,
    mut amount_out: u64,
    swap_for_y: bool,
    bin_arrays: HashMap<Pubkey, BinArray>,
    bitmap_extension: Option<&BinArrayBitmapExtension>,
    clock: &Clock,
    mint_x_account: &Account,
    mint_y_account: &Account,
) -> Result<SwapExactOutQuote> {
    let current_timestamp = clock.unix_timestamp as u64;
    let current_slot = clock.slot;
    let epoch = clock.epoch;

    let mut lb_pair = *lb_pair;
    lb_pair.update_references(current_timestamp as i64)?;

    let mut total_amount_in: u64 = 0;
    let mut total_fee: u64 = 0;

    let (in_mint_account, out_mint_account) = if swap_for_y {
        (mint_x_account, mint_y_account)
    } else {
        (mint_y_account, mint_x_account)
    };

    amount_out =
        calculate_transfer_fee_included_amount(out_mint_account, amount_out, epoch)?.amount;

    while amount_out > 0 {
        let active_bin_array_pubkey = get_bin_array_pubkeys_for_swap(
            lb_pair_pubkey,
            &lb_pair,
            bitmap_extension,
            swap_for_y,
            1,
        )?
        .pop()
        .context("Pool out of liquidity")?;

        let mut active_bin_array = bin_arrays
            .get(&active_bin_array_pubkey)
            .cloned()
            .context("Active bin array not found")?;

        shift_active_bin_if_empty_gap(&mut lb_pair, &active_bin_array, swap_for_y)?;

        loop {
            if !active_bin_array.is_bin_id_within_range(lb_pair.active_id)? || amount_out == 0 {
                break;
            }

            lb_pair.update_volatility_accumulator()?;

            let active_bin = active_bin_array.get_bin_mut(lb_pair.active_id)?;
            let price = active_bin.get_or_store_bin_price(lb_pair.active_id, lb_pair.bin_step)?;

            if !active_bin.is_empty(!swap_for_y) {
                let bin_max_amount_out = active_bin.get_max_amount_out(swap_for_y);
                if amount_out >= bin_max_amount_out {
                    let max_amount_in = active_bin.get_max_amount_in(price, swap_for_y)?;
                    let max_fee = lb_pair.compute_fee(max_amount_in)?;

                    total_amount_in = total_amount_in
                        .checked_add(max_amount_in)
                        .context("MathOverflow")?;

                    total_fee = total_fee.checked_add(max_fee).context("MathOverflow")?;

                    amount_out = amount_out
                        .checked_sub(bin_max_amount_out)
                        .context("MathOverflow")?;
                } else {
                    let amount_in = Bin::get_amount_in(amount_out, price, swap_for_y)?;
                    let fee = lb_pair.compute_fee(amount_in)?;

                    total_amount_in = total_amount_in
                        .checked_add(amount_in)
                        .context("MathOverflow")?;

                    total_fee = total_fee.checked_add(fee).context("MathOverflow")?;

                    amount_out = 0;
                }
            }

            if amount_out > 0 {
                lb_pair.advance_active_bin(swap_for_y)?;
            }
        }
    }

    total_amount_in = total_amount_in
        .checked_add(total_fee)
        .context("MathOverflow")?;

    total_amount_in =
        calculate_transfer_fee_included_amount(in_mint_account, total_amount_in, epoch)?.amount;

    Ok(SwapExactOutQuote {
        amount_in: total_amount_in,
        fee: total_fee,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn quote_exact_in(
    lb_pair_pubkey: Pubkey,
    lb_pair: &LbPair,
    amount_in: u64,
    swap_for_y: bool,
    bin_arrays: HashMap<Pubkey, BinArray>,
    bitmap_extension: Option<&BinArrayBitmapExtension>,
    clock: &Clock,
    mint_x_account: &Account,
    mint_y_account: &Account,
) -> Result<SwapExactInQuote> {
    let current_timestamp = clock.unix_timestamp as u64;
    let current_slot = clock.slot;
    let epoch = clock.epoch;

    validate_swap_activation(lb_pair, current_timestamp, current_slot)?;

    let mut lb_pair = *lb_pair;
    lb_pair.update_references(current_timestamp as i64)?;

    let mut total_amount_out: u64 = 0;
    let mut total_fee: u64 = 0;

    let (in_mint_account, out_mint_account) = if swap_for_y {
        (mint_x_account, mint_y_account)
    } else {
        (mint_y_account, mint_x_account)
    };

    let transfer_fee_excluded_amount_in =
        calculate_transfer_fee_excluded_amount(in_mint_account, amount_in, epoch)?.amount;

    let mut amount_left = transfer_fee_excluded_amount_in;

    while amount_left > 0 {
        let active_bin_array_pubkey = get_bin_array_pubkeys_for_swap(
            lb_pair_pubkey,
            &lb_pair,
            bitmap_extension,
            swap_for_y,
            1,
        )?
        .pop()
        .context("Pool out of liquidity")?;

        let mut active_bin_array = bin_arrays
            .get(&active_bin_array_pubkey)
            .cloned()
            .context("Active bin array not found")?;

        shift_active_bin_if_empty_gap(&mut lb_pair, &active_bin_array, swap_for_y)?;

        loop {
            if !active_bin_array.is_bin_id_within_range(lb_pair.active_id)? || amount_left == 0 {
                break;
            }

            lb_pair.update_volatility_accumulator()?;

            let active_bin = active_bin_array.get_bin_mut(lb_pair.active_id)?;
            let price = active_bin.get_or_store_bin_price(lb_pair.active_id, lb_pair.bin_step)?;

            if !active_bin.is_empty(!swap_for_y) {
                let SwapResult {
                    amount_in_with_fees,
                    amount_out,
                    fee,
                    ..
                } = active_bin.swap(amount_left, price, swap_for_y, &lb_pair, None)?;

                amount_left = amount_left
                    .checked_sub(amount_in_with_fees)
                    .context("MathOverflow")?;

                total_amount_out = total_amount_out
                    .checked_add(amount_out)
                    .context("MathOverflow")?;
                total_fee = total_fee.checked_add(fee).context("MathOverflow")?;
            }

            if amount_left > 0 {
                lb_pair.advance_active_bin(swap_for_y)?;
            }
        }
    }

    let transfer_fee_excluded_amount_out =
        calculate_transfer_fee_excluded_amount(out_mint_account, total_amount_out, epoch)?.amount;

    Ok(SwapExactInQuote {
        amount_out: transfer_fee_excluded_amount_out,
        fee: total_fee,
    })
}

pub fn get_bin_array_pubkeys_for_swap(
    lb_pair_pubkey: Pubkey,
    lb_pair: &LbPair,
    bitmap_extension: Option<&BinArrayBitmapExtension>,
    swap_for_y: bool,
    take_count: u8,
) -> Result<Vec<Pubkey>> {
    let mut start_bin_array_idx = BinArray::bin_id_to_bin_array_index(lb_pair.active_id)?;
    let mut bin_array_idx = vec![];
    let increment = if swap_for_y { -1 } else { 1 };

    loop {
        if bin_array_idx.len() == take_count as usize {
            break;
        }

        if lb_pair.is_overflow_default_bin_array_bitmap(start_bin_array_idx) {
            let Some(bitmap_extension) = bitmap_extension else {
                break;
            };
            let Ok((next_bin_array_idx, has_liquidity)) = bitmap_extension
                .next_bin_array_index_with_liquidity(swap_for_y, start_bin_array_idx)
            else {
                // Out of search range. No liquidity.
                break;
            };
            if has_liquidity {
                bin_array_idx.push(next_bin_array_idx);
                start_bin_array_idx = next_bin_array_idx + increment;
            } else {
                // Switch to internal bitmap
                start_bin_array_idx = next_bin_array_idx;
            }
        } else {
            let Ok((next_bin_array_idx, has_liquidity)) = lb_pair
                .next_bin_array_index_with_liquidity_internal(swap_for_y, start_bin_array_idx)
            else {
                break;
            };
            if has_liquidity {
                bin_array_idx.push(next_bin_array_idx);
                start_bin_array_idx = next_bin_array_idx + increment;
            } else {
                // Switch to external bitmap
                start_bin_array_idx = next_bin_array_idx;
            }
        }
    }

    let bin_array_pubkeys = bin_array_idx
        .into_iter()
        .map(|idx| derive_bin_array_pda(lb_pair_pubkey, idx.into()).0)
        .collect();

    Ok(bin_array_pubkeys)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anchor_client::solana_client::rpc_response::RpcKeyedAccount;
    use anchor_client::solana_sdk::clock::Clock;
    use anchor_client::{
        solana_client::nonblocking::rpc_client::RpcClient, solana_sdk::pubkey::Pubkey, Cluster,
    };
    use litesvm::LiteSVM;

    pub const DLMM_PROGRAM_FILE_PATH: &str = "../artifacts/lb_clmm.so";

    /// Get on chain clock
    async fn get_clock(rpc_client: RpcClient) -> Result<Clock> {
        let clock_account = rpc_client
            .get_account(&anchor_client::solana_sdk::sysvar::clock::ID)
            .await?;

        let clock_state: Clock = bincode::deserialize(clock_account.data.as_ref())?;

        Ok(clock_state)
    }

    #[tokio::test]
    async fn test_swap_quote_exact_out() {
        // RPC client. No gPA is required.
        let rpc_client = RpcClient::new(Cluster::Mainnet.url().to_string());

        let sol_usdc = Pubkey::from_str_const("HTvjzsfX3yU6BUodCjZ5vZkUrAxMDTrBs3CJaq43ashR");

        let lb_pair_account = rpc_client.get_account(&sol_usdc).await.unwrap();

        let lb_pair: LbPair = bytemuck::pod_read_unaligned(&lb_pair_account.data[8..]);

        let mut mint_accounts = rpc_client
            .get_multiple_accounts(&[lb_pair.token_x_mint, lb_pair.token_y_mint])
            .await
            .unwrap();

        let mint_x_account = mint_accounts[0].take().unwrap();
        let mint_y_account = mint_accounts[1].take().unwrap();

        // 3 bin arrays to left, and right is enough to cover most of the swap, and stay under 1.4m CU constraint.
        // Get 3 bin arrays to the left from the active bin
        let left_bin_array_pubkeys =
            get_bin_array_pubkeys_for_swap(sol_usdc, &lb_pair, None, true, 3).unwrap();

        // Get 3 bin arrays to the right the from active bin
        let right_bin_array_pubkeys =
            get_bin_array_pubkeys_for_swap(sol_usdc, &lb_pair, None, false, 3).unwrap();

        // Fetch bin arrays
        let bin_array_pubkeys = left_bin_array_pubkeys
            .into_iter()
            .chain(right_bin_array_pubkeys.into_iter())
            .collect::<Vec<Pubkey>>();

        let accounts = rpc_client
            .get_multiple_accounts(&bin_array_pubkeys)
            .await
            .unwrap();

        let bin_arrays = accounts
            .into_iter()
            .zip(bin_array_pubkeys.into_iter())
            .map(|(account, key)| {
                (
                    key,
                    bytemuck::pod_read_unaligned(&account.unwrap().data[8..]),
                )
            })
            .collect::<HashMap<_, _>>();

        let usdc_token_multiplier = 1_000_000.0;
        let sol_token_multiplier = 1_000_000_000.0;

        let out_sol_amount = 1_000_000_000;
        let clock = get_clock(rpc_client).await.unwrap();

        let quote_result = quote_exact_out(
            sol_usdc,
            &lb_pair,
            out_sol_amount,
            false,
            bin_arrays.clone(),
            None,
            &clock,
            &mint_x_account,
            &mint_y_account,
        )
        .unwrap();

        let in_amount = quote_result.amount_in + quote_result.fee;

        println!(
            "{} USDC -> exact 1 SOL",
            in_amount as f64 / usdc_token_multiplier
        );

        let quote_result = quote_exact_in(
            sol_usdc,
            &lb_pair,
            in_amount,
            false,
            bin_arrays.clone(),
            None,
            &clock,
            &mint_x_account,
            &mint_y_account,
        )
        .unwrap();

        println!(
            "{} USDC -> {} SOL",
            in_amount as f64 / usdc_token_multiplier,
            quote_result.amount_out as f64 / sol_token_multiplier
        );

        let out_usdc_amount = 200_000_000;

        let quote_result = quote_exact_out(
            sol_usdc,
            &lb_pair,
            out_usdc_amount,
            true,
            bin_arrays.clone(),
            None,
            &clock,
            &mint_x_account,
            &mint_y_account,
        )
        .unwrap();

        let in_amount = quote_result.amount_in + quote_result.fee;

        println!(
            "{} SOL -> exact 200 USDC",
            in_amount as f64 / sol_token_multiplier
        );

        let quote_result = quote_exact_in(
            sol_usdc,
            &lb_pair,
            in_amount,
            true,
            bin_arrays,
            None,
            &clock,
            &mint_x_account,
            &mint_y_account,
        )
        .unwrap();

        println!(
            "{} SOL -> {} USDC",
            in_amount as f64 / sol_token_multiplier,
            quote_result.amount_out as f64 / usdc_token_multiplier
        );
    }

    #[tokio::test]
    async fn test_swap_quote_exact_in() {
        // RPC client. No gPA is required.
        let rpc_client = RpcClient::new(Cluster::Mainnet.url().to_string());

        let sol_usdc = Pubkey::from_str_const("HTvjzsfX3yU6BUodCjZ5vZkUrAxMDTrBs3CJaq43ashR");

        let lb_pair_account = rpc_client.get_account(&sol_usdc).await.unwrap();

        let lb_pair: LbPair = bytemuck::pod_read_unaligned(&lb_pair_account.data[8..]);

        let mut mint_accounts = rpc_client
            .get_multiple_accounts(&[lb_pair.token_x_mint, lb_pair.token_y_mint])
            .await
            .unwrap();

        let mint_x_account = mint_accounts[0].take().unwrap();
        let mint_y_account = mint_accounts[1].take().unwrap();

        // 3 bin arrays to left, and right is enough to cover most of the swap, and stay under 1.4m CU constraint.
        // Get 3 bin arrays to the left from the active bin
        let left_bin_array_pubkeys =
            get_bin_array_pubkeys_for_swap(sol_usdc, &lb_pair, None, true, 3).unwrap();

        // Get 3 bin arrays to the right the from active bin
        let right_bin_array_pubkeys =
            get_bin_array_pubkeys_for_swap(sol_usdc, &lb_pair, None, false, 3).unwrap();

        // Fetch bin arrays
        let bin_array_pubkeys = left_bin_array_pubkeys
            .into_iter()
            .chain(right_bin_array_pubkeys.into_iter())
            .collect::<Vec<Pubkey>>();

        let accounts = rpc_client
            .get_multiple_accounts(&bin_array_pubkeys)
            .await
            .unwrap();

        let bin_arrays = accounts
            .into_iter()
            .zip(bin_array_pubkeys.into_iter())
            .map(|(account, key)| {
                (
                    key,
                    bytemuck::pod_read_unaligned(&account.unwrap().data[8..]),
                )
            })
            .collect::<HashMap<_, _>>();

        // 1 SOL -> USDC
        let in_sol_amount = 5_000_000_000;

        let clock = get_clock(rpc_client).await.unwrap();

        let quote_result = quote_exact_in(
            sol_usdc,
            &lb_pair,
            in_sol_amount,
            true,
            bin_arrays.clone(),
            None,
            &clock,
            &mint_x_account,
            &mint_y_account,
        )
        .unwrap();

        println!(
            "1 SOL -> {:?} USDC",
            quote_result.amount_out as f64 / 1_000_000.0
        );

        // 100 USDC -> SOL
        let in_usdc_amount = 100_000_000;

        let quote_result = quote_exact_in(
            sol_usdc,
            &lb_pair,
            in_usdc_amount,
            false,
            bin_arrays.clone(),
            None,
            &clock,
            &mint_x_account,
            &mint_y_account,
        )
        .unwrap();

        println!(
            "100 USDC -> {:?} SOL",
            quote_result.amount_out as f64 / 1_000_000_000.0
        );
    }

    #[test]
    fn test_swap_quote_infinite_loop() {
        let test_pair = Pubkey::from_str_const("FJbEo74c2W4QLBBVUfUvi8VBWXtMdJVPuFpq2f6UV1iB");
        let associated_accounts_folder_path = format!("../artifacts/{}", test_pair);

        let mut svm = LiteSVM::new().with_sysvars();
        let program_bytes = std::fs::read(DLMM_PROGRAM_FILE_PATH).unwrap();
        svm.add_program(dlmm::ID, &program_bytes);

        let accounts_dir = std::fs::read_dir(associated_accounts_folder_path).unwrap();
        for entry in accounts_dir {
            let account_data = std::fs::read_to_string(entry.unwrap().path()).unwrap();
            let rpc_account: RpcKeyedAccount =
                serde_json::from_str(&account_data).expect("Failed to deserialize account data");
            let account: anchor_client::solana_sdk::account::Account =
                rpc_account.account.decode().unwrap();
            let account_pubkey = Pubkey::from_str_const(&rpc_account.pubkey);

            svm.set_account(account_pubkey, account.clone()).unwrap();
        }

        let lb_pair_account = svm.get_account(&test_pair).unwrap();
        let lb_pair: LbPair = bytemuck::pod_read_unaligned(&lb_pair_account.data[8..]);

        let left_bin_array_pubkeys =
            get_bin_array_pubkeys_for_swap(test_pair, &lb_pair, None, true, 3).unwrap();

        let right_bin_array_pubkeys =
            get_bin_array_pubkeys_for_swap(test_pair, &lb_pair, None, false, 3).unwrap();

        let bin_array_pubkeys = [left_bin_array_pubkeys, right_bin_array_pubkeys].concat();

        let mut bin_arrays = HashMap::new();

        for bin_array_pubkey in bin_array_pubkeys {
            let bin_array_account = svm.get_account(&bin_array_pubkey).unwrap();
            let bin_array: BinArray = bytemuck::pod_read_unaligned(&bin_array_account.data[8..]);
            bin_arrays.insert(bin_array_pubkey, bin_array);
        }

        let in_base_amount = 5_000_000_000;
        let clock: Clock = svm.get_sysvar();

        let mint_x_account = svm.get_account(&lb_pair.token_x_mint).unwrap();
        let mint_y_account = svm.get_account(&lb_pair.token_y_mint).unwrap();

        // 1. Quote in ask
        let quote_result = quote_exact_in(
            test_pair,
            &lb_pair,
            in_base_amount,
            true,
            bin_arrays.clone(),
            None,
            &clock,
            &mint_x_account,
            &mint_y_account,
        );

        assert!(quote_result.is_err());
        let err = quote_result.unwrap_err();
        assert_eq!(err.to_string(), "Pool out of liquidity");

        // 2. Quote in bid
        let in_quote_amount = 5_000_000_000;
        let quote_result = quote_exact_in(
            test_pair,
            &lb_pair,
            in_quote_amount,
            false,
            bin_arrays.clone(),
            None,
            &clock,
            &mint_x_account,
            &mint_y_account,
        );
        assert!(quote_result.is_err());
        let err = quote_result.unwrap_err();
        assert_eq!(err.to_string(), "Pool out of liquidity");

        // 3. Quote out ask
        let out_quote_amount = 5_000_000_000;
        let quote_result = quote_exact_out(
            test_pair,
            &lb_pair,
            out_quote_amount,
            true,
            bin_arrays.clone(),
            None,
            &clock,
            &mint_x_account,
            &mint_y_account,
        );

        assert!(quote_result.is_err());
        let err = quote_result.unwrap_err();
        assert_eq!(err.to_string(), "Pool out of liquidity");

        // 4. Quote out bid
        let out_base_amount = 5_000_000_000;
        let quote_result = quote_exact_out(
            test_pair,
            &lb_pair,
            out_base_amount,
            false,
            bin_arrays.clone(),
            None,
            &clock,
            &mint_x_account,
            &mint_y_account,
        );

        assert!(quote_result.is_err());
        let err = quote_result.unwrap_err();
        assert_eq!(err.to_string(), "Pool out of liquidity");
    }
}
