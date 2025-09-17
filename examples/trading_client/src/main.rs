use sol_trade_sdk::{
    common::{AnyResult, PriorityFee, TradeConfig},
    swqos::{SwqosConfig, SwqosRegion},
    SolanaTrade,
};
use solana_sdk::{commitment_config::CommitmentConfig, signature::Keypair};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = test_create_solana_trade_client().await?;
    println!("Successfully created SolanaTrade client!");
    Ok(())
}

/// Create SolanaTrade client
/// Initializes a new SolanaTrade client with configuration
async fn test_create_solana_trade_client() -> AnyResult<SolanaTrade> {
    println!("Creating SolanaTrade client...");

    let payer = Keypair::new();
    let rpc_url = "https://mainnet.helius-rpc.com/?api-key=xxxxxx".to_string();

    println!("rpc_url: {}", rpc_url);

    let swqos_configs = create_swqos_configs(&rpc_url);
    let trade_config = create_trade_config(rpc_url, swqos_configs);

    let solana_trade_client = SolanaTrade::new(Arc::new(payer), trade_config).await;
    println!("SolanaTrade client created successfully!");

    Ok(solana_trade_client)
}

fn create_swqos_configs(rpc_url: &str) -> Vec<SwqosConfig> {
    vec![
        // First parameter is UUID, pass empty string if no UUID
        SwqosConfig::Jito("your uuid".to_string(), SwqosRegion::Frankfurt, None),
        SwqosConfig::NextBlock("your api_token".to_string(), SwqosRegion::Frankfurt, None),
        SwqosConfig::Bloxroute("your api_token".to_string(), SwqosRegion::Frankfurt, None),
        SwqosConfig::ZeroSlot("your api_token".to_string(), SwqosRegion::Frankfurt, None),
        SwqosConfig::Temporal("your api_token".to_string(), SwqosRegion::Frankfurt, None),
        // Add tg official customer https://t.me/FlashBlock_Official to get free FlashBlock key
        SwqosConfig::FlashBlock("your api_token".to_string(), SwqosRegion::Frankfurt, None),
        // Add tg official customer https://t.me/node1_me to get free Node1 key
        SwqosConfig::Node1("your api_token".to_string(), SwqosRegion::Frankfurt, None),
        SwqosConfig::BlockRazor("your api_token".to_string(), SwqosRegion::Frankfurt, None),
        SwqosConfig::Astralane("your api_token".to_string(), SwqosRegion::Frankfurt, None),
        SwqosConfig::Default(rpc_url.to_string()),
    ]
}

fn create_trade_config(rpc_url: String, swqos_configs: Vec<SwqosConfig>) -> TradeConfig {
    TradeConfig {
        rpc_url,
        commitment: CommitmentConfig::confirmed(),
        priority_fee: PriorityFee::default(),
        swqos_configs,
    }
}
