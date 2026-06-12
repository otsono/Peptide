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
3. per-character tuning goes in the JSONC mappings; code changes must be universal and proven
   by the conversion sweep.
4. convert-time success is not success: only live-engine behavior counts.
5. never hand-edit converter OUTPUT; fix the converter and regenerate.

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
# the whole character corpus must still convert
for f in ../ssf2-ssfs/*.ssf; do peptide convert "$f" --out /tmp/charcorpus || echo "FAIL $f"; done
cargo test --workspace && cargo clippy --workspace -- -D warnings
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
- SSF2 timer repeat 0 means "forever" (AS3 convention); FM uses -1. the converter rewrites
  this; if a polled behavior fires once and dies, suspect a timer that missed the rewrite.
- persistent cross-frame state in scripts uses `self.make*` + `.get()/.set()`; a plain `var`
  re-initializes every frame.
- batch live tests inside ONE engine session; rapid spawn/kill cycling leaves stuck engine
  processes (harmless to fresh boots, but don't accumulate them).
