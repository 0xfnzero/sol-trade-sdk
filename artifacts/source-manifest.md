# Source Manifest — sol-trade-sdk

> Generated: 2026-07-28
> Reference: Pre-flight Control Document §4.1

## Locked Sources

| Source | Locked Commit | Permanent URL |
|---|---|---|
| `0xfnzero/sol-trade-sdk` | `8e92f8f333424a73bc71dd6031649a3df1af8c3e` | https://github.com/0xfnzero/sol-trade-sdk/commit/8e92f8f333424a73bc71dd6031649a3df1af8c3e |
| `shredstream/shredstream-sdk-rust` | `274db4909bf597783541989305a67046bc4eed33` | https://github.com/shredstream/shredstream-sdk-rust/commit/274db4909bf597783541989305a67046bc4eed33 |
| `pump-fun/pump-public-docs` | `9c82f61cb711b044a17f770ab8ce9f9bdf78f333` | https://github.com/pump-fun/pump-public-docs/commit/9c82f61cb711b044a17f770ab8ce9f9bdf78f333 |
| `raydium-io/raydium-cp-swap` | `78f254e1023751e706df7dc15c453fc3e046697c` | https://github.com/raydium-io/raydium-cp-swap/commit/78f254e1023751e706df7dc15c453fc3e046697c |
| `raydium-io/raydium-idl` | `e7e0c96fe77bcf6a020b84a44c47a722aac8e359` | https://github.com/raydium-io/raydium-idl/commit/e7e0c96fe77bcf6a020b84a44c47a722aac8e359 |
| `raydium-io/raydium-sdk-V2` | `fb2d829a559f9b6ca95922e4e6c69e3b5bddc95c` | https://github.com/raydium-io/raydium-sdk-V2/commit/fb2d829a559f9b6ca95922e4e6c69e3b5bddc95c |
| `jito-labs/jito-ts` | `77e94053c4293de71e100b8b42b592ad7d2e567d` | https://github.com/jito-labs/jito-ts/commit/77e94053c4293de71e100b8b42b592ad7d2e567d |
| `jito-labs/searcher-examples` | `f710c30b0ec7449b5122f827b0997218ab1a0315` | https://github.com/jito-labs/searcher-examples/commit/f710c30b0ec7449b5122f827b0997218ab1a0315 |
| `solana-foundation/surfpool` | `626045b434cd1bb2dd40747ff8dc658804e03409` | https://github.com/solana-foundation/surfpool/commit/626045b434cd1bb2dd40747ff8dc658804e03409 |

## Resolution Commands

```bash
# Verify current HEAD against locked commit
cd /path/to/sol-trade-sdk
git rev-parse HEAD                              # should match 8e92f8f
git rev-parse HEAD | xargs -I {} sh -c 'test {} = 8e92f8f... && echo OK || echo MISMATCH'

# Verify Cargo.lock is committed and current
git ls-files Cargo.lock                         # should return Cargo.lock
cargo generate-lockfile                         # should be a no-op if already current

# Verify dependency tree
cargo metadata --format-version 1 > /dev/null
cargo tree --workspace
cargo tree -d
```

## Build Identity

| Field | Value |
|---|---|
| Package version (Cargo.toml) | `5.0.0` |
| README advertised version | `4.0.23` |
| Current commit SHA | `8e92f8f333424a73bc71dd6031649a3df1af8c3e` |
| Rust toolchain | `1.82.0` (pinned via rust-toolchain.toml) |
| Cargo.lock | Committed |
| Build profile (production) | `release` |