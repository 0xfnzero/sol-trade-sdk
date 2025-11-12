use sol_trade_sdk::common::spl_associated_token_account::get_associated_token_address;
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
use solana_sdk::signer::Signer;
use solana_streamer_sdk::streaming::event_parser::common::filter::EventTypeFilter;
use solana_streamer_sdk::streaming::event_parser::common::EventType;
use solana_streamer_sdk::streaming::event_parser::protocols::pumpfun::PumpFunTradeEvent;
use solana_streamer_sdk::streaming::event_parser::{Protocol, UnifiedEvent};
use solana_streamer_sdk::{match_event, streaming::ShredStreamGrpc};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

/// Atomic flag to ensure the sniper trade is executed only once
static ALREADY_EXECUTED: AtomicBool = AtomicBool::new(false);

/// Main entry point - subscribes to PumpFun events and executes sniper trades on token creation
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Subscribing to ShredStream events...");
    let shred_stream = ShredStreamGrpc::new("use_your_shred_stream_url_here".to_string()).await?;
    let callback = create_event_callback();
    let protocols = vec![Protocol::PumpFun];
    let event_type_filter = EventTypeFilter {
        include: vec![EventType::PumpFunBuy, EventType::PumpFunSell, EventType::PumpFunCreateToken],
    };
    println!("Starting to listen for events, press Ctrl+C to stop...");
    shred_stream.shredstream_subscribe(protocols, None, Some(event_type_filter), callback).await?;
    tokio::signal::ctrl_c().await?;
    Ok(())
}

/// Create an event callback function that handles different types of events
fn create_event_callback() -> impl Fn(Box<dyn UnifiedEvent>) {
    |event: Box<dyn UnifiedEvent>| {
        match_event!(event, {
            PumpFunTradeEvent => |e: PumpFunTradeEvent| {
                // Only process developer token creation events
                if !e.is_dev_create_token_trade {
                    return;
                }
                // Ensure we only execute the trade once using atomic compare-and-swap
                if !ALREADY_EXECUTED.swap(true, Ordering::SeqCst) {
                    let event_clone = e.clone();
                    // Spawn a new task to handle the trading operation
                    tokio::spawn(async move {
                        if let Err(err) = pumpfun_sniper_trade_with_shreds(event_clone).await {
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
    println!("✅ SolanaTrade client initialized successfully!");
    Ok(solana_trade)
}

/// Execute PumpFun sniper trading strategy based on received token creation event
/// This function buys tokens immediately after creation and then sells all tokens
async fn pumpfun_sniper_trade_with_shreds(trade_info: PumpFunTradeEvent) -> AnyResult<()> {
    println!("Testing PumpFun trading...");

    let client = create_solana_trade_client().await?;
    let mint_pubkey = trade_info.mint;
    let slippage_basis_points = Some(300);
    let recent_blockhash = client.rpc.get_latest_blockhash().await?;

    let gas_fee_strategy = sol_trade_sdk::common::GasFeeStrategy::new();
    gas_fee_strategy.set_global_fee_strategy(150000, 500000, 0.001, 0.001, 256 * 1024, 0);

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
        extension_params: Box::new(PumpFunParams::from_dev_trade(
            trade_info.mint,
            trade_info.token_amount,
            trade_info.max_sol_cost,
            trade_info.creator,
            trade_info.bonding_curve,
            trade_info.associated_bonding_curve,
            trade_info.creator_vault,
            None,
            trade_info.fee_recipient,
        )),
        address_lookup_table_account: None,
        wait_transaction_confirmed: true,
        create_input_token_ata: true,
        close_input_token_ata: true,
        create_mint_ata: true,
        durable_nonce: None,
        fixed_output_token_amount: None,
        gas_fee_strategy: gas_fee_strategy.clone(),
        simulate: false,
    };
    client.buy(buy_params).await?;

    // Sell tokens
    println!("Selling tokens from PumpFun...");

    let rpc = client.rpc.clone();
    let payer = client.payer.pubkey();
    let account = get_associated_token_address(&payer, &mint_pubkey);
    let balance = rpc.get_token_account_balance(&account).await?;
    println!("Balance: {:?}", balance);
    let amount_token = balance.amount.parse::<u64>().unwrap();

    println!("Selling {} tokens", amount_token);
    let sell_params = sol_trade_sdk::TradeSellParams {
        dex_type: DexType::PumpFun,
        output_token_type: TradeTokenType::SOL,
        mint: mint_pubkey,
        input_token_amount: amount_token,
        slippage_basis_points: slippage_basis_points,
        recent_blockhash: Some(recent_blockhash),
        with_tip: false,
        extension_params: Box::new(PumpFunParams::immediate_sell(trade_info.creator_vault, true)),
        address_lookup_table_account: None,
        wait_transaction_confirmed: true,
        create_output_token_ata: true,
        close_output_token_ata: true,
        close_mint_token_ata: false,
        durable_nonce: None,
        fixed_output_token_amount: None,
        gas_fee_strategy: gas_fee_strategy,
        simulate: false,
    };
    client.sell(sell_params).await?;

    // PumpFunParams can also be set as PumpFunParams::immediate_sell(creator_vault, close_token_account_when_sell)
    // creator_vault can be obtained from the trade event

    // Exit program after completing the trade
    std::process::exit(0);
}
