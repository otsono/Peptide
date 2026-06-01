# SSF2 → Fraymakers Converter: Agent Context

This document is the authoritative reference for AI agents working on this codebase.
Both SSF2's SWF format and Fraymakers' entity format are largely undocumented.
Everything here was reverse-engineered from first principles during development.

> **Copyright boundary — do not paste, do not publish.** SSF2 (© McLeodGaming)
> and Fraymakers / FrayTools (© Fraymakers) are proprietary. Everything in this
> file is described **in our own words** for interoperability; the JSON shapes
> below are **illustrative examples we authored**, not copied from any copyrighted
> file. Never add verbatim third-party source, bytecode, decompiled output, or
> assets to the repo — see [`NOTICE.md`](NOTICE.md) "Reverse-engineering &
> copyright boundary".

> **Structure note (current).** **Peptide is the parent product** and the single
> shipping binary at the repo root (`src/`, package `peptide`). The SSF2 →
> Fraymakers converter is now a **library crate** at `crates/ssf2-converter/`
> (still package-named `ssf2_converter`); conversion runs in-process via
> `run_conversion` / `peptide convert <file.ssf>` — there is no standalone
> converter binary and no egui GUI. Parts of this file predate that move and may
> say "the converter" / `ssf2_converter input.ssf`; read those as the library
> crate and `peptide convert …`. Peptide specifics:
> [`docs/PEPTIDE_README.md`](docs/PEPTIDE_README.md).

> **Architecture convention — minimum bytecode, maximum hscript (read before every commit).**
> Hand-emitted HashLink bytecode in `src/main.rs` (`connect_edit` and its `inject_*`
> helpers) is the most expensive and fragile code in the repo: a wrong register type or
> jump offset silently corrupts the engine and can only be caught by launching the game.
> So: **add only the minimum amount of bytecode needed to make a thing possible, and
> implement as much of the actual behaviour as you can in `commands.hsx`.** Bytecode is
> allowed — just keep it to the irreducible engine-side primitive that hscript/host can't
> otherwise reach. Where each kind of logic belongs:
> - **`commands.hsx` (hscript in the engine's own interpreter via the `e` hook) — the
>   DEFAULT for engine-side behaviour.** Anything you can express by calling the script
>   API or the bound values (p0/match/CState/…) goes here, not in bytecode.
> - **`src/interpreter.rs` (host-side Rust, runs in the Peptide process)** for anything
>   Peptide can do itself: translating commands, parsing `error.log`, building diagnostics,
>   routing TCP channels, reading character files. No engine round-trip needed → host-side.
> - **Bytecode (`connect_edit`/`inject_*`) — only the minimum primitive** hscript/host
>   can't reach: the socket bootstrap, per-frame dispatch, the `s`/`x`/`l`/`c`/`e` hooks,
>   binding a value into the eval scope, or pulling one fact the others genuinely can't get
>   (e.g. the failing stage id in `inject_stage_diag`). Push everything ELSE up into hscript.
>
> Before adding to `connect_edit`/`inject_*`, ask: is this the *smallest* engine primitive
> that unblocks the feature, with the rest done in `commands.hsx` / `interpreter.rs`?

Cross-reference: [`DEVELOPMENT.md`](DEVELOPMENT.md) covers build / pipeline / modules and
the current set of mapping JSONC files; this file covers the **input** and **output**
formats themselves.

---

## Reference Resources

Always check these before guessing about either format:
- **Fraymakers community docs**: https://github.com/aJewelofRarity/FraymakersDocs/tree/main
- **Fraymakers character template** (official): https://github.com/Fraymakers/character-template
- **SSF2 modding docs**: https://ssf2-modding.readthedocs.io/en/latest/reference/index.html

The character template's `library/entities/character.entity` is the ground truth for entity format.
It is a 2.67 MB file — use `curl | python3` to parse it programmatically, not web_fetch (truncates).

```bash
curl -s https://raw.githubusercontent.com/Fraymakers/character-template/main/library/entities/character.entity | python3 -c "
import sys, json; obj = json.load(sys.stdin)
# inspect layers, keyframes, symbols as needed
"
```

---

## What this tool does

Converts Super Smash Flash 2 (SSF2) character `.ssf` files into Fraymakers character packages
compatible with FrayTools. It extracts:
- Bitmap images (PNG sprites per frame)
- Collision box data (hitboxes, hurtboxes, grab/ledge/reflect/absorb boxes) per animation frame
- Frame scripts (decompiled from ABC bytecode and rewritten through the JSONC command table)
- Sound references (WAV via `ffmpeg`)
- Palette / costume data (15 costumes per character from `misc.ssf`)
- Projectile and effect sprites as standalone `.entity` files
- A menu / portrait entity for the character-select screen

Output is a FrayTools character package directory (full layout in [`README.md`](README.md) and
[`DEVELOPMENT.md`](DEVELOPMENT.md) §7).

---

## SSF2 (.ssf) File Format

`.ssf` files are SSF-wrapped SWF files. The unwrapper lives in `src/ssf.rs`:

- A `.ssf` is either a raw SWF (`FWS` / `CWS` / `ZWS` magic — passed through), or
- An SSF-wrapped file: `u32 swf_len` + `u32 garbage_header_size` + zlib payload.

After unwrapping, `swf_parser::parse` uses the Ruffle `swf` crate (`decompress_swf` +
`parse_swf`) to turn the bytes into a tag tree.

### SWF Structure for SSF2 Characters

```
SymbolClass        → maps char_id (u16) → class name string
DefineBitsLossless → raw bitmap data (palette / RGBA / ARGB)
DefineBitsJpeg3    → JPEG image data (with optional zlib alpha mask)
DefineShape        → shape with a bitmap fill (wraps a DefineBits*)
DefineSprite       → animation timeline (PlaceObject/ShowFrame/RemoveObject tags)
DoABC / DoABC2     → AS3 bytecode blocks (the character's logic + stats + costume tables)
DefineSound        → audio (Nellymoser, MP3, ADPCM)
```

### Animation Sprites

Each character animation lives in a named `DefineSprite`:
- Name format: `{char}_fla.{AnimLabel}_{index}` e.g. `mario_fla.FAir_42`
- The root MC (main timeline) places these sprites at specific frame labels (`stance` placements)
- Each animation sprite contains a sequence of PlaceObject/ShowFrame/RemoveObject tags

The character's main timeline (root) is a DefineSprite whose `SymbolClass` name matches
the character id exactly (e.g. `mario`, `fox`). Frame labels on the root MC drive the
`xframe` map: SSF2 frame methods assign an animation label to an `xframe` field, which
the extractor recovers from the ABC bytecode.

### Root MC Transform

Every animation sprite is placed by the root MC with a transform:
```
tx, ty  = world offset of the character origin (typically negative, e.g. -24.70, -55.30)
sx, sy  = character scale (typically 1.1 for Mario)
```
All child positions must be composed through this transform via the full affine matrix
(a, b, c, d, tx, ty) — not just translation × scale, because some root placements rotate.
See `sprite_parser::XframeTransform` and `image_extractor::ImageLocalMatrix`.

### SWF Matrix Decomposition

SWF PlaceObject matrices use `Fixed16` (fixed-point) values:
```
a, b, c, d = matrix components (b and c carry shear/rotation)
tx, ty     = translation in TWIPS (divide by 20 to get pixels)
```

Decompose into scale/rotation:
```rust
scale_x = sqrt(a² + b²)
scale_y = sqrt(c² + d²)
// Flip detection: encode the sign in scale_y when the determinant is negative.
let det = a*d - b*c;
let sy  = if det < 0.0 { -scale_y } else { scale_y };
rotation_deg = atan2(b, a).to_degrees()
```

### Rotation convention — both SWF and FrayTools are CW-positive

SWF's `atan2(b, a)` in y-down screen space and FrayTools' rotation field use the
**same CW-positive convention**. **Do not negate.** The converter emits rotation
values normalized to the `[0, 360)` range:

```rust
rotation: round2(((swf_rotation % 360.0) + 360.0) % 360.0)
```

This applies to **both** IMAGE symbols and COLLISION_BOX symbols (commit `40fad65d`
brought collision-box rotation in line with the IMAGE-symbol convention that was
already established by commit `f472a2dd`). If you spot an older comment claiming
"always negate rotation", it's stale — the code's `((swf_rotation % 360.0) + 360.0)
% 360.0` formula is the source of truth.

### Skew handling

FrayTools' IMAGE keyframe can express translation + rotation + scaleX + scaleY, but
**not shear**. When a SWF placement matrix has shear (non-perpendicular x/y column
vectors), `image_extractor::prerender_skewed_frames` bakes the world-space linear
part into a fresh PNG and rewrites the placement as a plain translation. Shear is
detected via `ImageLocalMatrix::has_skew()`. Non-sheared placements (pure rotation +
scale + flip) take the faithful scale+rotation path.

### Collision Boxes

SSF2 encodes ALL collision-box data in the SWF timeline, not in AS3 code.

The collision-box character (typically called `CollisonBox_6` — note SSF2's internal
typo "Collison") is a small square shape (~100×100 unit, scaled per instance by the
PlaceObject matrix). `sprite_parser::find_collision_box_base_size` measures the
shape's true bounds at runtime (not hardcoded):
```
width   = |scale_x| * BASE_SIZE
height  = |scale_y| * BASE_SIZE
center_x = tx_pixels   (tx/ty ARE the centre, not top-left)
top_left_x = center_x - width/2
top_left_y = center_y - height/2
```

#### Box Instance Names → BoxType

The SSF2 instance name on the PlaceObject determines the FM box type. See
`sprite_parser::BoxType::from_instance_name` and `entity_gen::box_type_to_fm`:

| SSF2 instance name | Fraymakers type | Notes |
|---|---|---|
| `attackBox`, `attackBox2`… | `HIT_BOX` | Active hitbox |
| `hitBox`, `hitBox2`… | `HURT_BOX` | Hurtbox (SSF2 "hit" = FM "hurt") |
| `hurtBox` | `HURT_BOX` | Hurtbox (alternate name) |
| `grabBox` | `GRAB_BOX` | Grab range |
| `itemBox` | `HURT_BOX` | Item pickup — no native FM equivalent; emitted as hurtbox |
| `touchBox` | `GRAB_HOLD_POINT` | **POINT layer** (not COLLISION_BOX) |
| `shieldBox` | `REFLECT_BOX` | |
| `reflectBox` | `REFLECT_BOX` | |
| `absorbBox` | `COUNTER_BOX` | |
| `ledgeBox` / `ledgegrabbox` | `LEDGE_GRAB_BOX` | |
| anything else `*box` | `HURT_BOX` | Fallback |

**`touchBox` → grabholdpoint (verified):**
- Layer type is `POINT`, not `COLLISION_BOX`.
- Layer name is `grabholdpoint<N>`.
- `pluginMetadata["com.fraymakers.FraymakersMetadata"]` uses `pointType: "GRAB_HOLD_POINT"`.
- Symbol is type `POINT` with just `x` / `y` / `alpha` / `color` / `rotation` (bottom-centre
  of the touchBox area — where the opponent's feet anchor).
- Verified against the official Fraymakers character template `grab_hold` animation.

#### ItemBox Special Case

`itemBox` (typically id=991) is placed relative to the **hand attachment point**
(bottom-centre of the box). The PlaceObject tx/ty is the hand position; the inner
shape geometry hangs upward from it (inner_w ≈ 3.7, inner_h ≈ 21.9 pixels). The
emitted COLLISION_BOX symbol uses `pivotY = height` (instead of `height / 2`) so
rotation pivots around the hand. `itemBox` is the only routinely rotated collision
box, so the pivot-at-bottom convention matters in practice.

### Image Sprites

Each animation's visual content sits inside the animation's DefineSprite as a stack
of `PlaceObject` tags placing one or more named `DefineSprite`s (the sub-sprites)
or unnamed effect sprites. `image_extractor::build_anim_frame_images` walks each
animation's display list and records, per frame, every placed image with its full
world-space affine matrix (`FrameImageEntry`).

#### Effect Sprites

Some animations contain nested effect movieclips (e.g. `mario_fla.ChargeSpark_25`).
These are `_fla.`-named sub-sprites that are not top-level animation containers.
The image extractor flattens them — composes each effect frame's content into the
parent timeline via matrix multiplication (`ImageLocalMatrix::compose`).

Pure-vector effect shapes (solid-colour fills, no bitmap) **cannot be rendered**
without a full SWF vector rasterizer. They are silently skipped — only bitmap-backed
shapes are exported. Affected examples include some sparkle / charge effects.

#### shape_to_bitmap Map

`DefineShape` tags may have multiple fill entries:
1. `id = 65535` — the SWF null / clipping bitmap (**always skip**).
2. The real bitmap id.

The extractor takes the first non-65535 bitmap fill. `image_extractor::shape_pivot`
captures where the shape's local (0,0) lands inside its bitmap (computed from the
fill matrix `tx / (a/20)`, `ty / (d/20)`); this drives image-placement pivot offsets
in the emitted entity.

#### Unnamed Sprites

Some sub-sprites and shapes have no `SymbolClass` entry. Those are resolved via the
display list (their inner content placed in by depth/timeline). They produce
synthetic `id_NNNN` names internally and are stripped from the final entity output
if they carry no real image content.

---

## Fraymakers Entity Format (.entity)

The `.entity` file is a JSON document consumed by FrayTools. It's the core of a character
package. All GUIDs are deterministic — seeded by `{char_id}::{context}` and run through
UUID v5 (SHA-1 namespace) in `src/uuid_gen.rs`.

### Top-Level Structure

```json
{
  "animations": [...],        // per-animation objects
  "export": true,
  "guid": "...",              // deterministic GUID for this entity
  "id": "mario",              // character id (lowercase)
  "keyframes": [...],         // ALL keyframes from all animations, flat array
  "layers": [...],            // ALL layers from all animations, flat array
  "paletteMap": null,         // or { "paletteCollection": "...", "paletteMap": "..." }
  "pluginMetadata": {
    "com.fraymakers.FraymakersMetadata": {
      "objectType": "CHARACTER",
      "version": "0.4.0"
    }
  },
  "plugins": ["com.fraymakers.FraymakersMetadata"],
  "symbols": [...],           // ALL symbols (IMAGE, COLLISION_BOX, COLLISION_BODY, POINT)
  "tags": [],
  "terrains": [],
  "tilesets": [],
  "version": 14
}
```

### Animation Object

```json
{
  "$id": "...",         // deterministic GUID
  "name": "idle",       // Fraymakers animation name
  "layers": ["..."],    // ordered array of layer $ids
  "pluginMetadata": {}
}
```

Empty animations (those whose IMAGE timeline carries no real symbols across all
frames) are dropped from the final entity. The exception is a small allowlist
controlled by `populated_jabs` in `haxe_gen.rs`: when the character has exactly two
populated jabs the converter keeps `jab3` as an empty placeholder so the Script.hx
jab-chain references don't break at runtime.

### Layer Types

Every animation typically has this layer stack (in order):
1. `LABEL` — animation name label on frame 0, plus any inner FrameLabel tags.
2. `FRAME_SCRIPT` — per-frame Haxe code (`code` is the function body, no wrapper).
3. `COLLISION_BODY` — character ECB/body diamond, per frame.
4. `COLLISION_BOX` — one layer per box instance (hitbox0, hurtbox0, etc.).
5. `POINT` — one layer per `touchBox` (grab-hold point).
6. `IMAGE` — one layer per depth slot (Image 0, Image 1, …).

#### LABEL Layer

```json
{ "$id": "...", "name": "Labels", "type": "LABEL",
  "keyframes": ["kf_id"], "hidden": false, "locked": false, "pluginMetadata": {} }
```

LABEL keyframe:
```json
{ "$id": "...", "type": "LABEL", "length": 1, "name": "idle", "pluginMetadata": {} }
```

#### FRAME_SCRIPT Layer

```json
{ "$id": "...", "name": "Scripts", "type": "FRAME_SCRIPT",
  "keyframes": [...], "hidden": false, "locked": false,
  "language": "", "pluginMetadata": {} }
```

FRAME_SCRIPT keyframe — `code` is the function body **only** (no `function name() {`
wrapper). Blank frames use `"code": ""`:

```json
{ "$id": "...", "type": "FRAME_SCRIPT", "length": 1,
  "code": "self.playAnimation(\"jab2\");", "pluginMetadata": {} }
```

#### COLLISION_BODY Layer (ECB / body diamond)

The ECB is a 4-vertex diamond: foot (bottom), head (top), and two hip vertices at the
sides at the foot/head midpoint. `entity_gen.rs` computes the per-frame diamond as the
axis-aligned bounding box of that frame's HURTBOX-typed collision boxes, so the body
auto-fits the character pose. Consecutive frames with identical bodies are run-length
encoded into a single keyframe.

```json
{ "$id": "...", "name": "Body", "type": "COLLISION_BODY", "keyframes": [...],
  "defaultAlpha": 0.5, "defaultColor": "0xffa500",
  "defaultFoot": 0, "defaultHead": 86, "defaultHipWidth": 40,
  "defaultHipXOffset": 0, "defaultHipYOffset": 0, "pluginMetadata": {} }
```

COLLISION_BODY symbol:
```json
{ "$id": "...", "alpha": null, "color": null,
  "foot": 0, "head": 86, "hipWidth": 40, "hipXOffset": 0, "hipYOffset": 0,
  "pluginMetadata": {}, "type": "COLLISION_BODY" }
```

#### COLLISION_BOX Layer

```json
{ "$id": "...", "name": "hitbox0", "type": "COLLISION_BOX", "keyframes": [...],
  "defaultAlpha": 0.5, "defaultColor": "0xff0000",
  "pluginMetadata": {
    "com.fraymakers.FraymakersMetadata": {
      "collisionBoxType": "HIT_BOX",
      "index": 0
    }
  } }
```

COLLISION_BOX symbol:
```json
{ "$id": "...", "alpha": 0.5, "color": "0xff0000",
  "pivotX": 24.0, "pivotY": 13.0,         // centre of the box (bottom-centre for itemBox)
  "pluginMetadata": {},
  "rotation": 12.5,                        // CW-positive degrees, normalized to [0, 360)
  "scaleX": 48.0,                          // width in pixels
  "scaleY": 26.0,                          // height in pixels
  "type": "COLLISION_BOX",
  "x": -54.0, "y": -35.0 }                 // top-left in world space, y-down
```

COLLISION_BOX keyframe — blank frame uses `"symbol": null`:
```json
{ "$id": "...", "type": "COLLISION_BOX", "length": 2,
  "symbol": "sym_$id_or_null", "tweened": false, "tweenType": "LINEAR",
  "pluginMetadata": {} }
```

**Fraymakers collision-box type strings** (used in `collisionBoxType`):

| String | Description |
|---|---|
| `HIT_BOX` | Active hitbox |
| `HURT_BOX` | Hurtbox |
| `GRAB_BOX` | Grab range |
| `LEDGE_GRAB_BOX` | Ledge grab |
| `REFLECT_BOX` | Reflect / shield (from SSF2 `shieldBox` and `reflectBox`) |
| `COUNTER_BOX` | Counter / absorb (from SSF2 `absorbBox`) |

#### POINT Layer

Used for named points in space (grab hold position, pivot points, etc.).

```json
{ "$id": "...", "name": "grabholdpoint0", "type": "POINT", "keyframes": [...],
  "hidden": false, "locked": false,
  "pluginMetadata": {
    "com.fraymakers.FraymakersMetadata": {
      "pointType": "GRAB_HOLD_POINT"
    }
  } }
```

POINT symbol — coordinate only, no size:
```json
{ "$id": "...", "alpha": 1, "color": "0xff0000",
  "pluginMetadata": {}, "rotation": 0, "type": "POINT",
  "x": 9.62, "y": -9.7 }
```

POINT keyframe:
```json
{ "$id": "...", "type": "POINT", "length": 1,
  "symbol": "sym_$id_or_null", "tweened": false, "tweenType": "LINEAR",
  "pluginMetadata": {} }
```

**Known point types** (in `pluginMetadata.com.fraymakers.FraymakersMetadata.pointType`):
- `GRAB_HOLD_POINT` — where a grabbed opponent is positioned (from SSF2 `touchBox`).
- `PIVOT_POINT` — rotation pivot override.

#### IMAGE Layer

```json
{ "$id": "...", "name": "Image 0", "type": "IMAGE", "keyframes": [...],
  "hidden": false, "locked": false, "pluginMetadata": {} }
```

IMAGE symbol — **one is created per placement** (each anim/slot/frame gets its own
symbol; symbols are NOT shared across frames because the world matrix varies):

```json
{ "$id": "...", "alpha": 1,
  "imageAsset": "meta_guid",         // GUID from the PNG's .meta sidecar (NOT a file path)
  "pivotX": 0, "pivotY": 0,          // bitmap (0,0); shape_pivot offsets are folded into x/y
  "pluginMetadata": {},
  "rotation": 350.5,                  // CW-positive degrees, normalized to [0, 360)
  "scaleX": 1.1, "scaleY": 1.1,       // signed: negative = flip
  "type": "IMAGE",
  "x": -54.0, "y": -35.0 }            // world-space, y-down
```

The model FrayTools uses is: `world_pt = (x, y) + R(rot) · diag(sx, sy) · local_pt`.
The converter mirrors this exactly — `x` / `y` come from the SWF world matrix's
translation plus a shape-pivot correction; rotation and scale come from the matrix's
linear part. Sheared placements are pre-rendered to a baked bitmap (see "Skew handling"
above) and emitted as identity-rotation, identity-scale plain translations.

IMAGE keyframe:
```json
{ "$id": "...", "type": "IMAGE", "length": 3,
  "symbol": "sym_$id_or_null", "tweened": false, "tweenType": "LINEAR",
  "pluginMetadata": {} }
```

Consecutive frames with the same symbol AND identical world matrix are run-length
encoded into one keyframe.

### .meta Sidecar Files

Every PNG needs a `.meta` JSON file at the same path + `.meta` extension:
```json
{ "export": false, "guid": "deterministic-guid", "id": "",
  "pluginMetadata": {}, "plugins": [], "tags": [], "version": 2 }
```

The GUID in the `.meta` file is what `imageAsset` in IMAGE symbols references — not the
PNG file path.

### Per-projectile and per-effect entities

Each discovered projectile (SSF2 sprite carrying an `attack_idle` FrameLabel + a `stance`
PlaceObject) gets its own `library/entities/<name>.entity` plus a set of script files at
`library/scripts/Projectile/<Pascal>{Script,Stats,HitboxStats,AnimationStats}.hx`.
Animation names come from the inner sprite's FrameLabel tags when present, else fall back
to the FM template trio `projectileSpawn` / `projectileIdle` / `projectileDestroy`.

Each discovered effect (root-level SymbolClass'd sprite that's neither a projectile, the
character itself, the head sprite, an `_fla.*` timeline, nor a HUD/icon) is emitted as a
plain `library/entities/<name>.entity` with one IMAGE layer per inner FrameLabel-derived
segment. No scripts, no stats. The character's `Script.hx` references these via
`match.createVfx(new VfxStats({…}), self)` calls produced by the `attachEffect` rewriter
in `api_mappings.rs`.

---

## How FrayTools renders what we emit (render internals)

An independent specification of how **FrayTools 0.4.0** interprets the `.entity`
files we produce, derived by black-box observation + reading the minified
`app.asar` bundle. It exists so placement / rotation / timing bugs become
provable from documented behaviour instead of guesswork — the same
reverse-engineering-for-interoperability practice that produced the SWF/entity
notes above.

> **IP boundary.** Everything here is described in our own words from observed
> *behaviour*. No FrayTools source, strings, or assets are quoted or committed.
> The bundle is extracted locally (`npx @electron/asar extract`, never committed);
> domain property names survive minification because they are JSON keys, so the
> logic around them is followable. FrayTools is McLeodGaming proprietary software.

Confidence: **[observed]** = read from the bundle logic; **[inferred]** = deduced
from behaviour + our round-trip results (confirm with a probe before relying on it).

- **Stored space is Y-down; FrayTools negates Y at render time. [observed]**
  The per-keyframe screen-position builder computes the render position's Y as
  the negation of the stored keyframe `y` (and pivot Y). So the stored `.entity`
  `y` is Y-down — exactly what the converter emits (negative = above the foot).
  Don't double-apply the negation.

- **Transform order: place, then rotate the pivot offset, then translate. [observed]**
  `calculateAbsolutePivotPosition(position, pivotOffset, angleDeg)`: for a non-360
  angle it converts the pivot offset to polar, adds the angle, converts back, and
  adds to `position`. The caller passes rotation **negated**; combined with the Y
  negation the net on-screen convention is **clockwise-positive** — which is what
  the converter emits (no negation, normalised to `[0, 360)`). Scale is folded
  into the box dimensions / pivot before this step, not applied as a separate stage.

- **Rotation is honored only for rotation-capable box types. [observed — load-bearing]**
  FrayTools rotates `ItemBox` and custom collision boxes, but treats hurt / hit /
  grab / shield / reflect / absorb / ledge / grab-hold boxes as **axis-aligned** —
  a non-zero `rotation` on those is ignored. The converter therefore collapses
  rotation into the containing AABB (`w·|cosθ|+h·|sinθ|`, `w·|sinθ|+h·|cosθ|`,
  `rotation = 0`) for every non-rotation-capable type and keeps rotation only for
  `ItemBox` — unified in `sprite_parser::finalize_box_geometry` +
  `BoxType::supports_rotation()` (commits `7172bfcc`, `8ac39d49`, `d16ecfa9`).
  *Open:* SSF2's lone `customBox` (bandanadee) currently maps to `HURT_BOX` and is
  AABB-collapsed — correct for a hurtbox; promoting it to a rotatable FM custom box
  needs the FM custom-box type string (plugin-side, not in the local bundle).

- **COLLISION_BOX pivot is NOT multiplied by scale; other types ARE. [observed]**
  A COLLISION_BOX's `scaleX`/`scaleY` *are* its width/height in pixels, so its
  `pivotX`/`pivotY` are already in final pixel units; IMAGE pivots get multiplied
  by `scaleX`/`scaleY`. The converter emits COLLISION_BOX with `scaleX = width`,
  `scaleY = height`, `pivotX = width/2`, `pivotY = height/2` — consistent.

- **Keyframe `length` drives duration. [observed]** Timelines lay out purely by
  sequential keyframe `length`; a keyframe occupies `length` frames before the
  next. This is why 30→60 fps doubling works by doubling every keyframe `length`
  in lockstep (`entity_gen::double_keyframe_lengths`). LINEAR is the dominant
  tween; EASE_IN/EASE_OUT exist but are rare — LINEAR/none is the safe default.

- **The manifest is the registry; `.meta` sidecars bind id↔file. [observed]**
  `library/manifest.json :: content[]` entries reference flat string ids
  (`objectStatsId`, `scriptId`, `costumesId`, …); each asset's `.meta` sidecar
  declares its `guid` + `id`, and the id (not the filename) is what the manifest
  matches. So a script is found via its `.hx.meta` `id`, not its path. The
  `.fraytools` file holds **project settings only** (frame rate, palette shader
  mode, plugins, publish folders) — no content list. (`.fraytools` `version` 12,
  entity `version` 14 — independent schemas.)

- **Palette swap uses an RG-map shader. [observed]** A base palette + per-costume
  replacement map interpreted via a red/green-channel lookup; the converter emits
  `paletteShaderMode: "RG_MAP"` in the `.fraytools` and a `paletteMap` /
  `paletteCollection` pair per entity.

- **Layer/symbol vocabulary [observed]:** `IMAGE`, `COLLISION_BOX`,
  `COLLISION_BODY`, `POINT`, `LABEL`, `FRAME_SCRIPT` — exactly the set the
  converter emits.

**Still guesswork (confirm with a render diff before treating as load-bearing):**
runtime frame indexing 0- vs 1-based (the editor's "Frame: N" display is 1-based;
the harness uses 0-based `SET_FRAME` — see `TESTING.md`); whether the per-frame
COLLISION_BODY diamond expectation matches our AABB-of-hurtboxes approximation;
and the exact RG-map channel math.

---

## Coordinate System

Both SSF2 and Fraymakers use **y-down** screen coordinates (positive y = down).
No y-flip is needed between the two systems.

```
Origin: character foot (ground contact point)
Positive x: right
Positive y: down (into ground)
Negative y: up (into air)
```

World space = root MC transform applied to local SWF coordinates.

---

## Animation Name Mapping (SSF2 → Fraymakers)

The SSF2 → FM animation-name table is **data-driven** via
`mappings/character/animations.jsonc`. Keys:

- `ssf2_to_fm` — SSF2 xframe / animation name (e.g. `stand`, `a_air_forward`) → FM
  animation name (e.g. `idle`, `aerial_forward`).
- `label_to_ssf2` — sprite-symbol AnimLabel (lowercased, suffix stripped, e.g.
  `nair`) → SSF2 animation name.

Loaded by `crate::mappings::character_animations()` in `src/mappings.rs`. Used by
`extractor::build_ssf2_to_fm_anim`, `sprite_parser::static_ssf2_to_fm`, and the
sprite-symbol resolver `sprite_parser::extract_ssf2_anim_name`. Edit this file (not
the Rust source) to fix animation-name mapping.

Some SSF2 sprites pack multiple FM animations into one timeline separated by internal
FrameLabel tags (e.g. a "Jab" sprite contains jab1 / jab2 / jab3 / jab4; a "Strong"
sprite contains in/charge/attack). The splitter lives in `src/anim_splitter.rs` (and
`sprite_parser::sub_anim_splits` for the sprite-level equivalent). The label patterns
that drive which splits are **hardcoded match arms** in `anim_splitter.rs` — there is
no external rules file.

---

## UUID Generation

All GUIDs in the entity are **deterministic**, seeded by `{char_id}::{context}`.
Using UUID v5 (SHA-1 namespace). This ensures regeneration produces the same entity
byte-for-byte (in the GUID dimension). See `src/uuid_gen.rs`.

---

## Diagnostic Binaries

Several `--bin` targets exist for debugging:
```
dump_image_placement   — per-frame PlaceObject data for any sprite
dump_collision_box     — collision-box geometry for an animation
dump_images            — list all extracted bitmaps
dump_sprites           — list all DefineSprite symbols
dump_shape_bounds      — measure CollisonBox shape bounds
dump_shape_origins     — shape origin offsets
dump_pivots            — pivot points
dump_frame_labels      — frame labels inside a sprite timeline
dump_raw_frame         — raw tag dump for one frame
dump_inner_sprite      — inspect a nested (effect) sprite
dump_proj_states       — projectile state discovery
dump_stage             — root timeline inspection
dump_costumes          — costume tables from misc.ssf
dump_aerial_down_frames— targeted debug for the aerial-down animation
dump_trail_matrices    — trail-effect matrix inspection
check_shape_bitmap     — inspect a shape's bitmap fill + fill matrix
what_is_id             — identify what a numeric SWF character id refers to
```

These tools were the workbench used to figure the formats out. Costume extraction
itself is now in-process inside `ssf2_converter`; there is no longer a separate
`extract_costumes` binary.

Usage example:
```bash
./target/release/dump_image_placement ../ssf2-ssfs/mario.ssf "FAir_42"
./target/release/dump_collision_box ../ssf2-ssfs/mario.ssf "a_air_forward"
```

---

## Known Issues / Open Questions

### Vector-only effect sprites are silently skipped
Effects whose visuals are pure-vector shapes with solid-colour fills (e.g. some
charge sparkles, the Mario F-air twinkle) cannot be rasterized without a full SWF
vector renderer. Only bitmap-backed shapes are exported; vector effects appear
missing in the converted character.

### Shape-only menu portraits
A handful of characters (e.g. `donkeykong`, `fox`, `marth`) have `*_head` portraits
composed entirely of shapes rather than a bitmap. `image_extractor::discover_…`
prefers a Bitmap placement when available; when there isn't one, the head image is
missing and the menu entity ships with a placeholder. Needs a small SWF shape
rasterizer.

### Mario sprite placement not re-verified
After the recent rotation-convention / itemBox / shear-baking work, Mario in
particular hasn't been re-verified frame by frame in FrayTools. Most characters
look right; Mario was the canary that drove the rotation work and may need a
focused pass.

### One `.ssf` historically failed conversion outright
A single character file in the roster historically tripped a hard error
during conversion. Needs re-check against the current pipeline — path 2 +
the constructor walker have changed enough of the detection path that this
may already be resolved or may now surface differently.

### Frame-script / API translation is incomplete
`mappings/commands.jsonc` covers the bulk of SSF2 API calls, but many remain in
the `ssf2_only` list (intentionally surfaced as `// [SSF2-only: NAME] …` markers)
or end up in the `conversion_log.json` `unknown` list. Generated `.hx` always
needs human review.

### Projectile logic is largely stubbed
Projectile *entities* (visuals, boxes, animations, palettes) are generated.
Projectile *behaviour* (`ProjectileScript.hx`) is template scaffolding with
`// TODO: tune X_SPEED / Y_SPEED` placeholders. Multi-state projectiles use a
local state machine (`LState.IDLE` / `ACTIVE` / `HELD` / …) that registers via
`Common.initLocalStateMachine()` + `Common.registerLocalState(…)`, but the
state transitions are stubbed.

### Stat scaling is approximate
The `scale("gravity", v)` / `scale("speed", v)` calls in `haxe_gen.rs` go through
`mappings/character/stats.jsonc` multipliers that were hand-tuned by comparing
template characters to SSF2 data. Generated `CharacterStats.hx` deliberately
flags uncertain numbers with `/*TODO*/`.

---

## Source File Summary

| File | Purpose |
|---|---|
| `main.rs` | Entry point, orchestrates extraction pipeline |
| `extractor.rs` | Bridge between ABC parser and generator (`CharacterData`) |
| `abc_parser.rs` | AVM2/ABC bytecode parser + semantic extractors (~2500 LOC) |
| `decompiler.rs` | Bytecode → Haxe-ish source, full CFG reconstruction (~1700 LOC) |
| `sprite_parser.rs` | Per-frame collision-box geometry from SWF timelines |
| `image_extractor.rs` | PNG extraction, per-frame image placement, skew baking, projectile/effect/head discovery |
| `entity_gen.rs` | `.entity` JSON generator (~2000 LOC) |
| `haxe_gen.rs` | Output orchestrator (writes the whole character package) |
| `anim_splitter.rs` | Multi-label SSF2 sprite → multiple FM animations |
| `api_mappings.rs` | SSF2 → FM API translation pipeline + JSONC-driven rewriters |
| `mappings.rs` | JSONC loader / cache for the editable mapping files under `mappings/` |
| `sound_extractor.rs` | DefineSound → WAV via synthetic FLV + `ffmpeg` |
| `palette_gen.rs` | SSF2 costume / k-means palette generation |
| `swf_parser.rs` | Thin wrapper over the Ruffle `swf` crate |
| `ssf.rs` | SSF wrapper → raw SWF |
| `uuid_gen.rs` | Deterministic UUID v5 generation |
| `fraytools_project.rs` | `<name>.fraytools` project descriptor |
| `lib.rs` | `pub mod` declarations (so `src/bin/*` can `use ssf2_converter::*`) |
