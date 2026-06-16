#!/usr/bin/env bash
set -euo pipefail

echo "Uninstalling velo-de (requires sudo)…"
sudo make uninstall PREFIX=/usr/local
echo "Done."
