# Peptide future — modder-facing roadmap

Peptide started as a converter-validation harness. The bigger vision (from a
Fraymakers modder): a **live mod-development tool** where you steer the engine
from the command line, drive the exact move you're editing on repeat against a
dummy, tweak stats, and see the result immediately. This doc captures that
trajectory so each increment is built as a foundation for it, not a dead end.

Status legend: **[done]** shipped · **[next]** tractable soon · **[planned]**
designed, not built · **[research]** needs investigation.

## The canonical use case: hitbox-stats live-tuning loop

> Load a stage with the character + a dummy opponent, loop the attack you're
> editing so it keeps hitting the dummy, adjust a hitbox stat, see it reflected
> immediately, and read back: knockback distance, the angle, and the % at which
> it would KO. Sit and tune numbers until it feels right.

Decomposed into commands (see "Building blocks" below): `spawn` + `dummy` →
`loop <move>` → (edit stat) → hot-reload → `snapshot`/`hitresult` readback →
`kill_threshold`. The pieces are being built bottom-up.

## Building blocks

### Driving
- **`spawn <char>`** — start a match. **[done]**
- **`move <name>`** — drive any move via the engine state machine. **[done]**
- **`loop <move> [interval]`** — re-dispatch a move every N frames until
  `stop`. **[next]** Design for repeatability + interruption from the start: a
  `g_loop_move` global (selector + interval + countdown) checked each frame in
  the existing per-frame dispatch; `stop`/`loop none` clears it. Pure layer-2
  (Rust-generated), small.
- **`dummy [char] [pos]`** — spawn a second fighter as a hit target. **[planned]**
  The `s`-handler already builds a 1-player match; extend the players array to 2
  (CPU/idle behavior on P1). Needed for any hit/knockback measurement.

### Readback (capture what a modder needs)
- **`state`** — current CState name. **[done]**
- **`physics`** — player 0 position / velocity / damage. **[done]**
- **ANIM stream** — per-state-change `ANIM:<state>` telemetry. **[done]**
- **`snapshot`** — one atomic line with BOTH fighters' position, velocity,
  damage, current animation+frame, and the live hitbox set. **[planned]** More
  accurate than separate `state`/`physics` calls (single-frame consistency).
- **`hitbox` / `boxes`** — for the current frame, dump every active hit/hurt box:
  index, x/y/w/h, damage, angle, base KB, KB growth, active-frame window.
  **[next]** The modder's core debugging view. Reads the character's runtime
  hitbox component; layer-2 if expressible as field reads, else `.hl`.
- **`anim step [n]` / `anim info`** — frame-by-frame scrub: advance the animation
  one frame (pause playback), report frame index + what's active each frame.
  **[planned]** Needs engine animation-playback control (`pauseAnimationPlayback`
  is a known Character field).
- **`hitresult`** — after a hit lands: damage dealt, victim knockback
  distance + launch angle (sampled over the next K frames), hitstun frames.
  **[planned]** Requires `dummy`. This is the "is it a good angle / would it
  kill" data.
- **`kill_threshold <move>`** — binary-search the dummy's starting % to find the
  lowest % at which the move KOs from center stage. **[planned]** Composes
  `dummy` + `loop` + boundary detection; likely a layer-3 (client) driver loop
  over engine primitives, not new bytecode.

### Iteration
- **Stat hot-reload** — re-read `HitboxStats.hx` / `Stats.hx` into the running
  match so the next move uses new values without a full re-export. **[research]**
  Unknown whether the engine re-reads stat scripts mid-match or caches them at
  spawn. Two fallbacks: (a) re-run just the stats script via the console; (b)
  tighten the export→reload loop (`s <char>` already re-loads a fresh `.fra`
  in-session, so "edit → re-export → `spawn` again" is the baseline today).
- **`verify <move>`** — drive the move, capture behavior, diff against a per-move
  SSF2 reference spec (damage/angle/KB/active-frames/sound/vfx). **[planned]**
  This is the *functional-parity* harness: the reference comes from SSF2 source
  values (the converter already extracts AttackStats into `HitboxStats.hx`), so
  `verify` compares engine-observed behavior to the intended SSF2 numbers and
  reports divergence. Drives the converter-fix loop.

### Mod-dev quality-of-life (broader)
- **State-machine introspection** — current state + legal transitions + "why is
  it stuck" (e.g. can't cancel into X). **[planned]**
- **Crash diagnostics** — on engine fault, surface last N state transitions +
  current move + frame instead of a raw stack trace. **[planned]** The ANIM
  stream already gives a transition history the bridge could buffer and dump.
- **A/B comparison** — run identical input sequences against two characters (or
  two versions) and diff the readback. Regression testing for mod changes.
  **[planned]** Mostly a layer-3/shell driver over existing commands.
- **Save/restore engine state** — snapshot the match to disk, restore later to
  share "the exact moment my move broke". **[research]** Depends on engine
  serialization surface.
- **Recipe scripting** — a small stable DSL (JSON or a tiny line script) to drive
  Peptide programmatically: `spawn mario; move jab; snapshot; exit`. **[next]**
  `runseq.sh` is the proto-version; a JSON recipe runner in the bridge is the
  clean form. Shareable, reusable, no opcodes.

## Architectural commitment

Everything above is built per `docs/PEPTIDE_ARCHITECTURE.md`: client-side
(layer 3) wherever possible, Rust-generated `Asm` (layer 2) for simple
field-read/method-call readbacks, and **Haxe→`.hl`→bootstrap** for anything with
real control flow. The hand-written opcode surface does not grow. As features
accumulate, expect a one-time migration of the layer-1 skeleton to "load a `.hl`
module + forward commands", after which features land in readable Haxe.
