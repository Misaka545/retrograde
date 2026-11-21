#!/bin/sh
# Installs Retrograde (Rust build) for the current user: the binary in ~/.local/opt/retrograde,
# a launcher in ~/.local/bin, app drawer entry and icon under ~/.local/share.
#   ./install.sh            (from the extracted release folder, or packaging/linux in a checkout
#                            after `cargo build --release`)
#   ./install.sh uninstall
# Runtime needs: libmpv.so.2 (mpv), libasound.so.2 (ALSA), libpipewire-0.3 (linked by the
# default build), and a shell that hosts StatusNotifierItem tray icons for background play.
set -eu
HERE=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
OPT="$HOME/.local/opt/retrograde"
BIN="$HOME/.local/bin"
APPS="$HOME/.local/share/applications"
ICONS="$HOME/.local/share/icons/hicolor/256x256/apps"

if [ "${1:-}" = "uninstall" ]; then
  rm -rf "$OPT" "$BIN/retrograde" "$APPS/retrograde.desktop" "$ICONS/retrograde.png"
  command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$APPS" || true
  echo "Retrograde removed. Your library in ~/.retrograde was kept."
  exit 0
fi

# Release folder: the binary sits next to this script as Retrograde (or retrograde-rs).
# Checkout: packaging/linux/install.sh picks up target/release/retrograde-rs.
SRC=""
for f in "$HERE/Retrograde" "$HERE/retrograde-rs" "$HERE/../../target/release/retrograde-rs"; do
  if [ -f "$f" ] && [ -x "$f" ]; then SRC="$f"; break; fi
done
[ -n "$SRC" ] || { echo "Retrograde binary not found (run cargo build --release first)" >&2; exit 1; }

mkdir -p "$OPT" "$BIN" "$APPS" "$ICONS"
# copy beside, then rename: replacing a running binary in place fails with "Text file busy"
install -m 755 "$SRC" "$OPT/Retrograde.new" && mv -f "$OPT/Retrograde.new" "$OPT/Retrograde"
# leftovers of the C# build
rm -f "$OPT/libSkiaSharp.so" "$OPT/libHarfBuzzSharp.so"
install -m 644 "$HERE/retrograde.png" "$ICONS/retrograde.png"
ln -sf "$OPT/Retrograde" "$BIN/retrograde"
sed "s|@EXEC@|$OPT/Retrograde|" "$HERE/retrograde.desktop" > "$APPS/retrograde.desktop"
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$APPS" || true
command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q -t "$HOME/.local/share/icons/hicolor" 2>/dev/null || true

have_lib() { ldconfig -p 2>/dev/null | grep -q "$1"; }
have_lib 'libmpv.so.2' || echo "warning: libmpv.so.2 not found, install mpv (Arch: sudo pacman -S mpv, Debian/Ubuntu: sudo apt install libmpv2)" >&2
have_lib 'libasound.so.2' || echo "warning: libasound.so.2 not found, install alsa-lib (Debian/Ubuntu: libasound2)" >&2
have_lib 'libpipewire-0.3.so.0' || echo "warning: libpipewire-0.3.so.0 not found, install pipewire (the default build links it)" >&2
echo "Installed. Launch Retrograde from the app drawer or run: retrograde"
