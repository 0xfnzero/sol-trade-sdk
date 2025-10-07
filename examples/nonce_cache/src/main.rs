use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use sol_trade_sdk::common::nonce_cache::NonceCache;
use sol_trade_sdk::common::TradeConfig;
use sol_trade_sdk::TradeTokenType;
use sol_trade_sdk::{
    common::AnyResult,
    swqos::SwqosConfig,
    trading::{core::params::PumpFunParams, factory::DexType},
    SolanaTrade,
};
use solana_commitment_config::CommitmentConfig;
use solana_sdk::signature::Keypair;
use solana_streamer_sdk::match_event;
use solana_streamer_sdk::streaming::event_parser::common::filter::EventTypeFilter;
use solana_streamer_sdk::streaming::event_parser::common::EventType;
use solana_streamer_sdk::streaming::event_parser::protocols::pumpfun::parser::PUMPFUN_PROGRAM_ID;
use solana_streamer_sdk::streaming::event_parser::protocols::pumpfun::PumpFunTradeEvent;
use solana_streamer_sdk::streaming::event_parser::{Protocol, UnifiedEvent};
use solana_streamer_sdk::streaming::yellowstone_grpc::{AccountFilter, TransactionFilter};
use solana_streamer_sdk::streaming::YellowstoneGrpc;

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
    let protocols = vec![Protocol::PumpFun];
    // Filter accounts
    let account_include = vec![
        PUMPFUN_PROGRAM_ID.to_string(), // Listen to pumpfun program ID
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
        EventTypeFilter { include: vec![EventType::PumpFunBuy, EventType::PumpFunSell] };

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
            PumpFunTradeEvent => |e: PumpFunTradeEvent| {
                // Test code, only test one transaction
                if !ALREADY_EXECUTED.swap(true, Ordering::SeqCst) {
                    let event_clone = e.clone();
                    tokio::spawn(async move {
                        if let Err(err) = pumpfun_copy_trade_with_grpc(event_clone).await {
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

/// PumpFun sniper trade
/// This function demonstrates how to snipe a new token from a PumpFun trade event
async fn pumpfun_copy_trade_with_grpc(trade_info: PumpFunTradeEvent) -> AnyResult<()> {
    println!("Testing PumpFun trading...");

    let client = create_solana_trade_client().await?;
    let mint_pubkey = trade_info.mint;
    let slippage_basis_points = Some(100);
    let recent_blockhash = client.rpc.get_latest_blockhash().await?;

    // Setup nonce cache
    let nonce_account_str = "use_your_nonce_account_here";
    NonceCache::get_instance().init(Some(nonce_account_str.to_string()));
    NonceCache::get_instance().fetch_nonce_info_use_rpc(&client.rpc).await?;
    let durable_nonce = NonceCache::get_durable_nonce_info();

    // Buy tokens
    println!("Buying tokens from PumpFun...");
    let buy_sol_amount = 100_000;
    let buy_params = sol_trade_sdk::TradeBuyParams {
        dex_type: DexType::PumpFun,
        input_token_type: TradeTokenType::SOL,
        mint: mint_pubkey,
        input_token_amount: buy_sol_amount,
        slippage_basis_points: slippage_basis_points,
        recent_blockhash: Some(recent_blockhash),
        extension_params: Box::new(PumpFunParams::from_trade(
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
        )),
        address_lookup_table_account: None,
        wait_transaction_confirmed: true,
        create_input_token_ata: false,
        close_input_token_ata: false,
        create_mint_ata: true,
        open_seed_optimize: false,
        durable_nonce: Some(durable_nonce),
        fixed_output_token_amount: None,
    };
    client.buy(buy_params).await?;

    // Exit program
    std::process::exit(0);
}
