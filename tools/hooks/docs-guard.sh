#!/usr/bin/env bash
# PreToolUse(Write) hook: WARN (never block) when creating a NEW .md file in the repo
# outside docs/. The repo keeps only 6 canonical top-level docs; docs/ is throwaway
# scratch. Editing an existing doc, writing under docs/, and writing outside the repo
# (e.g. the ~/.claude auto-memory dir) are all silent. Reads the hook JSON on stdin.
#
# Warn-only: emits systemMessage (shown to the user) + additionalContext (fed to the
# model) and exits 0 with no permissionDecision, so the Write proceeds normally.

f=$(jq -r '.tool_input.file_path // ""' 2>/dev/null || echo "")

case "$f" in *.md) ;; *) exit 0 ;; esac                 # only .md files
case "$f" in "$PWD"/*) ;; *) exit 0 ;; esac             # only inside this repo
case "$f" in
  "$PWD"/docs/*|"$PWD"/build/*|"$PWD"/vendor/*|"$PWD"/.claude/*|*/node_modules/*) exit 0 ;;
esac
[ -e "$f" ] && exit 0                                    # editing an existing doc is fine

cat <<'JSON'
{"systemMessage":"Docs guard: about to create a NEW .md in the repo (outside docs/). Prefer folding into a canonical doc, or use docs/ for scratch.","hookSpecificOutput":{"hookEventName":"PreToolUse","additionalContext":"DOCS GUARD (warn-only): you are creating a new .md file outside docs/. This repo keeps only 6 canonical top-level docs: README.md, DEVELOPMENT.md, TESTING.md, CONTRIBUTING.md, AGENT_CONTEXT.md, NOTICE.md. Fold this content into one of those, or write it under docs/ (throwaway scratch). Do NOT add new top-level .md files unless the user explicitly asked for a new document."}}
JSON
exit 0
