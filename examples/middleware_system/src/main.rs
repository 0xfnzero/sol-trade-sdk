use anyhow::Result;
use sol_trade_sdk::{
    common::{AnyResult, TradeConfig},
    swqos::{SwqosConfig, SwqosRegion},
    trading::{
        core::params::PumpSwapParams, factory::DexType, middleware::builtin::LoggingMiddleware,
        InstructionMiddleware, MiddlewareManager,
    },
    SolanaTrade, TradeTokenType,
};
use solana_commitment_config::CommitmentConfig;
use solana_sdk::{instruction::Instruction, pubkey::Pubkey, signature::Keypair};
use std::{str::FromStr, sync::Arc};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    test_middleware().await?;
    Ok(())
}

/// Custom middleware
#[derive(Clone)]
pub struct CustomMiddleware;

impl InstructionMiddleware for CustomMiddleware {
    fn name(&self) -> &'static str {
        "CustomMiddleware"
    }

    fn process_protocol_instructions(
        &self,
        protocol_instructions: Vec<Instruction>,
        protocol_name: String,
        is_buy: bool,
    ) -> Result<Vec<Instruction>> {
        // do anything you want here
        // you can modify the instructions here
        Ok(protocol_instructions)
    }

    fn process_full_instructions(
        &self,
        full_instructions: Vec<Instruction>,
        protocol_name: String,
        is_buy: bool,
    ) -> Result<Vec<Instruction>> {
        // do anything you want here
        // you can modify the instructions here
        Ok(full_instructions)
    }

    fn clone_box(&self) -> Box<dyn InstructionMiddleware> {
        Box::new(self.clone())
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
    // set global strategy
    sol_trade_sdk::common::GasFeeStrategy::set_global_fee_strategy(150000, 500000, 0.001, 0.001);
    println!("✅ SolanaTrade client initialized successfully!");
    Ok(solana_trade)
}

async fn test_middleware() -> AnyResult<()> {
    let mut client = create_solana_trade_client().await?;
    // SDK example middleware that prints instruction information
    // You can reference LoggingMiddleware to implement the InstructionMiddleware trait for your own middleware
    let middleware_manager = MiddlewareManager::new().add_middleware(Box::new(CustomMiddleware));
    client = client.with_middleware_manager(middleware_manager);
    let mint_pubkey = Pubkey::from_str("pumpCmXqMfrsAkQ5r49WcJnRayYRqmXz6ae8H7H9Dfn")?;
    let buy_sol_cost = 100_000;
    let slippage_basis_points = Some(100);
    let recent_blockhash = client.rpc.get_latest_blockhash().await?;
    let pool_address = Pubkey::from_str("539m4mVWt6iduB6W8rDGPMarzNCMesuqY5eUTiiYHAgR")?;

    let buy_params = sol_trade_sdk::TradeBuyParams {
        dex_type: DexType::PumpSwap,
        input_token_type: TradeTokenType::WSOL,
        mint: mint_pubkey,
        input_token_amount: buy_sol_cost,
        slippage_basis_points: slippage_basis_points,
        recent_blockhash: Some(recent_blockhash),
        extension_params: Box::new(
            PumpSwapParams::from_pool_address_by_rpc(&client.rpc, &pool_address).await?,
        ),
        lookup_table_key: None,
        wait_transaction_confirmed: true,
        create_input_token_ata: true,
        close_input_token_ata: true,
        create_mint_ata: true,
        open_seed_optimize: false,
        durable_nonce: None,
        fixed_output_token_amount: None,
    };
    client.buy(buy_params).await?;
    println!("tip: This transaction will not succeed because we're using a test account. You can modify the code to initialize the payer with your own private key");
    Ok(())
}
