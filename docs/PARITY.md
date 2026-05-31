# SSF2 functional-parity status (sandbag + mario)

The bar for a "done" converted character is **functional equivalence with SSF2**,
not just "spawns + animates + doesn't crash" (that's P0, already true for 25
characters). Every move must behave like SSF2 top to bottom: damage, knockback
angle/growth, hitbox active frames, state transitions, sounds, VFX, momentum.

This doc tracks the parity gaps found by audit of mario + sandbag and their fix
status. Methodology: a 4-agent read-only audit cross-referenced the generated
`.hx` against the converter source, the SSF2-derived stat values, and the
official FM character template. Findings below are grounded in file:line
citations (see commit history for the audit run).

## FIXED this session

- **Split sub-animations were 0-damage.** `jab2`/`jab3` (and `strong_*_in/charge`)
  emitted `damage:0 /*TODO*/` because SSF2 stores a combo as ONE attack keyed by
  the base name; the FM split produced extra names with no stats → non-functional
  moves. Now they **inherit the base attack's hitboxes** (with an explicit
  comment). `base_attack_name()` in `haxe_gen.rs`. *(commit: split sub-anim
  inherit)*
- **Inverted/empty-then branches.** The decompiler surfaced AVM2's skip-body
  branch as `if (x != 0) {} else { body }` — reads inverted, a correctness
  hazard. `make_if()` normalizes to `if (x == 0) { body }` (faithful; render
  folds the negation). `decompiler.rs`. *(commit: normalize skip-body branch)*
- **Valid local-function calls wrongly commented.** `comment_out_unknown_calls`
  runs per-method, so a call to a function *defined locally* (mario's
  `jumpToContinue`, also a COLLIDE_FLOOR listener) was flagged `[SSF2-only]` and
  dropped → landing/continue transition lost. `uncomment_local_fn_calls()`
  whole-file post-pass restores them. *(commit: restore local-fn calls)*
- **Dangling `/* ? */.self.` receiver (interim).** A decompiler cross-block stack
  underflow emitted an invalid-Haxe receiver; a `commands.jsonc` rule collapses
  `/* ? */.self.` → `self.`. *(same commit)*

- **`getGlobalVariable`/`setGlobalVariable` → makeX persistent state.** Was
  mis-mapped to `get/updateAnimationStatsMetadata` (per-animation metadata, NOT
  persistent state, and not a real FM API — absent from the template). Now the
  extractor collects each string key as a persistent ext var (`var name =
  self.makeInt(0)/makeBool(false)` — Int if used numerically, else Bool; hscript
  is dynamic so the kind only sets the pre-first-set default) and `commands.jsonc`
  regex rules rewrite the calls to `name.get()/name.set(v)`. **Verified in-engine:**
  mario `canStartRise`→makeBool, `standtime`→makeInt; special_up/down + idle run,
  no crash. *(commit: get/setGlobalVariable → makeX)*

All five fixes were regression-tested in-engine: mario (jab/strong_forward/
special_up/special_down/aerial_down + idle) and sandbag (jab/tilt_forward/
special_neutral) spawn, animate, and dispatch with no crash; a 3-character batch
(kirby/luigi/link) re-verified no broad breakage.

## OPEN — high impact

- **Deeper `/* ? */` fix (stack-threading) — PARTIALLY DONE.** The **BranchCmp
  arm** now seeds both branch bodies with the block's residual operand stack
  (`decompiler.rs`), recovering receivers/exprs that previously underflowed to
  `/* ? */`. Verified: `/* ? */` markers DROP (fox 1→0, bowser 4→2), none added,
  fox/bowser/mario spawn+drive clean. **Still open:** the **Branch arm** (boolean
  `iftrue`/`iffalse`) — mario's `continueCombo` `if (/* ? */)` is a Branch case,
  not BranchCmp, so it's untouched. The Branch-else seeding interacts with the
  ternary-detection heuristic (the empty-then/else-leftover length check), so it's
  the riskier half; gate any attempt on `tools/translation_completeness.sh` (markers
  must drop, none added) + the in-engine spawn sweep.
- **Per-segment hitbox fidelity.** The jab fix inherits jab1's stats for
  jab2/jab3. Investigation: the extracted `Hitbox` struct carries NO activation
  frame (just damage/angle/KB/hitstop/hitstun), and SSF2's `attackBox`/`attackBox2`
  are simultaneous boxes of the one combo attack — so true per-hit values would
  need NEW extraction of hitbox activation frames from the SSF2 sprite timeline,
  correlated to the anim_splitter's frame ranges. Bigger change. **Note the
  interim is sound, not merely non-zero:** FM gates a hitbox by the animation's
  COLLISION_BOX *layers*, so any inherited stat entry whose box isn't active on
  jab2/jab3's frames is simply inert — the only approximation is the damage/KB
  *values* (jab3's real SSF2 finisher is typically a bit stronger than jab1).
- **`forceAttack`** (mario `grounded`) has no FM mapping; the auto-special-on-land
  branch is dead.

## OPEN — medium / low

- `CharacterStats` `friction`, `shortHopSpeed`, ECB head/hip/foot positions are
  `/*TODO*/` placeholders, not SSF2-derived → ground deceleration / short-hop /
  hurtbox sizing approximate. (medium)
- CPU-AI branches (`isCPU`/`getCPULevel`/`getCPUAction`) and z-order
  (`getMC`/`swapChildren`) are `[SSF2-only]` — no gameplay impact for human play.
  (low)
- Item system (`getItem`/`tossItem`/`pickupItem`/`activateItem`) has no FM
  equivalent — item moves are no-ops. (low for non-item characters)

## Measuring parity progress (the safe metric)

`tools/translation_completeness.sh` counts untranslated markers per character
across generated output — `/* ? */` (decompiler couldn't recover an
expr/condition/receiver), `[SSF2-only:` (no FM equivalent, commented out), and
`/*TODO*/`/`TODO` (value punted to a default). Lower is better. It's the SAFE
before/after gate for decompiler/mapping changes: a fix must REDUCE markers
without adding new ones, and the in-engine spawn sweep must still pass — this lets
broad changes (e.g. the deferred `/* ? */` stack-threading) be evaluated without
hand-reading every script. Baseline spot-check (deep-validated chars are clean;
`kirby`'s 26 `/* ? */` flags it as a decompiler-quality outlier / future target):

```
mario   /*?*/ 1   SSF2 2   TODO 18      (clean after the 5 parity fixes)
sandbag /*?*/ 1   SSF2 3   TODO 12
kirby   /*?*/ 26  SSF2 45  TODO 63      ← decompiler struggles; future parity pass
```

(`TODO` counts include the known `CharacterStats` physics-tuning placeholders.)

**Full-corpus baseline (final converter, all 8 fixes; run the tool for current
numbers):** 45 characters, totals `/* ? */`≈252, `[SSF2-only]`≈420, `TODO`≈2125.
Ten characters are fully `/* ? */`-clean (captainfalcon, donkeykong, fox,
ganondorf, lloyd, metaknight, samus, simon, zamus, zelda). The decompiler-quality
outliers worth a future focused pass (most lost exprs/conditions): `kirby` 26,
`tails` 23, `rayman` 18, `pacman` 17, `goku`/`lucario` 15, `dedede` 14, `sonic`
13, `yoshi` 11. These are where the still-open Branch-arm stack-threading + per-
character decompiler edge cases would pay off most.

## How parity should be verified going forward

Build a `verify <move>` Peptide command (see `docs/PEPTIDE_FUTURE.md`) that drives
a move against a dummy and diffs observed behavior (damage dealt, knockback
distance/angle, active frames) against the SSF2 reference values (already
extracted into `HitboxStats.hx`). That turns parity from eyeballing into a
pass/fail suite. The hitbox-stat reference is in-hand; the missing piece is the
in-engine measurement (dummy opponent + post-hit readback).
