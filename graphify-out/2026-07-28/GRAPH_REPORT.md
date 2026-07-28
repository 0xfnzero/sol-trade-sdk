# Graph Report - .  (2026-07-28)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 2479 nodes · 5839 edges · 106 communities (103 shown, 3 thin omitted)
- Extraction: 97% EXTRACTED · 3% INFERRED · 0% AMBIGUOUS · INFERRED: 185 edges (avg confidence: 0.8)
- Token cost: 5,900 input · 3,671 output

## Graph Freshness
- Built from commit: `8e92f8f3`
- Run `git rev-parse HEAD` and compare to check if the graph is stale.
- Run `graphify update .` after code changes (no API cost).

## Community Hubs (Navigation)
- Trading Client Params
- Gas Fee Strategy
- Associated Token Accounts
- Bonk Instruction Builder
- Fast Token Account Creation
- Syscall Bypass & Timing
- gRPC Compression Middleware
- PumpSwap Trading Main
- Direct Memory Access
- Hardware Optimizations
- Infrastructure Config
- Address Lookup & Copy Trade
- CLI Trading Commands
- Bonding Curve Calculations
- Ultra Low Latency
- PumpFun Utilities
- Protocol Optimization
- Slippage & Fee Calculations
- Kernel Bypass UDP
- Astralane QUIC Client
- Serialization & Encoding
- Shared Infrastructure
- TLS Credentials
- Node1 QUIC Client
- Meteora Damm V2 Instructions
- Seed-based Token Accounts
- Realtime System Tuning
- Glaive Client
- Generic Trade Executor
- Jito Transaction Client
- BlockRazor Client
- PumpFun Instructions
- PumpFun Instruction Data
- Fast Timing Utilities
- LunarLander QUIC Client
- Trading Client Creation
- Astralane Client
- Transaction Pool
- Fast SHA256 Hashing
- Zero Slot Client
- Instruction Execution Processing
- Node1 Client
- Solami Client
- Speedlanding Client
- Bonk Swap Calculations
- SWQOS Endpoint Config
- Transaction Sending & Confirmation
- DEX Swap Parameters
- Bonk Pool Types
- Stellium Client
- Raydium AMM v4 Utilities
- Helius Client
- Transaction Builder
- FlashBlock Client
- Middleware System
- PumpSwap Pool Types
- Compiler Optimization
- SIMD Performance Optimizations
- Middleware Manager
- Bonk Pool Utilities
- Bloxroute Client
- Lightspeed Client
- Bonk Copy Trading
- Bonk Sniper Trading
- Raydium AMM v4 Copy Trade
- Raydium CPMM Copy Trade
- High Performance Clock
- Solana RPC Client
- Logging Middleware
- SDK Logging
- Codegen & Compiler Optimization
- SIMD Iterator & Serializer
- Compute Budget Manager
- Nonce Manager
- Meteora Damm v2 Types
- Cargo & Compiler Config
- Compile-Time Event Processor
- Pump Fee Metadata
- WSOL Wrapper Trading
- Subscription Handle
- Meteora Damm v2 Direct Trade
- Global Account
- SPL Token 2022
- PumpSwap Direct Trade
- Seed Trading
- Simple Trading
- Raydium CPMM Types
- Price & Decimals
- PumpFun Cashback Claim
- Raydium CLMM Price
- Latency Test Script

## God Nodes (most connected - your core abstractions)
1. `VersionedTransaction` - 64 edges
2. `TradeType` - 57 edges
3. `SwqosType` - 45 edges
4. `SwapParams` - 45 edges
5. `GasFeeStrategy` - 41 edges
6. `SwqosClientTrait` - 36 edges
7. `TradingClient` - 30 edges
8. `SimpleSellParams` - 27 edges
9. `SimpleBuyParams` - 26 edges
10. `execute_parallel()` - 26 edges

## Surprising Connections (you probably didn't know these)
- `check_mint_ata()` --calls--> `get_associated_token_address_with_program_id_fast_use_seed()`  [INFERRED]
  examples/cli_trading/src/main.rs → src/common/fast_fn.rs
- `close_wsol_real()` --calls--> `get_associated_token_address()`  [INFERRED]
  examples/cli_trading/src/main.rs → src/common/spl_associated_token_account.rs
- `close_wsol_real()` --calls--> `close_account()`  [INFERRED]
  examples/cli_trading/src/main.rs → src/common/spl_token.rs
- `pumpfun_copy_trade_with_grpc()` --calls--> `fetch_nonce_info()`  [INFERRED]
  examples/nonce_cache/src/main.rs → src/common/nonce_cache.rs
- `pumpfun_copy_trade_with_grpc()` --calls--> `fetch_address_lookup_table_account()`  [INFERRED]
  examples/address_lookup/src/main.rs → src/common/address_lookup.rs

## Import Cycles
- None detected.

## Communities (106 total, 3 thin omitted)

### Community 0 - "Trading Client Params"
Cohesion: 0.06
Nodes (61): Any, AccountPolicy, buy_account_flags(), BuyAmount, dummy_pumpfun_params(), find_pool_by_mint(), normalize_swqos_configs(), normalize_swqos_configs_adds_default_rpc_route() (+53 more)

### Community 1 - "Gas Fee Strategy"
Cohesion: 0.06
Nodes (60): GasFeeConfig, Notify, default_rpc_strategy_uses_priority_fee_without_tip(), dynamic_updates_do_not_collapse_dual_lane_fees(), find_strategy(), GasFeeStrategy, GasFeeStrategyType, GasFeeStrategyValue (+52 more)

### Community 2 - "Associated Token Accounts"
Cohesion: 0.07
Nodes (86): Account, create_associated_token_account_idempotent(), get_associated_token_address(), get_associated_token_address_with_program_id(), Instruction, Pubkey, cached_fee_config(), cached_global_config() (+78 more)

### Community 3 - "Bonk Instruction Builder"
Cohesion: 0.05
Nodes (73): bonk_buy_uses_exact_out_when_fixed_output_is_set(), bonk_params(), bonk_sell_uses_exact_out_when_fixed_output_is_set(), bonk_usd1_buy_create_input_builds_usd1_ata_not_wsol_wrap(), BonkInstructionBuilder, pk(), Instruction, Pubkey (+65 more)

### Community 4 - "Fast Token Account Creation"
Cohesion: 0.06
Nodes (62): AtaCacheKey, create_associated_token_account_idempotent_fast(), create_associated_token_account_idempotent_fast_use_seed(), fast_init(), get_associated_token_address_with_program_id_fast(), get_associated_token_address_with_program_id_fast_use_seed(), get_cached_instructions(), get_cached_pda() (+54 more)

### Community 5 - "Syscall Bypass & Timing"
Cohesion: 0.09
Nodes (28): Handle, AsyncIOStats, FastTimeProvider, IOOptimizer, MemoryMappedRegion, Arc, ArrayQueue, AtomicU64 (+20 more)

### Community 6 - "gRPC Compression Middleware"
Cohesion: 0.07
Nodes (36): B, CompressionEncoding, Context, D, EnabledCompressionEncodings, Future, InterceptedService, IntoRequest (+28 more)

### Community 7 - "PumpSwap Trading Main"
Cohesion: 0.09
Nodes (40): BlockhashCache, CachedBlockhash, create_event_callback(), create_solana_trade_client(), EventAction, EventSelection, is_event_fresh(), main() (+32 more)

### Community 8 - "Direct Memory Access"
Cohesion: 0.09
Nodes (26): NonNull, DirectMemoryAccessManager, DMAChannel, DMAStats, DMATransfer, MemoryMappedBuffer, Arc, ArrayQueue (+18 more)

### Community 9 - "Hardware Optimizations"
Cohesion: 0.06
Nodes (17): BranchOptimizer, CacheAlignedCounter, CacheLineAligned, CacheOptimizedRingBuffer, CacheOptimizedRingBuffer<T>, MemoryBarriers, AtomicU64, CachePadded (+9 more)

### Community 10 - "Infrastructure Config"
Cohesion: 0.09
Nodes (27): Eq, H, PartialEq, InfrastructureConfig, CommitmentConfig, Hash, Self, String (+19 more)

### Community 11 - "Address Lookup & Copy Trade"
Cohesion: 0.06
Nodes (43): create_solana_trade_client(), main(), pumpfun_copy_trade_with_grpc(), AnyResult, Box, Error, PumpFunTradeEvent, Result (+35 more)

### Community 12 - "CLI Trading Commands"
Cohesion: 0.22
Nodes (47): check_mint_ata(), Cli, close_wsol_real(), Command, get_wallet_real(), handle_buy(), handle_buy_bonk(), handle_buy_pumpfun() (+39 more)

### Community 13 - "Bonding Curve Calculations"
Cohesion: 0.10
Nodes (12): BondingCurveAccount, Pubkey, Result, Self, PumpFunParams, Arc, Error, Option (+4 more)

### Community 14 - "Ultra Low Latency"
Cohesion: 0.11
Nodes (23): CpuAffinityConfig, LockFreeEventDispatcher, PrefetchOptimizer, Arc, ArrayQueue, AtomicBool, AtomicU64, AtomicUsize (+15 more)

### Community 15 - "PumpFun Utilities"
Cohesion: 0.11
Nodes (36): default_creator_yields_fixed_creator_vault(), extend_bonding_curve_account_instruction(), fee_recipient_ok_for_bonding_curve_mode(), fee_sharing_config_pda_deterministic(), fetch_bonding_curve_account(), fetch_fee_sharing_creator_vault_if_active(), get_bonding_curve_v2_pda(), get_creator() (+28 more)

### Community 16 - "Protocol Optimization"
Cohesion: 0.11
Nodes (22): FastPathCache, ProtocolOptimizationConfig, ProtocolOptimizationStats, ProtocolOptimizationStatsSnapshot, ProtocolStackOptimizer, RouteInfo, Arc, AtomicBool (+14 more)

### Community 17 - "Slippage & Fee Calculations"
Cohesion: 0.15
Nodes (37): calculate_with_slippage_buy(), ceil_div(), compute_fee(), get_buy_token_amount_from_sol_amount(), get_sell_sol_amount_from_token_amount(), Pubkey, buy_base_input_internal(), buy_base_input_internal_with_fees() (+29 more)

### Community 18 - "Kernel Bypass UDP"
Cohesion: 0.11
Nodes (21): AtomicNetworkStats, KernelBypassUDP, NetworkStats, PacketDescriptor, Arc, AtomicBool, AtomicU64, CachePadded (+13 more)

### Community 19 - "Astralane QUIC Client"
Cohesion: 0.09
Nodes (23): AstralaneQuicClient, AtomicUsize, CertificateDer, ClientConfig, Connection, DigitallySignedStruct, Drop, Endpoint (+15 more)

### Community 20 - "Serialization & Encoding"
Cohesion: 0.15
Nodes (29): Deref, Base64Encoder, get_serializer_stats(), legacy_eager_zero_fill_serializer(), perf_serializer_cold_start_vs_legacy_eager(), perf_serializer_lazy_growth_amortization(), PooledTxBufGuard, return_serialization_buffer() (+21 more)

### Community 21 - "Shared Infrastructure"
Cohesion: 0.10
Nodes (28): main(), Box, Error, Result, load_keypair_from_env(), load_keypair_from_string(), Keypair, Result (+20 more)

### Community 22 - "TLS Credentials"
Cohesion: 0.09
Nodes (26): PrivateKeyDer, generate_client_tls_credentials(), Arc, ArcSwap, CertificateDer, ClientConfig, Connection, DigitallySignedStruct (+18 more)

### Community 23 - "Node1 QUIC Client"
Cohesion: 0.10
Nodes (23): RecvStream, Node1QuicClient, Arc, CertificateDer, ClientConfig, Connection, DigitallySignedStruct, Drop (+15 more)

### Community 24 - "Meteora Damm V2 Instructions"
Cohesion: 0.10
Nodes (28): meteora_includes_sysvar_only_when_rate_limiter_is_set(), meteora_includes_writable_referral_account_when_set(), meteora_omits_optional_referral_account(), meteora_params(), meteora_sol_buy_uses_pool_wsol_mint_for_user_input_account(), meteora_swap2_exact_out_uses_amount_out_then_max_input(), MeteoraDammV2InstructionBuilder, pk() (+20 more)

### Community 25 - "Seed-based Token Accounts"
Cohesion: 0.12
Nodes (33): create_associated_token_account_use_seed(), derive_seed_from_mint(), fetch_rent_for_token_account(), get_associated_token_address_with_program_id_use_seed(), Arc, Error, Instruction, Pubkey (+25 more)

### Community 26 - "Realtime System Tuning"
Cohesion: 0.13
Nodes (17): CpuGovernor, OptimizationState, OptimizationStatus, RealtimeConfig, RealtimeStats, RealtimeStatsSnapshot, RealtimeSystemOptimizer, Arc (+9 more)

### Community 27 - "Glaive Client"
Cohesion: 0.12
Nodes (26): binary_response_surfaces_http_and_rpc_errors(), binary_url_uses_official_query_names(), bounded_body(), build_binary_url(), build_health_url(), custom_binary_url_is_not_duplicated_and_health_is_rooted(), GlaiveBackend, GlaiveClient (+18 more)

### Community 28 - "Generic Trade Executor"
Cohesion: 0.12
Nodes (20): GenericTradeExecutor, AddressLookupTableAccount, Arc, Error, Hash, Instruction, Keypair, Option (+12 more)

### Community 29 - "Jito Transaction Client"
Cohesion: 0.13
Nodes (16): Vec, FormatBase64VersionedTransaction, VersionedTransaction, Vec, JitoClient, Arc, Client, Result (+8 more)

### Community 30 - "BlockRazor Client"
Cohesion: 0.16
Nodes (16): Channel, BlockRazorBackend, BlockRazorClient, BlockRazorGrpcClient, Arc, ArcSwap, AtomicBool, Client (+8 more)

### Community 31 - "PumpFun Instructions"
Cohesion: 0.22
Nodes (29): build_buy(), build_sell(), non_pump_buy_respects_explicit_legacy_token_program(), pump_mint(), pump_suffix_buy_forces_token_2022_even_with_explicit_legacy_token_program(), pump_suffix_sell_forces_token_2022_even_with_explicit_legacy_token_program(), pumpfun_from_trade_wsol_quote_regular_buy_selects_v1(), pumpfun_rpc_sol_sentinel_quote_mint_selects_v1() (+21 more)

### Community 32 - "PumpFun Instruction Data"
Cohesion: 0.18
Nodes (25): build_buy_legacy(), build_buy_unified(), build_sell_legacy(), build_sell_unified(), effective_pump_mint_token_program(), effective_quote_mint_and_token_program(), is_explicit_wsol_settlement_mint(), is_native_sol_settlement_mint() (+17 more)

### Community 33 - "Fast Timing Utilities"
Cohesion: 0.15
Nodes (14): fast_elapsed(), fast_elapsed_nanos(), fast_now_micros(), fast_now_millis(), fast_now_nanos(), FastStopwatch, FastTimer, Duration (+6 more)

### Community 34 - "LunarLander QUIC Client"
Cohesion: 0.13
Nodes (15): ClientOptions, LunarLanderQuicClient, LunarLanderBackend, LunarLanderClient, quic_client_options(), Arc, AtomicBool, Client (+7 more)

### Community 35 - "Trading Client Creation"
Cohesion: 0.18
Nodes (13): create_trading_client_from_infrastructure(), create_trading_client_simple(), main(), AnyResult, Box, Error, Result, Error (+5 more)

### Community 36 - "Astralane Client"
Cohesion: 0.16
Nodes (14): AstralaneBackend, AstralaneClient, Arc, AtomicBool, Client, Drop, JoinHandle, Mutex (+6 more)

### Community 37 - "Transaction Pool"
Cohesion: 0.13
Nodes (15): MessageAddressTableLookup, acquire_builder(), PreallocatedTxBuilder, release_builder(), AddressLookupTableAccount, Drop, Hash, Instruction (+7 more)

### Community 38 - "Fast SHA256 Hashing"
Cohesion: 0.15
Nodes (14): fast_sha256_hex(), Arc, AtomicBool, Client, Drop, JoinHandle, Mutex, Option (+6 more)

### Community 39 - "Zero Slot Client"
Cohesion: 0.14
Nodes (13): Arc, AtomicBool, Client, Drop, JoinHandle, Mutex, Option, Result (+5 more)

### Community 40 - "Instruction Execution Processing"
Cohesion: 0.12
Nodes (10): FnOnce, ExecutionPath, InstructionProcessor, MemoryOps, Prefetch, Instruction, Keypair, Pubkey (+2 more)

### Community 41 - "Node1 Client"
Cohesion: 0.14
Nodes (13): Node1Client, Arc, AtomicBool, Client, Drop, JoinHandle, Mutex, Option (+5 more)

### Community 42 - "Solami Client"
Cohesion: 0.18
Nodes (13): Arc, ArcSwap, ClientConfig, Connection, Endpoint, Mutex, Result, Self (+5 more)

### Community 43 - "Speedlanding Client"
Cohesion: 0.18
Nodes (13): Arc, ArcSwap, ClientConfig, Connection, Endpoint, Mutex, Result, Self (+5 more)

### Community 44 - "Bonk Swap Calculations"
Cohesion: 0.16
Nodes (13): compute_protocol_fund_fee(), compute_swap_amount(), compute_trading_fee(), ComputeSwapParams, swap_base_input(), SwapResult, compute_creator_fee_new(), compute_protocol_fund_fee() (+5 more)

### Community 45 - "SWQOS Endpoint Config"
Cohesion: 0.13
Nodes (8): NextBlockClient, Arc, Client, Result, Self, SolanaRpcClient, String, Vec

### Community 46 - "Transaction Sending & Confirmation"
Cohesion: 0.24
Nodes (15): poll_any_transaction_confirmation(), poll_transaction_confirmation(), Client, Result, SerializableTransaction, Signature, SolanaRpcClient, String (+7 more)

### Community 47 - "DEX Swap Parameters"
Cohesion: 0.16
Nodes (15): Debug, AddressLookupTableAccount, Arc, CoreId, Formatter, Hash, Keypair, Option (+7 more)

### Community 48 - "Bonk Pool Types"
Cohesion: 0.14
Nodes (17): AmmFeeOn, ConstantCurve, CurveParams, FixedCurve, LinearCurve, MintParams, pool_state_decode(), PoolState (+9 more)

### Community 49 - "Stellium Client"
Cohesion: 0.15
Nodes (10): Arc, AtomicBool, Client, Drop, Result, Self, SolanaRpcClient, String (+2 more)

### Community 50 - "Raydium AMM v4 Utilities"
Cohesion: 0.25
Nodes (15): derive_serum_vault_signer(), fetch_amm_info(), fetch_market_state(), Error, Pubkey, Result, SolanaRpcClient, amm_info_decode() (+7 more)

### Community 51 - "Helius Client"
Cohesion: 0.21
Nodes (9): HeliusClient, Arc, Client, Option, Result, Self, SolanaRpcClient, String (+1 more)

### Community 52 - "Transaction Builder"
Cohesion: 0.30
Nodes (16): build_transaction(), build_transaction_inner(), build_versioned_transaction(), oversized_instruction(), oversized_transaction_returns_error_without_dropping_priority_semantics(), AddressLookupTableAccount, Arc, Error (+8 more)

### Community 53 - "FlashBlock Client"
Cohesion: 0.18
Nodes (10): ClientBuilder, default_http_client_builder(), FlashBlockClient, Arc, Client, Result, Self, SolanaRpcClient (+2 more)

### Community 54 - "Middleware System"
Cohesion: 0.20
Nodes (11): create_solana_trade_client(), CustomMiddleware, main(), AnyResult, Box, Error, Instruction, Result (+3 more)

### Community 55 - "PumpSwap Pool Types"
Cohesion: 0.23
Nodes (13): decodes_current_pool_virtual_quote_reserves(), decodes_legacy_pool_with_zero_virtual_quote_reserves(), effective_quote_reserves(), LegacyPool, Pool, pool_decode(), pool_payload(), rejects_partial_current_pool_layout() (+5 more)

### Community 56 - "Compiler Optimization"
Cohesion: 0.19
Nodes (13): CodeModel, generate_build_script(), generate_cargo_config_toml(), OptimizationFlags, OptLevel, ProfileConfig, Option, String (+5 more)

### Community 57 - "SIMD Performance Optimizations"
Cohesion: 0.15
Nodes (6): SIMDHash, SIMDMath, SIMDMemory, test_fast_hash(), test_simd_math(), test_simd_memory_copy()

### Community 58 - "Middleware Manager"
Cohesion: 0.25
Nodes (7): MiddlewareManager, Box, Clone, Instruction, Result, Self, Vec

### Community 59 - "Bonk Pool Utilities"
Cohesion: 0.21
Nodes (11): fetch_pool_state(), get_creator_associated_account(), get_platform_associated_account(), get_pool_pda(), get_vault_pda(), Error, Option, PoolState (+3 more)

### Community 60 - "Bloxroute Client"
Cohesion: 0.21
Nodes (9): BloxrouteClient, Arc, Client, Result, Self, SolanaRpcClient, String, Vec (+1 more)

### Community 61 - "Lightspeed Client"
Cohesion: 0.22
Nodes (8): LightspeedClient, Arc, Client, Result, Self, SolanaRpcClient, String, Vec

### Community 62 - "Bonk Copy Trading"
Cohesion: 0.22
Nodes (12): bonk_copy_trade_with_grpc(), create_event_callback(), create_solana_trade_client(), main(), AnyResult, BonkTradeEvent, Box, DexEvent (+4 more)

### Community 63 - "Bonk Sniper Trading"
Cohesion: 0.22
Nodes (12): bonk_sniper_trade_with_shreds(), create_event_callback(), create_solana_trade_client(), main(), AnyResult, BonkTradeEvent, Box, DexEvent (+4 more)

### Community 64 - "Raydium AMM v4 Copy Trade"
Cohesion: 0.22
Nodes (12): create_event_callback(), create_solana_trade_client(), main(), raydium_amm_v4_copy_trade_with_grpc(), AnyResult, Box, DexEvent, Error (+4 more)

### Community 65 - "Raydium CPMM Copy Trade"
Cohesion: 0.22
Nodes (12): create_event_callback(), create_solana_trade_client(), main(), raydium_cpmm_copy_trade_with_grpc(), AnyResult, Box, DexEvent, Error (+4 more)

### Community 66 - "High Performance Clock"
Cohesion: 0.26
Nodes (6): elapsed_micros_since(), HighPerformanceClock, now_micros(), Default, Instant, Self

### Community 67 - "Solana RPC Client"
Cohesion: 0.23
Nodes (7): Arc, Result, Self, SolanaRpcClient, String, Vec, SolRpcClient

### Community 68 - "Logging Middleware"
Cohesion: 0.24
Nodes (8): LoggingMiddleware, Box, Instruction, Result, Vec, InstructionMiddleware, Send, Sync

### Community 69 - "SDK Logging"
Cohesion: 0.26
Nodes (9): extract_swqos_error_message(), format_elapsed(), log_swqos_submission_failed(), log_swqos_submitted(), print_sdk_timing_block(), Display, Duration, Option (+1 more)

### Community 70 - "Codegen & Compiler Optimization"
Cohesion: 0.27
Nodes (7): CodegenConfig, CompilerOptimizationStats, CompilerOptimizer, InlineStrategy, AtomicU64, Self, test_ultra_performance_config()

### Community 71 - "SIMD Iterator & Serializer"
Cohesion: 0.25
Nodes (6): F, String, T, Vec, SIMDIterator, SIMDSerializer

### Community 72 - "Compute Budget Manager"
Cohesion: 0.29
Nodes (10): compute_budget_instructions(), ComputeBudgetCacheKey, extend_compute_budget_instructions(), prune_cache(), DashMap, Instruction, K, SmallVec (+2 more)

### Community 73 - "Nonce Manager"
Cohesion: 0.25
Nodes (9): add_nonce_instruction(), get_transaction_blockhash(), Error, Hash, Instruction, Keypair, Option, Result (+1 more)

### Community 74 - "Meteora Damm v2 Types"
Cohesion: 0.38
Nodes (9): BaseFeeStruct, DynamicFeeStruct, Pool, pool_decode(), PoolFeesStruct, PoolMetrics, RewardInfo, Option (+1 more)

### Community 75 - "Cargo & Compiler Config"
Cohesion: 0.28
Nodes (6): CargoConfig, CompilerConfig, HashMap, Result, Vec, test_compiler_config_generation()

### Community 76 - "Compile-Time Event Processor"
Cohesion: 0.36
Nodes (3): CompileTimeOptimizedEventProcessor, test_compile_time_processor(), test_compiler_optimizer_creation()

### Community 77 - "Pump Fee Metadata"
Cohesion: 0.32
Nodes (8): get_mayhem_fee_recipient_meta_random(), get_standard_fee_recipient_meta_random(), pump_fee_meta_default_standard_uses_main_fee_recipient(), pump_fee_meta_rejects_amm_fee_for_standard_ix(), pump_fee_meta_rejects_amm_fee_when_building_mayhem_ix(), pump_fee_meta_uses_observed_non_amm_fee_for_standard_ix(), pump_fun_fee_recipient_meta(), AccountMeta

### Community 78 - "WSOL Wrapper Trading"
Cohesion: 0.48
Nodes (6): create_solana_trade_client(), main(), Box, Error, Result, SolanaTrade

### Community 79 - "Subscription Handle"
Cohesion: 0.33
Nodes (5): Box, Fn, JoinHandle, Send, SubscriptionHandle

### Community 80 - "Meteora Damm v2 Direct Trade"
Cohesion: 0.60
Nodes (5): create_solana_trade_client(), main(), required_u64_env(), AnyResult, SolanaTrade

### Community 81 - "Global Account"
Cohesion: 0.40
Nodes (3): GlobalAccount, Pubkey, Self

### Community 82 - "SPL Token 2022"
Cohesion: 0.40
Nodes (5): initialize_account3(), Instruction, ProgramError, Pubkey, Result

### Community 83 - "PumpSwap Direct Trade"
Cohesion: 0.60
Nodes (4): create_solana_trade_client(), main(), AnyResult, SolanaTrade

### Community 84 - "Seed Trading"
Cohesion: 0.60
Nodes (4): create_solana_trade_client(), main(), AnyResult, SolanaTrade

### Community 85 - "Simple Trading"
Cohesion: 0.40
Nodes (4): main(), Box, Error, Result

### Community 86 - "Raydium CPMM Types"
Cohesion: 0.60
Nodes (4): pool_state_decode(), PoolState, Option, Pubkey

### Community 88 - "PumpFun Cashback Claim"
Cohesion: 0.67
Nodes (3): claim_cashback_pumpfun_instruction(), Option, test_claim_cashback_instruction()

## Knowledge Gaps
- **10 isolated node(s):** `TradeDirection`, `PoolStatus`, `VestingParams`, `AmmFeeOn`, `ZeroCostAbstraction` (+5 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **3 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `SwapParams` connect `DEX Swap Parameters` to `PumpFun Instruction Data`, `Gas Fee Strategy`, `Trading Client Params`, `Bonk Instruction Builder`, `Fast Token Account Creation`, `Meteora Damm V2 Instructions`, `Middleware Manager`, `Generic Trade Executor`, `Jito Transaction Client`, `PumpFun Instructions`?**
  _High betweenness centrality (0.221) - this node is a cross-community bridge._
- **Why does `SyscallBypassConfig` connect `Syscall Bypass & Timing` to `Generic Trade Executor`?**
  _High betweenness centrality (0.157) - this node is a cross-community bridge._
- **Why does `simulate_transaction()` connect `Generic Trade Executor` to `Trading Client Params`, `Gas Fee Strategy`, `Middleware Manager`, `Transaction Builder`?**
  _High betweenness centrality (0.145) - this node is a cross-community bridge._
- **What connects `TradeDirection`, `PoolStatus`, `VestingParams` to the rest of the system?**
  _10 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `Trading Client Params` be split into smaller, more focused modules?**
  _Cohesion score 0.0586283185840708 - nodes in this community are weakly interconnected._
- **Should `Gas Fee Strategy` be split into smaller, more focused modules?**
  _Cohesion score 0.05986842105263158 - nodes in this community are weakly interconnected._
- **Should `Associated Token Accounts` be split into smaller, more focused modules?**
  _Cohesion score 0.0691912108461898 - nodes in this community are weakly interconnected._