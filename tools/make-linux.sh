#!/usr/bin/env bash
# Build Peptide for Linux. Peptide is a single binary (engine harness + converter
# + FrayTools driver); the GUI is a wry webview backed by WebKitGTK.
#
# Run this NATIVELY on Linux (like make-app.sh on macOS). Cross-compiling from
# macOS is impractical because wry links the system WebKitGTK + GTK3 dev libs.
#
# Build prerequisites (Debian/Ubuntu names; adjust for your distro):
#   sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev build-essential
# (Older distros: libwebkit2gtk-4.0-dev. Fedora: webkit2gtk4.1-devel gtk3-devel.)
#
# Runtime note: the GUI defaults the DMABUF-renderer workaround on (see main.rs)
# so the webview doesn't boot to a blank screen on NVIDIA/VM/Wayland setups.
#
# Usage:
#   ./make-linux.sh            build peptide into build/linux/ (+ data/)
#   ./make-linux.sh --run      build, then launch the staged binary
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
cd "$ROOT"

RUN_AFTER=0
[ "${1:-}" = "--run" ] && RUN_AFTER=1

OUT="build/linux"
BIN="peptide"

mkdir -p "$OUT"

echo "==> Building release binary ($BIN)…"
cargo build --release -p peptide --bin peptide

echo "==> Staging binary into $OUT…"
cp "build/release/$BIN" "$OUT/$BIN"
chmod +x "$OUT/$BIN"

# ---- runtime asset files (NOT embedded — peptide reads them from disk). Both the
# app and the converter check a `data/` subfolder next to the binary first, so
# stage everything into $OUT/data/.
DATA="$OUT/data"
mkdir -p "$DATA/mappings"
cp commands.hsx match_settings.conf "$DATA/"   # peptide_ui.html is embedded in the binary
cp -R crates/ssf2-converter/mappings/. "$DATA/mappings/"

cat <<DONE

Build complete. Linux files staged in:
  $OUT/$BIN          (+ $OUT/data/ runtime assets)

Run it from anywhere — the binary resolves data/ next to itself:
  $OUT/$BIN          (GUI; needs WebKitGTK + GTK3 installed)
  $OUT/$BIN convert <file.ssf>   (CLI modes work too)

If the window opens blank, the DMABUF workaround is already defaulted on; the
older fallback is: WEBKIT_DISABLE_COMPOSITING_MODE=1 $OUT/$BIN
DONE

if [ "$RUN_AFTER" = "1" ]; then
  echo "==> Launching $OUT/$BIN…"
  "$OUT/$BIN" || true
fi
