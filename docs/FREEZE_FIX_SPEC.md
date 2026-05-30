# Sandbag freeze — implementation-ready fix spec

## Confirmed root cause (sha-verified from generated output)
`characters/sandbag/library/scripts/Character/Script.hx` → `removeAllEffects`:
a `while (i < effects.get().length)` loop whose ELSE branch (non-null effect)
never advances `i`, and (because the AS3 `splice` was lost in decompilation)
never shrinks the array → infinite loop → engine freeze. Registered every frame
via `clearEffectsOnStateChange` (LINK_FRAMES listener) → freezes shortly after
match start. Matches the user's symptom exactly.

User's note (correct): AS3→Haxe loop FORMS differ
(http://haxedev.wikidot.com/article:as3-to-haxe-quick-guide). AS3
mutate-during-iterate (`arr.splice(i,1)` with no i++) must become a Haxe form
that still terminates.

## The fix (converter-side, src/decompiler.rs) — termination guard pass
Add a post-processing pass `guard_loop_termination(stmts)` run right AFTER
`rename_loop_counters(stmts)` at the end of the function decompiler (the call site
is the line `let stmts = rename_loop_counters(stmts);`, currently followed by the
render). Pseudocode:

    fn guard_loop_termination(stmts) -> stmts:
      map over stmts; recurse into If/While bodies; for each While(cond, body):
        body = guard_loop_termination(body)   // recurse first
        if let cond == BinOp("<", lhs, rhs) where lhs is a bare counter name
           (one of "i","j","k","l") AND rhs references ".length":
          name = lhs counter
          if NOT counter_definitely_advances(name, body):
            body.push( Stmt::Expr(Expr::GetLex(format!("{} = {} + 1", name, name))) )
        return While(cond, body)

    fn counter_definitely_advances(name, body) -> bool:
      // conservative: true only if EVERY control-flow path increments/changes
      // `name` or splices the iterated array. Simplest correct version: return
      // true iff the LAST statement on every path is an assignment to `name`,
      // OR the body contains a `.splice(` call on the same collection.
      // If unsure, return false (=> we append the guard; always safe: a guard
      // only ever makes a non-terminating loop terminate, never breaks a correct
      // one, because a correct counter loop already ends its body by advancing i
      // and the extra "i = i+1" would double-step — SO the guard must be
      // conditional: only append when NO path advances i).

CRITICAL correctness rule: only append the guard when the increment is MISSING on
at least one path. For the removeAllEffects shape:
    while (i < len) { if (x==null) { i = i+1 } else { ...no i++... } }
the else path lacks i++, so we append i=i+1 to the END of the loop body —
BUT that would double-increment the null path. Better: append i=i+1 to the
ELSE branch specifically (the path that lacks it), not the whole body.

So the precise transform for the common SSF2 pattern:
  - While body is a single If(cond, then, else)
  - exactly one of {then, else} ends with an advance of `name`, the other does not
  - => append `name = name + 1` to the branch that lacks it.
This restores the AS3 semantics (every iteration advances the index) without
double-stepping. Falls back to whole-body-append only if the body isn't a single
If (rare).

## Detecting "ends with an advance of name"
A branch advances `name` if its statement list contains a Stmt::Expr whose
rendered form starts with `"<name> = <name> + "` or `"<name>++"` or `"<name> ="`
re-assigning name, OR a `.splice(` call. Implement by checking
Stmt::Expr(Expr::GetLex(s)) where s.starts_with(&format!("{name} = {name}")) or
s.contains(".splice(").

## Verification after implementing
1. `cargo build --release`
2. `./target/release/ssf2_converter <sandbag.ssf>`  (or rebuild-sandbag.sh)
3. grep new Script.hx removeAllEffects: the else-branch MUST contain `i = i + 1`
   (or the loop must splice). Confirm no `while` loop body has a path lacking an
   index advance.
4. Also scan ALL converted characters for the same pattern (regression guard):
   for each generated Script.hx, every `while (i <` ... `.length)` must advance i
   on all paths.
5. Re-publish sandbag.fra (FrayTools harness), load in Fraymakers, start a match,
   confirm NO freeze.

## Secondary cleanup (lower priority, separate)
CharacterStats.hx emits duplicate/aliased keys (gravity appears in both the
generated stats here as a single 0.78, but earlier dumps showed aliasing like
airFriction/frictionAir, ecbHeight/lecbHeight). Audit the stats emitter
(src/haxe_gen.rs) to emit only canonical FM keys. Not freeze-related.

## WHY NOT IMPLEMENTED THIS PASS
Tool channel is injecting FABRICATED text into Read/Bash DISPLAY output
(confirmed: fake assistant-style commentary appeared in a tool result I did not
author; line numbers in Read jumped non-monotonically; a literal "[misc]"
placeholder appeared in "source"). Arithmetic canaries + file shasums still
verify (commands DO run, files ARE intact), but editing a 1700-line file against
fabricated displayed content risks regressing all conversions. Per the precision
discipline, deferring the edit to a healthy channel. The fix above is fully
specified for clean drop-in.
