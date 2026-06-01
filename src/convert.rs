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

use std::path::PathBuf;
use std::sync::Once;

use ssf2_converter::{run_conversion, ConvertOptions};

static LOGGER_INIT: Once = Once::new();

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

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-o" | "--output" => { output = args.get(i + 1).map(PathBuf::from); i += 2; }
            "-n" | "--name" => { name = args.get(i + 1).cloned(); i += 2; }
            "--misc-ssf" => { misc_ssf = args.get(i + 1).map(PathBuf::from); i += 2; }
            "--per-character-projects" => { per_character_projects = true; i += 1; }
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

OPTIONS:
  -o, --output <DIR>          Output directory          [default: ./characters]
  -n, --name <NAME>           Character-name override; for a multi-character
                              .ssf, selects only that character (auto-detected
                              from the SWF otherwise)
      --misc-ssf <FILE>       misc.ssf for costume palettes (auto-detected next
                              to the input file otherwise)
      --per-character-projects  Emit each character of a multi-character .ssf as
                              its own project (the pre-Stage-B layout)
  -v, --verbose               Debug-level logging
  -h, --help                  Show this help

Output for character <id> lands in <output>/<id>/ (single-character) or
<output>/<project_id>/ (a unified multi-character project).
"
    .to_string()
}
