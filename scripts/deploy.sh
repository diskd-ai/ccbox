#!/usr/bin/env bash
# scripts/deploy.sh
#
# Deploy ccbox-relay + ccbox-remote-web to a server over SSH.
#
# Usage:
#   ./scripts/deploy.sh                # full: install deps + build + deploy
#   ./scripts/deploy.sh --update       # quick: sync, rebuild, restart services
#   ./scripts/deploy.sh --setup-only   # install deps + configure services (no build)
#
# Environment:
#   DEPLOY_SERVER   SSH target          (required, e.g. root@your-server)
#   DEPLOY_DIR      Remote install dir  (default: /opt/ccbox)
#   RELAY_PORT      Relay listen port   (default: 8787)
#   DOMAIN          Primary domain      (default: ccbox.app)

set -euo pipefail

# ---------------------------------------------------------------------------
# Config
# ---------------------------------------------------------------------------
SERVER="${DEPLOY_SERVER:?set DEPLOY_SERVER to the SSH target, e.g. root@your-server}"
DEPLOY_DIR="${DEPLOY_DIR:-/opt/ccbox}"
RELAY_PORT="${RELAY_PORT:-8787}"
DOMAIN="${DOMAIN:-ccbox.app}"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

MODE="full"
[[ "${1:-}" == "--update" ]]     && MODE="update"
[[ "${1:-}" == "--setup-only" ]] && MODE="setup"

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; BOLD='\033[1m'; NC='\033[0m'
info()    { echo -e "${GREEN}>>>${NC} $*"; }
warn()    { echo -e "${YELLOW}!!${NC} $*"; }
header()  { echo -e "\n${BOLD}=== $* ===${NC}"; }
fail()    { echo -e "${RED}ERROR:${NC} $*" >&2; exit 1; }

# ---------------------------------------------------------------------------
# Preflight
# ---------------------------------------------------------------------------
command -v ssh   >/dev/null || fail "ssh not found"
command -v rsync >/dev/null || fail "rsync not found"

header "Deploying to ${SERVER} (mode: ${MODE})"

# ---------------------------------------------------------------------------
# Step 1 -- Sync source to server
# ---------------------------------------------------------------------------
header "Syncing source"
info "rsync ${PROJECT_DIR}/ -> ${SERVER}:${DEPLOY_DIR}/src/"
ssh "$SERVER" "mkdir -p ${DEPLOY_DIR}/{src,bin,web,data}"
rsync -az --delete \
  --exclude target/ \
  --exclude node_modules/ \
  --exclude .git/ \
  --exclude dist/ \
  "$PROJECT_DIR/" "${SERVER}:${DEPLOY_DIR}/src/"

# ---------------------------------------------------------------------------
# Step 2 -- Remote: install deps, build, configure, start
# ---------------------------------------------------------------------------
ssh -t "$SERVER" bash -s -- "$MODE" "$DEPLOY_DIR" "$RELAY_PORT" "$DOMAIN" <<'REMOTE'
set -euo pipefail

MODE="$1"
DEPLOY_DIR="$2"
RELAY_PORT="$3"
DOMAIN="$4"

GREEN='\033[0;32m'; YELLOW='\033[1;33m'; BOLD='\033[1m'; NC='\033[0m'
info()   { echo -e "${GREEN}>>>${NC} $*"; }
warn()   { echo -e "${YELLOW}!!${NC} $*"; }
header() { echo -e "\n${BOLD}--- $* ---${NC}"; }

export DEBIAN_FRONTEND=noninteractive

# -------------------------------------------------------
# Install system dependencies (skip for --update)
# -------------------------------------------------------
install_deps() {
    header "System packages"
    apt-get update -qq
    apt-get install -y -qq build-essential pkg-config libssl-dev curl unzip >/dev/null

    # -- Rust --
    if ! command -v rustup &>/dev/null; then
        info "Installing Rust toolchain ..."
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
    fi
    export PATH="$HOME/.cargo/bin:$PATH"
    info "Rust: $(rustc --version)"
    rustup update stable 2>/dev/null || true

    # -- Bun --
    if ! command -v bun &>/dev/null; then
        info "Installing bun ..."
        curl -fsSL https://bun.sh/install | bash
    fi
    export BUN_INSTALL="$HOME/.bun"
    export PATH="$BUN_INSTALL/bin:$PATH"
    info "Bun: $(bun --version)"

    # -- Caddy --
    if ! command -v caddy &>/dev/null; then
        info "Installing Caddy ..."
        apt-get install -y -qq debian-keyring debian-archive-keyring apt-transport-https >/dev/null
        curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/gpg.key' \
            | gpg --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg 2>/dev/null
        curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt' \
            > /etc/apt/sources.list.d/caddy-stable.list
        apt-get update -qq && apt-get install -y -qq caddy >/dev/null
    fi
    info "Caddy: $(caddy version)"
}

# -------------------------------------------------------
# Build ccbox-relay
# -------------------------------------------------------
build_relay() {
    header "Building ccbox-relay"
    export PATH="$HOME/.cargo/bin:$PATH"
    cd "$DEPLOY_DIR/src"
    cargo build --release --package ccbox-relay 2>&1
    cp -f target/release/ccbox-relay "$DEPLOY_DIR/bin/ccbox-relay"
    info "Binary: $DEPLOY_DIR/bin/ccbox-relay ($(du -h "$DEPLOY_DIR/bin/ccbox-relay" | cut -f1))"
}

# -------------------------------------------------------
# Build ccbox-remote-web
# -------------------------------------------------------
build_web() {
    header "Building ccbox-remote-web"
    export BUN_INSTALL="$HOME/.bun"
    export PATH="$BUN_INSTALL/bin:$PATH"
    cd "$DEPLOY_DIR/src/packages/ccbox-remote-web"
    bun install 2>&1
    bun run build 2>&1
    rsync -a --delete dist/ "$DEPLOY_DIR/web/"
    info "Static files: $DEPLOY_DIR/web/ ($(du -sh "$DEPLOY_DIR/web/" | cut -f1))"
}

# -------------------------------------------------------
# Configure systemd + Caddy (idempotent)
# -------------------------------------------------------
configure_services() {
    header "Configuring systemd service: ccbox-relay"
    cat > /etc/systemd/system/ccbox-relay.service <<UNIT
[Unit]
Description=CCBox Relay Server
After=network.target

[Service]
Type=simple
ExecStart=${DEPLOY_DIR}/bin/ccbox-relay serve --port ${RELAY_PORT} --data-dir ${DEPLOY_DIR}/data
WorkingDirectory=${DEPLOY_DIR}
Restart=on-failure
RestartSec=5
Environment=RUST_LOG=info

[Install]
WantedBy=multi-user.target
UNIT
    systemctl daemon-reload
    systemctl enable ccbox-relay

    header "Configuring Caddy"
    mkdir -p /etc/caddy
    cat > /etc/caddy/Caddyfile <<CADDY
# ccbox.app -- static web UI + relay proxy
${DOMAIN}, www.${DOMAIN} {
    # Relay API / WebSocket endpoints
    handle /health {
        reverse_proxy localhost:${RELAY_PORT}
    }
    handle /pair {
        reverse_proxy localhost:${RELAY_PORT}
    }
    handle /ccbox {
        reverse_proxy localhost:${RELAY_PORT}
    }
    handle /client {
        reverse_proxy localhost:${RELAY_PORT}
    }

    # SPA static files (client-side routing fallback)
    handle {
        root * ${DEPLOY_DIR}/web
        try_files {path} /index.html
        file_server
        encode gzip
    }
}
CADDY
    caddy validate --config /etc/caddy/Caddyfile
    info "Caddyfile validated"
}

# -------------------------------------------------------
# Start / restart services
# -------------------------------------------------------
restart_services() {
    header "Restarting services"
    systemctl restart ccbox-relay
    systemctl restart caddy
    sleep 2

    if systemctl is-active --quiet ccbox-relay; then
        info "ccbox-relay: active"
    else
        warn "ccbox-relay: FAILED -- check 'journalctl -u ccbox-relay'"
    fi
    if systemctl is-active --quiet caddy; then
        info "caddy: active"
    else
        warn "caddy: FAILED -- check 'journalctl -u caddy'"
    fi
}

# -------------------------------------------------------
# Firewall (ufw if present)
# -------------------------------------------------------
ensure_firewall() {
    if command -v ufw &>/dev/null && ufw status | grep -q "active"; then
        info "Ensuring firewall allows 80, 443, 22 ..."
        ufw allow 22/tcp  >/dev/null 2>&1 || true
        ufw allow 80/tcp  >/dev/null 2>&1 || true
        ufw allow 443/tcp >/dev/null 2>&1 || true
    fi
}

# -------------------------------------------------------
# Run
# -------------------------------------------------------
case "$MODE" in
    full)
        install_deps
        build_relay
        build_web
        configure_services
        ensure_firewall
        restart_services
        ;;
    update)
        export PATH="$HOME/.cargo/bin:$HOME/.bun/bin:$PATH"
        build_relay
        build_web
        restart_services
        ;;
    setup)
        install_deps
        configure_services
        ensure_firewall
        ;;
esac

echo ""
info "Deployment complete!"
info "  Relay health: https://${DOMAIN}/health"
info "  Web UI:       https://${DOMAIN}"
REMOTE

header "Done"
info "https://${DOMAIN}"
