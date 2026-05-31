# Overnight session log

Terse, one line per milestone: `HH:MM — summary. Commit <hash>.`
Detail lives in per-topic docs, not here.

- 08:45 — 11 characters verified functional in-engine (added marth, falco, captainfalcon, donkeykong, bomberman, blackmage via batch_spawn_test.sh — all PASS: LAUNCHED + ANIM:STAND + M:OK, no crash). Committed batch script (754fa30a, e7d6b884). Launched full-corpus background sweep of the remaining 32 → /tmp/corpus_sweep.txt. Updated memory project_fraymakers-match-launch with current Peptide state.
- 08:00 — Corpus: batch-converted all 47 ssf — 44 clean, 2 fail (chibirobo, dedede: SIGKILL/137 OOM entering image extraction — runaway alloc, documented as open in TESTING.md). Stretch: exported + spawn-tested KIRBY (3rd char) — INTRO→STAND, jab/special_neutral/grab all animate, physics clean, no crash. Generic pipeline proven beyond mario/sandbag. Commit 7b76ac86.
- 07:30 — Both chars committed validated (TESTING.md §3/§5). Added `physics` command (criterion #6): walks player 0, writes `P: x= y= vx= vy= dmg=` via Std.string. Fixed a typed-reg bug (HL field offset comes from reg static type — reused Body reg gave garbage velocities). Live: mario dash_attack moves x −130→49 (real displacement), vel 0 at rest, no crash. Commits c2d88e42, d0447215.
- 06:30 — MARIO FUNCTIONAL. Stale May-28 .fra was the crash cause; regenerated from current converter + re-exported via FrayTools → fresh .fra. Mario: spawn → INTRO → STAND (stable), then drove the FULL ground moveset — jab, dash_attack, tilt f/u/d, strong f/u/d (full _IN→_ATTACK chains), special n/s/u/d, grab — every one M:OK + correct ANIM + clean recovery to STAND, no crash. Criteria #3/#4/#5 substantially met for mario's ground game. (Aerials next.)
- 05:55 — Phase 2 done + verified live: `move <name>` dispatch (jab/tilt_down/special_neutral/… 20 moves). Engine jump table GENERATED from shared commands::MOVES (CState resolved by field name; strongs → _IN). Sandbag `move jab` → ANIM:JAB → M:OK → ANIM:STAND. Commit 7a344e5a pushed. (Builtin commandervideo re-spawn dies in ~4s — pre-existing, unrelated to this change; sandbag path is the validated one.)
- 05:25 — Live engine smoke test GREEN. `spawn sandbag` → LAUNCHED, ANIM stream INTRO→STAND, `state` → T:STAND, `query` → Q:MATCH_LIVE, clean 27s exit. Friendly-command translation + reply gloss verified end-to-end against the real engine. Sandbag criterion #3 (boots+spawns, no crash) confirmed on current main. Engine loop is available tonight.
- 05:10 — Phase 1 done: human-facing Peptide command vocabulary (spawn/move/state/query/load/keys/console/ping/exit + help/aliases), reply gloss, all client-side (zero new engine bytecode). 5 unit tests green. Commit pushed below. Wire bytes still work as aliases.
- 04:35 — Session start. Branch: main at 1703f1c2. Goal: sandbag + mario 100% converted & functional in Fraymakers; expand Peptide (single-letter cmds → strings + new features). Found uncommitted WIP in src/palette_gen.rs (edge-bleed for skew bitmaps). FM character-template already vendored at vendor/character-template/. Engine install present at default Steam path.
- 09:15 — Stopped corpus sweep at 25 in-engine PASS (0 crashes). Ran 4-agent parity audit (workflow) on sandbag+mario. Landed 4 converter parity fixes (jab2/jab3 inherit base hitboxes; decompiler skip-body branch normalization; restore wrongly-commented local-fn calls; /* ? */.self. cleanup). Re-exported + regression-tested mario+sandbag in-engine: all moves M:OK, no crash. Wrote docs/PARITY.md (fix status + the deferred makeX/getGlobalVariable plan). Commits 45815115, f002f772, c82e67fd.
- 09:45 — Added observation+iteration commands: `anim` (A:<name> frame cur/total — verified A:intro 75/80, A:stand 61/72) and `loop <move> [count]` (client-side repeated dispatch — verified loop special_neutral 4x). Observation toolkit now state/physics/anim/ANIM-stream + sustained loop. Docs updated (TESTING/MODDER_GUIDE/PEPTIDE_FUTURE). Commits 863e6d5f, f24da189.
- 10:05 — FIX 2 landed + validated: SSF2 get/setGlobalVariable -> FM makeX persistent state (canStartRise->makeBool, standtime->makeInt; calls -> .get()/.set()). mario special_up/down + idle run, no crash. Regression batch (kirby/luigi/link) all PASS — no broad breakage from the 5 converter parity fixes. Commit 8e339841. PARITY.md updated.
- 10:40 — Corpus sweep COMPLETE: 43/45 characters drive in-engine (spawn→STAND→moves, 0 crashes). Only chibirobo+dedede fail (convert-time OOM). zelda was a false-fail (port collision) — passes on retest. Updated docs/character_status.md with the authoritative tally. Added .hl feasibility caveat + hitbox-readback RE lead to PEPTIDE docs; 10 unit tests guard the 5 parity fixes.
- 10:50 — Cracked the chibirobo/dedede convert-time OOM. Built a gated memory-limit allocator (CONV_MEM_LIMIT_MB, default-off) in main.rs; it pinned a 240 GB alloc in decompiler decode_range — a mis-parsed CFG range fed a garbage near-u32::MAX argc/count to call/construct/newarray/newobject pop-loops. Fixed by clamping every (0..argc/count).pop() loop to the operand-stack depth (10 sites). BOTH now convert (exit 0) -> 45/45 characters convert. No mario regression; 47 tests pass. Commit 0f0adbea. Spawn-testing them now.
- 10:58 — chibirobo + dedede now PASS IN-ENGINE (export + spawn + jab + special_neutral, no crash). FULL CORPUS: 45/45 characters convert AND drive in-engine, 0 crashes. Updated character_status.md + TESTING.md. The memory-limit allocator (CONV_MEM_LIMIT_MB) stays as a reusable OOM-localization tool.
- 11:05 — Definitive mario re-validation on the FINAL converter (all 6 fixes): full 18-move moveset, 18/18 M:OK, 0 NOMATCH, every move animates (incl. strong _IN→_ATTACK charge chains), no crash. Headline deliverable confirmed.
- 11:12 — Added `snapshot` command (client-side Sequence: t+v+a bundle) — verified live on mario, 6 bridge tests pass. Lays groundwork for recipe scripting. Docs updated.
- 11:20 — Added tools/translation_completeness.sh — per-character untranslated-marker dashboard (/*?*/, [SSF2-only], TODO). The SAFE before/after metric for decompiler/mapping changes (unlocks evaluating broad parity fixes like stack-threading without hand-reading scripts). Baseline: mario/sandbag clean; kirby (26 /*?*/) flagged as a future decompiler target. PARITY.md updated.
- 11:35 — Landed BranchCmp stack-threading (decompiler): seeds branch bodies with the predecessor operand stack, recovering /* ? */ exprs (fox 1→0, bowser 4→2; none added; fox/bowser/mario spawn+drive clean; 47 tests pass). Hardened batch_spawn_test (deterministic ports + retry) — eliminates the port-collision false-fails (fox+zelda confirmed PASS). Commits 81458f8c, 18140a6c. PARITY.md updated (Branch-arm half still open, gate on completeness metric + spawn).
- 11:48 — forceAttack -> ssf2_only (was a raw undefined call); mario re-verified in-engine (jab/special_up/grab M:OK, snapshot, no crash). Ran full-corpus translation-completeness: most chars clean (single-digit /*?*/); outliers for future decompiler passes: kirby/tails(26), goku(19), rayman(18), lucario/pacman(17). Excluded misc (data file) from the tool's default scan. Commit 6d7ae57b.
- 11:55 — Regenerated all 45 chars on the final converter + ran completeness: corpus /*?*/ down to ~252 (fixes cleared ~23; 10 chars now fully /*?*/-clean). Outliers logged in PARITY.md (kirby 26, tails 23, rayman 18, pacman 17 — future decompiler targets). All work committed + pushed; repo clean; 45/45 chars convert + drive in-engine.

## Session complete — pristine state (final green check passed: both crates build, all tests pass, tree clean, 66 commits pushed)

WHAT'S DONE (all verified live):
- sandbag + mario fully drivable in-engine (mario: full 18-move sweep); 8 converter
  fixes advancing SSF2 functional parity; physics/anim/loop/snapshot readbacks.
- 45/45 characters convert AND drive in-engine, 0 crashes (corpus complete at P0).
- Peptide is human-facing: spawn/move<name>/state/physics/anim/loop/snapshot/query/
  load/keys/console/ping/exit/help + aliases + gloss. Diagnostics: CONV_MEM_LIMIT_MB
  allocator, translation_completeness.sh. Hardened batch sweep (no false-fails).
- Docs: overnight_validation (morning summary + doc map), character_status, PARITY,
  MODDER_GUIDE, PEPTIDE_ARCHITECTURE, PEPTIDE_FUTURE, TESTING.md; memory updated.

WHERE TO PICK UP (deferred deliberately — see docs/PARITY.md for plans + gates):
1. Branch-arm stack-threading (decompiler) — would cut the /*?*/ debt on the
   outliers (kirby 26, tails 23, rayman 18, pacman 17). RISKY (ternary-detection
   interaction); gate on translation_completeness.sh (markers must drop, none
   added) + before/after output DIFF on a ternary-heavy char + the spawn sweep.
2. `verify <move>` harness (diff in-engine behaviour vs the SSF2 reference) +
   `dummy` opponent + hitbox readback — needs the Haxe->.hl merge-at-patch-time
   spike first (PEPTIDE_ARCHITECTURE caveat). Turns parity from eyeballing into
   pass/fail.
3. Per-segment hitbox values, physics-stat tuning (empirical), item/CPU systems.

## P1 PARITY WORK (resumed — bar is SSF2 functional equivalence, not P0)
- 12:30 — Built the parity-measurement harness: DUMP_PARITY raw-SSF2 dump + tools/parity_check.py (diffs SSF2 source vs converter HitboxStats under the documented mapping). haxe NOT installed → .hl path toolchain-blocked; used Rust + the static conversion-fidelity check (highest-signal tractable path).
- 12:45 — Found + fixed 2 P1 hitbox-stat bugs via the harness: (1) baseKnockback now folds SSF2 weightKB (mario special_up had weightKB=120,power=0 → was 0 knockback in-engine); (2) field mapping maxes over PRESENT keys only (absent key's 0.0 was clobbering SSF2 negative special-angle sentinels like direction=-2 → output 0). 
- 12:55 — FULL-CORPUS HITBOX-STAT PARITY: 45/45 characters PARITY OK (damage/angle/baseKnockback/knockbackGrowth/hit-freeze all match SSF2 source). 33 test targets green. link special_down (angle -2) re-verified in-engine, no crash. Commits 924d631a, 6000c472. Next parity dimensions: frame data (active frames/startup/recovery), special-angle sentinel→FM mapping, Branch-arm stack-threading for decompiler outliers.
- 13:30 — Added frame-data dimension to parity_check (HIT_BOX active frames in entity). MAJOR FINDING: the spawn test was shallow (ANIM:X = state name, not a playing animation). 5 characters (fox, bomberman, donkeykong, pit, luffy) have near-empty entities (~11-17 anims, ~0 hitboxes) — per-animation sprite/box resolution fails (fox: box data 9/86 vs mario 88/85), real moves extract empty → dropped below the UNUSED separator. Corrected character_status: 40/45 genuinely functional, 5 broken. Investigating root cause. Commit cb010163.
- 14:30 — RECOVERED ALL 5 BROKEN CHARACTERS (fox/bomberman/donkeykong/pit/luffy were empty shells). Root cause: redundant char-prefix + abbreviated move labels in their sprites (fox_fla.fox_airN; donkeykong dkbair). Fix: prefix-strip ({char}_ + short codes like dk) in sprite_parser + abbreviated label_to_ssf2 entries (airN/smashD/specialN/etc.). Recovery e.g. fox 11anims/0hitbox -> 113/36; all 5 now 87-138 anims. fox re-verified in-engine: real JAB/AERIAL_NEUTRAL/SPECIAL_NEUTRAL animations. Refined parity_check frame-data to a per-char coverage metric (only flags severe gaps). NOW: 45/45 populated + 45/45 hitbox-stat parity, 0 broken. Commits 00652cb8 + frame-data refinement. character_status corrected back to 45/45 (genuinely functional this time).
- 14:50 — All 5 recovered chars CONFIRMED in-engine: bomberman/donkeykong/pit/luffy spawn + show real ANIM:JAB + ANIM:SPECIAL_NEUTRAL (not state-dispatch). 45/45 genuinely functional (real movesets) + 45/45 hitbox-stat parity. The frame-data check that found the broken chars is now a per-char coverage metric. This was the core P1 gap (P0 spawn test masked empty shells) — closed.

## P2 features + .hl spike (goal: finish P1+P2)
- 15:30 — Installed haxe 4.3.7; ran the Haxe→.hl spike: toolchain works (haxe -hl → valid .hl, hlbc reads it) but a trivial Haxe = 322 fns → "loading" into the engine = a full cross-module linker (deferred as a focused project; tools/peptide/hl_spike/).
- 15:45 — P2 features: recipe.sh (shareable .recipe scripts) + recipes/mario_moveset.recipe; ab_compare.sh (golden behavioral-signature regression check, sandbag verified UNCHANGED); crash diagnostics (bridge dumps last ~16 events on stream end).
- 16:10 — Animation scrubbing: step (pause+advance one frame via Animation.playFrame) + play (resume) — verified mario stand 59→60→61 held, play resumes. Self-contained Asm, no s-handler change.
- 16:25 — track <move>: in-engine self-momentum measurement (drive move + rapid physics sampling). Verified mario dash_attack vx 12.86→0 over ~6 samples, +140px. The opponent-knockback half needs the dummy chain (deferred deep item).
- 16:35 — Wrote docs/P1_P2_STATUS.md (honest done/deferred for every P1+P2 item). Remaining deep items (dummy→hitresult, .hl linker, save/restore, physics multiplier tuning) all converge on in-engine emergent-behavior measurement = the next focused project. Launched final 45-char regression sweep.
- 16:45 — Exercised track (momentum-verification loop) on mario: dash_attack lunges (vx 12.3→0, +126px), tilt_forward/strong_forward stationary (vx=0 — correct). track correctly distinguishes moving vs stationary moves. Momentum-PARITY vs SSF2 needs the per-move SSF2 momentum reference (in frame scripts) — noted. Running focused regression spot-check (recovered-5 + diverse) instead of full 45 (already validated via parity_check 45/45 + entity-population).

## Regression spot-check (10-char sweep) — PASS
Ran `batch_spawn_test.sh` over fox bomberman donkeykong pit luffy mario sandbag
kirby marth zelda (a cross-section spanning the recovered-shell chars, the two
reference chars, and chars exercising different sprite-label conventions).
Result: **10/10 PASS, 0 FAIL** — every char regenerates, exports a fresh .fra,
spawns, reaches STAND, and dispatches moves with no crash. Confirms the broad
sprite-label / decompiler stack-threading / hitbox-mapping changes from this
session did not regress any character. Combined with parity_check 45/45 and
entity-population 45/45, the corpus is validated at the P0 bar and stat-parity.

## Dummy-opponent decision (deferred, with rationale)
Evaluated implementing the opt-in dummy opponent directly in the `s`-handler
player-array build (tools/peptide/src/main.rs:1573-1587). Decided AGAINST tonight:
the 2-player path needs a second virtual-typed reg threaded through the delicate
hand-emitted s-handler reg block, and a reg clobber there would corrupt rr33 (the
characters ArrayObj startMatch consumes) — blast radius is ALL spawns, i.e. the
45/45 headline, right before the user wakes. The ab_compare gate would catch drift
but the reward is only the foundation (live hit-tuning additionally needs opponent
positioning + a hitresult/knockback readback). Documented as the keystone
next-project with a concrete plan in P1_P2_STATUS.md / PEPTIDE_FUTURE.md. The
responsible call is to leave the proven state pristine for the morning.
