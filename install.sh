#!/usr/bin/env bash
set -euo pipefail

BOLD='\033[1m'
DIM='\033[2m'
BLUE='\033[34m'
CYAN='\033[36m'
GREEN='\033[32m'
YELLOW='\033[33m'
RED='\033[31m'
RESET='\033[0m'

banner()  { echo -e "\n${BOLD}${BLUE}━━━  $*  ━━━${RESET}\n"; }
header()  { echo -e "\n${BOLD}${CYAN}  $*${RESET}\n"; }
ok()      { echo -e "  ${GREEN}✓${RESET}  $*"; }
err()     { echo -e "  ${RED}✗${RESET}  $*"; }
info()    { echo -e "  ${DIM}→${RESET}  $*"; }
warn()    { echo -e "  ${YELLOW}!${RESET}  $*"; }
skip()    { echo -e "  ${DIM}–${RESET}  $* ${DIM}(skipped)${RESET}"; }

# ── Apps ─────────────────────────────────────────────────────────────────────

declare -A REPOS=(
  [velo-shell]="https://github.com/sauderayrton-maker/Velo-shell.git"
  [velo-launcher]="https://github.com/sauderayrton-maker/Velo-launcher.git"
  [velo-osd]="https://github.com/sauderayrton-maker/velo-osd.git"
  [velo-files]="https://github.com/sauderayrton-maker/Velo-files.git"
  [velo-browser]="https://github.com/sauderayrton-maker/Velo-Browser.git"
  [velo-player]="https://github.com/sauderayrton-maker/Velo-player.git"
  [velo-assistant]="https://github.com/sauderayrton-maker/velo-assistant.git"
  [velo-paper]="https://github.com/sauderayrton-maker/velo-paper.git"
  [velo-commit]="https://github.com/sauderayrton-maker/velo-commit.git"
)

# Ordered install sequence (shell/bar first, then utilities, then optional)
INSTALL_ORDER=(
  velo-shell
  velo-launcher
  velo-osd
  velo-files
  velo-browser
  velo-player
  velo-assistant
  velo-paper
  velo-commit
)

# ── Parse flags ───────────────────────────────────────────────────────────────

SKIP_APPS=()
SELECT_APPS=()

for arg in "$@"; do
  case "$arg" in
    --skip=*) SKIP_APPS+=("${arg#--skip=}") ;;
    --only=*) SELECT_APPS+=("${arg#--only=}") ;;
    --help|-h)
      echo ""
      echo -e "${BOLD}  Velo DE — Full installer${RESET}"
      echo ""
      echo "  Usage:  bash install.sh [options]"
      echo ""
      echo "  Options:"
      echo "    --skip=<app>    Skip a specific app (repeatable)"
      echo "    --only=<app>    Install only this app (repeatable)"
      echo "    --help          Show this message"
      echo ""
      echo "  Apps: ${INSTALL_ORDER[*]}"
      echo ""
      exit 0
      ;;
  esac
done

should_install() {
  local app="$1"
  for s in "${SKIP_APPS[@]:-}"; do [[ "$s" == "$app" ]] && return 1; done
  if [[ ${#SELECT_APPS[@]} -gt 0 ]]; then
    for s in "${SELECT_APPS[@]}"; do [[ "$s" == "$app" ]] && return 0; done
    return 1
  fi
  return 0
}

# ── Header ────────────────────────────────────────────────────────────────────

echo ""
echo -e "${BOLD}${BLUE}"
echo "  ██╗   ██╗███████╗██╗      ██████╗     ██████╗ ███████╗"
echo "  ██║   ██║██╔════╝██║     ██╔═══██╗    ██╔══██╗██╔════╝"
echo "  ██║   ██║█████╗  ██║     ██║   ██║    ██║  ██║█████╗  "
echo "  ╚██╗ ██╔╝██╔══╝  ██║     ██║   ██║    ██║  ██║██╔══╝  "
echo "   ╚████╔╝ ███████╗███████╗╚██████╔╝    ██████╔╝███████╗"
echo "    ╚═══╝  ╚══════╝╚══════╝ ╚═════╝     ╚═════╝ ╚══════╝"
echo -e "${RESET}"
echo -e "  ${DIM}A glass-and-steel desktop environment for Wayland${RESET}"
echo ""

# ── Preflight ─────────────────────────────────────────────────────────────────

banner "Preflight"

# Git
if ! command -v git &>/dev/null; then
  err "git not found — install it first"
  exit 1
fi
ok "git $(git --version | cut -d' ' -f3)"

# Rust
if ! command -v cargo &>/dev/null; then
  info "Rust not found — installing via rustup..."
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path
  # shellcheck source=/dev/null
  source "$HOME/.cargo/env"
fi
ok "Rust $(rustc --version | cut -d' ' -f2)"

# Package manager
PM=""
if command -v pacman &>/dev/null;  then PM="pacman"
elif command -v apt-get &>/dev/null; then PM="apt"
elif command -v dnf &>/dev/null;   then PM="dnf"
fi
[[ -n "$PM" ]] && ok "Package manager: $PM" || warn "Unknown package manager — you may need to install deps manually"

# ── Work dir ──────────────────────────────────────────────────────────────────

WORKDIR="$(mktemp -d /tmp/velo-install.XXXXXX)"
trap 'rm -rf "$WORKDIR"' EXIT

# ── Install each app ──────────────────────────────────────────────────────────

FAILED=()

install_with_script() {
  local name="$1" dir="$2"
  bash "$dir/install.sh"
}

install_with_make() {
  local name="$1" dir="$2"
  (cd "$dir" && cargo build --release && sudo make install PREFIX=/usr/local)
  ok "$name installed"
}

install_cargo_only() {
  local name="$1" dir="$2"
  (cd "$dir" && cargo build --release && sudo install -Dm755 "target/release/$name" "/usr/local/bin/$name")
  ok "$name  →  /usr/local/bin/$name"
}

for app in "${INSTALL_ORDER[@]}"; do
  if ! should_install "$app"; then
    skip "$app"
    continue
  fi

  url="${REPOS[$app]}"
  dest="$WORKDIR/$app"

  header "$app"
  info "Cloning $url"

  if ! git clone --depth 1 "$url" "$dest" 2>&1 | sed 's/^/    /'; then
    err "Failed to clone $app"
    FAILED+=("$app")
    continue
  fi

  set +e
  case "$app" in
    velo-paper)
      install_with_make "$app" "$dest"
      ;;
    velo-commit)
      install_cargo_only "$app" "$dest"
      ;;
    *)
      install_with_script "$app" "$dest"
      ;;
  esac
  exit_code=$?
  set -e

  if [[ $exit_code -ne 0 ]]; then
    err "$app install failed (exit $exit_code)"
    FAILED+=("$app")
  fi
done

# ── Summary ───────────────────────────────────────────────────────────────────

banner "Done"

if [[ ${#FAILED[@]} -eq 0 ]]; then
  echo -e "  ${BOLD}${GREEN}All Velo apps installed successfully.${RESET}"
  echo ""
  echo -e "  Installed:"
  for app in "${INSTALL_ORDER[@]}"; do
    should_install "$app" && echo -e "    ${GREEN}✓${RESET}  $app"
  done
else
  echo -e "  ${YELLOW}Installed with errors:${RESET}"
  for app in "${INSTALL_ORDER[@]}"; do
    if should_install "$app"; then
      failed=false
      for f in "${FAILED[@]}"; do [[ "$f" == "$app" ]] && failed=true; done
      if $failed; then
        echo -e "    ${RED}✗${RESET}  $app"
      else
        echo -e "    ${GREEN}✓${RESET}  $app"
      fi
    fi
  done
  echo ""
  echo -e "  Re-run with ${DIM}--only=<app>${RESET} to retry a specific app."
fi

echo ""
echo -e "  ${DIM}Next: configure niri or Hyprland to launch velo-shell on startup.${RESET}"
echo ""
