use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use sol_trade_sdk::solana_streamer_sdk::streaming::event_parser::common::filter::EventTypeFilter;
use sol_trade_sdk::solana_streamer_sdk::streaming::event_parser::common::EventType;
use sol_trade_sdk::solana_streamer_sdk::streaming::event_parser::protocols::bonk::parser::BONK_PROGRAM_ID;
use sol_trade_sdk::solana_streamer_sdk::streaming::event_parser::protocols::bonk::BonkTradeEvent;
use sol_trade_sdk::solana_streamer_sdk::streaming::event_parser::{Protocol, UnifiedEvent};
use sol_trade_sdk::solana_streamer_sdk::streaming::yellowstone_grpc::{
    AccountFilter, TransactionFilter,
};
use sol_trade_sdk::solana_streamer_sdk::streaming::YellowstoneGrpc;
use sol_trade_sdk::{
    common::AnyResult,
    swqos::{SwqosConfig, TradeType},
    trading::{core::params::BonkParams, factory::DexType},
    SolanaTrade,
};
use sol_trade_sdk::{common::TradeConfig, solana_streamer_sdk::match_event};
use solana_sdk::signer::Signer;
use solana_sdk::{commitment_config::CommitmentConfig, signature::Keypair};
use spl_associated_token_account::get_associated_token_address;

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
    let protocols = vec![Protocol::Bonk];
    // Filter accounts
    let account_include = vec![
        BONK_PROGRAM_ID.to_string(), // Listen to bonk program ID
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
    let event_type_filter = EventTypeFilter {
        include: vec![
            EventType::BonkBuyExactIn,
            EventType::BonkSellExactIn,
            EventType::BonkBuyExactOut,
            EventType::BonkSellExactOut,
        ],
    };

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
            BonkTradeEvent => |e: BonkTradeEvent| {
                // Test code, only test one transaction
                if !ALREADY_EXECUTED.swap(true, Ordering::SeqCst) {
                    let event_clone = e.clone();
                    tokio::spawn(async move {
                        if let Err(err) = bonk_copy_trade_with_grpc(event_clone).await {
                            eprintln!("Error in copy trade: {:?}", err);
                            std::process::exit(0);
                        }
                    });
                }
            },
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

/// Bonk sniper trade
/// This function demonstrates how to snipe a new token from a Bonk trade event
async fn bonk_copy_trade_with_grpc(trade_info: BonkTradeEvent) -> AnyResult<()> {
    println!("Testing Bonk trading...");

    let client = create_solana_trade_client().await?;
    let mint_pubkey = trade_info.base_token_mint;
    let quote_mint_pubkey = trade_info.quote_token_mint;
    let slippage_basis_points = Some(100);
    let recent_blockhash = client.rpc.get_latest_blockhash().await?;

    // Buy tokens
    println!("Buying tokens from Bonk...");
    let buy_sol_amount = 100_000;
    let buy_params = sol_trade_sdk::TradeSwapParams {
        dex_type: DexType::Bonk,
        trade_type: TradeType::Buy,
        input_mint: quote_mint_pubkey,
        output_mint: mint_pubkey,
        input_token_program: trade_info.quote_token_program,
        output_token_program: trade_info.base_token_program,
        input_amount: buy_sol_amount,
        slippage_basis_points: slippage_basis_points,
        recent_blockhash: Some(recent_blockhash),
        extension_params: Box::new(BonkParams::from_trade(trade_info.clone())),
        lookup_table_key: None,
        wait_transaction_confirmed: true,
        create_input_mint_ata: true,
        close_input_mint_ata: false,
        create_output_mint_ata: true,
        close_output_mint_ata: true,
        open_seed_optimize: false,
        durable_nonce: None,
        with_tip: true,
    };
    client.swap(buy_params).await?;

    // Sell tokens
    println!("Selling tokens from Bonk...");

    let rpc = client.rpc.clone();
    let payer = client.payer.pubkey();
    let account = get_associated_token_address(&payer, &mint_pubkey);
    let balance = rpc.get_token_account_balance(&account).await?;
    println!("Balance: {:?}", balance);
    let amount_token = balance.amount.parse::<u64>().unwrap();

    println!("Selling {} tokens", amount_token);
    let sell_params = sol_trade_sdk::TradeSwapParams {
        dex_type: DexType::Bonk,
        trade_type: TradeType::Sell,
        input_mint: mint_pubkey,
        output_mint: quote_mint_pubkey,
        input_token_program: trade_info.base_token_program,
        output_token_program: trade_info.quote_token_program,
        input_amount: amount_token,
        slippage_basis_points: slippage_basis_points,
        recent_blockhash: Some(recent_blockhash),
        extension_params: Box::new(BonkParams::from_trade(trade_info.clone())),
        lookup_table_key: None,
        wait_transaction_confirmed: true,
        open_seed_optimize: false,
        with_tip: false,
        durable_nonce: None,
        create_input_mint_ata: false,
        close_input_mint_ata: false,
        create_output_mint_ata: true,
        close_output_mint_ata: false,
    };
    client.swap(sell_params).await?;

    // Exit program
    std::process::exit(0);
}
