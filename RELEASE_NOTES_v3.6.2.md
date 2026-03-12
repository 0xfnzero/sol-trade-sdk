## sol-trade-sdk v3.6.2

### Changes

- **Node1 QUIC support** — Node1 SWQOS can use QUIC transport: `SwqosConfig::Node1(api_token, region, custom_url, Some(SwqosTransport::Quic))`. Uses UUID auth on first bi stream, one bi stream per transaction, bincode-serialized `VersionedTransaction`. Region endpoints: `SWQOS_ENDPOINTS_NODE1_QUIC` (ny, fra, ams, lon, tk). New dependency: `uuid`.
- **Speedlanding QUIC reliability** — Proactive connection check before send (`ensure_connected()`); 5s connect and send timeouts; reconnect uses `lock().await` so concurrent senders wait for the new connection instead of failing on `try_lock()`; TLS SNI is derived from the endpoint host (e.g. `nyc.speedlanding.trade`) with fallback to `speed-landing` for IP or unknown host. Addresses user reports of transactions failing to send.

### Crates.io

```toml
sol-trade-sdk = "3.6.2"
```

### Repository

- **Tag:** [v3.6.2](https://github.com/0xfnzero/sol-trade-sdk/releases/tag/v3.6.2)
