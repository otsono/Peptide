# SSF2 → Fraymakers Converter — Developer Guide

> **Audience:** a developer or AI agent picking up this project cold.
> This document explains what the project is, how to build and run it, how the
> conversion pipeline is wired together, what every module does, what currently
> works, what is unfinished, and what to do next.
>
> **Companion docs:**
> - [`AGENT_CONTEXT.md`](AGENT_CONTEXT.md) — the authoritative *format* reference
>   (SSF2 `.ssf`/SWF internals and the Fraymakers `.entity` JSON schema). Read it
>   alongside this file. Where the two disagree, see
>   [§9 "Known issues"](#9-known-issues--gaps) — `AGENT_CONTEXT.md` has drifted
>   slightly out of date on rotation handling.
> - [`README.md`](README.md) — short user-facing summary (also slightly stale;
>   see [§3.6](#36-readme--agent_context-discrepancies)).

---

## Table of contents

1. [What this project is](#1-what-this-project-is)
2. [Repository layout](#2-repository-layout)
3. [Setup, build & run](#3-setup-build--run)
4. [The conversion pipeline](#4-the-conversion-pipeline)
5. [Module-by-module reference](#5-module-by-module-reference)
6. [Input format: SSF2 `.ssf`](#6-input-format-ssf2-ssf)
7. [Output format: Fraymakers character package](#7-output-format-fraymakers-character-package)
8. [Current status — what works](#8-current-status--what-works)
9. [Known issues & gaps](#9-known-issues--gaps)
10. [Prioritized next steps](#10-prioritized-next-steps)
11. [Git state & history](#11-git-state--history)
12. [Tips for the next agent](#12-tips-for-the-next-agent)

---

## 1. What this project is

A **one-way asset/data converter** that turns a **Super Smash Flash 2 (SSF2)**
character into a **Fraymakers** character mod package.

Both SSF2 and Fraymakers are indie platform-fighting games. SSF2 ships its
characters as Flash content; Fraymakers characters are authored in **FrayTools**
(the official Fraymakers modding editor) as a directory of JSON + Haxe + PNG
assets. This tool bridges the two: feed it an SSF2 `.ssf` file and it writes a
complete FrayTools-compatible character folder.

It converts, per character:

| SSF2 source | → | Fraymakers output |
|---|---|---|
| Bitmap sprites (one per animation frame) | → | `library/sprites/*.png` + `.meta` sidecars |
| Per-frame collision boxes (hitboxes, hurtboxes, grab/ledge/reflect boxes…) | → | `COLLISION_BOX` / `COLLISION_BODY` / `POINT` layers in `Character.entity` |
| Animation timelines | → | `animations` / `layers` / `keyframes` in `Character.entity` |
| AS3 frame scripts (ABC bytecode) | → | `FRAME_SCRIPT` keyframe code in the entity |
| AS3 character logic (`XxxExt` class methods) | → | decompiled into `Script.hx` |
| Character stats (weight, gravity, speeds…) | → | `CharacterStats.hx` |
| Attack/hitbox data | → | `HitboxStats.hx` |
| Animation flags | → | `AnimationStats.hx` |
| Costume palettes (`misc.ssf`) | → | `costumes.palettes` + entity `paletteMap` |
| Sounds | → | `library/sounds/*.ogg` + sound content entries |
| Projectiles & menu head sprite | → | extra `*.entity` files + projectile scripts |

The conversion is **fully automatic and deterministic** — every GUID in the
output is derived (UUID v5) from the character id + a context string, so
re-running the converter on the same input reproduces byte-identical GUIDs.

**Scope note (confirmed from code, not assumed):** this is a *data* converter,
not a runtime. It does not run SSF2 or Fraymakers. It does not produce gameplay
behaviour — translated logic lands in `.hx` files that a human must still review
(many lines are emitted with `/*TODO*/` markers where no equivalent exists).

---

## 2. Repository layout

```
ssf2-fraymakers-converter/
├── Cargo.toml              Rust package manifest (package name: ssf2_converter)
├── Cargo.lock
├── README.md               Short user-facing readme (slightly stale)
├── AGENT_CONTEXT.md         Authoritative SSF2/Fraymakers FORMAT reference
├── DEVELOPMENT.md           ← this file
├── build-app.sh             Build Rust + Swift, assemble the macOS .app, launch it
├── rebuild-sandbag.sh       Quick: rebuild release binary + convert sandbag.ssf
├── .gitignore              Ignores *.ssf, *.swf, /target, characters/
├── src/                    All Rust source (the converter itself)
│   ├── main.rs             CLI entry point + pipeline orchestration
│   ├── lib.rs              Module declarations (exposes modules to src/bin/)
│   ├── ssf.rs              .ssf → raw SWF decompression
│   ├── swf_parser.rs       SWF tag parsing (thin wrapper over the `swf` crate)
│   ├── abc_parser.rs       AS3 ABC bytecode parser  (largest module, ~2200 LOC)
│   ├── decompiler.rs       ABC bytecode → readable Haxe-ish source (~1700 LOC)
│   ├── extractor.rs        Pulls attacks/stats/scripts/anim-map out of ABC
│   ├── anim_splitter.rs    Splits multi-move SSF2 sprites into FM animations
│   ├── sprite_parser.rs    Per-frame collision-box geometry from SWF timelines
│   ├── image_extractor.rs  PNG sprite extraction + per-frame image placement
│   ├── sound_extractor.rs  Audio extraction (Nellymoser/MP3 → OGG via FLV)
│   ├── palette_gen.rs      Costume palette generation & sprite re-indexing
│   ├── api_mappings.rs     SSF2 AS3 API → Fraymakers Haxe API translation table
│   ├── entity_gen.rs       Builds the Fraymakers .entity JSON (~1700 LOC)
│   ├── haxe_gen.rs         Top-level output generator — writes the whole package
│   ├── fraytools_project.rs  Emits the `<name>.fraytools` project file
│   ├── uuid_gen.rs         Deterministic UUID v5 generation
│   └── bin/                17 diagnostic / reverse-engineering binaries
├── docs/
│   └── anim_split_rules.json   Data table consumed by anim_splitter.rs
├── SSF2ConverterApp/       Native macOS SwiftUI GUI wrapper
│   ├── Package.swift
│   └── Sources/SSF2ConverterApp/{App.swift, ContentView.swift}
├── characters/             Converter OUTPUT (git-ignored; sample runs present)
└── target/                Cargo build output (git-ignored)
```

**Where the test inputs live.** The `.ssf` input files are *not* in this repo
(`.gitignore` excludes `*.ssf`). They sit in a **sibling directory**:

```
~/.openclaw/workspace-main/
├── ssf2-fraymakers-converter/   ← this repo
└── ssf2-ssfs/                   ← 47 SSF2 character .ssf files + misc.ssf
```

`ssf2-ssfs/` contains the full SSF2 roster (`mario.ssf`, `fox.ssf`,
`link.ssf`, `bowser.ssf`, … 46 characters) plus **`misc.ssf`** (the shared file
that holds costume palette data). A fresh checkout on another machine will need
those files supplied separately.

---

## 3. Setup, build & run

### 3.1 Prerequisites

- **Rust** (stable toolchain) — the only requirement for the core converter.
- **Swift 5.9 + Xcode command-line tools** — only if you want to build the
  macOS GUI app. Core conversion does not need it.
- **macOS 13+** — only for the GUI app. The Rust converter itself is
  platform-agnostic (it has been developed and run on macOS).

There is **no external runtime dependency** — SWF parsing/decompression,
bitmap decoding and audio remuxing are all done in-process by Rust crates.
(Historically the project shelled out to JPEXS/FFDec; that is gone.)

### 3.2 Build the converter

```bash
cargo build --release
```

This produces, in `target/release/`:

- **`ssf2_converter`** — the main CLI (this is `src/main.rs`; the binary takes
  the package name from `Cargo.toml`).
- 17 **diagnostic binaries** (one per file in `src/bin/`) — see [§5.6](#56-diagnostic-binaries-srcbin).

### 3.3 Run the converter (CLI)

```
ssf2_converter <FILE.ssf> [OPTIONS]

Arguments:
  <FILE>                 Path to the .ssf file to convert

Options:
  -o, --output <DIR>     Output directory            [default: ./characters]
  -n, --name <NAME>      Character-name override. For a multi-character .ssf,
                         selects only that character. Auto-detected if omitted.
      --misc-ssf <FILE>  Path to misc.ssf for costume palettes.
                         Auto-detected next to the input file if omitted.
  -v, --verbose          Debug-level logging
```

Examples:

```bash
# Convert mario; costumes auto-loaded from ssf2-ssfs/misc.ssf next to it
./target/release/ssf2_converter ../ssf2-ssfs/mario.ssf

# Explicit output dir + explicit misc.ssf
./target/release/ssf2_converter ../ssf2-ssfs/fox.ssf \
    --output ./characters --misc-ssf ../ssf2-ssfs/misc.ssf
```

Output for character `mario` lands in `./characters/mario/` — a complete
FrayTools character package (see [§7](#7-output-format-fraymakers-character-package)).

### 3.4 Quick rebuild loop

`rebuild-sandbag.sh` is the inner dev loop: it rebuilds the release binary and
re-converts `sandbag.ssf`, then prints a sprite count.

```bash
./rebuild-sandbag.sh
```

> ⚠️ **Hardcoded path.** The script has an absolute path baked in:
> `/Users/jimmy/.openclaw/workspace-main/ssf2-ssfs/sandbag.ssf`. If the repo
> moves, edit that line. `sandbag` is the standard smoke-test character
> (small, simple) — keep using it for fast iteration.

### 3.5 Build & run the macOS GUI app

```bash
./build-app.sh
```

This script: (1) `cargo build --release`, (2) `swift build -c release` in
`SSF2ConverterApp/`, (3) assembles `SSF2ConverterApp/build/SSF2ConverterApp.app`
with an `Info.plist` and **both** executables inside `Contents/MacOS/`
(the SwiftUI app *and* the `ssf2_converter` binary it shells out to), then
launches it.

The GUI (`SSF2ConverterApp/Sources/SSF2ConverterApp/`) is intentionally thin:

- `App.swift` — `@main` SwiftUI `App` + an `AppDelegate` that forces a normal
  foreground window.
- `ContentView.swift` — a drag-and-drop / file-picker window. On drop it runs
  the bundled `ssf2_converter` binary as a child `Process`, animates a fake
  progress bar, and shows success/failure with the captured log. It auto-detects
  a sibling `misc.ssf` and lets the user override the output dir.

The GUI adds **no conversion logic** — all behaviour lives in the Rust binary.

### 3.6 README / AGENT_CONTEXT discrepancies

Both older docs mention an **`extract_costumes`** standalone binary and a
two-step "extract costumes first, then convert" workflow. **That binary no
longer exists.** Costume extraction is now done **in-process** by `main.rs`
(`extract_costumes_to_temp`), which reads `misc.ssf`, caches costume data to a
temporary `.ssf2_costumes_cache.json`, and deletes it after the run. The only
costume-related binary still present is the diagnostic `dump_costumes`.

---

## 4. The conversion pipeline

End to end, one `ssf2_converter` invocation does this (`src/main.rs`):

```
 .ssf file
    │
    ▼
[1] ssf::decompress              .ssf → raw SWF bytes
    │                            (.ssf = 8-byte header + zlib payload, OR a raw SWF)
    ▼
[2] swf_parser::parse            SWF → { version, symbols (id→class name),
    │                                    abc_blocks (raw ABC bytecode) }
    ▼
[3] detect_char_names            Scan ABC for `XxxExt` classes (MarioExt, FoxExt…)
    │                            → list of character ids to process.
    │                            (--name overrides; filename is last-resort fallback)
    │
    ├─ (once) extract_costumes_to_temp:
    │     misc.ssf → ssf::decompress → swf_parser::parse →
    │     abc_parser::scan_all_costume_methods → temp costumes JSON
    │
    ▼  for each character id:
process_character():
    │
[4] extractor::extract           ABC → CharacterData {
    │                              attacks + hitboxes, stats, decompiled
    │                              Ext-method scripts, frame scripts,
    │                              ssf2→fm animation-name map }
    │                            (delegates to abc_parser + decompiler)
    ▼
[5] sprite_parser::extract_xframe_scale     root MovieClip → base scaleX/scaleY
    │
[6] sprite_parser::parse_sprite_boxes       SWF timelines → per-animation,
    │                                       per-frame collision-box geometry
    ▼
[7] image_extractor::extract_images         SWF bitmaps → PNG files + per-frame
    │                                       image PLACEMENT (matrix) data
    ▼
[8] sound_extractor::extract_all_sounds     SWF audio → library/sounds/*.ogg
    │
[9] image_extractor::discover_projectiles_and_head
    │                                       find projectile sprites + menu head
    ▼
[10] haxe_gen::generate          Writes the ENTIRE output package:
       ├─ HitboxStats.hx / CharacterStats.hx / AnimationStats.hx / Script.hx
       │   (+ .hx.meta sidecars)
       ├─ anim_splitter::split_animations  (jab→jab1/2/3, taunt→3 slots, …)
       ├─ entity_gen::generate_entity      → library/entities/Character.entity
       ├─ <name>.fraytools  (fraytools_project)   + library/manifest.json
       ├─ entity_gen::get_image_meta_guids → a .meta sidecar per sprite PNG
       ├─ palette_gen::generate_palettes_and_remap
       │     → costumes.palettes(+.meta), palette_preview.png,
       │       then REWRITES Character.entity with paletteMap filled in
       ├─ menu.entity        (from the discovered head sprite)
       ├─ <projectile>.entity + Projectile_<name>/*.hx scripts (per projectile)
       └─ sound content entries + conversion_stats.json
```

Two sub-systems are worth calling out:

- **ABC path** (`abc_parser` + `decompiler`): SSF2 keeps character *logic*,
  *stats*, *attack tables* and *costume data* inside AS3 bytecode. `abc_parser`
  is a from-scratch ABC (AVM2) parser; `decompiler` reconstructs control flow
  (a CFG with proper if/else/while) and renders Haxe-like source. `api_mappings`
  then rewrites SSF2 API calls into Fraymakers API calls.

- **SWF-timeline path** (`sprite_parser` + `image_extractor`): SSF2 keeps all
  *visual* and *collision* data in the SWF display list (`DefineSprite`
  timelines of `PlaceObject`/`ShowFrame`/`RemoveObject` tags), **not** in code.
  These two modules walk those timelines frame-by-frame.

`haxe_gen::generate` is the real orchestrator of *output* — `main.rs` only
prepares the inputs and hands everything to it.

---

## 5. Module-by-module reference

Sizes are approximate (Rust LOC). Modules are grouped by pipeline role.

### 5.1 Entry point & wiring

#### `main.rs` (~305 LOC)
CLI definition (`clap`), logging setup, and the top-level orchestration in
`fn main` + `process_character`. Key functions:
- `extract_costumes_to_temp(misc_ssf)` — extracts every character's costume data
  from `misc.ssf` into a temp JSON cache; drops noise (`unknown` key, <10
  costumes).
- `detect_char_names(swf, input)` — finds every `XxxExt` ABC class and lowercases
  the prefix; reconciles truncated names against the filename (`CaptainExt` +
  `captainfalcon.ssf` → `captainfalcon`).
- `process_character(...)` — runs pipeline steps [4]–[10] for one character;
  every stage is wrapped so a failure logs a warning and continues with a
  default rather than aborting the whole run.

#### `lib.rs` (17 LOC)
Just `pub mod` declarations. Exists so the 17 binaries in `src/bin/` can
`use ssf2_converter::*`. The crate is **both** a library and a binary.

### 5.2 SSF / SWF layer

#### `ssf.rs` (~73 LOC)
`decompress(data) -> Vec<u8>`: turns a `.ssf` into raw SWF bytes. A `.ssf` is
either already a raw SWF (`FWS`/`CWS`/`ZWS` magic — passed through) or an
SSF-wrapped file: `u32 swf_len` + `u32 garbage_header_size` + zlib payload.
Falls back gracefully if the payload is already uncompressed.

#### `swf_parser.rs` (~67 LOC)
`parse(data) -> SwfFile`: thin wrapper over the **`swf` crate** (Ruffle's SWF
library). Calls `swf::decompress_swf` + `swf::parse_swf`, then collects the
`SymbolClass` table (`id → class name`) and every `DoAbc`/`DoAbc2` block's raw
bytecode into `SwfFile { version, frame_count, frame_rate, symbols, abc_blocks }`.

### 5.3 ABC (ActionScript bytecode) layer

#### `abc_parser.rs` (~2200 LOC — largest module)
A complete AVM2/ABC parser written from scratch. Parses the ABC constant pool
(ints, uints, doubles, strings, namespaces, multinames), `Method`s, `Class`es,
`Trait`s, `Script`s and `MethodBody`s into `AbcFile`. Beyond plain parsing it
contains a lot of *semantic* extraction tuned to SSF2's code shape:
- `extract_character(abc, char_name)` — the main entry: pulls the character's
  attacks, stats, frame scripts and the **xframe map** (frame-method → SSF2
  animation name).
- `extract_attack_objects` / `extract_hitboxes_from_val` — recovers attack
  tables by interpreting bytecode with a small stack machine (`StackVal`).
- `extract_stats_from_body` / `extract_ssf2_stats` /
  `extract_largest_numeric_object` — three strategies (in fallback order) for
  finding the character's stat object.
- `extract_costume_data*` / `scan_all_costume_methods` / `decode_costume_objects`
  — recovers the 15-costume palette tables from `misc.ssf`.
- `extract_xframe_name` / `is_root_xframe_method` — maps internal frame methods
  to animation labels.

#### `decompiler.rs` (~1700 LOC)
Turns ABC method bytecode into readable, Haxe-ish source. This is a real
decompiler, not a disassembler:
- `build_blocks` — splits bytecode into basic blocks with `Terminator`s.
- `BlockDecoder` / `StructuredDecoder` — reconstruct a CFG and recover
  structured control flow (`if`/`else`/`while`, branch inversion).
- `Expr` / `Stmt` ASTs + `render_stmts` / `render_closure` — pretty-printing.
- `infer_activation_slots` / `slots_from_traits` / `rename_loop_counters` /
  `rename_locals_in_*` — give locals meaningful names.
- `lookup_api` — a small inline SSF2→FM API table used during decompilation
  (distinct from the larger `api_mappings` table; several entries are stubbed
  `/* TODO */`).
- `decompile_method(...)` — public entry.

### 5.4 Extraction layer

#### `extractor.rs` (~419 LOC)
The bridge between the raw ABC data and the generator. Defines the central
**`CharacterData`** struct (`attacks`, `stats`, `animations`, `scripts`,
`ssf2_to_fm_anim`) and the supporting `Attack`/`Hitbox`/`CharacterStats`/
`AnimationInfo`/`ScriptInfo` types.
- `extract(swf, char_name)` — drives `abc_parser`, converts the results.
- `build_ssf2_to_fm_anim` — the **static SSF2→Fraymakers animation-name table**
  (`stand`→`idle`, `a_air_forward`→`aerial_forward`, `b_down`→`special_down`, …;
  ~90 entries). Edit this table to fix animation-name mapping.
- `convert_hitboxes` / `convert_stats` — map SSF2 field names to FM fields
  (e.g. SSF2 `weight1`/`norm_xSpeed`/`max_ySpeed` → FM weight/walk/fall).
- `expand_split_anim` / `is_split_sub_anim` — declare which FM anims split into
  sub-anims (`jab`→`jab1..jab4`, `taunt`→`taunt`/`taunt_up`/`taunt_down`).

#### `anim_splitter.rs` (~450 LOC)
`split_animations(animations, sprite_boxes) -> Vec<SplitAnim>`: some SSF2 sprites
pack several Fraymakers moves into one timeline separated by internal frame
labels (the classic case: a "Jab" sprite contains jab1→jab2→jab3). This module
decides where to cut, driven by `docs/anim_split_rules.json`.

#### `sprite_parser.rs` (~1280 LOC)
Extracts **collision-box geometry** from SWF `DefineSprite` timelines (SSF2
stores all box data in the display list, never in code).
- `BoxType` enum + `parse_sprite_boxes` — per-animation, per-frame boxes
  (`FrameBox` / `AnimationBoxData`).
- `extract_xframe_transforms` / `extract_xframe_scale` / `XframeTransform` —
  recover the root-MovieClip transform (origin offset + character scale).
- `extract_ssf2_anim_name` / `static_ssf2_to_fm` / `normalize_anim_label` —
  animation-name resolution at the sprite level.
- `find_collision_box_base_size` — measures the `CollisonBox_6` shape's true
  size (note the SSF2-internal typo "Collison").
- `matrix_to_box` / `matrix_to_itembox` — turn a `PlaceObject` matrix into a
  Fraymakers box `(x, y, w, h[, pivot])`.
- `split_jab` / `split_taunt` / `sub_anim_image_splits` — sprite-level splitting
  that mirrors `anim_splitter`.

#### `image_extractor.rs` (~1340 LOC)
Extracts the **visual sprites** and their **per-frame placement**.
- `extract_images(...)` — decodes every `DefineBitsLossless`/`DefineBitsJpeg3`
  bitmap to a PNG (`decode_lossless` / `decode_jpeg3` / `write_png`); builds
  `shape_to_bitmap` (resolves `DefineShape`→bitmap, skipping the SWF null
  bitmap id `65535`) and `shape_pivot` (where a shape's local origin sits inside
  its bitmap — computed from the fill matrix).
- `build_anim_frame_images` — walks each animation timeline and records, per
  frame, every placed image with its full world-space matrix (`FrameImageEntry`).
- `ImageLocalMatrix` — SWF matrix decomposition (`from_abcd`): scale, rotation,
  and **flip encoded as a negative `sy`**; `has_skew()` detects non-rigid
  matrices.
- `prerender_skewed_frames` — pre-renders skewed image placements (a skew can't
  be represented by FrayTools' scale+rotation, so the pixels are baked).
- `discover_projectiles_and_head` — locates projectile sprites and the menu
  head sprite; `extract_projectile_frame_images` flattens nested effect sprites.

#### `sound_extractor.rs` (~330 LOC)
`extract_all_sounds(...)`: pulls `DefineSound` data out of the SWF and writes
`.ogg` files. Handles Nellymoser and MP3 by wrapping the raw audio in a
synthetic **FLV** container and remuxing (`build_nellymoser_flv` /
`build_generic_flv` / `convert_via_flv`).

### 5.5 Generation layer

#### `haxe_gen.rs` (~1290 LOC)
The **output orchestrator**. `generate(...)` writes the entire character package
(see pipeline step [10]). Also owns:
- The **SSF2→FM stat scaling** functions (`ssf2_gravity_to_fm`,
  `ssf2_speed_to_fm`, `ssf2_jump_to_fm`, …) — *approximate* conversions reverse-
  engineered by comparing template characters to SSF2 data.
- `generate_hitbox_stats` / `generate_character_stats` / `generate_animation_stats`
  / `generate_script` — the four character `.hx` files.
- `generate_jab_scripts` — synthesizes jab-chain logic.
- `generate_manifest` — `library/manifest.json`.
- `generate_projectile_*` — projectile `.hx` files (logic is largely **stubbed**;
  see [§9](#9-known-issues--gaps)).
- `generate_sound_entries`, `script_meta`, `sanitize_entity_name`.

#### `entity_gen.rs` (~1700 LOC)
Builds the Fraymakers `.entity` JSON — the heart of a FrayTools character.
- `generate_entity` / `generate_entity_with_palette` — the main `Character.entity`
  builder (with/without `paletteMap`).
- `generate_menu_entity` — the character-select head entity.
- `generate_projectile_entity` (+ `ProjectileInfo` / `ProjectileStateData`) —
  per-projectile entities, including multi-state projectiles (e.g. `link_bomb`).
- `generate_meta` / `get_image_meta_guids` — sprite `.meta` sidecars.
- `box_type_to_fm` / `ssf2_box_name_to_fm` / `fm_box_index` / `box_color` —
  the SSF2-box-name → Fraymakers-layer mapping (see table in `AGENT_CONTEXT.md`).
- `uuid` / `det_uuid` — deterministic GUID helper used throughout the entity.
- ⚠️ **This file holds the active uncommitted work** — the new "spinning vs
  direct" image-placement logic. See [§8](#8-current-status--what-works) /
  [§9](#9-known-issues--gaps).

#### `palette_gen.rs` (~410 LOC)
`generate_palettes_and_remap(...)`: builds Fraymakers costume palettes.
- `load_ssf2_costumes` / `build_from_ssf2` — preferred path: uses the real
  `misc.ssf` costume colour tables (15 costumes × 76 colour slots).
- `build_from_sprites` / `kmeans` / `rotate_hue` — fallback when no `misc.ssf`:
  derives a palette from the sprite pixels via k-means and synthesizes
  team-colour variants by hue rotation.
- `build_output` — writes `costumes.palettes`, the `.meta`, and re-indexes the
  sprite PNGs against the palette; `argb_to_fm_hex` for colour formatting.

#### `api_mappings.rs` (~920 LOC)
The big **SSF2 AS3 → Fraymakers Haxe** translation table + the rewriter that
applies it to decompiled script text.
- `build_method_map` / `build_property_map` / `build_state_map` /
  `build_event_map` / `build_hitbox_prop_map` — the lookup tables
  (`MethodMapping`, `ArgTransform`).
- `translate_ssf2_to_fm(code)` — rewrites a block of decompiled code.
- `remove_readiness_guards`, `fix_intangibility_pairs`,
  `strip_last_frame_end_animation`, `comment_out_unknown_calls` — cleanup passes.
  `comment_out_unknown_calls` is what produces the `/* unknown */`-style markers
  in generated scripts.

#### `fraytools_project.rs` (~60 LOC)
`generate_fraytools_project(char_name)` — emits the small `<name>.fraytools`
project descriptor FrayTools opens.

#### `uuid_gen.rs` (~80 LOC)
`det_uuid(seed)` — RFC-4122 **UUID v5** (SHA-1 namespace). Every GUID in the
output is `det_uuid("{char_id}::{context}")`, which is what makes conversions
reproducible.

### 5.6 Diagnostic binaries (`src/bin/`)

These are **reverse-engineering / debugging tools**, not part of the conversion.
Each builds to its own executable in `target/release/`. They were the workbench
used to figure the formats out and remain useful when a conversion looks wrong.

| Binary | Purpose |
|---|---|
| `dump_sprites` | List every `DefineSprite` symbol in a SWF |
| `dump_images` | List all extracted bitmaps |
| `dump_image_placement` | Per-frame `PlaceObject` data for a named sprite |
| `dump_collision_box` | Collision-box geometry for an animation |
| `dump_shape_bounds` | Measure the true bounds of the `CollisonBox_6` shape |
| `dump_shape_origins` | Shape origin offsets |
| `dump_pivots` | Pivot points |
| `dump_frame_labels` | Frame labels inside a sprite timeline |
| `dump_raw_frame` | Raw tag dump for one frame |
| `dump_inner_sprite` | Inspect a nested (effect) sprite |
| `dump_proj_states` | Projectile state discovery |
| `dump_stage` | Stage / root timeline inspection |
| `dump_costumes` | Costume tables from `misc.ssf` |
| `dump_aerial_down_frames` | Targeted debug for the aerial-down animation |
| `dump_trail_matrices` | **New (untracked)** — trail-effect matrices; created for the in-progress spinning/trail placement work |
| `check_shape_bitmap` | Inspect a shape's bitmap fill + fill matrix |
| `what_is_id` | Identify what a numeric SWF character id refers to |

Typical use: `./target/release/dump_image_placement ../ssf2-ssfs/mario.ssf "FAir_42"`.

---

## 6. Input format: SSF2 `.ssf`

Summary of what the code assumes — full detail in
[`AGENT_CONTEXT.md`](AGENT_CONTEXT.md).

- A `.ssf` file is a **renamed/SSF-wrapped SWF (Flash)**. `ssf.rs` unwraps it.
- **Character logic, stats, attack tables and costume data live in AS3 bytecode**
  (`DoABC` tags) — parsed by `abc_parser`, decompiled by `decompiler`.
- **Sprites, animation timelines and collision boxes live in the SWF display
  list** — `DefineBitsLossless`/`DefineShape`/`DefineSprite` tags with
  `PlaceObject`/`ShowFrame`/`RemoveObject` — parsed by `sprite_parser` /
  `image_extractor`.
- Each character has an **`XxxExt` AS3 class** (`MarioExt`, `FoxExt`, …) — this
  is the signal `detect_char_names` keys on.
- Animation sprites are named `{char}_fla.{AnimLabel}_{index}` (e.g.
  `mario_fla.FAir_42`).
- Collision boxes are a 100×100 unit square (`CollisonBox_6` — note SSF2's
  internal typo) scaled/positioned by a `PlaceObject` matrix; box *type* comes
  from the instance name (`attackBox`→hitbox, `hitBox`→hurtbox, `grabBox`,
  `touchBox`, `ledgeBox`, `reflectBox`, …).
- SWF matrices use fixed-point components; translations are in **twips**
  (÷20 = pixels). Both SSF2 and Fraymakers use **y-down** screen coordinates,
  so no vertical flip is needed.
- Costume data is in `misc.ssf` → `Misc.as` → `getCostumeData()`: 15 costumes
  per character (Red/Green/Blue/Default + 11 alts), each with 76 source colours
  and 76 replacements.

## 7. Output format: Fraymakers character package

The converter writes a directory FrayTools can open. For character `mario`:

```
characters/mario/
├── mario.fraytools                     FrayTools project file
├── conversion_stats.json               debug summary of the run
└── library/
    ├── manifest.json
    ├── costumes.palettes  (+ .meta)     15 SSF2 costumes as FM palettes
    ├── entities/
    │   ├── Character.entity             main entity (animations/layers/keyframes/symbols)
    │   ├── menu.entity                  character-select head
    │   └── <projectile>.entity          one per discovered projectile
    ├── sprites/
    │   ├── *.png                        extracted frame bitmaps
    │   ├── *.png.meta                   GUID sidecar per PNG (entity refs the GUID)
    │   └── palette_preview.png (+ .meta)
    ├── scripts/
    │   ├── Character/
    │   │   ├── CharacterStats.hx         movement physics
    │   │   ├── HitboxStats.hx            per-attack hitbox data
    │   │   ├── AnimationStats.hx         animation flags
    │   │   ├── Script.hx                 decompiled character logic
    │   │   └── *.hx.meta
    │   └── Projectile_<name>/            one folder per projectile
    │       └── Projectile{Script,Stats,HitboxStats,AnimationStats}.hx (+ .meta)
    └── sounds/
        └── *.ogg
```

`.entity` files are JSON. The top-level shape is `{ animations[], layers[],
keyframes[], symbols[], paletteMap, pluginMetadata, plugins, version: 14, … }`.
Each animation owns an ordered layer stack — `LABEL`, `FRAME_SCRIPT`,
`COLLISION_BODY`, one `COLLISION_BOX`/`POINT` per box instance, one `IMAGE` per
depth slot — and layers reference keyframes which reference symbols. The full
schema (every layer/symbol/keyframe shape, the box-type enum, `.meta` sidecar
format) is documented in **`AGENT_CONTEXT.md` §"Fraymakers Entity Format"** and
is the ground truth — keep it open when working on `entity_gen.rs`.

---

## 8. Current status — what works

**The pipeline runs end to end and produces complete, FrayTools-shaped
character packages.** Evidence in the repo:

- `characters/mario/` — a full conversion: **858** sprite PNGs (each with a
  `.meta` sidecar), a **~4 MB** `Character.entity`, `menu.entity`, two
  projectile entities (`mario_fireball`, `mario_finale_projectile`) with their
  script folders, palettes, sounds.
- `characters/sandbag/` — the smoke-test character.
- Sibling `../test_chars/` holds additional converted outputs for `fox`,
  `link`, `naruto`, `jigglypuff`, `samus`, plus helper entities
  (`cstate_fraymakers`, `displayobject_fraymakers`, `object_fraymakers`).
- 47 SSF2 inputs are available in `../ssf2-ssfs/`.

What is solid:

- `.ssf`→SWF decompression, SWF parsing, ABC parsing.
- Character / stats / attack / frame-script extraction from ABC.
- The AS3 decompiler (CFG reconstruction, structured control flow).
- Bitmap extraction → PNG, sprite `.meta` generation.
- Collision-box geometry extraction.
- Costume/palette extraction from `misc.ssf` (with k-means fallback).
- Sound extraction to OGG.
- Deterministic GUIDs.
- Entity JSON assembly (layers/keyframes/symbols), manifest, `.fraytools`,
  projectile & menu entities.
- The macOS SwiftUI GUI wrapper.

### What the last agent was working on (uncommitted)

`git status` shows **8 commits ahead of `origin/main`** and **uncommitted
changes** in 4 tracked files plus 1 new untracked file. The work in flight is
**image-placement geometry** — getting sprites to sit and rotate correctly in
the entity:

- `src/image_extractor.rs` — matrix decomposition now **encodes mirror/flip as a
  negative `sy`** (instead of dropping the sign); `has_skew()` updated to match;
  new `shape_pivot` map (where a shape's local origin lands inside its bitmap,
  derived from the fill matrix); new `anim_origin_x/y` fields on
  `FrameImageEntry` (world-space rotation centre).
- `src/entity_gen.rs` — image placement now picks between **two modes**:
  *spinning* (rotation varies a lot while the sprite's world position stays put
  → pin the rotation centre to `anim_origin` via an inverse-matrix pivot) and
  *direct* (use the fill-matrix `shape_pivot`). This is a heuristic
  (`rot_range > 30° && pos_spread < 20px && ≥4 frames`).
- `src/bin/check_shape_bitmap.rs` — now also prints the fill matrix.
- `src/bin/dump_trail_matrices.rs` — **new, untracked** — a probe for the
  trail/spinning-effect case.
- `src/main.rs` — one line: passes the new `shape_pivot` field through.

This work is **mid-stream**: it compiles-clean intent is there, but the
spinning/direct heuristic is unverified across the roster and is the immediate
thing to finish (see [§10](#10-prioritized-next-steps)). Recent committed
history (last ~9 commits) is all in the same area — rotation pivot, skew
pre-rendering, rotation convention — so image placement is the project's
current frontier.

---

## 9. Known issues & gaps

1. **Image placement geometry is not fully solved.** The committed history
   churns on rotation/pivot/skew, and the uncommitted "spinning vs direct"
   heuristic is a work in progress. Expect some animations to have sprites
   offset, mis-rotated, or mis-pivoted until this is finished and verified.

2. **`AGENT_CONTEXT.md` is stale on rotation.** It says *"always negate rotation
   when writing to the entity"*. That is **still true for collision boxes**
   (`entity_gen.rs` writes `rotation: -fb.rotation`) but **no longer true for
   images** — commit `f472a2dd` ("don't negate rotation — SWF and FrayTools use
   same CW convention") changed image rotation to a non-negated, 0–360-normalized
   value. Trust the code over `AGENT_CONTEXT.md` here.

3. **Vector-only effect sprites are silently skipped.** Effects that are pure
   vector shapes with solid-colour fills (e.g. `mario_fla.ChargeSpark_25`, the
   F-air twinkle) cannot be rasterized without a full SWF vector renderer. Only
   bitmap-backed shapes are exported, so some effects are visually missing.

4. **`grabHoldPoint` / `touchBox` format unverified.** SSF2 `touchBox` marks
   where a grabbed opponent is held. It is currently emitted as a
   `COLLISION_BOX`, but FrayTools likely expects a `POINT` layer
   (`pointType: "GRAB_HOLD_POINT"`). Needs a reference FrayTools entity to
   confirm. (`AGENT_CONTEXT.md` documents both the current and the suspected-
   correct form.)

5. **Frame-script / API translation is incomplete.** `api_mappings.rs` and
   `decompiler.rs` have many SSF2 calls with no Fraymakers equivalent — these
   are emitted as `/* TODO */` or commented out by `comment_out_unknown_calls`.
   Generated `.hx` always needs human review.

6. **Projectile logic is stubbed.** `generate_projectile_script` /
   `generate_projectile_stats` emit structure with `// TODO: implement state
   logic` / `// TODO: tune X_SPEED/Y_SPEED` placeholders. Projectile *entities*
   (visuals, boxes, animations) are generated; projectile *behaviour* is not.

7. **Stat scaling is approximate.** The `ssf2_*_to_fm` functions in
   `haxe_gen.rs` are hand-tuned ratios. Generated `CharacterStats.hx` /
   `HitboxStats.hx` deliberately mark uncertain numbers with `/*TODO*/`.

8. **Doc drift.** `README.md` and `AGENT_CONTEXT.md` both reference an
   `extract_costumes` binary and a separate costume-extraction step that no
   longer exist (costumes are extracted in-process — see [§3.6](#36-readme--agent_context-discrepancies)).

9. **`rebuild-sandbag.sh` has an absolute path** baked in — breaks if the repo
   moves.

10. **`tokio` is a declared dependency but the converter is synchronous** —
    `main.rs` has no async. It is likely vestigial; verify before relying on it,
    and consider removing it to cut build time.

11. **Robustness.** `process_character` swallows per-stage errors and continues
    with defaults — good for batch runs, but means a partly-broken character can
    be produced without an obvious failure. Check `conversion_stats.json` and
    the warning log after a run.

---

## 10. Prioritized next steps

Roughly in the order a fresh agent should tackle them.

1. **Finish & commit the image-placement work.** Decide whether the
   "spinning vs direct" heuristic in `entity_gen.rs` is the right model, verify
   it against several characters in FrayTools (sandbag first — `rebuild-sandbag.sh`
   — then mario aerials and a trail-effect move), tune the
   `rot_range / pos_spread / frame-count` thresholds, then commit. Use the new
   `dump_trail_matrices` binary to inspect cases. This unblocks everything
   visual.
2. **Verify the build is green** with the uncommitted changes: `cargo build
   --release` and a sandbag conversion. The new `shape_pivot` field is threaded
   through `main.rs` and `entity_gen.rs` — confirm no other `ImageExtractionResult`
   constructor was missed.
3. **Reconcile `AGENT_CONTEXT.md` with the code** — fix the rotation section
   (collision boxes negate, images don't) and remove the dead `extract_costumes`
   references from both `AGENT_CONTEXT.md` and `README.md`.
4. **Confirm the `grabHoldPoint` format.** Get a reference FrayTools character
   that has a grab-hold point, compare, and switch `touchBox` output to a
   `POINT` layer if needed.
5. **Vector effect sprites.** Either integrate a minimal SWF shape rasterizer or
   formally document which effects are dropped so users aren't surprised.
6. **Projectile behaviour.** Replace the `// TODO` stubs in the projectile `.hx`
   generators with real translated logic (reuse `decompiler` + `api_mappings`).
7. **Validate stat scaling** against a handful of hand-tuned reference
   characters and tighten the `ssf2_*_to_fm` ratios.
8. **Batch-convert the full 47-character roster** from `../ssf2-ssfs/`, triage
   which characters convert cleanly, and capture a per-character status list.
9. **Housekeeping:** make `rebuild-sandbag.sh` path-relative; drop unused
   `tokio`; consider committing or `.gitignore`-ing `dump_trail_matrices.rs`.

---

## 11. Git state & history

- **Branch:** `main`, tracking `origin/main`. **8 local commits ahead**,
  not pushed.
- **Uncommitted:** modified `src/entity_gen.rs`, `src/image_extractor.rs`,
  `src/main.rs`, `src/bin/check_shape_bitmap.rs`; untracked
  `src/bin/dump_trail_matrices.rs`. (All the image-placement work — [§8](#8-current-status--what-works).)
- **History:** 109 commits. The project began as a **Python** script
  (`convert.py`, "47 SSF2 characters ported"), was **rewritten in Rust** with a
  from-scratch native SWF/ABC parser, then gained the **AVM2 decompiler**, then
  the **SwiftUI app**, and most recently a long run of **image
  rotation / pivot / skew** fixes — which is exactly where the uncommitted work
  continues.

> **Do not commit or push** as part of documentation work. Leave the working
> tree as-is unless Jimmy explicitly asks. These docs are intentionally left
> uncommitted too.

---

## 12. Tips for the next agent

- **Smoke test = `sandbag`.** `./rebuild-sandbag.sh` is the fast loop. `mario`
  is the heavy, full-featured test (projectiles, costumes, big roster of moves).
- **`AGENT_CONTEXT.md` is the format bible** — but cross-check it against the
  code for rotation (see [§9.2](#9-known-issues--gaps)). It is otherwise
  detailed and reliable.
- **Reach for the `dump_*` binaries** before guessing. They exist precisely so
  you can inspect a SWF without re-deriving the format. Example:
  `./target/release/dump_collision_box ../ssf2-ssfs/mario.ssf "a_air_forward"`.
- **Inputs are the sibling `../ssf2-ssfs/` folder**, not in this repo
  (`.gitignore` excludes `*.ssf`). `misc.ssf` lives there too and is needed for
  real costume colours.
- **Output is git-ignored** (`characters/`). Deleting it and re-running is safe.
- **GUIDs are deterministic** — re-running the converter is idempotent w.r.t.
  GUIDs, so diffs between runs reflect real logic changes, not GUID churn.
- **Reference resources** (from `AGENT_CONTEXT.md`): the official
  [Fraymakers character-template](https://github.com/Fraymakers/character-template)
  repo's `library/entities/character.entity` is ground truth for the entity
  format; the [Fraymakers community docs](https://github.com/aJewelofRarity/FraymakersDocs)
  and [SSF2 modding docs](https://ssf2-modding.readthedocs.io/) cover the rest.
- **The pipeline is fail-soft.** A stage that errors logs a warning and
  continues — always scan the run log and `conversion_stats.json`, don't assume
  a non-crashing run was a clean run.
