use sol_trade_sdk::{
    common::AnyResult,
    constants::trade::trade::{
        DEFAULT_BUY_TIP_FEE, DEFAULT_CU_LIMIT, DEFAULT_CU_PRICE, DEFAULT_SELL_TIP_FEE,
    },
    swqos::{settings::SwqosSettings, SwqosConfig, SwqosRegion},
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
    let payer = Keypair::new();
    let rpc_url = "https://mainnet.helius-rpc.com/?api-key=xxxxxx".to_string();
    println!("rpc_url: {}", rpc_url);
    let commitment = CommitmentConfig::processed();
    let cu_limit = DEFAULT_CU_LIMIT;
    let cu_price = DEFAULT_CU_PRICE;
    let buy_tip_fee = DEFAULT_BUY_TIP_FEE;
    let sell_tip_fee = DEFAULT_SELL_TIP_FEE;
    let swqos_settings: Vec<SwqosSettings> = vec![
        SwqosSettings::new(SwqosConfig::Default(rpc_url.clone()), cu_limit, cu_price, 0.0, 0.0),
        // First parameter is UUID, pass empty string if no UUID
        SwqosSettings::new(SwqosConfig::Jito("your uuid".to_string(), SwqosRegion::Frankfurt, None), cu_limit, cu_price, buy_tip_fee, sell_tip_fee),
        SwqosSettings::new(SwqosConfig::NextBlock("your api_token".to_string(), SwqosRegion::Frankfurt, None), cu_limit, cu_price, buy_tip_fee, sell_tip_fee),
        SwqosSettings::new(SwqosConfig::Bloxroute("your api_token".to_string(), SwqosRegion::Frankfurt, None), cu_limit, cu_price, buy_tip_fee, sell_tip_fee),
        SwqosSettings::new(SwqosConfig::ZeroSlot("your api_token".to_string(), SwqosRegion::Frankfurt, None), cu_limit, cu_price, buy_tip_fee, sell_tip_fee),
        SwqosSettings::new(SwqosConfig::Temporal("your api_token".to_string(), SwqosRegion::Frankfurt, None), cu_limit, cu_price, buy_tip_fee, sell_tip_fee),
        SwqosSettings::new(SwqosConfig::FlashBlock("your api_token".to_string(), SwqosRegion::Frankfurt, None), cu_limit, cu_price, buy_tip_fee, sell_tip_fee),
        SwqosSettings::new(SwqosConfig::Node1("your api_token".to_string(), SwqosRegion::Frankfurt, None), cu_limit, cu_price, buy_tip_fee, sell_tip_fee),
        SwqosSettings::new(SwqosConfig::BlockRazor("your api_token".to_string(), SwqosRegion::Frankfurt, None), cu_limit, cu_price, buy_tip_fee, sell_tip_fee),
        SwqosSettings::new(SwqosConfig::Astralane("your api_token".to_string(), SwqosRegion::Frankfurt, None), cu_limit, cu_price, buy_tip_fee, sell_tip_fee),
    ];
    let solana_trade_client =
        SolanaTrade::new(Arc::new(payer), rpc_url, commitment, swqos_settings).await;
    println!("SolanaTrade client created successfully!");
    Ok(solana_trade_client)
}
