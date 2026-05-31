# Peptide — a modder's guide

Peptide lets you **drive a running Fraymakers from the command line**: spawn a
character, make it perform any move, and read back what happens — without
clicking through menus or mashing a controller. It's built for testing converted
SSF2 characters, but it's useful for any Fraymakers mod work.

> Peptide drives your *local* Fraymakers install over a loopback socket by
> patching a throwaway copy of the engine bytecode at launch. It never modifies
> your real install (beyond the `custom/<id>/` mod folder the publish step
> writes) and contains no Fraymakers code. macOS; needs the engine present.

## 30-second start

```
cd tools/peptide
./run.sh "spawn sandbag" 20          # boot, spawn sandbag, hold 20s
```

You'll see the engine boot, then lines like:

```
<< LAUNCHED private::sandbag.sandbag ...   (match launched: ...)
<< ANIM:INTRO                              (animation: INTRO)
<< ANIM:STAND                              (animation: STAND)
```

`ANIM:` lines stream as the character changes state. The `(...)` after each line
is a plain-English gloss.

## Commands

Run `./target/release/peptide-bridge help` (no engine needed) for the live list.
You type full words; single-letter wire codes still work as aliases.

| Command | What it does |
|---|---|
| `spawn <char> [stage] [assist]` | start a match (loads your custom `.fra`); stage/assist default to thespire / commandervideoassist |
| `move <name>` | perform a move on player 0 — `jab`, `tilt_down`, `strong_forward`, `aerial_up`, `special_neutral`, `grab`, … (no arg = jab) |
| `state` | print the current state name (`T:STAND`) |
| `physics` | print player 0 position / velocity / damage (`P: x=.. y=.. vx=.. vy=.. dmg=..`) |
| `anim` | print player 0 current animation + frame (`A:<name> frame <cur>/<total>`) |
| `loop <move> [count]` | re-dispatch a move repeatedly (default 8×) for sustained observation / live tuning |
| `snapshot` | one bundle of state + physics + animation (`t`+`v`+`a`) |
| `step` / `play` | frame-step: pause + advance the animation one frame (`step`), resume (`play`) — scrub through a move |
| `track <move> [n]` | drive a move then rapid-sample physics — capture its velocity/position trajectory (self-momentum) |
| `query` | is a match live? |
| `keys` | dump the loaded resource keys |
| `exit` | cleanly shut the engine down |
| `help` | list everything |

`move` names follow the Fraymakers vocabulary, so they're guessable
(`special_neutral`, `aerial_forward`, `tilt_up`). `help` prints the full set.

## Driving a sequence (one engine session)

A match doesn't survive a reboot, so to drive several commands against the *same*
match use `runseq.sh <gap_seconds> "cmd1" "cmd2" …`:

```
./runseq.sh 3 "spawn mario" "state" "move special_neutral" "physics" "move grab"
```

It boots once, fires the first command at engine-ready, paces the rest by
`gap_seconds`, and shuts the engine down cleanly at the end. Read the `<<` lines
to see each move's `ANIM:` transition and `M:OK` ack.

## The iteration loop (converted character → fix → re-test)

When a converted character misbehaves:

```
# 1. regenerate from the converter
./target/release/ssf2_converter ../ssf2-ssfs/<id>.ssf

# 2. publish a FRESH .fra  (ALWAYS — an old .fra silently lingers and lies)
node tools/fraytools-harness/export-in-fraytools.js \
  --project "$PWD/characters/<id>/<id>.fraytools"

# 3. drive it and observe
cd tools/peptide
FRAY_CHAR=<id> ./runseq.sh 3 "spawn <id>" "move jab" "move special_neutral" "physics"
```

**The #1 gotcha:** the published `.fra` in `custom/<id>/` is *not* rebuilt by the
converter — only by the FrayTools publish step. If you skip step 2 you're testing
stale output. (This caused a phantom "mario crash" that was just a 3-day-old
`.fra`.)

## Batch-testing many characters

`tools/peptide/batch_spawn_test.sh <id> <id> …` regenerates, exports, and
spawn-drives each character, recording PASS/FAIL (PASS = launched + reached STAND
+ moves dispatched + no crash) to `/tmp/batch_results.txt` (override with
`BATCH_RESULTS=`). Good for a regression sweep after a converter change.

## Recipes (shareable scripts)

`tools/peptide/recipe.sh <file>` runs a **recipe** — a text file of friendly
commands (plus `#!char`/`#!stage`/`#!gap` directives) driven into one engine
session. Reusable + shareable (e.g. `recipes/mario_moveset.recipe`). The
scriptable form of a manual `runseq.sh` sequence.

## A/B regression check (golden behavioral signature)

`tools/peptide/ab_compare.sh <char> <recipe> --save` captures a character's
behavioral signature (anim states + move acks + resting position, timing noise
normalized out) as a golden; re-running without `--save` diffs against it and
exits non-zero on drift. Use it to catch behavioral regressions after a converter
change, or to A/B two builds of the same character.

## Parity verification (does it behave like SSF2?)

`DUMP_PARITY=1 ./target/release/ssf2_converter ../ssf2-ssfs/<id>.ssf` dumps the
raw SSF2 hitbox values; `tools/parity_check.py <id>` diffs them against the
generated `HitboxStats.hx` (damage/angle/knockback/hit-freeze) + reports hitbox
frame-coverage. All 45 characters currently pass. For move *momentum*, drive
`track <move>` and compare the velocity trajectory to SSF2. See `docs/PARITY.md`.

## Where things are

- Friendly command vocabulary: `tools/peptide/src/commands.rs` (one table, shared
  by the client and the patcher — edit here to add/rename a command).
- Full engine RE map + validation status: `TESTING.md`.
- Layering and design decisions: `docs/PEPTIDE_ARCHITECTURE.md`.
- Where Peptide is headed (live hitbox tuning, `verify`, `loop`, dummy opponent):
  `docs/PEPTIDE_FUTURE.md`.

## Coming soon (see PEPTIDE_FUTURE.md)

`loop <move>` (repeat a move), `dummy` (spawn a hit target), `hitbox` (dump active
box stats for the current frame), `verify <move>` (diff behavior vs the SSF2
reference), and live stat tuning. Built incrementally on the commands above.

## The console UI (recommended for interactive use)

```
cd tools/peptide
./run-ui.sh
```

Boots Fraymakers and opens a full-screen console (built on ratatui). Type a command
at the bottom, press Enter, and replies stream into the color-coded scrollback above:

- **Tab** — command palette (pick a starter command)
- **↑ / ↓** — command history
- **PgUp / PgDn** — scroll the output
- **F1** — help panel
- **Esc / Ctrl+C** — quit (shuts the engine down cleanly)

Everything you can type in the CLI works here, including raw hscript:
`match.getCharacters()`, `p0.getStateName()`, `p0.body.x`, `Engine.log("hi")`. Errors
never crash the engine — they show in the scrollback (red) and in `Engine.log`.

Use the headless `run.sh` / `runseq.sh` for scripted/automated runs; use `run-ui.sh`
for interactive exploration.
