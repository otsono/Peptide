//! fraytools — drive the user's LOCAL FrayTools install (Electron) over the Chrome
//! DevTools Protocol. Three subcommands, ported from the Node tools/fraytools-harness/*.js
//! to pure Rust (no Node, no chrome-remote-interface):
//!   * `peptide export`  — run FrayTools' own Publish to build the game-ready .fra.
//!   * `peptide render`  — open an entity on the stage and clip-capture the canvas PNG.
//!   * `peptide harness` — open + navigate to an animation/frame, extract box geometry
//!                         from the entity JSON, capture a PNG, and emit a JSON report.
//!
//! The in-page JavaScript (React-16 fiber walking, FrayTools controller calls, Redux
//! dispatch) is preserved VERBATIM from the original scripts — it's the fragile,
//! load-bearing part. Rust only owns the transport (HTTP + WebSocket CDP), the process
//! orchestration, and the pure filesystem logic (publish-folder edit, box extraction).
//!
//! IP boundary (load-bearing): contains NO FrayTools source, assets, or strings. It only
//! (a) launches the user's existing FrayTools binary and (b) speaks standard CDP to it.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Map, Value};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

// ─────────────────────────────── entry points ───────────────────────────────

/// `peptide export --project <abs.fraytools> [--fraytools BIN] [--port N] [--settle MS]
/// [--timeout MS] [--fraymakers-root DIR]` — cold-launch FrayTools, open the project,
/// run Publish All, and print the freshly written .fra path to stdout.
pub fn export(args: &[String]) -> Result<()> {
    let port = arg_u16(args, "port", 9222);
    let ft_bin = arg(args, "fraytools").map(String::from).unwrap_or_else(default_fraytools);
    let project = arg(args, "project").ok_or_else(|| anyhow!("--project <abs path to .fraytools> is required"))?;
    let settle = arg_u64(args, "settle", 6000);
    let timeout_ms = arg_u64(args, "timeout", 120_000);

    let project = PathBuf::from(project);
    let project_dir = project.parent().unwrap_or(Path::new(".")).to_path_buf();
    let build_dir = project_dir.join("build");

    // Ensure the game's custom/<char> dir is a publish target (re-added every export
    // because the converter rewrites publishFolders to just ./build).
    let fraymakers_root = arg(args, "fraymakers-root").map(PathBuf::from).unwrap_or_else(default_fraymakers_root);
    let char_id = project.file_name().and_then(|s| s.to_str()).unwrap_or("")
        .trim_end_matches(".fraytools").to_string();
    let fraymakers_custom = fraymakers_root.join("custom").join(&char_id);
    ensure_publish_folder(&project, &project_dir, &fraymakers_custom);

    // Baseline: newest .fra before we publish, so we can detect a NEW one.
    let baseline = newest_fra(&build_dir).map(|(_, m)| m).unwrap_or(0.0);

    // 1. (Re)launch FrayTools fresh. The in-place "reopen the already-open project" path
    // intermittently loses the React controller for good; a COLD launch + fresh open is
    // reliable. So if FrayTools is already running, force-kill it first (SIGKILL avoids the
    // "Are you sure you want to quit?" dialog that would otherwise hold the debug port).
    if cdp_up(port) {
        eprintln!("FrayTools already running — force-killing it for a clean cold relaunch");
        kill_fraytools(port);
        let qdl = Instant::now() + Duration::from_secs(15);
        while Instant::now() < qdl {
            if !cdp_up(port) { break; }
            thread::sleep(Duration::from_millis(500));
        }
        thread::sleep(Duration::from_secs(3)); // let it fully exit + release the port
    }
    eprintln!("launching FrayTools ({ft_bin}) with --remote-debugging-port={port}");
    launch_fraytools(&ft_bin, port)?;
    if !wait_for_cdp(port, Duration::from_secs(45)) { bail!("CDP never came up"); }

    let ws = wait_for_target(port, Duration::from_secs(30))
        .ok_or_else(|| anyhow!("no inspectable FrayTools page target appeared"))?;
    let mut cdp = Cdp::connect(&ws)?;
    thread::sleep(Duration::from_millis(1500));

    // 2. Locate the controller (getLibraryDirectory + openProject + publish).
    if cdp.eval_str(STASH_CTRL_JS)? != "ok" {
        bail!("could not locate FrayTools controller");
    }

    // 3. ALWAYS (re)open the project so FrayTools reloads fresh on-disk source + the
    // publishFolders we just edited. Publishing from a stale in-memory copy was silently
    // shipping the OLD converted scripts.
    eprintln!("opening project: {}", project.display());
    cdp.eval(&format!("window.__ctrl.openProject({})", js_string(&project.to_string_lossy())))?;
    thread::sleep(Duration::from_millis(settle));
    // A project load REMOUNTS the controller (and can briefly tear down the React root).
    // Poll until it reappears instead of checking once.
    let restash_dl = Instant::now() + Duration::from_secs(60);
    let mut stash = String::from("no controller");
    while Instant::now() < restash_dl {
        stash = cdp.eval_str(STASH_CTRL_JS).unwrap_or_else(|e| format!("err:{e}"));
        if stash == "ok" { break; }
        thread::sleep(Duration::from_secs(1));
    }
    if stash != "ok" { bail!("controller lost after openProject: {stash}"); }

    // 4. Publish via "Publish All" (every configured output folder, not just primary).
    let close = cdp.eval_str(CLOSE_DIALOG_JS)?;
    if close.starts_with("ERR") { bail!("pre-publish cleanup failed: {close}"); }
    if close == "closed:yes" { thread::sleep(Duration::from_millis(800)); }
    let open = cdp.eval_str("(()=>{ try { window.__ctrl.showPublishDialog(false); return 'ok'; } catch(e){ return 'ERR '+e.message; } })()")?;
    if open.starts_with("ERR") { bail!("showPublishDialog failed: {open}"); }
    thread::sleep(Duration::from_millis(2500)); // let the dialog + its buttons render
    let pub_res = cdp.eval_str(PUBLISH_ALL_JS)?;
    if pub_res.starts_with("ERR") { bail!("publish failed: {pub_res}"); }
    eprintln!("publish invoked ({pub_res}) — waiting for the .fra to be written…");

    // 5. Poll for a freshly written .fra under build/, then wait for its size to settle.
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut out: Option<PathBuf> = None;
    while Instant::now() < deadline {
        if let Some((file, m)) = newest_fra(&build_dir) {
            if m > baseline {
                let mut last_size = -1i64;
                let mut stable = 0;
                while Instant::now() < deadline {
                    let size = std::fs::metadata(&file).map(|md| md.len() as i64).unwrap_or(0);
                    if size > 0 && size == last_size {
                        stable += 1;
                        if stable >= 3 { break; }
                    } else {
                        stable = 0;
                    }
                    last_size = size;
                    thread::sleep(Duration::from_millis(500));
                }
                out = Some(file);
                break;
            }
        }
        thread::sleep(Duration::from_millis(500));
    }
    let out = out.ok_or_else(|| anyhow!("timed out after {timeout_ms}ms waiting for a .fra under {}", build_dir.display()))?;
    eprintln!("published: {}", out.display());
    println!("{}", out.display()); // machine-readable result on stdout
    Ok(())
}

/// `peptide render --entity <rel-under-library> [--project ...] [--out PNG] [...]` —
/// open the entity on the stage and clip-capture the largest canvas to a PNG.
pub fn render(args: &[String]) -> Result<()> {
    let port = arg_u16(args, "port", 9222);
    let ft_bin = arg(args, "fraytools").map(String::from).unwrap_or_else(default_fraytools);
    let project = arg(args, "project");
    let entity = arg(args, "entity").ok_or_else(|| anyhow!("--entity <relpath under library/> is required"))?;
    let out = arg(args, "out").unwrap_or("/tmp/fraytools_render.png");
    let settle = arg_u64(args, "settle", 6000);

    let mut cdp = attach_or_launch(port, &ft_bin)?;
    thread::sleep(Duration::from_millis(1500));

    stash_components(&mut cdp)?;
    let mut roles = identify_roles(&mut cdp)?;
    if roles.0.is_none() { bail!("could not locate FrayTools controller component"); }

    if let Some(project) = project {
        if open_project_if_needed(&mut cdp, project, settle)? {
            stash_components(&mut cdp)?;
            roles = identify_roles(&mut cdp)?;
        }
    }
    if roles.1.is_none() { bail!("could not locate library tree component; is a project open?"); }

    let _entity_abs = open_entity(&mut cdp, entity)?;
    thread::sleep(Duration::from_millis(settle));

    let png = capture_canvas(&mut cdp)?.ok_or_else(|| anyhow!("no stage canvas found"))?;
    std::fs::write(out, &png)?;
    println!("{out}");
    Ok(())
}

/// `peptide harness --entity <rel> [--project ...] [--animation NAME] [--frame N]
/// [--out-json J] [--out-png P] [...]` — navigate to an animation/frame, extract box
/// geometry from the entity JSON, optionally capture a PNG, and emit a JSON report.
pub fn harness(args: &[String]) -> Result<()> {
    let port = arg_u16(args, "port", 9222);
    let ft_bin = arg(args, "fraytools").map(String::from).unwrap_or_else(default_fraytools);
    let project = arg(args, "project");
    let entity = arg(args, "entity").ok_or_else(|| anyhow!("--entity <relpath under library/> is required"))?;
    let target_anim = arg(args, "animation");
    let target_frame = arg(args, "frame");
    let out_json = arg(args, "out-json");
    let out_png = arg(args, "out-png");
    let settle = arg_u64(args, "settle", 5000);

    let mut cdp = attach_or_launch(port, &ft_bin)?;
    thread::sleep(Duration::from_millis(1500));

    // 2. Stash components + identify roles.
    stash_components(&mut cdp)?;
    let mut roles = identify_roles(&mut cdp)?;
    if roles.0.is_none() { bail!("could not locate FrayTools controller component"); }

    // 3. Open the project (if provided / not already loaded).
    if let Some(project) = project {
        let opened = open_project_if_needed(&mut cdp, project, settle)?;
        if opened || roles.1.is_none() {
            stash_components(&mut cdp)?;
            roles = identify_roles(&mut cdp)?;
        }
    }
    if roles.1.is_none() { bail!("could not locate library tree component; is a project open?"); }

    // 4. Resolve + open the entity, then read its JSON from disk.
    let entity_abs = open_entity(&mut cdp, entity)?;
    thread::sleep(Duration::from_millis(settle));
    eprintln!("reading entity data: {entity_abs}");
    let raw = std::fs::read_to_string(&entity_abs)?;
    let ctx = EntityCtx::parse(&raw)?;
    let all_anims = ctx.animation_names();
    eprintln!("animations: {} total", all_anims.len());

    let anim_name = target_anim.map(String::from)
        .or_else(|| all_anims.first().cloned())
        .ok_or_else(|| anyhow!("entity has no animations"))?;
    let anim = ctx.animation(&anim_name)
        .ok_or_else(|| anyhow!("animation not found in entity: \"{anim_name}\". Available: {}", all_anims.join(", ")))?;
    let anim_id = anim.get("$id").cloned().unwrap_or(Value::Null);
    let n_frames = ctx.total_frames(&anim);
    let frame_n: i64 = target_frame.and_then(|f| f.parse().ok()).unwrap_or(0);
    if frame_n < 0 || frame_n >= n_frames {
        eprintln!("WARN: frame {frame_n} out of range [0, {}] — clamping", n_frames - 1);
    }
    let clamped = frame_n.max(0).min((n_frames - 1).max(0));

    // 6. Navigate to the animation + frame via the Redux store (always dispatch).
    eprintln!("navigating to animation: \"{anim_name}\" ({anim_id})");
    let nav_anim = select_animation(&mut cdp, &anim_id)?;
    eprintln!("animation nav: {nav_anim}");
    thread::sleep(Duration::from_millis(settle.min(4000) / 2));
    // Seek the frame AFTER the animation switch (resetSequence resets the playhead to 0).
    eprintln!("seeking to frame: {clamped}");
    let nav_frame = seek_to_frame(&mut cdp, clamped)?;
    eprintln!("frame seek: {nav_frame}");
    thread::sleep(Duration::from_millis(settle.min(4000) / 2));

    // 7. Extract box data from entity JSON (pure FS read — no UI dependency).
    let boxes = ctx.boxes_at_frame(&anim_name, clamped);
    eprintln!("extracted {} box(es) at {anim_name} frame {clamped}", boxes.len());

    // 8. Capture stage PNG (if requested).
    let mut png_path: Option<String> = None;
    if let Some(out_png) = out_png {
        match capture_canvas(&mut cdp)? {
            Some(png) => {
                std::fs::write(out_png, &png)?;
                eprintln!("stage PNG → {out_png}");
                png_path = Some(out_png.to_string());
            }
            None => eprintln!("WARN: no stage canvas found for PNG capture"),
        }
    }

    // 9. Build + emit the report.
    let mut report = Map::new();
    report.insert("entity_path".into(), json!(entity));
    report.insert("animation".into(), json!(anim_name));
    report.insert("frame".into(), json!(clamped));
    report.insert("total_frames".into(), json!(n_frames));
    report.insert("animations".into(), json!(all_anims));
    report.insert("nav".into(), json!({ "animation": nav_anim, "frame": nav_frame }));
    report.insert("boxes".into(), Value::Array(boxes));
    if let Some(p) = &png_path { report.insert("png".into(), json!(p)); }
    let report = Value::Object(report);

    if let Some(out_json) = out_json {
        std::fs::write(out_json, serde_json::to_string_pretty(&report)?)?;
        eprintln!("report JSON → {out_json}");
    }
    println!("{}", serde_json::to_string(&report)?); // compact JSON to stdout for piping
    Ok(())
}

// ───────────────────────────── shared CDP steps ─────────────────────────────

/// Attach to a running FrayTools on `port`, or cold-launch one, then connect CDP to its
/// page target.
fn attach_or_launch(port: u16, ft_bin: &str) -> Result<Cdp> {
    if !cdp_up(port) {
        eprintln!("launching FrayTools ({ft_bin}) with --remote-debugging-port={port}");
        launch_fraytools(ft_bin, port)?;
        if !wait_for_cdp(port, Duration::from_secs(20)) { bail!("CDP never came up"); }
    } else {
        eprintln!("attaching to existing FrayTools on port {port}");
    }
    let ws = wait_for_target(port, Duration::from_secs(30))
        .ok_or_else(|| anyhow!("no inspectable FrayTools page target appeared"))?;
    Cdp::connect(&ws)
}

fn stash_components(cdp: &mut Cdp) -> Result<()> {
    cdp.eval(STASH_COMPONENTS_JS)?;
    Ok(())
}

/// Returns (controller-class, tree-class) — either is None if not found.
fn identify_roles(cdp: &mut Cdp) -> Result<(Option<String>, Option<String>)> {
    let s = cdp.eval_str(IDENTIFY_ROLES_JS)?;
    let v: Value = serde_json::from_str(&s).unwrap_or(Value::Null);
    let ctrl = v.get("ctrl").and_then(|x| x.as_str()).map(String::from);
    let tree = v.get("tree").and_then(|x| x.as_str()).map(String::from);
    Ok((ctrl, tree))
}

/// Open `project` if it isn't already the loaded library root. Returns true if it opened
/// (so the caller knows to re-stash, since a project load remounts the tree).
fn open_project_if_needed(cdp: &mut Cdp, project: &str, settle: u64) -> Result<bool> {
    let cur = cdp.eval(r#"(()=>{ try { return window.__ctrl.getLibraryDirectory().getPath(); } catch(e){ return null; } })()"#)?;
    let cur_root = cur.as_str().map(|c| c.trim_end_matches("/library").trim_end_matches("/library/").to_string());
    let need = match &cur_root {
        Some(root) if !root.is_empty() => !project.starts_with(root),
        _ => true,
    };
    if need {
        eprintln!("opening project: {project}");
        cdp.eval(&format!("window.__ctrl.openProject({})", js_string(project)))?;
        thread::sleep(Duration::from_millis(settle));
    }
    Ok(need)
}

/// Resolve the entity FS-node under the library root and open it on the stage via the
/// library tree's own onFileDoubleClicked callback. Returns the entity's absolute path.
fn open_entity(cdp: &mut Cdp, entity_rel: &str) -> Result<String> {
    let open = cdp.eval_str(&format!(
        r#"(()=>{{ try {{
          const root = window.__ctrl.getLibraryDirectory();
          const node = root.resolvePath({rel});
          if (!node.exists()) return 'ERR entity not found: ' + node.getPath();
          window.__entityNode = node;
          window.__tree.props.onFileDoubleClicked(node);
          return 'opened:' + node.getPath();
        }} catch(e) {{ return 'ERR ' + e.message; }} }})()"#,
        rel = js_string(entity_rel)
    ))?;
    eprintln!("{open}");
    if open.starts_with("ERR") { bail!("{open}"); }
    let abs = cdp.eval_str("window.__entityNode.getPath()")?;
    Ok(abs)
}

/// Select an animation by FrayTools $id via the Redux store (the same SET_ANIMATION +
/// EDIT_SPRITE_ANIMATION pair the UI fires on a click).
fn select_animation(cdp: &mut Cdp, anim_id: &Value) -> Result<String> {
    let located = cdp.eval_str(LOCATE_STORE_JS)?;
    if located == "no react root" || located == "store not found" {
        return Ok(format!("ERR store: {located}"));
    }
    cdp.eval_str(&format!(
        r#"(()=>{{
          const store = window.__store;
          const ID = {id};
          try {{
            store.dispatch({{ type: 'timeline::SET_ANIMATION', payload: {{ animationId: ID }} }});
            store.dispatch({{ type: 'timeline::EDIT_SPRITE_ANIMATION', payload: {{ animationId: ID, resetSequence: true }} }});
          }} catch (e) {{ return 'ERR dispatch: ' + e.message; }}
          const now = store.getState().timeline.animationId;
          return now === ID ? 'ok:store-dispatch' : 'ERR animationId is ' + now + ' after dispatch (wanted ' + ID + ')';
        }})()"#,
        id = serde_json::to_string(anim_id).unwrap_or_else(|_| "null".into())
    ))
}

/// Seek the timeline to a 0-based frame index via the Redux store (timeline::SET_FRAME).
fn seek_to_frame(cdp: &mut Cdp, frame_n: i64) -> Result<String> {
    let located = cdp.eval_str(LOCATE_STORE_JS)?;
    if located == "no react root" || located == "store not found" {
        return Ok(format!("ERR store: {located}"));
    }
    cdp.eval_str(&format!(
        r#"(()=>{{
          const store = window.__store;
          const F = {frame_n};
          try {{
            store.dispatch({{ type: 'timeline::SET_FRAME', payload: {{ frameIndex: F }} }});
          }} catch (e) {{ return 'ERR dispatch: ' + e.message; }}
          const now = store.getState().timeline.frameIndex;
          return now === F ? 'ok:store-dispatch' : 'ERR frameIndex is ' + now + ' after dispatch (wanted ' + F + ')';
        }})()"#
    ))
}

/// Clip-capture the largest on-screen canvas (the stage) to PNG bytes, or None if no
/// canvas is present.
fn capture_canvas(cdp: &mut Cdp) -> Result<Option<Vec<u8>>> {
    let rect_s = cdp.eval_str(r#"(()=>{
      const cs = Array.from(document.querySelectorAll('canvas'));
      let best = null, ba = 0;
      for (const c of cs) { const r = c.getBoundingClientRect(); const a = r.width * r.height; if (a > ba) { ba = a; best = r; } }
      return best ? JSON.stringify({ x: best.x, y: best.y, w: best.width, h: best.height }) : 'null';
    })()"#)?;
    if rect_s == "null" || rect_s.is_empty() { return Ok(None); }
    let rect: Value = serde_json::from_str(&rect_s).unwrap_or(Value::Null);
    let (x, y, w, h) = (
        rect.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0),
        rect.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0),
        rect.get("w").and_then(|v| v.as_f64()).unwrap_or(0.0),
        rect.get("h").and_then(|v| v.as_f64()).unwrap_or(0.0),
    );
    if w <= 0.0 || h <= 0.0 { return Ok(None); }
    let png = cdp.screenshot(json!({ "x": x, "y": y, "width": w, "height": h, "scale": 1 }))?;
    Ok(Some(png))
}

// ───────────────────────────────── CDP client ───────────────────────────────

struct Cdp {
    ws: WebSocket<MaybeTlsStream<TcpStream>>,
    next_id: i64,
}

impl Cdp {
    fn connect(ws_url: &str) -> Result<Cdp> {
        let (ws, _resp) = tungstenite::connect(ws_url)
            .map_err(|e| anyhow!("CDP websocket connect failed ({ws_url}): {e}"))?;
        Ok(Cdp { ws, next_id: 1 })
    }

    /// Send a CDP command and block for the response with the matching id (events are
    /// skipped).
    fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let msg = json!({ "id": id, "method": method, "params": params });
        self.ws.send(Message::Text(msg.to_string()))?;
        loop {
            let txt = match self.ws.read()? {
                Message::Text(t) => t,
                Message::Close(_) => bail!("CDP socket closed"),
                _ => continue,
            };
            let v: Value = serde_json::from_str(&txt)?;
            if v.get("id").and_then(|x| x.as_i64()) == Some(id) {
                if let Some(err) = v.get("error") {
                    bail!("CDP error from {method}: {err}");
                }
                return Ok(v.get("result").cloned().unwrap_or(Value::Null));
            }
            // otherwise it's an event for some other id — keep reading
        }
    }

    /// Runtime.evaluate -> result.value. Errors carry the exception text.
    fn eval(&mut self, expr: &str) -> Result<Value> {
        let res = self.call("Runtime.evaluate", json!({
            "expression": expr, "returnByValue": true, "awaitPromise": true,
        }))?;
        if let Some(exc) = res.get("exceptionDetails") {
            let text = exc.get("text").and_then(|v| v.as_str()).unwrap_or("evaluation failed");
            bail!("eval exception: {text}");
        }
        Ok(res.get("result").and_then(|r| r.get("value")).cloned().unwrap_or(Value::Null))
    }

    fn eval_str(&mut self, expr: &str) -> Result<String> {
        Ok(self.eval(expr)?.as_str().unwrap_or("").to_string())
    }

    fn screenshot(&mut self, clip: Value) -> Result<Vec<u8>> {
        let res = self.call("Page.captureScreenshot", json!({ "format": "png", "clip": clip }))?;
        let data = res.get("data").and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Page.captureScreenshot returned no data"))?;
        base64_decode(data).ok_or_else(|| anyhow!("could not decode screenshot base64"))
    }
}

// ───────────────────────────── HTTP (localhost) ─────────────────────────────

/// Minimal HTTP/1.1 GET to the local DevTools endpoint. Returns the body on 200, else None.
fn http_get(port: u16, path: &str, timeout: Duration) -> Option<String> {
    let stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream.set_read_timeout(Some(timeout)).ok()?;
    stream.set_write_timeout(Some(timeout)).ok()?;
    let mut w = stream.try_clone().ok()?;
    w.write_all(format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n").as_bytes()).ok()?;
    let mut reader = BufReader::new(stream);
    let mut status = String::new();
    reader.read_line(&mut status).ok()?;
    if !status.contains(" 200") { return None; }
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 { break; }
        let t = line.trim_end();
        if t.is_empty() { break; }
        if let Some(v) = t.to_ascii_lowercase().strip_prefix("content-length:") {
            content_length = v.trim().parse().ok();
        }
    }
    let mut body = Vec::new();
    match content_length {
        Some(n) => { body.resize(n, 0); reader.read_exact(&mut body).ok()?; }
        None => { reader.read_to_end(&mut body).ok()?; }
    }
    Some(String::from_utf8_lossy(&body).into_owned())
}

fn cdp_up(port: u16) -> bool {
    http_get(port, "/json/version", Duration::from_millis(800)).is_some()
}

fn wait_for_cdp(port: u16, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if cdp_up(port) { return true; }
        thread::sleep(Duration::from_millis(500));
    }
    false
}

/// Wait for an inspectable page/webview target (NOT just the HTTP endpoint — on a cold
/// launch /json/version answers 200 before the renderer registers a target). Returns the
/// target's webSocketDebuggerUrl.
fn wait_for_target(port: u16, timeout: Duration) -> Option<String> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Some(body) = http_get(port, "/json/list", Duration::from_millis(800)) {
            if let Ok(Value::Array(targets)) = serde_json::from_str::<Value>(&body) {
                for t in targets {
                    let ty = t.get("type").and_then(|v| v.as_str()).unwrap_or("");
                    if ty == "page" || ty == "webview" {
                        if let Some(url) = t.get("webSocketDebuggerUrl").and_then(|v| v.as_str()) {
                            return Some(url.to_string());
                        }
                    }
                }
            }
        }
        thread::sleep(Duration::from_millis(500));
    }
    None
}

// ─────────────────────────── process orchestration ──────────────────────────

fn launch_fraytools(bin: &str, port: u16) -> Result<()> {
    Command::new(bin)
        .arg(format!("--remote-debugging-port={port}"))
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
        .spawn()
        .map(|_child| ()) // detached: we never wait/kill it — FrayTools stays running
        .map_err(|e| anyhow!("failed to launch FrayTools ({bin}): {e}"))
}

/// SIGKILL the FrayTools holding the debug port. SIGKILL (not a graceful quit) avoids the
/// "Are you sure you want to quit?" dialog that would otherwise hold the port open.
fn kill_fraytools(port: u16) {
    #[cfg(not(target_os = "windows"))]
    {
        let _ = Command::new("pkill")
            .args(["-9", "-f", &format!("remote-debugging-port={port}")])
            .stdout(Stdio::null()).stderr(Stdio::null()).status();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = port; // taskkill matches by image name, not cmdline
        let _ = Command::new("taskkill")
            .args(["/IM", "FrayTools.exe", "/F"])
            .stdout(Stdio::null()).stderr(Stdio::null()).status();
    }
}

fn default_fraytools() -> String {
    #[cfg(target_os = "windows")]
    {
        match std::env::var("LOCALAPPDATA") {
            Ok(la) => format!("{la}\\Programs\\FrayTools\\FrayTools.exe"),
            Err(_) => "FrayTools.exe".into(),
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        "/Applications/FrayTools.app/Contents/MacOS/FrayTools".to_string()
    }
}

fn default_fraymakers_root() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        PathBuf::from(std::env::var("HOME").unwrap_or_default())
            .join("Library/Application Support/Steam/steamapps/common/Fraymakers")
    }
    #[cfg(target_os = "windows")]
    {
        PathBuf::from("C:\\Program Files (x86)\\Steam\\steamapps\\common\\Fraymakers")
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        PathBuf::from(std::env::var("HOME").unwrap_or_default())
            .join(".steam/steam/steamapps/common/Fraymakers")
    }
}

// ─────────────────────────── publish-folder + .fra ──────────────────────────

/// Ensure the project's `publishFolders` includes `folder` (e.g. the Fraymakers
/// custom/<char> dir), stored RELATIVE to the project dir. FrayTools resolves
/// publishFolders[].path relative to the project dir, so an absolute path would re-resolve
/// into a bogus nested location. The converter rewrites publishFolders to just ./build, so
/// this runs on every export.
fn ensure_publish_folder(project_file: &Path, project_dir: &Path, folder: &Path) {
    let mut proj: Value = match std::fs::read_to_string(project_file).ok()
        .and_then(|s| serde_json::from_str(&s).ok())
    {
        Some(v) => v,
        None => { eprintln!("WARN: could not read project to set publishFolders"); return; }
    };
    let _ = std::fs::create_dir_all(folder);
    let target = std::fs::canonicalize(folder).unwrap_or_else(|_| folder.to_path_buf());
    let rel = pathdiff_relative(project_dir, folder);

    let arr = proj.get_mut("publishFolders").and_then(|v| v.as_array_mut());
    let mut folders = match arr {
        Some(a) => a.clone(),
        None => Vec::new(),
    };
    // existing entry pointing at the same resolved dir?
    for f in folders.iter_mut() {
        if let Some(p) = f.get("path").and_then(|v| v.as_str()) {
            let resolved = std::fs::canonicalize(project_dir.join(p)).unwrap_or_else(|_| project_dir.join(p));
            if resolved == target {
                if p == rel { return; } // already correct
                f["path"] = json!(rel); // self-heal a stale absolute/wrong entry
                proj["publishFolders"] = Value::Array(folders);
                let _ = std::fs::write(project_file, serde_json::to_string_pretty(&proj).unwrap_or_default());
                eprintln!("normalized Fraymakers publish folder to relative: {rel}");
                return;
            }
        }
    }
    folders.push(json!({ "id": "fraymakers0", "path": rel }));
    proj["publishFolders"] = Value::Array(folders);
    let _ = std::fs::write(project_file, serde_json::to_string_pretty(&proj).unwrap_or_default());
    eprintln!("added Fraymakers publish folder: {rel}");
}

/// Path of `to` relative to `from` (both treated as dirs), using `..` as needed. Falls back
/// to the absolute path string if they share no common base.
fn pathdiff_relative(from: &Path, to: &Path) -> String {
    let from = std::fs::canonicalize(from).unwrap_or_else(|_| from.to_path_buf());
    let to = std::fs::canonicalize(to).unwrap_or_else(|_| to.to_path_buf());
    let fc: Vec<_> = from.components().collect();
    let tc: Vec<_> = to.components().collect();
    let mut i = 0;
    while i < fc.len() && i < tc.len() && fc[i] == tc[i] { i += 1; }
    let mut rel = PathBuf::new();
    for _ in i..fc.len() { rel.push(".."); }
    for c in &tc[i..] { rel.push(c.as_os_str()); }
    let s = rel.to_string_lossy().to_string();
    if s.is_empty() { ".".into() } else { s }
}

/// Newest .fra under `dir` (recursive), as (path, mtime-seconds-f64). None if none found.
fn newest_fra(dir: &Path) -> Option<(PathBuf, f64)> {
    fn mtime(p: &Path) -> f64 {
        std::fs::metadata(p).and_then(|m| m.modified()).ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0)
    }
    fn scan(d: &Path, best: &mut Option<(PathBuf, f64)>) {
        let entries = match std::fs::read_dir(d) { Ok(e) => e, Err(_) => return };
        for e in entries.flatten() {
            let full = e.path();
            if full.is_dir() {
                scan(&full, best);
            } else if full.extension().and_then(|x| x.to_str()).map(|x| x.eq_ignore_ascii_case("fra")).unwrap_or(false) {
                let m = mtime(&full);
                if best.as_ref().map(|(_, bm)| m > *bm).unwrap_or(true) {
                    *best = Some((full, m));
                }
            }
        }
    }
    let _ = SystemTime::now(); // keep the import in use on all platforms
    let mut best = None;
    scan(dir, &mut best);
    best
}

// ──────────────────────── entity JSON box extraction ────────────────────────
// Pure filesystem-derived geometry (mirrors the Node harness + fraytools_transform.rs).

struct EntityCtx {
    data: Value,
    sym_by_id: HashMap<String, Value>,
    layer_by_id: HashMap<String, Value>,
    kf_by_id: HashMap<String, Value>,
}

impl EntityCtx {
    fn parse(raw: &str) -> Result<EntityCtx> {
        let data: Value = serde_json::from_str(raw)?;
        let index = |key: &str| -> HashMap<String, Value> {
            let mut m = HashMap::new();
            if let Some(arr) = data.get(key).and_then(|v| v.as_array()) {
                for item in arr {
                    m.insert(id_key(item.get("$id").unwrap_or(&Value::Null)), item.clone());
                }
            }
            m
        };
        let sym_by_id = index("symbols");
        let layer_by_id = index("layers");
        let kf_by_id = index("keyframes");
        Ok(EntityCtx { data, sym_by_id, layer_by_id, kf_by_id })
    }

    fn animation_names(&self) -> Vec<String> {
        self.data.get("animations").and_then(|v| v.as_array()).map(|a| {
            a.iter().filter_map(|x| x.get("name").and_then(|n| n.as_str()).map(String::from)).collect()
        }).unwrap_or_default()
    }

    fn animation(&self, name: &str) -> Option<Value> {
        self.data.get("animations").and_then(|v| v.as_array()).and_then(|a| {
            a.iter().find(|x| x.get("name").and_then(|n| n.as_str()) == Some(name)).cloned()
        })
    }

    /// Total frames: max keyframe span across the box/body/image layers of `anim`.
    fn total_frames(&self, anim: &Value) -> i64 {
        let mut max = 0i64;
        for lid in anim.get("layers").and_then(|v| v.as_array()).into_iter().flatten() {
            let layer = match self.layer_by_id.get(&id_key(lid)) { Some(l) => l, None => continue };
            let ty = layer.get("type").and_then(|v| v.as_str()).unwrap_or("");
            if !["COLLISION_BOX", "COLLISION_BODY", "IMAGE"].contains(&ty) { continue; }
            let span: i64 = layer.get("keyframes").and_then(|v| v.as_array()).map(|kfs| {
                kfs.iter().map(|kfid| self.kf_by_id.get(&id_key(kfid))
                    .and_then(|kf| kf.get("length")).and_then(|l| l.as_i64()).unwrap_or(0)).sum()
            }).unwrap_or(0);
            if span > max { max = span; }
        }
        max
    }

    /// All active COLLISION_BOX / COLLISION_BODY descriptors at (animName, 0-based frameN),
    /// each with a computed rendered_anchor.
    fn boxes_at_frame(&self, anim_name: &str, frame_n: i64) -> Vec<Value> {
        let mut out = Vec::new();
        let anim = match self.animation(anim_name) { Some(a) => a, None => return out };
        for lid in anim.get("layers").and_then(|v| v.as_array()).into_iter().flatten() {
            let layer = match self.layer_by_id.get(&id_key(lid)) { Some(l) => l, None => continue };
            let lty = layer.get("type").and_then(|v| v.as_str()).unwrap_or("");
            if lty != "COLLISION_BOX" && lty != "COLLISION_BODY" { continue; }

            let mut accum = 0i64;
            for kfid in layer.get("keyframes").and_then(|v| v.as_array()).into_iter().flatten() {
                let kf = match self.kf_by_id.get(&id_key(kfid)) { Some(k) => k, None => continue };
                let klen = kf.get("length").and_then(|l| l.as_i64()).unwrap_or(0);
                if frame_n >= accum && frame_n < accum + klen {
                    if let Some(sym_ref) = kf.get("symbol") {
                        if let Some(sym) = self.sym_by_id.get(&id_key(sym_ref)) {
                            let sty = sym.get("type").and_then(|v| v.as_str()).unwrap_or("");
                            if sty == "COLLISION_BOX" || sty == "COLLISION_BODY" {
                                out.push(box_descriptor(layer, sym));
                            }
                        }
                    }
                    break; // found the keyframe for this layer
                }
                accum += klen;
            }
        }
        out
    }
}

fn box_descriptor(layer: &Value, sym: &Value) -> Value {
    let x = numf(sym, "x", 0.0);
    let y = numf(sym, "y", 0.0);
    let width = numf(sym, "scaleX", 0.0);
    let height = numf(sym, "scaleY", 0.0);
    let rot = numf(sym, "rotation", 0.0);
    let piv_x = sym.get("pivotX").and_then(|v| v.as_f64()).unwrap_or(width / 2.0);
    let piv_y = sym.get("pivotY").and_then(|v| v.as_f64()).unwrap_or(height / 2.0);
    let color = color_int(sym.get("color"));
    let alpha = numf(sym, "alpha", 1.0);
    let (ax, ay) = collision_box_anchor(x, y, piv_x, piv_y, rot);
    json!({
        "layer_name":  layer.get("name").cloned().unwrap_or(Value::Null),
        "layer_type":  layer.get("type").cloned().unwrap_or(Value::Null),
        "symbol_type": sym.get("type").cloned().unwrap_or(Value::Null),
        "fm_box_type": fm_box_type(color),
        "x": x, "y": y, "width": width, "height": height,
        "rotation": rot,
        "pivot_x": piv_x, "pivot_y": piv_y,
        "color": color, "alpha": alpha,
        "rendered_anchor": { "x": ax, "y": ay },
    })
}

// FrayTools rendered-anchor transform (mirrors fraytools_transform.rs):
fn collision_box_anchor(x: f64, y: f64, pivot_x: f64, pivot_y: f64, rotation_deg: f64) -> (f64, f64) {
    let s = (x, -y); // §1 negate Y into render space
    let c = (pivot_x, -pivot_y);
    let p = abs_pivot(s, c, -rotation_deg); // §2 rotate
    (p.0, -p.1) // §3 negate Y back to stored space
}

fn abs_pivot(pos: (f64, f64), pivot: (f64, f64), angle_deg: f64) -> (f64, f64) {
    if angle_deg % 360.0 == 0.0 {
        return (pos.0 + pivot.0, pos.1 + pivot.1);
    }
    let mag = (pivot.0.powi(2) + pivot.1.powi(2)).sqrt();
    let ang = pivot.1.atan2(pivot.0);
    let theta = ang + angle_deg * std::f64::consts::PI / 180.0;
    (pos.0 + mag * theta.cos(), pos.1 + mag * theta.sin())
}

fn fm_box_type(color: i64) -> String {
    match color {
        0xff0000 => "HIT_BOX".into(),
        0xfcba03 => "HURT_BOX".into(),
        0xff00ff => "GRAB_BOX".into(),
        0xffff00 => "ITEM_BOX".into(),
        0x48f748 => "REFLECT_BOX".into(),
        0x42ecff => "COUNTER_BOX".into(),
        0xbababa => "LEDGE_GRAB_BOX".into(),
        0x9999ff => "GRAB_HOLD_POINT".into(),
        _ => format!("UNKNOWN(0x{color:x})"),
    }
}

// ──────────────────────────────── small helpers ─────────────────────────────

fn arg<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    let flag = format!("--{name}");
    args.iter().position(|a| a == &flag).and_then(|i| args.get(i + 1)).map(|s| s.as_str())
}
fn arg_u16(args: &[String], name: &str, def: u16) -> u16 {
    arg(args, name).and_then(|s| s.parse().ok()).unwrap_or(def)
}
fn arg_u64(args: &[String], name: &str, def: u64) -> u64 {
    arg(args, name).and_then(|s| s.parse().ok()).unwrap_or(def)
}

/// JSON-encode a string into a JS string literal (for safe interpolation into eval'd JS).
fn js_string(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

fn numf(v: &Value, key: &str, def: f64) -> f64 {
    v.get(key).and_then(|x| x.as_f64()).unwrap_or(def)
}

/// entity_gen stores box color as either an int or a "0xff0000" string; normalize to int.
fn color_int(v: Option<&Value>) -> i64 {
    match v {
        Some(Value::Number(n)) => n.as_i64().unwrap_or(0),
        Some(Value::String(s)) => {
            let s = s.trim();
            if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
                i64::from_str_radix(hex, 16).unwrap_or(0)
            } else {
                s.parse().unwrap_or(0)
            }
        }
        _ => 0,
    }
}

/// A FrayTools `$id` may be a number or string; use its canonical string form as a map key.
fn id_key(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let mut buf = 0u32;
    let mut bits = 0u32;
    for &c in s.as_bytes() {
        if c == b'=' || c == b'\n' || c == b'\r' || c == b' ' || c == b'\t' { continue; }
        let v = val(c)? as u32;
        buf = (buf << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Some(out)
}

// ───────────────────────── in-page JS (verbatim) ────────────────────────────
// These run inside the FrayTools renderer via Runtime.evaluate and are kept byte-for-byte
// as in the original Node harness. Our own code — no FrayTools source.

/// export: stash the controller (getLibraryDirectory + openProject + publish) on window.__ctrl.
const STASH_CTRL_JS: &str = r#"(()=>{
  let host=null; for(const el of document.querySelectorAll('*')){ if(el._reactRootContainer){host=el;break;} }
  if(!host) return 'no react root';
  const fiber = host._reactRootContainer._internalRoot.current;
  let ctrl=null;
  (function w(f,d){ if(!f||d>80||ctrl)return; const sn=f.stateNode;
    if(sn&&typeof sn.getLibraryDirectory==='function'&&typeof sn.openProject==='function'&&typeof sn.publish==='function') ctrl=sn;
    w(f.child,d+1); w(f.sibling,d); })(fiber,0);
  window.__ctrl = ctrl;
  return ctrl ? 'ok' : 'no controller';
})()"#;

/// export: close any stale publish dialog so a fresh one can be opened + clicked.
const CLOSE_DIALOG_JS: &str = r#"(()=>{ try {
  const c = window.__ctrl;
  const dlgOpen = (c.state && c.state.publishDialogVisible) || !!document.querySelector('.PublishSettingsDialog');
  if (dlgOpen && typeof c.onPublishDialogClose === 'function') c.onPublishDialogClose();
  return 'closed:' + (dlgOpen ? 'yes' : 'no');
} catch(e){ return 'ERR '+e.message; } })()"#;

/// export: click "Publish All" (every output folder), falling back to force publish().
const PUBLISH_ALL_JS: &str = r#"(()=>{ try {
  const b = Array.from(document.querySelectorAll('button')).find(x => /publish all/i.test((x.textContent||'').trim()));
  if (b) {
    const fk = Object.keys(b).find(k => k.startsWith('__reactInternalInstance'));
    const oc = fk && b[fk].memoizedProps && b[fk].memoizedProps.onClick;
    if (oc) { oc({ type:'click', target:b, currentTarget:b, bubbles:true }); return 'publish-all'; }
    b.click(); return 'publish-all:native';
  }
  window.__ctrl.publish();
  return 'force-publish-fallback';
} catch(e){ return 'ERR '+e.message; } })()"#;

/// harness/render: stash all component instances by class name on window.__ft.fc.
const STASH_COMPONENTS_JS: &str = r#"(()=>{
  let host = null;
  for (const el of document.querySelectorAll('*')) { if (el._reactRootContainer) { host = el; break; } }
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
})()"#;

/// harness/render: identify controller + tree from window.__ft.fc, stash on window.__ctrl/__tree.
const IDENTIFY_ROLES_JS: &str = r#"(()=>{
  let ctrl = null, tree = null;
  for (const [k, c] of Object.entries(window.__ft.fc)) {
    if (!ctrl && typeof c.getLibraryDirectory === 'function' && typeof c.openProject === 'function') ctrl = k;
    if (!tree && c.props && typeof c.props.onFileDoubleClicked === 'function') tree = k;
  }
  window.__ctrl = window.__ft.fc[ctrl];
  window.__tree = window.__ft.fc[tree];
  return JSON.stringify({ ctrl, tree });
})()"#;

/// harness: locate the Redux store (getState + dispatch) and stash it on window.__store.
const LOCATE_STORE_JS: &str = r#"(()=>{
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
})()"#;

#[cfg(test)]
mod tests {
    use super::*;

    // A minimal entity: one animation "atk" with a single COLLISION_BOX keyframe.
    fn synthetic() -> String {
        json!({
            "symbols": [{
                "$id": "s1", "type": "COLLISION_BOX",
                "x": 10.0, "y": 20.0, "scaleX": 30.0, "scaleY": 40.0,
                "rotation": 0.0, "color": "0xff0000", "alpha": 0.5
            }],
            "layers": [{ "$id": "l1", "name": "hitbox0", "type": "COLLISION_BOX", "keyframes": ["k1"] }],
            "keyframes": [{ "$id": "k1", "length": 1, "symbol": "s1" }],
            "animations": [{ "$id": "a1", "name": "atk", "layers": ["l1"] }],
        }).to_string()
    }

    #[test]
    fn extracts_box_with_anchor() {
        let ctx = EntityCtx::parse(&synthetic()).unwrap();
        assert_eq!(ctx.animation_names(), vec!["atk"]);
        let anim = ctx.animation("atk").unwrap();
        assert_eq!(ctx.total_frames(&anim), 1);

        let boxes = ctx.boxes_at_frame("atk", 0);
        assert_eq!(boxes.len(), 1);
        let b = &boxes[0];
        assert_eq!(b["layer_name"], "hitbox0");
        assert_eq!(b["fm_box_type"], "HIT_BOX"); // 0xff0000
        assert_eq!(b["width"], 30.0);
        assert_eq!(b["height"], 40.0);
        assert_eq!(b["color"], 0xff0000);
        // pivot defaults to width/2, height/2 when absent
        assert_eq!(b["pivot_x"], 15.0);
        assert_eq!(b["pivot_y"], 20.0);
        // rotation 0: anchor = (x+pivotX, y+pivotY) = (25, 40)
        assert_eq!(b["rendered_anchor"]["x"], 25.0);
        assert_eq!(b["rendered_anchor"]["y"], 40.0);
    }

    #[test]
    fn rotated_anchor_matches_transform() {
        // pivot (10,0) rotated by 90° about origin -> (~0, 10) in stored space.
        let (x, y) = collision_box_anchor(0.0, 0.0, 10.0, 0.0, 90.0);
        assert!(x.abs() < 1e-9, "x={x}");
        assert!((y - 10.0).abs() < 1e-9, "y={y}");
    }

    #[test]
    fn color_and_type_mapping() {
        assert_eq!(color_int(Some(&json!("0xfcba03"))), 0xfcba03);
        assert_eq!(color_int(Some(&json!(0xff00ff))), 0xff00ff);
        assert_eq!(fm_box_type(0xfcba03), "HURT_BOX");
        assert_eq!(fm_box_type(0x123456), "UNKNOWN(0x123456)");
    }

    #[test]
    fn parses_a_real_entity_without_panicking() {
        // ../../ from tools/peptide -> repo root
        let path = format!("{}/../../characters/pacman/library/entities/pacman_tauntsprite24.entity",
            env!("CARGO_MANIFEST_DIR"));
        let raw = match std::fs::read_to_string(&path) {
            Ok(r) => r,
            Err(_) => return, // entity not present in this checkout — skip
        };
        let ctx = EntityCtx::parse(&raw).expect("real entity should parse");
        let anims = ctx.animation_names();
        assert!(!anims.is_empty(), "entity should have animations");
        // exercise extraction on the first animation, frame 0 — must not panic
        let _ = ctx.boxes_at_frame(&anims[0], 0);
    }
}
