//! fastboot — the SINGLE definition of "what a quick boot launches".
//!
//! A quick (fast / headless) boot lands the engine straight in a live match instead
//! of parking the bridge at READY. The start-match is socket-driven on both engines
//! (Fraymakers `s`, SSF2 `SPAWN`+`GO`), so SOMETHING has to fire it once the engine is
//! ready. That decision — *whether* to autostart, *which* character, and the exact
//! launch command — used to live in four places (FM CLI, SSF2 CLI, FM GUI page-JS,
//! SSF2 GUI host) that didn't even agree. This module is the one home for it, shared by
//! every frontend (CLI + GUI) and parameterized by engine. Tweak fastboot here and both
//! the CLI and the GUI move together.
//!
//! The command is expressed in the engine-agnostic `spawn …` vocabulary (the same one a
//! user types); each transport translates it on the way out (FM: `command_to_wire` →
//! socket; SSF2: `run_command` → reflection). So this module never touches a socket.
//!
//! ── Quick-boot feature parity (OOP seam: shared policy here, per-engine integrations) ──
//!
//! A quick boot is a SET of features. The decision/orchestration is engine-agnostic and
//! lives host-side (this module + the session layer); each feature's *implementation* is
//! owned by the platform-specific integration, because they're different bytecode VMs.
//! Keep this table honest when either engine's quick boot changes:
//!
//!   Feature                 Fraymakers (HashLink, src/main.rs)   SSF2 (AVM2, crates/…/abc_inject.rs)
//!   ─────────────────────   ──────────────────────────────────   ──────────────────────────────────
//!   skip menus / title      no-op launchScreen@17771              inject_quickboot rewrites
//!                           (skip Title + loadUgc)                 MenuController.showInitialMenu
//!   preload match assets     bake char + `s` handler resolves      inject_quickboot:
//!                           + fetchThreaded's stage/assist          queueResources([char, stage])
//!   loading presentation     MatchController.startMatch builds      inject_quickboot:
//!     (boot → match)         _loadingMenu via loadingScreenFactory   MenuController.loadingMenu.show()
//!                           + showMenu  (NATIVE; see note)           (torn down by GO's disposeAllMenus)
//!   readiness signal         READY at Main.onLoaded                 responsiveness heuristic
//!                                                                   (disclaimer is skipped)
//!   auto-spawn into match    `command(Fraymakers, …)` → `s`         `command(Ssf2, …)` → `spawn`
//!
//! NOTE on FM loading presentation: `MatchController.startMatch@18315` already builds and
//! shows the loading screen (`loadingScreenFactory` → `_loadingMenu` → `showMenu`) on the
//! quick-boot path, but it is not visibly held — under investigation whether the factory is
//! unset on a fast boot or the screen merely flashes because the `s` handler pre-fetches the
//! match content. SSF2's is held because its on-demand asset decrypt genuinely takes seconds.

use crate::config::Config;

/// Which engine a boot is targeting. The transport/streaming differences live in the
/// session layers; here we only need it to pick the right launch command shape.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Engine {
    Fraymakers,
    Ssf2,
}

/// The inputs that decide a quick boot, independent of CLI-vs-GUI. `char_name` is the
/// explicitly chosen character (CLI `--char`, GUI picked project); `None` falls back to
/// the configured default. `full` = a bare bridge boot (CLI `--full`/`--attach`, GUI
/// "regular boot") that must NOT autostart — you drive it by hand.
#[derive(Clone, Debug, Default)]
pub struct BootOptions {
    pub char_name: Option<String>,
    pub full: bool,
}

impl BootOptions {
    /// Build from CLI args. `--full`, `--no-boot`, and `--attach` all mean "bridge only,
    /// no autostart"; `--char X` names the character (else the config default is used).
    pub fn from_cli(args: &[String]) -> Self {
        let full = args.iter().any(|a| a == "--full" || a == "--no-boot" || a == "--attach");
        let char_name = arg_val(args, "--char");
        BootOptions { char_name, full }
    }
}

/// The canonical start-match command for a quick boot, or `None` to NOT autostart
/// (a `--full`/bridge boot, or no character available). The ONE place this is decided.
///
/// The character/stage/assist all come from the same source the bake uses (CLI `--char`
/// or the config default + `Config::stage()`/`assist()`), so a fast boot launches exactly
/// what was baked. Stage/assist are made EXPLICIT for Fraymakers on purpose: the engine's
/// `s` handler only fetch-loads the args it's given, so omitting them leaves their content
/// refs null → the null-namespace crash. SSF2 resolves its stage from config inside
/// `Ssf2Target::spawn`, so only the character is needed.
pub fn command(engine: Engine, opts: &BootOptions) -> Option<String> {
    if opts.full {
        return None;
    }
    let cfg = Config::load();
    let ch = opts.char_name.clone().unwrap_or_else(|| cfg.char_name());
    if ch.trim().is_empty() {
        return None;
    }
    Some(match engine {
        Engine::Fraymakers => format!("spawn {ch} {} {}", cfg.stage(), cfg.assist()),
        Engine::Ssf2 => format!("spawn {ch}"),
    })
}

/// `--key value` lookup (shared shape with the session arg parsers).
fn arg_val(args: &[String], key: &str) -> Option<String> {
    args.iter().position(|a| a == key).and_then(|i| args.get(i + 1)).cloned()
}
