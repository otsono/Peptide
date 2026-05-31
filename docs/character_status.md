# Character status

Per-character status across the SSF2 roster (46 `.ssf` files; `misc.ssf` is the
shared costume/palette data file, not a character → **45 characters**).

Two axes:
- **Convert** — does `ssf2_converter` produce output (exit 0)?
- **In-engine** — does it spawn in Fraymakers via Peptide, reach `STAND`, and
  dispatch moves with no crash? (PASS = `LAUNCHED` + `ANIM:STAND` + `M:OK`, no
  rosetta crash.)

> **In-engine PASS = "boots + drives + animates, no crash" (P0).** It does NOT yet
> assert full SSF2 functional parity (exact per-move damage/knockback/angles/active
> frames). Parity work (5 fixes landed; see `docs/PARITY.md`) is tracked
> separately, with mario/sandbag as the deep test beds.

## Headline: 43 / 45 characters drive in-engine, 0 crashes

Every character that converts also spawns, reaches STAND, and dispatches moves
without crashing. The only failures are 2 convert-time OOMs.

## Convert failures (2)

- **`chibirobo`** — SIGKILL/137 (OOM) entering the extractor's animation-build
  phase (after `animations.jsonc` loads, before the "Total: attacks" summary);
  runaway heap allocation, not stack recursion. Open (needs an instrumented run
  to localize — see `TESTING.md` §5).
- **`dedede`** — same OOM signature. Both are large characters (87–89 frame
  methods / sub-MCs).

## In-engine PASS (43)

Deep / hand-verified (drive moves + physics + anim):
`sandbag` (reference), `mario` (full 18-move sweep + physics + anim — deepest),
`kirby`, `bowser`, `fox`.

Batch-verified (spawn + jab + special_neutral [+ more], no crash):
`marth`, `falco`, `captainfalcon`, `donkeykong`, `bomberman`, `blackmage`,
`bandanadee`, `gameandwatch`, `ganondorf`, `goku`, `isaac`, `jigglypuff`,
`krystal`, `link`, `lloyd`, `lucario`, `luffy`, `luigi`, `megaman`, `metaknight`,
`naruto`, `ness`, `pacman`, `peach`, `pichu`, `pikachu`, `pit`, `rayman`,
`samus`, `simon`, `sonic`, `sora`, `tails`, `waluigi`, `wario`, `yoshi`,
`zamus`, `zelda`.

(`zelda` initially logged a false FAIL — a transient `Address already in use`
port collision during the batch, not a character issue; it PASSES on retest:
LAUNCHED + JAB + SPECIAL_NEUTRAL, no crash.)

## How to reproduce / extend

`docs/MODDER_GUIDE.md` (the iteration loop) and `tools/peptide/batch_spawn_test.sh`
(unattended sweep: regen → export → spawn-drive → PASS/FAIL). Always re-export
before trusting a result (the stale-`.fra` trap). Pass `FRAY_PORT=<n>` /
`BATCH_RESULTS=<file>` to avoid port collisions across concurrent runs.

All 43 were (re)generated with the current converter — including the 5 SSF2-parity
fixes — and the regression batch (kirby/luigi/link) confirmed those fixes don't
break spawning.
