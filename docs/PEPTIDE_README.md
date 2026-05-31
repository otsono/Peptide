# Fraymakers match-launch harness

A CLI-driven programmatic link into a running **Fraymakers** that launches a
match (chosen character / stage / assist) directly into the engine, for fast
iteration testing of converted SSF2 characters — no menu navigation, no input
injection.

## IP boundary

This tool contains **no Fraymakers code, assets, bytecode, or strings**. It
only (a) reads the user's *local* Fraymakers bytecode at runtime, (b) writes a
patched copy back into the user's *local* install dir, and (c) speaks a simple
line protocol over a loopback TCP socket. Fraymakers is McLeodGaming
proprietary software; its bytecode (`hlboot-sdl.dat`), any patched output
(`_conn.dat`), and any `.fra` packages stay on the user's machine and are
**never committed** (see `.gitignore`).

## How it works

`peptide` parses the engine's HashLink bytecode (via the `hlbc` crate),
injects a small per-frame block into `fraymakers.Main.update`, and writes a
patched `_conn.dat`. When run, the patched engine:

1. waits for content load (the title screen's "press any button" state),
2. dials a loopback TCP socket back to `peptide-bridge` (auth handshake first), and
3. on receiving `s <char> <stage> <assist>`, builds a real `TrainingMode` and
   calls the engine's own offline match-start flow (gates transition + menu
   teardown), so the match renders exactly as it would from the menus.

Content ids accept short names: a bare `commandervideo` is resolved by
searching the engine's loaded content registry by id (type-segregated:
characters vs. stages), or you can pass a full `namespace::package.id`.

## Bins

- `peptide <in.dat> <out.dat> connect <port> <token>` — patch the bytecode.
  Also has read-only inspection modes: `dis <findex>`, `typefields <type>`,
  `fnsof <type>`, `fninfo <findex>`, `callers <findex>`, `strgrep <s>`,
  `whoref <s>`, `inspect`, and `doctor` (see below).
- `peptide-bridge serve|send|help --port <p> --token <t> ["<cmd>"]` — loopback bridge.

## Surviving Fraymakers updates

Fraymakers ships as compiled HashLink bytecode. **Every Fraymakers update is a
full recompile** that renumbers function indices (`findex`), field slots, and type
indices. A patcher that hardcodes those integers breaks silently — it injects into
whatever function now happens to sit at the old index, corrupting the engine with
no error. Peptide is built so that a new build is a *fast, self-diagnosing*
turnaround. Four rules, in priority order:

1. **Resolve by name, never by index.** Use the read-only resolvers in
   [`src/main.rs`](src/main.rs): `find_fn`/`require_fn` (name + parent type),
   `find_native`, `find_type`/`require_type`, `find_field`/`require_field`. A name
   survives a recompile as long as the symbol still exists. Raw `RefFun(12345)` /
   `function_index_by_findex(code, 12345)` literals are a last resort, allowed only
   for unnameable symbols (e.g. unnamed enum types like `hscript.Expr`), and each
   must carry a comment saying so and how to re-derive it.
2. **Never silently fall back to a pinned index.** A name miss must fail **loudly**
   (`require_*` returns `Err`, aborting the patch). `find_fn(...).unwrap_or(12345)`
   is dangerous: on a build where the symbol moved it injects into a stale index
   instead of telling you. The few that remain are non-critical paths (marked
   `critical: false` in the manifest and logged); do not add new ones.
3. **Prefer hscript over hand-emitted bytecode (the strategic direction).** The
   engine bundles the full `hscript` interpreter — the same `Parser` + `Interp`
   that runs every character script. The `e` command parses and executes script
   *text* in-engine (see [`prelude.hsx`](prelude.hsx)). Logic expressed as an
   hscript string is resolved by the engine's own linker at runtime, so it is
   **immune to findex drift entirely.** The migration goal: the only brittle
   bytecode left is the minimal eval-bootstrap hook; every handler (move dispatch,
   telemetry, content loading, queries) becomes readable Haxe text. Add new
   behavior as hscript in the prelude unless there is a hard reason it must be
   bytecode.
4. **Avoid mid-function opcode-index injection.** Inserting ops at a fixed offset
   inside an engine function (asserting `f.ops[6]` is a `JSGte`, etc.) breaks if
   the engine changes a single opcode anywhere in that function. Prefer the
   layout-robust `insert_ops_front`/`insert_ops_end` helpers, or move the logic
   into hscript. The one remaining mid-function injection (`inject_required_filter`)
   asserts its expected opcodes and aborts loudly if they shifted — match that
   discipline if you must do it again.

### The symbol manifest + `doctor`

Every engine symbol the patcher depends on is declared in one place:
[`src/manifest.rs`](src/manifest.rs) (`MANIFEST`), grouped by subsystem, each entry
tagged `critical` (a miss aborts the patch) or not (a miss warns). This is the
single source of truth — **any new engine dependency must be added here.**

```bash
# read-only preflight: resolve the whole manifest, print a checklist, exit
peptide "<install>/hlboot-sdl.dat" _ doctor
```

```
Peptide preflight — Fraymakers bytecode v<N>
  socket-bridge:
    [ ok ] sys.net.Socket                  #<findex>
    [ ok ] sys.net.Socket::connect         #<findex>
    ...
  hscript-eval:
    [ ok ] hscript.Interp::execute         #<findex>
    ...
  <resolved>/<total> resolved · 0 critical missing · 0 warnings
```

(The `#<findex>` values are placeholders — real findices are build-specific and
printed at runtime. The symbol *names* in `manifest.rs` are RE facts cited for
interoperability, which [NOTICE.md](../../NOTICE.md) permits; only verbatim
bytecode/disassembly/assets are kept out of the repo.)

The same check (`run_preflight`) runs at the top of every real `connect` patch: it
renders a progress bar while resolving and **aborts before mutating a single
opcode** if a critical symbol is missing — so an incompatible build fails loudly
and precisely instead of producing a corrupt `_conn.dat`. The progress surfaces
everywhere the patch runs:

- **CLI/TUI** (stderr is a TTY): an in-place progress bar + the checklist on stderr.
- **GUI** (default mode): the patcher runs as a subprocess and emits
  `@@PFP <done> <total> <label>` lines; [`src/ui.rs`](src/ui.rs)
  (`patch_and_launch_with_progress`) parses them and [`src/gui.rs`](src/gui.rs)
  forwards them to the boot modal's progress bar (`onPatchProgress` in
  [`src/peptide_ui.html`](src/peptide_ui.html), shown as "Verifying engine N/71").
  A failed patch surfaces the doctor's reason in the boot-failed message.

### Version-bump checklist

1. Point `FRAY_DIR` at the new install (or copy the new `hlboot-sdl.dat`).
2. Run `peptide <new hlboot-sdl.dat> _ doctor`. `0 critical missing` → already
   compatible; skip to the spawn-test.
3. For each `[MISS]`: find the symbol's new name with the read-only inspection
   modes (`inspect`, `fnsof`, `whoref`, `dis` — see
   [TESTING.md](../../TESTING.md) §3), then update the entry in `manifest.rs`
   **and** the matching `require_*` call in `connect_edit`. Re-run `doctor` until
   clean.
4. Re-run the in-engine spawn-test (sandbag loads, plays, no freeze) per
   [TESTING.md](../../TESTING.md). `doctor` proves symbols *resolve*; the
   spawn-test proves the patch *behaves*.
5. If a handler was brittle bytecode, consider porting it to hscript (rule 3)
   while you are in there.

## Commands (human-facing)

You drive the engine with **full-word commands**; the bridge translates them to
the engine's terse wire protocol. Run `peptide-bridge help` (no engine needed)
to print the list. The single-letter wire bytes still work as aliases, so old
scripts keep running.

| Command | Aliases | Args | Does |
|---|---|---|---|
| `spawn` | start, launch, s | `<char> [stage] [assist]` | start a match (loads custom content) |
| `move` | attack, m | `[move-name]` | drive a move on player 0 (no arg = jab) |
| `state` | status, t | | report player 0's state name |
| `query` | matchlive, q | | is a match live? |
| `load` | l | | synchronous custom-`.fra` load probe |
| `keys` | pool, k | | dump resource-pool keys |
| `console` | c | | run the engine console `help` |
| `ping` | p | | liveness (PONG) |
| `exit` | quit, stop, x | | clean engine shutdown |
| `help` | h, ? | | print this list (client-side) |

`move <name>` accepts the Fraymakers move vocabulary (`jab`, `tilt_down`,
`aerial_forward`, `special_neutral`, …); see `help` for the full set. The
friendly vocabulary lives in one place — [`src/commands.rs`](src/commands.rs) —
shared by the bridge and the patcher, so a future GUI can wrap it directly.

## Quick start

```
./run.sh "spawn sandbag thespire commandervideoassist" 20   # friendly
./run.sh "s commandervideo thespire commandervideoassist" 20 # wire bytes still work
```

`run.sh` is self-contained: it writes `steam_appid.txt`, builds the bins,
patches the bytecode into the install dir, launches the engine, and bridges the
command — cleaning up `_conn.dat` afterward. Override the install path with
`FRAY_DIR=...`.

## Match settings (`match_settings.conf`)

Headless matches default to **999 lives, no timer**. Tune this without touching
Rust by editing [`match_settings.conf`](match_settings.conf) (`key = value`,
`#` comments):

```
lives = 999   # stock count per player
time  = 0     # match timer in seconds (0 = unlimited)
```

The file is baked into the binary at build time *and* read from disk at patch
time, so an edited copy takes effect on the next `./run.sh` with **no rebuild**.
Resolution order: `$PEPTIDE_MATCH_SETTINGS` → `match_settings.conf` next to the
`peptide` binary → the repo's `tools/peptide/match_settings.conf` → `./match_settings.conf`
→ the built-in default. (Maps to the engine's `MatchSettingsConfig` `lives`/`time`
fields via the `matchSettings` virtual.)

## Status / known issues

See `memory/project_fraymakers-match-launch.md` for the full RE map and the
current open items (non-consuming socket read worked around with a one-shot
launch guard; assist content-type validation; `q` live-match query).
