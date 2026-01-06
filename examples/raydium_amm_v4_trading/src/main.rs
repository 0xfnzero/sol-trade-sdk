use sol_trade_sdk::common::fast_fn::get_associated_token_address_with_program_id_fast_use_seed;
use sol_trade_sdk::{
    common::AnyResult,
    swqos::SwqosConfig,
    trading::{core::params::{RaydiumAmmV4Params, DexParamEnum}, factory::DexType},
    SolanaTrade,
};
use sol_trade_sdk::{
    common::TradeConfig, instruction::utils::raydium_amm_v4::fetch_amm_info,
    trading::common::get_multi_token_balances, TradeTokenType,
};
use solana_commitment_config::CommitmentConfig;
use solana_sdk::signature::Keypair;
use solana_sdk::signer::Signer;
use solana_streamer_sdk::streaming::event_parser::common::filter::EventTypeFilter;
use solana_streamer_sdk::streaming::event_parser::common::EventType;
use solana_streamer_sdk::streaming::event_parser::protocols::raydium_amm_v4::parser::RAYDIUM_AMM_V4_PROGRAM_ID;
use solana_streamer_sdk::streaming::event_parser::{Protocol, UnifiedEvent};
use solana_streamer_sdk::streaming::yellowstone_grpc::{AccountFilter, TransactionFilter};
use solana_streamer_sdk::streaming::YellowstoneGrpc;
use solana_streamer_sdk::{
    match_event, streaming::event_parser::protocols::raydium_amm_v4::RaydiumAmmV4SwapEvent,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

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
    let protocols = vec![Protocol::RaydiumAmmV4];
    // Filter accounts
    let account_include = vec![
        RAYDIUM_AMM_V4_PROGRAM_ID.to_string(), // Listen to raydium_amm_v4 program ID
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
        include: vec![EventType::RaydiumAmmV4SwapBaseIn, EventType::RaydiumAmmV4SwapBaseOut],
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
            RaydiumAmmV4SwapEvent => |e: RaydiumAmmV4SwapEvent| {
                // Test code, only test one transaction
                if !ALREADY_EXECUTED.swap(true, Ordering::SeqCst) {
                    let event_clone = e.clone();
                    tokio::spawn(async move {
                        if let Err(err) = raydium_amm_v4_copy_trade_with_grpc(event_clone).await {
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
    let payer = Keypair::from_base58_string("your_payer_keypair_here");
    let rpc_url = "https://api.mainnet-beta.solana.com".to_string();
    let commitment = CommitmentConfig::confirmed();
    let swqos_configs: Vec<SwqosConfig> = vec![SwqosConfig::Default(rpc_url.clone())];
    let trade_config = TradeConfig::new(rpc_url, swqos_configs, commitment);
    let solana_trade = SolanaTrade::new(Arc::new(payer), trade_config).await;
    println!("✅ SolanaTrade client initialized successfully!");
    Ok(solana_trade)
}

/// Raydium_amm_v4 sniper trade
/// This function demonstrates how to snipe a new token from a Raydium_amm_v4 trade event
async fn raydium_amm_v4_copy_trade_with_grpc(trade_info: RaydiumAmmV4SwapEvent) -> AnyResult<()> {
    println!("Testing Raydium_amm_v4 trading...");

    let client = create_solana_trade_client().await?;
    let slippage_basis_points = Some(100);
    let recent_blockhash = client.rpc.get_latest_blockhash().await?;

    let amm_info = fetch_amm_info(&client.rpc, trade_info.amm).await?;
    let (coin_reserve, pc_reserve) =
        get_multi_token_balances(&client.rpc, &amm_info.token_coin, &amm_info.token_pc).await?;
    let mint_pubkey = if amm_info.pc_mint == sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT
        || amm_info.pc_mint == sol_trade_sdk::constants::USDC_TOKEN_ACCOUNT
    {
        amm_info.coin_mint
    } else {
        amm_info.pc_mint
    };
    let params = RaydiumAmmV4Params::new(
        trade_info.amm,
        amm_info.coin_mint,
        amm_info.pc_mint,
        amm_info.token_coin,
        amm_info.token_pc,
        coin_reserve,
        pc_reserve,
    );

    let gas_fee_strategy = sol_trade_sdk::common::GasFeeStrategy::new();
    gas_fee_strategy.set_global_fee_strategy(
        150000,
        150000,
        500000,
        500000,
        0.001,
        0.001,
    );

    // Buy tokens
    println!("Buying tokens from Raydium_amm_v4...");
    let input_token_amount = 100_000;
    let is_wsol = amm_info.pc_mint == sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT
        || amm_info.coin_mint == sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT;
    let buy_params = sol_trade_sdk::TradeBuyParams {
        dex_type: DexType::RaydiumAmmV4,
        input_token_type: if is_wsol { TradeTokenType::WSOL } else { TradeTokenType::USDC },
        mint: mint_pubkey,
        input_token_amount: input_token_amount,
        slippage_basis_points: slippage_basis_points,
        recent_blockhash: Some(recent_blockhash),
        extension_params: DexParamEnum::RaydiumAmmV4(params),
        address_lookup_table_account: None,
        wait_transaction_confirmed: true,
        create_input_token_ata: is_wsol,
        close_input_token_ata: is_wsol,
        create_mint_ata: true,
        durable_nonce: None,
        fixed_output_token_amount: None,
        gas_fee_strategy: gas_fee_strategy.clone(),
        simulate: false,
        use_exact_sol_amount: None,
    };
    client.buy(buy_params).await?;

    // Sell tokens
    println!("Selling tokens from Raydium_amm_v4...");

    let rpc = client.rpc.clone();
    let payer = client.payer.pubkey();
    let account = get_associated_token_address_with_program_id_fast_use_seed(
        &payer,
        &mint_pubkey,
        &trade_info.token_program,
        client.use_seed_optimize,
    );
    let balance = rpc.get_token_account_balance(&account).await?;
    println!("Balance: {:?}", balance);
    let amount_token = balance.amount.parse::<u64>().unwrap();

    println!("Selling {} tokens", amount_token);
    let params = RaydiumAmmV4Params::from_amm_address_by_rpc(&client.rpc, trade_info.amm).await?;
    let sell_params = sol_trade_sdk::TradeSellParams {
        dex_type: DexType::RaydiumAmmV4,
        output_token_type: if is_wsol { TradeTokenType::WSOL } else { TradeTokenType::USDC },
        mint: mint_pubkey,
        input_token_amount: amount_token,
        slippage_basis_points: slippage_basis_points,
        recent_blockhash: Some(recent_blockhash),
        with_tip: false,
        extension_params: DexParamEnum::RaydiumAmmV4(params),
        address_lookup_table_account: None,
        wait_transaction_confirmed: true,
        create_output_token_ata: is_wsol,
        close_output_token_ata: is_wsol,
        close_mint_token_ata: false,
        durable_nonce: None,
        fixed_output_token_amount: None,
        gas_fee_strategy: gas_fee_strategy,
        simulate: false,
    };
    client.sell(sell_params).await?;

    // Exit program
    std::process::exit(0);
}
