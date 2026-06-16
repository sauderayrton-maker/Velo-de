#!/usr/bin/env bash
set -euo pipefail

# install.sh — build and install velo-de + hyprctl shim to /usr/local
#
# Runtime deps (Arch): libseat libinput mesa libdrm libxkbcommon wayland
# All should already be present on a system running Hyprland.

for pkg in libseat libinput mesa libdrm libxkbcommon wayland; do
    if ! pacman -Qq "$pkg" &>/dev/null; then
        echo "Missing package: $pkg  →  sudo pacman -S $pkg"
    fi
done

echo "Building velo-de + hyprctl (release)…"
cargo build --release --bin velo-de --bin hyprctl

echo "Installing (requires sudo)…"
sudo make install PREFIX=/usr/local

echo ""
echo "Done. You should now see 'Velo' in SDDM's session list."
echo "Log out and select it, or test on a spare VT first:"
echo "  Ctrl+Alt+F3 → log in → unset WAYLAND_DISPLAY DISPLAY && RUST_LOG=info /usr/local/bin/velo-de"
