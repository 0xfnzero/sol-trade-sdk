#!/usr/bin/env bash
# ── sol-trade-sdk Deployment Script ──────────────────────────────────────────
# Phase 13: Production Deployment
#
# Builds the release binary, creates directory structure, installs config and
# systemd service, and starts the bot.
#
# Usage:
#   sudo ./scripts/deploy.sh [--install-dir /opt/solbot] [--config-dir /etc/solbot]
#
# Environment variables:
#   JITO_AUTH_TOKEN  — Jito MEV auth token (required for bundle submission)
#   SOLANA_KEYPAIR   — Path to production keypair JSON (required)
#
# Prerequisites:
#   - Rust toolchain (stable)
#   - Systemd (Linux)
#   - sudo access

set -euo pipefail

# ── Defaults ──────────────────────────────────────────────────────────────────
INSTALL_DIR="${1:-/opt/solbot}"
CONFIG_DIR="${2:-/etc/solbot}"
BINARY_NAME="solbot"
REPO_DIR="$(cd "$(dirname "$0")/.." && pwd)"

# Color output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

info()  { echo -e "${GREEN}[INFO]${NC} $*"; }
warn()  { echo -e "${YELLOW}[WARN]${NC} $*"; }
error() { echo -e "${RED}[ERROR]${NC} $*"; exit 1; }

# ── Pre-flight checks ─────────────────────────────────────────────────────────

info "Running pre-flight checks..."

# Check for required tools
command -v cargo &>/dev/null || error "cargo not found — install Rust toolchain"
command -v systemctl &>/dev/null || warn "systemctl not found — not running on systemd?"

# Check for required env vars
if [ -z "${JITO_AUTH_TOKEN:-}" ]; then
    warn "JITO_AUTH_TOKEN not set — bundle submission will use empty auth token"
fi
if [ -z "${SOLANA_KEYPAIR:-}" ]; then
    warn "SOLANA_KEYPAIR not set — will use path from config.toml"
fi

# Check for required files
[ -f "$REPO_DIR/Cargo.toml" ] || error "Not inside sol-trade-sdk repo (Cargo.toml not found)"
[ -f "$REPO_DIR/configs/production.toml" ] || error "configs/production.toml not found"
[ -f "$REPO_DIR/configs/solbot.service" ] || error "configs/solbot.service not found"

# ── Step 1: Build release binary ──────────────────────────────────────────────

info "Building solbot release binary..."
cd "$REPO_DIR"
cargo build --release --features dev-insecure-tls --bin solbot

info "Build complete."
ls -lh "target/release/$BINARY_NAME"

# Verify binary runs
"target/release/$BINARY_NAME" --help &>/dev/null || error "Binary smoke test failed"

# ── Step 2: Create directory structure ────────────────────────────────────────

info "Creating directory structure..."
mkdir -p "$INSTALL_DIR/bin"
mkdir -p "$INSTALL_DIR/traces"
mkdir -p "$CONFIG_DIR/env"
mkdir -p /var/lib/solbot/state
mkdir -p /var/log/solbot/traces

# ── Step 3: Install binary ────────────────────────────────────────────────────

info "Installing binary to $INSTALL_DIR/bin/$BINARY_NAME..."
cp "target/release/$BINARY_NAME" "$INSTALL_DIR/bin/$BINARY_NAME"
chmod 755 "$INSTALL_DIR/bin/$BINARY_NAME"

# ── Step 4: Install config ────────────────────────────────────────────────────

info "Installing configuration to $CONFIG_DIR/config.toml..."
cp "$REPO_DIR/configs/production.toml" "$CONFIG_DIR/config.toml"
chmod 644 "$CONFIG_DIR/config.toml"

# Generate environment file
info "Creating environment file at $CONFIG_DIR/env/production.env..."
cat > "$CONFIG_DIR/env/production.env" << 'ENVEOF'
# sol-trade-sdk Production Environment
# Owner: root:root, Permissions: 0600
SOLBOT_CONFIG=/etc/solbot/config.toml
SOLBOT_RPC_PRIMARY=https://api.mainnet-beta.solana.com
SOLBOT_RPC_FALLBACK=https://solana-api.projectserum.com
SOLBOT_WALLET_KEYPAIR=/etc/solbot/execution-keypair.json
SOLBOT_SWQOS_JITO_UUID=
SOLBOT_METRICS_ENDPOINT=http://localhost:9090/api/v1/write
RUST_LOG=warn,sol_trade_sdk=info
ENVEOF
chmod 600 "$CONFIG_DIR/env/production.env"

# If SOLANA_KEYPAIR is set, copy the keypair
if [ -n "${SOLANA_KEYPAIR:-}" ] && [ -f "$SOLANA_KEYPAIR" ]; then
    info "Installing keypair from $SOLANA_KEYPAIR..."
    cp "$SOLANA_KEYPAIR" "$CONFIG_DIR/execution-keypair.json"
    chmod 600 "$CONFIG_DIR/execution-keypair.json"
fi

# ── Step 5: Install systemd service ───────────────────────────────────────────

info "Installing systemd service..."
cp "$REPO_DIR/configs/solbot.service" /etc/systemd/system/solbot.service
chmod 644 /etc/systemd/system/solbot.service
systemctl daemon-reload

# ── Step 6: Start the bot ─────────────────────────────────────────────────────

info "Enabling and starting solbot service..."
systemctl enable solbot
systemctl start solbot

# Wait for service to start
sleep 2

# Check status
if systemctl is-active --quiet solbot; then
    info "solbot service is running!"
    systemctl status solbot --no-pager | head -10
    info "Health check: http://127.0.0.1:9091/health"
    info "Metrics:     http://127.0.0.1:9090/metrics"
    info "Logs:        sudo journalctl -u solbot -f"
else
    error "solbot service failed to start. Check: sudo journalctl -u solbot -n 50"
fi

# ── Done ──────────────────────────────────────────────────────────────────────

info "Deployment complete!"
echo ""
echo "  Binary:  $INSTALL_DIR/bin/$BINARY_NAME"
echo "  Config:  $CONFIG_DIR/config.toml"
echo "  Service: /etc/systemd/system/solbot.service"
echo "  Logs:    journalctl -u solbot -f"
echo "  Health:  http://127.0.0.1:9091/health"
echo "  Metrics: http://127.0.0.1:9090/metrics"
echo ""
echo "Next steps:"
echo "  1. Verify health endpoint returns 200"
echo "  2. Check metrics for strategy signal activity"
echo "  3. Monitor logs for any warnings"
echo "  4. When satisfied, switch to --features production in Cargo.toml"