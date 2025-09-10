use sol_trade_sdk::solana_streamer_sdk::{
    match_event,
    streaming::{
        event_parser::{
            common::{filter::EventTypeFilter, EventType},
            protocols::{
                bonk::{parser::BONK_PROGRAM_ID, BonkPoolCreateEvent, BonkTradeEvent},
                pumpfun::{parser::PUMPFUN_PROGRAM_ID, PumpFunCreateTokenEvent, PumpFunTradeEvent},
                pumpswap::{
                    parser::PUMPSWAP_PROGRAM_ID, PumpSwapBuyEvent, PumpSwapCreatePoolEvent,
                    PumpSwapDepositEvent, PumpSwapSellEvent, PumpSwapWithdrawEvent,
                },
                raydium_amm_v4::parser::RAYDIUM_AMM_V4_PROGRAM_ID,
                raydium_clmm::parser::RAYDIUM_CLMM_PROGRAM_ID,
                raydium_cpmm::{parser::RAYDIUM_CPMM_PROGRAM_ID, RaydiumCpmmSwapEvent},
            },
            Protocol, UnifiedEvent,
        },
        yellowstone_grpc::{AccountFilter, TransactionFilter},
        ShredStreamGrpc, YellowstoneGrpc,
    },
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("This example demonstrates how to subscribe to events using Yellowstone gRPC and ShredStream.");
    println!("You can choose which example to run by uncommenting the relevant function call.");

    // Uncomment one of these to run the example:
    test_grpc().await?; // Use public Yellowstone gRPC endpoint
                        // test_shreds().await?;  // Use local ShredStream endpoint

    Ok(())
}

/// Subscribe to events using Yellowstone gRPC
async fn test_grpc() -> Result<(), Box<dyn std::error::Error>> {
    println!("Subscribing to GRPC events...");

    // Initialize gRPC client with public endpoint
    let grpc = YellowstoneGrpc::new(
        "https://solana-yellowstone-grpc.publicnode.com:443".to_string(),
        None, // No auth token needed
    )?;

    let callback = create_event_callback();

    // Define protocols to monitor
    let protocols =
        vec![Protocol::PumpFun, Protocol::PumpSwap, Protocol::Bonk, Protocol::RaydiumCpmm];

    // Define program IDs to monitor
    let account_include = vec![
        PUMPFUN_PROGRAM_ID.to_string(),        // Listen to pumpfun program ID
        PUMPSWAP_PROGRAM_ID.to_string(),       // Listen to pumpswap program ID
        BONK_PROGRAM_ID.to_string(),           // Listen to bonk program ID
        RAYDIUM_CPMM_PROGRAM_ID.to_string(),   // Listen to raydium_cpmm program ID
        RAYDIUM_CLMM_PROGRAM_ID.to_string(),   // Listen to raydium_clmm program ID
        RAYDIUM_AMM_V4_PROGRAM_ID.to_string(), // Listen to raydium_amm_v4 program ID
    ];
    let account_exclude = vec![];
    let account_required = vec![];

    // Configure transaction filter
    let transaction_filter = TransactionFilter {
        account_include: account_include.clone(),
        account_exclude,
        account_required,
    };

    // Configure account filter for program-owned accounts
    let account_filter =
        AccountFilter { account: vec![], owner: account_include.clone(), filters: vec![] };

    // Configure event type filter (all events)
    let event_type_filter = None;
    // For specific events only:
    // let event_type_filter =
    //     EventTypeFilter { include: vec![EventType::PumpSwapBuy, EventType::PumpSwapSell] };

    println!("Starting to listen for events, press Ctrl+C to stop...");

    // Start subscription
    grpc.subscribe_events_immediate(
        protocols,
        None,
        vec![transaction_filter],
        vec![account_filter],
        event_type_filter,
        None,
        callback,
    )
    .await?;

    // Wait for termination signal
    tokio::signal::ctrl_c().await?;

    Ok(())
}

/// Subscribe to events using ShredStream
async fn test_shreds() -> Result<(), Box<dyn std::error::Error>> {
    println!("Subscribing to ShredStream events...");

    // Initialize ShredStream client with local endpoint
    let shred_stream = ShredStreamGrpc::new("http://127.0.0.1:10800".to_string()).await?;

    let callback = create_event_callback();

    // Define protocols to monitor
    let protocols = vec![Protocol::PumpFun, Protocol::PumpSwap, Protocol::Bonk];

    // Configure event type filter (all events)
    let event_type_filter = None;
    // For specific events only:
    // let event_type_filter =
    //     EventTypeFilter { include: vec![EventType::PumpSwapBuy, EventType::PumpSwapSell] };

    println!("Starting to listen for events, press Ctrl+C to stop...");

    // Start subscription
    shred_stream
        .shredstream_subscribe(
            protocols,
            None, // No slot range specified
            event_type_filter,
            callback,
        )
        .await?;

    // Wait for termination signal
    tokio::signal::ctrl_c().await?;

    Ok(())
}

/// Create an event callback function that handles different types of events
fn create_event_callback() -> impl Fn(Box<dyn UnifiedEvent>) {
    |event: Box<dyn UnifiedEvent>| {
        // Process events using match_event! macro
        match_event!(event, {
            // Bonk protocol events
            BonkPoolCreateEvent => |e: BonkPoolCreateEvent| {
                println!("BonkPoolCreateEvent: {:?}", e.base_mint_param.symbol);
            },
            BonkTradeEvent => |e: BonkTradeEvent| {
                println!("BonkTradeEvent: {:?}", e);
            },

            // PumpFun protocol events
            PumpFunTradeEvent => |e: PumpFunTradeEvent| {
                println!("PumpFunTradeEvent: {:?}", e);
            },
            PumpFunCreateTokenEvent => |e: PumpFunCreateTokenEvent| {
                println!("PumpFunCreateTokenEvent: {:?}", e);
            },

            // PumpSwap protocol events
            PumpSwapBuyEvent => |e: PumpSwapBuyEvent| {
                println!("Buy event: {:?}", e);
            },
            PumpSwapSellEvent => |e: PumpSwapSellEvent| {
                println!("Sell event: {:?}", e);
            },
            PumpSwapCreatePoolEvent => |e: PumpSwapCreatePoolEvent| {
                println!("CreatePool event: {:?}", e);
            },
            PumpSwapDepositEvent => |e: PumpSwapDepositEvent| {
                println!("Deposit event: {:?}", e);
            },
            PumpSwapWithdrawEvent => |e: PumpSwapWithdrawEvent| {
                println!("Withdraw event: {:?}", e);
            },

            // Raydium protocol events
            RaydiumCpmmSwapEvent => |e: RaydiumCpmmSwapEvent| {
                println!("RaydiumCpmmSwapEvent: {:?}", e);
            },
            // For more events and documentation, please refer to https://github.com/0xfnzero/solana-streamer
        });
    }
}
