# Peptide design — architecture, version resilience, roadmap

Peptide drives a running Fraymakers engine for testing converted characters (and,
increasingly, for live mod development). This doc records the **layering and
stability decisions** so the system stays maintainable and survives engine
updates. The guiding rule:

> **Push logic to the highest, most readable layer it can live in.**
> Hand-written HashLink opcodes are the layer of last resort.

## The layers (lowest → highest)

| Layer | Where | What lives here |
|---|---|---|
| 1. Bytecode dispatch | `src/main.rs` `connect_edit` (+ `src/asm.rs`) | The thin per-frame skeleton spliced into `fraymakers.Main.update`: socket connect/auth and a single-byte wire dispatch. Irreducible (HashLink has no plugin hook) but kept minimal. Where a handler must still be bytecode, it's emitted from Rust data tables via the `Asm` helper (registers + jump offsets resolved at build time, turning runtime engine crashes into patch-time errors) — not hand-placed. |
| 2. Host vocabulary / routing | `src/interpreter.rs` | The Rust front end: translates the small friendly command set into wire bytes, and forwards **every unrecognized line to the `e` (eval) handler as hscript**. Deliberately flat — a tiny `COMMANDS` table plus helpers (`expand_sequence`, `controls_mask`) that aren't meant to be invoked on their own. It never runs game logic; it only routes. Shared by both the patcher and the bridge so the host surface and the patched protocol can't drift. |
| 3. In-engine command vocabulary | `commands.hsx` | The human-readable, **non-helper** commands you actually run for tests and scripting — `match.getCharacters()`, `matchStatus()`, `log()`, the icon feeds. Loaded once into the engine-scoped interpreter by the `e` hook. The Fraymakers script API (`CState`, `HitboxStats`, `Assist`, …) is already in scope; `commands.hsx`'s structural job is to expose the live match/character context a non-entity script otherwise lacks. **This is the most-used surface.** |
| 4. Shell orchestration | `tools/run.sh`, `tools/runseq.sh`, `tools/tests/*` | Boot lifecycle, multi-command sequencing, batch sweeps, cleanup. |

### The two seams

**Bytecode ↔ host (layers 1–2).** The wire protocol is deliberately minimal:
**one byte selects a handler** (`s` spawn, `e` eval-hscript, `i` input, `l` load,
`c` console, `x` exit); only the arg-bearing bytes (`s`/`e`/`i`) drain a trailing
line. The engine never parses words. `interpreter.rs` turns what you type into
that wire line. Because anything it doesn't recognize is forwarded verbatim to
`e`, the host vocabulary stays tiny — `interpreter.rs` adds only the few commands
that genuinely *need* bytecode side-effects (input injection via `hold`/`seq`, the
console call) plus the handful that bootstrap a session (`spawn`, `load`, `exit`).

**Host ↔ engine (layers 2–3).** The `e` handler hands the hscript text to the
engine's own `hscript.Interp` — the same interpreter that runs every character
script — so logic expressed as hscript is resolved by the engine's linker at
runtime and is immune to findex drift entirely. `commands.hsx` is what that
hscript can call beyond the stock script API.

### Where new features should go

The default home for a new capability is **`commands.hsx` (layer 3)** — write it
as hscript. The script API already exposes the whole engine surface; you almost
never need new bytecode. Escalate only when forced:

1. **hscript in `commands.hsx`** — the strategic direction and the right answer
   for nearly everything. Readable, drift-proof, the most-used surface.
2. **A new wire command in `interpreter.rs`** — only when the feature needs a
   bytecode side-effect the interpreter can't reach from script (input injection,
   a side-effecting engine call). Keep it flat: route to a thin bytecode handler,
   don't grow logic here.
3. **A Rust-generated `Asm` bytecode handler** — the last resort, only when even
   the wire command can't be done from hscript. Drive it from a data table via
   `Asm`; never hand-place opcodes.

## Version resilience — surviving Fraymakers updates

Fraymakers ships as compiled HashLink bytecode. **Every Fraymakers update is a
full recompile** that renumbers function indices (`findex`), field slots, and type
indices. A patcher that hardcodes those integers breaks silently — it injects into
whatever function now sits at the old index, corrupting the engine with no error.
Four rules, in priority order:

1. **Resolve by name, never by index.** Use the read-only resolvers in
   [`src/main.rs`](../src/main.rs) (`require_fn`/`find_fn` by name + parent type,
   `require_type`, `require_field`, …). A name survives a recompile as long as the
   symbol still exists. Raw `RefFun(N)` literals are a last resort, allowed only
   for unnameable symbols (e.g. unnamed enum types like `hscript.Expr`), each with
   a comment on how to re-derive it.
2. **Never silently fall back to a pinned index.** A name miss must fail **loudly**
   (`require_*` returns `Err`, aborting the patch). `find_fn(...).unwrap_or(N)` is
   dangerous: on a build where the symbol moved it injects into a stale index. The
   few that remain are non-critical paths (marked `critical: false` in the
   manifest); do not add new ones.
3. **Prefer hscript over hand-emitted bytecode.** The engine bundles the full
   `hscript` interpreter — the same `Parser` + `Interp` that runs every character
   script. The `e` command parses and executes script *text* in-engine, so logic
   expressed as an hscript string is resolved by the engine's own linker at runtime
   and is **immune to findex drift entirely**. The migration goal: the only brittle
   bytecode left is the minimal eval-bootstrap hook; every handler becomes readable
   Haxe text. Add new behavior as hscript unless there's a hard reason it must be
   bytecode.
4. **Avoid mid-function opcode-index injection.** Inserting ops at a fixed offset
   inside an engine function breaks if the engine changes a single opcode in that
   function. Prefer the layout-robust `insert_ops_front`/`insert_ops_end` helpers,
   or move the logic into hscript. Any remaining mid-function injection must assert
   its expected opcodes and abort loudly if they shifted.

### The symbol manifest + `doctor`

Every engine symbol the patcher depends on is declared in one place —
[`src/manifest.rs`](../src/manifest.rs) (`MANIFEST`), grouped by subsystem, each
entry tagged `critical` (a miss aborts the patch) or not (a miss warns). This is
the single source of truth — **any new engine dependency must be added here.**

```bash
# read-only preflight: resolve the whole manifest, print a checklist, exit
peptide "<install>/hlboot-sdl.dat" _ doctor
```

The same check (`run_preflight`) runs at the top of every real `connect` patch: it
**aborts before mutating a single opcode** if a critical symbol is missing — so an
incompatible build fails loudly and precisely instead of producing corrupt output.
Progress surfaces on the CLI/TUI (stderr checklist) and in the GUI boot modal
("Verifying engine N/71").

(The symbol *names* in `manifest.rs` are RE facts cited for interoperability, which
[NOTICE.md](../NOTICE.md) permits; only verbatim bytecode/disassembly/assets are
kept out of the repo.)

### Version-bump checklist

1. Point `FRAY_DIR` at the new install (or copy the new `hlboot-sdl.dat`).
2. Run `peptide <new hlboot-sdl.dat> _ doctor`. `0 critical missing` → already
   compatible; skip to the spawn-test.
3. For each `[MISS]`: find the symbol's new name with the read-only inspection
   modes (`inspect`, `fnsof`, `whoref`, `dis` — see [TESTING.md](../TESTING.md)),
   then update the entry in `manifest.rs` **and** the matching `require_*` call.
   Re-run `doctor` until clean.
4. Re-run the in-engine spawn-test (sandbag loads, plays, no freeze). `doctor`
   proves symbols *resolve*; the spawn-test proves the patch *behaves*.
5. If a handler was brittle bytecode, consider porting it to hscript (rule 3).

## Roadmap — toward a live mod-development tool

The bigger vision: steer the engine from the command line, drive the exact move
you're editing on repeat against a dummy, tweak stats, and see the result
immediately. The canonical use case is a **hitbox-stats live-tuning loop**: load a
stage with the character + a dummy, loop the attack you're editing, adjust a stat,
read back knockback distance / angle / KO %, and tune until it feels right.

Shipped today: `spawn`, input injection (`hold` / `seq` frame-accurate
timelines), and the hscript-eval surface — driving any move (`toState(CState.…)`),
reading state / position / velocity / damage, and the `matchStatus` telemetry feed
are all expressed as hscript against the live match (see
[`commands.hsx`](../commands.hsx)). Plus crash diagnostics, recipe scripting, and
A/B regression checks.

The remaining deep items all converge on **one missing capability: in-engine
measurement of emergent behavior** (what a move actually does to an opponent):

- **A dummy opponent** — spawn a second fighter as a hit target. Extends the
  proven single-player `spawn` handler to a 2-player array (gated opt-in to keep
  the proven path byte-equivalent) and binds `p1` in `commands.hsx`. Prerequisite
  for everything below.
- **Hit-result readback** — after a hit lands, report damage dealt, victim
  knockback distance + launch angle, and hitstun frames. Once `p1` is bound this
  is plain hscript (the same field reads already used on `p0`). The "is it a good
  angle / would it kill" data.
- **KO-threshold search** — binary-search the dummy's starting % for the lowest KO
  %. A driver loop (host-side, or an hscript helper) over the dummy + repeated
  moves + boundary detection.
- **Active-box dump** — every active hit/hurt box for the current frame. Reading
  the engine's nested per-anim/per-hitbox stats structure is more involved than the
  simple field reads, but still hscript over the script API.
- **`verify` harness** — drive a move, capture behavior, and diff against the SSF2
  reference values (already extracted into `HitboxStats.hx`). Turns parity from
  eyeballing into a pass/fail suite — the functional-parity harness.
- **Stat hot-reload** — re-read stats into a running match. Today's baseline is
  "edit → re-export → `spawn` again" (the `spawn` handler reloads a fresh `.fra`
  in-session); a mid-match in-place re-read is unresearched.

### The hscript / `.hl` direction

Features with real control flow should be written in Haxe (hscript text today, a
compiled `.hl` module later) rather than hand-emitted opcodes, so the fragile
hand-bytecode surface stops growing. HashLink bytecode is *monolithic* with no
runtime module-load facility, so a compiled-`.hl` path most likely means **merging**
a Haxe-compiled `.hl` into the engine bytecode at patch time (remapping every
findex / type index / string index across the two modules). `hlbc` (already a
dependency) gives read/write access to those tables, so it's mechanically possible,
but the index-remapping is a non-trivial cross-module linker — it needs a dedicated
feasibility spike before any feature depends on it. Until then, complex features
stay on the hscript-eval or Rust-generated-`Asm` path.

## IP boundary

Peptide contains **no** Fraymakers code, bytecode, strings, or assets. It reads the
user's *local* engine bytecode at runtime, writes a patched *copy*, and speaks a
loopback TCP protocol. Method/field/type names appear only as interoperability
facts in our own words. See [`NOTICE.md`](../NOTICE.md).
