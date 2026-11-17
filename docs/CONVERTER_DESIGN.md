# converter design -- keep the source's shape

how the converter turns an SSF2 package into a Fraymakers project, what that project looks like,
and where each part of the job lives. the same rule as [`PEPTIDE_DESIGN.md`](PEPTIDE_DESIGN.md):

> **push logic to the highest, most readable layer it can live in.** Rust does mechanics. what a
> human would want to read or change about how content behaves lives in Haxe or in data.

status and open issues live in [`STATUS.md`](STATUS.md).

## principles

1. **keep the source's shape.** a source class's fields, methods and frame scripts stay its own,
   under its own names. a frame script stays on its frame.
2. **one path.** a fighter, a projectile, an item, a stage object, a projectile a stage fires and
   a stage all go through the same model and the same writer, and come out as plain Fraymakers
   code. nothing falls back anywhere.
3. **read the model once.** one pass reads a package into a source model (classes, symbol
   bindings, timelines, declared stats). everything after reads the model, never the package
   again and never a name guessed from another name.
4. **code in `.hx`, data in json, mechanics in Rust.** nothing a human would edit lives in a Rust
   string or a json field.
5. **only our code and the input's.** output holds the content the user gave the converter and
   the skeletons it is filled into, nothing of SSF2's: the SDK classes every package bundles are
   answered by the API tables and never emitted, and SSF2's API names are renamed
   (`runtime/api_names.jsonc`). the one exception is the type strings `getType()` answers, which
   content compares against.
6. **nothing is silently dropped.** a statement the writer has no plain form for is left out with
   a `// (not ported: <why>)` comment where it stood and listed in that object's gaps. a writer
   failure that isn't a statement (a structural problem) fails the conversion.

## the converter's structure

```
.ssf -> source model -> native writer -> each object's script, filled into its skeleton
                            |                 + port_report.json (what each leaves out)
          API tables (runtime/*.jsonc) + skeletons (runtime/native/*.hx)
```

| part | language | home | owns |
|---|---|---|---|
| source model | Rust | `src/model/` | classes, their superclasses, fields, methods, frame-script registrations; which clip each symbol and timeline frame binds; declared stats as the source states them |
| native writer | Rust | `src/compile/native.rs` | the model's trees as Fraymakers code: fields as the engine's state containers, methods and frame scripts as the script's functions, every API call as its row |
| emitters | Rust | `src/emit/native_fighter.rs`, `native_projectile.rs`, `stage.rs` | fill each object's skeleton: its state, its own code, the blocks that code needs, the tables its declarations give |
| API tables | JSONC | `runtime/*.jsonc`, `runtime/engine.names` | the API's class surface, each call as the engine's code, names, constants, what declared stats and the source engine's states become |
| skeletons | Haxe | `runtime/native/*.hx` | every object's script and the shared blocks it takes |
| extraction | Rust + JSONC | `src/`, `mappings/` | art, entities, boxes, stats tables, frame-data doubling, packing |

files under `runtime/` are read from disk at convert time, like `commands.hsx` and the mappings,
so an edit needs no rebuild.

### how a call is written

1. the model is read through `runtime/api_names.jsonc`, so content uses the names output does
   (`playFrame`, `GROUND_TOUCH`). a content class named like one of the engine's own script
   globals (`runtime/engine.names`) is written under another name, so it never hides the
   engine's.
2. a name content inherits from an API class resolves against `runtime/api_classes.jsonc`: each
   class, what it extends, its members, getters and setters, plus `globals`.
3. the call is written as its row in `runtime/native_calls.jsonc`: the engine's code, what its
   value is, the block of `runtime/native/utils.hx` it calls (`helper`) and the gap it leaves
   (`gap`). a call with no row is a gap.
4. Flash's classes the engine has its own version of (`Point`, `Rectangle`, `ColorTransform`,
   `RegExp` ..) are written as that version (`runtime/native_classes.jsonc`); drawing calls this
   engine does nothing for are not written when their arguments do nothing on their own
   (`runtime/inert.jsonc`).

a package that defines a member with a listed name keeps it. what another object's code asks of
an object goes through what that object publishes on its exports (its fields, its declared stats
as its stats' metadata, the methods others call on it).

### skeletons

| skeleton | object | blocks it takes where its code needs them |
|---|---|---|
| `fighter.hx` | a fighter | `fighter_rules.hx` (what the source's engine does around every fighter: hits, refresh, freshness, attacks, items, ledges, rockets, copied powers ..), `fighter_tables.hx` (lookups worked out from what it declares), `parts.hx` |
| `projectile.hx` | a projectile | `helpers.hx`, `floor.hx`, `touch.hx`, `refresh.hx`, rows of `runtime/projectile_stats.jsonc` |
| `item.hx` | an item (a custom game object carrying the item handler) | as a projectile |
| `object.hx` | a stage object | `object_rules.hx` (its body, its own floor, its rehit) |
| `stage.hx` | a stage | `stage_boot.hx` (the pieces its boot puts down and the upkeep its frame runs) |

`utils.hx` holds the API's helpers as plain functions and `classes.hx` the classes a field can
hold one of, flattened under the field's name. `//:` lines in a skeleton are notes for whoever
edits it, left out of what is written.

### what an object makes

every name an object's code makes something by is read off the source, wherever it declares it:

- a projectile it fires: its owner's declarations, its own class's `getProjectileStats`, or its
  own declaration's `projectiles` (an item that fires shots lists them there).
- an object it makes by its class: that class, with its `ownStats` as its declaration (an enemy),
  or the API's projectile running its clip's code (a clip class).
- one it makes that its package does not carry: lent from the game's shared package that declares
  it, art and class, and so is what the lent object's own code makes in turn.
- a clip class is dynamic in the source, so a name its own code writes on itself is a field.
- content named like one of the engine's own globals (`runtime/engine.names`) is renamed, so it
  never hides the engine's.

### gaps

each conversion writes `port_report.json`:

| object | fields |
|---|---|
| fighter | `gaps` (the fighter's own), `projectile_gaps` (by entity), `items_without_own_code`, `unplaced`, `projectiles`, `box_writes` |
| stage | `gaps`, `objects` (each `{gaps}`; one whose class cannot be written is left out, `not written: why`), `projectiles` (each `{gaps, unplaced}`), `unplaced` |

`tools/tests/gap_scan.py` converts every installed package to scratch and tallies the gaps, so the
next row to write is the one most content needs.

## the converted project

- **frame code is on the keyframes.** an animation's first engine frame of each source frame
  with a script carries that script, written in place; keyframe code and the script share one
  scope.
- **the engine plays the animations.** a source frame is two engine frames: its script runs as
  the first is entered. the source's timers are the engine's at twice the frames, and the code the
  source runs every frame runs whole on every second engine frame.
- **native at conversion.** what the source's engine does from an attack's declared stats is
  settled when converting, as the engine's own stats or a few lines in that move:
  - declared stats go into the move's AnimationStats (`runtime/attack_stats.jsonc`: slowdowns to
    friction, `air_ease` to `terminalVelocity`, what may cancel it); a literal write its code
    makes becomes `updateAnimationStats` on the spot.
  - a jump by label from a clip's own code is `playFrameLabel` inside the playing animation, or
    `playAnimation` for a label that starts one.
  - a floor-touch listener whose only work is to take the move to its landing section is
    `landType: TOUCH` into the move's landing animation.
  - steering is the move's `allowMovement`; a declared `cancel` is a check on its first frame.
  - an autolink toward a point the clip names is the engine's autolink (`runtime/autolink.jsonc`),
    with an `autolinkpoint<n>` layer per hitbox.
  - the source engine's own states are their animations' stats, first-frame code and points
    (`runtime/engine_animations.jsonc`).
  - box stats a clip writes as an animation starts are that animation's own HitboxStats;
    intangibility, invincibility and super armor are the animation's `bodyStatus`.
  - what an attack declares that the engine has no stat for (combo, charge) is code its first
    frame runs (`code` in `runtime/attack_stats.jsonc`).
- **effects.** an effect the code attaches and lets go of is a plain `match.createVfx`; one it
  holds is followed by its clip's frame and what the code sets on it.
- **events.** a source listener on an event the engine raises is the engine's own listener
  (`runtime/events.jsonc`), handed a record in the source's terms. a listener on a fighter ends
  with the state it was added in unless it is added to persist.
- **speeds.** the source slows a speed and then moves on it; this engine moves and then slows it.
  a fighter's x speed written, or carried onto or off the floor, is held a step and a half of the
  slowdown where it falls short of the source's; one an attack keeps a share of is held at
  2m/(1+m). a y speed is held a step and a half of gravity ahead in the air.

### state and online play

an online match rolls back: the engine saves its state every frame and replays from a saved one
when a remote input arrives late. a script's plain variables are not part of that state; the
engine's containers (`self.makeInt` and the rest) are, and the engine makes them only at the
script's top level, before `initialize`. so every skeleton opens with the object's state in
containers, a class's fields included, and a container holds a value, not what was done to it
(an array a field holds is changed as a copy put back in its place). the engine keeps few string
and true-or-false containers for a whole match, so those are object containers.

`netcode = 3` in `match_settings.conf` runs a local match that rolls back as an online one does;
`tools/tests/rollback_parity.py` checks each replayed frame against its first playing.

## engine facts the design rests on

**one Fraymakers frame**, measured on a converted fighter:

1. the script's update
2. timers due this frame
3. end of frame: hit detection, hit events, then the engine's own state and animation changes
4. the animation advances one frame and the entered frame's keyframe script runs
5. draw

setting frame F during update or a timer shows F+1. an engine-driven animation change shows up at
the next update.

**the source's schedule**: a timer of N frames runs before the update of its Nth frame, a frame's
script runs before the frame is drawn, a chain of label jumps inside one frame runs within that
frame, a label asked of another object lands on its next source frame, and a new clip's first
frame runs once its maker's script is done. a state's first frame shows the frame its entry put
the clip on, and the clip moves on from the next frame.

**calls between objects.** a function an object publishes runs in that object's own scope when
another fighter, a projectile, the stage or the console calls it: its own variables, its own
functions, its own `self`, synchronously. once the object's script has ended, its functions can no
longer reach its script's names.

**the script interpreter** differs from AS3 where the writer has to say the source's meaning:

| construct | this interpreter |
|---|---|
| a test, `!x`, `a && b` on anything but a Boolean | only a literal `true` passes; `&&` answers a Boolean |
| `<` `<=` `>` `>=` with a null or a NaN | not AS3's answer |
| an int compare | by the difference, so 0 > -2^31 is false (int's least and greatest are written 2^30 either side) |
| `>>>` | signed |
| `o[name]`, `Reflect` | no field by a run-time name; no `Reflect` or `Type` |
| `a[-1]` | stops the script |
| a call with fewer arguments | an error |
| `catch (e:Type)` | hangs |
| `Math.floor` above 2^31 | overflows |

**costs** in Fraymakers' script interpreter, per operation: an arithmetic step ~2 us, an engine
API call ~3.4 us, a call to a script function ~8 us. the frame is 16.7 ms.

**the decompiler** is the source model's own (`src/model/`), built on the swf crate's instruction
decoder. every class of the 47 corpus packages reads with nothing unreadable, in a few seconds;
`tests/source_model.rs` holds that bar.

## verification

- **gates.** `tools/verify.py` (six for a fighter, eight for a stage) on both engines.
- **parity.** `hit_parity.py`, `proj_parity.py`, `rollback_parity.py` and the rest in
  [`TESTING.md`](../TESTING.md).
- **gaps.** `port_report.json` per conversion; `gap_scan.py` across the installed game.
- **structure checks.** `tests/native_parses.rs` parses the skeletons, `tests/conventions.rs`
  keeps content ids and keyword lists out, and the goldens, the tally, the soak and the
  frame-data checks stay.
