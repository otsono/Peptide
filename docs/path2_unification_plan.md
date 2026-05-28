# Path 1 → Path 2 architectural switch

## Background

The converter currently has two parallel sources of character stats inside every
`.ssf`:

* **Path 1 — INLINE.** Each character's per-character extension class
  `<X>Ext` (e.g. `MarioExt`) exposes `getOwnStats()`, `getAttackStats()`,
  `getProjectileStats()`, `getItemStats()` as four separate methods. Each returns
  a flat literal object.
* **Path 2 — BUNDLE.** A top-level `Main` class exposes one method
  `Main::get<Name>()` per character that returns a wrapper literal
  `{ cData, aData, pData, iData }`. The four sub-objects are byte-identical
  literals to the four `<X>Ext::get*Stats` returns.

We confirmed byte-for-byte equivalence across 10 characters spanning the
complexity range (sandbag, sandbag → naruto). Probe results in
`/tmp/compare/`; corpus-wide bundle scan in `/tmp/char_sources.tsv`.

We also confirmed that **path 2 is strictly more complete than path 1**: every
of the 44 normal characters has both a `<X>Ext` class and a `Main::get<X>`
method, *and* Sheik exists only via `Main::getSheik` — she has no `SheikExt`,
because she shares `ZeldaExt` with Zelda. Dropping path 1 picks up Sheik for
free.

## Goals

1. Drop path 1 (INLINE stat extraction) entirely.
2. Detect characters by enumerating every `get*` instance method on the
   `Main` class — corpus audit confirms 1:1 with character bundles, no
   filtering needed.
3. Continue using each character's `<X>Ext` (where one exists) for behavior
   code — `frameXXX` handlers, helper methods, instance var declarations and
   their constructor initializers. None of that lives in the bundle.
4. Keep the public `CharacterData` shape unchanged so downstream generators
   (`haxe_gen`, sprite/palette pipelines, etc.) don't move.

## 1. Detection — `detect_char_names` rewrite

### Current behaviour
* [src/main.rs:191-217](src/main.rs:191): `CHARACTER_MARKER_METHODS` constant
  plus `is_character_ext_class()` predicate — class name must end in `Ext`,
  prefix must be ≥ 2 alphabetic chars, instance methods must include at least
  one of `getOwnStats` / `getAttackStats` / `getProjectileStats`.
* [src/main.rs:219-269](src/main.rs:219): walks every ABC block's classes,
  picks `is_character_ext_class` matches, lowercases the prefix, dedupes,
  reconciles against filename for truncated cases (`CaptainExt` →
  `captainfalcon`).

### New behaviour

```text
for each abc_block in swf.abc_blocks:
    abc = parse(abc_block) [skip on parse error]
    main = find class named "Main", skip block if absent
    for each instance method on main whose name starts with "get":
        push (method_name, method_idx, abc_block_idx) into candidate list
return [derive_id(method_name) for each candidate]
```

That's it. No bundle-shape check, no `normalStats_id` reading, no collision
rule. A corpus-wide audit (`src/bin/audit_main_gets.rs`) confirms **every
single `Main::get*` instance method across all 45 SSFs is a character bundle
— zero exceptions, zero non-character `get*` helpers, zero static `get*`
methods**. The `Main` class exists solely to expose the character roster.

### Why this is safe

* 48 `get*` methods enumerated across 44 SSFs that have a `Main` class.
* misc.ssf is the only SSF without a `Main` class — handled by the
  "no-bundles fallback" below.
* Every observed `get*` body matches the bundle shape (pushes the strings
  `cData`, `aData`, `pData`, `iData`, `normalStats_id`).
* Zero `Main` static `get*` methods in the corpus. `Main::getCostumeData` is
  a misc.ssf concept and lives on a different class entirely
  ([src/main.rs:80-90](src/main.rs:80)).
* The derived ids — `bandanadee`, `captainfalcon`, `gigabowser`, `wario_man`,
  `sheik`, etc. — are all distinct and match the existing converter's output
  directory names for the 44 normal characters, with three new clean
  additions for the sub-characters.

### Derived-id rule

```
derive_id(method_name):
    name = method_name.strip_prefix("get").unwrap()
    return name.to_lowercase()
```

Lowercase. That's the whole rule. Explicit `_` characters in the source
method name are preserved (`getWario_Man` → `wario_man`); camelCase humps
are NOT split (`getCaptainFalcon` → `captainfalcon`, matching the existing
filename-derived id, not `captain_falcon`).

Every observed method maps cleanly:

| method               | derived id        |
|----------------------|-------------------|
| `getMario`           | `mario`           |
| `getBandanaDee`      | `bandanadee`      |
| `getCaptainFalcon`   | `captainfalcon`   |
| `getChibiRobo`       | `chibirobo`       |
| `getDonkeyKong`      | `donkeykong`      |
| `getgameandwatch`    | `gameandwatch`    |
| `getMegaMan`         | `megaman`         |
| `getMetaKnight`      | `metaknight`      |
| `getPacMan`          | `pacman`          |
| `getBlackMage`       | `blackmage`       |
| `getGigaBowser`      | `gigabowser`      |
| `getWario_Man`       | `wario_man`       |
| `getSheik`           | `sheik`           |
| ... (33 more, all trivial) | |

No collisions across the 48 methods. The lowercase-strip approach preserves
the existing converter's output naming convention exactly while picking up
the three sub-characters as first-class outputs.

### Iteration

One pass per ABC block. We look at `main.instance_methods` only. We do NOT
look at:

* `main.class_methods` (static methods) — none observed in the corpus.
* `script.traits` — bundles only live on `Main` instance methods.
* Any other class — there is no other class that emits a bundle anywhere in
  the corpus.

This makes detection O(n_methods_on_Main) — typically 1–2 methods per file.

### Detection output shape

The candidate list is a `Vec<BundleRef>`:

```rust
struct BundleRef {
    derived_id:    String,   // e.g. "bowser", "gigabowser", "wario_man"
    method_name:   String,   // e.g. "getBowser", "getGigaBowser"
    method_idx:    u32,      // body lookup key
    abc_block_idx: usize,    // which ABC owns it
}
```

`detect_char_names` returns the `derived_id`s in iteration order.

Luffy Gear 2nd, Kaioken Goku, and similar animation-only sub-characters have
*no* `Main::get<X>` method at all, so they never enter the candidate list
— exactly what we want.

### Zero-`Main` SWFs

If no `Main` class is found in any ABC block (the misc.ssf case), fall back
to the filename stem (same behaviour as the current "no `*Ext` class"
fallback at [src/main.rs:264-266](src/main.rs:264)). misc.ssf is handled by
a separate cosume-extraction path anyway ([src/main.rs:80](src/main.rs:80)),
so this fallback is only exercised when someone hand-feeds a non-character
SWF to the converter.

## 1.5. Sub-characters and transformations

In the simplified detection model, sub-characters are just additional `get*`
methods on `Main`. The user-facing distinction between "the parent character"
and "an alternate form" disappears at the detection layer — they're all
characters, indexed by their derived id.

A corpus-wide scan (`src/bin/audit_main_gets.rs`) shows exactly four SSFs
where `Main` has more than one `get*` method:

| SSF       | `get*` methods                              | derived ids                  |
|-----------|---------------------------------------------|------------------------------|
| zelda     | `getZelda`, `getSheik`                      | `zelda`, `sheik`             |
| bowser    | `getBowser`, `getGigaBowser`                | `bowser`, `gigabowser`       |
| wario     | `getWario`, `getWario_Man`                  | `wario`, `wario_man`         |

(zelda's two are peer characters; bowser's and wario's second method is a
Final Smash transformation form.) The other 41 SSFs each expose exactly one
`get*` method. No `Bowser_Jr`, no alternate skins, no other surprises — the
corpus is closed on this question.

The detection code treats all six of those derived ids identically. The
*content-author-facing distinction* between "Sheik is a peer of Zelda" and
"Giga Bowser is a transformation of Bowser" lives downstream — see §1.6.

### What actually differs (parent vs transformation)

These are *not* mostly-numeric overrides — they are largely independent stat
blocks. Probe output from `find_transformations` + `diff_inline_vs_bundle` +
`/tmp/diff_trans.py`:

**Giga Bowser vs Bowser**

* `cData`: parent has 21 keys, Giga has 9. 13 fields only on parent
  (`roll_speed`, `tetherGrab`, `finalSmashCutin`, …), 1 only on Giga
  (`launchResistance`), 1 differing value (`hurtFrames` 5→1). Giga doesn't
  roll, isn't tether-grabbable, has no Final-Smash-of-its-own cutin.
* `aData`: completely disjoint attack sets. Parent's are item-based
  (`item_screw`, `item_firedash`, `item_heavyshoot`, `item_bubblebounce`,
  `outro`). Giga has `special`, `taunt`, `item_dash/jab/smash/tilt`.
* `pData`: parent → `fireBreath{,Blue,Purple}`. Giga → `gigaFireBreath{,Blue,Purple}`.
  Same concept, different linkage IDs (so different sprites/scales).
* `iData`: both null.

**Wario Man vs Wario**

* `cData`: 19 vs 19 keys. 5 only on parent
  (`decel_rate_air`, `finalSmashCutin`, `fs_time_limit`, `holdJump`,
  `tech_roll_delay`), 5 only on trans (`canHoldItems`, `canShield`,
  `canUseItems`, `crouchWalkSpeed`, `launchResistance`), 4 differing values
  (`grabDamage` 1→5, `max_projectile` 2→3, etc.). Wario Man is faster,
  hits harder, can't hold/use items, can't shield.
* `aData`: parent has 3 item attacks; Wario Man has 14 *fundamentally
  different* attack entries — `b_forward`, `b_up`, `b_down` + their air
  variants, all four throws, `crouch_attack`, `getup_attack`, `ledge_attack`,
  `special`, `taunt`. A full alternate moveset.
* `pData`: parent → `wario_afterimage`. Wario Man → `warioMan_afterimage`.
* `iData`: both null.

Treating these as "patch the parent" is the wrong mental model. They are
co-resident stat blocks that *share an identity* with the parent (for HUD /
announcer / SFX routing inside SSF2) but carry independent data.

### Fraymakers does not expose transformation hooks

We checked: there is no `fraymakers-api.d.ts` or equivalent reference file
present in this workspace, no `FinalSmash` / `Transformation` / alternate-
stats API surface anywhere in the converter's mappings or generators, and
`fraytips.md` describes only generic FM scripting patterns (timers, frame
scripts, projectiles via community plugin). The community might offer such a
mechanism externally, but from the converter's perspective the FM character
data model is *one* monolithic `CharacterStats.hx` per character. No second
stat block, no mode switch.

This makes Option C (silently drop) genuinely lossy — we'd be discarding
Giga Bowser's entire moveset.

### Emission

Each bundle becomes its own output character package, indexed by its derived
id. No special-casing in the extraction code path — the same Stage A bundle
split + Stage B Ext-class behavior pipeline runs for `bowser`, `gigabowser`,
`sheik`, etc.

* `Main::getGigaBowser` → `characters/gigabowser/`
* `Main::getWario_Man`  → `characters/wario_man/`
* `Main::getSheik`      → `characters/sheik/`

## 1.6. Conversion-log metadata for sub-characters

While detection is shape-blind, the *output* still wants to tell the content
author "this is a Final Smash form of Bowser, FM has no native
transformation hooks, you'll need to wire it up in Script.hx". That note
lives in `conversion_stats.json`, which is already surfaced in the SwiftUI
conversion-log popup.

Post-extraction, each bundle's `cData.normalStats_id` is read from the
captured object literal and copied into the conversion log:

```json
{
  "name": "gigabowser",
  "ssf2_source": {
    "method":          "Main::getGigaBowser",
    "normalStats_id":  "bowser"
  }
}
```

The author can then see, at a glance, which characters share an SSF2
identity (a transformation pair) versus which are peers (zelda + sheik have
different `normalStats_id` values; bowser + gigabowser share `bowser`).

**Important:** the `normalStats_id` field is read for *display/metadata
purposes only*. It does NOT influence detection, id derivation, or directory
naming. If a future SSF2 build has a character whose `normalStats_id`
diverges from its method name, both the directory and the displayed metadata
reflect what's in the file — no surprises.

### Ext-class lookup for sub-characters

Giga Bowser and Wario Man have no `GigaBowserExt` / `Wario_ManExt` classes —
their behavior code (when there is any) lives on the parent's `BowserExt` /
`WarioExt`. The §3 Ext lookup handles this with the cross-character fallback:
exact-name match first (`GigabowserExt` — won't exist), then any `*Ext` whose
lowercased prefix matches *another* derived id in the same SWF, then the
sole-Ext fallback. For bowser.ssf, the sole Ext is `BowserExt`; for wario.ssf
it's `WarioExt`. The transformation's `Script.hx` ends up identical to the
parent's — same known limitation as Sheik (§3), accepted for v1 for the same
reasons.

### MovieClip lookup for sub-characters

The "scan the main character class" pass keys off `char_name`. For
`gigabowser` we look for a MovieClip named `gigabowser` (case-insensitive,
[src/abc_parser.rs:827](src/abc_parser.rs:827)). If absent — Giga and Wario
Man may animate inside the parent's MovieClip with engine-level mode flags —
we degrade gracefully: empty `animations`, empty `frame_scripts`, the stats
and attacks are still emitted. The conversion log notes the missing
MovieClip so the author knows what's not there.

### Animation-only transformations stay invisible

Luffy Gear 2nd, Kaioken Goku, Ichigo (a dev-leftover in donkeykong.ssf),
and similar have no `Main::get<X>` bundle at all. They are NOT in the
candidate list and therefore NOT emitted as separate characters — correct
behavior, since they have no distinct stat data to emit.

### Sheik specifically

zelda.ssf has two bundle methods on `Main` — `getZelda` (cData.normalStats_id
= `zelda`) and `getSheik` (cData.normalStats_id = `sheik`). They are *not*
transformations — they have distinct `normal_stats_id` values, so the §1.5
collision rule never fires. They flow through `extract_character`
independently and land as `characters/zelda/` and `characters/sheik/`.

### Truncated-name reconciliation

The current filename-vs-class reconciliation (handles `CaptainExt` →
`captainfalcon`) is no longer needed: `normalStats_id` is the canonical name
SSF2 itself wrote down. We log the filename if it disagrees with the
discovered names (useful for triage of bugs) but we trust `normalStats_id`.

### Zero-bundle SWFs

Falls back to filename stem (same as the current
"no `*Ext` class" fallback at [src/main.rs:264-266](src/main.rs:264)).
Behaviour is unchanged for misc.ssf and any other non-character SWFs the user
might pass in.

## 2. Extraction — `extract_character` rewrite

### Current behaviour
* [src/abc_parser.rs:660-948](src/abc_parser.rs:660). Finds the `<X>Ext` class
  via name match + prefix match + sole-Ext fallback. For each Ext method:
  * `getOwnStats`  → `extract_ssf2_stats` → `Option<CharStats>`
  * `getAttackStats` → `extract_attack_objects` → `BTreeMap<String, AttackData>`
  * `getProjectileStats` → `extract_projectile_objects` → `BTreeMap<String, ProjectileData>`
  * `frame*` → animation extraction + decompilation
  * Anything else → Script.hx decompilation
  * Slot/Const traits → `ext_vars`; iinit → `ext_var_inits`.

  Then sweeps the main MovieClip class and per-character `_fla.*` sub-classes
  for additional frame handlers and helper methods.

### New behaviour

Signature unchanged:
```rust
pub fn extract_character(abc: &AbcFile, char_name: &str) -> Result<ExtractedCharacter>;
```

Split into two stages:

**Stage A — stats from the bundle.** Locate the `Main::get<X>` method whose
body is a bundle with `cData.normalStats_id == char_name`. Run a new
`BundleSplitVisitor` over its body that captures the four sub-objects as
`StackVal::Object`s. From those:

| sub-object | feed to                                        | producer field on `ExtractedCharacter` |
|------------|------------------------------------------------|-----------------------------------------|
| cData      | new `cdata_to_char_stats()`                    | `stats: Option<CharStats>`              |
| aData      | new `adata_to_attacks()`                       | `attacks: BTreeMap<String, AttackData>` |
| pData      | new `pdata_to_projectiles()`                   | `projectiles: BTreeMap<String, ProjectileData>` |
| iData      | new `idata_to_items()` (drop initially — unused) | (not stored) |

Three of these (`cdata`, `adata`, `pdata`) are *exactly* the data shape the
existing `extract_ssf2_stats` / `extract_attack_objects` /
`extract_projectile_objects` already consume — they each scan a method body
for newobject literals and extract their flat-scalar / hitbox content. The
straightforward implementation is:

* Keep `scan_method` and its visitors as-is.
* Add a `BundleSplitVisitor` that snapshots the cData / aData / pData / iData
  sub-objects as raw `BTreeMap<String, StackVal>` values.
* Implement the three `*_to_*` helpers as direct walks over those captured
  sub-objects, reusing the existing flatten/extract logic shared with the
  attack/projectile visitors.

Migration shortcut option: because we proved the four `<X>Ext::get*Stats`
bodies and the `Main::get<X>` body contain byte-identical literals, we could
simply run the *existing* `extract_ssf2_stats` / `extract_attack_objects` /
`extract_projectile_objects` on the `Main::get<X>` body. Each one is a
bytecode-pattern scan that doesn't care which method body it's called on. This
is a low-risk shim for step A of the migration (see §5).

`iData` is captured but unused at first. The current pipeline never reads
item stats anywhere, and `getItemStats` is explicitly excluded everywhere it
appears today (e.g. [src/abc_parser.rs:801](src/abc_parser.rs:801)). We keep
`iData` extraction off the critical path until someone needs it; flagged as
follow-up.

**Stage B — behavior from the Ext class.** Unchanged in structure, only the
Ext-class lookup changes:

* Try `format!("{}Ext", capitalize(char_name))`.
* If not found, fall back to: any `*Ext` class whose lowercased prefix matches
  any *other* discovered character name in the same SWF. For Sheik this lands
  on `ZeldaExt` (the only Ext class in zelda.ssf, and Zelda's char name is in
  the discovered list).
* If still nothing, fall back to the single-Ext-class case (current behaviour
  at [src/abc_parser.rs:698](src/abc_parser.rs:698)).
* If still nothing, skip Stage B and return an `ExtractedCharacter` with empty
  behavior fields — stats-only is a valid degenerate case.

The Ext-class methods feed `ext_vars`, `ext_var_inits`, `frame_scripts`, and
`ext_methods` exactly as today.

The "scan the main MovieClip class for frame methods" pass
([src/abc_parser.rs:820-879](src/abc_parser.rs:820)) is unchanged: it finds the
class by name match against `char_name`, which works for Sheik (her MovieClip
is named `sheik` and has 89 instance methods, 88 frame*; confirmed via
`dump_main_class` probe). The `_fla.*` sweep
([src/abc_parser.rs:881-921](src/abc_parser.rs:881)) is unchanged.

### Output directory

[src/main.rs:326](src/main.rs:326) builds `output.join(char_name)`. After the
switch, `char_name` flows in from `detect_char_names` which now returns the
derived ids described in §1.5 (the canonical `normal_stats_id` for normal
characters; a method-name-derived id for transformations). So directories
become `characters/zelda/`, `characters/sheik/`, `characters/giga_bowser/`,
`characters/wario_man/`. The CLI `--name` override still short-circuits
detection if the user wants to force a specific extraction — and now it can
target a transformation by its derived id (`--name giga_bowser`).

## 3. `*Ext` class discovery becomes derivative

After the switch, `*Ext` no longer *defines* who is a character — `Main` bundles
do. But `*Ext` is still the source of behavior code for the characters who
have one.

### Discovery strategy in the new world

Given a character name from detection, look up the Ext class in this order:

1. **Exact-name match:** `format!("{}Ext", capitalize(name))`. Handles all 44
   normal characters.
2. **Cross-character fallback:** scan all `*Ext` classes in the ABC; if exactly
   one is shared by another character also discovered in this SWF (i.e. the
   `*Ext` whose lowercase prefix matches a *peer* character), use it.
   This is the Sheik case: in zelda.ssf the only Ext is `ZeldaExt`, and `zelda`
   is a peer character.
3. **Sole-Ext fallback:** if there's exactly one non-base `*Ext` class in the
   file, use it (current behaviour preserved as a safety net).
4. **No Ext:** skip Stage B (stats-only character; no Script.hx output).

Reverse mapping is also useful: from the Ext class's lowercased prefix back to
a character name. For zelda.ssf, `ZeldaExt` prefix lowercased is `zelda`,
which matches one peer (`zelda`). So Sheik's lookup explicitly chooses
`ZeldaExt` because (a) no `SheikExt` exists and (b) `ZeldaExt` is owned by a
peer character.

### Sheik's behavior code — what we know

* `Main::getSheik` exists and is the bundle source. ✓
* There is no `SheikExt` class. ✓ (`dump_main_class` confirms this — only
  `Main::getSheik` carries any sheik-named method.)
* There IS a `sheik` MovieClip class, super = `MovieClip`, with 89 instance
  methods (88 `frame*` + one `stance`). This is her animation timeline and is
  picked up by the existing "scan the main character class" path
  ([src/abc_parser.rs:820-879](src/abc_parser.rs:820)) when called with
  `char_name = "sheik"` — the case-insensitive name match at
  [line 827](src/abc_parser.rs:827) finds it.
* No frame method on `ZeldaExt` has "sheik" in its name; the AS3 behavior
  appears to be genuinely shared between the two characters, with engine-level
  mode switching at runtime.

### Sheik's Script.hx — known caveat for v1

Because Sheik uses `ZeldaExt` as her Ext class, her Script.hx will contain
the same decompiled bodies as Zelda's. This is *correct in the sense that the
AS3 source is genuinely shared*, but the output is duplicative. We accept it
for v1; a follow-up could attempt to identify which branches inside ZeldaExt
methods are Sheik-specific (e.g. by static analysis of `if (this.xframe ==
"sheik_…")` branches) and split them, but that is out of scope.

If we want to defend against accidentally diverging behaviour we could add a
post-extraction note that `ext_methods` for Sheik are identical to Zelda's;
not a blocker.

## 4. Consumer wiring downstream

The `CharacterData` struct ([src/extractor.rs:10-36](src/extractor.rs:10)) is
unchanged — same fields with the same semantics. Consumers don't care which
extraction path produced them. Verified by inspection:

* [src/extractor.rs:120-180](src/extractor.rs:120) merges the `ExtractedCharacter`
  fields into `CharacterData`. The fields it reads — `attacks`, `stats`,
  `ext_methods`, `ext_vars`, `ext_var_inits`, `frame_scripts`, `projectiles`,
  `xframe_map` — all stay populated.
* [src/haxe_gen.rs](src/haxe_gen.rs) `generate_hitbox_stats` / `generate_character_stats`
  / `generate_animation_stats` / `generate_script` read from `CharacterData`,
  not from the ABC directly.
* `sprite_parser`, palette, sound, costume extraction operate on the SWF, not
  the ABC — completely orthogonal to this change.

The one consumer touchpoint worth flagging is
[src/main.rs:96-107](src/main.rs:96): the
`--name` CLI override. It currently expects to find a `<X>Ext` class for that
name. After the switch it should pivot to expecting a `Main::get<X>` method —
trivial code change, but the CLI semantics shift slightly: passing `--name
sheik` now works against zelda.ssf.

`extract_xframe_scale_from_swf` and `extract_xframe_transforms_from_swf` at
[src/main.rs:300-313](src/main.rs:300) take the character name as a lookup
key into the SWF's symbol table. Their behaviour is filename-independent;
they keyed off the *MovieClip* name (`mario`, `sheik`, etc.) all along.

`misc.ssf` handling at [src/main.rs:80](src/main.rs:80) — orthogonal; misc
is processed for costume palettes only, never as a character.

## 5. Migration plan

### Step A — Add path 2 alongside path 1

1. Add `extract_character_bundle()` to `abc_parser.rs` that mirrors the
   signature of `extract_character` but pulls stats / attacks / projectiles
   from `Main::get<X>()` instead of the four `<X>Ext::get*Stats` methods.
   Stage B (Ext class behavior extraction) is shared code.
2. Add a `#[cfg(test)]` golden assertion in `src/extractor.rs` (or a new
   `tests/path1_path2_equivalence.rs`) that for each character in the local
   corpus, both paths produce byte-identical `CharacterData.attacks`,
   `CharacterData.stats`, and `CharacterData.projectile_data`.
3. Run that test against the full corpus locally (44 characters). Expect
   pass. Investigate any non-match before proceeding.

This step ships zero behaviour change to the default path.

### Step B — Switch detection to BUNDLE

1. Rewrite `detect_char_names` per §1: enumerate `Main` instance methods
   whose name starts with `get`, return the derived ids (lowercased
   method-name suffix). Keep the function signature the same.
2. Wire `extract_character_bundle` (from Step A) in as the default in
   `extractor::extract`. Keep `extract_character` (path 1) callable behind a
   `--legacy-inline` CLI flag for one release as an escape hatch.
3. Add Sheik to the golden-snapshot test corpus. Verify her output package
   makes sense (cf. Step D below).
4. Verify Giga Bowser + Wario Man emit as separate packages
   (`characters/gigabowser/`, `characters/wario_man/`) and that their
   `conversion_stats.json` carries the `ssf2_source.normalStats_id` field
   from §1.6 (`"bowser"` and `"wario"` respectively).

### Step C — Remove path 1

1. Delete `extract_attack_objects`, `extract_projectile_objects`, and the four
   `<X>Ext::get*Stats` branches in `extract_character` (which becomes a thin
   wrapper around `extract_character_bundle` and can then be inlined away).
2. Delete `is_character_ext_class` from `main.rs` and the
   `CHARACTER_MARKER_METHODS` constant.
3. Delete the `is_character_ext_class` unit tests at
   [src/main.rs:422-485](src/main.rs:422). Their replacements live in the
   new `detect_char_names` tests written in Step B.
4. Delete the `--legacy-inline` CLI flag.
5. Trim the now-unused `extract_ssf2_stats` if it isn't reused by the bundle
   path. (My current sketch keeps it — it works on either body.)

### Step D — Sheik + transformation tests

1. Add a `tests/sheik_extraction.rs` that runs the full converter against
   `zelda.ssf` with `--name sheik` and asserts:
   * The output package directory `characters/sheik/` exists.
   * `CharacterStats.hx` is non-empty and parses as Haxe.
   * `AnimationStats.hx` references the `sheik` MovieClip's frame names.
   * `HitboxStats.hx` references Sheik's attacks (e.g. her up-special needle).
   * `Script.hx` is non-empty (likely identical to Zelda's per §3).
2. Add `tests/transformation_extraction.rs` that runs against `bowser.ssf`
   (full corpus / no `--name`) and asserts:
   * Both `characters/bowser/` and `characters/gigabowser/` exist.
   * `characters/gigabowser/CharacterStats.hx` carries the Giga-specific
     `hurtFrames: 1` value, not the parent's 5.
   * `characters/gigabowser/HitboxStats.hx` references `gigaFireBreath`,
     not `fireBreath`.
   * `characters/gigabowser/conversion_stats.json` carries
     `ssf2_source.normalStats_id = "bowser"` per §1.6.
   * Symmetric assertions for `wario.ssf` → `wario` + `wario_man`.
3. Capture a SHA-256 golden snapshot of Sheik's, Giga Bowser's, and Wario
   Man's `conversion_stats.json` so future regressions are caught.

### When does `is_character_ext_class` get removed?

In Step C. It survives Step A (path 1 still callable) and Step B (legacy flag
still uses it). The moment the legacy flag goes away, the predicate goes with
it.

## 6. Testing strategy

### Existing tests that protect the migration

* `tests/golden_sandbag.rs` — full-pipeline SHA-256 snapshot of the sandbag
  output. Primary regression net. Doesn't change.
* `tests/character_byte_compare.rs` (or similar — the four-character
  byte-equivalence test we added during the consolidation): re-running it
  after Step B should produce identical bytes for those four characters.
* Unit tests in `src/abc_parser.rs` for the scan_method visitors —
  Attack/Projectile visitors still get exercised by the new
  BundleSplitVisitor implementation.

### New tests required

* `tests/path1_path2_equivalence.rs` — Step A safety net. Asserts that
  every character in the local corpus produces identical `CharacterData`
  under both extraction paths. Delete after Step C.
* `src/main.rs` `#[cfg(test)] mod tests` — replace the
  `is_character_ext_class` tests with tests for the new detection
  (enumerate `Main` instance `get*` methods, derive_id mapping).
* `tests/sheik_extraction.rs` — Step D. Sheik's golden snapshot.
* `tests/transformation_extraction.rs` — Step D. Giga Bowser + Wario Man.
* Unit test for `derive_id`: covers every observed method-name shape
  (`getMario`, `getBandanaDee`, `getgameandwatch`, `getGigaBowser`,
  `getWario_Man`).

### Tests to delete

* `is_character_ext_class` tests at
  [src/main.rs:422-485](src/main.rs:422) in Step C (~60 lines).

## 7. Risks and mitigations

### Risk: the byte-equivalence we proved is not universal

We sampled 10 characters with byte-for-byte identity across all four sections.
The full-corpus equivalence test in Step A nails this down before we delete
anything. If any character diverges, that's a real signal — investigate
before proceeding.

### Risk: Sheik's behavior code on ZeldaExt is not actually correct for Sheik

The output Script.hx will be identical to Zelda's because the Ext class is
shared. This may cause runtime issues in Fraymakers — e.g. ZeldaExt's
specials are coded with Zelda's transformation logic in mind. Mitigation: ship
Sheik with a known-limitation note in the converter output. Plan B is to
extract only the methods on ZeldaExt that contain a Sheik-relevant string
(`needle`, `lightarrow`, `chain`, `vanish`) into Sheik's Script.hx and omit
the Zelda-only ones — but that's a heuristic that needs runtime testing in
the engine to validate, so it's deferred.

### Risk: `Main::get<X>` decompilation perf regression

The Main bundle methods are larger than the corresponding `<X>Ext::get*Stats`
methods individually (they're the union of the four). But we *only* run
`scan_method` over the bundle, not full decompilation, and we run it once per
character instead of four times — net wash or slight win. Validate by timing
a representative character before/after.

### Risk: ABC blocks we don't handle today

`detect_char_names` walks every ABC block in the SWF. Today this is robust
because the marker-method scan is cheap. The bundle scan is *also* cheap per
body — single `scan_method` pass with an early `Stop` once we see a bundle —
so the corpus-wide cost is bounded.

### Risk: forced filename override breaks for legitimate use cases

`--name foo` against a SWF that doesn't have a `Main::getFoo` bundle should
fail cleanly. Current code falls back to filename-as-name; the new code
should fall back to filename only when *no* bundles are discovered, and
should error explicitly when bundles exist but none matches `--name`.

### Risk: SSF2's mod community has characters in non-standard SWFs

The `--name` override exists for a reason. We preserve it as a force-extract
path that bypasses bundle detection: if the user says "the character is
called `foo`", we look for `Main::getFoo`, then for `FooExt` (legacy mode),
and finally fall back to filename. Documented in `--help`.

### Risk: a future SSF2 build adds a non-bundle `Main::get*` method

The detection rule "any `Main` instance `get*` method = character" is
currently safe across the entire 45-SSF corpus (0 exceptions). If a future
build adds e.g. `Main::getRoster()` or `Main::getVersion()`, we'd
mis-identify it as a character and explode somewhere in Stage A trying to
split a non-bundle body. Mitigation: cheap defensive shape check at the top
of `extract_character_bundle` — if the method body doesn't pushstring
`cData`/`aData`/`pData`/`iData`/`normalStats_id`, log a warning and skip.
This costs ~5 lines and bounds the blast radius without restoring the heavy
bundle-detection-via-`scan_method` machinery to the detection layer.

### Risk: derived ids collide with existing characters

If a future SSF2 build adds two `get*` methods that derive to the same id
(e.g. `getMario` and `getmario` in the same `Main` — vanishingly unlikely
given AS3 naming) the second overwrites the first in the output directory.
If the same id is emitted twice from *different* SSFs in the same run, the
second overwrites the first silently — that's a pre-existing failure mode,
not new. Note it but don't address in this plan.

### Risk: transformations have no MovieClip / no Ext

For Giga Bowser and Wario Man, the parent's `*Ext` is shared (cross-character
fallback handles this) and their MovieClip may or may not exist as a
separate class. If absent, Stage B yields empty `animations` / `frame_scripts`
and the conversion log notes it. Stats and attacks are still emitted; the
content author can fill in the animation side manually. This matches
Fraymakers' general "transformations aren't a first-class concept; build with
scripts" philosophy.

### Risk: Fraymakers community DOES have a transformation mechanism we missed

The conclusion that FM has no transformation API came from this workspace's
docs, which are incomplete. If a community plugin or undocumented API
supports a second stat block per character, the right migration path could
be merging the transformation's data back into the parent's output package
rather than emitting it as a separate character. Mitigation: surface the
`derived_from` metadata in `conversion_stats.json` (§1.5) so this is a
post-hoc data refactor, not a re-extraction. Schema is stable either way.

## 8. Line-count estimate

Net negative. Rough deltas:

| change | direction | lines |
|--------|-----------|-------|
| Add `extract_character_bundle` + `BundleSplitVisitor` (detection no longer needs a visitor) | + | ~200 |
| Rewrite `detect_char_names` (enumerate `Main.instance_methods` filtering `name.starts_with("get")` + `derive_id`) | – | ~50 in / ~30 out |
| Delete `extract_attack_objects`, `extract_projectile_objects` and their visitors (now subsumed) | – | ~140 |
| Delete `extract_ssf2_stats` if unreused | – | ~85 |
| Delete `is_character_ext_class` + `CHARACTER_MARKER_METHODS` + tests | – | ~80 |
| Delete `<X>Ext::get*Stats` switch-arm body in `extract_character` | – | ~30 |
| Defensive bundle-shape check at top of `extract_character_bundle` (§7 risk) | + | ~10 |
| Add `ssf2_source.normalStats_id` field to conversion-log struct + serialiser | + | ~25 |
| New tests: path1/path2 equivalence + Sheik golden + transformation goldens + `derive_id` unit test | + | ~200 |
| Delete legacy tests | – | ~60 |

Estimated net: **~–130 lines** in `src/`, plus ~+140 in `tests/`. Bigger
shrink than before because detection collapsed to a one-pass-filter rather
than a visitor over every method body. Conceptual win unchanged: one
extraction path, one trivial identifier model that accommodates Sheik, Giga
Bowser, and Wario Man as first-class outputs without any special-casing.
