#!/usr/bin/env bash
# Install build dependencies on Pop!_OS 24.04 (COSMIC)
set -euo pipefail

sudo apt-get update
sudo apt-get install -y \
  build-essential pkg-config mold just \
  libpipewire-0.3-dev libspa-0.2-dev \
  libwayland-dev libxkbcommon-dev libvulkan-dev \
  libpulse-dev

if ! command -v rustc >/dev/null 2>&1; then
  echo "Installing Rust via rustup..."
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
  # shellcheck disable=SC1090
  source "$HOME/.cargo/env"
fi

echo "Dependencies installed."
