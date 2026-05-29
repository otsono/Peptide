#!/usr/bin/env node
//
// Tier-3 FrayTools render harness.
//
// Drives the user's LOCAL FrayTools install (Electron, Chrome DevTools
// Protocol) to load a converted entity onto the editor stage and capture
// exactly how FrayTools renders it. This is the ground-truth check that
// our reconstructed transform math (docs/fraytools_internals.md) can't
// give on its own — e.g. whether a rotated itembox lands on the hand.
//
// ── IP boundary (load-bearing) ───────────────────────────────────────
// This file contains NO FrayTools source, assets, or strings. It only
//   (a) launches the user's existing FrayTools binary, and
//   (b) speaks the standard Chrome DevTools Protocol to it.
// FrayTools is McLeodGaming proprietary software; the binary stays on the
// user's machine and is never committed. Everything here is our own code.
//
// ── How it navigates (no pixel coordinates) ──────────────────────────
// Pure code navigation through FrayTools' own runtime objects:
//   1. Attach to (or launch) FrayTools with --remote-debugging-port.
//   2. Walk the React 16 fiber tree (instances are attached to DOM nodes
//      as __reactInternalInstance$… in this Electron/Chrome 80 build) to
//      grab the app's component instances by class.
//   3. Call the controller component's openProject(path).
//   4. Ask the controller for its library directory FS-node and
//      resolvePath() the target entity — a real node with the methods
//      FrayTools' open handler needs (isDirectory(), getPath(), …).
//   5. Hand that node to the library tree's onFileDoubleClicked() prop —
//      the same callback a real double-click fires — to open the entity
//      onto the stage.
//   6. Clip-capture the stage <canvas> to a PNG.
//
// ── Usage ────────────────────────────────────────────────────────────
//   node render-entity.js \
//     --fraytools "/Applications/FrayTools.app/Contents/MacOS/FrayTools" \
//     --project   /abs/path/characters/mario/mario.fraytools \
//     --entity    entities/Mario.entity \
//     --out       /tmp/mario_aerial_back.png \
//     [--port 9222] [--settle 6000]
//
// If a FrayTools is already listening on --port, the harness attaches to
// it instead of launching a new one (and leaves it running).

const CDP = require('chrome-remote-interface');
const { spawn } = require('child_process');
const http = require('http');
const fs = require('fs');

const sleep = ms => new Promise(r => setTimeout(r, ms));

function arg(name, def) {
  const i = process.argv.indexOf('--' + name);
  return i >= 0 && i + 1 < process.argv.length ? process.argv[i + 1] : def;
}

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

(async () => {
  const port = parseInt(arg('port', '9222'), 10);
  const fraytools = arg('fraytools', '/Applications/FrayTools.app/Contents/MacOS/FrayTools');
  const project = arg('project');
  const entity = arg('entity');             // e.g. entities/Mario.entity (relative to library/)
  const out = arg('out', '/tmp/fraytools_render.png');
  const settle = parseInt(arg('settle', '6000'), 10);

  if (!entity) { console.error('ERROR: --entity <relpath under library/> is required'); process.exit(2); }

  // 1. Attach or launch.
  let launched = null;
  if (!(await cdpUp(port))) {
    console.error(`launching FrayTools (${fraytools}) with --remote-debugging-port=${port}`);
    launched = spawn(fraytools, [`--remote-debugging-port=${port}`], { detached: true, stdio: 'ignore' });
    launched.unref();
    if (!(await waitForCdp(port, 20000))) { console.error('ERROR: CDP never came up'); process.exit(1); }
  } else {
    console.error(`attaching to existing FrayTools on port ${port}`);
  }

  let client;
  try {
    client = await CDP({ port });
    const { Runtime, Page } = client;
    await Page.enable();
    const ev = async (expr) =>
      (await Runtime.evaluate({ expression: expr, returnByValue: true, awaitPromise: true })).result.value;

    await sleep(1500);

    // 2. Stash the app's React component instances by class name on window.__fc.
    await ev(`(()=>{
      let host=null; for(const el of document.querySelectorAll('*')){ if(el._reactRootContainer){host=el;break;} }
      if(!host) return 'no react root';
      const fiber=host._reactRootContainer._internalRoot.current; const fc={}; let c=0;
      (function w(f,d){ if(!f||d>60||c>6000)return; c++; const sn=f.stateNode;
        if(sn&&typeof sn==='object'&&sn.constructor&&sn!==window){ const n=sn.constructor.name; if(n&&!fc[n]) fc[n]=sn; }
        w(f.child,d+1); w(f.sibling,d); })(fiber,0);
      window.__fc=fc; return Object.keys(fc).length;
    })()`);

    // Identify the controller (has getLibraryDirectory) and the tree (has onFileDoubleClicked prop).
    const roles = JSON.parse(await ev(`(()=>{
      let ctrl=null, tree=null;
      for(const k of Object.keys(window.__fc)){ const c=window.__fc[k];
        if(!ctrl && typeof c.getLibraryDirectory==='function' && typeof c.openProject==='function') ctrl=k;
        if(!tree && c.props && typeof c.props.onFileDoubleClicked==='function') tree=k;
      }
      window.__ctrl=window.__fc[ctrl]; window.__tree=window.__fc[tree];
      return JSON.stringify({ctrl, tree});
    })()`));
    if (!roles.ctrl || !roles.tree) { console.error('ERROR: could not locate controller/tree components', roles); process.exit(1); }

    // 3. Open the project if one was provided and it's not already loaded.
    if (project) {
      const cur = await ev(`(()=>{ try{ return window.__ctrl.getLibraryDirectory().getPath(); }catch(e){ return null; } })()`);
      if (!cur || !project.startsWith(cur.replace(/\/library\/?$/, ''))) {
        console.error(`opening project: ${project}`);
        await ev(`(()=>{ try{ window.__ctrl.openProject(${JSON.stringify(project)}); return 'ok'; }catch(e){ return 'ERR '+e.message; } })()`);
        await sleep(settle);
        // Re-stash components (a project load can remount the tree).
        await ev(`(()=>{ let host=null; for(const el of document.querySelectorAll('*')){ if(el._reactRootContainer){host=el;break;} }
          const fiber=host._reactRootContainer._internalRoot.current; const fc={}; let c=0;
          (function w(f,d){ if(!f||d>60||c>6000)return; c++; const sn=f.stateNode;
            if(sn&&typeof sn==='object'&&sn.constructor&&sn!==window){ const n=sn.constructor.name; if(n&&!fc[n]) fc[n]=sn; } w(f.child,d+1); w(f.sibling,d); })(fiber,0);
          window.__fc=fc;
          for(const k of Object.keys(fc)){ const c=fc[k];
            if(typeof c.getLibraryDirectory==='function'&&typeof c.openProject==='function') window.__ctrl=c;
            if(c.props&&typeof c.props.onFileDoubleClicked==='function') window.__tree=c; }
          return 'restashed'; })()`);
      }
    }

    // 4+5. Resolve the real entity node and open it via the tree's own callback.
    const open = await ev(`(()=>{ try{
      const root=window.__ctrl.getLibraryDirectory();
      const node=root.resolvePath(${JSON.stringify(entity)});
      if(!node.exists()) return 'ERR entity not found: '+node.getPath();
      window.__tree.props.onFileDoubleClicked(node);
      return 'opened '+node.getPath();
    }catch(e){ return 'ERR '+e.message; } })()`);
    console.error(open);
    if (String(open).startsWith('ERR')) process.exit(1);
    await sleep(settle);

    // 6. Clip-capture the largest on-screen canvas (the stage).
    const rect = JSON.parse(await ev(`(()=>{ const cs=Array.from(document.querySelectorAll('canvas'));
      let best=null,ba=0; for(const c of cs){const r=c.getBoundingClientRect(); const a=r.width*r.height; if(a>ba){ba=a;best=r;}}
      return best?JSON.stringify({x:best.x,y:best.y,w:best.width,h:best.height}):'null'; })()`));
    if (!rect || rect === 'null') { console.error('ERROR: no stage canvas found'); process.exit(1); }
    const ss = await Page.captureScreenshot({ format: 'png',
      clip: { x: rect.x, y: rect.y, width: rect.w, height: rect.h, scale: 1 } });
    fs.writeFileSync(out, Buffer.from(ss.data, 'base64'));
    console.log(out);
  } catch (e) {
    console.error('HARNESS ERROR:', e.message); process.exit(1);
  } finally {
    if (client) await client.close();
    // Leave FrayTools running (attach model); the caller can reuse it.
  }
})();
