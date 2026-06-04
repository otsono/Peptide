# CLAUDE.md

agent on-ramp for this repo. read this first, then jump to the doc it points you at for
whatever you're doing.

**what this is:** Peptide -- a Fraymakers modding toolkit (one Rust binary) that (1) converts
SSF2 characters into Fraymakers mods, (2) drives the live engine to test them, and (3) drives
FrayTools to publish. see [`README.md`](README.md) for the pitch.

## where to look

- [`AGENT_CONTEXT.md`](AGENT_CONTEXT.md) -- the authoritative SSF2 / Fraymakers **format**
  reference (`.ssf` internals, the `.entity` schema) plus the conventions below. read it before
  touching `entity_gen.rs`, `image_extractor.rs`, or `sprite_parser.rs`.
- [`DEVELOPMENT.md`](DEVELOPMENT.md) -- converter dev guide: build, the pipeline, every module,
  the mapping config, output layout.
- [`TESTING.md`](TESTING.md) -- the two validation harnesses + the iteration loop.
- [`docs/STATUS.md`](docs/STATUS.md) -- the single home for status, known issues, and TODOs.
- [`CONTRIBUTING.md`](CONTRIBUTING.md) -- the per-change checklist (run it before a PR).

## must-knows (don't learn these the hard way)

- **edit the JSONC, not the Rust.** most conversion behavior lives in
  `crates/ssf2-converter/mappings/*.jsonc` (API renames, stat scaling, anim names). tune the
  data, not the code. (DEVELOPMENT §6.)
- **engine-side code: minimum bytecode, maximum hscript.** put engine behavior in
  `commands.hsx`, not hand-emitted bytecode. (AGENT_CONTEXT.)
- **ONE command vocabulary, TWO engines.** a host-facing feature must behave the same on
  Fraymakers and SSF2 via the `DebugTarget` seam, never an `if engine == ...` branch in feature
  logic. (enforced by `tests/conventions.rs`.)
- **compliance: no engine-RE in the docs.** never put the methodology for decompiling/patching
  the Fraymakers engine or FrayTools, or specific non-hscript engine
  class/function/field names, in any tracked file. (AGENT_CONTEXT "engine-side knowledge is not
  in this repo"; enforced by `tests/doc_freshness.rs`.)
- **docs describe current state only, in lowercase casual voice, no em-dashes.** no "(fixed)"
  archaeology, no commit refs, no "previously/formerly". (em-dashes enforced by
  `tests/conventions.rs`.)
- **keep docs succinct.** lead with the point, cut the throat-clearing and the AI tells (no
  "it's not just X, it's Y" antithesis, no rule-of-three padding). say it once: each topic has
  one home (status/TODOs live in `docs/STATUS.md`, the format reference in `AGENT_CONTEXT.md`),
  so link across docs instead of restating. prefer a table or list over a paragraph when it
  carries the same info shorter. if a sentence doesn't change what the reader does, delete it.
- **the test corpus isn't in the repo.** the SSF2 `.ssf` files are yours to supply (default
  `../ssf2-ssfs/`, or set `$SSF2_SSFS_DIR`). you don't need them to build or `cargo test` --
  corpus-dependent tests skip cleanly without them.

## before a PR

`cargo test --workspace` and `cargo clippy --workspace -- -D warnings` both green, then the CONTRIBUTING
checklist. there's a `justfile` with shortcuts (`just test`, `just lint`, `just convert <char>`,
`just smoke`).
