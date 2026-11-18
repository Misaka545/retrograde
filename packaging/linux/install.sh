#!/bin/sh
# Installs Retrograde for the current user: files in ~/.local/opt/retrograde,
# launcher in ~/.local/bin, app drawer entry and icon under ~/.local/share.
#   ./install.sh            (from the extracted release folder)
#   ./install.sh uninstall
# Needs the mpv package (libmpv.so.2): sudo pacman -S mpv
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

[ -x "$HERE/Retrograde" ] || { echo "Retrograde binary not found next to install.sh" >&2; exit 1; }
mkdir -p "$OPT" "$BIN" "$APPS" "$ICONS"
# copy beside, then rename: replacing a running binary in place fails with "Text file busy"
for f in Retrograde libSkiaSharp.so libHarfBuzzSharp.so; do install -m 755 "$HERE/$f" "$OPT/$f.new" && mv -f "$OPT/$f.new" "$OPT/$f"; done
install -m 644 "$HERE/retrograde.png" "$ICONS/retrograde.png"
ln -sf "$OPT/Retrograde" "$BIN/retrograde"
sed "s|@EXEC@|$OPT/Retrograde|" "$HERE/retrograde.desktop" > "$APPS/retrograde.desktop"
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$APPS" || true
command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q -t "$HOME/.local/share/icons/hicolor" 2>/dev/null || true
ldconfig -p 2>/dev/null | grep -q 'libmpv.so.2' || echo "warning: libmpv.so.2 not found, install mpv (sudo pacman -S mpv)" >&2
echo "Installed. Launch Retrograde from the app drawer or run: retrograde"
