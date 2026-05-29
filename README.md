# ssf2-to-fraymakers

Converts Super Smash Flash 2 character data to [Fraymakers](https://www.fraymakersthegame.com/) mod format.

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
./target/release/ssf2_converter ../ssf2-ssfs/foo.ssf --name mario
```

Costume palettes are extracted in-process from `misc.ssf` (no separate step, no temp files left
behind). If `misc.ssf` isn't found next to the input, the converter falls back to a k-means
palette derived from the character's idle sprites.

Output goes to `./characters/<name>/` — a complete Fraymakers character package.

## Output structure

```
characters/mario/
├── mario.fraytools                       FrayTools project file
├── conversion_stats.json                 debug summary of the run
├── conversion_log.json                   unhandled / SSF2-only calls (counts per name)
└── library/
    ├── manifest.json (+ .meta)
    ├── costumes.palettes (+ .meta)        15 SSF2 costumes as FM palettes
    ├── entities/
    │   ├── Character.entity               main character entity
    │   ├── menu.entity                    character-select head / HUD portraits
    │   ├── <projectile>.entity            one per discovered projectile
    │   └── <effect>.entity                one per discovered VFX sprite
    ├── sprites/
    │   ├── *.png                          extracted frame bitmaps
    │   ├── *.png.meta                     GUID sidecar per PNG
    │   └── palette_preview.png (+ .meta)
    ├── scripts/
    │   ├── Character/
    │   │   ├── CharacterStats.hx          movement physics
    │   │   ├── HitboxStats.hx             per-attack hitbox data
    │   │   ├── AnimationStats.hx          animation flags
    │   │   ├── Script.hx                  decompiled character logic
    │   │   └── *.hx.meta
    │   └── Projectile/
    │       └── <Pascal>{Script,Stats,HitboxStats,AnimationStats}.hx (+ .meta)
    └── audio/
        ├── *.wav                          extracted sounds
        └── *.wav.meta                     per-sound content sidecar
```

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

- [`DEVELOPMENT.md`](DEVELOPMENT.md) — developer guide: build, pipeline, modules, current
  status, known issues, and next steps.
- [`AGENT_CONTEXT.md`](AGENT_CONTEXT.md) — authoritative SSF2 / Fraymakers format
  reference. Read this when working on `entity_gen.rs`, `image_extractor.rs`, or
  `sprite_parser.rs`.
- [`CONTRIBUTING.md`](CONTRIBUTING.md) — module → doc-section mapping + the
  per-change checklist. Read before landing a substantial change so the docs
  stay current.
- [`docs/codebase_analysis.md`](docs/codebase_analysis.md) — optimization /
  cleanup / bug audit with file-and-line refs. Carries a "what's been done
  since" status banner at the top.
- [`docs/path2_unification_plan.md`](docs/path2_unification_plan.md) +
  [`docs/constructor_walk_detection.md`](docs/constructor_walk_detection.md)
  — historical pre-implementation plans for the path 1 → path 2 migration
  and the constructor-walk detection that followed. Implemented; kept for
  architectural history.
- [`docs/anim_split_rules.json`](docs/anim_split_rules.json) — historical reference for
  the multi-label split patterns (the splitter logic itself now lives in
  `src/anim_splitter.rs` as hardcoded match arms, not loaded from this file).

## Notes

- Original SSF2 character data © McLeodGaming — this tool is for mod development only.
- Test inputs (`*.ssf`, `misc.ssf`) are deliberately not in this repo (`.gitignore`
  excludes them). They live in a sibling `ssf2-ssfs/` directory.
