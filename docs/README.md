# Documentation

Start with the [examples](../examples/README.md) / [中文示例索引](../examples/README_CN.md).

- [Architecture and contribution boundaries](ARCHITECTURE.md): state, quotes, routing and naming.

- [Streaming quotes](STREAMING_QUOTES.md): parser/streamer sources and account-cache lifecycle.
- [PumpSwap quote routing](PUMPSWAP_QUOTE_ROUTING.md): quoting, route selection and protection.
- [Bridge defaults](PUMPSWAP_BRIDGE_DEFAULTS.md): catalog policy and cross-DEX pool candidates.
- [StonkFun routes](STONKFUN_ROUTES.md): explicit target and funding legs.
- [StonkFun gRPC examples](STONKFUN_GRPC_EXAMPLES.md): online simulation prerequisites.
- [Simulation commands](../examples/SIMULATION.md): simulation, not transaction submission.
- [Trading parameters](TRADING_PARAMETERS.md) / [中文](TRADING_PARAMETERS_CN.md).
- [Low latency](LOW_LATENCY_BOTS.md) / [中文](LOW_LATENCY_BOTS_CN.md).
- [Address lookup tables](ADDRESS_LOOKUP_TABLE.md), [nonce cache](NONCE_CACHE.md),
  [gas policy](GAS_FEE_STRATEGY.md), [pre-buy risk gate](PRE_BUY_RISK_GATE.md).
- [CPMM creator fees](cpmm-creator-fee-share.md): fee-specific handling and regressions.

Topic guides own their configuration and behavior descriptions. Example indexes
link to them instead of duplicating defaults. `research/` and reported-failure
documents are historical evidence, not fresh quote or execution guarantees.
