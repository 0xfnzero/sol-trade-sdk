//! Simple high-level trading API example.
//!
//! This example focuses on parameter construction. Replace the placeholder
//! PumpFun params with real params from your parser/RPC cache before sending.

use std::sync::Arc;

use sol_trade_sdk::{
    common::{GasFeeStrategy, TradeConfig},
    constants::{TOKEN_PROGRAM_2022, WSOL_TOKEN_ACCOUNT},
    instruction::utils::pumpfun::global_constants,
    swqos::SwqosConfig,
    trading::{
        core::params::{DexParamEnum, PumpFunParams},
        factory::DexType,
    },
    AccountPolicy, BuyAmount, DurableNonceInfo, SellAmount, SimpleBuyParams, SimpleSellParams,
    SolanaTrade, TradeTokenType,
};
use solana_commitment_config::CommitmentConfig;
use solana_sdk::{hash::Hash, pubkey::Pubkey, signature::Keypair, signer::Signer};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let payer = Keypair::from_base58_string("use_your_payer_keypair_here");
    let rpc_url = std::env::var("RPC_URL")
        .unwrap_or_else(|_| "https://api.mainnet-beta.solana.com".to_string());
    let commitment = CommitmentConfig::processed();
    let swqos_configs = vec![SwqosConfig::Default(rpc_url.clone())];
    let trade_config = TradeConfig::builder(rpc_url, swqos_configs, commitment)
        // Default is true. WSOL ATA is prepared on startup, not in the hot trade tx.
        .create_wsol_ata_on_startup(true)
        .build();
    let client = SolanaTrade::new(Arc::new(payer), trade_config).await;

    let mint = Pubkey::new_unique();
    let recent_blockhash = client.infrastructure.rpc.get_latest_blockhash().await?;
    let gas_fee_strategy = GasFeeStrategy::new();
    gas_fee_strategy.set_global_fee_strategy(150_000, 150_000, 500_000, 500_000, 0.001, 0.001);

    // In production, fill these fields from your parser/RPC cache. They are
    // protocol state, not user preferences.
    let pumpfun_params = DexParamEnum::PumpFun(PumpFunParams::from_trade(
        Pubkey::new_unique(), // bonding_curve
        Pubkey::new_unique(), // associated_bonding_curve
        mint,
        // WSOL quote_mint selects PumpFun V2 SOL layout. Users still pay with
        // native SOL below by setting `TradeTokenType::SOL`.
        WSOL_TOKEN_ACCOUNT,
        Pubkey::new_unique(),  // creator from event/cache
        Pubkey::default(),     // creator_vault; SDK derives it if unavailable
        1_073_000_000_000_000, // virtual_token_reserves
        30_000_000_000,        // virtual_quote_reserves
        793_100_000_000_000,   // real_token_reserves
        0,                     // real_quote_reserves
        None,                  // close_token_account_when_sell
        global_constants::FEE_RECIPIENT,
        // If parser/cache does not know the mint owner, PumpFun now defaults to
        // Token-2022. Passing it explicitly makes the example easier to read.
        TOKEN_PROGRAM_2022,
        false,       // is_cashback_coin
        Some(false), // mayhem_mode
    ));

    let buy_params = SimpleBuyParams::new(
        DexType::PumpFun,
        // For PumpFun V2 SOL-paired coins, keep this as SOL even though V2
        // account metas use the WSOL mint as quote_mint.
        TradeTokenType::SOL,
        mint,
        // Regular PumpFun/PumpSwap buy. The SDK estimates expected output and
        // applies slippage to the maximum quote cost.
        BuyAmount::WithMaxInput { quote_amount: 100_000 },
        pumpfun_params.clone(),
        recent_blockhash,
        gas_fee_strategy.clone(),
    )
    .slippage_basis_points(300)
    // Best for bots: do not create/close ATAs in the hot transaction.
    // Use AccountPolicy::Auto for normal integrations.
    .account_policy(AccountPolicy::HotPathMinimal);

    // client.buy_simple(buy_params).await?;
    let _ = buy_params;

    // Durable nonce is supported by the same high-level params. In production,
    // fetch it with `sol_trade_sdk::fetch_nonce_info(...)` immediately before
    // building the transaction.
    let nonce_buy_params = SimpleBuyParams::new(
        DexType::PumpFun,
        TradeTokenType::SOL,
        mint,
        BuyAmount::WithMaxInput { quote_amount: 100_000 },
        pumpfun_params.clone(),
        recent_blockhash,
        gas_fee_strategy.clone(),
    )
    .durable_nonce(DurableNonceInfo {
        nonce_account: Some(Pubkey::new_unique()),
        current_nonce: Some(Hash::new_unique()),
    })
    .account_policy(AccountPolicy::HotPathMinimal);
    let _ = nonce_buy_params;

    let sell_params = SimpleSellParams::new(
        DexType::PumpFun,
        TradeTokenType::SOL,
        mint,
        SellAmount::ExactInput(1_000_000),
        pumpfun_params.clone(),
        client.infrastructure.rpc.get_latest_blockhash().await?,
        gas_fee_strategy.clone(),
    )
    .slippage_basis_points(300)
    .account_policy(AccountPolicy::HotPathMinimal);

    // client.sell_simple(sell_params).await?;
    let _ = sell_params;

    let nonce_sell_params = SimpleSellParams::new(
        DexType::PumpFun,
        TradeTokenType::SOL,
        mint,
        SellAmount::ExactInput(1_000_000),
        pumpfun_params,
        client.infrastructure.rpc.get_latest_blockhash().await?,
        gas_fee_strategy,
    )
    .durable_nonce(DurableNonceInfo {
        nonce_account: Some(Pubkey::new_unique()),
        current_nonce: Some(Hash::new_unique()),
    })
    .account_policy(AccountPolicy::HotPathMinimal);
    let _ = nonce_sell_params;

    println!("Built simple buy/sell params for payer {}", client.payer.pubkey());
    Ok(())
}
