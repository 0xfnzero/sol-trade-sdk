//! TradingClient Creation Example
//!
//! This example demonstrates two ways to create a TradingClient:
//!
//! 1. Simple method: `TradingClient::new()` - creates client with its own infrastructure
//! 2. Shared method: `TradingClient::from_infrastructure()` - reuses existing infrastructure
//!
//! For multi-wallet scenarios, see the `shared_infrastructure` example.

use sol_trade_sdk::{
    common::{AnyResult, InfrastructureConfig, TradeConfig},
    swqos::{SwqosConfig, SwqosRegion},
    TradingClient, TradingInfrastructure,
};
use solana_commitment_config::CommitmentConfig;
use solana_sdk::signature::Keypair;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Method 1: Simple - TradingClient::new() (recommended for single wallet)
    let client = create_trading_client_simple().await?;
    println!("Method 1: Created TradingClient with new()");
    println!("  Wallet: {}", client.get_payer_pubkey());

    // Method 2: From infrastructure (recommended for multiple wallets)
    let client2 = create_trading_client_from_infrastructure().await?;
    println!("\nMethod 2: Created TradingClient with from_infrastructure()");
    println!("  Wallet: {}", client2.get_payer_pubkey());

    Ok(())
}

/// Method 1: Create TradingClient using TradeConfig (simple, self-contained)
///
/// Use this when you have a single wallet or don't need to share infrastructure.
async fn create_trading_client_simple() -> AnyResult<TradingClient> {
    let payer = Keypair::from_base58_string("use_your_payer_keypair_here");
    let rpc_url = "https://mainnet.helius-rpc.com/?api-key=xxxxxx".to_string();
    let commitment = CommitmentConfig::processed();

    let swqos_configs: Vec<SwqosConfig> = vec![
        SwqosConfig::Default(rpc_url.clone()),
        SwqosConfig::Jito("your_uuid".to_string(), SwqosRegion::Frankfurt, None),
        SwqosConfig::Bloxroute("your_api_token".to_string(), SwqosRegion::Frankfurt, None),
        SwqosConfig::ZeroSlot("your_api_token".to_string(), SwqosRegion::Frankfurt, None),
        SwqosConfig::Temporal("your_api_token".to_string(), SwqosRegion::Frankfurt, None),
        SwqosConfig::FlashBlock("your_api_token".to_string(), SwqosRegion::Frankfurt, None),
        SwqosConfig::Node1("your_api_token".to_string(), SwqosRegion::Frankfurt, None),
        SwqosConfig::BlockRazor("your_api_token".to_string(), SwqosRegion::Frankfurt, None),
        SwqosConfig::Astralane("your_api_token".to_string(), SwqosRegion::Frankfurt, None),
    ];

    // Optional: Customize WSOL ATA and Seed optimization settings
    let trade_config = TradeConfig::new(rpc_url, swqos_configs, commitment)
        .with_wsol_ata_config(
            true,  // create_wsol_ata_on_startup: Check and create WSOL ATA on startup
            true,  // use_seed_optimize: Enable seed optimization for all ATA operations
        );

    // Creates new infrastructure internally
    let client = TradingClient::new(Arc::new(payer), trade_config).await;
    Ok(client)
}

/// Method 2: Create TradingClient from shared infrastructure
///
/// Use this when you have multiple wallets sharing the same configuration.
/// The infrastructure (RPC client, SWQOS clients) is created once and shared.
async fn create_trading_client_from_infrastructure() -> AnyResult<TradingClient> {
    let payer = Keypair::from_base58_string("use_your_payer_keypair_here");
    let rpc_url = "https://mainnet.helius-rpc.com/?api-key=xxxxxx".to_string();
    let commitment = CommitmentConfig::processed();

    let swqos_configs: Vec<SwqosConfig> = vec![
        SwqosConfig::Default(rpc_url.clone()),
        SwqosConfig::Jito("your_uuid".to_string(), SwqosRegion::Frankfurt, None),
    ];

    // Create infrastructure separately (can be shared across multiple wallets)
    let infra_config = InfrastructureConfig::new(rpc_url, swqos_configs, commitment);
    let infrastructure = Arc::new(TradingInfrastructure::new(infra_config).await);

    // Create client from existing infrastructure (fast, no async needed)
    let client = TradingClient::from_infrastructure(
        Arc::new(payer),
        infrastructure,
        true, // use_seed_optimize
    );

    Ok(client)
}
