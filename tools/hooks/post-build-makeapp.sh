#!/usr/bin/env bash
# PostToolUse(Bash) hook: keep build/Peptide.app in sync with the compiled binary, so the
# GUI bundle always matches whatever is under test (the user wants to open the app and watch).
#
# We key off the BINARY, not the command text: any Bash call may have recompiled the binary
# (cargo build, cargo run, cargo test, a script that builds, …), so matching command strings
# misses most of them. Instead, after every Bash call we compare mtimes and re-bundle only when
# the freshly built binary is newer than the one inside the .app (or the bundle is missing).
# This is cheap, never recurses (make-app's internal cargo build is not a tool call), and a
# failed build never produces a newer binary so it won't trigger a stale re-bundle.
#
# Runs from the repo root (hook cwd); never fails the hook. Each worktree has its own build/,
# so this only ever refreshes the bundle belonging to the checkout that did the compiling.
set -uo pipefail

bin="build/release/peptide"
app_bin="build/Peptide.app/Contents/MacOS/peptide"

# Nothing compiled yet -> nothing to bundle.
[ -f "$bin" ] || exit 0

# Bundle is current -> skip. (-nt is false when $app_bin is missing, so a missing bundle
# falls through to a rebuild.)
if [ -f "$app_bin" ] && ! [ "$bin" -nt "$app_bin" ]; then
  exit 0
fi

./tools/make-app.sh --no-open >/tmp/peptide-makeapp.log 2>&1 || true
exit 0
