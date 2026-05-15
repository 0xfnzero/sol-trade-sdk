# IDL sources

Bitquery IDLs synced from `bitquery/solana-idl-lib` commit `f804b17`.

| SDK protocol | Root IDL | Bitquery source |
| --- | --- | --- |
| Bonk / Raydium Launchpad | `bonk.json` | `bitquery/raydium/launchpad.json` |
| Raydium CPMM | `raydium_cpmm.json` | `bitquery/raydium/raydium_cp.json` |
| Raydium AMM v4 | `raydium_amm_v4.json` | `bitquery/raydium/raydium_amm.json` |
| Meteora DAMM v2 | `meteora_damm_v2.json` | `bitquery/meteora/cp_amm_016.json` |
| PumpFun | `pump.json` | `bitquery/pumpfun/pump.json` |
| PumpSwap | `pump_amm.json` | `bitquery/pumpswap/amm.json` |

The root PumpFun/PumpSwap IDLs remain the newer local copies used for cashback and v2 instruction parity. The Bitquery snapshots are kept under `idl/bitquery/` for source comparison.
