# Harness milestone: move-dispatch + telemetry VERIFIED in a live match

Commit 2c76547d added two harness commands; both confirmed working against a live
`commandervideo` match (builtin; loads fine headless), clean (no error.log):

```
<< LAUNCHED public::commandervideo.commandervideo ...
<< T:STAND          # 't' telemetry: player-0 live state via Character.getStateName
<< M:JAB            # 'm' move dispatch: Character.toState(char, CState.JAB, null)
<< T:JAB  T:JAB  T:JAB   # rapid-sampled right after 'm' — the move DID transition
<< T:STAND          # jab finished (~0.36s), returned to idle
<< Q:MATCH_LIVE     # match still alive after driving the move — no crash/freeze
```

This is the core validation primitive working end-to-end:
**load → drive input via internal engine function (NOT key-press) → measure state → assert.**

## How it works (resolved by name, version-robust)
- `m`: walk `MatchController.currentMatch` (statics g3511, field `currentMatch`) →
  `Match.characters[0]` (field 35, ArrayObj) → UnsafeCast to `Character` (t783) →
  `toState@4497(char, CState.JAB, null)`. CState.JAB read at runtime via
  `GetGlobal($CState g3946).field(JAB=68)` — exactly how the engine's own callers do it.
- `t`: same walk → `getStateName@4491(char)` → returns the live CState name as a String.

## Verified telemetry detail
Sampling `t` every ~0.12s catches transient states; a single delayed sample misses
short moves (jab returns to STAND in ~0.36s). The runner must sample at frame cadence.

## What this unblocks / what's next
- Capability proven for ANY character the harness can LOAD. Builtins load; **our
  converted characters still don't load headless** (blocker #2: `[API loaded no]`,
  poolHash empty). Solving #2 (inject `addResource@18230` for the custom .fra — see
  ENGINE_RE_MAP_v2) is the gate to running this drive+measure across all 47.
- Build-out remaining: parameterize `m <move>` (jump table over CState fields, or
  pass the move name → resolve), numeric physics in `t` (x/y/vel/% via Body f18/19,
  Physics f24-27, Damage f20 — needs a number→string formatter), per-frame capture,
  and a runner that scripts a move sequence + asserts (no freeze = state keeps changing).

## Wedge note
Each headless boot risks an unkillable `hl _conn.dat` zombie (uninterruptible sleep).
Two test boots this session. Mitigation TODO: inject `Sys.exit(0)` after a scripted
run so the process self-terminates instead of spinning in the render loop.
