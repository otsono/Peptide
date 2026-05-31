# Character status

Per-character status across the 47-character SSF2 roster. Two axes:
- **Convert** — does `ssf2_converter` produce output (exit 0)?
- **In-engine** — does it spawn in Fraymakers via Peptide, reach `STAND`, and
  dispatch moves without crashing? (PASS = LAUNCHED + `ANIM:STAND` + `M:OK`, no
  rosetta crash.)

> **In-engine PASS means "boots + drives + animates, no crash" (P0).** It does
> NOT yet mean full SSF2 functional parity (correct damage/knockback/angles/
> active-frames/sound/vfx per move). Parity verification (the `verify` harness +
> converter stat fixes) is tracked separately — see TESTING.md and the parity
> work. mario/sandbag are the parity test beds.

Last updated mid-session; the corpus in-engine sweep was still running when this
was written (see `docs/overnight_session_log.md` for the live timeline).

## Convert: 44 / 47 clean

**Convert FAIL (2):**
- `chibirobo` — SIGKILL/137 (OOM) entering image/animation extraction; runaway
  heap allocation, open (TESTING.md §5).
- `dedede` — same OOM signature.

`misc.ssf` is the shared costume/palette data file, **not a character** (n/a).

## In-engine verified PASS (this session)

Deep / spot-checked by hand (drive moves + physics):
- `sandbag` — reference; spot-checked across categories.
- `mario` — **full 18-move sweep** (jab, dash_attack, tilts, strongs as
  `_IN`→`_ATTACK`, specials, all aerials, grab) + physics. Deepest validation.
- `kirby`, `bowser`, `fox` — spawn + jab/special_neutral/grab + physics.
- `marth`, `falco`, `captainfalcon`, `donkeykong`, `bomberman`, `blackmage` —
  batch spawn + jab + special_neutral.

Batch sweep PASS (spawn + jab + special_neutral, no crash):
- `bandanadee`, `gameandwatch`, `ganondorf`, `goku`, `isaac`, `jigglypuff`,
  `krystal`, `link`, `lloyd`, `lucario`, `luffy`, `luigi` … (sweep ongoing;
  remaining: megaman, metaknight, naruto, ness, pacman, peach, pichu, pikachu,
  pit, rayman, samus, simon, sonic, sora, tails, waluigi, wario, yoshi, zamus,
  zelda).

**No in-engine crash has been observed in any character that converts.** Every
character driven so far reaches STAND and dispatches moves cleanly.

## How to reproduce / extend

See `docs/MODDER_GUIDE.md` (iteration loop) and `tools/peptide/batch_spawn_test.sh`
(unattended sweep). Re-export before trusting a result (stale-`.fra` trap).
