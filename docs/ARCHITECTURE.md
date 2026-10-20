# Cached trading boundaries and contribution guide

`PumpSwapRoutingConfig` owns approved market identities and bounded search
limits. Configuration and route search share pool validation. A routing instance
compiles immutable adjacency once; each order still quotes its own size against
one coherently published account snapshot. Deterministic pool ordering governs
ties, and search-budget exhaustion remains an error.
Search paths, visited mints and quote-step scratch buffers use inline storage for
the supported three conversion hops plus one target leg. The public result types
and route selection policy remain unchanged.

`PumpSwapRoutingFeed` is background ingestion, not an RPC discovery service.
Bootstrap/catch-up and provider/fork continuity belong to the application.
`SubscriptionAccountCache` preserves real slot/write_version values and validates
owners, layouts, mints, transfer fees and freshness. A confirmed observation
watermark may keep unchanged accounts fresh; a repeated update is not a heartbeat.
See [streaming lifecycle](STREAMING_QUOTES.md) for the full contract.

`cached_quote` performs amount-specific math. `pumpswap_route_search` chooses
paths. `pumpswap_preparation` produces validated execution parameters.
`instruction/` builds instructions with explicit per-hop protection. Keep
network calls and signing out of quote preparation; do not create a second quote
engine inside an example. Snapshot publication can copy account state in the
background; order processing borrows one immutable snapshot.

Clearing routing state invalidates in-flight preparation, including when a new
snapshot is published before the old search finishes. Normal publication keeps
an already-running coherent search valid within its freshness budget. A returned
prepared trade still needs the application's normal pre-send risk checks.

Retain StonkFun protocol and official IDL names. Public aliases can describe a
different use case without breaking established users. Keep SOL/WSOL
normalization explicit, and distinguish pool identities from live executable state.
Do not merge protocol-specific fee/math modules solely because their APIs look alike.

Examples share offline account loading and CLI/replay mechanics under
`examples/support/`. A shared snapshot loader checks Clock ownership and slot
before quoting. Keep simulation and transaction submission explicit; default
replays do not require RPC. [Example indexes](../examples/README.md) link to
authoritative topic guides rather than restating every configuration default.

Regression checks cover both trade directions, amount-dependent route selection,
transfer fees, slippage, same-slot updates, stale/missing state and deterministic
search limits. Use the documented release benchmark with a fixed fixture for
latency comparisons; debug timings and theoretical allocation counts are not
production p99 measurements.
