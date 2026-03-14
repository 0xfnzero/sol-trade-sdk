use std::{
    str::FromStr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use sol_parser_sdk::grpc::{
    AccountFilter, ClientConfig, EventType, EventTypeFilter, OrderMode, Protocol,
    TransactionFilter, YellowstoneGrpc,
};
use sol_parser_sdk::DexEvent;
use sol_trade_sdk::common::{nonce_cache::fetch_nonce_info, TradeConfig};
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
use solana_sdk::{pubkey::Pubkey, signature::Keypair};

static ALREADY_EXECUTED: AtomicBool = AtomicBool::new(false);

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Subscribing to GRPC events (sol-parser-sdk, is_cashback_coin from event)...");

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
        grpc_endpoint,
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
                    if let Err(err) = pumpfun_copy_trade_with_grpc(e).await {
                        eprintln!("Error in copy trade: {:?}", err);
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
    println!("🚀 Initializing SolanaTrade client...");
    let payer = Keypair::from_base58_string("use_your_payer_keypair_here");
    let rpc_url = "https://api.mainnet-beta.solana.com".to_string();
    let commitment = CommitmentConfig::confirmed();
    let swqos_configs: Vec<SwqosConfig> = vec![SwqosConfig::Default(rpc_url.clone())];
    let trade_config = TradeConfig::new(rpc_url, swqos_configs, commitment);
    let solana_trade = SolanaTrade::new(Arc::new(payer), trade_config).await;
    println!("✅ SolanaTrade client initialized successfully!");
    Ok(solana_trade)
}

/// PumpFun copy trade: use is_cashback_coin from gRPC event (sol-parser-sdk)
async fn pumpfun_copy_trade_with_grpc(
    trade_info: sol_parser_sdk::core::events::PumpFunTradeEvent,
) -> AnyResult<()> {
    println!("Testing PumpFun trading...");

    let client = create_solana_trade_client().await?;
    let mint_pubkey = trade_info.mint;
    let slippage_basis_points = Some(100);
    let recent_blockhash = client.infrastructure.rpc.get_latest_blockhash().await?;

    let nonce_account_str = Pubkey::from_str("use_your_nonce_account_here")?;
    let durable_nonce = fetch_nonce_info(&client.infrastructure.rpc, nonce_account_str).await;

    let gas_fee_strategy = sol_trade_sdk::common::GasFeeStrategy::new();
    gas_fee_strategy.set_global_fee_strategy(150000, 150000, 500000, 500000, 0.001, 0.001);

    // is_cashback_coin from gRPC event (sol-parser-sdk parses it from trade event)
    let buy_params = sol_trade_sdk::TradeBuyParams {
        dex_type: DexType::PumpFun,
        input_token_type: TradeTokenType::SOL,
        mint: mint_pubkey,
        input_token_amount: 100_000,
        slippage_basis_points,
        recent_blockhash: Some(recent_blockhash),
        extension_params: DexParamEnum::PumpFun(PumpFunParams::from_trade(
            trade_info.bonding_curve,
            trade_info.associated_bonding_curve,
            trade_info.mint,
            trade_info.creator,
            trade_info.creator_vault,
            trade_info.virtual_token_reserves,
            trade_info.virtual_sol_reserves,
            trade_info.real_token_reserves,
            trade_info.real_sol_reserves,
            None,
            trade_info.fee_recipient,
            trade_info.token_program,
            trade_info.is_cashback_coin,
        )),
        address_lookup_table_account: None,
        wait_transaction_confirmed: true,
        create_input_token_ata: false,
        close_input_token_ata: false,
        create_mint_ata: true,
        durable_nonce,
        fixed_output_token_amount: None,
        gas_fee_strategy: gas_fee_strategy.clone(),
        simulate: false,
        use_exact_sol_amount: None,
        grpc_recv_us: None,
    };
    client.buy(buy_params).await?;

    std::process::exit(0);
}
