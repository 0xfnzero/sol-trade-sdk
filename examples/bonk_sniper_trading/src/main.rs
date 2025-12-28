use sol_trade_sdk::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed;
use sol_trade_sdk::common::TradeConfig;
use sol_trade_sdk::{
    common::AnyResult,
    swqos::SwqosConfig,
    trading::{core::params::{BonkParams, DexParamEnum}, factory::DexType},
    SolanaTrade,
};
use solana_commitment_config::CommitmentConfig;
use solana_sdk::signature::Keypair;
use solana_sdk::signer::Signer;
use solana_streamer_sdk::streaming::event_parser::common::filter::EventTypeFilter;
use solana_streamer_sdk::streaming::event_parser::common::EventType;
use solana_streamer_sdk::streaming::event_parser::protocols::bonk::BonkTradeEvent;
use solana_streamer_sdk::streaming::event_parser::{Protocol, UnifiedEvent};
use solana_streamer_sdk::{match_event, streaming::ShredStreamGrpc};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

/// Atomic flag to ensure the sniper trade is executed only once
static ALREADY_EXECUTED: AtomicBool = AtomicBool::new(false);

/// Main entry point - subscribes to Bonk events and executes sniper trades on token creation
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Subscribing to ShredStream events...");
    let shred_stream = ShredStreamGrpc::new("use_your_shred_stream_url_here".to_string()).await?;
    let callback = create_event_callback();
    let protocols = vec![Protocol::Bonk];
    let event_type_filter = EventTypeFilter {
        include: vec![
            EventType::BonkBuyExactIn,
            EventType::BonkBuyExactOut,
            EventType::BonkSellExactIn,
            EventType::BonkSellExactOut,
            EventType::BonkInitialize,
            EventType::BonkInitializeV2,
        ],
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
            BonkTradeEvent => |e: BonkTradeEvent| {
                // Only process developer token creation events
                if !e.is_dev_create_token_trade {
                    return;
                }
                // Ensure we only execute the trade once using atomic compare-and-swap
                if !ALREADY_EXECUTED.swap(true, Ordering::SeqCst) {
                    let event_clone = e.clone();
                    // Spawn a new task to handle the trading operation
                    tokio::spawn(async move {
                        if let Err(err) = bonk_sniper_trade_with_shreds(event_clone).await {
                            eprintln!("Error in sniper trade: {:?}", err);
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

/// Execute Bonk sniper trading strategy based on received token creation event
/// This function buys tokens immediately after creation and then sells all tokens
async fn bonk_sniper_trade_with_shreds(trade_info: BonkTradeEvent) -> AnyResult<()> {
    println!("Testing Bonk trading...");

    let client = create_solana_trade_client().await?;
    let mint_pubkey = trade_info.base_token_mint;
    let slippage_basis_points = Some(300);
    let recent_blockhash = client.rpc.get_latest_blockhash().await?;

    let gas_fee_strategy = sol_trade_sdk::common::GasFeeStrategy::new();
    gas_fee_strategy.set_global_fee_strategy(
        150000,
        150000,
        500000,
        500000,
        0.001,
        0.001,
    );

    let token_type = if trade_info.quote_token_mint == sol_trade_sdk::constants::USD1_TOKEN_ACCOUNT
    {
        sol_trade_sdk::TradeTokenType::USD1
    } else {
        sol_trade_sdk::TradeTokenType::SOL
    };

    // Buy tokens
    println!("Buying tokens from Bonk...");
    let buy_sol_amount = 100_000;
    let buy_params = sol_trade_sdk::TradeBuyParams {
        dex_type: DexType::Bonk,
        input_token_type: token_type.clone(),
        mint: mint_pubkey,
        input_token_amount: buy_sol_amount,
        slippage_basis_points: slippage_basis_points,
        recent_blockhash: Some(recent_blockhash),
        extension_params: DexParamEnum::Bonk(BonkParams::from_dev_trade(
            trade_info.exact_in,
            trade_info.amount_in,
            trade_info.amount_out,
            trade_info.pool_state,
            trade_info.base_vault,
            trade_info.quote_vault,
            trade_info.base_token_program,
            trade_info.platform_config,
            trade_info.platform_associated_account,
            trade_info.creator_associated_account,
            trade_info.global_config,
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
    println!("Selling tokens from Bonk...");

    let rpc = client.rpc.clone();
    let payer = client.payer.pubkey();
    let account = get_associated_token_address_with_program_id_fast_use_seed(
        &payer,
        &mint_pubkey,
        &trade_info.base_token_program,
        client.use_seed_optimize,
    );
    let balance = rpc.get_token_account_balance(&account).await?;
    println!("Balance: {:?}", balance);
    let amount_token = balance.amount.parse::<u64>().unwrap();

    println!("Selling {} tokens", amount_token);
    let sell_params = sol_trade_sdk::TradeSellParams {
        dex_type: DexType::Bonk,
        output_token_type: token_type,
        mint: mint_pubkey,
        input_token_amount: amount_token,
        slippage_basis_points: slippage_basis_points,
        recent_blockhash: Some(recent_blockhash),
        extension_params: DexParamEnum::Bonk(BonkParams::immediate_sell(
            trade_info.base_token_program,
            trade_info.platform_config,
            trade_info.platform_associated_account,
            trade_info.creator_associated_account,
            trade_info.global_config,
        )),
        address_lookup_table_account: None,
        wait_transaction_confirmed: true,
        create_output_token_ata: true,
        close_output_token_ata: true,
        close_mint_token_ata: false,
        with_tip: false,
        durable_nonce: None,
        fixed_output_token_amount: None,
        gas_fee_strategy: gas_fee_strategy,
        simulate: false,
    };
    client.sell(sell_params).await?;

    // Exit program after completing the trade
    std::process::exit(0);
}
