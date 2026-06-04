//! overlay — a standalone, semi-transparent debugger HUD that floats over the running
//! game (any engine). It is NOT the Peptide GUI: it's a separate, decoration-less,
//! click-through, always-on-top window with a tiny webview, spawned as its own process so
//! it works identically whether the session was started from the CLI (`peptide session`)
//! or the GUI. It just tails the session's `out.log` and renders the live state + the
//! `SCRIPTERR:` stream (the socket-mirrored engine/script errors).
//!
//! `peptide overlay [--log <path>]` — `--log` points at the session mirror to tail
//! (defaults to the standard session `out.log`). The session spawns this for you.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::Duration;

use tao::dpi::{LogicalPosition, LogicalSize};
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::WindowBuilder;
use wry::WebViewBuilder;

enum Ev {
    Line(String),
    /// Move the overlay to (x, y) screen points — the follow thread keeps it pinned to the
    /// game window's top-left corner.
    Move(f64, f64),
}

fn arg_val(args: &[String], flag: &str) -> Option<String> {
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned()
}

/// Kills the spawned overlay process when the owning session drops.
pub(crate) struct OverlayGuard(std::process::Child);
impl Drop for OverlayGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Spawn the overlay for a debug session, tailing `logp`. Engine-agnostic: BOTH the
/// Fraymakers (`bridge`) and SSF2 (`ssf2_bridge`) sessions call this with their own log
/// path, so the host-facing overlay feature is shared, not bolted onto one engine. Returns
/// None when disabled (`--no-overlay` / `PEPTIDE_OVERLAY=0`) or if spawn fails — the overlay
/// is cosmetic and must never take a session down. The child also watchdogs `--parent`, so
/// it self-exits on a session SIGKILL where Drop can't run.
pub(crate) fn spawn_for_session(logp: &std::path::Path, args: &[String]) -> Option<OverlayGuard> {
    let off = args.iter().any(|a| a == "--no-overlay")
        || std::env::var("PEPTIDE_OVERLAY").map(|v| v == "0").unwrap_or(false);
    if off {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    std::process::Command::new(exe)
        .arg("overlay")
        .arg("--log")
        .arg(logp)
        .arg("--parent")
        .arg(std::process::id().to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()
        .map(OverlayGuard)
}

/// Minimal JSON string literal (the only host→page payload is one log line).
fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

pub fn run(args: &[String]) -> std::io::Result<()> {
    let log_path = arg_val(args, "--log")
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::bridge::default_session_dir().join("out.log"));

    let event_loop = EventLoopBuilder::<Ev>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    let window = WindowBuilder::new()
        .with_title("Peptide Overlay")
        .with_decorations(false)
        .with_transparent(true)
        .with_always_on_top(true)
        .with_resizable(false)
        .with_inner_size(LogicalSize::new(396.0, 240.0))
        .build(&event_loop)
        .map_err(|e| std::io::Error::other(e.to_string()))?;

    // Park top-left of the primary monitor (out of the way of most of the play area).
    if let Some(mon) = window.current_monitor() {
        let _ = mon; // position is in logical px from the top-left origin
    }
    window.set_outer_position(LogicalPosition::new(24.0, 48.0));
    // Never steal input from the game underneath — the HUD is display-only.
    let _ = window.set_ignore_cursor_events(true);
    // tao's with_transparent alone leaves the NSWindow opaque on macOS; force it clear.
    #[cfg(target_os = "macos")]
    apply_macos_transparency(&window);

    let html = crate::read_asset("peptide_overlay.html");
    #[cfg(not(target_os = "linux"))]
    let _webview = WebViewBuilder::new()
        .with_transparent(true)
        .with_html(&html)
        .build(&window)
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    #[cfg(target_os = "linux")]
    let _webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        let vbox = window.default_vbox().unwrap();
        WebViewBuilder::new()
            .with_transparent(true)
            .with_html(&html)
            .build_gtk(vbox)
            .map_err(|e| std::io::Error::other(e.to_string()))?
    };

    // Tail the session log on a background thread; forward each line to the webview.
    {
        let proxy = proxy.clone();
        std::thread::spawn(move || tail_log(&log_path, proxy));
    }

    // Attach to the game: pin the overlay to the Fraymakers window's top-left corner and
    // follow it as it moves. Polls a couple of times a second; no-op until the game appears.
    #[cfg(target_os = "macos")]
    {
        let proxy = proxy.clone();
        std::thread::spawn(move || {
            let mut last: Option<(i64, i64)> = None;
            let mut ever_seen = false;
            let mut gone_polls = 0u32;
            loop {
                std::thread::sleep(Duration::from_millis(500));
                match find_game_window() {
                    Some((gx, gy, _w, _h)) => {
                        ever_seen = true;
                        gone_polls = 0;
                        let pos = ((gx + 16.0) as i64, (gy + 16.0) as i64);
                        if last != Some(pos) {
                            last = Some(pos);
                            if proxy.send_event(Ev::Move(pos.0 as f64, pos.1 as f64)).is_err() {
                                return;
                            }
                        }
                    }
                    None => {
                        // The game window vanished. If we'd already attached to it, the engine
                        // crashed or was closed — tear the overlay down too (the GUI/session
                        // process may still be alive, so the pid watchdog alone won't catch
                        // this). A short grace avoids a transient miss during a fullscreen flip.
                        if ever_seen {
                            gone_polls += 1;
                            if gone_polls >= 4 {
                                std::process::exit(0);
                            }
                        }
                    }
                }
            }
        });
    }

    // Watchdog: when the spawning session dies (incl. SIGKILL, where its Drop guard never
    // runs), tear the overlay down too so no orphaned window is left floating.
    if let Some(parent) = arg_val(args, "--parent").and_then(|p| p.parse::<i32>().ok()) {
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_millis(1000));
            if !parent_alive(parent) {
                std::process::exit(0);
            }
        });
    }

    event_loop.run(move |event, _t, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(Ev::Line(line)) => {
                let _ = _webview.evaluate_script(&format!("window.push && push({})", json_str(&line)));
            }
            Event::UserEvent(Ev::Move(x, y)) => {
                window.set_outer_position(LogicalPosition::new(x, y));
            }
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                *control_flow = ControlFlow::Exit;
            }
            _ => {}
        }
    });
}

/// Force the overlay's NSWindow to be genuinely transparent. tao's `with_transparent`
/// doesn't reliably clear the macOS window background on its own, so set it directly:
/// non-opaque + clear background + no shadow. The webview (transparent) then composites
/// over the live desktop/game beneath.
#[cfg(target_os = "macos")]
fn apply_macos_transparency(window: &tao::window::Window) {
    use objc2_app_kit::{NSColor, NSWindow};
    use tao::platform::macos::WindowExtMacOS;
    let ptr = window.ns_window();
    if ptr.is_null() {
        return;
    }
    // SAFETY: tao hands back the live NSWindow pointer; objc2's NSWindow is a transparent
    // wrapper over that objc object, so the cast-and-borrow is valid for the call duration.
    unsafe {
        let nsw: &NSWindow = &*(ptr as *const NSWindow);
        nsw.setOpaque(false);
        nsw.setBackgroundColor(Some(&NSColor::clearColor()));
        nsw.setHasShadow(false);
    }
}

/// Find the on-screen bounds (x, y, width, height in screen points) of the Fraymakers game
/// window via the CoreGraphics window list. Raw CF FFI to dodge the typed wrappers' generics.
/// Returns the first sufficiently-large window owned by the engine process.
#[cfg(target_os = "macos")]
fn find_game_window() -> Option<(f64, f64, f64, f64)> {
    use core_foundation::array::CFArrayRef;
    use core_foundation::base::TCFType;
    use core_foundation::string::{CFString, CFStringRef};
    use core_graphics::window::{kCGWindowBounds, kCGWindowListOptionOnScreenOnly, kCGWindowOwnerName};
    use std::ffi::c_void;

    const FLOAT64: i64 = 6; // kCFNumberFloat64Type
    extern "C" {
        fn CGWindowListCopyWindowInfo(option: u32, relative: u32) -> CFArrayRef;
        fn CFArrayGetCount(arr: CFArrayRef) -> isize;
        fn CFArrayGetValueAtIndex(arr: CFArrayRef, idx: isize) -> *const c_void;
        fn CFDictionaryGetValueIfPresent(d: *const c_void, k: *const c_void, v: *mut *const c_void) -> u8;
        fn CFNumberGetValue(n: *const c_void, ty: i64, out: *mut c_void) -> u8;
        fn CFRelease(cf: *const c_void);
    }

    unsafe fn dict_get(d: *const c_void, key: *const c_void) -> Option<*const c_void> {
        let mut out: *const c_void = std::ptr::null();
        if CFDictionaryGetValueIfPresent(d, key, &mut out) != 0 && !out.is_null() {
            Some(out)
        } else {
            None
        }
    }
    unsafe fn dict_num(d: *const c_void, key_name: &str) -> Option<f64> {
        let key = CFString::new(key_name);
        let v = dict_get(d, key.as_concrete_TypeRef() as *const c_void)?;
        let mut n: f64 = 0.0;
        if CFNumberGetValue(v, FLOAT64, &mut n as *mut f64 as *mut c_void) != 0 {
            Some(n)
        } else {
            None
        }
    }

    unsafe {
        let arr = CGWindowListCopyWindowInfo(kCGWindowListOptionOnScreenOnly, 0);
        if arr.is_null() {
            return None;
        }
        let mut found = None;
        let count = CFArrayGetCount(arr);
        for i in 0..count {
            let dict = CFArrayGetValueAtIndex(arr, i);
            if dict.is_null() {
                continue;
            }
            // owner name == the engine?
            let owner = match dict_get(dict, kCGWindowOwnerName as *const c_void) {
                Some(s) => CFString::wrap_under_get_rule(s as CFStringRef).to_string().to_lowercase(),
                None => continue,
            };
            // Match either engine's game window: Fraymakers runs as the HashLink binary `hl`
            // (or a "Fraymakers" bundle); SSF2 as its own app. NOT "peptide" — that's our own
            // overlay window, which would self-attach and drift.
            if !(owner.contains("fraymakers") || owner == "hl" || owner.contains("ssf2")) {
                continue;
            }
            let Some(bounds) = dict_get(dict, kCGWindowBounds as *const c_void) else { continue };
            let (Some(x), Some(y), Some(w), Some(h)) = (
                dict_num(bounds, "X"),
                dict_num(bounds, "Y"),
                dict_num(bounds, "Width"),
                dict_num(bounds, "Height"),
            ) else {
                continue;
            };
            if w >= 200.0 && h >= 150.0 {
                found = Some((x, y, w, h));
                break;
            }
        }
        CFRelease(arr as *const c_void);
        found
    }
}

/// Is process `pid` still alive? `kill(pid, 0)` sends no signal but errors with ESRCH when
/// the process is gone. Non-unix has no cheap equivalent here, so assume alive (Drop on the
/// session side still covers the clean-exit case there).
#[cfg(unix)]
fn parent_alive(pid: i32) -> bool {
    unsafe { libc::kill(pid, 0) == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM) }
}
#[cfg(not(unix))]
fn parent_alive(_pid: i32) -> bool {
    true
}

/// Follow `path`, emitting each newly appended line. Polls for the file to appear (the
/// session may create it just after spawning us) and re-opens it if it's truncated/rotated
/// (the session truncates `out.log` on each fresh boot).
fn tail_log(path: &Path, proxy: tao::event_loop::EventLoopProxy<Ev>) {
    loop {
        let file = match std::fs::File::open(path) {
            Ok(f) => f,
            Err(_) => {
                std::thread::sleep(Duration::from_millis(250));
                continue;
            }
        };
        // bytes consumed from THIS open (BufReader starts at offset 0), used only to detect
        // a shrink (truncation/rotation). Must start at 0 — seeding it with the current file
        // size double-counts every byte we then read and spuriously trips the shrink check.
        let mut len_seen = 0u64;
        let mut reader = BufReader::new(file);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => {
                    // EOF — detect truncation (file shrank → fresh session), else wait.
                    if let Ok(m) = std::fs::metadata(path) {
                        if m.len() < len_seen {
                            break; // re-open from the top
                        }
                        len_seen = m.len();
                    }
                    std::thread::sleep(Duration::from_millis(120));
                }
                Ok(n) => {
                    len_seen += n as u64;
                    let trimmed = line.trim_end();
                    // Close the overlay the moment the engine/TCP goes away — the session
                    // logs this when the stream ends (engine quit, window closed, or the
                    // socket dropped). (peptide itself closing is covered by the pid watchdog.)
                    if trimmed.contains("[engine stream ended]")
                        || trimmed.contains("engine gone")
                        || trimmed.contains("engine exited")
                        || trimmed.contains("[ssf2-session] exit") // SSF2 session's stop marker
                    {
                        std::process::exit(0);
                    }
                    if !trimmed.is_empty() && proxy.send_event(Ev::Line(trimmed.to_string())).is_err() {
                        return; // event loop gone
                    }
                }
                Err(_) => break,
            }
        }
    }
}
