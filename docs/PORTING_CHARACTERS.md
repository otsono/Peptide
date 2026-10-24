# porting a character accurately: the deep-RE playbook

the character analogue of [`PORTING_STAGES.md`](PORTING_STAGES.md): how to take one SSF2
character from "converts" to "verified 1:1", enumerating every move, stat, projectile, sound,
and behavior, and verifying each against the live engines. run the phases in order; produce
the artifacts; never substitute a plausible value for a measured one.

the per-change build/iterate mechanics live in [`TESTING.md`](../TESTING.md) and
[`PEPTIDE_GUIDE.md`](PEPTIDE_GUIDE.md); this doc is the AUDIT procedure layered on top.

## the iron rules (same as stages)

1. every value comes from a static read, a live measurement, or a declared gap. never a guess.
2. derived values get verified against a second source at least once per mechanism.
3. never invent a CATEGORY either. the converter does not need to know WHICH character it is
   looking at, only what that character's own data and classes say to do. no table keyed by
   character id, no per-character branch in the emitter, no template that only one fighter fits.
   a table you would have to add a row to before a NEW character converts is a bug, however
   honestly its current rows were measured.
4. per-character tuning goes in the JSONC mappings; code changes must be universal and proven on
   a small sample (`cargo test` goldens + a few conversions). never convert the whole corpus.
5. convert-time success is not success: only live-engine behavior counts.
6. never hand-edit converter OUTPUT; fix the converter and regenerate.

## what a port actually IS

the same model as stages (see PORTING_STAGES "what a port actually IS"), and for the same reason:
the SSF2 corpus is a sample, not the target, and mods will bring characters nobody has named yet.

  the character's own stats    ->  read from getOwnStats / getAttackStats, never a per-name table
  its moves that are CODE      ->  translated from its class methods, not matched to a template
  its animation labels         ->  its own labels, split by the SSF2 naming CONVENTION
  anything still unmapped      ->  a declared gap in the conversion log, never a plausible default

the line to hold: SSF2's FORMAT vocabulary is fair game, content names are not. `getOwnStats`,
`_loop`, the taunt label names, the attack-move names are all part of the format contract, so any
mod that uses SSF2 uses them too, and matching on them is reading the source. `"donkeykong"` is
not: it is one fighter that shipped, and matching it converts nothing new.

## phase 0: setup

```bash
cargo build --release --bin peptide
cargo build --release -p ssf2_converter --bin ssf2_objgraph --features dev-tools
# corpus: ../ssf2-ssfs/*.ssf (or $SSF2_SSFS_DIR)
```

## phase 1: static inventory (every object in the character)

build the complete object list before judging anything:

```bash
peptide convert <char.ssf>          # also writes characters/<id>/conversion_log.json

# the conversion log is the FIRST gap detector. read it, don't skim it:
#  - ssf2_only tallies: every SSF2 api call that had no FM mapping (each is a behavior
#    that silently does nothing in FM until mapped in mappings/commands.jsonc)
#  - any warnings about skipped frames/sounds/sprites
python3 -m json.tool characters/<id>/conversion_log.json | less

# dev-tools dumps (each bin prints usage when run bare):
./build/release/dump_frame_labels <char.ssf>   # every animation label + frame counts
./build/release/dump_costumes <char.ssf>       # costume/palette table
./build/release/dump_raw_stats <char.ssf>      # raw stat blocks

# the AS3 side, same tool as stages: the character class's methods carry the moves that are
# CODE rather than data (specials, custom physics, item logic)
ssf2_objgraph <char.ssf> scripts
ssf2_objgraph <char.ssf> methods <CharClass>
ssf2_objgraph <char.ssf> disasm <CharClass> <method>
ssf2_objgraph <char.ssf> slots <CharClass>     # prints slot defaults (class constants)
```

inventory dimensions to fill in (a row per item, with "ported / partial / gap"):
animations (every frame label), attacks (every hitbox: damage, direction, power, kbConstant,
refreshRate per the shared key map), stats (walk/run/jump/gravity/friction/weight), framescripts
(gotoAndPlay/timers/state pokes), projectiles (NO behavior script in SSF2; they're engine-class
instances, base speed/gravity/bounce/lifetime are engine constants, only accel/ease lives in
the char file), sounds (PCM extraction), costumes, hurtboxes, ledge/teeter/land behaviors.

## phase 2: physics ground truth (static + simulated)

```bash
# raw SSF2 constants -> simulated 30fps motion -> derived FM stats. this is the authority for
# walk/run/jump/gravity/terminal velocity BEFORE you ever boot an engine:
peptide ssf2 stats <char.ssf> [char]
peptide ssf2 scale <char.ssf> [char]   # the one-knob velocity/accel scaling table
```

unit table (SSF2 30fps -> FM 60fps, size_multiplier 1.3): frames x2, px x1.3,
px/frame x0.65, px/frame^2 x0.325. key map: direction -> angle, power -> baseKnockback,
kbConstant -> knockbackGrowth (`mappings/commands.jsonc`).

## phase 3: live SSF2 ground truth

```bash
peptide ssf2 session --char <char>     # boots SSF2 into a match with the character

# the live object enumerator works here too: the character, its projectiles, effects, and
# attack boxes are all in the display tree with live positions/frames
peptide ssf2 tell "tree 3"

# character probes (SSF2 reflection; same `e` syntax as FM):
peptide ssf2 tell "e match.getCharacter(0).X"        # X/Y/XSpeed/YSpeed are FIELDS
peptide ssf2 tell "e match.getCharacter(0).Y = 200"  # writable (placement for tests)

# captured jump trajectory (per-frame t,X,Y,YSpeed csv) for apex/gravity verification:
peptide ssf2 jumpcapture <char>
```

measure: standing y on a known stage (floor anchor), jump apex + per-frame trajectory, walk and
run speeds (sample X over frames), projectile spawn offsets and velocities (tree + sampling),
move timings (currentFrame/totalFrames of the live anim while holding inputs).

## phase 4: live FM verification

the gate for this phase is `tools/verify.py char <id>` (TESTING "the verify gates"): it
publishes, boots, reads the fighter's own frame counter, measures its ground speed against
the gait it declares and drives every move off the engine's per-frame trace. everything below
is how to look closer when a gate fails.

```bash
# the .fra the engine loads is built by FrayTools. ALWAYS re-export after regenerating:
peptide export --project "$(pwd)/characters/<id>/<id>.fraytools"
peptide session --char <id>,<id>     # boots FM; out.log streams ANIM/SCRIPTERR
```

- **parse health first**: zero SCRIPTERR lines from the character's scripts. a frame-script
  "Unknown variable" means the main Script failed to parse upstream; fix that, not the frame.
- **physics**: place + sample, exactly like the SSF2 side:
  `peptide tell "e p0.physics.currentVelocityX"` sampled over frames (fractional values
  appear on alternating frames at 60fps; sample several and compare the per-2-frame deltas
  against the phase-2/phase-3 numbers).
- **repeatable move tests**: `scenario <p0 x,y> <p1 x,y> <ctrl:frames...>` places both
  fighters, resets to neutral, and plays a frame-accurate input timeline; re-run the same
  line to replay. `hold`/`seq` drive raw control timelines.
- **knockback parity**: `tools/tests/parity_check.py` (data-level hitbox comparison) and
  `tools/tests/translation_completeness.sh` (no untranslated SSF2 calls left in the output).
- **status feed**: `matchStatus()` (per-char damage/anim) for watching hits land headlessly.
- both engines speak the SAME command vocabulary (spawn / e / hold / seq / scenario / tree),
  so every SSF2 measurement recipe has a 1:1 FM twin. run the same probe on both and diff.

## phase 5: the per-move audit loop

for EVERY animation label from phase 1 (not just the ones that looked wrong):

1. trigger it on both engines (scenario/seq with the move's input).
2. compare: frame count (x2 rule), hitbox timing windows, damage/knockback numbers
   (parity_check), motion deltas (sampled positions), spawned projectiles (count, offset,
   velocity), sounds fired.
3. record the row as verified / fixed / gap. a move you didn't test is a gap, not a pass.

projectiles get their own pass: SSF2 projectiles are engine-class instances, so their base
kinematics come from the engine model (phase 2 sim + live sampling), and only the char-file
accel/ease is static. verify each projectile live on both sides.

## phase 6: regression gates

```bash
just verify                              # the ten-fighter, ten-stage sample, every gate
cargo test --workspace && cargo clippy --workspace -- -D warnings
tools/verify.py corpus --chars           # every fighter, when a change touches them all
```

## the per-character completion checklist

- [ ] conversion_log read: every ssf2_only tally either mapped or in the gap report
- [ ] inventory table: every label/attack/stat/projectile/sound/costume rowed
- [ ] phase-2 physics table produced (stats + scale)
- [ ] live SSF2 measurements: floor anchor, jump trajectory, walk/run, projectile kinematics
- [ ] FM: zero SCRIPTERR, physics sampled and matching, parity_check PARITY OK,
      translation_completeness clean
- [ ] per-move audit: every animation row verified or gapped (none untested)
- [ ] corpus sweep + tests + clippy green
- [ ] gap report: every unported behavior with its measured/disasm'd spec

## known gotchas (character-side)

- the engine loads the FrayTools-published `.fra`, not the converter's source output;
  regenerating without re-exporting tests stale code.
- frame scripts read the main Script's vars by bare name; a frame-script "Unknown variable"
  error means the main Script has a parse error upstream.
- SSF2 `setXSpeed` is facing-relative; keep it 1:1 (mapping it to a world-space velocity
  reverses momentum when facing left).
- the engine walks the fighter: a walk clip's frame scripts write the x speed as they play
  (a lean-in frame writes 0, the loop re-states the walk), and the source's own walk speed
  wins over those writes every frame (bowser's clip writes 2.0 and 0 while he walks a flat
  2.46 there), so a literal x-speed write inside `walk_in`/`walk_loop`/`walk_out` is dropped
  in the port; a computed one stays. measure it as per-frame x off `trace raw`, not a window
  average: a six-frame stall a lap reads as a slow walk.
- a speed a script sets or reads crosses the boundary (`__sv`/`__isv`, DEVELOPMENT §7); a
  move that reaches something frames early or late in the port, with the same animation
  lengths, is a speed that missed it (a `setXVelocity(<number>)` with no `__sv` around it).
- both engines' `trace raw` rows carry the animation frame (SSF2's second column, Fraymakers'
  fifth), so a label jump shows as the frame number leaping; compare those, not run lengths,
  when a move's length depends on where something else is.
- `peptide session --char X` boots the INSTALLED pack under the engine's custom folder; it does
  not repack. after a reconversion run `peptide pack characters/X --project
  characters/X/X.fraytools` before an ad-hoc probe, or the probe tests the old script (the
  verify gates pack for themselves).
- a match option this engine lacks (the source's `levelData.<x>`) reads as off through
  `__optionOff`, an object whose `toString(radix)` is "0", so a bit-field test runs and takes
  the plain-match branch; a chained `reverse()` renders as `__arrayReversed(..)` because the
  host's reverses in place and answers nothing; `endAttack("<state>")` ends into that state.
- a frame whose script ends the move (`endAttack`) is never shown in the source, the clip's
  last frame included (measured on the source's own per-frame record: pichu's down special
  shows its 59th of 60, simon's jab its 24th of 25). the port trims it and joins its script
  to the last shown frame's, where this engine runs it after that frame has shown, which is
  when the source runs it. the one exception is a clip of a single frame that ends the move
  on it (an air special the source refuses): its end goes one frame past, since ending a
  state on the first frame this engine has entered it takes the engine down natively.
- `air_ease` is a cap on the fighter's downward speed (-1 is none): an animation starts with
  what its attack declares, a frame script's write replaces it, and the port clamps to it in
  `update` through `__applyAirEase`. a move that floats or falls wrong on the port while its
  source sets `air_ease` is the first thing to check, with `tools/trajectory.py` (both
  engines' records side by side by source frame).
- a fighter's or projectile's script speed crosses the boundary unchanged (the source's api
  layer already divides by the size-and-rate factor); a port that rises two thirds as high as
  the source on a scripted write is scaling that write twice. a y write carries one source
  frame of gravity (`__svy`); a port that rises one row's worth higher than the source on
  every scripted rise has lost it.
- a zero x speed a script writes on the floor holds until the next write (`__xHold`); a port
  that creeps forward where the source stands still under a held direction has lost it.
- a position a script states crosses the size scale (`__sp`/`__isp`); a thing placed by its
  script a third too close on the port has lost it.
- the harness runs both engines on the same stage (the source's battlefield and its port). a
  move that reads three times as long on the port with a `heavyland` follower on the source
  is usually landing on a platform there: check the stage before the move.
- a fighter the script set on the floor (a move that lands by its own code) is not attached in
  this engine: `isOnFloor()` answers false and `attachToFloor()` fails there. `__scriptFloor`
  is the script's own flag, and a stand from there goes through a one-pixel fall the engine
  lands.
- SSF2 timer repeat 0 means "forever" (AS3 convention); FM uses -1. the converter rewrites
  this; if a polled behavior fires once and dies, suspect a timer that missed the rewrite.
- persistent cross-frame state in scripts uses `self.make*` + `.get()/.set()`; a plain `var`
  re-initializes every frame.
- batch live tests inside ONE engine session; rapid spawn/kill cycling leaves stuck engine
  processes (harmless to fresh boots, but don't accumulate them).
