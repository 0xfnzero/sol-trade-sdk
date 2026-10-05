# CPMM creator-fee protocol share (October 2026)

Source: [Raydium changelog](https://docs.raydium.io/reference/changelog/2026-09-19-cpmm-creator-fee-protocol-share), verified against `raydium-io/raydium-cp-swap` program source.

Use `instruction::raydium_cpmm::collect_creator_fee(&pool_address, &pool)` for creator-signed collection, or `collect_creator_fee_permissionless(&payer, &pool_address, &pool)` for collection funded by another signer. Both return an `Instruction`; submit it with the creator or payer signature respectively. Fetch `pool` with `instruction::utils::raydium_cpmm::fetch_pool_state`.

The builders retain accounts 1–14 in their original order. Creator-signed collection appends the read-only `creator_fee_share` PDA at #15. Permissionless collection appends the read-only pool `amm_config` at #15 and `creator_fee_share` at #16. The PDA uses `[b"creator_fee_share", creator, amm_config]` under the CPMM program. It is always included, including when absent on-chain. Recipients are the creator's canonical ATAs, with each mint's token program (SPL Token or Token-2022).

`AmmConfig.creator_fee_share_rate` replaces the first old padding element; padding is now `[u64; 14]`. The account remains 236 bytes including its discriminator (228-byte body). Update any struct literals or serialized layouts that used `[u64; 15]`. `CreatorFeeShare` is 145 bytes including its discriminator.

Use `fetch_creator_fee_share_rate` to read the current rate. It fetches AmmConfig and the share PDA together in one confirmed `getMultipleAccounts` snapshot, avoiding config/override values from separate bank reads: a valid CPMM-owned share PDA overrides config, including a zero custom rate. Missing, empty or foreign-owned accounts use the config rate. Malformed CPMM accounts and RPC errors propagate instead of silently using defaults. Rates must not exceed 1,000,000.

`estimate_creator_fee_payout(&pool, rate)` estimates both creator payouts. `split_creator_fee(gross, rate)` returns `(creator, protocol)` with `protocol = floor(gross * rate / 1_000_000)` and `creator = gross - protocol`. This preserves on-chain rounding and uses u128 intermediates. Estimates are before Token-2022 transfer fees and may change if the rate changes before collection.

PoolState layout, swaps, quotes and LP paths are unchanged. Pool creator-fee counters are gross accrued amounts; protocol-fee counters also increase at collection. Reserve calculations must continue subtracting the full gross creator-fee counters.

The bundled CPMM IDL includes both collection account lists, the PDA seeds, the updated AmmConfig and CreatorFeeShare layouts.

## Executable simulation example

From this repository:

```bash
cargo run --example cpmm_creator_fee_simulate
```

Optional configuration:

```bash
RPC_URL=https://solana-rpc.publicnode.com \
CPMM_POOL=BUVzsLLLG7GWoyJVoU31pXiBveazA6GXTavZ9VD3CwS9 \
CPMM_PAYER=BH23VsesJX61Ve3ZgHGzPKgcgCcMKMhpGGAqNeG3NFou \
CPMM_REPORT_PATH=/tmp/cpmm-collection-report.json \
cargo run --example cpmm_creator_fee_simulate
```

The example loads mainnet accounts and calls `simulateTransaction` for both collection instructions. It never sends a transaction. No private key is needed: signatures are placeholders with `sigVerify=false`, so it verifies program execution and signer metadata, **not possession of signer keys**. The creator must have SOL for creator-signed ATA creation; the permissionless payer must be a funded system account. The default addresses are public test targets and their availability can change. Choose another pool if its creator-fee counters have been cleared.

Assertions check both token vault decreases, creator ATA increases after Token-2022 transfer fees, protocol-counter increases, creator-counter clearing, preservation of all other PoolState fields (except recent epoch), and unchanged config/share accounts. Reads bracket the simulation; if account state changes, the verifier retries rather than comparing unrelated snapshots.

Negative controls check old 14-account calls (`3005`), a wrong share PDA (`2006`), two consecutive collections in one transaction (the second fails `6014`), and permissionless collection with config present but share omitted (`3005`). Expected negative-control failures do not count as example failures.

Run the live integration test explicitly:

```bash
cargo test --test cpmm_creator_fee_simulation -- --ignored --nocapture
```

The live test is ignored by default because it requires RPC access and a pool with fees. Offline tests use captured mainnet bytes and validate decoder parity between both SDKs, fee settlement, and a captured Token-2022 mint's 3% transfer-fee rounding/cap:

```bash
cargo test --test cpmm_creator_fee_share
cargo test --test cpmm_creator_fee_simulation
```

Validated on October 4, 2026: the default pool uses the config's 50,000 rate (5%), with no share PDA. Pool `3Cv9uPpaSqQD55oiiegpC5SBm9664kqbJoB5K7YBQfGb` has a share PDA with rate 0, which overrides the config's 5%. Both collection modes passed all assertions and negative controls. Raw pre/post accounts, slot numbers, logs, amounts and instructions are saved in `tests/fixtures/cpmm_creator_fee_default_mainnet.json` and `cpmm_creator_fee_override_mainnet.json`.

The additional pool `EjqGHevzN2pgaTf5CZnbMBuiYdPpWYx4pmeY7abwqyrM` exercises a nonzero Token-2022 creator payout with 1% transfer fee and 5% protocol share. Both collection modes passed mainnet simulation. The recipient's withheld-fee extension increases by exactly the transfer fee; the vault's withheld fee is unchanged. LP trading reserves remain unchanged. Raw accounts and logs are saved in `tests/fixtures/cpmm_creator_fee_token2022_mainnet.json`, and offline regression checks recompute transfer fees from each captured mint's active epoch schedule.

To require this branch when running the live test:

```bash
CPMM_POOL=EjqGHevzN2pgaTf5CZnbMBuiYdPpWYx4pmeY7abwqyrM \
CPMM_REQUIRE_TRANSFER_FEE=1 \
cargo test --test cpmm_creator_fee_simulation -- --ignored --nocapture
```

The test fails if either collection mode does not exercise a nonzero transfer fee. All six captured collection cases also validate legacy-account, wrong-PDA and duplicate-collection negative controls. These remain unsigned simulations and do not test signer-key possession or transaction landing.

## Swap and quote comparison after collection

The `cpmm_swap_after_collection_simulate` example compares `swap` against `collect → swap` using the same SDK-built swap instruction and a fresh simulated trader. Run it from the trade SDK repository:

```bash
cargo run --example cpmm_swap_after_collection_simulate
cargo test --test cpmm_swap_after_collection -- --ignored --nocapture
```

It defaults to the official public mainnet RPC and the Token-2022 pool above. `RPC_URL`, `CPMM_POOL`, `CPMM_PAYER` and the absolute `CPMM_REPORT_PATH` can override the inputs. The pool creator must have SOL and sufficient token1 in its existing ATA to fund the synthetic trader. Funding and token transfers occur **only inside the unsigned simulation**; the example never broadcasts or requests a private key.

Four comparisons cover exact-input/exact-output swaps after creator-signed/permissionless collection. Each comparison executes two simulations, verifies equal trader input/output, validates exact-input output against the SDK quote, checks exact-output input caps, compares subsequent SDK quotes and LP reserves, checks creator/protocol/vault accounting differences, and compares parser-decoded SwapEvent amounts against actual token balance changes. If the chain changes between the bracketing reads, the comparison retries.

October 4, 2026 results: exact-input spends 1,000,000 raw units for 670,516 output units in both transactions. Exact-output receives 335,258 units for 500,003 input units in both transactions. These are fixture-specific raw token units, not reusable live quotes. All four comparisons passed, covering eight swap simulations. Raw accounts and logs are captured in `tests/fixtures/cpmm_swap_after_collection_mainnet.json` and checked offline with:

```bash
cargo test --test cpmm_swap_after_collection
```

The test recomputes quotes and balances from the saved bytes, validates all four distinct mode combinations, and decodes swap logs and post-swap pool counters through the parser SDK. LP reserve invariance is checked; deposit/withdraw instruction execution is not part of this comparison.

## Deposit/withdraw comparison after collection

Run the LP example or explicitly opt into its live test:

```bash
cargo run --example cpmm_lp_after_collection_simulate
cargo test --test cpmm_lp_after_collection -- --ignored --nocapture
cargo test --test cpmm_lp_after_collection
```

The example constructs deposit/withdraw instructions from the bundled IDL as test support; these are not new public SDK LP builders. It uses the same RPC/pool/payer/report environment variables and simulation-only funding as the swap example. A preparatory SDK swap obtains both tokens. Withdrawal preparation deposits LP first. It then compares the same LP operation with and without an immediately preceding collection, in both creator-signed and permissionless modes. Chain snapshots bracket the complete sequence; changed accounts cause retries.

Assertions recompute proportional deposit ceiling/withdrawal floor amounts, inverse/forward Token-2022 transfer fees, user token/LP deltas, pool LP supply, mint supply, effective vault reserves, withheld fees, collection protocol counters and net creator receipts. Raw `LpChangeEvent` fields must match the pre-operation reserves and actual transfers, and the event is identical before/after collection. This event does not contain the user or requested LP amount, so the verifier checks those through instruction inputs and account deltas. Both SDKs independently decode the post-operation pool and must agree on LP supply and fee counters. Other pool fields, config/share accounts and mints are checked for unexpected changes.

October 4, 2026 mainnet simulations passed all four mode combinations: deposits minted 399,258 LP units and withdrawals burned 199,629 LP units, identically with and without collection. These are raw units for this captured state, not future quotes. Actual RPC accounts and logs are saved in `tests/fixtures/cpmm_lp_after_collection_mainnet.json`. The offline suite revalidates these bytes and includes mutation controls that fail when a user balance or reported share rate is altered. Live tests are ignored by default; all transactions remain unsigned simulations and are never broadcast.

## Preferred runtime path: gRPC accounts and shred instructions, no RPC

Use `SubscriptionAccountCache::prepare_cpmm_creator_fee_collection(pool, payer, ctx)` for normal operation. `payer=None` builds creator-signed collection; `Some(payer)` builds permissionless collection. This synchronous API has no RPC client or fallback. It reads the cached PoolState, AmmConfig and derived CreatorFeeShare PDA, returns the ready instruction plus creator/protocol estimates, and checks owner/discriminator/identity/rate and slot age. Amounts exclude Token-2022 transfer fees. The existing RPC helpers remain available for explicit diagnostics/bootstrap; the simulation examples use RPC only because `simulateTransaction` requires it.

Feed `RawAccountSnapshotEvent` into `update_from_parser_snapshot` with `--features parser-adapter`. Subscribe by the three explicit account addresses with `EventType::AccountRawSnapshot`, so an empty/closed share account is not lost by a normalized discriminator filter. Updates preserve slot/write_version ordering; zero-lamport closures clear the old custom rate. Apply complete account batches before publishing a stable cache to the collection path. Processed gRPC updates can be on a fork; reset/reseed the cache on reconnect gaps or reorgs as appropriate for your feed.

A missing cache entry means **unknown**, not “PDA absent.” Yellowstone streams typically emit changes, not a complete snapshot of existing or nonexistent accounts. Seed a validated startup/persisted snapshot (including explicit absence where known), or wait for complete updates. Silence, stream startup and a shred collection instruction do not prove the share PDA is absent. Stale or incomplete caches return errors; the SDK never silently fetches RPC or assumes a zero/default rate. Set `maximum_slot_age` deliberately: rarely changing configs may need refreshed observations or a provider snapshot, rather than fabricated new slots.

The new stream example runs without RPC, private keys, simulation or broadcast:

```bash
CPMM_POOL=<pool> CPMM_CREATOR=<creator> CPMM_AMM_CONFIG=<config> \
GRPC_URL=<yellowstone-endpoint> GRPC_TOKEN=<provider-token> \
SHRED_ENDPOINT=http://localhost:10800 \
cargo run --features parser-adapter --example cpmm_creator_fee_stream
```

`SHRED_ENDPOINT` is optional. `CPMM_PAYER` selects permissionless mode. `CPMM_SNAPSHOT_JSONL` loads raw subscription snapshots before streaming; `MAXIMUM_SLOT_AGE` defaults to 256, and `DURATION_SECONDS` defaults to 30. `CPMM_STREAM_REPORT_PATH` records received raw updates as JSONL for later replay; this recording is incremental, not automatically a complete snapshot. Record these JSON lines with the parser SDK's `grpc_raw_cache` example, subscribing to pool/config/share addresses. For offline replay without any network:

```bash
CPMM_POOL=<pool> CPMM_SNAPSHOT_JSONL=/path/to/snapshots.jsonl \
cargo run --features parser-adapter --example cpmm_creator_fee_stream -- --replay
cargo test --features parser-adapter --test cpmm_creator_fee_cached
```

Shred subscription uses the existing outer-instruction parser to identify creator-signed/permissionless collection intent. It does not supply full account state, executed balances or logs and never clears fee counters in the cache. ALT-loaded identities may be unresolved; rely on resolved gRPC transactions for those cases. The example waits for raw account updates to reflect execution. Offline tests replay existing captured bytes through the cache (their historical source remains RPC simulation), exercise full/zero custom rates and config fallback, reject unknown/stale state, and verify that closures and delayed updates cannot resurrect an old custom rate.

October 5, 2026 live gRPC check: a short subscription received real AmmConfig updates with **zero RPC calls**. It did not receive the target pool state, so collection preparation correctly remained not ready. Three raw updates from the recorded run are saved in `tests/fixtures/cpmm_creator_fee_grpc_config_mainnet.jsonl`; the regression independently compares both SDK config decoders and ensures this partial capture cannot produce a collection estimate. Full cache/reorg/funding/execution readiness is not claimed for this partial live check. No shred endpoint was configured; shred verification used the offline transaction parsing tests.

The stream example checks `YellowstoneGrpc::subscription_status()` before and after each consumed batch. Its continuity revision changes on reconnect, disconnection, queue overflow or subscription-filter changes. After a gap the example discards its cache and queued data, then waits for fresh complete observations; it does not reuse a persisted seed across a reconnect. A connected transport alone is insufficient: fresh, increasing gRPC block metadata is required, and `STREAM_STALL_SECONDS` (default 10) withdraws readiness if block progress stops. Repeated metadata for the same slot does not extend freshness. These checks do not establish fork finality; processed feeds still require caller fork policy.

Cache preparation now rejects already-cleared creator fees before building a collection that would fail on-chain. A full (100%) protocol share remains valid when gross accrued fees are nonzero. Fee-share bytes are borrowed without allocating/copying the account on each preparation, and the example prints only changed estimates rather than repeating identical outputs every block. JSONL recording uses buffered writes and is explicitly flushed at shutdown.

Both parser snapshot ingestion methods normalize zero-lamport closures to empty cached bytes, including `update_from_parser_account`. Delayed older observations cannot restore a closed CreatorFeeShare account's old custom rate. Dynamic gRPC filters preserve BlockMeta and the latest accepted account set across reconnect; full control queues reject updates promptly rather than preventing shutdown. The parser SDK's local gRPC transport regression verifies these reconnect requests without external RPC.

Prepared collections expose `snapshot_slot` and `account_versions` for pool/config/share observations. Before using a staged estimate, call:

```rust,ignore
let prepared = cache.prepare_cpmm_creator_fee_collection(pool, payer, ctx)?;
// Before signing/submitting, with the current subscribed slot and stable cache:
cache.validate_cpmm_creator_fee_collection(&prepared, payer, current_ctx)?;
```

Validation rejects changed account versions (even when values are unchanged), stale state, an older read context, inconsistent snapshot metadata, different payer/mode, cleared fees and changed instruction/estimate fields. This remains a local cache check, not an execution/finality guarantee. Reprepare after relevant account updates and retain the transport/fork checks described above.

Block freshness in the stream example now uses `metadata.grpc_recv_us`, rather than the time at which the consumer drains its queue. Old queued blocks therefore cannot make a stalled feed appear fresh. Missing/future receive timestamps withdraw readiness. Advancing source versions still refreshes stored preparation metadata, while unchanged visible estimates remain suppressed in logs.

### 本地热路径与消费循环（2026-10-05 审查）

`update_from_owned_parser_snapshot(event)` 消费原始事件并转移账户字节，避免借用适配器的数据复制；两个接口保留相同的闭户、版本与冲突检查。

流示例每批处理至多 256 个事件，积压时暂停估算并协作让出执行权。工作后不固定等待 10ms；空闲等待由 `STREAM_POLL_US` 控制（默认 100，设置 0 为协作轮询，增加 CPU 占用）。Tokio 的实际唤醒延迟可能大于该间隔。记录文件和控制台输出也可能增加延迟。

本地 CPU 耗时测量不调用 RPC：

```bash
cargo test --release --features parser-adapter --test cpmm_creator_fee_cached cached_collection_latency -- --ignored --nocapture
```

本机热缓存 10,000 样本：prepare/validate P50 约 7.5–8.4µs，P99 约 8.2–12.6µs。该数值不包含网络、protobuf 接收、调度或链执行，不代表端到端保证，也没有修改前对照基线。

第二轮审查：借用账户适配器现在先检查版本，只有新观察才复制数据；旧更新不读取/复制数据，相同版本仍逐字节检查冲突。准备校验先拒绝已变化的来源版本，再对版本相同的准备进行完整重建比较，保留公开字段的篡改检测。

第三轮边界修复：接收时间非法或过期时保留最高有效 slot，重复旧 slot 不能重新证明区块进展；仅在订阅连续性丢失、重建缓存时重置该记录。批次积压结束后立即重新检查估算，最终 ready 检查放在录制文件 flush 之后，避免磁盘停顿掩盖过期或断线。停流轮询只在错误消息变化时分配错误字符串。
