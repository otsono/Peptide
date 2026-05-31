//! peptide-ui — boot Fraymakers and open the Peptide console.
//!
//! A single cross-platform executable: it patches a throwaway copy of the engine,
//! launches it, and runs the full-screen console UI. No shell script needed.
//!
//!   peptide-ui              boot + open the console (FRAY_CHAR defaults to sandbag)
//!
//! Overrides (all optional): FRAY_DIR (install path), FRAY_ENGINE, FRAY_BOOT,
//! FRAY_CHAR / FRAY_STAGE / FRAY_ASSIST.

#[path = "../commands.rs"]
mod commands;
#[path = "../ui.rs"]
mod ui;

fn main() {
    if let Err(e) = ui::launch() {
        eprintln!("peptide-ui: {e}");
        std::process::exit(1);
    }
}
