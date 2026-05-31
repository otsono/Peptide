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

use crate::commands::{translate, Translated};

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

pub fn launch() -> std::io::Result<()> {
    let event_loop = EventLoopBuilder::<Ev>::with_user_event().build();
    let window = WindowBuilder::new()
        .with_title("Peptide")
        .with_inner_size(LogicalSize::new(940.0, 720.0))
        .with_min_inner_size(LogicalSize::new(520.0, 400.0))
        .build(&event_loop)
        .map_err(|e| io(&e.to_string()))?;

    // initial connection
    let (reader, w, port, token, c) = crate::ui::boot()?;
    let writer: SharedWriter = Arc::new(Mutex::new(Some(w)));
    let cleanup: SharedCleanup = Arc::new(Mutex::new(Some(c)));
    let conn: SharedConn = Arc::new(Mutex::new((port, token)));
    spawn_reader(reader, event_loop.create_proxy());

    let char_name = crate::config::Config::load().char_name();
    let init = format!("window.__PORT__={port}; window.__CHAR__={};", js_str(&char_name));

    let ipc_writer = writer.clone();
    let ipc_cleanup = cleanup.clone();
    let ipc_conn = conn.clone();
    let ipc_proxy = event_loop.create_proxy();
    let ipc_char = char_name.clone();
    let webview = WebViewBuilder::new()
        .with_html(include_str!("peptide_ui.html"))
        .with_initialization_script(&init)
        .with_ipc_handler(move |req: Request<String>| {
            let body = req.body().to_string();
            let (w, cl, cn, px, ch) = (ipc_writer.clone(), ipc_cleanup.clone(), ipc_conn.clone(),
                                       ipc_proxy.clone(), ipc_char.clone());
            match body.as_str() {
                RECONNECT => { thread::spawn(move || reconnect_existing(w, cn, px, ch)); }
                BOOT_QUICK => { thread::spawn(move || boot_new(w, cl, cn, px, ch, true)); }
                BOOT_REGULAR => { thread::spawn(move || boot_new(w, cl, cn, px, ch, false)); }
                _ => handle_command(&body, &ipc_writer, &ipc_proxy),
            }
        })
        .build(&window)
        .map_err(|e| io(&e.to_string()))?;

    event_loop.run(move |event, _t, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::NewEvents(StartCause::Init) => {
                let _ = webview.evaluate_script("window.setStatus && setStatus(false)");
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
    match crate::ui::patch_and_launch_with_progress(Some(&on_progress)) {
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
                let _ = proxy.send_event(Ev::Js(
                    "window.onBootFailed && onBootFailed(\"Fraymakers didn’t connect.\")".into()));
            }
        },
        Err(e) => {
            let _ = proxy.send_event(Ev::Js(format!(
                "window.onBootFailed && onBootFailed({})", js_str(&e.to_string()))));
        }
    }
}

/// Socket bytes -> lines -> event loop (-> page). Dedup repeated per-frame ANIM lines.
/// On EOF/error the engine connection is gone -> tell the page to start its reconnect flow.
fn spawn_reader(mut reader: BufReader<TcpStream>, proxy: EventLoopProxy<Ev>) {
    thread::spawn(move || {
        let mut buf: Vec<u8> = Vec::with_capacity(256);
        let mut one = [0u8; 1];
        let mut last_anim = String::new();
        loop {
            match reader.read(&mut one) {
                Ok(0) => {
                    let _ = proxy.send_event(Ev::Js("window.onDisconnected && onDisconnected()".into()));
                    break;
                }
                Ok(_) => {
                    if one[0] == b'\n' {
                        let line = String::from_utf8_lossy(&buf).trim_end_matches('\r').to_string();
                        buf.clear();
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
                Err(_) => {
                    let _ = proxy.send_event(Ev::Js("window.onDisconnected && onDisconnected()".into()));
                    break;
                }
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
