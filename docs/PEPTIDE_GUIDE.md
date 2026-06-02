# Peptide — a modder's guide

Peptide lets you **drive a running Fraymakers from the command line**: spawn a
character, make it perform any move, and read back what happens — without
clicking through menus or mashing a controller. It's built for testing converted
SSF2 characters, but it's useful for any Fraymakers mod work.

> Peptide drives your *local* Fraymakers install over a loopback socket by
> patching a throwaway copy of the engine bytecode at launch. It never modifies
> your real install (beyond the `custom/<id>/` mod folder the publish step
> writes) and contains **no Fraymakers code, assets, bytecode, or strings**.
> Engine bytecode, patched output, and `.fra` packages stay on your machine and
> are never committed. See [`NOTICE.md`](../NOTICE.md).

## How it works

`peptide` parses the engine's HashLink bytecode, injects a small per-frame block
into `fraymakers.Main.update`, and writes a patched throwaway copy. When run, the
patched engine:

1. waits for content load (the title screen's "press any button" state),
2. dials a loopback TCP socket back to Peptide (auth handshake first), and
3. on `spawn <char> <stage> <assist>`, builds a real `TrainingMode` and calls the
   engine's own offline match-start flow, so the match renders exactly as it would
   from the menus.

Content ids accept short names: a bare `commandervideo` is resolved by searching
the engine's loaded content registry by id, or you can pass a full
`namespace::package.id`.

## 30-second start

```
./tools/run.sh "spawn sandbag" 20          # boot, spawn sandbag, hold 20s
```

You'll see the engine boot, then lines like:

```
<< LAUNCHED private::sandbag.sandbag ...   (match launched)
<< ANIM:INTRO                              (animation: INTRO)
<< ANIM:STAND                              (animation: STAND)
```

`ANIM:` lines stream as the character changes state. The `(...)` after each line
is a plain-English gloss.

`tools/run.sh` is self-contained: it builds the binary, patches the bytecode into
the install dir, launches the engine, bridges the command, and cleans up
afterward. Override the install path with `FRAY_DIR=...`.

## Commands — and the hscript model

The command set is deliberately **tiny**. The real interface is **hscript**: any
line Peptide doesn't recognize as one of the commands below is run as an hscript
expression in the engine, against the full Fraymakers script API. Run
`./build/release/peptide help` (no engine needed) for the live list.

| Command | What it does |
|---|---|
| `spawn <char> [stage] [assist]` | start a match (loads your custom `.fra`); stage/assist default to thespire / commandervideoassist |
| `eval <hscript>` | run an hscript expression and print `E:<result>`. **Also the default for any unrecognized line** — you rarely type `eval` explicitly |
| `hold <control[+control…]>` | hold control inputs (e.g. `hold down+special`) — feeds the engine's input→action mapping, not a synthetic keypress |
| `release` | release all injected controls |
| `seq <controls:frames> …` | play a frame-accurate input timeline (e.g. `seq down+special:2 right:12`) — one input per engine frame, auto-releases at the end |
| `load` | synchronous custom-`.fra` load probe (diagnostic; `spawn` does this itself) |
| `console` | run the engine's debug-console `help` |
| `exit` | cleanly shut the engine down |
| `help` | list these + the hscript model (client-side; sends nothing) |

Everything else is hscript. The engine interpreter already exposes the entire
Fraymakers script API (`CState`, `HitboxStats`, `Assist`, `MatchModifier`,
`Announcer`, …) plus live character access, so instead of a per-feature command
you write the script directly:

```
match.getCharacters()[0].getStateName()                  # current state
match.getCharacters()[0].toState(CState.SPECIAL_NEUTRAL) # drive a move
match.getCharacters()[0].damage._damage                  # read damage %
match.getCharacters()[0].body.x                          # read position
log("hi")                                                # write to the in-game console
```

The few in-engine helpers that wrap the live match/character context (which a
non-entity script otherwise can't reach) live in [`commands.hsx`](../commands.hsx)
— `match.getCharacters()`, `matchStatus()`, `log()`. The host-side command
vocabulary and input controls live in [`src/interpreter.rs`](../src/interpreter.rs),
shared by the bridge and the patcher. See
[`PEPTIDE_DESIGN.md`](PEPTIDE_DESIGN.md) for how the two layers fit together.

## Driving a sequence (one engine session)

A match doesn't survive a reboot, so to drive several commands against the *same*
match use `tools/runseq.sh <gap_seconds> "cmd1" "cmd2" …`:

```
./tools/runseq.sh 3 "spawn mario" \
  "match.getCharacters()[0].getStateName()" \
  "match.getCharacters()[0].toState(CState.SPECIAL_NEUTRAL)" \
  "match.getCharacters()[0].body.x"
```

It boots once, fires the first command at engine-ready, paces the rest by
`gap_seconds`, and shuts the engine down cleanly at the end. Every line after
`spawn` is hscript, run against the live match.

## The iteration loop (converted character → fix → re-test)

When a converted character misbehaves:

```
# 1. regenerate from the converter
./build/release/peptide convert ../ssf2-ssfs/<id>.ssf

# 2. publish a FRESH .fra  (ALWAYS — an old .fra silently lingers and lies)
./build/release/peptide export \
  --project "$PWD/characters/<id>/<id>.fraytools"

# 3. drive it and observe
FRAY_CHAR=<id> ./tools/runseq.sh 3 "spawn <id>" \
  "match.getCharacters()[0].toState(CState.JAB)" \
  "match.getCharacters()[0].toState(CState.SPECIAL_NEUTRAL)" \
  "match.getCharacters()[0].body.x"
```

**The #1 gotcha:** the published `.fra` in `custom/<id>/` is *not* rebuilt by the
converter — only by the FrayTools publish step. If you skip step 2 you're testing
stale output.

## Batch-testing, recipes, and regression checks

- **`tools/tests/batch_spawn_test.sh <id> <id> …`** regenerates, exports, and
  spawn-drives each character, recording PASS/FAIL (PASS = launched + reached STAND
  + moves dispatched + no crash). Good for a regression sweep after a converter
  change. Pass `FRAY_PORT=<n>` / `BATCH_RESULTS=<file>` to avoid collisions across
  concurrent runs.
- **`tools/recipe.sh <file>`** runs a *recipe* — a text file of friendly commands
  (plus `#!char`/`#!stage`/`#!gap` directives) driven into one engine session.
  Reusable and shareable (e.g. `tools/tests/recipes/mario_moveset.recipe`).
- **`tools/tests/ab_compare.sh <char> <recipe> --save`** captures a character's
  behavioral signature (anim states + move acks + resting position, timing noise
  normalized out) as a golden; re-running without `--save` diffs against it and
  exits non-zero on drift. Catches behavioral regressions across builds.

## Parity verification (does it behave like SSF2?)

`DUMP_PARITY=1 ./build/release/peptide convert ../ssf2-ssfs/<id>.ssf` dumps the raw
SSF2 hitbox values; `tools/tests/parity_check.py <id>` diffs them against the
generated `HitboxStats.hx` (damage/angle/knockback/hit-freeze) and reports hitbox
frame-coverage. For move *momentum*, drive the move and sample
`match.getCharacters()[0].body` / velocity over successive frames (via `seq` for
frame-accurate input, then repeated hscript reads), comparing the trajectory to
SSF2. See [`STATUS.md`](STATUS.md).

## The console UI (recommended for interactive use)

```
cargo build --release          # first time only
./build/release/peptide        # boots Fraymakers + opens the console
```

`peptide` is a single cross-platform executable — it patches a throwaway copy of
the engine, boots it, and opens a full-screen console (built on ratatui). Override
the install path with `FRAY_DIR` if Steam isn't in the default location. Type a
command at the bottom, press Enter, and replies stream into the color-coded
scrollback above:

- **Tab** — command palette (pick a starter command)
- **↑ / ↓** — command history
- **PgUp / PgDn** — scroll the output
- **F1** — help panel
- **Esc / Ctrl+C** — quit (shuts the engine down cleanly)

Everything you can type in the CLI works here, including raw hscript:
`match.getCharacters()`, `p0.getStateName()`, `p0.body.x`, `Engine.log("hi")`.
Errors never crash the engine — they show in the scrollback (red) and in
`Engine.log`. Use the headless `tools/run.sh` / `tools/runseq.sh` for
scripted/automated runs; use the console for interactive exploration.

## Match settings (`match_settings.conf`)

Headless matches default to **999 lives, no timer**. Tune this without touching
Rust by editing [`match_settings.conf`](../match_settings.conf) (`key = value`,
`#` comments):

```
lives = 999   # stock count per player
time  = 0     # match timer in seconds (0 = unlimited)
```

The file is read at patch time, so an edited copy takes effect on the next run
with no rebuild. Resolution order: `$PEPTIDE_MATCH_SETTINGS` → `match_settings.conf`
next to the binary → the repo's `match_settings.conf` → `./match_settings.conf` →
the built-in default.

## See also

- **Design and internals** — layering decisions, the version-resilience strategy
  (resolve by name, never by index), and the feature roadmap:
  [`PEPTIDE_DESIGN.md`](PEPTIDE_DESIGN.md).
- **Engine RE map + validation harnesses** — [`TESTING.md`](../TESTING.md).
- **Current parity / character status** — [`STATUS.md`](STATUS.md).
