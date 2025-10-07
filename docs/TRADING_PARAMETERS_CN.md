# 📋 交易参数参考手册

本文档提供 Sol Trade SDK 中所有交易参数的完整参考说明。

## 📋 目录

- [TradeBuyParams](#tradebuyparams)
- [TradeSellParams](#tradesellparams)
- [参数分类](#参数分类)
- [重要说明](#重要说明)

## TradeBuyParams

`TradeBuyParams` 结构体包含在不同 DEX 协议上执行买入订单所需的所有参数。

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
| `wait_transaction_confirmed` | `bool` | ✅ | 是否等待交易确认 |
| `create_input_token_ata` | `bool` | ✅ | 是否创建输入代币关联代币账户 |
| `close_input_token_ata` | `bool` | ✅ | 交易后是否关闭输入代币 ATA |
| `create_mint_ata` | `bool` | ✅ | 是否创建代币 mint ATA |
| `open_seed_optimize` | `bool` | ✅ | 是否使用 seed 优化以减少 CU 消耗 |
| `durable_nonce` | `Option<DurableNonceInfo>` | ❌ | 持久 nonce 信息，包含 nonce 账户和当前 nonce 值 |
| `fixed_output_token_amount` | `Option<u64>` | ❌ | 可选的固定输出代币数量。如果设置，此值将直接分配给输出数量而不是通过计算得出（Meteora DAMM V2 必需） |
| `gas_fee_strategy` | `GasFeeStrategy` | ✅ | Gas fee 策略实例，用于控制交易费用和优先级 |


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
| `wait_transaction_confirmed` | `bool` | ✅ | 是否等待交易确认 |
| `create_output_token_ata` | `bool` | ✅ | 是否创建输出代币关联代币账户 |
| `close_output_token_ata` | `bool` | ✅ | 交易后是否关闭输出代币 ATA |
| `open_seed_optimize` | `bool` | ✅ | 是否使用 seed 优化以减少 CU 消耗 |
| `durable_nonce` | `Option<DurableNonceInfo>` | ❌ | 持久 nonce 信息，包含 nonce 账户和当前 nonce 值 |
| `gas_fee_strategy` | `GasFeeStrategy` | ✅ | Gas fee 策略实例，用于控制交易费用和优先级 |
| `fixed_output_token_amount` | `Option<u64>` | ❌ | 可选的固定输出代币数量。如果设置，此值将直接分配给输出数量而不是通过计算得出（Meteora DAMM V2 必需） |


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
- **wait_transaction_confirmed**: 控制是否等待确认

### 🔧 账户管理参数

这些参数控制自动账户创建和管理：

- **create_input_token_ata** (买入) / **create_output_token_ata** (卖出): 自动为输入/输出代币创建代币账户
- **close_input_token_ata** (买入) / **close_output_token_ata** (卖出): 交易后自动关闭代币账户
- **create_mint_ata**: 自动为交易代币创建代币账户

### 🚀 优化参数

这些参数启用高级优化：

- **address_lookup_table_account**: 使用地址查找表减少交易大小
- **open_seed_optimize**: 使用基于 seed 的账户创建以降低 CU 消耗

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

当 `open_seed_optimize: true` 时：
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
