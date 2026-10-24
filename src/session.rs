//! session — shared plumbing for the CLI session loops (`peptide session` /
//! `peptide ssf2 session`).
//!
//! The two engines drive commands over different transports (Fraymakers = an async
//! full-duplex socket with a reader thread; SSF2 = synchronous request/response RPC),
//! so their boot/ready halves stay engine-specific — that split is honest, not
//! accidental. What they DO share is the front end of the loop: tailing the session
//! control file and dispatching each newly-appended line. That boilerplate lived twice;
//! it lives here once now.
//!
//! The GUI does not use this — it's event-loop driven over webview IPC, not a control
//! file — which is exactly why the shared surface is the control-file tail and not a
//! grand unified "session" object.

use std::io::Write;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::net::TcpStream;
use std::path::Path;
use std::time::{Duration, Instant};

/// Classification sink for the Fraymakers engine line stream. The byte-framing and the
/// line classification (RESDIAG breadcrumb / channel feed / ANIM telemetry / READY /
/// normal line) are shared by the CLI session loop and the GUI reader — they differ ONLY
/// in where each class is routed, which is what a sink implements. Default methods are
/// no-ops, so each sink overrides just the classes it cares about (e.g. the GUI drops ANIM
/// from the chat; the CLI drops channel feeds from the log).
pub trait FrayStreamSink {
    /// The engine reported READY (boot complete / match-ready).
    fn on_ready(&mut self) {}
    /// A `RESDIAG:` breadcrumb (the failing resource id) — for the enhanced crash log.
    fn on_resdiag(&mut self, _line: &str) {}
    /// A channel feed (`matchStatus`, `charIcon`, …) → its widget, not the transcript.
    fn on_channel(&mut self, _channel: &str, _payload: &str) {}
    /// Per-transition `ANIM:<state>` telemetry (fires on change).
    fn on_anim(&mut self, _state: &str) {}
    /// Per-frame `FRAME:` telemetry, the driven fighter's own animation each frame.
    fn on_frame(&mut self, _payload: &str) {}
    /// A normal engine line (includes the READY line itself, after `on_ready`).
    fn on_line(&mut self, _raw: &str) {}
    /// The socket closed (engine gone) or a read error ended the stream.
    fn on_eof(&mut self) {}
}

/// Read the Fraymakers engine socket byte-by-byte, frame `\n`-terminated lines, classify
/// each, and dispatch to `sink`. The ONE place FM stream framing + classification lives —
/// the CLI session and the GUI both drive it, differing only in their sink. Returns when
/// the stream ends (EOF / error), after calling `sink.on_eof()`.
pub fn pump_fray_stream(mut reader: BufReader<TcpStream>, sink: &mut impl FrayStreamSink) {
    let mut buf: Vec<u8> = Vec::with_capacity(256);
    let mut one = [0u8; 1];
    loop {
        match reader.read(&mut one) {
            Ok(0) => { sink.on_eof(); break; } // clean EOF
            Ok(_) => {
                if one[0] == b'\n' {
                    let line = String::from_utf8_lossy(&buf).trim_end_matches('\r').to_string();
                    buf.clear();
                    if line.contains("RESDIAG:") {
                        sink.on_resdiag(&line);
                    } else if let Some((ch, payload)) = crate::interpreter::channel_payload(&line) {
                        sink.on_channel(ch, payload);
                    } else if let Some(state) = line.strip_prefix("FRAME:") {
                        // Per-frame timing telemetry. Routed OUT of the normal line path
                        // (60/sec would swamp the log) into the frame buffer `trace` reads,
                        // mirroring how the SSF2 reader routes its own FRAME: pushes.
                        crate::debug_target::push_frame(state);
                        frame_tick();
                        sink.on_frame(state);
                    } else if let Some(state) = line.strip_prefix("ANIM:") {
                        sink.on_anim(state);
                    } else {
                        if line.contains("READY") { sink.on_ready(); }
                        sink.on_line(&line);
                    }
                } else {
                    buf.push(one[0]);
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => { sink.on_eof(); break; } // transient/decode error: stop mirroring
        }
    }
}

/// Tail `control`, dispatching each newly-appended, non-empty line to `on_line`.
///
/// - `tick` is polled once per cycle BEFORE reading; return `true` to stop the loop
///   (e.g. the engine stream ended). Use it for periodic liveness checks.
/// - `on_line` handles one command line; return `false` to stop the loop after it
///   (e.g. an `exit`/`quit` command). Lines are already trimmed and non-empty.
///
/// Blocks until either callback asks to stop. Tracks the read offset so only content
/// appended after the previous read is dispatched (stale queued commands aren't replayed
/// — the caller truncates the file at startup).
// ── the frame clock: the engine's own frames, as a thing to block on ─────────────────────
//
// both engines push one line per frame for the driven fighter. counting those is a clock
// that runs at the engine's rate, whatever speed it is set to, and a wait on it ends the
// moment the frame arrives: no host-side poll, no sleep sized for the slowest case.
static FRAME_CLOCK: (std::sync::Mutex<u64>, std::sync::Condvar) =
    (std::sync::Mutex::new(0), std::sync::Condvar::new());

/// one engine frame has arrived (called by the stream pumps, once per pushed frame).
pub fn frame_tick() {
    let (m, cv) = &FRAME_CLOCK;
    *m.lock().unwrap_or_else(|e| e.into_inner()) += 1;
    cv.notify_all();
}

/// every frame the session has seen since it connected: the engine's own count of the match
/// so far, which is what two engines are brought to the same moment of by
pub fn frames_seen() -> u64 {
    *FRAME_CLOCK.0.lock().unwrap_or_else(|e| e.into_inner())
}

/// block until `n` more frames have arrived, or `timeout` passes: the frames that did.
pub fn frame_wait(n: u64, timeout: Duration) -> u64 {
    let (m, cv) = &FRAME_CLOCK;
    let mut count = m.lock().unwrap_or_else(|e| e.into_inner());
    let start = *count;
    let deadline = Instant::now() + timeout;
    while *count < start + n {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() { break; }
        count = cv.wait_timeout(count, left).unwrap_or_else(|e| e.into_inner()).0;
    }
    *count - start
}

/// the reply to `tick N`: waited on the engine's clock, or ran out of patience first.
pub fn frame_wait_reply(n: u32) -> String {
    // a frame is 16ms at the slowest either engine runs; ten times that is a wedged engine
    let got = frame_wait(n as u64, Duration::from_millis(500 + 200 * n as u64));
    if got >= n as u64 { format!("TICK:{n} frames") } else { format!("TICK:timeout after {got} of {n} frames") }
}

pub fn tail_control(
    control: &Path,
    poll: Duration,
    mut tick: impl FnMut() -> bool,
    mut on_line: impl FnMut(&str) -> bool,
) {
    let mut offset: u64 = 0;
    let mut leftover = String::new();
    loop {
        if tick() {
            break;
        }
        if let Ok(mut f) = std::fs::File::open(control) {
            if f.seek(SeekFrom::Start(offset)).is_ok() {
                let mut chunk = String::new();
                if let Ok(n) = f.read_to_string(&mut chunk) {
                    if n > 0 {
                        offset += n as u64;
                        leftover.push_str(&chunk);
                        let mut stop = false;
                        while let Some(nl) = leftover.find('\n') {
                            let raw: String = leftover.drain(..=nl).collect();
                            let raw = raw.trim();
                            if raw.is_empty() {
                                continue;
                            }
                            if !on_line(raw) {
                                stop = true;
                                break;
                            }
                        }
                        if stop {
                            break;
                        }
                    }
                }
            }
        }
        // woken by the next engine frame, so a command and an announcement are both picked
        // up within a frame of arriving; `poll` only bounds the wait when no frames come
        frame_wait(1, poll);
    }
}

// ── the running pairing: one peptide session + the game it drives ─────────────────────
//
// A session records itself so the NEXT one can find it. Without that, launching a second
// session leaves the first one holding a game that still owns the port and the control
// file, and the two fight: commands land in whichever daemon read the file first, and the
// orphaned game sits there rendering a match nobody is talking to.

/// What a live session left behind: where to talk to it, and who to signal.
#[derive(Debug, Clone)]
pub struct Pairing {
    pub dir: std::path::PathBuf,
    /// The peptide session daemon.
    pub pid: u32,
    /// The game it launched, when it launched one (`--attach` sessions adopt a game they
    /// did not start, and must not kill it).
    pub engine_pid: Option<u32>,
    pub engine: String,
    pub port: u16,
}

fn read_kv(dir: &std::path::Path) -> std::collections::HashMap<String, String> {
    std::fs::read_to_string(dir.join("meta")).unwrap_or_default().lines()
        .filter_map(|l| l.split_once('=').map(|(k, v)| (k.trim().to_string(), v.trim().to_string())))
        .collect()
}

/// Is this process still there? Signal 0 asks the kernel without touching the process.
pub fn pid_alive(pid: u32) -> bool {
    if pid == 0 { return false; }
    #[cfg(unix)]
    { unsafe { libc::kill(pid as i32, 0) == 0 } }
    #[cfg(not(unix))]
    { std::path::Path::new(&format!("/proc/{pid}")).exists() }
}

/// The session running in `dir`, if one still is. A stale meta file (the daemon died, or
/// the machine restarted) reads as no pairing rather than as a session to talk to.
pub fn live_pairing(dir: &std::path::Path) -> Option<Pairing> {
    let kv = read_kv(dir);
    let pid: u32 = kv.get("pid")?.parse().ok()?;
    if !pid_alive(pid) { return None; }
    Some(Pairing {
        dir: dir.to_path_buf(),
        pid,
        engine_pid: kv.get("engine_pid").and_then(|s| s.parse().ok()).filter(|&p| pid_alive(p)),
        engine: kv.get("engine").cloned().unwrap_or_else(|| "fraymakers".into()),
        port: kv.get("port").and_then(|s| s.parse().ok()).unwrap_or(0),
    })
}

/// A game already open is driven, not replaced: the session already running is handed the
/// match this launch asks for (its `spawn`, or nothing when no roster was named), and this
/// process goes away. `--fresh` is the way to say "a new game regardless". Returns false
/// when the old pair has no live game to hand the match to (then it is replaced).
pub fn adopt_pairing(p: &Pairing, spawn: Option<&str>, args: &[String], tell: &str) -> bool {
    if args.iter().any(|a| a == "--fresh") { return false; }
    let game_alive = match p.engine_pid { Some(e) => pid_alive(e), None => true };
    if !game_alive { return false; }
    match spawn {
        Some(cmd) => {
            let mut f = match std::fs::OpenOptions::new().append(true).open(p.dir.join("control")) {
                Ok(f) => f, Err(_) => return false,
            };
            let _ = writeln!(f, "{cmd}");
            println!("peptide: the {} game is already open (session pid {}, port {}); starting the match there instead of launching another.\n  sent: {cmd}\n  drive it with: {tell} \"<cmd>\"  (--fresh launches a new game)",
                     p.engine, p.pid, p.port);
        }
        None => {
            println!("peptide: the {} game is already open (session pid {}, port {}); nothing launched.\n  drive it with: {tell} \"<cmd>\"  (--fresh launches a new game)",
                     p.engine, p.pid, p.port);
        }
    }
    true
}

/// Stand the previous pairing down before taking its place: ask the daemon to exit through
/// its own control file first (so it shuts the game down the clean way it already knows),
/// and only signal it directly if it will not go.
pub fn end_pairing(p: &Pairing) {
    let _ = std::fs::write(p.dir.join("control"), b"exit\n");
    for _ in 0..40 {
        if !pid_alive(p.pid) { break; }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    #[cfg(unix)]
    unsafe {
        if pid_alive(p.pid) { libc::kill(p.pid as i32, libc::SIGTERM); }
        // the game only gets signalled if this session started it AND it outlived its daemon
        if let Some(e) = p.engine_pid {
            std::thread::sleep(std::time::Duration::from_millis(300));
            if pid_alive(e) { libc::kill(e as i32, libc::SIGTERM); }
        }
    }
    let _ = std::fs::remove_file(p.dir.join("meta"));
}

/// Record this pairing so the next session can find (or replace) it.
pub fn write_pairing(dir: &std::path::Path, engine: &str, port: u16, token: &str, engine_pid: Option<u32>) {
    let ep = engine_pid.map(|p| p.to_string()).unwrap_or_default();
    let _ = std::fs::write(dir.join("meta"), format!(
        "port={port}\ntoken={token}\ncontrol={}\nlog={}\npid={}\nengine={engine}\nengine_pid={ep}\n",
        dir.join("control").display(), dir.join("out.log").display(), std::process::id()));
}
