#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

echo "[1/4] Running tests..."
cargo test

echo "[2/4] Building release binary..."
cargo build --release

GLOBAL_ROOT="$(npm root -g)"
PACKAGE_DIR="$GLOBAL_ROOT/catdesk"
DEST_DIR="$PACKAGE_DIR/npm/bin"
DEST_BIN="$DEST_DIR/catdesk"

if [[ ! -d "$PACKAGE_DIR" ]]; then
	echo "Global npm CatDesk package not found at: $PACKAGE_DIR" >&2
	exit 1
fi

mkdir -p "$DEST_DIR"
if [[ -f "$DEST_BIN" ]]; then
	cp -f "$DEST_BIN" "$DEST_BIN.bak"
fi

install -m 0755 "$ROOT_DIR/target/release/catdesk" "$DEST_BIN"

echo "[3/4] Installed local CatDesk binary to: $DEST_BIN"

WRAPPER_DIR="$HOME/.local/bin"
WRAPPER="$WRAPPER_DIR/catdesk-pi"
mkdir -p "$WRAPPER_DIR"
cat > "$WRAPPER" <<EOF
#!/usr/bin/env bash
export CATDESK_ALLOW_UNSANDBOXED_LINUX=1
exec node "$PACKAGE_DIR/npm/catdesk.js" "\$@"
EOF
chmod 0755 "$WRAPPER"

echo "[4/4] Created Raspberry Pi launcher: $WRAPPER"
echo
echo "Deployment complete. Stop the currently running CatDesk, then launch with:"
echo "  cd <workspace>"
echo "  WORKSPACE_ROOT=\"\$PWD\" $WRAPPER"
echo
echo "WARNING: catdesk-pi explicitly permits command execution without Landlock kernel filesystem isolation."
