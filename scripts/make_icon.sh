#!/usr/bin/env bash
# Regenerates assets/AppIcon.icns and assets/icon-256.rgba from assets/icon.svg.
set -euo pipefail
cd "$(dirname "$0")/.."

ICONSET="target/AppIcon.iconset"
rm -rf "$ICONSET"

cargo run --quiet --release \
    --manifest-path tools/icon-render/Cargo.toml \
    --target-dir target/icon-render \
    -- assets/icon.svg "$ICONSET" assets/icon-256.rgba

iconutil --convert icns "$ICONSET" --output assets/AppIcon.icns

echo "Wrote assets/AppIcon.icns and assets/icon-256.rgba"
