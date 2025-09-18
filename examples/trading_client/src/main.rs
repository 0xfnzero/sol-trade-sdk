use sol_trade_sdk::{
    common::{AnyResult, TradeConfig},
    swqos::{SwqosConfig, SwqosRegion},
    SolanaTrade,
};
use solana_sdk::{commitment_config::CommitmentConfig, signature::Keypair};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = create_solana_trade_client().await?;
    println!("Successfully created SolanaTrade client!");
    Ok(())
}

/// Create SolanaTrade client
/// Initializes a new SolanaTrade client with configuration
async fn create_solana_trade_client() -> AnyResult<SolanaTrade> {
    println!("Creating SolanaTrade client...");
    let payer = Keypair::from_base58_string("use_your_payer_keypair_here");
    let rpc_url = "https://mainnet.helius-rpc.com/?api-key=xxxxxx".to_string();
    println!("rpc_url: {}", rpc_url);
    let commitment = CommitmentConfig::processed();
    let swqos_configs: Vec<SwqosConfig> = vec![
        SwqosConfig::Default(rpc_url.clone()),
        SwqosConfig::Jito("your uuid".to_string(), SwqosRegion::Frankfurt, None),
        SwqosConfig::NextBlock("your api_token".to_string(), SwqosRegion::Frankfurt, None),
        SwqosConfig::Bloxroute("your api_token".to_string(), SwqosRegion::Frankfurt, None),
        SwqosConfig::ZeroSlot("your api_token".to_string(), SwqosRegion::Frankfurt, None),
        SwqosConfig::Temporal("your api_token".to_string(), SwqosRegion::Frankfurt, None),
        SwqosConfig::FlashBlock("your api_token".to_string(), SwqosRegion::Frankfurt, None),
        SwqosConfig::Node1("your api_token".to_string(), SwqosRegion::Frankfurt, None),
        SwqosConfig::BlockRazor("your api_token".to_string(), SwqosRegion::Frankfurt, None),
        SwqosConfig::Astralane("your api_token".to_string(), SwqosRegion::Frankfurt, None),
    ];
    let trade_config = TradeConfig::new(rpc_url, swqos_configs, commitment);
    let solana_trade_client = SolanaTrade::new(Arc::new(payer), trade_config).await;
    println!("SolanaTrade client created successfully!");
    Ok(solana_trade_client)
}
