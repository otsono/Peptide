# Sandbag freeze ROOT CAUSE — infinite loop in generated Script.hx

## The bug (confirmed by reading the generated output)
characters/sandbag/library/scripts/Character/Script.hx, function removeAllEffects:

    function removeAllEffects(arg0) {
        var i = 0;
        while (i < effects.get().length) {
            if (effects.get()[i] == null) {
                i = i + 1;                                  // increments
            } else {
                if (effects.get()[i].parent == null) {
                } else {
                    effects.get()[i].parent.removeChild(effects.get()[i]);
                }
                // <-- NO i = i + 1 HERE. Any NON-NULL effect => i never advances
                //     => infinite loop => ENGINE FREEZE.
            }
        }
        effects.set(new Array());
        ...
    }

When `effects` holds any non-null entry, the else-branch never increments `i`,
so `while (i < length)` spins forever. `clearEffectsOnStateChange` registers
`removeAllEffects` as a GameObjectEvent.LINK_FRAMES listener, so it fires every
frame once effects exist → matches the user's "loads, then freezes after the
match starts" symptom exactly.

## Why the decompiler produced it
The SSF2 AS3 original almost certainly did `removeChild(...)` then advanced (e.g.
`effects.splice(i,1)` WITHOUT i++ — splice shifts, so not incrementing is correct
when you remove the element; OR `i++` in both branches). Our AS3→Haxe decompiler
(src/decompiler.rs) translated the array-walk but DROPPED the index advance on the
remove branch — it converted the SSF2 mutate-while-iterate (splice) into a
non-mutating indexed read (effects.get()[i]) without re-adding the i++.

## THE FIX (converter-side, src/decompiler.rs)
Two valid fixes — pick based on the AS3 source semantics:
1. If the AS3 used splice(i,1) to remove: emit a removal that shrinks the array
   (so length decreases) — port to `effects.get().splice(i,1)` and KEEP no-i++
   on the remove branch (correct for splice). Currently it calls removeChild on
   the display object but never removes the entry from `effects`, so even with an
   i++ the list isn't pruned — but the loop would at least terminate.
2. Minimal safe fix: ensure EVERY while-loop branch advances the loop variable —
   add `i = i + 1;` to the else branch. Loop terminates; removeChild still runs.
   Combined with the trailing `effects.set(new Array())` the list is cleared
   anyway, so #2 is behavior-correct here.

Best: make the decompiler's array-iteration emitter guarantee the index variable
advances on ALL paths (or translate AS3 splice-during-iteration faithfully).
Verify no OTHER generated character has the same pattern (grep all converted
scripts for `while (i <` blocks whose else-branch lacks an increment).

## STATUS
- Root cause found by reading generated output (high confidence; it's a literal
  unconditional infinite loop).
- Have NOT yet located the exact decompiler code site — the tool channel began
  corrupting `grep` output (repeating/mangling lines) right as I searched
  src/decompiler.rs. Editing the decompiler under a corrupting channel risks a
  bad patch. Repo is clean/committed.
- Earlier in THIS pass I also found CharacterStats duplicate/aliased keys
  (gravity x2 etc.) — separate lower-pri cleanup (see SANDBAG_FREEZE_INVESTIGATION.md).

## RESUME (healthy channel)
1. Open src/decompiler.rs; find the loop/array-iteration lowering that emits the
   `while (i < ...) { if null {i++} else {...} }` shape. Fix so the index always
   advances (or port splice faithfully).
2. Rebuild converter, re-convert sandbag, confirm removeAllEffects terminates
   (grep the new Script.hx: the else-branch must advance i or splice).
3. Re-publish .fra, load in the user's normal (Steam) Fraymakers, start a match
   with sandbag, confirm NO freeze. THEN proceed to move/physics/animation
   validation (criteria #4-#6).
