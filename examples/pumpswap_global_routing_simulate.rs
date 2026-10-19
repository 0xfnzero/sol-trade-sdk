//! Live mainnet account reads -> RPC-free production preparation -> read-only simulation.
//! SOLANA_RPC_URL=... cargo run --example pumpswap_global_routing_simulate -- sol-repeat output.json
//! Modes: sol-repeat, wsol-repeat, roundtrip. No private funding key or send call.
#[path = "support/pumpswap_routing_demo.rs"]
mod demo;
use anyhow::{ensure, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use sol_trade_sdk::{
    common::{GasFeeStrategy, SolanaRpcClient},
    constants::{
        ASSOCIATED_TOKEN_PROGRAM_ID, TOKEN_PROGRAM, USDC_TOKEN_ACCOUNT, WSOL_TOKEN_ACCOUNT,
    },
    instruction::{
        pumpswap::PumpSwapInstructionBuilder,
        utils::{meteora_dlmm as dl, pumpswap as ps, pumpswap_types, raydium_amm_v4_types},
    },
    trading::{
        core::{
            params::{
                CacheReadContext, CachedAccount, DexParamEnum, PoolTradeHint,
                SubscriptionAccountCache,
            },
            traits::InstructionBuilder,
        },
        factory::DexType,
    },
    SellAmount, SimpleSellParams, TradeTokenType,
};
use solana_commitment_config::CommitmentConfig;
use solana_rpc_client_api::config::RpcSimulateTransactionConfig;
use solana_sdk::{
    hash::Hash,
    instruction::Instruction,
    pubkey,
    pubkey::Pubkey,
    signature::{Keypair, Signature},
    signer::Signer,
    transaction::VersionedTransaction,
};
use solana_transaction_status_client_types::UiTransactionEncoding;
use std::sync::Arc;

async fn fetch_snapshot(rpc: &SolanaRpcClient) -> Result<demo::DemoSnapshot> {
    // Discovery/refresh runs before the timed hot path. In a bot, subscriptions
    // publish these accounts plus Clock as one immutable snapshot instead.
    let historical = demo::historical_snapshot()?;
    let pump_key = historical.pool;
    let market = pubkey!("F1mbBZYKXGFx5nraY3cK4RgNR5EuShntQ8xDKDD5HY1z");
    let amm_key =
        historical.candidates.iter().find(|p| p.pool != market).context("AMM market missing")?.pool;
    let discovery = rpc.get_multiple_accounts(&[pump_key, market, amm_key]).await?;
    let pump = discovery[0].as_ref().context("Pump pool missing")?;
    let pool = pumpswap_types::pool_decode(&pump.data[8..]).context("Invalid Pump pool")?;
    let state = dl::decode_lb_pair(&discovery[1].as_ref().context("DLMM missing")?.data)?;
    let amm =
        raydium_amm_v4_types::amm_info_decode(&discovery[2].as_ref().context("AMM missing")?.data)
            .context("Invalid AMM")?;
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
    ];
    let start = dl::bin_id_to_bin_array_index(state.active_id);
    for offset in -6..=6 {
        keys.push(dl::bin_array_pda(&market, start + offset));
    }
    keys.sort_unstable();
    keys.dedup();
    let batch =
        rpc.get_multiple_accounts_with_commitment(&keys, CommitmentConfig::processed()).await?;
    let mut cache = SubscriptionAccountCache::default();
    let mut clock = None;
    for (key, account) in keys.into_iter().zip(batch.value) {
        if let Some(account) = account {
            if key == solana_sdk::sysvar::clock::ID {
                clock = Some((
                    u64::from_le_bytes(account.data[16..24].try_into()?),
                    i64::from_le_bytes(account.data[32..40].try_into()?) as u64,
                ));
            }
            cache.update(
                key,
                CachedAccount {
                    owner: account.owner,
                    data: account.data,
                    slot: batch.context.slot,
                    write_version: 0,
                },
            )?;
        }
    }
    let (epoch, unix_timestamp) = clock.context("Clock missing")?;
    Ok(demo::DemoSnapshot {
        cache: Arc::new(cache),
        context: CacheReadContext { slot: batch.context.slot, epoch, maximum_slot_age: 0 },
        unix_timestamp,
        pool: pump_key,
        token: pool.base_mint,
        candidates: vec![
            PoolTradeHint {
                pool: market,
                input_mint: pool.quote_mint,
                output_mint: USDC_TOKEN_ACCOUNT,
            },
            PoolTradeHint {
                pool: amm_key,
                input_mint: WSOL_TOKEN_ACCOUNT,
                output_mint: USDC_TOKEN_ACCOUNT,
            },
        ],
    })
}

fn simulation_transaction(
    funder: Pubkey,
    wallet: &Keypair,
    mut ixs: Vec<Instruction>,
    hash: Hash,
) -> Result<VersionedTransaction> {
    ixs.insert(
        0,
        solana_system_interface::instruction::transfer(&funder, &wallet.pubkey(), 100_000_000),
    );
    let config = solana_message::v1::TransactionConfig::empty()
        .with_compute_unit_limit(1_000_000)
        .with_loaded_accounts_data_size_limit(64 * 1024 * 1024);
    let message = solana_message::VersionedMessage::V1(
        solana_message::v1::Message::try_compile_with_config(&funder, &ixs, hash, config)?,
    );
    let mut signatures =
        vec![Signature::default(); message.header().num_required_signatures as usize];
    let index = message
        .static_account_keys()
        .iter()
        .position(|p| *p == wallet.pubkey())
        .context("Wallet signer missing")?;
    signatures[index] = wallet.sign_message(&message.serialize());
    ensure!(
        signatures[index].verify(wallet.pubkey().as_ref(), &message.serialize()),
        "Invalid local signature"
    );
    Ok(VersionedTransaction { message, signatures })
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let mode = args.first().map(String::as_str).unwrap_or("sol-repeat");
    ensure!(["sol-repeat", "wsol-repeat", "roundtrip"].contains(&mode), "Unknown mode");
    let url = std::env::var("SOLANA_RPC_URL")
        .unwrap_or_else(|_| "https://api.mainnet-beta.solana.com".into());
    let rpc = SolanaRpcClient::new_with_commitment(url, CommitmentConfig::processed());
    let funder = pubkey!("9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM");
    ensure!(rpc.get_balance(&funder).await? > 200_000_000, "Simulation funder unavailable");
    let hash = rpc.get_latest_blockhash().await?;
    let snapshot = fetch_snapshot(&rpc).await?;
    let wallet = Arc::new(Keypair::new());
    let (client, routing) = demo::configured_client(&snapshot, wallet.clone())?;
    routing.publish(snapshot.cache.clone(), snapshot.context, snapshot.unix_timestamp);
    let asset = if mode == "wsol-repeat" { TradeTokenType::WSOL } else { TradeTokenType::SOL };
    let start = std::time::Instant::now();
    let mut ixs = Vec::new();
    // WSOL mode starts with existing funded WSOL, prepared only in the simulation harness.
    if mode == "wsol-repeat" {
        ixs.extend(sol_trade_sdk::trading::common::handle_wsol(&wallet.pubkey(), 2_000_000));
    }
    let mut minimum_bought = 0;
    // Keep each simulation under the runtime CPI trace limit. Repetition and
    // roundtrip are separate scenarios; three full routes exceed that limit.
    let buy_count = if mode == "roundtrip" { 1 } else { 2 };
    for _ in 0..buy_count {
        let params = client.prepare_simple_buy(demo::buy_request(&snapshot, asset, hash))?;
        ensure!(params.rpc.is_none(), "RPC retained on hot path");
        let DexParamEnum::PumpSwapQuoteRoute(route) = &params.protocol_params else {
            anyhow::bail!("Missing automatic route")
        };
        minimum_bought += route
            .preview(params.input_mint, params.output_mint, 1_000_000, 500, true)?
            .minimum_output_credit;
        let leg = PumpSwapInstructionBuilder.build_buy_instructions(&params).await?;
        let setups: Vec<_> =
            leg.iter().filter(|ix| ix.program_id == ASSOCIATED_TOKEN_PROGRAM_ID).collect();
        ensure!(
            setups.len() == 4 && setups.iter().all(|ix| ix.data == [1]),
            "ATA setup is not idempotent"
        );
        ixs.extend(leg); // Repeat modes intentionally retain both complete setups.
    }
    if mode == "roundtrip" {
        let request = SimpleSellParams::new(
            DexType::PumpSwap,
            TradeTokenType::SOL,
            snapshot.token,
            SellAmount::ExactInput(minimum_bought),
            DexParamEnum::PumpSwapAutoRoute(snapshot.pool),
            hash,
            GasFeeStrategy::new(),
        )
        .slippage_basis_points(500);
        let params = client.prepare_simple_sell(request)?;
        ensure!(params.rpc.is_none(), "Sell retained RPC");
        let sell = PumpSwapInstructionBuilder.build_sell_instructions(&params).await?;
        ensure!(
            sell.iter().filter(|ix| ix.program_id == TOKEN_PROGRAM && ix.data == [9]).count() == 1,
            "Expected WSOL close"
        );
        ixs.extend(sell);
    }
    let hotpath_us = start.elapsed().as_micros();
    let setups = ixs.iter().filter(|ix| ix.program_id == ASSOCIATED_TOKEN_PROGRAM_ID).count();
    let tx = simulation_transaction(funder, &wallet, ixs, hash)?;
    let wire = wincode::serialize(&tx)?;
    ensure!(
        wire.len() <= solana_message::v1::MAX_TRANSACTION_SIZE,
        "Simulation transaction exceeds V1 size limit"
    );
    // Explicit boundary: network simulation happens AFTER the production hot path.
    let response = rpc
        .simulate_transaction_with_config(
            &tx,
            RpcSimulateTransactionConfig {
                sig_verify: false,
                replace_recent_blockhash: false,
                commitment: Some(CommitmentConfig::processed()),
                encoding: Some(UiTransactionEncoding::Base64),
                min_context_slot: Some(snapshot.context.slot),
                inner_instructions: true,
                ..Default::default()
            },
        )
        .await?;
    let report = serde_json::json!({"mode":mode,"snapshot_slot":snapshot.context.slot,"hotpath_us":hotpath_us,
        "ata_setup_count":setups,"wire_bytes":wire.len(),"wire_base64":STANDARD.encode(wire),
        "simulation_only":true,"sig_verify":false,"virtual_funder_signed":false,"response":response});
    if let Some(path) = args.get(1) {
        std::fs::write(path, serde_json::to_vec_pretty(&report)?)?;
    }
    println!("mode={mode} snapshot={} simulation={} hotpath_us={hotpath_us} ATA_setups={setups} CU={:?} err={:?}",
        snapshot.context.slot, response.context.slot, response.value.units_consumed, response.value.err);
    ensure!(
        response.value.err.is_none(),
        "Simulation failed: {:?}; logs: {:?}",
        response.value.err,
        response.value.logs
    );
    println!("Only simulateTransaction executed; no transaction was broadcast.");
    Ok(())
}
