#!/usr/bin/env bash
# Quick Phase 1 validation script (requires cosmic-ext-bg-ctl)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if ! command -v cosmic-ext-bg-ctl >/dev/null 2>&1; then
  echo "cosmic-ext-bg-ctl not found. Run scripts/install-prototype.sh first."
  exit 1
fi

echo "Stopping stock cosmic-bg..."
systemctl --user stop com.system76.CosmicBackground.service 2>/dev/null || true
pkill cosmic-bg 2>/dev/null || true
pkill cosmic-ext-bg 2>/dev/null || true

echo "Testing Plasma preset @ 30 FPS..."
cosmic-ext-bg-ctl shader Plasma --fps 30
sleep 3

echo "Testing Waves preset @ 60 FPS..."
cosmic-ext-bg-ctl shader Waves --fps 60
sleep 3

echo "Testing custom prototype shader..."
cosmic-ext-bg-ctl shader "$ROOT/shaders/pulse-aurora-prototype.wgsl" --fps 30

echo "Done. Check desktop visually and monitor thermals on battery if applicable."
