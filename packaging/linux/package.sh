#!/bin/sh
# Builds the Rust release binary and packs it with install.sh, the desktop entry and the icon:
#   packaging/linux/package.sh            -> dist/retrograde-<version>-linux-<arch>.tar.gz
#   SKIP_BUILD=1 packaging/linux/package.sh   (reuse target/release/retrograde-rs)
set -eu
HERE=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
ROOT=$(CDPATH= cd -- "$HERE/../.." && pwd)
cd "$ROOT"

if [ "${SKIP_BUILD:-0}" != "1" ]; then
  cargo build --release --locked -p retrograde-app
fi
BINARY="$ROOT/target/release/retrograde-rs"
[ -x "$BINARY" ] || { echo "missing $BINARY" >&2; exit 1; }

VERSION=$(sed -n '/^\[workspace.package\]/,/^\[/s/^version *= *"\(.*\)"/\1/p' Cargo.toml | head -n 1)
[ -n "$VERSION" ] || { echo "workspace version not found in Cargo.toml" >&2; exit 1; }
ARCH=$(uname -m)
case "$ARCH" in x86_64) ARCH=x64 ;; aarch64) ARCH=arm64 ;; esac
NAME="retrograde-$VERSION-linux-$ARCH"

DIST="$ROOT/dist"
STAGE="$DIST/$NAME"
rm -rf "$STAGE"
mkdir -p "$STAGE"
install -m 755 "$BINARY" "$STAGE/Retrograde"
install -m 755 "$HERE/install.sh" "$STAGE/install.sh"
install -m 644 "$HERE/retrograde.desktop" "$STAGE/retrograde.desktop"
install -m 644 "$HERE/retrograde.png" "$STAGE/retrograde.png"
install -m 644 "$ROOT/LICENSE.md" "$STAGE/LICENSE.md"

tar -C "$DIST" -czf "$DIST/$NAME.tar.gz" "$NAME"
rm -rf "$STAGE"
echo "$DIST/$NAME.tar.gz"
