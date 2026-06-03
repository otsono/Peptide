//! `peptide convert` — the CLI face of the in-process SSF2 → Fraymakers
//! converter (folded in from the old standalone `ssf2_converter` binary).
//!
//! Usage:
//!   peptide convert <file.ssf> [--output DIR] [--name NAME]
//!                              [--misc-ssf FILE] [--per-character-projects] [-v]
//!
//! The converter is a library now ([`ssf2_converter::run_conversion`]); this is a
//! thin argument adapter + human-readable summary. The Peptide GUI calls the same
//! library function directly on a worker thread (see the convert screen).

use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::sync::Once;

use ssf2_converter::{run_conversion, ConvertOptions};

static LOGGER_INIT: Once = Once::new();

/// The conversion-permission notice shown before any SSF2 → Fraymakers conversion.
const PERMISSION_NOTICE: &str = "\
Super Smash Flash 2 content is developed over months/years with a lot of
deliberate care and attention from an unpaid dev team.

Respecting the team's work and their wishes is paramount.
If this is a character that is built into the engine, please only proceed if
you've received permission from their team to convert the character.";

/// Show the permission notice and require explicit confirmation before converting.
///
/// `--yes`/`-y` (or a non-interactive stdin, where there is no one to answer)
/// records acknowledgement and proceeds; otherwise the user must type `y`/`yes`.
fn confirm_permission(assume_yes: bool) -> anyhow::Result<bool> {
    println!("\n{PERMISSION_NOTICE}\n");

    if assume_yes {
        println!("Proceeding (permission acknowledged via --yes).");
        return Ok(true);
    }

    if !std::io::stdin().is_terminal() {
        // No interactive terminal to prompt; refuse rather than convert silently.
        eprintln!(
            "convert: refusing to proceed without confirmation (no interactive \
             terminal). Re-run with --yes once you have permission."
        );
        return Ok(false);
    }

    print!("Have you received permission to convert this character? [y/N] ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let answer = line.trim().to_ascii_lowercase();
    Ok(matches!(answer.as_str(), "y" | "yes"))
}

/// Initialise the global logger exactly once for the whole process. The converter
/// library logs via the `log` facade but never installs a logger (it can be called
/// many times from the long-running GUI); the binary owns that, once.
pub fn init_logger(verbose: bool) {
    LOGGER_INIT.call_once(|| {
        let level = if verbose { log::LevelFilter::Debug } else { log::LevelFilter::Info };
        // `from_default_env` still lets RUST_LOG override the level.
        let _ = env_logger::Builder::from_default_env().filter_level(level).try_init();
    });
}

/// Parse `convert` args (everything after the `convert` word) and run a conversion.
pub fn run_cli(args: &[String]) -> anyhow::Result<()> {
    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut name: Option<String> = None;
    let mut misc_ssf: Option<PathBuf> = None;
    let mut per_character_projects = false;
    let mut verbose = false;
    let mut assume_yes = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-o" | "--output" => { output = args.get(i + 1).map(PathBuf::from); i += 2; }
            "-n" | "--name" => { name = args.get(i + 1).cloned(); i += 2; }
            "--misc-ssf" => { misc_ssf = args.get(i + 1).map(PathBuf::from); i += 2; }
            "--per-character-projects" => { per_character_projects = true; i += 1; }
            "-y" | "--yes" => { assume_yes = true; i += 1; }
            "-v" | "--verbose" => { verbose = true; i += 1; }
            "-h" | "--help" => { print!("{}", help_text()); return Ok(()); }
            s if s.starts_with('-') => {
                anyhow::bail!("convert: unknown flag {s:?}\n\n{}", help_text());
            }
            _ => {
                if input.is_none() { input = Some(PathBuf::from(&args[i])); }
                else { anyhow::bail!("convert: unexpected extra argument {:?}", args[i]); }
                i += 1;
            }
        }
    }

    let input = input.ok_or_else(|| {
        anyhow::anyhow!("convert: missing <file.ssf>\n\n{}", help_text())
    })?;

    if !confirm_permission(assume_yes)? {
        anyhow::bail!("convert: aborted — permission not confirmed");
    }

    init_logger(verbose);

    let mut opts = ConvertOptions::new(input);
    if let Some(o) = output { opts.output = o; }
    opts.name = name;
    opts.misc_ssf = misc_ssf;
    opts.per_character_projects = per_character_projects;

    let summary = run_conversion(opts)?;

    // Human-readable summary (stdout — the log facade goes to stderr).
    println!();
    if summary.multi_char {
        println!(
            "Converted {} characters into one project: {}",
            summary.characters.len(),
            summary.project_dir.display()
        );
    } else {
        println!(
            "Converted {} into {}",
            summary.characters.join(", "),
            summary.project_dir.display()
        );
    }
    for f in &summary.fraytools_files {
        println!("  project file: {}", f.display());
    }
    if !summary.warnings.is_empty() {
        println!("  {} warning(s):", summary.warnings.len());
        for w in &summary.warnings {
            println!("    - {w}");
        }
    }
    Ok(())
}

fn help_text() -> String {
    "\
peptide convert — SSF2 → Fraymakers character converter

USAGE:
  peptide convert <file.ssf> [OPTIONS]

The input may be a pre-decompiled character .swf, a SWF-wrapped .ssf, or one of
SSF2's shipped DAT<n>.ssf data archives (the embedded character SWF is unwrapped
automatically). Non-character DAT archives (stages, items, UI) are reported and
skipped.

OPTIONS:
  -o, --output <DIR>          Output directory          [default: ./characters]
  -n, --name <NAME>           Character-name override; for a multi-character
                              .ssf, selects only that character (auto-detected
                              from the SWF otherwise)
      --misc-ssf <FILE>       misc.ssf for costume palettes (auto-detected next
                              to the input file otherwise)
      --per-character-projects  Emit each character of a multi-character .ssf as
                              its own project (the pre-Stage-B layout)
  -y, --yes                   Acknowledge the conversion-permission notice and
                              skip the interactive prompt (for scripts/CI)
  -v, --verbose               Debug-level logging
  -h, --help                  Show this help

Output for character <id> lands in <output>/<id>/ (single-character) or
<output>/<project_id>/ (a unified multi-character project).
"
    .to_string()
}
