#!/usr/bin/env bash
#
# Install Kakiri into your user account (no sudo, no root files touched).
#
#   ./scripts/install.sh              # build a standalone app and install it
#   ./scripts/install.sh --prefix DIR # install somewhere else (default ~/.local)
#   ./scripts/install.sh --dev        # install the dev runner instead (Vite + release)
#   ./scripts/install.sh --uninstall  # remove it again
#
# Installs: $PREFIX/bin/kakiri, $PREFIX/share/applications/kakiri.desktop and the
# icon in every size under $PREFIX/share/icons/hicolor, so it shows up in the
# application menu like any other program.
#
set -euo pipefail

SELF="$(readlink -f "${BASH_SOURCE[0]}")"
ROOT="$(cd "$(dirname "$SELF")/.." && pwd)"
cd "$ROOT"

PREFIX="${XDG_DATA_HOME:+$(dirname "$XDG_DATA_HOME")}"
PREFIX="${PREFIX:-$HOME/.local}"
MODE="release"
ASSUME_YES=0

die() { printf '\033[31m%s\033[0m\n' "$*" >&2; exit 1; }
info() { printf '\033[36m▸\033[0m %s\n' "$*"; }
ok() { printf '\033[32m✓\033[0m %s\n' "$*"; }

while [ $# -gt 0 ]; do
  case "$1" in
    --prefix) shift; PREFIX="${1:?--prefix needs a directory}" ;;
    --dev) MODE="dev" ;;
    --uninstall) exec "$ROOT/scripts/uninstall.sh" ${PREFIX:+--prefix "$PREFIX"} ;;
    -y|--yes) ASSUME_YES=1 ;;
    -h|--help) sed -n '2,/^set -euo/p' "$SELF" | sed 's/^# \{0,1\}//' | sed '$d'; exit 0 ;;
    *) die "unknown option: $1 (try --help)" ;;
  esac
  shift
done

BIN_DIR="$PREFIX/bin"
APP_DIR="$PREFIX/share/applications"
ICON_ROOT="$PREFIX/share/icons/hicolor"
BIN="$BIN_DIR/kakiri"

install_desktop_entry() {
  local exec_path="$1"
  mkdir -p "$APP_DIR"
  cat >"$APP_DIR/kakiri.desktop" <<EOF
[Desktop Entry]
Type=Application
Version=1.0
Name=Kakiri
GenericName=Soundboard
Comment=Play sounds through a virtual microphone in Discord
Exec=$exec_path
Icon=kakiri
Terminal=false
Categories=AudioVideo;Audio;Mixer;
Keywords=soundboard;audio;mixer;discord;mic;voice;
StartupNotify=true
StartupWMClass=kakiri
EOF
  chmod 644 "$APP_DIR/kakiri.desktop"
  ok "desktop entry: $APP_DIR/kakiri.desktop"
}

install_icons() {
  # one file per size, so every desktop scales it properly
  local pairs="32:src-tauri/icons/32x32.png 64:src-tauri/icons/64x64.png \
128:src-tauri/icons/128x128.png 256:src-tauri/icons/128x128@2x.png 512:src-tauri/icons/icon.png"
  local pair size src dir
  for pair in $pairs; do
    size="${pair%%:*}"
    src="${pair#*:}"
    [ -f "$src" ] || continue
    dir="$ICON_ROOT/${size}x${size}/apps"
    mkdir -p "$dir"
    install -m644 "$src" "$dir/kakiri.png"
  done
  ok "icons: $ICON_ROOT/<size>/apps/kakiri.png"
}

refresh_caches() {
  command -v update-desktop-database >/dev/null && \
    update-desktop-database "$APP_DIR" >/dev/null 2>&1 || true
  command -v gtk-update-icon-cache >/dev/null && \
    gtk-update-icon-cache -q -t -f "$ICON_ROOT" >/dev/null 2>&1 || true
}

if [ "$MODE" = "dev" ]; then
  # A link to the dev runner: Vite serves the UI, the release engine plays audio.
  command -v npm >/dev/null || die "npm is not installed"
  mkdir -p "$BIN_DIR"
  ln -sfn "$ROOT/scripts/run.sh" "$BIN"
  install_icons
  install_desktop_entry "$BIN"
  refresh_caches
  ok "installed (dev mode): $BIN -> $ROOT/scripts/run.sh"
  echo "  start it with: kakiri            # or from the application menu"
  echo "  uninstall with: ./scripts/install.sh --uninstall"
  exit 0
fi

# --- standalone build ---------------------------------------------------------
command -v npm >/dev/null || die "npm is not installed"
command -v cargo >/dev/null || die "cargo is not installed"

info "building the standalone app (this compiles the frontend and the release binary)"
# --no-bundle: we install the binary ourselves, so no .deb/AppImage is produced.
npm run tauri build -- --no-bundle

BUILT="$ROOT/target/release/soundboard"
[ -f "$BUILT" ] || die "the build finished but $BUILT does not exist"

info "installing to $PREFIX"
mkdir -p "$BIN_DIR"
install -m755 "$BUILT" "$BIN"
ok "binary: $BIN"
install_icons
install_desktop_entry "$BIN"
refresh_caches

echo
ok "Kakiri is installed"
echo "  launch it with: kakiri        (or find \"Kakiri\" in your application menu)"
case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *) echo "  note: $BIN_DIR is not on your PATH — add it to use the command" ;;
esac
echo "  uninstall with: ./scripts/install.sh --uninstall"
