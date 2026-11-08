"""Build a distributable archive for the OpenVML Rust runtime.

Usage:
    python scripts/package_release.py --ort /path/to/libonnxruntime.so
    python scripts/package_release.py --ort ... --onedir

Archives use the same names as the Python runtime releases expected by the
backend: ovml-runtime-<version>-<platform>[-onedir].(tar.gz|zip).
"""
from __future__ import annotations

import argparse
import os
import platform
import re
import sys
import tarfile
import time
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PACKAGING = ROOT / "packaging"


def read_version() -> str:
    text = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    match = re.search(r'^version\s*=\s*"([^"]+)"', text, re.MULTILINE)
    if not match:
        raise SystemExit("Cannot read version from Cargo.toml")
    return match.group(1)


def platform_tag() -> str:
    system = {
        "linux": "linux",
        "darwin": "macos",
        "win32": "windows",
    }.get(sys.platform, sys.platform)
    machine = platform.machine().lower()
    machine = {
        "x86_64": "x86_64",
        "amd64": "x86_64",
        "aarch64": "arm64",
        "arm64": "arm64",
    }.get(machine, machine)
    return f"{system}-{machine}"


def binary_name() -> str:
    return "ovml-runtime.exe" if sys.platform == "win32" else "ovml-runtime"


def ort_name() -> str:
    if sys.platform == "win32":
        return "onnxruntime.dll"
    if sys.platform == "darwin":
        return "libonnxruntime.dylib"
    return "libonnxruntime.so"


def resolve_ort(value: str) -> tuple[Path, list[Path]]:
    candidate = Path(value)
    if candidate.is_dir():
        directory = candidate
        primary = None
        for name in (ort_name(), "libonnxruntime.so.1", "libonnxruntime.dylib", "onnxruntime.dll"):
            path = directory / name
            if path.is_file():
                primary = path
                break
        if primary is None:
            found = [p for p in sorted(directory.glob("libonnxruntime*")) if p.is_file()]
            found += [p for p in sorted(directory.glob("onnxruntime*.dll")) if p.is_file()]
            if not found:
                raise SystemExit(f"No ONNX Runtime library found in {candidate}")
            primary = found[0]
    elif candidate.is_file():
        directory = candidate.parent
        primary = candidate
    else:
        raise SystemExit(f"ONNX Runtime library not found: {candidate}")

    shared = [p for p in sorted(directory.glob("libonnxruntime*")) if p.is_file()]
    shared += [p for p in sorted(directory.glob("onnxruntime*.dll")) if p.is_file()]
    if primary not in shared:
        shared.append(primary)
    return primary, shared


ESPEAK_DIR_NAME = "espeak-ng-data"


def resolve_espeak_data(override: str, dist: Path) -> Path:
    candidates: list[Path] = []
    if override:
        candidates.append(Path(override))
    else:
        for pattern in (
            "build/espeak-rs-sys-*/out/share/espeak-ng-data",
            "build/espeak-rs-sys-*/out/build/espeak-ng-data",
        ):
            candidates.extend(sorted(dist.glob(pattern)))
    for candidate in candidates:
        if candidate.is_dir() and (candidate / "phontab").is_file():
            return candidate
    if candidates:
        raise SystemExit(
            "espeak-ng-data is incomplete (missing phontab): "
            + ", ".join(str(c) for c in candidates)
        )
    raise SystemExit(
        "espeak-ng-data not found. Build espeak-rs-sys first or pass --espeak-data."
    )


def collect_tree(root: Path, prefix: str) -> list[tuple[Path, str, int]]:
    entries: list[tuple[Path, str, int]] = []
    for path in sorted(root.rglob("*")):
        if path.is_file():
            rel = path.relative_to(root).as_posix()
            entries.append((path, f"{prefix}{rel}", 0o644))
    return entries


def add_tar(archive: Path, files: list[tuple[Path, str, int]]) -> None:
    with tarfile.open(archive, "w:gz") as tar:
        for source, name, mode in files:
            info = tar.gettarinfo(str(source), arcname=name)
            info.mode = mode
            with open(source, "rb") as handle:
                tar.addfile(info, handle)


def add_zip(archive: Path, files: list[tuple[Path, str, int]]) -> None:
    with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as bundle:
        for source, name, mode in files:
            info = zipfile.ZipInfo(name, date_time=time.localtime(source.stat().st_mtime)[:6])
            info.external_attr = (mode & 0xFFFF) << 16
            info.compress_type = zipfile.ZIP_DEFLATED
            with open(source, "rb") as handle:
                bundle.writestr(info, handle.read())


def main() -> int:
    parser = argparse.ArgumentParser(description="Package the OpenVML Rust runtime")
    parser.add_argument("--dist", default=str(ROOT / "target" / "release"))
    parser.add_argument("--out", default=str(ROOT / "release"))
    parser.add_argument("--ort", default=os.environ.get("ORT_DYLIB_PATH", ""))
    parser.add_argument("--espeak-data", default=os.environ.get("OVML_ESPEAK_DATA", ""))
    parser.add_argument("--version", default="")
    parser.add_argument("--tag", default="")
    parser.add_argument("--onedir", action="store_true")
    args = parser.parse_args()

    if not args.ort:
        raise SystemExit("Pass --ort or set ORT_DYLIB_PATH")

    version = args.version or read_version()
    tag = args.tag or platform_tag()
    is_windows = sys.platform == "win32"
    dist = Path(args.dist)
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)

    binary = dist / binary_name()
    if not binary.is_file():
        raise SystemExit(f"Missing executable: {binary}")
    primary_ort, ort_files = resolve_ort(args.ort)
    espeak_data = resolve_espeak_data(args.espeak_data, dist)

    prefix = "ovml-runtime/" if args.onedir else ""
    files: list[tuple[Path, str, int]] = [
        (binary, f"{prefix}{binary_name()}", 0o755),
        (primary_ort, f"{prefix}{ort_name()}", 0o755 if not is_windows else 0o644),
    ]
    for ort_file in ort_files:
        if ort_file == primary_ort:
            continue
        files.append((ort_file, f"{prefix}{ort_file.name}", 0o755 if not is_windows else 0o644))
    files.extend(collect_tree(espeak_data, f"{prefix}{ESPEAK_DIR_NAME}/"))
    files.append((PACKAGING / "README.md", "README.md", 0o644))
    scripts = ["install.ps1", "uninstall.ps1"] if is_windows else ["install.sh", "uninstall.sh"]
    for script in scripts:
        files.append((PACKAGING / script, script, 0o755 if script.endswith(".sh") else 0o644))

    suffix = "-onedir" if args.onedir else ""
    base = f"ovml-runtime-{version}-{tag}{suffix}"
    archive = out / (f"{base}.zip" if is_windows else f"{base}.tar.gz")

    if is_windows:
        add_zip(archive, files)
    else:
        add_tar(archive, files)

    print(f"Created {archive} ({archive.stat().st_size / 1e6:.1f} MB)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
