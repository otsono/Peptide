# Multi-character Fraymakers projects (and universal entity/script rename)

## Why this exists

Three SSF2 files contain peer character pairs that share a `.ssf` for
organisational reasons:

| SSF        | Constructor-walk order              |
|------------|-------------------------------------|
| zelda.ssf  | `getZelda`, `getSheik`              |
| bowser.ssf | `getBowser`, `getGigaBowser`        |
| wario.ssf  | `getWario`, `getWario_Man`          |

Today the converter emits each as its own standalone Fraytools project.
That matches every shipped community mod, but it breaks a forward
requirement: Fraymakers' transformation mechanic — when it lands —
will need both characters of a pair to live inside the same
`.fraytools` project so the engine can swap between them at runtime.

This document specifies:

1. A **universal rename** of the `Character.entity` and `scripts/Character/`
   conventions so all 47 character outputs land at character-id-named
   paths — applies to single- and multi-character SSFs alike.
2. The **multi-character merge** that consolidates the three pairs above
   into one project each.
3. A **per-character audio subfolder + collision-suffix rule** so the
   merged projects keep their per-character data legible.

## Structural template — Annie

`aJewelofRarity/FraymakersProjects/Characters/AnnieCharacter` is the
closest in-the-wild precedent: one Fraytools project with a character
+ AI + projectile + displayobject.

* `Annie.fraytools` carries project settings ONLY — no content list.
* `library/manifest.json :: content[]` is the single source of truth
  for what ships. Heterogeneous entries by `type` (`character`,
  `characterAi`, `projectile`, `displayobject`); adding a second
  `type: "character"` entry is structurally identical to adding a
  projectile.
* `library/scripts/Character/Script.hx`, `…/CharaStats.hx`, etc.
* Id namespacing is per-asset: `annieStats`, `annieAnim`, `annieHit`,
  `annieScript`, `annieCostumes`, `annieAI`; the projectile uses
  `cutCharaStats`. Each id is a string that resolves against the
  project-internal asset pool.

The Annie example uses capital `Entities/` / `Scripts/`. FrayTools is
case-insensitive on macOS; the existing converter ships lowercase
paths. Keeping lowercase preserves the rest of the layout we already
emit; this plan does NOT change case.

## Section 1 — Universal rename (applies to ALL 47 characters)

### What changes

For every character in every SSF:

| was | becomes |
|---|---|
| `library/entities/Character.entity`           | `library/entities/<Pascal>.entity` |
| `library/entities/Character.entity.meta`      | `library/entities/<Pascal>.entity.meta` |
| `library/entities/menu.entity`                | `library/entities/Menu.entity` (single-char); see Section 2 for multi-char |
| `library/entities/menu.entity.meta`           | `library/entities/Menu.entity.meta` |
| `library/scripts/Character/`                  | `library/scripts/<Pascal>/` |
| `library/scripts/Character/CharacterStats.hx` | `library/scripts/<Pascal>/CharacterStats.hx` |
| `library/scripts/Character/HitboxStats.hx`    | `library/scripts/<Pascal>/HitboxStats.hx` |
| `library/scripts/Character/AnimationStats.hx` | `library/scripts/<Pascal>/AnimationStats.hx` |
| `library/scripts/Character/Script.hx`         | `library/scripts/<Pascal>/Script.hx` |
| (each `.hx.meta` sidecar moves with its parent)    | |

Filename inside the scripts subdir doesn't change — only the parent
directory does.

### PascalCase rule

The `<Pascal>` form is derived from the character's `Main::get<X>()`
method name (already captured by the constructor walker on
`MainPackageMetadata.characters` as `(derived_id, method_name)`). The
method name is the authoritative source of SSF2's TitleCase intent
(SSF2 wrote `getBandanaDee`, not `getbandanadee`).

```
pascal_form(method_name):
    suffix = method_name.strip_prefix("get").unwrap_or("")
    no_underscores = suffix.replace("_", "")
    if no_underscores.is_empty(): return None
    return no_underscores[0].to_uppercase() + no_underscores[1..]
```

Coverage across the entire observed corpus (47 characters):

| method name        | derived id (lowercase) | `<Pascal>` form |
|--------------------|------------------------|------------------|
| `getMario`         | `mario`                | `Mario`          |
| `getSandbag`       | `sandbag`              | `Sandbag`        |
| `getSheik`         | `sheik`                | `Sheik`          |
| `getBandanaDee`    | `bandanadee`           | `BandanaDee`     |
| `getCaptainFalcon` | `captainfalcon`        | `CaptainFalcon`  |
| `getChibiRobo`     | `chibirobo`            | `ChibiRobo`      |
| `getDonkeyKong`    | `donkeykong`           | `DonkeyKong`     |
| `getMegaMan`       | `megaman`              | `MegaMan`        |
| `getMetaKnight`    | `metaknight`           | `MetaKnight`     |
| `getPacMan`        | `pacman`               | `PacMan`         |
| `getBlackMage`     | `blackmage`            | `BlackMage`      |
| `getGigaBowser`    | `gigabowser`           | `GigaBowser`     |
| `getWario_Man`     | `wario_man`            | `WarioMan`       |
| `getgameandwatch`  | `gameandwatch`         | `Gameandwatch`   |

The other 33 chars are single-word getters with the obvious
capitalisation (`getKirby` → `Kirby`, `getYoshi` → `Yoshi`, etc.).

Two name conventions cohabit:

1. **Character id** (lowercase, preserves underscores): used in
   `conversion_log.json`, `manifest.json :: content[i].id`, the audio
   subdir name, the constructor walker output. Examples: `mario`,
   `wario_man`, `gigabowser`, `gameandwatch`.
2. **PascalCase form** (no underscores, leading capital): used in entity
   filenames and the `library/scripts/<Pascal>/` directory. Examples:
   `Mario`, `WarioMan`, `GigaBowser`, `Gameandwatch`.

The id is the canonical identifier; the PascalCase form is purely a
filename/path convention that matches the existing FM community
convention (Annie's `BurstMeter.entity` uses TitleCase, no
underscores). This resolves the `Wario_Man` vs `WarioMan` open
question — **`WarioMan` (strip underscores)** matches the FM
PascalCase precedent.

The `getgameandwatch` lowercase-source-name case is the only spot
where the PascalCase form loses readability (`Gameandwatch` rather
than `GameAndWatch`). The trade-off: a hardcoded `GameAndWatch`
override would be a one-line lookup table, but every other character
derives cleanly from the SSF2 method name — adding a special case
introduces drift potential we don't need. The plain
`Gameandwatch` is what SSF2 itself wrote.

### Why the rename is universal

The user is explicit: "Universal naming changes that apply to ALL 47
characters (not just multi-char)." Two practical reasons reinforce
this:

1. The multi-char path needs character-id-named files anyway (two
   characters' `Character.entity` would collide). Universalising the
   convention avoids a "single-char path keeps `Character.entity` /
   multi-char path renames" fork, which would mean every downstream
   tool (the SwiftUI popup, future verification scripts, FrayTools
   project templates) has to handle two layouts.
2. The current `Character.entity` filename loses information — it
   doesn't say which character. After the rename, the filename matches
   the entity's `id` field inside, making the file tree self-describing.

### golden_sandbag will break — intentionally

Every sandbag output path containing `Character.entity` /
`Character/CharacterStats.hx` / etc. moves. The golden hash table
contains those filenames as keys, so every entry changes. We regenerate
the golden as part of Step A; this is a deliberate behaviour change,
not a regression. The post-rename outputs are still hash-stable
(deterministic GUIDs are seeded from `{char_id}::{kind}`, unchanged).

### Concrete file tree — sandbag, single-character, before vs after

**Before (HEAD `cada7e1c`):**

```
characters/sandbag/
├── sandbag.fraytools
├── conversion_log.json
├── conversion_stats.json
└── library/
    ├── manifest.json (+ .meta)
    ├── costumes.palettes (+ .meta)
    ├── palette_preview.png (+ .meta)
    ├── entities/
    │   ├── Character.entity (+ .meta)              ← renamed
    │   ├── menu.entity (+ .meta)
    │   └── <projectile>.entity (+ .meta)
    ├── scripts/
    │   ├── Character/                              ← renamed
    │   │   ├── CharacterStats.hx (+ .hx.meta)
    │   │   ├── HitboxStats.hx (+ .hx.meta)
    │   │   ├── AnimationStats.hx (+ .hx.meta)
    │   │   └── Script.hx (+ .hx.meta)
    │   └── Projectile/
    │       └── <Pascal>{Script,Stats,…}.hx (+ .meta)
    ├── sprites/
    │   └── *.png + *.png.meta
    └── audio/
        └── *.wav + *.wav.meta
```

**After Section 1 (universal rename only — multi-char merge has NOT
happened yet, sandbag is single-character):**

```
characters/sandbag/
├── sandbag.fraytools
├── conversion_log.json
├── conversion_stats.json
└── library/
    ├── manifest.json (+ .meta)
    ├── costumes.palettes (+ .meta)
    ├── palette_preview.png (+ .meta)
    ├── entities/
    │   ├── Sandbag.entity (+ .meta)                ← was Character.entity
    │   ├── Menu.entity (+ .meta)                   ← was menu.entity
    │   └── <Pascal>.entity (+ .meta)               ← per projectile (already PascalCase)
    ├── scripts/
    │   ├── Sandbag/                                ← was Character/
    │   │   ├── CharacterStats.hx (+ .hx.meta)
    │   │   ├── HitboxStats.hx (+ .hx.meta)
    │   │   ├── AnimationStats.hx (+ .hx.meta)
    │   │   └── Script.hx (+ .hx.meta)
    │   └── Projectile/
    │       └── <Pascal>{Script,Stats,…}.hx (+ .meta)
    ├── sprites/
    │   └── *.png + *.png.meta
    └── audio/
        └── *.wav + *.wav.meta
```

The script files inside `Sandbag/` keep their names. The character
entity (`Character.entity` → `Sandbag.entity`), the scripts subdir
(`Character/` → `Sandbag/`), and the menu entity (`menu.entity` →
`Menu.entity`) all rename. Projectile entities were already PascalCase
in the existing code (the converter PascalCases SSF2 projectile names
when emitting the `.entity` filename); no change needed there.

All 47 characters get the same treatment.

## Section 2 — Multi-character project merge

### Scope

After Section 1, every character is still its own project, just with
better filenames. Section 2 merges the three pairs:

| project dir name (= SSF stem)    | constituents                       |
|----------------------------------|------------------------------------|
| `characters/zelda/`              | zelda + sheik                      |
| `characters/bowser/`             | bowser + gigabowser                |
| `characters/wario/`              | wario + wario_man                  |

### Project file naming

* `<stem>.fraytools` — named after the SSF (e.g. `zelda.fraytools`).
* `<stem>` is the constructor walker's `MainPackageMetadata.id` (which
  equals the filename stem in every observed SSF — the Tier 1
  validation pass enforces this).

### Per-character file naming inside a multi-char project

Same rules as Section 1, applied per character:

* `library/entities/<Pascal>.entity` per character — e.g.
  `library/entities/Zelda.entity` AND `library/entities/Sheik.entity`.
* `library/scripts/<Pascal>/` per character — e.g.
  `library/scripts/Zelda/` AND `library/scripts/Sheik/`.

These don't collide between characters because the PascalCase form is
already character-specific.

### Menu entity naming in multi-char projects

Single-char keeps `Menu.entity` (Section 1). For multi-char, each
character has its own portrait set, so we need two distinct menu
entities. Convention:

* `library/entities/<Pascal>_Menu.entity` per character — e.g.
  `library/entities/Zelda_Menu.entity` and
  `library/entities/Sheik_Menu.entity`.

Rationale:

* Keeps the character name forward in the filename, which matches the
  user's stated intuition ("Zelda's menu" reads naturally).
* The underscore separator distinguishes the per-character qualifier
  from the role marker, mirroring the suffix-rule shape
  (`costumes.palettes` + `costumes.palettes2`): both characters have
  clearly-named files that don't pretend to be the same asset.
* Annie's precedent (`Menu.entity` with PascalCase role name)
  generalises cleanly to the multi-character case as
  `<Char>_Menu.entity` without inventing a new convention.

Considered and rejected:
* `Menu_<Pascal>.entity` (role first) — reads slightly less naturally;
  buries the character name.
* `<Pascal>Menu.entity` (no separator) — looks like a single
  PascalCase compound (`ZeldaMenu`) which is fine in isolation but
  becomes hard to parse for longer character names
  (`CaptainFalconMenu`).

### The collision-suffix rule

For files that would collide between characters at the **same path**,
the second-and-onward writers get a numeric suffix appended to the
extension. The rule:

* Files are written in **constructor-walk order** (the order Main's
  iinit declares the characters in — the same order
  `MainPackageMetadata.characters` returns).
* The first character to write a given path keeps the unsuffixed
  filename.
* The second appends `2` to the file extension, the third `3`, etc.
* This applies to the file AND its `.meta` sidecar — both get the same
  suffix.

**Why constructor-walk order, not alphabetical?** Constructor-walk
order is what SSF2 itself uses: the "primary" character (the one the
SSF is named after) is always declared first. This makes the
unsuffixed file deterministically the primary, matching player
intuition (Zelda before Sheik, Bowser before Giga Bowser, Wario
before Wario Man). Alphabetical would invert the
zelda/sheik case, which feels wrong.

Tiebreaker for hypothetical future SSFs with three+ characters: stay
in constructor order. The rule is "Nth character in
`MainPackageMetadata.characters` gets suffix `N` if N>1, else
unsuffixed."

### Which files collide (per-character data at a shared path)

| File path                                  | Multi-char treatment                                       |
|--------------------------------------------|------------------------------------------------------------|
| `library/entities/<Pascal>.entity` (+ .meta) | No collision — character `<Pascal>` is in the filename   |
| `library/scripts/<Pascal>/…`                | No collision — `<Pascal>` is in the path                  |
| `library/entities/<Pascal>_Menu.entity` (+ .meta) | No collision — character-qualified per Section 2     |
| `library/costumes.palettes` (+ .meta)       | **Collides** — first character plain, others suffixed     |
| `library/palette_preview.png` (+ .meta)     | **Collides** — first character plain, others suffixed     |
| `library/entities/<Pascal>.entity` (+ .meta) for projectiles | Collision only if two characters have same-named projectiles (none observed); suffix on collision |
| `library/scripts/Projectile/<Pascal>*.hx` (+ .meta) | Same as above                                       |
| `library/manifest.json` (+ .meta)           | Single shared file (the manifest IS the project)          |
| `library/audio/*`                           | **Special rule** — moves into per-character subdirs (see Section 3) |
| `library/sprites/*.png` (+ .meta)           | Shared dir; PNG basenames already carry SSF2 `<char>_fla.` prefix so no collision in practice |
| `conversion_log.json`                       | Project-scoped — one log with a `characters: […]` array   |
| `conversion_stats.json`                     | Project-scoped — same                                     |

### Concrete file tree — zelda + sheik after multi-char merge

```
characters/zelda/
├── zelda.fraytools
├── conversion_log.json                         ← one project log
├── conversion_stats.json
└── library/
    ├── manifest.json (+ .meta)                 ← TWO type:"character" entries
    ├── costumes.palettes (+ .meta)              ← zelda's (constructor-walk index 0)
    ├── costumes.palettes2 (+ costumes.palettes2.meta)
    │                                            ← sheik's (constructor-walk index 1)
    ├── palette_preview.png (+ .meta)            ← zelda's
    ├── palette_preview.png2 (+ palette_preview.png2.meta)
    │                                            ← sheik's
    ├── entities/
    │   ├── Zelda.entity (+ .meta)               ← character entity (id: "zelda")
    │   ├── Sheik.entity (+ .meta)               ← character entity (id: "sheik")
    │   ├── Zelda_Menu.entity (+ .meta)          ← zelda's portrait
    │   ├── Sheik_Menu.entity (+ .meta)          ← sheik's portrait
    │   └── <Pascal>.entity (+ .meta)            ← per projectile (no collision; suffixed on collision)
    ├── scripts/
    │   ├── Zelda/                               ← zelda's scripts subdir
    │   │   ├── CharacterStats.hx (+ .hx.meta)
    │   │   ├── HitboxStats.hx (+ .hx.meta)
    │   │   ├── AnimationStats.hx (+ .hx.meta)
    │   │   └── Script.hx (+ .hx.meta)
    │   ├── Sheik/                               ← sheik's scripts subdir
    │   │   ├── CharacterStats.hx (+ .hx.meta)
    │   │   ├── HitboxStats.hx (+ .hx.meta)
    │   │   ├── AnimationStats.hx (+ .hx.meta)
    │   │   └── Script.hx (+ .hx.meta)
    │   └── Projectile/                          ← shared dir, files Pascal-named per projectile
    │       └── <Pascal>{Script,Stats,…}.hx (+ .meta)
    ├── sprites/                                  ← shared; PNG basenames already
    │   └── *.png + *.png.meta                     carry `zelda_fla.*` / `sheik_fla.*` prefix
    └── audio/                                    ← per-character subdir (Section 3)
        ├── zelda/
        │   └── *.wav + *.wav.meta
        └── sheik/
            └── *.wav + *.wav.meta
```

`bowser.ssf` and `wario.ssf` mirror this exactly with their respective
pairs in constructor-walk order: bowser then gigabowser
(`Bowser.entity` + `GigaBowser.entity`,
`Bowser_Menu.entity` + `GigaBowser_Menu.entity`,
`scripts/Bowser/` + `scripts/GigaBowser/`), wario then wario_man
(`Wario.entity` + `WarioMan.entity`,
`Wario_Menu.entity` + `WarioMan_Menu.entity`,
`scripts/Wario/` + `scripts/WarioMan/`).

## Section 3 — Per-character `audio/` subfolders

For multi-character projects, `library/audio/` becomes a folder per
character:

```
library/audio/
├── <char_1>/
│   └── *.wav + *.wav.meta
└── <char_2>/
    └── *.wav + *.wav.meta
```

Single-character projects keep the flat `library/audio/*.wav` layout
unchanged.

The audio subdir is **named with the lowercase character id**
(`audio/wario_man/`, NOT `audio/WarioMan/`). The PascalCase form is
reserved for entity filenames and the scripts subdir; the id form is
used for the manifest's `content[i].id`, `conversion_log.json`, and
the audio subdir.

The existing `sound_extractor::extract_all_sounds` scans the whole SWF
regardless of `char_name` — paired SSFs currently extract audio twice
(once into each per-char output dir). Under Section 3 we extract once
per project but still partition by character (SSF2's SymbolClass
mapping tells us which sound belongs to which character; the
character-name prefix in the sound symbol name disambiguates).

Sounds shared by both characters of a pair (e.g. a `brawl_kick_s` SFX
used in both Zelda and Sheik attacks) are emitted under BOTH
characters' subdirs. Simpler than a global pool and lets the manifest
reference them per-character without cross-project lookups. Cost:
duplicate WAV bytes in the package; acceptable for now and trivially
de-duplicable later.

## Section 4 — Manifest shape

### Single-character project (Section 1 form)

Unchanged from today other than the renamed asset paths:

```json
{
  "resourceId": "sandbag",
  "content": [
    {
      "id": "sandbag",
      "type": "character",
      "objectStatsId": "sandbagStats",
      "animationStatsId": "sandbagAnimation",
      "hitboxStatsId": "sandbagHitbox",
      "scriptId": "sandbagScript",
      "costumesId": "sandbagCostumes",
      "metadata": { ... }
    },
    { "id": "sandbagAI",        "type": "characterAi", "scriptId": "sandbagAIScript" },
    { "id": "<proj>",           "type": "projectile",  ... }
  ]
}
```

### Multi-character project

```json
{
  "resourceId": "zelda",          // the SSF id (constructor-walked)
  "content": [
    {
      "id": "zelda",
      "type": "character",
      "objectStatsId": "zeldaStats",
      "animationStatsId": "zeldaAnimation",
      "hitboxStatsId": "zeldaHitbox",
      "scriptId": "zeldaScript",
      "costumesId": "zeldaCostumes",
      "metadata": { ... }
    },
    {
      "id": "sheik",
      "type": "character",
      "objectStatsId": "sheikStats",
      "animationStatsId": "sheikAnimation",
      "hitboxStatsId": "sheikHitbox",
      "scriptId": "sheikScript",
      "costumesId": "sheikCostumes",
      "metadata": { ... }
    },
    { "id": "zeldaAI", "type": "characterAi", "scriptId": "zeldaAIScript" },
    { "id": "sheikAI", "type": "characterAi", "scriptId": "sheikAIScript" },
    // per-projectile entries for both characters
    { "id": "lightArrow",  "type": "projectile", ... },
    { "id": "needle",      "type": "projectile", ... }
  ]
}
```

Asset ids are already character-name-prefixed in the existing
single-character output (`zeldaStats`, `sheikScript`); multi-char just
puts both into the same `content[]` array. No new namespacing rule
needed.

## Section 5 — `conversion_log.json` shape

### Single-character project

Unchanged. One JSON object with `character`, `ssf2_source`,
`validation_warnings`, `unknown`, `ssf2_only`.

### Multi-character project

One project-scoped log with a `characters: [...]` array:

```json
{
  "project": "zelda",
  "project_guid": "68019ba3-...",
  "characters": [
    {
      "id": "zelda",
      "ssf2_source": { "source_method": "Main::getZelda", ... },
      "validation_warnings": [],
      "unknown": [ ... ],
      "ssf2_only": [ ... ]
    },
    {
      "id": "sheik",
      "ssf2_source": { "source_method": "Main::getSheik", ... },
      "validation_warnings": [],
      "unknown": [ ... ],
      "ssf2_only": [ ... ]
    }
  ]
}
```

The SwiftUI popup that consumes this needs an update: detect the
`characters: [...]` array shape and render per-character sections.
Single-character log shape stays for backwards compatibility (the
popup branches on `"characters" in payload`).

## Section 6 — Migration plan

Four stages, each independently committable and shippable.

### Stage A — Universal rename + golden regeneration

The biggest blast radius but the simplest change per touched line.

**Code:**
* `src/haxe_gen.rs` — `generate_character_stats`, `generate_hitbox_stats`,
  `generate_animation_stats`, `generate_script`, the menu entity writer,
  the projectile writers — every path that produces
  `library/entities/Character.entity` or `library/scripts/Character/...`
  uses the new naming. ~15-20 path-construction sites; each is a small
  edit.
* `src/entity_gen.rs` — the `Character.entity` filename ref.
* Add a `pascal_id(char_id)` helper in `main.rs` or `extractor.rs`:
  uppercase first char, rest unchanged.
* `manifest.json` generator (`haxe_gen::generate_manifest`) is unchanged
  — it doesn't reference the script directory name (the manifest binds
  by `scriptId`, not by filename); the `.meta` files bind id ↔ filename.
* `.meta` sidecar GUIDs are unchanged — they're seeded from
  `{char_id}::{kind}`, not from filename.

**Tests:**
* `tests/golden/sandbag_hashes.txt` — regenerate every entry. Use the
  test's built-in regen instructions. Every hash that referenced
  `Character.entity`, `menu.entity`, or `scripts/Character/` rolls.
* `tests/sheik_extraction.rs` — paths change from
  `sheik/library/scripts/Character/` to
  `sheik/library/scripts/Sheik/` and from
  `sheik/library/entities/Character.entity` to
  `sheik/library/entities/Sheik.entity`.
* `tests/transformation_extraction.rs` — same per-character renames
  (`gigabowser/library/scripts/GigaBowser/...`,
  `gigabowser/library/entities/GigaBowser.entity`;
  `wario_man/library/scripts/WarioMan/...`,
  `wario_man/library/entities/WarioMan.entity`).
* New unit test for `pascal_form`: covers `getSandbag` → `Sandbag`,
  `getWario_Man` → `WarioMan`, `getgameandwatch` → `Gameandwatch`,
  `getGigaBowser` → `GigaBowser`, `getBandanaDee` → `BandanaDee`.

**Risk:** modest. Mechanical refactor. The biggest exposure is missing
a path-construction site; CI catches that via the rebuilt golden.

**Commit message:** `feat(layout): rename Character.entity / scripts/Character/ to character-id paths`

### Stage B — Multi-character project merge

The structural change. Builds on Stage A's character-id-named files.

**Code:**
* `src/main.rs` — split the `main()` per-SSF flow:
  - `char_names.len() > 1` → `process_multi_character_project(...)`.
  - else → existing `process_character(...)` (unchanged).
* Add `MultiCharacterProject` + `CharacterInProject` data types in
  `src/extractor.rs` or a new `src/project.rs`.
* Add `haxe_gen::generate_multi_character_project` that writes:
  - One `<project>.fraytools`
  - One `library/manifest.json` with N character entries
  - Per-character entity + scripts dirs
  - Shared sprites dir (PNGs already prefixed)
  - Audio still extracted per-character (per-char subdir lands in Stage C)
* Refactor `generate_manifest()` to take `&[&CharacterData]` instead of
  `&CharacterData`. Existing single-char call wraps in
  `std::slice::from_ref(&data)` — no behaviour change for the 41
  single-char SSFs.
* The collision-suffix rule is gated on writer order, so plumb a small
  `WriteCounter` (or just a `BTreeSet<PathBuf>` of paths-already-written)
  through `generate_multi_character_project`. On collision: append the
  next index to the file extension.
* `conversion_log.json` for multi-char projects writes once at project
  root with the `characters: [...]` array shape (Section 5).

**Sandbag stays single-character.** Stage B does NOT touch the
single-character emission path; only the three multi-char SSFs route
through the new code. golden_sandbag must continue passing
(unchanged from Stage A's regen).

**Tests:**
* Update `tests/sheik_extraction.rs` — Sheik now lives at
  `characters/zelda/library/entities/Sheik.entity` /
  `characters/zelda/library/scripts/Sheik/`. The standalone
  `characters/sheik/` directory does NOT exist.
* Update `tests/transformation_extraction.rs` — gigabowser is at
  `characters/bowser/library/entities/GigaBowser.entity` /
  `characters/bowser/library/scripts/GigaBowser/`. Wario Man is at
  `characters/wario/library/entities/WarioMan.entity` /
  `characters/wario/library/scripts/WarioMan/`.
* New `tests/multi_character_project.rs`:
  - Asserts `characters/zelda/zelda.fraytools` exists,
    `characters/sheik/sheik.fraytools` does NOT.
  - Asserts the manifest has two `type: "character"` entries with ids
    `zelda` and `sheik`.
  - Asserts both character entities (`Zelda.entity`, `Sheik.entity`),
    both menu entities (`Zelda_Menu.entity`, `Sheik_Menu.entity`),
    both scripts subdirs (`scripts/Zelda/`, `scripts/Sheik/`).
  - Asserts collision suffixes on `costumes.palettes2` and
    `palette_preview.png2` (Sheik's, since Zelda is constructor index 0).

**Risk:** moderate. The orchestration is genuinely new code. The
single-char path stays intact so the corpus regression surface is
bounded to the 3 multi-char SSFs.

**Commit message:** `feat(multi-char): emit one fraytools project per multi-character SSF`

### Stage C — Per-character `audio/` subfolders + sound-extraction dedup

Smaller follow-up that delivers the per-character audio organisation
+ the audio-dedup performance win.

**Code:**
* `src/main.rs` (or wherever `extract_all_sounds` is invoked) — for
  multi-character projects, partition the SymbolClass-to-character
  mapping and write each character's sounds to
  `library/audio/<char_id>/`. The existing
  `sound_extractor::parse_sounds` returns the full SymbolClass map; we
  use the SSF2 sound name prefix (`zelda_jump_vfx`, `sheik_hurt1`) to
  bucket each one to its character.
* Sounds whose prefix doesn't match either character's id (the shared
  brawl_* / engine sounds) are emitted under BOTH characters' subdirs.
  Plan accepts the duplication; future optimisation deduplicates.
* Single-character projects: unchanged — flat `library/audio/*.wav`
  layout.

**Tests:**
* Update `tests/multi_character_project.rs` from Stage B:
  - Assert `characters/zelda/library/audio/zelda/` exists with WAVs.
  - Assert `characters/zelda/library/audio/sheik/` exists with WAVs.
  - Assert `characters/zelda/library/audio/zelda_jump_vfx.wav`
    (flat-layout-shaped) does NOT exist.
* The single-char `golden_sandbag` test stays unchanged — sandbag's
  audio is still at flat `library/audio/*.wav`.

**Risk:** small. The split is a path-construction change; sound parsing
is unchanged.

**Commit message:** `feat(multi-char): split library/audio/ into per-character subdirs`

### Stage D — Validation + manual FrayTools verification

**Code: none.** Verification only.

**Steps:**
* Run the full local 47-character corpus through the converter; confirm
  44 single-char projects + 3 multi-char projects (zelda, bowser, wario).
* Open each multi-char project in FrayTools (manual). Confirm:
  - The editor lists both characters.
  - Each character's animations, hitboxes, palettes load.
  - The transformation TODO banner in gigabowser/wario_man
    `CharacterStats.hx` displays correctly.
* Update the SwiftUI popup that consumes `conversion_log.json` to
  handle the new `characters: [...]` array shape.
* Update `DEVELOPMENT.md`, `AGENT_CONTEXT.md`, `README.md` output
  diagrams to reflect the new layout. (Falls under the
  `CONTRIBUTING.md` hot-file → doc-section gate.)

**Commit messages:**
* `chore: SwiftUI popup handles multi-character conversion_log shape`
* `docs: refresh after universal entity/script rename + multi-char merge`

## Section 7 — TODO banner update

The transformation banner in `gigabowser` / `wario_man`
`CharacterStats.hx` currently reads:

> Fraymakers does not yet expose a transformation API, so we emit the
> form as a standalone character package.

Under the new layout, gigabowser is no longer a standalone package —
it's a peer entity in `characters/bowser/`. Update the banner in
Stage B (one mechanical edit in `haxe_gen::generate_character_stats`):

> Fraymakers does not yet expose a transformation API. Both characters
> ship as peer entities in the same project so that, when the API
> lands, the parent's Script.hx can swap between them at runtime
> without a cross-project asset reload.

## Section 8 — Risks + open questions

### Risk: FrayTools editor surprises us with multi-character UX

Annie + community precedent confirm the file format loads, but the
FrayTools editor's UX for multi-character projects is empirically
untested. The character-select dropdown, costume editor scope, palette
preview, and so on may treat the first manifest entry as primary and
surface the others oddly. Mitigation: Stage D manual verification
before declaring done.

**Rollback flag**: ship a `--per-character-projects` CLI flag in Stage
B that reverts to the pre-Section-2 emission shape. Delete after FM
editor verification. Single-character SSFs ignore the flag — they're
already per-character.

### Risk: `costumes.palettes` merge into one project may compose oddly

Zelda and Sheik likely have palette structures with different colour-
slot counts; emitting them as `costumes.palettes` + `costumes.palettes2`
(per the suffix rule) means each character has its own palette file
which the manifest binds via `costumesId: "zeldaCostumes"` vs
`"sheikCostumes"`. No merge logic needed if we follow the suffix rule
mechanically. This sidesteps the "do the palettes compose" question
entirely.

### Risk: shared sprites dir with two characters' PNGs

PNG basenames in the converter already derive from SSF2 symbol names
(`zelda_fla.frame47.png` vs `sheik_fla.frame47.png`). The prefix
disambiguates. The `image_extractor` writes PNGs directly to disk; for
multi-char projects we point all writes at the shared
`library/sprites/` dir. Add a defensive assertion in Stage B that no
PNG basename collision occurs; log a warning + suffix on collision.

### Risk: PascalCase rule loses readability for compound names

`captainfalcon` → `Captainfalcon` (not `CaptainFalcon`) because we
don't have word boundaries in the lowercase id. Acceptable per the
"simple, deterministic" rule. If a user wants better casing later, it
becomes a one-line lookup table in the PascalCase helper — additive
and non-breaking.

### Open question — does the user agree with constructor-walk order for the suffix rule?

The rule "first character in `MainPackageMetadata.characters` keeps the
unsuffixed filename, subsequent characters get `2`/`3`/..." gives:
- zelda.ssf: zelda unsuffixed, sheik = `2`
- bowser.ssf: bowser unsuffixed, gigabowser = `2`
- wario.ssf: wario unsuffixed, wario_man = `2`

This matches the "primary character" intuition. Confirm before
implementation.

### Resolved — PascalCase form strips underscores

Open question collapsed. The `pascal_form(method_name)` rule strips
`_` from the method-name suffix and uppercases the first character.
Output covers every observed corpus shape (Section 1's table). The
choice favours `WarioMan` over `Wario_Man` because Annie's precedent
(`BurstMeter.entity`) uses TitleCase without separators; the
character id (`wario_man`, with underscore) stays as-is for use in
`audio/<char_id>/` and `manifest.json :: content[i].id`.

## Line-count estimate (per stage)

| stage | source delta | tests delta |
|-------|--------------|-------------|
| A — universal rename | ~+50 / ~-20 (path construction edits + a small `pascal_id` helper) | ~+30 / ~-20 (test path updates) |
| B — multi-char merge | ~+350 / ~0 (the new project orchestration) | ~+150 / ~-40 (new tests, sheik/transformation rewrites) |
| C — audio subfolder | ~+40 / ~-10 | ~+30 / ~0 |
| D — verification + docs + SwiftUI | ~+30 in SwiftUI, ~+50 in docs | ~0 |

Total estimated net: **~+450 lines `src/`, ~+170 lines `tests/`,
~+50 lines SwiftUI / docs.** Largest of the recent migrations; the
universal rename is one-touch-everywhere and the multi-char
orchestration is genuinely new code, but each stage is independently
shippable and rollback-able.

## What's deferred

* **Manual FrayTools editor verification** (Stage D, out of CI scope).
* **Real audio-deduplication** between paired characters (Section 3
  emits duplicates for shared sounds; cleanup is a follow-up).
* **Better PascalCase** for compound character ids (`Captain Falcon`
  vs `Captainfalcon`) — additive future change behind a lookup table.
