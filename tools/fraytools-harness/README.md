# FrayTools render harness (Tier 3)

Drives the user's **local** FrayTools install over the Chrome DevTools
Protocol to load a converted entity onto the editor stage and capture
how FrayTools *actually* renders it. This is the ground-truth check that
the reconstructed transform math in
[`../../docs/fraytools_internals.md`](../../docs/fraytools_internals.md)
can't give on its own — e.g. whether a rotated itembox lands on the hand.

## IP boundary

This tool contains **no FrayTools source, assets, or strings**. It only
(a) launches the user's existing FrayTools binary and (b) speaks the
standard Chrome DevTools Protocol to it. FrayTools is McLeodGaming
proprietary software; its binary stays on the user's machine and is never
committed. `node_modules/` is git-ignored.

## Setup

```
cd tools/fraytools-harness
npm install        # pulls chrome-remote-interface
```

## Usage

```
node render-entity.js \
  --project   /abs/path/characters/mario/mario.fraytools \
  --entity    entities/Mario.entity \
  --out       /tmp/mario.png \
  [--fraytools "/Applications/FrayTools.app/Contents/MacOS/FrayTools"] \
  [--port 9222] [--settle 6000]
```

- `--entity` is a path **relative to the project's `library/` directory**.
- If a FrayTools is already listening on `--port` it attaches to that one
  (and leaves it running); otherwise it launches a fresh instance with
  `--remote-debugging-port`.
- Writes the stage `<canvas>` to `--out` as PNG; prints the path on stdout.

## How it navigates (no pixel coordinates)

FrayTools is an Electron/Chrome-80 app with `nodeIntegration`, so the
renderer exposes its full JS context over CDP. The harness navigates
entirely through FrayTools' own runtime objects — never screen clicks:

1. Attach to / launch FrayTools with remote debugging.
2. Walk the React 16 fiber tree (instances live on DOM nodes as
   `__reactInternalInstance$…`) and stash the app's component instances
   by class.
3. Identify the controller (has `getLibraryDirectory` + `openProject`)
   and the library tree (has an `onFileDoubleClicked` prop).
4. `controller.openProject(path)` if needed.
5. `controller.getLibraryDirectory().resolvePath(entityRelPath)` → a real
   FS-node with the methods FrayTools' open handler needs.
6. `tree.props.onFileDoubleClicked(node)` — the same callback a real
   double-click fires — opens the entity onto the stage.
7. Clip-capture the largest on-screen `<canvas>` (the stage).

## Notes / next steps

- Recovering `__webpack_require__` (via a probe chunk on `webpackJsonp`)
  also works and lets you call FrayTools' own transform functions
  directly — e.g. `calculateAbsolutePivotPosition`, which was confirmed
  byte-equivalent to our Rust port in `src/fraytools_transform.rs`.
- The next precision step is reading the rendered collision-box geometry
  straight from the stage scene graph (vendor `Container`/`DisplayObject`)
  instead of eyeballing the captured PNG — exact box coordinates as
  FrayTools computes them, for an automated render-diff.
- This build is observed against FrayTools 0.4.0. The fiber-key and
  component-shape assumptions may shift in other versions; the role
  detection (by method/prop presence) is written to be version-tolerant.
