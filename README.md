# ssf2-to-fraymakers

**Turn a Super Smash Flash 2 character into a ready-to-open Fraymakers mod — automatically.**

Point this tool at a single SSF2 `.ssf` file and it reverse-engineers the whole
character and writes a complete, FrayTools-compatible
[Fraymakers](https://www.fraymakersthegame.com/) character package — sprites,
animations, collision boxes, costumes, sounds, projectiles, effects, menu
portraits, and decompiled gameplay logic. No manual asset ripping, no rebuilding
timelines by hand, no copy-pasting stat tables. One command in, a FrayTools
project out.

## What it does

A single conversion pulls an entire SSF2 character across to Fraymakers, end to end:

- **Sprites & animations** — extracts every animation frame to a PNG and rebuilds
  the full animation timelines, with per-frame placement (position, rotation,
  scale, flip) faithfully reproduced. Sheared frames Flash could draw but FrayTools
  can't are pre-baked so they still look right.
- **Collision data** — per-frame hitboxes, hurtboxes, and grab / ledge / reflect /
  absorb / grab-hold boxes, plus an auto-fitted ECB body diamond — validated
  sub-pixel against the SSF2 source.
- **Gameplay logic** — a from-scratch ActionScript (AVM2/ABC) decompiler
  reconstructs the character's frame scripts and behaviour into readable Haxe,
  rewritten to the Fraymakers API.
- **Stats & hitboxes** — movement physics and per-attack hitbox data land in
  clean, data-driven `.hx` files.
- **Costumes & palettes** — all ~15 SSF2 costumes per character become Fraymakers
  palettes (with a k-means palette fallback when `misc.ssf` isn't available).
- **Sounds** — character audio extracted to WAV (Nellymoser / MP3 / ADPCM via
  `ffmpeg`).
- **Projectiles & effects** — discovered automatically and emitted as their own
  entities, wired back into the character's scripts.
- **Menu & CSS portraits** — character-select head and HUD portrait variants.
- **30 → 60 fps** — SSF2 runs at 30 fps, Fraymakers at 60; every timing value is
  doubled in lockstep so playback speed is preserved exactly.
- **Deterministic output** — every GUID is derived from the character id, so
  re-running the converter is reproducible and diffs reflect real changes, not churn.
- **Multi-character SSFs** — files that ship a pair (Zelda + Sheik, Bowser + Giga
  Bowser, Wario + Wario Man) convert into a single shared FrayTools project.

## What's in the repo

- **`ssf2_converter`** — the core command-line converter (Rust). The one tool you
  need to go from `.ssf` to a FrayTools package.
- **Desktop app** — a cross-platform GUI (`ssf2-converter-gui`): drag-and-drop a
  `.ssf`, watch a progress bar, and get a summary of any unhandled calls — no
  terminal required. On macOS, `./make-app.sh` wraps it in a double-clickable
  `SSF2 Converter.app`.
- **Editable conversion config** — JSONC mapping tables in `mappings/` drive API
  translation, stat scaling, animation names, and hitbox-field mapping. Tune the
  conversion by editing data, not recompiling code.
- **Validation harnesses** (`tools/`) — automation that drives the user's own
  FrayTools (box geometry + one-click publish) and the Fraymakers engine (load /
  spawn / drive-a-move / read state) to prove a converted character actually works.
  See [`TESTING.md`](TESTING.md).
- **Diagnostic toolkit** — ~17 `dump_*` / `check_*` SWF- and format-inspection
  binaries used to reverse-engineer and debug conversions.

---

# Developer guide

Everything below is for building, running, and hacking on the converter.

## Requirements

- Rust (stable) — required to build the converter.
- `ffmpeg` on `PATH` — required at runtime for sound conversion (Nellymoser / MP3 / ADPCM → WAV).
  If `ffmpeg` is missing, conversion still succeeds; sounds are skipped with a warning.

## Build

```bash
cargo build --release
```

Binaries land in `target/release/`:

- `ssf2_converter` — the main character converter (this is the only binary you need).
- ~17 `dump_*` / `check_*` diagnostic binaries — see [`DEVELOPMENT.md`](DEVELOPMENT.md) §"Diagnostic binaries".

## Usage

```bash
# One-step conversion. Costumes are auto-extracted from misc.ssf if it's next to the input.
./target/release/ssf2_converter ../ssf2-ssfs/mario.ssf

# Explicit output dir + explicit misc.ssf path
./target/release/ssf2_converter ../ssf2-ssfs/mario.ssf \
    --output ./characters --misc-ssf ../ssf2-ssfs/misc.ssf

# A specific character from a multi-character .ssf
./target/release/ssf2_converter ../ssf2-ssfs/zelda.ssf --name sheik

# Force a multi-character SSF to emit one project PER character
# (the pre-merge layout) instead of one shared project
./target/release/ssf2_converter ../ssf2-ssfs/zelda.ssf --per-character-projects
```

Costume palettes are extracted in-process from `misc.ssf` (no separate step, no temp files left
behind). If `misc.ssf` isn't found next to the input, the converter falls back to a k-means
palette derived from the character's idle sprites.

Output goes to `./characters/<name>/` — a complete Fraymakers character package.

## Output structure

Entity files and the scripts subdir are named after the character in
**PascalCase** (`Mario.entity`, `library/scripts/Mario/`). For a
single-character SSF:

```
characters/mario/
├── mario.fraytools                       FrayTools project file (project settings only)
├── conversion_stats.json                 debug summary of the run
├── conversion_log.json                   unhandled / SSF2-only calls + ssf2_source + validation
└── library/
    ├── manifest.json (+ .meta)           one type:"character" content[] entry
    ├── costumes.palettes (+ .meta)        15 SSF2 costumes as FM palettes
    ├── entities/
    │   ├── Mario.entity                   main character entity (PascalCase = character id)
    │   ├── Menu.entity                    character-select head / HUD portraits
    │   ├── <projectile>.entity            one per discovered projectile
    │   └── <effect>.entity                one per discovered VFX sprite
    ├── sprites/
    │   ├── *.png                          extracted frame bitmaps
    │   ├── *.png.meta                     GUID sidecar per PNG
    │   └── palette_preview.png (+ .meta)
    ├── scripts/
    │   ├── Mario/                         <Pascal>/ — was Character/
    │   │   ├── CharacterStats.hx          movement physics
    │   │   ├── HitboxStats.hx             per-attack hitbox data
    │   │   ├── AnimationStats.hx          animation flags
    │   │   ├── Script.hx                  decompiled character logic
    │   │   └── *.hx.meta
    │   └── Projectile/
    │       └── <Pascal>{Script,Stats,HitboxStats,AnimationStats}.hx (+ .meta)
    └── audio/
        ├── *.wav                          extracted sounds (flat layout for single-char)
        └── *.wav.meta                     per-sound content sidecar
```

### Multi-character SSFs

Three SSFs ship character pairs (`zelda` → Zelda + Sheik, `bowser` →
Bowser + Giga Bowser, `wario` → Wario + Wario Man). These emit **one
shared project** (a forward requirement for Fraymakers' future
transformation API, which needs both forms in the same project). For
`zelda.ssf`:

```
characters/zelda/
├── zelda.fraytools                       one project for both characters
├── conversion_log.json                   project-scoped, characters:[…] array
└── library/
    ├── manifest.json (+ .meta)           TWO type:"character" entries (zelda + sheik)
    ├── costumes.palettes (+ .meta)        Zelda's (constructor-walk slot 0)
    ├── costumes.palettes2 (+ .meta)       Sheik's (slot 1 → numeric collision suffix)
    ├── palette_preview.png  / .png2       same suffix rule
    ├── entities/
    │   ├── Zelda.entity   Sheik.entity    one character entity each
    │   ├── Zelda_Menu.entity  Sheik_Menu.entity   per-character portraits
    │   └── <projectile>.entity            shared projectile entities
    ├── scripts/
    │   ├── Zelda/   Sheik/                 per-character script subdirs
    │   └── Projectile/                     shared
    ├── sprites/                            shared (PNG names are SSF2 <char>_fla.-prefixed)
    └── audio/
        ├── zelda/*.wav                     per-character audio subdirs
        └── sheik/*.wav                     (multi-char only; single-char stays flat)
```

`--per-character-projects` reverts a multi-character SSF to the
single-character layout (one standalone project per character).
Transformation forms (Giga Bowser, Wario Man) carry a TODO banner in
their `CharacterStats.hx` explaining the manual wiring still owed for
the not-yet-shipped FM transformation API.

## How costumes work

SSF2 stores costume data in `misc.ssf` → `Misc.as` → `getCostumeData()`. Each character has
roughly 15 costumes:

- **Default / Red / Green / Blue** — base + team-colour variants.
- **Alt 1–11** — additional unlockable costumes.

Each costume contains a source-colour list (the base sprite palette, identical across all
costumes) and a parallel replacements list (what those colours become). The converter maps
these directly to Fraymakers palette colour slots + maps.

## Configuring the conversion

Most of what the converter does is driven by editable JSONC files under `mappings/`:

- `mappings/commands.jsonc` — universal SSF2 → Fraymakers API command conversions:
  literal replacements, regex replacements, multi-target call splits, frame-count flags
  for 30→60 fps doubling, attachEffect-prop routing, global-VFX names, and the
  passthrough / SSF2-only lists used by the conversion log.
- `mappings/character/animations.jsonc` — SSF2 xframe / sprite-label → Fraymakers
  animation-name table.
- `mappings/character/stats.jsonc` — every value in `CharacterStats.hx`: field-key
  preferences, per-stat multipliers, integer offsets, expression-based derivations
  (evaluated with `fasteval`), and flat constants.
- `mappings/character/hitbox_stats.jsonc` — SSF2 → Fraymakers hitbox field mapping
  (max-over-source-keys, optional `isframe` flag for 30→60 fps doubling).

These files ship as `include_str!`'d compile-time defaults; an on-disk copy (next to the
binary or in the working directory) overrides without recompiling.

## Documentation

- [`DEVELOPMENT.md`](DEVELOPMENT.md) — developer guide: build, pipeline, modules,
  current status, known issues, the code-quality backlog, and architectural history.
- [`AGENT_CONTEXT.md`](AGENT_CONTEXT.md) — authoritative SSF2 / Fraymakers format
  reference. Read this when working on `entity_gen.rs`, `image_extractor.rs`, or
  `sprite_parser.rs`.
- [`CONTRIBUTING.md`](CONTRIBUTING.md) — module → doc-section mapping + the
  per-change checklist. Read before landing a substantial change so the docs
  stay current.
- [`TESTING.md`](TESTING.md) — the FrayTools-side and Fraymakers-engine-side
  validation harnesses, the end-to-end iteration loop, the engine RE map, and
  in-engine validation status.

## Licence

This project is licensed under the **MIT License** — see [`LICENSE`](LICENSE).
Dependency attribution (the Ruffle `swf` crate and others) is collected in
[`NOTICE.md`](NOTICE.md).

## Notes

- Original SSF2 character data © McLeodGaming; Fraymakers / FrayTools © Fraymakers.
  This tool is for personal mod development against assets you already own. Never
  commit or publish their source, bytecode, or assets — see [`NOTICE.md`](NOTICE.md)
  "Reverse-engineering & copyright boundary".
- Test inputs (`*.ssf`, `misc.ssf`) are deliberately not in this repo (`.gitignore`
  excludes them). They live in a sibling `ssf2-ssfs/` directory.
```
