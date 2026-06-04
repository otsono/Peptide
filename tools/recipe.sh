#!/usr/bin/env bash
# recipe.sh — run a Peptide "recipe": a shareable text file of friendly commands
# driven into one engine session. The reusable/scriptable form of a manual
# runseq.sh sequence (see docs/PEPTIDE_GUIDE.md "recipes").
#
# Recipe file format (one directive per line):
#   # comment            — ignored
#   #!char  <id>         — character to boot headless (default sandbag)
#   #!stage <id>         — stage (default thespire)
#   #!assist <id>        — assist (default commandervideoassist)
#   #!gap   <seconds>    — seconds between commands (default 3)
#   <command>            — any friendly Peptide command: spawn / move <name> /
#                          state / physics / anim / snapshot / loop <move> / …
# Blank lines are skipped. The `spawn` is usually the first command; if the recipe
# omits it, add one (the engine needs a match before move/state/etc.).
#
# Usage:  ./recipe.sh <recipe-file>
#   Env FRAY_DIR overrides the install path; FRAY_PORT picks the loopback port.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
REC="${1:?usage: recipe.sh <recipe-file>}"
[ -f "$REC" ] || { echo "no such recipe: $REC" >&2; exit 1; }

char="sandbag"; stage="thespire"; assist="commandervideoassist"; gap="3"
cmds=()
while IFS= read -r line || [ -n "$line" ]; do
  # strip trailing CR, leading/trailing space
  line="${line%$'\r'}"
  case "$line" in
    '#!char '*)   char="${line#*#!char }";   char="${char// /}";   continue ;;
    '#!stage '*)  stage="${line#*#!stage }";  stage="${stage// /}";  continue ;;
    '#!assist '*) assist="${line#*#!assist }"; assist="${assist// /}"; continue ;;
    '#!gap '*)    gap="${line#*#!gap }";      gap="${gap// /}";      continue ;;
    '#'*) continue ;;          # comment
    '') continue ;;            # blank
  esac
  cmds+=("$line")
done < "$REC"

echo "[recipe] $REC → char=$char stage=$stage assist=$assist gap=${gap}s, ${#cmds[@]} command(s)"
FRAY_CHAR="$char" FRAY_STAGE="$stage" FRAY_ASSIST="$assist" \
  "$HERE/runseq.sh" "$gap" "${cmds[@]}"
