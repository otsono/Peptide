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

## Headline: 40 / 45 genuinely functional; 5 have broken sprite extraction

**Correction (don't over-claim):** all 45 convert and *spawn* without crashing, but
the spawn test is SHALLOW — `ANIM:JAB` reports the engine STATE name, not a real
animation playing. The frame-data check (`tools/parity_check.py`, HIT_BOX active
frames in the entity) revealed that **5 characters have near-empty entities** and
do NOT actually animate or hit:

| Broken char | entity anims | hitbox layers | active frames |
|---|---|---|---|
| `fox` | 11 | 0 | 0 |
| `bomberman` | 11 | 2 | 2 |
| `donkeykong` | 14 | 2 | 2 |
| `pit` | 16 | 2 | 2 |
| `luffy` | 17 | 2 | 2 |

(A healthy character has ~128-164 animations and 24-76 hitbox layers — e.g. mario
159/39, marth 139/76.) Root cause: per-animation sprite/box resolution fails for
these 5 (fox: box data for 9/86 animations vs mario's 88/85), so their real moves
extract empty and `entity_gen` drops them below the "UNUSED" separator, leaving
only the template `item_*` placeholders. **Open** — a deep `image_extractor` /
`sprite_parser` issue (see `docs/PARITY.md`).

**40 / 45 characters are genuinely functional** (real moveset, hitboxes, drive
in-engine). `misc.ssf` is shared data, not a character. Hitbox-STAT parity is
45/45 for the moves that DO exist (`docs/PARITY.md`); the 5 broken chars + the
frame-data dimension are the next priority.

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
