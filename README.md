# OpenVML TTS Runtime

Cross-platform device-side text-to-speech runtime for [OpenVML Studio](https://openvml.com). Provides local Piper and Edge TTS synthesis without Python or cloud dependencies.

## Features

- **Piper TTS** — 170 voices across 58 languages (ONNX Runtime)
- **Edge TTS** — 322 voices via Microsoft Edge (WebSocket)
- **HTTP API** — simple REST interface on `127.0.0.1:47821`
- **Auto-discovery** — writes `runtime.json` for automatic detection by OpenVML Studio
- **Model management** — downloads and caches Piper models on first use
- **Cross-platform** — Linux, Windows, macOS (Intel + Apple Silicon)

## Quick Start

### Download pre-built binary

Download the latest release for your platform from the [Releases](https://github.com/openvml/ovml-runtime-rs/releases) page:

| Platform | File |
|----------|------|
| Linux x86_64 | `ovml-runtime-<version>-linux-x86_64.tar.gz` |
| Windows x64 | `ovml-runtime-<version>-windows-x86_64.zip` |
| macOS Intel | `ovml-runtime-<version>-macos-x86_64.tar.gz` |
| macOS Apple Silicon | `ovml-runtime-<version>-macos-arm64.tar.gz` |

### Install (Linux / macOS)

```sh
tar -xzf ovml-runtime-*.tar.gz
cd ovml-runtime-*
./install.sh
```

The installer places the runtime in `~/.local/bin/`, keeps models and cache in `~/.openvml/runtime`, and registers autostart (systemd on Linux, LaunchAgent on macOS).

### Install (Windows)

Extract the zip and run in PowerShell:

```powershell
.\install.ps1
```

Autostart is registered via the Windows Startup folder.

### Run manually

```sh
~/.local/bin/ovml-runtime --data-dir ~/.openvml/runtime
```

The runtime listens on `127.0.0.1:47821` by default. OpenVML Studio discovers it automatically.

### Uninstall

```sh
# Linux / macOS
~/.local/lib/ovml-runtime/uninstall.sh

# Keep models and cache (default) or remove everything:
OVML_PURGE_DATA=1 ~/.local/lib/ovml-runtime/uninstall.sh

# Windows
.\uninstall.ps1
# or with data removal:
.\uninstall.ps1 -PurgeData
```

## HTTP API

All endpoints except `/v1/health` and `/v1/pair` require authentication via `X-OVML-Token` header or `Authorization: Bearer <token>`.

### Health check

```sh
curl http://127.0.0.1:47821/v1/health
```

```json
{
  "status": "ok",
  "version": "0.1.0",
  "engines": ["piper", "edge"],
  "piper_voices": 170,
  "edge_voices_cached": 322
}
```

### Pair (obtain token)

```sh
curl -X POST http://127.0.0.1:47821/v1/pair \
  -H "Origin: http://localhost:5173"
```

Returns `{"token": "...", "expires_at": "..."}`. The origin must be in the allowlist (localhost, 127.0.0.1, openvml.com, tauri://localhost).

### List voices

```sh
curl http://127.0.0.1:47821/v1/voices?engine=piper \
  -H "X-OVML-Token: <token>"
```

### Synthesize

```sh
curl -X POST http://127.0.0.1:47821/v1/synthesize \
  -H "X-OVML-Token: <token>" \
  -H "Content-Type: application/json" \
  -d '{"text": "Hello world", "engine": "piper", "voice": "en_US-amy-low", "rate": 1.0}' \
  --output hello.wav
```

Response includes headers: `X-OVML-Duration-Ms`, `X-OVML-Sample-Rate`, `X-OVML-Mime`, `X-OVML-Engine`, `X-OVML-Voice`.

### Stream (NDJSON)

```sh
curl -X POST http://127.0.0.1:47821/v1/synthesize/stream \
  -H "X-OVML-Token: <token>" \
  -H "Content-Type: application/json" \
  -d '{"text": "Sentence one. Sentence two.", "engine": "edge", "voice": "ru-RU-DmitryNeural"}'
```

Returns newline-delimited JSON, one object per sentence, ending with `{"done": true}`.

### Request body

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `text` | string | required | Text to synthesize |
| `engine` | string | auto | `piper` or `edge` |
| `voice` | string | auto | Voice ID (e.g. `en_US-amy-low`, `ru-RU-DmitryNeural`) |
| `rate` | float | 1.0 | Speech rate (0.5–2.0) |
| `pitch` | int | 0 | Pitch offset in Hz (-100 to +100) |
| `language` | string | auto | Language code (e.g. `ru`, `en`) |

## Configuration

| Environment variable | Default | Description |
|---------------------|---------|-------------|
| `OVML_DATA_DIR` | `~/.openvml/runtime` | Models, cache, token, discovery file |
| `OVML_PREFIX` | `~/.local` | Installation prefix (Linux/macOS) |
| `OVML_NO_AUTOSTART` | `0` | Set to `1` to skip autostart registration |
| `OVML_PURGE_DATA` | `0` | Set to `1` during uninstall to remove models and cache |
| `ORT_DYLIB_PATH` | auto | Path to ONNX Runtime shared library |

### CLI arguments

```
ovml-runtime --port <PORT> --data-dir <DIR>
```

- `--port` — listen port (default: 47821)
- `--data-dir` — data directory (default: `~/.openvml/runtime`)

## Building from source

### Prerequisites

- Rust 1.70+ (`rustup`)
- CMake, Clang, pkg-config, OpenSSL dev headers
- Python 3.10+ (for ONNX Runtime wheel)

### Build

```sh
git clone https://github.com/openvml/ovml-runtime-rs.git
cd ovml-runtime-rs

# Set up build environment (Linux)
source "$HOME/.cargo/env"
export PATH="$PWD/.deps/cmake/usr/bin:$PWD/.deps/pkgconf/usr/bin:$PWD/.deps/clang/usr/bin:$PATH"
export PKG_CONFIG_PATH="$PWD/.deps/openssl/usr/lib/x86_64-linux-gnu/pkgconfig:$PWD/.deps/pkgconfig/usr/lib/pkgconfig"
export LIBCLANG_PATH="$PWD/.deps/libclang/usr/lib/x86_64-linux-gnu"
export OPENSSL_DIR="$PWD/.deps/openssl/usr"
export OPENSSL_LIB_DIR="$PWD/.deps/openssl/usr/lib/x86_64-linux-gnu"
export OPENSSL_INCLUDE_DIR="$PWD/.deps/openssl/usr/include"

# Install ONNX Runtime
python -m pip install onnxruntime

# Build
cargo build --release

# Run tests
cargo test
```

### Package for distribution

```sh
ORT_DYLIB_PATH=$(python -c "import onnxruntime, pathlib; print(pathlib.Path(onnxruntime.__file__).parent/'capi'/'libonnxruntime.so.1.29.0')") \
  python scripts/package_release.py --out release
```

## Architecture

```
┌─────────────────────────────────────────────┐
│           OpenVML Studio (frontend)          │
│  discovers runtime via runtime.json           │
└──────────────┬──────────────────────────────┘
               │ HTTP (127.0.0.1:47821)
┌──────────────┴──────────────────────────────┐
│            ovml-runtime (Rust)               │
│  ┌─────────┐  ┌─────────┐  ┌────────────┐  │
│  │  Piper   │  │  Edge   │  │   Cache    │  │
│  │ (ort)    │  │ (WSS)   │  │ (SHA-256)  │  │
│  └─────────┘  └─────────┘  └────────────┘  │
│  ┌─────────────────────────────────────┐     │
│  │         Axum HTTP Server             │     │
│  └─────────────────────────────────────┘     │
└─────────────────────────────────────────────┘
```

## License

Apache-2.0
