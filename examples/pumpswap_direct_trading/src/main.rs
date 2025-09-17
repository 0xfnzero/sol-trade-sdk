use sol_trade_sdk::{
    common::{AnyResult, PriorityFee, TradeConfig},
    swqos::SwqosConfig,
    trading::{core::params::PumpSwapParams, factory::DexType},
    SolanaTrade,
};
use solana_sdk::{commitment_config::CommitmentConfig, signature::Keypair};
use solana_sdk::{pubkey::Pubkey, signer::Signer};
use spl_associated_token_account::get_associated_token_address_with_program_id;
use std::{str::FromStr, sync::Arc};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Testing PumpSwap trading...");

    let client = create_solana_trade_client().await?;
    let slippage_basis_points = Some(100);
    let recent_blockhash = client.rpc.get_latest_blockhash().await?;
    let pool = Pubkey::from_str("539m4mVWt6iduB6W8rDGPMarzNCMesuqY5eUTiiYHAgR").unwrap();
    let mint_pubkey = Pubkey::from_str("pumpCmXqMfrsAkQ5r49WcJnRayYRqmXz6ae8H7H9Dfn").unwrap();

    // Buy tokens
    println!("Buying tokens from PumpSwap...");
    let buy_sol_amount = 100_000;
    client
        .buy(
            DexType::PumpSwap,
            mint_pubkey,
            buy_sol_amount,
            slippage_basis_points,
            recent_blockhash,
            None,
            Box::new(PumpSwapParams::from_pool_address_by_rpc(&client.rpc, &pool).await?),
            None,
            true,
            true,
            true,
            true,
            false,
        )
        .await?;

    // Sell tokens
    println!("Selling tokens from PumpSwap...");

    let rpc = client.rpc.clone();
    let payer = client.payer.pubkey();
    let program_id = spl_token_2022::ID;
    let account = get_associated_token_address_with_program_id(&payer, &mint_pubkey, &program_id);
    let balance = rpc.get_token_account_balance(&account).await?;
    let amount_token = balance.amount.parse::<u64>().unwrap();
    client
        .sell(
            DexType::PumpSwap,
            mint_pubkey,
            amount_token,
            slippage_basis_points,
            recent_blockhash,
            None,
            false,
            Box::new(PumpSwapParams::from_pool_address_by_rpc(&client.rpc, &pool).await?),
            None,
            true,
            true,
            true,
            false,
        )
        .await?;

    tokio::signal::ctrl_c().await?;
    Ok(())
}

/// Create SolanaTrade client
/// Initializes a new SolanaTrade client with configuration
async fn create_solana_trade_client() -> AnyResult<SolanaTrade> {
    println!("Creating SolanaTrade client...");

    let payer = Keypair::from_base58_string("use_your_own_keypair");
    let rpc_url = "https://api.mainnet-beta.solana.com".to_string();

    let swqos_configs = vec![SwqosConfig::Default(rpc_url.clone())];

    let mut priority_fee = PriorityFee::default();
    priority_fee.buy_tip_fees = vec![0.001];
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
