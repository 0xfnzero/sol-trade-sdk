//! PumpFun 跟单示例（仅使用 sol-parser-sdk 订阅 gRPC 事件）
//!
//! 收到 PumpFun 买卖事件后，用事件中的参数（含 is_cashback_coin）构造交易并执行一次买+卖。

use std::sync::{atomic::{AtomicBool, Ordering}, Arc};

use sol_parser_sdk::grpc::{
    AccountFilter, ClientConfig, EventType, EventTypeFilter, OrderMode, Protocol,
    TransactionFilter, YellowstoneGrpc,
};
use sol_parser_sdk::DexEvent;
use sol_trade_sdk::common::{
    fast_fn::get_associated_token_address_with_program_id_fast_use_seed, TradeConfig,
};
use sol_trade_sdk::TradeTokenType;
use sol_trade_sdk::{
    common::AnyResult,
    swqos::SwqosConfig,
    trading::{core::params::{PumpFunParams, DexParamEnum}, factory::DexType},
    SolanaTrade,
};
use solana_commitment_config::CommitmentConfig;
use solana_sdk::signature::Keypair;
use solana_sdk::signer::Signer;

static ALREADY_EXECUTED: AtomicBool = AtomicBool::new(false);

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    println!("PumpFun 跟单示例（sol-parser-sdk gRPC）...");

    let config = ClientConfig {
        enable_metrics: false,
        connection_timeout_ms: 10000,
        request_timeout_ms: 30000,
        enable_tls: true,
        order_mode: OrderMode::Unordered,
        ..Default::default()
    };

    let grpc_endpoint = std::env::var("GRPC_ENDPOINT")
        .unwrap_or_else(|_| "https://solana-yellowstone-grpc.publicnode.com:443".to_string());
    let grpc = YellowstoneGrpc::new_with_config(
        grpc_endpoint.clone(),
        std::env::var("GRPC_AUTH_TOKEN").ok(),
        config,
    )?;

    let protocols = vec![Protocol::PumpFun];
    let transaction_filter = TransactionFilter::for_protocols(&protocols);
    let account_filter = AccountFilter::for_protocols(&protocols);
    let event_filter = EventTypeFilter::include_only(vec![
        EventType::PumpFunBuy,
        EventType::PumpFunSell,
        EventType::PumpFunBuyExactSolIn,
        EventType::PumpFunTrade,
    ]);

    let queue = grpc
        .subscribe_dex_events(vec![transaction_filter], vec![account_filter], Some(event_filter))
        .await?;

    println!("订阅已启动，等待一条 PumpFun 交易后执行跟单（仅一次）...\n");

    loop {
        if let Some(event) = queue.pop() {
            let run = match &event {
                DexEvent::PumpFunBuy(e) | DexEvent::PumpFunSell(e) | DexEvent::PumpFunBuyExactSolIn(e) => {
                    if !ALREADY_EXECUTED.swap(true, Ordering::SeqCst) {
                        Some(e.clone())
                    } else {
                        None
                    }
                }
                DexEvent::PumpFunTrade(e) => {
                    if !ALREADY_EXECUTED.swap(true, Ordering::SeqCst) {
                        Some(e.clone())
                    } else {
                        None
                    }
                }
                _ => None,
            };
            if let Some(e) = run {
                tokio::spawn(async move {
                    if let Err(err) = pumpfun_copy_trade(e).await {
                        eprintln!("跟单执行错误: {:?}", err);
                        std::process::exit(1);
                    }
                    std::process::exit(0);
                });
                break;
            }
        } else {
            tokio::time::sleep(tokio::time::Duration::from_millis(5)).await;
        }
    }

    tokio::signal::ctrl_c().await?;
    Ok(())
}

async fn create_solana_trade_client() -> AnyResult<SolanaTrade> {
    let payer = Keypair::from_base58_string("use_your_payer_keypair_here");
    let rpc_url = std::env::var("RPC_URL").unwrap_or_else(|_| "https://api.mainnet-beta.solana.com".to_string());
    let commitment = CommitmentConfig::confirmed();
    let swqos_configs: Vec<SwqosConfig> = vec![SwqosConfig::Default(rpc_url.clone())];
    let trade_config = TradeConfig::new(rpc_url, swqos_configs, commitment);
    Ok(SolanaTrade::new(Arc::new(payer), trade_config).await)
}

async fn pumpfun_copy_trade(
    e: sol_parser_sdk::core::events::PumpFunTradeEvent,
) -> AnyResult<()> {
    let client = create_solana_trade_client().await?;
    let mint_pubkey = e.mint;
    let slippage_basis_points = Some(100u64);
    let recent_blockhash = client.infrastructure.rpc.get_latest_blockhash().await?;

    let gas_fee_strategy = sol_trade_sdk::common::GasFeeStrategy::new();
    gas_fee_strategy.set_global_fee_strategy(150000, 150000, 500000, 500000, 0.001, 0.001);

    // 买入：使用事件参数，含 is_cashback_coin（来自 sol-parser-sdk 解析）
    let buy_sol_amount = 100_000u64;
    let buy_params = sol_trade_sdk::TradeBuyParams {
        dex_type: DexType::PumpFun,
        input_token_type: TradeTokenType::SOL,
        mint: mint_pubkey,
        input_token_amount: buy_sol_amount,
        slippage_basis_points,
        recent_blockhash: Some(recent_blockhash),
        extension_params: DexParamEnum::PumpFun(PumpFunParams::from_trade(
            e.bonding_curve,
            e.associated_bonding_curve,
            e.mint,
            e.creator,
            e.creator_vault,
            e.virtual_token_reserves,
            e.virtual_sol_reserves,
            e.real_token_reserves,
            e.real_sol_reserves,
            None,
            e.fee_recipient,
            e.token_program,
            e.is_cashback_coin,
        )),
        address_lookup_table_account: None,
        wait_transaction_confirmed: true,
        create_input_token_ata: false,
        close_input_token_ata: false,
        create_mint_ata: true,
        durable_nonce: None,
        fixed_output_token_amount: None,
        gas_fee_strategy: gas_fee_strategy.clone(),
        simulate: false,
        use_exact_sol_amount: None,
    };
    client.buy(buy_params).await?;

    // 卖出：查询余额后卖出，同样传入 is_cashback_coin
    let rpc = client.infrastructure.rpc.clone();
    let payer = client.payer.pubkey();
    let account = get_associated_token_address_with_program_id_fast_use_seed(
        &payer,
        &mint_pubkey,
        &e.token_program,
        client.use_seed_optimize,
    );
    let balance = rpc.get_token_account_balance(&account).await?;
    let amount_token = balance.amount.parse::<u64>().unwrap();

    let sell_params = sol_trade_sdk::TradeSellParams {
        dex_type: DexType::PumpFun,
        output_token_type: TradeTokenType::SOL,
        mint: mint_pubkey,
        input_token_amount: amount_token,
        slippage_basis_points,
        recent_blockhash: Some(recent_blockhash),
        with_tip: false,
        extension_params: DexParamEnum::PumpFun(PumpFunParams::from_trade(
            e.bonding_curve,
            e.associated_bonding_curve,
            e.mint,
            e.creator,
            e.creator_vault,
            e.virtual_token_reserves,
            e.virtual_sol_reserves,
            e.real_token_reserves,
            e.real_sol_reserves,
            Some(true),
            e.fee_recipient,
            e.token_program,
            e.is_cashback_coin,
        )),
        address_lookup_table_account: None,
        wait_transaction_confirmed: true,
        create_output_token_ata: false,
        close_output_token_ata: false,
        close_mint_token_ata: false,
        durable_nonce: None,
        fixed_output_token_amount: None,
        gas_fee_strategy,
        simulate: false,
    };
    client.sell(sell_params).await?;

    println!("跟单一次买+卖完成");
    Ok(())
}
