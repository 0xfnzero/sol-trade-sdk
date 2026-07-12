# sol-trade-sdk Examples

[中文](README_CN.md)

Run commands from the repository root with `cargo run --package <name>`. Read the example's README before running it: many protocol examples contain placeholder keys or addresses and submit real mainnet transactions after configuration.

## Choose an example

| Category | Examples |
|---|---|
| Start here | `simple_trading`, `trading_client`, `shared_infrastructure` |
| Low latency | `pumpswap_trading`; also read `../docs/LOW_LATENCY_BOTS.md` |
| PumpFun streams | `pumpfun_sniper_trading`, `pumpfun_copy_trading` |
| Other DEXs | `pumpswap_direct_trading`, `bonk_*`, `raydium_*`, `meteora_*` |
| Transaction construction | `address_lookup`, `nonce_cache`, `middleware_system`, `seed_trading` |
| Utilities | `gas_fee_strategy`, `wsol_wrapper`, `cli_trading` |

## Safety boundary

- Files containing `use_your_*` or `your_*_here` are templates. Replace every placeholder before running.
- `simple_trading` and `gas_fee_strategy` do not submit a swap as shipped. Most other trading examples can.
- Event-driven templates that create the client or fetch blockhash/state inside the callback demonstrate protocol mapping, not the final low-latency architecture.
- Never commit private keys. Use environment variables or a secure keystore in real applications.
- Use `SimpleBuyParams` / `SimpleSellParams` for new integrations unless low-level account flags are specifically required.

Each example directory contains matching English and Chinese documentation.
