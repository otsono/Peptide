# Contributing

A working agreement for keeping the code, tests, and docs in sync as
the project evolves. The intent is light-touch: small habits that catch
the routine drift, not bureaucratic gates.

## Quick checklist (do before opening a PR / pushing a branch)

Three questions, in order:

1. **Did you change `src/**`?**
   Run `cargo test --release`. The `golden_sandbag` snapshot is the primary
   safety net — if its hashes change, you've changed character output and the
   golden needs an explicit update (`tests/golden/sandbag_hashes.txt`).

2. **Did you touch one of the "hot files" below?**
   Check the doc section listed next to it. If the change shifts the
   description, update the doc in the same commit.

3. **Did you add or rename anything user-facing?**
   - New CLI flag? Update README §"Usage" + DEVELOPMENT §3.3.
   - New `conversion_log.json` field? Update DEVELOPMENT §4 step [11].
   - New diagnostic binary? Add a row in DEVELOPMENT §5.7.
   - New mapping JSONC section? Update README + DEVELOPMENT §6.

That's it. The rest is tooling that backs the checklist (see "Tooling"
below).

## Hot file → doc-section map

When you change one of these, scan the listed doc section in the same
commit. If the change is significant enough to need a doc edit, the
linked section is where to put it.

| Hot file | Affected doc sections |
|---|---|
| `src/main.rs` (CLI, `detect_char_names`, `process_character`, `write_conversion_log`) | DEVELOPMENT §3.3, §4, §5.1; README §"Usage" if CLI flags change |
| `src/extractor.rs` (`CharacterData` struct) | DEVELOPMENT §5.4 |
| `src/abc_parser.rs` (Stage A bundle extraction, constructor walker, normalStats_id) | DEVELOPMENT §5.3; §14 "Architectural history" if detection logic shifts |
| `src/decompiler.rs` | DEVELOPMENT §5.3 |
| `src/sprite_parser.rs` (box geometry, xframe transforms) | DEVELOPMENT §5.4; AGENT_CONTEXT §"Collision Boxes" |
| `src/image_extractor.rs` (PNG / placement / skew bake / projectile-and-head discovery) | DEVELOPMENT §5.4; AGENT_CONTEXT §"Image Sprites" |
| `src/sound_extractor.rs` | DEVELOPMENT §5.4 |
| `src/api_mappings.rs` (translation pipeline) | DEVELOPMENT §5.5; README §"Configuring the conversion" |
| `src/mappings.rs` | DEVELOPMENT §5.2 |
| `src/entity_gen.rs` | DEVELOPMENT §5.6; AGENT_CONTEXT §"Fraymakers Entity Format" |
| `src/haxe_gen.rs` (output orchestrator + transformation banner) | DEVELOPMENT §5.6 |
| `src/palette_gen.rs` | DEVELOPMENT §5.6; README §"How costumes work" |
| `src/uuid_gen.rs` | DEVELOPMENT §5.6; AGENT_CONTEXT §"Top-Level Structure" if GUID seeding changes |
| `mappings/commands.jsonc` | DEVELOPMENT §6.1; README §"Configuring the conversion" |
| `mappings/character/*.jsonc` | DEVELOPMENT §6.2–§6.4 |
| `src/bin/*` | DEVELOPMENT §5.7 (add a row to the table) |
| `tests/*` | Update the test count in DEVELOPMENT §10 "current status" if the change is significant |

## When you delete code

Deletion is the doc-drift case the tests have the hardest time catching.
The convention:

- **Removed a function?** Grep the repo for references first:
  `git grep -n <fn_name> -- '*.md' '*.rs'`. Update any doc hits.
- **Removed a flag / mapping / output field?** Same drill. Especially
  watch for the `*` substring matches — `extract_costume_*` removed
  meant grepping for any `extract_costume_*` reference.
- **Removed a "known issue" because you fixed it?** Strike its entry
  from DEVELOPMENT §11 and the corresponding §12 next-step.

## When you fix a "known issue"

Three places usually need an edit:

1. **DEVELOPMENT §11** — strike or update the entry (including the
   "Code-quality backlog" subsection if it was an audit item).
2. **DEVELOPMENT §12** — strike the corresponding next-step.

## Tooling

We don't gate on docs. We do have two light checks:

- **`tests/doc_freshness.rs`** — flags known-stale terms in the docs
  (references to deleted functions, doc paragraphs that describe a
  detection paradigm we replaced, etc.). Runs in `cargo test`. Add a
  new pattern any time you remove something user-visible and want a
  trip-wire if a doc edit later re-references it.
- **`tests/constructor_walk_detection.rs ::
  corpus_constructor_walk_matches_path2_enumeration`** — guards the
  detection invariant (constructor walk and the fallback enumeration
  agree across the corpus). Don't bypass this; fix the underlying
  divergence.

No pre-commit / pre-push hooks. They add friction for a one-person /
small-team flow and miss the real failure mode (someone confidently
breaking a doc reference). The test suite + the checklist are the
gates.

## Commit hygiene

- One logical change per commit. Big mixes (`fix X + refactor Y`) make
  doc drift harder to spot; split them.
- Commit message body explains the *why* and lists files-changed by
  intent group. See `git log` for recent examples.
- When the change touches docs, mention the section in the message:
  "DEVELOPMENT §5.3: updated abc_parser description." Future
  greppability.

## Architectural history

Why the code looks the way it does today lives in **`DEVELOPMENT.md` §14
"Architectural history"** (the path 1 → path 2 stat-extraction switch, the
constructor-walk detection, and the multi-character project layout). That's
exactly what a fresh agent or contributor needs when reading the source cold.

When you land a substantial design change, add a short condensed entry to
§14 — the *why* and the commit SHAs. Keep the **permanent** docs at the repo
top level; use the `docs/` folder only for *scratch* (see below), and promote
anything durable up before it gets lost.

## Scratch notes (`docs/`)

`docs/` is a **gitignored scratch space** for working notes during a task —
investigation logs, RE findings mid-flight, status snapshots, throwaway plans.
Nothing in it is tracked or ships.

The rule: **if a note in `docs/` would help anyone else** — a fact, a gotcha, an
RE finding, a design decision, a "what's still open" — **promote it into the
relevant top-level doc** (README / DEVELOPMENT / AGENT_CONTEXT / TESTING /
CONTRIBUTING / NOTICE), then the scratch note can be deleted. Don't let durable
knowledge accumulate only in `docs/`, because the next person (or agent) cloning
the repo never sees it. `docs/README.md` restates this for anyone who opens the
folder directly.
