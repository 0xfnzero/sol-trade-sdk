# SOL Trade CLI

一个用于在 Solana 上交易代币的命令行工具，支持多种 DEX 交易和钱包管理功能。

## 功能

此 CLI 工具支持以下操作模式：

### 🚀 **交易功能**

- **买入代币** - 使用 SOL 购买代币，支持多个 DEX
- **卖出代币** - 卖出代币换取 SOL，支持指定数量或全部卖出
- **多 DEX 支持** - pumpfun, pumpswap, bonk, raydium_v4, raydium_cpmm

### 💼 **钱包管理**

- **钱包状态** - 查看 SOL 和 WSOL 余额
- **SOL 包装** - 将 SOL 包装为 WSOL
- **WSOL 关闭** - 关闭 WSOL 账户并取回 SOL

### 🎛️ **使用模式**

- **交互式模式** - 持续的命令行界面
- **直接命令模式** - 单次命令执行

## 构建

```bash
cd examples/cli_trading
cargo build --release
```

## 使用方式

### 1. 交互式模式 (推荐)

直接运行程序进入交互模式：

```bash
cargo run
# 或者显式指定
cargo run -- interactive
```

然后在交互式命令行中输入命令：

```
sol-trade> help                                                             # 查看帮助
sol-trade> wallet                                                           # 查看钱包状态
sol-trade> buy xxxxxxxxxxxxxx pumpfun 1.0     # 用1.0 SOL买入
sol-trade> buy xxxxxxxxxxxxxx pumpfun 1.0 500 # 买入并设置500滑点
sol-trade> sell xxxxxxxxxxxxxx pumpfun        # 卖出所有代币
sol-trade> sell xxxxxxxxxxxxxx pumpfun 100.0  # 卖出100个代币
sol-trade> raydium_v4_buy <mint> <amm_address> 1.0                          # Raydium V4 买入
sol-trade> raydium_cpmm_buy <mint> <pool_address> 1.0                       # Raydium CPMM 买入
sol-trade> wrap_sol 2.5                                                     # 包装2.5 SOL
sol-trade> close_wsol                                                       # 关闭WSOL账户
sol-trade> quit                                                             # 退出
```

### 2. 直接命令行模式 ⭐️ **新功能**

现在支持直接通过命令行参数执行单个命令，无需进入交互模式：

#### 📋 查看帮助

```bash
cargo run -- --help              # 总体帮助
cargo run -- buy --help          # 买入命令帮助
cargo run -- sell --help         # 卖出命令帮助
```

#### 💰 买入代币

```bash
# 基础买入
cargo run -- buy <mint_address> pumpfun --amount 1.0

# 买入并设置滑点
cargo run -- buy <mint_address> pumpfun --amount 1.0 --slippage 500

# Raydium V4 买入
cargo run -- buy <mint_address> raydium_v4 --amount 1.0 --amm <amm_address>

# Raydium CPMM 买入
cargo run -- buy <mint_address> raydium_cpmm --amount 1.0 --pool <pool_address>
```

#### 💸 卖出代币

```bash
# 卖出所有代币
cargo run -- sell <mint_address> pumpfun

# 卖出指定数量
cargo run -- sell <mint_address> pumpfun --amount 100.0

# 卖出并设置滑点
cargo run -- sell <mint_address> pumpfun --amount 100.0 --slippage 500

# Raydium V4 卖出
cargo run -- sell <mint_address> raydium_v4 --amm <amm_address>

# Raydium CPMM 卖出
cargo run -- sell <mint_address> raydium_cpmm --pool <pool_address>
```

#### 🔄 钱包操作

```bash
# 包装SOL为WSOL
cargo run -- wrap-sol --amount 1.0

# 关闭WSOL账户
cargo run -- close-wsol

# 查看钱包状态
cargo run -- wallet
```

## 支持的 DEX

| DEX              | 状态        | 买入 | 卖出 | 特殊参数           |
| ---------------- | ----------- | ---- | ---- | ------------------ |
| **PumpFun**      | ✅ 完全支持 | ✅   | ✅   | 无                 |
| **PumpSwap**     | ✅ 完全支持 | ✅   | ✅   | 无                 |
| **Bonk**         | ✅ 完全支持 | ✅   | ✅   | 无                 |
| **Raydium V4**   | ✅ 完全支持 | ✅   | ✅   | 需要 `--amm` 参数  |
| **Raydium CPMM** | ✅ 完全支持 | ✅   | ✅   | 需要 `--pool` 参数 |

## 功能状态

✅ **已完全实现的功能：**

- **多 DEX 交易** - 支持 PumpFun, PumpSwap, Bonk, Raydium V4, Raydium CPMM
- **买入/卖出** - 完整的代币交易功能（真实区块链交易）
- **钱包管理** - SOL 包装、WSOL 关闭、余额查询
- **双模式操作** - 交互式模式和直接命令行模式
- **参数验证** - 智能检查必需参数（如 Raydium 的 AMM/Pool 地址）
- **完整帮助系统** - 每个命令都有详细的帮助文档

## 快速开始示例

```bash
# 1. 查看帮助
cargo run -- --help

# 2. 检查钱包状态
cargo run -- wallet

# 3. 直接买入代币（PumpFun）
cargo run -- buy xxxxxxxxxxxxxx pumpfun --amount 0.1

# 4. 使用Raydium V4买入
cargo run -- buy <mint> raydium_v4 --amount 0.1 --amm <amm_address>

# 5. 卖出代币
cargo run -- sell xxxxxxxxxxxxxx pumpfun

# 6. 启动交互模式
cargo run
```

## 特性亮点

- 🎯 **智能参数验证** - 自动检查各 DEX 所需的特殊参数
- 🔄 **双模式操作** - 支持交互式和命令行两种使用方式
- 📊 **实时状态显示** - 钱包余额、交易状态实时更新
- 🛡️ **安全交易** - 所有交易都有完整的错误处理和确认
- 📝 **详细日志** - 每笔交易都会显示签名用于追踪

## 安全提醒

⚠️ **重要安全提醒：**

- 本工具进行**真实的区块链交易**
- 所有交易会消耗真实的 SOL 作为交易费用
- 确保钱包私钥安全，不要泄露给他人
- 交易是不可逆的，请谨慎操作
- 建议先用小额进行测试
