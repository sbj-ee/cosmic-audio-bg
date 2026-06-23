#!/usr/bin/env bash
# Deploy cosmic-audio-bg to a second Pop!_OS laptop via rsync + remote install.
# Usage: ./scripts/deploy-second-machine.sh user@other-laptop
set -euo pipefail

if [[ $# -lt 1 ]]; then
  echo "Usage: $0 user@hostname"
  echo "Example: $0 steve@laptop-b.local"
  exit 1
fi

TARGET="$1"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REMOTE_DIR="~/cosmic-audio-bg"

echo "==> Syncing $ROOT to $TARGET:$REMOTE_DIR"
rsync -avz --delete \
  --exclude target \
  --exclude .git \
  "$ROOT/" "$TARGET:$REMOTE_DIR/"

echo "==> Running remote install"
ssh "$TARGET" "cd $REMOTE_DIR && ./scripts/install-deps.sh && ./scripts/install-service.sh"

HOST="$(echo "$TARGET" | cut -d@ -f2 | cut -d. -f1 | tr '[:upper:]' '[:lower:]')"
echo ""
echo "Deploy complete. On the remote machine:"
echo "  1. Optional: edit $REMOTE_DIR/config/machines/${HOST}.ron (copy from laptop-a.ron or laptop-b.ron)"
echo "  2. systemctl --user disable --now com.system76.CosmicBackground.service 2>/dev/null || true"
echo "  3. systemctl --user daemon-reload && systemctl --user enable --now cosmic-audio-bg.service"
