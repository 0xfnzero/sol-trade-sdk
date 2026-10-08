# IDL Sources

Unversioned IDLs track current protocol-maintained files. Explicitly versioned files such as
`_011`, `_015`, or `_090` remain compatibility snapshots. Canonical SHA-256 values use
`jq -S -c . | sha256sum`, so JSON formatting does not affect comparisons.

| Local IDL | Official upstream | Version | Canonical SHA-256 |
| --- | --- | --- | --- |
| `pump.json` | `pump-fun/pump-public-docs/idl/pump.json` | 0.1.0 | `f5863f6c1ee16e67954d6980abe1e4f35356a3c53514afe31ed26aa60b8983e0` |
| `pump_amm.json` | `pump-fun/pump-public-docs/idl/pump_amm.json` | 0.1.0 | `3b3e42f13901695c496492a9bb477c3e1916a757ccd14aaa9b0df94255cd1a31` |
| `pump_fees.json` | `pump-fun/pump-public-docs/idl/pump_fees.json` | 0.1.0 | `b0c160cbc12b6c14d5b6f0cfddc1237db1694c84f505fb6688aaa233bed82bc8` |
| `raydium_clmm.json` | `raydium-io/raydium-idl/raydium_clmm/raydium_clmm.json` | 0.1.0 | `ce860422ec1e3284e89e165d9740c9f19f861ab45e0ad5bfc7ca647b6af39496` |
| `raydium_cpmm.json` | `raydium-io/raydium-idl/raydium_cpmm/raydium_cp_swap.json` | 0.2.0 | `26548a2a32fda5b53329b3f79288e46e9a95df7db64d9eb0d4ebc68eafaab82b` |
| `raydium_launchpad.json` | `raydium-io/raydium-idl/raydium_launchpad/raydium_launchpad.json` | 0.2.0 | `03713b809cd62272f63ebf5ca5ad06a64f6f7a3ac493a9f49d2c3b2aa43aca68` |
| `meteora_lb_clmm.json` | `MeteoraAg/dlmm-sdk/idls/dlmm.json` | 0.12.0 | `1bc4333e5702dddb51d9ad92b6e9298940c9d9ff7f92fd761b634a03cf2d7daf` |
| `meteora_damm_v2.json` | `MeteoraAg/damm-v2-sdk/src/idl/cp_amm.json` | 0.2.5 | `ae28c5c71a9f054384c356e65d8d6e7b2675dc66c03b0b12ea3429eefb6dbe4e` |
| `meteora_dynamic_bonding_curve.json` | `MeteoraAg/dynamic-bonding-curve-sdk/packages/dynamic-bonding-curve/src/idl/dynamic-bonding-curve/idl.json` | 0.2.1 | `f9bc73acb25f6a873778146642cf2b68ab731806712d10498b3505ed87cc425f` |
| `meteora_amm.json` | `MeteoraAg/dynamic-bonding-curve/idls/dynamic_amm.json` | 0.5.2 | `21f59f98c8a593ac2a9d976889cff8d8b618fe3aa1fb5c0132d5ca598c59a833` |
| `orca_whirlpool.json` / `orca_whirlpool_v2.json` | `@orca-so/whirlpools-sdk@0.22.0/dist/artifacts/whirlpool.json` | 0.9.0 | `301335733544288d52aae316827c4e2bbf4e27edbd3d87bd1711de1251e3b1ee` |

Raydium AMM V4 is not an Anchor program and has no current protocol-maintained Anchor IDL.
`raydium_amm_v4.json` and `raydium_pool_v4.json` remain compatibility references; current
instruction and `ray_log` layouts are defined by `raydium-io/raydium-amm`.

Checked 2026-10-08. Raydium CPMM includes creator-fee-share definitions verified against
`raydium-io/raydium-cp-swap@b3187ae53a1b95a201f855a59024a12ca8f5b51a`; its aggregate
IDL repository omits these newer definitions and must not replace this snapshot.
