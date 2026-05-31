//! gui — the graphical chat console (the default `peptide` mode): a native window using
//! the system webview (wry) — WKWebView on macOS, WebView2/Edge on Windows, WebKitGTK on
//! Linux. The whole UI is HTML/CSS/JS (src/peptide_ui.html, Claude dark theme); this file
//! is the glue: boot the engine, stream replies into the page, send the page's commands to
//! the socket, and re-boot + reconnect on demand when the connection is lost.

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
    Line(String),  // an engine reply line -> onLine(...)
    Js(String),    // arbitrary JS to evaluate (status, modal, reconnect callbacks)
}

type SharedWriter = Arc<Mutex<Option<TcpStream>>>;
type SharedCleanup = Arc<Mutex<Option<crate::ui::Cleanup>>>;

const RECONNECT_CMD: &str = "@@reconnect";

pub fn launch() -> std::io::Result<()> {
    let event_loop = EventLoopBuilder::<Ev>::with_user_event().build();
    let window = WindowBuilder::new()
        .with_title("Peptide")
        .with_inner_size(LogicalSize::new(940.0, 720.0))
        .with_min_inner_size(LogicalSize::new(520.0, 400.0))
        .build(&event_loop)
        .map_err(|e| io(&e.to_string()))?;

    // initial connection
    let (reader, w, port, c) = crate::ui::boot()?;
    let writer: SharedWriter = Arc::new(Mutex::new(Some(w)));
    let cleanup: SharedCleanup = Arc::new(Mutex::new(Some(c)));
    spawn_reader(reader, event_loop.create_proxy());

    let char_name = std::env::var("FRAY_CHAR").unwrap_or_else(|_| "sandbag".into());
    let init = format!("window.__PORT__={port}; window.__CHAR__={};", js_str(&char_name));

    let ipc_writer = writer.clone();
    let ipc_cleanup = cleanup.clone();
    let ipc_proxy = event_loop.create_proxy();
    let ipc_char = char_name.clone();
    let webview = WebViewBuilder::new()
        .with_html(include_str!("peptide_ui.html"))
        .with_initialization_script(&init)
        .with_ipc_handler(move |req: Request<String>| {
            let body = req.body();
            if body == RECONNECT_CMD {
                let (w, c, p, ch) = (ipc_writer.clone(), ipc_cleanup.clone(), ipc_proxy.clone(), ipc_char.clone());
                thread::spawn(move || reconnect(w, c, p, ch));
            } else {
                handle_command(body, &ipc_writer, &ipc_proxy);
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

/// Re-patch + re-boot Fraymakers and swap the live connection in place. Runs on a worker
/// thread (ui::boot blocks until the new engine dials in). Drives the page's modal.
fn reconnect(writer: SharedWriter, cleanup: SharedCleanup, proxy: EventLoopProxy<Ev>, char_name: String) {
    let _ = proxy.send_event(Ev::Js("window.onReconnecting && onReconnecting()".into()));
    // tear down the old engine first (same temp-file paths are reused by the new boot)
    if let Some(mut c) = cleanup.lock().ok().and_then(|mut g| g.take()) {
        c.dispose();
    }
    if let Ok(mut g) = writer.lock() { *g = None; }

    match crate::ui::boot() {
        Ok((reader, w, port, c)) => {
            if let Ok(mut g) = writer.lock() { *g = Some(w); }
            if let Ok(mut g) = cleanup.lock() { *g = Some(c); }
            spawn_reader(reader, proxy.clone());
            let _ = proxy.send_event(Ev::Js(format!(
                "window.onReconnected && onReconnected({}, {})", port, js_str(&char_name))));
        }
        Err(e) => {
            let _ = proxy.send_event(Ev::Js(format!(
                "window.onReconnectFailed && onReconnectFailed({})", js_str(&e.to_string()))));
        }
    }
}

/// Socket bytes -> lines -> event loop (-> page). Dedup repeated per-frame ANIM lines.
/// On EOF/error the engine connection is gone -> tell the page to show the reconnect modal.
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
