//! PumpFun 跟单示例（仅使用 sol-parser-sdk 订阅 gRPC 事件）
//!
//! 收到 PumpFun 买卖事件后，用事件中的参数（含 is_cashback_coin）构造交易并执行一次买+卖。

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use sol_parser_sdk::grpc::{
    AccountFilter, ClientConfig, EventType, EventTypeFilter, OrderMode, Protocol,
    TransactionFilter, YellowstoneGrpc,
};
use sol_parser_sdk::DexEvent;
use sol_trade_sdk::common::TradeConfig;
use sol_trade_sdk::TradeTokenType;
use sol_trade_sdk::{
    common::AnyResult,
    swqos::SwqosConfig,
    trading::{
        core::params::{DexParamEnum, PumpFunParams},
        factory::DexType,
    },
    SolanaTrade,
};
use solana_commitment_config::CommitmentConfig;

static ALREADY_EXECUTED: AtomicBool = AtomicBool::new(false);

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
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
                DexEvent::PumpFunBuy(e)
                | DexEvent::PumpFunSell(e)
                | DexEvent::PumpFunBuyExactSolIn(e) => {
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
    let payer = sol_trade_sdk::common::keypair::load_keypair_from_env("PRIVATE_KEY")?;
    let rpc_url = std::env::var("RPC_URL")
        .unwrap_or_else(|_| "https://api.mainnet-beta.solana.com".to_string());
    let commitment = CommitmentConfig::confirmed();
    let swqos_configs: Vec<SwqosConfig> = vec![SwqosConfig::Default(rpc_url.clone())];
    let trade_config = TradeConfig::builder(rpc_url, swqos_configs, commitment)
        // .create_wsol_ata_on_startup(true)  // default: true
        // .use_seed_optimize(true)            // default: true
        // .log_enabled(true)                  // default: true
        // .check_min_tip(false)               // default: false
        // .swqos_cores_from_end(false)        // default: false
        // .mev_protection(false)              // default: false
        .build();
    Ok(SolanaTrade::new(Arc::new(payer), trade_config).await)
}

async fn pumpfun_copy_trade(e: sol_parser_sdk::core::events::PumpFunTradeEvent) -> AnyResult<()> {
    let client = create_solana_trade_client().await?;
    let mint_pubkey = e.mint;
    let virtual_quote_reserves = if e.virtual_quote_reserves != 0 {
        e.virtual_quote_reserves
    } else {
        e.virtual_sol_reserves
    };
    let real_quote_reserves =
        if e.virtual_quote_reserves != 0 { e.real_quote_reserves } else { e.real_sol_reserves };
    let slippage_basis_points = Some(100u64);
    let recent_blockhash = client.infrastructure.rpc.get_latest_blockhash().await?;

    let gas_fee_strategy = sol_trade_sdk::common::GasFeeStrategy::new();
    gas_fee_strategy.set_global_fee_strategy(150000, 150000, 500000, 500000, 0.001, 0.001);
    let balance_before =
        client.get_payer_token_balance_with_program(&mint_pubkey, &e.token_program).await?;

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
            e.quote_mint,
            e.creator,
            e.creator_vault,
            e.virtual_token_reserves,
            virtual_quote_reserves,
            e.real_token_reserves,
            real_quote_reserves,
            None,
            e.fee_recipient,
            e.token_program,
            e.is_cashback_coin,
            Some(e.mayhem_mode),
        )),
        address_lookup_table_accounts: Vec::new(),
        wait_tx_confirmed: true,
        wait_for_all_submits: false,
        create_input_token_ata: false,
        close_input_token_ata: false,
        create_mint_ata: true,
        durable_nonce: None,
        fixed_output_token_amount: None,
        gas_fee_strategy: gas_fee_strategy.clone(),
        simulate: false,
        use_exact_sol_amount: None,
        grpc_recv_us: None,
    };
    let (ok, sigs, err, _) = client.buy(buy_params).await?;
    if !ok {
        return Err(
            std::io::Error::other(format!("buy failed: {:?}; sigs: {:?}", err, sigs)).into()
        );
    }

    // 卖出：查询余额后卖出，同样传入 is_cashback_coin
    let balance_after =
        client.get_payer_token_balance_with_program(&mint_pubkey, &e.token_program).await?;
    let amount_token = balance_after
        .checked_sub(balance_before)
        .ok_or_else(|| std::io::Error::other("token balance decreased after buy"))?;
    if amount_token == 0 {
        return Err(std::io::Error::other("confirmed buy did not increase token balance").into());
    }
    let sell_extension = PumpFunParams::from_mint_by_rpc(&client.infrastructure.rpc, &mint_pubkey)
        .await?
        .with_creator_vault(e.creator_vault);

    let sell_params = sol_trade_sdk::TradeSellParams {
        dex_type: DexType::PumpFun,
        output_token_type: TradeTokenType::SOL,
        mint: mint_pubkey,
        input_token_amount: amount_token,
        slippage_basis_points,
        recent_blockhash: Some(client.infrastructure.rpc.get_latest_blockhash().await?),
        with_tip: false,
        extension_params: DexParamEnum::PumpFun(sell_extension),
        address_lookup_table_accounts: Vec::new(),
        wait_tx_confirmed: true,
        wait_for_all_submits: false,
        create_output_token_ata: false,
        close_output_token_ata: false,
        close_mint_token_ata: false,
        durable_nonce: None,
        fixed_output_token_amount: None,
        gas_fee_strategy,
        simulate: false,
        grpc_recv_us: None,
    };
    let (ok, sigs, err, _) = client.sell(sell_params).await?;
    if !ok {
        return Err(
            std::io::Error::other(format!("sell failed: {:?}; sigs: {:?}", err, sigs)).into()
        );
    }

    println!("跟单一次买+卖完成");
    Ok(())
}
