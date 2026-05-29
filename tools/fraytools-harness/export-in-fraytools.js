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

(async () => {
  const port      = parseInt(arg('port', '9222'), 10);
  const ftBin     = arg('fraytools', '/Applications/FrayTools.app/Contents/MacOS/FrayTools');
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

  let client;
  try {
    client = await CDP({ port });
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

    // 4. Trigger FrayTools' publish (force mode → auto-runs the exporter).
    // publish() force-runs the exporter only when the Publish dialog *mounts*.
    // If a dialog is already open (e.g. left over from a prior run), re-calling
    // publish() is a no-op — so close any open dialog first, then publish.
    const pub = await ev(`(()=>{ try {
      const c = window.__ctrl;
      const dlgOpen = (c.state && c.state.publishDialogVisible) || !!document.querySelector('.PublishSettingsDialog');
      if (dlgOpen && typeof c.onPublishDialogClose === 'function') c.onPublishDialogClose();
      return 'closed:' + (dlgOpen ? 'yes' : 'no');
    } catch(e){ return 'ERR '+e.message; } })()`);
    if (String(pub).startsWith('ERR')) die(`pre-publish cleanup failed: ${pub}`);
    if (pub === 'closed:yes') await sleep(800);  // let the dialog unmount

    const pub2 = await ev(`(()=>{ try { window.__ctrl.publish(); return 'published'; } catch(e){ return 'ERR '+e.message; } })()`);
    if (String(pub2).startsWith('ERR')) die(`publish() failed: ${pub2}`);
    console.error('publish() invoked — waiting for the .fra to be written…');

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
