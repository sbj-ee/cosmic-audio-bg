#!/usr/bin/env bash
# Build and install cosmic-audio-bg daemon + configs
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck disable=SC1090
source "${HOME}/.cargo/env" 2>/dev/null || true

echo "==> Building cosmic-audio-bg"
cd "$ROOT"
cargo build --release -p cosmic-audio-bg

INSTALL_BIN="${INSTALL_BIN:-$HOME/.local/bin}"
INSTALL_SHARE="${INSTALL_SHARE:-$HOME/.local/share/cosmic-audio-bg}"
INSTALL_CONFIG="${INSTALL_CONFIG:-$HOME/.config/cosmic-audio-bg}"

mkdir -p "$INSTALL_BIN" "$INSTALL_SHARE/shaders" "$INSTALL_CONFIG/machines"

install -m 755 "$ROOT/target/release/cosmic-audio-bg" "$INSTALL_BIN/cosmic-audio-bg"
cp -r "$ROOT/shaders/"*.wgsl "$INSTALL_SHARE/shaders/"
cp "$ROOT/config/default.ron" "$INSTALL_CONFIG/config.ron"
cp "$ROOT/config/machines/"*.ron "$INSTALL_CONFIG/machines/" 2>/dev/null || true

# Point config at installed shaders
SHADER_PATH="$INSTALL_SHARE/shaders/sinusoids.wgsl"
sed -i "s|shader_path: \".*\"|shader_path: \"$SHADER_PATH\"|" "$INSTALL_CONFIG/config.ron"

# Patch config to use installed shader path
HOSTNAME="$(hostname | tr '[:upper:]' '[:lower:]')"
MACHINE_OVERRIDE="$INSTALL_CONFIG/machines/${HOSTNAME}.ron"
if [[ -f "$MACHINE_OVERRIDE" ]]; then
  echo "Using machine override: $MACHINE_OVERRIDE"
else
  echo "No machine override for '$HOSTNAME' — using default config."
  echo "Copy config/machines/laptop-a.ron to $MACHINE_OVERRIDE to customize."
fi

echo ""
echo "Installed:"
echo "  binary:  $INSTALL_BIN/cosmic-audio-bg"
echo "  shaders: $INSTALL_SHARE/shaders/"
echo "  config:  $INSTALL_CONFIG/config.ron"
echo ""
echo "Run manually:"
echo "  COSMIC_AUDIO_BG_SHADER=$INSTALL_SHARE/shaders/sinusoids.wgsl \\"
echo "    $INSTALL_BIN/cosmic-audio-bg --config $INSTALL_CONFIG/config.ron"
echo ""
echo "==> Installing systemd user service"

SERVICE_SRC="$ROOT/systemd/cosmic-audio-bg.service"
SERVICE_DST="$HOME/.config/systemd/user/cosmic-audio-bg.service"
mkdir -p "$HOME/.config/systemd/user"

# Generate service file with correct paths
sed \
  -e "s|%h/.local/bin/cosmic-audio-bg|$INSTALL_BIN/cosmic-audio-bg|g" \
  -e "s|%h/.config/cosmic-audio-bg/config.ron|$INSTALL_CONFIG/config.ron|g" \
  -e "s|%h/cosmic-audio-bg|$ROOT|g" \
  "$SERVICE_SRC" > "$SERVICE_DST"

echo "Service written to $SERVICE_DST"
echo ""
echo "Coexistence with cosmic-bg:"
echo "  Only one background client should run at a time."
echo "  To use cosmic-audio-bg at login:"
echo "    systemctl --user disable --now com.system76.CosmicBackground.service 2>/dev/null || true"
echo "    systemctl --user daemon-reload"
echo "    systemctl --user enable --now cosmic-audio-bg.service"
echo ""
echo "  To restore stock wallpaper:"
echo "    systemctl --user disable --now cosmic-audio-bg.service"
echo "    systemctl --user enable --now com.system76.CosmicBackground.service"
