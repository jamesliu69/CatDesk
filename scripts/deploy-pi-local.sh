#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

if ! command -v bwrap >/dev/null 2>&1; then
	echo "Bubblewrap (bwrap) is required for Raspberry Pi command sandboxing." >&2
	echo "Install it with: sudo apt install bubblewrap" >&2
	exit 1
fi

CARGO_BIN="${CARGO_BIN:-$HOME/.cargo/bin/cargo}"
if [[ ! -x "$CARGO_BIN" ]]; then
	CARGO_BIN="$(command -v cargo || true)"
fi
if [[ -z "$CARGO_BIN" || ! -x "$CARGO_BIN" ]]; then
	echo "cargo was not found. Expected $HOME/.cargo/bin/cargo or cargo in PATH." >&2
	exit 1
fi

# Build into an isolated target directory. The running service must never execute
# target/release/catdesk because Cargo replaces that path during release builds.
DEPLOY_TARGET_DIR="$ROOT_DIR/target/catdesk-deploy"

echo "[1/6] Running release tests..."
"$CARGO_BIN" test --release

echo "[2/6] Building isolated release binary..."
"$CARGO_BIN" build --release --target-dir "$DEPLOY_TARGET_DIR"

GLOBAL_ROOT="$(npm root -g)"
PACKAGE_DIR="$GLOBAL_ROOT/catdesk"
DEST_DIR="$PACKAGE_DIR/npm/bin"
DEST_BIN="$DEST_DIR/catdesk"
BUILT_BIN="$DEPLOY_TARGET_DIR/release/catdesk"

if [[ ! -d "$PACKAGE_DIR" ]]; then
	echo "Global npm CatDesk package not found at: $PACKAGE_DIR" >&2
	exit 1
fi

mkdir -p "$DEST_DIR"
if [[ -f "$DEST_BIN" ]]; then
	cp -f "$DEST_BIN" "$DEST_BIN.bak"
fi

# Install by rename in the destination directory so an existing CatDesk process
# never observes a partially replaced executable.
INSTALL_TMP="$DEST_BIN.tmp.$$"
trap 'rm -f "$INSTALL_TMP"' EXIT
install -m 0755 "$BUILT_BIN" "$INSTALL_TMP"
mv -f "$INSTALL_TMP" "$DEST_BIN"
trap - EXIT

echo "[3/6] Installed CatDesk atomically to: $DEST_BIN"

WRAPPER_DIR="$HOME/.local/bin"
WRAPPER="$WRAPPER_DIR/catdesk-pi"
mkdir -p "$WRAPPER_DIR"
cat > "$WRAPPER" <<EOF
#!/usr/bin/env bash
set -euo pipefail
exec "$DEST_BIN" "\$@"
EOF
chmod 0755 "$WRAPPER"
echo "[4/6] Created Raspberry Pi launcher: $WRAPPER"

WORKSPACE_DIR="${CATDESK_WORKSPACE_ROOT:-$HOME/github}"
if [[ ! -d "$WORKSPACE_DIR" ]]; then
	echo "CatDesk workspace does not exist: $WORKSPACE_DIR" >&2
	exit 1
fi

SYSTEMD_USER_DIR="$HOME/.config/systemd/user"
SERVICE_FILE="$SYSTEMD_USER_DIR/catdesk.service"
mkdir -p "$SYSTEMD_USER_DIR"
cat > "$SERVICE_FILE" <<EOF
[Unit]
Description=CatDesk MCP headless service
Wants=network-online.target
After=network-online.target

[Service]
Type=simple
WorkingDirectory=$WORKSPACE_DIR
Environment=WORKSPACE_ROOT=$WORKSPACE_DIR
Environment=PORT=3200
Environment=PATH=$HOME/.cargo/bin:$HOME/.local/bin:$HOME/.npm-global/bin:/usr/local/bin:/usr/bin:/bin
ExecStart=$DEST_BIN --headless
Restart=on-failure
RestartSec=5
TimeoutStopSec=20

[Install]
WantedBy=default.target
EOF

echo "[5/6] Installed direct systemd user service: $SERVICE_FILE"

systemctl --user daemon-reload
systemctl --user enable catdesk.service >/dev/null

echo "[6/6] Restarting CatDesk under systemd supervision..."
systemctl --user restart catdesk.service

echo "Deployment complete. CatDesk is now supervised directly by systemd."
echo "Workspace: $WORKSPACE_DIR"
echo "Service: systemctl --user status catdesk.service"
echo "Linux commands are sandboxed with Bubblewrap on Raspberry Pi; CatDesk fails closed if no supported sandbox backend is available."
