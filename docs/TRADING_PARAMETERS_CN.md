# 📋 交易参数参考手册

本文档提供 Sol Trade SDK 中所有交易参数的完整参考说明。

## 📋 目录

- [SimpleBuyParams / SimpleSellParams](#simplebuyparams--simplesellparams)
- [TradeBuyParams](#tradebuyparams)
- [TradeSellParams](#tradesellparams)
- [参数分类](#参数分类)
- [重要说明](#重要说明)

## SimpleBuyParams / SimpleSellParams

新接入优先使用 `SimpleBuyParams` 和 `SimpleSellParams`。这两个结构体描述交易意图，SDK 内部会转换成低层 `TradeBuyParams` / `TradeSellParams`。

### SimpleBuyParams

| 参数 | 类型 | 必需 | 说明 |
|------|------|------|------|
| `dex_type` | `DexType` | ✅ | 使用哪个协议交易，例如 `DexType::PumpFun`。 |
| `pay_with` | `TradeTokenType` | ✅ | 买入时用什么 quote 支付。钱包实际花原生 SOL 就传 `SOL`。PumpFun V2 的 SOL/WSOL quote 池，如果你想用原生 SOL 结算，也仍然传 `SOL`。 |
| `mint` | `Pubkey` | ✅ | 要买入的 token mint。 |
| `amount` | `BuyAmount` | ✅ | 买入数量语义。选择一个枚举，不再组合多个低层数量字段。 |
| `extension_params` | `DexParamEnum` | ✅ | 协议状态参数，来自 parser/RPC 缓存，例如 `DexParamEnum::PumpFun(PumpFunParams::from_trade(...))`。 |
| `recent_blockhash` | `Hash` | ✅，使用 `new` 时 | 非 nonce 交易使用的 recent blockhash。SDK 不会在热路径临时获取。 |
| `gas_fee_strategy` | `GasFeeStrategy` | ✅ | CU price/limit 和 relay tip 配置。 |
| `slippage_basis_points` | `Option<u64>` | ❌ | 可选滑点覆盖。`100` 表示 1%。 |
| `account_policy` | `AccountPolicy` | ❌ | ATA 创建/关闭策略。默认 `Auto`。 |
| `address_lookup_table_account` | `Option<AddressLookupTableAccount>` | ❌ | 可选 ALT，用于减少交易体积。 |
| `wait_tx_confirmed` | `bool` | ❌ | 是否等链上确认后再返回。默认 `false`。 |
| `wait_for_all_submits` | `bool` | ❌ | fast-submit 模式下，是否等待所有 SWQoS 通道返回并拿到全部签名。 |
| `durable_nonce` | `Option<DurableNonceInfo>` | ❌ | durable nonce 信息。使用 `SimpleBuyParams::with_durable_nonce(...)` 设置，不要和 `recent_blockhash` 混用。 |
| `simulate` | `bool` | ❌ | 只构建并模拟交易，不提交。默认 `false`。 |
| `grpc_recv_us` | `Option<i64>` | ❌ | 上游收到事件的微秒时间戳，用于延迟追踪。 |

### SimpleSellParams

| 参数 | 类型 | 必需 | 说明 |
|------|------|------|------|
| `dex_type` | `DexType` | ✅ | 使用哪个协议交易，例如 `DexType::PumpFun`。 |
| `receive_as` | `TradeTokenType` | ✅ | 卖出后接收什么 quote。想收原生 SOL 就传 `SOL`。 |
| `mint` | `Pubkey` | ✅ | 要卖出的 token mint。 |
| `amount` | `SellAmount` | ✅ | 卖出数量语义。 |
| `extension_params` | `DexParamEnum` | ✅ | 协议状态参数，来自 parser/RPC 缓存。 |
| `recent_blockhash` | `Hash` | ✅，使用 `new` 时 | 非 nonce 交易使用的 recent blockhash。 |
| `gas_fee_strategy` | `GasFeeStrategy` | ✅ | CU price/limit 和 relay tip 配置。 |
| `slippage_basis_points` | `Option<u64>` | ❌ | 可选滑点覆盖。`100` 表示 1%。 |
| `account_policy` | `AccountPolicy` | ❌ | ATA 创建/关闭策略。默认 `Auto`。 |
| `address_lookup_table_account` | `Option<AddressLookupTableAccount>` | ❌ | 可选 ALT，用于减少交易体积。 |
| `wait_tx_confirmed` | `bool` | ❌ | 是否等链上确认后再返回。默认 `false`。 |
| `wait_for_all_submits` | `bool` | ❌ | fast-submit 模式下，是否等待所有 SWQoS 通道返回并拿到全部签名。 |
| `durable_nonce` | `Option<DurableNonceInfo>` | ❌ | durable nonce 信息。使用 `SimpleSellParams::with_durable_nonce(...)` 设置，不要和 `recent_blockhash` 混用。 |
| `simulate` | `bool` | ❌ | 只构建并模拟交易，不提交。默认 `false`。 |
| `with_tip` | `bool` | ❌ | 卖出交易是否带 relay tip。默认 `true`，可通过 `.with_tip(false)` 关闭。 |
| `grpc_recv_us` | `Option<i64>` | ❌ | 上游收到事件的微秒时间戳，用于延迟追踪。 |

### 数量如何选择

| 枚举 | 含义 | 底层映射 |
|------|------|----------|
| `BuyAmount::ExactInput(amount)` | 精确花费指定 quote 数量，滑点保护最小买到 token 数量。 | `input_token_amount = amount`，`use_exact_sol_amount = Some(true)` |
| `BuyAmount::WithMaxInput { quote_amount }` | 常规 PumpFun/PumpSwap buy。SDK 估算输出，并把滑点作用在最大 quote 成本上。 | `input_token_amount = quote_amount`，`use_exact_sol_amount = Some(false)` |
| `BuyAmount::ExactOutput { output_amount, max_input_amount }` | 精确买到指定 token 数量，并限制最多花多少 quote。 | `fixed_output_token_amount = Some(output_amount)`，`input_token_amount = max_input_amount` |
| `SellAmount::ExactInput(amount)` | 精确卖出指定 token 数量，滑点保护最少收到 quote 数量。 | `input_token_amount = amount` |
| `SellAmount::ExactOutput { output_amount, max_input_amount }` | 精确收到指定 quote 数量，并限制最多卖出多少 token；取决于 DEX 是否支持。 | `fixed_output_token_amount = Some(output_amount)`，`input_token_amount = max_input_amount` |

### AccountPolicy

| 枚举 | 行为 | 适用场景 |
|------|------|----------|
| `Auto` | SDK 按实际路径创建必要 ATA。买入会创建目标 token ATA；卖出接收非 SOL 时会创建输出 ATA。 | 普通应用、手动交易工具。 |
| `HotPathMinimal` | 交易内不创建/关闭 ATA。 | Bot、狙击、套利、对交易体积敏感的路径。 |
| `CreateMissing` | 尽量在交易内创建缺失 ATA。 | 更重视方便，不追求最小交易体积。 |
| `AssumePrepared` | 不创建也不关闭 token account，调用方保证都已准备好。 | 高级确定性流程。 |

## TradeBuyParams

`TradeBuyParams` 是高级低层买入 API。新接入建议优先使用 `SimpleBuyParams`，只有需要直接控制单个 ATA flag 时再使用它。

### 基础交易参数

| 参数 | 类型 | 必需 | 描述 |
|------|------|------|------|
| `dex_type` | `DexType` | ✅ | 要使用的交易协议 (PumpFun, PumpSwap, Bonk, RaydiumCpmm, RaydiumAmmV4, MeteoraDammV2) |
| `input_token_type` | `TradeTokenType` | ✅ | 要使用的输入代币类型 (SOL, WSOL, USD1) |
| `mint` | `Pubkey` | ✅ | 要购买的代币 mint 公钥 |
| `input_token_amount` | `u64` | ✅ | 要花费的输入代币数量（最小代币单位） |
| `slippage_basis_points` | `Option<u64>` | ❌ | 滑点容忍度（基点单位，例如 100 = 1%, 500 = 5%） |
| `recent_blockhash` | `Option<Hash>` | ❌ | 用于交易有效性的最新区块哈希 |
| `extension_params` | `Box<dyn ProtocolParams>` | ✅ | 协议特定参数 (PumpFunParams, PumpSwapParams 等) |

### 高级配置参数

| 参数 | 类型 | 必需 | 描述 |
|------|------|------|------|
| `address_lookup_table_account` | `Option<Pubkey>` | ❌ | 用于交易优化的地址查找表 |
| `wait_tx_confirmed` | `bool` | ✅ | 是否等待交易确认 |
| `create_input_token_ata` | `bool` | ✅ | 是否创建输入代币关联代币账户 |
| `close_input_token_ata` | `bool` | ✅ | 交易后是否关闭输入代币 ATA |
| `create_mint_ata` | `bool` | ✅ | 是否创建代币 mint ATA |
| `durable_nonce` | `Option<DurableNonceInfo>` | ❌ | 持久 nonce 信息，包含 nonce 账户和当前 nonce 值 |
| `fixed_output_token_amount` | `Option<u64>` | ❌ | 可选的固定输出代币数量。对于支持 exact-out 的 DEX，会使用 exact-out 指令，并将 input_token_amount 作为最大输入预算（Meteora DAMM V2 必需） |
| `gas_fee_strategy` | `GasFeeStrategy` | ✅ | Gas fee 策略实例，用于控制交易费用和优先级 |
| `simulate` | `bool` | ✅ | 是否模拟交易而不实际执行。当为 true 时，将通过 RPC 模拟交易以验证并显示详细日志、计算单元消耗和潜在错误，而不会实际提交到区块链 |


## TradeSellParams

`TradeSellParams` 结构体包含在不同 DEX 协议上执行卖出订单所需的所有参数。

### 基础交易参数

| 参数 | 类型 | 必需 | 描述 |
|------|------|------|------|
| `dex_type` | `DexType` | ✅ | 要使用的交易协议 (PumpFun, PumpSwap, Bonk, RaydiumCpmm, RaydiumAmmV4, MeteoraDammV2) |
| `output_token_type` | `TradeTokenType` | ✅ | 要接收的输出代币类型 (SOL, WSOL, USD1) |
| `mint` | `Pubkey` | ✅ | 要出售的代币 mint 公钥 |
| `input_token_amount` | `u64` | ✅ | 要出售的代币数量（最小代币单位） |
| `slippage_basis_points` | `Option<u64>` | ❌ | 滑点容忍度（基点单位，例如 100 = 1%, 500 = 5%） |
| `recent_blockhash` | `Option<Hash>` | ❌ | 用于交易有效性的最新区块哈希 |
| `with_tip` | `bool` | ✅ | 交易中是否包含小费 |
| `extension_params` | `Box<dyn ProtocolParams>` | ✅ | 协议特定参数 (PumpFunParams, PumpSwapParams 等) |

### 高级配置参数

| 参数 | 类型 | 必需 | 描述 |
|------|------|------|------|
| `address_lookup_table_account` | `Option<AddressLookupTableAccount>` | ❌ | 用于交易优化的地址查找表 |
| `wait_tx_confirmed` | `bool` | ✅ | 是否等待交易确认 |
| `create_output_token_ata` | `bool` | ✅ | 是否创建输出代币关联代币账户 |
| `close_output_token_ata` | `bool` | ✅ | 交易后是否关闭输出代币 ATA |
| `durable_nonce` | `Option<DurableNonceInfo>` | ❌ | 持久 nonce 信息，包含 nonce 账户和当前 nonce 值 |
| `gas_fee_strategy` | `GasFeeStrategy` | ✅ | Gas fee 策略实例，用于控制交易费用和优先级 |
| `fixed_output_token_amount` | `Option<u64>` | ❌ | 可选的固定输出代币数量。对于支持 exact-out 的 DEX，会使用 exact-out 指令，并将 input_token_amount 作为最大输入预算（Meteora DAMM V2 必需） |
| `simulate` | `bool` | ✅ | 是否模拟交易而不实际执行。当为 true 时，将通过 RPC 模拟交易以验证并显示详细日志、计算单元消耗和潜在错误，而不会实际提交到区块链 |


## 参数分类

### 🎯 核心交易参数

这些参数对于定义基本交易操作至关重要：

- **dex_type**: 确定用于交易的协议
- **input_token_type** (买入) / **output_token_type** (卖出): 指定基础代币类型 (SOL, WSOL, USD1)
- **mint**: 指定要交易的代币
- **input_token_amount**: 定义交易规模（买入和卖出操作都使用此参数）
- **recent_blockhash**: 确保交易有效性

### ⚙️ 交易控制参数

这些参数控制交易的处理方式：

- **slippage_basis_points**: 控制可接受的价格滑点
- **wait_tx_confirmed**: 控制是否等待确认

### 🔧 账户管理参数

这些参数控制自动账户创建和管理：

- **create_input_token_ata** (买入) / **create_output_token_ata** (卖出): 自动为输入/输出代币创建代币账户
- **close_input_token_ata** (买入) / **close_output_token_ata** (卖出): 交易后自动关闭代币账户
- **create_mint_ata**: 自动为交易代币创建代币账户

### 🚀 优化参数

这些参数启用高级优化：

- **address_lookup_table_account**: 使用地址查找表减少交易大小

### 🔄 代币类型参数

**TradeTokenType** 枚举支持以下基础代币：
- **SOL**: Solana 原生代币（通常与 PumpFun 协议一起使用）
- **WSOL**: 包装 SOL 代币（通常与 PumpSwap、Bonk、Raydium 协议一起使用）
- **USD1**: USD1 稳定币（目前仅在 Bonk 协议上支持）

### 🔄 非必填参数

当你需要使用 durable nonce 时，需要填入这个参数：
- **durable_nonce**: 持久 nonce 信息，包含 nonce 账户和当前 nonce 值

## 重要说明

### 🌱 Seed 优化

Seed 优化现在在创建 `SolanaTrade` 实例时通过 `TradeConfig` 全局配置：

```rust
// 全局启用 seed 优化（默认: true）
let trade_config = TradeConfig::new(rpc_url, swqos_configs, commitment)
    .with_wsol_ata_config(
        true,  // create_wsol_ata_on_startup: 启动时检查并创建 WSOL ATA（默认: true）
        true   // use_seed_optimize: 为所有 ATA 操作启用 seed 优化（默认: true）
    );
```

当 seed 优化启用时：
- ⚠️ **警告**: 使用 seed 优化购买的代币必须通过此 SDK 出售
- ⚠️ **警告**: 官方平台的出售方法可能会失败
- 📝 **注意**: 使用 `get_associated_token_address_with_program_id_fast_use_seed` 获取 ATA 地址

### 💰 代币账户管理

账户管理参数提供精细控制：

- **独立控制**: 创建和关闭操作可以分别控制
- **批量操作**: 创建一次，多次交易，然后关闭
- **租金优化**: 关闭账户时自动回收租金

### 🔍 地址查找表

使用 `address_lookup_table_account` 之前：
- 查找表减少交易大小并提高成功率
- 对于有许多账户引用的复杂交易特别有益

### 📊 滑点配置

推荐的滑点设置：
- **保守**: 100-300 基点 (1-3%)
- **中等**: 300-500 基点 (3-5%)
- **激进**: 500-1000 基点 (5-10%)

### 🎯 协议特定参数

每个 DEX 协议需要特定的 `extension_params`：
- **PumpFun**: `PumpFunParams`
- **PumpSwap**: `PumpSwapParams`
- **Bonk**: `BonkParams`
- **Raydium CPMM**: `RaydiumCpmmParams`
- **Raydium AMM V4**: `RaydiumAmmV4Params`
- **Meteora DAMM V2**: `MeteoraDammV2Params`

请参阅相应的协议文档了解详细的参数规格。

### 🔍 交易模拟

当 `simulate: true` 时：
- **不提交区块链**: 交易不会实际提交到区块链
- **验证功能**: 验证交易构建和执行，而不会消耗实际代币
- **详细输出**: 显示全面的信息，包括：
  - 带有详细执行步骤的交易日志
  - 计算单元消耗（用于优化 CU 预算）
  - 潜在错误和失败原因
  - 用于调试的内部指令
- **使用场景**:
  - 在真实执行前测试交易逻辑
  - 调试失败的交易
  - 估算计算单元消耗
  - 验证交易参数
- 📝 **注意**: 模拟使用 RPC 的 `simulateTransaction` 方法，采用 processed 承诺级别
