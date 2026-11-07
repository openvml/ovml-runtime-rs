#!/bin/sh
# Remove the OpenVML local TTS runtime installed by install.sh.
set -eu

PREFIX="${OVML_PREFIX:-$HOME/.local}"
BIN_DIR="$PREFIX/bin"
LIB_DIR="$PREFIX/lib/ovml-runtime"
DATA_DIR="${OVML_DATA_DIR:-$HOME/.openvml/runtime}"
PURGE="${OVML_PURGE_DATA:-0}"

if command -v systemctl >/dev/null 2>&1; then
    systemctl --user disable --now ovml-runtime.service >/dev/null 2>&1 || true
fi
rm -f "${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user/ovml-runtime.service"
rm -f "${XDG_CONFIG_HOME:-$HOME/.config}/autostart/ovml-runtime.desktop"

PLIST="$HOME/Library/LaunchAgents/com.openvml.runtime.plist"
if [ -f "$PLIST" ]; then
    if command -v launchctl >/dev/null 2>&1; then
        launchctl unload "$PLIST" >/dev/null 2>&1 || true
    fi
    rm -f "$PLIST"
fi

rm -f "$BIN_DIR/ovml-runtime"
rm -rf "$LIB_DIR"

case "$PURGE" in
    1|true|yes|on)
        rm -rf "$DATA_DIR"
        echo "Removed data: $DATA_DIR"
        ;;
esac

echo "Uninstalled OpenVML runtime."
echo "Models and cache kept in: $DATA_DIR (set OVML_PURGE_DATA=1 to remove)."
