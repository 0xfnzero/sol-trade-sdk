# Pump Cashback 集成说明

本 SDK 已支持 [Pump Cashback Rewards](https://github.com/pump-fun/pump-public-docs/blob/main/docs/PUMP_CASHBACK_README.md)：在启用 cashback 的币种上交易时，用户可获得手续费返还而非支付给创作者。

## 行为概览

- **Bonding Curve (Pump)**
  - **Buy**：无需改指令，若币种启用 cashback 会自动累计。
  - **Sell**：当 `bonding_curve.is_cashback_coin == true` 时，SDK 会在指令中追加 `UserVolumeAccumulator` PDA（remaining account），用于累计可领取的 cashback。
- **Pump Swap**
  - **Buy**：当 `PumpSwapParams.is_cashback_coin == true` 时，会追加 UserVolumeAccumulator 的 **WSOL ATA** 作为 remaining account。
  - **Sell**：当 `is_cashback_coin == true` 时，会追加 **WSOL ATA**（0th）和 **UserVolumeAccumulator PDA**（1st）作为 remaining accounts。

`PumpFunParams` 通过 `from_mint_by_rpc` 拉取 bonding curve 时会解析链上 `is_cashback_coin`；`PumpSwapParams` 通过 `from_pool_address_by_rpc` / `from_mint_by_rpc` 拉取 pool 时会解析 `is_cashback_coin`。

## 领取 Cashback

### 推荐：使用 TradingClient 一键领取

若已有 `TradingClient`（例如用于买卖的同一个客户端），可直接调用以下方法，内部会完成构建交易、签名与发送：

```rust
// 领取 Pump 曲线产生的 Cashback（到账为 native SOL）
let sig = client.claim_cashback_pumpfun().await?;

// 领取 PumpSwap 产生的 Cashback（到账为 WSOL，自动确保用户 WSOL ATA 存在）
let sig = client.claim_cashback_pumpswap().await?;
```

- **`claim_cashback_pumpfun()`**：领取 Bonding Curve (Pump) 的返还，到账为钱包 SOL。
- **`claim_cashback_pumpswap()`**：领取 PumpSwap (AMM) 的返还，到账为用户的 WSOL ATA；若用户尚无 WSOL ATA 会先自动创建再领取。

### 仅构建指令（自行组交易时使用）

#### Bonding Curve (Pump)

将 native lamports 从 UserVolumeAccumulator 转到用户钱包：

```rust
use sol_trade_sdk::instruction::pumpfun;

let ix = pumpfun::claim_cashback_pumpfun_instruction(&payer.pubkey());
// 将 ix 放入交易并发送
```

#### Pump Swap (AMM)

将 WSOL 从 UserVolumeAccumulator 的 WSOL ATA 转到用户的 WSOL ATA。**调用前需确保用户 WSOL ATA 已存在**（或使用上面的 `claim_cashback_pumpswap()` 会自动处理）：

```rust
use sol_trade_sdk::instruction::pumpswap;
use sol_trade_sdk::constants::WSOL_TOKEN_ACCOUNT;
use sol_trade_sdk::constants::TOKEN_PROGRAM;

let ix = pumpswap::claim_cashback_pumpswap_instruction(
    &payer.pubkey(),
    WSOL_TOKEN_ACCOUNT,
    TOKEN_PROGRAM,
);
```

## 读取未领取金额

- **Pump (Bonding Curve)**：读 Pump 程序的 `UserVolumeAccumulator` PDA 的 lamports，减去维持账户所需的 rent-exempt 金额，即为未领取 cashback（lamports）。
- **Pump Swap**：读 Pump AMM 程序的 UserVolumeAccumulator 的 **WSOL ATA** 的 token balance，即为未领取 cashback（WSOL 数量）。

PDA 推导（本 SDK 已实现）：

- Pump：`instruction::utils::pumpfun::get_user_volume_accumulator_pda(user)`
- Pump AMM：`instruction::utils::pumpswap::get_user_volume_accumulator_pda(user)`，WSOL ATA：`instruction::utils::pumpswap::get_user_volume_accumulator_wsol_ata(user)`

## IDL 来源

根目录 `idl/` 下的 `pump.json`、`pump_amm.json`、`pump_fees.json` 从 [pump-fun/pump-public-docs](https://github.com/pump-fun/pump-public-docs) 的 `idl` 目录同步，便于与官方 IDL 对照和后续升级。
