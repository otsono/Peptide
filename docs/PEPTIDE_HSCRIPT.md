# Peptide on hscript — the eval-hook architecture

This is the pivot away from per-command hand-emitted HashLink bytecode toward a
single generic **eval hook**: handler logic becomes readable Haxe script text,
parsed and executed in-engine by the bytecode interpreter the engine already ships.

## Why

Every Peptide command used to be hand-emitted HL opcodes. That surface is fragile:
register types must be exactly right (a wrong-typed `SetArray`/`Field` src is an
instant SIGSEGV with no compiler to catch it), jump offsets are computed by hand,
and adding a feature (e.g. the dummy opponent) means more opcodes and more risk.
The goal all along has been a human-facing tool where logic is NOT written in opcode.

## The key finding

The Fraymakers engine bundles the **full vanilla `hscript` library** — the same
`hscript.Parser` + `hscript.Interp` that runs every character/assist script
(`pxf.api.runners.hscript.HscriptRunner` wraps it; converted `.hx` scripts run
through it). So the interpreter is native, battle-tested, and already wired to call
engine APIs.

Resolved entry points (this engine build):
- `hscript.Parser` type=502, ctor findex=2284, `parseString(parser, code, origin)→Expr` findex=2239
- `hscript.Interp` type=506, ctor findex=2235, `execute(interp, expr)→Dynamic` findex=2215, `setVar(interp, name, value)` findex=2210
- `hscript.Expr` type=396 (unnamed enum `t396`; index from the parseString/execute sigs)
- `Std.string(Dynamic)→String` findex=5791 (marshals the result for the wire)

## The hook (bytecode, write-once)

The `e` command is the only new bytecode. Per invocation it:
1. drains the rest of the socket line into `g_buf` → `getString` → the script string
   (mirrors the proven `s`-handler drain),
2. `New hscript.Parser` + ctor → `parseString(script, null)` → `Expr`,
3. `New hscript.Interp` + ctor,
4. binds `p0` = `MatchController.currentMatch.characters[0]` (as Dynamic, null if no
   match) via `setVar` so scripts reach the live character,
5. `execute(expr)` → Dynamic,
6. `Std.string` → write `E:<result>\n` back to the socket.

After this hook, NEW handlers are just script strings sent from the client — no
patcher rebuild, no opcodes. The socket/connect/ready bootstrap and the `s`
match-launch handler remain bytecode (the irreducible "hooks").

## Status — UPDATED (top-scope interp working)

**PROVEN in-engine (branch `hscript-eval-hook`):** a PERSISTENT top-scope interpreter,
created once and loaded with the engine's global API via `applyInterpreterGlobals`
(fn 18218), programs run through `ApiScript.interpretScript` (fn 2202, which resets
depth/declared, runs exprReturn, and traps errors):
- `eval 1 + 2` -> `E:3`        (pure)
- `eval CState.JAB` -> `E:63`  (engine API loaded!)
- `eval CState.STAND` -> `E:2`
- `eval HitboxStats` -> `E:fraymakers.entity.stats.$FraymakersHitboxStats`
- ping/eval interleave cleanly; no crash.

The earlier bug (user-diagnosed): the hook used a BARE `new Interp()` with no init, so
it could only do arithmetic. Fixed by mirroring how `Main::init` readies scripts.

**Spawn regression — FOUND + FIXED (it was the eval hook, not the env).** Inserting the
`e` command check into the MIDDLE of the dispatch chain (before `x`) broke the `s` handler
at runtime, even though every jump offset recomputed correctly. Proven by back-to-back
(branch mute / main launches, same moment) and bisect (no-jump insertions launch; any
jump inserted mid-chain goes mute). FIX: append the `e` check at the END of the chain so
`x->p->c->s->...->g` stays byte-identical to baseline (`g` no-match -> `e` check, `e`
no-match -> L_ORIG). VERIFIED end-to-end: spawn->LAUNCHED, move/physics/anim/play/query
all respond, eval CState.JAB->E:63 / CState.STAND->E:2, no crash. Likely an HL JIT
control-flow/basic-block sensitivity to inserting a branch mid-chain.

### Old status (superseded)
## Status

**PROVEN in-engine (commits on branch `hscript-eval-hook`):**
- `eval 1 + 2` → `E:3` (hardcoded spike)
- `eval 3 * 7` → `E:21`, `eval 100 - 1` → `E:99`, `eval "hi" + "!"` → `E:hi!`
  (socket-driven arbitrary scripts)

The pure-script pipeline (parse → execute → marshal → writeback) works end to end.

**DRAFTED, runtime-unverified:** the `p0` engine-access binding (builds + patches
clean, 980 ops injected, no panic). It could not be runtime-tested because the local
test environment wedged (see below).

## Environment recovery REQUIRED before further in-engine tests

Heavy spawn/kill cycling during this session left ~10 **uninterruptible-sleep (`UE`)**
`hl _conn.dat` processes stuck in the kernel (one ~6h old). `kill -9` queues but
cannot complete until their GPU/IO syscall returns — effectively a **reboot** is
needed. While wedged, even the pristine baseline converter crashes on spawn with
`Exception: Null access .namespace` in `pxf.io.ResourceManager.getContentIdentifierString`
(ResourceManager.hx:301) — i.e. this is an environment symptom, NOT a converter or
Peptide regression (verified: pure `f2d2ee02` baseline crashes identically).

### Exact next test (after reboot)
```
cd tools/peptide
FRAY_CHAR=sandbag ./runseq.sh 3 "spawn sandbag" "eval p0.body.x" "eval p0.body.y"
```
Expect `E:<x>` / `E:<y>` matching the character's position (cross-check with `physics`).
If `p0.body.x` returns the position, engine-access works and the handler migration
proceeds: replace `physics`/`anim`/`state`/`move` with one-line scripts, then add the
dummy/hitresult/parity logic as scripts (no new bytecode).

## Migration plan (once p0 access is verified)
- `physics` → `eval p0.body.x + "," + p0.body.y + "," + p0.physics.currentVelocityX + ...`
- `state`   → `eval p0.getStateName()` (or the state field)
- `move`    → bind `CS` = CState statics, `eval p0.toState(CS.JAB)` etc.
- `dummy`   → script that builds/positions a 2nd fighter (needs match-launch class
  registration; investigate whether the interp's `resolve` reaches arbitrary classes)
- All handler scripts live client-side (peptide-bridge) as readable Haxe strings.
