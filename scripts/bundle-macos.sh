#!/usr/bin/env bash
# Build "Audio Interface.app" from a compiled binary.
#   scripts/bundle-macos.sh <path-to-binary> <version> [out-dir]
# The bundle includes NSMicrophoneUsageDescription, which macOS requires
# before it shows the microphone permission prompt for metering. It is signed
# ad hoc (no Developer ID), so the first launch needs right-click → Open.
set -euo pipefail
BIN=${1:?binary path}
VERSION=${2:?version}
OUT=${3:-dist}

APP="$OUT/Audio Interface.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/audio-interface"
# Icon: the binary draws its own icon. Export it and build an .icns.
ICONSET=$(mktemp -d)/AppIcon.iconset
mkdir -p "$ICONSET"
"$BIN" export-icon "$ICONSET/icon_512x512@2x.png" --size 1024
for s in 16 32 128 256 512; do
  sips -z $s $s "$ICONSET/icon_512x512@2x.png" --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
  d=$((s * 2))
  sips -z $d $d "$ICONSET/icon_512x512@2x.png" --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/AppIcon.icns"

cat > "$APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Audio Interface</string>
  <key>CFBundleDisplayName</key><string>Audio Interface</string>
  <key>CFBundleIdentifier</key><string>com.ikjbi.AudioInterface</string>
  <key>CFBundleExecutable</key><string>audio-interface</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>CFBundleVersion</key><string>${VERSION}</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSMicrophoneUsageDescription</key>
  <string>Audio Interface reads microphone levels to show live meters and detect a lost signal. Audio is never recorded or sent anywhere.</string>
</dict>
</plist>
EOF

codesign --force --deep --sign - "$APP"
echo "Built $APP"
