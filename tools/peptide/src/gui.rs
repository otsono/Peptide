//! gui — the graphical chat console (the default `peptide` mode): a native window
//! (egui/eframe), themed after Claude — warm paper background, clay/coral accent,
//! proportional type, rounded message bubbles, and buttons. Cross-platform, one binary.
//!
//! Live: `gui::launch()` boots the engine (ui::boot) and drives it over the socket;
//! a reader thread streams replies into the chat. PEPTIDE_GUI_DEMO=1 shows sample
//! content without an engine (for design iteration).

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use eframe::egui;
use egui::{Color32, CornerRadius, Margin, RichText, Vec2};

use crate::commands::{translate, Translated};

// ── Claude-ish palette ──────────────────────────────────────────────────────
const PAPER: Color32 = Color32::from_rgb(0xF0, 0xEE, 0xE6);
const CARD: Color32 = Color32::from_rgb(0xFF, 0xFF, 0xFF);
const CLAY: Color32 = Color32::from_rgb(0xCC, 0x78, 0x5C);
const INK: Color32 = Color32::from_rgb(0x2B, 0x29, 0x24);
const MUTED: Color32 = Color32::from_rgb(0x8A, 0x84, 0x78);
const BORDER: Color32 = Color32::from_rgb(0xE3, 0xDF, 0xD4);
const CHIP: Color32 = Color32::from_rgb(0xE9, 0xE6, 0xDB);
const ERR_BG: Color32 = Color32::from_rgb(0xFB, 0xE9, 0xE7);
const ERR_INK: Color32 = Color32::from_rgb(0xB0, 0x3A, 0x2E);
const OK_DOT: Color32 = Color32::from_rgb(0x4C, 0xAF, 0x6E);

pub fn launch() -> std::io::Result<()> {
    let demo = std::env::var("PEPTIDE_GUI_DEMO").is_ok();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([900.0, 660.0])
            .with_min_inner_size([560.0, 420.0])
            .with_title("Peptide"),
        ..Default::default()
    };

    if demo {
        return eframe::run_native("Peptide", options, Box::new(|cc| {
            setup_theme(&cc.egui_ctx);
            Ok(Box::new(ChatApp::demo()))
        }))
        .map_err(to_io);
    }

    // Live: boot the engine + connect, then stream replies in on a reader thread.
    let (reader, writer, port, cleanup) = crate::ui::boot()?;
    eframe::run_native("Peptide", options, Box::new(move |cc| {
        setup_theme(&cc.egui_ctx);
        let (tx, rx) = mpsc::channel::<String>();
        spawn_reader(reader, tx, cc.egui_ctx.clone());
        Ok(Box::new(ChatApp::live(writer, rx, port, cleanup)))
    }))
    .map_err(to_io)
}

fn to_io(e: eframe::Error) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Other, e.to_string())
}

/// Socket bytes -> lines -> channel; dedup repeated per-frame ANIM lines; wake the UI.
fn spawn_reader(mut reader: std::io::BufReader<TcpStream>, tx: mpsc::Sender<String>, ctx: egui::Context) {
    thread::spawn(move || {
        let mut buf: Vec<u8> = Vec::with_capacity(256);
        let mut one = [0u8; 1];
        let mut last_anim = String::new();
        loop {
            match reader.read(&mut one) {
                Ok(0) => {
                    let _ = tx.send("\u{1}disconnected".into());
                    ctx.request_repaint();
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
                        if tx.send(line).is_err() { break; }
                        ctx.request_repaint();
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

fn setup_theme(ctx: &egui::Context) {
    use egui::{FontFamily, FontId, TextStyle};
    let mut style = (*ctx.style()).clone();
    let mut v = egui::Visuals::light();
    v.panel_fill = PAPER;
    v.window_fill = PAPER;
    v.extreme_bg_color = CARD;
    v.faint_bg_color = CHIP;
    v.override_text_color = Some(INK);
    v.hyperlink_color = CLAY;
    v.selection.bg_fill = CLAY.gamma_multiply(0.35);
    v.selection.stroke = egui::Stroke::new(1.0, CLAY);
    let r = CornerRadius::same(10);
    for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = r;
        w.bg_stroke = egui::Stroke::new(1.0, BORDER);
    }
    v.widgets.noninteractive.corner_radius = r;
    v.widgets.inactive.bg_fill = CHIP;
    v.widgets.hovered.bg_fill = CHIP.gamma_multiply(0.85);
    v.window_corner_radius = CornerRadius::same(12);
    ctx.set_visuals(v);
    style.spacing.item_spacing = Vec2::new(8.0, 8.0);
    style.spacing.button_padding = Vec2::new(12.0, 7.0);
    style.spacing.window_margin = Margin::same(14);
    style.text_styles.insert(TextStyle::Heading, FontId::new(22.0, FontFamily::Proportional));
    style.text_styles.insert(TextStyle::Body, FontId::new(15.0, FontFamily::Proportional));
    style.text_styles.insert(TextStyle::Button, FontId::new(14.0, FontFamily::Proportional));
    style.text_styles.insert(TextStyle::Monospace, FontId::new(13.5, FontFamily::Monospace));
    ctx.set_style(style);
}

#[derive(Clone, Copy, PartialEq)]
enum Role { You, Engine, System }

struct Msg { role: Role, text: String, code: bool, error: bool }

struct ChatApp {
    messages: Vec<Msg>,
    input: String,
    connected: bool,
    char_name: String,
    port: u16,
    writer: Option<TcpStream>,
    rx: Option<Receiver<String>>,
    _cleanup: Option<crate::ui::Cleanup>,
    auto_spawned: bool,
}

const QUICK: &[(&str, &str)] = &[
    ("Spawn", "spawn sandbag"),
    ("State", "state"),
    ("Physics", "physics"),
    ("Anim", "anim"),
    ("Jab", "move jab"),
    ("Characters", "match.getCharacters()"),
];

impl ChatApp {
    fn live(writer: TcpStream, rx: Receiver<String>, port: u16, cleanup: crate::ui::Cleanup) -> Self {
        ChatApp {
            messages: vec![Msg { role: Role::System,
                text: "Connecting to Fraymakers…".into(), code: false, error: false }],
            input: String::new(),
            connected: false,
            char_name: std::env::var("FRAY_CHAR").unwrap_or_else(|_| "sandbag".into()),
            port,
            writer: Some(writer),
            rx: Some(rx),
            _cleanup: Some(cleanup),
            auto_spawned: false,
        }
    }

    fn demo() -> Self {
        let m = |role, text: &str, code, error| Msg { role, text: text.into(), code, error };
        ChatApp {
            input: String::new(),
            connected: true,
            char_name: "sandbag".into(),
            port: 19794,
            writer: None,
            rx: None,
            _cleanup: None,
            auto_spawned: true,
            messages: vec![
                m(Role::System, "Engine ready — try a button below, or type a command.", false, false),
                m(Role::You, "spawn sandbag", false, false),
                m(Role::Engine, "Match launched: sandbag on thespire", false, false),
                m(Role::You, "physics", false, false),
                m(Role::Engine, "x=-130  y=73  vx=0  vy=0  dmg=0", true, false),
                m(Role::You, "match.getCharacters()", false, false),
                m(Role::Engine, "[ FraymakersCharacter ]", true, false),
                m(Role::You, ")(oops", false, false),
                m(Role::Engine, "hscript:1: Unexpected token: \")\"", true, true),
            ],
        }
    }

    /// Classify + record a line from the engine.
    fn ingest(&mut self, line: &str) {
        if let Some(sys) = line.strip_prefix('\u{1}') {
            self.connected = false;
            self.messages.push(Msg { role: Role::System, text: format!("● {sys}"), code: false, error: false });
            return;
        }
        if line.contains("READY") {
            self.connected = true;
            if !self.auto_spawned {
                self.auto_spawned = true;
                self.messages.push(Msg { role: Role::System,
                    text: format!("Engine ready — spawning {}…", self.char_name), code: false, error: false });
                self.send_text("spawn".into());
            }
            return;
        }
        if line == "HELLO_FROM_FRAYMAKERS" { return; }
        if let Some(rest) = line.strip_prefix("LAUNCHED") {
            if let Some(tok) = rest.split_whitespace().next() {
                let id = tok.rsplit("::").next().unwrap_or(tok).split('.').next().unwrap_or(tok);
                if !id.is_empty() { self.char_name = id.to_string(); }
            }
            self.messages.push(Msg { role: Role::Engine,
                text: format!("Match launched · {}", self.char_name), code: false, error: false });
            return;
        }
        let error = line.contains("ERR");
        let body = line.strip_prefix("E:").unwrap_or(line);
        let body = body.strip_prefix("ERR: ").unwrap_or(body);
        // value-ish lines render in a monospace card
        let code = error || line.starts_with("E:") || line.starts_with("P:")
            || line.starts_with("T:") || line.starts_with("A:") || line.starts_with("M:");
        self.messages.push(Msg { role: Role::Engine, text: body.to_string(), code, error });
    }

    fn send_wire(&mut self, wire: &str) {
        if wire.is_empty() { return; }
        if let Some(w) = self.writer.as_mut() {
            let _ = w.write_all(format!("{wire}\n").as_bytes());
            let _ = w.flush();
        }
    }

    fn send_text(&mut self, text: String) {
        let t = text.trim().to_string();
        if t.is_empty() { return; }
        self.messages.push(Msg { role: Role::You, text: t.clone(), code: false, error: false });
        match translate(&t) {
            Translated::Wire(w) => self.send_wire(&w),
            Translated::Sequence(v) => for w in v { self.send_wire(&w); },
            Translated::Repeat { wire, count, gap_ms } => {
                if let Some(w) = self.writer.as_ref().and_then(|w| w.try_clone().ok()) {
                    let mut w = w;
                    thread::spawn(move || {
                        for _ in 0..count {
                            if w.write_all(format!("{wire}\n").as_bytes()).is_err() { break; }
                            let _ = w.flush();
                            thread::sleep(Duration::from_millis(gap_ms));
                        }
                    });
                }
            }
            Translated::Client(text) => for l in text.lines() {
                self.messages.push(Msg { role: Role::System, text: l.into(), code: false, error: false });
            },
            Translated::Error(e) => self.messages.push(Msg { role: Role::Engine, text: e, code: true, error: true }),
        }
    }
}

impl eframe::App for ChatApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // drain engine replies
        let mut lines = Vec::new();
        if let Some(rx) = &self.rx {
            while let Ok(l) = rx.try_recv() { lines.push(l); }
        }
        for l in lines { self.ingest(&l); }

        top_bar(self, ui);
        bottom_bar(self, ui);
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(PAPER).inner_margin(Margin::symmetric(18, 10)))
            .show_inside(ui, |ui| {
                egui::ScrollArea::vertical().stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
                    ui.add_space(4.0);
                    for i in 0..self.messages.len() {
                        bubble(ui, &self.messages[i]);
                        ui.add_space(6.0);
                    }
                });
            });
        // keep polling the socket channel even when idle
        ui.ctx().request_repaint_after(Duration::from_millis(80));
    }
}

fn top_bar(app: &ChatApp, ui: &mut egui::Ui) {
    egui::TopBottomPanel::top("top")
        .frame(egui::Frame::new().fill(PAPER).inner_margin(Margin::symmetric(18, 12)))
        .show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(26.0), egui::Sense::hover());
                ui.painter().rect_filled(rect, CornerRadius::same(8), CLAY);
                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "P",
                    egui::FontId::proportional(16.0), Color32::WHITE);
                ui.add_space(6.0);
                ui.heading("Peptide");
                ui.add_space(6.0);
                ui.label(RichText::new("Fraymakers console").color(MUTED).size(13.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| status_pill(ui, app));
            });
        });
}

fn status_pill(ui: &mut egui::Ui, app: &ChatApp) {
    let (dot, label, fill) = if app.connected {
        (OK_DOT, format!("{}  ·  :{}", app.char_name, app.port), Color32::from_rgb(0xE8, 0xF3, 0xEA))
    } else {
        (MUTED, "connecting…".into(), CHIP)
    };
    egui::Frame::new().fill(fill).corner_radius(CornerRadius::same(12)).inner_margin(Margin::symmetric(10, 4)).show(ui, |ui| {
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(Vec2::splat(8.0), egui::Sense::hover());
            ui.painter().circle_filled(r.center(), 4.0, dot);
            ui.label(RichText::new(label).color(INK).size(12.5));
        });
    });
}

fn bottom_bar(app: &mut ChatApp, ui: &mut egui::Ui) {
    egui::TopBottomPanel::bottom("bottom")
        .frame(egui::Frame::new().fill(PAPER).inner_margin(Margin::symmetric(18, 12)))
        .show_inside(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                for (label, cmd) in QUICK {
                    let btn = egui::Button::new(RichText::new(*label).size(13.0).color(INK))
                        .fill(CHIP).stroke(egui::Stroke::new(1.0, BORDER)).corner_radius(CornerRadius::same(14));
                    if ui.add(btn).clicked() { app.send_text((*cmd).to_string()); }
                }
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let send_w = 88.0;
                let mut go = false;
                let mut resp = None;
                egui::Frame::new().fill(CARD).stroke(egui::Stroke::new(1.0, BORDER))
                    .corner_radius(CornerRadius::same(11)).inner_margin(Margin::symmetric(12, 9)).show(ui, |ui| {
                        let avail = ui.available_width() - send_w - 4.0;
                        let edit = egui::TextEdit::singleline(&mut app.input)
                            .hint_text(RichText::new("Message Peptide  —  a command or raw hscript…").color(MUTED))
                            .desired_width(avail).frame(egui::Frame::NONE).font(egui::TextStyle::Body);
                        let r = ui.add(edit);
                        go = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        resp = Some(r);
                        let send = egui::Button::new(RichText::new("Send").color(Color32::WHITE).size(14.0).strong())
                            .fill(CLAY).corner_radius(CornerRadius::same(9)).min_size(Vec2::new(send_w, 30.0));
                        if ui.add(send).on_hover_text("Send  (Enter)").clicked() { go = true; }
                    });
                if go {
                    let t = std::mem::take(&mut app.input);
                    app.send_text(t);
                    if let Some(r) = resp { r.request_focus(); }
                }
            });
            ui.add_space(2.0);
        });
}

fn bubble(ui: &mut egui::Ui, m: &Msg) {
    match m.role {
        Role::System => { ui.vertical_centered(|ui| {
            ui.label(RichText::new(&m.text).color(MUTED).italics().size(13.0)); }); }
        Role::You => { ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
            let max = ui.available_width() * 0.78;
            egui::Frame::new().fill(CLAY)
                .corner_radius(CornerRadius { nw: 14, ne: 14, sw: 14, se: 4 })
                .inner_margin(Margin::symmetric(13, 9)).show(ui, |ui| {
                    ui.set_max_width(max);
                    ui.label(RichText::new(&m.text).color(Color32::WHITE).size(15.0));
                });
        }); }
        Role::Engine => { ui.with_layout(egui::Layout::left_to_right(egui::Align::TOP), |ui| {
            let max = ui.available_width() * 0.82;
            let (fill, ink) = if m.error { (ERR_BG, ERR_INK) } else { (CARD, INK) };
            egui::Frame::new().fill(fill)
                .stroke(egui::Stroke::new(1.0, if m.error { Color32::from_rgb(0xF0, 0xC4, 0xBE) } else { BORDER }))
                .corner_radius(CornerRadius { nw: 14, ne: 14, sw: 4, se: 14 })
                .inner_margin(Margin::symmetric(13, 9)).show(ui, |ui| {
                    ui.set_max_width(max);
                    ui.vertical(|ui| {
                        ui.label(RichText::new("engine").color(MUTED).size(11.0));
                        if m.code { ui.label(RichText::new(&m.text).monospace().color(ink).size(13.5)); }
                        else { ui.label(RichText::new(&m.text).color(ink).size(15.0)); }
                    });
                });
        }); }
    }
}
