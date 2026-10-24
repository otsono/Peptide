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
- [`docs/SSF2_PACKAGE_FORMAT.md`](docs/SSF2_PACKAGE_FORMAT.md) -- how an SSF2 stage package is
  BUILT (container, tag order, class graph, clip tree, api layer) and how to diagnose one that
  won't load. read it before authoring or debugging a package the game refuses.
- [`docs/PORTING_STAGES.md`](docs/PORTING_STAGES.md) -- the deep-RE playbook for porting a stage
  accurately (static inventory, AS3 disasm, live probing, coordinate model, verification gates).
  follow it phase by phase for EVERY stage.
- [`docs/PORTING_CHARACTERS.md`](docs/PORTING_CHARACTERS.md) -- the character analogue: the
  audit procedure for verifying a converted character move by move against both live engines.
- [`docs/STATUS.md`](docs/STATUS.md) -- the single home for status, known issues, and TODOs.
- [`CONTRIBUTING.md`](CONTRIBUTING.md) -- the per-change checklist (run it before a PR).

## must-knows (don't learn these the hard way)

- **edit the JSONC, not the Rust.** most conversion behavior lives in
  `crates/ssf2-converter/mappings/*.jsonc` (API renames, stat scaling, anim names). tune the
  data, not the code. (DEVELOPMENT §6.)
- **engine-side code: minimum bytecode, maximum hscript.** put engine behavior in
  `commands.hsx`, not hand-emitted bytecode. (AGENT_CONTEXT.)
- **THE converter must work on content that does not exist yet.** this is the first rule and
  everything else bends to it. the SSF2 corpus we have is a sample, not the target: there are
  infinite mods and content still to be made, and a conversion that only works because someone
  wrote the name down is a conversion that works for nothing new. so a converter change reads
  the SOURCE -- its script files, its classes, its manifest, the boxes and calls the format
  itself declares -- and never a list of names that ship in the game today.
  concretely, these are bugs, not shortcuts:
    - matching a content id or symbol name against a keyword list to decide WHAT something is
    - hardcoding stats for a named thing instead of reading the values the source declares
    - any table that has to gain a row before new content converts
  the sharpest form of the rule: **the converter never needs to know WHAT a thing is, only what
  its code says to do.** if you are writing code to decide which KIND of object you are looking
  at, stop, because that question has no answer for content nobody has written yet. SSF2 content
  carries its own classes; porting means TRANSLATING those classes, not recognising the things
  they describe.
  a converter-side TEMPLATE that only one piece of content fits is the same bug as a keyword
  list, just one step removed, and it is harder to see because the values inside it were read
  from the source. the test is not "did I read the numbers honestly", it is "would content
  nobody has written yet flow through this path". a template that answers no is SCAFFOLDING: it
  exists because that content could not yet run its own code, and the way out is to make its own
  code run, never to build a better selector to feed the template.
  so the order matters, and getting it backwards is how this goes wrong: make the general path
  (the object runs its own translated code) work FIRST, and the scaffolding then has no callers
  and deletes itself. deleting the selector first only forces you to invent a new one.
  if you find yourself about to add a keyword, a kind, or a category, that is the signal the
  detection is reading the wrong thing: go find where the source states it.
- **NEVER SLEEP. NEVER WAIT ON A WALL CLOCK. PACE EVERYTHING ON THE ENGINE'S FRAMES, AT SPEED.**
  a `sleep`, a fixed delay, a "give it a few seconds" is dead time the user sits through, and it
  under-measures too (animations get cut short). every wait in a probe, a gate, a tool or a
  script is `tick N` / `Engine.tick(n)` on the engine's own frame counter, after `speed 8`
  (both engines run 8 frames a tick), and a condition is waited for on the feed (`wait`,
  `reply`, `await`, `awaitmatch`), never polled with a delay. "take a shot after x frames" is
  `speed 8` + `tick x`, and takes a fraction of a second. the only sleeps allowed are a
  sub-second poll inside a feed reader that has no event to wake on. this rule is enforced by
  review: a diff that adds `sleep` is wrong until it says why that wait has no frame to count.
- **never convert the whole corpus, and never live-verify the whole stage corpus.** both take
  hours (the live one holds both engines the whole time) and stall every other iteration. prove
  a fix didn't regress other content with `cargo test` (the character goldens) + `just
  sweep-stages` (a fixed ten-stage sample chosen for coverage), and live-verify the stages the
  change was made for plus that sample. SCANNING the corpus is fine (`--info` parses
  without emitting, minutes not hours) when you need to know how widespread something is.
- **ONE command vocabulary, TWO engines.** a host-facing feature must behave the same on
  Fraymakers and SSF2 via the `DebugTarget` seam, never an `if engine == ...` branch in feature
  logic. (enforced by `crates/ssf2-converter/tests/conventions.rs`.)
- **compliance: no engine-RE in the repo, and that means BOTH engines.** never put the
  methodology for decompiling/patching an engine, its keys, or specific engine
  class/function/field names in any tracked file. this covers Fraymakers and FrayTools AND
  SSF2: the converter reads SSF2, but how SSF2 is built inside is not ours to publish.
  it applies to CODE COMMENTS as much as to docs, and to commit messages.
  where does the detail go: `docs/` is gitignored except for an explicit negation list, so a
  `*.local.md` there is the home for it (`docs/ENGINE_INTERNALS.local.md`,
  `docs/SSF2_ARCHIVE.local.md`, `docs/SSF2_MANIFEST_INTERNALS.local.md`). a tracked file may
  say WHAT a thing does and point at the local notes; it may not say how the engine does it.
  and prefer not to carry the game's own secrets at all: keys and the like can be recovered
  from the installed game at runtime, which is both compliant and survives them changing.
  (AGENT_CONTEXT "engine-side knowledge is not in this repo"; partly enforced by
  `tests/doc_freshness.rs` + `tests/conventions.rs`, whose needle lists are Fraymakers-side
  only, so the SSF2 half is on you.)
- **docs describe current state only, in lowercase casual voice, no em-dashes.** no "(fixed)"
  archaeology, no commit refs, no "previously/formerly". (em-dashes enforced by
  `crates/ssf2-converter/tests/conventions.rs`.)
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
