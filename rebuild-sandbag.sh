#!/bin/bash
# rebuild-sandbag.sh — quick rebuild + re-export sandbag.ssf

set -e

echo "Building release binary..."
cargo build --release 2>&1 | tail -5

echo ""
echo "Converting sandbag.ssf..."
./target/release/ssf2_converter /Users/jimmy/.openclaw/workspace-main/ssf2-ssfs/sandbag.ssf 2>&1 | grep -E "INFO|WARN|ERROR" | tail -10

echo ""
echo "✓ Done! Output in ./characters/sandbag/"
ls -lh characters/sandbag/library/sprites/*.png | wc -l
echo "sprite images"
