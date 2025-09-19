use sol_trade_sdk::{
    common::{
        fast_fn::get_associated_token_address_with_program_id_fast_use_seed, AnyResult, TradeConfig,
    },
    swqos::SwqosConfig,
    trading::{core::params::PumpSwapParams, factory::DexType},
    SolanaTrade,
};
use solana_sdk::{commitment_config::CommitmentConfig, signature::Keypair};
use solana_sdk::{pubkey::Pubkey, signer::Signer};
use std::{str::FromStr, sync::Arc};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Testing PumpSwap trading...");

    let client = create_solana_trade_client().await?;
    let slippage_basis_points = Some(100);
    let recent_blockhash = client.rpc.get_latest_blockhash().await?;
    let pool = Pubkey::from_str("9qKxzRejsV6Bp2zkefXWCbGvg61c3hHei7ShXJ4FythA").unwrap();
    let mint_pubkey = Pubkey::from_str("2zMMhcVQEXDtdE6vsFS7S7D5oUodfJHE8vd1gnBouauv").unwrap();

    // Buy tokens
    println!("Buying tokens from PumpSwap...");
    let buy_sol_amount = 100_000;
    let buy_params = sol_trade_sdk::TradeBuyParams {
        dex_type: DexType::PumpSwap,
        mint: mint_pubkey,
        sol_amount: buy_sol_amount,
        slippage_basis_points: slippage_basis_points,
        recent_blockhash: recent_blockhash,
        extension_params: Box::new(
            PumpSwapParams::from_pool_address_by_rpc(&client.rpc, &pool).await?,
        ),
        lookup_table_key: None,
        wait_transaction_confirmed: true,
        create_wsol_ata: true,
        close_wsol_ata: true,
        create_mint_ata: true,
        open_seed_optimize: true, // ❗️❗️❗️❗️ open seed optimize
        nonce_account: None,
        current_nonce: None,
    };
    client.buy(buy_params).await?;

    tokio::time::sleep(std::time::Duration::from_secs(4)).await;

    // Sell tokens
    println!("Selling tokens from PumpSwap...");

    let rpc = client.rpc.clone();
    let payer = client.payer.pubkey();
    let program_id = spl_token::ID;
    // ❗️❗️❗️❗️  Must use the 'use seed' method to get the ATA account, otherwise the transaction will fail
    let account = get_associated_token_address_with_program_id_fast_use_seed(
        &payer,
        &mint_pubkey,
        &program_id,
        true,
    );
    let balance = rpc.get_token_account_balance(&account).await?;
    let amount_token = balance.amount.parse::<u64>().unwrap();
    let sell_params = sol_trade_sdk::TradeSellParams {
        dex_type: DexType::PumpSwap,
        mint: mint_pubkey,
        token_amount: amount_token,
        slippage_basis_points: slippage_basis_points,
        recent_blockhash: recent_blockhash,
        with_tip: false,
        extension_params: Box::new(
            PumpSwapParams::from_pool_address_by_rpc(&client.rpc, &pool).await?,
        ),
        lookup_table_key: None,
        wait_transaction_confirmed: true,
        create_wsol_ata: true,
        close_wsol_ata: true,
        open_seed_optimize: true, // ❗️❗️❗️❗️ open seed optimize
        nonce_account: None,
        current_nonce: None,
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
    // set global strategy
    sol_trade_sdk::common::GasFeeStrategy::set_global_fee_strategy(150000, 500000, 0.001, 0.001);
    println!("✅ SolanaTrade client initialized successfully!");
    Ok(solana_trade)
}
