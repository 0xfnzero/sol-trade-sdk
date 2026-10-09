#!/bin/bash

# PumpFun 真实买入延迟测试
# 测试从调用buy到提交交易的完整流程

set -e

echo "================================"
echo "  PumpFun 真实买入延迟测试"
echo "================================"
echo ""

# 检查 pkg-config
if ! command -v pkg-config &> /dev/null; then
    echo "❌ 错误: 缺少 pkg-config"
    echo ""
    echo "请先安装 pkg-config:"
    echo "  brew install pkg-config"
    echo ""
    exit 1
fi

# 使用临时生成的私钥
if [ -z "$PAYER_KEYPAIR" ]; then
    echo "📝 未设置PAYER_KEYPAIR，将在测试代码中生成临时密钥对"
    PAYER_KEYPAIR="GENERATE_NEW"  # 标记让测试代码生成新密钥对
else
    echo "📝 使用用户提供的PAYER_KEYPAIR"
fi

# 使用真实的 PumpFun 代币
TEST_MINT=${TEST_MINT:-"Dna9Y9VwbFTfFzB4kN1hAbsMfPuwGHmrfD6LUQL2pump"}
echo "🪙 测试代币: $TEST_MINT"

RPC_URL=${RPC_URL:-"https://api.mainnet-beta.solana.com"}
echo "📡 RPC地址: $RPC_URL"

# SWQOS配置 - 4个并发发送节点
SWQOS_JITO=${SWQOS_JITO:-"https://mainnet.block-engine.jito.wtf/api/v1/transactions"}
SWQOS_BLOXROUTE=${SWQOS_BLOXROUTE:-"https://ny.solana.dex.blxrbdn.com"}
SWQOS_NEXTBLOCK=${SWQOS_NEXTBLOCK:-"https://api.nextblock.io/v1/solana"}
SWQOS_FLASHBLOCK=${SWQOS_FLASHBLOCK:-"https://api.flashblock.io/v1/solana"}
echo "🚀 SWQOS节点数: 4 (Jito, Bloxroute, NextBlock, FlashBlock)"

# 买入金额 (lamports, 默认0.001 SOL)
BUY_AMOUNT=${BUY_AMOUNT:-1000000}
echo "💰 买入金额: $BUY_AMOUNT lamports (0.001 SOL)"

# 滑点
SLIPPAGE=${SLIPPAGE:-1000}
echo "📊 滑点: $SLIPPAGE basis points (10%)"

# 设置日志级别
export RUST_LOG=${RUST_LOG:-"info,sol_trade_sdk=debug"}
echo "📊 日志级别: $RUST_LOG"

echo ""
echo "⚠️  注意: 此测试使用临时生成的密钥对（无余额）"
echo "   测试目的: 验证交易构建和提交流程的延迟"
echo "   交易预期失败（余额不足），但会测量完整的性能数据"
echo ""

# 导出环境变量
export PAYER_KEYPAIR
export RPC_URL
export TEST_MINT
export BUY_AMOUNT
export SLIPPAGE
export SWQOS_JITO
export SWQOS_BLOXROUTE
export SWQOS_NEXTBLOCK
export SWQOS_FLASHBLOCK

# 清理可能存在的旧目录
rm -rf examples/pumpfun_buy_test

# 创建测试程序目录
mkdir -p examples/pumpfun_buy_test/src

# 创建测试程序
cat > examples/pumpfun_buy_test/src/main.rs << 'EOF'
use sol_trade_sdk::{
    common::{TradeConfig, AnyResult},
    constants::TOKEN_PROGRAM,
    swqos::{SwqosConfig, SwqosRegion},
    trading::{core::params::PumpFunParams, factory::DexType},
    SolanaTrade, TradeTokenType, TradeBuyParams,
};
use solana_commitment_config::CommitmentConfig;
use solana_sdk::{signature::{Keypair, Signer}, pubkey::Pubkey};
use std::sync::Arc;
use std::env;

#[tokio::main]
async fn main() -> AnyResult<()> {
    env_logger::init();

    println!("\n🚀 初始化 PumpFun 交易客户端...\n");

    // 生成临时测试密钥对
    let payer_key = env::var("PAYER_KEYPAIR").unwrap_or_else(|_| "GENERATE_NEW".to_string());
    let payer = if payer_key == "GENERATE_NEW" {
        println!("📝 生成临时测试密钥对...");
        Keypair::new()
    } else {
        Keypair::from_base58_string(&payer_key)
    };
    println!("📝 钱包地址: {}", payer.pubkey());

    let rpc_url = env::var("RPC_URL").unwrap_or_else(|_| "https://api.mainnet-beta.solana.com".to_string());
    let commitment = CommitmentConfig::confirmed();

    // 配置4个SWQOS节点并发发送
    let swqos_configs: Vec<SwqosConfig> = vec![
        SwqosConfig::Jito(
            String::new(),  // uuid
            SwqosRegion::Default,
            Some(env::var("SWQOS_JITO").unwrap_or_else(|_| "https://mainnet.block-engine.jito.wtf/api/v1/transactions".to_string()))
        , None),
        SwqosConfig::Bloxroute(
            String::new(),  // api_token
            SwqosRegion::Default,
            Some(env::var("SWQOS_BLOXROUTE").unwrap_or_else(|_| "https://ny.solana.dex.blxrbdn.com".to_string()))
        , None),
        SwqosConfig::NextBlock(
            String::new(),  // api_token
            SwqosRegion::Default,
            Some(env::var("SWQOS_NEXTBLOCK").unwrap_or_else(|_| "https://api.nextblock.io/v1/solana".to_string()))
        , None),
        SwqosConfig::FlashBlock(
            String::new(),  // api_token
            SwqosRegion::Default,
            Some(env::var("SWQOS_FLASHBLOCK").unwrap_or_else(|_| "https://api.flashblock.io/v1/solana".to_string()))
        , None),
    ];

    println!("🚀 SWQOS配置: {} 个并发节点", swqos_configs.len());

    let trade_config = TradeConfig::new(rpc_url, swqos_configs, commitment);
    let client = SolanaTrade::new(Arc::new(payer), trade_config).await;

    // 设置 PumpFun 的 gas 策略
    sol_trade_sdk::common::GasFeeStrategy::set_global_fee_strategy(200000, 1000000, 0.005, 0.01);

    println!("✅ 客户端初始化完成\n");

    let mint_str = env::var("TEST_MINT").expect("TEST_MINT not set");
    let mint = mint_str.parse().expect("Invalid mint address");
    let buy_amount = env::var("BUY_AMOUNT")
        .unwrap_or_else(|_| "1000000".to_string())
        .parse::<u64>()
        .expect("Invalid buy amount");
    let slippage = env::var("SLIPPAGE")
        .unwrap_or_else(|_| "1000".to_string())
        .parse::<u64>()
        .expect("Invalid slippage");

    println!("🔍 获取最新区块哈希...");
    let recent_blockhash = client.rpc.get_latest_blockhash().await?;
    println!("✅ 区块哈希: {}\n", recent_blockhash);

    println!("================================");
    println!("  PumpFun 买入延迟测试");
    println!("================================");
    println!("🪙 代币: {}", mint);
    println!("💰 金额: {} lamports", buy_amount);
    println!("📊 滑点: {} basis points", slippage);
    println!("================================\n");

    // PumpFun买入参数 (占位；实际路径应使用 RPC 或事件填充)
    let params = PumpFunParams::from_trade(
        Pubkey::default(),  // bonding_curve
        Pubkey::default(),  // associated_bonding_curve
        mint,               // mint
        Pubkey::default(),  // creator
        Pubkey::default(),  // creator_vault
        0,                  // virtual_token_reserves
        0,                  // virtual_sol_reserves
        0,                  // real_token_reserves
        0,                  // real_sol_reserves
        None,               // close_token_account_when_sell
        Pubkey::default(),  // fee_recipient
        TOKEN_PROGRAM,
        false,              // is_cashback_coin
        None,               // mayhem_mode（None=按 fee_recipient 推断）
    );

    let buy_params = TradeBuyParams {
        dex_type: DexType::PumpFun,
        input_token_type: TradeTokenType::SOL,
        mint,
        input_token_amount: buy_amount,
        slippage_basis_points: Some(slippage),
        recent_blockhash: Some(recent_blockhash),
        extension_params: Box::new(params),
        address_lookup_table_account: None,
        wait_transaction_confirmed: false,  // 不等待确认，测试最快提交速度
        create_input_token_ata: true,
        close_input_token_ata: true,
        create_mint_ata: true,
        open_seed_optimize: false,
        durable_nonce: None,
        fixed_output_token_amount: None,
    };

    println!("⏱️  开始执行买入流程...");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

    match client.buy(buy_params).await {
        Ok((success, signature)) => {
            println!("\n================================");
            println!("  ✅ 买入流程完成");
            println!("================================");
            println!("✅ 提交成功: {}", success);
            println!("📝 签名: {}", signature);
            println!("================================\n");
        }
        Err(e) => {
            println!("\n================================");
            println!("  ⚠️  买入流程完成（交易失败）");
            println!("================================");
            println!("ℹ️  错误: {:?}", e);
            println!("\n💡 说明: 交易失败是预期的（测试账户无余额）");
            println!("   耗时统计见上方SDK日志输出");
            println!("================================\n");
        }
    }

    // 显示性能统计
    println!("================================");
    println!("  性能优化模块状态");
    println!("================================\n");

    use sol_trade_sdk::swqos::serialization::get_serializer_stats;
    let (available, capacity) = get_serializer_stats();
    println!("📦 序列化器缓冲池:");
    println!("   容量: {}", capacity);
    println!("   可用: {}", available);
    println!("   使用: {}", capacity - available);

    use sol_trade_sdk::trading::core::transaction_pool::get_pool_stats;
    let (pool_available, pool_capacity) = get_pool_stats();
    println!("\n🔧 交易构建器池:");
    println!("   容量: {}", pool_capacity);
    println!("   可用: {}", pool_available);
    println!("   使用: {}", pool_capacity - pool_available);

    println!("\n================================");
    println!("✅ 延迟测试完成！");
    println!("================================\n");

    println!("💡 提示: 查看上面的日志了解各环节详细耗时");
    println!("   日志中包含每个步骤的 step 和 total 时间\n");

    Ok(())
}
EOF

# 创建 Cargo.toml
cat > examples/pumpfun_buy_test/Cargo.toml << 'EOF'
[package]
name = "pumpfun_buy_test"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "pumpfun_buy_test"
path = "src/main.rs"

[dependencies]
sol-trade-sdk = { path = "../.." }
solana-sdk = "3.0.0"
solana-commitment-config = "3.0.0"
tokio = { version = "1", features = ["full"] }
anyhow = "1.0"
env_logger = "0.11"
EOF

# 添加到workspace
if ! grep -q "examples/pumpfun_buy_test" Cargo.toml; then
    sed -i.bak '/members = \[/a\
    "examples/pumpfun_buy_test",
' Cargo.toml
    rm -f Cargo.toml.bak
fi

echo "================================"
echo "  开始编译和运行测试..."
echo "================================"
echo ""

# 编译并运行
cargo run --release -p pumpfun_buy_test

# 从workspace中移除
sed -i.bak '/examples\/pumpfun_buy_test/d' Cargo.toml
rm -f Cargo.toml.bak

# 清理
echo ""
echo "清理测试文件..."
rm -rf examples/pumpfun_buy_test

echo ""
echo "================================"
echo "  测试完成"
echo "================================"
