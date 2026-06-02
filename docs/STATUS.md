# Converter status — coverage and parity

A snapshot of how complete the SSF2 → Fraymakers conversion is across the roster.
For *how* to verify any of this yourself, see [`PEPTIDE_GUIDE.md`](PEPTIDE_GUIDE.md).

## Coverage: 45 / 45 characters

The roster is 46 `.ssf` files minus `misc.ssf` (shared costume/palette data) →
**45 characters**. All 45:

- **convert** — the converter produces output (exit 0); no convert failures.
- **drive in-engine** — spawn in Fraymakers via Peptide, reach `STAND`, and
  dispatch moves with no crash (P0: `LAUNCHED` + `ANIM:STAND` + `M:OK`).
- **have populated entities** — real movesets and hitboxes, verified by
  `tools/tests/parity_check.py` (entity animation counts + HIT_BOX active-frame
  coverage), not just the shallow spawn signal. Hitbox frame-coverage is 52–90%+
  per character; the remainder are projectile specials / throws with no melee box.

Deep / hand-verified (drive moves + physics + anim): `sandbag` (reference),
`mario` (deepest — full 18-move sweep), `kirby`, `bowser`, `fox`. The rest are
batch-verified (spawn + jab + special_neutral, no crash).

## Hitbox-stat parity: 45 / 45

`tools/tests/parity_check.py` reports every hitbox's `damage` / `angle` /
`baseKnockback` / `knockbackGrowth` / `hitstop` / `selfHitstop` / `hitstun`
matching the SSF2 source (dumped via `DUMP_PARITY=1`). This is the hitbox-*stats*
dimension of functional equivalence.

The **bar for a "done" character is functional equivalence with SSF2**, not just
"spawns + animates + doesn't crash". Hitbox-stat parity is one dimension of that;
the open dimensions below are the rest.

## Open parity dimensions

- **Frame data (active-frame range).** Startup/active/recovery lives in the
  `.entity` collision-box keyframes + animation lengths, not `HitboxStats.hx`.
  Per-char coverage is checked (it caught the empty shells); exact active-frame
  comparison vs SSF2 (×2 for 30→60 fps) is a separate, fiddly harness pass.
- **Special-angle sentinels.** SSF2 sentinel angles (`-1`/`-2`/`-3`…) are now
  faithfully preserved but not yet mapped to FM's special-angle codes — needs the
  SSF2-sentinel → FM-angle table.
- **Per-segment hitbox fidelity.** Split sub-anims (`jab2`/`jab3`) currently
  inherit the base attack's hitbox stats. This is *sound* (FM gates a hitbox by the
  animation's collision-box layers, so an inactive inherited box is inert), but the
  damage/KB *values* approximate the real per-hit finisher. True per-hit values
  need extracting hitbox activation frames from the SSF2 sprite timeline.
- **Physics-stat tuning.** Movement stats (gravity/fall/walk/jump/weight) are
  mapped and scaled; `friction`, `shortHopSpeed`, and ECB head/hip/foot positions
  are hand-tuned constants, so ground deceleration / short-hop / hurtbox sizing are
  approximate. Getting the scale factors right needs in-engine SSF2-vs-converted
  comparison (the dummy / measurement path), not a static fix.
- **No-FM-equivalent calls.** CPU-AI branches, z-order (`getMC`/`swapChildren`),
  `forceAttack`, and the item system have no FM mapping and are commented out as
  `[SSF2-only]` — no impact for human play of non-item characters.

## Measuring progress

`tools/tests/translation_completeness.sh` counts untranslated markers per character
— `/* ? */` (decompiler couldn't recover an expr/condition/receiver),
`[SSF2-only:` (no FM equivalent), and `TODO` (value punted to a default). Lower is
better; it's the safe before/after gate for decompiler/mapping changes (a fix must
*reduce* markers without adding new ones, and the in-engine spawn sweep must still
pass). Ten characters are fully `/* ? */`-clean; the decompiler-quality outliers
worth a focused future pass are `kirby`, `tails`, `rayman`, `pacman`,
`goku`/`lucario`, `dedede`, `sonic`, `yoshi`.

## How parity should be verified going forward

The endgame is a `verify <move>` Peptide command (see
[`PEPTIDE_DESIGN.md`](PEPTIDE_DESIGN.md) roadmap) that drives a move against a dummy
and diffs observed behavior against the SSF2 reference values. The hitbox-stat
reference is in-hand; the missing piece is in-engine measurement of emergent
behavior (dummy opponent + post-hit readback) — the next focused project.
