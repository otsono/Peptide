//! gui — the graphical chat console (the default `peptide` mode): a native window using
//! the system webview (wry) — WKWebView on macOS, WebView2/Edge on Windows, WebKitGTK on
//! Linux. The whole UI is HTML/CSS/JS (src/peptide_ui.html, Claude dark theme); this file
//! is the glue: boot the engine, stream replies into the page, send the page's commands to
//! the socket, and drive the staged reconnect/boot flow when the connection is lost.

use std::io::{BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use tao::dpi::LogicalSize;
use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::window::WindowBuilder;
use wry::http::Request;
use wry::WebViewBuilder;

use crate::interpreter::{translate, Translated};

/// Events pushed into the loop from worker threads, forwarded to the page.
enum Ev {
    Line(String), // an engine reply line -> onLine(...)
    Js(String),   // arbitrary JS to evaluate (status, modal, reconnect/boot callbacks)
}

type SharedWriter = Arc<Mutex<Option<TcpStream>>>;
type SharedCleanup = Arc<Mutex<Option<crate::ui::Cleanup>>>;
type SharedConn = Arc<Mutex<(u16, String)>>; // the live session's port + auth token

// IPC control messages (everything else is a user command -> translate).
const RECONNECT: &str = "@@reconnect";     // TCP back into the current session
const BOOT_QUICK: &str = "@@boot:quick";   // launch a fresh engine, auto-spawn for testing
const BOOT_REGULAR: &str = "@@boot:regular"; // launch a fresh engine, no auto-spawn
// Screen-router verbs (Home / Setup / Converter / FrayTools Hook). Heavy work
// (conversion, FrayTools CDP, file dialogs) runs on a worker thread and posts
// results back via Ev::Js — never block the tao event loop.
const PICK_DIR_PREFIX: &str = "@@pick:dir:";       // @@pick:dir:<replyFn>
const PICK_FILE_PREFIX: &str = "@@pick:file:";     // @@pick:file:<replyFn>[:<ext>]
const SETUP_SAVE_PREFIX: &str = "@@setup:save:";   // @@setup:save:<json>
const SETUP_RESET: &str = "@@setup:reset";         // clear config -> reopen first-run wizard
const CONVERT_PREFIX: &str = "@@convert:start:";   // @@convert:start:<json>
const PUBLISH_PREFIX: &str = "@@publish:add:";      // @@publish:add:<json {char, output}>
const PROJECTS_LIST: &str = "@@projects:list";      // enumerate .fraytools projects (launch modal)
const FRAY_PREFIX: &str = "@@fray:";                // @@fray:export|render|harness:<json>

pub fn launch() -> std::io::Result<()> {
    let event_loop = EventLoopBuilder::<Ev>::with_user_event().build();
    let window = WindowBuilder::new()
        .with_title("Peptide")
        .with_inner_size(LogicalSize::new(940.0, 720.0))
        .with_min_inner_size(LogicalSize::new(520.0, 400.0))
        .build(&event_loop)
        .map_err(|e| io(&e.to_string()))?;

    // No eager engine boot: the window opens to Setup (first run) or Home, and the
    // engine is launched lazily only when the user picks "Launch Peptide". The
    // shared engine handles start empty and are filled by boot_new/reconnect.
    let writer: SharedWriter = Arc::new(Mutex::new(None));
    let cleanup: SharedCleanup = Arc::new(Mutex::new(None));
    let conn: SharedConn = Arc::new(Mutex::new((0, String::new())));

    // matchStatus feed: poll the engine ~5×/s while connected. The reply (E:MATCHSTATUS:…)
    // is routed to the status widgets by spawn_reader, NOT the chat. Host-driven so the
    // engine needs no per-frame bytecode (commands.hsx::matchStatus does the gathering).
    {
        let poll_writer = writer.clone();
        thread::spawn(move || loop {
            thread::sleep(Duration::from_millis(200));
            if let Ok(mut g) = poll_writer.lock() {
                if let Some(s) = g.as_mut() {
                    if s.write_all(b"e matchStatus()\n").and_then(|_| s.flush()).is_err() {
                        // socket dropped — leave it; boot_new/reconnect installs a fresh one
                    }
                }
            }
        });
    }

    let char_name = crate::config::Config::load().char_name();
    let init = format!("window.__CHAR__={};", js_str(&char_name));

    let ipc_writer = writer.clone();
    let ipc_cleanup = cleanup.clone();
    let ipc_conn = conn.clone();
    let ipc_proxy = event_loop.create_proxy();
    let ipc_char = char_name.clone();
    // Read the UI from disk at launch (no compiled-in copy); wry's with_html
    // borrows a &str, so the String must outlive the builder.
    let ui_html = crate::read_asset("peptide_ui.html");
    let webview = WebViewBuilder::new()
        .with_html(&ui_html)
        .with_initialization_script(&init)
        .with_ipc_handler(move |req: Request<String>| {
            let body = req.body().to_string();
            let (w, cl, cn, px, ch) = (ipc_writer.clone(), ipc_cleanup.clone(), ipc_conn.clone(),
                                       ipc_proxy.clone(), ipc_char.clone());
            let b = body.as_str();
            if b == RECONNECT {
                thread::spawn(move || reconnect_existing(w, cn, px, ch));
            } else if b.starts_with(BOOT_QUICK) {
                // @@boot:quick[:<char>] — the trailing char (the picked project) is
                // baked as the launch default; absent -> fall back to the config char.
                let chosen = boot_char(b, BOOT_QUICK).unwrap_or(ch);
                thread::spawn(move || boot_new(w, cl, cn, px, chosen, true));
            } else if b.starts_with(BOOT_REGULAR) {
                let chosen = boot_char(b, BOOT_REGULAR).unwrap_or(ch);
                thread::spawn(move || boot_new(w, cl, cn, px, chosen, false));
            } else if b.starts_with("@@") {
                handle_screen_verb(b, &ipc_proxy);
            } else {
                handle_command(&body, &ipc_writer, &ipc_proxy);
            }
        })
        .build(&window)
        .map_err(|e| io(&e.to_string()))?;

    event_loop.run(move |event, _t, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::NewEvents(StartCause::Init) => {
                // Route the page to Setup or Home based on the persisted config.
                let _ = webview.evaluate_script(&format!(
                    "window.bootRoute && bootRoute({})", config_json()));
            }
            Event::UserEvent(Ev::Line(line)) => {
                let _ = webview.evaluate_script(&format!("window.onLine && onLine({})", js_str(&line)));
            }
            Event::UserEvent(Ev::Js(js)) => {
                let _ = webview.evaluate_script(&js);
            }
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                if let Some(mut c) = cleanup.lock().ok().and_then(|mut g| g.take()) {
                    c.dispose();
                }
                *control_flow = ControlFlow::Exit;
            }
            _ => {}
        }
    });
}

fn io(msg: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Other, msg.to_string())
}

/// Parse the optional `:<char>` suffix off a boot verb (`@@boot:quick:mario` ->
/// `Some("mario")`, `@@boot:quick` -> `None`).
fn boot_char(verb: &str, prefix: &str) -> Option<String> {
    verb.strip_prefix(prefix)
        .and_then(|r| r.strip_prefix(':'))
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
}

/// A small JSON blob describing the current config + setup state, passed to the
/// page's `bootRoute` so it can open the Setup wizard (first run / incomplete) or
/// Home. Carries autodetect results so the wizard can pre-fill + badge the paths.
fn config_json() -> String {
    let cfg = crate::config::Config::load();
    let s = |v: &str| js_str(v);
    let frayroot = cfg.fraymakers_root().map(|p| p.display().to_string()).unwrap_or_default();
    let frayexe = cfg.fraytools_exe().map(|p| p.display().to_string()).unwrap_or_default();
    // Autodetect for the wizard: did we find each tool on disk? And the path to
    // pre-fill FrayTools with when the user hasn't set one yet.
    let fraymakers_detected = crate::platform::fraymakers_root().is_some();
    let fraytools_detected = crate::platform::detected_fraytools_path().is_some();
    let fraytools_val = if !cfg.fraytools_path.is_empty() {
        cfg.fraytools_path.clone()
    } else {
        crate::platform::detected_fraytools_path().map(|p| p.display().to_string()).unwrap_or_default()
    };
    format!(
        "{{\"configured\":{},\"setupComplete\":{},\"currentChar\":{},\"stage\":{},\"assist\":{},\
          \"fraymakersRoot\":{},\
          \"fraymakersDetected\":{},\"fraytoolsPath\":{},\"fraytoolsDetected\":{},\
          \"fraytoolsExe\":{},\"outputDir\":{},\"miscSsf\":{}}}",
        cfg.configured,
        cfg.setup_complete(),
        s(&cfg.char_name()),
        s(&cfg.stage()),
        s(&cfg.assist()),
        s(&frayroot),
        fraymakers_detected,
        s(&fraytools_val),
        fraytools_detected,
        s(&frayexe),
        s(&cfg.output_dir().display().to_string()),
        s(&cfg.misc_ssf),
    )
}

/// Dispatch a screen-router control message. Everything that blocks (file
/// dialogs, conversion, FrayTools CDP) runs on a worker thread that posts results
/// back via Ev::Js — the tao event loop must never block.
fn handle_screen_verb(verb: &str, proxy: &EventLoopProxy<Ev>) {
    if let Some(rest) = verb.strip_prefix(PICK_DIR_PREFIX) {
        let (reply, px) = (rest.to_string(), proxy.clone());
        thread::spawn(move || pick_path(true, &reply, "", &px));
    } else if let Some(rest) = verb.strip_prefix(PICK_FILE_PREFIX) {
        // <replyFn>[:<ext>]
        let mut it = rest.splitn(2, ':');
        let reply = it.next().unwrap_or("").to_string();
        let ext = it.next().unwrap_or("").to_string();
        let px = proxy.clone();
        thread::spawn(move || pick_path(false, &reply, &ext, &px));
    } else if let Some(json) = verb.strip_prefix(SETUP_SAVE_PREFIX) {
        let (json, px) = (json.to_string(), proxy.clone());
        thread::spawn(move || save_setup(&json, &px));
    } else if verb == SETUP_RESET {
        let px = proxy.clone();
        thread::spawn(move || reset_setup(&px));
    } else if verb == PROJECTS_LIST {
        let px = proxy.clone();
        thread::spawn(move || list_projects(&px));
    } else if let Some(json) = verb.strip_prefix(CONVERT_PREFIX) {
        let (json, px) = (json.to_string(), proxy.clone());
        thread::spawn(move || run_convert(&json, &px));
    } else if let Some(json) = verb.strip_prefix(PUBLISH_PREFIX) {
        let (json, px) = (json.to_string(), proxy.clone());
        thread::spawn(move || add_publish_folder(&json, &px));
    } else if let Some(rest) = verb.strip_prefix(FRAY_PREFIX) {
        // export:<json> | render:<json> | harness:<json>
        let (rest, px) = (rest.to_string(), proxy.clone());
        thread::spawn(move || run_fraytools(&rest, &px));
    }
    // Unknown @@verbs are ignored (forward-compat with the page).
}

/// Minimal JSON-string-field extractor for the small, flat objects the page
/// sends (`{"k":"v",...}`). Avoids pulling serde_json's Value into the hot path
/// and is enough for our known keys. Returns the unescaped string value for `key`.
fn json_str_field(json: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let i = json.find(&needle)? + needle.len();
    let rest = &json[i..];
    let colon = rest.find(':')?;
    let after = rest[colon + 1..].trim_start();
    let mut chars = after.chars();
    if chars.next()? != '"' { return None; }
    let mut out = String::new();
    let mut esc = false;
    for c in chars {
        if esc { out.push(c); esc = false; continue; }
        match c {
            '\\' => esc = true,
            '"' => return Some(out),
            _ => out.push(c),
        }
    }
    None
}

/// The configured FrayTools project folder (`output_dir`) as an absolute, existing
/// directory — for defaulting file pickers there. `None` if it doesn't exist.
fn resolved_project_dir() -> Option<std::path::PathBuf> {
    let d = crate::config::Config::load().output_dir();
    let abs = if d.is_absolute() { d } else { std::env::current_dir().ok()?.join(d) };
    abs.is_dir().then_some(abs)
}

/// Open a native folder/file picker (rfd) and call back the named JS function with
/// the chosen path (or empty string if cancelled).
fn pick_path(dir: bool, reply_fn: &str, ext: &str, proxy: &EventLoopProxy<Ev>) {
    let chosen = if dir {
        rfd::FileDialog::new().pick_folder()
    } else {
        let mut d = rfd::FileDialog::new();
        if !ext.is_empty() { d = d.add_filter(ext, &[ext]); }
        // A `.fraytools` pick (launch / FrayTools Hook) opens in the configured
        // project folder by default, since that's where projects live.
        if ext == "fraytools" {
            if let Some(start) = resolved_project_dir() {
                d = d.set_directory(start);
            }
        }
        d.pick_file()
    };
    let path = chosen.map(|p| p.display().to_string()).unwrap_or_default();
    // Guard the reply-fn name to a safe identifier so we never inject arbitrary JS.
    if reply_fn.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !reply_fn.is_empty() {
        let _ = proxy.send_event(Ev::Js(format!(
            "window.{f} && {f}({p})", f = reply_fn, p = js_str(&path))));
    }
}

/// Persist the setup wizard form (`{fraymakersRoot, fraytoolsPath, outputDir,
/// miscSsf}`) and report back whether setup is now complete. Marks the config as
/// `configured` so the first-run wizard doesn't reappear. The character is no
/// longer a setup field — it's chosen at launch by picking a `.fraytools` project.
fn save_setup(json: &str, proxy: &EventLoopProxy<Ev>) {
    let mut cfg = crate::config::Config::load();
    if let Some(v) = json_str_field(json, "fraymakersRoot") { cfg.fraymakers_root = v; }
    if let Some(v) = json_str_field(json, "fraytoolsPath")  { cfg.fraytools_path = v; }
    if let Some(v) = json_str_field(json, "outputDir")      { cfg.output_dir = v; }
    if let Some(v) = json_str_field(json, "miscSsf")        { cfg.misc_ssf = v; }
    cfg.configured = true;
    cfg.save();
    let _ = proxy.send_event(Ev::Js(format!(
        "window.onSetupSaved && onSetupSaved({})", config_json())));
}

/// Clear the persisted config and re-route the page to the first-run wizard with
/// freshly autodetected defaults (the Setup screen's "Reset to defaults" button).
fn reset_setup(proxy: &EventLoopProxy<Ev>) {
    crate::config::Config::reset();
    let _ = proxy.send_event(Ev::Js(format!(
        "window.bootRoute && bootRoute({})", config_json())));
}

/// Enumerate the converted `.fraytools` projects in the output dir for the launch
/// modal: each `<output>/<char>/<char>.fraytools` (and any top-level `.fraytools`).
/// Posts `onProjects([{char, path}, …])` back to the page.
fn list_projects(proxy: &EventLoopProxy<Ev>) {
    let dir = crate::config::Config::load().output_dir();
    let mut items: Vec<(String, String)> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for entry in rd.flatten() {
            let p = entry.path();
            let proj = if p.is_dir() {
                crate::platform::find_project_file(&p)
            } else if p.extension().map(|x| x == "fraytools").unwrap_or(false) {
                Some(p.clone())
            } else {
                None
            };
            if let Some(proj) = proj {
                if let Some(stem) = proj.file_stem().map(|s| s.to_string_lossy().into_owned()) {
                    // Show the folder that holds the .fraytools, relative as configured
                    // (e.g. "./characters/mario"), not the full path to the file.
                    let folder = proj.parent().unwrap_or(&proj).display().to_string();
                    if !stem.is_empty() { items.push((stem, folder)); }
                }
            }
        }
    }
    items.sort();
    items.dedup();
    let mut json = String::from("[");
    for (i, (c, path)) in items.iter().enumerate() {
        if i > 0 { json.push(','); }
        json.push_str(&format!("{{\"char\":{},\"path\":{}}}", js_str(c), js_str(path)));
    }
    json.push(']');
    let _ = proxy.send_event(Ev::Js(format!("window.onProjects && onProjects({json})")));
}

/// Run an in-process conversion (`{input, output?, name?, miscSsf?}`) on this
/// worker thread, reporting progress + the result back to the convert screen.
fn run_convert(json: &str, proxy: &EventLoopProxy<Ev>) {
    use ssf2_converter::{run_conversion, ConvertOptions};
    let Some(input) = json_str_field(json, "input").filter(|s| !s.is_empty()) else {
        let _ = proxy.send_event(Ev::Js(
            "window.onConvertError && onConvertError(\"no input file selected\")".into()));
        return;
    };
    let cfg = crate::config::Config::load();
    let mut opts = ConvertOptions::new(&input);
    opts.output = json_str_field(json, "output").filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from).unwrap_or_else(|| cfg.output_dir());
    opts.name = json_str_field(json, "name").filter(|s| !s.is_empty());
    opts.misc_ssf = json_str_field(json, "miscSsf").filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| if cfg.misc_ssf.is_empty() { None } else { Some(std::path::PathBuf::from(&cfg.misc_ssf)) });

    let _ = proxy.send_event(Ev::Js(format!(
        "window.onConvertProgress && onConvertProgress({})",
        js_str(&format!("Converting {}…", input)))));

    match run_conversion(opts) {
        Ok(summary) => {
            // Build a JSON result: project dir, chars, .fraytools files, warnings.
            let files: Vec<String> = summary.fraytools_files.iter()
                .map(|p| p.display().to_string()).collect();
            let result = format!(
                "{{\"projectDir\":{},\"characters\":{},\"files\":{},\"warnings\":{}}}",
                js_str(&summary.project_dir.display().to_string()),
                js_array(&summary.characters),
                js_array(&files),
                js_array(&summary.warnings),
            );
            let _ = proxy.send_event(Ev::Js(format!(
                "window.onConvertDone && onConvertDone({result})")));
        }
        Err(e) => {
            let _ = proxy.send_event(Ev::Js(format!(
                "window.onConvertError && onConvertError({})", js_str(&e.to_string()))));
        }
    }
}

/// Add the Fraymakers `custom/<Char>` folder to a converted character's
/// `.fraytools` publishFolders (`{char, output}`), reporting the result.
fn add_publish_folder(json: &str, proxy: &EventLoopProxy<Ev>) {
    let cfg = crate::config::Config::load();
    let char_name = json_str_field(json, "char").filter(|s| !s.is_empty())
        .unwrap_or_else(|| cfg.char_name());
    let output = json_str_field(json, "output").filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from).unwrap_or_else(|| cfg.output_dir());
    let char_dir = output.join(&char_name);
    match crate::platform::ensure_fraymakers_publish_folder(&char_name, &char_dir) {
        Ok(rel) => {
            let _ = proxy.send_event(Ev::Js(format!(
                "window.onPublishResult && onPublishResult(true, {})",
                js_str(&format!("Added publish folder: {rel}")))));
        }
        Err(e) => {
            let _ = proxy.send_event(Ev::Js(format!(
                "window.onPublishResult && onPublishResult(false, {})", js_str(&e))));
        }
    }
}

/// Drive the FrayTools CDP harness for the Hook screen. `rest` is
/// `export:<json>` | `render:<json>` | `harness:<json>` where json carries the
/// flags (project / entity / animation / frame). Reuses the same fraytools::*
/// entry points the CLI uses, by assembling an argv.
fn run_fraytools(rest: &str, proxy: &EventLoopProxy<Ev>) {
    let mut it = rest.splitn(2, ':');
    let action = it.next().unwrap_or("");
    let json = it.next().unwrap_or("{}");

    // Build an argv: ["peptide", "<action>", "--project", P, "--entity", E, ...].
    let mut argv: Vec<String> = vec!["peptide".into(), action.into()];
    let mut push = |flag: &str, key: &str| {
        if let Some(v) = json_str_field(json, key) {
            if !v.is_empty() { argv.push(flag.into()); argv.push(v); }
        }
    };
    push("--project", "project");
    push("--entity", "entity");
    push("--animation", "animation");
    push("--frame", "frame");
    push("--out", "out");
    push("--fraytools", "fraytools");

    let _ = proxy.send_event(Ev::Js(format!(
        "window.onFrayProgress && onFrayProgress({})",
        js_str(&format!("Running FrayTools {action}…")))));

    let res = match action {
        "export" => crate::fraytools::export(&argv),
        "render" => crate::fraytools::render(&argv),
        "harness" => crate::fraytools::harness(&argv),
        _ => Err(anyhow::anyhow!("unknown FrayTools action {action:?}")),
    };
    match res {
        Ok(()) => {
            let _ = proxy.send_event(Ev::Js(format!(
                "window.onFrayResult && onFrayResult(true, {})",
                js_str(&format!("FrayTools {action} finished")))));
        }
        Err(e) => {
            let _ = proxy.send_event(Ev::Js(format!(
                "window.onFrayResult && onFrayResult(false, {})", js_str(&e.to_string()))));
        }
    }
}

/// JSON array of strings for evaluate_script payloads.
fn js_array(items: &[String]) -> String {
    let mut s = String::from("[");
    for (i, it) in items.iter().enumerate() {
        if i > 0 { s.push(','); }
        s.push_str(&js_str(it));
    }
    s.push(']');
    s
}

/// Try to TCP back into the running session: re-bind the last port + token and wait a few
/// seconds for the engine to dial in again (no relaunch). Succeeds only if a Fraymakers is
/// alive and reconnects; otherwise the page advances its reconnect flow.
fn reconnect_existing(writer: SharedWriter, conn: SharedConn, proxy: EventLoopProxy<Ev>, char_name: String) {
    let (port, token) = conn.lock().ok().map(|g| g.clone()).unwrap_or((0, String::new()));
    if port == 0 {
        let _ = proxy.send_event(Ev::Js("window.onReconnectFailed && onReconnectFailed(\"no_session\")".into()));
        return;
    }
    match crate::ui::reawait(port, &token, 4) {
        Some((reader, w)) => {
            if let Ok(mut g) = writer.lock() { *g = Some(w); }
            spawn_reader(reader, proxy.clone());
            // reconnecting to a live match -> don't auto-spawn (the character already exists)
            let _ = proxy.send_event(Ev::Js(format!(
                "window.onReconnected && onReconnected({}, {}, false)", port, js_str(&char_name))));
        }
        None => {
            let _ = proxy.send_event(Ev::Js("window.onReconnectFailed && onReconnectFailed(\"no_session\")".into()));
        }
    }
}

/// Launch a fresh patched engine and wait (bounded) for it to connect. `quick` auto-spawns
/// the test character on the page; regular boots clean for normal play. On failure the page
/// returns to the "Fraymakers doesn't seem to be open" prompt.
fn boot_new(writer: SharedWriter, cleanup: SharedCleanup, conn: SharedConn,
            proxy: EventLoopProxy<Ev>, char_name: String, quick: bool) {
    // tear down whatever was there (dead engine + temp files), then patch + launch anew.
    if let Some(mut c) = cleanup.lock().ok().and_then(|mut g| g.take()) {
        c.dispose();
    }
    if let Ok(mut g) = writer.lock() { *g = None; }

    // Stream the bytecode preflight progress into the boot modal as a real bar.
    let pp = proxy.clone();
    let on_progress = move |done: usize, total: usize, label: &str| {
        let _ = pp.send_event(Ev::Js(format!(
            "window.onPatchProgress && onPatchProgress({}, {}, {})",
            done, total, js_str(label))));
    };
    // Quick boot = HEADLESS fast-boot (skip Title, bake the picked custom char); the page
    // sends `spawn <char>` on READY. Regular boot = full (non-headless) Title/UGC boot that
    // is just a live TCP bridge with no auto-spawn. Both keep active TCP.
    let bake = if quick && !char_name.is_empty() { Some(char_name.as_str()) } else { None };
    match crate::ui::patch_and_launch_with_progress(Some(&on_progress), bake) {
        Ok((port, token, mut guard)) => match crate::ui::reawait(port, &token, 30) {
            Some((reader, w)) => {
                if let Ok(mut g) = writer.lock() { *g = Some(w); }
                if let Ok(mut g) = cleanup.lock() { *g = Some(guard); }
                if let Ok(mut g) = conn.lock() { *g = (port, token); }
                spawn_reader(reader, proxy.clone());
                let _ = proxy.send_event(Ev::Js(format!(
                    "window.onReconnected && onReconnected({}, {}, {})",
                    port, js_str(&char_name), if quick { "true" } else { "false" })));
            }
            None => {
                guard.dispose(); // kill the engine that never dialed in
                // It may have crashed during boot — show the crash modal if it left a log.
                thread::sleep(Duration::from_millis(450));
                match read_crash_log() {
                    Some(log) => { let _ = proxy.send_event(Ev::Js(crash_js(&log, &[]))); }
                    None => { let _ = proxy.send_event(Ev::Js(
                        "window.onBootFailed && onBootFailed(\"Fraymakers didn’t connect.\")".into())); }
                }
            }
        },
        Err(e) => {
            let _ = proxy.send_event(Ev::Js(format!(
                "window.onBootFailed && onBootFailed({})", js_str(&e.to_string()))));
        }
    }
}

/// Read the engine crash log (`<fraymakers>/error.log`) if the engine left one. The boot
/// path deletes it before launch, so its presence after a drop means a real crash. Trimmed
/// to a modal-friendly tail. `None` = no crash log (a clean/transient disconnect).
fn read_crash_log() -> Option<String> {
    let root = crate::config::Config::load().fraymakers_root()?;
    let text = std::fs::read_to_string(root.join("error.log")).ok()?;
    let t = text.trim();
    if t.is_empty() { return None; }
    Some(if t.len() > 1600 { format!("…{}", &t[t.len() - 1600..]) } else { t.to_string() })
}

/// The engine's connection ended. If it left a crash log, surface the crash modal with it;
/// otherwise treat it as a transient drop and let the page run its reconnect flow.
fn engine_gone(proxy: &EventLoopProxy<Ev>, resdiag: &[String]) {
    // give the engine a moment to flush error.log after the socket closes
    thread::sleep(Duration::from_millis(450));
    match read_crash_log() {
        Some(log) => { let _ = proxy.send_event(Ev::Js(crash_js(&log, resdiag))); }
        None => { let _ = proxy.send_event(Ev::Js("window.onDisconnected && onDisconnected()".into())); }
    }
}

/// Build the `onCrash(rawEngineLog, enhancedLog)` call. `enhancedLog` is the host-side
/// Enhanced-log text (interpreter::interpret_crash), built from the intact engine log PLUS
/// the engine's `RESDIAG:` breadcrumbs (the failing resource id). The two views are kept
/// separate on the page: Enhanced (this) vs Engine log (the raw error.log).
fn crash_js(log: &str, resdiag: &[String]) -> String {
    let enhanced = crate::interpreter::interpret_crash(log, resdiag).unwrap_or_default();
    format!("window.onCrash && onCrash({}, {})", js_str(log), js_str(&enhanced))
}

/// Socket bytes -> lines -> event loop (-> page). Dedup repeated per-frame ANIM lines.
/// On EOF/error the engine connection is gone -> crash modal (if it crashed) or reconnect.
fn spawn_reader(mut reader: BufReader<TcpStream>, proxy: EventLoopProxy<Ev>) {
    thread::spawn(move || {
        let mut buf: Vec<u8> = Vec::with_capacity(256);
        let mut one = [0u8; 1];
        let mut last_anim = String::new();
        // RESDIAG breadcrumbs go to Peptide's Enhanced (advanced) log, NOT the engine chat —
        // captured here and handed to the crash diagnosis when the engine goes away.
        let mut resdiag: Vec<String> = Vec::new();
        loop {
            match reader.read(&mut one) {
                Ok(0) => { engine_gone(&proxy, &resdiag); break; }
                Ok(_) => {
                    if one[0] == b'\n' {
                        let line = String::from_utf8_lossy(&buf).trim_end_matches('\r').to_string();
                        buf.clear();
                        if line.contains("RESDIAG:") { resdiag.push(line); continue; } // enhanced log, not chat
                        // Channel feeds (matchStatus, …) route to their widget, not the chat.
                        if let Some((_ch, payload)) = crate::interpreter::channel_payload(&line) {
                            let _ = proxy.send_event(Ev::Js(format!(
                                "window.onMatchStatus && onMatchStatus({})", js_str(payload))));
                            continue;
                        }
                        if let Some(a) = line.strip_prefix("ANIM:") {
                            if a == last_anim { continue; }
                            last_anim = a.to_string();
                        }
                        if proxy.send_event(Ev::Line(line)).is_err() { break; }
                    } else {
                        buf.push(one[0]);
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => { engine_gone(&proxy, &resdiag); break; }
            }
        }
    });
}

/// A command from the page: translate (friendly -> wire / hscript) and send it to the
/// engine. Client/Error outcomes are echoed back to the page as SYS lines.
fn handle_command(text: &str, writer: &SharedWriter, proxy: &EventLoopProxy<Ev>) {
    let send = |w: &str| {
        if w.is_empty() { return; }
        if let Ok(mut g) = writer.lock() {
            if let Some(s) = g.as_mut() {
                let _ = s.write_all(format!("{w}\n").as_bytes());
                let _ = s.flush();
            }
        }
    };
    match translate(text) {
        Translated::Wire(w) => send(&w),
        Translated::Sequence(v) => for w in v { send(&w); },
        Translated::Repeat { wire, count, gap_ms } => {
            let w = writer.clone();
            thread::spawn(move || {
                for _ in 0..count {
                    if let Ok(mut g) = w.lock() {
                        match g.as_mut() {
                            Some(s) => { if s.write_all(format!("{wire}\n").as_bytes()).is_err() { break; } let _ = s.flush(); }
                            None => break,
                        }
                    }
                    thread::sleep(Duration::from_millis(gap_ms));
                }
            });
        }
        Translated::Client(t) => { let _ = proxy.send_event(Ev::Line(format!("SYS:{}", t.replace('\n', "   ")))); }
        Translated::Error(e) => { let _ = proxy.send_event(Ev::Line(format!("SYS:ERR:{e}"))); }
    }
}

/// Encode an arbitrary string as a safe JavaScript string literal.
fn js_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
