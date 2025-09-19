use std::{
    str::FromStr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use sol_trade_sdk::solana_streamer_sdk::streaming::event_parser::protocols::pumpfun::parser::PUMPFUN_PROGRAM_ID;
use sol_trade_sdk::solana_streamer_sdk::streaming::event_parser::protocols::pumpfun::PumpFunTradeEvent;
use sol_trade_sdk::solana_streamer_sdk::streaming::event_parser::{Protocol, UnifiedEvent};
use sol_trade_sdk::solana_streamer_sdk::streaming::yellowstone_grpc::{
    AccountFilter, TransactionFilter,
};
use sol_trade_sdk::solana_streamer_sdk::streaming::YellowstoneGrpc;
use sol_trade_sdk::{
    common::address_lookup_cache::AddressLookupTableCache,
    solana_streamer_sdk::streaming::event_parser::common::EventType,
};
use sol_trade_sdk::{
    common::AnyResult,
    swqos::SwqosConfig,
    trading::{core::params::PumpFunParams, factory::DexType},
    SolanaTrade,
};
use sol_trade_sdk::{
    common::SolanaRpcClient,
    solana_streamer_sdk::streaming::event_parser::common::filter::EventTypeFilter,
};
use sol_trade_sdk::{common::TradeConfig, solana_streamer_sdk::match_event};
use solana_sdk::pubkey::Pubkey;
use solana_sdk::{commitment_config::CommitmentConfig, signature::Keypair};

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

/// Setup lookup table cache
async fn setup_lookup_table_cache(
    client: Arc<SolanaRpcClient>,
    lookup_table_address: Pubkey,
) -> AnyResult<()> {
    AddressLookupTableCache::get_instance()
        .set_address_lookup_table(client, &lookup_table_address)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to set address lookup table: {}", e))?;
    Ok(())
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

/// PumpFun sniper trade
/// This function demonstrates how to snipe a new token from a PumpFun trade event
async fn pumpfun_copy_trade_with_grpc(trade_info: PumpFunTradeEvent) -> AnyResult<()> {
    println!("Testing PumpFun trading...");

    let client = create_solana_trade_client().await?;
    let mint_pubkey = trade_info.mint;
    let slippage_basis_points = Some(100);
    let recent_blockhash = client.rpc.get_latest_blockhash().await?;

    let lookup_table_key = Pubkey::from_str("use_your_lookup_table_key_here").unwrap();
    // Setup lookup table cache
    setup_lookup_table_cache(client.rpc.clone(), lookup_table_key).await?;

    // Buy tokens
    println!("Buying tokens from PumpFun...");
    let buy_sol_amount = 100_000;
    let buy_params = sol_trade_sdk::TradeBuyParams {
        dex_type: DexType::PumpFun,
        mint: mint_pubkey,
        sol_amount: buy_sol_amount,
        slippage_basis_points: slippage_basis_points,
        recent_blockhash: recent_blockhash,
        extension_params: Box::new(PumpFunParams::from_trade(&trade_info, None)),
        lookup_table_key: Some(lookup_table_key), // you still need to update the AddressLookupTableCache
        wait_transaction_confirmed: true,
        create_wsol_ata: false,
        close_wsol_ata: false,
        create_mint_ata: true,
        open_seed_optimize: false,
        nonce_account: None,
        current_nonce: None,
    };
    client.buy(buy_params).await?;

    // Exit program
    std::process::exit(0);
}
