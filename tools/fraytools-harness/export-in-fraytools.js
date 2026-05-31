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

/**
 * Ensure the project's `publishFolders` includes `folderPath` (e.g. the Fraymakers
 * custom/<char> dir), so FrayTools' "Publish All" lands the .fra straight in the game —
 * not just ./build. The converter regenerates publishFolders as just `./build`, so this
 * must run on every export. Edits the .fraytools on disk (FrayTools re-reads it on the
 * reopen below). Returns true if it added the folder.
 */
function ensurePublishFolder(projectFile, projectDir, folderPath) {
  let proj;
  try { proj = JSON.parse(fs.readFileSync(projectFile, 'utf8')); }
  catch (e) { console.error('WARN: could not read project to set publishFolders:', e.message); return false; }
  if (!Array.isArray(proj.publishFolders)) proj.publishFolders = [];
  const target = path.resolve(folderPath);
  try { fs.mkdirSync(folderPath, { recursive: true }); } catch {}
  // FrayTools resolves publishFolders[].path RELATIVE to the project dir (that's why the
  // converter's `./build` entry works). An ABSOLUTE path here gets re-resolved against the
  // project dir into a bogus nested location → the "Fraymakers folder doesn't exist" publish
  // error. So always STORE the path relative to the project dir.
  const relPath = path.relative(projectDir, folderPath);
  const existing = proj.publishFolders.find(f =>
    f && typeof f.path === 'string' && path.resolve(projectDir, f.path) === target);
  if (existing) {
    if (existing.path === relPath) return false;     // already correct
    // Self-heal a stale ABSOLUTE (or otherwise non-relative) entry left by an older export.
    existing.path = relPath;
    fs.writeFileSync(projectFile, JSON.stringify(proj, null, 2));
    console.error(`normalized Fraymakers publish folder to relative: ${relPath}`);
    return true;
  }
  proj.publishFolders.push({ id: 'fraymakers0', path: relPath });
  fs.writeFileSync(projectFile, JSON.stringify(proj, null, 2));
  console.error(`added Fraymakers publish folder: ${relPath}`);
  return true;
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

  // Ensure the game's custom/<char> dir is a publish target (re-added every export
  // because the converter rewrites publishFolders to just ./build).
  const fraymakersRoot = arg('fraymakers-root',
    path.join(process.env.HOME || '', 'Library/Application Support/Steam/steamapps/common/Fraymakers'));
  const charId = path.basename(project).replace(/\.fraytools$/, '');
  const fraymakersCustom = path.join(fraymakersRoot, 'custom', charId);
  ensurePublishFolder(project, projectDir, fraymakersCustom);
  // Baseline: newest .fra before we publish, so we can detect a NEW one.
  const before = newestFra(buildDir);
  const baselineMtime = before ? before.mtimeMs : 0;

  // 1. (Re)launch FrayTools fresh.
  // The in-place "reopen the already-open project" path intermittently loses the React
  // controller for good (the post-open re-stash never recovers), whereas a COLD launch +
  // fresh open is reliable. So if FrayTools is already running, quit it first and cold-launch.
  if (await cdpUp(port)) {
    console.error('FrayTools already running — quitting it for a clean cold relaunch');
    try { spawn('pkill', ['-f', `remote-debugging-port=${port}`], { stdio: 'ignore' }).unref(); } catch {}
    const qDeadline = Date.now() + 15000;
    while (Date.now() < qDeadline) { if (!(await cdpUp(port))) break; await sleep(500); }
    await sleep(1500);  // let the debug port fully release before relaunch
  }
  console.error(`launching FrayTools (${ftBin}) with --remote-debugging-port=${port}`);
  {
    const child = spawn(ftBin, [`--remote-debugging-port=${port}`], { detached: true, stdio: 'ignore' });
    child.unref();
    if (!(await waitForCdp(port, 20000))) die('CDP never came up');
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
    // ALWAYS (re)open the project — even if already open — so FrayTools reloads the
    // fresh on-disk source + the publishFolders we just edited. Publishing from a stale
    // in-memory copy was silently shipping the OLD converted scripts.
    const cur = await ev(`(()=>{ try { return window.__ctrl.getLibraryDirectory().getPath(); } catch(e){ return null; } })()`);
    const projRoot = project.replace(/\/[^/]+\.fraytools$/, '');
    console.error(cur && cur.startsWith(projRoot) ? `reopening project (reload from disk): ${project}` : `opening project: ${project}`);
    await ev(`window.__ctrl.openProject(${JSON.stringify(project)})`);
    await sleep(settle);
    // Re-stash — a project load REMOUNTS the controller (and can briefly tear down the
    // React root / execution context). The remount frequently lags the openProject call
    // by longer than `settle`, so a single check races it and reports "no controller".
    // Poll until the controller reappears, tolerating transient ev() failures during the
    // remount, instead of checking exactly once.
    const restashDeadline = Date.now() + 60000;
    stash = 'no controller';
    while (Date.now() < restashDeadline) {
      try { stash = await ev(stashJs); } catch (e) { stash = 'err:' + (e && e.message); }
      if (stash === 'ok') break;
      await sleep(1000);
    }
    if (stash !== 'ok') die(`controller lost after openProject: ${stash}`);

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
