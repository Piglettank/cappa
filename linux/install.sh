#!/bin/sh
# Builds Cappa and installs it for the current user (no root needed):
#   ~/.local/bin/cappa, its menu entry and icons.
#
#   ./install.sh              build and install
#   ./install.sh --autostart  also start Cappa when you log in
#   ./install.sh --uninstall  remove everything this script installed
set -eu

cd "$(dirname "$0")"
BIN="$HOME/.local/bin"
SHARE="${XDG_DATA_HOME:-$HOME/.local/share}"
AUTOSTART="${XDG_CONFIG_HOME:-$HOME/.config}/autostart"
DESKTOP=dev.fallberg.Cappa.desktop

if [ "${1:-}" = "--uninstall" ]; then
    rm -f "$BIN/cappa" "$SHARE/applications/$DESKTOP" "$AUTOSTART/$DESKTOP"
    for size in 64x64 128x128 256x256; do
        rm -f "$SHARE/icons/hicolor/$size/apps/dev.fallberg.Cappa.png"
    done
    echo "Cappa removed. Settings and the log are kept in ~/.config/cappa and ~/.local/state/cappa."
    exit 0
fi

cargo build --release

install -Dm755 target/release/cappa "$BIN/cappa"
for size in 64x64 128x128 256x256; do
    install -Dm644 "data/icons/hicolor/$size/apps/dev.fallberg.Cappa.png" "$SHARE/icons/hicolor/$size/apps/dev.fallberg.Cappa.png"
done
# Absolute paths, so the menu entry works even when ~/.local/bin isn't on PATH.
mkdir -p "$SHARE/applications"
sed "s|^Exec=cappa|Exec=$BIN/cappa|" "data/$DESKTOP" > "$SHARE/applications/$DESKTOP"
update-desktop-database "$SHARE/applications" 2>/dev/null || true
gtk-update-icon-cache -q -t "$SHARE/icons/hicolor" 2>/dev/null || true

if [ "${1:-}" = "--autostart" ]; then
    mkdir -p "$AUTOSTART"
    cp "$SHARE/applications/$DESKTOP" "$AUTOSTART/$DESKTOP"
    echo "Cappa will start when you log in."
fi

echo "Installed $BIN/cappa. Start it from your app menu, or run: cappa"
