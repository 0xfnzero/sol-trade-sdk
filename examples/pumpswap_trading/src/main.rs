use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use sol_trade_sdk::solana_streamer_sdk::streaming::event_parser::{Protocol, UnifiedEvent};
use sol_trade_sdk::solana_streamer_sdk::streaming::yellowstone_grpc::{
    AccountFilter, TransactionFilter,
};
use sol_trade_sdk::solana_streamer_sdk::streaming::YellowstoneGrpc;
use sol_trade_sdk::solana_streamer_sdk::{
    match_event, streaming::event_parser::protocols::pumpswap::parser::PUMPSWAP_PROGRAM_ID,
};
use sol_trade_sdk::{
    common::AnyResult,
    swqos::SwqosConfig,
    trading::{core::params::PumpSwapParams, factory::DexType},
    SolanaTrade,
};
use sol_trade_sdk::{
    common::TradeConfig,
    solana_streamer_sdk::streaming::event_parser::{
        common::EventType, protocols::pumpswap::PumpSwapSellEvent,
    },
};
use sol_trade_sdk::{
    instruction::utils::pumpswap::accounts,
    solana_streamer_sdk::streaming::event_parser::{
        common::filter::EventTypeFilter, protocols::pumpswap::PumpSwapBuyEvent,
    },
};
use solana_sdk::{commitment_config::CommitmentConfig, signature::Keypair};
use solana_sdk::{pubkey::Pubkey, signer::Signer};
use spl_associated_token_account::get_associated_token_address_with_program_id;

// Global static flag to ensure transaction is executed only once
static ALREADY_EXECUTED: AtomicBool = AtomicBool::new(false);

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Subscribing to GRPC events...");

    let grpc = YellowstoneGrpc::new(
        "https://solana-yellowstone-grpc.publicnode.com:443".to_string(),
        None,
    )?;

    let callback = create_event_callback();
    let protocols = vec![Protocol::PumpSwap];
    // Filter accounts
    let account_include = vec![
        PUMPSWAP_PROGRAM_ID.to_string(), // Listen to PumpSwap program ID
    ];
    let account_exclude = vec![];
    let account_required = vec![];

    // Listen to transaction data
    let transaction_filter = TransactionFilter {
        account_include: account_include.clone(),
        account_exclude,
        account_required,
    };

    // Listen to account data belonging to owner programs -> account event monitoring
    let account_filter = AccountFilter { account: vec![], owner: vec![], filters: vec![] };

    // listen to specific event type
    let event_type_filter =
        EventTypeFilter { include: vec![EventType::PumpSwapBuy, EventType::PumpSwapSell] };

    grpc.subscribe_events_immediate(
        protocols,
        None,
        vec![transaction_filter],
        vec![account_filter],
        Some(event_type_filter),
        None,
        callback,
    )
    .await?;

    tokio::signal::ctrl_c().await?;

    Ok(())
}

/// Create an event callback function that handles different types of events
fn create_event_callback() -> impl Fn(Box<dyn UnifiedEvent>) {
    |event: Box<dyn UnifiedEvent>| {
        match_event!(event, {
            PumpSwapBuyEvent => |e: PumpSwapBuyEvent| {
                if e.base_mint == sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT || e.quote_mint == sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT {
                    // Test code, only test one transaction
                    if !ALREADY_EXECUTED.swap(true, Ordering::SeqCst) {
                        let event_clone = e.clone();
                        tokio::spawn(async move {
                            if let Err(err) = pumpswap_trade_with_grpc_buy_event(event_clone).await {
                                eprintln!("Error in trade: {:?}", err);
                                std::process::exit(0);
                            }
                        });
                    }
                }
            },
            PumpSwapSellEvent => |e: PumpSwapSellEvent| {
                if e.base_mint == sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT || e.quote_mint == sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT {
                    // Test code, only test one transaction
                    if !ALREADY_EXECUTED.swap(true, Ordering::SeqCst) {
                        let event_clone = e.clone();
                        tokio::spawn(async move {
                            if let Err(err) = pumpswap_trade_with_grpc_sell_event(event_clone).await {
                                eprintln!("Error in trade: {:?}", err);
                                std::process::exit(0);
                            }
                        });
                    }
                }
            }
        });
    }
}

/// Create SolanaTrade client
/// Initializes a new SolanaTrade client with configuration
async fn create_solana_trade_client() -> AnyResult<SolanaTrade> {
    println!("🚀 Initializing SolanaTrade client...");
    let payer = Keypair::from_base58_string("use_your_payer_keypair_here");
    let rpc_url = "https://api.mainnet-beta.solana.com".to_string();
    let commitment = CommitmentConfig::confirmed();
    let swqos_configs: Vec<SwqosConfig> = vec![SwqosConfig::Default(rpc_url.clone())];
    let trade_config = TradeConfig::new(rpc_url, swqos_configs, commitment);
    let solana_trade = SolanaTrade::new(Arc::new(payer), trade_config).await;
    // set global strategy
    sol_trade_sdk::common::GasFeeStrategy::set_global_fee_strategy(150000, 500000, 0.001, 0.001);
    println!("✅ SolanaTrade client initialized successfully!");
    Ok(solana_trade)
}

async fn pumpswap_trade_with_grpc_buy_event(trade_info: PumpSwapBuyEvent) -> AnyResult<()> {
    let params = PumpSwapParams::from_buy_trade(&trade_info);
    let mint = if trade_info.base_mint == sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT {
        trade_info.quote_mint
    } else {
        trade_info.base_mint
    };
    pumpswap_trade_with_grpc(mint, params).await?;
    Ok(())
}

async fn pumpswap_trade_with_grpc_sell_event(trade_info: PumpSwapSellEvent) -> AnyResult<()> {
    let params = PumpSwapParams::from_sell_trade(&trade_info);
    let mint = if trade_info.base_mint == sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT {
        trade_info.quote_mint
    } else {
        trade_info.base_mint
    };
    pumpswap_trade_with_grpc(mint, params).await?;
    Ok(())
}

async fn pumpswap_trade_with_grpc(mint_pubkey: Pubkey, params: PumpSwapParams) -> AnyResult<()> {
    println!("Testing PumpSwap trading...");

    let client = create_solana_trade_client().await?;
    let slippage_basis_points = Some(500);
    let recent_blockhash = client.rpc.get_latest_blockhash().await?;

    // Buy tokens
    println!("Buying tokens from PumpSwap...");
    let buy_sol_amount = 100_000;
    let buy_params = sol_trade_sdk::TradeBuyParams {
        dex_type: DexType::PumpSwap,
        mint: mint_pubkey,
        sol_amount: buy_sol_amount,
        slippage_basis_points: slippage_basis_points,
        recent_blockhash: Some(recent_blockhash),
        extension_params: Box::new(params.clone()),
        lookup_table_key: None,
        wait_transaction_confirmed: true,
        create_wsol_ata: true,
        close_wsol_ata: true,
        create_mint_ata: true,
        open_seed_optimize: false,
        durable_nonce: None,
    };
    client.buy(buy_params).await?;

    // Sell tokens
    println!("Selling tokens from PumpSwap...");

    let rpc = client.rpc.clone();
    let payer = client.payer.pubkey();
    let program_id = if params.base_mint == mint_pubkey {
        params.base_token_program
    } else {
        params.quote_token_program
    };
    let account = get_associated_token_address_with_program_id(&payer, &mint_pubkey, &program_id);
    let balance = rpc.get_token_account_balance(&account).await?;
    let amount_token = balance.amount.parse::<u64>().unwrap();
    let sell_params = sol_trade_sdk::TradeSellParams {
        dex_type: DexType::PumpSwap,
        mint: mint_pubkey,
        token_amount: amount_token,
        slippage_basis_points: slippage_basis_points,
        recent_blockhash: Some(recent_blockhash),
        with_tip: false,
        extension_params: Box::new(params.clone()),
        lookup_table_key: None,
        wait_transaction_confirmed: true,
        create_wsol_ata: true,
        close_wsol_ata: true,
        open_seed_optimize: false,
        durable_nonce: None,
    };
    client.sell(sell_params).await?;

    // Exit program
    std::process::exit(0);
}
