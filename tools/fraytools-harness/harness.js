#!/usr/bin/env node
//
// Comprehensive Tier-3 FrayTools harness.
//
// Drives the user's LOCAL FrayTools install (Electron, Chrome DevTools
// Protocol) to:
//   1. Open a FrayTools project.
//   2. Open a specific entity onto the stage.
//   3. Navigate to a named animation (best-effort via React runtime).
//   4. Seek to a specific frame (best-effort via React runtime).
//   5. Extract box geometry directly from the entity JSON file (robust —
//      filesystem read, no UI dependency).
//   6. Capture the stage canvas as a PNG (compositor truth).
//   7. Write a JSON report with per-box data + rendered-anchor coordinates,
//      suitable for `compare_boxes` (see src/bin/compare_boxes.rs).
//
// ── IP boundary (load-bearing) ───────────────────────────────────────────────
// This file contains NO FrayTools source, assets, or strings. It only
//   (a) launches the user's existing FrayTools binary, and
//   (b) speaks the standard Chrome DevTools Protocol to it.
// FrayTools is McLeodGaming proprietary software; the binary stays on the
// user's machine and is never committed. Everything here is our own code.
//
// ── How navigation works (no pixel coordinates) ──────────────────────────────
// 1. Walk the React 16 fiber tree to stash component instances.
// 2. controller.openProject(path) / controller.getLibraryDirectory()
//    .resolvePath(entityRel) / tree.props.onFileDoubleClicked(node) — same
//    approach as render-entity.js, verified working.
// 3. Animation selection: walk all fiber DOM-element nodes; find the list
//    item whose text matches the animation name; call its memoizedProps.onClick
//    directly (React synthetic event, no coordinates).
// 4. Frame seeking: walk all fiber class-component nodes; find the one whose
//    state/props contain a frame counter; call setState or a named method.
//    Falls back to a filter-input approach if the list walk fails.
// 5. Entity data (box geometry) is read from the filesystem via
//    node.getPath() and parsed as JSON — completely independent of UI state,
//    always accurate.
//
// ── Output JSON schema ────────────────────────────────────────────────────────
// {
//   "entity_path": "entities/Character.entity",  // relative to library/
//   "animation":   "aerial_back",
//   "frame":       4,                             // 0-based
//   "total_frames": 18,
//   "animations":  ["aerial_back", ...],          // all animation names
//   "nav": {
//     "animation": "ok" | "not found" | "<error>",
//     "frame":     "ok" | "not found" | "skipped"
//   },
//   "boxes": [
//     {
//       "layer_name":  "hitbox0",
//       "layer_type":  "COLLISION_BOX",
//       "fm_box_type": "HIT_BOX",            // derived from color
//       "x":           -1.1,   "y":  31.82,  // stored top-left
//       "width":       33.69,  "height": 22.95,
//       "rotation":    0.0,                   // degrees, CW-positive
//       "pivot_x":     16.845, "pivot_y": 11.475,
//       "color":       16711680,
//       "alpha":       0.5,
//       "rendered_anchor": { "x": 15.745, "y": 43.295 }  // FrayTools anchor
//     }
//   ],
//   "png": "/tmp/out.png"    // omitted if --out-png not given
// }
//
// ── Usage ─────────────────────────────────────────────────────────────────────
//   node harness.js \
//     --project   /abs/path/character.fraytools \
//     --entity    entities/Character.entity \
//     [--animation aerial_back] \
//     [--frame    4] \
//     [--out-json /tmp/out.json] \
//     [--out-png  /tmp/out.png] \
//     [--fraytools "/Applications/FrayTools.app/Contents/MacOS/FrayTools"] \
//     [--port 9222] [--settle 5000]
//
// --entity is relative to the project's library/ directory.
// If FrayTools is already on --port the harness attaches to it (and leaves
// it running); otherwise it launches a new instance.
// --animation and --frame default to the entity's first animation, frame 0.

'use strict';
const CDP     = require('chrome-remote-interface');
const { spawn } = require('child_process');
const http    = require('http');
const fs      = require('fs');
const path    = require('path');

// ── Utilities ────────────────────────────────────────────────────────────────

const sleep = ms => new Promise(r => setTimeout(r, ms));

function arg(name, def) {
  const i = process.argv.indexOf('--' + name);
  return i >= 0 && i + 1 < process.argv.length ? process.argv[i + 1] : def;
}

function die(msg) { console.error('ERROR:', msg); process.exit(1); }

function cdpUp(port) {
  return new Promise(resolve => {
    const req = http.get({ host: '127.0.0.1', port, path: '/json/version', timeout: 800 },
      res => { res.resume(); resolve(res.statusCode === 200); });
    req.on('error', () => resolve(false));
    req.on('timeout', () => { req.destroy(); resolve(false); });
  });
}

async function waitForCdp(port, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (await cdpUp(port)) return true;
    await sleep(500);
  }
  return false;
}

// ── FrayTools transform math (mirrors fraytools_transform.rs) ─────────────────
// Used to compute `rendered_anchor` for each box — the on-screen position
// of the box's pivot point after FrayTools applies its rotation.

function _polar(mag, angleRad) {
  return [mag * Math.cos(angleRad), mag * Math.sin(angleRad)];
}

function _calculateAbsolutePivotPosition(pos, pivot, angleDeg) {
  if (angleDeg % 360 === 0) return [pos[0] + pivot[0], pos[1] + pivot[1]];
  const mag = Math.sqrt(pivot[0] ** 2 + pivot[1] ** 2);
  const ang = Math.atan2(pivot[1], pivot[0]);
  const [rx, ry] = _polar(mag, ang + angleDeg * Math.PI / 180);
  return [pos[0] + rx, pos[1] + ry];
}

/** FrayTools rendered anchor for a COLLISION_BOX (stored → screen → stored). */
function collisionBoxAnchor(x, y, pivotX, pivotY, rotationDeg) {
  const s = [x, -y];        // §1 negate Y into render space
  const c = [pivotX, -pivotY];
  const p = _calculateAbsolutePivotPosition(s, c, -rotationDeg);  // §2 rotate
  return { x: p[0], y: -p[1] };  // §3 negate Y back to stored space
}

// ── Box color → FM type string ────────────────────────────────────────────────

const COLOR_TO_FM_TYPE = {
  0xff0000: 'HIT_BOX',
  0xfcba03: 'HURT_BOX',
  0xff00ff: 'GRAB_BOX',
  0xffff00: 'ITEM_BOX',    // yellow — itembox (stored as hurtbox in FM layer type)
  0x48f748: 'REFLECT_BOX',
  0x42ecff: 'COUNTER_BOX',
  0xbababa: 'LEDGE_GRAB_BOX',
  0x9999ff: 'GRAB_HOLD_POINT',
};

function fmBoxType(colorInt) {
  return COLOR_TO_FM_TYPE[colorInt] || `UNKNOWN(0x${colorInt.toString(16)})`;
}

// ── Entity JSON data extraction ───────────────────────────────────────────────
// Pure filesystem read — no UI dependency.

function readEntityData(entityAbsPath) {
  const raw = fs.readFileSync(entityAbsPath, 'utf8');
  const data = JSON.parse(raw);

  const symById   = Object.fromEntries(data.symbols.map(s => [s['$id'], s]));
  const layerById = Object.fromEntries(data.layers.map(l => [l['$id'], l]));
  const kfById    = Object.fromEntries(data.keyframes.map(k => [k['$id'], k]));

  return { data, symById, layerById, kfById };
}

/** Returns all animation names in the entity, in order. */
function listAnimations(entityCtx) {
  return entityCtx.data.animations.map(a => a.name);
}

/** Total frames for an animation (max keyframe span across COLLISION_BOX layers). */
function totalFrames(anim, entityCtx) {
  const { layerById, kfById } = entityCtx;
  let max = 0;
  for (const lid of anim.layers) {
    const layer = layerById[lid];
    if (!layer || !['COLLISION_BOX', 'COLLISION_BODY', 'IMAGE'].includes(layer.type)) continue;
    const span = (layer.keyframes || []).reduce((acc, kfid) => acc + (kfById[kfid]?.length || 0), 0);
    if (span > max) max = span;
  }
  return max;
}

/**
 * Extract all active COLLISION_BOX (and optionally COLLISION_BODY) entries
 * for a given animation name + 0-based frame index.
 * Returns an array of box descriptors with computed rendered_anchor.
 */
function boxesAtFrame(animName, frameN, entityCtx) {
  const { data, symById, layerById, kfById } = entityCtx;
  const anim = data.animations.find(a => a.name === animName);
  if (!anim) return null;

  const boxes = [];
  for (const lid of anim.layers) {
    const layer = layerById[lid];
    if (!layer) continue;
    if (layer.type !== 'COLLISION_BOX' && layer.type !== 'COLLISION_BODY') continue;

    // Walk keyframes to find the one containing frameN.
    let frameAccum = 0;
    for (const kfid of (layer.keyframes || [])) {
      const kf = kfById[kfid];
      if (!kf) continue;
      const klen = kf.length || 0;

      if (frameN >= frameAccum && frameN < frameAccum + klen) {
        // This keyframe spans our target frame.
        if (kf.symbol) {
          const sym = symById[kf.symbol];
          if (sym && (sym.type === 'COLLISION_BOX' || sym.type === 'COLLISION_BODY')) {
            const x      = sym.x       ?? 0;
            const y      = sym.y       ?? 0;
            const width  = sym.scaleX  ?? 0;
            const height = sym.scaleY  ?? 0;
            const rot    = sym.rotation ?? 0;
            const pivX   = sym.pivotX  ?? (width  / 2);
            const pivY   = sym.pivotY  ?? (height / 2);
            // entity_gen stores color as "0xff0000" strings; convert to int.
            const colorRaw = sym.color ?? 0;
            const color = typeof colorRaw === 'string' ? Number(colorRaw) : colorRaw;
            const alpha  = sym.alpha   ?? 1;

            const anchor = collisionBoxAnchor(x, y, pivX, pivY, rot);

            boxes.push({
              layer_name:    layer.name,
              layer_type:    layer.type,
              symbol_type:   sym.type,
              fm_box_type:   fmBoxType(color),
              x, y, width, height,
              rotation:      rot,
              pivot_x:       pivX,
              pivot_y:       pivY,
              color, alpha,
              rendered_anchor: anchor,
            });
          }
        }
        break; // found the keyframe for this layer
      }
      frameAccum += klen;
    }
  }
  return boxes;
}

// ── React fiber walk helpers ──────────────────────────────────────────────────

/** JS to stash component instances and the fiber root into window.__ft. */
const STASH_COMPONENTS_JS = `(()=>{
  let host = null;
  for (const el of document.querySelectorAll('*')) {
    if (el._reactRootContainer) { host = el; break; }
  }
  if (!host) return 'no react root';

  const fiber = host._reactRootContainer._internalRoot.current;
  const fc = {}; let c = 0;
  (function w(f, d) {
    if (!f || d > 80 || c > 10000) return; c++;
    const sn = f.stateNode;
    if (sn && typeof sn === 'object' && sn.constructor && sn !== window) {
      const n = sn.constructor.name;
      if (n && !fc[n]) fc[n] = sn;
    }
    w(f.child, d + 1); w(f.sibling, d);
  })(fiber, 0);

  window.__ft = { fc, fiber };
  return Object.keys(fc).length;
})()`;

/** JS to find controller (getLibraryDirectory + openProject) and tree (onFileDoubleClicked). */
const IDENTIFY_ROLES_JS = `(()=>{
  let ctrl = null, tree = null;
  for (const [k, c] of Object.entries(window.__ft.fc)) {
    if (!ctrl && typeof c.getLibraryDirectory === 'function' && typeof c.openProject === 'function') ctrl = k;
    if (!tree && c.props && typeof c.props.onFileDoubleClicked === 'function') tree = k;
  }
  window.__ctrl = window.__ft.fc[ctrl];
  window.__tree = window.__ft.fc[tree];
  return JSON.stringify({ ctrl, tree });
})()`;

// ── FrayTools UI navigation ───────────────────────────────────────────────────
// Best-effort: tries several strategies in order, reports what worked.

// FrayTools drives the entity editor through a Redux store. The store is
// reachable from any react-redux Provider fiber as `memoizedProps.store`
// (it exposes getState() + dispatch()). Navigation is just dispatching the
// same actions the UI dispatches — verified by recording live UI actions:
//   - animation switch → timeline::SET_ANIMATION + timeline::EDIT_SPRITE_ANIMATION
//   - frame seek        → timeline::SET_FRAME { frameIndex }
// State of truth lives at store.getState().timeline.{animationId,frameIndex}.

/** JS expression that locates the Redux store and stashes it on window.__store. */
const LOCATE_STORE_JS = `(()=>{
  if (window.__store && typeof window.__store.dispatch === 'function') return 'cached';
  let host = null;
  for (const el of document.querySelectorAll('*')) { if (el._reactRootContainer) { host = el; break; } }
  if (!host) return 'no react root';
  const fiber = host._reactRootContainer._internalRoot.current;
  let store = null;
  (function walk(f, d) {
    if (!f || d > 250 || store) return;
    const pr = f.memoizedProps;
    if (pr && pr.store && typeof pr.store.getState === 'function' && typeof pr.store.dispatch === 'function') {
      store = pr.store;
    }
    walk(f.child, d + 1); walk(f.sibling, d);
  })(fiber, 0);
  if (!store) return 'store not found';
  window.__store = store;
  return 'located';
})()`;

/**
 * Select an animation by its FrayTools $id via the Redux store.
 * Dispatches SET_ANIMATION (sets the pointer) then EDIT_SPRITE_ANIMATION
 * with resetSequence:true (actually loads it onto the stage) — the exact
 * pair the UI fires on an animation click. Returns 'ok:<animId>' if the
 * store's timeline.animationId matches afterward, else an error string.
 */
async function selectAnimation(ev, animName, animId) {
  const located = await ev(LOCATE_STORE_JS);
  if (located === 'no react root' || located === 'store not found') {
    return 'ERR store: ' + located;
  }
  return await ev(`(()=>{
    const store = window.__store;
    const ID = ${JSON.stringify(animId)};
    try {
      store.dispatch({ type: 'timeline::SET_ANIMATION', payload: { animationId: ID } });
      store.dispatch({ type: 'timeline::EDIT_SPRITE_ANIMATION', payload: { animationId: ID, resetSequence: true } });
    } catch (e) { return 'ERR dispatch: ' + e.message; }
    const now = store.getState().timeline.animationId;
    return now === ID ? 'ok:store-dispatch' : 'ERR animationId is ' + now + ' after dispatch (wanted ' + ID + ')';
  })()`);
}

/**
 * Seek the timeline to a specific 0-based frame index via the Redux store.
 * Dispatches timeline::SET_FRAME { frameIndex } — the same action the UI
 * fires when you set the playhead. (FrayTools' "Frame: N" display is 1-based,
 * so frameIndex N shows as "Frame: N+1".) Returns 'ok' if the store's
 * timeline.frameIndex matches afterward.
 */
async function seekToFrame(ev, frameN) {
  const located = await ev(LOCATE_STORE_JS);
  if (located === 'no react root' || located === 'store not found') {
    return 'ERR store: ' + located;
  }
  return await ev(`(()=>{
    const store = window.__store;
    const F = ${JSON.stringify(frameN)};
    try {
      store.dispatch({ type: 'timeline::SET_FRAME', payload: { frameIndex: F } });
    } catch (e) { return 'ERR dispatch: ' + e.message; }
    const now = store.getState().timeline.frameIndex;
    return now === F ? 'ok:store-dispatch' : 'ERR frameIndex is ' + now + ' after dispatch (wanted ' + F + ')';
  })()`);
}

// ── Main ─────────────────────────────────────────────────────────────────────

(async () => {
  const port      = parseInt(arg('port', '9222'), 10);
  const ftBin     = arg('fraytools', '/Applications/FrayTools.app/Contents/MacOS/FrayTools');
  const project   = arg('project');
  const entityRel = arg('entity');           // relative to library/
  const targetAnim  = arg('animation', null);
  const targetFrame = arg('frame', null);
  const outJson   = arg('out-json', null);
  const outPng    = arg('out-png', null);
  const settle    = parseInt(arg('settle', '5000'), 10);

  if (!entityRel) die('--entity <relpath under library/> is required');

  // ── 1. Attach or launch FrayTools. ────────────────────────────────────────
  if (!(await cdpUp(port))) {
    console.error(`launching FrayTools (${ftBin}) with --remote-debugging-port=${port}`);
    const child = spawn(ftBin, [`--remote-debugging-port=${port}`], { detached: true, stdio: 'ignore' });
    child.unref();
    if (!(await waitForCdp(port, 20000))) die('CDP never came up');
  } else {
    console.error(`attaching to existing FrayTools on port ${port}`);
  }

  let client;
  try {
    client = await CDP({ port });
    const { Runtime, Page } = client;
    await Page.enable();

    /** Evaluate JS in the renderer. Returns result.value (primitive) or throws. */
    const ev = async (expr) => {
      const { result, exceptionDetails } = await Runtime.evaluate({
        expression: expr, returnByValue: true, awaitPromise: true,
      });
      if (exceptionDetails) throw new Error(exceptionDetails.text || JSON.stringify(exceptionDetails));
      return result.value;
    };

    await sleep(1500);

    // ── 2. Stash React component instances + identify roles. ───────────────
    await ev(STASH_COMPONENTS_JS);
    let roles = JSON.parse(await ev(IDENTIFY_ROLES_JS));
    if (!roles.ctrl) die(`could not locate FrayTools controller component`);

    // ── 3. Open project (if provided and not already loaded). ─────────────
    // If no project is open yet the library tree component won't be mounted,
    // so `roles.tree` may be null here. Open the project first, re-stash, then
    // check again.
    if (project) {
      const cur = await ev(`(()=>{ try { return window.__ctrl.getLibraryDirectory().getPath(); } catch(e) { return null; } })()`);
      const needOpen = !cur || !project.startsWith(cur.replace(/\/library\/?$/, ''));
      if (needOpen || !roles.tree) {
        if (needOpen) {
          console.error(`opening project: ${project}`);
          await ev(`window.__ctrl.openProject(${JSON.stringify(project)})`);
          await sleep(settle);
        }
        // Re-stash — opening a project remounts the tree.
        await ev(STASH_COMPONENTS_JS);
        roles = JSON.parse(await ev(IDENTIFY_ROLES_JS));
      }
    }
    if (!roles.tree) die(`could not locate library tree component (tree=${roles.tree}); is a project open?`);

    // ── 4. Resolve entity FS-node and open it. ────────────────────────────
    const openResult = await ev(`(()=>{ try {
      const root = window.__ctrl.getLibraryDirectory();
      const node = root.resolvePath(${JSON.stringify(entityRel)});
      if (!node.exists()) return 'ERR entity not found: ' + node.getPath();
      window.__entityNode = node;
      window.__tree.props.onFileDoubleClicked(node);
      return 'opened:' + node.getPath();
    } catch(e) { return 'ERR ' + e.message; } })()`);
    console.error(openResult);
    if (String(openResult).startsWith('ERR')) die(openResult);
    await sleep(settle);

    // ── 5. Get entity absolute path and read its JSON. ────────────────────
    const entityAbsPath = await ev(`window.__entityNode.getPath()`);
    console.error(`reading entity data: ${entityAbsPath}`);
    const entityCtx = readEntityData(entityAbsPath);
    const allAnims  = listAnimations(entityCtx);
    console.error(`animations: ${allAnims.length} total`);

    // Resolve target animation (default: first animation).
    const animName = targetAnim || allAnims[0] || null;
    if (!animName) die('entity has no animations');

    const anim = entityCtx.data.animations.find(a => a.name === animName);
    if (!anim) die(`animation not found in entity: "${animName}". Available: ${allAnims.join(', ')}`);

    const animId  = anim['$id'];
    const nFrames = totalFrames(anim, entityCtx);
    const frameN  = targetFrame !== null ? parseInt(targetFrame, 10) : 0;
    if (frameN < 0 || frameN >= nFrames) {
      console.error(`WARN: frame ${frameN} out of range [0, ${nFrames - 1}] — clamping to 0`);
    }
    const clampedFrame = Math.max(0, Math.min(frameN, nFrames - 1));

    // ── 6. Navigate to the target animation + frame via the Redux store. ──
    // Always dispatch (not conditional on "is it the first animation") so the
    // editor is deterministically on the requested animation/frame regardless
    // of what it opened to.
    const navResult = { animation: 'skipped', frame: 'skipped' };

    console.error(`navigating to animation: "${animName}" (${animId})`);
    navResult.animation = await selectAnimation(ev, animName, animId);
    console.error(`animation nav: ${navResult.animation}`);
    await sleep(Math.min(settle / 2, 2000));

    // Seek the frame AFTER the animation switch (EDIT_SPRITE_ANIMATION with
    // resetSequence resets the playhead to 0, so always set the frame last).
    console.error(`seeking to frame: ${clampedFrame}`);
    navResult.frame = await seekToFrame(ev, clampedFrame);
    console.error(`frame seek: ${navResult.frame}`);
    await sleep(Math.min(settle / 2, 2000));

    // ── 7. Extract box data from entity JSON. ─────────────────────────────
    const boxes = boxesAtFrame(animName, clampedFrame, entityCtx) || [];
    console.error(`extracted ${boxes.length} box(es) at ${animName} frame ${clampedFrame}`);

    // ── 8. Capture stage PNG (if --out-png provided). ─────────────────────
    let pngPath = null;
    if (outPng) {
      const rect = JSON.parse(await ev(`(()=>{
        const cs = Array.from(document.querySelectorAll('canvas'));
        let best = null, ba = 0;
        for (const c of cs) { const r = c.getBoundingClientRect(); const a = r.width * r.height; if (a > ba) { ba = a; best = r; } }
        return best ? JSON.stringify({ x: best.x, y: best.y, w: best.width, h: best.height }) : 'null';
      })()`));
      if (rect && rect !== 'null') {
        const ss = await Page.captureScreenshot({
          format: 'png',
          clip: { x: rect.x, y: rect.y, width: rect.w, height: rect.h, scale: 1 },
        });
        fs.writeFileSync(outPng, Buffer.from(ss.data, 'base64'));
        pngPath = outPng;
        console.error(`stage PNG → ${outPng}`);
      } else {
        console.error('WARN: no stage canvas found for PNG capture');
      }
    }

    // ── 9. Build and write output JSON. ───────────────────────────────────
    const report = {
      entity_path:   entityRel,
      animation:     animName,
      frame:         clampedFrame,
      total_frames:  nFrames,
      animations:    allAnims,
      nav:           navResult,
      boxes,
      ...(pngPath ? { png: pngPath } : {}),
    };

    if (outJson) {
      fs.writeFileSync(outJson, JSON.stringify(report, null, 2));
      console.error(`report JSON → ${outJson}`);
    }

    // Always emit compact JSON to stdout for piping.
    console.log(JSON.stringify(report));

  } catch (e) {
    console.error('HARNESS ERROR:', e.message);
    process.exit(1);
  } finally {
    if (client) await client.close();
    // Leave FrayTools running so the caller can reuse it.
  }
})();
