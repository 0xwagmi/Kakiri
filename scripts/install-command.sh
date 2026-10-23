#!/usr/bin/env bash
#
# Install a `kakiri` command that runs the app from anywhere.
#
# Creates a symlink in ~/.local/bin (no sudo, no copying). Re-run it after
# moving the repository, since the link points at this checkout.
#
set -euo pipefail

SELF="$(readlink -f "${BASH_SOURCE[0]}")"
ROOT="$(cd "$(dirname "$SELF")/.." && pwd)"
TARGET="$ROOT/scripts/run.sh"
BIN_DIR="${XDG_BIN_HOME:-$HOME/.local/bin}"
LINK="$BIN_DIR/kakiri"

[ -x "$TARGET" ] || { echo "missing $TARGET" >&2; exit 1; }

mkdir -p "$BIN_DIR"
if [ -e "$LINK" ] && [ ! -L "$LINK" ]; then
  echo "$LINK already exists and is not a symlink — remove it first" >&2
  exit 1
fi
ln -sfn "$TARGET" "$LINK"

echo "installed: $LINK -> $TARGET"
case ":$PATH:" in
  *":$BIN_DIR:"*) echo "run it with:  kakiri" ;;
  *)
    echo
    echo "$BIN_DIR is not on your PATH. Add this to ~/.bashrc (or ~/.zshrc):"
    echo "  export PATH=\"$BIN_DIR:\$PATH\""
    ;;
esac
echo
echo "  kakiri                 # release engine + hot-reloading UI"
echo "  kakiri --rebuild       # rebuild the Rust side first"
echo "  kakiri --server-only   # just the dev server"
