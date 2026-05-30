//! frayremote — harness-side control bridge for Fraymakers.
//!
//! The injected engine code is a TCP *client* (this build's HashLink std has no
//! socket_listen/accept), so THIS program is the server: it binds a localhost
//! port, waits for the engine to dial in, then bridges a line-based protocol:
//!   - lines you type on stdin  -> sent to the engine
//!   - lines the engine sends   -> printed to stdout (prefixed with "<< ")
//!
//! Modes:
//!   frayremote serve [--port N]            interactive CLI (default)
//!   frayremote send  [--port N] "<cmd>"    connect, send one command, print
//!                                          replies for a short window, exit
//!                                          (for scripted / automated tests)

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

const DEFAULT_PORT: u16 = 17999;

fn parse_port(args: &[String]) -> u16 {
    args.iter()
        .position(|a| a == "--port")
        .and_then(|i| args.get(i + 1))
        .and_then(|p| p.parse().ok())
        .unwrap_or(DEFAULT_PORT)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(|s| s.as_str()).unwrap_or("serve");
    let port = parse_port(&args);

    let token = parse_token(&args);
    match mode {
        "serve" => serve(port, token.as_deref()),
        "send" => {
            // The command is the first positional arg (skip --flag and its value).
            let mut cmd: Option<String> = None;
            let mut i = 2;
            while i < args.len() {
                if args[i] == "--port" || args[i] == "--token" || args[i] == "--delay" {
                    i += 2;
                    continue;
                }
                cmd = Some(args[i].clone());
                break;
            }
            let cmd = cmd.unwrap_or_else(|| {
                eprintln!("usage: frayremote send [--port N] [--token T] \"<command>\"");
                std::process::exit(2);
            });
            send_once(port, token.as_deref(), &cmd);
        }
        other => {
            eprintln!("unknown mode: {other} (use 'serve' or 'send')");
            std::process::exit(2);
        }
    }
}

fn parse_token(args: &[String]) -> Option<String> {
    args.iter()
        .position(|a| a == "--token")
        .and_then(|i| args.get(i + 1))
        .cloned()
}

/// Bind (loopback only) and wait for the *authenticated* engine to connect.
///
/// Security: the listener binds 127.0.0.1 only (never reachable off-machine).
/// If a token is required, the peer's first line must be `AUTH <token>`; any
/// other connection is dropped and we keep listening. This prevents a local
/// impostor process from driving the engine over the loopback port. Callers
/// must bind BEFORE launching the engine so the port can't be squatted.
fn await_engine(port: u16, token: Option<&str>) -> (BufReader<TcpStream>, TcpStream) {
    let listener = TcpListener::bind(("127.0.0.1", port)).unwrap_or_else(|e| {
        eprintln!("frayremote: cannot bind 127.0.0.1:{port}: {e}");
        std::process::exit(1);
    });
    eprintln!("frayremote: listening on 127.0.0.1:{port} — waiting for Fraymakers…");
    loop {
        let (stream, peer) = listener.accept().expect("accept failed");
        let writer = stream.try_clone().expect("clone");
        let mut reader = BufReader::new(stream);
        match token {
            None => {
                eprintln!("frayremote: engine connected from {peer} (no auth)");
                return (reader, writer);
            }
            Some(expected) => {
                // The first line must be `AUTH <token>`. Keep the same buffered
                // reader afterwards so no read-ahead bytes are lost.
                let mut first = String::new();
                if reader.read_line(&mut first).is_ok()
                    && first.trim().strip_prefix("AUTH ").map(str::trim) == Some(expected)
                {
                    eprintln!("frayremote: engine authenticated from {peer}");
                    return (reader, writer);
                }
                eprintln!("frayremote: rejected unauthenticated peer {peer}; first line = {first:?}; still listening");
            }
        }
    }
}

/// Interactive bridge: stdin <-> socket, line based.
fn serve(port: u16, token: Option<&str>) {
    let (reader, mut write_half) = await_engine(port, token);

    // socket -> stdout; signal once the engine reports READY.
    let (ready_tx, ready_rx) = mpsc::channel::<()>();
    thread::spawn(move || {
        for line in reader.lines() {
            match line {
                Ok(l) => {
                    println!("<< {l}");
                    if l.contains("READY") {
                        let _ = ready_tx.send(());
                    }
                }
                Err(_) => break,
            }
        }
        eprintln!("frayremote: engine disconnected");
        std::process::exit(0);
    });

    // Always wait for READY before accepting any input.
    eprintln!("frayremote: waiting for engine READY…");
    match ready_rx.recv_timeout(Duration::from_secs(60)) {
        Ok(()) => eprintln!("frayremote: engine is READY — enter commands:"),
        Err(_) => {
            eprintln!("frayremote: timed out waiting for READY");
            return;
        }
    }

    // stdin -> socket
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(mut l) = line else { break };
        eprintln!("frayremote: SENT {l:?}");
        l.push('\n');
        if write_half.write_all(l.as_bytes()).is_err() {
            eprintln!("frayremote: write failed (engine gone?)");
            break;
        }
        let _ = write_half.flush();
    }
}

/// Scripted: send one command, print whatever comes back for a short window.
fn send_once(port: u16, token: Option<&str>, cmd: &str) {
    let (reader, mut write_half) = await_engine(port, token);

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
    eprintln!("frayremote: waiting for engine READY…");
    loop {
        match rx.recv_timeout(Duration::from_secs(60)) {
            Ok(l) => {
                println!("<< {l}");
                if l.contains("READY") {
                    eprintln!("frayremote: engine is READY");
                    break;
                }
            }
            Err(_) => {
                eprintln!("frayremote: timed out waiting for READY — aborting send");
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
                eprintln!("frayremote: post-READY delay {secs}s (let UGC load)…");
                thread::sleep(Duration::from_millis((secs * 1000.0) as u64));
            }
        }
    }

    let mut payload = cmd.to_string();
    payload.push('\n');
    write_half.write_all(payload.as_bytes()).expect("write");
    write_half.flush().ok();
    eprintln!("frayremote: sent {cmd:?}");

    // Drain replies for ~1.5s of quiet.
    loop {
        match rx.recv_timeout(Duration::from_millis(1500)) {
            Ok(l) => println!("<< {l}"),
            Err(_) => break,
        }
    }
}
