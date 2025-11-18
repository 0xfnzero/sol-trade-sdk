# Node1 最小小费金额限制

## 概述

Node1 节点要求最小小费金额为 **0.002 SOL**，低于此金额的交易可能会被拒绝。

**重要**：这个限制**仅对 Node1** 生效，其他 swqos（Jito、BlockRazor、Astralane 等）不受影响。

sol-trade-sdk 已自动添加智能检测，只对 Node1 的 tip_account 应用最小小费金额检查。

## 实现位置

**文件**: `sol-trade-sdk/src/trading/common/transaction_builder.rs`

**修改内容**:
```rust
// Add tip transfer instruction
if with_tip && tip_amount > 0.0 {
    // 🔧 Node1 最小小费金额限制：0.002 SOL（仅限 Node1）
    const MIN_TIP_AMOUNT: f64 = 0.002;

    // 检查是否是 Node1 的 tip_account
    let is_node1 = NODE1_TIP_ACCOUNTS.iter().any(|&account| account == *tip_account);

    let actual_tip_amount = if is_node1 && tip_amount < MIN_TIP_AMOUNT {
        // Node1 要求最小 0.002 SOL
        MIN_TIP_AMOUNT
    } else {
        // 其他 swqos 使用原始金额
        tip_amount
    };

    instructions.push(transfer(
        &payer.pubkey(),
        tip_account,
        sol_str_to_lamports(actual_tip_amount.to_string().as_str()).unwrap_or(0),
    ));
}
```

## 工作原理

1. **识别 Node1**: 检查 tip_account 是否属于 Node1 的账户列表
2. **条件检查**: 只对 Node1 且 `tip_amount < 0.002 SOL` 时才调整
3. **自动调整**: Node1 的小费金额提升到 `0.002 SOL`
4. **其他保持**: 其他 swqos 使用原始配置的小费金额
5. **透明处理**: 对上层调用者透明，无需修改配置

## 示例

### Node1: 配置的小费 < 0.002 SOL
```yaml
# config/app.prod.yaml
trading:
  gas_fee:
    global_buy_tip: 0.001  # 0.001 SOL（低于 Node1 最小值）
```

**Node1 实际执行**: 自动调整为 `0.002 SOL`
**其他 swqos 实际执行**: 保持 `0.001 SOL`（不调整）

### Node1: 配置的小费 >= 0.002 SOL
```yaml
# config/app.prod.yaml
trading:
  gas_fee:
    global_buy_tip: 0.005  # 0.005 SOL（高于 Node1 最小值）
```

**Node1 实际执行**: 使用 `0.005 SOL`（不调整）
**其他 swqos 实际执行**: 使用 `0.005 SOL`（不调整）

## 影响范围

这个修改**只影响使用 Node1** 的交易：

- ✅ Node1 买入交易（小费 < 0.002 时自动调整）
- ✅ Node1 卖出交易（小费 < 0.002 时自动调整）
- ❌ 其他 swqos 交易（保持原始小费金额）

## 日志示例

当小费金额被自动调整时，交易仍会正常执行，不会有额外日志输出。

这是因为调整是在 SDK 内部完成的，对调用者完全透明。

## 注意事项

1. **最小值固定**: `MIN_TIP_AMOUNT = 0.002 SOL` 是硬编码的常量
2. **只增不减**: 只会向上调整小费，不会减少配置的小费金额
3. **Node1 专用**: 这个限制是 Node1 节点的要求

## 如果需要修改最小值

如果将来 Node1 调整最小小费要求，只需修改：

```rust
// 修改这个常量即可
const MIN_TIP_AMOUNT: f64 = 0.002;  // 改为新的最小值
```

**文件位置**: `sol-trade-sdk/src/trading/common/transaction_builder.rs:49`

## 相关配置

主项目配置文件中的小费设置：

```yaml
# config/app.prod.yaml
trading:
  gas_fee:
    global_buy_tip: 0.001    # 买入小费（Node1 会自动调整为 0.002）
    global_sell_tip: 0.0001  # 卖出小费（Node1 会自动调整为 0.002）
```

**建议**:
- 如果主要使用 Node1，可以直接配置为 `>= 0.002 SOL`
- 如果混合使用多个 swqos，配置任意值即可，SDK 会智能处理
