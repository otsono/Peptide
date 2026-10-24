#!/bin/sh
# freeze.sh DIR -- a copy of the build that a long live run can hold on to while the tree moves:
# the binary AND everything it reads from disk at run time (the converter's mappings and runtime,
# the engine-side scripts and match settings), which a copied binary would otherwise still read
# from the working tree. run with PEPTIDE_BIN=DIR/peptide; verify.py points the mapping and
# asset lookups at DIR itself.
set -e
[ -n "$1" ] || { echo "usage: tools/freeze.sh DIR"; exit 2; }
ROOT=$(cd "$(dirname "$0")/.." && pwd)
mkdir -p "$1/assets"
# a fresh file, never one copied over: the system kills a binary rewritten in place
rm -f "$1/peptide" && cp "$ROOT/build/release/peptide" "$1/peptide"
rm -rf "$1/mappings" && cp -R "$ROOT/crates/ssf2-converter/mappings" "$1/mappings"
rm -rf "$1/runtime" && cp -R "$ROOT/crates/ssf2-converter/runtime" "$1/runtime"
for f in commands.hsx match_settings.conf src/peptide_ui.html; do
  [ -f "$ROOT/$f" ] && cp "$ROOT/$f" "$1/assets/$(basename "$f")"
done
echo "PEPTIDE_BIN=$1/peptide"
