use sol_trade_sdk::{
    SolanaTrade, TradeTokenType, common::{
        AnyResult, TradeConfig, fast_fn::get_associated_token_address_with_program_id_fast_use_seed
    }, swqos::SwqosConfig, trading::{core::params::{MeteoraDammV2Params, DexParamEnum}, factory::DexType}
};
use solana_commitment_config::CommitmentConfig;
use solana_sdk::signature::Keypair;
use solana_sdk::{pubkey::Pubkey, signer::Signer};
use std::{str::FromStr, sync::Arc};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Testing Metaora Damm V2 trading...");

    let client = create_solana_trade_client().await?;
    let slippage_basis_points = Some(100);
    let recent_blockhash = client.infrastructure.rpc.get_latest_blockhash().await?;
    let pool = Pubkey::from_str("7dVri3qjYD3uobSZL3Zth8vSCgU6r6R2nvFsh7uVfDte").unwrap();
    let mint_pubkey = Pubkey::from_str("PRVT6TB7uss3FrUd2D9xs2zqDBsa3GbMJMwCQsgmeta").unwrap();

    let gas_fee_strategy = sol_trade_sdk::common::GasFeeStrategy::new();
    gas_fee_strategy.set_global_fee_strategy(150000, 150000, 500000, 500000, 0.001, 0.001);

    // Buy tokens
    println!("Buying tokens from Metaora Damm V2...");
    let input_token_amount = 100_000;
    let buy_params = sol_trade_sdk::TradeBuyParams {
        dex_type: DexType::MeteoraDammV2,
        input_token_type: TradeTokenType::USDC, // or USDC
        mint: mint_pubkey,
        input_token_amount: input_token_amount,
        slippage_basis_points: slippage_basis_points,
        recent_blockhash: Some(recent_blockhash),
        extension_params: DexParamEnum::MeteoraDammV2(
            MeteoraDammV2Params::from_pool_address_by_rpc(&client.infrastructure.rpc, &pool).await?,
        ),
        address_lookup_table_account: None,
        wait_transaction_confirmed: true,
        create_input_token_ata: false, //if input token is SOL/WSOL,set to true,if input token is USDC,set to false.
        close_input_token_ata: false, //if input token is SOL/WSOL,set to true,if input token is USDC,set to false.
        create_mint_ata: true,
        durable_nonce: None,
        fixed_output_token_amount: Some(1),
        gas_fee_strategy: gas_fee_strategy.clone(),
        simulate: false,
        use_exact_sol_amount: None,
    };
    client.buy(buy_params).await?;

    // Sell tokens
    println!("Selling tokens from Metaora Damm V2...");

    let rpc = client.infrastructure.rpc.clone();
    let payer = client.payer.pubkey();
    let program_id = sol_trade_sdk::constants::TOKEN_PROGRAM;
    let account = get_associated_token_address_with_program_id_fast_use_seed(&payer, &mint_pubkey, &program_id, client.use_seed_optimize);
    let balance = rpc.get_token_account_balance(&account).await?;
    let amount_token = balance.amount.parse::<u64>().unwrap();
    println!("Token balance: {}", amount_token);
    let sell_params = sol_trade_sdk::TradeSellParams {
        dex_type: DexType::MeteoraDammV2,
        output_token_type: TradeTokenType::USDC,
        mint: mint_pubkey,
        input_token_amount: amount_token,
        slippage_basis_points: slippage_basis_points,
        recent_blockhash: Some(recent_blockhash),
        with_tip: false,
        extension_params: DexParamEnum::MeteoraDammV2(
            MeteoraDammV2Params::from_pool_address_by_rpc(&client.infrastructure.rpc, &pool).await?,
        ),
        address_lookup_table_account: None,
        wait_transaction_confirmed: true,
        create_output_token_ata: false, //if output token is SOL/WSOL,set to true,if output token is USDC,set to false.
        close_output_token_ata: false, //if output token is SOL/WSOL,set to true,if output token is USDC,set to false.
        close_mint_token_ata: false,
        durable_nonce: None,
        fixed_output_token_amount: Some(1),
        gas_fee_strategy: gas_fee_strategy,
        simulate: false,
    };
    client.sell(sell_params).await?;

    // Exit program
    std::process::exit(0);
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
