# Peptide design -- architecture, version resilience, roadmap

Peptide drives a running Fraymakers engine for testing converted characters (and more and
more, for live mod development). this doc records the **layering and stability decisions** so
the system stays maintainable and survives engine updates. the guiding rule:

> **push logic to the highest, most readable layer it can live in.** hand-written
> engine-integration code is the layer of last resort.

> **compliance note.** this doc describes Peptide's *own* architecture and the patcher
> workflow, both totally fine to document. what's removed, at Team Fray's request,
> is the engine's internal **symbol map** (specific non-hscript engine class/function/field
> names, the load/dispatch/telemetry functions, the `CState` integer values) and the
> decompilation *technique*. don't name engine internals here; keep those in the gitignored
> `docs/` scratch space. see [`AGENT_CONTEXT.md`](../AGENT_CONTEXT.md) "engine-side knowledge
> is not in this repo".

## the layers (lowest → highest)

| Layer | Where | What lives here |
|---|---|---|
| 1. Bytecode dispatch | `src/main.rs` `connect_edit` (+ `src/asm.rs`) | the thin per-frame skeleton spliced into the engine's update loop: socket connect/auth and a single-byte wire dispatch. irreducible (HashLink has no plugin hook) but kept minimal. where a handler still has to be bytecode, it's emitted from Rust data tables via the `Asm` helper (registers + jump offsets resolved at build time, which turns runtime engine crashes into patch-time errors), not hand-placed. (don't name the specific engine function patched here, see the compliance note above.) |
| 2. Host vocabulary / routing | `src/interpreter.rs` | the Rust front end: it translates the small friendly command set into wire bytes, and forwards **every unrecognized line to the `e` (eval) handler as hscript**. deliberately flat, just a tiny `COMMANDS` table plus helpers (`expand_sequence`, `controls_mask`) that aren't meant to be invoked on their own. it just routes, no game logic. shared by both the patcher and the bridge so the host surface and the patched protocol can't drift. |
| 3. In-engine command vocabulary | `commands.hsx` | the human-readable, **non-helper** commands you actually run for tests and scripting: `match.getCharacters()`, `matchStatus()`, `log()`, the icon feeds. loaded once into the engine-scoped interpreter by the `e` hook. the Fraymakers script API (`CState`, `HitboxStats`, `Assist`, …) is already in scope; `commands.hsx`'s structural job is to expose the live match/character context a non-entity script otherwise lacks. **this is the most-used surface.** |
| 4. Shell orchestration | `tools/run.sh`, `tools/runseq.sh`, `tools/tests/*` | boot lifecycle, multi-command sequencing, batch sweeps, cleanup. |

### the two seams

**engine ↔ host (layers 1–2).** the wire protocol is deliberately minimal: **one byte selects
a handler** (spawn, eval-hscript, input, load, console, exit), and only the arg-bearing ones
drain a trailing line. the engine never parses words. `interpreter.rs` turns what you type
into that wire line. since anything it doesn't recognize gets forwarded verbatim to the eval
handler, the host vocabulary stays tiny: `interpreter.rs` adds only the few commands that
genuinely *need* an engine-side side-effect (input injection via `hold`/`seq`, the console
call) plus the handful that bootstrap a session (`spawn`, `load`, `exit`).

**host ↔ engine (layers 2–3).** the eval handler hands the hscript text to the engine's own
hscript interpreter, the same one that runs every character script, so logic expressed as
hscript gets resolved by the engine at runtime and is robust across engine updates.
`commands.hsx` is what that hscript can call beyond the stock script API.

### where new features should go

the default home for a new capability is **`commands.hsx` (layer 3)**, written as hscript.
the script API already exposes the whole engine surface, so you almost never need to touch
the engine-integration layer. only escalate when you're forced to:

1. **hscript in `commands.hsx`** -- the strategic direction and the right answer for nearly
   everything. readable, update-robust, the most-used surface.
2. **a new wire command in `interpreter.rs`** -- only when the feature needs an engine-side
   side-effect the interpreter can't reach from script (input injection, a side-effecting
   engine call). keep it flat: route to a thin handler, don't grow logic here.
3. **a handler at the engine-integration layer** -- the last resort, only when even the wire
   command can't be done from hscript. drive it from a data table, never hand-place
   engine-side code.

## version resilience -- surviving Fraymakers updates

Fraymakers ships as compiled HashLink bytecode, and every update is a full recompile that
renumbers the engine's internal indices. a patcher that hardcodes those integers breaks
silently, injecting into whatever now sits at the old index. four rules, in priority order:

1. **resolve by name, never by index.** use the read-only resolvers in
   [`src/main.rs`](../src/main.rs) (`require_fn`/`find_fn` by name + parent type,
   `require_type`, `require_field`, …). a name survives a recompile as long as the symbol
   still exists. raw index literals are a last resort, each commented with how to re-derive
   it.
2. **never silently fall back to a pinned index.** a name miss has to fail **loudly**
   (`require_*` returns `Err`, aborting the patch). the few non-critical fallbacks left are
   marked `critical: false` in the manifest; don't add new ones.
3. **prefer hscript over hand-emitted bytecode.** the engine bundles a full hscript
   interpreter, the same one that runs every character script. the eval command runs script
   *text* in-engine, so logic expressed as hscript gets resolved by the engine at runtime and
   is **immune to index drift**. the migration goal: the only brittle bytecode left is the
   minimal eval-bootstrap hook, and every handler becomes readable Haxe text.
4. **avoid mid-function opcode-index injection.** inserting ops at a fixed offset inside an
   engine function breaks if the engine changes a single opcode there. prefer the
   layout-robust `insert_ops_front`/`insert_ops_end` helpers, or move to hscript.

### the symbol manifest + `doctor`

every engine symbol the patcher depends on is declared in one place,
[`src/manifest.rs`](../src/manifest.rs) (`MANIFEST`), grouped by subsystem, each entry tagged
`critical` (a miss aborts the patch) or not. this is the single source of truth, so **any new
engine dependency gets added here.** a read-only **`doctor`** preflight resolves the whole
manifest against a given engine file and prints a pass/fail checklist. the same check runs at
the top of every real patch and **aborts before mutating anything** if a critical symbol is
missing, so an incompatible build fails precisely instead of producing corrupt output (the
GUI shows it in the boot modal, "Verifying engine N/N").

**version-bump loop:** run `doctor` against the new build → for each miss, find the symbol's
new name with the read-only inspection subcommands and update both `manifest.rs` and the
matching `require_*` call → repeat until clean → then re-run the in-engine spawn-test
(`doctor` proves symbols *resolve*; the spawn-test proves the patch *behaves*).

> **keep the engine specifics out of the prose docs.** this section documents the *process*
> (resolve by name, `doctor`, the manifest), which is fine. it doesn't enumerate the engine
> symbol map, the runtime `CState` values, or the technique for locating a moved symbol. to
> orient, read the code: the `MANIFEST` table in [`src/manifest.rs`](../src/manifest.rs) is
> the canonical, annotated list of the engine surface the patcher touches, and `connect_edit`
> consumes it. keep any deeper narrative in the gitignored `docs/` scratch space
> (`docs/ENGINE_INTERNALS.local.md`). see [`AGENT_CONTEXT.md`](../AGENT_CONTEXT.md)
> "engine-side knowledge is not in this repo".

## roadmap -- toward a live mod-development tool

the bigger vision: steer the engine from the command line, drive the exact move you're
editing on repeat against a dummy, tweak stats, and see the result right away. the canonical
use case is a **hitbox-stats live-tuning loop**: load a stage with the character + a dummy,
loop the attack you're editing, adjust a stat, read back knockback distance / angle / KO %,
and tune until it feels right.

shipped today: `spawn`, input injection (`hold` / `seq` frame-accurate timelines), and the
hscript-eval surface. driving any move (`toState(CState.…)`), reading state / position /
velocity / damage, and the `matchStatus` telemetry feed are all expressed as hscript against
the live match (see [`commands.hsx`](../commands.hsx)). plus crash diagnostics, recipe
scripting, and A/B regression checks.

the remaining deep items all converge on **one missing capability: in-engine measurement of
emergent behavior** (what a move actually does to an opponent), built on a dummy opponent plus
post-hit readback. the live, tracked todo list for all of this lives in
[`STATUS.md`](STATUS.md) "peptide / harness todos".

### the hscript / `.hl` direction

features with real control flow should be written in Haxe (hscript text today, a compiled
`.hl` module later) instead of hand-emitted opcodes, so the fragile hand-bytecode surface
stops growing. HashLink bytecode is *monolithic* with no runtime module-load facility, so a
compiled-`.hl` path most likely means **merging** a Haxe-compiled `.hl` into the engine
bytecode at patch time (remapping every function / type / string index across the two
modules). `hlbc` (already a dependency) gives read/write access to those tables, so it's
mechanically possible, but the index-remapping is a hefty cross-module linker and needs a
dedicated feasibility spike before any feature leans on it. until then, complex features stay
on the hscript-eval or Rust-generated-`Asm` path.

## IP boundary

Peptide contains **no** Fraymakers code, bytecode, strings, or assets. it reads the user's
*local* engine at runtime, writes a patched *copy*, and speaks a loopback TCP protocol. per
Team Fray's request, the tracked docs don't name specific engine internals
(classes/functions/fields, the symbol map) or explain the decompilation technique, though
Peptide's own architecture and patcher workflow are documented freely. see
[`NOTICE.md`](../NOTICE.md) and [`AGENT_CONTEXT.md`](../AGENT_CONTEXT.md) "engine-side
knowledge is not in this repo".
