# Peptide status, known issues, and next steps

where the SSF2 to Fraymakers conversion stands across the roster, plus the live list of
open issues and what's worth doing next. this is the one place we track status and TODOs
for the whole project, so known issues and next steps live here instead of scattered
around. the converter status is up top; the Peptide (engine-harness) feature TODOs are at
the very bottom. if you want to verify any of this yourself,
[`PEPTIDE_GUIDE.md`](PEPTIDE_GUIDE.md) shows you how.

## the converter's next shape

the converter is moving to a design that keeps the source's shape: one source model, a compiler, a
Haxe runtime and an api table, with one emit path for every kind of object and no code stored in
entity json. it's set out in [`CONVERTER_DESIGN.md`](CONVERTER_DESIGN.md). every fighter
converts on the runtime's path; stages are still on the text path, where parity work is paused.
new fixes go into that design, where each one is written once.

| step | state |
|---|---|
| call costs, calls between objects, decompiler scan | measured |
| 1. source model; decompiler pure and complete | done: every class of the 47 packages reads, nothing unreadable (`tests/source_model.rs`) |
| 2. compiler and language suite | done: every class of the corpus compiles and parses (`tests/compile_corpus.rs`); the 60-case language suite passes in the engine (`tools/tests/lang_suite.py`) |
| 3. runtime core, one emit path | done: fighters, their projectiles and their items share one emit path, the only one a fighter has |
| 4. projectiles and items through the same path | done: declared projectiles and items run their own code on the runtime. an item made from another package (sandbag's bob-omb, the capsule) keeps the current pipeline's script and is reported as a gap; the converter never compiles the game's own files |
| 5. coverage | the whole roster runs on the runtime's path: 48 of 50 fighters pass all six gates (`python3 tools/verify.py char <id> --ssf2`); gigabowser and wario man pass C1 to C5 (SSF2 cannot start a match as a transformation). measured on the source's own record: the source updates fighters before projectiles, so a projectile a fighter's code makes (pichu's cloud) starts its clip that frame, and one another projectile's code makes (the cloud's bolt) starts the next frame; a projectile another projectile's code makes, directly or through a call on its maker, starts a source frame late here too (its animation paused for one) and belongs to the fighter, and pichu's down special runs the source's frames. zelda's swap to sheik (`replaceCharacter`), wario's final form and `replacePalette` are runtime gaps (logged when reached, not errors). `hitTestPoint` answers from the stage's floors (a point under a floor's top is in it; pichu's thunder jolt climbs as the source's does) and camera `darken` dims the camera's background layers an enemy a fighter makes by its class (waluigi's plant) runs that class, found by the clip its own stats name, and lives as long as its code keeps it (a projectile's default lifetime is not its own). |
| 5b. direct translation | in progress: API calls are written at conversion (`runtime/api_calls.jsonc`): a Fraymakers call where there is one, else a function on its class called with the object; objects are data, with no methods. the engine plays the animations and each frame's script is written on its keyframe (a frame the runtime enters or a jump names stays its clip's method, noted on the keyframe). each script carries only the runtime it reaches. |
| 5c. no clock of its own | done: the source's frames are the engine's doubled with nothing counting them. timers are the engine's (`addTimer` at twice the frames, repeat 0 as forever, removed by function); the code the source runs every frame runs whole, as written, on every second engine frame |
| 5d. rollback | done for fighters: content's fields are the engine's state containers (pooled, made first thing at the script's top level), arrays and value objects a container holds are changed as copies, and the runtime's own state is kept a record a frame against a tick container, only in a match that can roll back. in a local match that rolls back (`netcode = 3`), every frame it replays is checked against its first playing: mario 9 of 2788 differ (the same on the build before the stage-target work), kirby 0 of 1413, megaman 2 of 1386 (the runtime a frame late following an animation the engine changed). offline, mario runs at the speed it did without any of it (`tools/tests/rollback_parity.py`) |
| 5e. what a move does, frame by frame | the C6 path check (`compare_paths` over `c6-paths-<id>.json`) found moves that last the right length and go the wrong way; fixed so far: a timer ticks the source frame it lands in, ahead of the movement, so one made in a frame's script comes due two engine frames sooner than one made in another timer (a one-frame one runs at once) and one made in an update a frame sooner (falcon's kick moved on its timer's speed a frame late; mario's up special launched a half step short); an x speed written, or carried onto or off the floor, is held a step and a half of the floor's or the air's slowdown short (the source slows it before it moves, this engine after), and a speed an attack keeps a share of is held at 2m/(1+m) (falcon's kick, bandana dee's slide and ganondorf's dive land on the source's pixel); a fighter's attack caps a speed its timer wrote at its air ease before the engine moves it (ganondorf's dive fell 1.5 of his gravity too far a frame), and the frame of gravity a speed its code writes is held ahead stops at the ease, past which the source adds none, while the speed written stands (simon's up special writes its ease and then the same speed: he swung a frame of gravity lower each frame, and now follows the source's arc within a pixel; dedede's up special still dives past its own); the declared slowdown and steering are put back after the engine's own entry of the state; a fighter's listener ends with the state it was added in unless it persists (a landing listener ganondorf's side special left behind landed his down special); an attack's code changing `cancelWhenAirborne` or `canFallOff` reaches the engine; a character's base scale is the size multiplier alone, since the root clip's placement scale is already in its art and boxes (bandana dee, placed at 1.4, drew and hit 1.4 times too large); a jump made in a timer or an event moves the engine at once (pac-man's side special looped on its hold); stats an attack's code rewrites mid-move take effect at once (peach's side special never slowed); a grounded attack goes off a floor's edge unless it declares `canFallOff` false (yoshi's egg roll stopped at the edge); a projectile's `playFrame` goes to its own clip, a jump back to the start of the animation it shows starts it again (bandana dee's spear ran on past its `loop` into `continueGrabbed`), and one touching its owner raises `projCollide` (link's boomerang is caught); the taunt button plays the neutral taunt's own section |
| 5f. native at conversion | attack stats are the engine's AnimationStats (declared and literal mid-move writes), label jumps from a clip's own code are `playFrameLabel`/`playAnimation`, a pure landing listener is the engine's `TOUCH` landing into the move's landing animation (mario's forward air lands and stands on the source's frames), and a declared `cancel` is generated into the move (kirby's stone leaves its hold on the source's frame, and ignores a press before its clip is waiting). steering is the move's `allowMovement`, with no ground acceleration where the move turns ground steering off. a listener for an event the engine raises is the engine's own listener (`runtime/events.jsonc`). an autolink is the engine's: the move's hitboxes take the autolink angle and all their launch from the engine's pull, and the animation carries the point (sonic's up tilt launches 0.64 of the source's against the 0.63 a speed converts by, at 127 degrees against 106; tails' 0.57 at 170 against 159; the engine measures the victim from elsewhere than the source does). the source engine's own states are their animations' (`runtime/engine_animations.jsonc`): mario's back jump plays its backflip and falls from the top, his back double jump falls instead of holding its last frame, a revived fighter waits in the air until a control or five seconds, an air dodge steers, and every ledge action hangs the fighter with the ledge at its origin (it hung 60 low). a dodge's intangibility is its animation's `bodyStatus`, so a roll off a ledge ends it. the up special lands heavy after its helpless fall (`fall_special` is the root's helpless, else its fall; it was the screw attack item's), `shield_out` is the raise run backwards, `run_turn` is the skid, and each jab carries the box stats its first frame writes (jab 3: 45 degrees, growth 76). a box's angle is the source's as written: both engines mirror a reversible one only for a victim behind (eighteen up and forward tilts that lean back at 95 to 110 degrees were written mirrored, and leaned forward). left for the runtime: a steering, stat or point the code works out as it plays, a kept share of speed, engine charge and combo, and label jumps made outside a clip's own code. a projectile's script carries only the per-frame pieces its declaration or the code of its clip tree names (`emit::projectile_frames`: floor start, trail, homing, steering, rotation ..), its death bounds are the engine's `deathBoundsDestroy` with the source's end run from `onKill`, and a holder asks its item through the item's exported `__ask` instead of a field the item polls a fighter, projectile or stage object whose clip the engine can play as its own (its animations show the clip's frames, a section played backwards built backwards and a lead-in laid on after it) has each frame's code on its keyframe and no playhead of the runtime's: its code reads `currentFrame` and `currentLabel` off the engine, a jump the compiler cannot name goes straight to the engine (passing through another animation's first frame without running it), and its update moves it on to the next section once an animation is played out (`DATA.sections`; a projectile's animation that ends with `AUTO` ends the projectile). a stage object whose clip qualifies the same way runs the same way (jungle island's monkey-ball platforms). the clip's first frame is on its animation's second engine frame as well, run there only as the object is made: the source runs a new clip's first frame at the end of the frame it was made in, after its maker's code (megaman's water wave is sent on first and never runs it; jungle island's platforms name their collision role there, after the stage has set them up), where this engine runs a new object's first keyframe while making it. an object whose animations show several clips (each the clip its own clip places under the animation's label, as a fighter's states are) runs the same way: entering an animation puts the object's clip on that label first (`enterLabel`), and a clip with children has its record put on each frame before the frame's code runs (`clipAt`): 114 of 162 projectile scripts across twelve fighters, the rest having no clip of their own (kirby's copies of other fighters' projectiles). a fighter runs the same way: an animation the engine starts is entered at its next update (the entry keyframe's code waits for the engine's state, as a walk's starting speed needs) unless the engine has already gone on from it in that frame (a special fall that touched down), one code starts, its own or another object's, is entered at once (link's caught boomerang sends him to the catch frame of his side special, samus's bomb into her rocket frame), a root label code names plays that label's animation, and a clip made anew runs its first frame even where its animation starts it later (mega man's fall). |
| 6. stage objects through the same path | done, and the default (`PEPTIDE_STAGE_TEXT=1` keeps the stage pipeline's scripts for now): the model finds a stage's class and its objects, and the sample stages' code compiles. a stage's boot (structures, objects, placed bodies, backdrop elements and their clip groups, camera background opacity, weather) is data, `DATA.stage.boot`, made by `runtime/stage/boot.hx`. the stage's script is the runtime's (`emit/stage.rs`, `runtime/stage/stages.hx`, `runtime/entry/stage.hx`): the class the package registers as its stage runs on the clip that is the whole stage, with the boot, its regions and the verify counters in the same places (battlefield and battlefield2 pass the live gates that way). its planes are that clip's own children on their own timelines (`runtime/stage/planes.hx`): a named clip is bound to the elements the stage pipeline drew under it (`DATA.stage.elements`), and what the code does to it reaches them, while an element nobody's code sends anywhere keeps the engine's loop. surfaces by name, regions, ground friction and push, surfaces riding a picture, library clips and the camera backgrounds' clips are there too, and the weather is the stage's own weather class drawing into the weather clip (`runtime/stage/weather.hx`). each object with a class of its own runs that class on its custom game object (`runtime/entry/object.hx`, `runtime/stage/bodies.hx`): its pieces are its entity's animations as its clip's labels, the engine starts it on the one its clip's first frame shows, a placed body is the outline its clip draws and is there before the stage's own code asks for it by name, the floor its code makes is a floor (or a solid box when asked as terrain), and what it declares (`ownStats`, `getAttackStats`) is read as it runs: surviving the bounds, no launch, gravity, the hit refresh. `makeEnemy` makes the content its class converted to (`DATA.pkg.objects`), and the maker holds a stand-in with the class's members until the object's script starts. places in a stage's code and its objects' are the source's stage space (`DATA.origin`). flatzoneplus, huecomundo, fourside, bowserscastle and bombfactory convert that way with the same `verify.json`, every script parsing and no unknown call left. live with `--ssf2`, all ten sample stages and kingdom2 pass every gate but butterbuilding's S6 (its tower climbs and carries the source's fighters up with it; the climb is not ported); S7 times a loop length by any pair of clips that behave alike on both engines (clock or code), so a code-driven clip sorting first no longer reads as half rate. an object's `onHurt` is handed the hurt as a listener gets it, unless its class already listens with it. measured live: an object's script is already running when the code that made it gets it back, and a method another script calls runs in the object's own scope (`Bodies.direct`). each weather field's particles are the stage's own `m_weather<n>` list, and the walk over them reads a clip's place as it stands, since a field is hundreds of clips a frame (crateria still runs hotter than on the text path: its weather class moves every particle in script). crystalsmash stays on the stage pipeline's scripts (its enemy's class is another package's). left for it: bowser's castle's platforms (a structure's class: the thwomp lands and nothing sinks), a ledge the code moves, and the engine's spawn points and item and smash ball areas (gaps). a method run both every frame and once (a helper the update and the start both call) steps by the runtime's step as it runs. a projectile a stage's object fires runs its own class too: what the stage's classes declare they fire (`getProjectileStats`) is every stage script's `DATA.pkg.projectiles`, and each projectile shipped for them gets the runtime's script, its entity's frames run by the runtime (kingdom2's birdo eggs fly; no sample stage fires one). fighters on a converted stage read places in the engine's space, as the stage's regions and camera do for them, so nothing they compare is off; the stage's origin reaches only the stage and its objects. a platform's path its code gives (`addMovement`) moves it, a ground its code makes icy slows a fighter on it, the stage's `setInterval` runs on its timers, its camera backgrounds are its own clips with what they hold, and anything a clip places under a name (a text field) is in it: yoshi's story and devil's machine pass all seven gates, galaxy tours all it passes on the text path. a stage's own `initialize` runs once there are fighters (the source's asks for its players), a clip's named children answer a name built as the code runs (`clip["region" + i]`), a bitmap a timeline places is a `Bitmap` whose data is its library class (`getQualifiedClassName` reads it; drawing into one is a gap, this engine draws no pixels), the stage's light clip is the code's to hold (the engine's light stays put, a gap), an object with no clip runs on an empty one, and an enemy a helper class spawns ships: race to the finish passes all seven gates. `tools/tests/unknown_scan.py` converts every installed stage and fighter and tallies what the code still asks for that nothing answers: fighters are down to none, stages to galaxy tours' surface friction (answered as it runs). what that scan turned up is answered: a body's box (`getBoundsRect`, its declared size over its feet), a clip's box in another's space (`getRect`), dates (no clock here: "now" is one fixed moment, a gap), plain-text patterns, the platforms list, an enemy's fade (a tenth a source frame, not measured), the stage's own effects and timers, and the players list (`playerSettings`, no tags); vector drawing, pixel drawing, the smash ball and the death-bound switches are gaps. the bodies a stage's tree places are put down once the stage's clip is there (their code asks it for its planes), and a loop's own counter steps a whole step a pass (urban champion's player loop never ended). a call on an object to a method its runtime base keeps as a static (`PeptideStage.x = function(me, ...)`) is that static, handed the object, and a platform object's own faces are the object when a fighter's floor is asked. a weather clip the stage's code leaves empty this match (smashville rains on a roll) is the code's choice. a plane the stage's code sends to a frame plays on from it and holds only where its own frame scripts stop it (measured on castle siege and galaxy tours with `tools/tests/stage_watch.py`; castle siege's statues spawn from its background's frame 30). a clip placed under a surface's name switches that surface by its `fallthrough` (castle siege's phases). an object's stance splits into one animation per label, named under the label that places it (mushroom kingdom 3's piranha plant attacks and dies; saffron city's pokemon each run their own `entrance` .. `end`), a frame script on a frame no animation shows stays on the clip, and an object plays a lead-in before its first section out itself (its frames set it up: the piranha plant's `self`). a projectile any code fires by its class (`spawnProjectile(CharmanderFire, owner)`) ships under that class's name from its own stats. a plane's segmented elements follow it into each labelled section however it gets there (its own play or frame script, not only the code's jumps), and S7 reads a plane the stage entity draws by the port's own plane clip. stages that pick at random (galaxy tours' galaxy, pokemon colosseum's stadium) differ run to run on S6/S7 by what each engine rolled. every display object has a name (`instanceN` where none was placed, as Flash names it), and a placed bitmap is a display object. the scoreboard (eight gates, `--ssf2`): every installed stage run passes every gate but these: galaxy tours, pokemon colosseum and crateria (their random rolls decide S6 or S7 run to run), saffron city (S8 counts its charmander's one error, which the source raises too, below), and the mode stages (rest area, and the one in DAT61, whose source classes throw outside their mode); butterbuilding's and crystal smash 2's S6 skip (their stages carry the fighters off before they rest); the fighter roster is 47 of 50 on all six gates (gigabowser and wario man are transformations the source cannot start a match as; captain falcon's down special runs long on C6 when it meets the other fighter on one engine only, and passes on a rerun; fox's side special after a walk reaches the ledge mid-dash and grabs it here at 47 engine frames, where the source dashes its full 32 source frames past it with the same ledge box on its frames; meta knight's side special after a walk runs 57 source frames in the source and longer here, 127 engine frames at v0.8.1 and 138 now, where 114 match). a clip only collides as a body when it declares a collision role (`platform`, `terrain`, `collision`): an enemy or a target is a thing to hit. a hazard whose clip carries only a body box (`hitBox`, no `attackBox`) is hit on it and hurts no one by touch (konoha's lanterns swing when struck and a fighter dropped through one lands untouched, as in the source), and a backdrop clip that ends on the picture it starts on loops on its own length (konoha's hinata, 268 frames, was cut to 265): konoha village passes every gate. S7 files a clip under an unnamed one by that clip's own named ancestor (jungle island's five-frame clips sit in three anonymous 140-frame ones, not in its bananas): jungle island passes every gate. a clip a stage declares as a target is the game's own target, `PeptideTarget` (`runtime/api/objects.hx`): it hangs where it is put (no fall, no launch, no end at the bounds), breaks when struck (its stance plays `break`, then it is gone and the stage hears `targetBroken`), answers its state to the stage's code, and the code reaches it by its clip (`getTarget`). a placed object the stage's code reaches by its placement name is shipped wherever it is parked (target test 4 parks its targets off the stage and moves each along a path). a frame jump on a named clip reaches an element under it that only rides that clip's timeline, and the stage's report marks a picture its code holds on a frame, which S5 reads as the code's. a piece's loop is the length every clip down its chain comes round on, named or not (target test's sliding platform is an unnamed 105-frame clip that starts and ends on the same 25 frames, and an 80 fitted it). a clip's timeline puts each child where its frame does (place, scale, turn) until the code sets one of those, as Flash hands a placed child over, and a plane's elements move only by what the code adds to that (target test luigi's damage zones stand where their markers do). several placements of one class under one name (black mage's ten targets, each placed as `stance`) are each their own content. a fighter lives in the stage clip's own space, so where the document places that clip scaled (thirteen of the target tests, at 0.942) the art and collision are read with that scale divided out. all 27 target-test stages pass every gate. a floor an object makes from a clip keeps that clip as its own (nintendo 3ds's tetris blocks hide it through the floor), what the stage fires is the stage's own, and a hold by no fighter (rainbow route's mirrors take a fighter in) is a stated gap: both pass every gate. a surface the tree placed under a name has that clip as its own, so the code shows and hides it by its clip (hylian skies swaps its light and dark terrain), and the track playing is the stage's own from the match's start, which final destination times its phases by: both pass every gate. a section whose label the script language keeps for itself is played as that word with `_` after it (waiting room's `default`), and an object's clip sits in a display list, so its code can order itself there (warioware's tree): both pass every gate. a clip's extent is carried for every linkage a name the code builds starts as (sand ocean's vehicles ask for `"terrain_" + name`; without it each vehicle's floor was none and its code failed every frame, which starved the gates' reads): sand ocean passes every gate. a clip class the code constructs (`new k1_HUD()`) has its symbol's timeline, its first frame's children there once it is made, and a fighter's name it states none of is empty text: kingdom1 passes every gate. a field the class never types but only ever gives whole numbers (a counter) compares and counts as a number, and a whole-number key reads an array in place, the general read kept for anything else: krazoa palace's code walks a 182 by 23 wind picture every frame with two such counters. a drawing call this engine does nothing for (`runtime/inert.jsonc`: a picture's `copyPixels`, `setPixel32` ..) is not written when its arguments do nothing on their own, a test that comes to nothing is dropped, and a loop whose only work is counting to a bound gives the bound at once: krazoa palace runs at 61% of a core where it took all of one (battlefield takes 37%). a named clip under a plane plays on whatever its own frame scripts stop, and its picture follows a jump its own script makes (krazoa palace's fan blades go back to frame 1 from 15 and keep turning, as the source's do): krazoa palace passes every gate. a foreground element whose clips loop on lengths that never meet (meteo voyage's arwings flicker on six frames while their flight loops on 3173) is baked over its longest chain's lap, so each lap's flicker starts where the source's does, rather than over the lengths' common multiple: meteo voyage passes every gate (its conversion takes about eight minutes). each class's static initializer runs once as its object starts, so a static it works out is set (fairy glade's piranha spawn points were null, and its stage script never loaded): fairy glade passes every gate. a jump tells the engine to follow before the landing frame's script runs, as the playhead's move stands whatever that script does. a frame script carries its own guard, so its error ends that script as the source's player ends it (the interpreter catches an error only in the function it happens in, and one escaping a frame script stopped the engine's whole update and ran the same frame again). saffron city's charmander, when its class skips `start` (where frame 22 fires the flame) and later sends it to `end`, reads the null flame on `end`'s first frame once, in the source and here; it comes up at random, and S8 counts it (the source's player keeps its own errors to itself, so the gate cannot tell one the source raises too). open: rest area runs only in its all-star mode (its code builds a class name from the mode's match count, and throws outside it on both engines); pokemon colosseum and galaxy tours pick at random. art inside a placed object's clip that never changes is a sprite in the object's own view; a sectioned backdrop piece chains its sections in its animation stats and the stage plays them with `playAnimation`, and one in a camera background is effects in that background's container, the next section played on an engine timer at the end of each (an effect's own `chain` takes the engine down), unless nothing can send it anywhere: then the section it settles into is the background's own art, or it is left out when that section draws nothing; a surface a sectioned piece carries is that piece's own structures (one per segment, all sections in one labelled animation, since a structure collides by the animation it was made with). still on the runtime each frame: art riding an object that changes (jungle island's four section-switched pieces), a picture a surface follows that is not sectioned, and the shown, opacity and colour schedules. |
| 7. delete the old emitters and text passes | fighters done: a fighter's entity keeps every frame and carries no code, and its script is the runtime's, with no text-path `Script.hx`, frame-script translation, terminator trim or text passes behind it. `conversion_log.json`'s `unknown`/`ssf2_only` now count only what the text path still translates (the runtime's own gaps are `runtime_report.json`). left: a projectile or item made from another package still gets the text path's script (step 4), and a stage's scripts still have the text path behind `PEPTIDE_STAGE_TEXT` (the script translation in `stage_emit.rs` and the `api_mappings.rs` passes it uses), which goes once the runtime has been the default for a cycle |

hits on both engines (`just hits-roster`, 48 fighters, 634 moves landed on both with the throws, the dropped aerials and the side and down specials, 74 differ: 37 aerials, 9 throws, 28 others, of which 56 show again in every one of three more plays (`--reps=3`) and the rest are the source's own run-to-run differences; a grounded move is tried at the source's distances in its order, twice at each on both, since a fighter is not always back on the floor between tries and misses every other one; the source runs eight frames a tick, since it takes an input sequence a frame at a time; a dropped aerial falls from 80 above the floor, and a back one is turned before it is parked, since the source takes a batch's commands a frame apiece; a drop or a hop and its aerial go to both engines as one input sequence, frame for frame, held past the fighter's jump squat, with the victim put at the distance in the same batch): grounded moves
pay the same share of the source's damage and launch at the scale a speed converts by. a hit aimed down at a
standing victim goes up at the mirrored angle in the source; this engine mirrors only a strong one itself
(its `rawAngle` keeps the stated angle) and slides a standing victim along the floor under a weak one, so the
fighter's runtime turns that launch up (`runtime/fighter/fighters.hx`): lucario's jab, stated at 291 degrees,
and fox's and bandanadee's downward airs now leave at the source's angle with its launch. a box the code rewrites as it runs into a fixed hit (`weightKB`) launches by the rule a declared
one converts by (ryu's down tilt, kirby's down air). `hit_parity.py` compares the angle a hit sends at as well (within 8 degrees). a projectile's hit follows the projectile's facing, as the source's does (its boxes were reversible because the projectile could be reflected, and a fast shot past the victim's middle sent it back: chibirobo's, krystal's, samus's, sora's, falco's, ichigo's and megaman's shots now match). a box's stated true or false reaches the conversion (goku's forward tilt states its boxes not reversible), and what a box states true or false that this engine has a field for reaches it (`flags` in hitbox_stats.jsonc): a knockback that does not stack (1142 boxes, most of them a multi-hit move's), a forced tumble (`tumbleType`), no DI, through shields, and a hit for grounded or airborne foes only (`physicsHitConditions`). a clip the fighter's timeline places mirrored draws mirrored, boxes and all (marth's back air), and an attack that says it `flip`s turns its fighter around as it begins; a box's `bypassGrabbed` (it passes over the fighter held) and `bypassNonGrabbed` (it hits that fighter alone) are honoured as the hit connects, the way the engine's own docs say to filter a hit (`HITBOX_CONNECTED`, `disabled`): a throw's box for bystanders no longer lands on the one held first (bowser's, falco's and dedede's four throws match the source's). an attack hits each fighter once until it is refreshed, whichever of its boxes reaches it, and its `refreshRate` refreshes it while it lasts (bowser's up throw hits six times here where it hit once; seven on the source, whose last hit is its launch). `hit_parity.py` counts every hit a move lands on each engine and reports a move whose count differs. a throw keeps hold of its fighter through its small hits, as the source's does: a box with no effect (`hasEffect: false`) deals its damage to the one held and lets the hold stand, and the throw's code gives a box its effect for the hit that throws (bowser's up throw hits on frames 0, 1, 3, 5, 7, 9 and launches on 10 in the source). zelda's, bandanadee's, samus's and ness's four throws match the source's; bowser's up throw lands six of its seven. each projectile a move fires keeps its own hits (sora's blizzard lands each of its eight, every two frames, on the source and here), and bowser's fire breath lands one flame of its stream on both. a projectile hits again at the `refreshRate` its declaration states, what its code writes to its boxes holds through every animation it plays and every refresh, a projectile reads its own declaration for its attack's stats, and a hit it lands in the frame that made it is told to its code once that code has started: ness's pk fire turns into its pillar and burns about every five frames for one, as in the source (one pillar hit more or less than the source's, which itself lands seven or eight), and black mage's meteor adds its charge to its own damage. open from `hit_parity.py`'s specials: naruto's rasengan projectile (refresh 3, two 0.75 boxes) lands three of its eight hits here and not its last 10.5, and the move lands at random on both engines when tapped; pichu's down special lands a 2-damage bolt before its 17 here where the source lands the 17 alone; mario's forward air drifts about twice as far here in `hit_parity.py`'s drop, which sends its presses with the watch's samples and so holds a one-frame tap for a round trip (pressed a frame apiece the drift matches, but the hops and aerials the sweep was matched with go off: 47 roster aerials differed against 36); lloyd's side special lands all six of its hits and its finisher, as in the source; sora's blizzard projectiles deal 1 where the source deals 0.5 after its first (the freeze they state is not it); goku's down special, a box stated at -3 striking a standing fighter, leaves at 75 on the source (the downward-hit gap above). an attack hits a fighter at most once a source frame, refreshed or not: this engine runs two frames to each of the source's, and a refresh in the first let the second land again (mario's down air hit twice in one source frame; goku's neutral special, refreshed every frame, lands on consecutive source frames on both). on the multi-hit moves `hit_parity.py` flagged, 34 differences come to 15. still apart: peach's down air keeps its fighter in reach for its later refreshes here, where the source's sends it out of reach after the first. isaac's up special fires its scoop at his place plus (his x speed, his y speed + 30): standing, that is 30 below the floor, where this engine puts it; the source's appears at the floor, level with him, and both then fall the same path (whether the source keeps a spawn out of the floor is not measured on a second move yet). two standing fighters are pushed apart by each engine's own amount (`hit_parity.py` reads the spacing on both): fox and megaman settle 30 to 36 apart here from 10 to 30, 34 to 43 in the source, and both leave them be from 40 on; two simons put 40 or more apart keep sliding apart here (69.5 from 56) with no speed of their own, where the source leaves them at 56. what drives this engine's push on simon is not found yet (his body width and standing boxes are fox's size). naruto's down throw matches hit for hit and damage for damage; its first hit (1, no launch, on the held fighter) reads a launch on both engines from the throw's own motion, and the two readings differ. samus's up air, dropped onto a fighter 34 source pixels away, lands its weak multi-hit (1) on frame 9 and its finisher on 11 in the source; here its frame 9 box falls just short of the fighter's edge and the finisher (5) is its only hit, whether hit_parity's drop starts the fall at the source's speed or (as it does, which the roster's aerials land with) half a step slower. samus's homing missile lives 171 frames here and 59 on the source: once it slows to 2 its code sends it to its explosion (clip frame 65) and from its last frame (70) back to `loop`, which here loops on, where the source's is gone as that last frame plays (what removes it there is not measured yet). a move with two boxes that both reach the fighter lands the stronger in the source (bomberman's back air 10 of 10 and 8, ganondorf's 18 of 17 and 18, ryu's 16 of 16 and 13) and whichever touches first here, half a source frame sooner: open, since the engine tells a hit one box at a time. a box that poisons (`poison` damage every `poisonInterval` frames for `poisonLength`) does here what it does on the source, one poison on a fighter at a time: black mage's down throw ends at 20 damage on both, at the same time. an attack's `hit_ease` moves nothing: on the source link's forward air (2) and pikachu's (-3) fall on through their hit and land into their landing, where holding or popping the fall kept them up for a second hit (link's forward air, sora's aerials and captain falcon's neutral air land once, as in the source; pikachu's forward air still lands a second, its first landing a frame higher here). a projectile's box turns for a fighter behind it as it states, judged from where the shot was before its frame's move (zelda's din's fire sends a fighter behind its blast back, as the source does, and the fast shots still send theirs the way they fly). a box that turns its launch for a fighter behind the attacker judges where the attacker stood before the frame's move, as the source checks its hits first (fox's illusion dashes through the fighter and sends it the way it was going, as in the source). a box that freezes the one it hits (`freeze`) freezes it here too, for this engine's length, and a projectile's box carries the same stated flags a fighter's does. an animation cut from a move (a charged move's attack, `special_side_attack`) carries the boxes the sprite parser kept under its own name and the stats of the move it was cut from: pikachu's skull bash hits as in the source, where before it flew through the fighter with no box (fifteen fighters had such a cut). a box that states `reverse_character` turns the fighter it hits around and its launch with it (mario's cape sends the fighter away at 0, as in the source). a package holding several characters (bowser and giga bowser) keeps a projectile as the character that declares it wrote it: a later one that declares nothing of it no longer rewrites it with placeholder stats. a projectile's box with no effect does not hit the fighter its owner holds (fox's down throw fires its lasers into it; only the throw's box hits it in the source), and one with an effect does (falco's lasers hit his before the throw's box, and his four throws match the source's), and one fired in a throw drawn already turned goes the way the thrower is about to face: falco's and fox's throws match, fox's back throw too: a box with no effect (`hasEffect` false) deals its damage without making the one it hits flinch, so the back throw's first laser leaves the flying fighter's flight as it was and the next two pass behind it, as in the source (fox's laser leaves a standing fighter standing; mario's cape, which states it too, still launches). `hit_parity.py` keeps each hit's frame and damage on both engines (`each` in its report). this engine turns no one mid-throw, so a throw that flips is drawn and launched already turned (its frames mirrored, its launch the other way) and the thrower turns once it is over (the back throws of fox, falco, bowser and nine others: fox's lands its victim at -162 here, -154 in the source). a clip the timeline places mirrored is also how forty other animations across the roster are drawn (most forward rolls and tech rolls, getup attacks, kirby's back air). `hit_parity.py` lands the four throws from a hold, and starts an aerial from rest 80 source pixels above the floor on both engines before it tries a hop (each engine reaches a hop's aerial at its own point of the jump). still apart there: goku's forward tilt, whose two boxes sit side by side on the same frames, so a victim in both takes the 120-degree one in some source runs and the 60-degree one in others (box 0 every run here), and sonic's up tilt (an autolink, below). `hit_parity.py` stands the victim still on the floor at 0% on both engines before
each move. an accepted gap: a downward hit on a fighter standing on the floor launches it with a different strength than the source's (naruto's down tilt reads 0.77 of this engine's at 0%), and stays as it is. a box that states no victim freeze (`-1`, or at or below zero, as ryu's `-1.8`) holds the victim
`floor(damage / 6 + 2.5)` source frames after the one the hit lands on, as the source works it out (`freeze_parity.py --victim`: twenty
tilts and jabs on fox, kirby, bowser, captain falcon and ryu; this engine's own default
held them half a frame to two frames longer), and one that states N holds it N (lloyd's side special, 1 a hit, kept its fighter moving with the attacker through six hits once it no longer froze it 2). a box that states no freeze works both freezes out from the damage it hits with, so code that rewrites its damage moves them (wario's side special, written 12 over its stated 8, holds wario 3 frames, not 2). through the attacker's freeze its attack's slowdown still runs, though it neither moves nor falls, so a speed written as the hit lands comes out of the freeze a frame's slowdown less for each frame held (wario's side special recoil slides 8 pixels back against the source's 7, where it slid 18). a launch fades across at the source's rate (this engine's own fade takes 1.31 times as much an engine frame, so the runtime gives back the difference; simon's cross sends its fighter 65 source pixels here and 70 there, the rest being the slide after it lands, which this engine stops sooner); its rise and fall are left to this engine's fade and gravity, which keep a launched fighter up a source frame or two longer than the source. a hit holds its fighter in its hurt state for the source's length, 1.05 source frames for each source pixel a frame it launches it plus 2.5 (measured on the jabs and tilts of mario, fox, bowser, kirby, link and pikachu with `tools/tests/hitstun_parity.py`), its freeze counted in, where this engine held near 12 for any launch; a fighter standing when it runs out is standing again, as in the source, where this engine played out the rest of its hurt animation (kirby's and pikachu's jabs held their fighter 10 frames, 5 on the source). those moves now land within two frames of the source, the rest following each move's own launch difference (`fighters.hx`, `ENTER_HITSTUN`). across the roster (`hit_parity.py --all`), 47 of the 658 moves both engines land differ in something, past the launches the accepted gaps account for, a reach or a random roll for most of the change from sweep to sweep (16 more land only on the source, listed under `unlanded`; 18 reaches are at the edge of this engine's reach, where the source lands the move at this engine's distance too, listed under `edges`); a move whose own code reports an error mid-watch is still measured, which lands yoshi's back throw, waluigi's neutral special and rayman's side special on both, and samus's bomb lands nothing here because two standing fighters are pushed apart past its blast before it goes off; isaac's down special (gaia) lands nothing here for the same reason: set 34 apart, the two isaacs stand 42 apart here and 36 on the source, and the pillar's widest box reaches 32 (`rehit_trace.py --proj` reads the place of both). how far this engine pushes two standing fighters apart against the source is open, and it moves every `=` spacing line `hit_parity.py` prints: `push_parity.py` sets two down at a distance and reads where they settle. this engine pushes them apart at once, the source a few pixels a frame, and they settle at different gaps for some fighters (simon 66 here and 56 there, isaac 41 and 50; mario, peach, falco and samus within 3). neither engine's gap follows the declared hip width. the source's comes near the width its stand animation's hurt boxes cover over all its frames (simon 56 and 56, mario 37 and 37, falco 45 and 47, isaac 50 and 54, samus 33 and 37); this engine pushes by the collision body the conversion makes of each frame's hurt boxes (`Body`: their span as its hip width, centred on them by `hipXOffset`): with every frame's body set to a centred W, two isaacs settle exactly W apart (10, 20 and 40 measured), and with no room between them they are not pushed at all. the offset is what puts this engine's gaps off the source's (two facing bodies set forward touch sooner). centring the body would bring the sample's gaps from 4.0 px off the source's on average to about 2.8 (simon 66 to 55 against 56, but samus and mario 3 to 4 further off), and the body also meets walls, floors and ledges, so it is not changed. tried on nine fighters (bodies centred at conversion): isaac's gaia, ness's pk flash and zelda's din's fire still land nothing and krystal's jab moves a degree, so the offset is not what keeps those apart. this engine's published api has nothing that pushes or stops a push (its fighters part as soon as their bodies overlap, the source's a few pixels a frame), so the gap is the rate, not a shape: it shows only where the harness sets two fighters down inside each other, and the moves it keeps from landing (gaia, pk flash, din's fire, krystal's jab angle) are measured mid-push on the source. `hit_parity.py` now waits for two fighters it sets down to stop parting on both engines before it plays the move: on six fighters the moves that differ fell from 20 to 8 (mario's all match, isaac's gaia and zelda's din's fire land on both), as many land on one engine only (11), samus's bomb now lands nothing here, and a few aerials land nothing on the source from where the two settle. where they settle still differs for some (isaac 50 there and 41 here, krystal 48 and 37.5): the bodies' widths, not the push's rate. a body as wide as every frame's hurt boxes together, centred (the span the source's own gap follows), tried on the same six: krystal's jab and simon's side special come right, isaac's gaia and down air and samus's up air go wrong, 10 moves differing against 8 and 13 landing on one engine against 11, so the bodies stay as the hurt boxes of each frame make them; `tools/tests/roster_report.py` reads a sweep). a fighter a hit stuns is hurt by the time the hit's events are told, as in the source, where this engine turns it after them, and a stat no character states reads as the source reads it (`canReceiveKnockback`, stats.jsonc `runtime_read_defaults`): krystal's jab pulls the fighter it strikes in with its own code, and does here too. a hit that launches nothing still counts as landed in `hit_parity.py`. a hit a fighter's code deals outright (`takeDamage`) is a whole hit, its launch and freeze as well as its damage, a growth it leaves out the source's 100 and one with no effect launching nothing, and an event names a projectile its script fired as the same object it was handed when it fired it: naruto's down throw lands its three hits as in the source (its clone's hit, recognised as the clone, sends the fighter straight up for the kick). a projectile fired with stats (`fireProjectile`'s last argument) reads them over its own for its life, and one that never says it is suspended reads `suspend` false: krystal's grenade throws as far as its charge says and explodes on what it strikes, as in the source. a projectile is drawn and struck at the scale its code gives its own clip (samus's bomb sets 1.75 on itself), a stat stated just above nothing stays above it (the bomb's fuse deals 0.00000001 so its touch sets it off). a falling projectile's first move takes its frame's pull first, as the source's does, and goes on with a step and a half more to stand for this engine adding the pull after each move (waluigi's dice, thrown up at 5, peaks where the source's does and lands on its fighter, where it rose 8 pixels too high and flew over; its fall still runs about a frame ahead). a projectile keeps its speed along the floor and through the air but for the decay it states, as the source's does, and a decay slows it to a stop and holds it there rather than turning it back (sora's strike raid stops and hangs over the fighter it drags, which it hits ten times and then throws, as in the source, where it drifted back off it here and hit eight) (this engine's own friction stopped simon's holy-water flame in two frames; it now runs along the floor and burns its fighter three times, as in the source), and a dying projectile is held still by the runtime rather than by its death animation, which can be a label it also lives in. a projectile's x speed is read and written along the way it was thrown, as the source's is, and one its own code has not turned says it faces right whichever way it was thrown, where this engine keeps the speed along the way the projectile faces (thrown left, fox's laser went right, chibi-robo's telly vision ran off from him and link's boomerang turned home at once). thrown left, simon's cross and link's boomerang and arrow now follow the source's path frame by frame. `proj_parity.py --left` plays every move thrown left, the layout mirrored so the thrower faces the other fighter. a projectile's code that reads a fighter's place in its update sees where the fighter is about to be, as the source's does (the source moves a fighter in its own update, before its projectiles' code runs; sora's strike raid rides his hand frame for frame, where it trailed a frame of his rise). a projectile's box that states a freeze of 0 freezes nothing, as a fighter's does (ness's pk fire states its own freeze 0, and its pillar's hits fall on the source's frames). the bomb's blast lifts samus as the source's does: a rocketed fighter starts the attack the source rockets it in at that attack's `rocket` frame and rides the speed and angle it was given, `decay` less a frame, until the attack ends (the path matches the source's from its first frame; the move ends a source frame late here). the attack is entered on the animation the conversion made of its label (`DATA.animOf`), and what a projectile's code writes to its boxes holds for its life and on the hit itself, even on a box that has only just come out with a new animation (the bomb's blast hits for 7 played by hand). `hit_parity.py`'s held press still never sets the bomb off (open). krystal's jab sends the fighter it pulls at 101 degrees here and 111 there: its code aims from the two fighters' places, and the angles say they stood about 40 apart here and 49 there, which is the push above. zelda's din's fire (side special) flies the source's path to the pixel and grows at its rate; its burst's boxes are out here (read off the engine as it bursts, the widest for two engine frames), and their edge stops a few pixels short of the fighter, which this engine has pushed further off as zelda walks in: the push above again. the source lands it set 64 apart (its fighter is not pushed there) and the port lands nothing. an attack's `refreshRate` refreshes it as the source's does: its frames are counted only while the attacker is not frozen, a refresh comes once the attack has landed a hit, on the count's Nth frame and every N after, and another only after a frame the attacker is free and has not just refreshed; a move pressed for counts a frame ahead of its first, a throw (begun from the hold) does not (meta knight's drill, refreshed every frame, lands one hit every three frames as the freezes go, where it landed one every two and held its fighter in; lloyd's side special and bowser's up throw land on the source's frames). a box that passes over the fighter held (`bypassGrabbed`) passes over the one the attack has just thrown too (sheik's and lloyd's throws struck it again with the box meant for those standing near). the source also counts toward a refresh the frame after a hit, while the attacker is frozen, and the frozen attacker's boxes still hit; this engine runs nothing of the fighter while it is frozen, so that refresh comes as the hit lands (meta knight's drill lands its second hit the frame after its first, as there). bandana dee's neutral special (refreshed every four, the attacker frozen one) lands that second hit here and not on the source, where meta knight's drill (every frame) does; what tells the two apart there is open. jigglypuff's down air lands its hits on the source's frames (0, 1, 4, 7, 10 on both); how many more it lands as it falls to the floor follows the hop, as every aerial's count does. a fighter's code that asks how long its controls have been held (`getSmashTimer`, telling a flick from a hold) is answered as the source counts it: 0 on a frame nothing is held but while landing or skidding, one more on any other, and out of reach once an attack ends or the fighter lands, where it read as never pressed: samus's side special fires its power missile off a fresh press, as in the source (it fired the homing missile every time). naruto's rasengan (neutral special) keeps its charge between uses (`gvar p0 NarutoNSpecCharge 75` sets it full on either engine), and charged full it lands as the source's does: its hit, the rasenshuriken's ten grinding hits on the source's frames, and one burst. `hit_parity.py` plays an aerial off a short hop, and on the source it names the move after a round trip, so the source's aerial starts wherever the trip lands (lloyd's down air at the top of the hop, about frame 8) while the port's is pressed on frame 3 of the hop (`hop_trace.py`: the two hops rise within a pixel of each other). pressing both on the same frame (`HP_AIR_SEQ=1`, `HP_AIR_AT=N`) moves which aerials land more than it settles them (on eight fighters' aerials: 14 differing as played, 21 at frame 3, 16 at frame 7), so the multi-hit aerials' counts (lloyd's, wario's and jigglypuff's down airs) are read against when each was started, not taken as the port's a projectile's code is told of its touchdown with the speed it came down at, and a bounce it gives itself there holds once the engine has settled the landing; a touch as it is made, short of where it is fired to, is no landing: waluigi's dice bounces once and lands its hit as in the source (its roll differs run to run). a fall cap a projectile's code writes (`maxgravity`) reaches the engine, 0 holding it where it is (ness's pk flash bursts and hangs; it fell on here, and landed by falling onto the fighter, where the source's lands from where it hangs: the hanging burst misses here as the two nesses stand, which is the push above). a projectile fired into the floor its fighter stands on starts on that floor, as the source's does (isaac's up special fires its scoop 30 below his feet, and it sat inside the stage here). a projectile's art and boxes sit where its clip places its stance (isaac's down tilt thorns were drawn and boxed 61 px below the floor, and hit nothing; mario's fireball box sat 9 px ahead and 10 up of the source's). `tools/tests/bounce_trace.py` shows a launch frame by frame. a fast shot
reaches from another distance on each engine (chibirobo's, krystal's, samus's and sheik's neutral specials:
each engine steps the shot past a different set of distances), game & watch's chef lands a pan and a sausage
in one source frame (its sausages fly at random), and isaac's up tilt reaches farther here. 31 of the 35 are aerials: each engine starts the aerial at a
different point of the hop and lands a different box of it (sweet against sour spots), so the aerial
rows are the harness's until it lands the same box; the boxes themselves are checked by
`tools/tests/parity_check.py`.

the scoreboard (`tools/tests/scoreboard.py`), fourteen fighters on the runtime's path: every one
passes all six gates and soaks clean; states off 0 but for bowser (1) and pichu (2), freeze off 0 or 1,
timing off 0, rollback replays off 0 to 4 of about 1400 each. on the text path the same twelve
it shares: eleven pass all six gates, and every one is four freezes off. what is left is mostly
the harness's (an item the source's record does not count as a projectile, a grab that closes
on nothing on one engine) and a few moves: pikachu's thunder jolt after its first hop (its first arc matches to a pixel or two; on landing the source runs it along the floor for two frames before it hops again, and here it bounces straight off, higher); all 34 specials that fire projectiles match on `tools/tests/proj_parity.py` (with several out, both records follow the newest, as the source's does: sonic's afterimages), and `tools/tests/proj_trace.py` lines them up frame by frame,
link's bomb, peach's and megaman's odd hit.

open on the runtime's path: what rollback does not reach yet. a class's statics (plain values), a
field a class adds at run time (not declared), a value object two fields share (a copy parts
them), the fields of a content object that is neither the script's own object nor a clip, and
state another script writes on this one are kept as plain values. in a match that rolls back, a
fighter's script costs about as much again for its records: mario's first neutral special,
tapped for one tick, was lost while the match waited on the other side's inputs, where the stock
fighter's was not.

## coverage

**the whole SSF2 roster converts and drives in-engine. nothing's currently broken!**
every character:

- **converts.** the converter exits 0 and produces output, with no hard-conversion failures.
- **drives in-engine.** it spawns in Fraymakers via Peptide, reaches `STAND`, and runs its
  moves without crashing (P0: `LAUNCHED` + `ANIM:STAND` + `M:OK`).
- **runs its moves with no script errors.** all 45, every one converted fresh, published,
  and driven through sixteen inputs covering the ground and air moves, the specials, shield
  and grab. nothing on the SCRIPTERR channel and no crash lines.

  booting clean proves nothing on its own: a character reaches `STAND` and then dies the
  moment a move runs, so the sweep drives the moves rather than reading the boot. the loop
  is convert, `peptide pack`, `peptide session --char <name>`, `tell seq <inputs>` per
  move, then read the distinct SCRIPTERR lines out of `~/.peptide/session/out.log`. two
  traps worth knowing: a single check right after one forced death reads as clean when it
  is not, and a conversion that fails leaves the PREVIOUS build in place, so the sweep runs
  happily against stale content and reports errors that are not in the source any more.
  check that the conversion succeeded before believing the sweep.

  `tools/check-emitted.py` reads every emitted script for what the engine would refuse to
  parse. a script it cannot parse is dropped whole and silently, and the first symptom is a
  frame reporting an unknown variable for a name that is right there in it.
- **carries every hitbox value the source declares, mapped correctly.** all 45 report PARITY
  OK from `tools/tests/parity_check.py`: damage, angle, base knockback, growth, hitstop and
  hitstun all present and converted by the one table in `mappings/character/hitbox_stats.jsonc`.
  1,086 of 1,277 stat-moves carry active frames (85%, 54-97% per character); the rest are
  projectile specials and throws with no melee box to carry.
- **plays every animation at exactly twice its source length.** 7,324 animations across the
  roster, none off the 30->60fps doubling, and 1,506 hitbox active windows exactly doubled
  (`tools/tests/timing_parity.py --all`).

- **passes the character gates.** all 50 fighters (the roster with its transformations),
  converted from the installed game data (`build/verify/ssf2-data.json` maps ids to
  `DATnnn.ssf`), pass `tools/verify.py char <id>` C1 to C5:
  converts, publishes, boots with its script running, holds its declared physics, and plays
  every move to the end and back to STAND with nothing on the SCRIPTERR channel while the
  moves run. the moves gate reads the engine's complaints as part of its verdict, and stops
  the moment the engine dies rather than waiting out every later move. C6 (the same inputs in
  SSF2) passes for every one SSF2 can start a match with; a transformation (gigabowser, wario
  man) is not a pick there. with `--deep` (the three smashes, the three tilts and the aerials too)
  it passes for the whole ten-fighter sample. `tools/proj_probe.py compare` passes for all 36 fighters whose
  specials declare a projectile spawn (the other 14 declare none).

  what the sweep turned up, now rules the converter follows:
  - a fighter's clips are the ones its own timeline places (the display tree), never the
    ones whose class name starts with its name. in a package two fighters share (zelda and
    sheik, bowser and gigabowser) every clip class carries the package's name, and by name
    sheik had none of her clips' methods: her neutral special handed the engine an empty
    slot as a timer callback and the engine died on the third frame.
  - a timeline frame script is `frame<N>`; a method that merely starts with the word
    (kirby's copied `frameCount`) is the clip's own and folds like any other.
  - a callback the source keeps in an instance var is handed over as it is, through `__cb`
    (the closure it holds, or a function that does nothing while the slot is empty). a
    function renamed for colliding with a var is handed over under its new name.
  - an accessor the source's api answers from a declared stat (`getWidth`, `getHeight`) reads
    the declared body here, through `__declaredStat`, whatever kind of object is asked
    (`mappings/commands.jsonc` `stat_accessors`).
  - a neutral or side special that charges inside its one clip is cut once, by the boxes
    (`sprite_parser::split_charge_special`), and the entity follows that cut, so the name
    the label table sends the move to when it fires is an animation the entity has. the
    base piece's final engine frame plays its `_attack` piece (`flow_into_next_piece`):
    in the source the play head runs off the last charge frame into the attack label once
    the charge's own loop-back stops asking for it, and here an animation that ends goes
    to the stand.
  - a helper a frame script needs (`__hbNote`, `__declaredStat`) is emitted when any frame
    script asks for it, not only when Script.hx does.
  - the runtime tables (a name the script plays as it runs, the labels a jump lands on) name
    the pieces the entity has, so a jump into a cut clip (`crouch` in/loop/out, a taunt per
    direction) lands in the piece that carries the label; a name no piece answers is not in
    the table and plays nothing.
  - a thing the source spawns by its class (`spawnEnemy(PacManHydrant)`: pacman's hydrant,
    rayman's plum, waluigi's plant) is a projectile of the character under the class's name:
    the class declares its clip, its numbers and its flags in `getOwnStats`, what it fires in
    its own `getProjectileStats`, and its methods run as the object's own.
  - a getter of the class is a function here, published as one; a projectile reading it as a
    plain field (lucario's aura) calls it through the summoner's exports. a slot a summoned
    thing keeps under a getter's name is not a slot of the summoner.
  - a boolean the source returns from a chain (`return a && b && c`) is a value, not an `if`
    with the last term as its body; a chain that goes on past a negated inner chain keeps
    going (falco's target search).

  - a loop counter of the port's own (a helper walking a table, a decompiled for-loop) is named
    in the port's namespace (`__i`): a frame script runs where those counters are visible, and
    one named `i` took the place of a fighter's own `i` (pacman counts its dots in one).
  - a projectile's own class initializer runs before its first frame's script; what the
    projectile builds for itself (`new FrameTimer(...)`) is ported as a factory, as the
    character's own are.
  - a slot is an effect holder only when an effect is what the assignment puts in it, not
    when one is merely made nearby: tails keeps a point of where it was beside its trail, and
    the point keeps its fields.
  - who an object is, asked as a stat (`getCharacterStat("linkage_id")`), is the id its
    declaration carries: every script of the package publishes its own (`__linkageId`) and
    `__linkageOf(o)` answers for any object. lucario's sphere asks whether lucario or kirby's
    copy fired it before deciding to live.
  - a value published under a name of its own (`exports.__linkageId = __ownLinkage`) is kept
    when what it hands is declared; only the handed name has to exist.
  - a projectile stored as it is fired (`proj = fireProjectile("aurasphere")`) is kept by the
    summoner: it gets no die-on-floor handlers, so a sphere charged at the feet stays.
  - a platform an object gives itself (`createSelfPlatform(x, y, w, h)`) is a structure content
    beside it: a `platformSprite{i}` animation on the object's entity carries the floor line, a
    follower script rides the object, and the manifest lists it (rayman's plum, and the same
    port for a fighter's own calls: fox's landmaster, whose frames reach here already struck
    and are un-struck by the port).
  - a call on something the script holds, by a name nobody has (`platform.addIgnoreObject`),
    is struck the way `self.x()` is; a name a factory of the script's own installs is kept.
  - a frame both of a projectile's clips carry (the wrapper's `frame2` and the art's) runs both
    bodies, the wrapper's first: kept as a replacement, the art's frame threw away the one that
    binds the object to its handle and its owner (pikachu's jolt read both as nothing).
  - the animations a projectile can ask for are every label its clip carries (the list its
    animation stats are written from), not the spawn/active/destroy trio: a floor landing that
    asks for `continue` finds it.
  - a label asked of something other than the script's own clip (a projectile it holds,
    another fighter) is asked of that script, through `exports.__askLabel`, and taken up on its
    next frame in its own scope; a projectile's labels are its animations, and what publishes
    nothing plays the label as its animation. the fighter's own label table is its clip's alone.
  - a projectile's label whose last frame says nothing runs on into the next label as its
    timeline does, and off the last round to the first (`__labelFlow`); a label whose last
    frame plays, stops or ends the object holds where that script leaves it.
  - a yes/no stat a script writes at runtime carries this engine's name and asks its question:
    `surviveDeathBounds: false` on a frame is `deathBoundsDestroy: true`, as the stats table
    already had it.
  - an intangibility or invincibility switch the frame pairing leaves alone (an `on` in a
    method, an `off` with no `on`) is a body status held on: this engine grants a status for a
    span and cannot end one early (measured live: -1 grants nothing, a second grant never
    shortens the first, no remove exists), so a status switched on is granted again every
    frame until it is switched off (`__setBodyStatus`, `__holdBodyStatuses`).
  - a character stat this engine keeps under another shape (`jumpSpeedMidair`, a
    `jumpSpeedList`) is read at runtime from the shape it has (`stats.jsonc`
    `runtime_read_exprs`).
  - a named child of a projectile's clip that its code addresses (`self.gordospin.rotation`,
    `self.food.playFrameLabel(...)`, `self.notes.n1.playFrame(...)`) is a part of its own: the
    child's frames are an animation on the projectile's entity (`part_<name>`, one more per
    label), a vfx drawn from it rides the projectile every frame (offset, facing, rotation
    composed), and the code talks to a record (`setRotation`, `playFrame`, `playFrameLabel`,
    `alpha`, `x`, `y`) the vfx follows. the child is left out of the parent's art. the outer
    clip, the clip it flies in and each state clip are searched, a clip placed without a name
    looked through, a child of a child composed. a frame number asked of it shows that frame
    and holds it (dedede's gordo spins its spikes, kirby's and waluigi's dice, jigglypuff's
    notes, game and watch's bacon picks its food, tails' afterimage its frame).
  - a field only ever written through (`self.posPoint.x = 41.5`) that nothing declares, no
    clip places and no method answers to is a record of the object's own, made empty and
    written as the source writes it (waluigi's plant keeps where its next spawn goes).
  - a fighter's named children work the same way: a child placed anywhere under an
    animation's clip (through wrappers, named or not) that the code reaches as its own field
    or the stance clip's (`getStanceMC().waftloc`) is a part riding the fighter, its frames
    an animation on the character entity, left out of the fighter's own art. the placement is
    per animation (`_ppName`/`_ppAnim`/`_ppArt`/...): as the animation changes the record
    takes that animation's placement and art, and where an animation places none the part is
    hidden (krystal's and yoshi's `shieldframe` drawn while the shield is up and fading as it
    drops, wario's `waftloc` placing the waft).
  - a clip inside a clip draws: the part walker's tables flatten a held clip's pictures into
    the wrapper that holds it, at the frame the held clip has reached (placed on frame p, the
    wrapper's frame f shows its frame f - p, looping), however deep the nesting. read one
    level only, krystal's shield frame (a wrapper holding a wrapper holding the picture) drew
    nothing.
  - a heal (`recover(x)`) is a negative add on the damage counter; the fighters on the asker's
    team (`getTeammates()`) are read off the match, less the asker; a dodge roll the code
    asks for is this engine's roll state; a thing the camera is told to keep in frame
    (`addToCamera`) or let go of is a target of this engine's camera; the grabber letting go
    under any of its names (`grabRelease`, `grabReleaseOpponent`, `releaseOpponent`) releases
    every character it holds; an attack stat asked of a piece cut from the attack's animation
    (`special_down_loop`) is the attack's, the longest base winning, and a bare prefix is no
    base (`special`, the final smash, is not `special_down`'s: read as one it capped zamus's
    flip at its top); snapping onto the ground (`attachToGround`) is `attachToFloor`,
    the size the match scales a fighter by (`getSizeRatio`) is `getSizeMultiplier`, and damage
    dealt to a foe by hand (`takeDamage({ damage })`) goes on its counter.

  - an item the code generates (`generateItem("bobomb")`) is a projectile of the character's,
    pulled from the game's items package the way a shared projectile is (its clip has the
    idle/hold/toss states link's bomb has). `generateItem` makes it and holds it: it rides the
    holder's item box (the `itemBox` each animation places, published on the holder's exports)
    until `tossItem(angle)` or a bare `toToss()` (the way the holder holds the stick) throws
    it at the speed the item declares (`tossSpeed`, published on its exports) or else the
    engine's default measured off the source (`stats.jsonc` `item_toss_speed`); `getItem`
    is the held one, activate/deactivate are asked of it. an item pulled with no physics of
    its own falls as the source drops a thrown item (`item_gravity`, measured), is capped at
    the fall the game's items declare (`item_max_fall`, since this engine's projectiles do
    not fall past a cap of nothing) and ends at the death bounds; an item's declared
    `max_gravity` is that cap, under either spelling. a plain field the holder writes on
    it, or a call by a name only the holder's own script has (sandbag's `explode`), goes to
    what the item publishes and is nothing otherwise. an item made by `generateItem`,
    `generateCharacterItem` or `spawnItem(Class)`, by a name written down or one the code
    picks at run time from the names it lists (peach's turnips), is pulled the same way; it is
    kept by its holder by definition (no die-on-contact handlers of the port's), touches
    nothing and does not age while held, and is not `IState.ACTIVE` while held, since held is
    a state of its own in the source.
  - the attacks that throw a held item are the item's own to name: its class installs a frame
    interrupt (`setFrameInterrupt(handleInterrupt)`), published on the item, and when the
    holder begins an attack (an animation its attack table lists) while holding, the item is
    asked with the source's name of that animation (the fighter's name table, a piece
    resolved to its clip, a jab piece to its stem) in the item's own scope, one frame on; a
    throw the item asks for plays the holder's own `item_throw_<direction>` animation for the
    stick it holds (or the air variant), whose frames throw. what stays engine-only is the
    item categories (`item_raise`, `item_shoot`, `item_fan`, `item_homerun`): which category
    an item falls in is the source engine's, so those animations play only when the
    character's own code plays them.
  - a slot filled on a frame script (`item = getItem()`) is a holder wherever the fighter's
    code writes through it, so the holders are read off all of the fighter's code.
  - the ground between two points (`getPlatformBetweenPoints`, `hitTestGroundBetweenPoints`) is
    this engine's segment test against its structures, one structure type per ask (floor,
    ceiling, both walls): a terrain body's surface counts unless `terrains` is false, a
    drop-through floor unless `platforms` is false or `ignoreFallthrough` is set. twenty fighters
    ask it in recoveries, landings and teleports; it was folded as always false, which took the
    empty-air branch over solid ground.
  - a launch speed (jump, double jump, short hop) converts as `0.65 x (V - 3/4 G)`, 1.5 of this
    engine's gravity off the scaled value: the source takes gravity off before it moves the
    fighter and this engine after, and scaled alone every jump rose one extra frame of its own
    speed (mario's full jump 158 px against 140, measured). a run-time read of one adds the step
    back (`runtime_read_units.launch`), and the double-jump reads unscale instead of handing
    the script this engine's number.
  - a name table in the fighter's script holds only what its entity has: an animation the
    entity leaves out as empty, or a move with none, leaves the table too, and the running
    script checks its own tables against its entity (`__selfCheck`, read by C3). a clip cut into
    pieces is checked through the piece that proves it (`_cutClip`/`_cutPiece`).
  - an attack the source turns off by name (`setAttackEnabled`) turns off the action its
    animations start, held with one `DISABLE_ACTION` status effect per action until turned back
    on (`commands.jsonc` `attack_actions`). a name no action starts (kirby's copied specials)
    does nothing. another object turning it on or off asks (`__askAttack`), taken up on the
    fighter's next frame. landing turns every attack back on, as the source's engine does
    (pikachu's side special from a jump refuses his up special until he is down).
  - a jump to a frame NUMBER of the clip an animation was cut from lands on the piece that holds
    that frame, at the frame inside it (`_playSrcFrame`, off the `_splitFm`/`_splitAt` table):
    lucario's recovery picks the pose for its angle that way, and a jump to a number played
    nothing at all before, so its climb ran while it was already falling.
  - a name the fighter's class declares but never fills is the child its clip places under that
    name, which the source's engine binds for it: ryu's costume trails and metaknight's colour
    pieces are parts the port shows and hides like any other. a name its own code fills (dedede's
    `gordo` holds the one it fired) stays a slot.
  - a FIXED hit's launch (`weightKB`) is what the source works out from the field, not the field
    itself: `(weightKB * 0.803 + 18) * kbConstant/100`, measured live on mario's jab and
    chibirobo's smashes. it is weight-scaled against the victim in the source and cannot be here,
    so it converts for a victim of the engine's own weight (`tools/tests/hit_parity.py` expects
    that difference when a fighter hits itself). mario's jab launched at 0.38 of the source's
    before, and lands at 0.650 of it now, which is the scale a speed converts by.
  - a jump into another animation's label enters it without running its first frame's script:
    this engine runs that script as the animation starts, and the source, jumping straight to the
    label, never ran it (sonic's tapped spin dash took the dash speed off the frame it skipped).
  - a slowdown the source states as a share (`xSpeedDecay: 0.8`) keeps that share of the speed
    each of its frames, which this engine's friction cannot state, so the share is kept here
    (jigglypuff's side special, measured on both engines).
  - an attack's motion stats (`xSpeedDecay`, `xSpeedDecayAir`, `xSpeedCap`, `xSpeedAccel`,
    `xSpeedAccelAir`) are put on its animation as it starts and when its code updates them, in this
    engine's units: the source states a slowdown as a negative amount a frame, which is positive
    friction here (peach's side special slows by exactly 3.8 a frame on both engines). a stats
    object written just before `updateAttackStats` is inlined into the call.
  - an attack that declares `resetMovement` starts from a standstill: both speeds go to zero as
    it begins, and a piece cut from the same attack is not a new start.
  - an animation that plays another's clip (an air special the source runs on its ground clip)
    runs that clip's frame scripts, not only its pictures and boxes. 46 fighters had air
    specials that drew the move and ran none of it (megaman's rush jet fell instead of climbing).
  - a collision box is read through the shape of the character it places, each measured on its
    own: width, height and where its middle sits off the registration point. a fighter can place
    boxes drawn more than one way (sonic and lucario draw one off its origin, krystal's is taller
    than wide), and `tools/tests/box_runtime.py` holds the result against the boxes the source
    places at run time.
  - a walk's starting speed carries one and a half frames of the walk acceleration: this engine
    starts a walk a frame later and moves before it accelerates (megaman's and goku's fifteen-frame
    walks measured 8% short without it).
  - a match global the fighter treats as a flag starts unset, which equals neither true nor false,
    the way the source reads a global nothing has set yet (waluigi's down special is off only
    while `plantspawn == false`).

  open, and reported as such by the gate:
  - nothing hands a fighter the stage here (measured: no stage, no blast zone, no camera bounds
    reachable from a fighter's script), so what a fighter asks the source's stage for is struck
    where it stands. answering it with a box nobody measured would take a branch the source never
    takes, which is worse than saying so.
  - C6 records each input's path on both engines (evidence, not a verdict). the widest gaps left
    are side specials (peach, jigglypuff, pacman, luffy, waluigi) and sonic's down special.
  - wario_man and gigabowser are transformed forms the source will not start a match with on
    their own, so C6 cannot drive them.
  - fox's landmaster makes six `createSelfPlatform` calls in its final smash, which the moves
    gate does not drive; they port the same way rayman's plum does (a structure content riding
    the fighter per call site, `platformSprite{i}` lines on the character entity, the manifest
    listing them) and are driven live by hand, not by a gate.

a few are hand-verified deeper still (drive moves + physics + anim): `sandbag` (the
reference), `mario` (the deepest, a full 18-move sweep), `kirby`, `bowser`, `fox`, `sora`.

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
a stage's objects (hazards, enemies, moving platforms, launchers) come from the classes the
package declares: each ships as a custom game object running its own translated class, with its
size, hit, art clip (`getOwnStats().linkage_id`), animations and spawn taken from what the class
says. nothing is detected by name and nothing is declared per stage: `metadata.jsonc` carries a
stage's display name and music only.

three stages boot with zero script errors and are the regression set for converter changes:
bowserscastle (lava + the recycling thwomp), clocktown, and bombfactory. always live-check a
stage with a STOCK fighter -- a converted character's own script noise looks exactly like a stage
regression otherwise.

bombfactory runs its whole gimmick from the converter: the stage fills a 17x8 grid with bricks
and bombs, each block is solid terrain a fighter stands on, bombs explode in a row-and-column
cross, and a fighter caught in one takes heavy knockback and a KO. it needs the class's OWN code
rather than a template, which the converter picks automatically -- when the stage calls a method
only that class defines, the class's code is used; where the stage just makes an object and
places it, the verified template stands.

what a converted object can be told, and what it can tell back, goes through `exports`: FM gives
a handle only its api, so an object's own methods reach the outside that way or not at all. the
stage hands itself over at spawn, so an object can call back into the stage that made it.

all three boot at ZERO script errors and no crashes, with their hazards behaving:

| stage | hazards, live | 
| --- | --- |
| bowserscastle | lava burns (23 hits + a KO in 100s); the thwomp runs its whole drop cycle (y -7 -> 294 -> 692) |
| clocktown | flaming rocks fall (fractional y mid-flight, one past the floor) and land hits; day/night reads correctly |
| bombfactory | 46 hits in 85s; bricks stand, bombs arm, explosions cross |

bombfactory's block mix is measured against the source rather than eyeballed. brick coverage of the
grid area, from frames taken inside each engine using the brick's own palette:

  SSF2        4.30 3.10 1.27           mean 2.89%
  fraymakers  2.40 0.02 2.32 0.11      mean 1.21%

same order, and the per-frame values overlap; the low samples are frames where an explosion covers
the view. the frames agree by eye.

bomb factory's block mix is measured on BOTH engines rather than inferred from one. the source is
read live the same way the port is (`peptide ssf2 session --char mario --stage bombfactory`, then
`tree`), which lists its blocks and the label each one is in.

  source      about 8 cells held, 7 of them bricks
  port        (9,11) (4,4) (8,8) (2,2) as (bricks, held), sampled 30s apart

the port's occupancy moves up and down now instead of climbing, and nearly everything it holds is a
brick, which is what the source does. brick coverage in frame went from 2.2% of the visible cells to
8.9%, against the source's 11.6%.

what it was: cells stayed held by blocks that were already gone, and the spawn picks a brick with
probability `(100 - held * 3.5) / 100`, so every cell held by nothing pushed the mix further toward
bombs. at the 15 to 23 the port used to hold that is 47% down to 18%, against 72% at the source's 8.
two things were leaking cells, both fixed and both general:

- a body's own platform was found by taking the structure list's LENGTH before creating it and using
  that as the index. when the create did not land right away the index was one past the end, and
  reaching past the list stopped the script that did it.
- a class that ends its own life released what it reserved on the way out, but only if it reached
  that line. the release is a queued ask and the destroy is immediate, so an object that left any
  other way never released. the cleanup now runs from `onKill` as well, once.

how a stage object is built now, and it is the same for all three: the class states what it is by
what it EXTENDS (`SSF2Enemy`, `SSF2Platform`), what it looks like by the `linkage_id` it names and
the labels its own code plays, what it hits for by its own `getAttackStats`, where it goes by its
own setX/setY against the terrain anchor, and what it DOES by its own `update()`/`initialize()`,
translated and run. no keyword decides any of it.

the one vocabulary left is the auto-detect fallback, which classifies placement-tree instances by
symbol name and hands back per-kind default stats. it is only reached by a package that declares
neither hazards nor actors, so none of the three stages touch it, and a test pins its keyword count
so it can only shrink. removing it needs a wider check than the ten-stage sample can give, since a
stage that currently gets hazards only from that path would silently lose them.

what got the three there is universal, none of it keyed to a stage:

- SSF2's hazard switch maps to the port's own `m_hazardsOn`, not a match-config key fraymakers
  does not have (every converted stage was running its hazards-OFF path)
- an object whose class ENDS ITS OWN LIFE runs that class's code instead of a template, or it
  never leaves the match
- the two-way channel between scripts: what an object KNOWS crosses as a value, what it DOES
  crosses as a queued ask, and an api chain only the stage can reach is worked out by the stage
  and published as its answer
- a `continue` advances a loop once, a source frame index is doubled for 60fps, and a field
  holding a flat list gets one slot per element

### the day/night fade

the source cross-fades day to night: a day copy of the backdrop goes out while a night copy comes
in, a step per frame across hundreds of frames. the port carries that as a per piece alpha schedule.

each backdrop piece is its own object on its own timeline, placed relative to the backdrop's own
top left, with one container per pan rate scrolled against the camera's travel (the engine no
longer scrolls them). a piece whose opacity changes over the source timeline hands its
`[frame, alpha]` ramp to the stage's frame loop, so the whole backdrop turns over together.

measured on clocktown, sampling the sky and the clock face every 5 seconds in engine:

  sky   154 149 147 116  98  97  43  18  18  18  155 152 150
  face   72  81  83  68  59  60  46  42  37  36   63  70  73

day, dusk, night held, then back round to day, with the face tracking the sky the whole way.

`PEPTIDE_COMPOSITE_CAMBG=1` selects the other build: the pieces go into the engine's own parallax,
one background entry per piece, so each keeps its own pan rate and gets its own container to fade.
it is not the default because each entry's animation plays and loops at its own length, so the
pieces drift out of phase with each other and the backdrop stops turning over as one.

the engine's parallax containers are reachable and fadeable from hscript:
`match.getCamera().getBackgroundContainers()` returns one container per declared background entry,
in declaration order, each with a settable alpha. verified live (19 entries, 19 containers).

stage porting is NOT finished. bowserscastle is the deepest port and the working testbed, and
even it has gaps: chunks of the reconstructed stage behavior are still commented out in the
emitted scripts pending verification (SFX calls among them), so the in-game stage is missing
sound effects and whatever else those blocks drive. other complex stages haven't had the same
1:1 treatment yet (each stage's gimmicks are bespoke AS3 that needs its own RE pass; the
machinery here generalizes, the per-stage work doesn't come for free).

## hitbox-stat parity

**every character's hitbox stats match the SSF2 source. no exceptions right now.**
`tools/tests/parity_check.py` checks every hitbox's `damage` / `angle` / `baseKnockback` /
`knockbackGrowth` / `hitstop` / `selfHitstop` / `hitstun` against the SSF2 source (dumped
via `DUMP_PARITY=1`). that covers the hitbox-*stats* slice of functional equivalence.

the real bar for a "done" character is functional equivalence with SSF2, which is a higher
bar than "spawns, animates, doesn't crash." hitbox-stat parity is one piece of it. the open
dimensions below are the rest.

## knockback parity

**a hit launches its victim the way SSF2 launches it, measured on both engines.**

`hits` reads the launch the ENGINE calculated, not where the victim ended up. Both engines set
the launch on the frame the damage lands and hold it through the hitstop freeze, so that frame's
velocity IS the knockback, before decay, gravity or DI touch it. Each engine keeps knockback in a
channel of its own that its plain speed fields do not include, and `hits` reports the sum:

    +10.0 damage at sample 3 -> launch 9.35/sample at 67 deg (sample 15), state HURT_LIGHT
       engine knockback 9.163/frame at 70 deg  (dvx 3.134, dvy -8.611, sample 3)

the second line is the number to compare; the first is what the launch did once friction and
gravity got at it, kept as a cross-check. on SSF2 the window comes from the injected per-frame
recorder rather than polling, because polling answers about once every 14 of its frames and a
hitstop can start and end inside one gap.

**each knockback stat carries its own measured multiplier.** the two engines agree on the SHAPE
of a launch and differ by a scale, so one factor per field is enough. each is isolated by the
move used to measure it: `special_side` has no growth, so its launch is the base response alone;
`tilt_down` gives the growth response at two victim damages once its base part is subtracted.

| field | multiplier |
|---|---|
| baseKnockback (`power`) | 0.775 |
| knockbackGrowth (`kbConstant`) | 0.764 |

verified live, SSF2's launch converted at `velocity_scale` (x0.65) against Fraymakers' own:

| move | base/growth | victim % | SSF2 | target | FM | ratio |
|---|---|---|---|---|---|---|
| special_side | 40 / 0 | 0 | 8.549 | 5.557 | 5.557 | 1.000 |
| tilt_down | 30 / 80 | 0 | 10.876 | 7.069 | 7.060 | 0.999 |
| tilt_down | 30 / 80 | 60 | 19.314 | 12.554 | 12.581 | 1.002 |
| tilt_forward | 10 / 100 | 0 | 7.488 | 4.867 | 4.803 | 0.987 |

the last row is the check: that move took no part in fitting either factor.

the values are emitted FRACTIONAL. rounding a scaled knockback to a whole number costs about 2%
of a light hit's launch, which is more than the factors themselves are uncertain by.

**hit TIMING converts too, and not the way it was being converted.**

| SSF2 | Fraymakers | conversion | why |
|---|---|---|---|
| `hitStun` | `hitstop` | n x 2, unstated (or below zero) floor(damage / 6 + 2.5) x 2 | both engines hold the victim still for that many frames after the frame the hit lands on (read off the victim's own record with `rehit_trace.py --victim`: fox's jab 1, mario's 3, kirby's forward tilt 4) |
| `selfHitStun` | `selfHitstop` | n x 2, unstated floor(damage / 6 + 1.5) x 2 | same field, attacker side |
| `hitLag` | `hitstun` | no override (-1) | `hitLag` is a MULTIPLIER, not a duration: the corpus states it as -1.0, -1.1, -1.2, -1.23, -1.3, 0.9. Fraymakers has no hitstun multiplier and works hitstun out from the knockback, as SSF2 does, by its own rule: the runtime puts in the source's (above) |

there is NO multiplier on the freeze itself: the engine returns a declared `hitstop` as it stands
and stores it as a frame count. verified live -- mario's jab (`hitStun` 3) holds its victim 3 frames in
SSF2 at 30fps and 6 in Fraymakers at 60. the x drift visible during a Fraymakers freeze is its
hitstop nudging, not the freeze ending early.

damage needs no factor, but a SSF2 hit does not deal the declared number unless the move has been
used enough to stop staling: fresh it deals up to half again (mario's forward tilt, declared 9,
deals 13.65 then 11.96 then 10.79 and settles at 9.3). so any measurement of a launch has to
stale the move first, since the launch is worked out from the damage that landed. Fraymakers
deals the declared number flat.

**a `weightKB` hit is FIXED knockback, and Fraymakers has no such thing.** SSF2's jab launches at
exactly the same speed at 0% and at 62%, where tilt down doubles across the same range, so the
growth field is dropped when the source states `weightKB` and the converted jab is flat in
Fraymakers too. what cannot follow is the magnitude: SSF2 scales those hits by the victim's
WEIGHT, which Fraymakers has no field for, so they land about 23% light (jab: 3.585 against
SSF2's 4.66 in Fraymakers units) and that is where they stay. parity is measured on the moves
that convert -- a `power`/`kbConstant` hit, within 9% -- and a weightKB hit is not a gap to
close.

these numbers apply to EVERY knockback the converter emits: a projectile's hitbox stats, a stage
hazard's, and a knockback a translated script writes at runtime. they all read the one table in
`mappings/character/hitbox_stats.jsonc`, and `tools/tests/parity_check.py` reads it too rather
than repeating it.

`hits [pN] [frames] [raw] [<ctrl:frames> …]` is how any of this is re-measured: it plays the
input and watches the victim in one command, on either engine. `hits pN <frames> raw` prints
every frame with position and velocity.

## the same input is the same move

a press turns into the move the source plays for it, read the source's way by the runtime
(`Fighters.smashInput`, `Fighters.squatFrame`), measured on thirteen fighters with
`tools/tests/input_parity.py`:

| input | both engines play |
|---|---|
| a direction and attack, the controls held 3 source frames or less before (2 for up and down) | the smash |
| the same, held longer | the tilt |
| the way behind and attack, pressed fresh | a turnaround forward smash |
| up and attack in a jump's squat (before its last grounded frame) | the up smash |
| any other attack in the squat, or up and attack from its last frame on | the aerial |

the count is of any control held, back to 0 only on a frame none is: a crouch's down followed by
up and attack is still the up tilt. what is left: an attack pressed on the frame a crouch is let go
plays nothing on the source and is unported, because the harness cannot crouch this engine's
fighter (it takes a crouch and a tap dash off the stick's travel, and the injected input is a
button mask).

the motion around a press follows the source's too (`tools/tests/move_trace.py` shows both engines
frame by frame):

- a stood fighter pushed the way behind it turns and walks on the same frame (`Fighters.turnWalk`),
  where this engine played its turn first: falco walked left 8 frames went 42 pixels there, 19 here
  and 37 now.
- an attack begun out of a walk slows from its first frame (`Fighters.attackStartSlow`), where this
  engine carried the walk's speed two of its frames first: falco's jab slid 27.6 pixels here against
  15.5 there, 18.1 now.
- how long an attack has run (`getExecTime`) is the attack's own count, 0 as it starts and one more
  each frame it is not held still by a hit (`Attacks.execTime`), where it read the match's running
  time: jigglypuff's forward smash lunges by its charge and slid 290 pixels here (off the stage)
  against 64.5 there, 65.2 now; ryu's forward smash deals damage by it.
- an attack in the air catches a ledge its grab box meets, whichever way the fighter faces, not
  while down is held (`Fighters.attackLedge`): falco's side special dashed off the stage hangs from
  the ledge on both, a frame later here. open: as it drops past the edge this engine shoves the
  fighter 33 pixels out from the stage's side (its collision body, sized per frame from the hurt
  boxes, is wide in the dash pose), where the source's drops straight; the grab snaps it back.
  a plain walk off the edge shows it too: falling past the stage's side, mario is shoved 6 pixels
  out in one frame here and none there (`move_trace.py mario "right:30" --from=200`), so the body's
  width against a wall, not the dash, is the open question. the source meets a wall at one point a
  short way out from the fighter's middle, well inside the body this engine pushes by; a body sized
  to that would also move the gaps two fighters are pushed to (the push section above), so it
  waits on re-running that sample.
- an attack that resets movement (`resetMovement`) is put back where it began on the floor as well
  as in the air: this engine moves a fighter once on its old speed as an attack starts (falco's
  side special out of a walk slid into it). open: near a floor's edge this engine slows a fighter
  in a move that may slide off far less, so that dash, ending 2 pixels short of the edge on the
  source, creeps over it here.
- a dodge roll moves from its first frame, as the source's does (`dodgeRollSpeedStartFrame` 1,
  stats_tables.jsonc), where this engine waited for its own third: falco's roll trailed the
  source's by two frames and now by one, standing on the same frame.
- a throw's small hits on the fighter it still holds freeze the thrower for their own count, as
  any hit does (`fighters.hx`, HITBOX_CONNECTED): pikachu's forward throw shocks twice before it
  throws, ran 26 frames here against 28 there, and now 28 (`state_parity.py`). open: jigglypuff's
  back throw slams down and sits on that frame two frames longer on the source before its hit (20
  against 18 here).
- the taunt button plays the neutral taunt's own section wherever its clip puts it
  (`anim_splitter.rs`): lucario's is alone in one clip and his side taunt in another, and the side
  one, reached first, took the button (46 frames there 39). lucario, bowser, sonic, fox, sandbag
  and mario now taunt for the source's length (`state_parity.py`).
- open: pikachu's down special in deep C6 (its second spot) ends at 28 frames on the source, its
  bolt never reaching it, where `move_trace.py` from any spot and the port play the full 39; the
  source does it only after the gate's own placement, so what of that placement the bolt meets is
  not yet read.

`hit_parity.py` still NAMES a grounded move on the source (`move <name> [pN]`, the label from
`conversion_stats.json`) and presses it here, a tilt with its direction held first.

measured that way, at 0% and 40 units apart:

| move | SSF2 (its 30fps) | -> 60fps | x0.65 | Fraymakers | ratio |
|---|---|---|---|---|---|
| tilt_down | 21.776 | 10.888 | 7.077 | 7.060 | 0.998 |
| special_side | 17.205 | 8.602 | 5.592 | 5.557 | 0.994 |
| tilt_forward | 10.722 | 5.361 | 3.485 | 4.803 | 1.378 |

`tools/tests/hit_parity.py <char> [move …]` does the whole thing: it finds a placement each move
actually lands from (one spacing for every move misses -- an up tilt does not reach a victim
standing level with it), asks the other engine to land from the same place, and reports the
launch each engine calculated. it also reports the ANGLE, which is a hitbox's fingerprint: a move
with several boxes lands a different one on each engine from the same spot, and comparing those
two numbers says nothing about the conversion.

mario, victim at 0%:

| move | ssf2 (its 30fps) | ->60fps | x0.65 | fraymakers | ratio | angle |
|---|---|---|---|---|---|---|
| tilt_down | 21.776 | 10.888 | 7.077 | 7.060 | 0.998 | 70 ~ 70 |
| special_side | 17.205 | 8.602 | 5.592 | 5.557 | 0.994 | 180 vs 35 |
| tilt_forward | 10.722 | 5.361 | 3.485 | 4.803 | 1.378 | 35 ~ 30 |
| tilt_up | 13.239 | 6.620 | 4.303 | 7.652 | 1.778 | 96 vs 85 |

**a move is worth less every time you use it, and that is most of what looked like noise.**
SSF2 stales a repeated move: mario's forward tilt deals 13.65 the first time, then 11.96, then
10.79, and settles at 9.3 against a declared 9. so the same move measured twice reads two
different launches, because the launch is worked out from the damage that actually landed.
measured after it settles, the reading is exact: six uses in a row, 10.722 every time. (this
corrects what was recorded here before -- the bonus on a fresh hit is not about 5%, it is up to
half again, and it decays with use. Fraymakers deals the declared 9.0 flat, every time.)

so the comparison has to be made where both engines are dealing what the move declares, which
means using the SSF2 move until it stops staling first. done that way:

| move | base/growth | ssf2 (30fps) | ->60fps | x0.65 | fraymakers | ratio |
|---|---|---|---|---|---|---|
| special_side | 40 / 0 | 17.205 | 8.602 | 5.592 | 5.557 | 0.994 |
| tilt_down | 30 / 80 | 21.776 | 10.888 | 7.077 | 7.060 | 0.998 |
| tilt_forward | 10 / 100 | 10.722 | 5.361 | 3.485 | 4.803 | **1.378** |

the first two are the moves the factors were FITTED on, and they still hold. the third is a real
divergence and it is not measurement: the fraymakers angle matches the declared direction (35)
and the reading repeats exactly.

**every hitbox in the roster now launches the way the source does.** 1,798 of 1,798, across all
45 characters, checked at four victim damages each -- and with no engine running, which is what
makes checking all of them possible at all.

that became a static check once one thing was established live: **Fraymakers reproduces its own
launches from the numbers this converter emits, exactly.** four moves, one scale for the whole
engine, predicted to within 0.01%. so what a converted hit will do is knowable while converting,
and the source's own numbers say what its hit does, and the two can simply be compared.
`tools/tests/knockback_parity.py <char>` (or `--all`) is that comparison.

it found two things wrong, both of which had been hiding behind a fit:

- **two factors where there is one.** a launch is LINEAR in base and in growth alike, so scaling
  both by a single factor scales the whole curve and the two engines then agree at every damage.
  the old pair (0.775 and 0.764) were two nearly-equal answers to a question with one answer, and
  the small difference between them rode on whichever field a move leaned on: twelve base-heavy
  throws across the roster came out up to 4.9% off. one factor, 0.7629.
- **damage cut to a whole number.** zelda's down throw declares half a point of damage and was
  emitting none. that does not merely lose the damage: both engines work a launch out FROM the
  damage that landed, so a rounded 0.5 moves the launch too.

measured live afterwards, against what the source's own arithmetic says each launch should be:

| move | base/growth | source says | target here | measured | ratio |
|---|---|---|---|---|---|
| special_side | 40 / 0 | 8.416 | 5.470 | 5.470 | 1.000 |
| tilt_down | 30 / 80 | 10.756 | 6.991 | 6.991 | 1.000 |
| tilt_forward | 10 / 100 | 7.349 | 4.777 | 4.777 | 1.000 |
| tilt_up | 26 / 125 | 11.677 | 7.590 | 7.589 | 1.000 |

`tilt_forward` is the one that read 1.378 before, and it was the growth-heavy move that exposed
the two-factor fit.

## open parity dimensions

- **frame data (active-frame range).** startup/active/recovery lives in the `.entity`
  collision-box keyframes and animation lengths, not in `HitboxStats.hx`.
  `tools/tests/timing_parity.py` checks both halves statically, with no engine running: the
  30→60fps doubling for every emitted animation against the source range the splitter sliced
  it from, and the COLLISION-BOX active window that decides startup/active/recovery. Both
  sum the emitted animations back onto the SOURCE they came from, since one source timeline
  becomes several (mario's grab is one 50-frame timeline carrying the grab and the hold).
  the whole roster: 7,324 animations across all 45 characters, every one exactly 2.00x, and
  1,506 hitbox active windows, every one exactly doubled. `tools/tests/timing_parity.py --all`
  re-runs it in a few minutes with no engine.
  a move whose last frame's code ends it is one source frame shorter on purpose and the
  check knows it. the live half is the `record`/`trace` frame recorder (both
  engines push per-frame telemetry); it confirms the emitted lengths actually play that
  way, on a sample rather than the whole roster.
- **a combo chains again**. SSF2 chains a combo by jumping its timeline to a label, and
  the jump was translated to a call nothing answers to, so it died with "Invalid function null"
  and the combo stopped at its first hit. The label that STARTS the next stage is an animation
  now, so the jump is `playAnimation` at the name the split gave it; a label still inside an
  animation stays `playFrameLabel`. The timer that reads the button was dying too -- its options
  argument is an object on both sides and content passes a bare boolean there. mario's jab: 3%
  and one hit before, 9% and all three after, no script errors.
- **a frame whose code ends the move is no longer emitted**. the two engines run a
  frame's code on opposite sides of drawing it: SSF2 runs it FIRST, so a frame carrying
  `endAttack` hands control away before anything is drawn and never appears; Fraymakers runs it
  AFTER, so emitting that frame showed a pose the original never displayed and the move ran one
  source frame long. the trim happens before any layer is built, because an animation's length is
  its LONGEST layer -- trimming only the images left the labels at the old length and the
  animation exactly as long as before, just inconsistent.

  ONLY an explicit `endAttack` counts. a `gotoAndStop` is as likely to be a LOOP inside the move
  (a smash holds its charge with one), and treating that as an ending trims a frame the original
  does draw.

  measured on the fixture, sandbag, both engines: jab, tilt_forward, tilt_up, tilt_down,
  aerial_neutral, aerial_forward, aerial_up, aerial_down and special_down are all now exactly
  2.00, against 2.10-2.17 before.

- **split animations were losing every frame script**. the mapping files speak in parent
  names -- `a` becomes `jab`, never `jab1` -- so looking an SSF2 animation up by a split PIECE
  name found nothing, and nothing is indistinguishable from "this animation has no scripts".
  sandbag's jab was emitted with zero frame scripts: no dust, no swing sound, and nothing marking
  where the move ends. pieces resolve through their parent now, and jab went from 3 missing
  scripts and 2.15 to 3 scripts and exactly 2.00.

  worth a corpus pass: every character with a multi-hit jab was missing those scripts.
- **stage weather is ported**. a stage can build particle weather in CODE rather than on a
  timeline, so nothing else in the pipeline sees it: the art walk finds no placement and the hazard
  path finds nothing that damages anyone. bowserscastle converted with every layer correct and the
  air empty.

  `abc_parser::extract_stage_weather` steps the stage's own `initialize` for the shape "construct a
  weather object from a particle CLASS, then hand it an area and a count" -- keyed on that shape,
  not on any stage or class name. bowserscastle reads back as 30 particles of `ember` over
  255.5x252.5. the particle is rasterised through the bitmap-fill path (SSF2 art nearly always is a
  bitmap fill, which the vector rasteriser declines by design), and the stage spawns that many
  looping VFX and drifts each one itself, rolling rise, drift and alpha per particle from the same
  ranges the source rolls them from. per-frame speeds convert through `velocity_scale` like every
  other one: SSF2's 1..3 rise becomes 0.65..1.95.

  verified live: 30 VFX in the match, positions scattered across the field and changing between
  samples.

- **weather is tethered to the camera, and sized by it**. SSF2 attaches the field to the
  VIEW: it fills the screen wherever the camera goes and a particle leaving one edge returns at the
  other. each particle now stores an offset from `match.getCamera()` (which reports the CENTRE of
  the view, confirmed live) resolved every frame.

  the field is sized from the camera too, not from SSF2's rectangle scaled up. SSF2's authored area
  is that engine's screen; fraymakers' screen is a different shape (640x360 at zoom 1 vs the 498x492
  the scaled numbers gave), so copying them across misses the sides and overshoots the top. sizing
  off `getViewportWidth()/getZoomScaleX()` re-derived per frame is what makes it the same field, and
  since both are one screenful the source's particle COUNT carries over unchanged.

  the viewport is reported in WORLD units and already accounts for zoom, so dividing by the zoom
  scale is wrong: it pins the field to a constant 480 half-width while the visible half-view is
  ~690, leaving the sides bare. sizing straight off the viewport tracks it. verified live through
  the borrowed content interpreter, which can read the stage script's own state: with the fighters
  1200 apart, `halfW` 1035.6 against a viewport of 1380.8, particles spanning -704..935, and the
  worst per-particle tether error across all 30 exactly 0.

- **`e` runs with the scripting surface content sees.** an expression has `match`, `camera`,
  `stage` and `self` as a stage's or character's own script has them, plus peptide's names on top
  (`p0`..`p3` and the bare `getCharacters()` reach the fighters themselves; `match.getCharacters()`
  answers the content api's character objects). it works on shipped stages too, whose scripts are
  compiled, by borrowing a character's, and an `e` sent before the match starts waits for it.
  verified live:

  | expression | answers |
  | --- | --- |
  | `match.getCamera().getX()` | 1047.4 |
  | `match.getCamera().getViewportWidth()` | 640.0 |
  | `match.getMatchSettingsConfig().hazards` | true |
  | `p0.getStateName()` | STAND |

- **weather is detected by the engine call it makes, not by a class name**. a stage hands
  its weather's container to the engine's weather layer, so "the object whose `getContainer()` is
  added to `getWeatherMC()`" identifies it exactly, on every stage that has any, without knowing a
  single class name. keying on a name containing "weather" found 2 of the 8 stages in the corpus
  that call it, and only if the construction sat in `initialize` (clocktown builds its rain behind
  a state machine, smashville in `syncStage`).

  the COUNT and area come from the class too, not from the call: the constructor says which
  parameter lands in which field, and the signature carries the defaults for whatever the stage
  did not pass. bowserscastle passes only the particle class and takes the rest from those
  defaults, which is why reading the call site alone found nothing. its real numbers are 50
  particles over 640x360 -- and that area being exactly one SSF2 screen is independent
  confirmation that a weather field is meant to be one screenful.

  | stage | particle | count |
  | --- | --- | --- |
  | bowserscastle | ember | 50 |
  | clocktown | ClockTownRainDrop | 100 |
  | crateria | CrateriaGreenRainDrop | 50 |
  | fairyglade | fairyglade_particle | 8 |
  | smashville | (rain) | 100 |
  | finalvalley | (rain) | not readable, skipped with a warning |

  a stage whose count cannot be read emits NO weather rather than a loop over nothing, which would
  render as a stage that carries weather and shows none.

- **the "revival platform frame" was the harness, not the converter**. a frame of the
  revival pose showed up at the head of tilt_up, down_smash and walk. the emitted animations are
  clean -- tilt_up carries a body layer and a motion smear and nothing else, and the unused image
  layers really are blank -- because nothing was wrong with them.

  the sweep parked aerials BELOW the floor, which is right on the fixture (open space, blast
  boundary far below) and wrong everywhere else: on an ordinary stage, below the floor IS the blast
  zone. the engine KOs and respawns the character, and the two frames of REVIVAL that follow land
  at the start of the next move. measured on thespire, whose floor is y=73: parking at y=600 gives
  `revival x2` then stand, parking at y=73 gives stand alone, and a bare `reset` never produces it.

  two fixes. the floor is MEASURED per engine at the start of a sweep (drop the character, see
  where it stops) instead of being the fixture's constant applied to every stage. and aerials park
  high ABOVE the floor with enough clearance for ~100 frames of fall, so a move still gets to
  finish: an aerial now reads `aerial_neutral x40` then fall then land, where before it was
  whatever the respawn left behind.

- **effects an object does not carry are not spawned.** the engine does not check the effect name
  it is handed: an effect made on a name the object has no content for draws the object's own first
  animation (a fighter's is `revival`, so the move drew a copy of the fighter on its platform). so
  the runtime's `Effects.attach` and a projectile's trail spawn only what the object carries
  (`DATA.effects`) or what maps to one of the engine's own (`DATA.globalVfx`, from `global_vfx_map`
  in `commands.jsonc`); anything else is reported (`no effect <name> (not carried, no global one)`)
  and its code gets a clip drawn nowhere, so code that moves it runs on.

  the source's SHARED effects are defined in whichever fighter's file registers them (sandbag's
  `effect_land` is bandanadee's), so a fighter that only names one carries nothing for it. the
  dust and landing ones map to the engine's own (`effect_land`, `ground_bounce`, `gbdust`, `land`
  to `LAND_DUST`, `dust` to `DUST_POOF`, `dust_effect_swirl` to `DUST_SPIN`, `effect_dust_light` to
  `DUST_START_LIGHT`, `global_dustboom` to `DUST_START_HEAVY`, and the `global_dust_*` family).
  a shared effect the engine has no match for is borrowed from the package that defines it, found
  by every name any class in the fighter's package asks for (its projectiles' code included: link's
  bomb borrows its explosion, dedede's `explosion_air` and `effect_cancel` come across too). what
  stays unspawned is named by a fighter but defined by no package in the installed game, so the
  source draws nothing for it either: bowser's `effect_bowser_sspec`, black mage's
  `blackmage_dsmash_hands1`/`2`, link's `link_galeEffect`, naruto's `naruto_rasenParticle`,
  pichu's `pichu_jolt_spark`, and kirby's `ness_pkflasheffect`, `Kirby_sandbag_*` and
  `sandbag_confetti`.

- **four animation/effect faults, all fixed.**

  | fault | cause | fix |
  | --- | --- | --- |
  | effect placements ignored facing | the source flips the offset by hand (`self.flipX(78)`) AND fraymakers mirrors it via `flipWith`, so the two cancelled | drop the source's flip exactly when `flipWith` is set. measured live: `flipWith` + raw `x: 78` lands at +78 facing right, -78 facing left |
  | `run_turn` faced backwards | only its FIRST frame was mirrored | a turn is drawn facing the direction turned FROM and the engine flips facing when the turn starts, so the whole animation mirrors -- scaleX, x AND rotation, since reflecting about the vertical axis maps an angle to its negative |
  | `crouch_out` played like a second crouch | the exit slot cloned the entry FORWARD | splits carry a `reversed` flag; the exit runs the entry backwards |
  | `fall_loop` looped wrong, and the fall looked like the jump never ended | the loop was sliced at a template constant, and the peeled entry frames were appended to the END -- so every cycle flashed the entry pose | the loop comes from the clip's OWN loop-back call |

  the loop detection is structural, not a name list: a looping SSF2 stance ends by playing a label
  again from its final frame, so `source_loop_frame` reads that call's target and looks up where
  that label sits. the name varies across the corpus (`again` on sandbag and mario, `redo` on fox
  and samus) and none of it is hardcoded. sandbag's fall: entry frames 0-5, loop 5-14.

  the ported loop-back call is then STRIPPED from a looping animation: fraymakers loops natively
  via `endType: LOOP`, and the label the call named was sliced off with the entry, so it threw once
  per cycle and looped nothing.

- **package size.** SSF2 authors a small looping element and the stage timeline it sits on runs far
  longer, so a naive port rasterises every repeat and writes a symbol per placement per frame. four
  lossless reductions keep that in hand:

  | where | reduction |
  |---|---|
  | element writer | a sequence that is one cycle repeated emits ONE cycle (the element loops anyway) |
  | element writer | an image used on more than one frame is written once and referenced again |
  | element writer | a large still picture with one small moving part emits a plate plus a crop of the part that moves; it samples first and declines when the element changes across most of its canvas |
  | `entity_compact` | identical symbols share one entry, invisible placements hold nothing, runs of identical keyframes fold into one |
  | `write_layer` | one file per distinct picture; a repeat is another reference, not another copy |

  clocktown lands at 29MB source / 10MB published and plays clean.

  what is left is the art itself: 376 distinct pictures, which is what the source draws.

  the per-angle bake is NOT part of that, contrary to the obvious guess. giving Fraymakers one
  picture plus a rotation would be smaller in principle, and both halves are known and tested
  (`fm_placement`: the engine turns an IMAGE about its stored x/y, and the decomposition
  round-trips at every angle). it is not built because it would never run: measured per object
  across the sample and beyond, no stage object CHANGES angle. pieces sit at a fixed angle, so the
  bake already emits one cel each. measure before optimising.

- **crateria converts and runs clean.** 2.4MB, 27 sprites, no script errors: 50 weather particles
  tethered to the camera (worst per-particle error 0) and spanning the view, the hazards switch
  wired, and both caution-sign hazards spawned.

- **a clip the mapping file does not name kept none of its scripts**. two passes named the
  same sub-MC differently: the sprite side reads the MAIN TIMELINE (which xframe label placed this
  clip), the script side guessed from the class name against `label_to_ssf2`. a variant the table
  does not list -- `DownSpecialAir` next to `DownSpecial` -- resolved to nothing on the script
  side, and a sub-MC that cannot be named is SKIPPED, so every frame script it had was dropped.

  sandbag's aerial down special had 0 of its 18 scripts while the grounded one had all 18: no
  input poll, no momentum, no floor listeners. it now runs them all, and driving it while airborne
  travels 2255 units on a held direction where it used to do nothing, with no script errors.

  the fix is to ask the timeline first and fall back to the label heuristic, so the two passes
  agree by construction rather than by both being taught the same names. what remains unnamed is
  what should be: ChargeSpark, CollisonBox, groundRef_mc, itemPlaceholder_mc.

- **sideways momentum reversed when facing left**. SSF2's `setXSpeed` is WORLD-space and
  fraymakers' is FACING-RELATIVE, so copying the call across is only right while facing right.
  measured in both engines rather than argued about: facing LEFT, SSF2's `setXSpeed(15)` moves the
  character RIGHT (+147) and fraymakers' moves it LEFT (-305); `setXSpeed(flipX(15))` facing left
  moves RIGHT, matching. so the argument is wrapped in `flipX`, which is what turns a world-space
  number into the facing-relative one fraymakers wants. a literal zero is left bare -- a stop needs
  no direction. `setYSpeed` needs no wrap; there is no facing on the vertical axis.

  `setXVelocity` is NOT the world-space alternative: measured airborne, it moves the character not
  at all.

- **a speed reached through a variable was not converted**. only literal arguments were
  rescaled, so a move that dashes at a constant its constructor assigned (sandbag's aerial down
  special: `leftSpeed = -15`, `rightSpeed = 15`) ran at the raw SSF2 number -- about 15 per frame
  at 60fps where 9.75 is the conversion. an argument is now scaled unless it came back OUT of the
  engine (`decel(getXSpeed(), …)` is already in fraymakers units and would shrink each frame).

  verified live on that move: from BOTH facings, holding right dashes right (+385, +386) and
  holding left gives -8.7 velocity, where before it ran at the raw speed and reversed facing left.

- **a still backdrop is no longer re-drawn because something moves over it**. SSF2 draws a
  backdrop as many shapes on many depths inside ONE clip, and the art walk flattened the whole clip
  to a bitmap per frame. clocktown's town backdrop is 739x436 with a clock hand turning two hundred
  pixels away, so it emitted that whole picture 623 times: 3379 sprites, 263MB, and FrayTools would
  not publish it at any timeout.

  a clip's shapes now split by the DEPTH each sits on, but only where it pays: a depth whose content
  changes becomes its own element, every static depth stays merged into one composite, and a clip
  that is entirely static or entirely moving is left whole. that last part matters -- splitting a
  group where everything moves trades one shared canvas for several union-of-path canvases that are
  mostly empty, which DOUBLED bowserscastle's art when the rule was unconditional. a blended group
  is never split either: a hard-light mask is baked by compositing it over the art beneath it.

  clocktown: 263MB -> 81MB, 3379 -> 1550 sprites, its town plate 664 frames -> 10. it exports (an
  18MB .fra, ~110s) and renders with no script errors. bowserscastle: 8.8MB/739 -> 8.7MB/732, i.e.
  untouched.

- **art the source MOVES is emitted once and moved, not baked per angle**. a turning clock
  ring is a handful of cels and a matrix per frame in SSF2; rasterising each frame stored it as a
  bitmap per ANGLE -- clocktown's rings alone were ~67MB across five elements whose every frame was
  unique. an element whose frames show at most ONE shape each (from a small set of cels) is now
  emitted as those cels, once, with the per-frame orientation carried on the frame. "at most one
  per frame" is the guard that separates a few things moving from a picture redrawn out of many
  shapes, so the cel ceiling can be generous.

  position by CENTRE, not by bounds: the art is the unrotated cel while the source's box is the
  rotated one, so placing by top-left slides the piece as it turns (visible as the clock's rings
  drifting apart). rotation keeps a centre still, and fraymakers draws an IMAGE centred at
  `x + scaleX*w/2`.

  clocktown: 263MB/3379 sprites -> 7.5MB/245, exporting in 25s to a 3MB .fra and rendering with the
  rings concentric and no script errors. bowserscastle: 8.7MB/732, untouched throughout.

  the conversion from an SWF placement to a fraymakers IMAGE is now ONE piece of code
  (`fm_placement`) shared by the character and stage paths -- SSF2 content is SSF2 content, and an
  arm on a character is placed by the same kind of matrix as a hand on a clock. the character path
  moved onto it with its golden snapshot unchanged, and mirroring (scale, position AND rotation)
  lives there too rather than being spelled out twice.

- **the hazards switch is ported, not decided**. both engines let a match turn hazards off.
  in SSF2 the engine does not apply that to a stage: the stage ASKS (`SSF2API.isHazardsOn()`) and
  chooses what to skip, and 47 stages in the corpus ask without agreeing on what it means. so the
  converter reads each stage's own answer out of its bytecode (`abc_parser::extract_hazard_gate`):
  the gated region is the branch after the call, and what counts as gated is what the stage SPAWNS
  in there. a stage that never asks runs everything regardless, and so does its port.

  on bowserscastle that comes out as `gated_classes: ["Thwomp"]` -- the faller is spawned inside the
  branch, the lava is placed regardless, and the embers update BEFORE the check. verified live both
  ways: hazards on = lava + thwomp + spectator, hazards off = lava + spectator, thwomp gone, all 36
  structures still there and the fighter still standing on them.

  the harness had been hiding this. peptide replaces the whole match config on a headless launch, so
  a field it never set read as false and every test match ran with hazards OFF. `hazards` is now a
  match setting like lives and time (`match_settings.conf`, default on), which also makes the off
  case testable on purpose.

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
count without adding new ones, and the in-engine spawn sweep still has to pass. read it
alongside the amount of code emitted, though: a decompiler fix that recovers a previously
dropped body ADDS markers, because the untranslatable calls inside it were invisible while
the whole body was missing. seven characters are fully `/* ? */`-clean. the
decompiler-quality outliers worth a focused pass are `bowser`, `kirby`, `dedede`, `rayman`,
`krystal`, `sora`, `lucario`, `wario`.

the other half of the measurement is `conversion_log.json :: unknown`, the per-character
tally of call sites the converter doesn't recognize. it counts only genuine gaps: names a
regex mapping emits, the character's own helper functions, and Haxe stdlib methods are all
discounted, so an entry there means "no Fraymakers equivalent" rather than "the converter
didn't recognize the name". corpus total is 1619 calls across 55 names (see "character
system gaps" below).

## what the verify gates found on the sample

the ten stages and ten fighters chosen for coverage, run through every gate. the element
count sweep had called all of them clean; the gates found eight port bugs underneath, each
general and each now fixed:

| found by | what it was |
|---|---|
| S3 sector z pinned the engine | a ported class kept each getter as a value worked out once and dropped its setters to field writes; a frame timer's `completed` read true from its first frame, so the stage spawned an arwing and fired the gun on every frame (accessors are code now) |
| S3 sector z, 256 errors a match | `gotoAndStop(N)` landed on `playFrameLabel(N)`: a frame number is not a label (`playFrame(N)`) |
| S3 galaxy tours, an error every frame | a receiver the port could not answer was written as the literal null and the call left standing (a call on nothing is a note) |
| S4 fourside, spaceship gone on frame one | the flag that lets a class survive the death bounds was dropped; the engine destroyed an object the source parks above the stage (a self-platform object survives the bounds) |
| C4 link walked at 0.33 a frame | his bomb class, which extends the item base, was folded into the fighter and its update exploded him out of every walk (a class that is its own object stays its own) |
| S3 sector z, written for a comparison | the persistent-field rewrite matched the first `=` of `==`, so `if (m_decision == "shoot")` became a write (a comparison is not a write) |
| C1 zelda and kirby, four divergences | the parity checker truncated fractional source damage to an integer |
| C5 several fighters | none: every remaining difference was the gate's own launch condition, told apart by the evidence (parked into the blast zone, cut by landing, a fall that loops, a reply paired with the wrong question) |

still open, and what decides each:

- sandbag walks at 3.4 a frame against a declared walk cap of 5.2, which equals its dash: its
  walk-in frame script sets 3.25, and SSF2 driven the same way walks at 3.67 a source frame,
  which scales to 3.49. the walk is faithful; the STAT is the odd one out (the extractor takes
  the dash figure for the walk cap when the source drives the walk from its frame scripts),
  and C4 now accepts a speed the fighter's own walk scripts set.
- three fighters keep a dash attack seven to ten frames past the emitted animation while mario
  is exact, and several moves end early by their own code. whether the source keeps the same
  frames is C6's question too; C5 reports both and rules on neither.
- sector z's arwing flies, decides and fires (live): its update reads as the source wrote
  it, the flight branch closed and the decision block and state switch on their own. the
  decompiler's merge finder takes a branch whose taken side sits after its fallthrough side
  (the fallthrough region's jump past the taken side's start is the merge); before, that
  taken side ran to the end of the method and swallowed everything after it.
- saffron city passes every gate: its two platforms run their class's cycle (rest, a target
  higher up, back, with the 60-frame sway on top) and carry their art with them. two general
  faults sat under it. an object's scripts converted places through an origin worked out from
  the places the stage's code states, and this stage's pokemon spawner has no art, so the
  parser had it at 0 and the median put every object's sums 700 px off the terrain; where the
  source has a terrain clip its registration is the origin now. and a clip group answered its
  class's moves from 0, because the group was built under the script's alias for a name that
  clashes with a stage field while the tree's box sat under the source's name; the group goes
  by the source's name, and keeps its place in its plane's space. `tools/floor-profile.py`
  came out of it: a fighter dropped at every x on both engines, rest heights side by side.
- C6 runs: the same inputs on both engines, each engine's own per-frame trace, the move
  named through the fighter's own map. what it has found so far, and what stands:
  - six fixed: a listener for touching the ground fired while standing (a landing is a
    transition now), a grounded special dropped to a fall the frame the fighter rose (grounded
    attacks keep their animation airborne now), a jab handing its control check to a timer got
    a helper clip's check with the same name (only the timeline's own renames reach frame
    scripts now), and every jab chained into its second hit on a single tap because "how long
    since this move was last used" had been struck to an empty object (the script keeps that
    count now), fox's jab found a flag already set because a field nobody declares took its
    initial value from the first literal write in file order, which was another clip's frame
    script (only a constructor write is an initial value now, and a helper clip's own fields
    start at their defaults on its first frame), and two clips that carry the same method name
    both reached the one that was kept (a clip's own methods are wired by label now).
  - and two more since: a helper clip's lead-in frames, the ones before its first labelled
    frame that no piece shows, dropped their scripts (a jab's setup, which reads the
    fighter's controls and the shared jab flags, never ran; they run on the first frame of
    the piece that follows now), and a jump to a label inside the same clip (link's bow
    jumping to `attack`) was read as a change of animation and struck (a frame jump now).
  - a timer the source runs every frame ran every engine frame in the port, twice the
    source's rate: a one-frame tap read as held across two of its checks, and samus's charge
    shot took that for a second press and stowed itself (the whole class of charge and hold
    specials was affected). a per-frame source timer now fires once a source frame, like
    every other frame count, under either name the source arms it by (donkey kong's punch
    armed its check by the other one and fired on a tap). the attack table the fighter reads
    its own numbers from is in the source's units for the same reason: the counters that read
    it tick once a source frame.
  - what looked like the source ending attacks early with no script saying so (mario's
    neutral special playing 17 of 23 source frames) was the corpus copy being another build
    of the fighter than the installed game's own data: the game's mario has an 18-frame
    fireball clip, the corpus a 23-frame one. the gates convert from the installed game's
    file when it is present (TESTING "the verify gates"), so the two halves of a comparison
    read the same content.
  - read from the game's own data, the five fighters it was run on (mario, fox, samus, link,
    donkeykong) pass every gate, after three more port fixes it surfaced: a method's jump to a
    label is resolved by the animation playing (inside it when it carries the label, else to
    the animation the label starts; link's bow and its charged smash both say `attack`), an
    attack's own numbers travel with it and a read of one looks them up by the animation
    playing (fox's reflector read its charge cap as nothing and released at once), and the
    terminator trim repeats while the frame now last still ends the move (fox's blaster ends
    on its last two frames). the driver puts the fighter back where it started between inputs
    and holds a direction before a side special, since a direction pressed on the same frame
    reads as a flick on the source (samus's super missile).
  - all ten sample stages pass every gate from the game's own data, the cross-engine half
    included.
  - a comparison on a field the fighter has not written yet went the other way from the
    source's undefined (samus's charge shot read a null charge as already full and stowed
    itself): the fighter's own fields compare through helpers that answer false for nothing.
    a clip's own frame read as the bare timeline property was struck before it was
    translated, and the block around it with it (fox's reflector); it is read first now.
  - an attack that declares a charge time and does not declare its own charge handling is
    charged by the source's engine while its clip sits in the "charging" label (held it stays,
    let go it plays "attack", run to its charge time "finishedcharge"; link's spin attack has
    no scripts of its own for any of it); the port's fighter update does the same. inferred
    from three fighters measured live and stated here as such.
  - a grounded attack that hops (sandbag's up special tosses a bomb from a short jump) was cut
    by the landing in the port, where the source runs the clip to its end: a grounded attack
    keeps playing when it comes back down now (`landType` NONE beside the `leaveGroundType`
    NONE the template already gave it).
  - two of what looked like the source's engine supplying a mechanic were the gate and the
    port: sonic's homing attack found a second fighter on Fraymakers (training mode's dummy)
    and nobody on SSF2 (a solo roster), so both engines get the same two-fighter roster now
    with the other fighter parked the same distance away; and pikachu's thunder cloud was
    launched sideways at the template's default speed and at the fighter's own height, since
    a projectile that states no x speed now launches with none (the source's cloud parks, its
    bolt drops straight down) and a slot a clip only reads, declared with a value, starts at
    that value (the cloud's height above the fighter, which the clip never writes).
  - still open: donkey kong's punch keeps charging after a tap where the port lets go, and
    sonic's side special ending in a fall in the port and on a ledge hang in the source.
  - the walk, held the same fifteen source frames from a stand on both engines and read off
    each engine's own per-frame record, is a new comparison in C6 and it finds two gaps: the
    source's zelda walks up to 5.7 a tick and holds it (the converter takes 4.0 from her
    declared figures, so the port walks 2.6 against the source's 3.7 scaled), and sandbag's
    walk is a hop on both sides but not the same hop: its walk clip sets 3.25 and then 0.65 in
    the source and in the port alike, yet the source moves 6.1, 7.2, then 8 a tick for five
    ticks. both gaps sit in how the source's engine turns a declared or set speed into motion,
    not in the scripts, and its speed model is the thing to measure next.
  - closing numbers, all ten fighters run on the finished driver from the game's own data:
    all ten pass every gate (mario, link, samus, kirby, sonic, donkey kong, fox, zelda,
    sandbag, pikachu). all ten sample stages pass every gate. two fighters take under a minute
    and a pair of stages about two and a half minutes, the games left open between runs and
    every subject's match started in them; a conversion is about eight seconds (the
    converter's patterns compile once).
  - donkey kong's jab connects with a fighter standing 150 source units away in the port
    (195 of its units, the world being scaled) and misses one 150 units away in the source,
    measured with `hits` on both; the input gate parks the other fighter at 250 so a hit's stop
    lengthens neither side's move, and which side's boxes differ is open.
  - the next ten fighters, run the same way: luigi, marth, peach, yoshi, ness, jigglypuff and
    metaknight pass every gate (yoshi's jab was the second kick until the jab cut read the
    clip's unlabelled head; ness's thunder poll and jigglypuff's roll end had no timer slot;
    metaknight's tornado check was decoded with its end test inside the wrong branch, its
    down special never restored gravity because the special-mode read was struck, and its
    idle jumped into the walk's `loop` label). bowser walks at his cap now: the source's engine
    overrides a walk clip's own speed writes every frame (his clip writes 2.0 and 0 while he
    walks a flat 2.46 there), so a literal x-speed write inside the animations this engine walks
    is dropped. pichu's down special and captain falcon's down special match now: a clip whose
    move ends on its LAST frame shows that frame in the source (pichu's 60th of 60), where a
    move ended mid-clip never shows the frame that ends it (marth's down special's 31st of
    40), so the port keeps a clip's last frame and runs its ending one frame past it.
    megaman walks at 4.31 a frame against his 4.42 cap now that an object's own setup runs
    where the tree put it, and passes every gate.
  - a third ten: rayman, tails, waluigi, wario, pichu and sora pass every gate (pichu and sora
    booted into `Unknown variable` errors until the fighter preamble carried the clip-clock and
    for-each helpers the stage objects already had; pit's only failure was the parity checker
    counting a source move with no hitbox as missing). the decompiler's chain combiner reads any
    nesting of `&&`/`||` off the jump targets now: lost conditions across the corpus went from 49
    to 18, and the 18 left are calls through a name computed at run time
    (`obj["get" + type + "Stat"](..)`), which it does not read yet. simon passes now too: its
    whip jab exits once a cancel of a timer nothing set is a miss, and its up special lands by
    the fighter's own update (DEVELOPMENT §6.1b). megaman passes: his neutral special's own
    `update` method no longer runs as the fighter's per-frame hook. open: bowser's walk and
    captain falcon's down special.
  - a fourth ten: bandanadee, blackmage, falco, isaac, goku and chibirobo pass every gate
    (goku's six specials each declare a `checkControls`, kept per class now; chibirobo's
    blaster ended against the animation's name where the source asks the clip's own label).
    bomberman's held bomb releases now (a fully charged move still lets go into `attack`), and
    game and watch's release timer starts with its declared default. open, one input each and
    all of one kind: the up specials of bomberman (48 against 78), dedede (104 against 114),
    game and watch (75 against 108) and ganondorf (40 against 56, and its down special 38
    against 64) came down early because the port held their speed where the source only caps
    it: `air_ease` is a fall cap, and a fighter's script speeds cross unchanged, with one
    source frame of gravity on a y write (§7 of DEVELOPMENT). ganondorf's, simon's and game
    and watch's arcs now peak on the source's frame, kirby's up special ends into stand on the
    ground instead of sinking through the floor, and sandbag's scripted walk stops where the
    source's does.
  - the last seven: lucario, luffy, naruto, pacman and zamus pass every gate; krystal's up
    special (54 against 60, then a fall where the source lands) and lloyd's down special (29
    against 78) are open, and neither declares an air ease.
  - the whole roster on both engines, on the same stage: 43 of 45 fighters pass every gate
    (re-run after the walk rule and the last-frame rule of 2026-09-10: every fighter but marth
    and sandbag passes all six). sandbag's up special makes a bob-omb and tosses it, and with
    one already in hand the source blows itself up three frames in (hurt, flying, falling);
    this engine has no item system, so the port plays the clip through. that is the item gap
    under "character system gaps", not a timing fault.
    what stays open is marth (an up special that runs
    50 against 30 and ends helpless where the source lands: both rise the same height, and
    the source's platform catches feet 8 px under its surface where this engine's does not,
    so the source lands on battlefield's top platform and the port falls past it) and wario
    (a down special whose length is the charge the match has built up: played alone from a
    stand the two engines match frame for frame, and the gate's runs reach it with different
    match times behind them, a harness limit).
  - a speed a script states crosses the boundary now (DEVELOPMENT §7): pikachu's bolt lands on
    the source's frame, and sandbag's hop walk averages the source's over the same fifteen
    frames (2.72 a frame against 4.19 scaled, within a percent). the source still starts its
    walk at three quarters of its cap on the first frame where the port accelerates from
    nothing; the average hides it and the per-frame record does not.
- the stage half of the cross-engine gate (S7) pairs each Fraymakers element to its source clip
  by name through the converter's plan, and what that surfaced on the sample:
  - a nested clip the stage keeps up came out baked over the stage's whole timeline (bowser's
    castle's 82-frame podoboos as 142-frame elements that restarted with the stage). a clip the
    stage keeps up loops on its own clock now, its period read off the pictures and decided
    once per clip from the slots that show something.
  - three stages drive every plane across section labels from their own code (castle siege,
    mushroom kingdom, galaxy tours: `getMidground().playFrameLabel("transition1")` and the
    like), and the port played each plane's timeline end to end with the jumps struck. a
    label-bearing plane clip is a state machine now: the track builder hands its timeline
    over as label-delimited segments, each an animation its group answers to by name; the
    stage's painted art carries the terrain clip's labels as frame labels; and a jump the
    stage makes on its midground or its mask fans out to every plane group and to the stage's
    own animation. what a section changes in the COLLISION is the stage class's own doing
    (the named terrain platforms it shows and hides), and stays as ported.
  - a section HOLDS. a plane's clip stops itself on a section's first frame (`stop()`, or a
    `gotoAndStop` onto the label) and waits for the stage's code to play the transition; read
    as a loop, every plane cycled through all twelve sections on its own and the stage
    flickered between its art and nothing. a segment ends at the first scripted frame inside
    it, a stop holds its last frame, a `gotoAndStop` target keeps one frame, and a segment only
    the stage's code reaches by name is still reachable. the stage's jump is left with each
    piece (`exports.__go`) and taken up by the piece's own script, which owns its place.
  - a piece of the terrain slot is never drawn. every clip placed there hides itself on its
    first frame (its class says so), and a clip that starts hidden and that no code ever shows
    is not art: painted in, castle siege's terrain sections sat over the castle in the
    collision grey, and moved with the transitions as objects of their own.
  - a camera background whose timeline is a state machine (castle siege's sky) is objects in
    the engine's own containers for it: each piece gets a blank camera-background entry of the
    plate's size and its declared rate, and its object is parented into
    `match.getCamera().getBackgroundContainers()[k]`, drawn at 1 in the picture's own pixels
    from the entry's centre. the engine places and pans it with the plate; the piece's script
    switches its section. the tree's own copy of a declared background is dropped (the game
    makes a camera background from the library, never from the placement).
  - an SSF2 session opened on a stage alone used to auto-launch with the other engine's
    default fighter, which SSF2 does not have: the stage drew and its class never ran (no
    stage data, no timers, no hazards), and a match after it on the same engine never
    started either. an SSF2 session takes SSF2's own default fighter now. castle siege moves
    top to middle at thirty seconds on both engines, and bomb factory spawns its blocks on
    both.
  - a hazard state the source enters without starting a clip (the thwomp's fall and rise)
    is registered under the clip that carries on and entered through the engine's value-only
    setter. left out of the registry to keep the clip playing, the engine refused the move and
    the thwomp fell onto the floor still in its entrance state.
  - a stage class's per-update counters were written for thirty frames a second and the port
    called its update sixty times a second (castle siege moved through its sections in half
    the time); the source's update now runs every other frame.
  - clips the source places at no opacity (bowser's castle's sixteen ember emitters) are
    placeholders the stage's code reads positions off; the SSF2 tree states opacity now and
    the checker leaves them out. a fighter's pose clip and a stage-root child of the game's
    own are left out the same way.
  - an object the package declares that the port defers (clocktown's fairy is an item, and
    there is no item system to port onto) is named in the plan, and the checker reports it as
    left out rather than missing.
  - the checker itself: anonymous children of one clip are as many objects as they have
    lengths, a clip's elements are claimed across its siblings before anything is a mismatch,
    a loop that divides the clip's length is the same pictures, and an element whose whole
    timeline came through at length has its labels read as marks on it, not branches.
  - tilts and smashes have no input that means the same thing on both engines and are
    left to C5.
- the engine-bound gates take `--speed=N` (several engine frames a tick, measured 4.1x at 4)
  and every wait in them is sized in engine frames. mario's whole run is 75s at speed 4, the
  move gate 26s of it.

## how a port is verified

`tools/verify.py` (TESTING "the verify gates"): the converter writes a plan beside every
package and the emitted script reports on itself from inside the running engine, and a subject
passes only when the two agree gate by gate (convert, publish, boot, built, motion; for a
fighter also physics and every move). what is still open on top of that is the cross-engine
half: `--ssf2` runs the same stage or the same inputs in SSF2 and compares, and a move that
its own code ends early is left to that gate to rule on.

---

## nested clips as vfx (branch `stage-vfx-containers`)

a nested movieclip is two things at once: it loops on its own few frames, and the stage's timeline
places it for a stretch and takes it away. today those are folded together and both come out wrong.
carrying the stage's timeline inside the object loses the loop (clocktown's 28-frame rain splash
becomes a one-shot as long as the day, shown once instead of the 265 times the source plays it);
keeping only the loop loses the placement, and it plays through the hours the stage never shows it.

the shape being tried: one vfx per nested clip, added to the container the clip hung off, looping
forever, with the stage timeline's handling of it translated into code that moves and removes it.

what the engine gives us, measured live:

| capability | verdict |
| --- | --- |
| `Container.create()` | works -- containers of our own ARE available |
| a container's position | plain `x` / `y` properties (no `setX`) |
| `addChild` onto a container we made | present |
| `createVfx`, reference held across frames | works |
| `setX` / `setY` on a vfx | works (set 300, read back 300) |
| `destroy` / `dispose` | both present |
| `loop: true` | loops forever, no code needed |
| `addChild` into a stage container | works, already how elements are placed |
| a child follows its parent container | CONFIRMED (hide the parent, the child goes with it) |

so the shape is supported end to end: make a container, add it where the clip hung off, put the vfx
in it, and move, hide or drop the container. measured with a probe holding art the stage has nowhere
else: 176-198 px of it on screen with the parent shown, 0 with the parent hidden.

every backdrop object now lives in a container of its own, which is the substrate the rest needs.
the schedule comes from the SOURCE: which frames of the stage's timeline PLACE the clip, read off
the timeline rather than guessed from what got rasterised. inferring it from the baked frames --
treating a blank frame as "the stage is not showing this" -- broke the clock twice, because a blank
frame means two different things: the stage has taken the clip away, and the clip's own cel is empty
right now.

the placement list alone is not the schedule either. a piece inside a nested clip is placed on its
OWN cycling timeline, so sampling it against the whole stage leaves a couple of frames on, a couple
off, all day -- clocktown's four scheduled objects came out with 797, 611, 549 and 468 runs apiece.
switching an object at that rate is the clip's animation read as if the stage meant it, and it costs
the object its motion too, since it is only ever up for a blink: torches that flicker, splashes
frozen on one cel. so an absence shorter than `REAL_GAP` (60 sampled frames) closes up and stays
inside the object. the four come out at one run each, spanning the day.

masks: no SWF stage in the corpus uses `clip_depth`, the only thing in the format that marks one --
measured across the sample, zero mask tags. so foreground occlusion cannot come from a mask in the
source; it comes from the plane the art sits in. the reader carries `clip_depth` anyway, since a
reader that drops it cannot tell a mask from art laid over the stage. what clocktown does carry is a
`shadowMask` clip, picked up from the unfiltered walk and emitted as a real `shadowLayers` entry with
its own mask sprite. whether the engine then draws a shadow from it is unverified: the live bridge is
wedged (see below).

a camera background's children have no names to carry. measured on clocktown: the clip holds 8694
placements over its 3720 frames and NOT ONE of them is named, while the background and foreground
planes name four children each -- which is exactly the set the stage class drives. so the clock and
the white flash are anonymous siblings in one clip, told apart only by draw order, and "give a
camera-background child its own name" is not a thing the source supports. anything left in the
layering between those two is an ordering question, not a naming one.

the moon-impact flashes cannot be ported from this stage file. the class names them, and the class's
own plane list calls them camera backgrounds -- but there is NO placement of them anywhere in the
`.ssf`: not on the root, not inside any sprite, and not in the declared camera-background list, which
holds only `clocktown_bg`. the art is not in the file, so the engine supplies it. nothing to port and
nothing to drive; the converter says so rather than implying art went unnamed.

## stage game objects

**what an object IS comes from the class graph, not from its name.** SSF2's api layer declares
the hierarchy, every package carries its own copy, and a stage's objects extend it:

```
SSF2BaseAPIObject -> SSF2GameObject      -> { Item, Enemy, Projectile, Target, Beacon, Character }
                  -> SSF2CollisionBoundary -> { Platform }
```

so "is this a game object" is ancestry and "what kind" is the base it reaches, both read by
`ssf2_objects::game_object_classes`. both halves count: a stage collides with a platform the same
way it spawns an enemy, and each declares its class the same way. discovery does not depend on
finding a spawn call: the spawn-site scan only sees an actor when the class reference and the
`spawnEnemy` call are close enough for the stack sim to carry the tag between them, and it is kept
only for the literal spawn coordinates it can read there.

**a stage PLACES objects as well as spawning them.** a clip in the tree states its own role on
frame 1 (`type = "platform"`) and the class the game builds over it (`classAPI`), and the stage's
code then reaches it by the name it was placed under (`getPlatform("left_plat")`). all three are
read: the class runs its own code, the collision clip it was declared over becomes its body, the
placement name files it so the lookup resolves, and a clip the stage HANDS the object
(`setStance(getMidground().plat1)`) becomes the object's art and is left out of the painted stage
so it is not drawn twice. central highway's two platforms and two pillars come through this way.

**a class a stage names does not have to be in the stage file.** the game keeps a handful of
packages loaded the whole time, so a stage can ask for a class it never shipped. the manifest says
which packages those are, and a name this package cannot define is looked for in all of them: the
one that answers is where the class ports from, along with the clip it names for itself. crystal
smash's crystals live in the shared enemies package and port from there.

**one terrain clip is not one floor.** a shape can hold several pieces stacked over each other, and
each is ground. every column's runs of solid pixels are traced, and runs that touch across columns
are one piece, so a stage that draws its whole structure as one clip (crystal smash) keeps the deck
under its fighters instead of only the highest ledge.

**an object can ask to be drawn ahead of the fighters** (`setForeground`), and it is: its drawing
moves into the foreground container, and when the name is a separate clip rather than the art it
already draws, that clip ships too and is kept on the object. hueco mundo's platform carries its
front piece; crateria's and fourside's move forward. ordering the plane the objects share
(`setChildIndex`) is the same move on the game-objects container.

**every method a stage class and its hazard classes define has a counterpart in the port.** the
check is mechanical: decompile each class, list the methods that are real code, and look for each
one in the emitted script (a declaration method like `getOwnStats` is read at convert time and
becomes the object's stats rather than a function). across the ten sample stages that comes out at
zero missing, stage classes and hazard classes alike.

what is left is per-LINE, and each struck line now carries the reason it was struck, so an audit
reads as decisions rather than holes. what remains on the ten:

| left out | why |
|---|---|
| star/screen KO switches, final-smash queries, item bounds, the smash ball region | Fraymakers has none of these to port onto |
| a per-object camera box (`setCamBoxSize`) | the object is handed to the camera as a target; there is no box to hand with it |
| the spawn beacons as objects (`getSpawnPositionMCs`) | a converted stage's spawn points are stats, not objects a scrolling stage can move |
| eight sounds (bomb factory's blast, crystal smash's shield hits, clocktown's balloon) | no package the manifest declares carries them; the converter names them |
| crateria's midground phases (`playFrameLabel("rain")`) | the stage's own art ships as one animation; per-phase stage art is not built yet |
| butter building's ledge switch-off | inside its tower rise, which cannot run (see the table below) |

**the engine's own scripts report "Invalid function null" too.** with script errors surfaced, the
engine's shared character script and stock content raise the same thing on a hazard-free stage with
no converted content in play, and its shared game-object script does it once per hit a stage hazard
lands. it is the engine's, not the conversion's: what the stage's own scripts raise is zero.

**hazard vs decoration is not a distinction the porter makes.** each class's own declarations say
what it does, and they are read per class: `getAttackStats` hitboxes, `getOwnStats` scalars, the
`forceAttack` labels naming its art clip, the stepped-out behaviour, the reconstructed script. an
object with no attack box does not hurt anyone because its code says so. clocktown's TingleBalloon
is an SSF2Enemy with ZERO hitboxes and a rideable self-platform; its ClockTownFlamingRock is an
SSF2Enemy with one. porting the code reproduces that without anyone classifying anything.

**SSF2Item is deferred, and not because it is hard to find.** Fraymakers has no item system, so a
pickup has nothing to port onto: carrying, throwing, effects, respawn all have to be BUILT first.
that is a project rather than a conversion and wants direct involvement, so an item is reported as
deferred instead of shipped half-built (`ssf2_objects::ITEM_BASE`). every other base should ship.

full audit of all 110 stages: which have scripted actor/hazard classes, what the converter
handles, and what's a declared gap. run via `ssf2_objgraph <stage.ssf> scripts` +
`ssf2_objgraph <stage.ssf> classgraph` + the debug-info pass. corpus conversion: PASS=110 FAIL=0.

### every stage, measured

Two questions, asked of all 110 rather than of a sample, because a sample cannot say whether a
stage nobody looked at kept its objects.

**what it is made of.** Each conversion says so in one line: how many objects the stage declares,
how many carry their own code across, and whether the stage's own class did. 110 of 110 convert;
every stage's own class ports; 162 of 163 declared objects bring their own code, and the one that
does not (`KrazoaWind`) declares nothing but stat getters, so there is no behaviour to bring. Two
stages name a class they do not define and both resolve from the shared package the game keeps
loaded (crystal smash's crystals, with their eighty animations).

**what still moves.** `tools/tests/stage_inventory.py <stage>` pairs every placed SSF2 object
against what was emitted; `tools/tests/sweep-stages-inventory.sh` does the whole corpus at once.
1,286 objects ported, 2 animations lost across 2 stages, 108 of 110 stages clean.

| | was | now |
|---|---|---|
| stages that convert | 107/110 | 110/110 |
| animations lost | 168 | 2 |
| stages with nothing lost | 45 | 108 |
| plane/layer errors | 11 | 0 |

**what draws in front of what.** Zero across the roster. The eleven flagged before were every one
a ported platform sitting in a structures container, which is where a structure goes whatever
plane the source drew it on, not a claim about depth.

three converter fixes moved most of that, and the rest was the check being wrong:

- **three stages had no ground at all.** a platform that becomes a structure carries its own
  collision and is skipped in the stage entity, but the skip also caught any platform the stage's
  code merely NAMES, and nothing emits a structure for those. the engine refuses a stage with no
  floor, so galaxytours, urbanchampion and warioware were refused outright. naming is not driving.
- **a stage places objects in its terrain container, not only collision.** the container was
  excluded whole, and the collision pieces in it already say what they are in their own linkage,
  which the filter beside it already reads. everything else placed there went with them: rainbow
  route's mirrors, the palace's pillars, a target test's targets.
- **an element on the stage's own plane keeps its own loop.** the plane is painted as one picture
  and a picture carries one timeline, so an element on it only went on moving when everything
  around it moved in step.
- **the check blamed the wrong objects.** it paired one at a time, so an early object took an
  element an exact match later needed; it counted a long clip whose art never changes as a lost
  animation; it could not pair a clip ported as an OBJECT, since a hazard entity is named for its
  class while the source knows it by the clip it draws with; and it let a `stance` -- an object's
  own art -- compete with the object it belongs to. roughly two thirds of the original 168 were
  never losses at all.

what is left is two, both understood. emerald cave's embers are a burst of particles each alive
for a frame, so read as objects they are correct objects that each hold still, and the movement
belongs to the emitter cycling through them rather than to any piece. skull fortress's `pipi` is
placed at the DOCUMENT root rather than inside the stage container the art walk descends into, so
it is never seen at all -- one decorative clip on one stage, against a walk that is right for the
other hundred and nine.

two checks answer the whole corpus at once, and both are clean:

- `tools/check-emitted.py <dir>` reads every emitted script for what the engine would refuse to
  parse. 33 of them once, then 9, now none. the engine drops a script it cannot parse WHOLE, so
  each one cost a stage every object it was going to make -- for a bracket, twice over.
- `tools/tests/unanswerable-calls.py <dir>` lists every call to a name neither this engine nor
  the script itself has. twenty names across fourteen stages once, now none. such a call does not
  fail quietly: it takes the rest of the frame with it, every frame.

both were written after driving stages one at a time proved too slow to be honest about -- an
hour a sweep, against a binary that kept changing under it. the scripts already say which names
they use, so the whole corpus can be asked at once, and the answer ranks causes by how many
stages each affects.

### a stage's objects, and the scope its methods run in

Six things kept objects a stage spawns from running at all, every one of them silent:

- a timer built INSIDE another call swallowed that call's closing paren, and a struck multi-line
  call left its tail behind. both leave a script the engine cannot parse, and it drops such a
  script WHOLE, so the stage lost every object that script was going to spawn.
- the source's api object READ on its own (`_v1 = SSF2API;`) threw, taking the handler with it.
- an AS3 cast read as a call: `MovieClip(x)`, and `EmeraldCaveWoodenPanel(x)` for the stage's own
  classes. the class graph names those, so there is no list to keep.
- an object's own methods were asked of the ENGINE's handle rather than of what the object
  publishes, so configuring it was refused and the rest of the stage's setup went with it.
- a field that holds TEXT, or whatever a caller hands it, was given a number store, which refuses
  the value outright. emerald cave's panel keeps which side of the frame it is in one of those.
- a line built on a temp whose assignment was struck threw every frame.

**and one thing that is the engine's, not ours.** A function invoked through another script's
exports runs WITHOUT the engine helpers its own script has. Fourside's spaceship calls
`toLocalState(1)` inside `activate()`; the same call in its `initialize()` is fine, and
`Common.toLocalState` is no better -- `Common` is not in scope there either, checked live by
patching the emitted script and re-exporting. So a state change driven from outside cannot be a
call: the object has to be signalled and change state in its own update, where its scope is whole.

Worth knowing when reading these numbers: an object that cannot be configured reports NO errors,
because it never runs. Fourside read clean while its spaceship sat inert, and reads as two errors
now that the stage can reach it. Routing an object's methods through its exports is what changed
that, and it is the same change that got emerald cave's panel working.

### a backdrop is objects, not pictures

A camera background is carried the way the source keeps it: one image per shape, with the motion
in keyframes. Composited instead -- a picture per frame -- the motion is baked into pixels and the
cost explodes, because what was one still sky and three sliding cloud shapes becomes hundreds of
full-width copies of the pair.

| stage | composited | as objects |
|---|---|---|
| castle siege | 957MB, 720 pictures | 12MB, all 720 frames |
| dream land | 121MB, 1512 pictures | 4.4MB, all 1512 frames |

Neither can be published at the composited size: FrayTools never finishes, the engine keeps
whichever package was installed before, and the stage is not in the game at all. Both now publish
and run with every element live.

Nothing is sampled, dropped or quantised. It is the same art, and the frame count and cycle length
are unchanged -- only where the motion is kept. The timeline walk already did this for a backdrop
the placement TREE puts down; one the package NAMES never reached that walk and was composited.

It applies where a backdrop is more than one object. A backdrop that is a single object is the
same picture either way.

### every stage, run

All 110 were driven, not just the sample: converted, published, and run with a stock fighter
walking, jumping and attacking over each one, then left to tick while the error channel was read.
Every one of them comes back with nothing on it.

That took 68 fixes, and almost none of them were about a stage. The shapes that kept turning up:

| what it was | what it cost |
|---|---|
| a script the engine could not parse | the whole script, and every object it makes, silently |
| an instruction the decompiler had no reading for | every read after it, one thing to the left |
| a name this engine has never had | the frame, every frame, wherever it was asked |
| a slot whose only filling was struck | the frame, one line after the strike |
| an argument list written as "no brackets in it" | whatever the source worked out inside one |

The parse failures are the ones worth naming, because they are invisible: the engine drops a
script it cannot parse WHOLE, and the first symptom is a frame reporting an unknown variable for
a name that is right there in the file. Nine stages lost a script that way, each to a single
character. `just sweep-stages` now reads what it emitted and refuses the shapes that do it -- a
close with nothing open for it, a block that never closes, an index reached through a dot, a
number run into what precedes it, a statement whose brackets do not close on it.

**What is still not carried across.** Lunar core draws its laser as three clips and its class
picks between them by name; the three came out as one entity with two animations, so there is no
`attack2` to name. Race to the finish names a damage-zone class this package has no content for.
Both are content that did not come across rather than code that does not run: the stages run.

### the ten-stage sample, live

All ten convert (`just sweep-stages`) and RUN with no script errors: `just sweep-stages-live`
runs the verify gates on each one, a stock fighter walking, jumping and attacking over it, and
reads the error channel. Stock, because a converted fighter's own
errors look exactly like a stage regression. Converting clean says nothing on its own -- three of
the ten converted clean and then asked the engine for an animation the object has not got, once
per entry, as soon as their scripts started ticking.

Each was also compared against SSF2 running the same stage at the same framing. One difference is
left, and it is the engine's rather than the conversion's:

| stage | state |
|---|---|
| battlefield | matches |
| battlefield2 | matches (its backdrop is stored as JPEG with shared tables, and read as standalone JPEGs it came out a black void) |
| bombfactory | matches: blocks, bombs and the cross blast all run, and block bodies are recycled rather than piling up |
| butterbuilding | its layout table drives its platforms (each is its own structure, filed under the name the stage reaches it by). Its tower does not rise, and cannot yet: the stage walks its camera box and blast zone upward every frame, and Fraymakers exposes a collision area's SIZE but not its position (asked to move one it answers "Invalid collision area stat") |
| bowserscastle | matches: thwomp and lava run, its deck opens on the way down and closes on landing, it lands with a sound, and its two bridges come from its own ledge beacons rather than a table of coordinates |
| crateria | matches: it falls rain and green acid at once, on the cycle its own code runs, its warning sign is dark until the acid comes, and the acid BURNS -- 27% per 40 seconds in SSF2, 32% here. All eight of its declared camera backgrounds ship, each at its own rate |
| crystalsmash | matches: its deck, its wooden platforms and its backdrop; its crystals port from the shared enemies package and, like SSF2, only stand when the stage's own code asks for them. crystal smash 2 passes S7: its platform holders ride the platforms and show the frame each platform picks by its height, on both the stage's and the platforms' own code |
| flatzoneplus | matches; its turtle platform runs its own class |
| fourside | matches: its spaceship and platform run their own classes, its three drawn camera backgrounds ship at their declared rates and in their declared order (two more are declared with clips that draw nothing), and its moon is the size SSF2 draws it and carries the same halo |
| huecomundo | matches: its art composites exactly as SSF2 draws it, and its platform runs its own class and carries its front piece |

**how a camera background is measured.** Drive both engines the same way and compare what the
backdrop moved against what the world moved: that ratio is what Fraymakers' `xPanMultiplier` means,
and SSF2's declarations are in the same terms, so a declared rate goes straight across. Battlefield
declares no rate at all, and moves its jungle 32px against the world's 406 -- 0.079 -- which is what
`(size - view) / (2 * (size + view))` gives for an 882x496 backdrop; the port measures 0.08 there.
The same two shots settle SIZE as well: normalise both windows to one view and the stage geometry
lands on itself, so whatever is left over is the backdrop's own error. A camera background is
tethered to the camera and sized against the SCREEN, so the stage's world scale is no part of it.
`/tmp` is not the place for the harness, so it is written down here: two shots per engine with the
camera translated and no zoom change, then patch-match a grid and read the two clusters (world art
lands on one offset, the backdrop on a smaller one).

One thing that is NOT a content difference and looks like one in every side-by-side: the two
engines frame the same match differently. SSF2 fits its whole camera box on a 640x360 screen; a
converted stage carries a fixed camera height and Fraymakers zooms to the fighters, so the same
moment shows more or less of the stage. Compare art by compositing the emitted layers at their
emitted offsets, not by eye against a screenshot.

**objects.** every class a stage declares ships running its own code (see the converter
notes above); the ones that still misbehave are in the known-issues list. a class whose body or
hit the port cannot read yet is a converter bug to fix in the class path, never a per-stage
number to write down.

**weather / ambient effects** (visual only; no hitbox output needed, but missing from
converted stages makes them look flat compared to SSF2):

| class | stages | what it adds |
|---|---|---|
| EmberWeather | bowserscastle | rising fire embers (camera-space, 50 particles, 6x6px, 2px/frame rise at 30fps) -- measured but unported |
| RainWeather | clocktown, crateria, finalvalley, smashville | rain particle field |
| SnowWeather | snowpointtemple, waitingroom | snow particle field |
| FairyGladeWeather | fairyglade | firefly sparkle field |
| WaterSplash | fairyglade, gangplankgalleon, lakeofrage, planetnamek | splash vfx on landing near water |

**stage-level special behaviors** (not hazards; structural/gameplay gaps):

- `pacmaze` EdgeWrap: FM has no native edge-wrap API; requires engine-side custom behavior or
  scripted position teleport in the stage Script. gap until FM exposes that primitive.
- moving platforms: every one in the shipped stages declares its class through the tree and
  ports as a placed body running that class (crateria, tower of salvation, crystal smash 2, the
  target tests). a platform whose motion lived in the stage class instead would come out static.

---

## mario, audited call by call

The deepest-verified character, and the shape of what "1:1" means for any of them.

| dimension | state |
| --- | --- |
| animation lengths | every one of 164 exactly 2.00x the source range it was sliced from |
| hitbox windows | every source animation's active box-frames exactly doubled |
| character stats | physics matches the simulated SSF2 ground truth; the DEFAULT collision body, roll and air-dodge speeds come from what mario declares |
| hitbox stats | every field this engine accepts is emitted; `priority`, `camShake` and a refresh rate are refused BY NAME, so they have no counterpart |
| frame scripts | 931 live lines, 0 decompiler gaps, 0 script errors driving 20 moves live |
| sounds | 16 of the 21 he names but does not carry come from shared packages; 5 are in no package at all |
| effects | the one he names but does not carry is borrowed from the package that defines it |
| projectile | the fireball flies at the 5.2 its own package declares, and bounces |

What is left is 150 calls across 12 names with nothing to translate them TO: the item handling a
character script cannot reach (`tossItem`, `activateItem`, `pickupItem`), an input buffer
(`forceAttack`), a metal state this engine does not have, an effects-clear it owns itself, and the
source's own bookkeeping (`setLastUsed`, `getCPULevel`). Each was checked against the engine
rather than assumed.

**two collision bodies, and which one stands.** The entity carries a body PER FRAME, fitted to
that frame's hurtbox union, and that is what the engine stands on -- it changes as the animation
does. `CharacterStats`' `floorHeadPosition`/`floorHipWidth` are the character's DEFAULT, for
frames that state none. Both are in the ENTITY's units, which already carry the root clip's
placement scale: the engine scales a character's whole body by its `baseScale` (the size
multiplier), so the declared body goes in unscaled, or it would be applied twice.

## what playing a port turned up that the gates did not

the gates play each attack alone into empty air and read names, lengths and positions. looking at
frames and playing everything else (tools listed in TESTING.md) found, all roster-wide and now
fixed:

| what a player saw | cause |
|---|---|
| every port's default costume wore the art's base colours (mario in a green cap) | the installed game keeps its costume table in a numbered DAT archive, never read |
| attack swings, grabs, landings silent or a stand-in whoosh | the shared sound bank was only found when an environment variable named the game |
| air dodge flew the fighter off the stage, and dodged nothing | this engine's airdash is a one-frame blink; the source's air dodge stays put |
| four fighters' throws stopped mid-move (kirby's up throw) | a camera target handed over as a view container, which the engine refuses |
| ten fighters' taunt button did nothing | the emote slot borrowed an empty or one-frame up taunt |
| hits stuck to the attacker a frame or more too long | half of all moves state no attacker freeze, and the engine's default is not the source's |
| a class's `== null` checks never passed (isaac's vine caught nobody, then threw every frame) | a field declared `= null`, `= true` or `= false` was read as 0 or 1 |
| bomberman's neutral special lost every bomb he had out | a field a move's clip declares shared the fighter's field of the same name |
| landing, timers and hit handlers threw or took the engine down (falcon, link, pac-man) | a handler named after one of the source engine's methods read back nothing |
| megaman's wave, bowser's fire breath and lucario's aura trail did nothing | a fire by a worked-out name, a projectile firing its own kind, a flag a projectile's frame writes on its summoner |
| a spent projectile sat in its dying animation for the rest of the match | a dying animation guessed from the last label can repeat; dying is now bounded |
| a projectile that stopped (pichu's thunder on the ground) stayed out for the match | a projectile that states no lifetime has the source's default one, and a lifetime set while it runs is kept |
| a projectile's code asked for fighter-only things (samus's bomb went dead after its first hit) | the call table reads a fighter's meaning; projectile code now translates in a projectile scope |
| calls on another object the port has no name for stopped their handler (yoshi's egg, naruto's voice clips, pichu's projectiles) | such a call goes through what the object publishes, and does nothing when it publishes nothing |
| megaman's landing handler became `true` | a field a move's clip writes shared the name of one of the fighter's functions |
| hitbox stats built in a variable were refused whole (lucario's aura sphere, wario's fart) | stats handed over as a value are converted when applied, from the same tables |
| marth's, peach's, dedede's and chibirobo's counters got them hit; ness's and game & watch's absorbs were counters | a box the parser did not know by name was read as a hurtbox (the source's counter box among them, and its camera and range boxes); the absorb box was emitted as a counter box |
| a projectile's "when I am destroyed" code never ran (megaman could throw one crash bomb), and listeners on the other fighter vanished when it was hit | the asks ran from an update that stops at death; this engine drops a listener that does not ask to persist on the object's next state change |
| code that asked about the stage (its bounds) never ran, and a projectile leaving the stage never ran its own end (megaman threw one crash bomb, ever) | the stage was asked of the match, which has no such method (every script has it as the global `stage`); the engine took a projectile off at the death bounds with nothing of its own run first |
| black mage's air down special took the engine down with no error | a fighter read its frame within the piece of the clip it was cut into, not over the whole clip as the source does; a jump "a few frames on" landed on the move's own start and recursed |
| fox's reflector let every shot through, and shots it should turn back flew on | this engine knows a box by its layer's name as well as its type, and the reflect box kept the source's name; a projectile that states nothing about being reversible is reversible in the source and was not here |
| ness's magnet ended at its first check however long special was held | the source's engine answers a held or pressed button on an attack that declares `combo_max` by telling the clip to go on; nothing here did (24 fighters declare it) |
| ness's and game & watch's absorb handlers threw ending the absorbed shot | the projectile an absorb takes arrives as the event's foe |
| a projectile another script ended (an absorb, black mage putting his away) skipped its own end: what it asked to be told of its end never heard | ending another object killed it outright; a converted projectile is now asked, and ends itself the way it does at the death bounds |
| falco's side special took the engine down (a crash with no error) | the helper that keeps a projectile's listeners on other objects was itself rewritten into one of those listeners, and called itself forever; every projectile that listens on its fighter hit it |
| a projectile jumping to one of its own labels asked the engine for a frame label its entity does not carry | a projectile's labels are its animations, so the jump plays the animation (from its own first frame it is already there) |
| moves the source makes invincible or armored (ryu's focus attack, donkey kong's spin, every final smash) could be hit and made to flinch | an attack's declared `invincible` and `superArmor`, and its own writes of them, were never applied; they are now held for the whole attack as the source does (`heavyArmor`, a threshold under which damage is still taken, has no counterpart here) |
| mario threw one fireball standing and never another until he jumped (29 fighters declare the same kind of cooldown) | an attack turned off with no count of frames comes back after the `reenableTimer` it declares; here it stayed off until the fighter next landed |
| specials the source lets the fighter steer (mario's tornado, 84 specials in all) could not be steered | an attack's declared `allowControl` (in the air, and on the ground unless `allowControlGround` is off) was never applied, and this engine gives a special no control of its own; it is now, and the attack's writes switch it |
| an aerial that stalls or lifts its fighter when it connects (pikachu's forward air, 11 in all) fell on as if it missed | an attack's `hit_ease`, the cap on vertical speed for five source frames after it connects, was never applied |
| mario's fireball fired in the air landed into its landing lag instead of carrying on | an attack that declares `linkFrames` switches between its ground and air versions at the same frame; the pair now link through the engine's own leave and land types |
| an attack's sound played on after the attack was cut short | an attack that declares `cancelSoundOnEnd` / `cancelVoiceOnEnd` stops its last sound / voice when it ends |
| sonic's running feet never blurred (85 frame lines struck) | a frame line on one of the fighter's parts was restored only when it asked nothing else, and these asked one of sonic's own functions |
| a projectile ended by its code played one of its own labels as it died, running that label's code (lucario's aura sphere "died" in the label that fires it and flew on; 84 projectiles died in `chibi`) | a clip that never says it dies in a label now dies in a still frame with nothing to run, as the source's end takes it off at once |
| pac-man's trampoline and dedede's inhale threw on an event's sender and on steering set through a helper | the event's sender is its `self` here; a steering write through another receiver goes to the engine's own field |
| bomb factory made no blocks and bowser's castle lost half its hazards (the engine refused them) | a stage's spawn by class was read as a fighter's projectile fire, owned by the stage; a stage's and its objects' spawns are the stage emitter's again, made as its own objects |
| once spawned, bomb factory's blocks and bowser's castle's thwomp threw on their clip length, a stopped sound and every call to their stage | an object's script reads its clip length on the clock it reads its frame on, carries the sound helpers it uses, and reaches its stage through the handle the stage gives it rather than the engine's stage |
| multi-hit moves that pull their victim along (sonic's up tilt, game & watch, ganondorf, ryu, tails) let it drop out | the pull is a helper the fighter inherits from a base class its own file ships; the base methods the fighter's code reaches are translated with it (never the engine's surface the package declares, whose methods only hand calls on), and the handler's string slice, named states and stat names cross over |
| a held grab, a fighter lying down, a held shield replayed their poses from the start (101 held frames across the roster) | the source's own `stop()` on a fighter's clip holds the animation where it is through the animation's pause, which lets go when the next one starts |
| peach's float never sparkled; pac-man's point and trail effects never showed | an effect named by something put together as it runs (`"peach_float_sparkles" + random`) was struck; it is looked up among the fighter's own effects when it runs, placed and scaled as a named one is, from the fighter's frames and its projectiles (pit's arrows, the dice) as much as its script |
| link's shield would have turned shots back | the source's shield boxes block (a projectile's shield box ends it; a fighter's shield box is shield damage), and were typed as reflect boxes |
| link's forward smash never swung its second slash | an attack that charges and also declares `combo_max` answers its button once it is attacking (from its `attack` label, which the fighter's own `xframe` names); it was left out of the combo protocol altogether |
| a move's cleanup never ran when it was interrupted or ended (effects lingered, listeners and timers the move set stayed) | the source's state change was read as this engine's ground/air link frames, which only fire at a landing or a jump; it is this engine's state change now, and the listener persists until the code takes it off (a listener that does not persist is dropped AT the state change, before it can hear it) |
| effects a move put on its fighter's list played on after the move was cut short | the list was dropped (`addEffectToList` unwrapped, `removeAllEffects` and `clearEffectsOnStateChange` struck); the fighter's own methods run now, from its class or the base its package ships, and an effect taken off the display list is ended |
| a grabbing fighter drew on the wrong side of the one it held; the held one never took the pose a throw asks for | the draw-order call is the fighter in front of (true) or behind (false) the one it holds; a pose asked by label is asked of the held fighter's script, and does nothing when its clip has no such label, as in the source |
| an effect pushed behind its fighter stayed in front | trading places with the fighter in the draw order is the engine's own swap |
| link's boomerang flew off forever and was never caught | four things, each general: a projectile starts in the label at its first frame (a label replaying itself further in is no start); a timer the source starts lasts until its object's state changes, not until its clip moves to the next label (this engine drops one at every animation change, and a clip's labels are separate animations here); a fighter's own id (`statsName`) is published and read, where it read as nothing; and a projectile asking the one it belongs to for one of the fighter's moves plays that move there, from the label or frame it names |
| the watch for a projectile touching something (the source's collide event) stopped at its first label change, and could not be taken off | it lasts until the code takes it off, and its handler sees the object it belongs to as `self` |
| meta knight's orange trail and dedede's second hammer never showed or hid (40 lines struck) | a clip's named children were read one per depth, and a later frame that puts another child at a depth an earlier one held was missed; every placement is read now (meta knight gained twelve trail parts), and a slot only ever set back to nothing is not one the code fills |
| a colour adjustment or a fade the source sets on a fighter or its projectile (evil ryu's fireball, a timed-out fade) was struck | hue, saturation, brightness and contrast are the engine's colour filter (its values are fractions of the source's -100..100), a fade is the alpha, and clearing takes both off; a setting that also multiplies or offsets the channels stays struck, since half of it would draw another colour |
| samus's and dedede's missiles never went on to their `continue` when they slowed | a call a table marks as having no counterpart, to a function the projectile (or the fighter, from a frame) defines itself, is that function; projectile scripts and their frames are restored the way the fighter's script is |
| pikachu's thunder jolt rose in a straight line and never bounced; luigi's fireball ran every frame script twice; kirby's afterimage and peach's peaches ran a frame of a clip the source never places in them; rayman's vortex and sora's strike raid lacked their stance's own frames | a projectile's code is its own clip's class and its stance's, which the timeline binds; it was also read from any class sharing the name the fighter's table fires it by (pikachu's `thunderJolt` is a ten-frame clip of another jolt's steering, and the jolt fired under that name is `thunderJolt2`), and the two clips' frames ran together |
| a projectile spawned by a fighter's own code started after its launch was set, so its own first-frame code could not change it (link's arrow flew at its uncharged speed however long the bow was drawn); a spawn hidden behind the launch's lines was not read as a projectile holder (pac-man's hydrant threw on every setting) | the launch is worked out first and left on the owner, the projectile takes it up before its own code runs, and what is left is given after; the holder read reaches past the launch's lines |
| a projectile with no death of its own sat a second in its still frame after its end (mario's fireball 72 frames against the source's 39) | a death is over when its last frame has played or as many frames have passed as it has |
| every shot was spawned from a height the fighter's art lifts to in its pose (link's arrow 100 px up where the source fires it 31) | a shot leaves from the fighter's own position plus its stated offset, as measured on the source's per-frame record |
| falco's laser and megaman's buster and crash bomb left 14, 7 and 8 px too low | a shot fired with an offset of its own leaves at that offset on top of the one its projectile declares, as the source's record measures it (falco's laser fired at (0, -10) with -14 declared leaves at -24) |
| ness's pk flash charged, was let go, and never fired (the fighter went to his stand; the flash lived 75 frames against the source's 35) | a charge special's base piece names the piece its clip plays on into (`on`), and a fighter whose clip is still playing at that piece's last frame goes there; held, the move's own loop-back keeps it in the charge. a projectile's animation clip plays on whatever a stop on it says, as a fighter's does, so the flash sent to `boom` with a gotoAndStop plays its blast out to the frame that ends it |
| pit's back orbitar never ended and stayed out until the move did (19 frames against the source's 10) | a jump another object's code makes on a clip (pit sending his orbitars to `end`) is taken up before that clip's next keyframe runs, and the keyframe it was about to enter is skipped; it used to drag the clip back to that frame and replay its script |
| a jump to a label on another object's clip from a projectile's or a stage object's script was left unanswered (pac-man's dots, ness's flash sending ness on); one from a fighter to one of its own parts was struck as a call on nothing | only the fighter's script resolves labels against its own table; a projectile or stage object asks the entity through its exports, and a part is a name the fighter's own helpers may be handed |
| samus's missile burst 26 source frames early (33 against the source's 59): its class hands its measured, decaying speed back as its homing speed every other frame, and the write was dropped as a stat this engine has no field for | a stat the projectile's script keeps for the engine (its homing speed and ease, its x decay, its lifetime) takes a runtime write through `__setStat`, from its own code or through its exports from whoever holds it; the keys are the projectile table's `handled_in_script` |
| a speed read back, kept in a local and written again shrank by the velocity scale each time (ness's rocket bounced at two thirds of its speed; simon's cross was patched for one shape of it) | scripts speak the source's units end to end: the decompiler leaves every speed as written and the boundary helpers convert both ways (`__sv` on a write, `__isv` on a read), so a comparison against a source number holds too; the four fighters the old measurement was taken on (ganondorf, simon, game and watch, mario) pass their gates unchanged |
| a handler a projectile put on its owner ran on after the engine took the projectile off (bomberman's held bomb, once a knockout removed it: an unknown `self` on the owner's every state change for the rest of the match) | a listener put on another object goes on wrapped, and the wrapper takes itself off the first time it finds its own script gone; a removal by the source's own call goes through the same record |
| every timer the source starts fired a source frame late here (samus's missile took its launch speed as its homing speed a frame after the drive had imposed the declared one, and flew two units slow for its whole life) | the source's timer of N frames runs before the update of its Nth frame; this engine's runs after an update, one frame sooner when booked during a frame's update or scripts than from a callback (measured). so a source timer is a record on a due list each script's update drains first: booked from a script its first hop is one engine frame short of the doubled delay, at spawn two (the object's first update is still to come that frame), and each fire books the next a whole period on from inside the drain; removals go through the record, whichever hop is booked. one timer per callback, as the source keeps them: a frame that starts its timer again each time it plays replaces the running one (link's boomerang slowed twice as fast with two) |
| a homing shot dived under the engine's gravity, turned twice as fast as the source's, chased a fighter the source passes over, and slowed at half the source's rate (samus's missile) | the drive owns the velocity (gravity off under it), keeps its heading as its own state, halves the ease's share per engine frame, passes over a fighter that is intangible or invincible, and takes the decay on its driven magnitude |
| the source's frames retune a trail as the shot flies (samus's missile drops one every 5, then 8, then 16 frames, and none once spent); the writes were dropped | the trail's interval and its effect (null ends it) are runtime setters like the other script-kept stats |
| megaman's buster flew through a fighter dealing nothing, then hit four times and flew on | a projectile's hitbox stats go on every animation it has (the source's attack belongs to the projectile, and its timeline places the boxes: the buster's box sits in its `loop`, which had an empty entry); an object's own listener lasts until its code takes it off, as the source's does (the buster listened for its hit in its first animation and this engine dropped the listener at its second) |
| megaman's buster dealt 3 a hit where the source's uncharged shot deals 1: its class lowers the damage as the shot flies, and the writes were made in its first animation and gone in its second (and at every replay of a looping one, which puts the declared stats back) | a projectile's hitbox writes are kept and put back every update, so its box is one box across its animations as the source's is (`__hbCarry`); `PEPTIDE_DUMP_PROJ_STATS=1` prints the boxes the parser read for each projectile |
| a clip that ends its move on its last frame showed that frame here (simon's jab ran 31 engine frames against the source's 28) | the frame whose script ends the move is never drawn on the source, its clip's last frame included: measured on the source's own per-frame record (pichu's down special shows its 59th of 60, simon's jab its 24th of 25; an earlier reading had the last frame shown, off the wrong column of the record), so the trim takes the last frame too. the end call itself still runs, joined to the last shown frame's script (the source runs it before drawing the frame it sits on, right after the previous frame has shown; here a frame's script runs after the frame has shown), since a move whose animation merely runs out is ended by this engine its own way. a clip of ONE frame that ends the move on it (an air special the source refuses) keeps its end one frame past: ending a state on the first frame this engine has entered it took the engine down natively (donkey kong's aerial down special, a signal 11 with no script error) |
| donkey kong's forward throw read its state table by a variable and the engine refused the string as an index | a collection the fighter's frames index by name marks the receiver for the script too (the frames were the only place the name was written down) |
| a label asked of another object was taken up on this engine's next frame, half the source's (megaman's water wave spawned a segment every five engine frames against the source's six) | the ask is taken on the receiver's next source frame: on the second engine frame of its current pair, the next frame drawn begins one |
| ness's thunder went right, struck the other fighter and died at 53 frames against the source's 69 (it had passed through harmlessly while its flight carried no box) | a shot the owner aims with the stick starts on the direction its declaration states (90, up), the launch being the magnitude along it |
| ness's fire sank at a constant rate and died on the floor at 8 frames against the source's 12 | a speed written into ANOTHER object crosses plainly: the step of gravity the fighter's own y writes take (the source's engine adds one before it moves) and the floor hold are the fighter's body's; and a projectile the source gives no gravity, or 0, has it written at start, since a 0 in the stats file reads as unset here |
| a carried hitbox write put back every update cleared the box's record of what it had hit (ness's thunder and fire struck again and died) | it goes back at a change of animation or a restart only |
| megaman's water wave never stopped (its segments lived 88 frames against the source's 40): every segment reset the wave's count | a clip's first frame's script runs at the projectile's first update unless a label was asked at the spawn, as the source's playhead reaches such a label before frame 1 (this engine ran it the moment the object was made, and folded into `initialize` it ran for every segment); and a jump to the label the first animation is named for lands on that label's frame inside it, since the first animation carries the clip's lead-in frames (replayed from frame 1, the wave ran frame 1's script every cycle) |
| pac-man's hydrant threw on turning its second water shot round, and the shot itself did not exist (a name a spawned class fires from its own table was never resolved to a clip) | a thing turned to face the other way is turned by the way it faces (a projectile here has no `flip` of its own); what a spawned class's own code fires is resolved to its clip the way the fighter's fires are |
| game & watch's bacon threw every time it landed | a timer taken off by a function whose name says `timer` (`timerDone`) was read as taken off by its number; a function the script defines is taken off as one |
| an engine method handed to a timer (`refreshAttackID`) was nothing to call, and took the engine down when the timer fired | it is wrapped in the call it translates to, the same as one handed to a listener |
| goku's kaioken meter threw on every hit from a move its table does not list | a field read off an entry a table does not have throws in the source too, ending the function; the port ends it there quietly |
| a projectile asked for "the character" behind it got nothing | the fighter behind `self` is the fighter in its own code and the owner in a projectile's |
| pac-man's trampoline threw from a knockout handler after a newer trampoline replaced it | a projectile taken off because its fighter is at its limit lets go of what it listened to on others first |
| marth's counter hit back for 10.8 off a 9 where the source hits for 10 | a field declared `int` keeps only the whole part of what is written to it (cut toward zero, not-a-number is 0); the declared type was never read, and a `Number` field starting at 0 was kept as a whole number too. a hit's stats read back in the source's units come back as the source's own numbers, not 29.99999 for 30 |

open: marth's counter returns 10 off a 9 on both, launching at 130 degrees with the same speed (the source's 10.5 is its fresh bonus); a `seq` with long pauses is taken by the engine several frames a line at speed 1 (measured on its own record), so its presses land late; press with `hold`/`release` paced by `tick` when timing matters; a projectile made on the floor is told of that floor its first frame, as the source tells it (pikachu's jolt starts its hops from that touch); black mage's warp and meteor and megaman's beat differ in life or spawn on `proj_parity.py` and are next; bowser's castle's thwomp strikes a fighter standing at the second spawn on both engines, but the source launches him to a star KO where the port juggles him in place (hit up to the ceiling, dropped back onto the same spot, hit again) and its S6 rest sample lands at a different moment each run; the same on the build before this batch; its background podoboos and bubbles step 32 frames where the source steps 56 (S7); the SSF2 comparison gate cannot start giga bowser or wario-man: the game runs no match with a transformed form as its fighter, and asking bowser for his final form outside the final smash does nothing; a runtime tint that multiplies or offsets the colour channels (a colour transform, most `setColorFilters`) is struck: the engine's colour shader has the same fields but shows nothing on a fighter, and its filters are hue/saturation/brightness/contrast and glow only; walking off a ledge facing away does not catch it on either engine (the stock fighter here does, with a larger box); the engine's warning that a projectile "sat in the DESTROYING state" for twenty seconds fires for any projectile twenty seconds without a change of state, dying or not: the ones soak play trips it with are long-lived active ones (lucario's sphere held while charging, sandbag's held items, pac-man's trampoline), and the soak now names them as long-lived rather than as errors; wario's fart no longer carries the dizzy it states (no hitbox stat for it here); a
move that ends itself on its clip's last frame reads one source frame longer here on the
recorder, against an earlier measurement that the source shows that frame; and the VICTIM freeze
for a move that states none is measured on the source (the attacker's rule plus one) but not yet
on this engine, whose frozen victim shakes in place.

## character system gaps

calls in the `conversion_log.json :: unknown` stream: SSF2 API the converter doesn't classify at
all, as opposed to the `ssf2_only` universal gaps (item system, getCharacter, getMetalStatus)
covered above in "SSF2-only / no-FM-equivalent inventory".

a character's OWN helper functions don't appear here. `updateAuraPaws`, `isDarkPit`, `hurtSelf`
and friends are emitted and translated into that character's `Script.hx`, so their calls resolve;
the tally discounts every name the assembled script defines. the work they represent is decompiler
BODY quality, tracked by the `/* ? */` counts in "measuring progress", not call mapping.

all 49 characters (47 SSF files, incl. multi-char bowser/wario/zelda) convert exit 0.
corpus sweep: PASS=49 FAIL=0. the only empty function bodies left in the corpus are the
intentional template stubs (`inputUpdateHook` on all 49, `onTeardown` on 2).

**what a struck line is now**, by the strike tally's own baseline (5447 lines, 126 kinds, one
scratch conversion of the whole roster):

| kind | count | what it is |
|---|---|---|
| always false here / always true here | 1825 / 460 | a test of something this engine does not have (the metal box, a cpu player, an item state, a costume named by a swapped colour); the branch that runs here is kept, the test is struck |
| SSF2-dead | 1878 | the runs behind those conditions |
| SSF2-only: attachEffect | 4 | an effect whose art is in a package the manifest does not carry (a shared effect another character's file declares) |
| it has no animation of this name | 191 | a label on an item's clip, which lives in a package of its own |
| the rest | 1-100 each | the item system, cpu control, display-list ordering, the grabbed-foe pose, a colour transform while the match runs |

ported this round, and no longer struck: the end of an attack (kept as an ask, answered when the
fighter leaves it), being destroyed (the same, for an object, and watched from whoever made it
when it is something else's), a sound stopped by name (the handle the engine hands back is kept
under it), an effect attached to something other than the fighter, the id an object was declared
under, and an object's held timeline.

**the older call stream**, 1619 calls across 55 names. the item system alone is 73% of it:

| call | count | chars | what it is | port path |
|---|---|---|---|---|
| activateItem / deactivateItem | 678 / 506 | 42 | item grab/drop | part of the item-system gap; no FM item API |
| forceGrabbedHurtFrame | 130 | 32 | holds the grabbed character on a specific hurt frame during a throw | check whether FM's `GRABBED` state already drives the right frame; if so these are no-ops to comment out |
| getScale | 62 | 9 | reads the character's uniform scale | FM splits the axes: `getScaleX()` / `getScaleY()`, no uniform accessor |
| applyPalette | 22 | 4 | runtime palette swap | FM: `setCostumeShader()` |
| getOwnStats | 15 | 2 | the actor's own stat block | no FM equivalent; SSF2 hazard/actor idiom |
| setupAutolinkAngle / stopAutolinkAngle / updateAutolinkTarget | 10 / 12 / 4 | 4 | SSF2 autolink (an attack steering itself toward the target it hit) | no FM equivalent; needs a Script.hx hook on HIT_DEALT |
| addToDamageMeter / removeFromDamageMeter / updateBar / throbDamageCounter | 9 / 9 / 9 / 8 | 9 | SSF2 HUD damage-meter drawing | no FM equivalent; FM owns its own HUD |
| takeDamage | 8 | 7 | deal damage to another object | no FM equivalent by that name; `updateHitboxStats` / a real hitbox is the FM route |

the remaining ~40 names are 1-6 calls each on 1-4 characters (`toDodgeRoll`, `grab`,
`generateCharacterItem`, `getHomingTarget`, `spitFoe`, `setInvincibility`, …). they're
long-tail per-move SSF2 API, worth mapping only when a specific character is being audited.
`reset`, `tick`, `dispose`, `start` and `distance` all exist in the Fraymakers API under the
same names but on types the converter can't confirm from the call site, so confirm the receiver
before treating one as a gap.

---

## known issues & gaps

**the release sample's two failures.** `verify.py sample --ssf2` passes 18 of 20. fox's side
special runs 47 engine frames where the source's 32 frames want 64 (C6), and galaxy tours'
background has a 45-frame child that loops in 59 engine frames against the source's 90 (S7, the
nested-loop backdrop family below). both fail the same way with fighters off the native playhead.

**the SSF2 test fixture's platforms draw with the floor's art.** a platform draws with the clip
named by its `linkage_id`, and the fixture answers with its terrain clip for everything
([`SSF2_PACKAGE_FORMAT.md`](SSF2_PACKAGE_FORMAT.md)). collision is unaffected.

**accurate stage-failure accounting (2026-09-10).** several saved gate reports predate this
session's fixes and read as failures a re-gate clears: yoshisisland64 now passes 7/7 (its S4
"FlyGuy gone" was stale -- the FlyGuy is live and flying, measured). DAT61 and rest area pass
S3-S5 and fail only S6, and only on the SSF2 SIDE: their source home-run/all-star classes throw
under a versus boot (they read an unset mode global), so the harness cannot read the source's
fighters to compare against -- the PORTS run correctly. those are a comparison limitation on a
mode stage, not a port defect. the genuine port-side remainder is the nested-loop backdrop
family (a sub-clip's loop flattened in the stage-art walk; see the depth_animates note below).


the live list of open converter issues. strike an entry when you fix it.

- **jungle island's moving platforms lift a standing fighter in the source, not in the port.**
  live A/B ground truth (drop a fighter at an entrance, sample port vs source rest over ~200
  source frames): both engines agree on the static floor under the entrance (~131 in both), then
  the SOURCE fighter is carried up 131 -> -11 as its MonkeyBall platform rises into it, while the
  PORT fighter stays on the floor. the platforms exist and cycle in step (S4/S5 pass) and the
  stage script wires `__platFollow`, but `__platFollow` only carries a fighter already registered
  on the platform; the source's rising SOLID platform collects a fighter standing above it by
  collision (pushes it up), which the port does not reproduce. this is what makes S6 flaky (the
  two engines catch the fighter on different surfaces, floor vs platform, at different phases),
  and it is a real port defect, not a gate artifact. an S6 harness change comparing rest TRAVEL
  BANDS instead of one shared frame was tried and reverted: it cannot fix a floor-vs-platform
  surface mismatch and would mask the carry defect. saffron city's platforms carry correctly and
  pass S6, so the mechanism exists; next is why the MonkeyBall platform's collision does not
  pick up a fighter it rises into.
- **pokemon colosseum's S6 lands on whichever stadium each engine rolled.** S7 pairs every element now (its nested clips were filed under a named sibling by the comparer); the stadium is picked at random on both engines, so S6 compares two different floors on a run where the rolls differ.
- **race to the finish's ice and fourside's spaceship deck carry their declared friction.** a
  fighter standing in such a piece slows by the piece's own numbers (the port sets its friction
  and walk acceleration while its feet are in the box, restoring its own after); on the ice the
  port loses 0.00325 a frame against 0.65 on the floor. the source's ice was not measured live:
  its ice sits where a placed fighter revives, and the race floor itself carries a fighter, so
  the check there is the stop on a plain floor (1.04 a source frame, the declared decel).
- **eleven stages declare the game's dynamic platform class and its legs; the game leaves flat
  zone plus's still.** a fighter placed on the plate stays put for 600 source frames live, so
  the port keeps such plates where the tree put them and the declaration stays a record on the
  site. whether the game moves them on another stage or mode (the target tests declare the same
  class) is still to be measured stage by stage.
- **a nested clip inside a label-segmented section restarts its clock at every segment.** the
  port cuts a plane's pieces by the plane's labels, and a clip nested in a section (pokemon
  colosseum's 23-frame steel terrain child, jungle island's 5-frame bananas child) is drawn
  frame by frame inside those segments, so its loop restarts wherever a segment does, where the
  source's clip runs on its own clock. the fix is a cut of its own for such a clip (one vfx per
  movieclip, free-running) with the section's segments drawn without it.
  scoped by eight measurement experiments (2026-09-10): the sub-loop is a GRANDCHILD baked into
  one element's composite, not a distinct piece any of the element-model's grouping keys reach.
  measured: jungle's banana element carries ONE `inst_clip` (the outer banana clip); its 16- and
  5-frame loops are deeper, composited into the 9-frame element the `built` loop pads to 16. what
  was tried and proven a no-op: keying the padding by animating depth (loops share a depth);
  adding `inst_clip` to the `built` key and to `timeline_tracks_keyed` (each placement+depth
  carries one clip); an lcm period in the `built` loop (the element's own instances expose one
  clip length). the `timeline_tracks` merge step only combines TIME-DISJOINT tracks, so it would
  not undo simultaneous loops -- it is not the obstacle. what a real fix needs: the walk
  (`walk_frame`) must carry the FULL clip-nesting path per instance (not just the innermost named
  clip), and the element builder must composite each independent nested loop separately at its
  own period, drawing the parent composite without it. that is a change to the instance model and
  every element builder, validated against the whole backdrop corpus.
  measured deeper (tenth experiment): NO bg element in jungle exposes two multi-frame clips at
  the instance level. tracking anonymous animated clips in `inst_clip` was byte-identical because
  the banana leaves are shapes whose immediate clip is the one banana clip, not the sub-loops. the
  walk composites a clip's leaves into one picture per frame, and the sway's and glint's motion is
  already merged in those pixels before any builder sees it. so the redesign is at the WALK: it
  would have to emit a distinct instance stream per nested movieclip and never composite across
  them -- a rewrite of the leaf-gathering in `walk_frame`, the deepest change in the stage-art path.
  STRUCTURE PINNED (eleventh step, `peptide ssf2 tags --sprite 83`): jungle's banana clip MC#83
  (16 frames) is TWO depth layers -- depth 1 a STATIC shape (the body, placed once at frame 0),
  depth 2 an ANIMATING shape sequence (#68 at f0, #70 at f2, #72 at f4, ..., the glint, ~5
  distinct pictures). the comparer reads the depth-2 glint as the "5f child" and MC#83 as the
  "16f" one. the port composites both depths into one 16f element. so the split is BY DEPTH
  within one clip, which `depth_animates`/`leaf_depth` is meant to do (a still part and a moving
  part at one placement become separate layers) but does not fire here -- worth checking why
  `depth_animates` returns false for MC#83's still-body + moving-glint before any walk rewrite.
  this is the concrete next lead, narrower than a full walk redesign.
  RESOLVED, and it redirects everything (final step): `peptide ssf2 tags --sprite 83` shows the
  banana clip is ONLY a static depth-1 shape (#66, 13x2.5) plus a 16-frame depth-2 animation
  (#68 f0, #70 f2, ... #82 f14 -- eight shapes, one every two frames). there is NO 5-frame
  structure anywhere in the SWF, and `bg_bananas` has exactly 12 children, all 16-frame. so the
  depth-split / walk rewrite CANNOT fix the "5f child" -- there is nothing 5-frame to split. the
  comparer's `bg_bananas child 5f` (read `total=5` off the LIVE tree, advancing 1->2 over 30
  ticks) is a RUNTIME phenomenon with no static counterpart the converter reads: Flash's
  `getTotalFrames()` for this instance is 5 where the SWF definition is 16, most likely from the
  clip's `DefineSceneAndFrameLabelData` naming a 5-frame scene the runtime loops. the converter
  reads the SWF, not the runtime, so it emits the 16-frame animation the SWF holds. the real
  options are (a) read `DefineSceneAndFrameLabelData` scenes and truncate a clip to its runtime
  scene length, or (b) accept on the comparer side that a source element whose SWF length is a
  whole multiple of the runtime length is reproduced. NEITHER is the walk/depth change; both are
  new, narrower work. this is the corrected scoping.
  ACTUAL BUG FOUND (live probe of the SSF2 tree): the "5f child" is NOT the banana (the bananas
  are 16f at runtime, `frame=1/16`, matching the SWF). it is a DIFFERENT clip -- sprite#98, a
  real 5-frame clip (41x36, shapes #89..#99), 3 instances near the top of the stage
  (`instance30474/76/78`, `frame=5/5`). the port emits those three as 280-FRAME elements
  (`background_331361_1/331423/331485`, `frames: 280` in the plan), where the comparer wants 10
  (2x5). they are built by the parallax path's `timeline_tracks`, not the `built` loop: its
  `period_of` finds NO clean loop for these tracks (they scroll with the parallax, so the
  absolute-position composite never repeats) and falls back to the full sample window (280). the
  fix is bounded and specific: when `timeline_tracks` finds no period for a track, fall back to
  the track's OWN driving-clip frame length (from `sprite_frames` via the instance's `inst_clip`)
  instead of the whole window -- so sprite#98's track is 5 frames, not 280. that needs
  `sprite_frames` passed into `timeline_tracks` (a signature change) and a whole-backdrop-corpus
  re-gate, since `timeline_tracks` builds every stage's camera/parallax elements. this is the
  real, narrow fix, replacing the walk-rewrite theory above.
  TRUE ROOT (live two-sample probe, settles it): sprite#98's three instances are FLYING CREATURES.
  they BOTH move across the stage AND loop a 5-frame animation -- measured frame 5->3 while their
  positions slide (49,-151)->(-48,-172), (723,-174)->(645,-181), (741,-160)->(820,-195) over 90
  ticks. so the source element is a 5-frame flap loop on a moving object; the comparer reads its
  animation length (5) and wants 10 (2x5). the port's `timeline_tracks` composites ABSOLUTE
  positions, so it bakes the MOTION into 280 unique frames (one per position) instead of emitting
  a 5-frame loop whose POSITION is driven separately. that is why no pixel-period is found. the
  real fix is to separate an element's animation loop from its motion in `timeline_tracks`: detect
  the driving clip's own frame cycle (its frame index repeating, not its pixels), emit that as the
  loop, and carry the per-frame translation as a path the way `__platFollow`/`__follow` already
  move a vfx. this is a genuine feature -- moving animated backdrop objects -- not a period tweak,
  and it is the correct target for the nested-loop family. every prior theory (walk rewrite,
  depth split, scene truncation, period fallback) was ruled out by measurement on the way here.
  FEASIBILITY (final): the emitter side largely EXISTS. `single_shape_frames` already separates a
  moving element into (one raster per distinct shape) + (per-frame transform), so for sprite#98 it
  produces 5 images placed per frame -- confirmed via `PEPTIDE_PLACE_DEBUG` (shapes 89,91,93,95,97,
  no reject). and `__platFollow` (a boot element's `follow`, `plat_follow` in stage_emit.rs) already moves a bg vfx
  per-frame along a position list. what is missing is only the SPLIT: detect the shape-cycle period
  in the single_shape output (the images repeat every 5 while positions advance), emit those 5 as
  the element's loop `frames`, and hand the full position sequence to `__platFollow` as the path.
  BgLayer has 10 literal sites (a `path: Vec<(f64,f64)>` field is a bounded add). the scope is:
  parser split in `timeline_tracks`' single_shape branch + carry the path on BgLayer + route it in
  `plat_follow` (the boot's `follow` rows), then re-gate the backdrop corpus. this is the concrete build for the pass.
  TRIED once (implement->validate->revert): a naive png-only period split in `timeline_tracks`
  before `own_loop` over-triggers -- it caught scrolling backgrounds (jungleIslandBG0, bg0) as
  spurious "2-frame loops over 280/600 positions", and the actual flying-creature element (280f)
  did NOT reach that branch at all (built by a different path). so the real feature needs (a)
  discrimination: only a SMALL element whose short period is its clip's real animation, not a
  coincidental raster repeat on a big scrolling plate; and (b) locating the creature's actual
  builder first. reverted cleanly; no regression shipped. next: find the creature's build path,
  then split there with a size/own-clip-length guard, and add a per-frame vfx position driver
  (the existing __platFollow moves STRUCTURES, not a free vfx, so a new small driver is needed).
  NARROWED further (offline entity map): EVERY jungle backdrop element emits with distinct_x=1 --
  the motion is baked into the IMAGE content (background_331361 = 140 frames, 139 distinct
  full-canvas rasters, x constant), NOT into keyframe x/y. so a split that reads the path from
  frame x/y sees no motion and cannot fire; the position must be captured from the INSTANCE
  transforms during rasterization. and the creature takes the COMPOSITE path (139 baked images),
  not single_shape (which would give ~5 rasters) -- so step one is finding why single_shape is
  not used for it (multi-slot? returns None?), then capture per-frame positions there as the path.
  CORRECTED AGAIN (decisive): the creature IS single_shape (background_331361 = 140 frames drawn
  from 5 images, placed=true), but its 140 keyframes are ALL constant at (0,0) -- it flaps in
  place, it does NOT move in the emitted art. and the source motion measured live (x 49->-48) is
  therefore NOT in the SWF timeline the converter reads: the walk sees a static flap, the flight
  is applied by STAGE CODE at runtime. so the loop+path-of-a-moving-clip theory is WRONG; these
  creatures are code-driven and belong on an OBJECT path (a spawner/updater that moves them),
  not baked backdrop art. that reclassifies the fix entirely. next: find what moves them in the
  source (a class/updater) and whether they should port as hazards/objects rather than bg vfx.
  GROUND TRUTH (live A/B, ends the JSON forensics): booted the source and compared deltas over 60
  ticks -- the big plates are STATIC (delta 0) while the creatures MOVE independently, and in
  DIFFERENT directions (two go left dx=-128, one goes right dx=+128). so it is NOT camera scroll
  (plates static) and NOT a shared baked timeline (that would move every instance identically).
  the stage class does not move them either (handleBGTimer only drives the timer-counter display).
  so the motion is the PARENT background clip's own timeline (jungleIslandBG0) animating each
  nested creature along its own path, and the converter DROPS it -- single_shape places every
  creature frame at its local origin (0,0). the real defect: a nested clip MOVED by its parent's
  timeline loses that motion in the parallax/timeline_tracks path (emitted at local origin). the
  fix is to carry the parent-timeline per-frame placement of a nested moving clip through to the
  element, then reduce its own flap to a loop. confirmed by observation, not entity-JSON reads.
  MAJOR CORRECTION (read the IMAGE SYMBOL, not the keyframe): the port DOES move the creature.
  the entity keyframes carry NO x/y -- the position lives on the image symbol each keyframe
  references, and those x values VARY (0 -> 56.7, 136 distinct across 140 keyframes). so every
  earlier "motion dropped / static / reclassify as object" note above was a MISREAD of the
  keyframe. the port renders background_331361 moving AND flapping. the S7 failure is therefore
  NOT a rendering defect: it is a comparer grouping/length-pairing gap. the source child is
  grouped under base "bg_bananas" while the port's moving element is under base "background", so
  the comparer never offers the 140-frame port element (which same_pictures WOULD accept: 140 is
  a multiple of the wanted 10) as a candidate. next: confirm the source creature's true ancestry
  (is it really under bg_bananas, or a sibling under background that the comparer over-groups?),
  and align the grouping -- a comparer/naming fix, far smaller than the emitter feature I chased.
- **jungle island's bananas hold a 5-frame child the comparer cannot place.** every other
  element pairs (the 600-frame background child loops at its own clip's length now, the
  ladybug's child is drawn inside its parent's piece); the bananas' 16-frame child pairs and
  their 5-frame one finds no port piece of its own, and the source's timing table never saw
  it move, so it is most likely a held clip on both sides that the length contract should
  read as a still. the stage stands, cycles its sections and grabs its platforms' ledges.
- **a frame script on a clip placed without a name stays a comment.** the plane frame-script
  port keys on the instance name the tree gives a clip, so a timeline clip the tree places
  unnamed keeps its scripts as comments: phase 8's waterfall asks the stage to remove it,
  dreamland's backdrop asks whether dedede is in, smashville's and final valley's foregrounds
  reach the stage too. binding such a clip's group by its symbol would let them fire.


- **a self-landing move stops at the ledge.** a grounded move that goes airborne and lands
  by its own code keeps this engine's leave type NONE, which also keeps it from sliding off a
  ledge on the ground; the source's ganondorf down special slides off and grabs the ledge. the
  source states this per attack (`canFallOff`), and the default without it is to fall off;
  the mapping to a leave type is not made yet.
- **a class the package declares but never binds is not shipped, and a call on it goes to
  nobody.** bowser's castle declares `BowsersCastlePlatform` (a floor that sinks when told)
  but binds no symbol to it and spawns none, so the thwomp's `sink` on the plain terrain
  platform it lands on reaches nothing on either engine; the port routes such a call through
  the ask channel to the deck's owner (`__floorOwner`), which answers when a stage does bind
  one.
- **a target test's start platform is the mode's own.** bowser's target test places a 207 px
  wide platform clip whose only shape is one rectangle (`peptide ssf2 tags <file> --sprite
  <id>` lists a clip's children with their edges); dropped across it, the source's fighter
  rests 22 source px ABOVE that rectangle, on three spots (x -100, 0, 100) with gaps at ±50
  where it falls through. nothing in the package declares those spots, so the game's target
  test mode builds them; the port ships the rectangle the clip declares. the other target
  tests, mushroom kingdom 3 and sky sanctuary differ on S6 stand by other amounts. dracula's
  castle stands: its platforms run their class's cycle (a stop table the port had scaled as a
  speed, a rest position it had seeded as an empty list) and carry the fighters, read on both
  engines at one frame of the match. fourside stands the same way (its travelling platform is
  under the entrance on both engines at that frame).
- **a target test is a game mode.** the source holds its targets and counters still until
  the test starts (its clocks read 0 under the harness), the port ships the targets as art and
  plays them; a clip group's `stop`/`play` now pause and resume its parts, but nothing stops
  these, since the mode does.
- **grouped background art plays as one composite.** smashville's sleeping shopkeepers are
  88 and 91 frame clips the source loops on their own; the port folds them into a 2496 frame
  layer over the stage's whole day, which the comparer now reads as a schedule. its named
  platform's 97 frame clip ships one still per label (`dawnWithRain` and its seven siblings,
  two frames each), which is what the source shows: the stage sends the clip to each label
  with `gotoAndStop`, and the clip's own frame scripts stop it a frame later, so nothing
  inside a section ever plays.
- **the engine's structure raycast answers null.** `hitTestStructuresWithLineSegment` answers
  null on the port even from a fighter standing on the floor, so the `hitTestGround` rewrite
  that lowers onto it never finds ground, and the script's own landing does not ask whether a
  floor is there (the engine's own landing is the floor question now, see DEVELOPMENT §6.1b).
- **a jab clip is cut where its move ends.** a label starts a piece when a frame between the
  previous piece's start and the label carries the script's `endAttack` (the same read as the
  head rule); a label the move runs through (kirby's `again`, the rapid jab's loop point, and
  `finish`, its ending) is a mark inside the piece, kept as a frame label the class jumps to.
  a clip whose scripts state no end is cut at every label. no word list is consulted.
- **a script keeps the source's units; the engine boundary converts.** every engine position
  read in a stage's or an object's code comes in through `__isx`/`__isy` and every write goes
  out through `__sx`/`__sy`, groups and regions included, so no value is ever judged for its
  units (central highway stepped an engine read by a source literal; palace of twilight handed
  engine reads to an object that counted them as source numbers). an object's own step runs at
  the source's rate with its position writes held to the end of the step, and a placed object
  ignores position writes from asks made before the match is live, which is what the source's
  tree-placed objects do (jungle island's barrels walk their paths from where the tree put
  them). a placed instance is its own entity: one class placed seven times is seven objects
  with seven bodies (palace of twilight's platforms carried the pillar's box). a stage that
  lays its objects out by plane label moves them with the label. palace of twilight passes all
  seven gates; jungle island stands where the source does, with its banana art lengths the one
  open difference. see PORTING_STAGES phase 4 and the object rules.
- **kingdom1, crateria, planet namek, steel diver, jungle island and pokemon colosseum stand
  where the source does again.** the collision walk reads each clip's display list at the frame
  it holds on, a closed contour is simplified as two halves, and a counter's step is no longer
  redeclared (kingdom1's see-saw loop wedged the engine). lunar core stands too (its pit bridges are
  planks, not a box). fourside stands too, now that the stand gate brings both
  engines to the same moment of the match before it drops (the source engine's frames are
  accrued from wall time, since it has no counter to ask). lunar core stands at both entrances (each is judged from the lift that cleared it). crystal
  smash 2's stand gate skips: the source holds no fighter at its entrances outside its game mode
  (its starter platform is fall-through until the mode starts, and the fighters fall and respawn
  forever), so there is nothing to compare from a versus boot.
- **peach's castle, jungle hijinx and gangplank galleon stand where the source does; nintendo
  3ds boots and stands.** the static copy of a self-bodied object's plate is gone from the
  terrain, a class's forward-jump setup decompiles as the common tail, and a stage that keeps
  its planes as handles (pokemon colosseum) reaches them as objects of their children.
  pokemon colosseum boots, builds and moves (its painted stands are solid in the source, every
  bleacher step of them, so the port keeps them: six hundred structures, and the engine runs hot
  but delivers its frames); lunar core and crystal smash 2 stand on floors the source has
  elsewhere (open).
- **galaxy tours passes every gate and survives its galaxy changes.** the crash was a surface
  handle forwarding a label play to the group lookup, which handed the handle itself back (the
  stage fills the `chompworks_plat2` slot by name), so each galaxy change recursed until the
  stack ran out. the ground switching by plane frame is live; the galaxy changes themselves
  are verified over one full change, not yet against the source's timing.

- **krazoa palace and the world that never was paint a background per pixel.** each stage
  class builds a bitmap (670x540, 986x524), reads a mask picture pixel by pixel (`getPixel`)
  and paints into it every frame; the port has no per-pixel canvas, so those loops are struck
  and that layer is missing. the rest of each stage converts.

- **a midground clip the stats declare and the engine makes is not in the tree.** home-run
  contest's `barrierL`/`barrierR` are named only in the stats' midground list; nothing places
  them, so the class's reads of them (`getMidground().barrierL`) land on nothing. rest area
  plays its camera background by a numeric all-star label read off a mode global that is unset
  in versus.

- **a fighter's cpu asks and warning-bounds tests.** `isCPU()` / `getCPULevel()` have no
  reading yet (the block under them is struck), and the four off-screen warning-bounds tests
  answer false (this engine draws no warning bubble), so the ordinary branch runs.

- **character calls waiting on a live check of an engine name.** a projectile's
  `isReversed()` (a reflected flag; the reflect event's constant is not in the api list),
  `forceAttack("b_forward")` on a character (a special-move state transition), and
  `setAttackEnabled(bool, "b_air")` (a per-attack enable each special would consult) are
  struck until the names are confirmed in the engine. kirby's copy-ability calls
  (`getCurrentKirbyPower`) are a system of their own. a fighter's `statsName` read answers 0.

- **crystal smash 2 is a game mode.** its crystals come from `setupCrystals(level)`, which
  only the mode calls, and its starter platform is fall-through until the mode starts, so from
  a versus boot the source holds no fighter at all (they fall and revive forever) while the
  port keeps the platform solid from the first frame (its readiness read stands in for the
  mode's start). the stand gate skips it; the platforms, their holders and the backdrop match.

- **warioware's landing dust is the source's, not the fighter's.** a terrain-plane clip
  (`effect_land`) plays in the source whenever a fighter lands, and the cross-engine gate counts
  it as an element the port lacks (consistently). the port leaves landing effects to the
  engine's own fighters.
- **object gaps the seven gates report on the wider sweep**, each a clip the port lacks or plays
  wrong: desk's background plane (its own 480-frame timeline, not a child clip) and stapler
  platform, devil's machine's terrain-plane clip that hides itself and is shown later and its
  foreground, dedede arena's kabula child, central highway's `plat2`, dracula's castle's
  foreground, bowser's castle's dust cloud (an effect the thwomp attaches by name from the
  game's shared effects, `attachEffect("global_dust_cloud")`, which the port does not play).

- **gangplank galleon wedges the engine at the intro.** the match never reaches the stand;
  no script error is logged. not read yet.

- **jungle hijinx's barrel hits fighters at spawn.** the barrel cannon declares a launch of
  115 and the port makes that a hit box; the source's barrel is entered and fired from, which
  is a behaviour the port does not have.

- **sand ocean passes every gate.** its vehicle shapes its body from a clip named as it runs
  (`"terrain_" + vehicleName`, one of eleven measured), asks its own body whether it is off, is
  aimed at by the camera as itself, and reads its class's constants from the stage; its terrain
  plane's ten frames walk only warning boxes, so the plane is compared by its children.
- **fairy glade, sky pillar, phase 8 and world tournament pass every gate.** their butterflies,
  stars, rain drops and dragon balls are clips the stage's code makes from the library at run
  time, which the port ships as vfx made on demand and moved as the code moves them (into a
  plane, or into a clip the port keeps). fairy glade's removal test reads as the source wrote it
  (the chain reader pairs innermost links first), and the source never removes its butterflies
  either: it tests a spawn x against 0 and 1. sky pillar's camera ray hands the camera the
  engine's own point in the stage's px. the comparer reads a root clip whose frames hold the
  planes over one painted picture by its children.

- **devil's machine, hyrule temple and dedede arena pass every gate; desk stands and runs.**
  a plane's own timeline pairs with the pieces cut from it (hyrule temple's clouds run its
  foreground's 1601 frames), a child a clip has no piece for is a stand-in the class writes to
  (devil's machine's dialogue text), and a line is judged unparseable on its code alone (its
  whispers carry any characters). desk passes every gate too: its stapler is a bounce platform
  (the clip declares `bounce_speed` and a `bounce` label the source's platform layer reads), which
  the port stands up as an object that sends a landing fighter up at the declared speed and plays
  the label (the port's arc tops out at 175 px where the source's does); mushroom kingdom 3's
  note block and three target tests carry the same declaration.

- **phase 8's acid waterfall exists, and its own timeline drives the stage.** the stage's spawn
  asked the library for the class's raw name (`SR388_AcidWaterfall`) while the object shipped
  under its cleaned one, so nothing was ever made; a class spawn now asks for the id the object
  was emitted under. the waterfall's clip carries frame scripts that reach past it: it shows the
  foreground's waterfall clip and plays its `start` as it begins, plays `end` as it ends and
  has the stage remove it on its last frame. those run on the object by its own animation and
  frame and cross to the stage as asks on the stage's own queue; live, the object spawns at
  ~1000 frames, plays `Idle` into `loop` (the source clip runs on, where this engine's default
  looped it), the foreground clip shows and plays `start`, the object ends and the stage
  removes it, and the next spawns ~3500 frames later, with no script errors.

- **crateria's platform stands where the tree put it, moves, and carries its ledges.** its
  class keeps its start as the place it eases back to, read in its constructor; the port ran
  that constructor before the stage's placement had landed, so the platform sat at the origin
  with its body, for the whole match, on every stage whose object reads its own place at
  construction. an object's setup now runs on its first update. its two ledge beacons declare
  the platform's `syncPlatform` key, and the deck face the port builds reads `leftLedge` and
  `rightLedge` true (structure stats, live); a fighter grabbing it on both engines is still to
  be measured, since the harness has no ledge-grab probe yet.

- **mushroom kingdom 3 cycles through its four sections.** the stage's phase counter is a typed
  field the port started at null, so it never left its first phase; it starts at 0 now. its
  terrain plane slides one section in per phase on its own timeline and flips each section's
  `fallthrough` from its frame scripts, and the port does both: a named surface's structures
  follow its picture by label and frame, and the plane's frame scripts fire when its pieces
  reach the frame, spawning and killing each section's blocks, chests, plant, lava and podobos
  through the stage's own methods (its kill loops reach the blocks by computed name, which is
  a lookup now). measured across a whole cycle, a fighter dropped over the note block rests
  within a px of the source in the top, middle and castle sections, on the transition platform
  between them, and falls through the bottom section's pit to its death in both (the note
  block under it is let through by the plane's first frame script, which now waits for the
  object's script to have run).

- **rest area's ssf2 side cannot be compared from a versus boot.** the stage reads a game-mode
  global (the all-star match count) to pick its trophies and backgrounds, and outside that mode
  the read throws every frame, so the ssf2 match freezes at the countdown. the fm port runs; the
  stand gate skips (its fighter never leaves where it was put), and S7 has no source to measure
  it against until the harness can set that global before the spawn.

- **a stage object's projectiles ship.** a class declares them in its own stats, each by the
  linkage its clip is registered under, and the port ships each as the same projectile a
  fighter's would be, under the stage's id: the clip found by linkage, only its pictures
  extracted, the launch and spawn offsets read off the declaration, a labelled clip cut into one
  animation per label. birdo's eggs fly from its mouth, hit the floor, play `die` and go (live).
  twenty installed stages declare projectiles; kingdom2 is the one verified.

- **shadows work, and a stage needs four things for them.** a light (a `SHADOW_LIGHT_POINT`
  point layer, placed overhead of the ground shadows land on, not of the camera box, since a
  converted stage sits wherever ssf2 drew it), an entry in `shadowLayers`, a COLOUR on that entry
  whose top byte is the opacity, and FADE RADII on the light. the last two are the ones that read
  as "shadows are broken" when left out: with no colour the shadow is built, placed correctly under
  the fighter and drawn at zero alpha, and with no fade radii the default end radius rejects every
  distance, so nothing draws however good the light is. all four come off the stage's own boxes now
  and clocktown casts shadows live. a declared `light_source` clip is only believed when it sits
  above the play area, since a symbol by that name is as often a lamp standing in the scene.

- **clocktown's ambient tint plates carry no strength anywhere in the file.** four measurements
  agree: the placement alpha reads 0.00 on all 1220 frames, the shape's single fill is alpha 0,
  `SSF2StageQuality.init` does nothing at the default quality (every branch in it only takes
  effects AWAY at lower settings), and the only runtime alpha writes in the whole stage are
  `RainWeather.setAlpha` and the tint helper that quality class calls. so the plates are inert as
  authored, the missing red end-of-cycle wash is not these plates, and the current rule that shows
  a wholly-invisible decoration at full opacity is a difference FROM ssf2 rather than toward it.
  wants a look at ssf2 running before it is trusted either way.

- **fraymakers exposes no blend modes; ssf2 blends ship as a SOLVED alpha overlay.** an ssf2
  placement drawn with a flash blend mode (bowserscastle's `bowsers_lightmask` uses HardLight)
  can't be reproduced at runtime: the renderer underneath has `blendMode` on every display object,
  but a mod only ever gets a `ContainerApi`, whose whole surface is addChild/addChildAt/removeChild
  plus state, and the scripting api's entire visual vocabulary is scale, alpha and visibility. no
  setBlendMode exists anywhere in the scripting api. reaching the field
  needs engine injection, which is fine for peptide's own testing and useless for a converted
  stage, since that stage runs on the stock engine.
  colour, though, IS reachable, and an earlier version of this note said otherwise: a DRAWABLE
  takes `addShader(new RgbaColorShader(rMul, gMul, bMul, aMul, rOff, gOff, bOff, aOff))`, which is
  swf's colour transform term for term, and a CONTAINER takes `addFilter` (hsbc colour, glow,
  stroke, displacement, mask). both verified live. a shader is not a filter: handing a container a
  shader fails with a cast error, which is what made this look impossible. the converter drives the
  shader per frame for vfx-hosted elements and bakes the transform into the picture where nothing
  can carry it.
  so the converter SOLVES the blend against the art beneath it and emits the result as an ordinary
  alpha overlay in the foreground, above the character containers (`fit_blend_overlay` in
  `stage_parser.rs`): per pixel it picks `C = HardLight(base, mask)` at the mask's own alpha, which
  reproduces the static art exactly AND still tints whatever moves underneath. baking into the art
  is the fallback when there's nothing to solve against; it's equally exact but tints only what
  was baked, which is wrong for the layer fighters run around inside.
  three gotchas worth keeping: the overlay must ship at FULL layer alpha (the authored foreground
  sheet wants ~0.5, and halving a solved overlay halves the blend itself), a blended placement must
  not be split into its own per-element layer, and the overlay layer goes ABOVE the engine's
  foreground containers. those containers host the animated foreground elements, so an overlay laid
  down before them is drawn under the very art it is meant to light: bowserscastle's terrain came
  out plain stone against the source's deep red. authored foreground art is not a light and keeps
  its place under them.
  bowserscastle is the ONLY stage in the corpus that has a blended placement at all (scanned all
  110), so it is the one stage this path is exercised by.
  remaining gap, structural: a chromatic mask can't be matched exactly on MOVING art. normal
  blending gives `out_c = b_c(1-a) + C_c*a`, so the slope is shared across channels and only the
  offset is per-channel, while HardLight needs a per-channel slope. a greyscale mask IS exactly
  expressible (black at `1-2t` + white at `2t-1`); `mask_chroma` reports which case a stage is in.
  quantifying the residue against ssf2 needs camera-matched shots, since `shot` captures the
  display root and the two engines' frames are currently different views.

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

- **decompiler body quality is the main translation gap.** call mapping is in decent shape
  (1619 unmapped calls corpus-wide, mostly the item system); what's left is bodies the
  decompiler recovers imperfectly, counted per character by the `/* ? */` markers. an
  unrecognized opcode is the expensive failure mode: the instruction stream desyncs and the
  rest of the method decodes as nothing, silently, so a body can go missing without any
  marker at all. the converter logs each one once, with the method it sat in; a switch table
  (an if-chain on the index), a for-each loop (`__iterNext`/`__iterValue` on the list) and a
  base-class field read all have readings now, which is what steel diver and race to the
  finish needed. when auditing a character, check for suspiciously short or empty function
  bodies against `dump_method <ssf> find <name>`, not just the marker count.

- **a projectile runs its own translated code.** projectile *entities* (visuals, boxes,
  animations, palettes) and *behaviour* (`<Pascal>Script.hx`) both generate from the source: the
  script is the projectile's own clip class translated, and the physics the source's engine runs for
  it (homing, owner influence, owner aim, trail, x decay, launch, lifetime, bounce) is ported into
  its `update()`. what is not yet 1:1 is caught by the cross-engine flight compare below, not left as
  a `// TODO` placeholder.

- **a projectile reads its own world position through `parent`, and rockets its summoner.** a
  frame script that takes `parent` as a value (`bomb = parent; ... bomb.x`) is reading the object's
  OWN world position through the positioned container SSF2 hangs it in; here that is
  `self.getX()`/`getY()`, so `parent.x`/`.y` are rewritten and the assigned form is snapshotted
  into a plain `{x, y}` so the aliased read has a value. the same script then calls
  `rocketCharacter` on its summoner; the engine's own version of that sets no velocity for this
  fighter -- it guards the fighter's state, drops a shield or dodge to idle, and forces the down
  special AIR attack, whose own motion is the hop -- and the port does the same (forces
  `special_down_air` under the same guards, keyed on the fighter's animation family). samus's
  bomb jump is verified live: the bomb explodes, the proximity check passes, `SPECIAL_DOWN_AIR` is
  forced, the fighter launches, falls, and lands, with no script error. found and confirmed by
  `tools/proj_probe.py verify <char>` (it fires the specials and reads what they spawn, which
  the C1-C6 fighter gates never do).

- **a projectile's flight is compared across both engines.** `tools/proj_probe.py compare <char>`
  fires each special that DECLARES a spawn on Fraymakers and SSF2 and reads where its projectile
  gets to off each engine's object walk. it moves the other fighter far downrange first (a shot that
  hits a parked target reads exactly like one that froze), charges a charge move to the top before
  measuring (or it reads as a dead projectile), and fails a build only on what no wall or target can
  fake: the port spawns nothing the source spawns, the source flies a shot a real distance and the
  port barely moves it, or a spawn the source keeps at the muzzle (an effect, a wave) sits well off
  on the port. a boomerang that flies out but does not come home is left to the printed travel and
  settled numbers, since isolating one shot's whole path off the source's effect-laden object walk
  is not reliable enough to fail on. moves and launch speeds come from the source's own declaration,
  so it holds for content that does not exist yet.
  the Fraymakers reader now trims the object walk to the engine's own live-projectile count: the
  walk reads a display container that holds a killed projectile a beat past its death and then goes
  stale, so a despawn was invisible (a shot that had died still read as present). the tagged eval
  behind the walk carries `getProjectiles().length`, and the reader keeps that many, favouring the
  entries that carry an animation. a projectile that should end but does not (or the reverse) is now
  observable, which is how pit's arrow despawn bug was caught.
  a sustained spawn (a held reflector, an owner-linked flight effect, an aimed teleport) only exists
  while its button is down or its owner stays in the move, so the single-tap driver never keeps it
  up. these are recognised from the source (the move's frame scripts or the spawn's own script read
  the held controls, or the spawn kills itself when its owner leaves the move) and reported as data
  rather than failed on, since the tap driver cannot sustain them on either engine; their spawn is
  verified where the move can actually be held. this cleared the false spawns-nothing failures on
  pit's orbitars and up-special effect and black mage's warp.
  the STUCK check now gates on the HORIZONTAL launch a move declares, not the greatest of the two
  axes: a spawn handed only a vertical launch (samus's morph-ball bomb pops up and drops on gravity,
  x speed 0) is a dropped/placed one whose cross-engine "travel" is a vertical drop plus the source's
  explosion effects, not a flight to compare, so it is no longer STUCK-flagged. a real horizontal
  flier that stalls (link's n-special arrow) still is. mario, black mage, pit and samus pass; link
  fails on exactly one genuine bug (its arrow, below).
  four general converter bugs it found are fixed:
  - x decay was a multiply (now the per-frame subtraction the source runs, so a boomerang comes back
    and a fast punch no longer leaves the screen in a frame).
  - a write of a thing's own position was left as a getter on the left of `=` (invalid Haxe that took
    the whole script down at load; now the setter).
  - a shot aimed by angle (`angleControl`) was struck out, so blackmage's meteor charged, hovered,
    and then sat there on release instead of dropping (now the two velocity writes the source makes;
    game and watch, kirby, ness, pikachu, chibi robo and tails aim shots the same way).
  - asking what attack a fighter runs (`getCurrentAttackFrame`) was struck, so a projectile riding a
    fighter neither followed it nor ended: pit's orbitars and its up-special effect both stick to pit
    while it runs the move now, and end when it stops (simon reads the same way).
  a fifth is fixed: a projectile that reacts to hitting something (`PROJ_COLLIDE`) was struck, so
  link's boomerang flew out, reversed, and sailed off instead of stopping at link. it now becomes a
  per-frame overlap read off each entity's collision box, firing on enter only, so the boomerang
  returns and despawns at the thrower (verified live; bomberman and the other collision-reacting
  characters boot clean).
  a sixth general fix: a neutral/side special whose whole charge lives inside one clip (samus's beam)
  played straight through and dropped to stand before it could fire, because Fraymakers loops a clip
  once. the clip is now split (in both the animation splitter and the sprite parser) so the base
  loops its charge section and the attack frames become `<name>_attack`, and the source's label jumps
  to "attack"/"attack2" route through `_jumpLabel` (keyed on the source clip, since many split
  animations share those label names). samus's beam now holds the charge, builds to full, and stores,
  verified live; the fire path reaches its createProjectile (force-play confirms). the one gap left is
  the fire's frame-14 decision is a single-frame script the live harness (which advances several
  frames a tick) can land past, so the fire is not observable in-harness, though at real 60fps that
  frame is hit; the re-press fire path runs off a timer and is not subject to the skip.
  a seventh general fix: the source has a macro-state named ATTACKING (one name covering every
  attack), and this engine keeps that as a state GROUP, not a single state, so `inState(ATTACKING)`
  read a state name the engine answers with null and the check meant nothing (it asked
  `inState(null)`). it now maps to the group ask (`inStateGroup(CStateGroup.ATTACK)`), method and
  constant together. pit's arrow, which ends itself once its owner is no longer attacking, never
  despawned without this; it now dies exactly as its owner leaves the attack group, matching the
  source (verified live off the engine's own live-projectile count). 52 other call sites across the
  corpus were reading the same dead check.
  a shot leaves from the fighter's own position plus its stated offset: the source's `getY()`
  stays at the feet whatever the pose (read off the per-frame record: link's arrow leaves 31 px
  above him, the boomerang 21, fox's laser 23, pikachu's thunder 220), and the port's spawn heights
  match those to a few pixels (`proj_parity.py`).
  a harness reliability fix went in alongside: the live session boots the INSTALLED package as-is
  and never rebuilds it, and `peptide convert` writes only source, so a reconvert left the installed
  `.fra` stale and a probe silently tested the OLD build (the lift fix read as still-broken for a
  while because of exactly this). the projectile probe now rebuilds a character's package whenever
  its source library is newer than the package before booting it, so a probe always tests what was
  just converted.
  a ninth general fix: a shot aimed by angle whose magnitude the decompiler could not recover came
  through as a null operand, and lowering it wrote a velocity of null-times-scale = 0, which clobbered
  the launch the spawn had already handed the projectile and froze it in place. the source's engine
  ignores a velocity write with an undefined magnitude, so the write is dropped and the launch stands.
  pichu's grounded thunder jolt travels again.
  a tenth general fix: a projectile with several custom local states named them all from the same base
  (three attack labels to CUSTOM), so the LState object literal carried duplicate keys. that is an
  invalid object, so the whole projectile script failed to load and it spawned with no working body
  (a null position, speed and animation). repeated names now take a numeric suffix (CUSTOM, CUSTOM2,
  CUSTOM3), and rayman's vortex flies forward off its launch again.
  the flight driver holds a held-to-fire special by the move's OWN chord (direction plus special, not
  a bare special that fires neutral-b) from a clean grounded stand, releases a charge once its window
  has passed to catch a release-fire shot (pac-man's power pellet launch), and reports a sustained or
  steered move (din's fire, aimed by the stick while held) as data rather than failing it on the short
  travel the tap/hold driver cannot steer. 31 of the 33 projectile characters pass the flight compare.
  two former failures were the harness mistaking a move it cannot drive or observe for a defect, and
  the compare now reads the source and reports them as data: isaac's growth positions itself off
  getGrabbedFoe().getX() so it fires only while a grab is live (the driver holds none, so the port
  rightly spawns nothing and the source's "travel" is the fighter's own moving parts), and rayman's
  vortex declares its own projectile_grabbed state, a grab trap that flies out then grabs and
  transforms in a couple of ticks, rendered in a grab layer the projectile walk does not sample (its
  spawn and launch are verified in-engine, the LState fix above; its brief flight is not a 1:1
  travel signal). both are read structurally from the source, so they hold for content not written yet.
  the other two are open converter gaps, and they have DIFFERENT roots, each traced to specific code.
  sonic's spin dash (special_down) is a multi-mechanism charge move, and it is now PARTLY fixed. the
  in-clip charge loop (anim_splitter) that only ran for special_neutral/special_side now runs for a
  CHARGING special_down too, gated on the "charging" label (reflectors and counters on special_down
  wear the generic "loop" label, not "charging", so they are left alone). with that landed, sonic's
  charge holds, the release reaches the dash clip, the dash's handoff runs (xConstantTurn set to
  xconstant and held stable), and the afterimage timer is created. the same widen made isaac and tails
  hold their own charging special_down as a real charge (C5 now reads it "held by its own code"), and
  blackmage/sonic/isaac/tails are unregressed on projectile flight and C1-C5, fox's reflector
  untouched, goldens unchanged. the roll's afterimage now spawns and the flight compare no longer
  flags special_down at all: sonic's one open projectile is special_up. a separate converter
  limitation remains for a self-looping fire clip (the roll's own dash loop is a frame-script
  gotoAndPlay that becomes playFrameLabel, which does NOT redirect the playhead mid-playback, so the
  clip cannot sustain its own roll from inside itself), but it is not what the compare fails on.
  sonic's grounded up special spring used to be the last open one, now fixed. the spring the source
  places is a two-step handoff: the character spawns sonicspringgroundduring, whose own frame script
  spawns sonicspringground and then self.kill()s. discovery read only the CHARACTER's spawn calls, so
  sonicspringground was never emitted (springidle was emitted and orphaned), the handoff spawned
  nothing, and the spring vanished at once. discovery now follows the projectiles' OWN spawn calls to
  a fixpoint (scanning each discovered projectile's behavior for what IT fires, discovering by linkage
  and pulling from a shared package the same way character-fired clips are), so sonicspringground
  emits and the spring spawns and persists. a name a projectile fires is resolved through the
  file's OWN table first (its `linkage_id` says which clip draws it), the way a character's
  spawns are; asked of a package beside the file only, it worked from the corpus copy (the
  file is its own neighbour there) and not from the installed archive. sonic passes C1-C6 and
  the projectile compare from the installed archive, with samus and link projectiles unregressed.
  sora's keyblade throw (special_neutral) is a different gap. SSF2's `forceAttack("b")` buffers the
  input that starts a move so the fighter performs it, and it has no script input-buffer analog here.
  it used to be struck out entirely, so an air special that forced its grounded finisher on landing
  did nothing on 28 characters. now the converter forces the move the call names: the argument is the
  SSF2 name of the move, `ssf2_to_fm_anim` maps it to the animation the entity carries, and a move IS
  its animation here (its keyframe scripts spawn its hitboxes), so the call becomes a playAnimation of
  that animation. fifteen of the affected characters were checked C1-C5 (blackmage, captainfalcon,
  link, megaman, simon, sora, mario, luigi, pichu, zelda, ness, wario, dedede, bowser, isaac), three
  of them C6 cross-engine (blackmage, link, captainfalcon), and link and megaman projectiles are
  unregressed, so the forced moves play and return to stand with parity intact.
  sora's keyblade (strikeraid) flies, comes back and is caught. its declaration is read in full:
  `sora_strikeraid: { classAPI: SoraStrikeRaid, linkage_id: "sora_strikeraidproj", xspeed: 25,
  xoffset: 30, yoffset: -17, xdecay: 2.4, ... }`. a getProjectileStats entry is a projectile because
  of the method it sits in, so a declaration with its own behaviour class (SoraStrikeRaid extends
  the api's projectile) is kept, its clip found by the linkage_id the table states, and its declared
  launch and spawn offset both installed. the projectile's other states are its own now: the clip
  under each label of the outer clip is cut into its own animation with ITS OWN frame scripts on it
  (merged onto one timeline, the return's frame 7 ended the flight on the seventh frame of the
  throw), the behaviour class's methods and constants come over too (`HAND_X_OFFSET = -24` read off
  the class's traits, `catchIt` and friends as functions), a projectile's `forceAttack("pull")` is a
  move to its own state (`Common.toLocalState(LState.PULL)`, the state named after the label), a
  label it asks its summoner to jump to is signalled onto the summoner's exports and taken up on its
  next frame (a call across scripts loses the caller's helpers), and the summoner's move it asks
  about is renamed to this engine's name. the invented "TODO: implement state logic" branch is gone:
  what a state does is on its frames. the projectile's stance clips are also kept OUT of the
  fighter's script (they were folded in by name and struck line by line). live: the keyblade leaves
  at 16.25, decays through zero, turns back, enters `projectile_pull`, tweens to the hand and ends.
  sora's play-test defects were all in the converter and all general:
  - side special float and arc: `air_ease: 0` is a cap of ZERO, not "none" (the source holds a
    vertical speed of exactly 0 for the twelve frames before the dash and again after the rise,
    until the frame that writes -1). the port fell at terminal through it; now it hangs, dashes,
    rises 7.8 on up and holds at the apex, and falls again at the -1, frame for frame with the
    source.
  - side special rotation: the source's `rotate` stat turns the art to face the way the fighter
    moves; this engine's autoRotate reads its art as pointing up, a quarter turn off. rotate is
    now the script's own (`__rotate({ on })` + `__applyRotate` each frame, atan2 of the velocity
    measured from the facing), declared per attack like air ease, reset upright when it ends.
  - side special hold (the multi-hit that carries the foe): three struck calls. `getType()` answers
    a kind number here and a name there ("SSF2Character") that the script compares and slices, so
    it is answered by name (`__typeName`, from the entity_type_names table); a method called by a
    worked-out name (`foe["get" + kind + "Stat"](...)`) goes through `__callByName`/`__objStat`
    (the api's stat getters, source stat names renamed, a stat this engine lacks answering the
    source's default); `getState() == 14` compares the source's own state numbers, which the file
    ships in its CState class, so the number is read back to its name there and asked through
    `__srcStateIs` against the states here it covers (source_state_names); `calculateKnockback` is
    carried as `__calculateKnockback` in this engine's knockback units; `getAttackBoxStat(1,
    "damage")` is `__hbStat`, answered from the move's declared boxes and the writes made since
    (every `updateHitboxStats` is noted for it); `move(0, -25)` crosses the size scale; a forced
    hitstun counts source frames.
  - double jump held back: the source's `endAttack()` at the end of the backflip ended the
    animation, and in JUMP_MIDAIR that left the fighter falling with no control until it died.
    `__endAttack` ends an attack's animation (the engine follows with its own landing) and puts
    any other airborne state into FALL. the backflip segment goes out as `jump_midair_back`,
    which this engine plays of its own accord on a double jump made holding back (without one it
    fell with no animation and no control), and a segment cut from a jump carries
    `allowMovement: true` so the drift the source keeps through it is kept here (an animation
    the engine does not know starts without it, measured).
  - effects: a sign on a scale is the source art's (its dust is drawn upside down and flipped back
    with `scaleY: -0.5`), so on a substituted engine asset only the size carries over; an effect's
    x/y cross the size scale like a projectile spawn's. three art
    defects behind "not aligned, not sized": a clip that draws several parts at once kept only
    the first (the fire ring is six fireballs, and one was shown), so every part now gets its own
    image layer, in effects and projectiles alike; a rasterized vector's picture starts at the
    shape's bounds, not its origin, so that offset now rides the placement (the blizzard ellipse
    hung below the feet where the source draws it over the chest); and a placement that states
    no matrix keeps the one its depth already has (a `Replace` swaps the picture and nothing
    else), where it was read as identity, so the keyblade's glow lost its 0.18 scale on its second
    frame and drew as a 250 px orange arc across the sky with the key itself dropped as a second
    part. pictured frame for frame against the installed game: the fire ring around the body, the
    blizzard ellipse over the chest with its bolts, the keyblade as a small spinning key with a
    small crescent.
  - the spell gauge is a clip the script builds for itself, and a label played on it threw and
    took the rest of the frame with it, so the spell sections played out of order; a label is
    played only on something that can play one.
  the flight compare covers 33 projectiles. the back double jump is driven through `hold`
  (`hold left`, then `hold left+jump`: the state enters, the backflip plays, the stick answers
  through it, the state ends into FALL) and through `seq` with `release:` as the neutral token
  (`none` is refused client-side). a matched-frame picture is taken at `speed 1` with `hold`,
  one `tick 1` + `shot` per frame, and the record read after each shot to say which frame it
  landed on (an eval between shots costs frames, the trace read does not). an adopted session
  keeps whatever fighter and package it had, so a probe packs first and boots. a stat another
  object is asked for (`foe.getCharacterStat`) comes back in the source's units
  (runtime_read_units). sora passes every character gate C1 through C6.
  the whole roster carries this breadth, not just the projectile characters. the character gates ran
  over all 45: every one passes C1 through C5 (convert, publish, boot with no script errors, held-
  ground speed matching the walkSpeedCap it declares, and every move plays and returns to stand), and
  every one checked cross-engine passes C6 too. the one C6 failure was the harness reading a move
  wrong, not a port defect, and it is fixed: simon's `attack` read as fall_loop where the source
  jabbed, because a heavy fighter lands slower after the gate's park (simon settles over ~6 frames of
  land_light and fall_loop) and the C6 move comparison capped a leading fall/land settling run at 4
  frames, so it read that landing as the move though jab1/special_neutral/etc. all played right after.
  fall/land are state names no move carries, so the cap is now 12 (well under any real move length),
  and simon passes C6 with mario/ganondorf/dedede unregressed. chibirobo's earlier C5 aerial_up miss
  was flaky driver timing and passes on a re-run. the port was correct in both cases.
  a C6 recovery-driver limit was found and mostly closed, and none of it is a port defect. the gate
  parks a fighter above the floor to drive its recovery (up special) from the air, but some fighters
  (fox, blackmage, pikachu) re-attach to the floor the instant they are set above it, so the recovery
  could not be launched from the air and read as fall_loop where the source flew. driven from a real
  jump the same recoveries play their special_up_air correctly (checked by hand), so the port is right;
  the gate simply cannot stage a recovery on a floor-snapping fighter (its set-Y does not take). the
  gate now detects the snap (on-floor right after the air park) and reports that input as un-driveable
  rather than failing it, the same way S6 skips a stage it cannot stand a fighter on; fox, blackmage
  and pikachu pass C6 with mario/dedede unregressed. (a jump-to-unstick fix was tried first and
  reverted: pressing jump made the recovery read as a double jump and regressed mario/dedede.) two more
  recovery quirks were the driven input, both fixed and neither a port defect: nothing was released
  between inputs, so the side special's held right leaked into up+special and goku read a side special
  first, now cleared by a release between inputs; and goku's teleport reads the heading only if up is
  already down as special fires, so the recovery is driven as up:2 up+special (a non-directional up
  special is unaffected by the extra up frames). isaac, tails, marth, metaknight pass C6. chibirobo
  also passes now: its aerial_up is the same floor-snap case as the recovery (it re-attaches to the
  floor when parked above it), so C5 reports the aerial as un-staged rather than failing it, the same
  way the recovery skip works; its C6 then runs and passes. goku is fixed too: its teleport recovery
  reads its heading only if up is cleanly down as special fires, so it intermittently played the side
  special; the driver now re-drives a port recovery that came out as a non-up special (up to twice,
  and only when it actually staged wrong, so a correct recovery never re-drives), and goku passes 3/3
  with samus/mario/fox/pikachu unregressed. kirby is fixed too: its attack read aerial_neutral in the
  full run (a floaty fighter can still be a frame off the floor when standing() gives up, and an
  attack pressed a frame airborne comes out as the aerial), though its jab was fine in every isolated
  reproduction. a grounded input must fire from the floor, so before a non-recovery input the port is
  now set back on the floor and the engine attaches it, rather than trusting the pose; a fighter
  already grounded is put where it already is, so a correctly-staged one is unaffected. kirby passes
  C6 with fox (a floor-snapping fighter) unregressed. every character checked cross-engine now passes
  C1-C6.

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

roughly the order a fresh agent should pick these up. the stage and character system gaps
are catalogued in the two new sections above; the items here are the main converter work.

1. **keep the sample sweeps green.** after any parser/emitter change: `just sweep-stages` (the
   fixed ten-stage sample) and `just sweep-stages-live` (the same ten, driven in-engine) and `cargo test --workspace && cargo clippy -- -D warnings`.
   (currently all green.) do NOT convert the whole corpus: hours, and tens of gigabytes. to
   size how widespread something is, scan with `--info`, which parses without emitting.

2. **move as much of the runtime as possible onto Fraymakers' own behaviour.** the runtime
   reproduces many source rules to the frame and the pixel (speed hand-offs, refresh counts,
   half-frame freezes, event phases) whose effect in play is a rounding error, and each one costs
   per-frame engine calls and code to keep matched. where Fraymakers already does the job
   (movement, gravity, hit handling, state changes, timers), use its native behaviour and drop
   the emulation; keep the runtime for what the content's code says that Fraymakers has no
   answer for. judge each rule by whether a player could tell, not by whether a probe can
   measure it.

   measured (`runtime_cost.py mario`, two fighters, the time a frame saves with each part
   stubbed): standing, 5.67 ms a frame, of which the runtime's update is 2.73; `sync` 1.73,
   `kind.engineFrame` 1.33, `kind.frame` 0.85, `MovieClip.tick` 0.68, `Effects.show` 0.49.
   fighting, 10.8 ms, update 7.6, `follow` 2.9. the parts overlap, so these are a guide, not a
   split (and the tool's own stubbing drifts its fighting baseline; an unstubbed fight holds near
   12 ms).

   a stage object costs about 1.0 ms a frame, idle: bomb factory's blocks, by the slope of frame
   time over live blocks (five points a run). the stage keeps adding blocks until its grid is
   full, so a long match there runs past 60 ms a frame. of a block's cost, the runtime's update
   is about 0.65 ms (its frame code 0.27, of which the block's own code 0.12; `sync` 0.14;
   the kind's upkeep 0.08; the clip tick about 0.1) and the engine's own object and faces the
   rest. this is the case to measure every item below against.

   the order, cheapest loss first:

   a. **no loss, do first.**
      - per-object upkeep first: every idle stage object runs the whole update twice a source
        frame. what it does not need (an object with no frame code, a clip that does not move)
        is the largest share left once the steps below are done.
      - the clock reconcile (`sync`, `follow`, `settle`, `pin`): read the animation, frame and
        length once per update and merge the two follow and settle passes. the largest measured
        cost, standing and fighting.
      - read each engine value once a frame (held controls, state, floor, animation, hitstop)
        and pass it down: fighters read the same few values 3 to 5 times a frame.
      - write only on change: effect frames, the camera clip, stage schedules and tints,
        placed-body faces, library clips, pan containers.
      - gate idle upkeep on what the object declares or listens for: projectile stat features
        worked out once (not polled every frame), launch fade only during a launch, the dying
        watcher only once destroying, the hold timer only when the code reads it, landing
        re-enable only when something is disabled.
      - `MovieClip.tick`: skip clips with no scripts or labels (a global half-frame parity would move
        frame scripts by an engine frame, so it belongs in c). `isPlane` worked out once, not per
        call.
      - compiler side: the camera fields lower straight to the engine call (positions and speeds
        already do). helpers Fraymakers has no call for (the utility class's math) stay ported.
   b. **rounding-error rules that go native.** fired-and-forgotten single-piece effects as plain `createVfx`, static backdrop
      pieces as native camera backgrounds, pieces riding an object as owner-relative vfx, and
      platforms moved by their plane's labels as structure animation. the last three need a live
      check that depth and phase hold.
   c. **try, then judge by feel.** the smash input window (FM's own strong input), freshness
      folded into damage. (the content's per-frame code now runs whole on every second engine
      frame: its half-step rewriting is gone from the compiler and the runtime, about 12% off a
      stage object, and mario's moves still match SSF2's frame for frame.)
   d. **keep.** everything paid per hit, which saves nothing per frame and is what players feel:
      hitstun hold, launch fade and first-frame gravity, throw relaunch, weight knockback,
      attacker freeze, hit gating, staling queue, refresh counting. also the AS3 value
      semantics, listener lifetime, held construction scripts, homing, the declared
      projectile stats, and a projectile's own death bounds (the engine's runs none of its end code
      and tells no one, and content waits on that end to fire again).
   e. **measure before deciding.** plain content fields when not online (every field access
      goes through a container today) and typed AS3 operators where the compiler can prove the
      type. both are compiler-wide and could outweigh everything in a.

   `rocketAttack` keeps a per-fighter attack table the source's engine keeps too, so a fighter
   added later rockets in nothing until it gains a row; it should come from what the source
   declares, if the source declares it anywhere.

3. **the portrait is the head clip, composited and MASKED**. a head clip is a little
   scene: pieces stacked at depths, the character art scaled down inside it, and a mask cropping
   the result to a window. taking one placement out of it got the drop shadow; taking the art
   without its mask got the whole body at the wrong size, which then covered half the HUD.
   `image_extractor::composite_head` rasterises every piece the clip places, honours each one's own
   matrix, and renders a piece that is itself a clip the same way (portraits nest). masks are the
   clip saying which part of the picture is the head: a placement carrying a `clip_depth` is a
   window over the depths it covers, which is how SWF states masking, so it needs no idea what it
   is looking at. what falls outside a window is not drawn, and the picture is cropped to what the
   windows frame. a clip can carry more than one window (captainfalcon has two), which is why they
   are applied rather than just read as a crop rectangle. it runs twice, once through the windows
   (the head the damage counter shows) and once whole (the character-select picture).

   the crop is the SOURCE's framing, kept whole. the window rect comes from the clip's own mask,
   and the margins inside it are kept rather than trimmed to the art: nearly every character's
   window is 52x38, so keeping it is what makes one character's head the same size as the next
   one's in the counter, which is how SSF2 shows them. trimming to the art instead scaled each
   picture by how much of its own window it happened to fill.

   every character in the corpus goes through this: 45 of 45 declare a `thumbnail`, every one of
   those clips is in its own file and carries a window, and all 45 composite. a portrait that
   carries more than a head (captainfalcon's helmet next to a second picture, wario's, bandana
   dee's) carries the same thing in SSF2's own counter, checked live on both engines.

   WHICH clip is the portrait comes from the character's own stats: `thumbnail` names it, and
   that is the clip SSF2's own damage counter attaches. `abc_parser::extract_string_stat` reads
   it (mario `mario_head`, donkeykong `dkhead`, marth `marth_damageIcon`, sandbag `sandbag_icon`).
   the old name-convention scan is now only a fallback for a package that declares none.

   the menu entity places the head in the slots that show one (`hud*`, `stock`, `icon`), scaled to
   FILL the frame the engine draws there rather than to fit inside it: a portrait's window is
   wider than the frame is, so filling means the sides run past it and the engine's own mask crops
   them, which is how a base-game character's picture sits in the counter -- head to the edges,
   not floating in the middle. the whole picture goes in `full`/`css`. the
   `_front` variants are the part drawn in FRONT of the counter frame, outside its mask: nothing
   of an SSF2 portrait overhangs, so they are blank now rather than a second unmasked copy of the
   portrait sitting over the whole HUD.

   NOTHING RESAMPLES THE PICTURE. a costume recolors by matching colors EXACTLY (the palette is
   `<source color> -> <costume color>` pairs, 76 of them for mario), so a resampled pixel is
   either an invented color the palette says nothing about or a neighbour's color in the wrong
   place. either way it survives the swap wearing the base art's color, which reads as coloured
   speckle across the portrait. so the picture is handed over at its own pixels: the composite is
   built at the resolution of its largest piece (a clip usually places the character art scaled
   DOWN inside itself, mario's at 0.55, and baking that shrink in was a resample), and the engine
   scales it into the counter's frame at draw time, after the swap, which is where a scale
   belongs. 93% of the portrait's pixels are palette keys; the rest are the art's own antialiasing,
   unmapped in the source too.

   the engine's texture filter is `Nearest`, so what it draws is point-sampled. that is the same
   deal base-game characters get -- they ship full-size art too (commandervideo's `hud` frame is a
   249x314 full-body render) and the counter crops it.

   verified live: mario's head fills the counter's window edge to edge, framed by the engine the
   same way a base-game character's is, and its colors are the source's own. `dump_head <file.ssf>
   <clip>` prints what a clip places, with depths and clip depths, and composites it.

4. **verify mario in FrayTools.** re-run mario, scrub frame by frame in FrayTools.

5. **projectile behaviour.** the movement a projectile declares but no engine stat covers is
   ported into its own `update` in `haxe_gen::projectile_physics`, keyed on the stats its package
   declares (so content nobody has written yet flows through the same path): lifetime, x decay,
   floor bounce, instance `limit`, `homingSpeed`/`homingEase` (chase the nearest opponent),
   owner `influence*` (the held stick nudges it), `controlDirection` (the owner aims it 8-way),
   and `trailEffect*` (drop an effect behind it every interval; the linkage id is the string
   field the parser keeps in `ProjectileData.names`). trail is verified live (samus's missile
   trails `missileTrail` along its path); homing by an offline replay of the emitted `update`
   plus a live spawn. `tools/proj_probe.py` watches projectile flight and instance counts, reading
   positions off the object walk (never an hscript `getX` on a projectile handle, which crashes
   the engine). what remains: a projectile whose motion is its OWN class code (a bespoke steer or
   spiral) still needs that body translated well, same as any script.

6. **character system gaps (in priority order):** see the table in "character system gaps"
   for the full stream.
   a. `forceGrabbedHurtFrame` (130 calls, 32 chars) -- investigate whether the FM grabbed
      state already drives the correct frame; if yes, comment these out as no-ops. the
      biggest entry that isn't the item system.
   b. the SSF2 autolink group (`setupAutolinkAngle` / `stopAutolinkAngle` /
      `updateAutolinkTarget`, 4 chars) -- one Script.hx hook on HIT_DEALT covers all three.
   c. `getScale` (62 calls, 9 chars) -- FM splits the axes; decide whether to emit
      `getScaleX()` or carry a helper that asserts the two match.
   d. the HUD damage-meter group (`addToDamageMeter` / `removeFromDamageMeter` /
      `updateBar` / `throbDamageCounter`, 9 chars) -- FM owns its own HUD, so these are
      probably `ssf2_only` no_equivalent rather than a port.

7. **validate stat scaling** against hand-tuned reference characters.

8. **delete the path 2 enumeration fallback** once a shipped release confirms it's unused.

---

## peptide / harness todos

- **a compiled `.hl` path for engine-side features.** engine features with real control flow
  are hscript today. a compiled Haxe module would mean merging it into the engine's bytecode at
  patch time (the engine has no module loader), remapping every function, type and string index
  across the two; `hlbc` can read and write those tables, but the remapping is a cross-module
  linker and needs a feasibility spike before any feature leans on it.
- **a converted fighter costs the engine about 1 ms a frame standing still and 1.3 fighting.**
  two converted marios run about 6.6 ms a frame fighting where two built-in fighters run 4 (the
  engine at full speed, `runtime_cost.py`, `--fight`). a call in the engine's script costs about
  5 us, so the runtime's cost is its count of calls. the sharpest part left is an attack's first
  frame, which makes the clip its label places and every clip that places (`follow`, about 2.4 ms
  of a fighting frame). a named child no code can reach is not made (nothing reads its name or a
  text it starts with, nothing walks the children, and its class only sets its own properties):
  a fighter's frames place boxes under names its code never asks for, and those were most of it.
- **a match with a built-in fighter in a later slot does not start from the harness.** the first
  fighter is booted as built-in or custom by whether its package is installed, and two different
  converted fighters start together; a later slot is always loaded as a custom package, so a
  built-in one there (luigi with commandervideo) ends the start. put the built-in fighter first.
  matches started from the game's own menus are not affected.
- **projectile flight is not compared across engines yet, though each engine can now be read.**
  the C1-C6 character gates measure the FIGHTER's animation, never what it spawns, so homing,
  owner-aim, owner-influence and trails are verified per engine (offline replay of the emitted
  update, plus a live spawn) but not fighter-for-fighter against SSF2. `tools/proj_probe.py` +
  `Engine.projectiles()` read a projectile's position off the object walk (`tree`) crash-free
  now, on both engines. what a cross-engine comparer still needs: the `tree` walk should tag each
  node with a NEUTRAL role (character / projectile / vfx / structure), computed engine-side from
  the object's type, so the host can pick out projectiles on either engine WITHOUT matching the
  SSF2 projectile class name (which is engine-internal and must not enter a tracked file, unlike
  the public `FraymakersProjectile`). with that tag, a `compare` mode fires the same move on both
  engines and lines the two projectile paths up frame by frame (aligned through the same coord
  model the fighter comparison already uses). a known thing it will surface: a converted homing
  missile whose flight animation ends can stall on-screen (its own state machine stops driving
  it); whether SSF2 does the same is exactly what the comparer would answer.
- **three stages that would not boot now do, each a general emit fault.** kingdom1's HUD
  template default (`hudTextString`, full of spaces and newlines) shattered the `[class-fields]`
  marker (packed `name=value` split on whitespace) into an unterminated string and a bogus var
  per word, stopping the whole parse; only a whitespace-free default rides the marker now, and a
  camera-background piece whose container the engine did not make is skipped instead of throwing.
  krazoapalace called `__addTo` (a Flash addChild of a bitmap with no FM analog) that shipped
  only when the stage made a library clip; the helpers ship whenever the body names one.
  traininggrid, a bare grid with no hazards or weather, never declared the verify counters its
  update increments and the gate reads; the set is declared once at the top when nothing else
  did. `check-emitted` no longer false-flags a `;`-line that closes more than it opens.
- **a flaky SSF2 boot no longer reads as a stage regression.** the source engine decrypts its
  assets on demand and a cold boot occasionally misses the window; worse, the gate cleaned up a
  prior session by its literal binary name (`peptide ssf2 session`), so a run under a frozen
  copy (`PEPTIDE_BIN`) left the old daemon holding its port and every following stage failed
  with address-in-use, reported as "the SSF2 session never came up". a batch of 52 stages read
  as failed on that alone. the cleanup now matches `ssf2 session` whatever the binary is called
  and a timed-out boot retries once; re-gated, those stages pass. `tools/floor-profile.py` and
  the sweep script take `PEPTIDE_BIN` the same way.
- **a backdrop carried by a parallax layer is invisible to the live walk.** the engine plays a
  parallax layer's animation itself, and the tree walk lists entities and containers, not the
  camera's layers, so S7 reports such a backdrop as missing on the fraymakers side (yoshi's
  island's sky: 35 frames in the source, three parallax layers here). the port draws it; the
  walk cannot yet read a parallax layer's frame counter. either the walk learns the camera's
  layers or an animated declared backdrop rides as objects in the camera containers, the way a
  segmented one does.
- a stage element that mirrors the fighters (kingdom2's shadow mask holds a clip per fighter
  whose length follows the fighter's current animation) makes the object walk depend on what
  the fighters are doing at the sample: the same stage passes or reports a MISSING child from
  run to run. the walk should leave out children that track a fighter.

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
   versus mode for a 1-2 player roster (wire byte `S`). SSF2 input injection is live too:
   `hold`/`release`/`seq` drive the controlled player via a per-frame applicator injected into
   `Controller.getControlStatus` (host control masks translated to SSF2's control-bit layout),
   verified live (`hold right` walks: XSpeed 0 -> 4 -> 0 on release; `seq jump:…` leaves the
   ground). still open: they don't yet take hits in a verified way. prerequisite for #6. (impl note:
   extras are built as UNROLLED straight-line blocks -- emitting the
   char-resolve helper inside a runtime back-branch corrupts the loop counter, so each of the <=3
   extras is its own guarded block.)
3. **`addCharacter`** -- drop one more fighter into the LIVE match on the fly, **working on both
   engines**. Fraymakers (`addCharacter`, aliases `addchar`/`add`, wire `n`) re-arms the per-frame
   deferred-spawn. SSF2 does a real live add: its per-match player containers are fixed-size at
   `StageData.startGame`, so `spawn` reserves one spare slot (exist=false, null character, capped at
   4) and `addCharacter` fills it -- set the slot's PlayerSetting, importData the stats, then
   construct the Character directly (its ctor self-registers via `addPlayer`/`addCharacter`),
   bracketed by de/activateCharacters like makePlayer, skipping makePlayer's start-game-only
   attachHealthBox. verified live: `addCharacter` on a 1-char match adds a real drivable `sandbag`
   (count 1->2, walks under input, match stable). note: the added fighter can bind to a different
   `pN` slot than expected. open on FM: the deferred add hasn't been re-verified against a
   versus-mode match for slots past the initial roster.
4. **scenario replay test env.** the `scenario` command sets up a deterministic, re-runnable
   scene: `scenario <p0 x,y[,vx,vy]> <p1 x,y[,vx,vy]> [<ctrl:frames>…]` places both players at
   fixed positions (optionally with world-space momentum), resets them to neutral STAND, then
   plays an input timeline on p0. re-run the exact line to replay it. host-side macro composing
   `eval` + `seq` through the DebugTarget seam, working on **both engines** (verified live on SSF2:
   the setup lowers `setX`/`setY`/`setXVelocity` to field writes and `toState(CState.STAND)` to
   `setState(0)`, then the input timeline plays via the SSF2 input applicator). still open: setting
   a precise animation FRAME (not just the state) and the hit-measurement readback (#6) that makes
   a scenario's outcome quantifiable.
5. **live move-tuning** -- the `tune <player> <hitbox> <stat>=<value> …` command hot-reloads a
   move's hitbox stats into the running match with no relaunch, **working on both engines**.
   Fraymakers addresses a hitbox by INDEX (`tune p0 0 damage=15 baseKnockback=50 angle=45`) via the
   engine's `updateHitboxStats`. SSF2 addresses by MOVE NAME (its attacks are move-name keyed):
   `tune p0 a damage=15 angle=45` mutates the `a` move's stored `AttackBoxes["attackBox"]` (an
   AttackDamage with public setters) via the TUNE verb -- `AttackDataObj()` (protected getter,
   reused QName) -> `getAttack(move)` -> the box -> set Damage/KBConstant/Power/Direction, read back
   as confirmation. mutating the stored box persists (it's re-imported each hit). move keys are the
   raw SSF2 attack names (`a`, `a_forward_tilt`, `special`, …). verified live: `tune p0 a damage=77`
   -> box0 Damage=77; `tune p0 a_forward_tilt damage=33 angle=200` -> Damage=33, Direction=200.
   still open: the UI surface, tweaking move *code* (not just stats) on the fly, persisting tweaks
   back to source, and tuning boxes past box 0 / by frame.
6. **in-engine hit measurement** -- the `hits [pN] [frames] [raw]` command watches one character
   and reports every hit it takes: damage dealt, and the launch it was sent with (distance per
   sample and the angle, 0 = right, counter-clockwise, which is how SSF2 states a hit's
   direction). host-side poll through the DebugTarget seam, **working on both engines**: the feed
   is `damage` + POSITION, because a launch never shows up in `currentVelocityX` on Fraymakers (it
   reads zero through the whole hitstop and the launch never lands there) and because how far
   something moved is the same question on either engine. the launch is read as the biggest
   single-sample move in a short window after the hit: not the hit's own frame (hitstop freezes
   the victim), not the first movement (the victim still drifts during hitstop), and the launch
   decays every frame after it lands, so the biggest move in the window IS its first frame.
   `hits raw` dumps every sample instead, which is how the hitstop and the launch are read apart.

   verified live on Fraymakers (sandbag jab: +9.0, launch 14.72 at 40 deg; dash attack: +14.0,
   launch 16.81 at 72 deg) and on SSF2 (mario jab: +5.1, launch 15.30 at 326 deg).

   SAMPLING RATE is the caveat, and it is SSF2's: every field there is a separate reflection round
   trip, so a sample covers many frames and the launch is per SAMPLE, not per frame. damage totals
   are still right (damage persists), and Fraymakers samples about once a frame. making the SSF2
   number per-frame means logging damage in its injected per-frame recorder, which today records
   p0's label/frame/position only.

   still open: hitstun frames, the KO-threshold search (binary-search the dummy's % for the lowest
   KO), the active-box dump (every active hit/hurt box this frame), and two position confounds:
   `p0.flipX()` returns 0 not a facing sign, and `getX` carries an offset from `setX` (`setX(0)`
   reads back `getX -19.5`).
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
10. **way more hscript commands.** convenience commands wrap common eval patterns through the
    one dispatch path: `scenario` (#4), `tune` (#5), `dmg <player> <value>` (set damage percent),
    `info` (one-shot readout of both players' x / state / damage / team), `reset` (neutral state +
    zero momentum), and `kill <player>` (force a KO into the bottom blast zone). all validated +
    unit-tested. more can be added the same way (a `Cmd` registry entry + a `parse_*` that returns
    `Command::Eval`).
11. **SSF2 command parity.** the SSF2 backend lowers the same command vocabulary to reflection
    verbs, so the host-side eval macros work there too. `dmg`/`info`/`reset`/`kill` are verified
    live on SSF2 (the evaluator handles a `;`-joined multi-statement line, an `[…]` array literal,
    the `damage._damage` getter/setter idiom, position/velocity setters as property writes, and
    `toState(neutral)` as the SSF2 state setter); `console` declares its gap (SSF2 has no debug
    console). `dmg`/`info`/`kill` are verified on Fraymakers too. input injection
    (`hold`/`release`/`seq`, `scenario`'s timeline), `addCharacter`, and `tune` all work on SSF2
    now (see #1/#3/#4/#5). parity is NOT closed out; still open:
    - **the `status` feed on SSF2** isn't working 100% yet (the matchStatus readout the GUI
      widgets consume), so the command set is close but not at parity.
    - **`reset` on Fraymakers** trips a script error (one of its setters / `toState` resolves to
      null on the FM character); the SSF2 path is unaffected. pre-existing, unrelated to the SSF2
      parity work, worth a separate look.

### stage geometry is 1:1, measured

`PEPTIDE_AUDIT=1 peptide ssf2 stage <file>` compares every placed object's emitted transform against
the source matrix it came from, frame by frame. clocktown: **91 objects, 51709 frames, worst position
error 0.00px, worst rotation 0.0000deg.**

two things that hid behind it:

- the audit read a frame's own scale and compared it against a source quad already scaled to the
  stage, reporting every placed piece a `size_multiplier` small (a 632px miss on the largest plate)
  that was never in the output. the emitter composes the stage scale onto the frame's; the audit
  now does too.
- the audit shared the emitter's mirror handling, so it read 0.0000deg while the output was snapping
  half a turn. a per-frame turn now comes from that frame's OWN matrix -- first column gives angle and
  width, second column read along that angle gives a height whose sign carries the reflection -- so
  one unmirrored picture serves every frame. checking the emitted keyframes directly, rather than
  through the audit, is what caught it.

### the live bridge is wedged

the engine launches a match (`LAUNCHED`, no script errors) and then never answers: no `ANIM:` lines,
no reply to `shot` or `tree`. it reproduces on a STOCK stage with no converted content, so it is the
environment and not the port. a previously-good package fails the same way, which is the control.
until it clears, stage work is verified against the emitted artifacts (the audit above, and reading
the emitted keyframes) rather than in engine.

## a projectile that outlives its move

measured on chibirobo's down air, frame by frame through the log mirror:

- `initialize()` runs and `update()` keeps running. state stays `ACTIVE`,
  animation `projectileIdle`, for as long as the object is listed.
- timers registered in `initialize` work: `addTimer` hands back a uid and the
  callbacks fire every frame.
- `onTeardown()` runs, once per spawn. the object ends.

so the object is not stuck. what is unexplained is that `tree` keeps counting it
afterwards, one per aerial, while the control does not: mario fires five
fireballs and the count goes back to one.

the engine's 20-second DESTROYING warning is NOT evidence of a conversion
defect. a stock fighter produces it too, about its own stock projectile, in a
match with no converted content in it. the sweep filters it out.

## the engine's own scripts complain when a converted fighter dies

two `Invalid function null` errors from `CharacterScriptCommon` and
`GameObjectScriptCommon`, once per death. it is universal to converted fighters
and it is ours: driven the same way, twice each, a stock fighter dies clean and
every converted one does not.

ruled out by testing rather than argument:

- a missing `ko` animation. no converted fighter has one, and giving mario one
  changed nothing.
- a listener registered on a handler that does not exist. mario has none.
- the fighter not handling its own knock-out. a listener added for it never
  fires at all, so nothing is dispatched on this path.

the error lands BEFORE the ko animation, so it is the moment of death rather
than the revival. the fighter revives and the match carries on, so nothing is
visibly wrong yet.

worth knowing for whoever picks this up: a single forced death is not enough to
see it. checking too soon after one shows nothing and reads as a clean fighter,
which is what made this look per-character for a while.

## two emitters, one filename

a clip a character fires can also be emitted as art that stands on its own. both
used to be written as `<sourceName>.entity`, so whichever went second was the
only one left, and the move fired a thing with no behaviour behind it. the
engine then falls back to the owner's own content and plays a CHARACTER
animation on a projectile, which is how it surfaced: a frame script of the
character's ran with a projectile as `self`, so nothing the character's script
declares was in scope and the frame reported `Unknown variable` for a name that
is right there in its own file.

the entity file is now named by the id it is addressed by rather than by the
name the source spells it with, so the two no longer share one name.

worth keeping in mind when a frame reports `Unknown variable` for a name its own
script clearly declares: log `Std.string(self)` from the frame before assuming
anything about scoping. it names the class of whatever is running it.

## content a character asks for and its package does not ship

11 references across 8 of the 45 characters, 9 distinct ids. `content_audit`
names every one at convert time, so the list is a `peptide convert` away.

| character | asks for |
|---|---|
| donkeykong | `bulletProjectile` |
| kirby | `sorastrikeraidProjectile` |
| naruto | `fthrowlunaiProjectile` |
| pacman | `pacmanhydrantwaterProjectile` |
| pit | `pitfsfirst/second/fourthProjectile` |
| sonic | `sonicspringgroundProjectile` |
| sora | `sorastrikeraidProjectile` |
| wario | `wariomanafterimageProjectile` |

they are not converter mistakes: the reference is faithful to what the source
asks for. what makes these the last of them is that nothing states where to
look. a fired name is answered three ways, all of them the source's own words:
a clip of that name, an entry in the package's projectile table giving the clip
it draws with, and the same table read out of ANOTHER package when the name is
another fighter's. these nine are answered by none of the three.

sora is the clean example. it fires `sora_strikeraid`, ships one clip called
`sora_strikeraidproj`, and its table has four entries, none of them that one.
the file also declares a class whose name folds to exactly what is fired -- but
the package does not bind that class to a clip, so the binding is not the link
either. a rule about how the two names LOOK would join them, and would be a
guess about every name that ever gets written.

every place the source could state it has now been read: the clips the package
names, what its symbol table binds each class to, its projectile table, the
behaviour class's own code, the other files beside it, and the game's whole
library of three hundred and seventy-two packages. six of the nine are declared
by NONE of them -- the fired name is a real string in the fighter's code and
there is no clip of that name anywhere, so the art is not in anything that can
be read. wario is the plainest: it spawns `warioMan_afterimage`, ships
`wario_afterimage` for the ordinary form, and carries no clip for the wario man
one at all.

the other three are sora's `sora_strikeraid` (and kirby's copy of it), where the
package declares a CLASS of that name but binds it to no clip, and ships one
clip called `sora_strikeraidproj`. two facts the package never joins. a rule
about how the names look would join them and would be a guess about every name
anyone writes after this.

so this is closed as far as reading takes it. what would move it is content, not
code.

the number was 273 across 43 until the check that produced it was fixed twice --
it ran before the package was written, and it could not read an ask written into
a frame script at all -- and 34 until a borrowed clip was looked for in the files
beside the one being converted as well as in the game's own library, and until a
package was searched under the name it gives for itself rather than the name of
the file it is in.
