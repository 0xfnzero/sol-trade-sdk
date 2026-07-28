# Configuration — sol-trade-sdk

> **Status:** Design reference  
> All values are examples. Production deployment requires explicit typed configuration with startup validation.

## 1. Network

```toml
[network]
rpc_url = "https://api.mainnet-beta.solana.com"
rpc_fallback_url = "https://solana-api.projectserum.com"
ws_url = "wss://api.mainnet-beta.solana.com"
connection_timeout_secs = 10
request_timeout_secs = 30
rate_limit_rps = 100
```

## 2. RPC

```toml
[rpc]
primary = "https://api.mainnet-beta.solana.com"
fallback = "https://solana-api.projectserum.com"
commitment = "processed"          # processed/confirmed/finalized
rate_limit_per_second = 100
max_retries = 3
retry_backoff_ms = [100, 500, 2000]
```

## 3. ShredStream

```toml
[shredstream]
enabled = true
udp_bind_addr = "0.0.0.0:8001"
receive_buffer_bytes = 67108864   # 64 MiB
busy_poll_usec = 200
slot_window = 3                   # concurrent slots for reconstruction
fec_enabled = true
stuck_batch_timeout_ms = 50

# Program ID allowlist — only process shreds containing these programs
program_ids = [
    "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P",  # PumpFun
    "pAMMBay6oceH9fJKBRHGP5D4bD8sKbwv3Y5SQE2sTjQ",  # PumpSwap AMM
    "675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8",  # Raydium AMM V4
    "CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C",  # Raydium CPMM
    "LBUZKhRxPF3XUpBCjp4YzTKgY6bE1UEsLzD2jF5K5oM",  # Meteora DAMM V2
    "BonkMZ9ZcCJo3J8E5UJB4ZfG1KjJZxJm9K5pX9v5Y6Z",  # Bonk DEX
]

[cpu_affinity]
receive_core = 0
reconstruct_core = 1
state_strategy_core = 2
execute_core = 3
```

## 4. Protocols

```toml
[protocols]
enabled = ["pumpfun", "pumpswap", "raydium_cpmm"]
disabled = ["raydium_amm_v4", "meteora_damm_v2", "bonk"]

[protocols.pumpfun]
max_quote_age_ms = 500
min_liquidity_sol = 5.0

[protocols.pumpswap]
max_quote_age_ms = 300
require_pool_state = true          # fail if pool state not verified on-chain
```

## 5. Submission

```toml
[submission]
primary_provider = "rpc"
fallback_provider = "jito"
max_simultaneous_lanes = 2
require_same_signature = true      # same tx across lanes

[submission.jito]
endpoint = "https://mainnet.block-engine.jito.wtf"
auth_token = "${JITO_AUTH_TOKEN}"
tip_min_lamports = 1000
tip_max_lamports = 100000
bundle_tip_strategy = "fixed"      # fixed/percentile/dynamic
```

## 6. Wallet

```toml
[wallet]
execution_keypair_path = "/etc/solbot/execution-keypair.json"
payer_keypair_path = "/etc/solbot/payer-keypair.json"
min_balance_sol = 0.5              # emergency stop below this
max_balance_sol = 10.0             # don't hold more than this
```

## 7. Blockhash

```toml
[blockhash]
refresh_interval_ms = 200
max_age_slots = 150                # ~60-90 seconds
stale_threshold_ms = 500
commitment = "confirmed"
fail_on_lag = true                 # stop trading if RPC lags
```

## 8. Fees

```toml
[fees]
compute_unit_limit = 400_000       # per-route override via CU tracking
compute_unit_price = 1000          # microlamports per CU
max_priority_fee_lamports = 10_000
max_relay_tip_lamports = 5_000
max_total_tx_cost_lamports = 20_000
profit_threshold_lamports = 1_000  # minimum net profit to trade
cu_tracking_enabled = true         # per-route CU distribution tracking
cu_tracking_window = 100           # representative transactions per route
```

## 9. Risk

```toml
[risk]
max_sol_per_trade = 0.1
max_token_amount_per_trade = 500_000_000
max_slippage_basis_points = 100    # 1%
max_quote_age_ms = 200
max_source_slot_age = 2            # slots
min_expected_output = 1            # minimum tokens out
min_expected_net_profit_lamports = 500

[risk.global]
max_open_exposure_sol = 1.0
max_exposure_per_mint_pct = 25.0
max_exposure_per_protocol_pct = 50.0
max_tx_per_minute = 60
max_failures_per_5min = 10

[risk.fee_ceilings]
max_priority_fee_lamports = 10_000
max_relay_tip_lamports = 5_000
max_total_tx_cost_lamports = 20_000
max_daily_loss_lamports = 500_000
max_consecutive_losses = 3

[risk.health]
max_rpc_slot_lag = 5
max_packet_loss_pct = 1.0
max_queue_delay_ms = 50
max_blockhash_age_ms = 500
max_reconciliation_backlog = 100

[risk.switches]
global_kill = false
buy_only = false
sell_only = false
per_protocol_disabled = []
per_provider_disabled = []
per_token_denylist = []
```

## 10. Strategy

```toml
[strategy]
type = "arbitrage"                 # arbitrage/market-making/sniper
max_open_positions = 1
position_size_sol = 0.05
take_profit_basis_points = 50
stop_loss_basis_points = 30
max_hold_ms = 30_000
min_slot_distance = 1              # min slots between trades
```

## 11. Observability

```toml
[observability]
metrics_endpoint = "0.0.0.0:9090"
metrics_prefix = "solbot"
log_level = "info"
log_format = "json"
log_sampling_rate = 0.1            # sample repetitive errors
trace_storage_path = "/var/log/solbot/traces"
```

## 12. Storage

```toml
[storage]
state_path = "/var/lib/solbot/state"
dedup_cache_size = 262144          # LRU entries
reconciliation_db = "/var/lib/solbot/reconciliation.db"
max_disk_usage_gb = 10.0
```

## 13. Runtime

```toml
[runtime]
graceful_shutdown_timeout_secs = 10
restart_max_retries = 3
restart_delay_ms = 1000
cpu_affinity_enabled = true
memory_limit_mb = 1024
file_descriptor_limit = 65536
```