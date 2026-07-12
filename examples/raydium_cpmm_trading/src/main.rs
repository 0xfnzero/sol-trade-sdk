use sol_trade_sdk::common::TradeConfig;
use sol_trade_sdk::constants::{USDC_TOKEN_ACCOUNT, WSOL_TOKEN_ACCOUNT};
use sol_trade_sdk::trading::core::params::{DexParamEnum, RaydiumCpmmParams};
use sol_trade_sdk::trading::factory::DexType;
use sol_trade_sdk::TradeTokenType;
use sol_trade_sdk::{common::AnyResult, swqos::SwqosConfig, SolanaTrade};
use solana_commitment_config::CommitmentConfig;
use solana_streamer_sdk::streaming::event_parser::common::filter::EventTypeFilter;
use solana_streamer_sdk::streaming::event_parser::common::EventType;
use solana_streamer_sdk::streaming::event_parser::protocols::raydium_cpmm::parser::RAYDIUM_CPMM_PROGRAM_ID;
use solana_streamer_sdk::streaming::event_parser::{Protocol, UnifiedEvent};
use solana_streamer_sdk::streaming::yellowstone_grpc::{AccountFilter, TransactionFilter};
use solana_streamer_sdk::streaming::YellowstoneGrpc;
use solana_streamer_sdk::{
    match_event, streaming::event_parser::protocols::raydium_cpmm::RaydiumCpmmSwapEvent,
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
                let is_wsol = e.input_token_mint == WSOL_TOKEN_ACCOUNT || e.output_token_mint == WSOL_TOKEN_ACCOUNT;
                let is_usdc = e.input_token_mint == USDC_TOKEN_ACCOUNT || e.output_token_mint == USDC_TOKEN_ACCOUNT;
                if !is_wsol && !is_usdc {
                    return;
                }
                // Test code, only test one transaction
                if !ALREADY_EXECUTED.swap(true, Ordering::SeqCst) {
                    let event_clone = e.clone();
                    tokio::spawn(async move {
                        if let Err(err) = raydium_cpmm_copy_trade_with_grpc(event_clone).await {
                            eprintln!("Error in copy trade: {:?}", err);
                            std::process::exit(1);
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
    let payer = sol_trade_sdk::common::keypair::load_keypair_from_env("PRIVATE_KEY")?;
    let rpc_url = "https://api.mainnet-beta.solana.com".to_string();
    let commitment = CommitmentConfig::confirmed();
    let swqos_configs: Vec<SwqosConfig> = vec![SwqosConfig::Default(rpc_url.clone())];
    let trade_config = TradeConfig::builder(rpc_url, swqos_configs, commitment)
        // .create_wsol_ata_on_startup(true)  // default: true
        // .use_seed_optimize(true)            // default: true
        // .log_enabled(true)                  // default: true
        // .check_min_tip(false)               // default: false
        // .swqos_cores_from_end(false)        // default: false
        // .mev_protection(false)              // default: false
        .build();
    let solana_trade = SolanaTrade::new(Arc::new(payer), trade_config).await;
    println!("✅ SolanaTrade client initialized successfully!");
    Ok(solana_trade)
}

/// Raydium_cpmm sniper trade
/// This function demonstrates how to snipe a new token from a Raydium_cpmm trade event
async fn raydium_cpmm_copy_trade_with_grpc(trade_info: RaydiumCpmmSwapEvent) -> AnyResult<()> {
    println!("Testing Raydium_cpmm trading...");

    let client = create_solana_trade_client().await?;
    let mint_pubkey = if trade_info.input_token_mint == sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT
        || trade_info.input_token_mint == sol_trade_sdk::constants::USDC_TOKEN_ACCOUNT
    {
        trade_info.output_token_mint
    } else {
        trade_info.input_token_mint
    };
    let slippage_basis_points = Some(100);
    let recent_blockhash = client.infrastructure.rpc.get_latest_blockhash().await?;

    let gas_fee_strategy = sol_trade_sdk::common::GasFeeStrategy::new();
    gas_fee_strategy.set_global_fee_strategy(150000, 150000, 500000, 500000, 0.001, 0.001);

    let buy_params = RaydiumCpmmParams::from_pool_address_by_rpc(
        &client.infrastructure.rpc,
        &trade_info.pool_state,
    )
    .await?;

    let is_wsol = trade_info.input_token_mint == sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT
        || trade_info.output_token_mint == sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT;
    let mint_token_program = if buy_params.base_mint == mint_pubkey {
        buy_params.base_token_program
    } else if buy_params.quote_mint == mint_pubkey {
        buy_params.quote_token_program
    } else {
        return Err(std::io::Error::other("target mint does not belong to pool").into());
    };
    let balance_before =
        client.get_payer_token_balance_with_program(&mint_pubkey, &mint_token_program).await?;

    // Buy tokens
    println!("Buying tokens from Raydium_cpmm...");
    let input_token_amount = 100_000;
    let buy_params = sol_trade_sdk::TradeBuyParams {
        dex_type: DexType::RaydiumCpmm,
        input_token_type: if is_wsol { TradeTokenType::SOL } else { TradeTokenType::USDC },
        mint: mint_pubkey,
        input_token_amount: input_token_amount,
        slippage_basis_points: slippage_basis_points,
        recent_blockhash: Some(recent_blockhash),
        extension_params: DexParamEnum::RaydiumCpmm(buy_params),
        address_lookup_table_accounts: Vec::new(),
        wait_tx_confirmed: true,
        wait_for_all_submits: false,
        create_input_token_ata: is_wsol,
        close_input_token_ata: is_wsol,
        create_mint_ata: true,
        durable_nonce: None,
        fixed_output_token_amount: None,
        gas_fee_strategy: gas_fee_strategy.clone(),
        simulate: false,
        use_exact_sol_amount: None,
        grpc_recv_us: None,
    };
    let (ok, sigs, err, _) = client.buy(buy_params).await?;
    if !ok {
        return Err(
            std::io::Error::other(format!("buy failed: {:?}; sigs: {:?}", err, sigs)).into()
        );
    }

    // Sell tokens
    println!("Selling tokens from Raydium_cpmm...");

    let balance_after =
        client.get_payer_token_balance_with_program(&mint_pubkey, &mint_token_program).await?;
    let amount_token = balance_after
        .checked_sub(balance_before)
        .ok_or_else(|| std::io::Error::other("token balance decreased after buy"))?;
    if amount_token == 0 {
        return Err(std::io::Error::other("confirmed buy did not increase token balance").into());
    }

    let sell_params = RaydiumCpmmParams::from_pool_address_by_rpc(
        &client.infrastructure.rpc,
        &trade_info.pool_state,
    )
    .await?;

    println!("Selling {} tokens", amount_token);
    let sell_params = sol_trade_sdk::TradeSellParams {
        dex_type: DexType::RaydiumCpmm,
        output_token_type: if is_wsol { TradeTokenType::SOL } else { TradeTokenType::USDC },
        mint: mint_pubkey,
        input_token_amount: amount_token,
        slippage_basis_points: slippage_basis_points,
        recent_blockhash: Some(client.infrastructure.rpc.get_latest_blockhash().await?),
        with_tip: false,
        extension_params: DexParamEnum::RaydiumCpmm(sell_params),
        address_lookup_table_accounts: Vec::new(),
        wait_tx_confirmed: true,
        wait_for_all_submits: false,
        create_output_token_ata: is_wsol,
        close_output_token_ata: is_wsol,
        close_mint_token_ata: false,
        durable_nonce: None,
        fixed_output_token_amount: None,
        gas_fee_strategy: gas_fee_strategy,
        simulate: false,
        grpc_recv_us: None,
    };
    let (ok, sigs, err, _) = client.sell(sell_params).await?;
    if !ok {
        return Err(
            std::io::Error::other(format!("sell failed: {:?}; sigs: {:?}", err, sigs)).into()
        );
    }

    // Exit program
    std::process::exit(0);
}
