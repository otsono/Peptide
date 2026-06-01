#!/usr/bin/env bash
# ab_compare.sh — behavioral A/B regression check for a converted character.
#
# Runs a recipe against a character, distills a stable "behavioral signature" from
# the engine readback (the set of animation states reached + per-move dispatch
# results + resting position/damage — the timing-variable bits like frame indices
# and the gloss are normalized out), and either SAVES it as a golden or DIFFS the
# current run against the saved golden. Use it to catch behavioral regressions
# after a converter change (re-run; a clean diff = no behavioral change), or to
# A/B two converter builds of the same character.
#
# Usage:
#   ./ab_compare.sh <char> <recipe> --save     # capture golden -> recipes/<char>.golden
#   ./ab_compare.sh <char> <recipe>            # diff current run vs golden (exit 1 on drift)
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
CHAR="${1:?usage: ab_compare.sh <char> <recipe> [--save]}"
REC="${2:?usage: ab_compare.sh <char> <recipe> [--save]}"
MODE="${3:-diff}"
GOLDEN="$HERE/recipes/${CHAR}.golden"

# Distill a stable signature: deduped ANIM states, move acks (M:*), state reads
# (T:*), match liveness (Q:*), and physics (P:) with floats rounded so sub-pixel
# jitter doesn't churn. Drops SENT/LOOP/SEQ lines, the "(gloss)", and A: frame
# indices (timing-variable).
signature() {
  grep -aE "^<< (ANIM:|M:|T:|Q:|P:|LAUNCHED)" \
    | sed -E 's/ +\(.*\)$//; s/^<< //' \
    | sed -E 's#(P: x=)(-?[0-9]+)\.[0-9]+#\1\2#g; s#(y=)(-?[0-9]+)\.[0-9]+#\1\2#g' \
    | awk '!(/^ANIM:/ && $0==prev); {prev=$0}'   # collapse consecutive dup ANIM lines
}

# Force the recipe to run as $CHAR (override any #!char and rewrite the spawn target)
# so the golden is actually this character's behavior, not the recipe's baked char.
TMPREC="$(mktemp -t abrec).recipe"
{
  echo "#!char $CHAR"
  sed -E "/^#!char/d; s/^(spawn|start|launch)[[:space:]]+[A-Za-z0-9_]+/\1 $CHAR/" "$REC"
} > "$TMPREC"
trap 'rm -f "$TMPREC"' EXIT
OUT=$(FRAY_PORT="${FRAY_PORT:-$(( (RANDOM % 2000) + 20300 ))}" "$ROOT/tools/recipe.sh" "$TMPREC" 2>&1)
SIG=$(printf '%s\n' "$OUT" | signature)

if [ "$MODE" = "--save" ]; then
  mkdir -p "$HERE/recipes"
  printf '%s\n' "$SIG" > "$GOLDEN"
  echo "[ab] saved golden ($(printf '%s\n' "$SIG" | wc -l | tr -d ' ') lines) -> $GOLDEN"
  exit 0
fi

[ -f "$GOLDEN" ] || { echo "[ab] no golden for $CHAR — run with --save first ($GOLDEN)"; exit 2; }
if diff -u "$GOLDEN" <(printf '%s\n' "$SIG") > /tmp/ab_${CHAR}.diff; then
  echo "[ab] $CHAR: behavioral signature UNCHANGED vs golden ✓"
else
  echo "[ab] $CHAR: behavioral DRIFT vs golden:"; cat /tmp/ab_${CHAR}.diff
  exit 1
fi
