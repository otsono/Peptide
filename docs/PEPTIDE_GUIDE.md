# Peptide -- a modder's guide

Peptide drives a running Fraymakers from the command line: spawn a character, make it do any
move, and read back what happens. it's built for testing converted SSF2 characters, but it's
handy for any Fraymakers mod work.

> Peptide drives your *local* install over a loopback socket by booting a throwaway patched
> copy of the engine. it never touches your real install (beyond the `custom/<id>/` mod
> folder the publish step writes) and contains **no Fraymakers code, assets, bytecode, or
> strings**. the engine files, patched copy, and `.fra` packages stay on your machine. see
> [`NOTICE.md`](../NOTICE.md).

## how it works

`peptide` parses your local engine's HashLink bytecode, injects a small per-frame block, and
writes a patched **throwaway copy**. on launch the patched engine:

1. waits for content load (the title screen's "press any button" state),
2. dials a loopback TCP socket back to Peptide (auth handshake first), and
3. on `spawn <char> <stage> <assist>`, runs the engine's **own** offline match-start flow, so
   the match renders exactly like it would from the menus.

content ids accept short names: a bare `commandervideo` resolves against the engine's loaded
content, or pass a full `namespace::package.id`.

(the engine's internal symbol map stays out of the tracked repo at Team Fray's
request. see [`AGENT_CONTEXT.md`](../AGENT_CONTEXT.md) "engine-side knowledge is not in this
repo".)

## 30-second start

```
./tools/run.sh "spawn sandbag" 20          # boot, spawn sandbag, hold 20s
```

you'll see the engine boot, then lines like:

```
<< LAUNCHED private::sandbag.sandbag ...   (match launched)
<< ANIM:INTRO                              (animation: INTRO)
<< ANIM:STAND                              (animation: STAND)
```

`ANIM:` lines stream as the character changes state; the `(...)` gloss is plain english.
`tools/run.sh` builds the binary, patches the engine, launches it, bridges the command, and
cleans up after. override the install path with `FRAY_DIR=...`.

## commands -- and the hscript model

the command set is deliberately **tiny**. the real interface is **hscript**: any line Peptide
doesn't recognize as a command below runs as an hscript expression against the full Fraymakers
script API. run `./build/release/peptide help` for the live list.

| Command | What it does |
|---|---|
| `spawn <char> [stage] [assist]` | start a match (loads your custom `.fra`); stage/assist default to thespire / commandervideoassist |
| `eval <hscript>` | run an hscript expression and print `E:<result>`. **also the default for any unrecognized line** |
| `hold <control[+control…]>` | hold control inputs (e.g. `hold down+special`); feeds the engine's input→action mapping, not a synthetic keypress |
| `release` | release all injected controls |
| `seq <controls:frames> …` | play a frame-accurate input timeline (e.g. `seq down+special:2 right:12`), one input per frame, auto-releases at the end |
| `load` | synchronous custom-`.fra` load probe (diagnostic; `spawn` does this itself) |
| `console` | run the engine's debug-console `help` |
| `exit` | cleanly shut the engine down |
| `help` | list these + the hscript model (sends nothing) |

everything else is hscript. the engine already exposes the entire Fraymakers script API
(`CState`, `HitboxStats`, `Assist`, `MatchModifier`, `Announcer`, …) plus live character
access, so you write the script directly:

```
match.getCharacters()[0].getStateName()                  # current state
match.getCharacters()[0].toState(CState.SPECIAL_NEUTRAL) # drive a move
match.getCharacters()[0].damage._damage                  # read damage %
match.getCharacters()[0].body.x                          # read position
log("hi")                                                # write to the in-game console
```

the helpers that wrap the live match/character context (which a non-entity script can't
reach) live in [`commands.hsx`](../commands.hsx): `match.getCharacters()`, `matchStatus()`,
`log()`. the host-side command vocabulary lives in
[`src/interpreter.rs`](../src/interpreter.rs); see
[`PEPTIDE_DESIGN.md`](PEPTIDE_DESIGN.md) for how the layers fit.

## driving a sequence (one engine session)

a match doesn't survive a reboot, so to drive several commands against the *same* match use
`tools/runseq.sh <gap_seconds> "cmd1" "cmd2" …`. it boots once, fires the first command at
engine-ready, paces the rest by `gap_seconds`, and shuts down cleanly at the end:

```
./tools/runseq.sh 3 "spawn mario" \
  "match.getCharacters()[0].getStateName()" \
  "match.getCharacters()[0].toState(CState.SPECIAL_NEUTRAL)" \
  "match.getCharacters()[0].body.x"
```

## the iteration loop (converted character → fix → re-test)

when a converted character misbehaves:

```
# 1. regenerate from the converter
./build/release/peptide convert ../ssf2-ssfs/<id>.ssf

# 2. publish a FRESH .fra  (ALWAYS -- an old .fra silently lingers and lies)
./build/release/peptide export \
  --project "$PWD/characters/<id>/<id>.fraytools"

# 3. drive it and observe
FRAY_CHAR=<id> ./tools/runseq.sh 3 "spawn <id>" \
  "match.getCharacters()[0].toState(CState.JAB)" \
  "match.getCharacters()[0].toState(CState.SPECIAL_NEUTRAL)" \
  "match.getCharacters()[0].body.x"
```

**the #1 gotcha:** the published `.fra` in `custom/<id>/` only gets rebuilt by the FrayTools
publish step, not the converter. skip step 2 and you're testing stale output!

## batch-testing, recipes, and regression checks

- **`tools/tests/batch_spawn_test.sh <id> …`** regenerates, exports, and spawn-drives each
  character, recording PASS/FAIL (PASS = launched + reached STAND + moves dispatched + no
  crash). pass `FRAY_PORT=<n>` / `BATCH_RESULTS=<file>` to avoid collisions across concurrent
  runs.
- **`tools/recipe.sh <file>`** runs a *recipe*: a text file of friendly commands (plus
  `#!char`/`#!stage`/`#!gap` directives) driven into one session. reusable and shareable.
- **`tools/tests/ab_compare.sh <char> <recipe> --save`** captures a character's behavioral
  signature (anim states + move acks + resting position, timing noise normalized out) as a
  golden; re-running without `--save` diffs against it and exits non-zero on drift.

## parity verification (does it behave like SSF2?)

`DUMP_PARITY=1 ./build/release/peptide convert ../ssf2-ssfs/<id>.ssf` dumps the raw SSF2
hitbox values; `tools/tests/parity_check.py <id>` diffs them against the generated
`HitboxStats.hx` (damage/angle/knockback/hit-freeze) and reports hitbox frame-coverage. for
move *momentum*, drive the move and sample `match.getCharacters()[0].body` / velocity over
successive frames (use `seq` for frame-accurate input). see [`STATUS.md`](STATUS.md).

## the console UI (recommended for interactive use)

```
./build/release/peptide        # boots Fraymakers + opens the console
```

`peptide` patches a throwaway copy of the engine, boots it, and opens a full-screen console
(ratatui). override the install path with `FRAY_DIR` if Steam isn't in the default location.
type a command, press Enter, replies stream into the color-coded scrollback:

- **Tab** -- command palette
- **↑ / ↓** -- command history
- **PgUp / PgDn** -- scroll the output
- **F1** -- help panel
- **Esc / Ctrl+C** -- quit (shuts the engine down cleanly)

everything from the CLI works here, including raw hscript. errors never crash the engine, they
just show up red in the scrollback and in `Engine.log`. use `tools/run.sh` / `tools/runseq.sh`
for scripted runs, the console for poking around.

## match settings (`match_settings.conf`)

headless matches default to **999 lives, no timer**. tune that without touching Rust by
editing [`match_settings.conf`](../match_settings.conf):

```
lives = 999   # stock count per player
time  = 0     # match timer in seconds (0 = unlimited)
```

it's read at patch time, so an edit takes effect next run with no rebuild. resolution order:
`$PEPTIDE_MATCH_SETTINGS` → `match_settings.conf` next to the binary → the repo's copy →
`./match_settings.conf` → the built-in default.

## see also

- [`PEPTIDE_DESIGN.md`](PEPTIDE_DESIGN.md) -- layering, version-resilience, roadmap.
- [`TESTING.md`](../TESTING.md) -- validation harnesses + iteration loop.
- [`STATUS.md`](STATUS.md) -- parity / character status.
