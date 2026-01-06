//! Shared Infrastructure Example
//!
//! This example demonstrates how to share expensive infrastructure (RPC client, SWQOS clients)
//! across multiple wallets, significantly reducing resource usage and initialization time.
//!
//! Use this pattern when:
//! - Running a trading service with multiple wallets
//! - All wallets use the same RPC endpoint and SWQOS configuration
//! - You want to minimize memory usage and connection overhead
//!
//! Benefits:
//! - First wallet: Full async initialization (~200-500ms)
//! - Additional wallets: Fast sync initialization (~1-2ms)
//! - Shared RPC connection pool and SWQOS clients

use sol_trade_sdk::{
    common::InfrastructureConfig,
    swqos::{SwqosConfig, SwqosRegion},
    TradingClient, TradingInfrastructure,
};
use solana_commitment_config::CommitmentConfig;
use solana_sdk::signature::Keypair;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Configuration (same for all wallets)
    let rpc_url = "https://mainnet.helius-rpc.com/?api-key=xxxxxx".to_string();
    let commitment = CommitmentConfig::processed();
    let swqos_configs: Vec<SwqosConfig> = vec![
        SwqosConfig::Default(rpc_url.clone()),
        SwqosConfig::Jito("your_uuid".to_string(), SwqosRegion::Frankfurt, None),
        SwqosConfig::Bloxroute("your_api_token".to_string(), SwqosRegion::Frankfurt, None),
    ];

    // Step 1: Create shared infrastructure (expensive, do once)
    println!("Creating shared infrastructure...");
    let infra_config = InfrastructureConfig::new(rpc_url, swqos_configs, commitment);
    let infrastructure = Arc::new(TradingInfrastructure::new(infra_config).await);
    println!("Infrastructure created with {} SWQOS clients", infrastructure.swqos_clients.len());

    // Step 2: Create multiple TradingClients sharing the same infrastructure (fast)
    let wallet_keys = vec![
        "wallet1_base58_private_key_here",
        "wallet2_base58_private_key_here",
        "wallet3_base58_private_key_here",
    ];

    let mut clients = Vec::new();
    for (i, key) in wallet_keys.iter().enumerate() {
        println!("Creating client for wallet {}...", i + 1);
        let payer = Arc::new(Keypair::from_base58_string(key));

        // Fast: reuses existing infrastructure
        let client = TradingClient::from_infrastructure(
            payer,
            infrastructure.clone(),
            true, // use_seed_optimize
        );
        clients.push(client);
        println!("  Client {} created (shares infrastructure)", i + 1);
    }

    println!("\nCreated {} clients sharing 1 infrastructure instance", clients.len());
    println!("  - 1 RPC client (shared)");
    println!("  - {} SWQOS clients (shared)", infrastructure.swqos_clients.len());

    // All clients can now trade concurrently using shared resources
    // Example: clients[0].buy(buy_params).await?;

    Ok(())
}
