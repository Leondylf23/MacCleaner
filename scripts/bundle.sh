#!/usr/bin/env bash
# Builds target/release/bundle/MacCleaner.app.
set -euo pipefail
cd "$(dirname "$0")/.."

cargo build --release

APP="target/release/bundle/MacCleaner.app"
VERSION="$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)"

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/MacCleaner "$APP/Contents/MacOS/MacCleaner"
cp assets/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"

cat > "$APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>MacCleaner</string>
    <key>CFBundleDisplayName</key>
    <string>MacCleaner</string>
    <key>CFBundleIdentifier</key>
    <string>com.maccleaner.app</string>
    <key>CFBundleExecutable</key>
    <string>MacCleaner</string>
    <key>CFBundleIconFile</key>
    <string>AppIcon</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>${VERSION}</string>
    <key>CFBundleVersion</key>
    <string>${VERSION}</string>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>LSApplicationCategoryType</key>
    <string>public.app-category.utilities</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>NSAppleEventsUsageDescription</key>
    <string>MacCleaner uses System Events to list, remove and restore your login items.</string>
</dict>
</plist>
EOF

# An ad-hoc signature gives the app a stable identity, so privacy permissions
# (Full Disk Access, Automation) stick to it.
codesign --force --deep --sign - "$APP"

echo "Built $APP"
