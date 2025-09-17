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
    match_event, streaming::event_parser::protocols::raydium_cpmm::RaydiumCpmmSwapEvent,
};
use sol_trade_sdk::{
    common::{AnyResult, PriorityFee, TradeConfig},
    swqos::SwqosConfig,
    SolanaTrade,
};
use sol_trade_sdk::{
    instruction::utils::raydium_cpmm::accounts,
    solana_streamer_sdk::streaming::event_parser::protocols::raydium_cpmm::parser::RAYDIUM_CPMM_PROGRAM_ID,
};
use sol_trade_sdk::{
    solana_streamer_sdk::streaming::event_parser::common::filter::EventTypeFilter,
    trading::factory::DexType,
};
use sol_trade_sdk::{
    solana_streamer_sdk::streaming::event_parser::common::EventType,
    trading::core::params::RaydiumCpmmParams,
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
    let protocols = vec![Protocol::RaydiumCpmm];
    // Filter accounts
    let account_include = vec![
        RAYDIUM_CPMM_PROGRAM_ID.to_string(), // Listen to raydium_cpmm program ID
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
        include: vec![EventType::RaydiumCpmmSwapBaseInput, EventType::RaydiumCpmmSwapBaseOutput],
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
            RaydiumCpmmSwapEvent => |e: RaydiumCpmmSwapEvent| {
                // Test code, only test one transaction
                if !ALREADY_EXECUTED.swap(true, Ordering::SeqCst) {
                    let event_clone = e.clone();
                    tokio::spawn(async move {
                        if let Err(err) = raydium_cpmm_copy_trade_with_grpc(event_clone).await {
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
    println!("Creating SolanaTrade client...");

    let payer = Keypair::from_base58_string("use_your_payer_keypair_here");
    let rpc_url = "https://api.mainnet-beta.solana.com".to_string();

    let swqos_configs = vec![SwqosConfig::Default(rpc_url.clone())];

    let mut priority_fee = PriorityFee::default();
    // Configure according to your needs
    priority_fee.rpc_unit_limit = 150000;

    let trade_config = TradeConfig {
        rpc_url,
        commitment: CommitmentConfig::confirmed(),
        priority_fee: priority_fee,
        swqos_configs,
    };

    let solana_trade_client = SolanaTrade::new(Arc::new(payer), trade_config).await;
    println!("SolanaTrade client created successfully!");

    Ok(solana_trade_client)
}

/// Raydium_cpmm sniper trade
/// This function demonstrates how to snipe a new token from a Raydium_cpmm trade event
async fn raydium_cpmm_copy_trade_with_grpc(trade_info: RaydiumCpmmSwapEvent) -> AnyResult<()> {
    println!("Testing Raydium_cpmm trading...");

    let client = create_solana_trade_client().await?;
    let mint_pubkey = if trade_info.input_token_mint == sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT
    {
        trade_info.output_token_mint
    } else {
        trade_info.input_token_mint
    };
    let slippage_basis_points = Some(100);
    let recent_blockhash = client.rpc.get_latest_blockhash().await?;

    let buy_params =
        RaydiumCpmmParams::from_pool_address_by_rpc(&client.rpc, &trade_info.pool_state).await?;
    // Buy tokens
    println!("Buying tokens from Raydium_cpmm...");
    let buy_sol_amount = 100_000;
    client
        .buy(
            DexType::RaydiumCpmm,
            mint_pubkey,
            buy_sol_amount,
            slippage_basis_points,
            recent_blockhash,
            None,
            Box::new(buy_params),
            None,
            true,
            true,
            true,
            true,
            false,
        )
        .await?;

    // Sell tokens
    println!("Selling tokens from Raydium_cpmm...");

    let rpc = client.rpc.clone();
    let payer = client.payer.pubkey();
    let account = get_associated_token_address(&payer, &mint_pubkey);
    let balance = rpc.get_token_account_balance(&account).await?;
    println!("Balance: {:?}", balance);
    let amount_token = balance.amount.parse::<u64>().unwrap();

    let sell_params =
        RaydiumCpmmParams::from_pool_address_by_rpc(&client.rpc, &trade_info.pool_state).await?;

    println!("Selling {} tokens", amount_token);
    client
        .sell(
            DexType::RaydiumCpmm,
            mint_pubkey,
            amount_token,
            slippage_basis_points,
            recent_blockhash,
            None,
            false,
            Box::new(sell_params),
            None,
            true,
            true,
            true,
            false,
        )
        .await?;

    // Exit program
    std::process::exit(0);
}
