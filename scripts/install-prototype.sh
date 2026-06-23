#!/usr/bin/env bash
# Phase 1: Install and test cosmic-ext-bg prototype on Pop!_OS COSMIC
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROTOTYPE_DIR="${XDG_CACHE_HOME:-$HOME/.cache}/cosmic-ext-bg"

echo "==> Phase 1 prototype: cosmic-ext-bg"
echo "This validates WGSL shaders and COSMIC layer-shell compatibility before the custom daemon."

if ! command -v just >/dev/null 2>&1; then
  echo "Run scripts/install-deps.sh first."
  exit 1
fi

sudo apt-get install -y \
  libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev \
  gstreamer1.0-plugins-good gstreamer1.0-plugins-bad

if [[ ! -d "$PROTOTYPE_DIR/.git" ]]; then
  git clone --depth 1 https://github.com/olafkfreund/cosmic-ext-bg.git "$PROTOTYPE_DIR"
fi

cd "$PROTOTYPE_DIR"
if ! just build-release; then
  echo ""
  echo "WARNING: cosmic-ext-bg failed to build (known issue: jxl-bitstream needs newer Rust features)."
  echo "Phase 1 prototype shaders are still available in $ROOT/shaders/pulse-aurora-prototype.wgsl"
  echo "Skip to Phase 2: ./scripts/install-service.sh && cosmic-audio-bg"
  exit 0
fi
sudo just install
sudo just install-ctl

echo ""
echo "Prototype installed. Test built-in shaders:"
echo "  cosmic-ext-bg-ctl shader Plasma --fps 30"
echo "  cosmic-ext-bg-ctl shader Waves --fps 60"
echo ""
echo "Test custom Phase 1 shader (simulated audio pulses):"
echo "  cosmic-ext-bg-ctl shader $ROOT/shaders/pulse-aurora-prototype.wgsl --fps 30"
echo ""
echo "Note: cosmic-ext-bg uses a smaller uniform struct (time/resolution only)."
echo "The prototype shader ignores real audio; the custom daemon adds PipeWire FFT."
echo ""
echo "Stop stock background before testing:"
echo "  systemctl --user stop com.system76.CosmicBackground.service 2>/dev/null || true"
echo "  pkill cosmic-bg 2>/dev/null || true"
