use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

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
    swqos::SwqosConfig,
    trading::{core::params::BonkParams, factory::DexType},
    SolanaTrade,
};
use sol_trade_sdk::{common::TradeConfig, solana_streamer_sdk::match_event};
use sol_trade_sdk::{
    constants::WSOL_TOKEN_ACCOUNT,
    solana_streamer_sdk::streaming::event_parser::common::filter::EventTypeFilter,
};
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
                if e.base_token_mint != WSOL_TOKEN_ACCOUNT && e.quote_token_mint != WSOL_TOKEN_ACCOUNT {
                    return;
                }
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
    // init gas fee strategy
    sol_trade_sdk::common::GasFeeStrategy::init_builtin_fee_strategies();
    println!("✅ SolanaTrade client initialized successfully!");
    Ok(solana_trade)
}

/// Bonk sniper trade
/// This function demonstrates how to snipe a new token from a Bonk trade event
async fn bonk_copy_trade_with_grpc(trade_info: BonkTradeEvent) -> AnyResult<()> {
    println!("Testing Bonk trading...");

    let client = create_solana_trade_client().await?;
    let mint_pubkey = trade_info.base_token_mint;
    let slippage_basis_points = Some(100);
    let recent_blockhash = client.rpc.get_latest_blockhash().await?;

    // Buy tokens
    println!("Buying tokens from Bonk...");
    let buy_sol_amount = 100_000;
    let buy_params = sol_trade_sdk::TradeBuyParams {
        dex_type: DexType::Bonk,
        mint: mint_pubkey,
        sol_amount: buy_sol_amount,
        slippage_basis_points: slippage_basis_points,
        recent_blockhash: recent_blockhash,
        extension_params: Box::new(BonkParams::from_trade(trade_info.clone())),
        lookup_table_key: None,
        wait_transaction_confirmed: true,
        create_wsol_ata: true,
        close_wsol_ata: true,
        create_mint_ata: true,
        open_seed_optimize: false,
        nonce_account: None,
        current_nonce: None,
    };
    client.buy(buy_params).await?;

    // Sell tokens
    println!("Selling tokens from Bonk...");

    let rpc = client.rpc.clone();
    let payer = client.payer.pubkey();
    let account = get_associated_token_address(&payer, &mint_pubkey);
    let balance = rpc.get_token_account_balance(&account).await?;
    println!("Balance: {:?}", balance);
    let amount_token = balance.amount.parse::<u64>().unwrap();

    println!("Selling {} tokens", amount_token);
    let sell_params = sol_trade_sdk::TradeSellParams {
        dex_type: DexType::Bonk,
        mint: mint_pubkey,
        token_amount: amount_token,
        slippage_basis_points: slippage_basis_points,
        recent_blockhash: recent_blockhash,
        extension_params: Box::new(BonkParams::from_trade(trade_info.clone())),
        lookup_table_key: None,
        wait_transaction_confirmed: true,
        create_wsol_ata: true,
        close_wsol_ata: true,
        open_seed_optimize: false,
        with_tip: false,
        nonce_account: None,
        current_nonce: None,
    };
    client.sell(sell_params).await?;

    // Exit program
    std::process::exit(0);
}
