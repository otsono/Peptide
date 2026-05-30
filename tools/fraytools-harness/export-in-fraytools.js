#!/usr/bin/env node
//
// Post-conversion export harness.
//
// Drives the user's LOCAL FrayTools install (Electron, Chrome DevTools
// Protocol) to open a converted project and run FrayTools' own "Publish"
// (the Fraymakers Content Exporter) — producing the game-ready .fra package
// without the user touching FrayTools.
//
// ── IP boundary (load-bearing) ───────────────────────────────────────────────
// This file contains NO FrayTools source, assets, or strings. It only
//   (a) launches the user's existing FrayTools binary, and
//   (b) speaks the standard Chrome DevTools Protocol to it.
// FrayTools is McLeodGaming proprietary software; the binary stays on the
// user's machine and is never committed. Everything here is our own code.
//
// ── How it works (no pixel coordinates) ──────────────────────────────────────
// 1. Attach to (or launch) FrayTools with --remote-debugging-port.
// 2. Walk the React 16 fiber tree to find the controller component (the one
//    exposing getLibraryDirectory + openProject + publish).
// 3. controller.openProject(path) if the project isn't already open.
// 4. controller.publish() — FrayTools' own publish entry point. It opens the
//    Publish dialog in "force" mode, which auto-runs the Fraymakers Content
//    Exporter (verified: writes <projectDir>/build/<id>.fra).
// 5. Poll <projectDir>/build/ for a freshly-written .fra and report it.
//
// ── Usage ─────────────────────────────────────────────────────────────────────
//   node export-in-fraytools.js \
//     --project   /abs/path/character.fraytools \
//     [--fraytools "/Applications/FrayTools.app/Contents/MacOS/FrayTools"] \
//     [--port 9222] [--settle 6000] [--timeout 120000]
//
// Prints the path to the published .fra on stdout on success.

'use strict';
const CDP       = require('chrome-remote-interface');
const { spawn } = require('child_process');
const http      = require('http');
const fs        = require('fs');
const path      = require('path');

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

/** Fetch /json/list and resolve to the parsed array (or [] on any failure). */
function cdpTargets(port) {
  return new Promise(resolve => {
    const req = http.get({ host: '127.0.0.1', port, path: '/json/list', timeout: 800 },
      res => {
        let body = '';
        res.on('data', d => { body += d; });
        res.on('end', () => { try { resolve(JSON.parse(body)); } catch { resolve([]); } });
      });
    req.on('error', () => resolve([]));
    req.on('timeout', () => { req.destroy(); resolve([]); });
  });
}

/**
 * Wait until at least one inspectable page/webview target exists — NOT just the
 * HTTP endpoint. On a cold launch the /json/version endpoint answers 200 well
 * before the renderer registers a page target, so `CDP({port})` would fail with
 * "No inspectable targets". Returns the chosen target, or null on timeout.
 */
async function waitForTarget(port, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const targets = await cdpTargets(port);
    const page = targets.find(t =>
      (t.type === 'page' || t.type === 'webview') && t.webSocketDebuggerUrl);
    if (page) return page;
    await sleep(500);
  }
  return null;
}

/** Newest .fra file under <dir>, or null. Returns { file, mtimeMs }. */
function newestFra(buildDir) {
  let best = null;
  function scan(d) {
    let entries;
    try { entries = fs.readdirSync(d, { withFileTypes: true }); } catch { return; }
    for (const e of entries) {
      const full = path.join(d, e.name);
      if (e.isDirectory()) scan(full);
      else if (e.name.toLowerCase().endsWith('.fra')) {
        try {
          const m = fs.statSync(full).mtimeMs;
          if (!best || m > best.mtimeMs) best = { file: full, mtimeMs: m };
        } catch {}
      }
    }
  }
  scan(buildDir);
  return best;
}

function defaultFrayTools() {
  if (process.platform === 'win32') {
    const la = process.env.LOCALAPPDATA;
    return la ? path.join(la, 'Programs', 'FrayTools', 'FrayTools.exe') : 'FrayTools.exe';
  }
  // macOS (and a reasonable Linux fallback).
  return '/Applications/FrayTools.app/Contents/MacOS/FrayTools';
}

(async () => {
  const port      = parseInt(arg('port', '9222'), 10);
  const ftBin     = arg('fraytools', defaultFrayTools());
  const project   = arg('project');
  const settle    = parseInt(arg('settle', '6000'), 10);
  const timeoutMs = parseInt(arg('timeout', '120000'), 10);

  if (!project) die('--project <abs path to .fraytools> is required');

  const projectDir = path.dirname(project);
  const buildDir   = path.join(projectDir, 'build');
  // Baseline: newest .fra before we publish, so we can detect a NEW one.
  const before = newestFra(buildDir);
  const baselineMtime = before ? before.mtimeMs : 0;

  // 1. Attach or launch FrayTools.
  if (!(await cdpUp(port))) {
    console.error(`launching FrayTools (${ftBin}) with --remote-debugging-port=${port}`);
    const child = spawn(ftBin, [`--remote-debugging-port=${port}`], { detached: true, stdio: 'ignore' });
    child.unref();
    if (!(await waitForCdp(port, 20000))) die('CDP never came up');
  } else {
    console.error(`attaching to existing FrayTools on port ${port}`);
  }

  // The HTTP endpoint answering 200 does NOT mean a page target exists yet — on a
  // cold launch the renderer registers its target a few seconds later. Wait for an
  // actual inspectable page/webview before connecting, else CDP() throws
  // "No inspectable targets". (This was the cold-launch publish failure.)
  const target = await waitForTarget(port, 30000);
  if (!target) die('no inspectable FrayTools page target appeared');

  let client;
  try {
    client = await CDP({ port, target });
    const { Runtime } = client;
    const ev = async (expr) => {
      const { result, exceptionDetails } = await Runtime.evaluate({
        expression: expr, returnByValue: true, awaitPromise: true });
      if (exceptionDetails) throw new Error(exceptionDetails.text || JSON.stringify(exceptionDetails));
      return result.value;
    };

    await sleep(1500);

    // 2. Find the controller component (getLibraryDirectory + openProject + publish).
    const stashJs = `(()=>{
      let host=null; for(const el of document.querySelectorAll('*')){ if(el._reactRootContainer){host=el;break;} }
      if(!host) return 'no react root';
      const fiber = host._reactRootContainer._internalRoot.current;
      let ctrl=null;
      (function w(f,d){ if(!f||d>80||ctrl)return; const sn=f.stateNode;
        if(sn&&typeof sn.getLibraryDirectory==='function'&&typeof sn.openProject==='function'&&typeof sn.publish==='function') ctrl=sn;
        w(f.child,d+1); w(f.sibling,d); })(fiber,0);
      window.__ctrl = ctrl;
      return ctrl ? 'ok' : 'no controller';
    })()`;
    let stash = await ev(stashJs);
    if (stash !== 'ok') die(`could not locate FrayTools controller: ${stash}`);

    // 3. Open the project if it isn't already loaded.
    const cur = await ev(`(()=>{ try { return window.__ctrl.getLibraryDirectory().getPath(); } catch(e){ return null; } })()`);
    const projRoot = project.replace(/\/[^/]+\.fraytools$/, '');
    if (!cur || !cur.startsWith(projRoot)) {
      console.error(`opening project: ${project}`);
      await ev(`window.__ctrl.openProject(${JSON.stringify(project)})`);
      await sleep(settle);
      // Re-stash — a project load can remount the controller.
      stash = await ev(stashJs);
      if (stash !== 'ok') die(`controller lost after openProject: ${stash}`);
    } else {
      console.error('project already open');
    }

    // 4. Trigger FrayTools' publish via "Publish All".
    // The Publish dialog has two actions: "Publish" (force mode, primary
    // folder only) and "Publish All" (every configured output folder). We use
    // Publish All so the package lands in BOTH ./build and any extra folder
    // the converter added (e.g. the Fraymakers custom/<Char> dir). publish()
    // force-mode would only hit the primary folder.
    //
    // First close any stale dialog (so the click targets a fresh one), open
    // the dialog non-force, then click "Publish All". Falls back to force
    // publish() if the button can't be found.
    const close = await ev(`(()=>{ try {
      const c = window.__ctrl;
      const dlgOpen = (c.state && c.state.publishDialogVisible) || !!document.querySelector('.PublishSettingsDialog');
      if (dlgOpen && typeof c.onPublishDialogClose === 'function') c.onPublishDialogClose();
      return 'closed:' + (dlgOpen ? 'yes' : 'no');
    } catch(e){ return 'ERR '+e.message; } })()`);
    if (String(close).startsWith('ERR')) die(`pre-publish cleanup failed: ${close}`);
    if (close === 'closed:yes') await sleep(800);  // let the dialog unmount

    const open = await ev(`(()=>{ try { window.__ctrl.showPublishDialog(false); return 'ok'; } catch(e){ return 'ERR '+e.message; } })()`);
    if (String(open).startsWith('ERR')) die(`showPublishDialog failed: ${open}`);
    await sleep(2500);  // let the dialog + its buttons render

    const pub = await ev(`(()=>{ try {
      const b = Array.from(document.querySelectorAll('button')).find(x => /publish all/i.test((x.textContent||'').trim()));
      if (b) {
        const fk = Object.keys(b).find(k => k.startsWith('__reactInternalInstance'));
        const oc = fk && b[fk].memoizedProps && b[fk].memoizedProps.onClick;
        if (oc) { oc({ type:'click', target:b, currentTarget:b, bubbles:true }); return 'publish-all'; }
        b.click(); return 'publish-all:native';
      }
      // Fallback: force publish (primary folder only).
      window.__ctrl.publish();
      return 'force-publish-fallback';
    } catch(e){ return 'ERR '+e.message; } })()`);
    if (String(pub).startsWith('ERR')) die(`publish failed: ${pub}`);
    console.error(`publish invoked (${pub}) — waiting for the .fra to be written…`);

    // 5. Poll for a freshly written .fra under build/.
    const deadline = Date.now() + timeoutMs;
    let out = null;
    while (Date.now() < deadline) {
      const cand = newestFra(buildDir);
      if (cand && cand.mtimeMs > baselineMtime) {
        // Wait until the file size stops growing (publish finished writing).
        let lastSize = -1, stableTicks = 0;
        while (Date.now() < deadline) {
          let size = 0;
          try { size = fs.statSync(cand.file).size; } catch {}
          if (size > 0 && size === lastSize) { stableTicks++; if (stableTicks >= 3) break; }
          else stableTicks = 0;
          lastSize = size;
          await sleep(500);
        }
        out = cand.file;
        break;
      }
      await sleep(500);
    }

    if (!out) die(`timed out after ${timeoutMs}ms waiting for a .fra under ${buildDir}`);
    console.error(`published: ${out}`);
    console.log(out);   // machine-readable result on stdout
  } catch (e) {
    console.error('EXPORT ERROR:', e.message);
    process.exit(1);
  } finally {
    if (client) await client.close();
    // Leave FrayTools running (attach model) so the caller can reuse it.
  }
})();
