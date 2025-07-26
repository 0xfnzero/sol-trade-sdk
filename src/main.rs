use std::{str::FromStr, sync::Arc};

use sol_trade_sdk::{
    common::{bonding_curve::BondingCurveAccount, AnyResult, PriorityFee, TradeConfig},
    swqos::{SwqosConfig, SwqosRegion},
    trading::{core::params::{BonkParams, PumpFunParams, RaydiumCpmmParams}, factory::DexType, raydium_cpmm::{common::{get_buy_token_amount, get_sell_sol_amount}}},
    SolanaTrade,
};
use sol_trade_sdk::solana_streamer_sdk::{
    match_event,
    streaming::{
        event_parser::{
            protocols::{
                bonk::{BonkPoolCreateEvent, BonkTradeEvent},
                pumpfun::{PumpFunCreateTokenEvent, PumpFunTradeEvent},
                pumpswap::{
                    PumpSwapBuyEvent, PumpSwapCreatePoolEvent, PumpSwapDepositEvent,
                    PumpSwapSellEvent, PumpSwapWithdrawEvent,
                }, raydium_cpmm::RaydiumCpmmSwapEvent,
            },
            Protocol, UnifiedEvent,
        },
        ShredStreamGrpc, YellowstoneGrpc,
    },
};
use solana_sdk::{commitment_config::CommitmentConfig, pubkey::Pubkey, signature::Keypair};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    test_create_solana_trade_client().await?;
    test_pumpswap().await?;
    test_bonk().await?;
    test_raydium_cpmm().await?;
    test_grpc().await?;
    test_shreds().await?;
    Ok(())
}

/// 创建 SolanaTrade 客户端
async fn test_create_solana_trade_client() -> AnyResult<SolanaTrade> {
    println!("Creating SolanaTrade client...");

    let payer = Keypair::new();
    let rpc_url = "https://mainnet.helius-rpc.com/?api-key=xxxxxx".to_string();

    let swqos_configs = create_swqos_configs(&rpc_url);
    let trade_config = create_trade_config(rpc_url, swqos_configs);

    let solana_trade_client = SolanaTrade::new(Arc::new(payer), trade_config).await;
    println!("SolanaTrade client created successfully!");

    Ok(solana_trade_client)
}

fn create_swqos_configs(rpc_url: &str) -> Vec<SwqosConfig> {
    vec![
        SwqosConfig::Jito(SwqosRegion::Frankfurt),
        SwqosConfig::NextBlock("your api_token".to_string(), SwqosRegion::Frankfurt),
        SwqosConfig::Bloxroute("your api_token".to_string(), SwqosRegion::Frankfurt), 
        SwqosConfig::ZeroSlot("your api_token".to_string(), SwqosRegion::Frankfurt),
        SwqosConfig::Temporal("your api_token".to_string(), SwqosRegion::Frankfurt),
        SwqosConfig::Default(rpc_url.to_string()),
    ]
}

fn create_trade_config(rpc_url: String, swqos_configs: Vec<SwqosConfig>) -> TradeConfig {
    TradeConfig {
        rpc_url,
        commitment: CommitmentConfig::confirmed(),
        priority_fee: PriorityFee::default(),
        swqos_configs,
        lookup_table_key: None,
    }
}

async fn test_pumpfun_copy_trade_with_grpc(trade_info: PumpFunTradeEvent) -> AnyResult<()> {
    println!("Testing PumpFun trading...");

    let client = test_create_solana_trade_client().await?;
    let creator = Pubkey::from_str("xxxxxx")?;
    let mint_pubkey = Pubkey::from_str("xxxxxx")?;
    let buy_sol_cost = 100_000;
    let slippage_basis_points = Some(100);
    let recent_blockhash = client.rpc.get_latest_blockhash().await?;
    let bonding_curve = BondingCurveAccount::from_trade(&trade_info);

    // Buy tokens
    println!("Buying tokens from PumpFun...");
    client.buy(
        DexType::PumpFun,
        mint_pubkey,
        Some(creator),
        buy_sol_cost,
        slippage_basis_points,
        recent_blockhash,
        None,
        Some(Box::new(PumpFunParams {
            bonding_curve: Some(Arc::new(bonding_curve.clone())),
        })),
    ).await?;

    // Sell tokens  
    println!("Selling tokens from PumpFun...");
    let amount_token = 0;
    client.sell(
        DexType::PumpFun,
        mint_pubkey,
        Some(creator),
        amount_token,
        slippage_basis_points,
        recent_blockhash,
        None,
        None,
    ).await?;

    Ok(())
}

async fn test_pumpfun_sniper_trade_with_shreds(trade_info: PumpFunTradeEvent) -> AnyResult<()> {
    println!("Testing PumpFun trading...");

    if !trade_info.is_dev_create_token_trade {
        return Ok(());
    }

    let client = test_create_solana_trade_client().await?;
    let mint_pubkey = trade_info.mint;
    let creator = trade_info.creator;
    let slippage_basis_points = Some(100);
    let recent_blockhash = client.rpc.get_latest_blockhash().await?;

    let bonding_curve = BondingCurveAccount::from_dev_trade(
        &mint_pubkey,
        trade_info.token_amount,
        trade_info.max_sol_cost,
        creator,
    );

    // Buy tokens
    println!("Buying tokens from PumpFun...");
    let buy_sol_amount = 100_000;
    client.buy(
        DexType::PumpFun,
        mint_pubkey,
        Some(creator),
        buy_sol_amount,
        slippage_basis_points,
        recent_blockhash,
        None,
        Some(Box::new(PumpFunParams {
            bonding_curve: Some(Arc::new(bonding_curve.clone())),
        })),
    ).await?;

    // Sell tokens
    println!("Selling tokens from PumpFun...");
    let amount_token = 0;
    client.sell(
        DexType::PumpFun,
        mint_pubkey,
        Some(creator),
        amount_token,
        slippage_basis_points,
        recent_blockhash,
        None,
        None,
    ).await?;

    Ok(())
}

async fn test_pumpswap() -> AnyResult<()> {
    println!("Testing PumpSwap trading...");

    let client = test_create_solana_trade_client().await?;
    let creator = Pubkey::from_str("11111111111111111111111111111111")?;
    let mint_pubkey = Pubkey::from_str("2zMMhcVQEXDtdE6vsFS7S7D5oUodfJHE8vd1gnBouauv")?;
    let buy_sol_cost = 100_000;
    let slippage_basis_points = Some(100);
    let recent_blockhash = client.rpc.get_latest_blockhash().await?;

    // Buy tokens
    println!("Buying tokens from PumpSwap...");
    client.buy(
        DexType::PumpSwap,
        mint_pubkey,
        Some(creator),
        buy_sol_cost,
        slippage_basis_points,
        recent_blockhash,
        None,
        None,
    ).await?;

    // Sell tokens
    println!("Selling tokens from PumpSwap...");
    let amount_token = 0;
    client.sell(
        DexType::PumpSwap,
        mint_pubkey,
        Some(creator),
        amount_token,
        slippage_basis_points,
        recent_blockhash,
        None,
        None,
    ).await?;

    Ok(())
}



async fn test_bonk_copy_trade_with_grpc(trade_info: BonkTradeEvent) -> AnyResult<()> {
    println!("Testing Bonk trading...");

    let client = test_create_solana_trade_client().await?;
    let mint_pubkey = Pubkey::from_str("xxxxxxx")?;
    let buy_sol_cost = 100_000;
    let slippage_basis_points = Some(100);
    let recent_blockhash = client.rpc.get_latest_blockhash().await?;

    // Buy tokens
    println!("Buying tokens from letsbonk.fun...");
    client.buy(
        DexType::Bonk,
        mint_pubkey,
        None,
        buy_sol_cost,
        slippage_basis_points,
        recent_blockhash,
        None,
        Some(Box::new(BonkParams::from_trade(trade_info))),
    ).await?;

    // Sell tokens
    println!("Selling tokens from letsbonk.fun...");
    let amount_token = 0;
    client.sell(
        DexType::Bonk,
        mint_pubkey,
        None,
        amount_token,
        slippage_basis_points,
        recent_blockhash,
        None,
        None,
    ).await?;

    Ok(())
}

async fn test_bonk_sniper_trade_with_shreds(trade_info: BonkTradeEvent) -> AnyResult<()> {
    println!("Testing Bonk trading...");

    if !trade_info.is_dev_create_token_trade {
        return Ok(());
    }

    let client = test_create_solana_trade_client().await?;
    let mint_pubkey = Pubkey::from_str("xxxxxxx")?;
    let buy_sol_cost = 100_000;
    let slippage_basis_points = Some(100);
    let recent_blockhash = client.rpc.get_latest_blockhash().await?;

    // Buy tokens
    println!("Buying tokens from letsbonk.fun...");
    client.buy(
        DexType::Bonk,
        mint_pubkey,
        None,
        buy_sol_cost,
        slippage_basis_points,
        recent_blockhash,
        None,
        Some(Box::new(BonkParams::from_dev_trade(trade_info))),
    ).await?;

    // Sell tokens
    println!("Selling tokens from letsbonk.fun...");
    let amount_token = 0;
    client.sell(
        DexType::Bonk,
        mint_pubkey,
        None,
        amount_token,
        slippage_basis_points,
        recent_blockhash,
        None,
        None,
    ).await?;

    Ok(())
}


async fn test_bonk() -> Result<(), Box<dyn std::error::Error>> {
    println!("Testing Bonk trading...");

    let client = test_create_solana_trade_client().await?;
    let mint_pubkey = Pubkey::from_str("xxxxxxx")?;
    let buy_sol_cost = 100_000;
    let slippage_basis_points = Some(100);
    let recent_blockhash = client.rpc.get_latest_blockhash().await?;

    // Buy tokens
    println!("Buying tokens from letsbonk.fun...");
    client.buy(
        DexType::Bonk,
        mint_pubkey,
        None,
        buy_sol_cost,
        slippage_basis_points,
        recent_blockhash,
        None,
        None,
    ).await?;

    // Sell tokens
    println!("Selling tokens from letsbonk.fun...");
    let amount_token = 0;
    client.sell(
        DexType::Bonk,
        mint_pubkey,
        None,
        amount_token,
        slippage_basis_points,
        recent_blockhash,
        None,
        None,
    ).await?;

    Ok(())
}


async fn test_raydium_cpmm() -> Result<(), Box<dyn std::error::Error>> {
    println!("Testing Raydium Cpmm trading...");

    let client = test_create_solana_trade_client().await?;
    let mint_pubkey = Pubkey::from_str("xxxxxxxx")?;
    let buy_sol_cost = 100_000;
    let slippage_basis_points = Some(100);
    let recent_blockhash = client.rpc.get_latest_blockhash().await?;
    let pool_state = Pubkey::from_str("xxxxxxx")?;
    let buy_amount_out = get_buy_token_amount(&client.rpc, &pool_state, buy_sol_cost).await?;
    // Buy tokens
    println!("Buying tokens from Raydium Cpmm...");
    client.buy(
        DexType::RaydiumCpmm,
        mint_pubkey,
        None,
        buy_sol_cost,
        slippage_basis_points,
        recent_blockhash,
        None,
        Some(Box::new(RaydiumCpmmParams {
            pool_state: Some(pool_state), // 如果不传，会自动计算
            mint_token_program: Some(spl_token::ID), // spl_token_2022::ID
            mint_token_in_pool_state_index: Some(1), // mint_token 在 pool_state 中的索引,默认在索引1
            minimum_amount_out: Some(buy_amount_out), // 如果不传、默认为0
            auto_handle_wsol: true,
        })),
    ).await?;

    // Sell tokens
    println!("Selling tokens from Raydium Cpmm...");
    let amount_token = 0;
    let sell_sol_amount = get_sell_sol_amount(&client.rpc, &pool_state, amount_token).await?;
    client.sell(
        DexType::RaydiumCpmm,
        mint_pubkey,
        None,
        amount_token,
        slippage_basis_points,
        recent_blockhash,
        None,
        Some(Box::new(RaydiumCpmmParams {
            pool_state: Some(pool_state), // 如果不传，会自动计算
            mint_token_program: Some(spl_token::ID), // spl_token_2022::ID
            mint_token_in_pool_state_index: Some(1), // mint_token 在 pool_state 中的索引,默认在索引1
            minimum_amount_out: Some(sell_sol_amount), // 如果不传、默认为0
            auto_handle_wsol: true,
        })),
    ).await?;

    Ok(())
}

async fn test_grpc() -> Result<(), Box<dyn std::error::Error>> {
    println!("正在订阅 GRPC 事件...");

    let grpc = YellowstoneGrpc::new(
        "https://solana-yellowstone-grpc.publicnode.com:443".to_string(),
        None,
    )?;

    let callback = create_event_callback();
    let protocols = vec![Protocol::PumpFun, Protocol::PumpSwap, Protocol::Bonk, Protocol::RaydiumCpmm];

    println!("开始监听事件，按 Ctrl+C 停止...");
    grpc.subscribe_events(protocols, None, None, None, callback).await?;

    Ok(())
}

async fn test_shreds() -> Result<(), Box<dyn std::error::Error>> {
    println!("正在订阅 ShredStream 事件...");

    let shred_stream = ShredStreamGrpc::new("http://127.0.0.1:10800".to_string()).await?;
    let callback = create_event_callback();
    let protocols = vec![Protocol::PumpFun, Protocol::PumpSwap, Protocol::Bonk];

    println!("开始监听事件，按 Ctrl+C 停止...");
    shred_stream.shredstream_subscribe(protocols, None, callback).await?;

    Ok(())
}

fn create_event_callback() -> impl Fn(Box<dyn UnifiedEvent>) {
    |event: Box<dyn UnifiedEvent>| {
        match_event!(event, {
            BonkPoolCreateEvent => |e: BonkPoolCreateEvent| {
                println!("BonkPoolCreateEvent: {:?}", e.base_mint_param.symbol);
            },
            BonkTradeEvent => |e: BonkTradeEvent| {
                println!("BonkTradeEvent: {:?}", e);
            },
            PumpFunTradeEvent => |e: PumpFunTradeEvent| {
                println!("PumpFunTradeEvent: {:?}", e);
            },
            PumpFunCreateTokenEvent => |e: PumpFunCreateTokenEvent| {
                println!("PumpFunCreateTokenEvent: {:?}", e);
            },
            PumpSwapBuyEvent => |e: PumpSwapBuyEvent| {
                println!("Buy event: {:?}", e);
            },
            PumpSwapSellEvent => |e: PumpSwapSellEvent| {
                println!("Sell event: {:?}", e);
            },
            PumpSwapCreatePoolEvent => |e: PumpSwapCreatePoolEvent| {
                println!("CreatePool event: {:?}", e);
            },
            PumpSwapDepositEvent => |e: PumpSwapDepositEvent| {
                println!("Deposit event: {:?}", e);
            },
            PumpSwapWithdrawEvent => |e: PumpSwapWithdrawEvent| {
                println!("Withdraw event: {:?}", e);
            },
            RaydiumCpmmSwapEvent => |e: RaydiumCpmmSwapEvent| {
                println!("RaydiumCpmmSwapEvent: {:?}", e);
            },
        });
    }
}
