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

All four were regression-tested in-engine: mario (jab/strong_forward/special_up/
aerial_down) and sandbag (jab/tilt_forward/special_neutral) spawn, animate, and
dispatch with no crash after the changes.

## OPEN — high impact

- **`getGlobalVariable`/`setGlobalVariable` → wrong API.** Currently mapped to
  `get/updateAnimationStatsMetadata` (per-animation metadata, NOT persistent
  per-character state, and not in the FM template API). SSF2 globals are
  string-keyed persistent vars. **Correct fix:** route them into the existing
  `makeX` persistent-state system the converter already uses for ext-class
  instance vars — `var name = self.makeInt(0)/makeBool(false)/makeObject(null)`
  + `name.get()/.set()`. Affects mario's `canStartRise` (up-special gate),
  `standtime` (stand timer), `crouchdown`. *Not a crash in observed play* (those
  paths ran without faulting), so it's a semantic-fidelity gap. **Deferred
  deliberately**: the fix needs per-var type inference and has high blast radius
  (a wrong type → Script.hx won't load for many characters), so it must land with
  full-corpus re-verification, not rushed. Plan: detect the names post-translate,
  infer type (Bool for boolean-context / true-false sets, else Int), add to
  `ext_vars`/`ext_var_inits`, rewrite calls to `.get()/.set()`. Files:
  `mappings/commands.jsonc` (get/setGlobalVariable lines), `extractor.rs`
  (implicit-var detection block — add a sibling global-var pass), `haxe_gen.rs`
  (makeX emission already handles ext_vars).
- **Deeper `/* ? */` fix (stack-threading).** The interim text rule only handles
  the `.self.` receiver case; lost *conditions* (`if (/* ? */)` in mario's
  `continueCombo`) and `/* ? */.self.forceAttack` need the real fix: thread the
  predecessor block's residual operand stack into branch bodies
  (`decompiler.rs` BranchCmp arm seeds no stack; Branch else seeds empty). Broad
  change → needs full-corpus re-verification.
- **Per-segment hitbox fidelity.** The jab fix inherits jab1's stats for
  jab2/jab3; true SSF2 fidelity assigns each combo hit its own hitbox by frame
  window (the anim_splitter already computes split frame ranges; carry the source
  attack's per-frame hitbox activation into each split). Bigger change.
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

## How parity should be verified going forward

Build a `verify <move>` Peptide command (see `docs/PEPTIDE_FUTURE.md`) that drives
a move against a dummy and diffs observed behavior (damage dealt, knockback
distance/angle, active frames) against the SSF2 reference values (already
extracted into `HitboxStats.hx`). That turns parity from eyeballing into a
pass/fail suite. The hitbox-stat reference is in-hand; the missing piece is the
in-engine measurement (dummy opponent + post-hit readback).
