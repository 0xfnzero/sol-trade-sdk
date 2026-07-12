# SOL / WSOL Operations

[中文](README_CN.md)

Demonstrates wrapping SOL, partially unwrapping WSOL through the SDK seed account, and closing the WSOL account to recover the remaining SOL.

> All three steps submit real transactions. Set `PRIVATE_KEY` and RPC, use a test wallet first, and reduce amounts as appropriate.

```bash
cargo run --package wsol_wrapper
```

Closing WSOL unwraps the account's full remaining balance and removes the account. Do not run that step when another workflow expects the same WSOL account to remain available.
