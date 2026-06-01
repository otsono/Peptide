#!/usr/bin/env bash
# Build Peptide for Windows (.exe) from macOS/Linux, or natively on Windows.
# Peptide is a single binary (engine harness + converter + FrayTools driver).
#
# Cross-compiling needs one of:
#   - cargo-xwin  (MSVC ABI; recommended)   cargo install cargo-xwin + LLVM
#   - mingw-w64   (GNU ABI; fallback)        brew install mingw-w64
#
# Runtime note: the webview UI uses WebView2. The WebView2 runtime ships with
# Windows 10/11 by default; on older images install the Evergreen runtime.
#
# Usage:
#   ./make-win.sh            build peptide.exe into build/windows/
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
cd "$ROOT"

OUT="build/windows"
BIN="peptide"

mkdir -p "$OUT"

# Pick a toolchain.
TARGET=""
MODE=""
if command -v cargo-xwin >/dev/null 2>&1; then
  TARGET="x86_64-pc-windows-msvc"
  MODE="xwin"
elif rustup target list --installed 2>/dev/null | grep -q x86_64-pc-windows-gnu; then
  TARGET="x86_64-pc-windows-gnu"
  MODE="gnu"
else
  echo "No Windows toolchain found. Install one of:" >&2
  echo "  cargo install cargo-xwin   (MSVC ABI; recommended)" >&2
  echo "  rustup target add x86_64-pc-windows-gnu + brew install mingw-w64" >&2
  exit 1
fi

echo "==> Building peptide for $TARGET ($MODE)…"

if [ "$MODE" = "xwin" ]; then
  cargo xwin build --release --target "$TARGET" -p peptide --bin peptide >&2
else
  cargo build --release --target "$TARGET" -p peptide --bin peptide >&2
fi

echo "==> Staging .exe into $OUT…"
cp "build/$TARGET/release/$BIN.exe" "$OUT/" 2>/dev/null || true

# ---- runtime asset files (NOT embedded — peptide.exe reads them from disk next
# to itself; the converter reads mappings/ next to the .exe). Ship them alongside.
cp commands.hsx match_settings.conf src/peptide_ui.html "$OUT/" 2>/dev/null || true
mkdir -p "$OUT/mappings"
cp -R crates/ssf2-converter/mappings/. "$OUT/mappings/" 2>/dev/null || true

cat <<DONE

Build complete. Windows file staged in:
  $OUT/$BIN.exe

Run it by double-clicking peptide.exe (needs the WebView2 runtime, which is
preinstalled on Windows 10/11). CLI modes work too: peptide.exe convert <file.ssf>
DONE

echo "    $OUT/$BIN.exe"
