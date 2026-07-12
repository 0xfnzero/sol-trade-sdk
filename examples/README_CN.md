# sol-trade-sdk 示例索引

[English](README.md)

请在仓库根目录使用 `cargo run --package <name>`。运行前先阅读对应示例的 README：很多协议示例包含占位私钥或地址，配置完成后会提交真实主网交易。

## 如何选择

| 分类 | 示例 |
|---|---|
| 入门 | `simple_trading`、`trading_client`、`shared_infrastructure` |
| 低延迟 | `pumpswap_trading`；同时阅读 `../docs/LOW_LATENCY_BOTS_CN.md` |
| PumpFun 事件流 | `pumpfun_sniper_trading`、`pumpfun_copy_trading` |
| 其他 DEX | `pumpswap_direct_trading`、`bonk_*`、`raydium_*`、`meteora_*` |
| 交易构造 | `address_lookup`、`nonce_cache`、`middleware_system`、`seed_trading` |
| 工具 | `gas_fee_strategy`、`wsol_wrapper`、`cli_trading` |

## 安全边界

- 含 `use_your_*` 或 `your_*_here` 的源码都是模板，运行前必须替换全部占位值。
- `simple_trading` 和 `gas_fee_strategy` 默认不会提交 swap；多数其他交易示例配置后会发真实交易。
- 若事件回调内仍创建客户端或同步查询 blockhash/状态，该示例只展示协议参数映射，不代表最终低延迟架构。
- 不要提交私钥。真实应用应使用环境变量或安全 keystore。
- 新接入优先使用 `SimpleBuyParams` / `SimpleSellParams`，只有必须控制低层账户 flag 时才使用低层参数。

每个示例目录都提供内容对应的英文和中文文档。
