# Overnight validation — sandbag & mario

This is the validation record for the two reference characters, and the template
for validating the rest of the corpus. It records WHAT was checked, HOW, and the
commits/tooling that prove it. All runtime checks were performed live against the
user's local Fraymakers engine via the Peptide harness (`tools/peptide`).

## Morning summary (read this first)

**Headline results:**
- **sandbag + mario fully drivable in-engine** — every move dispatches, animates,
  and recovers; physics + animation-frame readbacks work; no crashes. mario got
  the deepest sweep (all 18 moves).
- **45 / 45 SSF2 characters convert AND drive in-engine, 0 crashes** (full corpus
  at the P0 bar; `misc.ssf` is shared data, not a character). See
  `docs/character_status.md`.
- **6 converter bugs fixed** (all regression-tested): 5 SSF2 functional-parity
  fixes on top of P0 + the decompiler OOM that had blocked chibirobo/dedede. See
  `docs/PARITY.md`.

**Peptide is now a human-facing tool** (full-word commands, was single letters):
`spawn`, `move <name>`, `state`, `physics`, `anim`, `loop <move>`, `query`,
`load`, `keys`, `console`, `ping`, `exit`, `help` — with single-letter aliases and
plain-English reply glosses. The drive→observe→iterate loop works end-to-end (and
was used to find + verify the parity fixes). Modder guide: `docs/MODDER_GUIDE.md`;
layering + the greenlit `.hl` direction: `docs/PEPTIDE_ARCHITECTURE.md`; feature
roadmap (hitbox live-tuning, `verify`, dummy opponent): `docs/PEPTIDE_FUTURE.md`.

**What's NOT done (the real "100%" parity bar):** P0 (boots/animates/no-crash) is
met corpus-wide, but exact SSF2 behavioral parity (per-move damage/knockback/
angles/active-frames, item system, CPU AI, physics-stat tuning) is only partially
in. Five high-confidence parity fixes landed; the remaining items — deeper
`/* ? */` stack-threading, per-segment hitbox values, `forceAttack`, friction
tuning — are catalogued with plans in `docs/PARITY.md`. They were deferred as
either broad/regression-risky (not worth risking the pristine 45/45 state at
session end) or empirical-tuning problems. The `verify <move>` harness (diff
in-engine behavior vs the SSF2 reference) is the recommended next build to turn
parity from eyeballing into a pass/fail suite.

Full chronological timeline: `docs/overnight_session_log.md`. Engine RE map +
command reference: `TESTING.md`.

---

## How to reproduce

```
# 1. (re)generate the character from the current converter
./target/release/ssf2_converter ../ssf2-ssfs/<id>.ssf          # -> characters/<id>/

# 2. publish a FRESH .fra (NEVER trust an old one — see "stale .fra" below)
node tools/fraytools-harness/export-in-fraytools.js \
  --project "$PWD/characters/<id>/<id>.fraytools"               # -> custom/<id>/<id>.fra

# 3. drive it in the engine with friendly commands
cd tools/peptide
FRAY_CHAR=<id> ./runseq.sh 3 "spawn <id> thespire commandervideoassist" \
  "move jab" "move tilt_forward" "move strong_forward" "move special_neutral" \
  "move aerial_neutral" "move grab" "state"
```

Read the `<<` lines: every `move X` should produce an `ANIM:X` transition, an
`M:OK` ack, and a recovery `ANIM:STAND`. A crash shows as the engine process
dying early (run ends well under its cap) and a `rosetta error: … synchronous
exception` in `FRAY_ENGINE_LOG`.

## The stale-`.fra` trap (root cause of the original mario "freeze")

Mario was crashing the engine immediately after its INTRO animation. The cause
was **not** a converter bug in the current code — it was a 3-day-old published
`.fra` (May 28) sitting in `custom/mario/`, built before recent converter fixes.
Re-generating from the current converter and re-exporting produced a `.fra` that
boots, idles, and drives its whole moveset with no crash. **Always re-export
before trusting a runtime result.** `characters/` and the `.fra` are git-ignored,
so the only way to know an export is current is to rebuild it.

## Validation matrix (the 6 criteria, sandbag as reference)

| # | Criterion | sandbag | mario |
|---|---|---|---|
| 1 | Conversion clean (exit 0, log triaged) | MET | MET |
| 2 | FrayTools layout match (`compare_boxes`) | MET (prior) | not re-run this session |
| 3 | Engine boots + spawns, no crash | **MET** | **MET** (after fresh export) |
| 4 | Every move runs via internal control path | **MET** | **MET** |
| 5 | Animations play (per-state ANIM stream) | **MET** | **MET** |
| 6 | Physics within tolerance of CharacterStats | pending physics readback | pending physics readback |

## Move-by-move (driven via `move <name>` → engine `Character.toState`)

Each entry below was observed live as `ANIM:<STATE>` (+ `M:OK`) followed by a
clean return to `ANIM:STAND`. The strong attacks show the full charge chain
(`_IN` → `_ATTACK`), confirming the state machine drives real animation
sequences, not just a single pose.

**mario — full moveset, all clean (no crash):**
- jab → JAB
- dash_attack → DASH_ATTACK
- tilt_forward / tilt_up / tilt_down → TILT_FORWARD / TILT_UP / TILT_DOWN (tilt_down via CROUCH_OUT)
- strong_forward / strong_up / strong_down → *_IN → *_ATTACK (full charge→release chain)
- special_neutral / special_side / special_up / special_down → SPECIAL_* (special_up went airborne → LAND)
- aerial_neutral / aerial_forward / aerial_back / aerial_up / aerial_down → AERIAL_*
- grab → GRAB

**sandbag — spot-checked across categories, all clean:**
- jab → JAB, tilt_forward → TILT_FORWARD, strong_forward → STRONG_FORWARD_IN→ATTACK,
  special_neutral → SPECIAL_NEUTRAL, aerial_neutral → AERIAL_NEUTRAL, grab → GRAB.

## Tooling proven / extended this session

- **Friendly command vocabulary** (`tools/peptide/src/commands.rs`): `spawn`,
  `move <name>`, `state`, `query`, `load`, `keys`, `console`, `ping`, `exit`,
  `help`. Single-letter wire bytes remain as aliases. Reply lines get a plain
  gloss. 5 unit tests.
- **Move-by-name dispatch**: the bridge maps `move <name>` → `m <selector>`; the
  engine selects the CState via a jump table generated from `commands::MOVES`
  (fields resolved by name at patch time). Strongs map to the `_IN` entry.
- **ANIM stream**: `Character.toState` is hooked to emit `ANIM:<state>` per
  transition; the bridge dedups so only changes print — this is the per-state
  animation capture (criterion #5).

## Open

- Criterion #6 (physics): needs a numeric readback command (position / velocity /
  damage / frame). In progress.
- `compare_boxes` re-run for mario (layout) was not repeated this session.
