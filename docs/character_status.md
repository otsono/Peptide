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

## Headline: 45 / 45 genuinely functional + 45/45 hitbox-stat parity

All 45 characters convert, spawn, and have **populated entities with real movesets
and hitboxes** (verified by `tools/parity_check.py`: entity animation counts +
HIT_BOX active-frame coverage, NOT just the shallow `ANIM:STATE` spawn signal).
`misc.ssf` is shared data, not a character.

**5 characters were empty shells and are now FIXED.** The frame-data check first
exposed that `fox`/`bomberman`/`donkeykong`/`pit`/`luffy` had near-empty entities
(~11-17 animations, ~0-2 hitboxes) — they spawned (state dispatch) but didn't
animate or hit. Root cause: those characters name move sprites with a redundant
char prefix (`fox_fla.fox_airN`; donkeykong uses the short code `dkbair`) and
abbreviated labels (`airN`/`smashD`/`specialN`), which the label resolver missed →
moves extracted empty → dropped. Fixed by a `{char}_`/short-code prefix strip
(`sprite_parser.rs`) + abbreviated-label entries (`animations.jsonc`). Recovery:

| char | before (anims/hitbox) | after |
|---|---|---|
| fox | 11 / 0 | 113 / 36 |
| bomberman | 11 / 2 | 129 / 32 |
| donkeykong | 14 / 2 | 87 / 25 |
| pit | 16 / 2 | 138 / 46 |
| luffy | 17 / 2 | 124 / 34 |

fox re-verified in-engine: JAB / AERIAL_NEUTRAL / SPECIAL_NEUTRAL now play real
animations (was state-dispatch only), no crash.

**Hitbox-STAT parity: 45/45** — every hitbox's damage/angle/knockback/hit-freeze
matches the SSF2 source (`docs/PARITY.md`). Hitbox frame-coverage is 52-90%+ per
character (the remainder are projectile specials / throws that legitimately have no
melee hitbox).

## Convert failures: none

`chibirobo` and `dedede` previously SIGKILL'd (240 GB alloc) — a decompiler bug
on their large methods where a mis-parsed CFG range fed a garbage near-`u32::MAX`
argc/count to the call/construct/newarray pop-loops. **Fixed** by clamping every
pop-loop to the operand-stack depth (`src/decompiler.rs`); both now convert and
drive in-engine (chibirobo, dedede PASS).

## In-engine PASS (45)

Deep / hand-verified (drive moves + physics + anim):
`sandbag` (reference), `mario` (full 18-move sweep + physics + anim — deepest),
`kirby`, `bowser`, `fox`.

Batch-verified (spawn + jab + special_neutral [+ more], no crash):
`chibirobo`, `dedede`, `marth`, `falco`, `captainfalcon`, `donkeykong`, `bomberman`, `blackmage`,
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
