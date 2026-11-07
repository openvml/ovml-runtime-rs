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

# Remove OpenVML Runtime PATH entries from shell config files.
remove_from_path() {
    config_file="$1"
    if [ -f "$config_file" ]; then
        # Remove the OpenVML Runtime block (comment + export line).
        # Use a temp file for safe in-place editing.
        tmp_file="${config_file}.ovml_tmp"
        awk '
            /^# OpenVML Runtime$/ { skip=1; next }
            skip && /^export PATH=/ { skip=0; next }
            { print }
        ' "$config_file" > "$tmp_file" && mv "$tmp_file" "$config_file"
    fi
}

for rc in "$HOME/.bashrc" "$HOME/.zshrc" "$HOME/.profile"; do
    remove_from_path "$rc"
done

case "$PURGE" in
    1|true|yes|on)
        rm -rf "$DATA_DIR"
        echo "Removed data: $DATA_DIR"
        ;;
esac

echo "Uninstalled OpenVML runtime."
echo "Models and cache kept in: $DATA_DIR (set OVML_PURGE_DATA=1 to remove)."
