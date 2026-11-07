#!/bin/sh
# Install the OpenVML local TTS runtime for the current user (Linux/macOS).
#
# Layout:
#   ~/.local/bin/ovml-runtime          launcher
#   ~/.local/lib/ovml-runtime          binary + ONNX Runtime library
#   ~/.openvml/runtime                 models, cache, token, runtime.json
set -eu

PREFIX="${OVML_PREFIX:-$HOME/.local}"
BIN_DIR="$PREFIX/bin"
LIB_DIR="$PREFIX/lib/ovml-runtime"
DATA_DIR="${OVML_DATA_DIR:-$HOME/.openvml/runtime}"
HERE=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)

if [ -f "$HERE/ovml-runtime" ]; then
    SRC_DIR="$HERE"
elif [ -f "$HERE/ovml-runtime/ovml-runtime" ]; then
    SRC_DIR="$HERE/ovml-runtime"
else
    echo "ovml-runtime binary not found next to install.sh" >&2
    exit 1
fi

rm -rf "$LIB_DIR"
mkdir -p "$LIB_DIR" "$BIN_DIR"
install -m 0755 "$SRC_DIR/ovml-runtime" "$LIB_DIR/ovml-runtime"
if [ -f "$HERE/uninstall.sh" ]; then
    install -m 0755 "$HERE/uninstall.sh" "$LIB_DIR/uninstall.sh"
fi

for candidate in "$SRC_DIR"/libonnxruntime* "$SRC_DIR"/onnxruntime*.dll; do
    if [ -f "$candidate" ]; then
        cp -f "$candidate" "$LIB_DIR/"
    fi
done

if ! ls "$LIB_DIR"/libonnxruntime* >/dev/null 2>&1; then
    echo "Warning: ONNX Runtime library not found in the package." >&2
    echo "The runtime will only work if onnxruntime is installed system-wide." >&2
fi

cat > "$BIN_DIR/ovml-runtime" <<EOF
#!/bin/sh
DIR="\${OVML_RUNTIME_HOME:-$LIB_DIR}"
if [ -z "\${ORT_DYLIB_PATH:-}" ]; then
    for candidate in "\$DIR/libonnxruntime.so" "\$DIR/libonnxruntime.so.1" \\
        "\$DIR/libonnxruntime.dylib" "\$DIR/onnxruntime.dll"; do
        if [ -f "\$candidate" ]; then
            ORT_DYLIB_PATH="\$candidate"
            export ORT_DYLIB_PATH
            break
        fi
    done
fi
exec "\$DIR/ovml-runtime" "\$@"
EOF
chmod 0755 "$BIN_DIR/ovml-runtime"

mkdir -p "$DATA_DIR"

install_linux_autostart() {
    unit_dir="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
    unit="$unit_dir/ovml-runtime.service"
    mkdir -p "$unit_dir"
    cat > "$unit" <<EOF
[Unit]
Description=OpenVML local TTS runtime
After=network-online.target

[Service]
ExecStart="$BIN_DIR/ovml-runtime" --data-dir "$DATA_DIR"
Environment="HOME=$HOME"
Restart=on-failure
RestartSec=3

[Install]
WantedBy=default.target
EOF

    if command -v systemctl >/dev/null 2>&1 && systemctl --user show-environment >/dev/null 2>&1; then
        systemctl --user daemon-reload >/dev/null 2>&1 || true
        if systemctl --user enable --now ovml-runtime.service >/dev/null 2>&1; then
            echo "Autostart: systemd --user service ($unit)"
            rm -f "${XDG_CONFIG_HOME:-$HOME/.config}/autostart/ovml-runtime.desktop"
            return
        fi
    fi

    desktop_dir="${XDG_CONFIG_HOME:-$HOME/.config}/autostart"
    mkdir -p "$desktop_dir"
    cat > "$desktop_dir/ovml-runtime.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=OpenVML Runtime
Comment=Local text-to-speech runtime for OpenVML Studio
Exec="$BIN_DIR/ovml-runtime" --data-dir "$DATA_DIR"
Terminal=false
NoDisplay=true
X-GNOME-Autostart-enabled=true
EOF
    echo "Autostart: XDG desktop entry ($desktop_dir/ovml-runtime.desktop)"
}

install_macos_autostart() {
    agent_dir="$HOME/Library/LaunchAgents"
    plist="$agent_dir/com.openvml.runtime.plist"
    log_dir="$HOME/Library/Logs"
    mkdir -p "$agent_dir" "$log_dir"
    cat > "$plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.openvml.runtime</string>
    <key>ProgramArguments</key>
    <array>
        <string>$BIN_DIR/ovml-runtime</string>
        <string>--data-dir</string>
        <string>$DATA_DIR</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>$log_dir/ovml-runtime.log</string>
    <key>StandardErrorPath</key>
    <string>$log_dir/ovml-runtime.error.log</string>
</dict>
</plist>
EOF
    if command -v launchctl >/dev/null 2>&1; then
        launchctl unload "$plist" >/dev/null 2>&1 || true
        launchctl load -w "$plist" >/dev/null 2>&1 || true
    fi
    echo "Autostart: macOS LaunchAgent ($plist)"
}

case "${OVML_NO_AUTOSTART:-0}" in
    1|true|yes|on) SKIP_AUTOSTART=1 ;;
    *) SKIP_AUTOSTART=0 ;;
esac

if [ "$SKIP_AUTOSTART" = "1" ]; then
    echo "Autostart skipped (OVML_NO_AUTOSTART)"
else
    case "$(uname -s 2>/dev/null || echo unknown)" in
        Darwin) install_macos_autostart ;;
        *) install_linux_autostart ;;
    esac
fi

case ":$PATH:" in
    *":$BIN_DIR:"*) ;;
    *) echo "Note: add $BIN_DIR to PATH to run 'ovml-runtime' from anywhere." ;;
esac

echo "Installed: $BIN_DIR/ovml-runtime"
echo "Models and cache: $DATA_DIR"
echo "OpenVML Studio will detect the runtime automatically."
