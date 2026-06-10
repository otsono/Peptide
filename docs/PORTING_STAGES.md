# porting a stage accurately: the deep-RE playbook

how to take ONE SSF2 stage from "converts" to "verified 1:1" and know you found every object,
behavior, and constant. this is a procedure, not background reading: run every phase in order,
produce every artifact, and don't skip a phase because the stage "looks simple". the phases are
cheap; the bugs they catch are not.

validated end to end on bowserscastle (lava, thwomp, sinking platforms, bridge fg/bg split,
weather, per-element animation). works on any stage in the corpus.

## the iron rules

1. **never invent a value.** every constant (position, speed, timer, damage, count) comes from
   one of exactly three sources, in this order of preference:
   - **static read**: the `.ssf` file (placement matrices, shape bounds, AS3 disasm literals,
     slot defaults, frame labels).
   - **live measurement**: probe the RUNNING SSF2 engine (phase 3) when the engine computes the
     value at runtime.
   - **declared gap**: if you can't read it and can't measure it, write it in the gap report.
     do NOT substitute a plausible number. a wrong-but-plausible value is worse than a gap
     because nobody knows to fix it.
2. **a value you derived must be verified against a second source.** static reads get checked
   against live measurements (or vice versa) at least once per mechanism. when they disagree,
   the live engine wins and you find out why the static read was wrong before reusing it.
3. **tune the JSONC, not the Rust.** per-stage facts go in `mappings/stage/metadata.jsonc` with
   a provenance comment saying exactly where each number came from ("from <class>::update
   disasm", "= the ledge_mc line"). universal mechanisms go in the parser/emitter.
4. **every fix must be universal.** before changing parser/emitter code for your stage, ask:
   what does this do to the other 109? then prove it with the corpus sweep (phase 8).
5. **convert-time success is not success.** a stage that emits cleanly can still be silently
   broken in-engine (dead hazard, frozen layer, culled object). only phase 7 counts.

## phase 0: setup

```bash
cargo build --release --bin peptide
cargo build --release -p ssf2_converter --bin ssf2_objgraph --features dev-tools
# the corpus: ../ssf2-ssfs/stages/*.ssf (or $SSF2_SSFS_DIR)
```

## phase 1: static inventory (what is IN the file)

run all of these and read all of the output. the goal: a complete list of every symbol, plane,
actor, bitmap, and clip before you form any opinion about the stage.

```bash
# parsed model + per-instance debug: linkage table, plane map, AS3 actors, detected hazards,
# art instances with world AABBs
PEPTIDE_STAGE_DEBUG=1 peptide ssf2 stage <stage.ssf> --info

# the full placement tree (every PlaceObject with name/symbol/plane/position, indented)
PEPTIDE_STAGE_TREE=1 PEPTIDE_STAGE_DEBUG=1 peptide ssf2 stage <stage.ssf> --info

# orphan bitmaps: library bitmaps placed by NO shape/sprite/root tag. the engine instantiates
# these by id as named bg children (standable bridges, fg/bg structure splits). same-size
# orphan PAIRS are usually a background piece (solid deck) + a foreground occluder (deck cut
# out, drawn in front of the fighter)
PEPTIDE_ORPHAN_DEBUG=1 PEPTIDE_ORPHAN_DUMP=1 peptide ssf2 stage <stage.ssf> --info
# (dumps each candidate to /tmp/orphan_<id>.png; LOOK at them)
```

build the inventory table: for every SymbolClass linkage and every named instance, note what it
is (terrain / boundary / spawn / art plane / hazard clip / actor class) and whether the
converter already consumes it. anything unexplained is a finding, not noise.

## phase 2: AS3 disasm (what the stage's CODE does)

SSF2 stages are programs. the stage class (extends SSF2Stage) and every actor class it spawns
contain the real behavior constants. read them all:

```bash
peptide-objgraph() { ./build/release/ssf2_objgraph "$@"; }  # alias for brevity

# every class in the file. note which are hand-written (hazards, platforms) vs fla-generated
ssf2_objgraph <stage.ssf> scripts

# the stage class: initialize (plane bindings, spawnEnemy + setX/setY literals, weather,
# timers, event listeners) and update (the hazard state machine: timer values, column arrays,
# spawn positions)
ssf2_objgraph <stage.ssf> methods <StageClass>
ssf2_objgraph <stage.ssf> disasm <StageClass> initialize
ssf2_objgraph <stage.ssf> disasm <StageClass> update

# EVERY actor class (everything passed to spawnEnemy/spawnProjectile + every class with
# initialize/update): all four methods matter
ssf2_objgraph <stage.ssf> disasm <Actor> initialize     # timers, self-platforms, camera, anims
ssf2_objgraph <stage.ssf> disasm <Actor> update         # the behavior state machine
ssf2_objgraph <stage.ssf> disasm <Actor> getOwnStats    # linkage_id, width/height, gravity,
                                                        # max_ySpeed, bypass/survive flags
ssf2_objgraph <stage.ssf> disasm <Actor> getAttackStats # per-anim attack boxes: damage,
                                                        # direction, power, kbConstant,
                                                        # refreshRate, effects, sounds

# class constants that aren't in any method body (slot DEFAULTS, printed with values)
ssf2_objgraph <stage.ssf> slots <Actor>

# who references a symbol (find the spawner of anything)
ssf2_objgraph <stage.ssf> uses <ClassName>
```

extract into a constants table with the source line for each. translate stat keys with the
SHARED key map (the same one the character converter uses, `mappings/commands.jsonc`):
`direction -> angle`, `power -> baseKnockback`, `kbConstant -> knockbackGrowth`.

### unit conversion (SSF2 30fps -> FM 60fps, size_multiplier 1.3)

| SSF2 quantity | FM value |
|---|---|
| frames (timers, durations) | x2 |
| px (positions, sizes) | x1.3 |
| px/frame (velocities) | x0.65 (= 1.3 x 0.5) |
| px/frame^2 (accelerations) | x0.325 (= 1.3 x 0.25) |
| FrameTimer(n) | n x2 FM frames |

note: an actor with `gravity == max_ySpeed` falls at CONSTANT terminal velocity from frame 1
(the thwomp pattern). port it as a constant velocity, not an integrated gravity.

## phase 3: live ground truth (what the RUNNING engine actually does)

boot the real SSF2 on the stage and enumerate the live object tree. this is the phase that
catches everything the file can't tell you (runtime spawn positions, reparenting, engine-added
objects, actual floor heights, weather).

```bash
PEPTIDE_SSF2_STAGE=<stageid> peptide ssf2 session --char mario
# wait for "spawn ... on <stageid>" in ~/.peptide/ssf2-session/out.log

# THE enumerator: dump every live display object (class, name, x/y, w/h, visible,
# currentFrame/totalFrames, frame label), recursively
peptide ssf2 tell "tree 3"          # depth 3 first; deepen into subtrees you care about
```

read the whole dump. typical structure: root backdrop, stageMC (background plane, terrain MC
with spawns/ledges/boundaries/characters/ENEMIES, shadowMask, foreground plane), weather MC,
HUD. everything the stage spawned at runtime is IN this tree with its live position.

targeted probes (the cursor walk; each GET moves the cursor, so re-navigate per read):

```bash
peptide ssf2 tell "e ROOT"                       # cursor = document root
peptide ssf2 tell "e CALL1 getChildAt 2"         # descend by child index (from the tree dump)
peptide ssf2 tell "e GET x"                      # cursor = the property
peptide ssf2 tell "e READ"                       # print it
```

measure with this:
- **anchors**: the player's standing y (`e match.getCharacter(0).Y`), ledge instances, spawn
  markers. these calibrate the coordinate model (phase 4) and the real floor heights.
- **runtime hazard geometry**: navigate into the live hazard clip, read its position and its
  attackBox child's x/y/width/height plus the inner shape's native size. this is the value
  your static derivation must match.
- **velocities**: sample one object's y every ~0.5s, several times. consistent deltas = the
  rate; large opposite-sign jumps = wrap/respawn. compute px/frame at 30fps.
- **counts**: particle/instance counts (weather embers etc.) are right there in the tree.

## phase 4: the coordinate model

three spaces. get this wrong and every position is wrong:

- **game space** = the `terrain` MC's local space. the stage's own AS3 (spawnEnemy + setX/setY
  literals) and the engine's runtime X/Y fields are terrain-local.
- **world space** = the SWF root timeline. `world = terrain_local + terrain_registration`
  (pure translation, scale 1; verify with the ledge anchors).
- **FM space**: `FM = (terrain_local + terrain_off) x size_multiplier`, where
  `terrain_off = terrain registration - stageMC registration`. the parser records this
  statically (`WalkAnchors` in `stage_parser.rs`) and exposes it; AS3 actor coords and
  metadata `sink_columns` go through it automatically.

verification: convert the player's measured standing y to FM and compare with the emitted
floor line. agreement within ~1px = model holds. (AABB tops and registration points differ;
when a number is ~5px off, you're mixing them.)

beware the **parked frame-0 trap**: a hazard clip placed on the timeline at an offscreen
position (below the pit, above the sky) is PARKED; the engine moves it at runtime. its static
AABB gives you the correct SIZE but a wrong POSITION. position comes from the AS3 spawn
literals (game space) or a live measurement, never from the parked placement.

## phase 5: where ported data goes

- **per-stage facts** -> `mappings/stage/metadata.jsonc` with provenance comments:
  `platforms` (deck surfaces; y from the ledge line), `hazard_floors` (standable molten
  lakes: real collision that never anchors the main floor), `sink_columns` (hazard target
  columns in GAME coords, verbatim from the update disasm), `keep_foreground`, hazard
  overrides only when auto-detection is wrong.
- **`hazards` with `game_coords: true`** -> the way to place a hazard whose position is a
  static AS3 literal (a `spawnEnemy(X)` + `setX/setY` in the stage `update`) but isn't in the
  placement tree, so auto-detection misses or mis-places it. give x/y/w/h in the verbatim SSF2
  game (terrain-local) numbers -- the raw `setX/setY` + `getOwnStats` width/height -- and the
  converter runs them through the stage's static `terrain_off` + `scale` (the same transform as
  `sink_columns`). kingdom1's two piranha plants are the worked example: `(74,74)` + `(670,244)`
  game -> on-stage FM, replacing an auto-detect that stacked both at one wrong spot. confirm the
  result with `PEPTIDE_STAGE_DEBUG=1 ... --info` and check the emitted `hazard:` line lands
  inside the camera box. use this ONLY for stationary hazards; a walk-in / flying actor's
  in-play position is NOT its spawn literal (that's a live read, phase 3).
- **universal mechanisms** -> parser/emitter. existing ones you should reuse, not rebuild:
  - actor coords + `clip_attack_box` place a hazard hitbox exactly (static, generic).
  - orphan-pair classification splits standable structures into bg layer + fg occluder.
  - the multi-animation hazard CGO (frame-label clips -> one FM animation per label, HIT_BOX
    on the damaging labels, Substate switching).
  - per-layer span filling (static layers stretch, short cycles tile) in the entity builder.
  - independent-loop backdrop elements (`PEPTIDE_BG_ELEMENTS`, prototype, off by default): a
    single FM stage animation gives every baked layer ONE shared master clock, so a long element
    loop tiles to the master length and a non-divisor loop phase-jumps each restart (Flash nested
    movieclips loop independently of the parent; you also can't put two distinct objects on one
    layer+frame). the flag promotes each ANIMATED backdrop element to its own CUSTOM_GAME_OBJECT
    whose `gameObjectIdle` animation is just that element's frames on LOOP (its own clock), and the
    stage reparents its view into a background CONTAINER for depth:
    `var e = match.createCustomGameObject(getContent(eid), null);
    self.getBackgroundBehindContainer().addChild(e.getViewRootContainer());`
    each distinct SSF2 backdrop symbol becomes its OWN entity (grouped by symbol id in
    stage_parser `group_bg_layers`), so the elements are NOT merged into one composite image --
    bowserscastle yields 5 (BowserSpectator, Torches Lit, Torchembers, Podoboos, Bubbles).
    the depth control is the Stage container API. there is NO generic background container; the
    background is four sub-bands (back to front): `getBackgroundBehindContainer()` (deepest, in
    front of the painted backdrop), `getBackgroundEffectsContainer()`, `getBackgroundShadows
    Container()`, `getBackgroundStructuresContainer()` (static structures, just behind characters),
    then `getCharactersBack/Characters/CharactersFrontContainer()` and `getForeground*Container()`
    (each returns a `Container`), plus `Entity.getViewRootContainer()` and
    `Container.addChild(DisplayObject)`. use BACKGROUND_BEHIND for scenery (the default); a plain
    `createVfx` with VfxLayer drew at the wrong depth, and BACKGROUND_EFFECTS is for particle fx,
    not scenery. null owner: createCustomGameObject's owner is optional, and a stage's `self` is a
    StageApi (not the GameObject the owner expects), so pass null. LIVE-VERIFIED on bowserscastle:
    the 5 elements (Bubbles 134f, the non-divisor of the 284 master) loop on their own clock,
    removed from the baked entity (no double-render), zero stage script errors, rendering behind
    the fighters and in front of the painted background (window capture confirmed). remaining
    tuning: per-element band (embers/bubbles could go to EFFECTS, torches/Bowser to BEHIND).
- **behavior scripts** (`stage_emit.rs` script generators): port the disasm'd state machine
  1:1 with the unit table above. comment each constant with its SSF2 source.

## phase 6: emitted-output checks (before touching the engine)

```bash
peptide ssf2 stage <stage.ssf> --out stages
```

- hazard lines in the output: position/size/damage match your constants table.
- every IMAGE layer spans the full stage animation (a short layer goes blank or freezes for
  the rest of each loop). check with: sum each layer's keyframe lengths in the `.entity` and
  compare to the max; all must be equal.
- multi-anim hazard entity: one animation per frame label, HIT_BOX layers only on damaging
  labels, AnimationStats LOOP for cyclic anims and NONE for one-shot entrances.

## phase 7: live FM verification (the only phase that counts)

```bash
# the .fra the engine loads is built by FrayTools, NOT the CLI. always re-export:
peptide export --project "$(pwd)/stages/<id>/<id>.fraytools"
FRAY_STAGE=<id> peptide session --char mario,mario
```

- **the log is the canonical signal** (`~/.peptide/session/out.log`): zero SCRIPTERR from the
  stage/hazard scripts, then watch the ANIM stream. hazard hits show up as HURT_*/TUMBLE/KO
  on idle fighters. watch at least 2 full hazard cycles before concluding anything.
- **zero events over multiple cycles is a finding, not luck**: the most common cause is the
  engine culling a game object parked outside the blast zone (SSF2 hazards use
  surviveDeathBounds for exactly that spot; park FM hazards just INSIDE the top bound,
  above the camera ceiling so it looks identical).
- **visuals**: plain `screencapture -x` only sees the active macOS Space and the game window
  often opens elsewhere. capture by window id: a tiny swift helper around
  `CGWindowListCopyWindowInfo(.optionAll)` lists ids for owner "hl" (FM) and "SSF2";
  `screencapture -x -o -l<id> out.png` grabs that window on any Space. boot BOTH engines on
  the same stage and compare side by side.
- **cross-frame state in CGO scripts must use `self.makeInt/makeFloat/makeBool` +
  `.get()/.set()`.** a plain `var` re-initializes every frame and the object silently
  freezes. this is the most expensive bug in the whole pipeline; there's a regression test,
  keep it passing.

## phase 8: regression gates

```bash
# all 110 stages must still convert (the universal-mechanism proof)
for f in ../ssf2-ssfs/stages/*.ssf; do peptide ssf2 stage "$f" --out /tmp/corpus || echo "FAIL $f"; done
cargo test --workspace && cargo clippy --workspace -- -D warnings
```

## the per-stage completion checklist

- [ ] phase 1 inventory table: every linkage/instance/orphan explained
- [ ] phase 2 constants table: every actor's 4 methods + slots read, units converted
- [ ] phase 3 tree dump saved; anchors, hazard geometry, velocities, counts measured
- [ ] coordinate model verified against a live anchor (<= ~1px)
- [ ] metadata entries carry provenance comments
- [ ] emitted checks pass (hazard table, layer spans, hazard entity shape)
- [ ] live FM: no script errors, hazard cycle observed >= 2 periods, hits land, side-by-side
      capture vs live SSF2
- [ ] corpus sweep PASS=all, tests + clippy green
- [ ] gap report updated: every unported behavior listed WITH its measured/disasm'd spec, so
      porting it later needs no re-research

## known engine-behavior gotchas (hard-won; don't relearn these)

- plain `var` in a game-object script resets every frame; use `self.make*` state.
- a game object outside the blast zone is culled; park hazards inside the top bound.
- a layer whose keyframes end before the stage animation goes blank/frozen for the rest of
  each loop; span every layer.
- one stage animation = one master loop. SSF2 movieclips loop independently, so non-divisor
  cycle lengths phase-jump at each master restart. layers must tile; the residual jump is an
  engine limit, document it per stage.
- SSF2 collision boxes are 50x50 shapes drawn CENTERED on their registration and scaled by
  the placement matrix; treat box position as center +- half-extent.
- molten lakes (lava/acid floors) are STANDABLE terrain in SSF2; the damage comes from a
  hitbox volume above the surface. port them as hazard floors, not holes.
- the first boot's window may land on a different macOS Space; capture by window id.
