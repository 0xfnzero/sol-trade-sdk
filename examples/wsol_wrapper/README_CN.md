# SOL / WSOL 操作

[English](README.md)

展示包装 SOL、通过 SDK seed 账户部分解包 WSOL，以及关闭 WSOL 账户取回剩余 SOL。

> 三个步骤都会提交真实交易。运行前设置 `PRIVATE_KEY` 和 RPC，先使用测试钱包，并按需降低金额。

```bash
cargo run --package wsol_wrapper
```

关闭 WSOL 会解包账户中的全部剩余余额并删除该账户。如果其他流程仍依赖该 WSOL 账户，不要执行关闭步骤。
