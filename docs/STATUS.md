# Peptide status, known issues, and next steps

where the SSF2 to Fraymakers conversion stands across the roster, plus the live list of
open issues and what's worth doing next. this is the one place we track status and TODOs
for the whole project, so known issues and next steps live here instead of scattered
around. the converter status is up top; the Peptide (engine-harness) feature TODOs are at
the very bottom. if you want to verify any of this yourself,
[`PEPTIDE_GUIDE.md`](PEPTIDE_GUIDE.md) shows you how.

## coverage

**the whole SSF2 roster converts and drives in-engine. nothing's currently broken!**
every character:

- **converts.** the converter exits 0 and produces output, with no hard-conversion failures.
- **drives in-engine.** it spawns in Fraymakers via Peptide, reaches `STAND`, and runs its
  moves without crashing (P0: `LAUNCHED` + `ANIM:STAND` + `M:OK`).
- **has populated entities.** real movesets and hitboxes, checked by
  `tools/tests/parity_check.py` (entity animation counts plus HIT_BOX active-frame
  coverage) so we're not just trusting a shallow spawn signal. hitbox frame-coverage runs
  52-90%+ per character; what's left are projectile specials and throws that have no melee
  box anyway.

a few are hand-verified deep (drive moves + physics + anim): `sandbag` (the reference),
`mario` (the deepest, a full 18-move sweep), `kirby`, `bowser`, `fox`. everybody else is
batch-verified (spawn + jab + special_neutral, no crash).

## hitbox-stat parity

**every character's hitbox stats match the SSF2 source. no exceptions right now.**
`tools/tests/parity_check.py` checks every hitbox's `damage` / `angle` / `baseKnockback` /
`knockbackGrowth` / `hitstop` / `selfHitstop` / `hitstun` against the SSF2 source (dumped
via `DUMP_PARITY=1`). that covers the hitbox-*stats* slice of functional equivalence.

the real bar for a "done" character is functional equivalence with SSF2, which is a higher
bar than "spawns, animates, doesn't crash." hitbox-stat parity is one piece of it. the open
dimensions below are the rest.

## open parity dimensions

- **frame data (active-frame range).** startup/active/recovery lives in the `.entity`
  collision-box keyframes and animation lengths, not in `HitboxStats.hx`. we check per-char
  coverage (it's what caught the empty shells), but an exact active-frame comparison against
  SSF2 (×2 for the 30→60 fps bump) is its own fiddly harness pass.
- **special-angle sentinels.** SSF2 sentinel angles (`-1`/`-2`/`-3`…) are preserved
  faithfully, we just haven't mapped them to FM's special-angle codes yet. needs the
  SSF2-sentinel → FM-angle table.
- **per-segment hitbox fidelity.** split sub-anims (`jab2`/`jab3`) inherit the base
  attack's hitbox stats for now. that's sound (FM gates a hitbox by the animation's
  collision-box layers, so an inactive inherited box just sits there inert), but the
  damage/KB *values* are an approximation of the real per-hit finisher. getting the true
  per-hit values means pulling hitbox activation frames out of the SSF2 sprite timeline.
- **physics-stat tuning.** movement stats (gravity/fall/walk/jump/weight) are mapped and
  scaled, but the `stats.jsonc :: multipliers` got hand-tuned by eyeballing template
  characters against SSF2 data. `friction`, `shortHopSpeed`, and the ECB head/hip/foot
  positions are hand-tuned constants too, so ground deceleration, short-hop, and hurtbox
  sizing are all approximate. generated `CharacterStats.hx` flags the shaky numbers with
  `/*TODO*/`. dialing the scale factors in for real needs an in-engine SSF2-vs-converted
  comparison (the dummy / measurement path), not a static fix.
- **no-FM-equivalent calls.** CPU-AI branches, z-order (`getMC`/`swapChildren`),
  `forceAttack`, and the item system have no FM mapping, so they get commented out as
  `[SSF2-only]`. no impact for human play of non-item characters.

## measuring progress

`tools/tests/translation_completeness.sh` counts untranslated markers per character:
`/* ? */` (decompiler couldn't recover an expr/condition/receiver), `[SSF2-only:` (no FM
equivalent), and `TODO` (value punted to a default). lower is better. it's the safe
before/after gate for decompiler/mapping changes, since a real fix should *drop* the marker
count without adding new ones, and the in-engine spawn sweep still has to pass. ten
characters are fully `/* ? */`-clean. the decompiler-quality outliers worth a focused pass
later are `kirby`, `tails`, `rayman`, `pacman`, `goku`/`lucario`, `dedede`, `sonic`,
`yoshi`.

## how parity should be verified going forward

the endgame is a `verify <move>` Peptide command (see
[`PEPTIDE_DESIGN.md`](PEPTIDE_DESIGN.md) roadmap) that drives a move against a dummy and
diffs what actually happens against the SSF2 reference values. we've already got the
hitbox-stat reference in hand. the missing piece is in-engine measurement of emergent
behavior (a dummy opponent plus post-hit readback), and that's the next focused project.

---

## known issues & gaps

the live list of open converter issues. strike an entry when you fix it.

- **shape-only menu portraits.** a few characters (`donkeykong`, `fox`, `marth`) have
  `*_head` portraits built entirely from shapes instead of a bitmap. the head finder grabs
  a Bitmap placement when there is one; when there isn't, the head image comes up empty and
  `Menu.entity` ships a placeholder. wants a small SWF shape rasterizer.

- **mario sprite placement not re-verified.** after the recent rotation / itemBox /
  shear-baking work most characters look right in FrayTools, but mario specifically hasn't
  been re-checked frame by frame. he was the canary that drove the rotation work, so some
  of his animations might still need a focused pass.

- **vector-only effect sprites get skipped silently.** effects whose visuals are pure
  vector shapes with solid-colour fills (some charge sparkles, the F-air twinkle) can't be
  rasterized without a full SWF vector renderer, so only bitmap-backed shapes get exported.

- **frame-script / API translation is incomplete.** `commands.jsonc` handles the bulk of
  SSF2 API calls, and the `ssf2_only` list plus the `conversion_log.json :: unknown` stream
  surface whatever's left. the generated `.hx` always wants a human read.

- **projectile behaviour is stubbed.** projectile *entities* (visuals, boxes, animations,
  palettes) generate fine. projectile *behaviour* (`<Pascal>Script.hx`) is template
  scaffolding with `// TODO: tune X_SPEED / Y_SPEED` placeholders and, for multi-state
  projectiles, empty `LState` transitions.

- **stat scaling is approximate.** see "physics-stat tuning" above. the
  `stats.jsonc :: multipliers` are hand-tuned and the generated `CharacterStats.hx` marks
  the shaky ones with `/*TODO*/`.

- **`tools/rebuild-sandbag.sh` has an absolute path** baked in, so it breaks if the repo
  moves.

- **`tokio` is declared as a dependency but the converter is synchronous.** `main.rs` has
  no async. it's probably vestigial. verify before relying on it, and consider dropping it
  to cut build time.

- **robustness.** `process_character` swallows per-stage errors and carries on with
  defaults. great for batch runs, but it means a partly-broken character can sneak out
  without an obvious failure. the Tier 1 validation pass catches the most common silent
  regressions (empty stats, empty attacks, declared-vs-extracted mismatch) and writes them
  to `conversion_log.json :: validation_warnings`. always skim `conversion_stats.json` and
  `conversion_log.json` after a run.

- **transformation characters need manual FM-side wiring.** giga bowser and wario man come
  out as standalone packages (`characters/gigabowser/`, `characters/wario_man/`) because
  Fraymakers has no native transformation API. the TODO banner in `CharacterStats.hx` and
  the `ssf2_source` block in `conversion_log.json` both flag it. the content author still
  has to script the swap by hand in the parent character's `Script.hx`.

- **`build_*_map` legacy block deferred.** there's ~350 lines of `build_method_map` /
  `build_property_map` / `build_state_map` / `build_event_map` / `build_hitbox_prop_map` /
  `load_api_methods_json` in `api_mappings.rs`, marked TODO and kept around until JSONC parity
  is confirmed. once the JSONC tables clearly cover all of it, delete the block.

- **path 2 enumeration fallback is still around.** the fallback in `detect_char_names`
  (instance-method enumeration on Main) is kept for one release as a safety net for a
  hypothetical future SSF that builds its roster array dynamically. it's slated for deletion
  in a follow-up; watch the warn log to see if it ever fires.

### code-quality backlog

a standing audit: optimization, cleanup, and latent-bug items with file refs. verify each
one against `git log` before acting, since line numbers drift. what's still open, roughly
by leverage:

- **parse the SWF once per character** (`main.rs`, `sprite_parser.rs`, `image_extractor.rs`).
  a handful of entry points still re-run `swf::decompress_swf` + `swf::parse_swf` on the
  same buffer (and `extract_xframe_transforms` runs twice). thread the parsed `swf::Swf`
  through instead. biggest correctness-preserving perf win on the board.
- **per-multiname / per-method `String` clones in `abc_parser`.** store `name_idx` and look
  up on demand instead of cloning every multiname name.
- **replace the hand-rolled SWF tag walker in `sound_extractor.rs`** (`parse_sounds`) with
  the `swf` crate's `DefineSound` parsing. deletes ~80 lines and a second parse.
- **`entity_gen` UUID seed can collide** when one animation frame has two boxes sharing an
  instance name (`sym_box_{anim}_{inst}_{frame}`). there's a defensive comment on it for
  now; either confirm SSF2 never duplicates an instance name within a frame, or fold depth
  into the seed.
- **`setlocal_0` → `self = …`** in `decompiler.rs` emits uncompilable Haxe if a frame script
  rebinds `this`-as-local to some arbitrary expression. the self-assign guard only covers
  `this`→`this`.
- **duplicate fallback / split tables.** `apply_fallbacks` (`sprite_parser`) vs
  `apply_image_fallbacks` (`image_extractor`), and `expand_split_anim` (`extractor`) vs the
  splitter rules. lift each pair to one shared table.
- **`build_*_map` legacy block** in `api_mappings.rs` (~350 lines). delete once JSONC parity
  is confirmed (see the known-issue above).

---

## prioritized next steps

roughly the order a fresh agent should pick these up.

1. **keep the full-corpus convert sweep green.** re-run the whole `../ssf2-ssfs/` corpus
   against the current converter after any decompiler/parser change, and trace anything that
   hard-fails before it quietly regresses the clean status.

2. **shape-only head rasterizer.** add a minimal SWF shape rasterizer (or pull one out of
   `ruffle`) so the `donkeykong` / `fox` / `marth` menu portraits actually have pixels
   instead of placeholders.

3. **verify mario in FrayTools.** re-run mario, open him in FrayTools, scrub frame by frame,
   and tune any leftover placement / rotation / scale issues.

4. **projectile behaviour.** swap the `// TODO` stubs in the projectile `<Pascal>Script.hx`
   generators for real translated logic, reusing the decompiler + JSONC rewriter pipeline we
   already have.

5. **validate stat scaling** against a handful of hand-tuned reference characters and tighten
   the `stats.jsonc :: multipliers`.

6. **per-character status sweep.** batch-convert the full roster, triage who converts
   cleanly, and capture a per-character status list. the `validation_warnings` block in
   `conversion_log.json` is a good first-pass triage signal.

7. **sweep the `build_*_map` deferred block.** confirm JSONC parity, delete the legacy code,
   and simplify the shape of `api_mappings.rs`.

8. **delete the path 2 enumeration fallback** in `detect_char_names` once a release confirms
   the constructor walker handles everything. that collapses `derive_id_from_getter` and
   `derive_id_from_bundle_method_name` into a single identity.

9. **housekeeping:** make `tools/rebuild-sandbag.sh` path-relative, and drop the unused
   `tokio` dependency.

---

## peptide / harness todos

the live-engine tooling features we want to build. this is the one list (PEPTIDE_DESIGN.md
points here).

1. **multiplayer in quick boot.** on Fraymakers extra players spawn and are fully accessible:
   `startMatch mario,mario` adds each extra player into the live match one per frame, and `p1`
   binds to the live 2nd character (`match.characterCount()`==2, `p1.getStateName()`/`p1.damage`
   readable). still open: distinct CUSTOM chars as p1 don't self-bootstrap (same-char / base-game
   only), they don't yet take hits in a verified way, and the SSF2 side. prerequisite for #6.
3. **`addCharacter`** -- drop one more fighter into the LIVE match on the fly. the command
   (`addCharacter`, aliases `addchar`/`add`, wire `n`) re-arms the per-frame deferred-spawn from a
   stashed copy of the roster and fires one extra spawn, verified firing live. still open: the live
   match allocates 2 player slots, so the per-frame spawn past slot 2 returns null (`SP:0`) and the count
   stays 2. the cap is the match MODE: the self-bootstrap launch uses training mode (the only mode
   that starts from the minimal headless config), which is 1v1. breaking past 2 needs a versus /
   free-for-all mode, and that mode's launch needs the CSS/menu/scene context the injected-bytecode
   path can't supply (see `fraymakers-engine-internals`). so addChar's per-frame spawn trigger is
   done and correct; the 2-player ceiling is the shared architectural wall under #1/#6/#7.
4. **scenario replay test env.** the `scenario` command sets up a deterministic, re-runnable
   scene: `scenario <p0 x,y[,vx,vy]> <p1 x,y[,vx,vy]> [<ctrl:frames>…]` places both players at
   fixed positions (optionally with world-space momentum), resets them to neutral STAND, then
   plays an input timeline on p0. re-run the exact line to replay it. host-side macro composing
   `eval` + `seq` through the DebugTarget seam (so it works on both engines). still open: setting
   a precise animation FRAME (not just the state) and the hit-measurement readback (#6) that makes
   a scenario's outcome quantifiable.
5. **live move-tuning** -- the `tune <player> <hitboxIndex> <stat>=<value> …` command
   hot-reloads a move's hitbox stats into the running match with no relaunch (e.g.
   `tune p0 0 damage=15 baseKnockback=50 angle=45`), via the engine's own
   `updateHitboxStats`. host-side eval wrapper, validated + unit-tested. still open: the
   UI surface for it, tweaking move *code* (not just stats) on the fly, and persisting
   tweaks back to the source stats files.
6. **in-engine hit measurement** (needs #1): hit-result readback (damage dealt, knockback
   distance + angle, hitstun frames), KO-threshold search (binary-search the dummy's % for
   the lowest KO), and an active-box dump (every active hit/hurt box this frame). open
   confounds to solve: `toState(JAB)` doesn't arm a hitbox to damage an overlapping dummy,
   `p0.flipX()` returns 0 not a facing sign, and `getX` has a Y-dependent offset from `setX`.
7. **frame-advantage display** in the Peptide UI, on shield hit and on hit.
8. **overlay mode.** `$PEPTIDE_OVERLAY=1` floats the console ON TOP of the running game:
   always-on-top, compact (440x560), parked top-right of the primary monitor, with the full UI
   (matching our theming). F8 toggles always-on-top on/off live so you can pop it over the match
   and drop it back without relaunching. still open: true window transparency (the system webview
   makes this fiddly per-OS) and auto re-fit when the game window moves/resizes (needs OS window
   tracking).
9. **batch commands / inputs from a file** -- the UI half. the CLI half is `peptide tell
   --file <path>` (one command per line, `#` comments skipped; mixes engine cmds, `e`
   hscript, and `seq`/`hold` inputs through the one dispatch path).
10. **way more hscript commands.** convenience commands now wrap common eval patterns through
    the one dispatch path (so they work on both engines): `scenario` (#4), `tune` (#5),
    `dmg <player> <value>` (set damage percent), and `info` (one-shot readout of both players'
    x / state / damage / team). all validated + unit-tested. more can be added the same way (a
    `Cmd` registry entry + a `parse_*` that returns `Command::Eval`).
