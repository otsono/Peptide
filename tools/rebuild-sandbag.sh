#!/bin/bash
# rebuild-sandbag.sh — quick rebuild + re-export sandbag.ssf

set -e

# Run from the repo root regardless of where this script lives (tools/).
cd "$(cd "$(dirname "$0")/.." && pwd)"

echo "Building release binary..."
cargo build --release 2>&1 | tail -5

echo ""
echo "Converting sandbag.ssf..."
./build/release/peptide convert ../ssf2-ssfs/sandbag.ssf 2>&1 | grep -E "INFO|WARN|ERROR" | tail -10

echo ""
echo "✓ Done! Output in ./characters/sandbag/"
ls -lh characters/sandbag/library/sprites/*.png | wc -l
echo "sprite images"
