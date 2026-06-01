#!/usr/bin/env bash
# translation_completeness.sh — quantify how much SSF2 logic the converter left
# untranslated, per character, across generated Haxe output.
#
# Counts three marker classes in characters/<id>/library/scripts/**/*.hx:
#   ?   = "/* ? */"          decompiler couldn't recover an expr/condition/receiver
#   S   = "[SSF2-only:"      a call with no Fraymakers equivalent (commented out)
#   T   = "/*TODO*/" / "TODO" a value/stat the converter punted to a default
#
# Read-only. Lower is better. Use as a before/after metric when changing the
# decompiler or mappings: a fix should reduce the markers WITHOUT introducing new
# ones, and the in-engine spawn sweep (batch_spawn_test.sh) should still pass.
#
# Usage: tools/tests/translation_completeness.sh            # all converted characters
#        tools/tests/translation_completeness.sh mario kirby # specific ones
set -u
cd "$(cd "$(dirname "$0")/../.." && pwd)"
chars=("$@")
if [ ${#chars[@]} -eq 0 ]; then
  chars=()
  for d in characters/*/; do
    id="$(basename "$d")"
    [ "$id" = "misc" ] && continue           # misc.ssf is shared data, not a character
    [ -d "${d}library/scripts" ] && chars+=("$id")
  done
fi
printf "%-16s %6s %6s %6s\n" "character" "/*?*/" "SSF2" "TODO"
printf "%-16s %6s %6s %6s\n" "---------" "-----" "----" "----"
tot_q=0 tot_s=0 tot_t=0
for c in "${chars[@]}"; do
  dir="characters/$c/library/scripts"
  [ -d "$dir" ] || { printf "%-16s %6s %6s %6s\n" "$c" "-" "-" "-"; continue; }
  q=$(grep -rF "/* ? */" "$dir" 2>/dev/null | wc -l | tr -d ' ')
  s=$(grep -rF "[SSF2-only:" "$dir" 2>/dev/null | wc -l | tr -d ' ')
  t=$(grep -rEo "/\*TODO\*/|TODO" "$dir" 2>/dev/null | wc -l | tr -d ' ')
  printf "%-16s %6s %6s %6s\n" "$c" "$q" "$s" "$t"
  tot_q=$((tot_q + q)); tot_s=$((tot_s + s)); tot_t=$((tot_t + t))
done
printf "%-16s %6s %6s %6s\n" "---------" "-----" "----" "----"
printf "%-16s %6s %6s %6s\n" "TOTAL" "$tot_q" "$tot_s" "$tot_t"
