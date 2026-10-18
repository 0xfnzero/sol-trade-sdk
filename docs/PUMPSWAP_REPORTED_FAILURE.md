# PumpSwap 3007 失败修复与跨语言一致性

用户提供的失败交易：
[34mNYS59…Gbkaodbe](https://solscan.io/tx/34mNYS59JLuy6q55gS1PbYY2DN9v1mtkuCMhdoXKRaiB5WczkfweVAhuTC7dVyEpLzg94T4HvNd5Eu75Gbkaodbe)。
第 6 条外层指令是 `Buy`，错误为 `coin_creator_vault_ata` / 3007。
其账户位置 17、18 都是 `11111111111111111111111111111111`，即默认公钥／System Program；
该程序地址不能充当 creator 的 SPL Token 账户。

该交易的实际币对为 `dev/null / WSOL`，base mint 是
`2kuY5DHqRvYKfytfLfrey1oB1Z1J3voq343FWMp6pump`（Token-2022），quote 是经典 SPL WSOL，
pool 是 `BwTKRFjssVL6c7Wb8MjvHMy9BET1Jb3xSKXcxz8t4RH`。
这笔交易不能证明缺少 SOL→TTWO 转换，也不能确定哪个调用方或 SDK 版本漏填了账户。

## 参数准备

从经过验证的订阅缓存或完整池快照补齐 creator、mint、token program、vault 和费用配置。
creator authority 必须由实际 `coin_creator` 派生；creator ATA 必须使用该 authority、
**实际 quote mint 和 quote token program** 派生，不能固定使用 WSOL 或经典 Token program。
构建器在生成指令前拒绝缺失／错误的 creator 账户、两侧相同的 vault 和显式零地址收款方。
这属于本地参数校验，不代替链上账户存在性、余额或快照新鲜度验证。

两个方向的 mint 要一起提供。quote→base 生成 Buy，base→quote 生成 Sell，
方向由币对决定；两个 mint 都省略时保留低层旧接口的默认方向。
任意 quote 池可直接使用该 quote 代币交易；以 SOL 支付其它 quote 的池，需提供完整转换路径。
现有缓存路径支持准备、报价和签名，支持的 Token-2022 扩展仍以各路径的检查为准。

## 收款方与账户权限

显式提供的协议费与 buyback 收款方优先于静态／随机回退，包括 mayhem 模式。
调用方应从当前 GlobalConfig 中选择对应模式允许的收款方；非零校验不会证明其当前有效性。
收款 ATA 同样使用实际 quote mint/program 派生。

`Buy` 与 `buy_exact_quote_in` 的 `global_volume_accumulator`（账户位置 19）
按 Pump AMM IDL 标记为只读，直接构建和缓存构建保持一致，避免不必要的全局写锁。
用户成交量账户仍按 IDL 标记为可写。

## 回归证据与边界

四种语言使用字节相同的 `pumpswap_reported_failure.json`：原失败指令加上后来单独抓取的
账户快照，**不是失败 slot 的历史 bank**。测试重建原 Buy payload、拒绝原默认账户，
并验证普通／mayhem 模式的指定收款方。

设置 `PUMPSWAP_FIXTURE_EXPORT_DIR` 后，专项测试导出相同公开测试 payer、
1,000,000 base 原始单位输出、最大 1,000,000 quote 原始单位输入的固定输出 Buy。
工作区 `artifacts/pumpswap-non-wsol-20261018/cross-language/compare_instructions.py`
比较四种原生构建结果的 payload、全部账户顺序、签名／写权限，并核对 IDL 权限。
导出的只是用于离线比对的业务指令，不是完整签名交易。

TTWO 直接买卖与 SOL→USDC→TTWO→目标币及反向路线另有缓存／本地 SBF bank 回归。
这些回归使用历史公共状态与本地虚拟钱包，不代表当前主网成交保证；本次验证不广播交易。

## Rust 接口与复验

使用 `PumpSwapParams::with_fee_recipients(protocol, buyback)` 或对应的两个
`protocol_*_fee_recipient_override` 字段设置收款方。
完整参数可由已验证缓存或 `PumpSwapParams::from_pool_address_by_rpc` 构造。

```sh
cargo test -p sol-trade-sdk --lib pumpswap_reported --features parser-adapter --locked
```

需要当前状态只读对照模拟时，显式运行默认忽略的 `pumpswap_reported_3007_mainnet` 测试。
它使用新报价和虚拟资金，不重放原 nonce 或历史价格。
