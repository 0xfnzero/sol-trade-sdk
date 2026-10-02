//! Gated validation: RPC is used only by the test's snapshot/funding preparation
//! and simulation harness. Quote and instruction construction receive no RPC.
#![cfg(test)]
use super::*;
use crate::{
    common::mainnet_sim::{self, fixtures},
    instruction::{
        stonkfun_quote_route::build_quote_route,
        utils::{meteora_dlmm as dl, raydium_clmm as cl, whirlpool as wp},
    },
    swqos::TradeType,
};
use solana_account_decoder::UiAccountEncoding;
use solana_commitment_config::CommitmentConfig;
use solana_rpc_client_api::config::{
    RpcSimulateTransactionAccountsConfig, RpcSimulateTransactionConfig,
};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    signer::Signer,
};
use solana_transaction_status_client_types::UiTransactionEncoding;

async fn prepare(
    pool: Pubkey,
) -> (crate::common::SolanaRpcClient, SubscriptionAccountCache, CacheReadContext, u64) {
    let rpc = mainnet_sim::rpc_client();
    let a = rpc.get_account(&pool).await.expect("snapshot pool");
    let mut keys = vec![pool, solana_sdk::sysvar::clock::ID];
    if a.owner == wp::PROGRAM_ID {
        let p = wp::decode_whirlpool(&a.data).unwrap();
        keys.extend([p.token_mint_a, p.token_mint_b, wp::oracle(&pool)]);
        for offset in -6..=6 {
            keys.push(wp::tick_array_pda(
                &pool,
                wp::get_start_tick_index(p.tick_current_index, p.tick_spacing, offset),
            ));
        }
    } else if a.owner == cl::PROGRAM_ID {
        let p = cl::decode_pool_state(&a.data).unwrap();
        keys.extend([
            p.amm_config,
            p.token_mint_0,
            p.token_mint_1,
            cl::tick_array_bitmap_extension(&pool),
        ]);
        let start = cl::get_array_start_index(p.tick_current, p.tick_spacing);
        for offset in -6..=6 {
            keys.push(cl::tick_array_pda(&pool, start + offset * cl::tick_count(p.tick_spacing)));
        }
    } else {
        let p = dl::decode_lb_pair(&a.data).unwrap();
        keys.extend([p.token_x_mint, p.token_y_mint, dl::bitmap_extension_pda(&pool)]);
        let start = dl::bin_id_to_bin_array_index(p.active_id);
        for offset in -6..=6 {
            keys.push(dl::bin_array_pda(&pool, start + offset));
        }
    }
    let response = rpc
        .get_multiple_accounts_with_commitment(&keys, CommitmentConfig::processed())
        .await
        .expect("background snapshot batch");
    let mut cache = SubscriptionAccountCache::default();
    let mut epoch = 0;
    let mut timestamp = 0;
    for (key, account) in keys.into_iter().zip(response.value) {
        if let Some(account) = account {
            if key == solana_sdk::sysvar::clock::ID {
                epoch = u64::from_le_bytes(account.data[16..24].try_into().unwrap());
                timestamp = i64::from_le_bytes(account.data[32..40].try_into().unwrap()) as u64;
            }
            cache
                .update(
                    key,
                    CachedAccount {
                        owner: account.owner,
                        data: account.data,
                        slot: response.context.slot,
                        write_version: 0,
                    },
                )
                .unwrap();
        }
    }
    let ctx = CacheReadContext { slot: response.context.slot, epoch, maximum_slot_age: 0 };
    (rpc, cache, ctx, timestamp)
}
async fn check(pool: Pubkey, reverse: bool) {
    if !mainnet_sim::enabled() {
        return;
    }
    let (rpc, cache, ctx, time) = prepare(pool).await;
    let (input, output, amount) = if reverse {
        (fixtures::USDC_MINT, crate::constants::WSOL_TOKEN_ACCOUNT, 100_000)
    } else {
        (crate::constants::WSOL_TOKEN_ACCOUNT, fixtures::USDC_MINT, 1_000_000)
    };
    let hint = PoolTradeHint { pool, input_mint: input, output_mint: output };
    let q = cache
        .quote_exact_in(
            hint,
            CachedQuoteRequest {
                amount_in: amount,
                slippage_basis_points: 100,
                unix_timestamp: time,
                maximum_arrays: 6,
            },
            ctx,
        )
        .unwrap();
    let route = StonkFunQuoteRoute::buy(vec![q.hop]);
    let wallet = mainnet_sim::create_wallet();
    let mut params = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        input,
        output,
        amount,
        100,
        DexParamEnum::StonkFunQuoteRoute(route.clone()),
    );
    params.simulate = false;
    params.wait_tx_confirmed = false;
    params.rpc = None;
    params.create_input_mint_ata = false;
    params.close_input_mint_ata = false;
    params.close_output_mint_ata = false;
    params = params.without_rpc().unwrap();
    // Independent test funding. No buy/sell roundtrip is composed.
    let user_input =
        crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
            &wallet.pubkey(),
            &input,
            &crate::constants::TOKEN_PROGRAM,
            params.open_seed_optimize,
        );
    let mut funding = Vec::new();
    if reverse {
        crate::instruction::token_account_setup::push_create_user_token_account(
            &mut funding,
            &wallet.pubkey(),
            &input,
            &crate::constants::TOKEN_PROGRAM,
            params.open_seed_optimize,
        );
        // Test-only funding from an unrelated venue. Signature verification
        // is disabled by the simulator; this is never a submitted transaction.
        let funding_pool = if pool == fixtures::ORCA_WHIRLPOOL_SOL_USDC {
            fixtures::RAYDIUM_CLMM_SOL_USDC
        } else {
            fixtures::ORCA_WHIRLPOOL_SOL_USDC
        };
        let funding_state = rpc.get_account(&funding_pool).await.expect("test funding pool");
        let source = if funding_state.owner == wp::PROGRAM_ID {
            wp::decode_whirlpool(&funding_state.data).unwrap().token_vault_b
        } else {
            cl::decode_pool_state(&funding_state.data).unwrap().token_vault_1
        };
        let holder = rpc.get_account(&source).await.expect("test source");
        assert_eq!(&holder.data[..32], input.as_ref());
        let authority = Pubkey::new_from_array(holder.data[32..64].try_into().unwrap());
        let mut data = vec![3];
        data.extend_from_slice(&amount.to_le_bytes());
        funding.push(Instruction::new_with_bytes(
            crate::constants::TOKEN_PROGRAM,
            &data,
            vec![
                AccountMeta::new(source, false),
                AccountMeta::new(user_input, false),
                AccountMeta::new_readonly(authority, true),
            ],
        ));
    } else {
        crate::instruction::token_account_setup::push_create_or_wrap_user_token_account(
            &mut funding,
            &wallet.pubkey(),
            &input,
            &crate::constants::TOKEN_PROGRAM,
            amount,
            params.open_seed_optimize,
        );
    }
    // Hot-path boundary: cache → quote/params → instructions, no RPC dependency.
    let business = build_quote_route(&params, &route, true).await.unwrap();
    funding.extend(business);
    let funder = mainnet_sim::pick_funder(&rpc).await;
    let hash = rpc.get_latest_blockhash().await.unwrap();
    let tx = mainnet_sim::build_sim_tx(funder, &wallet, funding, hash, &[]);
    let user_output =
        crate::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed(
            &wallet.pubkey(),
            &output,
            &crate::constants::TOKEN_PROGRAM,
            params.open_seed_optimize,
        );
    let response = rpc
        .simulate_transaction_with_config(
            &tx,
            RpcSimulateTransactionConfig {
                sig_verify: false,
                replace_recent_blockhash: true,
                commitment: Some(CommitmentConfig::processed()),
                encoding: Some(UiTransactionEncoding::Base64),
                inner_instructions: true,
                accounts: Some(RpcSimulateTransactionAccountsConfig {
                    encoding: Some(UiAccountEncoding::Base64),
                    addresses: vec![user_output.to_string()],
                }),
                ..Default::default()
            },
        )
        .await
        .expect("simulation transport");
    if let Some(err) = response.value.err {
        mainnet_sim::print_tail_logs(&response.value.logs.unwrap_or_default(), 30);
        panic!("Offline quote simulation failed: {err:?}");
    }
    let accounts = response.value.accounts.unwrap();
    let account = accounts[0].as_ref().unwrap();
    let bytes = account.data.decode().unwrap();
    let actual = u64::from_le_bytes(bytes[64..72].try_into().unwrap());
    assert!(actual >= q.minimum_net_amount_out);
    assert!(
        actual.abs_diff(q.estimated_net_amount_out) <= q.estimated_net_amount_out / 100 + 2,
        "Quote mismatch: estimate={} actual={actual}",
        q.estimated_net_amount_out
    );
    println!(
        "cached quote pool={pool} reverse={reverse} predicted={} actual={actual} cu={:?}",
        q.estimated_net_amount_out, response.value.units_consumed
    );
}
#[tokio::test]
async fn offline_whirlpool_mainnet_forward() {
    check(fixtures::ORCA_WHIRLPOOL_SOL_USDC, false).await;
}
#[tokio::test]
async fn offline_clmm_mainnet_forward() {
    check(fixtures::RAYDIUM_CLMM_SOL_USDC, false).await;
}
#[tokio::test]
async fn offline_dlmm_mainnet_forward() {
    check(fixtures::METEORA_DLMM_SOL_USDC, false).await;
}
#[tokio::test]
async fn offline_whirlpool_mainnet_reverse() {
    check(fixtures::ORCA_WHIRLPOOL_SOL_USDC, true).await;
}
#[tokio::test]
async fn offline_clmm_mainnet_reverse() {
    check(fixtures::RAYDIUM_CLMM_SOL_USDC, true).await;
}
#[tokio::test]
async fn offline_dlmm_mainnet_reverse() {
    check(fixtures::METEORA_DLMM_SOL_USDC, true).await;
}
