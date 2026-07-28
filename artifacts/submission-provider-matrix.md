# Submission Provider (SWQoS) Matrix

> Source: `src/swqos/mod.rs`, `src/constants/swqos.rs`
> Generated 2026-07-28

## Provider Inventory

| # | Provider | SDK Type | Min Tip (SOL) | Transport(s) | Blacklisted? | Tip Accounts |
|---|---|---|---|---|---|---|
| 1 | **Jito** | `SwqosType::Jito` | 0.00001 | HTTP | ❌ | 8 (jito-tip accounts) |
| 2 | **NextBlock** | `SwqosType::NextBlock` | 0.001 | HTTP | ✅ (disabled by default) | 8 (NextbLoCk... pattern) |
| 3 | **ZeroSlot** | `SwqosType::ZeroSlot` | 0.0001 | HTTP | ❌ | 5 |
| 4 | **Temporal** | `SwqosType::Temporal` | 0.0001 | HTTP | ❌ | — (Nozomi tip accounts) |
| 5 | **Bloxroute** | `SwqosType::Bloxroute` | 0.0001 | HTTP | ❌ | 4 |
| 6 | **Node1** | `SwqosType::Node1` | 0.0001 | HTTP, QUIC | ❌ | 6 (node1... pattern) |
| 7 | **FlashBlock** | `SwqosType::FlashBlock` | 0.0001 | HTTP | ❌ | 10 (FLaSh... pattern) |
| 8 | **BlockRazor** | `SwqosType::BlockRazor` | 0.0001 | gRPC (default), HTTP | ❌ | 12 |
| 9 | **Astralane** | `SwqosType::Astralane` | 0.00001 | Binary HTTP, Plain HTTP, QUIC | ❌ | 17 (astra... pattern) |
| 10 | **Stellium** | `SwqosType::Stellium` | 0.0001 | HTTP | ❌ | 5 (ste11... pattern) |
| 11 | **Lightspeed** (Solana Vibe Station) | `SwqosType::Lightspeed` | 0.0001 | HTTP (requires custom URL + API key) | ❌ | 2 |
| 12 | **Soyas** | `SwqosType::Soyas` | 0.001 | HTTP | ❌ | — |
| 13 | **Speedlanding** | `SwqosType::Speedlanding` | 0.001 | HTTP | ❌ | — |
| 14 | **Helius** | `SwqosType::Helius` | 0.0002 (normal) / 0.000005 (SWQoS-only) | HTTP | ❌ | 10 (HELIUS_TIP_ACCOUNTS) |
| 15 | **Solami** | `SwqosType::Solami` | 0.0001 | HTTP | ❌ | — |
| 16 | **LunarLander** (HelloMoon) | `SwqosType::LunarLander` | 0.001 | QUIC (default), HTTP | ❌ | — |
| 17 | **Glaive** | `SwqosType::Glaive` | 0.0001 | QUIC (default), HTTP | ❌ | — |
| 18 | **Default** (Solana RPC) | `SwqosType::Default` | 0.00001 | HTTP | ❌ | — |

**Total: 18 providers** (17 named + 1 Default/Solana RPC)

## Config Shapes

Each provider uses a specific `SwqosConfig` enum variant:

| Provider | Config Enum | Parameters |
|---|---|---|
| Jito | `SwqosConfig::Jito(uuid, region, custom_url)` | UUID auth token, region, optional custom URL |
| NextBlock | `SwqosConfig::NextBlock(api_token, region, custom_url)` | API token, region, optional custom URL |
| ZeroSlot | `SwqosConfig::ZeroSlot(api_token, region, custom_url)` | API token, region, optional custom URL |
| Temporal | `SwqosConfig::Temporal(api_token, region, custom_url)` | API token, region, optional custom URL |
| Bloxroute | `SwqosConfig::Bloxroute(api_token, region, custom_url)` | API token, region, optional custom URL |
| Node1 | `SwqosConfig::Node1(api_token, region, custom_url, transport)` | API token, region, optional URL, transport (HTTP/QUIC) |
| FlashBlock | `SwqosConfig::FlashBlock(api_token, region, custom_url)` | API token, region, optional custom URL |
| BlockRazor | `SwqosConfig::BlockRazor(api_token, region, custom_url, transport)` | API token, region, optional URL, transport (gRPC/HTTP) |
| Astralane | `SwqosConfig::Astralane(api_token, region, custom_url, mode)` | API token, region, optional URL, transport mode (Binary/Plain/QUIC) |
| Stellium | `SwqosConfig::Stellium(api_token, region, custom_url)` | API token, region, optional custom URL |
| Lightspeed | `SwqosConfig::Lightspeed(api_key, region, custom_url)` | API key, region, optional custom URL |
| Soyas | `SwqosConfig::Soyas(api_token, region, custom_url)` | API token, region, optional custom URL |
| Speedlanding | `SwqosConfig::Speedlanding(api_token, region, custom_url)` | API token, region, optional custom URL |
| Helius | `SwqosConfig::Helius(api_key, region, custom_url, swqos_only)` | API key, region, optional URL, SWQoS-only flag (affects min tip) |
| Solami | `SwqosConfig::Solami(api_key, region, custom_url)` | API key, region, optional custom URL |
| LunarLander | `SwqosConfig::LunarLander(api_key, region, custom_url, transport)` | API key, region, optional URL, transport (QUIC/HTTP) |
| Glaive | `SwqosConfig::Glaive(api_key_uuid, region, custom_url, transport)` | UUID auth key, region, optional URL, transport (QUIC/HTTP) |
| Default | `SwqosConfig::Default(url)` | Direct Solana RPC URL |

## Regions (Geographic Routing)

| Region | Enum | ISO |
|---|---|---|
| New York | `SwqosRegion::NewYork` | US-NY |
| Frankfurt | `SwqosRegion::Frankfurt` | DE-HE |
| Amsterdam | `SwqosRegion::Amsterdam` | NL-NH |
| Dublin | `SwqosRegion::Dublin` | IE-D |
| Salt Lake City | `SwqosRegion::SLC` | US-UT |
| Tokyo | `SwqosRegion::Tokyo` | JP-13 |
| Singapore | `SwqosRegion::Singapore` | SG |
| London | `SwqosRegion::London` | GB-ENG |
| Los Angeles | `SwqosRegion::LosAngeles` | US-CA |
| Default (global fallback) | `SwqosRegion::Default` | — |

## Notes

1. **NextBlock** is blacklisted by default (`SWQOS_BLACKLIST.includes(SwqosType::NextBlock)`).
2. **Helius** has dual min-tip pricing: 0.0002 SOL for standard routing, 0.000005 SOL for SWQoS-only (
`swqos_only=true`).
3. **Node1**, **Glaive**, and **LunarLander** support QUIC transport for lower latency.
4. **BlockRazor** defaults to gRPC, with HTTP as an alternative.
5. **Astralane** has 3 transport modes: Binary HTTP (`/irisb`), Plain HTTP (`/iris`), and QUIC.
6. **Lightspeed** requires a custom URL with embedded API key (no default endpoint).
7. **Speedlanding** onboarding: `https://t.me/speedlanding_bot?start=0xzero`
8. **LunarLander** onboarding: `https://docs.hellomoon.io/reference/lunar-lander`
9. **Glaive** docs: `https://glaive.trade/docs`

## Source Files

- Provider enum: `src/swqos/mod.rs` (lines 114-134)
- Client trait: `src/swqos/mod.rs` (lines 188-228)
- Config enum: `src/swqos/mod.rs` (lines 250-295)
- Min tips: `src/constants/swqos.rs` (lines 546-567)
- Tip accounts: `src/constants/swqos.rs` (lines 4-150+)
- Endpoint constants: `src/constants/swqos.rs` (imported constants for each provider)