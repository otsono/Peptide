#!/usr/bin/env bash
# PostToolUse(Bash) hook: after a successful `cargo build`, refresh build/Peptide.app
# so the GUI bundle always matches the freshly built binary (the user wants to be able
# to open the app and watch whatever is under test). Reads the hook JSON on stdin.
#
# PostToolUse only fires on a SUCCESSFUL tool call, so a failed build never reaches here.
# make-app.sh runs its own `cargo build` internally, but that is not a tool call, so this
# hook does not recurse. Runs from the repo root (hook cwd); never fails the hook.
set -uo pipefail

cmd=$(jq -r '.tool_input.command // ""' 2>/dev/null || echo "")
case "$cmd" in
  *"cargo build"*) ;;
  *) exit 0 ;;
esac

./tools/make-app.sh --no-open >/tmp/peptide-makeapp.log 2>&1 || true
exit 0
