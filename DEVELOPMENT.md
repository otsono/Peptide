# SSF2 → Fraymakers Converter — Developer Guide

> **Audience:** a developer or AI agent picking up this project cold.
> This document explains what the project is, how to build and run it, how the
> conversion pipeline is wired together, what every module does, what currently
> works, what is unfinished, and what to do next.
>
> **Companion docs (all top-level):**
> - [`AGENT_CONTEXT.md`](AGENT_CONTEXT.md) — the authoritative *format* reference
>   (SSF2 `.ssf`/SWF internals, the Fraymakers `.entity` JSON schema, and how
>   FrayTools renders what we emit). Read it alongside this file.
> - [`README.md`](README.md) — short user-facing summary.
> - [`TESTING.md`](TESTING.md) — the two validation harnesses (FrayTools-side +
>   Fraymakers-engine-side), the end-to-end iteration loop, the engine RE map, and
>   in-engine validation status.
> - [`CONTRIBUTING.md`](CONTRIBUTING.md) — hot-file → doc-section map + the
>   per-change checklist.
> - The "code-quality backlog" in [§11](#11-known-issues--gaps) collects the
>   still-open optimization / cleanup / bug-audit items (file-and-line refs) for
>   "what should I fix next" work.

---

## Table of contents

1. [What this project is](#1-what-this-project-is)
2. [Repository layout](#2-repository-layout)
3. [Setup, build & run](#3-setup-build--run)
4. [The conversion pipeline](#4-the-conversion-pipeline)
5. [Module-by-module reference](#5-module-by-module-reference)
6. [Mapping files (`mappings/`)](#6-mapping-files-mappings)
7. [30 → 60 fps doubling](#7-30--60-fps-doubling)
8. [Input format: SSF2 `.ssf`](#8-input-format-ssf2-ssf)
9. [Output format: Fraymakers character package](#9-output-format-fraymakers-character-package)
10. [Current status — what works](#10-current-status--what-works)
11. [Known issues & gaps](#11-known-issues--gaps)
12. [Prioritized next steps](#12-prioritized-next-steps)
13. [Tips for the next agent](#13-tips-for-the-next-agent)
14. [Architectural history](#14-architectural-history)

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
| Per-frame collision boxes (hitboxes, hurtboxes, grab/ledge/reflect/absorb/touch boxes…) | → | `COLLISION_BOX` / `COLLISION_BODY` / `POINT` layers in the character entity (`<Pascal>.entity`) |
| Animation timelines | → | `animations` / `layers` / `keyframes` in the character entity |
| AS3 frame scripts (ABC bytecode) | → | `FRAME_SCRIPT` keyframe code in the entity |
| AS3 character logic (`XxxExt` class methods) | → | decompiled into `Script.hx` |
| Character stats (weight, gravity, speeds…) | → | `CharacterStats.hx` (data-driven via `mappings/character/stats.jsonc`) |
| Attack / hitbox data | → | `HitboxStats.hx` (data-driven via `mappings/character/hitbox_stats.jsonc`) |
| Animation flags | → | `AnimationStats.hx` |
| Costume palettes (`misc.ssf`) | → | `costumes.palettes` + entity `paletteMap` |
| Sounds | → | `library/audio/*.wav` + per-sound `.wav.meta` content entries |
| Projectiles | → | `library/entities/<name>.entity` + `library/scripts/Projectile/<Pascal>*.hx` |
| Effects (VFX) | → | `library/entities/<effect>.entity` (one per effect; no scripts/stats); referenced from `Script.hx` via `match.createVfx(…)` |
| Menu / portrait head sprite | → | `Menu.entity` (full / css / icon / hud variants; `<Pascal>_Menu.entity` in multi-char projects) |

The conversion is **fully automatic and deterministic** — every GUID in the
output is derived (UUID v5) from the character id + a context string, so
re-running the converter on the same input reproduces byte-identical GUIDs.

**Scope note:** this is a *data* converter, not a runtime. It does not run SSF2 or
Fraymakers. Translated logic lands in `.hx` files that a human must still review —
many lines are emitted with `/*TODO*/` markers or as `// [SSF2-only: NAME] …`
comments where no equivalent exists. The set of "still SSF2-only" calls per run
is logged to `conversion_log.json`.

---

## 2. Repository layout

```
peptide/  (repo root — Cargo workspace; the `peptide` binary is the product)
├── Cargo.toml                Workspace + the `peptide` package
├── Cargo.lock
├── README.md                 Short user-facing readme
├── AGENT_CONTEXT.md          Authoritative SSF2 / Fraymakers FORMAT reference
├── DEVELOPMENT.md            ← this file (the converter library internals)
├── CONTRIBUTING.md           Hot-file → doc-section map + per-change checklist
├── TESTING.md                Validation harnesses + engine RE map + validation status
├── LICENSE                   MIT License
├── NOTICE.md                 Dependency attribution (Ruffle swf crate, etc.)
├── .gitignore                Ignores *.ssf, *.swf, /target, characters/, engine artifacts
│
├── src/                       Peptide — engine harness, webview UI, bytecode patcher
│   ├── main.rs                CLI dispatch (gui/tui/convert/export/render/harness/patcher) + allocator
│   ├── convert.rs             `peptide convert` adapter → ssf2_converter::run_conversion
│   ├── config.rs              persisted config + env-override resolvers
│   ├── platform.rs            FrayTools/Fraymakers path discovery + publish-folder editing
│   ├── gui.rs                 webview glue (wry/tao): screen router IPC, boot flow
│   ├── peptide_ui.html        the UI (home / setup / converter / frayhook / console)
│   ├── ui.rs / bridge.rs      ratatui TUI + headless TCP runtime + the launcher
│   ├── manifest.rs            engine-symbol dependency table (doctor preflight)
│   ├── fraytools.rs           FrayTools CDP driver (export / render / harness)
│   └── asm.rs / interpreter.rs  opcode helpers + the friendly-command vocabulary
│
├── crates/ssf2-converter/     The SSF2 → Fraymakers converter, as a library crate
│   ├── Cargo.toml             package `ssf2_converter` (lib; dev bins gated by `dev-tools`)
│   ├── src/
│   │   ├── lib.rs             pub mod declarations + run_conversion re-export
│   │   ├── convert.rs         run_conversion(ConvertOptions) — the in-process entry point
│   │   ├── ssf.rs             .ssf → raw SWF decompression
│   │   ├── swf_parser.rs      SWF tag parsing (thin wrapper over the Ruffle `swf` crate)
│   │   ├── abc_parser.rs      AVM2/ABC bytecode parser + semantic extractors (~2500 LOC)
│   │   ├── decompiler.rs      ABC bytecode → Haxe-ish source (~1700 LOC)
│   │   ├── extractor.rs       Bridges abc_parser output into CharacterData
│   │   ├── anim_splitter.rs   Splits multi-move SSF2 sprites into FM animations
│   │   ├── sprite_parser.rs   Per-frame collision-box geometry from SWF timelines
│   │   ├── image_extractor.rs PNG extraction, placement, skew baking, projectile/effect/head
│   │   ├── sound_extractor.rs Audio extraction (Nellymoser / MP3 / ADPCM → WAV via ffmpeg)
│   │   ├── palette_gen.rs      Costume palette generation
│   │   ├── api_mappings.rs     Decompiled-Haxe rewriter pipeline (JSONC-driven, ~1900 LOC)
│   │   ├── mappings.rs         JSONC loader + OnceLock-cached accessors for mappings/*.jsonc
│   │   ├── entity_gen.rs       Fraymakers .entity JSON builder (~2000 LOC)
│   │   ├── haxe_gen.rs         Top-level output orchestrator — writes the whole package
│   │   ├── fraytools_project.rs  Emits the `<name>.fraytools` project file
│   │   ├── uuid_gen.rs         Deterministic UUID v5 generation
│   │   └── bin/               ~30 diagnostic binaries (gated: --features dev-tools)
│   ├── mappings/              ← editable runtime config (JSONC), baked in via include_str!
│   │   ├── commands.jsonc      universal SSF2 → FM API command conversions
│   │   └── character/          animations.jsonc / stats.jsonc / hitbox_stats.jsonc
│   └── tests/                 integration + golden tests
│
├── tools/                     Build + engine-harness orchestration (shell + Node)
│   ├── make-app.sh             macOS: build peptide + wrap it in dist/Peptide.app
│   ├── make-win.sh             Cross-compile the Windows peptide.exe into dist/windows/
│   ├── run.sh / runseq.sh      Boot Fraymakers + send one / a sequence of console commands
│   ├── recipe.sh               Run a shareable .recipe (commands + #!char/#!gap directives)
│   ├── rebuild-sandbag.sh      Quick: rebuild peptide + convert sandbag.ssf
│   ├── tests/                  Test + parity harnesses and golden fixtures
│   │   ├── ab_compare.sh        Capture/diff a golden behavior signature (regression gate)
│   │   ├── batch_spawn_test.sh  Batch export + in-engine spawn-test a set of characters
│   │   ├── parity_check.py      Static SSF2-source-vs-output hitbox-parity diff
│   │   ├── translation_completeness.sh  Per-character untranslated-marker dashboard
│   │   └── recipes/             .recipe scripts + .golden behavior signatures
│   └── fraytools-harness/      Legacy Node FrayTools scripts (reference; superseded by peptide)
├── characters/                Converter OUTPUT (`*.hx`, `*.json`, `*.entity` tracked; media ignored)
└── target/                    Cargo build output (git-ignored)
```

**Where the test inputs live.** The `.ssf` input files are *not* in this repo
(`.gitignore` excludes `*.ssf`). They sit in a **sibling directory**:

```
<workspace>/
├── ssf2-fraymakers-converter/   ← this repo
└── ssf2-ssfs/                   ← 47 SSF2 character .ssf files + misc.ssf
```

So from the repo root the inputs are reachable as `../ssf2-ssfs` — the relative
path the CLI examples, `tools/rebuild-sandbag.sh`, and the `src/bin/` diagnostic
defaults all use.

`ssf2-ssfs/` contains the full SSF2 roster (`mario.ssf`, `fox.ssf`, `link.ssf`,
`bowser.ssf`, …) plus **`misc.ssf`** (the shared file that holds costume palette
data). A fresh checkout on another machine will need those files supplied
separately.

---

## 3. Setup, build & run

### 3.1 Prerequisites

- **Rust** (stable toolchain) — required to build the core converter.
- **`ffmpeg`** on `PATH` — used at runtime for sound conversion (Nellymoser / MP3 /
  ADPCM → WAV). If `ffmpeg` is absent the conversion still completes; sound
  extraction is skipped with a warning.
- The desktop app is the `peptide` binary itself (a system webview) — no extra
  toolchain or OS-specific SDK. The converter itself is platform-agnostic.

There is **no external Rust runtime dependency** for SWF decompression, bitmap
decoding, or ABC parsing — those all happen in-process (`src/ssf.rs`,
`src/swf_parser.rs`, `src/abc_parser.rs`, `src/decompiler.rs`).

### 3.2 Build

```bash
cargo build --release
```

This produces `target/release/peptide` — the single binary (engine harness +
in-process converter + FrayTools driver). The converter is a **library**
(`crates/ssf2-converter`), not its own binary. The ~30 **diagnostic binaries**
live in the converter crate behind the `dev-tools` feature (see
[§5.7](#57-diagnostic-binaries-srcbin)); they are excluded from the default build.

### 3.3 Run the converter (CLI)

Conversion is a Peptide subcommand:

```
peptide convert <FILE.ssf> [OPTIONS]

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

The same conversion runs behind the app's **SSF2 → Fraymakers Converter** screen
and programmatically via `ssf2_converter::run_conversion(ConvertOptions)`.

Costume extraction is **in-process** — `convert.rs::extract_costumes_to_temp`
unwraps `misc.ssf` and runs `abc_parser::scan_all_costume_methods` directly,
writing a temporary JSON cache that's deleted after the run. There is no
separate `extract_costumes` binary anymore.

Examples:

```bash
# Convert mario; costumes auto-loaded from ssf2-ssfs/misc.ssf next to it
./target/release/peptide convert ../ssf2-ssfs/mario.ssf

# Explicit output dir + explicit misc.ssf
./target/release/peptide convert ../ssf2-ssfs/fox.ssf \
    --output ./characters --misc-ssf ../ssf2-ssfs/misc.ssf
```

Output for character `mario` lands in `./characters/mario/` — a complete
FrayTools character package (see [§9](#9-output-format-fraymakers-character-package)).

### 3.4 Quick rebuild loop

`tools/rebuild-sandbag.sh` is the inner dev loop: it rebuilds the release binary and
re-converts `sandbag.ssf`, then prints a sprite count.

```bash
./tools/rebuild-sandbag.sh
```

> **Input path.** The script reads `../ssf2-ssfs/sandbag.ssf` (relative to the
> repo root), so run it from the repo root. `sandbag` is the standard smoke-test
> character (small, simple) — keep using it for fast iteration.

### 3.5 The desktop app

The graphical app **is** `peptide` — running the binary with no arguments opens
the system-webview window (wry/tao; WKWebView on macOS, WebView2 on Windows,
WebKitGTK on Linux). The whole UI is `src/peptide_ui.html`. On first run a
**Setup** screen captures where Fraymakers and FrayTools live and the current
character; after that a **Home** screen offers three buttons:

- **Launch Peptide** — boot the engine and drive a live match (the console).
- **SSF2 → Fraymakers Converter** — run a conversion in-process (a worker thread
  calling `run_conversion`), with a progress bar and a result panel.
- **FrayTools Hook** — publish to `.fra` / render an entity / extract box
  geometry, driving FrayTools over CDP.

The webview glue lives in `src/gui.rs` (see [`docs/PEPTIDE_README.md`](docs/PEPTIDE_README.md)
for the harness internals); there is no separate GUI crate anymore.

**Double-clickable macOS app.** `./tools/make-app.sh` builds the single `peptide`
binary and wraps it in `dist/Peptide.app` — a normal Finder app (name, dock
icon, `.ssf` association) with `peptide` as the bundle executable. It
ad-hoc-codesigns the bundle so Gatekeeper allows a locally-built app to launch,
and opens it on success (`--no-open` to just build). `dist/` is git-ignored.

```bash
./tools/make-app.sh            # build + assemble dist/Peptide.app + launch
./tools/make-app.sh --no-open  # build + assemble only (packaging / CI)
```

**Windows build.** `./tools/make-win.sh` cross-compiles `peptide.exe` into
`dist/windows/` (prefers `cargo-xwin` for the MSVC ABI, falls back to
`mingw-w64`; prints the exact install command if neither toolchain is present),
or build natively on Windows with `cargo build --release`. The webview uses the
WebView2 runtime, which ships with Windows 10/11.

---

## 4. The conversion pipeline

End to end, one `run_conversion` call does this (`crates/ssf2-converter/src/convert.rs`):

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
[3] detect_char_names            Walk `Main`'s constructor:
    │                            `self.register("characters", [self.getX(), …])`
    │                            → list of character ids to process. The id
    │                            comes from each method name (strip `get`,
    │                            lowercase). `Main`'s constructor is the
    │                            SSF's canonical "table of contents".
    │                            Fallback path (defensive, for one release):
    │                            enumerate `Main`'s instance `get*` methods.
    │                            (`--name` overrides; filename is the
    │                            last-resort fallback for misc.ssf-style SWFs.)
    │                            See §5.1 + "Architectural history" below.
    │
    ├─ (once) extract_costumes_to_temp:
    │     misc.ssf → ssf::decompress → swf_parser::parse →
    │     abc_parser::scan_all_costume_methods → temp costumes JSON
    │     (in-process; temp file deleted at end of run)
    │
    ▼  for each character id:
process_character():
    │
    ├─ api_mappings::reset_conversion_log()   start a fresh per-character log
    │
[4] extractor::extract           ABC → CharacterData {
    │                              attacks + hitboxes, stats, decompiled Ext methods,
    │                              frame scripts, ssf2→fm anim-map, ext_vars +
    │                              ext_var_inits, projectile_data (per-projectile
    │                              SSF2 physics + hitboxes from getProjectileStats()) }
    │                            (delegates to abc_parser + decompiler)
    ▼
[5] sprite_parser::extract_xframe_scale     root MovieClip → base scaleX/scaleY
    │
[6] sprite_parser::parse_sprite_boxes       SWF timelines → per-animation,
    │                                       per-frame collision-box geometry
    ▼
[7] image_extractor::extract_images         SWF bitmaps → PNG files + per-frame
    │                                       image PLACEMENT (full affine matrix) data
    │                                       + skew-bake on sheared placements
    ▼
[8] sound_extractor::extract_all_sounds     SWF audio → library/audio/*.wav (via ffmpeg)
    │
[9] image_extractor::discover_projectiles_and_head
    │                                       find projectile sprites, effect sprites,
    │                                       and menu head sprite
    ▼
[10] haxe_gen::generate          Writes the ENTIRE output package:
       │
       ├─ EffectAnimGuard::install(effect→primary-animation map)
       │   so all translate_ssf2_to_fm calls in this scope can resolve
       │   attachEffect("name") → match.createVfx(…) with the right animation.
       │
       ├─ HitboxStats.hx / CharacterStats.hx / AnimationStats.hx / Script.hx
       │   (+ .hx.meta sidecars)
       ├─ anim_splitter::split_animations  (jab→jab1/2/3/4, taunt→3 slots,
       │                                    aerial → active + land, strong → in/charge/attack,
       │                                    grab → grab/dash_grab/grab_hold/grab_pummel, …)
       ├─ entity_gen::generate_entity      → library/entities/<Pascal>.entity
       │   (drops empty animations; jab-count-driven jab keep-list)
       ├─ <project>.fraytools  (fraytools_project)   + library/manifest.json (+ .meta)
       ├─ entity_gen::get_image_meta_guids → a .meta sidecar per sprite PNG
       ├─ palette_gen::generate_palettes_and_remap
       │     → costumes.palettes (+ .meta), palette_preview.png,
       │       then REWRITES <Pascal>.entity with paletteMap filled in
       ├─ entity_gen::generate_menu_entity      from the discovered head sprite
       ├─ for each projectile:
       │     ├─ <proj>.entity   (visuals + boxes + multi-state animations)
       │     └─ library/scripts/Projectile/<Pascal>{Script,Stats,HitboxStats,AnimationStats}.hx
       │        (Annie / aJewelofRarity convention: single Projectile/ dir,
       │        Pascal-cased filename per projectile)
       ├─ for each effect:
       │     └─ <effect>.entity   (image-only; one animation per inner FrameLabel,
       │                          or single `active` if no labels)
       ├─ generate_sound_entries  (per-WAV .meta sidecar)
       └─ conversion_stats.json
    │
[11] write_conversion_log        Snapshot api_mappings::snapshot_conversion_log()
                                 → <char>/conversion_log.json with:
                                   - character (the derived id),
                                   - unknown + ssf2_only counts,
                                   - ssf2_source: { package_id, package_guid,
                                     source_method } (always present when
                                     `Main` exists); adds parent_normal_stats_id
                                     + note for transformations (Giga Bowser,
                                     Wario Man — bundles whose cData.normalStats_id
                                     mismatches the derived id),
                                   - validation_warnings (Tier 1 soft logs:
                                     empty stats/attacks, char_name not in
                                     declared roster, id ≠ filename stem).
                                 Used by the GUI "Unhandled Calls" popup.
```

Three sub-systems are worth calling out:

- **ABC path** (`abc_parser` + `decompiler`): SSF2 keeps character *logic*,
  *stats*, *attack tables*, *projectile stats* and *costume data* inside AS3
  bytecode. `abc_parser` is a from-scratch ABC (AVM2) parser; `decompiler`
  reconstructs control flow (a CFG with proper if/else/while) and renders
  Haxe-like source.

- **SWF-timeline path** (`sprite_parser` + `image_extractor`): SSF2 keeps all
  *visual* and *collision* data in the SWF display list (`DefineSprite`
  timelines of `PlaceObject` / `ShowFrame` / `RemoveObject` tags), **not** in
  code. These two modules walk those timelines frame-by-frame and produce per-
  animation, per-frame, per-layer data structures.

- **Rewriter pipeline** (`api_mappings::translate_ssf2_to_fm`): every block of
  decompiled Haxe (Script.hx ext methods, ext-class iinit, frame scripts) is
  passed through a fixed pipeline of structural and text transforms — see
  [§5.5](#55-the-translation-pipeline-api_mappingsrs) for the exact order.

`haxe_gen::generate` is the real orchestrator of *output* — `main.rs` only
prepares the inputs and hands everything to it.

---

## 5. Module-by-module reference

Sizes are approximate (Rust LOC). Modules are grouped by pipeline role.

### 5.1 Entry point & wiring

#### `main.rs` (~440 LOC)
CLI definition (`clap`), logging setup, and the top-level orchestration in
`fn main` + `process_character`. Key functions:
- `extract_costumes_to_temp(misc_ssf)` — extracts every character's costume data
  from `misc.ssf` into a temp JSON cache; drops noise (`unknown` key, <10
  costumes). The temp file is deleted after the run.
- `detect_char_names(swf, input)` — walks `Main`'s constructor for
  `register("characters", [self.getX(), …])` and derives an id per array
  entry (strip `get`, lowercase, preserve `_` in the source method name —
  `getGigaBowser` → `gigabowser`, `getWario_Man` → `wario_man`). The
  constructor is the SSF's canonical roster; sub-characters (Sheik in
  zelda.ssf, Giga Bowser in bowser.ssf, Wario Man in wario.ssf) drop out
  naturally as additional array elements. See the "Architectural history"
  section below for why this replaced the earlier `get*`-enumeration approach.
  - **Fallback** when the constructor walk returns empty:
    enumerate `Main`'s instance `get*` methods directly (path 2's original
    detection). Survives for one release as a defensive net for a
    hypothetical future SSF whose roster is built dynamically.
- `derive_id_from_getter(name)` — the strip-`get` + lowercase rule; the
  fallback enumeration's id-derivation. Also lives in `abc_parser` as
  `derive_id_from_bundle_method_name` (the walker calls into it). Both
  collapse into one identity when the fallback is removed.
- `process_character(...)` — runs pipeline steps [4]–[10] for one character;
  every stage is wrapped so a failure logs a warning and continues with a
  default rather than aborting the whole run. Lifts the package metadata
  (`abc_parser::extract_main_package_metadata`) once per call and runs the
  Tier 1 validation pass (`run_tier1_validation`).
- `run_tier1_validation(...)` — soft-log warnings for: empty attacks map,
  all-default stats block, char_name not in the declared roster, or
  Main.id ≠ filename stem. Never hard-fails the run; the warnings land in
  `conversion_log.json :: validation_warnings`.
- `write_conversion_log(...)` — writes `<char_dir>/conversion_log.json` with
  the per-character `unknown` / `ssf2_only` snapshot from `api_mappings`,
  plus the `ssf2_source` block (`package_id`, `package_guid`,
  `source_method`; adds `parent_normal_stats_id` + `note` for transformation
  characters) and `validation_warnings`.

#### `lib.rs`
`pub mod` declarations + the `run_conversion` re-export. Every module is public
so the diagnostic binaries in `src/bin/` can `use ssf2_converter::*`. The crate is
a **library**; conversion is driven by `peptide convert` or `run_conversion`, not
a standalone binary.

### 5.2 SSF / SWF / mapping layer

#### `ssf.rs`
`decompress(data) -> Vec<u8>`: turns a `.ssf` into raw SWF bytes. A `.ssf` is
either already a raw SWF (`FWS` / `CWS` / `ZWS` magic — passed through) or an
SSF-wrapped file: `u32 swf_len` + `u32 garbage_header_size` + zlib payload.

#### `swf_parser.rs`
`parse(data) -> SwfFile`: thin wrapper over the **`swf` crate** (Ruffle's SWF
library). Returns `SwfFile { version, frame_count, frame_rate, symbols,
abc_blocks }`. **Note**: many downstream modules re-run `swf::decompress_swf` +
`swf::parse_swf` themselves on the raw `swf_data` buffer rather than reusing
this parsed value — a known optimization opportunity (parse the SWF once and
thread it through); see the code-quality backlog in [§11](#11-known-issues--gaps).

#### `mappings.rs`
The JSONC loader for `mappings/*.jsonc`. `strip_jsonc(src)` removes `//` and
`/* */` comments and trailing commas; then `serde_json` parses the result.
Each accessor is cached via `OnceLock` so the JSONC is parsed exactly once per
process:
- `character_animations()` → `AnimationMappings { ssf2_to_fm, label_to_ssf2 }`.
- `character_stats()` → `StatMappings { field_keys, multipliers, offsets,
  derivations, constants }`.
- `character_hitbox_stats()` → `HitboxStatsMapping { fields: Vec<HitboxField
  { fm_field, ssf2_keys, isframe }> }`.
- `api_commands()` → `ApiCommands { replacements, regex_replacements,
  call_splits, attach_effect_props, global_vfx_map, frame_params,
  passthrough_fm_apis, ssf2_only }`.

For each file, an on-disk copy (working-dir, next-to-binary, or
`<bin-parent>/<bin-parent>` for the repo root) overrides the `include_str!`'d
default. Malformed override files log a warning and fall back to the default.

`evaluate_stat_derivation(name, vars)` compiles `stats.jsonc :: derivations`
expression strings once via `fasteval` and evaluates them with the converted
stats exposed as variables (so e.g. `aerialSpeedCap` can be defined as
`max(air_mobility_raw, aerial_friction) * 5.0`).

### 5.3 ABC (ActionScript bytecode) layer

#### `abc_parser.rs` (~2400 LOC — largest module)
A complete AVM2/ABC parser written from scratch. Parses the ABC constant pool
(ints, uints, doubles, strings, namespaces, multinames), `Method`s, `Class`es,
`Trait`s, `Script`s and `MethodBody`s into `AbcFile`. Beyond plain parsing it
contains a lot of *semantic* extraction tuned to SSF2's code shape:

**Character data — Stage A (stats / attacks / projectiles) via `Main::get<X>()`:**
- `find_bundle_method(abc, char_name)` — locates the `Main` instance method
  whose name lowercased-after-`get` matches `char_name`. Returns `(body,
  method_name)`. Used by both detection (via the constructor walker — see
  below) and Stage A extraction.
- `extract_character(abc, char_name)` — the main entry. Locates the bundle
  method body via `find_bundle_method`, runs the three stage-A extractors on
  it (`extract_attack_objects`, `extract_projectile_objects`,
  `extract_ssf2_stats`), and records `derived_from` (`DerivedFrom { parent_normal_stats_id, source_method }`)
  when `cData.normalStats_id` mismatches `char_name`. Then Stage B walks the
  Ext class (frame scripts, ext_methods, ext_vars + iinit-derived init values)
  and per-character `_fla.*` sub-classes for helper methods.
- `extract_attack_objects` / `extract_hitboxes_from_val` — recovers attack
  tables by interpreting bytecode with the shared `scan_method` +
  `AbcVisitor` stack interpreter (`StackVal`).
- `extract_projectile_objects` — pulls flat-scalar physics fields + nested
  `attackBoxes` from the bundle's `pData` object. Keys are SSF2 projectile names.
- `extract_ssf2_stats` / `extract_stats_from_body` — two strategies (in
  fallback order) for finding the character's stat object inside the bundle
  body.
- `extract_normal_stats_id` — linear-scans a method body for
  `pushstring "normalStats_id" ; pushstring VALUE` and returns the VALUE.
  Drives transformation detection.

**Detection / package metadata — Main constructor walker:**
- `extract_main_package_metadata(abc) -> Option<MainPackageMetadata>` —
  locates Main's iinit and pulls `id`, `guid`, and the `characters` array
  in one pass. The array is `Vec<(derived_id, method_name)>` — the
  derived ids drive detection, the method names drive Stage A lookup.
- `derive_id_from_bundle_method_name(name)` — the shared id-derivation
  rule: strip `get`, lowercase the remainder, preserve `_`. Mirrored
  by `main.rs::derive_id_from_getter` for the fallback enumeration; both
  collapse when the fallback is removed.
- `MainPackageMetadata { id, guid, characters }` — the SSF's "table of
  contents" lifted out of Main's iinit. Fed into
  `conversion_log.json :: ssf2_source` (in `main.rs`); never lands on
  `CharacterData` (per-character data has no use for the package id/guid).
- `DerivedFrom { parent_normal_stats_id, source_method }` — populated on
  `ExtractedCharacter` when a bundle's cData.normalStats_id mismatches its
  derived id; forwarded to `CharacterData.derived_from` and drives the
  CharacterStats.hx TODO transformation banner.

**Shared walker plumbing:**
- `scan_method<V: AbcVisitor>` + the `AbcVisitor` trait — single shared
  AVM2 stack simulator that drives `AttackVisitor`, `ProjectileVisitor`,
  `StatsVisitor`, and `CostumeVisitor` (the four formerly-duplicated stack
  simulators, unified in commit `21562ab6`).
- `scan_register_string_arg` / `scan_register_characters_array` /
  `skip_opcode_operands` — small dedicated bytecode walkers used by
  `extract_main_package_metadata` and `extract_normal_stats_id`. Live
  alongside `scan_method` but don't model the stack — they only need
  pattern recognition.

**Other extractors:**
- `scan_all_costume_methods` / `decode_costume_objects` — recovers per-character
  costume tables from `misc.ssf` (Pattern A: `paletteSwap: { colors,
  replacements }`; Pattern B: `{ name, colors }`).
- `extract_xframe_name` / `is_root_xframe_method` — maps internal frame methods
  to animation labels via their `xframe`-field assignments in the bytecode.

#### `decompiler.rs` (~1700 LOC)
Turns ABC method bytecode into readable, Haxe-ish source. This is a real
decompiler, not a disassembler:
- `build_blocks` — splits bytecode into basic blocks with `Terminator`s.
- `BlockDecoder` / `StructuredDecoder` — reconstruct a CFG and recover
  structured control flow (`if` / `else` / `while`, short-circuit `&&` / `||`
  collapsing, branch inversion).
- `Expr` / `Stmt` ASTs + `render_stmts` / `render_closure` — pretty-printing.
- `infer_activation_slots` / `slots_from_traits` / `rename_loop_counters` /
  `rename_locals_in_*` — give locals meaningful names.
- `guard_loop_termination` — post-decompile AST pass (runs right after
  `rename_loop_counters`) that appends an `i = i + 1` to any `while (i <
  ….length)` counter loop whose body doesn't already advance the counter on
  every path (and doesn't splice the iterated array). SSF2's AS3 frequently
  mutates an array during iteration via `splice`; the decompiler renders the
  read as an indexed access but can drop the advance, producing a
  non-terminating loop. This was the **engine-freeze bug** (sandbag's
  `removeAllEffects`, fired every frame via a `LINK_FRAMES` listener, hung the
  game loop shortly after match start). The guard only ever adds an advance to a
  loop that lacks one — a correct loop is untouched. See [`TESTING.md`](TESTING.md)
  for the in-engine confirmation.
- `lookup_api` — a small inline SSF2 → FM API table used during decompilation
  (distinct from the larger JSONC-driven rewriter table in `api_mappings`).
- `decompile_method(...)` / `decompile_closure(...)` — public entry points.

### 5.4 Extraction layer

#### `extractor.rs`
The bridge between the raw ABC data and the generator. Defines the central
`CharacterData` struct (`attacks`, `stats`, `animations`, `scripts`, `ext_vars`,
`ext_var_inits`, `projectile_data`, `ssf2_to_fm_anim`, `derived_from`) and
supporting types. `derived_from: Option<DerivedFrom>` is populated when the
character is a transformation / alternate form (Giga Bowser, Wario Man) and
drives the TODO transformation banner in `CharacterStats.hx`.
- `extract(swf, char_name)` — drives `abc_parser`, converts the results, dedupes
  raw SSF2 names against known-FM names, and seeds split sub-animations
  (`jab1`/`jab2`/…).
- `build_ssf2_to_fm_anim` — looks up SSF2 names through
  `mappings/character/animations.jsonc` (no longer a hardcoded table).
- `convert_hitboxes` / `convert_stats` — map SSF2 field names to FM fields via
  `mappings/character/hitbox_stats.jsonc` and `mappings/character/stats.jsonc`.
- `expand_split_anim` / `is_split_sub_anim` — declare which FM anims split into
  sub-anims (`jab` → `jab1..jab4`, `taunt` → `taunt` / `taunt_up` / `taunt_down`).

#### `anim_splitter.rs`
`split_animations(animations, sprite_boxes) -> Vec<SplitAnim>`: some SSF2
sprites pack several Fraymakers moves into one timeline separated by internal
frame labels (the classic case: a "Jab" sprite contains jab1 → jab2 → jab3).
This module decides where to cut.

Rules are **hardcoded match arms** keyed on `anim_name` (aerial / strong /
grab / run / jump / jump_aerial / idle / crouch / special_up / item_smash /
item_throw / fly / helpless / ledge_hang / shield / fall / land / jab4 / hurt /
tech / item_float / ladder / respawn / special / select_screen / taunt_up /
taunt_down). The patterns were derived by scanning the corpus' inner FrameLabels;
they are hardcoded match arms in `anim_splitter.rs`, not loaded from any data file.

#### `sprite_parser.rs`
Extracts **collision-box geometry** from SWF `DefineSprite` timelines.
- `BoxType` enum + `parse_sprite_boxes` — per-animation, per-frame boxes
  (`FrameBox` / `AnimationBoxData`).
- `extract_xframe_transforms` / `extract_xframe_scale` / `XframeTransform` —
  recover the root-MovieClip transform (full affine matrix `a, b, c, d, tx,
  ty`, plus convenience `sx`, `sy`).
- `extract_ssf2_anim_name` / `static_ssf2_to_fm` / `normalize_anim_label` —
  animation-name resolution at the sprite level.
- `find_collision_box_base_size` — measures the `CollisonBox_6` shape's true
  size at runtime (note the SSF2-internal typo "Collison"); tallies which char
  id box-typed instances actually place to find the right shape (since the
  symbol isn't reliably named `CollisonBox`).
- `matrix_to_box` / `matrix_to_itembox` — turn a `PlaceObject` matrix into a
  Fraymakers box `(x, y, w, h[, pivot])`.
- `split_jab` / `split_taunt` / `sub_anim_image_splits` — sprite-level splitting
  that mirrors `anim_splitter`; called from `image_extractor` so collision-box
  splits and image-frame splits stay aligned.
- `apply_fallbacks` — clones box data from a related animation when a target
  has none (e.g. `stunned ← hurt`, `swim ← fall`, `victory ← taunt`).

#### `image_extractor.rs` (~1800 LOC)
Extracts the **visual sprites** and their **per-frame placement**.
- `extract_images(...)` — decodes every `DefineBitsLossless` / `DefineBitsJpeg3`
  bitmap to a PNG (`decode_lossless` / `decode_jpeg3` / `write_png`); builds
  `shape_to_bitmap` (resolves `DefineShape` → bitmap, skipping the SWF null
  bitmap id `65535`) and `shape_pivot` (where a shape's local origin sits inside
  its bitmap — computed from the fill matrix).
- `build_anim_frame_images` — walks each animation timeline and records, per
  frame, every placed image with its full world-space affine matrix
  (`FrameImageEntry`). Two-pass effect-sprite flattening: build unnamed sub-
  sprite frame tables first, then expand named `_fla.` effect placements by
  composing the parent matrix with each inner frame.
- `ImageLocalMatrix` — SWF matrix decomposition (`from_abcd`): scale, rotation
  (via `atan2(b, a)`), **flip encoded as a negative `sy`**, and `has_skew()`
  detects non-rigid matrices.
- `prerender_skewed_frames` — bakes sheared placements (which FrayTools' IMAGE
  symbol can't express) into fresh PNGs and rewrites the placement as a plain
  translation. Uses bicubic (Catmull-Rom) resampling + a mild unsharp pass.
  Cache key uses the bit pattern of the linear matrix + source bitmap id so
  near-identical placements never accidentally share a bake.
- `apply_image_fallbacks` — image-side counterpart to `sprite_parser`'s
  fallback table.
- `discover_projectiles_and_head` — locates projectile sprites (have an
  `attack_idle` FrameLabel + a `stance` PlaceObject), effect sprites (everything
  else at SWF root that isn't a projectile / the character / `_fla.*` / HUD /
  icon / sparkle), and the menu head sprite. The head detection prefers `*_head`,
  also accepts `*_icon`, and explicitly excludes `*_hud` (the animated damage
  counter, which is not a portrait).
- `extract_projectile_frame_images` — flattens nested effect/sub-sprites for a
  given inner sprite id, returning per-frame image symbol names + a
  symbol→meta-GUID map. Used by both projectile and effect entity generation.

#### `sound_extractor.rs`
`extract_all_sounds(...)`: pulls `DefineSound` data out of the SWF and writes
**`library/audio/*.wav`** files (PCM 16-bit). Handles Nellymoser (8 kHz +
variable-rate) and MP3 by wrapping the raw audio in a synthetic **FLV** container
and remuxing through `ffmpeg`; ADPCM goes through a generic FLV wrapper.

> The hand-rolled SWF tag walker (`parse_sounds`) is a historical artifact — it
> predates the `swf` crate's `DefineSound` support and could be replaced with the
> crate's `DefineSound` parsing (code-quality backlog, [§11](#11-known-issues--gaps)).

### 5.5 The translation pipeline (`api_mappings.rs`)

`translate_ssf2_to_fm(code)` is the single entry that every block of decompiled
Haxe passes through (Script.hx ext methods, ext-class iinit, embedded frame
scripts in the entity). The fixed pipeline order is:

1. **`remove_readiness_guards`** — strips `if (SSF2API.isReady()) { … }` /
   `if (self && SSF2API.isReady())` wrappers and inlines the body one tab to
   the left.
2. **`double_frame_counts`** — 30 → 60 fps scaling, driven by
   `commands.jsonc :: frame_params`. Runs *before* the rename pass so SSF2
   field names (`hitStun:`, `hitLag:`, `refreshRate:`, …) still match the
   `isframe`-flagged entries. See [§7](#7-30--60-fps-doubling).
3. **`apply_call_splits`** — fans out SSF2 umbrella calls (e.g.
   `self.updateAttackStats({ … })`) into multiple FM calls (e.g.
   `self.updateAnimationStats({ leaveGroundCancel: false, … });` +
   `self.updateHitboxStats({ … });`) per `commands.jsonc :: call_splits`.
   Source fields with no mapping become `// TODO:` comments. Source fields
   with a `skip_if_value` match are dropped silently. The call splitter is
   bracket-aware (so commas inside nested calls/arrays/objects don't break
   field splitting) and comment-aware (matches inside `//` are skipped).
4. **Literal `replacements`** — ordered find/replace pairs from
   `commands.jsonc :: replacements`. Order matters (e.g. `self.self.` must
   run before bare `self.self`).
5. **`regex_replacements`** — `commands.jsonc :: regex_replacements`,
   precompiled once into a `OnceLock<Vec<CompiledRegexReplacement>>`. Used
   for arg-dropping, arg-aware dispatch, etc.
6. **`rewrite_attach_effect_calls`** — context-aware
   `self.attachEffect("name") / self.attachEffect("name", { props })` →
   `match.createVfx(new VfxStats({ spriteContent: …, animation: …,
   <translated props> }), self)`.
   - The shape of the VfxStats head is decided in `build_vfx_head(name)`:
     - If `name` is in `commands.jsonc :: global_vfx_map`, emit
       `spriteContent: "global::vfx.vfx", animation: GlobalVfx.<CONST>`.
     - Otherwise emit per-character
       `spriteContent: self.getResource().getContent("name"),
       animation: "<primary or active fallback>"`. The animation name comes
       from a thread-local `EFFECT_PRIMARY_ANIMS` map installed by
       `haxe_gen::generate` via the RAII `EffectAnimGuard`.
   - Per-prop translation goes through `commands.jsonc :: attach_effect_props`,
     supporting `Simple` direct renames, `Detailed` 1→N `expand_to`
     (e.g. `parentLock` → `relativeWith` + `resizeWith` + `flipWith`), and
     explicit `todo` notes.
7. **`strip_last_frame_end_animation`** — removes redundant
   `self.endAnimation();` calls in the last-numbered frame function of each
   animation group (FM ends animations naturally on the final frame).
8. **`comment_out_unknown_calls`** — turns lines containing
   `.<name>(` for any name listed in `commands.jsonc :: ssf2_only` into
   `// [SSF2-only: NAME] <original>`. Counts these into the per-character
   `ConversionLog::ssf2_only` map.

Beyond that pipeline, two **whole-script** passes also run (in `haxe_gen`):

- **`fix_intangibility_pairs(full_script)`** — pairs each
  `self.setIntangibility(true);` with the next `self.setIntangibility(false);`
  in the same anim-prefix and rewrites to a single
  `self.applyGlobalBodyStatus(BodyStatus.INTANGIBLE, N);` with the duration
  baked in. Unpaired calls are surfaced with a `// [SSF2-only:
  setIntangibility]` marker.
- **`wrap_persistent_state(code, var_types)`** — rewrites SSF2 ext-class
  instance-variable references into Fraymakers persistent-state wrappers:
  `self.foo++` → `foo.inc()`, `self.foo = X;` → `foo.set(X);`,
  `self.foo` reads → `foo.get()`. `var_types` comes from
  `api_mappings::infer_ext_var_types(ext_vars, ext_var_inits)` and chooses
  `makeBool` / `makeInt` / `makeObject` factories for the declaration.

`comment_out_unknown_calls` also records every `.NAME(` call that doesn't
appear in **any** `commands.jsonc` section (`replacements`,
`regex_replacements`, `passthrough_fm_apis`, `ssf2_only`, `frame_params`,
`call_splits` source methods) into `ConversionLog::unknown`. The combined
log lands in `<char>/conversion_log.json` at the end of the run and drives
the GUI "Unhandled Calls" popup.

### 5.6 Generation layer

#### `haxe_gen.rs` (~1750 LOC)
The **output orchestrator**. `generate(...)` writes the entire character package
(see pipeline step [10]). Also owns:
- Installation of the `EffectAnimGuard` (per-character effect → primary-animation
  map) for the duration of the generation pass.
- `count_populated_jabs(img_result)` → drives the jab-chain decision (a
  single-jab character gets no chain boilerplate; `populated_jabs == 2`
  keeps `jab3` as an empty-but-allowlisted placeholder so the chain doesn't
  break on the last link).
- **Stat scaling** wrappers (`ssf2_gravity_to_fm`, `ssf2_speed_to_fm`,
  `ssf2_jump_to_fm`, `ssf2_walk_to_fm`, `ssf2_dash_to_fm`, `ssf2_air_to_fm`) —
  thin wrappers around `crate::mappings::character_stats().scale("name", v)`
  so every value comes from `stats.jsonc :: multipliers`.
- `generate_hitbox_stats` / `generate_character_stats` /
  `generate_animation_stats` / `generate_script` — the four character `.hx`
  files. `generate_character_stats` pulls every value from `stats.jsonc` (raw
  values via `field_keys` + `multipliers`, derivations via
  `evaluate_stat_derivation`, flat sections like ECB / camera / shield /
  voice via `stats.jsonc :: constants`). `generate_script` MERGES SSF2 ext
  methods whose names match template functions (`initialize`, `update`,
  `inputUpdateHook`, `handleLinkFrames`, `onTeardown`) into the template
  versions instead of renaming them, with ext-var init assignments emitted in
  `initialize()` (skip-dup against names the merged SSF2 init body already
  assigns).
- **`generate_jab_scripts`** — emitted only when `populated_jabs >= 2`.
- `generate_manifest` — `library/manifest.json` (includes projectile manifest
  entries, `*ProjectileStats` / `…AnimationStats` / `…HitboxStats` /
  `…Script` references).
- **`generate_projectile_script`** — uses a local-state-machine pattern
  (`Common.initLocalStateMachine()` + `Common.registerLocalState(...)`) for
  multi-state projectiles, with one `LState` per SSF2 frame label.
- `generate_projectile_animation_stats` / `generate_projectile_stats` /
  `generate_projectile_hitbox_stats` — pull real values out of the SSF2
  `getProjectileStats()` data via `best_match_projectile_data(name,
  projectile_data)` (exact name → substring heuristic → most-populated
  fallback). Field names go through the shared `hitbox_stats.jsonc` canon.
- `generate_sound_entries` — writes `.wav.meta` sidecars next to each
  extracted audio file.
- `script_meta(id, guid, kind)` — emits the `.hx.meta` sidecar variants
  (`CharacterScript` / `CharacterStats` / `CharacterAnimationStats` /
  `CharacterHitboxStats` / `ProjectileScript` / `ProjectileStats` /
  `ProjectileAnimationStats` / `ProjectileHitboxStats`).

#### `entity_gen.rs` (~2000 LOC)
Builds the Fraymakers `.entity` JSON — the heart of a FrayTools character.
- `generate_entity` / `generate_entity_with_palette` — the main
  character-entity builder (with/without `paletteMap`); written to
  `library/entities/<Pascal>.entity`.
- `double_keyframe_lengths(keyframes)` — runs at the end of every entity
  build, doubling every keyframe's `length` for the 30 → 60 fps move
  ([§7](#7-30--60-fps-doubling)).
- Empty-animation drop pass: animations whose IMAGE timeline carries no
  symbols anywhere are removed, except a small `keep_empty` allowlist driven
  by `populated_jabs`.
- `generate_menu_entity` — the character-select head entity, with `full`,
  `css` (2-layer fg/bg), `icon`, `icon_no_palette`, `stock`, and ten
  `hud[/_front/_angry/_happy/_hurt/_sad]` variants.
- `generate_projectile_entity` (+ `ProjectileInfo` / `ProjectileStateData`) —
  per-projectile entities, including multi-state projectiles (e.g.
  `link_bomb`). Animation names come from inner-sprite FrameLabels when
  present (`projectile_anim_names(proj)`); else fall back to the FM template
  trio (`projectileSpawn` / `projectileIdle` / `projectileDestroy`).
- `generate_effect_entity` — emits one `.entity` per discovered effect, with
  one IMAGE layer per inner FrameLabel segment (or a single `active`
  animation if there are no labels). No scripts, no stats — the character's
  `Script.hx` triggers them via the `attachEffect` rewrite.
- `effect_animation_names(effect)` — the lockstep helper used by
  `haxe_gen::generate` to build the per-character `EFFECT_PRIMARY_ANIMS`
  map. Keep this in sync with the `segments` block of `generate_effect_entity`.
- `generate_meta` / `get_image_meta_guids` — sprite `.meta` sidecars.
- `box_type_to_fm` / `ssf2_box_name_to_fm` / `fm_box_index` / `box_color` —
  the SSF2-box-name → Fraymakers-layer mapping (see the table in
  `AGENT_CONTEXT.md`).
- COLLISION_BOX and IMAGE rotation are both emitted with the same
  **CW-positive, 0-360-normalized** convention — no negation, matching SWF's
  `atan2(b, a)` convention. (Commit `40fad65d` brought collision-box rotation
  in line with the IMAGE-symbol convention; both paths now agree.)

#### `palette_gen.rs`
`generate_palettes_and_remap(...)`: builds Fraymakers costume palettes.
- `load_ssf2_costumes` / `build_from_ssf2` — preferred path: uses real
  `misc.ssf` costume tables (typically 15 costumes × variable colour-slot
  count).
- `build_from_sprites` / `kmeans` / `rotate_hue` — fallback when no
  `misc.ssf`: derives a palette from idle-sprite pixels via k-means and
  synthesizes team-colour variants by hue rotation.
- `build_output` — writes `costumes.palettes`, the `.meta`, and writes the
  preview PNG (one column per colour slot).

#### `fraytools_project.rs`
`generate_fraytools_project(char_name)` — emits the small `<name>.fraytools`
project descriptor FrayTools opens. Includes the default collision-box layer
preset (hitbox / hurtbox / grabbox / counterbox / reflectbox / ledgegrabbox /
holdbox / absorbbox / custom × 3) and the project-wide `frame_rate: 60` /
`paletteShaderMode: "RG_MAP"` settings.

#### `uuid_gen.rs`
`det_uuid(seed)` — RFC-4122 **UUID v5** (SHA-1 namespace). Every GUID in the
output is `det_uuid("{char_id}::{context}")`, which is what makes conversions
reproducible.

### 5.7 Diagnostic binaries (`src/bin/`)

These are **reverse-engineering / debugging tools**, not part of the conversion.
Each builds to its own executable in `target/release/`. They were the workbench
used to figure the formats out and remain useful when a conversion looks wrong.
Roughly grouped by what they investigate:

**SWF format inspection (the original toolset):**

| Binary | Purpose |
|---|---|
| `dump_sprites` | List every `DefineSprite` symbol in a SWF |
| `dump_images` | List all extracted bitmaps |
| `dump_image_placement` | Per-frame `PlaceObject` data for a named sprite |
| `dump_collision_box` | Collision-box geometry for an animation |
| `dump_shape_bounds` | Measure the true bounds of the `CollisonBox` shape |
| `dump_shape_origins` | Shape origin offsets |
| `dump_pivots` | Pivot points |
| `dump_frame_labels` | Frame labels inside a sprite timeline |
| `dump_raw_frame` | Raw tag dump for one frame |
| `dump_inner_sprite` | Inspect a nested (effect) sprite |
| `dump_proj_states` | Projectile state discovery |
| `dump_stage` | Stage / root timeline inspection |
| `dump_costumes` | Costume tables from `misc.ssf` |
| `dump_aerial_down_frames` | Targeted debug for the aerial-down animation |
| `dump_trail_matrices` | Trail-effect matrix inspection (rotation/pivot/skew work) |
| `check_shape_bitmap` | Inspect a shape's bitmap fill + fill matrix |
| `what_is_id` | Identify what a numeric SWF character id refers to |

**Path 2 / sub-character / constructor-walk investigation:**

| Binary | Purpose |
|---|---|
| `audit_main_iinit` | Corpus audit: extract `register("id"/"guid"/"characters")` per SSF; produced the data behind the constructor-walk detection (see "Architectural history") |
| `audit_main_gets` | Corpus audit: every `Main::get*` instance method, with bundle-shape classification |
| `find_transformations` | Find SSFs with multiple bundles sharing a normalStats_id (Giga Bowser, Wario Man) |
| `find_get_callers` | Who calls `Main::get<X>()` across the SWF (used to confirm only iinit calls them) |
| `decompile_main_iinit` | One-shot decompile of a single SSF's `Main` constructor |
| `dump_main_class` | Full dump of `Main`'s trait list for a given SSF |
| `find_char_sources` | Earlier-iteration probe: scan all bodies for `normalStats_id` + bundle-shape markers; classifies INLINE vs BUNDLE |
| `diff_inline_vs_bundle` | Dump `<X>Ext::get*Stats` + `Main::get<X>()` decompiled bodies to disk for hand-diffing |
| `dump_getzelda` / `dump_zeldaext` / `zelda_method_diff` / `zelda_method_hashes` | The Sheik investigation — confirmed that `Main::getSheik` exists with no `SheikExt` |
| `dump_misc` | Summary of `misc.ssf`'s content (no `Main` class, just costume / shared data) |
| `find_char_mcs` | Find character-like MovieClip timelines (≥30 `frame*` methods) |
| `grep_classes` / `scan_all_classes` / `scan_ext` | Class-name corpus searches |

Typical use: `./target/release/dump_image_placement ../ssf2-ssfs/mario.ssf "FAir_42"` or `./target/release/audit_main_iinit`. (There is no longer a standalone `extract_costumes` binary — costume extraction is in-process inside `ssf2_converter`.)

---

## 6. Mapping files (`mappings/`)

Most of the converter's *behaviour* is data — JSONC files loaded at runtime
with `include_str!`'d defaults as a guaranteed-valid fallback. Edit these
files (not the Rust source) to tune the conversion. They live in
`mappings/` at the repo root.

### 6.1 `mappings/commands.jsonc` (universal SSF2 → FM API conversions)

Sections, in the order they're applied by `translate_ssf2_to_fm`:

- **`replacements`** — ordered literal find/replace pairs. ORDER MATTERS.
  Covers the bulk of SSF2 → FM API renames (`self.self.` → `self.`,
  `.endAttack()` → `.endAnimation()`, `.setXSpeed(` → `.setXVelocity(`,
  `SSF2API.print(` → `Engine.log(`, hitbox field renames like `direction:` →
  `angle:`, etc.).
- **`regex_replacements`** — `{ pattern, replacement, note? }`. Compiled
  once into a `OnceLock` cache; bad patterns log a warning and are skipped.
  Used for cases the literal table can't express (arg-dropping, arg-aware
  dispatch).
- **`call_splits`** — map of `<source_method>` → `{ fields: { <ssf2_field>
  → <"target_method.fm_field"> | { target?, value_map?, skip_if_value?,
  todo? } } }`. Drives the fan-out of SSF2 umbrella calls into multiple FM
  target methods (currently `updateAttackStats` → `updateAnimationStats` +
  `updateHitboxStats`). Fields sharing a target method are GROUPED into a
  single combined call. Fields with no mapping become `// TODO:` comments
  above the emitted calls.
- **`attach_effect_props`** — map of `<ssf2_prop>` → `"fmPropName"` |
  `{ target?, expand_to?, todo? }`. Drives the inline translation that
  injects translated fields into `new VfxStats({…})`. `expand_to` lets one
  SSF2 prop become several FM props (e.g. `parentLock` →
  `relativeWith` + `resizeWith` + `flipWith`).
- **`global_vfx_map`** — `<effect_name>` → `<GlobalVfx constant name>`.
  Names listed here are rewritten to `spriteContent: "global::vfx.vfx",
  animation: GlobalVfx.<C>` instead of the per-character resource lookup.
- **`frame_params`** — `[{ kind: "call" | "field", name, arg?, isframe,
  sentinel? }]`. Drives 30 → 60 fps doubling. `kind: "call"` doubles the
  literal at positional arg index `arg` of every `name(...)` call. `kind:
  "field"` doubles the literal that follows `name:`. Only entries with
  `isframe: true` are doubled; values ≥ `sentinel` are left alone (e.g.
  255 / -1 "no override" sentinels for hitStun).
- **`passthrough_fm_apis`** — `[{ name, note? }]`. Names listed here are
  "known FM API calls" — left untouched and suppressed from the
  `conversion_log.json :: unknown` stream.
- **`ssf2_only`** — `[{ name, note? }]`. Names listed here have no FM
  equivalent — every call site gets replaced with
  `// [SSF2-only: <name>] <original>` and counted in `conversion_log.json
  :: ssf2_only`.

### 6.2 `mappings/character/animations.jsonc`

```jsonc
{
  "ssf2_to_fm":     { "stand": "idle", "a_air_forward": "aerial_forward", ... },
  "label_to_ssf2":  { "nair": "a_air", "fair": "a_air_forward", ... }
}
```

- `ssf2_to_fm` is the primary table; used everywhere SSF2 anim names need
  to become FM anim names (extractor, sprite_parser, image_extractor,
  haxe_gen).
- `label_to_ssf2` is consulted by `sprite_parser::extract_ssf2_anim_name`
  when a sub-sprite symbol's local label (lowercased, suffix-stripped, e.g.
  `NAir` → `nair`) doesn't appear directly in `ssf2_to_fm` — it bridges the
  sprite-label-shorthand to the SSF2 xframe name.

### 6.3 `mappings/character/stats.jsonc`

Drives **every value** in `CharacterStats.hx`. Sections:

- **`field_keys`** — `<fm_field>` → `[<ssf2_key>, …]`. Ordered list of
  SSF2 keys to try when extracting an FM stat (the first matching key wins).
- **`multipliers`** — `<name>` → `{ divisor, target, floor }`. Applied as
  `(raw / divisor * target).max(floor)`. Referenced by the `scale("name",
  v)` wrappers in `haxe_gen.rs` (gravity / speed / jump / walk / dash /
  air_friction).
- **`offsets`** — `<stat>` → integer offset added after extraction (used
  for `max_jumps + 1` to bridge SSF2's midair-jumps count and FM's total-
  jumps count).
- **`derivations`** — `<stat>` → expression string. Compiled once with
  `fasteval` (built-ins `max` / `min` plus our `clamp(x, lo, hi)`);
  evaluated against the already-converted stats exposed as variables. Used
  for `shortHopSpeed`, `aerialSpeedCap`, etc.
- **`constants`** — `<field>` → raw JSON value. Emitted Haxe-literally for
  fields the converter can't derive from SSF2 source (ECB, camera box,
  shield 9-slice, voice IDs).

### 6.4 `mappings/character/hitbox_stats.jsonc`

```jsonc
{
  "fields": [
    { "fm_field": "damage",          "ssf2_keys": ["damage"] },
    { "fm_field": "angle",           "ssf2_keys": ["direction", "angle"] },
    { "fm_field": "baseKnockback",   "ssf2_keys": ["power", "kbConstant", "weightKB"] },
    { "fm_field": "knockbackGrowth", "ssf2_keys": ["kbGrowth"] },
    { "fm_field": "hitstop",         "ssf2_keys": ["hitLag", "hitstop"], "isframe": true },
    { "fm_field": "selfHitstop",     "ssf2_keys": ["selfHitLag"],       "isframe": true },
    { "fm_field": "hitstun",         "ssf2_keys": ["hitStun", "hitstun"], "isframe": true }
  ]
}
```

Per FM hitbox field, the converter takes the **max** over all listed SSF2
source keys (an absent key counts as 0). `isframe: true` flags the field as
a frame count → it gets doubled for the 30 → 60 fps move. This same canon
is used by `entity_gen` (character hitboxes), `haxe_gen`
(`generate_projectile_hitbox_stats`), and `extractor::convert_hitboxes`
(single source of truth across the codebase).

---

## 7. 30 → 60 fps doubling

SSF2 plays at 30 fps; Fraymakers at 60 fps. To preserve playback speed,
every frame-count value must be doubled. The converter does this in **two
unified places**:

1. **Timeline lengths** — `entity_gen::double_keyframe_lengths` doubles the
   `length` field of every keyframe (LABEL, FRAME_SCRIPT, COLLISION_BODY,
   COLLISION_BOX, POINT, IMAGE) right before the entity JSON is finalized.
   Because FrayTools timelines are laid out purely by sequential keyframe
   length, doubling every length doubles every layer's span and every
   keyframe's start position in lockstep — image, collision-box, frame-
   script and label layers can never fall out of sync.

2. **Frame-count arguments / fields in code** —
   `api_mappings::double_frame_counts` doubles specific literal arguments
   and object-literal fields based on `commands.jsonc :: frame_params`. Two
   helpers do the work:
   - `double_int_after_marker(code, "name:", skip_at)` for `kind: "field"`.
   - `double_call_arg(code, "fn_name", arg_idx, skip_at)` for `kind:
     "call"` — bracket-depth-aware so commas inside nested calls / arrays /
     objects don't miscount.

   Runs **first** in the translation pipeline so SSF2 field names still
   match the `frame_params` entries (the literal `replacements` pass below
   it renames `hitStun:` → `hitstop:`, etc.). Same pass covers Script.hx
   ext methods, decompiled ext-class iinit, and embedded frame scripts in
   the entity — they all go through `translate_ssf2_to_fm`.

3. **HitboxStats.hx and projectile-hitbox values** — `entity_gen` and
   `haxe_gen::generate_projectile_hitbox_stats` both consult
   `hitbox_stats.jsonc :: isframe` to decide which extracted hitbox values
   to double (hitstop / hitstun / selfHitstop, with sentinel handling for
   the 255 / -1 "no override" values).

---

## 8. Input format: SSF2 `.ssf`

Summary of what the code assumes — full detail in
[`AGENT_CONTEXT.md`](AGENT_CONTEXT.md).

- A `.ssf` file is a **renamed/SSF-wrapped SWF (Flash)**. `ssf.rs` unwraps it.
- **Character logic, stats, attack tables and costume data live in AS3 bytecode**
  (`DoABC` tags) — parsed by `abc_parser`, decompiled by `decompiler`.
- **Sprites, animation timelines and collision boxes live in the SWF display
  list** — `DefineBitsLossless` / `DefineBitsJpeg3` / `DefineShape` /
  `DefineSprite` tags with `PlaceObject` / `ShowFrame` / `RemoveObject` —
  parsed by `sprite_parser` / `image_extractor`.
- The roster is detected by **walking `Main`'s constructor** for its
  `register("characters", [self.getX(), …])` table (see §4 / §5.1); each
  character's stats/attacks/projectiles come from the matching `Main::get<X>()`
  bundle method (Stage A) and its behaviour code from the per-character `XxxExt`
  AS3 class where one exists (`MarioExt`, `FoxExt`, … — Stage B). Sub-characters
  that share an Ext class (Sheik shares `ZeldaExt`) still appear as their own
  `Main::getSheik` bundle, so the constructor walk picks them up.
- Animation sprites are named `{char}_fla.{AnimLabel}_{index}` (e.g.
  `mario_fla.FAir_42`).
- Collision boxes are a square shape (`CollisonBox_6` — note SSF2's internal
  typo) scaled/positioned by a `PlaceObject` matrix; box *type* comes from
  the instance name (`attackBox` → hitbox, `hitBox` → hurtbox, `grabBox`,
  `touchBox`, `ledgeBox`, `reflectBox`, …).
- SWF matrices use fixed-point components; translations are in **twips**
  (÷20 = pixels). Both SSF2 and Fraymakers use **y-down** screen coordinates,
  so no vertical flip is needed.
- Both SWF (`atan2(b, a)`) and FrayTools use **CW-positive rotation** — the
  converter does NOT negate; it normalizes to `[0, 360)`.
- Costume data is in `misc.ssf` → `Misc.as` → `getCostumeData()`: ~15
  costumes per character (Red / Green / Blue / Default + ~11 alts), each
  with parallel `colors` and `replacements` arrays.

---

## 9. Output format: Fraymakers character package

Entity files and the per-character scripts subdir are named after the
character in **PascalCase**, derived from the `Main::get<X>` method name
(`getMario` → `Mario`, `getBandanaDee` → `BandanaDee`, `getWario_Man` →
`WarioMan`, `getgameandwatch` → `Gameandwatch`) by the `pascal_form` rule.

For the single-character SSF `mario`:

```
characters/mario/
├── mario.fraytools                       FrayTools project file (project settings only)
├── conversion_stats.json                 debug summary of the run
├── conversion_log.json                   unhandled / SSF2-only counts + ssf2_source + validation_warnings
└── library/
    ├── manifest.json (+ .meta)            ← one type:"character" entry + AI + per-projectile entries
    ├── costumes.palettes  (+ .meta)        15 SSF2 costumes as FM palettes
    ├── entities/
    │   ├── Mario.entity                    main entity, PascalCase = character id (was Character.entity)
    │   ├── Menu.entity                     full / css / icon / stock / hud variants (was menu.entity)
    │   ├── <projectile>.entity             one per discovered projectile
    │   └── <effect>.entity                 one per discovered VFX sprite
    ├── sprites/
    │   ├── *.png                           extracted frame bitmaps
    │   ├── *.png.meta                      GUID sidecar per PNG (entity refs the GUID)
    │   └── palette_preview.png (+ .meta)
    ├── scripts/
    │   ├── Mario/                          <Pascal>/ — was Character/
    │   │   ├── CharacterStats.hx           movement physics
    │   │   ├── HitboxStats.hx              per-attack hitbox data
    │   │   ├── AnimationStats.hx           animation flags
    │   │   ├── Script.hx                   decompiled character logic
    │   │   └── *.hx.meta
    │   └── Projectile/
    │       └── <Pascal>{Script,Stats,HitboxStats,AnimationStats}.hx (+ .meta)
    └── audio/
        ├── *.wav                           extracted sounds (PCM 16-bit via ffmpeg) — flat for single-char
        └── *.wav.meta                      per-sound content sidecar
```

### Multi-character SSFs

`zelda.ssf` (Zelda + Sheik), `bowser.ssf` (Bowser + Giga Bowser), and
`wario.ssf` (Wario + Wario Man) each emit **one shared project**
containing both characters as peer entities — a forward requirement for
Fraymakers' future transformation API (which needs both forms in one
project to swap between them at runtime). For `zelda.ssf`:

```
characters/zelda/
├── zelda.fraytools                       one project for both characters
├── conversion_log.json                   project-scoped, characters:[…] array (per-char ssf2_source)
└── library/
    ├── manifest.json (+ .meta)            TWO type:"character" entries (zelda + sheik), namespaced ids
    ├── costumes.palettes  (+ .meta)        Zelda's (constructor-walk slot 0)
    ├── costumes.palettes2 (+ .meta)        Sheik's (slot 1 → numeric collision suffix)
    ├── palette_preview.png / .png2         same collision-suffix rule
    ├── entities/
    │   ├── Zelda.entity   Sheik.entity     one character entity each
    │   ├── Zelda_Menu.entity  Sheik_Menu.entity   per-character portraits
    │   └── <projectile>.entity             shared projectile entities
    ├── scripts/
    │   ├── Zelda/   Sheik/                  per-character script subdirs
    │   └── Projectile/                      shared
    ├── sprites/                             shared (PNG names are SSF2 <char>_fla.-prefixed)
    └── audio/
        ├── zelda/*.wav                      per-character audio subdirs (multi-char only)
        └── sheik/*.wav
```

Collision-suffix rule: files that would collide at a shared library
path (`costumes.palettes`, `palette_preview.png`) get a numeric suffix
in constructor-walk order — slot 0 unsuffixed, slot 1 → `2`, etc.
Per-character entities/scripts don't collide (the PascalCase name is in
the path). `--per-character-projects` reverts a multi-char SSF to the
single-character standalone layout. Transformation forms (Giga Bowser,
Wario Man) carry a TODO banner in `CharacterStats.hx` about the manual
wiring still owed for the not-yet-shipped FM transformation API.

`.entity` files are JSON. The top-level shape is `{ animations[], layers[],
keyframes[], symbols[], paletteMap, pluginMetadata, plugins, version: 14, … }`.
Each animation owns an ordered layer stack — `LABEL`, `FRAME_SCRIPT`,
`COLLISION_BODY`, one `COLLISION_BOX` / `POINT` per box instance, one `IMAGE`
per depth slot — and layers reference keyframes which reference symbols. The
full schema (every layer/symbol/keyframe shape, the box-type enum, `.meta`
sidecar format) is in [`AGENT_CONTEXT.md`](AGENT_CONTEXT.md) and is the ground
truth — keep it open when working on `entity_gen.rs`.

Projectile scripts use the Annie/aJewelofRarity convention: a **single**
`library/scripts/Projectile/` directory holds files for every projectile,
each prefixed by the projectile name in PascalCase
(`DeeNspecScript.hx`, `DeeNspecStats.hx`, `DeeNspecHitboxStats.hx`,
`DeeNspecAnimationStats.hx`, …).

Effects (VFX) are emitted as plain entities under
`library/entities/<effect>.entity` — one IMAGE layer per inner FrameLabel
segment (or a single `active` animation if there are no labels). No scripts,
no stats. The character's `Script.hx` triggers them via the
`attachEffect` → `match.createVfx(...)` rewrite.

---

## 10. Current status — what works

**The pipeline runs end to end and produces complete, FrayTools-shaped
character packages.** Evidence in the repo:

- `characters/sandbag/` — the smoke-test character (small, simple).
- `characters/mario/`, `characters/fox/`, `characters/link/`,
  `characters/naruto/`, `characters/jigglypuff/`, `characters/samus/`,
  `characters/bandanadee/`, `characters/captainfalcon/`, … — full
  conversions of most of the roster.
- 46 SSF2 character inputs (plus `misc.ssf`) in the sibling
  `../ssf2-ssfs/`. The converter emits **47 character packages** because
  `zelda.ssf` ships both Zelda and Sheik, `bowser.ssf` ships both Bowser
  and Giga Bowser, and `wario.ssf` ships both Wario and Wario Man — all
  six are registered peers in their SSFs' `Main::<init>` rosters.

What is solid:

- `.ssf` → SWF decompression, SWF parsing, ABC parsing.
- **Constructor-walk character detection** from `Main`'s iinit
  `register("characters", […])` array; sub-characters (Sheik, Giga
  Bowser, Wario Man) fall out naturally; transformations carry a
  `derived_from` marker that drives a TODO banner in
  `CharacterStats.hx` and metadata in `conversion_log.json`. Tier 1
  validation hooks (empty stats / id ≠ filename / declared-vs-extracted)
  emit soft logs to `conversion_log.json :: validation_warnings`.
- Character / stats / attack / frame-script / projectile-stat / ext-var
  extraction from ABC. Stats / attacks / projectiles come from the
  `Main::get<X>()` bundle method body (Stage A); behavior code (frame
  scripts, ext methods, ext vars + iinit-derived inits) comes from the
  matching `<X>Ext` class (Stage B).
- The AS3 decompiler (CFG reconstruction, structured control flow,
  short-circuit `&&` / `||` collapsing, loop counter renaming).
- Bitmap extraction → PNG, sprite `.meta` generation.
- Collision-box geometry extraction; full affine matrix composition with
  root MC transform; CW-positive 0-360 rotation on both IMAGE and
  COLLISION_BOX symbols; itemBox pivot-at-bottom-centre.
- Shear-bake fallback for sheared image placements
  (`prerender_skewed_frames`).
- Costume/palette extraction from `misc.ssf` (with k-means fallback).
- Sound extraction to WAV via `ffmpeg` (Nellymoser, MP3, ADPCM).
- Deterministic GUIDs.
- Entity JSON assembly: LABEL / FRAME_SCRIPT / COLLISION_BODY /
  COLLISION_BOX / POINT / IMAGE layers; per-frame ECB diamond auto-fit
  to hurtboxes; run-length-encoded keyframes; 30 → 60 fps length doubling.
- Multi-state projectile entities (LState local state machine in the
  emitted script).
- Per-effect entities (`<name>.entity`) split by inner FrameLabels;
  `match.createVfx(new VfxStats({…}), self)` rewrite for both 1-arg and
  2-arg `attachEffect` calls; `global_vfx_map` constant routing for FM
  built-in VFX.
- Persistent-state wrappers for SSF2 ext-class vars (`var foo =
  self.makeInt(0)`; `foo.get()` / `foo.set(v)` / `foo.inc()` /
  `foo.dec()`); ext-class iinit assignments emitted in
  `initialize()` (skip-dup against the merged SSF2 init body).
- Jab-chain emission gated by populated-jab count.
- Empty-animation dropping (with a jab-aware keep-list).
- `Menu.entity` with broadened head-sprite detection (`_head`, `_icon`;
  excludes `_hud`).
- Conversion log + the converter screen's warnings panel.
- Peptide as the single-binary parent product (webview UI; converter folded in as a library).

---

## 11. Known issues & gaps

1. **Shape-only menu portraits.** A handful of characters
   (`donkeykong`, `fox`, `marth`) have `*_head` portraits composed entirely
   of shapes rather than a bitmap. The head finder prefers a Bitmap
   placement when one exists; when it doesn't, the head image is missing
   and `Menu.entity` ships with a placeholder. Needs a small SWF shape
   rasterizer.

2. **Mario sprite placement not re-verified.** After the recent rotation /
   itemBox / shear-baking work, most characters look right in FrayTools,
   but Mario in particular hasn't been re-verified frame by frame. Mario
   was the canary that drove the rotation work; some animations may still
   need a focused pass.

3. **One `.ssf` still fails conversion outright.** Historically one
   character file in the roster tripped a hard error; needs re-check
   against the current code now that path 2 + the constructor walker
   have landed. Tracked separately; not yet re-confirmed.

4. **Vector-only effect sprites are silently skipped.** Effects whose
   visuals are pure-vector shapes with solid-colour fills (e.g. some
   charge sparkles, the F-air twinkle) cannot be rasterized without a
   full SWF vector renderer. Only bitmap-backed shapes are exported.

5. **Frame-script / API translation is incomplete.** `commands.jsonc`
   covers the bulk of SSF2 API calls, but the `ssf2_only` list and the
   `conversion_log.json :: unknown` stream surface the rest. Generated
   `.hx` always needs human review.

6. **Projectile behaviour is stubbed.** Projectile *entities* (visuals,
   boxes, animations, palettes) are generated. Projectile *behaviour*
   (`<Pascal>Script.hx`) is template scaffolding with `// TODO: tune
   X_SPEED / Y_SPEED` placeholders and (for multi-state projectiles) empty
   LState transitions.

7. **Stat scaling is approximate.** The `stats.jsonc :: multipliers` were
   hand-tuned by comparing template characters to SSF2 data. Generated
   `CharacterStats.hx` marks uncertain numbers with `/*TODO*/`.

8. **`tools/rebuild-sandbag.sh` has an absolute path** baked in — breaks if the
   repo moves.

9. **`tokio` is a declared dependency but the converter is synchronous** —
   `main.rs` has no async. Likely vestigial; verify before relying on it,
   and consider removing it to cut build time.

10. **Robustness.** `process_character` swallows per-stage errors and
    continues with defaults — good for batch runs, but means a partly-broken
    character can be produced without an obvious failure. The new Tier 1
    validation pass surfaces the most common silent regressions
    (empty stats, empty attacks, declared-vs-extracted mismatch) as
    `conversion_log.json :: validation_warnings`. Always check
    `conversion_stats.json` and `conversion_log.json` after a run.

11. **Transformation characters need manual FM-side wiring.** Giga
    Bowser and Wario Man emit as standalone packages
    (`characters/gigabowser/`, `characters/wario_man/`) because
    Fraymakers has no native transformation API. The TODO banner in
    `CharacterStats.hx` and the `ssf2_source` block in
    `conversion_log.json` flag this; the content author must script the
    swap manually in the parent character's `Script.hx`.

12. **`build_*_map` legacy block deferred.** ~350 lines of
    `build_method_map` / `build_property_map` / `build_state_map` /
    `build_event_map` / `build_hitbox_prop_map` /
    `load_api_methods_json` in `api_mappings.rs` are marked TODO and
    preserved until JSONC parity is confirmed (commit `43a13638`).
    Once the JSONC tables are proven to fully replace them, delete the
    block. The dead duplicate costume / stat extractors in `abc_parser.rs`
    are already gone (commit `7671defe`); see the code-quality backlog below
    for what remains.

13. **Path 2 enumeration fallback still present.** The fallback path
    in `detect_char_names` (instance-method enumeration on Main) is
    retained for one release as defence-in-depth against a future SSF
    whose constructor builds the array dynamically. Slated for deletion
    in a follow-up commit; track via the warn log when it fires.

### Code-quality backlog

A standing audit (optimization / cleanup / latent-bug list with file-and-line
refs). The big-ticket items it originally flagged are **done** — parse-the-SWF-once
hot paths reduced (`5f34c666`), the four AVM2 stack simulators unified (`21562ab6`),
`wrap_persistent_state` regex caching (`e7e62111`), `getproperty` mishandling fixed
with the visitor unification, `infer_ext_var_types` float-as-Int fixed, dead costume
/ stat extractors deleted (`7671defe`). What's still open, by leverage:

- **Parse the SWF once per character** (`main.rs`, `sprite_parser.rs`,
  `image_extractor.rs`). Several entry points still re-run `swf::decompress_swf` +
  `swf::parse_swf` on the same buffer (and `extract_xframe_transforms` runs twice).
  Thread the parsed `swf::Swf` through instead — biggest correctness-preserving perf win.
- **Per-multiname / per-method `String` clones in `abc_parser`** — store `name_idx`
  + look up on demand instead of cloning every multiname name.
- **Replace the hand-rolled SWF tag walker in `sound_extractor.rs`** (`parse_sounds`)
  with the `swf` crate's `DefineSound` parsing — deletes ~80 lines + a second parse.
- **`entity_gen` UUID seed can collide** when one animation frame has two boxes with
  the same instance name (`sym_box_{anim}_{inst}_{frame}`). Defensive comment added;
  verify SSF2 never duplicates an instance name within a frame, or fold depth into the seed.
- **`setlocal_0` → `self = …`** in `decompiler.rs` emits uncompilable Haxe if a frame
  script rebinds `this`-as-local to an arbitrary expression (the self-assign guard only
  covers `this`→`this`).
- **Duplicate fallback / split tables** — `apply_fallbacks` (`sprite_parser`) vs
  `apply_image_fallbacks` (`image_extractor`); `expand_split_anim` (`extractor`) vs the
  splitter rules. Lift each to one shared table.
- **`build_*_map` legacy block** in `api_mappings.rs` (~350 lines, see §11.12) — delete
  once JSONC parity is confirmed.

Treat each as "verify against `git log` before acting" — line numbers drift.

---

## 12. Prioritized next steps

Roughly in the order a fresh agent should tackle them.

1. **Re-check / fix the failing `.ssf`** (§11.3). Re-run the full
   `../ssf2-ssfs/` corpus against the current converter; surface any
   character that hard-fails and trace it.

2. **Shape-only head rasterizer.** Add a minimal SWF shape rasterizer (or
   pull one from `ruffle`) so `donkeykong` / `fox` / `marth` menu portraits
   actually contain pixels instead of placeholders.

3. **Verify Mario in FrayTools.** Re-run mario, open in FrayTools, scrub
   frame by frame; tune any remaining placement / rotation / scale issues.

4. **Projectile behaviour.** Replace the `// TODO` stubs in the projectile
   `<Pascal>Script.hx` generators with real translated logic (reuse the
   existing decompiler + JSONC rewriter pipeline).

5. **Validate stat scaling** against a handful of hand-tuned reference
   characters and tighten the `stats.jsonc :: multipliers`.

6. **Batch-convert the full 47-character roster** from `../ssf2-ssfs/`,
   triage which characters convert cleanly, and capture a per-character
   status list. Use the new `validation_warnings` block in
   `conversion_log.json` as a first-pass triage signal.

7. **Sweep the `build_*_map` deferred block** (§11.12). Confirm JSONC
   parity, delete the legacy code, simplify `api_mappings.rs` shape.

8. **Delete the path 2 enumeration fallback** in `detect_char_names`
   (§11.13) once a release has confirmed the constructor walker is
   universal. Collapses `derive_id_from_getter` /
   `derive_id_from_bundle_method_name` into one identity.

9. **Housekeeping:** make `tools/rebuild-sandbag.sh` path-relative; drop unused
   `tokio`.

---

## 13. Tips for the next agent

- **Smoke test = `sandbag`.** `./tools/rebuild-sandbag.sh` is the fast loop.
  Mario is the heavy, full-featured test (projectiles, costumes, big
  roster of moves).
- **`AGENT_CONTEXT.md` is the format bible** — formats are reverse-
  engineered and not documented anywhere else.
- **Edit the JSONC, not the Rust.** Most behaviour lives in
  `mappings/`. Adding a new API rename, marking a method as
  SSF2-only, tweaking a stat scaling — all JSONC edits, no rebuild
  needed beyond a fresh run.
- **Reach for the `dump_*` binaries** before guessing. They exist
  precisely so you can inspect a SWF without re-deriving the format.
  Example: `./target/release/dump_collision_box ../ssf2-ssfs/mario.ssf
  "a_air_forward"`.
- **Inputs are the sibling `../ssf2-ssfs/` folder**, not in this repo
  (`.gitignore` excludes `*.ssf`). `misc.ssf` lives there too and is
  needed for real costume colours.
- **Output is git-tracked partially** — `.hx`, `.json`, `.entity`,
  `.meta` are committed in `characters/`, but binary media (PNGs,
  WAVs) are git-ignored. Deleting `characters/` and re-running is safe;
  the entity / script files will regenerate identically (deterministic
  GUIDs).
- **GUIDs are deterministic** — re-running the converter is idempotent
  w.r.t. GUIDs, so diffs between runs reflect real logic changes, not
  GUID churn.
- **Always check `conversion_log.json`** after a conversion. The
  `unknown` list tells you which SSF2 API calls slipped past every
  mapping table; the `ssf2_only` list tells you which calls we
  deliberately gave up on. Both are good signals when something feels
  off in the converted output.
- **Reference resources** (from `AGENT_CONTEXT.md`): the official
  [Fraymakers character-template](https://github.com/Fraymakers/character-template)
  repo's `library/entities/character.entity` is ground truth for the
  entity format; the [Fraymakers community
  docs](https://github.com/aJewelofRarity/FraymakersDocs) and
  [SSF2 modding docs](https://ssf2-modding.readthedocs.io/) cover the
  rest.
- **The pipeline is fail-soft.** A stage that errors logs a warning and
  continues — always scan the run log, `conversion_stats.json`, and
  `conversion_log.json`. Don't assume a non-crashing run was a clean run.

---

## 14. Architectural history

Why the code looks the way it does today. These three migrations are **shipped**;
this is the condensed record of the reasoning behind them (it used to live in
separate `docs/` design plans).

### Path 1 → Path 2 (stat extraction source)

Each `.ssf` exposes character stats **twice**: inline on a per-character
`<X>Ext` class (`getOwnStats` / `getAttackStats` / `getProjectileStats` —
"path 1"), and bundled in a single `Main::get<X>()` method returning
`{ cData, aData, pData, iData }` ("path 2"). The two were confirmed
byte-for-byte equivalent across the complexity range, and path 2 is strictly
more complete: every normal character has a `Main::get<X>` bundle, **and** Sheik
exists only as `Main::getSheik` (she shares `ZeldaExt`, so path 1 missed her).
The converter dropped path 1 entirely (commits `71d2d39e`–`de7b480e`); stats
now come from the bundle (Stage A in `abc_parser`), behaviour code still from
`<X>Ext` where one exists (Stage B). The `derived_from` marker + `CharacterStats.hx`
TODO banner handle transformation forms (Giga Bowser, Wario Man) whose bundle's
`normalStats_id` mismatches their derived id.

### Constructor-walk detection

Path 2 detected characters by enumerating every `Main::get*` instance method.
A corpus audit (`src/bin/audit_main_iinit`) found a more direct signal: `Main`'s
constructor literally registers the roster — `self.register("characters",
[self.getX(), …])` — alongside `register("id", …)` / `register("guid", …)`.
Across all 45 character SSFs the constructor's `characters[]` array matched the
`get*` enumeration exactly (0 orphans, 0 id/filename disagreements), so detection
switched to walking the constructor (commit `d8e328af`), which also yields the
`ssf2_source` metadata (`package_id` / `package_guid` / `source_method`) now in
`conversion_log.json`. The old `get*` enumeration survives as a one-release
defensive fallback in `detect_char_names` (§11.13).

### Multi-character projects + universal PascalCase rename

Three SSFs ship character pairs (`zelda`→Zelda+Sheik, `bowser`→Bowser+Giga
Bowser, `wario`→Wario+Wario Man). Modelled on the in-the-wild Annie project
(one `.fraytools` project, a `manifest.json :: content[]` that's the sole source
of truth for what ships, asset binding by id not filename), the converter now:
(A) names all entity/script paths after the character id in PascalCase
(`Character.entity` → `<Pascal>.entity`, `scripts/Character/` → `scripts/<Pascal>/`
— commit `388e6faf`); (B) emits **one shared project per multi-character SSF**
with a merged manifest, per-character entities/scripts, collision-suffixed shared
files, and a `--per-character-projects` rollback flag (`4db74a89`); and (C) gives
multi-char projects per-character `library/audio/<char_id>/` subdirs (`7729ecda`).
One shared project per pair is a forward requirement for Fraymakers'
not-yet-shipped transformation API, which needs both forms in one project to swap
at runtime. Current output layout is in §9 and `README.md`.
