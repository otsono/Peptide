# converter design -- keep the source's shape

how the converter turns an SSF2 package into a Fraymakers project, what that project looks like,
and where each part of the job lives. the same rule as [`PEPTIDE_DESIGN.md`](PEPTIDE_DESIGN.md):

> **push logic to the highest, most readable layer it can live in.** Rust does mechanics. what a
> human would want to read or change about how content behaves lives in Haxe or in data.

this is the target design. the status of the move to it lives in [`STATUS.md`](STATUS.md).

## why

the converter reshapes the source into Fraymakers' shape first and translates second. the source
is classes, instances and timelines. the output is one script per engine object, with every class
merged into it and every frame's code cut out and stored in the animation data. each thing the
reshaping lost then gets put back by a patch, one symptom at a time.

across the converted roster:

| what the reshaping produces | count |
|---|---|
| frame-script fragments stored inside `.entity` json | 30,959 fragments, 134,766 lines |
| helper definitions copied into scripts | 56,941 |
| lookup tables generated into scripts to map names back | 8,409 |
| functions renamed with a class suffix because two classes were merged | 705 |
| "reset this clip's fields on entry" blocks, pichu's entity alone | 129 |

and the translation on top of it is text rewriting: the decompiler builds a real syntax tree,
prints it, and hundreds of regex and find/replace passes read the text back. fighters,
projectiles and stage objects each have their own emitter with its own helper set, so one fix
lands three times (a spawn-timing fix touched five converter files).

what that costs, concretely:

- a clip class's fields become fighter-wide variables, so two clips' state can collide, and an
  instance's lifetime is faked by a reset block on entering its animation
- frame code lives in json, where no human reads it, no diff shows it and no parser checks it
  as a whole
- a timeline operation (a label jump, a stop, a jump into another object's clip) has to be
  mapped back through tables, and each mismatch is a new patch (label asks on even frames, a
  first animation's lead-in frames, a trimmed end frame and its exception, a segfault from it)
- the knowledge is scattered: Rust comments, Haxe written as Rust strings, TOML templates, three
  emitters

## principles

1. **keep the source's shape.** a source class is one class in the output, with its own name,
   fields and methods. an instance is an object with the source's lifetime. a timeline is a
   timeline: frame scripts stay methods of their class, registered the way the class registers
   them.
2. **one path for every object.** a fighter, a projectile, an item and a stage object go through
   the same model, compiler and emitter. what differs between them is the base class the source
   says each one extends, and that picks a runtime module. no emitter per kind.
3. **read the model once.** one pass reads a package into a source model (classes, symbol
   bindings, timelines, declared stats). everything after reads the model, never the package
   again and never a name guessed from another name.
4. **code in `.hx`, data in json, mechanics in Rust.** nothing a human would edit lives in a Rust
   string or a json field.
5. **only our code and the input's.** output holds this runtime and the content the user gave the
   converter, nothing of SSF2's: the SDK classes every package bundles are answered by the
   runtime's own (`runtime/api/tables.hx`, `api/helpers.hx`, `fighter/fighter_ext.hx`) and never
   emitted, and SSF2's API names are renamed to the runtime's (`runtime/api_names.jsonc`). the one
   exception is the type strings `getType()` answers, which content compares against.
6. **nothing is silently dropped.** a member the runtime lacks is listed in the conversion report
   and logs once when it runs. nothing gets commented out.

## the converter's structure

| part | language | home | owns |
|---|---|---|---|
| source model | Rust | `src/model/` | classes, their superclasses, fields, methods, frame-script registrations; which clip each symbol and timeline frame binds; declared stats as the source states them |
| compiler | Rust | `src/compile/` | the decompiler's tree to Haxe, one lowering per AS3 construct, no game API names |
| emitter | Rust | `src/emit/` | assembles each engine object's script from runtime modules, the object's data and its compiled classes |
| runtime | Haxe | `crates/ssf2-converter/runtime/**/*.hx` | the source's API and engine behaviour, implemented once; its files are the list of what exists |
| extraction | Rust + JSONC | as today | art, entities, boxes, stats tables, frame-data doubling, packing |

the extraction side stays as it is. what goes: the text passes in `api_mappings.rs`, the script
assembly in `haxe_gen.rs` and `projectile_gen.rs`, the script text passes in `stage_emit.rs`, the
helper strings in Rust and the TOML templates, and the decompiler's own renames and unit rules.

### runtime layout

| module | holds |
|---|---|
| `runtime/core/` | units at the boundary, object handles, timelines and the frame-script registry, timers (the engine's own), events and listeners, spawning, the AS3 language helpers |
| `runtime/fighter/` | what the source engine does around every fighter that Fraymakers does differently: air ease, attacks re-enabled on landing, engine charge, combo continuation, the source's knockback rules |
| `runtime/projectile/` | the source engine's projectile: launch, lifetime, decay, homing, the stick's pull, control, trail, bounce, limit, bounds, its death |
| `runtime/stage/` | stage objects, placed bodies, parts; `boot.hx` makes what a stage puts down as the match starts, from its `DATA.stage.boot` tables; `stages.hx` is the stage's kind (the boot once the match is live, names and regions, the schedules the boot put down); `planes.hx` is its planes (the stage clip's children, bound to the elements drawn for them, plus named surfaces, ground friction and push, library clips); `weather.hx` draws the clips the stage's weather class moves; `bodies.hx` is a stage object's kind (`entry/object.hx`): its body (a placed outline, or the floor its code makes), the stats it declares in the engine's terms, its hit refresh, no hit freeze, and the stand-in its maker holds until its script starts (`DATA.faces`, `DATA.object.invoke`) |
| `runtime/api/` | the API members content calls, grouped by the object they are called on |
| `runtime/flash/` | the Flash classes content uses (Point, Rectangle, Dictionary, ColorTransform, filters as data, Event, Error) |
| `runtime/entry/` | each object kind's entry (`initialize`, `update`, what the gates ask), emitted last |
| `runtime/engine.names` | the engine's own script globals, measured in a running fighter; a content class by one of these names is written with a `_` after it |
| `runtime/api_calls.jsonc` | content's API calls written as the Fraymakers calls they are, at conversion (`getX` is `{eng}.getX() / 1.3`); a listed call leaves nothing for the runtime to answer, and the runtime member it replaces is deleted. `by_kind` holds the rows one kind writes its own way (a stage's planes are its own clips; anyone else's are the fighters' layer) |
| `runtime/api_names.jsonc` | SSF2's API and SDK names against the runtime's own (`SSF2Character` is `PeptideCharacter`; `stancePlayFrame` takes Fraymakers' `playFrame`, and a verb takes Fraymakers' name wherever it has one); the runtime's core is `PeptideRuntime`; the model is renamed through it as a package is read, so output carries this runtime's names |

files are read from disk at convert time, like `commands.hsx` and the jsonc tables, so an edit
needs no rebuild.

### direct translation first

most of SSF2's API has a Fraymakers counterpart, so a call is written as that Fraymakers call at
conversion wherever it can be (`api_calls.jsonc`), and the runtime keeps only what cannot: state
the source reads back that the engine does not keep, the source's frame schedule, and engine
behaviour Fraymakers lacks. every API member is written at conversion, the same way on every
object that has it:

| member | written as |
|---|---|
| Fraymakers has it | the Fraymakers call, in place (`{eng}.setXSpeed(v * {speed})`), several in a row where it takes several |
| behaviour of our own | a function on its class, `PeptideGameObject.startAttack(me, ...)`, which the template calls with the object |
| what the object is (`getType`) | a field the object carries |

an object is data (its engine object, its clip, the state the source reads back); nothing lives on
it as a method. where the source's engine answers a call differently from this one, the answer
belongs to the object's kind, not the call: a fighter's walk and gravity act on the speeds its
code set, seen around the code the runtime runs (`speedsSet`), so `setXSpeed` stays one call.

a package that defines a member with a listed name keeps it: the call asks the object for its own
first and makes the listed call otherwise. a listed member handed around as a value becomes one
function per object and name (`PeptideRuntime.bound`), so a listener added with it is removed with
it. an object's declared stats sit on its exports (`__declared`), so any script can read another
object's without asking that object's script. a call content builds the name of as it runs (`o["get" + kind + "Stat"]`)
is answered from `DATA.api`: the same written calls, as functions of the object, for the names the
package's own strings can form (one it writes whole, or one that starts with one of them and ends
with another). a shared effect the code names (`global_dust_cloud`) plays this engine's
global one (`global_vfx_map` in `mappings/commands.jsonc`, written into `DATA.globalVfx`), and the
package's own copy of its clip is not shipped.

## the converted project's structure

Fraymakers gives each engine object exactly one script (`scriptId` in the manifest), so the
emitter assembles that script from marked sections, always in the same order:

```haxe
// ---- runtime: runtime/core/as3.hx (copies of runtime/*.hx: edit those, not these)
...
// ---- class sandbag_fla.Entrance_37 extends MovieClip
var Entrance_37 = { name: "sandbag_fla.Entrance_37" };
Entrance_37.build = function(me) {
	MovieClip.build(me);
	me.self = null;
	me.frame9 = function() { Effects.shake(4); ... };
};
Entrance_37.init = function(me) { MovieClip.init(me); me.addFrameScript([0, me.frame1, 8, me.frame9, ...]); };
Entrance_37.create = function() { var me = {}; Entrance_37.build(me); Entrance_37.init(me); return me; };
// ---- data: sandbag (generated from the package)
DATA.timelines.set(311, { frames: [...], labels: [...], bind: ... });
DATA.pieces = [ { anim: "intro", label: "entrance", sym: 311, from: 1, frames: 45, ... }, ... ];
DATA.object = { kind: "fighter", cls: SandbagExt, clip: 491, stats: ... };
DATA.pkg = { attacks: ..., projectiles: ..., items: ..., contents: ... };
// ---- runtime: runtime/entry/fighter.hx
function initialize() { PeptideRuntime.start(self, DATA); ... }
function update() { PeptideRuntime.update(); }
```

- **class sections** keep the source's names, fields and methods. `checkLightningStop` stays
  `checkLightningStop` on the instance that owns it; no suffixes, no fighter-wide copies.
- **instances** are made when the source makes them (a clip when its animation is entered) and
  dropped when the source drops them, so there are no reset blocks.
- **frame code is on the keyframes.** each animation gets a "Source frames" layer: the first
  engine frame of each source frame with a script carries that script, written in place, and
  the engine runs it as it enters the frame. a frame stays its clip's method, with a note on its
  keyframe, when the runtime runs it itself: an animation's first frame (the entry), a frame a
  jump names outright (a jump runs the frame it lands on at once, from the code that made it), a
  frame of a clip an effect also plays, and a script anything else calls by name.
- **the data section** holds each clip's timeline, the pieces (which engine animation shows which
  frames of which clip, and the root label it is under), the object's declaration and the
  package's tables, as literals.
- **the engine plays the animations.** the clip an animation shows follows the engine's frame.
  a source frame is two engine frames: its script runs as the first is entered. nothing counts
  frames of its own: the source's timers are the engine's at twice the frames (frames an object
  stands frozen by a hit still do not count toward its timers). the code the source runs every frame (the object's update, its enter-frame
  listeners) runs whole, as written, on every second engine frame (a modulo on the object's own
  frame count), and so do the clips the engine does not play. the runtime's own rules still run
  every engine frame, since they read the engine's controls and physics. an animation the engine starts is entered at
  the next update, once the engine has entered its state. a jump by the source's code moves the
  engine to the frame that shows where it landed once the code is done (whatever ran the code: an
  update, a timer, an event), and a jump into a frame
  whose script is on its keyframe runs it there. an animation that runs out is the engine's to
  act on for a fighter; a projectile goes on to the animation that shows its clip's next frame.
- **native at conversion.** what the source's engine does from an attack's declared stats is
  settled when converting, as the engine's own stats or a few lines in that move, never a rule the
  runtime reads every frame:
  - the declared stats go into the move's AnimationStats (`runtime/attack_stats.jsonc` maps them:
    slowdowns to friction, `air_ease` to `terminalVelocity`, the floor's edge, what may cancel it),
    a literal write its code makes becomes `updateAnimationStats` on the spot, and a read of one
    is the engine's field read back.
  - a jump by label from a clip's own code is the engine's: `playFrameLabel` inside the animation
    playing, `playAnimation` for a label that starts one of the clip's animations (every entity,
    a projectile's too, carries the source's labels, and an animation's first frame is named for
    it only when no label inside it has that name; both run the frame they land on at once, as
    the source's does).
  - a floor-touch listener whose only work is to take the move to its landing section is the
    engine's landing: `landType: TOUCH` with the animation that label starts (it keeps the attack
    going, from the landing section's first frame); one that lands in the landing state is
    `landType: NORMAL`. an aerial's landing section is its own `<move>_land` animation, and the
    stats the listener wrote (the attack's and its boxes') are that animation's own
    AnimationStats and HitboxStats.
  - a declared `cancel` is a check on the move's first frame: holding the button it was made with,
    once its clip is waiting, sends it to `finish` (kirby's stone).
  - steering is the move's `allowMovement` (`allowControl`), with no ground acceleration where
    `allowControlGround` turns it off on the floor; a value the code works out as it plays is the
    runtime's.
  - an autolink the clip's code starts toward a point it names (written out, or a field of its own
    it set to one) is the engine's: the animation's hitboxes state an autolinked box's fields
    (`runtime/autolink.jsonc`: the autolink angle, and the launch all the engine's pull) from the
    frame it starts, and their own again where it stops, and an `autolinkpoint<n>` layer per
    hitbox follows each point it names, scaled out by the fighter's size since the engine reads
    it unscaled. a rule reshaping the pull, or a cap on its power, has no engine setting and is
    reported.
  - what the source's engine does in its own states is each state's animation's
    (`runtime/engine_animations.jsonc`): stats (an air dodge steers, a revival has no gravity),
    code its first frame runs (a jump under the held back direction plays the backflip, a
    backflip falls once it tops out, a revival waits for a control or five seconds), and points
    (every ledge action carries `ledgeholdpoint0` at the origin, where the source hangs a fighter).
  - a box stat the clip's frame writes as an animation starts (a combo's later hits restating
    their boxes, written straight or through a field set the same frame) is that animation's own
    HitboxStats. a reversible box (the source's default) sends its victim away from the attacker,
    so a backward angle is written mirrored; one the source states is not reversible says so.
  - intangibility and invincibility the code switches are the animation's `bodyStatus`, and end
    with it, as the template's dodges do.
- **effects are clips.** an attached effect is the source's clip for it, running its own frame
  scripts; the engine's effect follows the clip's frame and what the code sets on it (position,
  visibility, alpha, rotation, scale). it plays through `loop` times (once by default) unless its
  clip stops itself, and taken off its parent it is gone.
- **events.** a source listener on an event the engine raises (landing, a hit, a grab;
  `runtime/events.jsonc`) is the engine's own listener, named at conversion when the code names the
  event: `addEventListener(<engine event>, handler)` on the object's handle, where the handler
  (one per listener, so one added is taken off) hands it a record in the source's terms
  (`receiver`, `opponent`, `attackBoxData` converted back). a listener on a fighter ends with the
  state it was added in unless it is added to persist, as the engine's own do; on anything else it
  lasts until it is taken off. a state change is the engine's event told with the source's states
  either side. the rest the runtime raises where the source's engine would (an attack's end, a
  projectile's end, the end of a frame, a projectile touching its owner).
- **speeds.** the source slows a speed and then moves on it; this engine moves and then slows it.
  a fighter's x speed written, or carried onto or off the floor, is held a step and a half of the
  slowdown where it is short of the source's (the floor's friction or the air's, the attack's own
  where it states one), and read back with it; one an attack keeps a share of is held at
  2m/(1+m). a y speed is held a step and a half of gravity ahead in the air. a timer's code runs
  after the update that holds an attack's fall under its air ease, so the ease is held again once
  a timer is done.
- **other objects.** a call on another converted object's source object runs in that object's
  own script. a view of an engine object no converted script runs (another fighter) only uses the
  engine object, so its answers are that object's own. an object whose script has ended stops
  updating, and its engine object stops naming it as its source, since the engine can reuse that
  engine object for a new instance (a fired shot, a made item) that must not be handed the old one.
  a held item goes where its holder goes, so it never dies at the bounds by itself.
- **only the input's own code.** the converter translates the package it is given and nothing
  else: an item a fighter makes from another package (sandbag's bob-omb) is reported as a gap,
  never compiled from the game's own files.
- **making an object.** frame scripts entered while an object is made wait for its first update,
  as a new clip's first frame does in the source; one sent to another frame before then (a
  projectile's maker asking a label) runs that frame's instead.
- **runtime sections** are verbatim copies of the modules this object uses, each naming its file.

a human porting by hand reads a class section and sees the source's class. a converter developer
edits a runtime file or an api entry and reconverts.

### state and online play

an online match rolls back: every frame the engine saves its state, and when a remote input
arrives late it puts the saved state back some frames and plays them again. a script's plain
variables are not part of that state; the engine's state containers are (`self.makeInt` and the
rest). the engine makes them only at the script's top level, before its `initialize`, so the
script makes all of them first thing, ahead of its classes (the tick and a pool). so:

- **content's fields are containers.** a field a class of the package declares is written as a
  container of the engine's (`me.__c_hp.get()`, `.set(v)`), taken from the pool as its object is
  made. a class's statics stay plain values (constants, mostly). once an object is gone from what the source shows its
  containers go back to the pool and it keeps its values as plain ones. which containers are taken
  is itself kept in containers, so a rollback takes back what was taken since. past the pool a field
  is a plain value, and a conversion notes it as a gap.
- **a container holds a value, not what was done to it.** a rollback puts back the object a
  container held, not the fields written into it since, so an array a field holds is changed as a
  copy put back in its place (`push`, `splice`, an element written).
- **the runtime's own state** (playheads, children, listeners, the fields it keeps on an object,
  which listeners it has wired to the engine) is kept one record a frame (`runtime/core/rollback.hx`),
  filed under the frame a tick container says the match is on. a rollback takes the tick back, and
  the record of that frame is put back before the update runs. which of an object's fields the
  runtime writes, and which of them hold lists, is read off its own files at conversion. records
  are kept only in a match that can roll back (its netcode says so): an offline match pays
  nothing for them.
- `netcode = 3` in `match_settings.conf` runs a local match that rolls back as an online one does.
  it replays frames whose inputs it already had, so every replayed frame must come out as it
  first did wherever the engine's own state does; the fighter's script checks each one
  (`tools/tests/rollback_parity.py`).

## compiler: AS3 in, Haxe out, nothing about the game

one lowering rule per language construct. when the compiler meets a call on an API object, it
looks the member up in the api table and emits what the entry says.

| AS3 semantics | lowering |
|---|---|
| truthiness (`0`, `""`, `null`, `NaN` are false) | plain test when the static type says Boolean, else the runtime's truthy helper |
| a comparison with `null`/`undefined` is false | plain operator when both sides are typed Number, else the runtime's compare helper |
| `int`/`uint` fields keep the whole part | truncation on write to a slot typed int |
| `a \|\| b` and `a && b` return an operand | plain when both are Boolean, else the runtime's or/and |
| string `+` coerces | Std.string on the non-string side |
| `is` / `as`, `int()`, `Number()` casts | runtime cast helpers |
| dynamic keys on an Object (`o[name]`) | the runtime's map helpers |
| classes, instances, fields, statics, getters | constructor functions building instance objects |
| closures and activation scopes | Haxe closures; activation slots become locals |
| `for in` / `for each`, `switch`, `try` | the Haxe forms |

static types matter for speed as much as correctness: the bytecode carries slot, parameter and
return types, and a typed expression gets a plain operator instead of a helper call.

the decompiler is pure: no renames, no unit conversion, no facing rules.

## the runtime's files are the api list

a receiver in the source is mostly untyped (`curLightBolt: *`), so which API object a call is
made on is usually not known when converting. so API objects are runtime objects that carry the
source's own member names: the compiler writes `x.isDisposed()` as the source does, and the
runtime answers it. the runtime's files are the one list of what exists: the compiler reads their
classes (`// class NAME extends BASE`, members as `me.NAME = ...`) and globals, resolves a name
the content inherits from an API class to that member, and reports a name nothing defines.

a call through a runtime object costs one hop (~8 us, see costs). if a hot path needs it, a member
with no behaviour can later be written inline where the receiver's class is known; that is an
optimisation on top, not the design.

### what stays out of Rust

enforced by `tests/conventions.rs`:

- no string in the model's or compiler's Rust defines a named Haxe function: the compiler writes
  the source's code, and every behaviour of the runtime's lives in `runtime/**/*.hx`
- no per-kind emitter: one emit path, checked by a test that every object kind goes through it
- no content ids or content-kind keyword lists anywhere (the existing rule)

## the language layer

measured once, in the engine's own interpreter (`tools/tests/lang_suite.py` runs the cases):

| construct | this interpreter | the compiler writes |
|---|---|---|
| a test, `!x`, `a && b` on anything but a Boolean | only a literal `true` passes; `&&` answers a Boolean | `AS3.truthy`, and `&&`/`\|\|` as a value keep the operand they settle on |
| `<` `<=` `>` `>=` with a null or a NaN | not AS3's answer | `AS3.lt` and the rest, unless both sides are known ints |
| `>>>` | signed | `AS3.ushr` |
| `o[name]`, `Reflect` | no field by a run-time name; no `Reflect` or `Type` | `AS3.at`/`AS3.put`, keys in the object's own map |
| `a[-1]` | stops the script | `AS3.at` |
| a closure made in a loop | captures a per-turn variable | every variable declared once at the function's top, as AS3 scopes it |
| a call with fewer arguments | an error | every parameter optional |
| `catch (e:Type)` | hangs | `catch (e:Dynamic)` and a type test |
| `Math.floor` above 2^31 | overflows | truncation by arithmetic in `AS3.int` |
| `Math.random`, `Array` as a value | missing | `AS3.random`, `AS3.array` |
| an object literal read or enumerated by a run-time key | no way to name a field at run time | the literal carries its class's shape for its keys (`X.__shapes`: the names, a reader and a writer), which `AS3.at`, `AS3.put` and `for in` use |
| `n.toString(2)`, `n.toFixed(d)`, `s.slice(-2)`, `s.replace(a, b)`, `a.reverse()` as a value, `a.sort(16)` | missing on this engine's numbers and strings, or answered differently | `AS3.toString` and the rest, unless the receiver's class is the package's own and defines the method |
| `this` in a static method | | the class |
| `NaN`, `undefined`, `Number.MAX_VALUE` and the other numeric constants by name | not names here | their values |

closures capture by reference, and `switch`, `do`/`while`, `try`, assignment in an expression,
`i++`/`i--` and the array methods behave as AS3's do.

## engine facts the design rests on

**one Fraymakers frame**, measured on a converted fighter:

1. the script's update
2. timers due this frame
3. end of frame: hit detection, hit events, then the engine's own state and animation changes
4. the animation advances one frame and the entered frame's keyframe script runs
5. draw

setting frame F during update or a timer shows F+1. an engine-driven animation change shows up at
the next update, and a driver that steers only its own animation never fights it.

**the source's schedule**, as the current converter already models it and the runtime starts
from: a timer of N frames runs before the update of its Nth frame, a frame's script runs before
the frame is drawn, a chain of label jumps inside one frame runs within that frame, a label asked
of another object lands on its next source frame, and a new clip's first frame runs once its
maker's script is done. a state's first frame shows the frame its entry put the clip on (or the
one that frame's script jumped to), and the clip moves on from the next frame; a timer started by
that entry first comes due on the next frame too.

**calls between objects.** a function an object publishes runs in that object's own scope when
another fighter, a projectile, the stage or the console calls it: its own variables, its own
functions, its own `self`, synchronously. measured on fighters, a projectile and a stage object.
once the object's script has ended, its functions can no longer reach its script's names. an
object one script makes and calls at once (a stage making its blocks) is a stand-in until its own
script starts, and the calls wait for it there; after that they go to the object directly
(`Bodies.direct`), or to its next update where an engine runs them without the script's helpers.

**costs** in Fraymakers' script interpreter, per operation: an arithmetic step ~2 us, an engine
API call ~3.4 us, a call to a script function ~8 us, each extra wrapper hop ~8 us more. the frame
is 16.7 ms. so members without behaviour are expanded inline, the runtime's per-frame path stays
to a handful of calls per object, and typed arithmetic compiles to plain operators.

**the decompiler** is the source model's own (`src/model/`), built on the swf crate's
instruction decoder. every class of the 47 corpus packages reads with nothing unreadable, in a
few seconds; `tests/source_model.rs` holds that bar.

## verification

- **language suite.** trees for each construct, compiled, run in Fraymakers' own interpreter in one
  boot, expected values from AS3's rules.
- **api suite.** each runtime member driven on both engines through the shared command vocabulary.
- **roster trace diff.** the same inputs on both engines for every fighter's moves, per-frame state
  recorded, differences grouped by kind. a change is judged by how many it removes across the
  roster.
- **structure checks.** no code in entity json; every source class present as a class section;
  no suffixed names; the conventions tests above.
- the goldens, the tally, the soak and the frame-data checks stay.

## the move, in order

each step has an exit test. the current converter keeps shipping until the new one beats it on the
roster diff.

| step | work | done when |
|---|---|---|
| 1 | source model; decompiler pure and complete | every content class of every corpus package is in the model with its bindings; no unreadable nodes in content classes |
| 2 | compiler and language suite | every content method compiles and parses; suite green in the engine |
| 3 | runtime core, one emit path, roster trace diff | sandbag, then mario, pass every gate with code-free entities and class sections |
| 4 | projectiles and items through the same path | every fighter's projectiles pass proj parity at least as well as today |
| 5 | coverage by corpus frequency | all fighters convert with no member missing an entry; soak clean; roster diff better than today |
| 6 | stage objects through the same path | the ten-stage sample passes its gates |
| 7 | delete the old emitters, text passes and helper strings | only the new path is left |

## what carries over

the fixes list in [`STATUS.md`](STATUS.md) is the requirements list. each measured rule there
becomes a runtime rule or a suite case: timer phase, label asks, speeds in source units, int
fields, null comparisons, one timer per callback, repeat 0 meaning forever, spawn offsets adding, a
first frame at the end of its maker's script, listener lifetimes, every projectile lifecycle rule.
the extraction side, the frame-data doubling, the entity and hitbox generation, the stats tables,
the packer and the whole harness stay as they are.
