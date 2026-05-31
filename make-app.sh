#!/usr/bin/env bash
#
# make-app.sh — build the cross-platform GUI and wrap it in a double-clickable
# macOS "SSF2 Converter.app" bundle.
#
# The GUI (ssf2-converter-gui) shells out to the ssf2_converter CLI that sits
# next to it, so the bundle ships BOTH binaries together in Contents/MacOS/.
#
# Usage:
#   ./make-app.sh            build + assemble into dist/, then launch
#   ./make-app.sh --no-open  build + assemble only (for packaging/CI)
#
# macOS only. On Windows/Linux the GUI binary in target/release/ is already
# directly runnable — no bundle needed.
set -euo pipefail

REPO="$(cd "$(dirname "$0")" && pwd)"
APP_NAME="SSF2 Converter"
APP="$REPO/dist/$APP_NAME.app"
OPEN=1
[ "${1:-}" = "--no-open" ] && OPEN=0

if [ "$(uname)" != "Darwin" ]; then
    echo "make-app.sh builds a macOS .app bundle and only runs on macOS."
    echo "On this OS just run: cargo build --release && ./target/release/ssf2-converter-gui"
    exit 1
fi

echo "→ Building converter + GUI (release)…"
cd "$REPO"
cargo build --release -p ssf2_converter --bin ssf2_converter
cargo build --release -p ssf2-converter-gui

GUI_BIN="$REPO/target/release/ssf2-converter-gui"
CLI_BIN="$REPO/target/release/ssf2_converter"
for b in "$GUI_BIN" "$CLI_BIN"; do
    [ -x "$b" ] || { echo "✗ missing build output: $b" >&2; exit 1; }
done

echo "→ Assembling $APP_NAME.app…"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

# The GUI is the bundle's main executable; the CLI rides alongside it so the
# GUI's sibling-lookup (current_exe().parent()/ssf2_converter) finds it.
cp "$GUI_BIN" "$APP/Contents/MacOS/SSF2 Converter"
cp "$CLI_BIN" "$APP/Contents/MacOS/ssf2_converter"
chmod +x "$APP/Contents/MacOS/SSF2 Converter" "$APP/Contents/MacOS/ssf2_converter"

# Optional custom icon: drop an AppIcon.icns next to this script to use it.
ICON_KEYS=""
if [ -f "$REPO/AppIcon.icns" ]; then
    cp "$REPO/AppIcon.icns" "$APP/Contents/Resources/AppIcon.icns"
    ICON_KEYS='    <key>CFBundleIconFile</key>
    <string>AppIcon</string>'
fi

cat > "$APP/Contents/Info.plist" << PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>
    <string>SSF2 Converter</string>
    <key>CFBundleIdentifier</key>
    <string>com.ssf2converter.app</string>
    <key>CFBundleName</key>
    <string>$APP_NAME</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>1.0</string>
    <key>CFBundleVersion</key>
    <string>1</string>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>NSPrincipalClass</key>
    <string>NSApplication</string>
    <key>NSHighResolutionCapable</key>
    <true/>
$ICON_KEYS
    <key>CFBundleDocumentTypes</key>
    <array>
        <dict>
            <key>CFBundleTypeName</key>
            <string>SSF File</string>
            <key>CFBundleTypeExtensions</key>
            <array><string>ssf</string></array>
            <key>CFBundleTypeRole</key>
            <string>Viewer</string>
        </dict>
    </array>
</dict>
</plist>
PLIST

# Ad-hoc codesign so Gatekeeper lets a locally-built app launch without the
# "damaged / unidentified developer" nag. Harmless if codesign is unavailable.
codesign --force --deep --sign - "$APP" >/dev/null 2>&1 || true

echo "✓ Built: $APP"

if [ "$OPEN" = "1" ]; then
    echo "→ Launching…"
    open "$APP"
fi
