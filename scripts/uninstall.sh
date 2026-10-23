#!/usr/bin/env bash
#
# Remove Kakiri from your user account.
#
#   ./scripts/uninstall.sh            # remove the app, icons and menu entry
#   ./scripts/uninstall.sh --purge    # also delete library settings and tools
#   ./scripts/uninstall.sh --prefix DIR
#
# --purge deletes the app's data directory (~/.local/share/com.kakiri.soundboard):
# window preferences, the library folder list, the timeline, and the downloader
# (yt-dlp) that the import panel fetched. Your sound files are never touched.
#
set -euo pipefail

SELF="$(readlink -f "${BASH_SOURCE[0]}")"
ROOT="$(cd "$(dirname "$SELF")/.." && pwd)"

PREFIX="${XDG_DATA_HOME:+$(dirname "$XDG_DATA_HOME")}"
PREFIX="${PREFIX:-$HOME/.local}"
PURGE=0
ASSUME_YES=0

info() { printf '\033[36m▸\033[0m %s\n' "$*"; }
ok() { printf '\033[32m✓\033[0m %s\n' "$*"; }
warn() { printf '\033[33m!\033[0m %s\n' "$*"; }

while [ $# -gt 0 ]; do
  case "$1" in
    --prefix) shift; PREFIX="${1:?--prefix needs a directory}" ;;
    --purge) PURGE=1 ;;
    -y|--yes) ASSUME_YES=1 ;;
    -h|--help) sed -n '2,/^set -euo/p' "$SELF" | sed 's/^# \{0,1\}//' | sed '$d'; exit 0 ;;
    *) echo "unknown option: $1 (try --help)" >&2; exit 2 ;;
  esac
  shift
done

BIN_DIR="$PREFIX/bin"
APP_DIR="$PREFIX/share/applications"
ICON_ROOT="$PREFIX/share/icons/hicolor"
DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/com.kakiri.soundboard"

confirm() {
  [ "$ASSUME_YES" = "1" ] && return 0
  printf '%s [y/N] ' "$1"
  read -r answer
  case "$answer" in [yY]*) return 0 ;; *) return 1 ;; esac
}

removed=0
if [ -e "$BIN_DIR/kakiri" ] || [ -L "$BIN_DIR/kakiri" ]; then
  rm -f "$BIN_DIR/kakiri"
  ok "removed $BIN_DIR/kakiri"
  removed=1
fi

if [ -e "$APP_DIR/kakiri.desktop" ]; then
  rm -f "$APP_DIR/kakiri.desktop"
  ok "removed $APP_DIR/kakiri.desktop"
  removed=1
fi

for size in 32 64 128 256 512; do
  icon="$ICON_ROOT/${size}x${size}/apps/kakiri.png"
  if [ -e "$icon" ]; then
    rm -f "$icon"
    ok "removed $icon"
    removed=1
  fi
done

command -v update-desktop-database >/dev/null && \
  update-desktop-database "$APP_DIR" >/dev/null 2>&1 || true

if [ "$PURGE" = "1" ]; then
  if [ -d "$DATA_DIR" ]; then
    if confirm "Delete app data (settings, library list, downloaded tools) in $DATA_DIR?"; then
      rm -rf "$DATA_DIR"
      ok "removed $DATA_DIR"
      removed=1
    else
      warn "kept $DATA_DIR"
    fi
  fi
elif [ -d "$DATA_DIR" ]; then
  echo
  info "your settings and tools are still in $DATA_DIR"
  echo "  remove them as well with: ./scripts/uninstall.sh --purge"
fi

if [ "$removed" = "0" ]; then
  warn "nothing was installed in $PREFIX"
else
  echo
  ok "Kakiri removed"
  echo "  (the repository itself is untouched: $ROOT)"
fi
