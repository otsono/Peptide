# validating a converted character

how to take a converted SSF2 character end-to-end and prove it's correct, both in the
**FrayTools editor** (does it lay out / render right?) and in the **Fraymakers engine** (does
it load, spawn, animate, and behave at runtime?).

there are **two** harnesses doing two different jobs. each lives under `tools/` and contains
**no Fraymakers/FrayTools code, assets, or strings** -- they just drive the user's *local*
installs over standard protocols. everything they touch (engine artifacts, patched copies,
`.fra` packages, `node_modules/`) stays on the user's machine and is git-ignored.

> **copyright / compliance boundary -- never publish.** to respect Team Fray's wishes, the
> tracked docs **don't explain how to decompile or patch the Fraymakers engine or
> the FrayTools bundle**, and **don't name specific non-hscript engine classes / functions /
> fields** or the engine's internal symbol map (move-dispatch / telemetry / load symbols, the
> FrayTools render internals). we haven't included that material; please keep it that way. it's
> fine to document what the harness *does* and how to contribute to it (the commands, the
> patcher workflow, resolve-by-name, the `doctor` preflight). never commit or publish the
> Fraymakers engine bytecode/Haxe/disassembly, FrayTools bundle code, SSF2 AS3, decompiled
> output, or any of their assets. see [`NOTICE.md`](NOTICE.md) "Reverse-engineering &
> copyright boundary" and [`AGENT_CONTEXT.md`](AGENT_CONTEXT.md) "engine-side knowledge is not
> in this repo".

| Harness | Dir | Drives | Transport | Answers |
|---|---|---|---|---|
| **FrayTools** (`peptide`) | `src/fraytools.rs` | the user's FrayTools editor | the editor, run locally | "does it render / lay out right? publish me a `.fra`" |
| **Fraymakers** (`peptide`) | `src/` + `tools/` | the user's Fraymakers engine | patched engine copy + loopback TCP | "does it behave at runtime? load / spawn / move / read state" |

both surfaces are subcommands of the one `peptide` binary; the detailed command surface lives in:
- [`docs/PEPTIDE_GUIDE.md`](docs/PEPTIDE_GUIDE.md) -- `peptide export` (publish), `peptide harness` (box geometry + capture), `peptide render`, the live-engine harness, the loopback bridge, and `tools/run.sh`. `compare_boxes` is a `dev-tools`-gated diagnostic bin.

this doc is the **cross-tool workflow + the current validation status**. the engine-side
integration map isn't in the tracked repo (removed for compliance, see the boundary note).

---

## the verify gates (run these before calling anything ported)

`tools/verify.py` is the one answer to "is this port right, and is it running": a fixed set of
gates per subject, each needing POSITIVE evidence, a failed gate blocking the ones after it.
it exists because every earlier check answered one question and each new way a port could be
wrong was invisible to the check before it: a script the engine drops whole reads as "no
errors", a stage pinning the engine reads as "no errors", an element in a container the walk
never opens reads as "not there", a publish that timed out leaves the previous package
installed and the next check measures that. finding those one at a time, after the port had
been called done, is the cycle the gates end.

```bash
tools/verify.py stage bowserscastle      # one stage, all gates
tools/verify.py char mario               # one fighter
tools/verify.py sample                   # the ten stages + ten fighters chosen for coverage
tools/verify.py corpus --stages          # everything (hours; the package goes after each)
tools/verify.py stage fourside --ssf2    # add the cross-engine gate
just verify-roster                       # every fighter, runtime path, cross-engine; hours
just hits-roster                         # every fighter's hits on both engines, aerials included
```

reports land in `build/verify/<kind>/<id>.json` (every gate's evidence) and
`build/verify/summary.md` (one row per subject). exit 1 if anything failed.

a subject is converted from the installed game's own data file when the game is present
(`/Applications/SSF2.app/Contents/Resources/data`, or `SSF2_DATA_DIR`; the id-to-file table comes
from `peptide ssf2 identify` and is cached under `build/verify/`), and from the corpus copy
otherwise or with `PEPTIDE_VERIFY_SOURCE=corpus`. the corpus is a sample that can be another
build of the same content (mario's fireball clip is 18 frames in one and 23 in the other), and
a port compared against a build it was not read from reports every difference between the two
builds as a port bug.

for a stage, the cross-engine gate pairs every Fraymakers element to its source clip by name:
the converter's plan beside the package names the clip behind each vfx it made and where it put
it, and the live tree reports each vfx at that position. an element the plan marks as keeping the
stage's whole timeline under a schedule (a clip the stage puts down over and over) is read for
what it is, an element driven across frame labels is reproduced when its game object was seen
playing them, and a clip the source places at no opacity is a placeholder nobody sees and is not
looked for. the whole report lands beside the gate's own in `build/verify/stage/<id>.s6.txt`.

the cross-engine gate drives both engines with the same inputs, each on an engine that has
played nothing yet (what a move stores survives it: a charge kept between uses fires on the next
tap), and reads each engine's own per-frame record; a direction is held before a side special (pressed on the same frame it reads
as a flick on the source), the fighter is put back where it started between inputs on both
engines, and the walk is compared too, the source's read off its record and scaled by the
converter's own velocity scale. a move whose frame scripts jump back to an earlier label of
their own is held until an input ends it: the move gate reports those and this gate decides.

the engine-bound gates pace on the engine's own frame count, never the wall clock, so
`--speed=4` (four engine frames per tick, `peptide tell "speed 4"` by hand) makes them arrive
sooner and read the same. two stay at the engine's own rate whatever the flag: the held walk
(a few seconds; sped up, each round trip walks the fighter to the ledge) and the move gate
(parking, launching and reading a move are a dozen round trips each, and sped up those are
frames of falling no park height covers). the stage side is dominated by converting and
publishing, which no frame rate touches.

the evidence comes from two sources that cannot lie by leaving something out:

- **the plan**: the converter writes `verify.json` beside every package: what the script
  makes, where, on what clock; which scripts the package must carry; how much art it ships.
  one statement of what the port is meant to be, written from the values that were emitted.
  a vfx row names every clip over it (`names`), the object it rides (`rides`) and whether the
  stage's own code drives it (`driven`: a frame the code picks, or a ride); an object row and
  a `spawned` row list the source clips the object draws with (`clips`); the `stage` row lists
  the stills painted into the stage art by clip (`painted`); `symbols` lists every clip class
  the package declares, so a source row of a class outside it (an item the match dropped in)
  is the game's own and not paired.
- **the self-report**: the emitted script counts its own frames and keeps a registry of
  everything it made, and the live gates ask it (`e __verifyDump()` inside its own scope; a
  fighter through what its script hands out). a script that was dropped, hung, or never
  reached its spawn block cannot give the right answer, which is the point.

| gate | proves | fails when |
|---|---|---|
| S1 convert | exit 0, plan written, scripts parse, art resolves, within the publish budget | any of those; over 250MB of art will not publish |
| S2 publish | export succeeded AND the installed package carries every planned script and entity, and is newer than the conversion | a stale package would otherwise be measured as this one |
| S3 boot | STAND reached, engine not pinned, the script's frame counter advances, the spawn block completed, no script errors | a hang, a dropped script, an error on the first frame |
| S4 built | every planned object exists, at its planned position (unless the code moves it or it rides an object), on its planned clock | a struck spawn, a wrong container, a wrong animation length |
| S5 motion | every loop advanced by exactly the frames the stage ran (read atomically with the frame count); a picture the plan marks `driven` keeps the frame the code picks and is counted, not timed | a stalled clock; no wall clock and no aliasing can fake it |
| S6 stand | each fighter is lifted 40 px over its entrance in both engines and let fall (`--ssf2`), twice a stretch of frames apart so a passing object cannot decide it, and the height it lands at (read the first frame it is on a floor, before a slope can slide it) is within 16 px of where the source rests it, through the port's own coordinate model (`verify.json` carries the scale, the fighter-space origin, which is the terrain's registration, and the entrances). a second drop from 240 px up clears an entrance that sits inside a plate, and each entrance is judged from the lift that cleared it. an entrance the source itself does not hold (its fighter falls and respawns, read over a stretch longer than the revival platform's hold) skips the gate: a stage whose ground waits for its game mode; so does a source fighter that never left where it was put (its engine is not running the match). both fighters are also read at one frame of the match on both engines, each walked there on its own clock, since a rest on something that moves is only the same rest at the same moment. the source's own landing is read too, beside the port's, and a landing that matches is a match; a source that lands at one height and is at another a moment later was moved by the stage on its own clock (rainbow route's mirrors, a scrolling ground), so that entrance is left out, and the gate skips when every entrance is | a floor the port has and the source has not, or the other way round: a fighter standing in the air or through the ground |
| S7 ssf2 | the same stage in SSF2, object for object (`--ssf2`), compared in-process on the run's live engines by `tools/tests/stage_cross_engine.py` (it boots its own only when run by hand); each element's frame advance is read against an exact tick gap between two walks; a plane or root clip whose own frames place only stills (the plan's `painted`, or a stage painted once with nothing cut from the root), or whose own timeline moves nothing drawn (the `stage` row's `plane_art_moves`), is compared by its children; a piece the plan marks `driven` is held, not timed | the two engines animate differently |
| S8 quiet | no script error in the whole match once every gate above has driven it: S3 counts only the boot's, and a stage's code can throw once a minigame or a phase starts | a stage that throws after boot |
| C1 convert | exit 0, plan, every animation 2.00x its source range, hitbox stats at parity, art resolves | any of those |
| C2 publish | as S2 (`peptide pack`) | |
| C3 boot | STAND, the fighter's script frame counter advances, no script errors | |
| C4 physics | held-direction ground speed equals a gait the fighter declares: a stat (walk cap, dash, run) or a speed its own walk frame scripts set | off by more than 8% of every declared figure |
| C5 moves | every move driven from the middle of the floor; its own run read off the engine's per-frame trace against the emitted length; back to STAND. aerials launched airborne at a height from the declared gravity. the engine's SCRIPTERR channel read over the whole run | a move that never plays, or never returns; any script error while the moves ran (one line per distinct complaint in the evidence); the engine dying, reported at once rather than after every later move's timeout. a move whose own code ends it early or keeps it past the animation's end is reported and left to C6 |
| C6 ssf2 | the same inputs on both engines (`--ssf2`): jab, the four specials, jump, each read off the engine's own per-frame trace and each started at the same source frame of its match on both (paced on `clock`, so whatever a fighter charges over time is charged the same); the move's identity through the fighter's own name map, its length as source frames x2 wherever the source ended the move itself and the port's run was neither a loop nor cut by the floor | a different move, or a different length. the smashes and tilts (a direction pressed with attack, or held first) are in the `--deep` set with the aerials |

the input gate runs on a fresh engine on each side with the same two-fighter roster, the
fighter put back in the middle of the floor before every input and the other fighter parked
the same distance away in each engine's own units (the port's world is scaled), out of a
jab's reach and within a homing attack's, so a move that seeks the nearest fighter meets one
at the same moment on both engines and a jab's hit stop lengthens neither. an input is judged only once the engine has acknowledged every step of it,
and the move is over the moment the engine announces a pose a move comes back to. a wait
that has to be "a few frames from now" is `tick N`: both engines push one line per frame for
the driven fighter, the session counts them, and `tick` answers the moment the Nth arrives,
so the driver never sleeps and never sizes a wait for the slowest case. `clock` answers how
many of those frames the session has seen (`CLOCK:N frames`), the same way on both engines:
the source engine has no match counter to ask, so its frame since the spawn is this clock less
its reading when the match came up, and two engines are brought to one moment of the match
by walking each to that frame on its own clock (the stand gate reads both fighters there,
which is the only way a rest on something that moves is the same rest). `--speed N` runs
both engines at N times their rate (on SSF2 that is its frame rate raised by N), and every
gate paces on frames, so the numbers are the same and arrive sooner: a fighter's six gates
take under a minute for two fighters at `--speed 4` once the games are open: the driver
keeps one engine of each kind up and starts each subject's match in it with `spawn` (a
fighter or stage published after the boot is loaded by that launch). the engines stay up after
the run, in `build/verify/sessions/<engine>/`, and the next run drives them instead of
booting (`--shutdown` ends them at the end of a run). the SSF2 engine boots in the background
from the start of a run, and the next subject converts and publishes while the current one
is being driven, from the same installed data the gates use. what a subject reads of the log
(errors, waits, traces) starts at its own match, and the floor the move and input gates put
the fighter back on is where it stood when its match began. an engine that cannot start the
match asked of it is replaced by a fresh one. two SSF2 gotchas the driver works around: `reset` puts the
fighter back at its spawn, in the air, and a reflection read of its height answers the
spawn's, so the floor height comes off the fighter's own per-frame record and the fighter is
set down on it before every input; and the source's announcements trail its frames, so a
window is opened only once the record itself shows the fighter standing.

what a gate cannot know it says so: C5 reports a move whose own code chose its length (ended
early into another state, or kept past the animation's end while a slide finishes) because
whether the source does the same at the same frame is a cross-engine question, and a launch
condition that is the gate's own fault (parked into the blast zone, cut by landing) is
reported as that rather than as a port failure.

the sample: stages `bowserscastle castlesiege sectorz mushroomkingdom3 finalvalley fourside
racetothefinish galaxytours battlefield clocktown` (hazards with their own classes, a phased
backdrop, a nested-clip backdrop, moving plane elements, a ported class with weather,
cross-script objects, two hundred elements, a thousand assets, the plainest stage, a day
cycle); fighters `sandbag mario zelda link samus kirby pikachu sonic donkeykong fox` (the two
references, a two-character package, projectiles, a charge special, a multi-jump, a fast
faller, a heavy, unusual specials).

## 0. two test tiers: the fast gate, and the corpus tests

`cargo test` is a BUILD GATE, not a conversion run. it finishes in a few seconds and works the
same on a fresh checkout with no corpus.

exactly ONE test in the gate reads a real SSF2 file: `golden_sandbag`. it converts sandbag and
diffs the output against committed hashes, which is the only thing that catches an unintended
change to real converted output -- no synthetic fixture can stand in for that. it costs ~2s,
and it skips itself when the corpus isn't on disk. every OTHER corpus test is `#[ignore]`d.

```bash
cargo test --workspace            # the gate: unit tests + from-scratch fixtures
cargo test -- --ignored           # the corpus tests: real .ssf files, minutes
```

everything else that needs a real `.ssf` is marked `#[ignore]`. two reasons, and both matter:
official SSF2 content can never live in this repo, and a full character conversion is seconds
of work that has no place in a gate you run on every edit.

when you add a test, prefer the gate. most assertions don't need a real character or stage:

| fixture | file | what it covers |
| --- | --- | --- |
| hand-built `StageModel` | `tests/synthetic_stage.rs` | the stage emitter: depth containers, guids, determinism |
| hand-written SWF | `tests/synthetic_swf.rs` | the parser on degenerate input: empty, truncated, non-swf |
| hand-written SWF + `--name` | `tests/synthetic_character.rs` | the character pipeline: project layout, manifest ids, conversion log, determinism |

the SWFs are real ones written by the `swf` crate, not fixture blobs -- `ssf::decompress`
passes a raw `FWS` stream straight through, so the converter sees them exactly as it sees a
real input, and nothing copyrighted is involved. passing `--name` skips character DETECTION,
which is the only part of the character path that needs real AS3.

reach for the corpus only when the thing under test IS what lives in AS3 -- character
detection, `ssf2_source` provenance, stat extraction, transformation forms -- then mark it
`#[ignore]` with a reason.

two roster-wide checks sit between the gate and the live harness. both convert every fighter in
`build/verify/ssf2-data.json` in parallel (about 40s) and need no engine:

| check | what it catches |
| --- | --- |
| `just converter-audit` (`tools/tests/strike_tally.py`) | a strike kind that is new or grew against `tools/tests/strike_tally.json`, an emitted script the engine would refuse to parse, a call nothing answers (`tools/tests/unanswerable-calls.py`: `self.`, `match.` and bare calls, frames read in their script's scope). `-- --update` keeps a lower floor |
| `tools/tests/output_snapshot.py --save NAME` then `--against NAME --show N` | every emitted file a refactor changed, with diffs. a change meant to be output-neutral should report `0 changed` |

a live run that outlasts an edit holds one build with `tools/freeze.sh DIR` and
`PEPTIDE_BIN=DIR/peptide`: the converter reads its mappings and the engine scripts from disk, so
a copied binary alone still picks up the working tree's.

two live checks re-measure what the converter rests on, one fighter at a time:

| check | what it catches |
| --- | --- |
| `tools/tests/calibrate.py [fighter ...]` | the size scale, full-jump and short-hop arcs drifting from `stats.jsonc` on either engine |
| `tools/tests/hit_parity.py fighter [move ...] [--spread=N] [--alone=0]` | a converted hit that lands for something other than the source's (a hit count that differs is played again N times on the source, 3 by default, and one the source itself lands that way too is listed apart as its own spread; a move still apart after that is played once more alone, on a fresh session of each engine where the source landed it, and one the two agree on there is listed apart (`~`, under `alone`) with what each showed, since what the sweep's moves before it left behind parted them; `--alone=0` skips it): the damage it deals as a share of the source's (the same share on every move), and the launch the engine gives, against the scale a speed converts by. the source is told the move by name, this engine is given the buttons (`HP_PRESS=1` presses a grounded move on the source too, for a move whose code reads the buttons as it plays, like pac-man's side special). an aerial starts from rest 80 source pixels above the floor on both engines (a back aerial turned away from the victim), the victim put at the distance in the same batch and the buttons pressed frame for frame on both, and falls back to a short hop, then a full one, when that lands nothing (the hop and its aerial go to both engines as one sequence, frame for frame; `HP_AIR_SEQ=0` names the move on the source after a round trip instead); the hop each move landed from is in the report. the four throws are pressed from a hold on both engines (shield and attack is the source's grab). the angle a hit sends at is compared too, and how many times the move hits; a difference in the hits, the angle, the launch or the damage carries the boxes that landed each hit on each engine, in order, by the source's names (`[boxes: source b b2; here b b b2 b]`, read off the attacker's `gvar p0 __peptideHits`). before each move the victim stands still on the floor and is set to 0% on both engines, so no hit lands on a victim still coming down or carrying another engine's damage. `--reps=N` plays a move that landed N times more where it landed, on each engine, and compares the outcome most plays show (the source lands some moves differently run to run). a fighter the source has only as another's final form (giga bowser, wario man) is played there as that fighter, given its final smash and made to use it before each move, against the base fighter on both engines (this engine's side needs two different fighters in one match). a move only one engine lands is listed under `unlanded` in `build/verify/hits-<fighter>.json`, not among the differences; one the source lands and this engine missed is played again in a fresh session first, since what the moves before it leave behind can stop it. a launch or angle apart on a move whose declared box points down at the standing fighter (the accepted downward-hit gap) or is an autolink is listed under `accepted` (`~` in the report), not among the differences. a move each engine landed first at another distance is played on the source where this engine landed it: when the source lands it there too, the reach is listed under `edges` (`?`), at the edge of this engine's reach rather than out of it. both engines run on between the harness's words and push two standing fighters apart while they do, so the distance a move was played at is read on the frame it begins, off each engine's own record (`gap`/`gap_here`, source pixels), and the port's victim is put at the source's distance on that frame; a move begun 3 or more apart is listed under `spacing` (`=`), and `HP_GAPS=1` lists every move's. before each move the watch also waits out what an earlier move left hurting the victim (a poison outlives the reset on both engines). what still moves a result run to run: a move whose own code rolls at random (luigi's misfire, game & watch's judge and the angle his chef throws at, waluigi's dice), one that keeps a charge between uses (black mage's down special, played again at each distance tried), and an aerial from a hop, which each engine reaches at its own point of the jump |
| `tools/tests/runtime_cost.py fighter [Name.path ...] [--frames=N]` | what each part of the runtime costs a converted fighter a frame on Fraymakers: the engine runs as fast as it can, and each runtime function named (default: the per-frame entry points) is stubbed on both fighters in turn and put back, so the time a frame saves without it is its cost, with a baseline between every two. 4000 frames a run keeps the baseline within about 0.2 ms; shorter runs drift by a millisecond. a guide to where the time goes, not an exact split (a stubbed part changes what the rest does) |
| `tools/tests/bounce_trace.py fighter move [--frames=N] [--at=X] [--dmg=0,40,80]` | one move's hit on a victim standing at 0% (or at each damage named, in one boot), per source frame from the frame its damage rises, side by side: the victim's place and how far it moved that frame on each engine, both ways. the place to look once hit_parity says a spike's launch differs: a bounce off the floor that turns a frame later, or leaves at another speed, shows as a column that drifts |
| `tools/tests/box_entity.py fighter ... [--all]` | a box the converter extracted that the entity it writes lacks, has more of, or places differently, frame by frame (offline: converts with `PEPTIDE_DUMP_BOXES` and reads the entity; an animation cut from later in its clip is read from where the conversion's own table starts it, and item, hold-point and camera boxes are left out). every installed fighter comes out clean |
| `tools/tests/box_runtime.py fighter [input ...]` | a hurt, hit or item box the converter extracted that the source places differently at run time: missing on a frame, a different size, or out of place after the best fit (`PEPTIDE_DUMP_BOXES=path` dumps the extraction) |
| `tools/tests/move_frames.py fighter [input ...]` | what a move LOOKS like on each engine, frame for frame, side by side (cropped to what the fighter touched). no gate reads colour or pose; this is where the invented default costumes showed up |
| `tools/tests/state_sweep.py fighter [scenario ...]` | everything that is not an attack on the port: walk, dash, shield, rolls, dodges, double jump, air dodge, taunt, grab and all four throws, pummel, getting hit, ledge, KO and respawn. reports what played, a missing animation, a stuck state, and every script error, each scenario in a match of its own |
| `tools/tests/state_parity.py fighter [scenario ...]` | how long rolls, dodges, grabs, throws and the taunt last on each engine, in source frames |
| `tools/tests/roster_report.py [--save=NAME] [--against=NAME] [--list]` | what a `hit_parity.py --all` sweep left, offline: the moves that differ and by what (reach, hits, damage, launch, angle), the accepted lines, the reaches at an edge, and the moves only the source lands; `--save` keeps the sweep's files under a name and `--against` lists each line new or gone since one saved |
| `tools/tests/rehit_trace.py fighter move [distance] [hop] [--victim] [--proj]` | why a move hits a different number of times: plays it as `hit_parity.py` does (same distance, same hop) and prints each engine's own per-frame record of the attacker (animation, frame, place, speed), so where it went after the first hit (fell on and landed, or hung in the air for a second) reads side by side. `--proj` prints the attacker's projectile's path through the hits on both engines (a multi-hit projectile that drifts off its target shows where); a tool's watcher goes on through `hit_parity`'s `before_record` hook, after its reset |
| `tools/tests/push_parity.py fighter ... [--at=10,20,30,40,50]` | how far apart two standing fighters settle when set down at each distance, on both engines, in source pixels: the distance a move is really played at |
| `tools/tests/hop_trace.py fighter ... [--frames=N] [--full] [--then=down+attack:3]` | a short (or full) hop's height and animation each source frame from the tap, on both engines; `--then` presses an input so many frames after the tap in the same sequence, to see on which frame of the hop each engine starts it |
| `tools/tests/move_trace.py fighter "input" [--frames=N] [--at=X] [--from=X] [--floor] [--all]` | any input frame by frame on both engines side by side: the animation, its frame and the fighter's position each source frame from the first one out of the stand, pressed from the stage middle like `input_parity.py` (`--at` puts the other fighter that far in front, facing back; `--from` starts the fighter that far right of the middle, where C6's second spot is a platform; `--floor` writes the source fighter's height too, as the gates do; `--all` prints every frame, where a still stretch is otherwise one line: a throw, a freeze). use it when a gate says a move ran a different length: it shows the frame each engine took a branch the move's own code makes on a landing, a hit or a wall |
| `tools/tests/input_parity.py fighter [fighter ...] [--inputs="seq|seq"] [--show]` | what a press turns into: the same input sequences (`a:N b`, source frames) pressed on both engines, frame for frame from standing at the stage's middle, and the moves each played, by this engine's names (the source's mapped through `conversion_stats.json`); a sequence the two play differently is marked `<<` (`--show` prints both records under it). each input starts with the source's fighter written back to the floor's height as well as the middle, so one the last input left on a platform starts on the floor. the default set covers the attacks and specials from standing, the grab, an attack on each frame of a jump's squat and after it, a walk and a crouch into an attack, and how long a direction held before attack still reads as a smash. it found the source's up smash out of a jump's squat, its smash window (a direction held 3 source frames or less, 2 for up and down, counted over any control held) and its turnaround smash (the way behind pressed fresh). a crouch and a dash off a double tap are left out: this engine takes them off the stick's travel, which the harness's pressed buttons do not move, so its fighter only stands or walks there |
| `tools/tests/hitstun_parity.py fighter [move ...] [--all] [--flight]` | a hit that holds the fighter it lands on for a different time than the source, or flies it a different way: plays each move where the last `hit_parity.py` sweep landed it (one that sweep did not land is found the same way: the source tries its distances in order, this engine plays where the source landed it and then the others, and a line says where each landed when they differ) and reads, off each engine's record, the source frames from the last hit until the fighter leaves its hurt or flying state (a fall or tumble after it is free again), its launch speed, and how far it has gone across and up 10 source frames into its flight (from its first real move, since this engine slides a frozen fighter a little). `--flight` adds the whole flight: across and up at 5, 10, 20 and 30 source frames, the peak, and how far the port's fighter got against the source's. `--all` plays every move. `HP_DMG=80` sets the damage the fighter takes the hit at: a launch's shape only shows at damage. a fighter knocked out and put back inside the watch (a jump of over 100 source pixels in a frame) has its flight cut there and is marked `KO` |
| `tools/tests/freeze_parity.py fighter [move ...]` | how long a landed hit freezes the attacker on each engine (a whiff against a hit); `--source-only a b c` sweeps the source's ground moves, `--victim` compares how long each move holds the fighter it hits (the source's off where the victim stands, this engine's off the freeze it hands the victim) |
| `tools/refresh.py fighter ... \| --all` | converts from the installed game's data and packs, in parallel (the roster in about three minutes); run it before any live check, since the engine plays the packed build |
| `tools/tests/proj_parity.py fighter ... [--moves=...] [--fresh] [--check] [--save] [--left]` | fires each special on both engines (thrown left with `--left`, the layout mirrored so it faces the other fighter, kept apart in the baseline) and compares what the thrower's own projectiles do, per frame on each engine's clock: when the first appears, how long they are out, how many at once, whether the first comes back, and how often the fighter plays the move (a catch plays it again). with several out at once it follows the newest, as the source's record does. the SSF2 side reads the count and first projectile's position off the injected per-frame record. `--fresh` converts and packs each fighter from the data the source engine runs first (the installed game's, which can be another build than the corpus copy); without it the installed package is measured. `--check` fails on a move `tools/tests/proj_parity_baseline.json` lists as matching that differs now; `--save` records the run's matching moves there |
| `tools/tests/proj_trace.py fighter move [--frames=N] [--every=K] [--fresh] [--thrower]` | one move's first projectile, per source frame from its first, side by side: where it is from its thrower and how far it moved that frame on each engine, and the animation it is in here. the place to look once proj_parity says a life or reach differs: a later start, another slowdown or another turn shows as a column that drifts. `--thrower` prints the thrower's own path from the move's start instead, with the projectile's height over it: a projectile that sits still while its thrower moves drifts on whichever of the two is off |
| `tools/tests/ssf2_probe.py fighter move "path;path..." [distance] [frames] [--victim="path;..."] [--hop=N] [--dmg=N]` | any public value of the source's fighter, read on every frame while a move plays: the paths (dotted, from the fighter) are handed to the injected per-frame record, which reads them each frame and appends them to its line. `--victim` puts the other fighter's record of the same frame on the row (its label, place, damage and its own paths), so the victim's freeze and launch read beside the attacker's refresh (`trace raw all`); `--hop=N` taps jump first and names the move N frames later, for an aerial; `--dmg=N` puts the victim at N% first; a throw is played by its grab inputs, as `hit_parity.py` plays it. for a rule that turns on a single frame and a value the record does not carry; a `e` read lands only every dozen frames or so |
| `tools/tests/ssf2_watch.py fighter move pattern [--samples=N] [--every=K] [--speed=S]` | what the source's display holds while a move plays: every display object whose class or name matches `pattern`, its frame, label and place, each walk stamped with the engine's own frame count (the source runs slowed, a quarter by default). for what the source's per-frame record does not carry: an enemy a fighter makes, an effect, a clip inside a projectile |
| `tools/tests/stage_watch.py stage pattern [--from=F] [--samples=N] [--every=K]` | a source stage's clips frame by frame from frame F of the match: the frame and label of every clip whose class or name matches, printed when it changes. for what its clips do between the gates' reads: a plane that plays on after its code sends it to a label, a frame script that sends it back |
| `tools/tests/unknown_scan.py [stage\|character] [--jobs=N] [--only=a,b]` | every installed package converted on the runtime path, one at a time to scratch and deleted after: each name the converted code asks for that nothing answers, how many packages ask and which, plus their problems and any conversion that fails. offline, so it runs beside a live sweep; the whole tally goes to `build/verify/unknown_scan-<kind>.json` |
| `tools/tests/soak.py fighter ...` / `--changed` / `--all` | a few thousand frames of seeded random play beside the other fighter (hits both ways, grabs, KOs), every distinct script error with its origin, load errors included. several fighters share one engine session; `--changed` plays only those whose scripts changed since their last soak or that reported something then |
| `tools/tests/rollback_parity.py fighter [--baseline]` | a fighter that plays differently when the match rolls back: the same seeded inputs in an ordinary match and in a local one that rolls back as an online one does (`netcode = 3`), both fighters read every few frames; `--baseline` plays the ordinary one twice to show the harness's own noise |
| `tools/tests/frame_hold.py fighter [input ...]` / `--all` | an object the source holds on one frame (its own timeline stopped) that runs here instead, or the reverse, per object the fighter's own moves put on the stage plus the fighter's own clip |

`crates/ssf2-converter/tests/renamed_real_character.rs` (`cargo test --ignored the_converter_handles_a_character_it_has_never_seen`) is a different kind of check: a corpus
character's own compiled AS3, byte for byte, with every name the mod author chose fabricated
through the lossless ABC codec so nothing in it exists anywhere in the corpus. It proves a
mechanism reads structure rather than a specific character's vocabulary -- run it after any change
to how the converter reads a name off the source, not as a substitute for the roster gates above.

## 1. the end-to-end iteration loop

```
# 1. Make a converter change
$EDITOR src/<...>.rs

# 2. Rebuild + regenerate the target character
cargo build --release
./build/release/peptide convert ../ssf2-ssfs/<id>.ssf
#    → ./characters/<id>/   (⚠ verify it is FRESH — see §6 stale-output trap)

# 3. Publish to .fra → lands in Fraymakers' custom/<id>/ (built in-process, no FrayTools)
./build/release/peptide pack \
  --project "$PWD/characters/<id>/<id>.fraytools"

# 4. Boot ONE persistent engine session, then drive + observe it iteratively
./build/release/peptide session --full &        # boots + holds a live engine
#   wait for "engine READY" in:  ./build/release/peptide log
./build/release/peptide tell "spawn <id>"                       # start a real match
./build/release/peptide tell "p0.getStateName()"                       # read state
./build/release/peptide tell "p0.toState(CState.JAB)"                  # drive a move
./build/release/peptide log -n 20                # read everything the engine streamed back
./build/release/peptide tell "exit"             # clean shutdown when done

# 5. Compare behaviour vs expected → fix in the converter (never hand-edit output) → repeat
```

**the session IS the loop.** `peptide session` is the canonical way to test and iterate on a
converted character: it boots the engine ONCE and HOLDS the TCP link open, so you send a
command (`peptide tell …`), read the reply (`peptide log`), decide the next one, and repeat,
all against the *same* live match. full command surface in §3. (the old one-shot
`tools/run.sh "<cmd>" <secs>` / `tools/runseq.sh` paths still work for scripted runs, but they
reboot per invocation, so use a session for interactive iteration.)

**one session, one game.** a session and the engine it drives are a pair, recorded in the
session dir. starting another session REPLACES that pair (it asks the old daemon to exit, which
shuts its game down the clean way, then takes its place), so you never end up with two engines
reading the same control file and stealing each other's commands. `peptide session --attach`
drives the pair already running and launches nothing, which is what you want when a session is
up and you only need to send it commands. `peptide ssf2 session` behaves the same way.

**is it actually animating?** ask per OBJECT, never per pixel. a whole-frame diff cannot answer
it: parallax, the fighters and the hud move whatever the backdrop does, so a frozen element hides
inside a frame that "changed". take a few `tree` dumps and run `tools/anim-advanced.py`, which
compares each object's own animation frame across samples. take at least THREE: two can land on
the same frame of a short loop by chance and report a moving object as frozen.

**comparing against ssf2.** `peptide ssf2 session --char <c> --stage <s>` boots the patched app
into a match and `peptide ssf2 tell "shot <path>"` captures it, which is how a converted stage is
judged against the source rather than against memory. two things to know: the stage must be baked
at boot (re-spawning over a live match reports the new stage and keeps the old one), and a stale
ssf2 already running makes the bridge come up mute, so close it first.

**validating a boot (notes for agents):**
- a quick boot is **fast** -- you'll see `READY` → `LAUNCHED` → `ANIM:STAND` in **under ~20s**.
  don't budget 40s; snapshot `peptide log -n 40` once the boot's had ~15-20s and you'll have
  the whole sequence.
- **`timeout` doesn't exist on macOS** (it's `gtimeout` from coreutils, if installed). to
  observe the stream without blocking, just take a couple of `peptide log -n N` snapshots
  rather than `timeout … peptide log --follow`.

**which harness when:**
- **`peptide pack`** -- the publishable `.fra`, built from the project's library in-process in
  well under a second. this is what the gates publish with.
- **FrayTools-side** (`peptide harness` / `peptide export`) -- visual/layout ground truth (box
  geometry, pivots, rendering), and FrayTools' own publish as a cross-check of `pack`
  (`PEPTIDE_PUBLISH=fraytools` makes the gates use it). pair with `compare_boxes` for a
  numeric verdict.
- **Fraymakers-side** (`peptide session`) -- runtime behaviour (loads, spawns, animates,
  transitions state, responds to a move dispatch). where freeze / crash / physics bugs surface.

**branch note:** `main` holds the converter **and** the merged harness work. (older
`fraymakers-match-harness` / `steam-shim-experiments` branches are historical.)

---

## 2. FrayTools harness -- publish + box geometry

code: `src/fraytools.rs`. it drives the user's local FrayTools, launching it when it isn't
running.

- **`peptide pack`** builds the `.fra` from a project's library without FrayTools and writes it
  to every publish folder the project names (`--out <dir>` to choose). its header matches
  FrayTools' field for field on the sample and its sheets hold the same pictures, their colours
  rounded through premultiplied alpha the way FrayTools stores them (a see-through stage's sheets
  shrink by a third or more); the sheets' compression is its own. the builder is `crates/fra-pack`: pictures decode, sheets compress in
  bands of rows and a fighter's animations compile on every core, and the output is the same
  bytes on any machine and any number of threads.
- **the Lazy Town Exporter** (its own repository) is the same builder as a FrayTools publish
  plugin, on a pool of WebAssembly threads: publishing kirby or sky pillars through it takes about
  1 s of the plugin's own time where FrayTools' exporter takes about 25 s and 8 s; the rest of a
  publish (24 to 43 s) is FrayTools gathering the project before any exporter runs.
- **`peptide export`** has FrayTools publish a converted project and prints the published `.fra`
  path. "Publish All" writes both `<projectDir>/build/` and the Fraymakers `custom/<id>/` dir the
  converter wired up. this is what the GUI's "Publish via FrayTools" button runs; its "Publish"
  button (and the home screen's "Publish a Project") runs `peptide pack` in-process instead, the
  same packer the verify gates publish with.
- **`peptide harness`** opens an entity at an animation and frame, reads its box geometry,
  captures it as a PNG, and emits a JSON report whose `rendered_anchor` field is *FrayTools' own*
  placement of each box's pivot.

tested against **FrayTools 0.4.0**. FrayTools is left running on success; if a run can't reach it,
quit FrayTools and run again so it starts fresh.

### `compare_boxes` -- numeric verdict

```
cargo run -p ssf2_converter --features dev-tools --bin compare_boxes -- \
  --ssf2 ../ssf2-ssfs/<id>.ssf \
  --char <id> --json /tmp/box.json [--tolerance 2.0]
```

reads the harness JSON, parses the SSF2 source, matches boxes by type + size, and reports
per-box drift (FrayTools rendered anchor vs SSF2 intended position). it's **split-aware** --
split sub-animations (`strong_forward_attack`, `*_land`, `grab_hold`, …) map back to their
source SSF2 anim + start-frame offset so they verify instead of being skipped. exits 0 on full
pass, 1 on any failure.

---

## 3. Fraymakers engine harness (`peptide`)

code: `src/` (Rust), with shell orchestration in `tools/`. two bins:
- **`peptide`** parses the Fraymakers engine's bytecode (`hlbc` crate), injects a per-frame
  dispatch block, and writes a patched throwaway copy. also has read-only inspection subcommands.
- **`peptide`** is the loopback TCP server (the injected engine code is the client).

the command loop runs against the live match. the **engine's internal symbol map** (the named
move-dispatch / telemetry / content-load functions, type/field layout, `CState` integer values)
is intentionally **not documented in this repo**, see the compliance boundary note at the top.
any such notes belong only in the gitignored `docs/` scratch space.

### boot model -- `peptide session` (canonical) + `tell` / `log`

a game already open is driven, not replaced: `peptide session --char <x>` (and `peptide ssf2
session`) with a live session in the same directory hands it a `spawn` for the roster and
stage asked for and exits, so the match changes without the app relaunching. `--fresh` forces
a new game. on SSF2 a `spawn` during a match runs the engine's own end-of-game first, so the
old fighters stop polling and their scripts stop; on Fraymakers the launch tears the old match
down itself. with both games open, each session's overlay follows its own game's window
(the session that spawns the overlay names the window owners it follows).

`peptide session` is the primary surface for interactive, agent-driven testing: one command
boots a throwaway-patched engine, holds the loopback TCP link open, and runs a command loop you
feed over time.

```
peptide session [--full | --char <id>] [--dir D]   # boot + hold a live engine (long-lived)
peptide tell "<command>"                           # queue a command for the running session
peptide log  [-n N] [--follow]                     # print/tail the engine replies it mirrored
```

run in a terminal, `session` holds focus and is also an interactive REPL, the terminal
equivalent of the GUI chat. type a command, press Enter, and the engine's reply streams
straight back to the same window (the TCP link stays live the whole time). it's enabled
automatically when stdin is a TTY; `peptide tell` from another terminal still works alongside
it. force it on for a piped stdin with `-i` / `--interactive`, or off (pure `tell`-driven
daemon) with `--no-input`. EOF (Ctrl-D) just stops reading input; type `exit` (or Ctrl-C) to
shut the engine down.

- **`session`** boots a throwaway-patched copy of the engine (the pristine install is never
  touched), binds the loopback server, and waits for the engine to dial in + reach READY. on
  clean exit (or `tell exit`) it kills the engine and removes the transient files. needs
  `dangerouslyDisableSandbox` (the engine's window).
  - `--full` = Title/UGC bridge boot, then drive it with `tell "spawn <id>"` once READY (the
    reliable path; avoids the headless filtered-load crash).
  - `--char <id>` = headless fast-boot baking that character.
  - `--no-boot --port N --token T` = attach to an engine launched elsewhere.
- **`tell`** appends one command to the session's control file; the daemon picks it up within
  ~50ms, translates it, and sends it. commands are the same friendly vocabulary as everywhere
  else (`spawn`, `exit`, or any hscript expression).
- **`log`** prints the mirrored engine output (the canonical record of replies); `--follow`
  tails it live. session state lives under `~/.peptide/session/` (override with `--dir` or
  `PEPTIDE_SESSION_DIR` for parallel sessions).

a session beats one-shots because it keeps the engine alive, so you send → observe → decide →
send again against the SAME match while the game streams TCP back. the one-shot `tools/run.sh`
and `tools/runseq.sh` reboot per run: fine for scripted batches, wrong for iteration.

**wedge discipline:** a crashed engine can leave an uninterruptible (UE) `hl _conn.dat` orphan
that `kill -9` can't reap (GPU/Metal wedge). batch your checks into ONE session; if orphans
pile up, the box needs a reboot before more boots behave. see
`memory/project_peptide-env-wedge-lesson.md`.

### commands

the full command surface -- the tiny friendly vocabulary (`spawn`, `eval`, `hold`, `release`,
`seq`, `load`, `console`, `exit`, `help`) and the **hscript-first model** (anything
unrecognized runs as an hscript expression against the live match) -- is documented once in
[`docs/PEPTIDE_GUIDE.md`](docs/PEPTIDE_GUIDE.md) "Commands -- and the hscript model". type
`peptide help` for the live list. below are only the testing-workflow specifics.

per-state-change animation telemetry streams as `ANIM:<state>` (the bridge dedups to changes
only), the animation-capture readback (criterion #5).

short-name resolution (`spawn` / `load`): a bare `sandbag` (no `::`) is tried against
`private::`, `custom::`, `public::`, `global::` in order; first existing resource wins.
`private::` is first so a bare name resolves to headless-loaded custom content. or pass a full
`namespace::package.id`.

**multi-command sessions:** hscript probes need the *same* live match (a reboot loses it). use
`tools/runseq.sh <gap_s> "cmd1" "cmd2" …` to feed a gapped sequence into one session: it boots
once (boot→READY budget via `FRAY_READY_BUDGET`), fires the first command at READY, and paces
the rest by `gap_s` (fractional OK). example:
`./tools/runseq.sh 6 "spawn sandbag" "p0.getStateName()" "p0.toState(CState.JAB)"`.

### timing parity (is the 30->60fps doubling right?)

`tools/tests/timing_parity.py <char>…` (or `--all`) checks the converter's core timing rule
for EVERY animation, statically, with no engine running:

```
tools/tests/timing_parity.py mario          # 164 animations checked, 0 off 2.00x
tools/tests/timing_parity.py --all          # the whole roster
```

SSF2 runs at 30fps and Fraymakers at 60, so a converted animation should be exactly twice
its source. Expected length comes from the splitter itself (`PEPTIDE_DUMP_ANIM_SPLITS`),
which reports for each emitted FM animation the SSF2 animation it came from and the
`[start..end)` range it was sliced from; actual length is the emitted `.entity`'s IMAGE
layer.

**Pair from the splitter, never from names.** One SSF2 timeline becomes several FM
animations (a Jab sprite -> jab1/jab2/jab3/jab4, see AGENT_CONTEXT "animations" and
`src/anim_splitter.rs`), so name-matching either misses a split or sums pieces that
shouldn't be summed. a first version that guessed from name suffixes reported 17 failures
on mario that were all its own grouping. A looping split is reported but not counted: its
emitted length is one cycle the engine repeats, not a duration.

This is the cheap, total-coverage half of frame parity. The live half (below) is what
confirms the emitted lengths actually PLAY that way.

### one move, frame by frame, on both engines

`tools/trajectory.py <character> "<input>" [source frames] [--park] [--at=X] [--recovery]`
plays one input from a stand on SSF2 and then on Fraymakers and prints the two engines' own
per-frame records side by side, aligned by source frame (the port's two engine frames a source
frame are paired, its speed shown over the velocity scale so the columns read in the source's
units; x and y are each engine's own pixels). the source's `fr` is its pose clip's frame, which
is what plays a move. `--park` puts the fighters where the verify harness puts them (the
fighter at x 0, the other one `OTHER_AT` away in the source's pixels, `OTHER_AT * SIZE_SCALE`
in the port's: a dash that meets it on one engine must meet it on the other); `--at=X` parks at
X instead (the harness parks every other input off centre); `--recovery` parks high and lets
the fighter fall a few frames first, as C6 does an up special. C6 itself writes every input's
per-frame record on both engines to `build/verify/c6-paths-<id>.json`, so a C6 difference can
be read off the run that produced it. the harness's home
spot is the middle of the floor (`floor_spot`: the spawn is a side platform on battlefield, and
the middle of the stage at that height is the gap), reached through a short fall so the engine
lands the fighter: one set down on the floor's height directly is not attached and takes no
input. the stand gate lifts each fighter above its entrance the same way, in a fall, since one
set down at a height in its stand state sinks through a drop-through platform on the port
(lunar core's entrance platform caught the source and not the port on that account). the C6 match is spawned on the baked stage by name, since an unnamed spawn comes up on
the engine's default stage. a source session whose app never dials in reads the parked
process's descriptor for the runtime's startup semaphore, unlinks it and starts the app once
more (an instance killed outright leaves that lock held in the kernel, and every launch after
it waits on it); after stopping a sweep by hand, kill the leftover engine processes by
executable name before the next run. both tools boot the
same stage on both engines (`CHAR_STAGE_SSF2` / `CHAR_STAGE_FM` in verify.py: the source's
battlefield and its port), since a move that comes down onto a platform on one stage and the
floor on the other reads as two different moves:

```
python3 tools/trajectory.py gameandwatch "up+special" 60
 sf  source          fr        y      ys   |  port                fr        y vy/scale
  4  b_up            24    20.05  -23.70   |  special_up           9    41.73   -31.85
 22  b_up            24  -153.30    3.50   |  special_up          45   -97.86     8.38
```

what it answers, and nothing else does: which frame a move's own y-speed writes land on,
what a declared stat does to a rise or a fall (game and watch's up special: the source rises
at 1.46x the written speed under its declared `air_ease` 3.5, falls under a gravity of 1.0
against its declared 1.3, and holds 3.5 as its fall cap; the port applies the write and the
gravity as declared), and where a landing happens. paced on the engines' frames, never on a
clock.

### finding a sound a character names but does not carry

```bash
PEPTIDE_SSF2_APP=/path/to/SSF2.app peptide ssf2 findsound jump_common mario_lift
```

Walks every package beside the game and says which one holds each id, so a sound that comes out
silent is a fact rather than a guess. The conversion does this itself (manifest packages first,
then every package, then the game's own movie); this is for when you want to know WHERE, or to
confirm that an id exists nowhere.

### api check (does the engine HAVE the method we emit?)

```bash
peptide "<Fraymakers>/hlboot.dat" _ apicheck
```

Every method name the mapping tables emit, resolved against the engine that will run it. A call
the engine has no method for dies where it runs and takes the rest of its handler with it, so the
symptom is a combo that stops at its first hit or a poll that never fires, with nothing said about
why. It reads both places a method name is authored: the mapping tables and the script templates.
Cheap, needs no match running, and catches the whole class before any live test. The names come
from the converter's own files and the engine is read at runtime, so this writes nothing down
about the engine.

### content-mistake guards (a mod bug should not end the match)

Some content mistakes don't raise a script error, they take the whole match down: asking for an
animation the entity doesn't have leaves the object with NO animation, and the engine dies on the
next frame the camera bounds it, deep in `GameCamera`, saying nothing about which script asked for
what.

That's the worst kind of bug to be handed, and it's the kind mod development produces all day: a
renamed animation, a label that became its own animation, a typo. So the engine Peptide patches
checks first and reports instead:

```
SCRIPTERR: playAnimation: this entity has no animation named uncrouch
```

The entity stays on the animation it was already playing. The move looks wrong, which is a bug you can
see and fix, rather than the game vanishing. The line arrives on the same channel as every other
script error, so `peptide log` and the in-game console both show it.

This is a growing set. When you find a content mistake that CRASHES rather than reports, add a
guard for it beside `inject_animation_guard` in `src/main.rs` and register its symbols in
`manifest.rs` under `guards` (non-critical: if a future build moves them, the guard is skipped and
the engine behaves as it always did). Guards belong on the one door every script goes through, and
should ask the engine's own question (`hasAnimation` for `playAnimation`) so the check and the call
can't disagree.

Guards live in the patched engine, so they cover YOUR test sessions. A published `.fra` runs on an
unpatched engine, so the converter must still not emit the mistake: the guard is the safety net,
not the fix.

### script-error scan (does the translated code actually RUN?)

`tools/tests/script_error_scan.sh <id> …` converts, exports, then drives the character through
every `CState` and collects each trapped hscript error into a per-character signature:

```
SCAN_OUT=/tmp/before tools/tests/script_error_scan.sh sandbag falco   # baseline
…converter change…
SCAN_OUT=/tmp/after  tools/tests/script_error_scan.sh sandbag falco
diff -r /tmp/before /tmp/after
```

this is a different question from the spawn sweep. `batch_spawn_test.sh` asks "does it launch
and move"; Fraymakers TRAPS a script error, logs it, and carries on with that frame script dead
from the failing line onward, so a broken handler passes a spawn test and looks fine from the
outside. the signature is error text only (no counts, no positions), so two runs of the same
build match exactly and a diff is all real change.

two things the driving has to get right, both of which produce convincing garbage if you skip
them: **park between states** (driving FALL or an aerial leaves the fighter airborne, it drifts
out of the blast zone and dies, and every later action then errors on frame 1) and **spawn two
characters** (the `scenario` park addresses p0 AND p1, so a solo roster half-fails on a null p1
and floods the signature). `SCAN_EXPORT=0` reuses the published `.fra` when you only changed the
driving.

### frame recording (how long did that move actually take?)

Both engines PUSH per-frame telemetry over their harness socket and the host buffers it;
`record` opens a window and `trace` closes it, printing `<animation> xN` runs:

```
record                      # clear the buffer
seq attack:2                # drive an input
trace                       # TRACE:37 frames, 2 animations / stand x25 / jab1 x12
```

Fraymakers emits `FRAME:<animation>|<state>` from a `Character.updateGameInput` hook; SSF2
emits `FRAME:<label>|<frame>|<x>|<y>` from an injected ENTER_FRAME handler. The
run-length encoding is shared (`debug_target::rle_frames`) so neither side can format it
differently and make a formatting difference look like a timing difference.

**Compare animations, not states.** SSF2's frame LABEL and Fraymakers' ANIMATION are the
same unit; a Fraymakers STATE is coarser and spans a whole split chain, so keying on it
collapses jab1/jab2/jab3 into one run that can never line up with SSF2's per-label counts.

**Don't poll for this.** Polling races the move: a 7-frame attack is ~233ms, the same order
as the round trip between sending the input and starting to watch, so the watcher can
arrive after it's over. measured: the SSF2 recorder captured `a x7` in a window where a
concurrent poll saw only `stand`. The engine is present at every frame; the host isn't.

the `peptide` binary also exposes **read-only engine-inspection subcommands** used when
re-deriving the engine integration on a new build. their existence is noted in
[`AGENT_CONTEXT.md`](AGENT_CONTEXT.md) "engine-side knowledge is not in this repo"; the
specifics and any findings stay in local-only notes, not here.

### hit measurement (what did that move actually do?)

`speed N` runs the engine N frames a tick; on SSF2 it raises the frame rate to 30N, which
runs the same frames sooner (everything the gates measure is counted in frames).

`tick <frames>` waits that many engine frames on the engine's own clock (the pushed per-frame
line counted host-side, the same on both engines) and answers `TICK:N frames`. the pacing
primitive for anything driven from outside: no sleep sized for the slowest engine, no poll.

`hits [pN] [frames] [raw] [ctrl:frames ...]` watches one character and reports every hit it
takes: damage dealt, the launch it was sent with (distance per sample, angle with 0 = right,
counter-clockwise, the way SSF2 states a hit's direction), and the knockback the engine itself
calculated on the frame the damage lands. same command on both engines:

```
scenario 10,73 60,73                   # place the pair, both neutral
dmg p1 0
hits p1 40 down:8 down+attack:3        # p0's input timeline plays INSIDE the watch
```

- the `engine knockback` line is the number to compare. the launch line is position, which is
  the hit plus hitstop drift, friction and gravity.
- SSF2 reports its knockback in its own px per frame; x0.65 is Fraymakers units (the
  `velocity_scale` a speed converts by). side by side unconverted, every move looks a third light.
- `hits` holds the command queue while it watches, so a `move` sent after it lands after the
  window. drive the attack through its input timeline, or send the `move` first when the move's
  startup is longer than the round trip.
- `hits raw` dumps every sample (damage, position, velocity, state), which is how you read the
  hitstop and the launch apart.

### the debugger overlay (a HUD that floats over the game)

`peptide overlay` is a standalone, semi-transparent, click-through HUD that floats on top of
the running game and shows the live state (current animation) plus the `SCRIPTERR:` error
stream. it's a separate process/window, not the Peptide GUI, so it works the same whether the
session was started from the CLI (`peptide session`) or the GUI. both session paths spawn it
for you and it self-exits when the engine/session dies. opt out with `--no-overlay` or
`PEPTIDE_OVERLAY=0`. on macOS it pins to the Fraymakers window and follows it; on other
platforms it shows as a fixed-position HUD (the per-platform window-find is the open piece).

it reads the session's `out.log`, so anything the engine surfaces into that log shows up: the
error half is per-engine (Fraymakers surfaces trapped script errors as `SCRIPTERR:` via the
bytecode patch; SSF2 surfaces its own through its bridge), the overlay just renders whatever
arrives. same ONE-vocabulary-TWO-engines seam as everything else (see AGENT_CONTEXT).

### driving + observing the GUI from code

the GUI can be driven headlessly for reproducible boot testing:

- `PEPTIDE_GUI_AUTOBOOT=<verb>` fires one boot IPC after the page loads -- `<verb>` is the part
  after `@@boot:`, e.g. `quick:mario`, `regular`, `ssf2:mario`. same as clicking that button.
- `PEPTIDE_GUI_TRACE=1` echoes every incoming IPC message and page-bound event to stderr, so a
  run can be captured + read back (pair it with `screencapture` to see the result).
- `PEPTIDE_NO_POLL=1` disables the per-200ms `matchStatus` poll (isolation knob when you suspect
  the host poll is perturbing a fragile match-start).

example: `PEPTIDE_GUI_TRACE=1 PEPTIDE_GUI_AUTOBOOT=quick:mario peptide gui` boots the GUI, quick-
boots mario, and traces the whole boot flow to stderr. note the freeze class some converted
chars hit on match frame 1 is NOT visible to `peptide tell`/eval (the frame loop is hung), so
confirm those visually (a screenshot), not over the socket.

### no storefront needed

the patched engine boots without Steam running: the client init's "restart through Steam" exit
and its "failed to connect" throw are disarmed, and the match comes up with the online
features inactive. that is what a test harness wants; a Steam that has logged itself out (it
shows a sign-in dialog and quits the moment a game attaches) took the whole live pipeline down
for an afternoon before this. `PEPTIDE_NEED_STEAM=1` leaves the engine as shipped.

### 3.0 surviving Fraymakers updates (the version-compatibility gate)

every Fraymakers update renumbers the engine's internal indices, so a patcher that pins
integers breaks silently. Peptide is built so a new build is a fast, self-diagnosing
turnaround. the rules (full write-up in [`docs/PEPTIDE_DESIGN.md`](docs/PEPTIDE_DESIGN.md)
"Version resilience"):

1. **resolve by name, not index** -- `find_fn`/`require_fn`, `find_type`, `find_field`. names survive a recompile; pinned integers silently point at the wrong thing.
2. **fail loudly, never fall back to a pinned index** -- a missing name aborts the patch instead of corrupting it.
3. **prefer hscript over hand-emitted bytecode** -- logic in [`commands.hsx`](commands.hsx) runs through the engine's own interpreter and is immune to index drift.
4. **avoid mid-function opcode injection** -- use the layout-robust insert helpers or hscript.

every engine symbol the patcher needs is declared once in [`src/manifest.rs`](src/manifest.rs);
**`doctor`** is the read-only preflight that resolves the whole manifest against a given engine
file and prints a pass/fail checklist. the same check runs at the top of every real patch and
**aborts before mutating anything** if a critical symbol is missing, so an incompatible build
fails loudly instead of producing broken output. in the GUI it shows in the boot modal
("Verifying engine N/N").

**version-bump loop:** run `doctor` against the new build → for each miss, find the symbol's
new name with the read-only inspection subcommands and update both `manifest.rs` and the
matching `require_*` call → repeat until clean → then the in-engine spawn-test below. (the
concrete engine symbol names and the technique for locating them are compliance-sensitive, so
keep them in local notes only; see the boundary note above.)

### 3.1 headless content load · 4. engine symbol map

> **not in this repo (compliance).** this repo doesn't document how custom content loads into a
> headless engine, or the engine's internal **symbol map** (the named move-dispatch /
> telemetry / load functions, type/field layout, `CState` integer values, the live-character
> access path). to respect Team Fray's wishes, that material -- specific non-hscript engine
> class/function/field names and the decompilation technique -- stays out of the tracked repo.
>
> to orient without it: the engine surface the harness uses is enumerated in code. read the
> **`MANIFEST` table in `src/manifest.rs`** (grouped by subsystem, each entry annotated with
> what it's for) and the **`connect_edit`** patcher in `src/main.rs`, which consumes those
> symbols. re-resolve anything that moved on a new build with the read-only inspection
> subcommands (`doctor`, `inspect`, `fnsof`, `dis`, …), and keep deeper notes in the gitignored
> `docs/` scratch space. full pointer: [`AGENT_CONTEXT.md`](AGENT_CONTEXT.md) "engine-side
> knowledge is not in this repo". the drive-and-read validation walk itself (`LAUNCHED → STAND
> → drive a move → read state → match-live`) runs through the `spawn` + hscript commands above
> and shows up in the validation status below.

---

## 5. in-engine validation status

stop-condition for "a converted character is validated" (the original 6-criterion mandate,
sandbag as the reference character):

| # | Criterion | sandbag | mario |
|---|---|---|---|
| 1 | **Conversion clean** -- exit 0; `conversion_log.json` triaged | **MET** | **MET** |
| 2 | **FrayTools layout match** -- `compare_boxes` in tolerance | **MET** (§6) | not re-run |
| 3 | **Engine boots + spawns**, no crash | **MET** -- spawn → `T:STAND` | **MET** -- spawn → INTRO → STAND, stable |
| 4 | **Every move runs** via the internal control path | **MET** (spot-checked across categories) | **MET** -- full moveset driven |
| 5 | **Animations play** (per-state `ANIM:` stream) | **MET** | **MET** |
| 6 | **Physics within tolerance** of `CharacterStats.hx` | readback live | readback live (dash_attack moves x −130→49) |

**both reference characters (sandbag, mario) drive their full movesets in-engine, verified
live.** every state produces the expected `ANIM:<STATE>` transition + `M:OK` and recovers to
`STAND`, no crash. the generic load+drive pipeline is validated broadly across the roster too;
per-character coverage lives in [`docs/STATUS.md`](docs/STATUS.md).

**a callback the source hands to a timer or an event is the function it names.** a charge
smash's `effects` is both the clip's effects array and the clip's `effects()` method; the
handed name goes to the method (its own clip's copy, `effects__DSmash_223`), and a callback
held in an instance var goes through `__cb`, which hands over the closure it holds or a
function that does nothing while the slot is empty. nothing is neutralized with a TODO.

---

## 6. validation template -- sandbag box geometry

the reference sweep, the template for the rest of the roster. convert clean, then:

```
peptide harness \
  --project "$PWD/characters/sandbag/sandbag.fraytools" \
  --entity entities/Sandbag.entity --animation <anim> --frame <n> \
  --out-json /tmp/box.json --port 9222
cargo run -p ssf2_converter --features dev-tools --bin compare_boxes -- \
  --ssf2 ../ssf2-ssfs/sandbag.ssf \
  --char sandbag --json /tmp/box.json --tolerance 2.0
```

(`compare_boxes` is a `dev-tools`-gated diagnostic bin.)

FrayTools renders only **static** collision layers (hurt / item / body); hitboxes are runtime
script data and don't appear here, so they're validated at runtime instead (§5).

**result (current output):** every hurt / hit / body box converts sub-pixel (0.000–0.001 px
drift) across the move set (idle, jabs, tilts, strongs, aerials, grab, specials), 0 FAIL.

**itembox -- the one to watch.** the `itemBox` is the only routinely rotated collision box
(placed relative to the hand attachment point, with `pivotY = height` so it rotates about the
hand). its rotated-affine anchor bake (`entity_gen.rs`) lands at <0.01 px drift in current
output. it's gameplay-noncritical (item pickup range, not hit/hurt detection), but since it's
the trickiest geometry it's the **first box type to re-check** if anything looks off. `peptide
harness`'s `rendered_anchor` is FrayTools' ground truth to bake against.

---

## 7. pitfalls

- **stale-output trap -- verify you're debugging FRESH output.** the converter **merges into**
  whatever already exists in `characters/<id>/`; it doesn't wipe the dir first, so files from
  old converter versions (renamed-away paths like `scripts/Character/Script.hx` with a broken
  loop, old `library/sounds/*.ogg`) silently persist. before trusting a runtime result, confirm
  the `.fra` you published came from a fresh regen (check
  `characters/<id>/library/scripts/<Pascal>/` mtimes; the dir should hold *only*
  current-converter paths). a full corpus purge once removed ~7,300 such files. *prevention
  idea:* have the converter `library/`-scope clean each char's output dir at the start of a run.
- **engine builds shift the integration target.** a new Fraymakers build can move the engine
  symbols the harness depends on, so the harness re-resolves them by name at patch time and
  fails loudly if one is missing. the how-to for re-resolving is compliance-sensitive and lives
  only in local notes, see [`AGENT_CONTEXT.md`](AGENT_CONTEXT.md) "engine-side knowledge is not
  in this repo".
- **wedged `hl` processes.** orphaned `hl _conn.dat` procs from earlier boots occasionally
  wedge into uninterruptible sleep (`ps` STAT contains `U`, e.g. `UNE`); `kill -9` can't reap
  them and they only clear on a full reboot. a wedged orphan does **not** block a new engine
  from launching, so the "kill prior instance" step gets a hard ~2-second budget, then move on.
  don't loop / retry / wait it out.
- **stale `.fra` haunting -- verify the LOADED file, not just the source.** converter fixes
  silently never reach the game until every affected `.fra` is re-exported. before judging any
  in-game behavior: check `ls -la <install>/custom/*/` dates against the fix date, and grep the
  install `.fra` for a marker only the new build emits (`grep -ac <marker> <install .fra>`).
  a week-old character `.fra` once presented as a fresh animation-mapping bug ("spinning
  falls"), and a stage `.fra` raced an export twice and faked two thwomp regressions.
- **serialize session tests.** overlapping launches make the log reader match the PREVIOUS
  session's `out.log`. the discipline: kill `peptide session` AND the engine process, sleep a
  few seconds, regen + export, verify the install `.fra` marker, truncate `out.log` in the
  FOREGROUND, then launch once and wait for `LAUNCHED`.
- **silent engine-launch failure after heavy cycling.** occasionally a launch produces no
  engine process and an empty `out.log` (distinct from the harmless wedged-`U` orphans above).
  kill the session daemon, wait ~12 s, retry once; it has always come up on the second try.
- **live probing without a debugger.** the `.fra` embeds hscript SOURCE, so same-length text
  swaps byte-patched into the install `.fra` make cheap probes: rename a call (`inc()` to
  `inq()`) and the SCRIPTERR line number proves the statement runs; replace a condition with a
  padded `true` to test a branch body. for value probes, encode state in position
  (`self.setX(800 + bitmask)`) and read it through the `tree` command.
- **output is not the deliverable.** `characters/` is git-ignored, so converter output never
  shows in git. commit `src/` + docs changes, and re-derive output by re-running the converter
  (deterministic GUIDs make regen idempotent).
