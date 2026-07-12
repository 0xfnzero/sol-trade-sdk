use sol_trade_sdk::{
    common::{AnyResult, TradeConfig},
    swqos::SwqosConfig,
    trading::{
        core::params::{DexParamEnum, MeteoraDammV2Params},
        factory::DexType,
    },
    SolanaTrade, TradeTokenType,
};
use solana_commitment_config::CommitmentConfig;
use solana_sdk::pubkey::Pubkey;
use std::{str::FromStr, sync::Arc};

#[tokio::main]
async fn main() -> AnyResult<()> {
    println!("Testing Metaora Damm V2 trading...");

    let client = create_solana_trade_client().await?;
    let slippage_basis_points = Some(100);
    let recent_blockhash = client.infrastructure.rpc.get_latest_blockhash().await?;
    let pool = Pubkey::from_str("7dVri3qjYD3uobSZL3Zth8vSCgU6r6R2nvFsh7uVfDte").unwrap();
    let mint_pubkey = Pubkey::from_str("PRVT6TB7uss3FrUd2D9xs2zqDBsa3GbMJMwCQsgmeta").unwrap();
    let min_buy_output = required_u64_env("MIN_BUY_OUTPUT_AMOUNT")?;
    let min_sell_output = required_u64_env("MIN_SELL_OUTPUT_AMOUNT")?;

    let gas_fee_strategy = sol_trade_sdk::common::GasFeeStrategy::new();
    gas_fee_strategy.set_global_fee_strategy(150000, 150000, 500000, 500000, 0.001, 0.001);
    let pool_params =
        MeteoraDammV2Params::from_pool_address_by_rpc(&client.infrastructure.rpc, &pool).await?;
    let token_program = if pool_params.token_a_mint == mint_pubkey {
        pool_params.token_a_program
    } else if pool_params.token_b_mint == mint_pubkey {
        pool_params.token_b_program
    } else {
        anyhow::bail!("target mint does not belong to the configured Meteora pool");
    };
    let balance_before =
        client.get_payer_token_balance_with_program(&mint_pubkey, &token_program).await?;

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
        extension_params: DexParamEnum::MeteoraDammV2(pool_params),
        address_lookup_table_accounts: Vec::new(),
        wait_tx_confirmed: true,
        wait_for_all_submits: false,
        create_input_token_ata: false, //if input token is SOL/WSOL,set to true,if input token is USDC,set to false.
        close_input_token_ata: false, //if input token is SOL/WSOL,set to true,if input token is USDC,set to false.
        create_mint_ata: true,
        durable_nonce: None,
        fixed_output_token_amount: Some(min_buy_output),
        gas_fee_strategy: gas_fee_strategy.clone(),
        simulate: false,
        use_exact_sol_amount: None,
        grpc_recv_us: None,
    };
    let (ok, sigs, err, _) = client.buy(buy_params).await?;
    if !ok {
        anyhow::bail!("buy failed: {:?}; signatures: {:?}", err, sigs);
    }

    // Sell tokens
    println!("Selling tokens from Metaora Damm V2...");

    let balance_after =
        client.get_payer_token_balance_with_program(&mint_pubkey, &token_program).await?;
    let amount_token = balance_after.checked_sub(balance_before).ok_or_else(|| {
        anyhow::anyhow!("token balance decreased after buy; refusing to sell existing holdings")
    })?;
    if amount_token == 0 {
        anyhow::bail!("confirmed buy did not increase token balance");
    }
    println!("Position acquired by this run: {}", amount_token);
    let sell_params = sol_trade_sdk::TradeSellParams {
        dex_type: DexType::MeteoraDammV2,
        output_token_type: TradeTokenType::USDC,
        mint: mint_pubkey,
        input_token_amount: amount_token,
        slippage_basis_points: slippage_basis_points,
        recent_blockhash: Some(client.infrastructure.rpc.get_latest_blockhash().await?),
        with_tip: false,
        extension_params: DexParamEnum::MeteoraDammV2(
            MeteoraDammV2Params::from_pool_address_by_rpc(&client.infrastructure.rpc, &pool)
                .await?,
        ),
        address_lookup_table_accounts: Vec::new(),
        wait_tx_confirmed: true,
        wait_for_all_submits: false,
        create_output_token_ata: false, //if output token is SOL/WSOL,set to true,if output token is USDC,set to false.
        close_output_token_ata: false, //if output token is SOL/WSOL,set to true,if output token is USDC,set to false.
        close_mint_token_ata: false,
        durable_nonce: None,
        fixed_output_token_amount: Some(min_sell_output),
        gas_fee_strategy: gas_fee_strategy,
        simulate: false,
        grpc_recv_us: None,
    };
    let (ok, sigs, err, _) = client.sell(sell_params).await?;
    if !ok {
        anyhow::bail!("sell failed: {:?}; signatures: {:?}", err, sigs);
    }
    Ok(())
}

fn required_u64_env(name: &str) -> AnyResult<u64> {
    let value = std::env::var(name).map_err(|_| {
        anyhow::anyhow!("{} is required and must be a real raw-unit minimum output quote", name)
    })?;
    let amount = value.parse::<u64>().map_err(|_| anyhow::anyhow!("{} must be u64", name))?;
    if amount == 0 {
        anyhow::bail!("{} must be greater than zero", name);
    }
    Ok(amount)
}

/// Create SolanaTrade client
/// Initializes a new SolanaTrade client with configuration
async fn create_solana_trade_client() -> AnyResult<SolanaTrade> {
    println!("🚀 Initializing SolanaTrade client...");
    let payer = sol_trade_sdk::common::keypair::load_keypair_from_env("PRIVATE_KEY")?;
    let rpc_url = std::env::var("RPC_URL")
        .unwrap_or_else(|_| "https://api.mainnet-beta.solana.com".to_string());
    let commitment = CommitmentConfig::confirmed();
    let swqos_configs: Vec<SwqosConfig> = vec![SwqosConfig::Default(rpc_url.clone())];
    let trade_config = TradeConfig::builder(rpc_url, swqos_configs, commitment)
        // .create_wsol_ata_on_startup(true)  // default: true
        // .use_seed_optimize(true)            // default: true
        // .log_enabled(true)                  // default: true
        // .check_min_tip(false)               // default: false
        // .swqos_cores_from_end(false)        // default: false
        // .mev_protection(false)              // default: false
        .build();
    let solana_trade = SolanaTrade::new(Arc::new(payer), trade_config).await;
    println!("✅ SolanaTrade client initialized successfully!");
    Ok(solana_trade)
}
