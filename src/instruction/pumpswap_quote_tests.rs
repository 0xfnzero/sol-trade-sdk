use super::{tests::swap_params, *};
use crate::{
    constants::{TOKEN_PROGRAM, TOKEN_PROGRAM_2022, USDC_TOKEN_ACCOUNT, WSOL_TOKEN_ACCOUNT},
    swqos::TradeType,
    trading::core::params::{
        DexParamEnum, PumpSwapQuoteHop, PumpSwapQuoteRoute, PumpSwapQuoteVenue, TokenTransferFee,
    },
};

fn fixture() -> serde_json::Value {
    serde_json::from_str(include_str!("../../tests/fixtures/pumpswap_ttwo.json")).unwrap()
}

fn ttwo_pool() -> PumpSwapParams {
    let f = fixture();
    let key = |i: usize| f["accounts"][i].as_str().unwrap().parse::<Pubkey>().unwrap();
    PumpSwapParams::new(
        key(0),
        key(3),
        key(4),
        key(7),
        key(8),
        1_000_000_000_000,
        100_000_000,
        0, // Synthetic pricing, real account identities.
        key(17),
        key(18),
        key(11),
        key(12),
        key(9),
        f["coin_creator"].as_str().unwrap().parse().unwrap(),
        false,
        0,
    )
    .with_fee_recipients(key(9), key(24))
}

fn refresh_creator_accounts(pool: &mut PumpSwapParams) {
    pool.coin_creator_vault_authority =
        crate::instruction::utils::pumpswap::coin_creator_vault_authority(pool.coin_creator);
    pool.coin_creator_vault_ata = crate::instruction::utils::pumpswap::coin_creator_vault_ata(
        pool.coin_creator,
        pool.quote_mint,
        pool.quote_token_program,
    );
}

async fn build(params: &SwapParams, buy: bool) -> Result<Vec<Instruction>> {
    if buy {
        PumpSwapInstructionBuilder.build_buy_instructions(params).await
    } else {
        PumpSwapInstructionBuilder.build_sell_instructions(params).await
    }
}

#[tokio::test]
async fn pumpswap_ttwo_accounts_match_the_successful_mainnet_leg() {
    let pool = ttwo_pool();
    let mut params = swap_params(TradeType::Buy, None);
    params.input_mint = pool.quote_mint;
    params.output_mint = pool.base_mint;
    params.protocol_params = DexParamEnum::PumpSwap(pool);
    let instructions = build(&params, true).await.unwrap();
    let ix = instructions.last().unwrap();
    let f = fixture();
    // Wallet accounts and volume accumulator depend on our fresh test payer.
    for i in [0, 2, 3, 4, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 21, 22, 23, 24, 25] {
        assert_eq!(
            ix.accounts[i].pubkey.to_string(),
            f["accounts"][i].as_str().unwrap(),
            "account {i}"
        );
    }
    assert_eq!(
        &ix.data[..8],
        crate::instruction::utils::pumpswap::BUY_EXACT_QUOTE_IN_DISCRIMINATOR
    );
}

#[tokio::test]
async fn pumpswap_direction_follows_requested_pair_for_both_api_labels() {
    for stable_pair in [false, true] {
        let mut pool = ttwo_pool();
        if stable_pair {
            pool.base_mint = USDC_TOKEN_ACCOUNT;
            pool.quote_mint = WSOL_TOKEN_ACCOUNT;
            pool.base_token_program = TOKEN_PROGRAM;
            pool.quote_token_program = TOKEN_PROGRAM;
            refresh_creator_accounts(&mut pool);
        }
        for buy in [false, true] {
            for quote_in in [false, true] {
                let mut params = swap_params(TradeType::Buy, None);
                (params.input_mint, params.output_mint) = if quote_in {
                    (pool.quote_mint, pool.base_mint)
                } else {
                    (pool.base_mint, pool.quote_mint)
                };
                params.protocol_params = DexParamEnum::PumpSwap(pool.clone());
                let ixs = build(&params, buy).await.unwrap();
                let ix = ixs.last().unwrap();
                let expected = if quote_in {
                    crate::instruction::utils::pumpswap::BUY_EXACT_QUOTE_IN_DISCRIMINATOR
                } else {
                    crate::instruction::utils::pumpswap::SELL_DISCRIMINATOR
                };
                assert_eq!(&ix.data[..8], expected);
                assert_eq!(
                    u64::from_le_bytes(ix.data[8..16].try_into().unwrap()),
                    params.input_amount.unwrap()
                );
                params.fixed_output_amount = Some(10);
                assert_eq!(build(&params, buy).await.is_ok(), quote_in);
            }
        }
    }
}

#[tokio::test]
async fn pumpswap_incomplete_or_wrong_creator_accounts_fail_before_building() {
    for buy in [true, false] {
        for field in 0..6 {
            let mut pool = ttwo_pool();
            match field {
                0 => pool.coin_creator_vault_ata = Pubkey::default(),
                1 => pool.base_mint = Pubkey::default(),
                2 => pool.pool_quote_token_account = Pubkey::default(),
                3 => pool.quote_token_program = Pubkey::default(),
                4 => pool.coin_creator_vault_authority = Pubkey::new_unique(),
                _ => {
                    pool.coin_creator_vault_ata =
                        crate::instruction::utils::pumpswap::coin_creator_vault_ata(
                            pool.coin_creator,
                            pool.quote_mint,
                            TOKEN_PROGRAM,
                        )
                }
            }
            let mut params = swap_params(TradeType::Buy, None);
            params.input_mint = pool.quote_mint;
            params.output_mint = pool.base_mint;
            params.protocol_params = DexParamEnum::PumpSwap(pool);
            assert!(build(&params, buy).await.is_err(), "invalid field {field}");
        }
    }
    let pool = ttwo_pool();
    let mut params = swap_params(TradeType::Buy, None);
    params.input_mint = pool.quote_mint;
    params.output_mint = pool.base_mint;
    params.input_token_program = Some(TOKEN_PROGRAM);
    params.protocol_params = DexParamEnum::PumpSwap(pool);
    assert!(build(&params, true).await.unwrap_err().to_string().contains("request token programs"));
}

fn hop(pool: PumpSwapParams, input: Pubkey, output: Pubkey, minimum: u64) -> PumpSwapQuoteHop {
    PumpSwapQuoteHop::exact_in(
        PumpSwapQuoteVenue::PumpSwap {
            pool,
            base_transfer_fee: TokenTransferFee::default(),
            quote_transfer_fee: TokenTransferFee::default(),
        },
        input,
        output,
        minimum,
    )
}

#[tokio::test]
async fn pumpswap_quote_route_spends_only_guaranteed_bridge_credit() {
    let pool = ttwo_pool();
    let mut funding = super::tests::pumpswap_params();
    funding.base_mint = pool.quote_mint;
    funding.base_token_program = TOKEN_PROGRAM_2022;
    let route = PumpSwapQuoteRoute::buy(vec![
        hop(funding.clone(), WSOL_TOKEN_ACCOUNT, pool.quote_mint, 20_000),
        hop(pool.clone(), pool.quote_mint, pool.base_mint, 100_000),
    ])
    .with_sell_hops(vec![
        hop(pool.clone(), pool.base_mint, pool.quote_mint, 1_000),
        hop(funding, pool.quote_mint, WSOL_TOKEN_ACCOUNT, 1_000),
    ]);
    let mut params = swap_params(TradeType::Buy, None);
    params.input_mint = crate::constants::SOL_TOKEN_ACCOUNT;
    params.output_mint = pool.base_mint;
    params.protocol_params = DexParamEnum::PumpSwapQuoteRoute(route.clone());
    params.open_seed_optimize = false;
    params.create_input_mint_ata = true;
    params.create_output_mint_ata = true;
    let ixs = build(&params, true).await.unwrap();
    let swaps: Vec<_> = ixs.iter().filter(|ix| ix.program_id == accounts::AMM_PROGRAM).collect();
    assert_eq!(swaps.len(), 2);
    assert_eq!(u64::from_le_bytes(swaps[1].data[8..16].try_into().unwrap()), 20_000);
    // Neither the bridge nor the final token account is closed.
    assert!(!ixs.iter().any(|ix| ix.program_id == TOKEN_PROGRAM_2022 && ix.data == [9]));
    params.fixed_output_amount = Some(1);
    assert!(build(&params, true).await.unwrap_err().to_string().contains("exact-input sizing"));
    params.fixed_output_amount = None;
    params.input_mint = pool.base_mint;
    params.output_mint = WSOL_TOKEN_ACCOUNT;
    params.input_amount = Some(100_000_000);
    let ixs = build(&params, false).await.unwrap();
    let swaps: Vec<_> = ixs.iter().filter(|ix| ix.program_id == accounts::AMM_PROGRAM).collect();
    assert_eq!(swaps.len(), 2);
    assert_eq!(u64::from_le_bytes(swaps[1].data[8..16].try_into().unwrap()), 1_000);
    assert!(route.preview(WSOL_TOKEN_ACCOUNT, pool.base_mint, 1, 100, true).is_err());
}

#[test]
fn pumpswap_quote_route_rejects_unhandled_transfer_fees() {
    let pool = ttwo_pool();
    let mut step = hop(pool.clone(), pool.quote_mint, pool.base_mint, 10);
    if let PumpSwapQuoteVenue::PumpSwap { quote_transfer_fee, .. } = &mut step.venue {
        quote_transfer_fee.basis_points = 100;
    }
    let error = PumpSwapQuoteRoute::buy(vec![step])
        .preview(pool.quote_mint, pool.base_mint, 100_000, 100, true)
        .err()
        .unwrap();
    assert!(error.to_string().contains("zero transfer fees"));
}

#[tokio::test]
async fn pumpswap_external_quote_conversion_precedes_the_pool_buy() {
    use crate::trading::core::params::WhirlpoolParams;
    let pool = ttwo_pool();
    let conversion = WhirlpoolParams::new(
        Pubkey::new_unique(),
        USDC_TOKEN_ACCOUNT,
        pool.quote_mint,
        Pubkey::new_unique(),
        Pubkey::new_unique(),
        TOKEN_PROGRAM,
        TOKEN_PROGRAM_2022,
        vec![Pubkey::new_unique(), Pubkey::new_unique(), Pubkey::new_unique()],
    );
    let route = PumpSwapQuoteRoute::buy(vec![
        PumpSwapQuoteHop::exact_in(
            PumpSwapQuoteVenue::OrcaWhirlpool(conversion),
            USDC_TOKEN_ACCOUNT,
            pool.quote_mint,
            20_000,
        )
        .with_output_transfer_fee(TokenTransferFee::default()),
        hop(pool.clone(), pool.quote_mint, pool.base_mint, 100_000),
    ]);
    let mut params = swap_params(TradeType::Buy, None);
    params.input_mint = USDC_TOKEN_ACCOUNT;
    params.output_mint = pool.base_mint;
    params.protocol_params = DexParamEnum::PumpSwapQuoteRoute(route);
    params.open_seed_optimize = false;
    let ixs = build(&params, true).await.unwrap();
    assert_eq!(ixs.len(), 2);
    assert_eq!(ixs[0].program_id, crate::instruction::utils::whirlpool::PROGRAM_ID);
    assert_eq!(ixs[1].program_id, accounts::AMM_PROGRAM);
    assert_eq!(u64::from_le_bytes(ixs[0].data[16..24].try_into().unwrap()), 20_000);
    assert_eq!(u64::from_le_bytes(ixs[1].data[8..16].try_into().unwrap()), 20_000);
}

/// Virtual SOL and TTWO funding only; no private source key or submission.
#[tokio::test]
#[ignore = "requires current mainnet state and simulateTransaction"]
async fn pumpswap_ttwo_mainnet_simulates_sdk_buy_and_sell() {
    use crate::common::mainnet_sim;
    use solana_sdk::{instruction::AccountMeta, pubkey};
    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let pool_address = pubkey!("G2UuRbmVDzDFP4qyoBJrSKBXSi7d6V4bPP9tHPs36y4b");
    let pool = mainnet_sim::rpc_retry("load TTWO pool", || async {
        PumpSwapParams::from_pool_address_by_rpc(&rpc, &pool_address).await
    })
    .await
    .unwrap();
    pool.validate_accounts().unwrap();
    assert_eq!(pool.quote_mint, pubkey!("TTWofwAge91oFhZs7kpQdyrVRkmevgM88xijGvQFbKo"));
    assert_eq!(pool.quote_token_program, TOKEN_PROGRAM_2022);
    let source = pubkey!("7UES8BEyUwjMMcsN7pavDTjiQJNVcPP9qAUTjJH1cwZJ");
    let authority = pubkey!("JCRGumoE9Qi5BBgULTgdgTLjSgkCMSbF62ZZfGs84JeU");
    let source_account = rpc.get_account(&source).await.unwrap();
    assert_eq!(source_account.owner, TOKEN_PROGRAM_2022);
    assert_eq!(&source_account.data[0..32], pool.quote_mint.as_ref());
    assert_eq!(&source_account.data[32..64], authority.as_ref());
    let amount = 10_000u64;
    assert!(u64::from_le_bytes(source_account.data[64..72].try_into().unwrap()) >= amount);
    let quote_ata = crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
        &wallet.pubkey(),
        &pool.quote_mint,
        &pool.quote_token_program,
    );
    let mut ixs = Vec::new();
    super::super::token_account_setup::push_create_user_token_account(
        &mut ixs,
        &wallet.pubkey(),
        &pool.quote_mint,
        &pool.quote_token_program,
        false,
    );
    let mut transfer_data = vec![12]; // SPL Token-2022 TransferChecked
    transfer_data.extend_from_slice(&amount.to_le_bytes());
    transfer_data.push(6);
    ixs.push(Instruction {
        program_id: TOKEN_PROGRAM_2022,
        accounts: vec![
            AccountMeta::new(source, false),
            AccountMeta::new_readonly(pool.quote_mint, false),
            AccountMeta::new(quote_ata, false),
            AccountMeta::new_readonly(authority, true),
        ],
        data: transfer_data,
    });
    let mut buy = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        pool.quote_mint,
        pool.base_mint,
        amount,
        500,
        DexParamEnum::PumpSwap(pool.clone()),
    );
    buy.create_input_mint_ata = false;
    let buy_ixs = build(&buy, true).await.unwrap();
    let sell_amount = mainnet_sim::pumpswap_buy_min_base_out(&buy_ixs).unwrap() / 2;
    assert!(sell_amount > 0);
    ixs.extend(buy_ixs);
    let mut sell = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Sell,
        pool.base_mint,
        pool.quote_mint,
        sell_amount,
        500,
        DexParamEnum::PumpSwap(pool),
    );
    sell.create_input_mint_ata = false;
    sell.create_output_mint_ata = false;
    ixs.extend(build(&sell, false).await.unwrap());
    mainnet_sim::run_business_sim(&rpc, &wallet, ixs, &[], "TTWO SDK buy+sell").await;
}

/// Both directions are prepared from a single RPC batch. The sell simulation
/// starts from virtually funded Dacity, independently of the buy simulation.
async fn simulate_cached_ttwo_route(is_buy: bool, search_routes: bool, repeat_buy: bool) {
    use crate::common::mainnet_sim;
    use crate::instruction::utils::{
        meteora_dlmm as dl, pumpswap as ps, pumpswap_types, raydium_amm_v4_types,
    };
    use crate::trading::core::params::{
        CacheReadContext, CachedAccount, CachedQuoteRequest, CachedRouteStep, PoolTradeHint,
        PumpSwapRouteSearch, SubscriptionAccountCache,
    };
    use solana_commitment_config::CommitmentConfig;
    use solana_sdk::{instruction::AccountMeta, pubkey};
    let rpc = mainnet_sim::rpc_client();
    let wallet = mainnet_sim::create_wallet();
    let pump_key = pubkey!("G2UuRbmVDzDFP4qyoBJrSKBXSi7d6V4bPP9tHPs36y4b");
    let market = pubkey!("F1mbBZYKXGFx5nraY3cK4RgNR5EuShntQ8xDKDD5HY1z");
    let amm_key = mainnet_sim::fixtures::AMM_V4_WSOL_USDC;
    // Discovery is separate from quoting; every discovered dependency is fetched
    // again with the pools in the coherent batch below.
    let discovery = rpc.get_multiple_accounts(&[pump_key, market, amm_key]).await.unwrap();
    let pool = pumpswap_types::pool_decode(&discovery[0].as_ref().unwrap().data[8..]).unwrap();
    let state = dl::decode_lb_pair(&discovery[1].as_ref().unwrap().data).unwrap();
    let amm = raydium_amm_v4_types::amm_info_decode(&discovery[2].as_ref().unwrap().data).unwrap();
    let source = pubkey!("D97LWtC2oMu6rjWi283bAGx7xrQPZP92gyFEcfCEsHVh");
    let mut keys = vec![
        pump_key,
        pool.base_mint,
        pool.quote_mint,
        pool.pool_base_token_account,
        pool.pool_quote_token_account,
        ps::accounts::GLOBAL_ACCOUNT,
        ps::accounts::FEE_CONFIG,
        market,
        solana_sdk::sysvar::clock::ID,
        state.token_x_mint,
        state.token_y_mint,
        dl::bitmap_extension_pda(&market),
        amm_key,
        amm.coin_mint,
        amm.pc_mint,
        amm.token_coin,
        amm.token_pc,
        source,
    ];
    let start = dl::bin_id_to_bin_array_index(state.active_id);
    for offset in -6..=6 {
        keys.push(dl::bin_array_pda(&market, start + offset));
    }
    keys.sort_unstable();
    keys.dedup();
    let batch = rpc
        .get_multiple_accounts_with_commitment(&keys, CommitmentConfig::processed())
        .await
        .unwrap();
    let mut cache = SubscriptionAccountCache::default();
    let mut epoch = None;
    let mut timestamp = None;
    let mut funding_account = None;
    for (key, account) in keys.into_iter().zip(batch.value) {
        if let Some(account) = account {
            if key == solana_sdk::sysvar::clock::ID {
                epoch = Some(u64::from_le_bytes(account.data[16..24].try_into().unwrap()));
                timestamp =
                    Some(i64::from_le_bytes(account.data[32..40].try_into().unwrap()) as u64);
            }
            if key == source {
                funding_account = Some(account.clone());
            }
            cache
                .update(
                    key,
                    CachedAccount {
                        owner: account.owner,
                        data: account.data,
                        slot: batch.context.slot,
                        write_version: 0,
                    },
                )
                .unwrap();
        }
    }
    let sol = crate::constants::SOL_TOKEN_ACCOUNT;
    let mut steps = vec![
        CachedRouteStep {
            pool: PoolTradeHint { pool: amm_key, input_mint: sol, output_mint: USDC_TOKEN_ACCOUNT },
            input_amount: None,
        },
        CachedRouteStep {
            pool: PoolTradeHint {
                pool: market,
                input_mint: USDC_TOKEN_ACCOUNT,
                output_mint: pool.quote_mint,
            },
            input_amount: None,
        },
        CachedRouteStep {
            pool: PoolTradeHint {
                pool: pump_key,
                input_mint: pool.quote_mint,
                output_mint: pool.base_mint,
            },
            input_amount: None,
        },
    ];
    // Choose sell order explicitly and re-quote; never reuse buy hop minima.
    if !is_buy {
        steps.reverse();
        for step in &mut steps {
            std::mem::swap(&mut step.pool.input_mint, &mut step.pool.output_mint);
        }
    }
    let amount = if is_buy { 1_000_000 } else { 1_000_000_000 };
    let (input, output) = if is_buy { (sol, pool.base_mint) } else { (pool.base_mint, sol) };
    let request = CachedQuoteRequest {
        amount_in: amount,
        slippage_basis_points: 500,
        unix_timestamp: timestamp.unwrap(),
        maximum_arrays: 6,
    };
    let context =
        CacheReadContext { slot: batch.context.slot, epoch: epoch.unwrap(), maximum_slot_age: 0 };
    let prepared = if search_routes {
        let search = PumpSwapRouteSearch::new(
            pump_key,
            pool.base_mint,
            sol,
            vec![
                PoolTradeHint {
                    pool: market,
                    input_mint: pool.quote_mint,
                    output_mint: USDC_TOKEN_ACCOUNT,
                },
                PoolTradeHint { pool: amm_key, input_mint: sol, output_mint: USDC_TOKEN_ACCOUNT },
            ],
        );
        let selected =
            cache.prepare_best_pumpswap_trade(&search, request, context, is_buy).unwrap();
        assert_eq!(selected.routes_evaluated, 1);
        assert!(selected.rejected_routes.is_empty());
        assert_eq!(
            selected.trade.steps().iter().map(|s| s.pool.pool).collect::<Vec<_>>(),
            steps.iter().map(|s| s.pool.pool).collect::<Vec<_>>()
        );
        selected.trade
    } else {
        cache.prepare_pumpswap_trade(&steps, input, output, request, context, is_buy).unwrap()
    };
    println!("Automatic cached route selection={search_routes}");
    println!(
        "Cached TTWO route buy={is_buy} snapshot={} preview={:?}",
        batch.context.slot,
        prepared.preview()
    );
    let mut params = mainnet_sim::swap_params(
        wallet.clone(),
        TradeType::Buy,
        input,
        output,
        amount,
        500,
        DexParamEnum::PumpSwapQuoteRoute(Default::default()),
    );
    params.simulate = false;
    params.open_seed_optimize = true;
    params.close_output_mint_ata = !is_buy;
    let params = prepared.apply_to(params).unwrap();
    let mut ixs = Vec::new();
    if !is_buy {
        // This source is a public holder from the captured transaction. No source
        // private key is available; the debit exists only with sigVerify=false.
        let funding = funding_account.expect("Dacity simulation funding account missing");
        assert_eq!(funding.owner, TOKEN_PROGRAM_2022);
        assert_eq!(&funding.data[..32], pool.base_mint.as_ref());
        assert!(u64::from_le_bytes(funding.data[64..72].try_into().unwrap()) >= amount);
        let authority = Pubkey::new_from_array(funding.data[32..64].try_into().unwrap());
        let destination = crate::common::fast_fn::get_associated_token_address_with_program_id_fast(
            &wallet.pubkey(),
            &pool.base_mint,
            &TOKEN_PROGRAM_2022,
        );
        super::super::token_account_setup::push_create_user_token_account(
            &mut ixs,
            &wallet.pubkey(),
            &pool.base_mint,
            &TOKEN_PROGRAM_2022,
            false,
        );
        let mut data = vec![12];
        data.extend_from_slice(&amount.to_le_bytes());
        data.push(6);
        ixs.push(Instruction {
            program_id: TOKEN_PROGRAM_2022,
            accounts: vec![
                AccountMeta::new(source, false),
                AccountMeta::new_readonly(pool.base_mint, false),
                AccountMeta::new(destination, false),
                AccountMeta::new_readonly(authority, true),
            ],
            data,
        });
    }
    ixs.extend(build(&params, is_buy).await.unwrap());
    if !is_buy {
        assert!(ixs.iter().any(|ix| ix.program_id == TOKEN_PROGRAM && ix.data == [9]));
    }
    let funder = mainnet_sim::pick_funder(&rpc).await;
    let blockhash = rpc.get_latest_blockhash().await.unwrap();
    let tx = if repeat_buy {
        assert!(is_buy);
        // Retain both copies of setup. The shared simulation helper deduplicates
        // ATA creation, which would conceal whether the second setup succeeds.
        let repeated = build(&params, true).await.unwrap();
        ixs.extend(repeated);
        let creates = ixs
            .iter()
            .filter(|ix| ix.program_id == crate::constants::ASSOCIATED_TOKEN_PROGRAM_ID)
            .count();
        assert_eq!(creates, 8);
        ixs.insert(
            0,
            solana_system_interface::instruction::transfer(
                &funder,
                &wallet.pubkey(),
                mainnet_sim::SIM_FUND_LAMPORTS,
            ),
        );
        let config = solana_message::v1::TransactionConfig::empty()
            .with_compute_unit_limit(700_000)
            .with_loaded_accounts_data_size_limit(64 * 1024 * 1024);
        let message = solana_message::VersionedMessage::V1(
            solana_message::v1::Message::try_compile_with_config(&funder, &ixs, blockhash, config)
                .unwrap(),
        );
        let mut signatures = vec![
            solana_sdk::signature::Signature::default();
            message.header().num_required_signatures as usize
        ];
        let index =
            message.static_account_keys().iter().position(|key| *key == wallet.pubkey()).unwrap();
        signatures[index] = wallet.sign_message(&message.serialize());
        assert!(signatures[index].verify(wallet.pubkey().as_ref(), &message.serialize()));
        println!("Repeated buy seed=true: two full routes, {creates} ATA creates, no setup deduplication");
        solana_sdk::transaction::VersionedTransaction { signatures, message }
    } else {
        mainnet_sim::build_sim_tx(funder, &wallet, ixs, blockhash, &[])
    };
    let result = mainnet_sim::simulate(&rpc, &tx).await;
    mainnet_sim::assert_sim_ok(
        if is_buy { "Cached SOL -> Dacity" } else { "Cached Dacity -> SOL" },
        &result,
    );
}

#[tokio::test]
#[ignore = "requires current mainnet state and simulateTransaction"]
async fn pumpswap_ttwo_mainnet_simulates_sol_conversion_route() {
    simulate_cached_ttwo_route(true, false, false).await;
}

#[tokio::test]
#[ignore = "requires current mainnet state and simulateTransaction"]
async fn pumpswap_ttwo_mainnet_simulates_cached_sell_to_sol() {
    simulate_cached_ttwo_route(false, false, false).await;
}

#[tokio::test]
#[ignore = "requires current mainnet state and simulateTransaction"]
async fn pumpswap_ttwo_mainnet_simulates_searched_buy() {
    simulate_cached_ttwo_route(true, true, false).await;
}

#[tokio::test]
#[ignore = "requires current mainnet state and simulateTransaction"]
async fn pumpswap_ttwo_mainnet_simulates_searched_sell() {
    simulate_cached_ttwo_route(false, true, false).await;
}

#[tokio::test]
#[ignore = "requires current mainnet state and simulateTransaction"]
async fn pumpswap_ttwo_mainnet_simulates_repeated_seed_default_buy() {
    simulate_cached_ttwo_route(true, true, true).await;
}
