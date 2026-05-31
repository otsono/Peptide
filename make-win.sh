#!/usr/bin/env bash
#
# make-win.sh — cross-compile the Windows build of the converter + GUI from
# macOS or Linux, and stage both .exe files in dist/windows/.
#
# Produces:
#   dist/windows/ssf2-converter-gui.exe   (the GUI; no console window)
#   dist/windows/ssf2_converter.exe       (the CLI it shells out to)
#   dist/windows/README.txt               (ship-these-together note)
#
# Ship those two .exe files in the SAME folder — the GUI finds the CLI as a
# sibling. (If you build natively ON Windows instead, just run
# `cargo build --release` and grab the two .exe from target\release\.)
#
# Toolchain: prefers cargo-xwin (targets the MSVC ABI, most compatible with
# Windows). Falls back to the GNU ABI via mingw-w64 if cargo-xwin isn't
# present. The script tells you exactly what to install if neither is set up.
set -euo pipefail

REPO="$(cd "$(dirname "$0")" && pwd)"
OUT="$REPO/dist/windows"
cd "$REPO"

# cargo-xwin shells out to `lld-link` (from Homebrew's `lld` package) and needs
# rustup's `cargo`. A non-login shell may not have these on PATH, so prepend the
# common locations when they exist (portable across the brew prefix on Apple
# Silicon / Intel / Linux).
if command -v brew >/dev/null 2>&1; then
    BREW_PREFIX="$(brew --prefix 2>/dev/null || true)"
    for d in "$BREW_PREFIX/opt/lld/bin" "$BREW_PREFIX/opt/llvm/bin"; do
        [ -d "$d" ] && PATH="$d:$PATH"
    done
fi
[ -d "$HOME/.cargo/bin" ] && PATH="$HOME/.cargo/bin:$PATH"
export PATH

have() { command -v "$1" >/dev/null 2>&1; }

build_msvc() {
    echo "→ Cross-compiling for x86_64-pc-windows-msvc (via cargo-xwin)…" >&2
    rustup target add x86_64-pc-windows-msvc >/dev/null 2>&1 || true
    cargo xwin build --release --target x86_64-pc-windows-msvc \
        -p ssf2_converter --bin ssf2_converter >&2
    cargo xwin build --release --target x86_64-pc-windows-msvc \
        -p ssf2-converter-gui >&2
    echo "x86_64-pc-windows-msvc"
}

build_gnu() {
    echo "→ Cross-compiling for x86_64-pc-windows-gnu (via mingw-w64)…" >&2
    rustup target add x86_64-pc-windows-gnu >/dev/null 2>&1 || true
    cargo build --release --target x86_64-pc-windows-gnu \
        -p ssf2_converter --bin ssf2_converter >&2
    cargo build --release --target x86_64-pc-windows-gnu \
        -p ssf2-converter-gui >&2
    echo "x86_64-pc-windows-gnu"
}

if have cargo-xwin; then
    TRIPLE="$(build_msvc)"
elif have x86_64-w64-mingw32-gcc; then
    TRIPLE="$(build_gnu)"
else
    cat >&2 <<'EOF'
✗ No Windows cross-toolchain found. Install ONE of these, then re-run:

  Option A — cargo-xwin (recommended; MSVC ABI):
      cargo install cargo-xwin
      # macOS also needs LLVM's lld linker:  brew install llvm
      # (xwin downloads the Windows SDK on first build — a few hundred MB, cached)

  Option B — mingw-w64 (GNU ABI):
      brew install mingw-w64            # macOS
      # or:  sudo apt install gcc-mingw-w64-x86-64   # Debian/Ubuntu

Or just build natively ON a Windows machine:
      rustup default stable-x86_64-pc-windows-msvc
      cargo build --release
      # → target\release\ssf2-converter-gui.exe + ssf2_converter.exe
EOF
    exit 1
fi

GUI="$REPO/target/$TRIPLE/release/ssf2-converter-gui.exe"
CLI="$REPO/target/$TRIPLE/release/ssf2_converter.exe"
for b in "$GUI" "$CLI"; do
    [ -f "$b" ] || { echo "✗ missing build output: $b" >&2; exit 1; }
done

echo "→ Staging into dist/windows/…" >&2
rm -rf "$OUT"; mkdir -p "$OUT"
cp "$GUI" "$CLI" "$OUT/"
cat > "$OUT/README.txt" <<'EOF'
SSF2 -> Fraymakers Converter (Windows)

Keep these two files in the SAME folder:
  ssf2-converter-gui.exe   <- double-click this (the GUI)
  ssf2_converter.exe       <- the converter the GUI calls

The GUI finds the converter next to itself, so don't separate them.

"Export in FrayTools" additionally needs Node.js installed.
EOF

echo "✓ Built ($TRIPLE):"
echo "    $OUT/ssf2-converter-gui.exe"
echo "    $OUT/ssf2_converter.exe"
