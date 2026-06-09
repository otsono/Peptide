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

## stage porting

`peptide ssf2 stage <file.ssf>` converts an SSF2 stage to a playable Fraymakers stage. it
parses the SSF2 placement tree for the floor, soft platforms, death/camera boxes, and
entrance/respawn beacons, rasterizes the stage's art (vector shapes + bitmap backgrounds) into the stage sprite, and emits
the FM `.entity` + manifest + StageStats/Script + `.fraytools`; FrayTools builds + publishes
the `.fra` to the engine's `custom/` dir. all 110 corpus stages convert. battlefield (DAT328)
is live-verified: `spawn sandbag battlefieldssf2` (drive with `FRAY_STAGE=<stage>`) loads the
stage, the fighter stands on the platforms, walks, and falls off the edges into the blast
zone (KO + respawn), no crash. NB: a stage needs visible content to place players (the engine
sizes it from the sprite bounds); the sprite is the SSF2 art (vector shapes + bitmap backgrounds rasterized + composited);
stages with no rasterizable art fall back to a geometry placeholder.

the SSF2 `foreground` plane normally folds behind fighters (it's usually a structure's re-drawn
front face, which would read as a duplicate platform); a `keep_foreground` metadata flag opts a
stage out so a genuine overlay stays in front (bowserscastle's glowing lava sheet over the floor).

the backdrop is emitted the way SSF2 authors it: each animated element (lava bubbles, torches,
embers, podoboos, a spectator) is its own FM layer/animation on its own loop, in back-to-front
draw order, rather than one baked composite. elements are grouped by their SSF2 symbol id; a
static element is one frame, an animated one keeps its loop (run-length-encoded at the 30->60fps
doubling, so a held source frame reads as a pause). a single-element backdrop is one layer, same
as before.

the converted stage carries a clean display name (a per-stage override in
`mappings/stage/metadata.jsonc`, with the SSF2 id title-cased as the fallback) plus a source
series, and records its original SSF2 soundtrack (the `bgm_*` ids
pulled from the stage DAT) in the description. it plays a real FM bgm (the override map's pick
or `default_music`) since the SSF2 audio isn't shipped with FM. the content id is suffixed
`<id>ssf2` so it can't shadow a built-in stage.

auto-detected moving platforms (the `moving`-named SSF2 container, propagated to its collision
child) are kept as static collision at their start position, labelled `(SSF2 moving, static)`
in the entity and surfaced in a warning. the motion itself isn't ported there: SSF2 moving
platforms are bespoke per stage (custom AS3 classes / timeline animation), so reproducing them
is per-stage work like porting a character special.

declared `platforms` in metadata become real FM moving Structures (the engine idiom from the
stage-template): a per-platform grey block animation (an IMAGE sized to the platform + a FLOOR
line segment) and one structure CONTENT per platform that the stage spawns and that moves itself
in its own Script. bowserscastle has three grey standing platforms RE'd from the SSF2 terrain
(two wide ledge-bounded side platforms + the small central `terrainGround` block, at their real
positions and widths, in the SSF2 platform grey), sitting over the lava. the thwomp lands on
them and pushes them down: the shared sink/rise Script polls the stage's custom game objects,
sinks the column the thwomp landed on, holds, then rises back. the thwomp is a custom game object
that falls onto each platform column in turn (each at its own height), damages on landing,
re-arms its hitbox each cycle, then rises and moves to the next column.

stage hazards become Fraymakers custom game objects the stage spawns with a null owner, so each
is neutral and damages everyone. damage and knockback come from the entity's native HIT_BOX
(HitboxStats), and three pieces are load-bearing for that hitbox to connect: a null owner (passes
team-hit validation), `stateTransitionMapOverrides` mapping `PState.ACTIVE` to the hitbox
animation (so an animation actually plays and the collision detector resolves it against the
HitboxStats map), and a content-ref `spriteContent` (a bare id string doesn't load the sprite). a
local state machine plays the active/inactive animations for the on/off pulse, and the hitbox is
re-armed on a cadence (`rehit`) so a lingering fighter keeps taking hits. movement patterns
(oscillate / circle / thwomp-fall) move the whole object. each hazard renders its real rasterized
SSF2 art (the thwomp's stone face, the bumper, the lava) recovered from the placement tree; a
hand-declared hazard borrows the detected sprite of its kind, and only a hazard with no recoverable
art falls back to a translucent placeholder box.

damaging hazards auto-detect from the SSF2 placement tree (lava, acid, thwomps, spikes, bumpers,
tornados, damage zones, podoboos, piranhas) by linkage name. the kind is propagated down from the
named container, leaf shapes of one hazard cluster into a single box, and the result is filtered to
the reachable area inside the blast zone (so a hazard parked off-screen at frame 0 doesn't ship as a
phantom hitbox). a `hazards` list in metadata overrides detection for a stage (full manual control,
e.g. a thwomp whose frame-0 position is parked, or a dynamically-spawned hazard); `no_hazards: true`
suppresses a false positive. dynamically-spawned hazards (cannonballs, arwing lasers, birdo eggs,
holy lances) that don't sit in the static tree are a per-stage follow-up, declared by hand as the
need arises. follow-ups: the moving-platform motion, dynamically-spawned hazards, per-stage FM-music
mapping, porting the actual SSF2 audio (licensing-permitting).

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
- **SSF2-only / no-FM-equivalent inventory.** SSF2 API surface with no Fraymakers
  mapping. Calls get commented out as `[SSF2-only]` (no impact for human play of
  non-item characters); stat fields and events are handled as noted. The
  authoritative per-entry list (with notes + `manual_port`/`no_equivalent`
  category) lives in `mappings/commands.jsonc :: ssf2_only`; runtime tallies land
  in each character's `conversion_log.json :: ssf2_only`. Confirm any entry as
  droppable (like `priority`) or map it as the API surfaces.

  - **mapped this pass (live-verified on zelda/sandbag/ganondorf):** `getMC`→
    `getViewRootContainer()`, `getStanceMC`→`getSprite()` (members read off it may
    differ; flagged inline), `toCrashLand`→`toState(CState.CRASH_BOUNCE)`, `toIdle`→
    `toState(CState.STAND)`, `toFlying`→`toState(CState.HURT_HEAVY)`; `hitTestGround(x,y)`
    →`hitTestStructuresWithLineSegment(new Point(x,y), new Point(x,y), null, null)` with
    the `!= null`/`== null` checks rewritten to `.length > 0`/`== 0` (Array return), the
    options arg dropped, and a TODO on the point→segment approximation; `if (… isForcedCrash …)`
    blocks DELETED (no forced-crash in FM). AS3 global casts the decompiler wrongly
    attached to `self` (so they were null → "Invalid function null"): `self.int`/
    `self.uint`→`Std.int`, `self.Number(x)`→`Std.parseFloat(Std.string(x))` (`Std` is
    in character-script scope, confirmed by a live probe). Stat fields: `hasEffect`→
    `flinch`, `sdiDistance`→`hitstopNudgeMultiplier` (value/6, since SSF2 default is 6),
    `shieldDamage`→`shieldDamageMultiplier: 1` with the old value kept in a TODO.
  - **method calls, `no_equivalent` (57), commented out.** item system (`getItem`,
    `getItems`, `getItemStat`, `pickupItem`, `tossItem`, `toToss`, `removeItem`,
    `generateItem`, `updateItemStats`, `isZDropped`); CPU/AI (`isCPU`, `getCPULevel`,
    `getCPUAction`, `getCPUForcedAction`, `getCPUTarget`, `setCPUAttackQueue`,
    `importCPUControls`, `resetCPUControls`, `isStandby`, `getStandby`); Flash/MovieClip
    (`getMCByLinkageName`, `getHitBox`, `getKirbyHatMC`, `getHUDBackgroundMC`,
    `getHUDForegroundMC`, `killDarkener`); sound-needs-a-clip-handle (`stopSound`,
    `stopSFX`, `stopHoldSound`, `stopListening`); final-smash/kirby/misc
    (`isUsingFinalSmash`, `setFinalSmashMeterCharge`, `getCurrentKirbyPower`,
    `getCurrentMusicInfo`, `getQualitySettings`); other (`addIgnoreObject`,
    `calculateKnockback`, `checkAtkilled`, `clearEffectsOnStateChange`, `endControl`,
    `exportStats`, `getLedges`, `getNearestPath`, `isReversed`, `jumpToContinue`,
    `killAttackboxes`, `removeSelfPlatform`, `replacePalette`, `setAttackEnabled`,
    `setLastUsed`, `shootOutOpponent`, `spawnEnemy`, `stop`, `updateEnemyStats`).
  - **method calls, `manual_port` (26), close FM equivalent needs hand-work.**
    `fireProjectile`→`match.createProjectile`, `unnattachFromGround`→`unattachFromFloor`,
    `swapDepthsWithGrabbedOpponent`→`swapDepths`, `getMetalStatus`→StatusEffect/BodyStatus,
    `setColorFilters`→`setCostumeShader`, `getCharacter`/`getProjectile`→
    `match.getCharacters`/`getProjectiles`, plus `angleControl`, `createSelfPlatform`,
    `forceAttack`, `getAttackBoxStat`, `getAttackStat`, `getCurrentAttackFrame`,
    `getCurrentProjectile`, `getExecTime`, `getHealthBox`, `getLinkageID`, `getMidground`,
    `getNearest`, `getNearestLedge`, `getPlatformBetweenPoints`,
    `hitTestGroundBetweenPoints`, `homeTowardsTarget`, `inUpperLeftWarningBounds`,
    `isEqual`, `setHurtInterrupt`.
  - **hitbox/attack stat fields still with no FM mapping**, surfaced as compile/runtime
    "invalid stat" for per-site fixing: `camShake`, `chargedPriority`, `ignoreChargeDamage`,
    `meteorBounce`, `onlyAffectsGround`. `stackKnockback` is actually a real FM
    `HitboxStatsProps` field (same name) so it passes through fine, not a gap. NB: the
    "Invalid hitbox stat" check fires only when a hitbox is ACTIVE in-engine, so an eval
    `updateHitboxStats(0, {x: 1})` on an inactive box won't surface it; confirm field
    validity against the FM API types.
    - `priority` DROPPED (confirmed unnecessary). `burn`/`shock` `: true` map to FM
      `element` (`ElementType.FIRE`/`ELECTRIC`, live-verified); their `: false` /
      `pitfall: 0` no-element forms are dropped. Still open (carry a value or an
      unconfirmed FM enum, so per-site, not a flag→enum rename): `aura` (no confirmed
      `ElementType.AURA`), `paralysis` (numeric stun duration), `pitfall` > 0 (FM
      `ElementType.BURY` + `bury*` timing fields, not a bare flag).
  - **Flash display-list / timeline methods called on the mapped view objects**
    (`getMC`→`getViewRootContainer()`, `getStanceMC`→`getSprite()`): `swapChildren`
    (35), `removeChild` (48), `addChild` (12), `getChildByName` (14), `getChildAt`/
    `numChildren`, and timeline reads `currentLabel` (37), `currentFrame` (18),
    `gotoAndPlay`. NONE exist in the FraymakersApiTypes (FM rendering/animation is not
    a Flash display list), so they're null at runtime and need per-site porting (e.g.
    z-order via FM layering, timeline reads via the FM animation API). This is the main
    remaining `Invalid function null` source after the AS3-cast fix.
  - **`SSF2Event` types with no confirmed FM equivalent, neutralized** (the line is
    commented so it can't pass a null event): `KO_POINT`, `CHAR_ATTACK_COMPLETE`,
    `CHAR_ATTACK_CHANGED`, `REVERSE`, `REVERSE_HIT`, `CHAR_COUNTER`, `CHAR_METAL_CHANGE`,
    `PROJ_COLLIDE`, `PROJ_DESTROYED`, `PROJ_HURT`, `HOMING_TARGET`, `ENEMY_DESTROYED`,
    `ITEM_TOSSED`, `ITEM_DESTROYED`, `CHAR_SELF_DESTRUCT`, `CHAR_ABSORB`,
    `CHAR_POWER_SHIELD_HIT`, `CHAR_SIZE_CHANGE`, `CHAR_TRANSFORM`, `ATTACK_ENABLED`,
    `GAME_ITEM_CREATED`. (Mapped this pass: `ATTACK_HIT`, `ATTACK_HIT_SHIELD`,
    `GROUND_LEAVE`, `CHAR_SHIELD_HIT`, `CHAR_KO_DEATH`→`CharacterEvent.KNOCK_OUT`.)

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

- **path 2 enumeration fallback is still around.** the fallback in `detect_char_names`
  (instance-method enumeration on Main) is kept as a safety net for a hypothetical future SSF
  that builds its roster array dynamically. a full 47-character corpus sweep never triggers
  the "constructor walk returned empty" warn, so the constructor walk handles the whole known
  roster; the net stays until a shipped release confirms the same in the wild, then it and
  `derive_id_from_getter` can go.

---

## prioritized next steps

roughly the order a fresh agent should pick these up.

1. **keep the full-corpus convert sweep green.** re-run the whole `../ssf2-ssfs/` corpus
   against the current converter after any decompiler/parser change, and trace anything that
   hard-fails before it quietly regresses the clean status. (currently green: 47/47 convert
   exit 0, 45/46 fighters with zero `validation_warnings`; `misc.ssf`, a palette archive, is
   the only one with a benign "no attacks" note.)

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

6. **delete the path 2 enumeration fallback** in `detect_char_names` once a shipped release
   confirms the constructor walker handles everything in the wild (a full local corpus sweep
   already never trips its warn). that collapses `derive_id_from_getter` and
   `derive_id_from_bundle_method_name` into a single identity.

---

## peptide / harness todos

the live-engine tooling features we want to build. this is the one list (PEPTIDE_DESIGN.md
points here).

1. **multiplayer in quick boot.** 1-4 player matches work on Fraymakers. the `spawn`/fastboot
   vocabulary takes a roster of up to 4 comma-separated characters (`spawn a,b,c,d [stage] [assist]`,
   `FRAY_ROSTER`/config `roster`, `--char a,b,c,d`); the launch builds the full N-player characters
   array and the engine constructs every fighter natively. 1-2 players use training mode (the
   parity-harness "fighter + dummy" path), 3-4 auto-engage versus mode (selected at runtime by roster
   size). each extra is synchronously self-bootstrapped, so DISTINCT custom chars in one roster work
   (`spawn sandbag,mario,sandbag,mario` -> 4 live, verified). every fighter binds to `p0`/`p1`/`p2`/`p3`
   and `match.getCharacters().length` reports the real count (read host-side from the live match, since
   hscript lacks RTTI on the raw character list). verified live: rosters of 1/2/2-distinct/3/4 ->
   `getCharacters()` 2/2/2/3/4, zero crashes, `p3.getStateName()` drivable. `spawn --versus a,b` forces
   versus mode for a 1-2 player roster (wire byte `S`). still open: they don't yet take hits in a
   verified way, and the SSF2 side. prerequisite for #6. (impl note: extras are built as UNROLLED straight-line blocks -- emitting the
   char-resolve helper inside a runtime back-branch corrupts the loop counter, so each of the <=3
   extras is its own guarded block.)
3. **`addCharacter`** -- drop one more fighter into the LIVE match on the fly. the command
   (`addCharacter`, aliases `addchar`/`add`, wire `n`) re-arms the per-frame deferred-spawn from a
   stashed copy of the roster and fires one extra spawn, verified firing live. now that the launch
   path lands the full roster up front (#1), the typical use case is covered there; this is for adding
   a fighter mid-match. open: the deferred per-frame add hasn't been re-verified against a versus-mode
   match for slots past the initial roster.
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
   the lowest KO), and an active-box dump (every active hit/hurt box this frame). damage
   readback already works live: `scenario 0,0 20,0` (dummy overlapping) + `seq attack:3`
   (drive the attack via INPUT, not `toState`) + reading `p1.damage._damage` shows the dummy
   take the hit (0 → 9 on a sandbag jab). so the `toState(JAB)` "doesn't arm a hitbox" confound
   is bypassed by input-driving the attack. still open: knockback/hitstun readback, the
   KO-search, the active-box dump, and two position confounds: `p0.flipX()` returns 0 not a
   facing sign, and `getX` carries an offset from `setX` (`setX(0)` reads back `getX -19.5`).
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
