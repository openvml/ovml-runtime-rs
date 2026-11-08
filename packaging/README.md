# OpenVML local TTS runtime

Device-side Piper and Edge text-to-speech for OpenVML Studio. No Python required.

## Install

Linux / macOS:

```sh
tar -xzf ovml-runtime-*.tar.gz
cd ovml-runtime-*
./install.sh
```

Windows: extract the zip and run `install.ps1` in PowerShell.

The installer places the runtime in your user profile, keeps models and cache in
`~/.openvml/runtime`, and starts it automatically at login. Set `OVML_NO_AUTOSTART=1`
(Linux/macOS) or pass `-NoAutostart` (Windows) to skip autostart.

Run it manually:

```sh
~/.local/bin/ovml-runtime --data-dir ~/.openvml/runtime
```

The default port is `47821`. OpenVML Studio discovers the runtime automatically.

## Uninstall

```sh
~/.local/lib/ovml-runtime/uninstall.sh
```

or run `uninstall.sh` from the extracted package. Models and cache are kept; set
`OVML_PURGE_DATA=1` (Linux/macOS) or `-PurgeData` (Windows) to remove them too.

## Package contents

- `ovml-runtime` — runtime binary
- `libonnxruntime` — ONNX Runtime shared library used by Piper
- `espeak-ng-data/` — eSpeak-ng data required for Piper phonemization
- `install.sh` / `uninstall.sh` — Linux/macOS installers
- `install.ps1` / `uninstall.ps1` — Windows installers
