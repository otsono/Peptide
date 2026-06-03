# Contributing

A working agreement for keeping the code, tests, and docs in sync as
the project evolves. The intent is light-touch: small habits that catch
the routine drift, not bureaucratic gates.

## Copyright boundary (read first)

SSF2 (© McLeodGaming) and Fraymakers / FrayTools (© Fraymakers) are
proprietary. **Never commit or publish their source, bytecode, decompiled
output, disassembly, stack traces, assets, or extracted strings** — not in
code, not in docs, not in commit messages. Reverse-engineering notes are fine
**in our own words** for interoperability; paraphrase, never paste. Full policy
in [`NOTICE.md`](NOTICE.md) "Reverse-engineering & copyright boundary". The
git-ignored `docs/` scratch folder is still subject to this — it just isn't the
place durable notes belong (see "Scratch notes" below).

## Quick checklist (do before opening a PR / pushing a branch)

Three questions, in order:

1. **Did you change `src/**` or `crates/**`?**
   Run `cargo test --release`. The `golden_sandbag` snapshot is the primary
   safety net — if its hashes change, you've changed character output and the
   golden needs an explicit update
   (`crates/ssf2-converter/tests/golden/sandbag_hashes.txt`).

2. **Did you touch one of the "hot files" below?**
   Check the doc section listed next to it. If the change shifts the
   description, update the doc in the same commit.

3. **Did you add or rename anything user-facing?**
   - New CLI flag? Update README §"Usage" + DEVELOPMENT §3.3.
   - New `conversion_log.json` field? Update DEVELOPMENT §4 step [11].
   - New diagnostic binary? Add a row in DEVELOPMENT §5.7.
   - New mapping JSONC section? Update README + DEVELOPMENT §6.

4. **Did you add a host-facing debug command or feature?**
   It must work **identically on both engines** (Fraymakers + SSF2). Define it once
   through the shared seam — a `DebugTarget` trait method (default = `eval`) plus the
   engine helper in `commands.hsx` AND `src/ssf2_target.rs` — never an
   `if engine == …` branch in command/feature logic. Full rule in
   [`AGENT_CONTEXT.md`](AGENT_CONTEXT.md) "ONE command vocabulary, TWO engines".

That's it. The rest is tooling that backs the checklist (see "Tooling"
below).

## Hot file → doc-section map

When you change one of these, scan the listed doc section in the same
commit. If the change is significant enough to need a doc edit, the
linked section is where to put it.

The converter source is the `crates/ssf2-converter/` library; Peptide (engine
harness, webview UI, bytecode patcher) is the root `src/`.

| Hot file | Affected doc sections |
|---|---|
| `crates/ssf2-converter/src/convert.rs` (orchestration: `detect_char_names`, `process_character`, `write_conversion_log`, `run_conversion`) | DEVELOPMENT §3.3, §4, §5.1 |
| `src/convert.rs` (peptide `convert` CLI adapter) | DEVELOPMENT §3.3; README §"Usage" if CLI flags change |
| `crates/ssf2-converter/src/extractor.rs` (`CharacterData` struct) | DEVELOPMENT §5.4 |
| `crates/ssf2-converter/src/abc_parser.rs` (Stage A bundle extraction, constructor walker, normalStats_id) | DEVELOPMENT §5.3 |
| `crates/ssf2-converter/src/decompiler.rs` | DEVELOPMENT §5.3 |
| `crates/ssf2-converter/src/sprite_parser.rs` (box geometry, xframe transforms) | DEVELOPMENT §5.4; AGENT_CONTEXT §"Collision Boxes" |
| `crates/ssf2-converter/src/image_extractor.rs` (PNG / placement / skew bake / projectile-and-head discovery) | DEVELOPMENT §5.4; AGENT_CONTEXT §"Image Sprites" |
| `crates/ssf2-converter/src/sound_extractor.rs` | DEVELOPMENT §5.4 |
| `crates/ssf2-converter/src/api_mappings.rs` (translation pipeline) | DEVELOPMENT §5.5; README §"Configuring the conversion" |
| `crates/ssf2-converter/src/mappings.rs` | DEVELOPMENT §5.2 |
| `crates/ssf2-converter/src/entity_gen.rs` | DEVELOPMENT §5.6; AGENT_CONTEXT §"Fraymakers Entity Format" |
| `crates/ssf2-converter/src/haxe_gen.rs` (output orchestrator + transformation banner) | DEVELOPMENT §5.6 |
| `crates/ssf2-converter/src/palette_gen.rs` | DEVELOPMENT §5.6; README §"How costumes work" |
| `crates/ssf2-converter/src/uuid_gen.rs` | DEVELOPMENT §5.6; AGENT_CONTEXT §"Top-Level Structure" if GUID seeding changes |
| Peptide root `src/` (gui.rs, peptide_ui.html, config.rs, platform.rs, manifest.rs, fraytools.rs) | docs/PEPTIDE_DESIGN.md; docs/PEPTIDE_GUIDE.md; TESTING.md |
| `crates/ssf2-converter/mappings/commands.jsonc` | DEVELOPMENT §6.1; README §"Configuring the conversion" |
| `crates/ssf2-converter/mappings/character/*.jsonc` | DEVELOPMENT §6.2–§6.4 |
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

## Architectural rationale

The *why* behind major design decisions (the path 1 → path 2 stat-extraction
switch, the constructor-walk detection, the multi-character project layout) lives
in `git log` / commit messages, not a dedicated doc section. When you land a
substantial design change, explain the *why* in the commit message. Keep the
durable, current-state docs at the repo top level (README / DEVELOPMENT /
AGENT_CONTEXT / TESTING / CONTRIBUTING / NOTICE) plus the Peptide docs in `docs/`;
don't accrete development history into them.

## The `docs/` folder

`docs/` holds the **tracked** Peptide harness docs — `PEPTIDE_GUIDE.md`,
`PEPTIDE_DESIGN.md`, `STATUS.md` (re-included via `.gitignore` negations) — and is
otherwise a **gitignored scratch space** for working notes during a task
(investigation logs, RE findings mid-flight, throwaway plans).

The rule: **if a scratch note would help anyone else** — a fact, a gotcha, an RE
finding, a design decision, a "what's still open" — **promote it into the relevant
durable doc** (a top-level doc, or one of the three `docs/` files), then delete the
scratch note. Don't let durable knowledge accumulate only in untracked scratch,
because the next person (or agent) cloning the repo never sees it.
