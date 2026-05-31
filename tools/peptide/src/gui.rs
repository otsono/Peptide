//! gui — the graphical chat console (the default `peptide` mode): a native window using
//! the system webview (wry) — WKWebView on macOS, WebView2/Edge on Windows, WebKitGTK on
//! Linux. The whole UI is HTML/CSS/JS (src/peptide_ui.html, Claude dark theme); this file
//! is just the glue: boot the engine, stream replies into the page, send the page's
//! commands to the socket. One cross-platform binary.

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

/// Lines pushed from the reader thread (and synthetic SYS lines from the IPC handler)
/// into the event loop, which forwards them to the page.
enum Ev {
    Line(String),
}

type SharedWriter = Arc<Mutex<TcpStream>>;

pub fn launch() -> std::io::Result<()> {
    let event_loop = EventLoopBuilder::<Ev>::with_user_event().build();
    let window = WindowBuilder::new()
        .with_title("Peptide")
        .with_inner_size(LogicalSize::new(940.0, 720.0))
        .with_min_inner_size(LogicalSize::new(520.0, 400.0))
        .build(&event_loop)
        .map_err(|e| io(&e.to_string()))?;

    // Boot the engine + connect; stream its replies in on a reader thread.
    let (reader, writer, port, cleanup) = crate::ui::boot()?;
    let writer: SharedWriter = Arc::new(Mutex::new(writer));
    spawn_reader(reader, event_loop.create_proxy());

    let char_name = std::env::var("FRAY_CHAR").unwrap_or_else(|_| "sandbag".into());
    let init = format!(
        "window.__PORT__={port}; window.__CHAR__={};",
        js_str(&char_name)
    );

    let ipc_writer = writer.clone();
    let ipc_proxy = event_loop.create_proxy();
    let webview = WebViewBuilder::new()
        .with_html(include_str!("peptide_ui.html"))
        .with_initialization_script(&init)
        .with_ipc_handler(move |req: Request<String>| {
            handle_command(req.body(), &ipc_writer, &ipc_proxy);
        })
        .build(&window)
        .map_err(|e| io(&e.to_string()))?;

    let mut cleanup = Some(cleanup);
    event_loop.run(move |event, _target, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::NewEvents(StartCause::Init) => {
                let _ = webview.evaluate_script("window.setStatus && setStatus(false)");
            }
            Event::UserEvent(Ev::Line(line)) => {
                let _ = webview.evaluate_script(&format!("window.onLine && onLine({})", js_str(&line)));
            }
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                if let Some(mut c) = cleanup.take() {
                    c.dispose(); // kill the engine + remove temp files before we exit the process
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

/// Socket bytes -> lines -> event loop (-> page). Dedup repeated per-frame ANIM lines.
fn spawn_reader(mut reader: BufReader<TcpStream>, proxy: EventLoopProxy<Ev>) {
    thread::spawn(move || {
        let mut buf: Vec<u8> = Vec::with_capacity(256);
        let mut one = [0u8; 1];
        let mut last_anim = String::new();
        loop {
            match reader.read(&mut one) {
                Ok(0) => {
                    let _ = proxy.send_event(Ev::Line("SYS:● engine disconnected".into()));
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
                Err(_) => break,
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
            let _ = g.write_all(format!("{w}\n").as_bytes());
            let _ = g.flush();
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
                        if g.write_all(format!("{wire}\n").as_bytes()).is_err() { break; }
                        let _ = g.flush();
                    }
                    thread::sleep(Duration::from_millis(gap_ms));
                }
            });
        }
        Translated::Client(t) => {
            let _ = proxy.send_event(Ev::Line(format!("SYS:{}", t.replace('\n', "   "))));
        }
        Translated::Error(e) => {
            let _ = proxy.send_event(Ev::Line(format!("SYS:ERR:{e}")));
        }
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
