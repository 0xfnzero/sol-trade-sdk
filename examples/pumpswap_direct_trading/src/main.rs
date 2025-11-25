use sol_trade_sdk::{
    SolanaTrade, TradeTokenType, common::{
        AnyResult, TradeConfig, fast_fn::get_associated_token_address_with_program_id_fast_use_seed
    }, swqos::SwqosConfig, trading::{core::params::PumpSwapParams, factory::DexType}
};
use solana_commitment_config::CommitmentConfig;
use solana_sdk::signature::Keypair;
use solana_sdk::{pubkey::Pubkey, signer::Signer};
use std::{str::FromStr, sync::Arc};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Testing PumpSwap trading...");

    let client = create_solana_trade_client().await?;
    let slippage_basis_points = Some(100);
    let recent_blockhash = client.rpc.get_latest_blockhash().await?;
    let pool = Pubkey::from_str("539m4mVWt6iduB6W8rDGPMarzNCMesuqY5eUTiiYHAgR").unwrap();
    let mint_pubkey = Pubkey::from_str("pumpCmXqMfrsAkQ5r49WcJnRayYRqmXz6ae8H7H9Dfn").unwrap();

    let gas_fee_strategy = sol_trade_sdk::common::GasFeeStrategy::new();
    gas_fee_strategy.set_global_fee_strategy(150000,150000, 500000,500000, 0.001, 0.001, 256 * 1024, 0);

    // Buy tokens
    println!("Buying tokens from PumpSwap...");
    let buy_sol_amount = 100_000;
    let buy_params = sol_trade_sdk::TradeBuyParams {
        dex_type: DexType::PumpSwap,
        input_token_type: TradeTokenType::SOL,
        mint: mint_pubkey,
        input_token_amount: buy_sol_amount,
        slippage_basis_points: slippage_basis_points,
        recent_blockhash: Some(recent_blockhash),
        extension_params: Box::new(
            PumpSwapParams::from_pool_address_by_rpc(&client.rpc, &pool).await?,
        ),
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
    println!("Selling tokens from PumpSwap...");

    let rpc = client.rpc.clone();
    let payer = client.payer.pubkey();
    let program_id = sol_trade_sdk::constants::TOKEN_PROGRAM_2022;
    let account = get_associated_token_address_with_program_id_fast_use_seed(&payer, &mint_pubkey, &program_id, client.use_seed_optimize);
    let balance = rpc.get_token_account_balance(&account).await?;
    let amount_token = balance.amount.parse::<u64>().unwrap();
    let sell_params = sol_trade_sdk::TradeSellParams {
        dex_type: DexType::PumpSwap,
        output_token_type: TradeTokenType::SOL,
        mint: mint_pubkey,
        input_token_amount: amount_token,
        slippage_basis_points: slippage_basis_points,
        recent_blockhash: Some(recent_blockhash),
        with_tip: false,
        extension_params: Box::new(
            PumpSwapParams::from_pool_address_by_rpc(&client.rpc, &pool).await?,
        ),
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

    tokio::signal::ctrl_c().await?;
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
    println!("✅ SolanaTrade client initialized successfully!");
    Ok(solana_trade)
}
