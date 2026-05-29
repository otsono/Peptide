# FrayTools reverse-engineering — feasibility scope

> **Status: feasibility investigation only.** No harness built yet. This
> doc answers "should we do it, and at what level" so the user can pick a
> tier. Findings are from inspecting the installed FrayTools 0.4.0
> `app.asar` on this machine.

## TL;DR recommendation

**Do Tier 1 (reference documentation) now — it's high-yield and
low-risk.** The minified bundle is readable enough around domain
strings that two `grep` lines already surfaced three concrete render
invariants the converter has only been guessing at. Formalising those
into `docs/fraytools_internals.md` makes whole classes of converter
bugs provable by text inspection (the rotated-hurtbox bug we just fixed
would have been caught instantly against a documented "scaleX = matrix
column magnitude" invariant).

**Do Tier 2 (targeted Rust probes) opportunistically** for the
highest-risk subsystem (collision-box transforms) — the exact transform
function is locatable by name and portable.

**Defer Tier 3 (full headless-render harness)** unless a visual bug
class emerges that text inspection can't resolve. Electron 8.3.4 +
DOM-coupled renderer make CI integration fragile and high-maintenance.

## 1. What's on disk

| item | value |
|------|-------|
| App | `/Applications/FrayTools.app` |
| Version | 0.4.0 (matches the `version: "0.4.0"` we emit in `pluginMetadata`) |
| Author | McLeodGaming Inc. |
| Electron | 8.3.4 (old — 2020-era) |
| Bundle | `Contents/Resources/app.asar`, 14.3 MB |
| Also present | `~/Downloads/FrayTools-0.4.0-osx.zip`, `~/Downloads/FrayTools-darwin-x64/`, `~/FrayToolsData/`, `~/Library/Application Support/FrayTools/` |

Extraction is trivial: `npx --yes @electron/asar extract <app.asar> <dir>`
(no global install needed; `npx` + `node` are on this machine).

## 2. Bundle contents (extracted)

| file | size | what it is |
|------|------|------------|
| `main.bundle.js`    | 883 KB  | The app logic — entity model, timeline interpolation, render setup. **The file we care about.** |
| `vendors.bundle.js` | 5.0 MB  | Third-party deps incl. the rendering math (Matrix/Transform/Container/DisplayObject). |
| `ts.worker.js`      | 4.3 MB  | Monaco's TypeScript language service (in-app script editing) — irrelevant to rendering. |
| `json.worker.js` / `editor.worker.js` | — | Monaco editor workers — irrelevant. |
| `index.js`          | 9 KB    | Electron main-process bootstrap. |
| `node_modules/`, `fonts/`, `images/` | — | assets. |

## 3. Minification level — readable enough?

`main.bundle.js` is a **webpack production bundle**: single line, variable
names mangled to single chars (`function t(t)`, `class as`). BUT —
critically — **domain string literals survive**, because they're property
names and type tags accessed as strings, which webpack/terser cannot
mangle without breaking JSON I/O:

| string | occurrences in `main.bundle.js` |
|--------|-------------------------------|
| `keyframes`      | 201 |
| `rotation`       | 198 |
| `COLLISION_BODY` | 103 |
| `scaleX`         | 71  |
| `COLLISION_BOX`  | 68  |
| `pivotX`         | 39  |
| `FRAME_SCRIPT`   | 27  |
| `PaletteSwapShader` | 1 |

Absent (stored differently — worth a deeper dig if needed): `HIT_BOX`,
`HURT_BOX`, `GRAB_HOLD_POINT`, `landType`, `objectStatsId` returned 0.
The box *subtype* enum is likely held in `pluginMetadata` under a
different key, or as a numeric enum — a known unknown, not a blocker.

Because the logic *around* those strings reads property accesses by
name, the minified function bodies are **followable**. Example: one grep
for `pivotX` returned this (reformatted):

```js
s = { x: t.x, y: -t.y };                       // (A) Y is negated
c = {
  x: t.type === "COLLISION_BOX"
       ? interpolate(t.pivotX,  n.pivotX,  a, ease)      // (B) box pivot NOT scaled
       : interpolate(t.pivotX*i, n.pivotX*i, a, ease),   //     other types scale pivot
  y: ... (negated, same branch) ...
};
// second site:
c.x = t.type === "COLLISION_BOX" ? t.pivotX          : t.pivotX * t.scaleX;
c.y = t.type === "COLLISION_BOX" ? -t.pivotY         : -t.pivotY * t.scaleY;
p = F.calculateAbsolutePivotPosition(s, c, -t.rotation);   // (C) rotation negated here
```

Three concrete, documentable invariants fell out of **two grep lines**:

- **(A) Y-axis negation.** FrayTools' internal render space negates the
  stored entity `y` (and `pivotY`). Our converter assumes y-down
  throughout (AGENT_CONTEXT: "Both SSF2 and Fraymakers use y-down… no
  vertical flip needed") — this is worth reconciling against (A) to
  confirm we're not off by a sign somewhere subtle.
- **(B) COLLISION_BOX pivot is NOT multiplied by scale**, whereas IMAGE
  and other types scale the pivot by `scaleX`/`scaleY`. Our
  `entity_gen` emits collision-box `pivotX = width/2` (already in
  "scaled" units), which is consistent with this — but now we can
  *prove* it rather than infer it.
- **(C) rotation is negated** when computing the absolute pivot
  position. This bears directly on the rotation-convention comments in
  `entity_gen` / `AGENT_CONTEXT` ("CW-positive, no negation") — the
  no-negation claim is about the stored value; FrayTools negates
  internally for pivot math. Documenting the exact composition removes
  the guesswork that produced the itemBox sign-fix churn.

## 4. The renderer

`vendors.bundle.js` carries the scene-graph math: `Transform` (552),
`Matrix` (330), `Container` (210), `DisplayObject` (24),
`updateTransform` (8). It is **not** PIXI / Phaser / CreateJS / Konva /
Fabric by name (all returned 0) — likely a custom or relabeled
PIXI-like scene graph. Doesn't matter for our purposes: the transform
composition we need lives in `main.bundle.js`'s
`calculateAbsolutePivotPosition` + the `interpolate` timeline functions,
which are readable.

## 5. The three tiers

### Tier 1 — Reference documentation (`docs/fraytools_internals.md`)

**Effort: low. Leverage: high. Recommend doing now.**

Read the readable-around-strings portions of `main.bundle.js` and write
up the invariants we've been guessing at:

- Coordinate system + the Y-negation (A).
- Keyframe interpolation model (the `interpolate(from, to, alpha, ease)`
  + `toEaseValue` easing — confirms how tween frames between keyframes
  behave, which affects our `tweened`/`tweenType` emission).
- Transform application order + `calculateAbsolutePivotPosition(pos,
  pivot, -rotation)` (C).
- Per-type pivot scaling rule (B): COLLISION_BOX vs IMAGE.
- The box-subtype enum encoding (the `HIT_BOX`/`HURT_BOX`-absent
  mystery) — resolve where the box type actually lives.
- Palette shader (`PaletteSwapShader`) — we already decompiled bits of
  this; formalise it.

We've **already done this opportunistically** (landType, PaletteSwapShader
shape earlier). Tier 1 is just systematising the same technique. The
payoff: converter bugs in collision/image placement become provable
from the doc + the emitted JSON, no FrayTools round-trip needed.

### Tier 2 — Targeted Rust probes

**Effort: medium. Leverage: medium-high for the riskiest subsystem.**

Port the specific transform path — `calculateAbsolutePivotPosition` +
the keyframe interpolation — to a small Rust probe (`src/bin/`). Feed it
a converted entity, compute where FrayTools *would* place a given
hitbox/image, and compare against an expected world position (or against
our own `sprite_parser` world coords). This validates the
matrix-decomposition assumptions (exactly the class of bug we just
fixed) without launching FrayTools.

Bounded scope: the relevant functions are a handful, locatable by the
domain strings. Risk: the minified math takes care to transcribe
correctly; a unit test per ported function (mirroring the
`matrix_to_box_tests` we just added) keeps it honest.

### Tier 3 — Full headless-render integration harness

**Effort: high. Leverage: high confidence, high maintenance. Defer.**

Load a converted entity into FrayTools' actual renderer (headless
Electron, or the extracted renderer wired to a headless canvas), capture
the rendered frame as image data, golden-diff against a reference image.
This is the only tier that catches "looks wrong in the editor" bugs that
text inspection misses.

Blockers:
- Electron 8.3.4 is old; headless rendering in CI is fragile and version-
  pinned.
- The renderer is DOM/Electron-coupled; isolating it from the editor
  shell is non-trivial.
- Golden *images* are machine-sensitive (GPU/font/AA drift) — the same
  reason our current `golden_sandbag` excludes PNGs.

Recommend only if Tier 1+2 leave a visual-correctness gap that keeps
biting.

## 6. Suggested path

1. **Now:** Tier 1 — write `docs/fraytools_internals.md` capturing the
   coordinate system, transform order, pivot rules, interpolation, and
   the box-type enum. Highest leverage per hour.
2. **Opportunistically:** Tier 2 — a `verify_collision_transform` probe
   when next touching `sprite_parser` / `entity_gen` collision code.
3. **Only if needed:** Tier 3.

## 7. Legal / hygiene note

FrayTools is McLeodGaming proprietary software. Reverse-engineering for
**interoperability** (making our converter emit correct files for a tool
the user owns and runs locally) is the standard, defensible use case.
We should:
- Keep extracted bundle copies OUT of the repo (`.gitignore` already
  excludes stray binaries; never commit `app.asar` or its extraction).
- Document *observed behaviour and invariants* in our own words in
  `docs/fraytools_internals.md`, not paste decompiled FrayTools source
  verbatim.
- Treat `docs/fraytools_internals.md` as our independent specification
  derived from black-box + minified-bundle observation, the same way
  `AGENT_CONTEXT.md` already documents the SWF and entity formats.
