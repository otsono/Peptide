#!/usr/bin/env bash
# Inventory EVERY stage, in parallel, and print one line each plus a roster total.
#
# Serial, this was an hour: a cargo-run per stage that spent all its time deciding not to
# rebuild. The tool is built once here and the stages run across the machine's cores, which
# turns the same sweep into a couple of minutes -- and a check nobody waits an hour for is a
# check that actually gets run.
set -u
cd "$(cd "$(dirname "$0")/../.." && pwd)"
SSFS="${SSF2_SSFS_DIR:-../ssf2-ssfs}"
OUT="${1:-/tmp/stage_inventory_all.txt}"
# HOW MANY AT ONCE IS A MEMORY QUESTION, NOT A CORE ONE. Rasterising a big stage holds most of
# a gigabyte, so one job per core took a 16GB machine down hard. Budget ~5GB per job against -- measured: three at a time on 16GB is fine, ten is not
# total RAM and never use more than half the cores, so the sweep leaves the machine usable.
# JOBS=n overrides. Measured on 16GB: three at a time still left under a gigabyte free, so the
# budget is deliberately generous -- a sweep that takes twice as long is better than one that
# takes the machine down.
_ram_gb=$(( $( (sysctl -n hw.memsize 2>/dev/null || echo $((8*1073741824))) ) / 1073741824 ))
_cpus=$(sysctl -n hw.ncpu 2>/dev/null || nproc)
_by_ram=$(( _ram_gb / 5 )); _by_cpu=$(( _cpus / 2 ))
_auto=$(( _by_ram < _by_cpu ? _by_ram : _by_cpu )); [ "$_auto" -lt 1 ] && _auto=1
JOBS="${JOBS:-$_auto}"
echo "sweeping $(ls "$SSFS"/stages/*.ssf | wc -l | tr -d ' ') stages, $JOBS at a time"

cargo build --release --bin peptide 2>&1 | grep -E "^error" && exit 1
cargo build --release --features dev-tools --bin ssf2_objgraph >/dev/null 2>&1

one() {
  n=$(basename "$1" .ssf)
  # each stage gets its own output dir: the runs are concurrent and share nothing
  r=$(PEPTIDE_STAGE_OUT="build/inv/$n" python3 tools/tests/stage_inventory.py "$n" 2>/dev/null \
      | grep -E "^ported=" | tail -1)
  echo "$n | ${r:-no inventory}"
}
export -f one

ls "$SSFS"/stages/*.ssf | xargs -P "$JOBS" -I{} bash -c 'one "$@"' _ {} > "$OUT"
sort -o "$OUT" "$OUT"

awk -F'LOST ANIMATION=' 'NF>1 {l+=$2; if($2>0) bad++} END {
  printf "\n%d stages, %d with something lost, %d animations lost in total\n", NR, bad+0, l+0 }' "$OUT"
