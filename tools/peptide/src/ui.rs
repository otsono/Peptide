//! peptide-ui — a friendly full-screen console for driving Fraymakers.
//!
//! Type commands (or raw hscript) at the bottom; the engine's replies stream into
//! the scrollback above, color-coded and glossed. Built for beginners: a command
//! palette (Tab), inline hints, history (↑/↓), and a help panel (F1).
//!
//! Launched by run-ui.sh, which patches + boots the engine and then runs
//! `peptide-bridge ui`. This module owns the terminal once the engine connects.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::commands::{translate, Translated};

/// How the engine launcher (run-ui.sh) tells us the spawn defaults, just for the
/// quick-start hint. Optional.
fn env_char() -> Option<String> {
    std::env::var("FRAY_CHAR").ok().filter(|s| !s.is_empty())
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Sent,
    Result,
    Error,
    Anim,
    Move,
    Physics,
    State,
    Launched,
    System,
}

struct Out {
    text: String,
    kind: Kind,
}

struct App {
    lines: Vec<Out>,
    input: String,
    history: Vec<String>,
    hist_pos: Option<usize>,
    scroll_back: u16, // how many lines scrolled up from the bottom
    show_help: bool,
    show_palette: bool,
    palette_sel: usize,
    port: u16,
    char_name: Option<String>,
    ready: bool,
}

impl App {
    fn new(port: u16) -> Self {
        let mut app = App {
            lines: Vec::new(),
            input: String::new(),
            history: Vec::new(),
            hist_pos: None,
            scroll_back: 0,
            show_help: false,
            show_palette: false,
            palette_sel: 0,
            port,
            char_name: env_char(),
            ready: false,
        };
        app.sys("Welcome to Peptide. The engine is loading…");
        app.sys("Type a command and press Enter. Tab = command palette · F1 = help.");
        app
    }

    fn push(&mut self, text: String, kind: Kind) {
        self.lines.push(Out { text, kind });
        self.scroll_back = 0; // jump back to live tail on new output
    }
    fn sys(&mut self, s: &str) {
        self.push(s.to_string(), Kind::System);
    }

    /// Classify + record a line that came FROM the engine.
    fn engine_line(&mut self, line: &str) {
        if line.contains("READY") {
            self.ready = true;
            self.sys("Engine ready. Try: spawn sandbag");
        }
        if let Some(rest) = line.strip_prefix("LAUNCHED") {
            // pull the char id (private::<id>.<id>) for the status bar
            if let Some(tok) = rest.split_whitespace().next() {
                let id = tok.rsplit("::").next().unwrap_or(tok);
                let id = id.split('.').next().unwrap_or(id);
                if !id.is_empty() {
                    self.char_name = Some(id.to_string());
                }
            }
        }
        let kind = if line.contains("ERR") {
            Kind::Error
        } else if line.starts_with("LAUNCHED") {
            Kind::Launched
        } else if line.starts_with("E:") {
            Kind::Result
        } else if line.starts_with("ANIM:") {
            Kind::Anim
        } else if line.starts_with("M:") {
            Kind::Move
        } else if line.starts_with("P:") {
            Kind::Physics
        } else if line.starts_with("T:") {
            Kind::State
        } else {
            Kind::System
        };
        // Trim the noisy "E:" prefix for display (keep ERR visible).
        let shown = line.strip_prefix("E:").unwrap_or(line).to_string();
        self.push(shown, kind);
    }
}

const PALETTE: &[(&str, &str)] = &[
    ("spawn sandbag", "start a match with sandbag"),
    ("state", "current state of player 0"),
    ("physics", "position / velocity / damage"),
    ("anim", "current animation + frame"),
    ("move jab", "drive a move (jab, tilt_up, special_neutral, …)"),
    ("play", "resume animation playback"),
    ("step", "advance one animation frame"),
    ("match.getCharacters()", "list the characters in the match (hscript)"),
    ("p0.getStateName()", "call any method on player 0 (hscript)"),
    ("Engine.log(\"hi\")", "log to the in-game console"),
    ("exit", "shut the engine down"),
];

pub fn run(port: u16, token: Option<&str>) -> std::io::Result<()> {
    eprintln!("peptide-ui: launching Fraymakers and waiting for it to connect…");
    let (reader, writer) = crate::await_engine(port, token);

    // socket -> channel (raw lines); dedup repeated ANIM frames.
    let (tx, rx): (mpsc::Sender<String>, Receiver<String>) = mpsc::channel();
    let mut byte_reader = reader;
    thread::spawn(move || {
        let mut buf: Vec<u8> = Vec::with_capacity(256);
        let mut one = [0u8; 1];
        let mut last_anim = String::new();
        loop {
            match byte_reader.read(&mut one) {
                Ok(0) => {
                    let _ = tx.send("\u{1}engine closed the connection".to_string());
                    break;
                }
                Ok(_) => {
                    if one[0] == b'\n' {
                        let line = String::from_utf8_lossy(&buf).trim_end_matches('\r').to_string();
                        buf.clear();
                        if let Some(a) = line.strip_prefix("ANIM:") {
                            if a == last_anim {
                                continue;
                            }
                            last_anim = a.to_string();
                        }
                        if tx.send(line).is_err() {
                            break;
                        }
                    } else {
                        buf.push(one[0]);
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
    });

    let mut term = ratatui::init();
    let mut app = App::new(port);
    let mut writer = writer;
    let res = event_loop(&mut term, &mut app, &rx, &mut writer);
    ratatui::restore();
    // be polite: ask the engine to exit cleanly on the way out
    let _ = writer.write_all(b"x\n");
    res
}

fn event_loop(
    term: &mut ratatui::DefaultTerminal,
    app: &mut App,
    rx: &Receiver<String>,
    writer: &mut TcpStream,
) -> std::io::Result<()> {
    loop {
        // drain engine output
        while let Ok(line) = rx.try_recv() {
            if let Some(sys) = line.strip_prefix('\u{1}') {
                app.sys(sys);
            } else {
                app.engine_line(&line);
            }
        }

        term.draw(|f| draw(f, app))?;

        if event::poll(Duration::from_millis(40))? {
            if let Event::Key(k) = event::read()? {
                if k.kind != KeyEventKind::Press {
                    continue;
                }
                let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
                match k.code {
                    KeyCode::Char('c') if ctrl => return Ok(()),
                    KeyCode::Char('d') if ctrl => return Ok(()),
                    KeyCode::Esc => {
                        if app.show_help || app.show_palette {
                            app.show_help = false;
                            app.show_palette = false;
                        } else {
                            return Ok(());
                        }
                    }
                    KeyCode::F(1) => app.show_help = !app.show_help,
                    KeyCode::Tab => {
                        app.show_palette = !app.show_palette;
                        app.palette_sel = 0;
                    }
                    KeyCode::Up if app.show_palette => {
                        app.palette_sel = app.palette_sel.saturating_sub(1);
                    }
                    KeyCode::Down if app.show_palette => {
                        app.palette_sel = (app.palette_sel + 1).min(PALETTE.len() - 1);
                    }
                    KeyCode::Enter if app.show_palette => {
                        app.input = PALETTE[app.palette_sel].0.to_string();
                        app.show_palette = false;
                    }
                    KeyCode::Up => history_prev(app),
                    KeyCode::Down => history_next(app),
                    KeyCode::PageUp => app.scroll_back = app.scroll_back.saturating_add(8),
                    KeyCode::PageDown => app.scroll_back = app.scroll_back.saturating_sub(8),
                    KeyCode::Backspace => {
                        app.input.pop();
                    }
                    KeyCode::Char(c) => app.input.push(c),
                    KeyCode::Enter => submit(app, writer)?,
                    _ => {}
                }
            }
        }
    }
}

fn history_prev(app: &mut App) {
    if app.history.is_empty() {
        return;
    }
    let pos = match app.hist_pos {
        None => app.history.len() - 1,
        Some(0) => 0,
        Some(p) => p - 1,
    };
    app.hist_pos = Some(pos);
    app.input = app.history[pos].clone();
}

fn history_next(app: &mut App) {
    match app.hist_pos {
        Some(p) if p + 1 < app.history.len() => {
            app.hist_pos = Some(p + 1);
            app.input = app.history[p + 1].clone();
        }
        _ => {
            app.hist_pos = None;
            app.input.clear();
        }
    }
}

fn submit(app: &mut App, writer: &mut TcpStream) -> std::io::Result<()> {
    let line = app.input.trim().to_string();
    app.input.clear();
    app.hist_pos = None;
    if line.is_empty() {
        return Ok(());
    }
    app.history.push(line.clone());
    app.push(format!("› {line}"), Kind::Sent);

    match translate(&line) {
        Translated::Wire(w) => send(writer, &w)?,
        Translated::Sequence(v) => {
            for w in v {
                send(writer, &w)?;
            }
        }
        Translated::Repeat { wire, count, gap_ms } => {
            // fire on a background thread so the UI stays responsive
            if let Ok(mut w) = writer.try_clone() {
                thread::spawn(move || {
                    for _ in 0..count {
                        if w.write_all(format!("{wire}\n").as_bytes()).is_err() {
                            break;
                        }
                        let _ = w.flush();
                        thread::sleep(Duration::from_millis(gap_ms));
                    }
                });
            }
        }
        Translated::Client(text) => {
            for l in text.lines() {
                app.sys(l);
            }
        }
        Translated::Error(e) => app.push(e, Kind::Error),
    }
    Ok(())
}

fn send(writer: &mut TcpStream, wire: &str) -> std::io::Result<()> {
    if wire.is_empty() {
        return Ok(());
    }
    writer.write_all(format!("{wire}\n").as_bytes())?;
    writer.flush()
}

fn draw(f: &mut Frame, app: &App) {
    let chunks = Layout::vertical([
        Constraint::Length(1), // title
        Constraint::Min(3),    // scrollback
        Constraint::Length(3), // input
        Constraint::Length(1), // hints
    ])
    .split(f.area());

    draw_title(f, chunks[0], app);
    draw_scrollback(f, chunks[1], app);
    draw_input(f, chunks[2], app);
    draw_hints(f, chunks[3], app);

    if app.show_palette {
        draw_palette(f, app);
    }
    if app.show_help {
        draw_help(f);
    }
}

fn draw_title(f: &mut Frame, area: Rect, app: &App) {
    let dot = if app.ready { "●".green() } else { "◌".yellow() };
    let status = if app.ready { "ready" } else { "loading…" };
    let chr = app.char_name.clone().unwrap_or_else(|| "—".into());
    let line = Line::from(vec![
        " Peptide ".bold().on_dark_gray(),
        "  ".into(),
        dot,
        format!(" {status}").into(),
        "   char: ".dark_gray(),
        chr.cyan(),
        "   port: ".dark_gray(),
        format!(":{}", app.port).into(),
    ]);
    f.render_widget(Paragraph::new(line), area);
}

fn draw_scrollback(f: &mut Frame, area: Rect, app: &App) {
    let inner_h = area.height.saturating_sub(2) as usize; // minus borders
    let mut lines: Vec<Line> = Vec::with_capacity(app.lines.len());
    for o in &app.lines {
        let style = match o.kind {
            Kind::Sent => Style::default().fg(Color::Cyan).add_modifier(Modifier::DIM),
            Kind::Result => Style::default().fg(Color::Green),
            Kind::Error => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            Kind::Anim => Style::default().fg(Color::Yellow),
            Kind::Move => Style::default().fg(Color::LightMagenta),
            Kind::Physics => Style::default().fg(Color::LightBlue),
            Kind::State => Style::default().fg(Color::LightCyan),
            Kind::Launched => Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD),
            Kind::System => Style::default().fg(Color::DarkGray),
        };
        let prefix = match o.kind {
            Kind::Sent => "",
            Kind::System => "  ",
            _ => "« ",
        };
        lines.push(Line::styled(format!("{prefix}{}", o.text), style));
    }
    let total = lines.len();
    // bottom-anchored scroll: show the tail unless the user scrolled up
    let max_top = total.saturating_sub(inner_h);
    let top = max_top.saturating_sub(app.scroll_back as usize);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(" output ".dark_gray());
    let para = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((top as u16, 0));
    f.render_widget(para, area);
}

fn draw_input(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Blue))
        .title(" command ".blue().bold());
    let text = Line::from(vec!["❯ ".blue().bold(), app.input.clone().white()]);
    f.render_widget(Paragraph::new(text).block(block), area);
    // cursor after the prompt + input
    let cx = area.x + 4 + app.input.chars().count() as u16;
    let cy = area.y + 1;
    f.set_cursor_position((cx.min(area.x + area.width.saturating_sub(2)), cy));
}

fn draw_hints(f: &mut Frame, area: Rect, _app: &App) {
    let hint = Line::from(vec![
        " Tab ".on_dark_gray(),
        " palette  ".dark_gray(),
        " ↑↓ ".on_dark_gray(),
        " history  ".dark_gray(),
        " PgUp/PgDn ".on_dark_gray(),
        " scroll  ".dark_gray(),
        " F1 ".on_dark_gray(),
        " help  ".dark_gray(),
        " Esc/Ctrl+C ".on_dark_gray(),
        " quit".dark_gray(),
    ]);
    f.render_widget(Paragraph::new(hint), area);
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    }
}

fn draw_palette(f: &mut Frame, app: &App) {
    let area = centered(f.area(), 62, (PALETTE.len() + 2) as u16);
    f.render_widget(Clear, area);
    let mut lines = Vec::new();
    for (i, (cmd, desc)) in PALETTE.iter().enumerate() {
        let sel = i == app.palette_sel;
        let marker = if sel { "❯ " } else { "  " };
        let style = if sel {
            Style::default().fg(Color::Black).bg(Color::Cyan)
        } else {
            Style::default().fg(Color::White)
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{marker}{cmd:<26}"), style),
            Span::styled(format!(" {desc}"), Style::default().fg(Color::DarkGray)),
        ]));
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(" command palette · Enter to pick · Esc to close ".cyan());
    f.render_widget(Paragraph::new(lines).block(block), area);
}

fn draw_help(f: &mut Frame) {
    let area = centered(f.area(), 70, 18);
    f.render_widget(Clear, area);
    let text = vec![
        Line::from("Peptide drives a running Fraymakers from this console.".bold()),
        Line::from(""),
        Line::from(vec!["spawn <char> ".cyan(), "start a match (e.g. spawn sandbag)".dark_gray()]),
        Line::from(vec!["state / physics / anim ".cyan(), "read player 0".dark_gray()]),
        Line::from(vec!["move <name> ".cyan(), "jab, tilt_up, strong_forward, special_neutral, aerial_up, grab".dark_gray()]),
        Line::from(vec!["play / step ".cyan(), "resume / advance one animation frame".dark_gray()]),
        Line::from(vec!["exit ".cyan(), "shut the engine down".dark_gray()]),
        Line::from(""),
        Line::from("Anything else is run as hscript in the engine:".bold()),
        Line::from(vec!["  match.getCharacters() ".green(), "list the characters".dark_gray()]),
        Line::from(vec!["  p0.getStateName() ".green(), "call any method on player 0".dark_gray()]),
        Line::from(vec!["  p0.body.x ".green(), "read any field".dark_gray()]),
        Line::from(vec!["  Engine.log(\"hi\") ".green(), "log to the in-game console".dark_gray()]),
        Line::from(""),
        Line::from("Errors never crash the engine — they show here and in Engine.log.".italic()),
        Line::from("Press F1 or Esc to close.".dark_gray()),
    ];
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green))
        .title(" help ".green().bold());
    f.render_widget(Paragraph::new(text).block(block).wrap(Wrap { trim: false }), area);
}
