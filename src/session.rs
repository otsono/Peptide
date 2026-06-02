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

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::Duration;

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
        std::thread::sleep(poll);
    }
}
