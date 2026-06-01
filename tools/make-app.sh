#!/usr/bin/env bash
# Build Peptide as a double-clickable macOS .app bundle.
#
# Peptide is now the whole product — a single binary that bundles the engine
# harness (webview UI + bytecode patcher), the in-process SSF2 → Fraymakers
# converter, and the FrayTools CDP driver. No sidecar binaries.
#
# Usage:
#   ./make-app.sh            build + assemble build/Peptide.app + launch it
#   ./make-app.sh --no-open  build + assemble only (for packaging / CI)
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
cd "$ROOT"

OPEN_AFTER=1
[ "${1:-}" = "--no-open" ] && OPEN_AFTER=0

APP_NAME="Peptide"
BIN="peptide"

echo "==> Building release binary ($BIN)…"
cargo build --release -p peptide --bin peptide

APP="build/$APP_NAME.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

# ---- Info.plist ----
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>$APP_NAME</string>
  <key>CFBundleDisplayName</key><string>$APP_NAME</string>
  <key>CFBundleIdentifier</key><string>com.peptide.app</string>
  <key>CFBundleVersion</key><string>1.0</string>
  <key>CFBundleShortVersionString</key><string>1.0</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleExecutable</key><string>$BIN</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>CFBundleDocumentTypes</key>
  <array>
    <dict>
      <key>CFBundleTypeName</key><string>SSF2 Character</string>
      <key>CFBundleTypeExtensions</key><array><string>ssf</string></array>
      <key>CFBundleTypeRole</key><string>Viewer</string>
    </dict>
  </array>
</dict>
</plist>
PLIST

# ---- the single peptide binary is the bundle executable ----
cp "build/release/$BIN" "$APP/Contents/MacOS/$BIN"
chmod +x "$APP/Contents/MacOS/$BIN"

# ---- runtime asset files (NOT embedded in the binary — read from disk next to it) ----
# peptide resolves these via asset_candidate_paths (exe-dir first); the converter
# resolves mappings/ via candidate_paths (exe-dir/mappings first). Both must ride
# in Contents/MacOS/ so a packaged app finds them.
RES="$APP/Contents/MacOS"
cp commands.hsx match_settings.conf src/peptide_ui.html "$RES/"
mkdir -p "$RES/mappings"
cp -R crates/ssf2-converter/mappings/. "$RES/mappings/"

# ---- ad-hoc codesign so Gatekeeper lets a local build run (WKWebView needs it) ----
echo "==> Ad-hoc codesigning…"
codesign --force --deep --sign - "$APP" 2>/dev/null || true

echo "==> Done: $APP"
[ "$OPEN_AFTER" = "1" ] && open "$APP" 2>/dev/null || true
echo "    (launch with: open \"$APP\")"
echo "Build complete."
