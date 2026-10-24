#!/usr/bin/env bash
#
# Install a *released* Kakiri into your user account — no sudo, no Rust/Node
# toolchain, nothing written outside $PREFIX (default ~/.local).
#
#   ./install-release.sh                 # latest release, user-level
#   ./install-release.sh --version v0.1.1
#   ./install-release.sh --appimage      # force the AppImage instead of the .deb
#   ./install-release.sh --prefix ~/.local
#
# Why the .deb and not the AppImage by default: the .deb is ~7 MB and already
# contains the binary, the desktop entry and every icon size, so unpacking it is
# the smallest correct install. The AppImage is ~82 MB because it bundles GTK and
# WebKit, and is used when dpkg-deb is not available (non-Debian systems).
#
# Only the listed artifact is downloaded, and it is verified against the release's
# SHA256SUMS before anything is installed.
#
set -euo pipefail

SELF="$(readlink -f "${BASH_SOURCE[0]}")"
ROOT="$(cd "$(dirname "$SELF")/.." && pwd)" # only used for --help context
REPO="${KAKIRI_REPO:-0xwagmi/Kakiri}"
PREFIX="${XDG_DATA_HOME:+$(dirname "$XDG_DATA_HOME")}"
PREFIX="${PREFIX:-$HOME/.local}"
VERSION=""
FORCE_APPIMAGE=0
API="https://api.github.com/repos/$REPO/releases"

die() { printf '\033[31m%s\033[0m\n' "$*" >&2; exit 1; }
info() { printf '\033[36m▸\033[0m %s\n' "$*"; }
ok() { printf '\033[32m✓\033[0m %s\n' "$*"; }
warn() { printf '\033[33m!\033[0m %s\n' "$*"; }

while [ $# -gt 0 ]; do
  case "$1" in
    --version) shift; VERSION="${1:?--version needs a tag like v0.1.1}" ;;
    --prefix) shift; PREFIX="${1:?--prefix needs a directory}" ;;
    --appimage) FORCE_APPIMAGE=1 ;;
    -h|--help) sed -n '2,/^set -euo/p' "$SELF" | sed 's/^# \{0,1\}//' | sed '$d'; exit 0 ;;
    *) die "unknown option: $1 (try --help)" ;;
  esac
  shift
done

command -v curl >/dev/null || die "curl is required"
command -v sha256sum >/dev/null || die "sha256sum is required"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# --- pick the release ---------------------------------------------------------
if [ -z "$VERSION" ]; then
  info "looking up the latest release"
  # releases/latest skips pre-releases, and 0.x is a pre-release, so list them
  VERSION="$(curl -fsSL "$API?per_page=1" |
    sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -1)"
  [ -n "$VERSION" ] || die "could not find a release of $REPO"
fi
ok "release: $VERSION"
BASE="https://github.com/$REPO/releases/download/$VERSION"

# --- choose the artifact ------------------------------------------------------
arch="$(uname -m)"
case "$arch" in
  x86_64|amd64) suffix="amd64" ;;
  *) die "no prebuilt release for '$arch' yet (only x86_64)" ;;
esac

use_deb=1
if [ "$FORCE_APPIMAGE" = "1" ] || ! command -v dpkg-deb >/dev/null; then
  use_deb=0
fi

if [ "$use_deb" = "1" ]; then
  artifact="Kakiri_${VERSION#v}_${suffix}.deb"
else
  artifact="Kakiri_${VERSION#v}_${suffix}.AppImage"
fi

info "downloading $artifact"
curl -fL --progress-bar "$BASE/$artifact" -o "$tmp/$artifact"
curl -fsSL "$BASE/SHA256SUMS" -o "$tmp/SHA256SUMS"

# --- verify what we got -------------------------------------------------------
expected="$(grep "  $artifact\$" "$tmp/SHA256SUMS" | cut -d' ' -f1)"
[ -n "$expected" ] || die "$artifact is not listed in SHA256SUMS — refusing to install"
actual="$(sha256sum "$tmp/$artifact" | cut -d' ' -f1)"
[ "$expected" = "$actual" ] || die "checksum mismatch for $artifact"
ok "checksum verified"

# --- install ------------------------------------------------------------------
BIN_DIR="$PREFIX/bin"
APP_DIR="$PREFIX/share/applications"
ICON_ROOT="$PREFIX/share/icons/hicolor"
BIN="$BIN_DIR/kakiri"
mkdir -p "$BIN_DIR" "$APP_DIR"

if [ "$use_deb" = "1" ]; then
  dpkg-deb -x "$tmp/$artifact" "$tmp/root"
  src_bin="$(find "$tmp/root" -path '*/bin/*' -type f | head -1)"
  [ -n "$src_bin" ] || die "no binary inside $artifact"
  install -m755 "$src_bin" "$BIN"
  # icons are already in every size inside the package
  while IFS= read -r icon; do
    size="$(basename "$(dirname "$(dirname "$icon")")")"
    mkdir -p "$ICON_ROOT/$size/apps"
    install -m644 "$icon" "$ICON_ROOT/$size/apps/kakiri.png"
  done < <(find "$tmp/root" -path '*icons/hicolor/*/apps/*.png' 2>/dev/null)
else
  install -m755 "$tmp/$artifact" "$BIN"
  if [ -d "$ROOT/src-tauri/icons" ]; then
    for pair in 32:32x32.png 64:64x64.png 128:128x128.png 256:128x128@2x.png 512:icon.png; do
      size="${pair%%:*}"; src="$ROOT/src-tauri/icons/${pair#*:}"
      [ -f "$src" ] || continue
      mkdir -p "$ICON_ROOT/${size}x${size}/apps"
      install -m644 "$src" "$ICON_ROOT/${size}x${size}/apps/kakiri.png"
    done
  fi
fi
ok "binary: $BIN"

# Ask the loader what it actually cannot find, instead of guessing library names.
# (A user-level install cannot pull in system packages, so say so up front.)
if command -v ldd >/dev/null; then
  missing="$(ldd "$BIN" 2>/dev/null | awk '/not found/ {print $1}' | sort -u | tr '\n' ' ' || true)"
  if [ -n "${missing// /}" ]; then
    warn "these libraries are missing: $missing"
    warn "install them first, e.g.:"
    warn "  sudo apt install libwebkit2gtk-4.1-0 libgtk-3-0 pipewire"
  fi
fi

cat >"$APP_DIR/kakiri.desktop" <<EOF
[Desktop Entry]
Type=Application
Version=1.0
Name=Kakiri
GenericName=Soundboard
Comment=Play sounds through a virtual microphone in Discord
Exec=$BIN
Icon=kakiri
Terminal=false
Categories=AudioVideo;Audio;Mixer;
Keywords=soundboard;audio;mixer;discord;mic;voice;
StartupNotify=true
StartupWMClass=kakiri
EOF
chmod 644 "$APP_DIR/kakiri.desktop"
ok "desktop entry: $APP_DIR/kakiri.desktop"

command -v update-desktop-database >/dev/null && \
  update-desktop-database "$APP_DIR" >/dev/null 2>&1 || true
command -v gtk-update-icon-cache >/dev/null && \
  gtk-update-icon-cache -q -t -f "$ICON_ROOT" >/dev/null 2>&1 || true

echo
ok "Kakiri $VERSION installed (user-level, no root)"
echo "  launch:    kakiri        (or \"Kakiri\" in the application menu)"
case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *) warn "$BIN_DIR is not on your PATH — add: export PATH=\"$BIN_DIR:\$PATH\"" ;;
esac
echo "  remove:    rm -f $BIN $APP_DIR/kakiri.desktop && rm -rf $PREFIX/share/icons/hicolor/*/apps/kakiri.png"
