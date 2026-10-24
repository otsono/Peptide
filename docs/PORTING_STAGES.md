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
4. **never invent a CATEGORY either.** rule 1 covers made-up numbers; this covers made-up
   kinds. the converter never needs to know WHAT an object is, only what its code says to do,
   because "what is it" has no answer for content nobody has written yet. so no keyword lists,
   no `HazardKind`, no motion categories, and no converter-side template that only one piece of
   content fits. reading the numbers honestly does not save a template: the test is whether
   content nobody has written yet would flow through the path at all.
5. **every fix must be universal.** before changing parser/emitter code for your stage, ask what
   it does to content that does not exist yet, then prove it on the ten-stage sample (`just
   sweep-stages`). do NOT convert the whole corpus; scan it with `--info` if you need to know how
   widespread something is.
6. **convert-time success is not success.** a stage that emits cleanly can still be silently
   broken in-engine (dead hazard, frozen layer, culled object). only phase 7 counts.

## what a port actually IS

an SSF2 stage is not a description of some scenery plus a list of hazards. it is a package of
CLASSES that run: the stage class has its own `initialize`/`update`, and every object it spawns
has its own. the port is a TRANSLATION of those classes onto Fraymakers' object model, and the
converter's job ends there. it does not identify content, and it has no opinion about what a
thwomp is.

the model, end to end:

  the stage's own update()  ->  the stage entity's script (spawns, timers, grid state, phases)
  each object's own class   ->  one CUSTOM_GAME_OBJECT per class, running its own translated code
  the labels it forceAttacks->  one FM animation per SSF2 frame label, switched by its own state
  what it declares          ->  size from getOwnStats, hit from getAttackStats, boxes from its clip

bomb factory is the worked example: the stage class runs the spawn loop and owns the grid, each
block runs its own class (a local state machine over its own labels), and the converter contains
nothing that knows what a bomb is. that is the shape every stage object should reach.

**scaffolding vs architecture.** anything in the converter that encodes a specific object's
BEHAVIOUR is scaffolding from before that object could run its own code: the thwomp faller
template, `motion` categories, `FallerCycle`, `hazard_kind`. these are not to be extended, tuned,
or given better selectors. they are to be made unreachable by porting the class, and then
deleted. extending one is the mistake this section exists to prevent: the values inside the
thwomp template were stepped honestly out of the disasm, and it is STILL wrong, because no
object anyone writes later is shaped like it.

**the order is not optional.** make the class's own code run first; the scaffolding then has no
callers and deletes itself. going the other way (delete the selector, then find something to
replace it with) forces you to invent a new category, which is the same bug wearing a new name.

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
# ...prints, among others: [art-frame] (what the art walk kept per sample), [plane-only] (what
# stays painted after the split-out pieces leave), [hz-clip-search] (an object's clip candidates)

# a stage-script line through the emitter's passes (needs --out: the passes run when emitting):
# every line holding the text, as it stands after each pass, and the span + written set the
# unwritten-slot pass judged it in; PEPTIDE_DUMP_PREPASS writes the text that pass received
PEPTIDE_TRACE_LINE=leftSecond PEPTIDE_DUMP_PREPASS=/tmp/pre.hx peptide ssf2 stage <stage.ssf> --out /tmp/x

# the full placement tree (every PlaceObject with name/symbol/plane/position, indented)
PEPTIDE_STAGE_TREE=1 PEPTIDE_STAGE_DEBUG=1 peptide ssf2 stage <stage.ssf> --info

# orphan bitmaps: library bitmaps placed by NO shape/sprite/root tag. the engine instantiates
# these by id as named bg children (standable bridges, fg/bg structure splits). same-size
# orphan PAIRS are usually a background piece (solid deck) + a foreground occluder (deck cut
# out, drawn in front of the fighter)
PEPTIDE_ORPHAN_DEBUG=1 PEPTIDE_ORPHAN_DUMP=1 peptide ssf2 stage <stage.ssf> --info
# (dumps each candidate to /tmp/orphan_<id>.png; LOOK at them)

# the package's tags: pictures, morphs, picture-filled shapes, direct picture placements
peptide ssf2 tags <stage.ssf>
# ...and one clip child by child: what it places at each depth, with each shape's edges and
# the fill on either side (a collision clip read without converting the stage)
peptide ssf2 tags <stage.ssf> --sprite <id>
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
  statically (`WalkAnchors` in `stage_parser.rs`) and exposes it; AS3 actor coords and a
  class's own spawn columns go through it automatically.
- **the scripts' origin is the terrain's.** every `__sx`/`__isx` in a stage's or object's script
  converts through the terrain registration whenever the source has a terrain clip (the plan's
  `origin` then equals `fighter_origin`). the places the stage's own code states only ever agree
  with it, and an object the parser could not place says nothing true (saffron city's pokemon
  spawner has no art and sat at 0, which put every object's sums 700 px off the terrain). the
  median of stated places is for a stage with no terrain clip; the art's origin for one that
  states nothing.
- **a clip group's place is in its plane's space.** the source writes a clip's `x` in the space
  of the plane that holds it, and a script's number comes in converted as a terrain-local one,
  so the group keeps its place shifted by the two origins' distance (`g.ox/g.oy`: the plane's
  registration less `__sx(0)`). saffron city's platform class puts its art at its body less 92
  in the background plane. the group goes by the SOURCE's name (an alias is the script's) so it
  finds the box the tree recorded for it.
- **a floor profile across the stage.** `tools/floor-profile.py <stage>` drops a fighter at
  every x on both engines and prints where it rests in the source's units, so a missing roof, a
  wall that ejects on one side only or a platform a few px off shows up as a row. a rest above
  the drop is a respawn (the fighter fell off) and a moving platform's column depends on when it
  was dropped, so read the rows against the plan, not as a pass/fail.

verification: convert the player's measured standing y to FM and compare with the emitted
floor line. agreement within ~1px = model holds. (AABB tops and registration points differ;
when a number is ~5px off, you're mixing them.)

**a script keeps the source's units, and only the engine boundary converts.** every literal,
table and step in a stage's or an object's code is in the source's units, so the code keeps
them: a place read off the engine comes in through `__isx`/`__isy`, a place written to it goes
out through `__sx`/`__sy`, and nothing in between sees the engine's numbers. that holds for
the stage script and every object script alike, and for the groups and regions the port keeps
for the source's clips (a field read or write on one is an engine read or write). judging each
value instead (a literal converts, a read does not) left the two mixed: central highway stepped
an engine read by a source literal, and palace of twilight handed engine reads to an object
that counted them as source numbers. a rectangle helper (`__childRect`, `__boundsRect`, a
region's `width`) answers in the source's units for the same reason.

beware the **parked frame-0 trap**: a hazard clip placed on the timeline at an offscreen
position (below the pit, above the sky) is PARKED; the engine moves it at runtime. its static
AABB gives you the correct SIZE but a wrong POSITION. position comes from the AS3 spawn
literals (game space) or a live measurement, never from the parked placement.

## phase 5: where ported data goes

- **per-stage facts** -> `mappings/stage/metadata.jsonc` holds a stage's display name, series,
  music and `keep_foreground`, nothing else. an object's size, hit, position, animations and
  what counts as a floor are read from the stage's own classes; a stage the class path gets
  wrong is fixed in the class path, never with a per-stage number.
- **universal mechanisms** -> parser/emitter. existing ones you should reuse, not rebuild:
  - actor coords + `clip_attack_box` place a hazard hitbox exactly (static, generic).
  - orphan-pair classification splits standable structures into bg layer + fg occluder.
  - the multi-animation hazard CGO (frame-label clips -> one FM animation per label, HIT_BOX
    on the damaging labels, Substate switching).
  - per-layer span filling (static layers stretch, short cycles tile) in the entity builder.
  - independent-loop backdrop elements (AUTOMATIC, every stage): a single FM stage animation gives
    every baked layer ONE shared master clock, so a long element loop tiles to the master length
    and a non-divisor loop phase-jumps each restart (Flash nested movieclips loop independently of
    the parent; you also can't put two distinct objects on one layer+frame). the converter
    UNIVERSALLY promotes each ANIMATED backdrop element (weather embers, jumping podoboos, lava
    bubbles/splashes, flickering torches, a spectating Bowser, …) to its own looping VFX -- there's
    no flag, any animated bg element on any stage is treated this way; static (1-frame) elements
    stay baked. `PEPTIDE_BG_INLINE=1` forces the old all-baked behavior for diffing. each element
    is a VFX content (objectType "VFX", no scripts/stats/manifest, resolved by id via getContent)
    spawned + depth-placed by the stage Script:
    `var v = match.createVfx(new VfxStats({ spriteContent: getContent(eid), animation: "active",
    loop: true, timeout: -1, relativeWith: false }));
    self.getBackgroundBehindContainer().addChild(v.getSprite());`
    each distinct SSF2 PLACEMENT becomes its OWN VFX, not each symbol: `group_bg_layers` keys by
    (symbol, instance anchor = the world position of the nearest named-MC ancestor, recorded as
    `Instance::inst_anchor` during the walk), so 16 torch-ember emitters placed around the castle
    become 16 separate VFX at their own positions (each a small per-placement sprite), NOT one
    merged union-bounds image. bowserscastle yields 20 (16 embers + BowserSpectator, Torches Lit,
    Podoboos, Bubbles); dreamland 1, casinonightzone 7, smashville 15.
    the depth control is the Stage container API: spawn the VFX, then `addChild` its `getSprite()`
    (a DisplayObject) into a stage container. there is NO generic background container; the
    background is four sub-bands (back to front): `getBackgroundBehindContainer()` (deepest, in
    front of the painted backdrop -- the scenery default), `getBackgroundEffectsContainer()`,
    `getBackgroundShadowsContainer()`, `getBackgroundStructuresContainer()` (just behind
    characters), then `getCharactersBack/Characters/CharactersFrontContainer()` and
    `getForeground*Container()`. a plain VfxLayer drew at the wrong depth, so the explicit container
    reparent is authoritative (no VfxLayer, no owner). LIVE-VERIFIED on bowserscastle: the elements
    (Bubbles 134f, the non-divisor of the 284 master; 16 embers at their wall positions) loop on
    their own clocks, removed from the baked entity (no double-render), zero stage script errors,
    rendering behind the fighters and in front of the painted background (window capture confirmed).
    remaining tuning: per-element band (embers/bubbles could go to EFFECTS, scenery to BEHIND); the
    runtime AS3 weather classes (EmberWeather/RainWeather/SnowWeather) are camera-space and NOT in
    the static tree, so they're a separate port, not caught here.
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

the gate for this phase is `tools/verify.py stage <id>` (TESTING "the verify gates"): it
publishes, boots and holds the running stage to the plan the converter wrote, by what the
stage's own script reports it made. everything below is how to look closer when a gate fails.

```bash
# the .fra the engine loads is built by FrayTools, NOT the CLI. always re-export:
peptide export --project "$(pwd)/stages/<id>/<id>.fraytools"
FRAY_STAGE=<id> peptide session --char mario,mario
```

- **a native death is named in the log**: `[session] the game ended: killed by signal 11` (or
  `exit code N`) follows `engine gone`, and what the engine itself printed is in
  `peptide-engine.log` beside the game. `PEPTIDE_ENGINE_DEBUGGER=1 peptide session ...` boots
  the engine under lldb so a crash leaves a backtrace in that file (developer mode has to be
  on: `sudo DevToolsSecurity -enable`, once, by a human; otherwise lldb waits on a dialog).
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
just verify                              # the ten-stage, ten-fighter sample, every gate
cargo test --workspace && cargo clippy --workspace -- -D warnings
tools/verify.py corpus --stages          # every stage, when a change touches them all (hours)
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
- a terrain child says what it is in its own frame script (`self.type = "terrain"` or
  `"platform"`); the converter reads that, ahead of any instance or symbol name, so a platform
  placed as `start1` is still a platform. a solid's collision is its OUTLINE, every edge typed by
  the side the format says is filled (floor, ceiling, left or right wall): a level drawn as one
  hollow polygon has surfaces all through it that no top line and box could carry. `peptide ssf2
  tags <file>` lists a package's pictures, morphs and direct picture placements (a Bitmap child
  with no shape between, which the walk gives a shape of its own).
- a stage object that extends another class of its own package is ported with that class:
  the base's fields, setup and methods come along, an overriding method's call up to the base
  reaches the base's version, and the chain stops at the package's api layer (the shims that
  forward to the engine), which is the engine's and not the author's. a rectangle a class reads
  off its own clip is the child's box measured at conversion, at the object's live position.
- a stage object's projectiles come from its own `getProjectileStats`: the clip by the declared
  linkage, the launch and spawn offsets off the declaration, shipped through the same generator a
  fighter's projectiles use, under the stage's id. a projectile clip with labels is one animation
  per label, and its frame scripts say which one it flies in and which one ends it.
- a collision clip placed under a class is a BODY, and its collision rides the object: the
  clip's own outline (a solid's every edge, a soft platform's floor alone; the plate the clip
  holds as its `ground` when it holds one, the saucer drawn round it being art), built as the
  object's faces where the object stands, never a second copy left in the stage at the boot
  placement (a fighter stood in mid air on that copy once the platform had floated off). the
  bodies stand from the stage's first frame, ahead of the fighters, and a fighter the engine put
  down inside one (its start placement reads only the stage's own collision) is stood on it.
  what a placed object draws is the clip it was placed as, less its collision: a clip holding
  its plate as `ground` draws the rest of itself (the saucer), and a clip that is the collision
  draws nothing of its own, since a collision shape is never drawn. hiding goes by CLASS, never
  by the placement name: a terrain plate placed as `cliff` hides itself, and the cliff's art
  placed as `cliff` in another plane does not; a clip that hides a named child on its first
  frame (`self.ground.visible = false`) hides that child too. a clip's frame-relative jump
  (`gotoAndPlay(currentFrame - 1)` on its second frame) is a hold on its first segment, not a
  run into the next label. a
  body that declares no hit has no hit box: one that deals nothing still puts a fighter landing
  on it into hitstun. an object asking whether a fighter stands on it reads the engine's own
  floor under that fighter against its own faces. a start position a class eases back to is kept
  in the units the class does its sums in.
- a surface the stage's own code reaches by name (`getPlatform("starshipTerrain")`, then
  `setFallthrough`) is the entity's line segments filed under that name: the script's
  `__objectNamed` answers with a handle that turns all of them on and off together, so a stage
  that keeps every section's ground in place and makes one solid at a time works. two surfaces
  the stage names differently are never merged, however they overlap.
- a clip that holds its collision as a `ground` child collides with that child alone: the role
  it declares reaches the plate and nothing else under it, which is art (central highway's
  pillars are drawn a hundred px taller than the plate). a surface handle answers the ignore
  list too: `addIgnoreObject` is the engine's blacklist, and an inversed list its whitelist
  (crystal smash's starting platform holds one player up), and every other call the stage
  makes on such a handle is answered as nothing. a variable filled from a name a clip lacks is
  nothing everywhere after, so the writes through it are struck with the line that filled it.
- a static the class computes in its own static initializer (a list of spawn points) is a
  script value, base-first; `peptide ssf2 decompile <Class> cinit --swf <file.ssf>` shows that
  initializer as the decompiler reads it, and `--asm` prints any method as its instructions
  instead, for a branch the decompiler leaves out (`/* ? */`, an empty block).
- the same surface reached by its bare name (`if (barrierL != null)`) is the same handle, and
  one being assigned is the class's own field. a real object the tree placed under a name
  (DAT61's barriers) keeps its ignore list on its published deck, the face the engine collides
  with, and the script keeps that list too so only what was added is taken off. a name nothing
  answers to, reached as an object (a clip folded into flat art whose visibility the class
  toggles: smashville's musicians), is answered by a stub that carries the region's place and
  does nothing; an existence test on it still sees nothing. an ask of an object's own clip
  (`platform.getMC().gotoAndPlay(..)`) goes to what draws for it: a surface's or a group's own
  view answers, and a real object is asked through its mailbox. a clip group answers its clock
  (the label and frame its pieces publish). a class the stage builds for itself keeps its
  getters on its object, and the plane helpers are emitted whenever the stage's script uses
  one, named clip or none.
- an ask of an object that already has its mailbox lands in it at once, and the object reads it
  at its next update, which comes after the stage's the way the source's own updates follow
  the stage's; only an object without a mailbox yet waits on the outbox. the stage delivers
  what its setup asked before any object's first step and marks every object it named as set
  up, and a placed object takes no step before that mark (the source's stage constructor runs
  before any frame). a place written onto something the script holds (`m_backgroundSprite.x`)
  goes to a point's field or an object's setter, in that engine's pixels. a class fetched by
  name that nothing answers to is nothing, and so is what is built from it; a clip that is not
  here, read in a condition, has the properties of nothing shown (alpha 1, visible false,
  place 0). a clip under a named clip (`globox.eye1.eye`) is a piece of the named thing. an
  object whose own spawn point lies outside the blast zone survives it (yoshi's fly guy flies
  in from above). a named region answers a clock of nothing.
- two surfaces are merged as one only when they share a top edge and neither is a traced solid:
  a no-drop platform laid over a terrain's box is a second floor (crateria's fighters start on
  one, a hundred and fifty px above the ground). a clip with a timeline of its own is art
  whatever it was named (butter building's `seg2platforms` is the layout's picture).
- a clip that hides itself on its first frame and carries labels is a state machine the stage
  shows by playing a label on it: it ships as its segments, starts unseen, and appears the first
  time the stage plays one (crateria's caution sign, chaos shrine's spirits). the stage reaches
  such a clip through the handle it keeps for it (`m_cautionMC = getBackground().caution`) or by
  a computed name (`getBackground()["spirit" + n]`, answered by `__groupNamed`), and a group
  reports whether it is shown.
- a slot declared with a type and no value starts at the type's zero (a counter declared `int`
  is 0, not null: dream land's whispy counts fighters from it every idle frame; the stage
  class's own fields too, carried as `name=0` on its field list: mushroom kingdom 3's phase
  counter read as null matched no phase and the stage never left its first), and a name
  declared twice at script level keeps one declaration, carrying the stated value. a flag a
  stage keeps on another object (`fighter.setGlobalVariable("hitWoodenPanel", false)`) lives in
  that object's exports. a clip with no labels whose script stops it holds where it stops.
- a plate held as a `ground` child answers to the name of the clip that holds it, which is the
  name the stage's code asks for it by (hylian skies swaps `lightTerrain` and `darkTerrain`).
  an object draws with the clip its class names for itself (`getOwnStats().linkage_id`),
  ahead of any label search, whether or not that clip is also placed on the timeline or worn
  by another object of the same class (emerald cave's five wooden panels share `ecave_wood`,
  and it carries every `ecave_wood_*` label the stage hands them plus the `_break` ones).
- a clip's clock is read the way the source reads it: `X.currentFrame` / `X.currentLabel` /
  `X.totalFrames` become `__currentFrame(X)` and friends, which answer for whatever X became
  here. a group of pieces answers with the clock its pieces publish in their exports (source
  frame count, raw label); the stage's own plane (`getMidground()`) is the stage's timeline,
  read against its label marks; an object answers with its own frames halved. a frame NUMBER
  played on a group (`gotoAndPlay(3)`) lands each piece on the segment holding that frame.
- a placed instance is its own object: one class placed seven times is seven objects, each
  with its own clip and its own body (palace of twilight's three platforms and four pillars
  share `TwilightObject`; emitted once per class, every one carried the pillar's box). an
  object the code spawns by class keeps the class's name, which is what the spawn asks for.
- an object's own step runs at the source's rate: `update` is called sixty times a second here
  and the source stepped its objects thirty, so a class carrying its own motion
  (`position.y += speed`, jungle island's barrels) ran double. the class's step runs every
  other engine frame and its timers are made with the source's counts; only the spawn wrapper
  (a recycled object) steps every frame with doubled timers.
- a place an object writes for itself lands when its step is done, and the last write of the
  step wins: measured live, a class that sets its place and reads it back in the same step
  reads the old place. and a placed object cannot be moved by an ask made before the match is
  live (the stage's own setup): the barrels are asked to go to a path's start during setup,
  stay where the tree put them, and walk the path from there, on both engines.
- a stage that lays its objects out by playing a label on a plane moves the placed clips with
  the timeline (`layout1`..`layout4`), so `__planesPlayFrameLabel` moves each placed object to
  where the timeline puts it at that label (`Hazard.placements`, read off the placing sprite's
  frames), and the stage's read-back of their positions sees the layout.
- a surface parsed off an object's own collision child is that object's body, not stage
  terrain: as wide as the body's plate, at its x, inside the object's box (peach's castle's
  rising platforms held their fighters on a static copy 7 px over the deck and never felt
  anyone stand on them). the terrain keeps nothing an object carries.
- a plane asked for as a whole (`stadium = getMidground()`, whatever the stage is called by)
  is an object of its children by name that also answers the plane's own asks: a label played
  on it is the stage's label player, its clock is the stage's timeline. a reach INTO a plane
  for a child the port lacks still reads as nothing.
- a factory field the factory never gives (`it.stadiumScreen`, filled only from a name a clip
  lacks) is nothing everywhere after, inside the factory's own methods too; the pass runs until
  it finds nothing more, since striking the fill is what makes the field read as never given.
- a global RegExp replace of one literal spelling by another is `__replaceAll`: this engine's
  script has no RegExp.
- decompiler: a compare that jumps forward over the code run on failure, where that code flows
  on into the jump's target, has its merge AT the target: `if (colour == null) { pick one }`
  then the setup. read as the then-body, the setup ran only for a named colour (nintendo 3ds's
  timers were never made). and a member or call on an operator expression keeps its brackets:
  `(i + 1).toString()`.
- every shape of a terrain clip collides in the source, the hidden plate and the visible
  drawing alike: pokemon colosseum's fighters stand on the bleacher steps of its painted stands
  (measured live, step by step). the collision walk reads each clip's DISPLAY LIST at the frame
  its own script holds on (a `stop()`, a `gotoAndStop`; else its first frame; the root its final
  state): a later frame placing over a depth is the clip's next phase, not more collision
  (pokemon's terrain runs 779 frames of art, and every frame's placement read as one list made
  nine hundred shapes). the outline chains edges into whole contours and simplifies them BEFORE
  typing floor and wall runs; a closed contour is simplified as two halves, since against a
  point-sized chord the simplifier keeps nothing (jungle island's floor vanished into raster
  bands that put it fifty px high). a stage with six hundred structures runs the engine hot,
  and the boot gate calls that a spin only when the frames stop coming.
- a platform is its planks: a clip the source declares (or names) a platform lands a fighter
  where its shape draws a floor, each floor run of its outline a surface of its own at its
  own height and span, dropped through or not as it declares, and never a block with walls
  (lunar core's four pit bridges, boxed by their bounds, closed the pit and stood a fighter
  over its bottom). a shape with no floor run keeps its box's top.
- the stand gate brings both engines to the same moment of the match before each drop: a
  travelling platform is wherever its cycle has reached (fourside's sat at opposite ends of
  its run in the two engines). fraymakers answers its match frame count; the source engine
  has no counter to ask, so its frames are accrued from wall time at its native rate and speed
  since its match spawned.
- the stand gate confirms a difference from twice the lift before it counts: an entrance
  under a platform (lunar core's sits 128 px below one) starts the first drop inside the
  plate, where the source pushes its fighter up and this engine lets it fall.
- a picture past 2048 px on a side ships downsampled by the smallest whole factor that fits
  and is drawn back up by its placement scale (an eleven-frame 2670 x 2209 shade ran the match
  at three frames a second).
- a fighter is stood on a placed body's deck at boot only when its ENTRANCE sits over that
  deck (the engine's start placement reads static collision alone and drops it to the ground
  beneath). an entrance under a body (jungle island's barrels hang over the start points)
  starts on the ground, as the source does.
- a clip group knows where it is (the clip's own origin where the tree states one, else its
  named box) and moves as one: a write to its `x`/`y` becomes a move of every piece by the same
  amount (urban champion's hole, race to the finish's shadow). a step from where it already is
  (`hole.x = hole.x + 900`) scales only the step (`__sd`), never the place again.
- a piece binds to every instance name over it, outermost first: the stage reaches the same
  picture as `movingplatLsprite` and as `movingplatLsprite.sprite`, and two platforms' holders
  both called `sprite` inside are told apart by the name outside (crystal smash 2). an ask made
  of a child of the named clip goes to the group.
- a frame number sent to a piece with no script of its own (the source's `gotoAndStop(n)`) holds
  that frame, at two engine frames per source frame; a frame off the start is the first.
- a field of an object's class is a per-frame speed only when the class ACCUMULATES it (adds
  it to the object's own position, or to a position it keeps: crateria's `position.x =
  position.x + xSpeed`); a table of stops added to a rest position read once (dracula's
  castle's `pos[0] + platformStops[..]`) is so many source pixels, converted at the position
  boundary and never scaled as a speed. a list field the class tests against null (`pos ==
  null`) starts null, since the class is what fills it; one it only indexes starts empty.
- a clip the stage's CODE makes from the library at run time (`getMCByLinkageName("x")`, put
  in a plane's container and moved by hand: fairy glade's butterflies) is rasterised on its own
  timeline (`StageArtSet::runtime_clips`) and shipped as a vfx entity the script makes on demand
  (`__libClip`), held by a handle the script moves by its fields in the source's units, flips
  about the clip's origin, reads the frame of, and takes down (`__addTo`/`__removeFrom`, the
  plane's own container; put into a clip the port keeps, `bg.addChild(star)`, it is placed from
  that clip's origin in that clip's plane). a chain of clip names (`globox.eye1.eye`) is the name whose pieces the
  others all hold. a clip group flips about its origin (`setScaleX`, each piece's width known).
- an object that shapes its body from a clip named as it runs (`createSelfPlatformWithMC(
  getMCByLinkageName("terrain_" + vehicleName))`, sand ocean's eleven vehicles) ships the
  measured shape of every clip under that literal prefix and picks by name at run time; the
  handle the source keeps to the body is its deck.
- a child of a clip the port has no piece for (`giygasTextBox.giygasSpeech`, the text field a
  class writes its dialogue into) is asked through `__childOf`: the group's own member when it
  has one, else a stand-in that takes the class's writes, so the script runs past text this
  engine has no field for. the planes' clips are reached as handles before any chain is read.
- a platform clip that declares a bounce (`self.bounce_speed = 25; self.bounce_frame =
  "bounce"` on its first frame, read by the game's platform layer) stands up as an object of
  nobody's class: its body is the clip's `ground` child, else the clip's own box; its art the
  clip's labels when it has them (a clip with none keeps its picture in the plane and its body
  all the same), and a fighter landing on it is sent up at the declared speed while the object
  plays the declared label and returns to its rest label when that has played (desk's stapler,
  mushroom kingdom 3's note block, three target tests). the clip's own box read as a surface is
  dropped for the body.
- a terrain piece that declares a sideways push (`self.x_influence = -5.9` on its first frame,
  desk's lamp slopes; also crateria, hyrule castle 64, race to the finish, galaxy tours, sand
  ocean) moves a fighter standing in its box by that much a frame in the stage's update
  (`__pushUpdate`), as the game's terrain layer does with the declaration.
- a named surface the plane's timeline moves by label (each named instance's placement per
  label is read off the plane's tags; one that differs anywhere is kept in the plan as
  `placements`) rides its own picture: the stage script moves its structures every frame by
  however far the picture's current frame sits from the boot placement (`__platFollow`, keyed
  by the piece's label and source frame, so the tween the source drew is followed and not only
  its ends). mushroom kingdom 3's castle floor rises 500 px into play at its label, and its top
  floor drops out of play at the first transition.
- a named clip's own frame scripts run: each (by the label it sits under and the frames past
  it) is a script entry the stage fires once when the clip's pieces reach that frame
  (`__planeScripts`). `self.<child>.fallthrough = v` on a child goes to that child's surface
  (its structures' `disabled` stat) or, for a placed object, its body (`exports.__setDisabled`;
  a write that lands before the object's script has run waits for it, since the plane's first
  frame fires before the objects it places have published their bodies); `self.<child>.visible
  = v` goes to the child's clip group or object; `SSF2API.getStage().m()` is the stage script's
  own `m()` when it defines one; an `if` block keeps its shape, with `isHazardsOn()` answered by
  the stage's own flag and a test the stage cannot answer read as false; a hold or jump is the
  pieces' own; anything else (the plane's own sounds) is kept as a comment. this is how
  mushroom kingdom 3 makes one terrain section solid per phase, its transition platform solid
  only between phases, and spawns and kills each section's blocks, chests, plant, lava and
  podobos as the section comes and goes; castle siege, jungle island and pokemon colosseum
  flip their sections the same way.
- an object's own setup runs on its first update, where the tree put it: this engine runs
  initialize inside the create call, before the stage's setX/setY have landed, so a
  constructor that reads its own place (crateria's platform keeps its start as the point it
  eases back to) read the origin and the platform sat at (0, 0) for the whole match. the
  class's constructor and its initialize now run once from `__buildOnce`, first thing in the
  object's update and ahead of any ask in its mailbox; the body's faces are still made at
  initialize and parked until the object moves them.
- a ledge beacon that declares a `syncPlatform` key (`self.syncPlatform = "crateriaPlat"` on
  its first frame) moves with the terrain piece that declares the same key: the key names
  nothing in the tree (crateria's platform carries no such name, live), so it is read as what
  it is, a shared key. the piece sits inside a placed object's clip, and the beacon's side is a
  ledge on that object's body: the body's floor face is built with the flag (`__spLedgeL`,
  `__spLedgeR`; the face type is the engine's enum, never a string, when the flag is tested).
  the key names the thing the beacons stand on three ways: a piece inside a placed object's
  clip (crateria), the object's own placed collision clip (gangplank galleon's boat platform,
  planet namek's cliff), or the object's instance name (sand ocean's big blue); a static
  piece that declares the key carries the flags on its own structure. crateria, jungle
  island, palace of twilight, steel diver, gangplank galleon, planet namek and sand ocean
  carry theirs; dedede arena declares a key on no beacon.
- the frame scripts of the clip an object draws with run on the object, by its own animation
  and frame (`__clipScripts`, each once as the animation reaches the frame), and what they ask
  of the stage crosses as a value on the object's exports (`__stageAsk`) that the stage drains
  every update (`__drainStageAsks`): a call on a method the stage script defines, a named clip
  shown or hidden (`__setVisibleNamed`), a named clip played by label (`__playNamed`); the
  object's own `self.self.m()` is its own script's `m()` when it has one, `self.self.f = v`
  its own state on its exports, `self.self.t.m()` its hoisted timer, `stancePlayFrame("l")`
  and `forceAttack("l")` its own animation, `destroy()` its `kill()`, and the camera calls the
  engine's target calls. an ask goes straight onto the stage's own queue through the handle
  the spawn hands the object (`exports.__stage`), so an ask made on the object's last frame
  outlives it. a label whose scripts neither stop nor send the clip anywhere runs on into
  the label after it (`__clipNext`, played when the animation reaches its last frame, checked
  before the once-per-frame gate since the last frame shares its stamp with the one before);
  the engine's default would loop it. phase 8's acid waterfall plays through `Idle` into
  `loop`, shows the foreground's waterfall clip and plays its `start` on the way, plays `end`
  as it ends and has the stage remove it on its last frame, and the stage spawns the next
  (live, ~3500 frames apart); planet namek's cliff has the stage take the players off its
  ledges.
- a clip named like one of the stage's own fields keeps a handle of its own: the source
  keeps a field on the stage and a child in a plane apart, this script has one scope, and
  pokemon colosseum's helper object `stadiumScreen` overwrote the clip group of that name (the
  helper then asked itself for an animation). a clashing clip's handle is `<name>__clip`
  wherever a handle is made or reached, its string key stays the source's, and a clip group
  carries its child clips as fields under their source names (from the pieces' named paths,
  and a plane holds every clip whose pieces all sit in it), so a chain that only takes its
  final shape late (`grandstand.stadiumScreen`) still lands on the child. a member read
  through a factory's field that holds a clip (`it.stadiumScreen.mainBox`, a text box the port
  has no clip for) goes to the child stand-in, which now answers a clock too (no animation,
  frame 0).
- a backdrop piece loops at the period its pictures repeat on only when its own clip comes
  round on that period: jungle island's background holds a 600-frame clip whose look repeats
  every 16, and the source restarts that look at 600 (no multiple of 16), so the clip's own
  length is the loop, within the sample window. the comparer files a piece under every clip
  above it for that clip's timeline but never counts it among that clip's leftovers.
- a piece that declares its own ground friction (`self.decel_friction = 0.01` with
  `accel_friction`, race to the finish's ice, fourside's spaceship deck) slows whoever stands in
  its box by its numbers: the stage's update sets the fighter's `friction` and walk acceleration
  to the declared values across the acceleration scale while its feet are in the box, and hands
  the fighter's own back on leaving (`__frictionUpdate`). measured: the source's natural stop
  takes its declared decel a source frame off the speed (mario 1.04), the engine takes its
  friction stat off every engine frame, so the two cross by the acceleration scale (0.325);
  on the ice the port loses 0.00325 a frame.
- a clip that asks the game's own platform class to run it (`self.className =
  "DynamicPlatform"`, with `movement1..N`, `startIndex`, `fallthrough`) is read, and the legs
  are kept on its site for the record, but nothing stands it up or moves it on that declaration:
  measured live, flat zone plus's game leaves a fighter standing on such a plate exactly where it
  was for 600 frames. eleven stages declare the class; its picture (`self.foreground =
  "targettestPlatform"`) resolves in no shipped package, the game draws it from its own code.
- `PEPTIDE_FIND_SYM=needle peptide ssf2 stage <file>` lists the symbols holding the needle in
  the package and in every package the game's manifest names: which package ships a clip a
  class names is the question it answers. a named foreground picture a package lacks is
  searched for in those packages and read out of the one that holds it.
- a class the stage spawns is asked for by the id the object shipped under: the class name
  with only its letters and digits (phase 8's `SR388_AcidWaterfall` ships as
  `SR388AcidWaterfall`); asked for by its raw name, the spawn made nothing.
- a field the class reaches by a computed name (`this["block" + i]`) is looked up over the
  script's own fields (`__field(n)`, `__setField(n, v)`, emitted over every field the class
  has when the script indexes itself); read as written it indexed the stage's handle as a
  list and stopped the script. a string index on a table whose keys the port read (jungle
  island's path table) is that table's own lookup (`__get_<NAME>(k)`, arms per key); one on
  any other object (pokemon colosseum's `screen["p" + i + "TextBox"]`) goes through
  `__fieldOf(o, n)` / `__setFieldOf(o, n, v)`, which answer nothing, since this engine throws
  on an index into an object and on a missing dotted field alike, and the truth test the
  source makes on the result guards the read.
- the plan says whether the terrain plane's own timeline moves anything drawn (each of its
  frames walked with the clips inside held on their first frame): sand ocean's ten frames walk
  only its warning boxes, so its picture loops the road inside and the plane itself is compared
  by its children.
- a class's own constants (`SandOceanVehicle.FOUR_CAR_MODE`, what its static initialiser assigns
  a literal to) are read off its reconstructed script and put in place wherever another script
  names them by class.
- an object that asks its own body whether it is off (`currentPlatform.getFallthrough()`) is
  answered from the flag the port keeps when it switches the body (the engine's face has no
  getter); the camera is aimed at the object itself where the source hands it the object's clip.
- a pair the camera is handed (`addTimedTargetPoint(camPoint, 110)`) goes as the engine's own
  point in the stage's px (`__pt`, `Point.create`), with its length doubled like every timing.
- a readiness guard goes only when readiness is all it tests (with `self`, `self.self`,
  `parent` and a `true` beside it); one that tests more keeps the rest of its condition.
- a piece placed INSIDE a placed object's own clip (the object's name with a name below it in
  the piece's path) is that object's picture: it rides the object at the offset the tree gave
  it (`__follow`), and the object reaches it as its own view (`getMC().sprite.gotoAndStop(n)`
  -> `__ownArt`, over the pieces the stage handed it in `exports.__art`). crystal smash 2's
  platforms show their holder by their own height. a clip that only shares the object's name
  in another plane (planet namek's cliff art, shown and played by label) stays where it is.
- an object that declares no size and carries no hit box is the size of the clip it names, and
  one that draws nothing measurable ships at a point: its code is what it is (race to the
  finish's damage zones size themselves at runtime). a class no code spawns and the tree does
  not place is a base other classes extend, or unused, and is not stood up parked at boot.
- a piece of the stage plane that moves, that its class hides until the stage shows it, or that
  the stage plays by label is an object of its own and is NOT painted into the plane. a plane
  whose every piece left paints nothing (galaxy tours' terrains: painted, they made the plane
  387 full-size frames, and the engine died on the textures).
- the first boot's window may land on a different macOS Space; capture by window id.
