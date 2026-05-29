# FrayTools harness (Tier 3)

Drives the user's **local** FrayTools install over the Chrome DevTools
Protocol to load a converted entity onto the editor stage, navigate to a
specific animation and frame, extract box geometry, and capture how FrayTools
*actually* renders it. This is the ground-truth check that math alone can't
give — e.g. whether a baked itembox pivot lands on the hand.

## IP boundary

This tool contains **no FrayTools source, assets, or strings**. It only
(a) launches the user's existing FrayTools binary and (b) speaks the standard
Chrome DevTools Protocol to it. FrayTools is McLeodGaming proprietary
software; its binary stays on the user's machine and is never committed.
`node_modules/` is git-ignored.

## Setup

```
cd tools/fraytools-harness
npm install        # pulls chrome-remote-interface
```

## Scripts

### `harness.js` — comprehensive harness (main tool)

Opens a project + entity, navigates to a specific animation + frame,
extracts box geometry from the entity JSON, captures the stage as a PNG,
and writes a JSON report. All steps except the PNG capture work without
FrayTools running (data comes from the entity file on disk).

```
node harness.js \
  --project   /abs/path/character.fraytools \
  --entity    entities/Character.entity \
  [--animation aerial_back] \
  [--frame    4] \
  [--out-json /tmp/out.json] \
  [--out-png  /tmp/out.png] \
  [--fraytools "/Applications/FrayTools.app/Contents/MacOS/FrayTools"] \
  [--port 9222] [--settle 5000]
```

- `--entity` is a path **relative to the project's `library/` directory**.
- `--animation` defaults to the entity's first animation.
- `--frame` is 0-based; defaults to 0.
- The report JSON is always printed to **stdout** (for piping); `--out-json`
  also writes a formatted copy to disk.
- If FrayTools is already listening on `--port` the harness attaches to it
  (and leaves it running); otherwise it launches a fresh instance.

**Output JSON schema:**
```json
{
  "entity_path":   "entities/Character.entity",
  "animation":     "aerial_back",
  "frame":         4,
  "total_frames":  18,
  "animations":    ["aerial_back", "aerial_down", ...],
  "nav": {
    "animation": "A:leaf-text | not found | default (first animation)",
    "frame":     "B:setState(currentFrame) | not found | default (frame 0)"
  },
  "boxes": [
    {
      "layer_name":      "itembox0",
      "layer_type":      "COLLISION_BOX",
      "fm_box_type":     "ITEM_BOX",
      "x":               59.84,  "y": 11.61,
      "width":           3.68,   "height": 21.8,
      "rotation":        72.85,
      "pivot_x":         1.84,   "pivot_y": 21.8,
      "color":           16776960,
      "alpha":           0.5,
      "rendered_anchor": { "x": 39.55, "y": 19.8 }
    }
  ],
  "png": "/tmp/out.png"
}
```

`rendered_anchor` is the position FrayTools places the box's pivot point —
computed via the same math as `src/fraytools_transform.rs`.

### `render-entity.js` — simple stage capture (legacy)

Opens a project + entity and captures the stage canvas as a PNG.
No animation/frame navigation or JSON report. Use `harness.js` instead
for new work.

```
node render-entity.js \
  --project   /abs/path/character.fraytools \
  --entity    entities/Character.entity \
  --out       /tmp/stage.png
```

## How navigation works (no pixel coordinates)

FrayTools is an Electron/Chrome-80 app with `nodeIntegration`, so the
renderer exposes its full JS context over CDP. The harness navigates
entirely through FrayTools' own runtime objects — never screen clicks or
pixel coordinates:

1. Walk the React 16 fiber tree; stash component instances by class.
2. Identify the **controller** (`getLibraryDirectory` + `openProject`) and
   the **library tree** (`onFileDoubleClicked` prop).
3. `controller.openProject(path)` if needed.
4. `controller.getLibraryDirectory().resolvePath(entityRelPath)` → a real
   FS-node with the methods FrayTools' open handler needs.
5. `tree.props.onFileDoubleClicked(node)` — the same callback a real
   double-click fires — opens the entity on the stage.
6. For **animation selection**: walk fiber DOM-element nodes and call the
   matching list item's `memoizedProps.onClick` directly (tries text-match,
   title-match, filter-input, then component method — returns which strategy
   worked).
7. For **frame seeking**: walk fiber class-component nodes; call a
   `setCurrentFrame`/`setFrame` method if present, or call `setState` on
   a component with frame state.
8. Clip-capture the largest on-screen `<canvas>` via `Page.captureScreenshot`.
9. Read box geometry from the entity JSON file on disk (always accurate,
   independent of UI state).

## AI-iterable test loop

The harness is designed so an AI can iterate on the converter without
human involvement:

```
# 1. Edit src/entity_gen.rs
# 2. Rebuild
cargo build

# 3. Convert character
cargo run -- --ssf2 mario.ssf --out /tmp/mario_ft/

# 4. Open entity + extract frame data
node tools/fraytools-harness/harness.js \
  --project /tmp/mario_ft/mario.fraytools \
  --entity  entities/Character.entity \
  --animation aerial_back --frame 4 \
  --out-json /tmp/mario_f4.json \
  --out-png  /tmp/mario_f4.png

# 5. Compare against SSF2 source → numeric verdict
cargo run --bin compare_boxes -- \
  --ssf2 /path/to/ssf2-ssfs/mario.ssf \
  --char mario \
  --json /tmp/mario_f4.json

# If exit 0: PASS — go to next animation/frame.
# If exit 1: FAIL — read the drift table, go to step 1.
```

No screenshots, no eyeballing, no human action needed.

## Rust companion: `compare_boxes`

```
cargo run --bin compare_boxes -- \
  --ssf2      /path/to/char.ssf \
  --char      <char_name> \
  --json      /tmp/harness_out.json \
  [--tolerance 1.5]
```

Reads the harness JSON, parses the SSF2 source, matches boxes by type
and size, and reports per-box drift (FrayTools rendered anchor vs. SSF2
intended position). Exits 0 on full pass, 1 on any failure.

**Verified end-to-end:** captain falcon `aerial_back` frame 4 —
- Raw (pre-bake) entity: itembox drifts **25.98 px** → FAIL
- Baked entity (current converter): itembox drifts **0.00 px** → PASS

## Notes

- Recovering `__webpack_require__` (via a probe chunk on `webpackJsonp`)
  also works and lets you call FrayTools' own transform functions directly —
  `calculateAbsolutePivotPosition` was confirmed byte-equivalent to our
  Rust port in `src/fraytools_transform.rs`.
- Navigation strategies are tried in order and the `nav` field in the JSON
  report tells you which one succeeded — useful for debugging future
  FrayTools versions.
- This build is tested against FrayTools 0.4.0. The fiber-key and
  component-shape assumptions are written to be version-tolerant (role
  detection by method/prop presence, not class name).
