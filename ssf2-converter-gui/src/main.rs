//! Cross-platform GUI for the SSF2 → Fraymakers converter.
//!
//! A pure-Rust egui/eframe app (Windows / macOS / Linux) that mirrors the
//! cross-platform egui app: drag-and-drop an .ssf, convert it (by shelling out to
//! the bundled `ssf2_converter` binary), optionally publish straight into the
//! Fraymakers custom-content folder, and export the .fra via the FrayTools
//! harness.
//!
//! Ships as a single self-contained executable with no runtime dependencies
//! (unlike a webview/Electron app), which makes the Windows story trivial.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // no console window on Windows release

mod platform;
mod prefs;

use prefs::Prefs;
use serde::Deserialize;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{channel, Receiver, Sender};

// ─── Theme ────────────────────────────────────────────────────────────────────

/// macOS-system accent blue (same in dark mode).
const ACCENT: egui::Color32 = egui::Color32::from_rgb(0x0A, 0x84, 0xFF);
/// Deep near-black window background + elevated card surface for a premium,
/// high-contrast dark look. Cards sit a step above the backdrop.
const BG: egui::Color32 = egui::Color32::from_rgb(0x0E, 0x0E, 0x12);
const CARD: egui::Color32 = egui::Color32::from_rgb(0x1B, 0x1B, 0x21);
const HAIRLINE: egui::Color32 = egui::Color32::from_rgb(0x2E, 0x2E, 0x36);
const HEADER_BG: egui::Color32 = egui::Color32::from_rgb(0x15, 0x15, 0x1A);
const TEXT: egui::Color32 = egui::Color32::from_rgb(0xEC, 0xEC, 0xF0);
/// Blue tint behind the drop zone while a file is hovering.
const DROP_HOVER: egui::Color32 = egui::Color32::from_rgb(0x16, 0x26, 0x40);
const OK_GREEN: egui::Color32 = egui::Color32::from_rgb(0x32, 0xD7, 0x4B);
const ERR_RED: egui::Color32 = egui::Color32::from_rgb(0xFF, 0x45, 0x3A);

/// Clean, dark, SwiftUI-ish styling — generous spacing, rounded widgets,
/// system-blue accent, comfortable type scale. Pure egui, no extra crates.
fn setup_style(ctx: &egui::Context) {
    use egui::{FontFamily::Proportional, FontFamily::Monospace, FontId, TextStyle};

    // Bundle Roboto (Apache-2.0 — static .ttf, not a crate dep): Regular as the
    // primary proportional font, Bold under a named "bold" family for headings.
    // egui's default Ubuntu-Light + emoji fonts stay as fallbacks (✓ / 📦 / ⬇).
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert("Roboto".to_owned(),
        egui::FontData::from_static(include_bytes!("../assets/Roboto-Regular.ttf")));
    fonts.font_data.insert("Roboto-Medium".to_owned(),
        egui::FontData::from_static(include_bytes!("../assets/Roboto-Medium.ttf")));
    fonts.font_data.insert("Roboto-Bold".to_owned(),
        egui::FontData::from_static(include_bytes!("../assets/Roboto-Bold.ttf")));
    fonts.families.entry(Proportional).or_default().insert(0, "Roboto".to_owned());
    fonts.families.insert(egui::FontFamily::Name("bold".into()),
        vec!["Roboto-Bold".to_owned(), "Roboto".to_owned()]);
    fonts.families.insert(egui::FontFamily::Name("medium".into()),
        vec!["Roboto-Medium".to_owned(), "Roboto".to_owned()]);
    ctx.set_fonts(fonts);

    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::dark();
    style.visuals.panel_fill = BG;
    style.visuals.window_fill = CARD;
    style.visuals.extreme_bg_color = BG;
    style.visuals.hyperlink_color = ACCENT;
    // Crisp label/button text (but leave weak() + colored labels to their own hues).
    style.visuals.widgets.noninteractive.fg_stroke.color = TEXT;
    style.visuals.widgets.inactive.fg_stroke.color = TEXT;
    style.visuals.selection.bg_fill = egui::Color32::from_rgb(0x0A, 0x46, 0x86);
    style.visuals.selection.stroke = egui::Stroke::new(1.0, ACCENT);
    style.visuals.window_rounding = egui::Rounding::same(14.0);
    style.visuals.window_shadow = egui::epaint::Shadow {
        offset: egui::vec2(0.0, 6.0), blur: 32.0, spread: 0.0,
        color: egui::Color32::from_black_alpha(90),
    };
    // Rounded, calm widget surfaces; subtle elevation on buttons.
    let round = egui::Rounding::same(8.0);
    for w in [
        &mut style.visuals.widgets.noninteractive,
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
        &mut style.visuals.widgets.open,
    ] {
        w.rounding = round;
    }
    style.visuals.widgets.inactive.weak_bg_fill = egui::Color32::from_rgb(0x2A, 0x2A, 0x31);
    style.visuals.widgets.hovered.weak_bg_fill = egui::Color32::from_rgb(0x34, 0x34, 0x3C);
    // Generous, airy spacing for a premium feel.
    style.spacing.item_spacing = egui::vec2(12.0, 12.0);
    style.spacing.button_padding = egui::vec2(16.0, 9.0);
    style.spacing.interact_size.y = 32.0;
    // Type scale — bold Roboto headings, clean regular body.
    let bold = egui::FontFamily::Name("bold".into());
    style.text_styles = [
        (TextStyle::Heading,   FontId::new(27.0, bold.clone())),
        (TextStyle::Body,      FontId::new(14.5, Proportional)),
        (TextStyle::Button,    FontId::new(14.5, egui::FontFamily::Name("medium".into()))),
        (TextStyle::Small,     FontId::new(12.0, Proportional)),
        (TextStyle::Monospace, FontId::new(12.5, Monospace)),
    ].into();
    ctx.set_style(style);
}

/// A bold Roboto `FontId` for emphasised titles/labels at a given size.
fn bold(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name("bold".into()))
}

/// A white, rounded "card" surface with a hairline border and inner padding —
/// the building block of the layout (mirrors SwiftUI grouped sections).
fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::none()
        .fill(CARD)
        .rounding(egui::Rounding::same(12.0))
        .stroke(egui::Stroke::new(1.0, HAIRLINE))
        .inner_margin(egui::Margin::same(18.0))
        .shadow(egui::epaint::Shadow {
            offset: egui::vec2(0.0, 2.0), blur: 14.0, spread: 0.0,
            color: egui::Color32::from_black_alpha(70),
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// Filled, system-blue primary call-to-action button.
fn accent_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(egui::Button::new(egui::RichText::new(text).color(egui::Color32::WHITE).strong())
        .fill(ACCENT)
        .min_size(egui::vec2(150.0, 34.0))
        .rounding(egui::Rounding::same(8.0)))
}

// ─── Conversion log (mirror of conversion_log.json written by the converter) ────

#[derive(Debug, Clone, Deserialize)]
struct LogEntry {
    name: String,
    count: u32,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
struct ConversionLog {
    character: String,
    unknown: Vec<LogEntry>,
    ssf2_only: Vec<LogEntry>,
}

impl ConversionLog {
    fn is_empty(&self) -> bool {
        self.unknown.is_empty() && self.ssf2_only.is_empty()
    }
    fn load(char_dir: &Path) -> Option<ConversionLog> {
        let text = std::fs::read_to_string(char_dir.join("conversion_log.json")).ok()?;
        serde_json::from_str(&text).ok()
    }
}

// ─── App state ──────────────────────────────────────────────────────────────────

enum Stage {
    Idle,
    Converting { status: String },
    Success { character: String, output_path: String, log: String },
    Failure { message: String, log: String },
}

#[derive(PartialEq)]
enum ExportStage {
    Idle,
    Running,
    Done(String),
    Failed(String),
}

enum Msg {
    Progress(String),
    Done { character: String, output_path: String, log: String, conv_log: Option<ConversionLog> },
    Failed { message: String, log: String },
    ExportDone(String),
    ExportFailed(String),
}

struct App {
    prefs: Prefs,
    stage: Stage,
    export: ExportStage,
    conv_log: Option<ConversionLog>,
    show_log_window: bool,
    show_fraymakers_prompt: bool,
    hovering_file: bool,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        setup_style(&cc.egui_ctx);
        let mut prefs = Prefs::load();
        if prefs.output_dir.is_empty() {
            // Default output: ~/Documents/FraymakersCharacters (cross-platform).
            if let Some(docs) = dirs::document_dir() {
                prefs.output_dir = docs.join("FraymakersCharacters").to_string_lossy().into_owned();
            } else if let Some(home) = dirs::home_dir() {
                prefs.output_dir = home.join("FraymakersCharacters").to_string_lossy().into_owned();
            }
        }
        // Pre-fill a detected FrayTools default if the user hasn't set one.
        if prefs.fraytools_path.is_empty() {
            if let Some(p) = platform::default_fraytools_exe() {
                prefs.fraytools_path = p.to_string_lossy().into_owned();
            }
        }

        let show_fraymakers_prompt = platform::fraymakers_root().is_some()
            && !prefs.fraymakers_auto_publish
            && !prefs.fraymakers_prompt_decided;

        let (tx, rx) = channel();
        Self {
            prefs,
            stage: Stage::Idle,
            export: ExportStage::Idle,
            conv_log: None,
            show_log_window: false,
            show_fraymakers_prompt,
            hovering_file: false,
            tx,
            rx,
        }
    }

    fn resolved_fraytools_exe(&self) -> Option<String> {
        if self.prefs.fraytools_path.is_empty() {
            None
        } else {
            Some(platform::resolve_fraytools_exe(&self.prefs.fraytools_path))
        }
    }

    // ── Conversion ──────────────────────────────────────────────────────────────

    fn start_conversion(&mut self, ctx: &egui::Context, input: PathBuf) {
        let bin = match platform::converter_bin() {
            Some(b) => b,
            None => {
                self.stage = Stage::Failure {
                    message: "Couldn't find the ssf2_converter binary next to this app.".into(),
                    log: String::new(),
                };
                return;
            }
        };

        // misc.ssf must be selected manually — no auto-detection.
        let misc: Option<String> = if !self.prefs.misc_ssf.is_empty() {
            Some(self.prefs.misc_ssf.clone())
        } else {
            None
        };

        let output = PathBuf::from(&self.prefs.output_dir);
        let auto_pub = self.prefs.fraymakers_auto_publish;
        self.stage = Stage::Converting { status: "Starting…".into() };
        self.export = ExportStage::Idle;
        self.conv_log = None;

        let tx = self.tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let char_name = input.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "character".into());
            let char_output = output.join(&char_name);

            let mut cmd = Command::new(&bin);
            cmd.arg(&input).arg("--output").arg(&output).arg("--verbose");
            if let Some(m) = &misc {
                cmd.arg("--misc-ssf").arg(m);
            }
            cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

            let mut child = match cmd.spawn() {
                Ok(c) => c,
                Err(e) => {
                    let _ = tx.send(Msg::Failed { message: format!("Failed to launch converter: {e}"), log: String::new() });
                    ctx.request_repaint();
                    return;
                }
            };

            // Stream stderr (env_logger output) → live status + full log.
            let stderr = child.stderr.take();
            let txp = tx.clone();
            let ctxp = ctx.clone();
            let log_handle = std::thread::spawn(move || {
                let mut all = String::new();
                if let Some(s) = stderr {
                    for line in BufReader::new(s).lines().map_while(Result::ok) {
                        all.push_str(&line);
                        all.push('\n');
                        let _ = txp.send(Msg::Progress(line));
                        ctxp.request_repaint();
                    }
                }
                all
            });

            // Drain stdout too (keeps the pipe from blocking).
            let mut stdout_buf = String::new();
            if let Some(mut so) = child.stdout.take() {
                use std::io::Read;
                let _ = so.read_to_string(&mut stdout_buf);
            }

            let status = child.wait();
            let mut log = log_handle.join().unwrap_or_default();
            if !stdout_buf.trim().is_empty() {
                log.push_str(&stdout_buf);
            }
            let log = log.trim().to_string();

            match status {
                Ok(s) if s.success() => {
                    let conv_log = ConversionLog::load(&char_output);
                    if auto_pub {
                        let _ = platform::ensure_fraymakers_publish_folder(&char_name, &char_output);
                    }
                    let _ = tx.send(Msg::Done {
                        character: char_name,
                        output_path: char_output.to_string_lossy().into_owned(),
                        log,
                        conv_log,
                    });
                }
                Ok(s) => {
                    let _ = tx.send(Msg::Failed { message: format!("Converter exited with code {}", s.code().unwrap_or(-1)), log });
                }
                Err(e) => {
                    let _ = tx.send(Msg::Failed { message: format!("Converter error: {e}"), log });
                }
            }
            ctx.request_repaint();
        });
    }

    // ── Export in FrayTools ───────────────────────────────────────────────────────

    fn run_export(&mut self, ctx: &egui::Context, char_output_path: String) {
        let exe = match self.resolved_fraytools_exe() {
            Some(e) => e,
            None => { self.export = ExportStage::Failed("Set your FrayTools path first.".into()); return; }
        };
        let node = match platform::find_node() {
            Some(n) => n,
            None => { self.export = ExportStage::Failed("Couldn't find `node`. Install Node.js.".into()); return; }
        };
        let script = match platform::find_export_script() {
            Some(s) => s,
            None => { self.export = ExportStage::Failed("Couldn't find export-in-fraytools.js.".into()); return; }
        };
        let out_dir = PathBuf::from(&char_output_path);
        let project = match platform::find_project_file(&out_dir) {
            Some(p) => p,
            None => { self.export = ExportStage::Failed("No .fraytools project found.".into()); return; }
        };

        // Ensure the Fraymakers folder is in publish settings if opted in.
        if self.prefs.fraymakers_auto_publish {
            if let Some(name) = project.file_stem() {
                let _ = platform::ensure_fraymakers_publish_folder(&name.to_string_lossy(), &out_dir);
            }
        }

        self.export = ExportStage::Running;
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        let script_dir = script.parent().map(|p| p.to_path_buf());
        std::thread::spawn(move || {
            let mut cmd = Command::new(&node);
            cmd.arg(&script).arg("--project").arg(&project).arg("--fraytools").arg(&exe);
            if let Some(d) = &script_dir {
                cmd.current_dir(d);
            }
            cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
            match cmd.output() {
                Ok(out) if out.status.success() => {
                    let s = String::from_utf8_lossy(&out.stdout);
                    let fra = s.trim().lines().last().unwrap_or("").to_string();
                    let _ = tx.send(Msg::ExportDone(fra));
                }
                Ok(out) => {
                    let err = String::from_utf8_lossy(&out.stderr);
                    let reason = err.trim().lines().last().unwrap_or("export failed").to_string();
                    let _ = tx.send(Msg::ExportFailed(reason));
                }
                Err(e) => { let _ = tx.send(Msg::ExportFailed(e.to_string())); }
            }
            ctx.request_repaint();
        });
    }

    // ── Drain background messages ──────────────────────────────────────────────────

    fn pump(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Progress(line) => {
                    if let Stage::Converting { status } = &mut self.stage {
                        *status = line;
                    }
                }
                Msg::Done { character, output_path, log, conv_log } => {
                    self.conv_log = conv_log;
                    self.show_log_window = self.conv_log.as_ref().map(|l| !l.is_empty()).unwrap_or(false);
                    self.stage = Stage::Success { character, output_path, log };
                }
                Msg::Failed { message, log } => {
                    self.stage = Stage::Failure { message, log };
                }
                Msg::ExportDone(fra) => { self.export = ExportStage::Done(fra); }
                Msg::ExportFailed(e) => { self.export = ExportStage::Failed(e); }
            }
        }
    }
}

// ─── Helpers ────────────────────────────────────────────────────────────────────

/// Reveal a file or folder in the OS file manager.
fn reveal_in_file_manager(path: &str) {
    #[cfg(target_os = "macos")]
    let _ = Command::new("open").arg("-R").arg(path).spawn();
    #[cfg(target_os = "windows")]
    let _ = Command::new("explorer").arg(format!("/select,{path}")).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let dir = Path::new(path).parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from(path));
        let _ = Command::new("xdg-open").arg(dir).spawn();
    }
}

// ─── UI ─────────────────────────────────────────────────────────────────────────

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.pump();

        self.fraymakers_prompt(ctx);
        self.log_window(ctx);

        egui::TopBottomPanel::top("header")
            .frame(egui::Frame::none()
                .fill(HEADER_BG)
                .inner_margin(egui::Margin { left: 26.0, right: 26.0, top: 18.0, bottom: 16.0 }))
            .show(ctx, |ui| {
                ui.heading(egui::RichText::new("tsonconvert").color(ACCENT));
                ui.add_space(2.0);
                ui.label(egui::RichText::new("SSF2 to Fraymakers character converter").weak());
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(BG).inner_margin(egui::Margin::same(20.0)))
            .show(ctx, |ui| {
                self.settings_card(ui);
                ui.add_space(16.0);
                match &self.stage {
                    Stage::Idle => self.idle_view(ctx, ui),
                    Stage::Converting { status } => {
                        let status = status.clone();
                        Self::converting_view(ui, &status);
                    }
                    Stage::Success { character, output_path, log } => {
                        let (c, o, l) = (character.clone(), output_path.clone(), log.clone());
                        self.success_view(ctx, ui, &c, &o, &l);
                    }
                    Stage::Failure { message, log } => {
                        let (m, l) = (message.clone(), log.clone());
                        self.failure_view(ui, &m, &l);
                    }
                }
            });

        // Handle dropped files (.ssf).
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect()
        });
        self.hovering_file = ctx.input(|i| !i.raw.hovered_files.is_empty());
        if let Some(ssf) = dropped.into_iter().find(|p| p.extension().map(|e| e.eq_ignore_ascii_case("ssf")).unwrap_or(false)) {
            self.start_conversion(ctx, ssf);
        }
    }
}

impl App {
    /// Settings grouped in a clean card: aligned label / value / action columns.
    fn settings_card(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            egui::Grid::new("settings_grid")
                .num_columns(3)
                .spacing(egui::vec2(16.0, 12.0))
                .min_col_width(70.0)
                .show(ui, |ui| {
                    // ── Output ──
                    ui.label(egui::RichText::new("Output").strong());
                    let name = Path::new(&self.prefs.output_dir).file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| self.prefs.output_dir.clone());
                    ui.label(name).on_hover_text(&self.prefs.output_dir);
                    if ui.button("Change…").clicked() {
                        if let Some(d) = rfd::FileDialog::new().pick_folder() {
                            self.prefs.output_dir = d.to_string_lossy().into_owned();
                            self.prefs.save();
                        }
                    }
                    ui.end_row();

                    // ── misc.ssf ──
                    ui.label(egui::RichText::new("misc.ssf").strong());
                    if self.prefs.misc_ssf.is_empty() {
                        ui.label(egui::RichText::new("not set").italics().weak());
                    } else {
                        ui.horizontal(|ui| {
                            let name = Path::new(&self.prefs.misc_ssf).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                            ui.colored_label(OK_GREEN, name).on_hover_text(&self.prefs.misc_ssf);
                            if ui.small_button("Clear").on_hover_text("Clear selection").clicked() { self.prefs.misc_ssf.clear(); self.prefs.save(); }
                        });
                    }
                    if ui.button("Set…").clicked() {
                        if let Some(f) = rfd::FileDialog::new().add_filter("SSF", &["ssf"]).pick_file() {
                            self.prefs.misc_ssf = f.to_string_lossy().into_owned();
                            self.prefs.save();
                        }
                    }
                    ui.end_row();

                    // ── FrayTools ──
                    ui.label(egui::RichText::new("FrayTools").strong());
                    if self.prefs.fraytools_path.is_empty() {
                        ui.label(egui::RichText::new("not detected").italics().weak())
                            .on_hover_text("Auto-detected at launch when FrayTools is in a standard location; otherwise set it here.");
                    } else {
                        let name = Path::new(&self.prefs.fraytools_path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        ui.colored_label(OK_GREEN, name).on_hover_text(&self.prefs.fraytools_path);
                    }
                    if ui.button(if self.prefs.fraytools_path.is_empty() { "Set…" } else { "Change…" }).clicked() {
                        let mut dlg = rfd::FileDialog::new();
                        if cfg!(target_os = "macos") { dlg = dlg.add_filter("App", &["app"]); }
                        else if cfg!(windows) { dlg = dlg.add_filter("Executable", &["exe"]); }
                        if let Some(f) = dlg.pick_file() {
                            self.prefs.fraytools_path = f.to_string_lossy().into_owned();
                            self.prefs.save();
                        }
                    }
                    ui.end_row();
                });
        });
    }

    fn idle_view(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let hovering = self.hovering_file;
        let (stroke, fill, icon_col) = if hovering {
            (egui::Stroke::new(2.0, ACCENT), DROP_HOVER, ACCENT)
        } else {
            (egui::Stroke::new(1.5, HAIRLINE), CARD, ui.visuals().weak_text_color())
        };
        egui::Frame::none()
            .fill(fill)
            .stroke(stroke)
            .rounding(egui::Rounding::same(14.0))
            .inner_margin(egui::Margin::same(24.0))
            .show(ui, |ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), ui.available_height().max(200.0)),
                    egui::Layout::centered_and_justified(egui::Direction::TopDown),
                    |ui| {
                        ui.vertical_centered(|ui| {
                            ui.label(egui::RichText::new("⬇").size(46.0).color(icon_col));
                            ui.add_space(12.0);
                            ui.label(egui::RichText::new("Drop an SSF file here").font(bold(18.0)));
                            ui.add_space(4.0);
                            ui.label(egui::RichText::new("or").weak());
                            ui.add_space(12.0);
                            if accent_button(ui, "Choose File…").clicked() {
                                if let Some(f) = rfd::FileDialog::new().add_filter("SSF", &["ssf"]).pick_file() {
                                    self.start_conversion(ctx, f);
                                }
                            }
                        });
                    },
                );
            });
    }

    fn converting_view(ui: &mut egui::Ui, status: &str) {
        ui.add_space(60.0);
        ui.vertical_centered(|ui| {
            ui.add(egui::Spinner::new().size(34.0).color(ACCENT));
            ui.add_space(16.0);
            ui.label(egui::RichText::new("Converting…").font(bold(17.0)));
            ui.add_space(6.0);
            ui.label(egui::RichText::new(status).weak());
        });
    }

    fn success_view(&mut self, ctx: &egui::Context, ui: &mut egui::Ui, character: &str, output_path: &str, log: &str) {
        ui.vertical_centered(|ui| {
            ui.add_space(16.0);
            ui.label(egui::RichText::new("✓").size(48.0).color(egui::Color32::from_rgb(60, 170, 60)));
            ui.add_space(6.0);
            ui.label(egui::RichText::new(format!("Converted {character}!")).font(bold(22.0)));
            ui.label(egui::RichText::new(output_path).monospace().weak());
            ui.add_space(10.0);
        });

        card(ui, |ui| {
            egui::ScrollArea::vertical().max_height(140.0).show(ui, |ui| {
                ui.add(egui::Label::new(egui::RichText::new(log).monospace().small()).wrap());
            });
        });

        ui.add_space(10.0);

        // Export status line.
        match &self.export {
            ExportStage::Idle => {}
            ExportStage::Running => { ui.horizontal(|ui| { ui.add(egui::Spinner::new()); ui.label("Publishing in FrayTools…"); }); }
            ExportStage::Done(fra) => {
                let fra = fra.clone();
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("📦 Published:").strong());
                    let name = Path::new(&fra).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| fra.clone());
                    if ui.link(egui::RichText::new(name).monospace()).clicked() {
                        reveal_in_file_manager(&fra);
                    }
                });
            }
            ExportStage::Failed(msg) => { ui.colored_label(egui::Color32::from_rgb(200, 70, 70), format!("Export failed: {msg}")); }
        }

        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui.button("Show in Folder").clicked() {
                reveal_in_file_manager(output_path);
            }
            if self.conv_log.as_ref().map(|l| !l.is_empty()).unwrap_or(false) {
                if ui.button("Unhandled Calls…").clicked() { self.show_log_window = true; }
            }
            let can_export = self.resolved_fraytools_exe().is_some() && self.export != ExportStage::Running;
            if ui.add_enabled(can_export, egui::Button::new(egui::RichText::new("Export in FrayTools").color(egui::Color32::WHITE).strong()).fill(ACCENT).rounding(egui::Rounding::same(8.0)))
                .on_hover_text(if self.resolved_fraytools_exe().is_none() { "Set your FrayTools path first" } else { "Open in FrayTools and publish the .fra" })
                .clicked()
            {
                self.run_export(ctx, output_path.to_string());
            }
            if ui.button("Convert Another").clicked() {
                self.stage = Stage::Idle;
                self.export = ExportStage::Idle;
                self.conv_log = None;
            }
        });
    }

    fn failure_view(&mut self, ui: &mut egui::Ui, message: &str, log: &str) {
        ui.vertical_centered(|ui| {
            ui.add_space(16.0);
            ui.label(egui::RichText::new("✕").size(48.0).color(egui::Color32::from_rgb(200, 70, 70)));
            ui.add_space(6.0);
            ui.label(egui::RichText::new("Conversion Failed").font(bold(22.0)));
            ui.colored_label(egui::Color32::from_rgb(200, 70, 70), message);
            ui.add_space(10.0);
        });
        card(ui, |ui| {
            egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                ui.add(egui::Label::new(egui::RichText::new(if log.is_empty() { "(no output)" } else { log }).monospace().small()).wrap());
            });
        });
        ui.add_space(10.0);
        ui.vertical_centered(|ui| {
            if ui.button("Try Again").clicked() {
                self.stage = Stage::Idle;
            }
        });
    }

    fn fraymakers_prompt(&mut self, ctx: &egui::Context) {
        if !self.show_fraymakers_prompt {
            return;
        }
        egui::Window::new("Publish into Fraymakers?")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.set_max_width(420.0);
                ui.label("Fraymakers is installed on this machine. Would you like converted characters to publish straight into your Fraymakers custom-content folder (custom/<Character>) so they're playable in-game?");
                ui.add_space(6.0);
                ui.label(egui::RichText::new("This adds that folder to each character's FrayTools publish settings; ./build still gets a copy too.").weak().small());
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.button("Yes").clicked() {
                        self.prefs.fraymakers_auto_publish = true;
                        self.prefs.fraymakers_prompt_decided = true;
                        self.prefs.save();
                        self.show_fraymakers_prompt = false;
                    }
                    if ui.button("Not now").clicked() {
                        self.show_fraymakers_prompt = false; // ask again next launch
                    }
                    if ui.button("Don't ask again").clicked() {
                        self.prefs.fraymakers_auto_publish = false;
                        self.prefs.fraymakers_prompt_decided = true;
                        self.prefs.save();
                        self.show_fraymakers_prompt = false;
                    }
                });
            });
    }

    fn log_window(&mut self, ctx: &egui::Context) {
        if !self.show_log_window {
            return;
        }
        let Some(log) = self.conv_log.clone() else {
            self.show_log_window = false;
            return;
        };
        let mut open = true;
        egui::Window::new("Unhandled Calls")
            .collapsible(false)
            .resizable(true)
            .default_size(egui::vec2(480.0, 400.0))
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(egui::RichText::new(&log.character).weak());
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    if !log.unknown.is_empty() {
                        ui.label(egui::RichText::new(format!("Unknown calls ({})", log.unknown.len())).strong().color(egui::Color32::from_rgb(220, 150, 40)));
                        ui.label(egui::RichText::new("No mapping in commands.jsonc — likely needs a replacement, passthrough, or ssf2_only entry.").small().weak());
                        for e in &log.unknown {
                            ui.horizontal(|ui| {
                                ui.monospace(&e.name);
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| { ui.monospace(e.count.to_string()); });
                            });
                        }
                        ui.add_space(10.0);
                    }
                    if !log.ssf2_only.is_empty() {
                        ui.label(egui::RichText::new(format!("SSF2-only calls ({})", log.ssf2_only.len())).strong().color(egui::Color32::from_rgb(70, 140, 220)));
                        ui.label(egui::RichText::new("No Fraymakers equivalent — commented out in the generated Haxe. Manual port required.").small().weak());
                        for e in &log.ssf2_only {
                            ui.horizontal(|ui| {
                                ui.monospace(&e.name);
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| { ui.monospace(e.count.to_string()); });
                            });
                        }
                    }
                });
            });
        if !open {
            self.show_log_window = false;
        }
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([720.0, 600.0])
            .with_min_inner_size([600.0, 480.0])
            .with_title("tsonconvert"),
        ..Default::default()
    };
    eframe::run_native(
        "tsonconvert",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}
