#!/bin/sh
# Freezes the Python sidecar with PyInstaller into
# src-tauri/binaries/genropy-sidecar-<target triple>, the name Tauri expects for
# bundle > externalBin. Optional argument: the target triple (default: the host).
set -e
ROOT=$(cd "$(dirname "$0")/.." && pwd)
TRIPLE=${1:-$(rustc -vV | sed -n 's/^host: //p')}
cd "$ROOT/sidecar"
uv run --quiet --group build pyinstaller --noconfirm --onefile --name genropy-sidecar \
    --distpath "$ROOT/sidecar/dist" --workpath "$ROOT/sidecar/build" --specpath "$ROOT/sidecar/build" \
    --paths "$ROOT/sidecar" main.py
mkdir -p "$ROOT/src-tauri/binaries"
cp "$ROOT/sidecar/dist/genropy-sidecar" "$ROOT/src-tauri/binaries/genropy-sidecar-$TRIPLE"
echo "sidecar: src-tauri/binaries/genropy-sidecar-$TRIPLE"
