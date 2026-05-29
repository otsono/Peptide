# FrayTools internals — observed invariants (Tier 1 RE reference)

> **What this is.** An independent specification of how FrayTools 0.4.0
> interprets the `.entity` files we emit, derived by black-box observation
> and by reading the minified `app.asar` bundle. It exists so converter
> bugs in placement / rotation / timing become provable from documented
> facts instead of guesswork. This is reverse engineering for
> **interoperability** — the same practice that produced
> `AGENT_CONTEXT.md`'s SWF + entity-format notes.
>
> **IP boundary (load-bearing).** Everything here is described in our own
> words. We cite *where* an invariant was observed (file + a minified
> token, when useful for a future re-check) but **quote no FrayTools
> source, strings, asset names, or class structure**. No FrayTools code,
> binary, or asset is in this repo. FrayTools is McLeodGaming proprietary
> software; the user runs their own install. If you extend this doc,
> describe *behaviour*, never transcribe implementation.
>
> **How observations were made.** `app.asar` extracted locally with
> `npx @electron/asar extract` (never committed). The app logic lives in
> a webpack-minified `main.bundle.js` (~883 KB, single-char variable
> names) and a vendored render/math bundle `vendors.bundle.js` (~5 MB).
> Domain property names and type tags survive minification because they
> are JSON keys, so the logic around them is followable. Citations below
> name the bundle + the surviving token to grep for.

---

## 0. Confidence legend

- **[observed]** — read directly from the bundle's logic around a
  surviving token; high confidence.
- **[inferred]** — deduced from observed behaviour + our converter's
  round-trip results; should be confirmed by a Tier 2 probe or Tier 3
  render diff before being treated as load-bearing.

---

## 1. Coordinate system

**Stored vs render Y is flipped. [observed]**
`main.bundle.js`, in the keyframe→screen-position logic (grep token:
`calculateAbsolutePivotPosition`, and the surrounding per-keyframe
interpolation that builds a position record): the render position's Y
component is computed as the **negation** of the stored keyframe `y`
(and likewise the pivot's Y is negated). The stored `.entity` `y` is
therefore in a Y-down space (matching SSF2 and what our converter emits
— negative = above the foot/origin), and FrayTools negates it when
composing the on-screen point.

**Units.** Stored `x` / `y` / pivot are in pixels (no twips; twips are an
SWF-only concept the converter already divides out). Scale fields
(`scaleX` / `scaleY`) are the box/image dimensions in pixels for
COLLISION_BOX (see §4), and multiplicative scale factors for IMAGE.

**Converter status:** our converter emits Y-down throughout
(`sprite_parser` keeps SWF's y-down; `entity_gen` does not vertical-flip).
This is consistent with the stored-space being Y-down. The render-time
negation is FrayTools-internal and we should not double-apply it. **[inferred]**
— worth a Tier 2 probe that reproduces the negation and checks a known
hitbox lands where the sprite shows it.

---

## 2. Transform application order

**Pivot offset is rotated about the keyframe position. [observed]**
`calculateAbsolutePivotPosition(position, pivotOffset, angleDeg)` behaves
as:

1. If `angleDeg` is an exact multiple of 360 → return
   `position + pivotOffset` (a plain vector add; the rotation fast-path).
2. Otherwise → treat `pivotOffset` as a vector, convert to polar
   (magnitude + `atan2(y, x)`), add `angleDeg` (converted to radians) to
   its polar angle, convert back to cartesian, and add to `position`.

So the composition is: **place at `position`, then rotate the pivot
offset vector around that point by the angle, then translate by the
rotated offset.** Scale is folded into the dimensions/pivot *before* this
step (see §4) rather than applied as a separate matrix stage.

**Angle sign.** The caller passes the rotation **negated** (grep: the
per-keyframe builder calls the pivot function with `-rotation`).
Combined with the §1 Y-negation, the net on-screen convention is
**clockwise-positive in screen space**, which is what
`AGENT_CONTEXT.md` already documents for both IMAGE and COLLISION_BOX
and what `entity_gen` emits (no negation; normalised to `[0, 360)`).

**Converter status:** consistent. The `40fad65d` / `f472a2dd` history
that aligned COLLISION_BOX rotation with the IMAGE convention matches
the observed CW-positive net behaviour.

---

## 3. Rotation conventions per entity type

**Rotation is in degrees. [observed]** The pivot math multiplies by
`π/180`, so the stored `rotation` field is degrees, not radians.

**IMAGE: rotation honored. [observed]** IMAGE keyframes carry and apply
rotation through the same pivot-position pipeline.

**COLLISION_BOX: rotation honored ONLY for rotation-capable box types. [observed + corrected]**
This is the load-bearing correction for this codebase. Fraymakers rotates
`ItemBox` and custom collision boxes, but treats hurt / hit / grab /
shield / reflect / absorb / ledge / grab-hold boxes as **axis-aligned** —
a non-zero `rotation` on those is not honored as a true oriented
rectangle. The collision-box layer types and their handling are defined
in the FraymakersMetadata plugin (NOT in `main.bundle.js` — the
box-type enum strings are absent from that bundle, consistent with the
type registry living plugin-side).

**Converter status — RESOLVED.** The converter previously emitted the
decomposed rotation on *every* collision box. Fixed across:
- `7172bfcc` — decompose the SWF matrix correctly (recover `w`, `h`, `θ`
  via `√(a²+b²)`, `√(c²+d²)`, `atan2(b,a)`), eliminating the earlier
  0×0-degenerate-box bug for ~90° boxes whose scale sat in the
  off-diagonal `b`/`c` terms.
- `8ac39d49` — collapse rotation into the containing AABB
  (`w·|cosθ| + h·|sinθ|`, `w·|sinθ| + h·|cosθ|`) with `rotation = 0` for
  every non-rotation-capable box type; keep rotation for `ItemBox`.
- `d16ecfa9` — unify the decision in one tested helper
  (`sprite_parser::finalize_box_geometry`) + `BoxType::supports_rotation()`.

**Open item — `CustomBox`.** SSF2 places a `customBox` instance exactly
once in the corpus (bandanadee, `b` neutral-special sprite). It currently
maps to `BoxType::Hurtbox` (the `*box` fallback) and is emitted as FM
`HURT_BOX`, so AABB-collapsing it is correct for what we emit. Promoting
it to a *rotatable* FM custom box requires the FM custom-box
`collisionBoxType` string + layer convention (the `.fraytools` preset
exposes `customboxa` / `customboxb` / `customboxc` colour slots, but the
runtime type tag is plugin-side and not in the local bundle). Deferred:
add a `BoxType::CustomBox` only once that type string is confirmed, to
avoid emitting a rotated `HURT_BOX` (the very bug above).

**Projectiles / effects.** Rendered through the same IMAGE/COLLISION
machinery as characters (they are entities with the same layer/symbol
vocabulary). Rotation conventions are identical; no separate pipeline
observed. **[inferred]**

---

## 4. Pivot handling per entity type

**COLLISION_BOX pivot is NOT multiplied by scale; other types ARE. [observed]**
In the per-keyframe interpolation builder, the pivot used for the
position math is computed with an explicit type check: for COLLISION_BOX
the stored `pivotX` / `pivotY` are used directly; for other types
(IMAGE etc.) the pivot is multiplied by `scaleX` / `scaleY`. This is
because a COLLISION_BOX's `scaleX` / `scaleY` **are** its width/height in
pixels (the box geometry is the scale), so its pivot is already in final
pixel units; an IMAGE's `scaleX` / `scaleY` are multiplicative factors on
a base sprite, so its pivot must be scaled to match.

**Converter status:** consistent. `entity_gen` emits COLLISION_BOX with
`scaleX = width`, `scaleY = height`, and `pivotX = width/2`,
`pivotY = height/2` (centre pivot, already in pixel units) — which is
exactly the "pivot not scaled" expectation.

---

## 5. Animation timing / frame indexing

**Keyframe `length` drives duration. [observed]** Timeline layout is by
sequential keyframe `length`; a keyframe occupies `length` frames before
the next begins. This is why the converter's 30→60fps doubling works by
doubling every keyframe `length` in lockstep (`entity_gen::double_keyframe_lengths`)
— image, collision, frame-script and label layers all stretch together.

**Tween types: LINEAR dominant; EASE_IN / EASE_OUT exist. [observed]**
Interpolation between two keyframes is `interpolate(from, to, alpha,
easing)` with an easing transform applied to `alpha` (grep: the easing
helper near `toEaseValue`). LINEAR is by far the most common tween tag in
the bundle; EASE_IN / EASE_OUT appear once each. Our converter emits
`tweenType` / `tweened` on keyframes; LINEAR/none is the safe default and
what we use.

**Frame indexing — NOT definitively confirmed. [inferred]** The editor
timeline uses a `frameSelections` concept, but I did not extract
unambiguous evidence of 0- vs 1-indexed *runtime* frame numbering. Treat
any off-by-one in frame-script timing as unverified until a Tier 2/3
probe pins it. Flagging this explicitly so it isn't assumed.

---

## 6. Asset references (manifest → entity resolution)

**The manifest is the registry; `.meta` sidecars bind id↔file. [observed, manifest-side]**
`library/manifest.json` carries a `content[]` array; each entry has a
`type` (`character`, `characterAi`, `projectile`, …) and flat string ids
(`objectStatsId`, `animationStatsId`, `hitboxStatsId`, `scriptId`,
`costumesId`, `aiId`). Those ids resolve against the project-global asset
pool. Each asset file has a `.meta` sidecar carrying its `guid` + `id`;
the id in the sidecar is what the manifest entry references — **not** the
filename. So a script at `library/scripts/Mario/Script.hx` is found via
its `.hx.meta` declaring `id: "marioScript"`, matched to the manifest's
`scriptId: "marioScript"`.

**Case sensitivity. [inferred]** Asset *ids* are matched exactly as
strings (case-sensitive). File *paths* are case-insensitive on macOS
(where FrayTools runs) but would be case-sensitive on Linux — the
converter keeps a consistent lowercase-id / PascalCase-path scheme so
this never bites in practice. Confirm before relying on cross-platform
path casing.

**Project file vs manifest.** The `.fraytools` file holds editor/project
settings only (frame rate, palette shader mode, plugins, publish
folders) — it carries **no** content list. The manifest is the sole
source of truth for what ships. (`.fraytools` `version` observed as 12;
the entity `version` we emit is 14 — these are independent schema
versions.)

---

## 7. Resource naming

Derived from the manifest+entity-id model in §6 plus our own layout
(`docs/multi_character_projects_plan.md`):

- **`<Pascal>.entity`** — one character entity per character; the
  in-file `id` is the lowercase character id (`mario`), the filename is
  PascalCase (`Mario.entity`). FrayTools resolves the entity by the
  manifest content entry's `id`, so the filename is a human convenience;
  the id binding is what matters.
- **`<Pascal>_Menu.entity`** (multi-char) / **`Menu.entity`** (single) —
  the portrait/HUD entity referenced from the character content entry's
  `metadata.ui.entityId` (`mario` → `menu`; multi-char → `zelda_menu`).
- **Projectile entities** — referenced by their own `content[]` entries
  with `type: "projectile"` and per-projectile stat/script ids.
- **`library/scripts/<Pascal>/`** — per-character script directory. The
  directory name is cosmetic; the `.hx.meta` `id` is the binding.

**Converter status:** the PascalCase rename (`388e6faf`) + multi-char
project layout (`4db74a89`) + per-character audio subdirs (`7729ecda`)
all conform to this id-binds-not-filename model.

---

## 8. Other observations

- **Layer / symbol vocabulary [observed].** The render layer + symbol
  types present in the bundle: `IMAGE`, `COLLISION_BOX`, `COLLISION_BODY`,
  `POINT`, `LABEL`, `FRAME_SCRIPT`. These match exactly the layer set the
  converter emits. `POINT` is used for the grab-hold point (our
  `BoxType::GrabHoldBox`), consistent with `AGENT_CONTEXT.md`.
- **Palette swap shader [observed].** The palette system uses an
  **RG-map** shader mode (grep: `RG_MAP`, 6 occurrences; `paletteMap`,
  3). A base palette + a per-costume replacement map are interpreted via
  a red/green-channel lookup. The converter emits `paletteShaderMode:
  "RG_MAP"` in the `.fraytools` and a `paletteMap` / `paletteCollection`
  pair on each entity — consistent. The detailed channel-mapping math is
  a candidate for a Tier 2 probe if costume colours ever look wrong.
- **COLLISION_BODY (ECB)** is a distinct layer type (the character's
  environmental-collision diamond), handled separately from
  COLLISION_BOX. The converter computes it per-frame as the AABB of the
  hurtboxes (`entity_gen`), which is an approximation of how FM expects
  the body diamond — flagged as **[inferred]**; a Tier 2/3 check could
  confirm the diamond-vs-AABB shape expectation.

---

## 9. What's still guesswork (Tier 2 / Tier 3 targets)

Ranked by how likely a wrong assumption is to cause a visible bug:

1. **Frame indexing / off-by-one** (§5) — unverified 0- vs 1-index.
2. **Y-negation round-trip** (§1) — confirm a known hitbox lands on the
   sprite once the negation + pivot rotation are reproduced.
3. **COLLISION_BODY diamond shape** (§8) — AABB-of-hurtboxes vs the true
   FM body-diamond expectation.
4. **Palette RG-map channel math** (§8) — exact red/green lookup.
5. **CustomBox FM type string** (§3) — needed before SSF2 `customBox`
   can become a rotatable FM custom box.

Tier 2 will port the §2 + §4 transform/pivot math from this spec into a
Rust probe and assert our emitted entities place boxes where the SSF2
source says they should. Tier 3 will drive the user's local FrayTools to
render an entity and diff the pixels — the only way to settle the
[inferred] items above with certainty.
