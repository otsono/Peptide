//! bridge — headless runtime: the loopback TCP server that the injected engine
//! dials into. `serve` is the interactive stdin<->socket bridge; `send_once` is a
//! one-shot for scripts. Connection setup (await_engine) lives in `ui` and is shared.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use crate::interpreter::{translate, gloss, Translated};

const DEFAULT_PORT: u16 = 17999;
pub fn parse_port(args: &[String]) -> u16 {
    args.iter()
        .position(|a| a == "--port")
        .and_then(|i| args.get(i + 1))
        .and_then(|p| p.parse().ok())
        .unwrap_or(DEFAULT_PORT)
}
pub fn parse_token(args: &[String]) -> Option<String> {
    args.iter()
        .position(|a| a == "--token")
        .and_then(|i| args.get(i + 1))
        .cloned()
}
pub fn serve(port: u16, token: Option<&str>) {
    let (reader, mut write_half) = crate::ui::await_engine(port, token);

    // socket -> stdout; signal once the engine reports READY.
    //
    // ROBUSTNESS: read RAW BYTES and lossy-decode each line, instead of
    // `reader.lines()` (which yields Err and aborts the loop the moment the
    // engine emits a non-UTF8 byte). Critically, on a read error or EOF we do
    // NOT std::process::exit — exiting closes our socket, and the engine's
    // injected per-frame write then faults with `Eof` in Main.update and CRASHES
    // the whole engine mid-match. Keeping the process (and thus the socket) alive
    // lets the engine keep running so a match can actually be observed.
    let (ready_tx, ready_rx) = mpsc::channel::<()>();
    let mut byte_reader = reader;
    thread::spawn(move || {
        let mut buf: Vec<u8> = Vec::with_capacity(256);
        let mut one = [0u8; 1];
        // The engine emits "ANIM:<state>" EVERY frame; dedup so only changes print.
        let mut last_anim = String::new();
        // Crash-diagnostics ring buffer: the last few meaningful events (state /
        // animation transitions, move acks, physics) so that when the engine stream
        // ends — especially on a crash — we can show what the character was doing.
        let mut history: std::collections::VecDeque<String> = std::collections::VecDeque::new();
        let dump_history = |h: &std::collections::VecDeque<String>| {
            if h.is_empty() { return; }
            eprintln!("peptide-bridge: ── last {} engine events before stream ended (crash context) ──", h.len());
            for ev in h { eprintln!("peptide-bridge:    {ev}"); }
        };
        loop {
            match byte_reader.read(&mut one) {
                Ok(0) => {
                    // Clean EOF: engine closed its write side. Do NOT exit; the
                    // engine may still be running and writing — hold our socket.
                    eprintln!("peptide-bridge: engine closed read stream (holding socket open)");
                    dump_history(&history);
                    break;
                }
                Ok(_) => {
                    if one[0] == b'\n' {
                        let line = String::from_utf8_lossy(&buf);
                        let line = line.trim_end_matches('\r');
                        // Channel feeds (matchStatus, …) are not for the CLI — drop them.
                        if crate::interpreter::channel_payload(line).is_some() { buf.clear(); continue; }
                        // Suppress repeated per-frame ANIM lines; print only on change.
                        // Append a plain-English gloss to recognized reply lines
                        // (additive — the raw line is preserved so scripts that
                        // match on it, e.g. READY/ANIM detection, still work).
                        let pretty = match gloss(line) {
                            Some(g) => format!("<< {line:<28} ({g})"),
                            None => format!("<< {line}"),
                        };
                        let mut emitted = true;
                        if let Some(a) = line.strip_prefix("ANIM:") {
                            if a != last_anim {
                                last_anim = a.to_string();
                                println!("{pretty}");
                            } else { emitted = false; }
                        } else {
                            println!("{pretty}");
                        }
                        // Buffer meaningful events for the crash-context dump.
                        if emitted && line.starts_with(|c: char| c.is_ascii_uppercase()) && line.contains(':') {
                            history.push_back(line.to_string());
                            if history.len() > 16 { history.pop_front(); }
                        }
                        if line.contains("READY") {
                            let _ = ready_tx.send(());
                        }
                        buf.clear();
                    } else {
                        buf.push(one[0]);
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => {
                    // Transient/decode-ish error: keep the socket alive, stop
                    // mirroring. NEVER exit here (see comment above).
                    eprintln!("peptide-bridge: read error (holding socket open)");
                    dump_history(&history);
                    break;
                }
            }
        }
    });

    // Always wait for READY before accepting any input.
    eprintln!("peptide-bridge: waiting for engine READY…");
    match ready_rx.recv_timeout(Duration::from_secs(60)) {
        Ok(()) => eprintln!("peptide-bridge: engine is READY — enter commands:"),
        Err(_) => {
            eprintln!("peptide-bridge: timed out waiting for READY");
            return;
        }
    }

    // stdin -> socket. The FIRST command fires the instant READY arrives
    // (event-driven, no artificial pre-delay). FRAY_CMD_GAP (seconds, may be
    // fractional) paces SUBSEQUENT commands so a piped multi-command sequence
    // doesn't all flush at once — callers dump every command immediately and
    // let us space them, instead of sleeping before READY.
    let cmd_gap = std::env::var("FRAY_CMD_GAP").ok()
        .and_then(|s| s.trim().parse::<f64>().ok())
        .filter(|g| *g > 0.0);
    // Write one wire line + flush; false if the socket is gone.
    fn send_wire(w: &mut TcpStream, wire: &str) -> bool {
        let mut line = wire.to_string();
        line.push('\n');
        w.write_all(line.as_bytes()).is_ok() && w.flush().is_ok()
    }
    let stdin = std::io::stdin();
    let mut first = true;
    'outer: for line in stdin.lock().lines() {
        let Ok(raw) = line else { break };
        // Translate the friendly command ("spawn sandbag", "move special_neutral")
        // into the engine wire line(s). `help` and bad input are handled here and
        // never reach the engine; `loop` expands to a repeated client-side send.
        match translate(&raw) {
            Translated::Client(text) => { print!("{text}"); continue; }
            Translated::Error(msg) => { eprintln!("peptide-bridge: {msg}"); continue; }
            Translated::Wire(wire) => {
                if !first {
                    if let Some(g) = cmd_gap { thread::sleep(Duration::from_secs_f64(g)); }
                }
                first = false;
                if wire == raw.trim() {
                    eprintln!("peptide-bridge: SENT {wire:?}");
                } else {
                    eprintln!("peptide-bridge: SENT {wire:?}  (from {:?})", raw.trim());
                }
                if !send_wire(&mut write_half, &wire) {
                    eprintln!("peptide-bridge: write failed (engine gone?)");
                    break;
                }
            }
            Translated::Repeat { wire, count, gap_ms } => {
                eprintln!("peptide-bridge: LOOP {wire:?} x{count} every {gap_ms}ms  (from {:?})", raw.trim());
                if !first {
                    if let Some(g) = cmd_gap { thread::sleep(Duration::from_secs_f64(g)); }
                }
                first = false;
                for i in 0..count {
                    if i > 0 { thread::sleep(Duration::from_millis(gap_ms)); }
                    eprintln!("peptide-bridge: SENT {wire:?} ({}/{count})", i + 1);
                    if !send_wire(&mut write_half, &wire) {
                        eprintln!("peptide-bridge: write failed (engine gone?)");
                        break 'outer;
                    }
                }
            }
            Translated::Sequence(wires) => {
                eprintln!("peptide-bridge: SEQ {wires:?}  (from {:?})", raw.trim());
                if !first {
                    if let Some(g) = cmd_gap { thread::sleep(Duration::from_secs_f64(g)); }
                }
                first = false;
                // Tight pacing so a `track` samples physics across a move's brief
                // active window (the engine reads ~1 command/frame ≈ 16ms).
                for (i, w) in wires.iter().enumerate() {
                    if i > 0 { thread::sleep(Duration::from_millis(60)); }
                    eprintln!("peptide-bridge: SENT {w:?}");
                    if !send_wire(&mut write_half, w) {
                        eprintln!("peptide-bridge: write failed (engine gone?)");
                        break 'outer;
                    }
                }
            }
        }
    }

    // CRITICAL: stdin EOF (e.g. a piped command list finished) must NOT close the
    // socket — that would Eof-crash the engine's per-frame write. Hold the socket
    // open until the engine itself goes away. FRAY_HOLD_SECS bounds the wait so a
    // scripted run still terminates; default is long enough for a match.
    let hold = std::env::var("FRAY_HOLD_SECS").ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(600);
    eprintln!("peptide-bridge: stdin closed; holding socket open up to {hold}s (engine keeps running)");
    let mut probe = [0u8; 1];
    let deadline = Duration::from_secs(hold);
    let step = Duration::from_millis(250);
    let mut waited = Duration::ZERO;
    // Keep the write half alive; periodically check the engine is still there by
    // attempting a zero-impact write of nothing (flush). If the engine vanished,
    // the flush/write eventually errors and we exit.
    while waited < deadline {
        thread::sleep(step);
        waited += step;
        if write_half.write_all(&[]).is_err() || write_half.flush().is_err() {
            eprintln!("peptide-bridge: engine gone; exiting");
            break;
        }
        let _ = &mut probe; // reserved for future liveness ping
    }
}
pub fn send_once(port: u16, token: Option<&str>, cmd: &str) {
    // Translate the friendly command up front; `help`/errors never open a socket.
    let cmd = match translate(cmd) {
        Translated::Wire(w) => w,
        // `send` is the one-shot scripted path; loop is an interactive/serve feature.
        // Degrade gracefully: fire the move once and note it.
        Translated::Repeat { wire, .. } => {
            eprintln!("peptide-bridge: 'loop' sends once in one-shot mode — use serve/runseq for repetition");
            wire
        }
        Translated::Sequence(wires) => {
            eprintln!("peptide-bridge: 'snapshot'/sequence sends only the first cmd in one-shot mode — use serve/runseq for the full bundle");
            wires.into_iter().next().unwrap_or_default()
        }
        Translated::Client(text) => { print!("{text}"); return; }
        Translated::Error(msg) => { eprintln!("peptide-bridge: {msg}"); std::process::exit(2); }
    };
    let cmd = cmd.as_str();
    let (reader, mut write_half) = crate::ui::await_engine(port, token);

    let (tx, rx) = mpsc::channel::<String>();
    thread::spawn(move || {
        for line in reader.lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });

    // ALWAYS wait for the engine's "READY" line (title screen / welcome announcer
    // = all .fra content loaded) before sending any command. Sending earlier runs
    // engine code mid-load, which crashes.
    eprintln!("peptide-bridge: waiting for engine READY…");
    loop {
        match rx.recv_timeout(Duration::from_secs(60)) {
            Ok(l) => {
                println!("<< {l}");
                if l.contains("READY") {
                    eprintln!("peptide-bridge: engine is READY");
                    break;
                }
            }
            Err(_) => {
                eprintln!("peptide-bridge: timed out waiting for READY — aborting send");
                return;
            }
        }
    }
    // Optional delay AFTER READY before sending the command. READY fires from the
    // MainMenu constructor, but custom/workshop content (UGC) loads ASYNC during
    // the title sequence and may not be in the ResourceManager pool yet. Sending
    // `s <custom char>` too early => getPXFResource() null => spawnPlayer crash.
    // FRAY_POST_READY_DELAY (seconds) lets UGC finish before we send.
    if let Ok(d) = std::env::var("FRAY_POST_READY_DELAY") {
        if let Ok(secs) = d.trim().parse::<f64>() {
            if secs > 0.0 {
                eprintln!("peptide-bridge: post-READY delay {secs}s (let UGC load)…");
                thread::sleep(Duration::from_millis((secs * 1000.0) as u64));
            }
        }
    }

    let mut payload = cmd.to_string();
    payload.push('\n');
    write_half.write_all(payload.as_bytes()).expect("write");
    write_half.flush().ok();
    eprintln!("peptide-bridge: sent {cmd:?}");

    // Drain replies for ~1.5s of quiet, glossing recognized lines.
    loop {
        match rx.recv_timeout(Duration::from_millis(1500)) {
            Ok(l) => match gloss(&l) {
                Some(g) => println!("<< {l:<28} ({g})"),
                None => println!("<< {l}"),
            },
            Err(_) => break,
        }
    }
}
