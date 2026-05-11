# Pump.fun（Bonding Curve）常见链上错误与处理思路

本文档面向使用 **sol-trade-sdk** 组装 Pump.fun Program（`6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P`）买卖交易的集成方，汇总**实战中高频**的失败形态、日志特征，以及与 SDK 参数的对应关系和**推荐处理方式**。

> 自定义错误码以仓库内 `idl/pump.json`、`idl/pump_fees.json` 为准；**2006** 等部分错误来自 **Anchor 框架**，不在 Pump 自定义枚举里。

---

## 1. Anchor `2006` / `ConstraintSeeds`（`creator_vault`）

### 现象

- Solana Explorer / `simulateTransaction`：**Program Error: custom program error: 2006**，或 Anchor 文案 **A seeds constraint was violated**。
- 日志里常见于账户 **`creator_vault`**：打印 **Left**（你传入的 pubkey）与 **Right**（程序按当前 curve 推导的 PDA）。

### 含义

Pump 校验 `creator_vault` 必须满足程序的 **PDAs seeds**（与 bonding curve 上记录的 **creator / fee-sharing 布局**一致）。传入地址与程序期望不一致即失败。

### 常见成因

1. **`bonding_curve.creator` 与链上不符**：事件/缓存里用的是 create 交易的 `creator` 字段、`user`，或陈旧快照，与 curve 账户内真实 creator 不一致；据此推导或缓存的 vault 会错。
2. **买单侧「旧 vault」被沿用到卖单**：买入时 ix 里的 `creator_vault` 在后续 trade 语境下已不再与程序约束一致（例如 creator / sharing 语义在链上演进），卖单仍填旧地址 → **Left ≠ Right**。
3. **Creator Rewards / pump-fees 分成布局**：部分 mint 使用 `sharing-config` 相关 seeds，单靠 `PDA(["creator-vault"], bonding_curve.creator)` 不足以覆盖全部历史状态；Stale 的 offline hint 也会把 resolve 引向错误 vault。
4. **Phantom vault**：历史上若用 `Pubkey::default()` 推导出的占位 vault（SDK 常量 `phantom_default_creator_vault`），链上必定失败。

### SDK 侧处理思路

| 方向 | 做法 |
|------|------|
| **买入 / 卖出指令**（同一套解析） | 使用 `resolve_creator_vault_for_ix_with_fee_sharing`（`src/instruction/utils/pumpfun.rs`）：有 **非 default、非 phantom** 的 ix / 解析器回填 **`creator_vault` 时原样采用**（**不会**再根据 `creator` 做 `get_creator_vault_pda` 覆盖费分成等非传统布局）；**未传 ix vault** 时依次：`fee_sharing_creator_vault_if_active` hint → `PDA(effective_creator)`。`PumpFunInstructionBuilder` 买卖均走此逻辑。 |
| **权威对齐** | 低延迟路径外，可 **`PumpFunParams::from_mint_by_rpc`** 或由解析器 **`fill_trade_accounts`**（如 sol-parser-sdk）持续刷新 `creator` / `creator_vault`；必要时对 **fee-sharing** 使用 `fetch_fee_sharing_creator_vault_if_active` / `refresh_fee_sharing_creator_vault_from_rpc`。 |
| **bonding_curve 账户地址** | 指令构建时使用 **`get_bonding_curve_pda(mint)`** 作为 canonical bonding curve pubkey，避免缓存中的曲线地址错位导致读到错误 **creator**。 |

集成方若在 **bot** 侧维护持仓快照，建议在**每笔**解析到的 Pump trade 后刷新缓存/仓位中的 `creator` 与 `creator_vault`，避免「只写一次建仓快照」。

---

## 2. Pump `6000` `NotAuthorized`（常见：`feeRecipient`）

### 现象

- 日志：`AnchorError thrown in programs/pump/src/fee_recipient.rs` 或 **`Error Code: NotAuthorized`**（与 Global 授权的 fee recipient 池有关）。

### 含义

账户 **#2 fee recipient** 不是当前 Pump **Global / 协议**允许使用的收款地址之一，或与 **Mayhem / 非 Mayhem** 池不一致。

### 常见成因

1. 使用了**过期**或**错误池**的 fee recipient（静态列表落后于主网轮换）。
2. **`mayhem_mode` 与 `fee_recipient` 不匹配**：声明 Mayhem 却传普通池地址，或相反（见 `reconcile_mayhem_mode_for_trade`，`src/instruction/utils/pumpfun.rs`）。

### SDK 侧处理思路

| 方向 | 做法 |
|------|------|
| **优先事件** | 使用 gRPC / 解析器里的 **`tradeEvent.feeRecipient`** 或同笔 **create_v2 + buy** 观测到的 fee recipient。 |
| **纠偏** | `PumpFunParams::from_trade` 会对 `mayhem_mode` 与 `fee_recipient` 做池一致性纠偏；发单前若事件缺省，可走 `pump_fun_fee_recipient_meta`（按 `is_mayhem_mode` 从静态池选）。 |
| **提交前保留观测值** | 不要无条件把已观测的 `fee_recipient` 清成 default；否则会退回 SDK 内置静态池，静态池若落后于主网 Global 授权，仍可能触发 6000。只有缺少观测值时才让 builder 兜底。 |

---

## 3. SPL：Token **`initializeAccount3`**——`incorrect program id for instruction`

### 现象

- 内联指令里 **`Token Program: initializeAccount3`**（或 Token-2022 等价指令）报错 **`incorrect program id for instruction`**。

### 含义

为 **Mint** 创建用户 ATA 时，使用的 **token program（Legacy SPL vs Token-2022）** 与 **Mint 的实际 owner（`mint.owner`）** 不一致。

### 常见成因

1. Pump 新发多为 **Token-2022**，但代码写死 **`Tokenkeg…`**。
2. 少数 Legacy mint（`Tokenkeg…`），却被强制按 Token-2022 建账。

### SDK 侧处理思路

- **`PumpFunParams::token_program`** 必须与非 default 的 **mint owner** 一致；从 **gRPC / 解析结果** 带入，**勿**在已明确 program 时再用「仅按 `.pump` 后缀猜 Token-2022」覆盖（业务层若做后缀启发，应仅在 `token_program == default` 时生效）。

---

## 4. Pump `6020` `BuyZeroAmount`

### 现象

- `buy` / `buy_exact_sol_in` 报 **Buy zero amount**。

### 常见成因

- `min_tokens_out == 0`（或等价路径算出可买 **0 枚**），协议直接拒绝。
- 使用 **Create / Shred** 事件构造曲线时 **virtual / real 储备全 0**，本地定价算出 **0**。

### SDK 侧处理思路

- 对「首买 / 无储备」场景用 **`PumpFunParams::from_dev_trade`** 或按协议初值回填虚拟储备（与 `global_constants` 一致），再算 **`min_tokens_out`**。
- 适当 **放宽买入滑点**（`slippage_basis_points`），避免估算代币量略小于链上。

---

## 5. Pump `6042` `BuySlippageBelowMinTokensOut`

### 现象

- 文案：**Slippage: Would buy less tokens than expected min_tokens_out**。

### 含义

链上实际可成交代币数量 **小于** 指令参数 **`min_tokens_out`**。

### 常见成因

- 市价波动、SOL 竞价导致曲线状态与本地快照不一致。
- 本地 **`get_buy_token_amount_from_sol_amount`** 所用 **creator / 费率假设**与链上 **pfee CPI**（动态费率）不一致，**预估偏多**。

### SDK 侧处理思路

- **提高滑点容忍**（或降低 **`min_tokens_out`**）。
- 尽量用 **较新**的 **virtual/real reserves**（来自最近一次 trade 解析或简短 RPC）。
- 若运行在 **狙击手**等极端延迟场景，需接受：**保守的 min_out**（更大滑点）换成功率。

---

## 6. Pump `6024` `Overflow` 与其它算术类错误

### 现象

- `6024 Overflow`、`6025 Truncation`、`6026 DivisionByZero`（见 IDL）。

### 常见成因

- 指令参数 **`amount` / SOL / lamports** 与曲线状态组合不合法（例如极端大卖、或为 0 与后续计算冲突）。
- SDK 外传入了 **不合理的储备快照**。

### SDK 侧处理思路

- 校验 **买入/卖出金额 > 0**、与余额一致。
- 使用 **`from_mint_by_rpc`** 或与链一致的储备后再算 **`min_sol_output` / `min_tokens_out`**。

---

## 7. Pump `6027` `NotEnoughRemainingAccounts`（返现等）

### 现象

- 返现代币等路径要求 **remaining accounts**（例如 `UserVolumeAccumulator`），数量不足。

### 常见成因

- **`is_cashback_coin`（或等价标志）为 true**，但组装指令时 **未追加**所需账户。

### SDK 侧处理思路

- **`PumpFunParams::from_trade` / `from_dev_trade`** 传入正确的 **`is_cashback_coin`**（来自事件）。
- README 中与 **Cashback** 章节一致：**事件路径必须带标志**，不能默认 false。

---

## 8. Pump `6022` `SellZeroAmount`

### 含义

卖出代币数量为 **0**。在业务层过滤即可。

---

## 9. 与 pump-fees / Creator 迁移相关的错误（`6049`–`6053` 等）

IDL 中例如：

- **`6049` `CreatorMigratedToSharingConfig`**
- **`6050` `UnableToDistributeCreatorVaultMigratedToSharingConfig`**
- **`6053` `BondingCurveAndSharingConfigCreatorMismatch`**

### 思路

这些是 **creator / fee-sharing** 生命周期中的**专用分支**，与一般买卖路径不同。若仿真或清算类指令触发：

- 以 **Pump / pump-fees 官方文档** 为准使用 **`distribute_creator_fees`**、**reset_fee_sharing_config** 等；
- **`creator_vault` resolve** 需结合 **`fetch_fee_sharing_creator_vault_if_active`** 与链上 **`SharingConfig` 状态**，避免离线 deduce 过时。

---

## 10. 调试清单（推荐给集成方）

1. **记下失败指令索引** + **Simulation / explorer 展开的账户列表**，重点核对：**mint、bonding_curve、associated_bonding_curve、creator_vault、fee_recipient、token_program**。  
2. **对比 Anchor 日志里的 Left / Right**（针对 2006）与本地 `PumpFunParams` 打印是否一致。  
3. **`mint.owner`** 与 **`PumpFunParams.token_program`** 是否一致。  
4. **`bonding_curve` 地址**是否与 **`get_bonding_curve_pda(mint)`** 一致。  
5. **`mayhem_mode` ↔ `fee_recipient`** 是否同池。  
6. 低延迟不足以覆盖 **creator 演进** 时，是否在卖前引入了 **RPC 或最新 trade** 刷新。

---

## 参考代码入口（本仓库）

| 主题 | 路径 |
|------|------|
| Buy / Sell vault resolve（显式 ix `creator_vault` → `fee_sharing` hint → `PDA(effective_creator)`） | `src/instruction/utils/pumpfun.rs` — `resolve_creator_vault_for_ix_with_fee_sharing`；`effective_creator_for_trade` 见 `src/trading/core/params/pumpfun.rs` |
| Fee recipient / Mayhem | `src/instruction/utils/pumpfun.rs` — `pump_fun_fee_recipient_meta`, `reconcile_mayhem_mode_for_trade` |
| 指令构建 | `src/instruction/pumpfun.rs` — `PumpFunInstructionBuilder` |
| Params | `src/trading/core/params/pumpfun.rs` — `PumpFunParams::{from_trade, from_dev_trade, from_mint_by_rpc, refresh_fee_sharing_creator_vault_from_rpc}` |

---

如需英文版或对 PumpSwap／其它 DEX 的同类文档，可在 `docs/` 下按相同结构扩展。
