//! `peptide ssf2` — Peptide's integration with SSF2's engine.
//!
//! SSF2 is an Adobe AIR / Flash title (SWF + ABC bytecode at 30fps). Where the
//! Fraymakers side patches HashLink bytecode and drives a live socket bridge,
//! the SSF2 side is approached through a faithful, deterministic re-implementation
//! of its physics — the engine's per-frame integration was reverse-engineered
//! from the SWF (see docs/ssf2-physics-model.md) and re-coded in Rust
//! (`ssf2_converter::physics_sim`). That model is the ground truth used to derive
//! the SSF2→Fraymakers stat scaling and to validate conversions.
//!
//! Subcommands:
//!   peptide ssf2 stats <file.ssf> [char] [--size-mult N]
//!         Raw SSF2 constants → simulated ground-truth motion (px/s, jump apex,
//!         body-relative) → the Fraymakers stats derived from the one knob.
//!   peptide ssf2 scale <file.ssf> [char] [--size-mult N]
//!         Compact raw→derived scaling table only.
//!   peptide ssf2 launch
//!         Quickboot the actual SSF2.app for manual observation.
//!
//! SSF2 install location: $PEPTIDE_SSF2_APP, else the standalone Mac default.

use anyhow::{anyhow, bail, Context, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ssf2_converter::physics_sim::{derive_fm_stats, simulate, ScaleParams, Ssf2Physics};
use ssf2_converter::{abc_parser, ssf, swf_parser};

/// The SSF2.app the patcher operates on. Resolved through the one config source
/// of truth: `PEPTIDE_SSF2_APP` env → persisted `config.ssf2_app` → autodetect
/// (`/Applications/SSF2.app` or a `~/Downloads/SSF2*/SSF2.app` standalone build).
/// `peptide setup` / the GUI Setup wizard persist the path; falling back to the
/// last-resort default keeps older invocations working if nothing is configured.
fn ssf2_app() -> PathBuf {
    crate::config::Config::load()
        .ssf2_app()
        .unwrap_or_else(|| {
            // Last-resort platform default (only reached when config + env are both empty
            // AND default_ssf2_app returned None — i.e. SSF2 isn't installed at any
            // known location). The path is intentionally wrong so the caller's
            // "SSF2.swf not found" error names the expected layout.
            if cfg!(target_os = "macos") {
                PathBuf::from("/Applications/SSF2.app")
            } else {
                PathBuf::from("C:\\Program Files (x86)\\Super Smash Flash 2 Beta 1.4")
            }
        })
}

// ── Cross-platform path helpers ────────────────────────────────────────────

/// Path to SSF2.swf inside the install. macOS: inside the .app bundle.
/// Windows/Linux: at the root of the install directory.
/// `peptide ssf2 manifest` -- the game's OWN content inventory: every id it can load and what
/// each one IS, followed from `SSF2.swf` to the mappings package it names. This is ground zero
/// for a conversion: what exists, and its type, stated by the game rather than guessed from a name.
fn cmd_manifest(args: &[String]) -> Result<()> {
    let app = ssf2_app();
    let swf = ssf2_swf_path(&app);
    let data = swf.parent().map(|p| p.join("data")).unwrap_or_default();
    let inv = ssf2_converter::ssf2_manifest::read(&swf, &data)?;
    if args.iter().any(|a| a == "--json") {
        println!("{}", serde_json::to_string_pretty(&inv)?);
        return Ok(());
    }
    println!("SSF2 content inventory (from {})", swf.display());
    for (kind, m) in [
        ("character", &inv.character), ("stage", &inv.stage), ("misc", &inv.misc),
        ("extra", &inv.extra), ("modes", &inv.modes), ("menu", &inv.menu),
    ] {
        println!("  {kind:10} {:>4}", m.len());
    }
    let want = args.iter().find(|a| !a.starts_with("--"));
    if let Some(w) = want {
        for (kind, m) in [("character", &inv.character), ("stage", &inv.stage)] {
            if let Some(e) = m.get(w.as_str()) {
                println!("\n{w} is a {kind}: file={} ships_as={} guid={} name={:?}",
                         e.file, e.file_pub, e.guid, e.name);
            }
        }
    }
    Ok(())
}

pub fn ssf2_swf_path(app: &Path) -> PathBuf {
    #[cfg(target_os = "macos")]
    return app.join("Contents/Resources/SSF2.swf");
    #[cfg(not(target_os = "macos"))]
    app.join("SSF2.swf")
}

/// Path to the SSF2 executable inside the install. macOS: inside the .app bundle.
/// Windows/Linux: SSF2.exe at the root.
pub fn ssf2_exe_path(app: &Path) -> PathBuf {
    #[cfg(target_os = "macos")]
    return app.join("Contents/MacOS/SSF2");
    #[cfg(not(target_os = "macos"))]
    app.join("SSF2.exe")
}

/// Stem name for the staged patched copy (keeps `.app` on macOS for Finder/AIR).
fn ssf2_patched_name() -> &'static str {
    if cfg!(target_os = "macos") { "SSF2-patched.app" } else { "SSF2-patched" }
}

/// Recursively copy `src` into `dst` (pure Rust — no `cp -R`).
fn copy_dir_all(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let dst_path = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_all(&entry.path(), &dst_path)?;
        } else {
            std::fs::copy(entry.path(), &dst_path)?;
        }
    }
    Ok(())
}

/// A path in the OS temp dir for a named file (forward-slash for AIR on Windows).
fn temp_str(name: &str) -> String {
    std::env::temp_dir().join(name).to_string_lossy().replace('\\', "/")
}

/// Strip quarantine and re-sign the patched bundle on macOS; no-op on other platforms.
fn macos_resign(app: &Path) {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("xattr").arg("-cr").arg(app).status();
        let _ = std::process::Command::new("codesign").args(["--force","--deep","--sign","-"]).arg(app).status();
    }
    #[cfg(not(target_os = "macos"))]
    let _ = app; // silence unused warning
}

/// Copy the SSF2 install to a writable staging location for patching. Prefers the
/// sibling `SSF2-patched[.app]`; falls back to a per-platform cache dir, then temp.
fn stage_patched_bundle(app: &Path) -> Result<PathBuf> {
    let name = ssf2_patched_name();
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(parent) = app.parent() {
        candidates.push(parent.join(name));
    }
    candidates.push(
        dirs::cache_dir()
            .map(|d| d.join("peptide"))
            .unwrap_or_else(|| std::env::temp_dir().join("peptide"))
            .join(name),
    );
    candidates.push(std::env::temp_dir().join("peptide").join(name));

    let mut last = String::from("no candidate locations");
    for dst in &candidates {
        if let Some(parent) = dst.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                last = format!("cannot create {}: {e}", parent.display());
                continue;
            }
        }
        let _ = std::fs::remove_dir_all(dst);
        match copy_dir_all(app, dst) {
            Ok(()) => return Ok(dst.clone()),
            Err(e) => last = format!("copy into {} failed: {e}", dst.display()),
        }
    }
    bail!("could not stage a writable patched SSF2 copy ({last})")
}

fn flag_f64(args: &[String], name: &str) -> Option<f64> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok())
}

/// Pull the raw SSF2 stat map for a character out of a .ssf file.
fn raw_stats(path: &Path, name_override: Option<&str>) -> Result<(String, BTreeMap<String, f64>)> {
    let bytes = std::fs::read(path).map_err(|e| anyhow!("read {}: {e}", path.display()))?;
    let swf_bytes = ssf::decompress(&bytes).map_err(|e| anyhow!("decompress: {e}"))?;
    let swf = swf_parser::parse(&swf_bytes).map_err(|e| anyhow!("swf parse: {e}"))?;
    for abc_bytes in &swf.abc_blocks {
        let Ok(abc) = abc_parser::parse(abc_bytes) else { continue };
        let mut names: Vec<String> = Vec::new();
        if let Some(n) = name_override {
            names.push(n.to_string());
        } else if let Some(meta) = abc_parser::extract_main_package_metadata(&abc) {
            for (id, _m) in &meta.characters {
                names.push(id.clone());
            }
        }
        for name in &names {
            if let Ok(ch) = abc_parser::extract_character(&abc, name) {
                if let Some(s) = ch.stats {
                    return Ok((name.clone(), s.values));
                }
            }
        }
    }
    bail!("no character stats found in {}", path.display())
}

fn scale_params(args: &[String]) -> ScaleParams {
    let mut sp = ScaleParams::default();
    if let Some(v) = flag_f64(args, "--size-mult") { sp.size_multiplier = v; }
    if let Some(v) = flag_f64(args, "--ssf2-fps") { sp.ssf2_fps = v; }
    if let Some(v) = flag_f64(args, "--fm-fps") { sp.fm_fps = v; }
    sp
}

/// Full report: raw → ground truth → derived FM stats.
fn cmd_stats(args: &[String]) -> Result<()> {
    let file = args.first().ok_or_else(|| anyhow!("usage: peptide ssf2 stats <file.ssf> [char] [--size-mult N]"))?;
    let char_name = args.get(1).filter(|s| !s.starts_with('-')).map(|s| s.as_str());
    let sp = scale_params(args);
    let (name, raw) = raw_stats(Path::new(file), char_name)?;
    let p = Ssf2Physics::from_raw(&raw);
    let m = simulate(&p, &sp);
    let fm = derive_fm_stats(&p, &sp);

    println!("SSF2 → Fraymakers physics :: {name}");
    println!("  knob size_multiplier={}  fps {}→{}  =>  velocity_scale={:.4}  accel_scale={:.4}",
        sp.size_multiplier, sp.ssf2_fps, sp.fm_fps, sp.velocity_scale(), sp.accel_scale());
    println!();
    println!("  SSF2 ground truth (simulated @ {}fps, SSF2 px):", sp.ssf2_fps);
    println!("    walk top   {:>7.1} px/s ({:.2} body-w/s)   dash top {:>7.1} px/s",
        m.walk_top_pps, m.walk_top_bw, m.dash_top_pps);
    println!("    jump apex  {:>7.1} px   ({:.2} body-h)   rise {:.3}s  air {:.3}s",
        m.full_jump_apex_px, m.full_jump_apex_bh, m.full_jump_rise_s, m.full_jump_air_s);
    println!("    short hop  {:>7.1} px   double-jump apex {:.1} px", m.short_hop_apex_px, m.double_jump_apex_px);
    println!("    terminal   {:>7.1} px/s   fast-fall {:.1} px/s", m.terminal_pps, m.fast_fall_pps);
    println!();
    println!("  Derived Fraymakers stats (px/frame @ {}fps):", sp.fm_fps);
    println!("    baseScale {:.3}   weight {}", fm.base_scale, p_weight(&raw));
    println!("    gravity {:.3}  jumpSpeed {:.3}  doubleJump {:.3}  shortHop {:.3}",
        fm.gravity, fm.jump_speed, fm.double_jump_speed, fm.short_hop_speed);
    println!("    terminalVelocity {:.3}  fastFall {:.3}", fm.terminal_velocity, fm.fast_fall_speed);
    println!("    walkSpeedCap {:.3}  dashSpeed {:.3}", fm.walk_speed_cap, fm.dash_speed);
    println!("    walkAccel {:.3}  aerialAccel {:.3}  friction {:.3}  aerialFriction {:.3}",
        fm.walk_accel, fm.aerial_accel, fm.friction, fm.aerial_friction);
    Ok(())
}

fn p_weight(raw: &BTreeMap<String, f64>) -> f64 {
    raw.get("weight1").or_else(|| raw.get("weight")).copied().unwrap_or(100.0)
}

/// Compact raw→derived table.
fn cmd_scale(args: &[String]) -> Result<()> {
    let file = args.first().ok_or_else(|| anyhow!("usage: peptide ssf2 scale <file.ssf> [char]"))?;
    let char_name = args.get(1).filter(|s| !s.starts_with('-')).map(|s| s.as_str());
    let sp = scale_params(args);
    let (name, raw) = raw_stats(Path::new(file), char_name)?;
    let p = Ssf2Physics::from_raw(&raw);
    let fm = derive_fm_stats(&p, &sp);
    let vs = sp.velocity_scale();
    let as_ = sp.accel_scale();
    println!("{name}: velocity_scale={vs:.4} (×{:.0}fps→{:.0}fps, size {}), accel_scale={as_:.4}",
        sp.ssf2_fps, sp.fm_fps, sp.size_multiplier);
    let row = |label: &str, raw_v: f64, fm_v: f64, kind: &str| {
        println!("  {label:<18} raw {raw_v:>8.3}  ×{kind}  =>  {fm_v:>8.3}");
    };
    row("walkSpeedCap", p.norm_x_speed, fm.walk_speed_cap, "vel");
    row("dashSpeed", p.max_x_speed, fm.dash_speed, "vel");
    row("jumpSpeed", p.jump_speed, fm.jump_speed, "vel");
    row("terminalVelocity", p.max_y_speed, fm.terminal_velocity, "vel");
    row("fastFallSpeed", p.fast_fall_speed, fm.fast_fall_speed, "vel");
    row("shortHopSpeed", p.short_hop_speed, fm.short_hop_speed, "vel");
    row("gravity", p.gravity, fm.gravity, "acc");
    row("friction", p.decel_rate.abs(), fm.friction, "acc");
    row("walkAccel", p.accel_rate, fm.walk_accel, "acc");
    row("aerialAccel", p.accel_rate_air, fm.aerial_accel, "acc");
    row("aerialFriction", p.decel_rate_air.abs(), fm.aerial_friction, "acc");
    Ok(())
}

/// The document class we hook (SymbolClass id=0).
const SSF2_DOC_CLASS: &str = "com.mcleodgaming.ssf2.Main";

/// `peptide ssf2 patch <in.swf> <out.swf>` — inject the startup-marker payload.
fn cmd_patch(args: &[String]) -> Result<()> {
    let inp = args.first().ok_or_else(|| anyhow!("usage: peptide ssf2 patch <in.swf> <out.swf>"))?;
    let outp = args.get(1).ok_or_else(|| anyhow!("usage: peptide ssf2 patch <in.swf> <out.swf>"))?;
    ssf2_converter::abc_inject::patch_file(
        Path::new(inp), Path::new(outp), SSF2_DOC_CLASS,
        "/tmp/peptide_ssf2_marker.txt", "alive",
    )?;
    println!("patched {} -> {}", inp, outp);
    Ok(())
}

/// `peptide ssf2 install [--heartbeat]` — build a patched copy of the SSF2 install
/// with the bridge payload injected (and resigned on macOS so Gatekeeper accepts it).
fn cmd_install(args: &[String]) -> Result<PathBuf> {
    let heartbeat = args.iter().any(|a| a == "--heartbeat");
    let app = ssf2_app();
    let src_swf = ssf2_swf_path(&app);
    if !src_swf.exists() { bail!("SSF2.swf not found at {}", src_swf.display()); }
    let dst_app = stage_patched_bundle(&app)?;
    let dst_swf = ssf2_swf_path(&dst_app);
    let marker = temp_str("peptide_ssf2_marker.txt");
    let echo = args.iter().any(|a| a == "--echo");
    if echo {
        let cmd_file  = temp_str("peptide_ssf2_cmd.txt");
        let resp_file = temp_str("peptide_ssf2_resp.txt");
        ssf2_converter::abc_inject::patch_file_with(&src_swf, &dst_swf, |abc| {
            ssf2_converter::abc_inject::inject_command_channel(abc, SSF2_DOC_CLASS, &cmd_file, &resp_file)
        })?;
    } else if heartbeat {
        ssf2_converter::abc_inject::patch_file_with(&src_swf, &dst_swf, |abc| {
            ssf2_converter::abc_inject::inject_enterframe_heartbeat(abc, SSF2_DOC_CLASS, &marker)
        })?;
    } else {
        ssf2_converter::abc_inject::patch_file(&src_swf, &dst_swf, SSF2_DOC_CLASS, &marker, "alive")?;
    }
    macos_resign(&dst_app);
    println!("installed patched bridge: {}", dst_app.display());
    Ok(dst_app)
}

/// Install a patched SSF2 copy carrying the socket bridge and return its path. The
/// bridge dials `127.0.0.1:<port>` from the document constructor so the host must
/// already be listening. On macOS the copy is resigned so Gatekeeper accepts it.
///
/// `fastboot = Some((char, stage))` produces a HEADLESS quick-boot patch: it rewrites
/// the boot's initial-menu entry point to skip the disclaimer (and the whole menu chain)
/// and instead queue that character + stage for loading, so the boot loads straight toward the
/// match (see `inject_quickboot`). `None` is a normal boot — the disclaimer plays and fires
/// the event-driven READY (`inject_ready_signal`).
pub fn install_patched(port: u16, fastboot: Option<(&str, &str)>) -> Result<PathBuf> {
    let app = ssf2_app();
    let src_swf = ssf2_swf_path(&app);
    if !src_swf.exists() { bail!("SSF2.swf not found at {}", src_swf.display()); }
    let dst_app = stage_patched_bundle(&app)?;
    let dst_swf = ssf2_swf_path(&dst_app);
    let traj = crate::ssf2_bridge::traj_path();
    ssf2_converter::abc_inject::patch_file_with(&src_swf, &dst_swf, |abc| {
        // resilience preflight: abort loudly (naming what moved) if a critical SSF2 engine
        // symbol the injections depend on was renamed/repackaged by an SSF2 update, instead
        // of emitting a patch whose injected calls fail silently at runtime.
        ssf2_converter::abc_inject::preflight(abc)?;
        ssf2_converter::abc_inject::inject_socket_bridge(abc, SSF2_DOC_CLASS, "127.0.0.1", port)?;
        // per-frame input applicator (hold/seq/scenario): reads the bridge's HOLD/SEQ
        // state and drives the target player's controls each frame.
        ssf2_converter::abc_inject::inject_input_applicator(abc, SSF2_DOC_CLASS)?;
        match fastboot {
            Some((ch, stage)) => {
                // Headless quick boot: skip the disclaimer/menus at the call site and queue
                // the match's char + stage so they load during the boot loading screen.
                ssf2_converter::abc_inject::inject_quickboot(abc, ch, stage, std::env::var("PEPTIDE_PRIME_STAGE").ok().as_deref())?;
            }
            None => {
                // Normal boot: the disclaimer plays and fires the event-driven READY.
                ssf2_converter::abc_inject::inject_ready_signal(abc, SSF2_DOC_CLASS)?;
            }
        }
        // a second ENTER_FRAME listener that logs Characters[0] physics each frame
        ssf2_converter::abc_inject::inject_jump_probe(abc, SSF2_DOC_CLASS, &traj, 0)?;
        // per-frame animation recorder: the engine PUSHES FRAME: lines over the same bridge
        // socket, exactly as Fraymakers pushes ANIM: telemetry. Polling can't do this job —
        // it races the move (a 7-frame attack is ~233ms, the same order as the round trip
        // between sending input and starting to watch). See inject_frame_recorder.
        // both fighters get one: a hit is two characters, and the launch belongs to whichever
        // one took it. Each line names the character it describes.
        ssf2_converter::abc_inject::inject_frame_recorder(abc, SSF2_DOC_CLASS, 0)?;
        ssf2_converter::abc_inject::inject_frame_recorder(abc, SSF2_DOC_CLASS, 1)?;
        // which box landed each hit, kept on the fighter as a named value (`gvar pN __peptideHits`)
        if let Err(e) = ssf2_converter::abc_inject::inject_hit_recorder(abc) {
            eprintln!("[ssf2] hit recorder not installed: {e}");
        }
        // engine-error telemetry, the SSF2 half of Fraymakers' SCRIPTERR channel
        ssf2_converter::abc_inject::inject_error_reporter(abc, SSF2_DOC_CLASS)?;
        // and the load-time half: a package the engine refuses names itself instead of
        // failing silently, which is the only signal a custom-content author ever gets
        ssf2_converter::abc_inject::inject_load_error_reporter(abc, SSF2_DOC_CLASS)?;
        // and the surface nothing was watching: an error thrown by a package's OWN code as it
        // loads goes to the loader, not the app, and unhandled there it ends the process
        ssf2_converter::abc_inject::inject_package_error_reporter(abc, SSF2_DOC_CLASS)?;
        // and the one an author can act on: which part of the stage clip tree is missing, by
        // name, instead of an undefined-value error from somewhere inside the engine.
        //
        // OPT-IN (`PEPTIDE_STAGE_CHECK`), because of WHERE it has to run. It reads a resource
        // while the game is validating it, and even wrapped in a catch-all it still perturbs the
        // load: with it on, the queue gets a fraction of the way it otherwise does. A diagnostic
        // that degrades the thing it is diagnosing is worse than no diagnostic, so it stays off
        // by default until it can run somewhere off the load path.
        if std::env::var("PEPTIDE_STAGE_CHECK").is_ok() {
            ssf2_converter::abc_inject::inject_stage_shape_check(abc, SSF2_DOC_CLASS)?;
        }
        // Attribute a thrown error to the method it came from. The runtime strips line
        // information out of errors, so without this an author gets a bare code and no idea
        // which file to open; each of these reports against its own compiled-in source location
        // and rethrows, leaving behaviour untouched.
        for (class, method, is_static) in if std::env::var("PEPTIDE_NO_LOCATOR").is_ok() { vec![] } else { vec![
            ("GameController", "actuallyStartMatch", true),
            ("GameController", "startMatch", true),
            ("StageData", "<ctor>", false),
            ("StageData", "startGame", false),
            ("Resource", "getProp", false),
            ("ResourceManager", "validateResource", true),
            ("StageData", "findObjects", false),
            ("MovingPlatform", "<ctor>", false),
            ("Platform", "<ctor>", false),
            ("BitmapCollisionBoundary", "<ctor>", false),
        ] } {
            ssf2_converter::abc_inject::inject_error_locator(abc, SSF2_DOC_CLASS, class, method, is_static)?;
        }
        if std::env::var("PEPTIDE_PROBE_LOAD").is_ok() {
            for (class, method, is_static, subject) in [
                ("Resource", "load", false, Some("ID")),
                ("Resource", "loadComplete", false, Some("ID")),
            ] {
                ssf2_converter::abc_inject::inject_method_probe(
                    abc, SSF2_DOC_CLASS, class, method, is_static, subject)?;
            }
        }
        // make the physics fixture askable by id. the engine's id-to-file map comes from a
        // manifest it carries, so a package outside that manifest is never opened; this adds
        // the one entry without rewriting the manifest or touching any shipped data file.
        ssf2_converter::abc_inject::inject_extra_resource(
            abc, ssf2_converter::test_fixture::FIXTURE_ID, FIXTURE_DAT_FILE,
            ssf2_converter::test_fixture::FIXTURE_GUID,
            ssf2_converter::abc_inject::SSF2_RESOURCE_TYPE_STAGE)
    })?;
    macos_resign(&dst_app);
    Ok(dst_app)
}

/// The data-directory name the fixture package is installed under. Deliberately far above the
/// shipped numbering so it can never collide with a future one.
pub const FIXTURE_DAT_FILE: &str = "DAT9000.ssf";

/// `peptide ssf2 selftest` — install, boot, confirm injected code executed
/// (the marker file appears). Proves the code-execution bridge end-to-end.
fn cmd_selftest(args: &[String]) -> Result<()> {
    let echo = args.iter().any(|a| a == "--echo");
    if echo {
        std::fs::write(temp_str("peptide_ssf2_cmd.txt"), b"")?;
        let _ = std::fs::remove_file(temp_str("peptide_ssf2_resp.txt"));
    }
    let dst_app = cmd_install(args)?;
    let marker_s = temp_str("peptide_ssf2_marker.txt");
    let marker = Path::new(&marker_s);
    let _ = std::fs::remove_file(marker);
    let exe = ssf2_exe_path(&dst_app);
    println!("booting patched SSF2…");
    let mut child = std::process::Command::new(&exe)
        .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
        .spawn()?;
    crate::platform::keep_behind(child.id());

    if echo {
        let resp_s = temp_str("peptide_ssf2_resp.txt");
        let resp = Path::new(&resp_s);
        std::thread::sleep(std::time::Duration::from_millis(3000));
        let token = "PING-12345";
        std::fs::write(temp_str("peptide_ssf2_cmd.txt"), token.as_bytes())?;
        let mut got = String::new();
        for _ in 0..20 {
            std::thread::sleep(std::time::Duration::from_millis(300));
            if let Ok(s) = std::fs::read_to_string(resp) { if s == token { got = s; break; } got = s; }
        }
        let _ = child.kill();
        if got == token {
            println!("PASS: bidirectional command channel live — host sent {token:?}, engine echoed {got:?}");
            return Ok(());
        }
        bail!("FAIL: echo channel did not return the command (got {got:?})");
    }
    let heartbeat = args.iter().any(|a| a == "--heartbeat");
    let mut ok = false;
    for _ in 0..30 {
        if marker.exists() { ok = true; break; }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    if !ok {
        let _ = child.kill();
        bail!("FAIL: marker never appeared — injected code did not run (check the patch)");
    }
    let first = std::fs::read_to_string(marker).unwrap_or_default();
    let result = if heartbeat {
        std::thread::sleep(std::time::Duration::from_millis(1200));
        let second = std::fs::read_to_string(marker).unwrap_or_default();
        if first != second {
            println!("PASS: per-frame ENTER_FRAME heartbeat live in SSF2 (getTimer {first:?} -> {second:?})");
            Ok(())
        } else {
            bail!("marker present but value did not change ({first:?}) — handler ran once but not per-frame")
        }
    } else {
        println!("PASS: injected AVM2 code executed in live SSF2 (marker = {first:?})");
        Ok(())
    };
    let _ = child.kill();
    result
}

/// `peptide ssf2 loadtest` — install ONLY a timer-driven load test (no per-frame
/// bridge), boot, and report whether the resource content parses (LIB/STATS).
fn cmd_loadtest(args: &[String]) -> Result<()> {
    let ch = args.iter().rev().find(|a| !a.starts_with("--")).cloned().unwrap_or_else(|| "sandbag".into());
    let app = ssf2_app();
    let src_swf = ssf2_swf_path(&app);
    let dst_app = stage_patched_bundle(&app)?;
    let dst_swf = ssf2_swf_path(&dst_app);
    let marker = temp_str("peptide_ssf2_loadtest.txt");
    let _ = std::fs::remove_file(&marker);
    ssf2_converter::abc_inject::patch_file_with(&src_swf, &dst_swf, |abc| {
        ssf2_converter::abc_inject::inject_load_test(abc, SSF2_DOC_CLASS, &marker, &ch, "battlefield")
    })?;
    macos_resign(&dst_app);
    let exe = ssf2_exe_path(&dst_app);
    println!("booting timer-driven load test (char={ch}, no per-frame bridge)…");
    let mut child = std::process::Command::new(&exe).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn()?;
    crate::platform::keep_behind(child.id());
    let mp = Path::new(&marker);
    for i in 0..30 {
        std::thread::sleep(std::time::Duration::from_millis(1000));
        if let Ok(s) = std::fs::read_to_string(mp) {
            println!("  t{}s: {}", i+1, s.trim());
            if s.contains("[object") && !s.contains("LIB:null") { break; }
        }
    }
    let final_s = std::fs::read_to_string(mp).unwrap_or_default();
    let _ = child.kill();
    if final_s.contains("LIB:[object") {
        println!("PASS: content PARSED via timer-driven load — the per-frame bridge IO was starving it! ({})", final_s.trim());
        Ok(())
    } else {
        bail!("content still did not parse via timer-driven load: {:?} — not a starvation issue", final_s.trim())
    }
}

/// `peptide ssf2 autojump [char]` — timer-driven autospawn (no per-frame bridge,
/// which would starve the load) + jump probe; boots, lets the engine load+spawn
/// the char, jump it, and capture the per-frame trajectory. The LIVE SSF2 jump.
fn cmd_autojump(args: &[String]) -> Result<()> {
    let ch = args.iter().rev().find(|a| !a.starts_with("--")).cloned().unwrap_or_else(|| "sandbag".into());
    let app = ssf2_app();
    let src_swf = ssf2_swf_path(&app);
    let dst_app = stage_patched_bundle(&app)?;
    let dst_swf = ssf2_swf_path(&dst_app);
    let traj = crate::ssf2_bridge::traj_path();
    let _ = std::fs::remove_file(&traj);
    let chc = ch.clone();
    ssf2_converter::abc_inject::patch_file_with(&src_swf, &dst_swf, |abc| {
        ssf2_converter::abc_inject::inject_autospawn(abc, SSF2_DOC_CLASS, &chc, "battlefield", 3000)?;
        ssf2_converter::abc_inject::inject_jump_probe(abc, SSF2_DOC_CLASS, &traj, 0)
    })?;
    macos_resign(&dst_app);
    let exe = ssf2_exe_path(&dst_app);
    println!("booting timer-driven autospawn ({ch} on battlefield) — load→startMatch→jump, capturing…");
    let mut child = std::process::Command::new(&exe).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn()?;
    crate::platform::keep_behind(child.id());
    let mut last_lines = 0;
    for i in 0..30 {
        std::thread::sleep(std::time::Duration::from_millis(1000));
        if let Ok(s) = std::fs::read_to_string(&traj) {
            let n = s.lines().count();
            if n != last_lines { println!("  t{}s: trajectory has {} samples", i+1, n); last_lines = n; }
            if n > 40 { break; }
        }
    }
    let traj_data = std::fs::read_to_string(&traj).unwrap_or_default();
    let _ = child.kill();
    if traj_data.trim().is_empty() {
        bail!("no trajectory captured — match/jump did not occur");
    }
    // parse the jump: columns t,X,Y,YSpeed ; apex = min Y
    let mut min_y = f64::INFINITY; let mut max_y = f64::NEG_INFINITY; let mut samples = 0;
    for line in traj_data.lines() {
        let cols: Vec<&str> = line.split(',').collect();
        if cols.len() >= 3 { if let Ok(y) = cols[2].parse::<f64>() { min_y = min_y.min(y); max_y = max_y.max(y); samples += 1; } }
    }
    println!("\n=== LIVE SSF2 {ch} jump trajectory (t,X,Y,YSpeed) — {samples} samples ===");
    for line in traj_data.lines().take(60) { println!("{line}"); }
    if min_y.is_finite() {
        println!("\nSSF2 LIVE apex: ground Y≈{:.1} → apex Y≈{:.1}  (displacement ≈ {:.1} SSF2 px)", max_y, min_y, max_y - min_y);
    }
    Ok(())
}

/// Quickboot the real SSF2.app (for manual observation / cross-checking).
fn cmd_launch(_args: &[String]) -> Result<()> {
    let app = ssf2_app();
    if !app.exists() {
        bail!("SSF2.app not found at {} (set $PEPTIDE_SSF2_APP)", app.display());
    }
    println!("launching {}", app.display());
    #[cfg(target_os = "macos")]
    {
        // `-g`: opened behind whatever the user is doing (the command line is not the game's window)
        std::process::Command::new("open").arg("-g").arg(&app).spawn()
            .map_err(|e| anyhow!("open {}: {e}", app.display()))?;
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        bail!("ssf2 launch is only wired for macOS standalone right now");
    }
}

/// `--key value` string flag (sibling of [`flag_f64`]).
fn flag_str(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

/// `peptide ssf2 identify <dir|file> [--copy-stages <dest>] [--kind character|stage|other]`
///
/// Classify SSF2 `.ssf` resources (character / stage / other) without converting them.
/// Over a directory it scans every `.ssf` and prints a table; `--copy-stages <dest>`
/// copies the stages out (renamed to `<Main.id>.ssf`) for iteration. `--kind` filters
/// the printed rows. This is how we find which `DATn.ssf` are stages to port.
fn cmd_identify(args: &[String]) -> Result<()> {
    let copy_dest = flag_str(args, "--copy-stages").map(PathBuf::from);
    let kind_filter = flag_str(args, "--kind");
    // first positional that isn't a flag or a flag's value.
    let flag_vals: std::collections::HashSet<&String> =
        ["--copy-stages", "--kind"].iter().filter_map(|f| {
            args.iter().position(|a| a == f).and_then(|i| args.get(i + 1))
        }).collect();
    let target = args.iter()
        .find(|a| !a.starts_with("--") && !flag_vals.contains(a))
        .ok_or_else(|| anyhow!("usage: peptide ssf2 identify <dir|file> [--copy-stages <dest>] [--kind character|stage|other]"))?;
    let target = PathBuf::from(target);

    // collect the .ssf files to classify (a single file, or every .ssf in a dir).
    let mut files: Vec<PathBuf> = Vec::new();
    if target.is_dir() {
        for entry in std::fs::read_dir(&target).with_context(|| format!("read dir {}", target.display()))? {
            let p = entry?.path();
            if p.extension().and_then(|e| e.to_str()) == Some("ssf") { files.push(p); }
        }
        files.sort_by_key(|p| natural_key(p.file_name().and_then(|s| s.to_str()).unwrap_or("")));
    } else {
        files.push(target.clone());
    }
    if files.is_empty() { bail!("no .ssf files found at {}", target.display()); }

    if let Some(dest) = &copy_dest {
        std::fs::create_dir_all(dest).with_context(|| format!("create {}", dest.display()))?;
    }

    let (mut n_char, mut n_stage, mut n_other, mut n_err, mut copied) = (0, 0, 0, 0, 0);
    println!("{:<16} {:<22} {:<10} detail", "file", "id", "kind");
    println!("{}", "-".repeat(72));
    for f in &files {
        let stem = f.file_name().and_then(|s| s.to_str()).unwrap_or("?");
        match ssf2_converter::classify_ssf(f) {
            Ok(c) => {
                let id = c.id.clone().unwrap_or_else(|| "-".into());
                let detail = match &c.kind {
                    ssf2_converter::AssetKind::Character(ids) => format!("chars: {}", ids.join(",")),
                    ssf2_converter::AssetKind::Stage => format!("markers: {}", c.markers.join(",")),
                    ssf2_converter::AssetKind::Other => String::new(),
                };
                match c.kind {
                    ssf2_converter::AssetKind::Character(_) => n_char += 1,
                    ssf2_converter::AssetKind::Stage => n_stage += 1,
                    ssf2_converter::AssetKind::Other => n_other += 1,
                }
                let show = kind_filter.as_deref().map(|k| k == c.kind.label()).unwrap_or(true);
                if show {
                    println!("{:<16} {:<22} {:<10} {}", stem, id, c.kind.label(), detail);
                }
                // copy stages out, named by Main.id (fallback: original stem).
                if let (Some(dest), ssf2_converter::AssetKind::Stage) = (&copy_dest, &c.kind) {
                    let name = c.id.clone().unwrap_or_else(|| stem.trim_end_matches(".ssf").to_string());
                    let out = dest.join(format!("{name}.ssf"));
                    std::fs::copy(f, &out).with_context(|| format!("copy {} -> {}", f.display(), out.display()))?;
                    copied += 1;
                }
            }
            Err(e) => { n_err += 1; eprintln!("{:<16} <error: {}>", stem, e); }
        }
    }
    println!("\n{} files: {} character, {} stage, {} other, {} error",
             files.len(), n_char, n_stage, n_other, n_err);
    if let Some(dest) = &copy_dest {
        println!("copied {copied} stage(s) to {}", dest.display());
    }
    Ok(())
}

/// `peptide ssf2 doctor`
///
/// The SSF2 analogue of `peptide <fraymakers.dat> _ doctor`: resolve every `SSF2.swf` engine
/// symbol the stage/parallax path depends on BY NAME against the installed SSF2, and print a
/// pass/fail checklist. A recompiled SSF2 renumbers every class/trait, so this triages a new
/// build in one command. Exits non-zero if a critical symbol is gone.
fn cmd_doctor(_args: &[String]) -> Result<()> {
    let app = ssf2_app();
    let swf = ssf2_swf_path(&app);
    if !swf.exists() { bail!("SSF2.swf not found at {}", swf.display()); }
    let bytes = std::fs::read(&swf)?;
    let checks = ssf2_converter::engine_probe::ssf2_doctor(&bytes)
        .ok_or_else(|| anyhow!("could not read ABC from {}", swf.display()))?;

    eprintln!("\nPeptide SSF2 doctor — {}", swf.display());
    eprintln!("{}", "-".repeat(60));
    let (mut ok, mut crit, mut warn) = (0usize, 0usize, 0usize);
    let mut last = "";
    for c in &checks {
        if c.group != last { eprintln!("  {}:", c.group); last = c.group; }
        if c.ok {
            ok += 1;
            eprintln!("    [ ok ] {}", c.label);
        } else {
            let tag = if c.critical { crit += 1; "CRITICAL" } else { warn += 1; "warn" };
            eprintln!("    [MISS] {:<32} MISSING ({tag}) — {}", c.label, c.why);
        }
    }
    eprintln!("\n  {ok}/{} resolved · {crit} critical missing · {warn} warnings", checks.len());
    if crit > 0 {
        bail!("{crit} critical SSF2 symbol(s) gone — this SSF2 build isn't compatible with the \
               converter. Re-find the names above and update crates/ssf2-converter/src/engine_probe.rs");
    }
    if warn > 0 {
        eprintln!("  -> converter is compatible; {warn} non-critical symbol(s) changed (the \
                   camera-background parallax preview degrades to a fallback rate).");
    } else {
        eprintln!("doctor: all SSF2 engine symbols resolved — converter is compatible with this build");
    }
    Ok(())
}

/// `peptide ssf2 stage <file.ssf> [--out <dir>] [--id <id>] [--info]`
///
/// Parse an SSF2 stage `.ssf` into a geometry model and emit a Fraymakers stage
/// package (geometry-only MVP) under `<out>/<id>/`. `--info` prints the parsed
/// model without emitting; `--id` overrides the output id (e.g. to avoid clashing
/// with a built-in FM stage id). This is the SSF2->FM stage converter front end.
fn cmd_stage(args: &[String]) -> Result<()> {
    // The converter library logs through the `log` facade and never installs a logger, so without
    // this every warning the stage path raises is discarded -- including the audit that says a
    // script asks for content the package does not ship. A conversion that cannot tell you what it
    // could not do is worth less than one that can. `-v` adds the debug detail; RUST_LOG still wins.
    crate::convert::init_logger(args.iter().any(|a| a == "-v" || a == "--verbose"));
    let out_dir = flag_str(args, "--out").unwrap_or_else(|| "stages".to_string());
    let id_override = flag_str(args, "--id");
    let info_only = args.iter().any(|a| a == "--info");
    let flag_vals: std::collections::HashSet<&String> =
        ["--out", "--id"].iter().filter_map(|f| {
            args.iter().position(|a| a == f).and_then(|i| args.get(i + 1))
        }).collect();
    let target = args.iter()
        .find(|a| !a.starts_with("--") && !flag_vals.contains(a))
        .ok_or_else(|| anyhow!("usage: peptide ssf2 stage <file.ssf> [--out <dir>] [--id <id>] [--info]"))?;
    let target = PathBuf::from(target);

    // A stage can name a class it does not carry: the game keeps shared packages loaded alongside
    // and resolves it from there. The converter looks in the same place, so it is told where the
    // game is through the one config source of truth.
    if std::env::var_os("PEPTIDE_SSF2_APP").is_none() && std::env::var_os("PEPTIDE_SSF2_DATA").is_none() {
        // SAFETY: single-threaded CLI setup, before any conversion work starts.
        unsafe { std::env::set_var("PEPTIDE_SSF2_APP", ssf2_app()) };
    }
    let mut model = ssf2_converter::parse_stage(&target)?;
    // suffix the content id (`<id>ssf2`) so a converted stage can't shadow a built-in FM
    // stage; the display name stays the clean SSF2 name. `--id` overrides outright.
    if let Some(id) = id_override { model.id = id; }
    else if !model.id.ends_with("ssf2") { model.id = format!("{}ssf2", model.id); }

    // print the parsed model (the phase-2 exit criteria: platforms + bounds + spawns).
    println!("stage '{}' \"{}\" (from {})", model.id, model.display_name, target.display());
    if !model.fm_music.is_empty() {
        println!("  music: {} (FM){}", model.fm_music.join(", "),
            if model.ssf2_music.is_empty() { String::new() } else { format!("  [SSF2: {}]", model.ssf2_music.join(", ")) });
    }
    if let Some(f) = model.main_floor() {
        println!("  main floor: x[{:.1},{:.1}] top y={:.1} (w={:.1})", f.rect.left(), f.rect.right(), f.rect.top(), f.rect.w);
    }
    for p in model.platforms.iter().filter(|p| p.drop_through) {
        println!("  platform:   x[{:.1},{:.1}] top y={:.1} (w={:.1}) drop-through", p.rect.left(), p.rect.right(), p.rect.top(), p.rect.w);
    }
    if std::env::var("PEPTIDE_STAGE_DEBUG").is_ok() {
        for p in &model.platforms {
            println!("  [all-plat] x[{:.1},{:.1}] y[{:.1},{:.1}] w={:.1} solid={} cx={:.1}",
                p.rect.left(), p.rect.right(), p.rect.top(), p.rect.bottom(), p.rect.w, !p.drop_through, p.rect.x + p.rect.w/2.0);
        }
        if let Some((l, r)) = model.ledges { println!("  [ledges] left={:.1} right={:.1}", l, r); }
    }
    if let Some(r) = &model.death_box {
        println!("  death box:  x[{:.1},{:.1}] y[{:.1},{:.1}]", r.left(), r.right(), r.top(), r.bottom());
    }
    if let Some(r) = &model.camera_box {
        println!("  camera box: x[{:.1},{:.1}] y[{:.1},{:.1}]", r.left(), r.right(), r.top(), r.bottom());
    }
    for s in &model.entrances {
        println!("  entrance {}: ({:.1},{:.1}){}", s.index, s.x, s.y, if s.face_left { " <-" } else { " ->" });
    }
    for s in &model.respawns {
        println!("  respawn  {}: ({:.1},{:.1})", s.index, s.x, s.y);
    }
    // art layer composition: backdrop/background (behind), parallax camera-bgs, stage-depth
    // frames, and a foreground (in front of fighters). A structure-foreground folds into the
    // background, so `fg` here means a DISTINCT in-front prop survived.
    let a = &model.art;
    println!("  art: background={} foreground={} parallax={} stage_frames={} stage_elements={}",
        a.background.len(), !a.foreground.is_empty(), a.parallax.len(), a.stage_frames.len(),
        a.stage_elements.len());
    for hz in &model.hazards {
        println!("  hazard: {:<12} ({:.0},{:.0}) {:.0}x{:.0} dmg={} kb={} code={}",
            hz.label, hz.x, hz.y, hz.w, hz.h, hz.damage, hz.knockback,
            if hz.reconstructed_script.is_some() { "own" } else { "none" });
    }
    for w in &model.warnings {
        eprintln!("  warning: {w}");
    }

    if info_only { return Ok(()); }

    let out_root = PathBuf::from(&out_dir);
    let (dir, fraytools) = ssf2_converter::emit_stage(&model, &out_root)?;
    println!("\nemitted FM stage package -> {}", dir.display());
    println!("publish with: peptide export --project \"{}\"", fraytools.display());
    Ok(())
}

/// Sort key that orders `DAT2.ssf` before `DAT10.ssf` (numeric run aware).
fn natural_key(s: &str) -> (String, u64) {
    let digits: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
    let prefix: String = s.chars().take_while(|c| !c.is_ascii_digit()).collect();
    (prefix, digits.parse().unwrap_or(0))
}

fn help() {
    print!("\
peptide ssf2 — SSF2 engine integration (physics model + scaling + quickboot)

USAGE:
  peptide ssf2 stats  <file.ssf> [char] [--size-mult N] [--ssf2-fps N] [--fm-fps N]
        Raw SSF2 constants → simulated ground-truth motion → derived Fraymakers stats.
  peptide ssf2 scale  <file.ssf> [char] [--size-mult N]
        Compact raw → derived scaling table (velocity_scale / accel_scale).
  peptide ssf2 identify <dir|file> [--copy-stages <dest>] [--kind character|stage|other]
        Classify SSF2 .ssf resources (character / stage / other). Over a dir, scans
        every .ssf and prints a table; --copy-stages copies the stages out (named by id).
  peptide ssf2 stage <file.ssf> [--out <dir>] [--id <id>] [--info]
        Convert an SSF2 stage .ssf into a Fraymakers stage package (geometry-only:
        floor + soft platforms, death/camera boxes, entrance/respawn points). Prints
        the parsed model; --info skips emitting; --id overrides the output id.
  peptide ssf2 tags <file.ssf|swf>
        What a package holds, tag by tag: every picture (size, storage, bound class),
        morph shapes and text, shapes painting with a picture, and pictures placed
        directly (a Bitmap child with no shape between).
  peptide ssf2 doctor
        Resolve, by name, every SSF2.swf engine symbol the stage/parallax path
        depends on (view dims, VcamBGSettings schema + mode consts) against the
        installed SSF2 and print a pass/fail checklist. Triages a new SSF2 build.
  peptide ssf2 launch
        Quickboot the standalone SSF2.app for manual observation.

  ── runtime code-execution bridge (AVM2 injection; see docs/ssf2-runtime-bridge.md) ──
  peptide ssf2 doctor
        Resolve every SSF2 engine symbol the injections need against the installed
        SSF2.swf and print a checklist (run after an SSF2 update; exits non-zero if a
        critical symbol moved).
  peptide ssf2 patch <in.swf> <out.swf>
        Inject the debug bridge into an SSF2 SWF.
  peptide ssf2 install [--heartbeat|--echo]
        Build build/SSF2-patched.app with the bridge injected (ad-hoc resigned).
  peptide ssf2 selftest [--heartbeat|--echo]
        Install, boot, and verify LIVE: injected code runs (default), fires every
        frame (--heartbeat), or round-trips host commands (--echo).

The ground-truth motion is computed by re-implementing SSF2's per-frame physics
(reverse-engineered from SSF2.swf; see docs/ssf2-physics-model.md). The whole
scaling profile is driven by the single `size_multiplier` knob in
mappings/character/stats.jsonc (velocity_scale = size_mult × 30/60;
accel_scale = size_mult × (30/60)²). Validated live in Fraymakers — see
docs/ssf2-scaling-validation.md.
");
}

/// The injection half of `peptide ssf2 doctor` — resolve every SSF2 engine symbol the
/// runtime-bridge injections depend on against the installed SSF2.swf and print a grouped
/// checklist (the stage/parallax half is `cmd_doctor`). Exits non-zero if a critical
/// symbol is missing.
/// Decompile one class out of the installed SSF2 (or any swf given as `--swf <path>`).
/// What a package holds, tag by tag: every picture (id, size, how it is stored, the class bound
/// to it) and every shape that paints with one. The question it answers is "which piece of art
/// did the walk lose, and how was it stored", asked whenever a converted stage draws less than
/// the game does.
fn cmd_tags(args: &[String]) -> Result<()> {
    let path = args.first().map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("usage: peptide ssf2 tags <file.ssf|swf>"))?;
    let raw = std::fs::read(&path)?;
    let bytes = ssf2_converter::ssf::decompress(&raw).unwrap_or(raw);
    let swf_buf = ssf2_converter::swf::decompress_swf(&bytes[..])?;
    let swf = ssf2_converter::swf::parse_swf(&swf_buf)?;
    let mut names: std::collections::BTreeMap<u16, String> = Default::default();
    let mut kinds: std::collections::BTreeMap<String, usize> = Default::default();
    for tag in &swf.tags {
        let name = format!("{tag:?}");
        let kind = name.split(|c: char| !c.is_alphanumeric()).next().unwrap_or("?").to_string();
        *kinds.entry(kind).or_default() += 1;
        if let ssf2_converter::swf::Tag::SymbolClass(links) = tag {
            for l in links { names.insert(l.id, String::from_utf8_lossy(l.class_name.as_bytes()).to_string()); }
        }
    }
    println!("tags by kind:");
    for (k, n) in &kinds { println!("  {n:5}  {k}"); }
    println!("pictures:");
    for tag in &swf.tags {
        let (id, how, size) = match tag {
            ssf2_converter::swf::Tag::DefineBitsLossless(b) => (b.id, format!("lossless v{} {:?}", b.version, b.format), format!("{}x{}", b.width, b.height)),
            ssf2_converter::swf::Tag::DefineBitsJpeg3(j) => (j.id, format!("jpeg3 (alpha {} bytes)", j.alpha_data.len()), format!("{} bytes", j.data.len())),
            ssf2_converter::swf::Tag::DefineBitsJpeg2 { id, jpeg_data } => (*id, "jpeg2".into(), format!("{} bytes", jpeg_data.len())),
            ssf2_converter::swf::Tag::DefineBits { id, jpeg_data } => (*id, "jpeg".into(), format!("{} bytes", jpeg_data.len())),
            _ => continue,
        };
        println!("  #{id:<6} {size:>14}  {how:<28} {}", names.get(&id).map(String::as_str).unwrap_or(""));
    }
    println!("morph shapes and text (placed as themselves, no shape):");
    for tag in &swf.tags {
        match tag {
            ssf2_converter::swf::Tag::DefineMorphShape(m) => {
                let b = &m.start.shape_bounds;
                println!("  morph#{:<5} start bounds {}x{}  fills {}", m.id, (b.x_max - b.x_min).to_pixels(), (b.y_max - b.y_min).to_pixels(), m.start.fill_styles.len());
            }
            ssf2_converter::swf::Tag::DefineText(t) => {
                let b = &t.bounds;
                println!("  text#{:<6} bounds {}x{}  records {}", t.id, (b.x_max - b.x_min).to_pixels(), (b.y_max - b.y_min).to_pixels(), t.records.len());
            }
            _ => {}
        }
    }
    println!("shapes painting with a picture:");
    for tag in &swf.tags {
        let ssf2_converter::swf::Tag::DefineShape(s) = tag else { continue };
        let fills: Vec<String> = s.styles.fill_styles.iter().map(|f| match f {
            ssf2_converter::swf::FillStyle::Bitmap { id, is_smoothed, is_repeating, .. } => format!("bitmap#{id}{}{}", if *is_smoothed { "" } else { " unsmoothed" }, if *is_repeating { " repeat" } else { "" }),
            ssf2_converter::swf::FillStyle::Color(c) => format!("rgba({},{},{},{})", c.r, c.g, c.b, c.a),
            ssf2_converter::swf::FillStyle::LinearGradient(_) => "linear".into(),
            ssf2_converter::swf::FillStyle::RadialGradient(_) => "radial".into(),
            ssf2_converter::swf::FillStyle::FocalGradient { .. } => "focal".into(),
        }).collect();
        if fills.iter().any(|f| f.starts_with("bitmap")) || s.styles.fill_styles.is_empty() {
            let b = &s.shape_bounds;
            println!("  shape#{:<5} bounds {}x{}  fills [{}] {}", s.id, (b.x_max - b.x_min).to_pixels(), (b.y_max - b.y_min).to_pixels(), fills.join(", "), names.get(&s.id).map(String::as_str).unwrap_or(""));
        }
    }
    // a placement can name a picture directly (no shape in between): the player then makes a
    // Bitmap child, which is what a script finds by class and paints into
    let mut pics: std::collections::BTreeSet<u16> = Default::default();
    for tag in &swf.tags {
        match tag {
            ssf2_converter::swf::Tag::DefineBitsLossless(b) => { pics.insert(b.id); }
            ssf2_converter::swf::Tag::DefineBitsJpeg3(j) => { pics.insert(j.id); }
            ssf2_converter::swf::Tag::DefineBitsJpeg2 { id, .. } | ssf2_converter::swf::Tag::DefineBits { id, .. } => { pics.insert(*id); }
            _ => {}
        }
    }
    println!("pictures placed directly (a Bitmap child, no shape):");
    let report = |owner: String, tags: &[ssf2_converter::swf::Tag]| {
        for t in tags {
            if let ssf2_converter::swf::Tag::PlaceObject(p) = t {
                if let ssf2_converter::swf::PlaceObjectAction::Place(id) = p.action {
                    if pics.contains(&id) {
                        let m = p.matrix.unwrap_or_default();
                        println!("  {owner} depth {} -> picture#{id} at ({}, {}) scale ({:.3}, {:.3}) name {:?}", p.depth, m.tx.to_pixels(), m.ty.to_pixels(), m.a.to_f32(), m.d.to_f32(), p.name.map(|n| String::from_utf8_lossy(n.as_bytes()).to_string()));
                    }
                }
            }
        }
    };
    report("root".into(), &swf.tags);
    for tag in &swf.tags {
        if let ssf2_converter::swf::Tag::DefineSprite(s) = tag {
            report(format!("sprite#{}{}", s.id, names.get(&s.id).map(|n| format!(" ({n})")).unwrap_or_default()), &s.tags);
        }
    }
    // one clip, child by child: what it places at which depth, with the shape's own edges, so a
    // collision clip can be read without converting the stage
    if let Some(want) = flag_str(args, "--sprite").and_then(|s| s.parse::<u16>().ok()) {
        let shapes: std::collections::BTreeMap<u16, &ssf2_converter::swf::Shape> = swf.tags.iter()
            .filter_map(|t| if let ssf2_converter::swf::Tag::DefineShape(s) = t { Some((s.id, s)) } else { None }).collect();
        let Some(sprite) = swf.tags.iter().find_map(|t| match t {
            ssf2_converter::swf::Tag::DefineSprite(s) if s.id == want => Some(s), _ => None }) else { bail!("no sprite #{want}") };
        println!("sprite#{want}{}: {} frames", names.get(&want).map(|n| format!(" ({n})")).unwrap_or_default(), sprite.num_frames);
        let mut frame = 0;
        for t in &sprite.tags {
            match t {
                ssf2_converter::swf::Tag::ShowFrame => frame += 1,
                ssf2_converter::swf::Tag::PlaceObject(p) => {
                    let m = p.matrix.unwrap_or_default();
                    let what = match p.action {
                        ssf2_converter::swf::PlaceObjectAction::Place(id) | ssf2_converter::swf::PlaceObjectAction::Replace(id) => {
                            if let Some(s) = shapes.get(&id) {
                                let b = &s.shape_bounds;
                                format!("shape#{id} {}x{} at ({}, {}) fills {}", (b.x_max - b.x_min).to_pixels(), (b.y_max - b.y_min).to_pixels(), b.x_min.to_pixels(), b.y_min.to_pixels(), s.styles.fill_styles.len())
                            } else { format!("#{id}{}", names.get(&id).map(|n| format!(" ({n})")).unwrap_or_default()) }
                        }
                        ssf2_converter::swf::PlaceObjectAction::Modify => "modify".to_string(),
                    };
                    println!("  frame {frame} depth {:<3} {what} matrix ({:.3}, {:.3}, {:.3}, {:.3}) tx ({}, {}) name {:?}",
                        p.depth, m.a.to_f32(), m.b.to_f32(), m.c.to_f32(), m.d.to_f32(), m.tx.to_pixels(), m.ty.to_pixels(),
                        p.name.map(|n| String::from_utf8_lossy(n.as_bytes()).to_string()));
                    if let ssf2_converter::swf::PlaceObjectAction::Place(id) = p.action {
                        if let Some(s) = shapes.get(&id) {
                            // the shape's edges, as moves and lines (a curve by its anchor), with the fill on each side
                            let (mut cx, mut cy) = (0.0f64, 0.0f64);
                            let mut line = String::new();
                            for r in &s.shape {
                                match r {
                                    ssf2_converter::swf::ShapeRecord::StyleChange(sc) => {
                                        if let Some(mv) = &sc.move_to { cx = mv.x.to_pixels(); cy = mv.y.to_pixels(); line.push_str(&format!(" M({cx:.0},{cy:.0})")); }
                                        if sc.fill_style_0.is_some() || sc.fill_style_1.is_some() { line.push_str(&format!(" fill[{:?}|{:?}]", sc.fill_style_0, sc.fill_style_1)); }
                                    }
                                    ssf2_converter::swf::ShapeRecord::StraightEdge { delta } => { cx += delta.dx.to_pixels(); cy += delta.dy.to_pixels(); line.push_str(&format!(" L({cx:.0},{cy:.0})")); }
                                    ssf2_converter::swf::ShapeRecord::CurvedEdge { control_delta, anchor_delta } => {
                                        cx += control_delta.dx.to_pixels() + anchor_delta.dx.to_pixels(); cy += control_delta.dy.to_pixels() + anchor_delta.dy.to_pixels();
                                        line.push_str(&format!(" C({cx:.0},{cy:.0})"));
                                    }
                                }
                            }
                            println!("      edges:{line}");
                        }
                    }
                }
                ssf2_converter::swf::Tag::RemoveObject(r) => println!("  frame {frame} depth {} removed", r.depth),
                _ => {}
            }
        }
    }
    Ok(())
}

fn cmd_compile(args: &[String]) -> Result<()> {
    use ssf2_converter::compile;
    if args.first().map(|s| s.as_str()) == Some("--lang-suite") {
        let rt = compile::Runtime::locate().map(|d| compile::Runtime::load(&d)).unwrap_or_default();
        print!("{}", compile::suite::language_suite(&rt));
        return Ok(());
    }
    let path = args.first().map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("usage: peptide ssf2 compile <file.ssf> [class | --fighters]"))?;
    let want = args.get(1).cloned();
    let rt = compile::Runtime::locate().map(|d| compile::Runtime::load(&d)).unwrap_or_default();
    let (m, renamed) = rt.read_package(&std::fs::read(&path)?)?;
    for p in &renamed { eprintln!("api names: {p}"); }
    // each fighter's script and its projectiles', as the runtime's path writes them: what they
    // call that nothing provides, one line each
    if want.as_deref() == Some("--fighters") {
        use ssf2_converter::emit;
        let sc = &ssf2_converter::mappings::character_stats().scaling;
        let units = emit::Units { size: sc.size_multiplier, speed: sc.velocity_scale(), item_toss: sc.item_toss_speed, accel: sc.accel_scale() };
        for f in m.fighters() {
            let mut unknown: std::collections::BTreeSet<String> = Default::default();
            let mut problems: Vec<String> = Vec::new();
            let none = Default::default();
            let no_entities = Default::default();
            let out = emit::fighter_script(&m, &rt, &f, &[], units, &f.getter, &[], &none, &no_entities, Default::default());
            unknown.extend(out.unknown);
            problems.extend(out.problems);
            for (name, decl) in f.projectiles.entries().iter().chain(f.items.entries()) {
                let spec = emit::ObjectSpec {
                    kind: "projectile", others: &[], effects: &none, entities: &no_entities,
                    class: decl.get("classAPI").and_then(|v| v.class()).unwrap_or("PeptideProjectile").to_string(),
                    clip: decl.get("linkage_id").and_then(|v| v.str()).unwrap_or(name).to_string(),
                    clip_sym: None,
                    stats: decl.clone(),
                    autolink: Default::default(),
                    data: String::new(),
                };
                let o = emit::object_script(&m, &rt, &spec, &emit::Decls::of(&f), &[], units, name);
                unknown.extend(o.unknown);
                problems.extend(o.problems);
            }
            for u in &unknown { println!("{}\t{}\tunknown\t{u}", path.display(), f.getter); }
            for p in &problems { println!("{}\t{}\tproblem\t{p}", path.display(), f.getter); }
        }
        return Ok(());
    }
    let out = compile::compile_package(&m, &rt);
    for (name, text) in out.classes.iter().filter(|(n, _)| want.as_deref().is_none_or(|w| n.contains(w))) {
        let _ = name;
        println!("{text}");
    }
    for p in &out.problems { eprintln!("problem: {p}"); }
    eprintln!("{} classes, {} problems, {} names nothing defines", out.classes.len(), out.problems.len(), out.unknown.len());
    Ok(())
}

fn cmd_model(args: &[String]) -> Result<()> {
    use ssf2_converter::model::{ast, SourceModel};
    let path = args.first().map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("usage: peptide ssf2 model <file.ssf> [class] [method]"))?;
    let want = args.get(1).cloned();
    let method = args.get(2).cloned();
    let m = SourceModel::read(&std::fs::read(&path)?)?;
    if want.as_deref() == Some("--fighters") {
        for f in m.fighters() {
            println!("fighter {} | class {} | clip {}", f.getter, f.class().unwrap_or("?"), f.clip().unwrap_or("?"));
            for a in m.animations(f.clip().unwrap_or("")) {
                println!("  {:24} frame {:4}  stance {}", a.label, a.frame, a.stance_class.as_deref().unwrap_or("-"));
            }
            let projectiles: Vec<String> = f.projectiles.entries().iter().map(|(k, v)| format!("{k} ({} / {})",
                v.get("classAPI").and_then(|c| c.class()).unwrap_or("?"), v.get("linkage_id").and_then(|c| c.str()).unwrap_or("?"))).collect();
            println!("  projectiles: {}", projectiles.join(", "));
        }
        return Ok(());
    }
    if want.is_none() {
        println!("{} classes, {} bound to symbols, {} sprites, root {} frames",
            m.classes.len(), m.symbols.len(), m.sprites.len(), m.root.frame_count);
    }
    let show = |name: &str, f: &ast::Function| {
        let ps: Vec<String> = f.params.iter().map(|p| format!("{}: {}{}", p.name, p.type_name.as_deref().unwrap_or("*"),
            p.default.as_ref().map(|d| format!(" = {d}")).unwrap_or_default())).collect();
        let mut body = String::new();
        ast::print_stmts(&mut body, &f.body, 1);
        println!("function {name}({}): {} {{\n{body}}}", ps.join(", "), f.return_type.as_deref().unwrap_or("*"));
    };
    for c in m.classes.values().filter(|c| want.as_deref().is_none_or(|w| c.name.contains(w))) {
        println!("// class {} : {}{} | block {} | fields {} methods {} frame scripts {} unreadable {}",
            c.name, c.super_name, c.symbol.map(|s| format!(" [symbol {s}]")).unwrap_or_default(),
            c.block, c.fields.len(), c.methods.len(), c.frame_scripts.len(), c.unreadable.len());
        if want.is_none() { continue }
        if method.is_none() {
            for f in &c.fields {
                println!("//  {}{}var {}: {}{}", if f.is_static { "static " } else { "" }, if f.is_const { "const " } else { "" },
                    f.name, f.type_name.as_deref().unwrap_or("*"), f.default.as_ref().map(|d| format!(" = {d}")).unwrap_or_default());
            }
            for fs in &c.frame_scripts { println!("//  frame {} -> {}", fs.frame, fs.method); }
            if let Some(sp) = c.symbol.and_then(|s| m.sprites.get(&s)) {
                println!("//  timeline: {} frames, labels {:?}", sp.frame_count, sp.labels);
            }
            for u in &c.unreadable { println!("//  unreadable: {u}"); }
        }
        if method.as_deref().is_none_or(|x| x == "new") { show("new", &c.constructor); }
        for x in c.methods.iter().filter(|x| method.as_deref().is_none_or(|w| x.name == w)) {
            show(&format!("{}{:?} {}", if x.is_static { "static " } else { "" }, x.kind, x.name), &x.function);
        }
    }
    Ok(())
}

fn cmd_decompile(args: &[String]) -> Result<()> {
    let want = args.first().cloned()
        .ok_or_else(|| anyhow::anyhow!("usage: peptide ssf2 decompile <ClassName> [method] [--swf <file>] [--asm]"))?;
    let method = args.get(1).filter(|a| !a.starts_with("--")).cloned();
    let swf_path = args.iter().position(|a| a == "--swf").and_then(|i| args.get(i + 1))
        .map(PathBuf::from).unwrap_or_else(|| ssf2_swf_path(&ssf2_app()));
    if !swf_path.exists() { bail!("swf not found: {}", swf_path.display()); }
    let raw = std::fs::read(&swf_path)?;
    // .ssf content is a wrapped/compressed SWF; the engine's own SWF is a plain one. Try it as
    // shipped content first so the same command reads a stage, a character or the engine.
    let bytes = ssf2_converter::ssf::decompress(&raw).unwrap_or(raw);
    let shown = swf_path.display().to_string();
    // `--asm`: the methods as instruction listings, for code the decompiler leaves out
    let asm = args.iter().any(|a| a == "--asm");
    cmd_decompile_in(&bytes, &want, method.as_deref(), &shown, asm)
}

fn cmd_decompile_in(bytes: &[u8], want: &str, method: Option<&str>, shown: &str, asm: bool) -> Result<()> {
    let bytes = bytes.to_vec();
    let method = method.map(|m| m.to_string());
    let swf = ssf2_converter::swf_parser::parse(&bytes)?;
    let mut found = false;
    for block in &swf.abc_blocks {
        let Ok(abc) = ssf2_converter::abc_parser::parse(block) else { continue };
        for class in abc.classes.iter().filter(|c| c.name.contains(want)) {
            found = true;
            println!("// class {} extends {}", class.name, class.super_name);
            // what the class states a value for: its constants and typed fields, as written
            let slots: Vec<&ssf2_converter::abc_parser::Trait> = class.class_methods.iter()
                .chain(class.instance_methods.iter()).filter(|t| t.kind == 0 || t.kind == 6).collect();
            for (n, v) in ssf2_converter::abc_parser::class_const_values(&abc, &class.name) {
                match slots.iter().find(|t| t.name == n).and_then(|t| t.type_name.as_deref()) {
                    Some(ty) => println!("// const {n}: {ty} = {v}"),
                    None => println!("// const {n} = {v}"),
                }
            }
            // the static initializer, asked for as `cinit`: where a static const with a computed
            // value is filled in
            if method.as_deref() == Some("cinit") {
                if let Some(body) = abc.method_bodies.iter().find(|b| b.method_idx == class.class_init_idx) {
                    println!("{}", ssf2_converter::decompiler::decompile_method(body, &abc, "cinit", &[]));
                }
            }
            // the constructor, asked for as `new`: it is where a class builds what it holds
            if method.as_deref().is_none_or(|m| m == "new") {
                if let Some(body) = abc.method_bodies.iter().find(|b| b.method_idx == class.constructor_idx) {
                    let params: Vec<String> = abc.methods.get(body.method_idx as usize)
                        .map(|m| (0..m.param_count).map(|i| format!("arg{i}")).collect())
                        .unwrap_or_default();
                    println!("{}", ssf2_converter::decompiler::decompile_method(body, &abc, "new", &params));
                }
            }
            for t in class.class_methods.iter().chain(class.instance_methods.iter()) {
                if method.as_deref().is_some_and(|m| m != t.name) { continue; }
                let Some(body) = abc.method_bodies.iter().find(|b| b.method_idx == t.method_idx) else { continue };
                if asm {
                    println!("// {} ({} bytes)\n{}", t.name, body.bytecode.len(), ssf2_converter::disasm::listing(&body.bytecode, &abc));
                    continue;
                }
                let params: Vec<String> = abc.methods.get(t.method_idx as usize)
                    .map(|m| (0..m.param_count).map(|i| format!("arg{i}")).collect())
                    .unwrap_or_default();
                // a parameter's declared default shows as the line that gives a missing one it
                // (a bare call takes it: the source's push and timer ticks read it)
                println!("{}", ssf2_converter::abc_parser::with_param_defaults(
                    ssf2_converter::decompiler::decompile_method(body, &abc, &t.name, &params),
                    abc.methods.get(t.method_idx as usize)));
            }
        }
    }
    if !found { bail!("no class matching {want:?} in {shown}"); }
    Ok(())
}

fn cmd_doctor_injections(_args: &[String]) -> Result<()> {
    let app = ssf2_app();
    let swf = ssf2_swf_path(&app);
    if !swf.exists() { bail!("SSF2.swf not found at {}", swf.display()); }
    let abc = ssf2_converter::abc_inject::read_abc(&swf)?;
    let results = ssf2_converter::abc_inject::verify_symbols(&abc);
    let mut group = "";
    let (mut ok_n, mut miss_crit, mut miss_warn) = (0usize, 0usize, 0usize);
    println!("SSF2 symbol doctor — {}\n", swf.display());
    for (sym, ok) in &results {
        if sym.group != group { group = sym.group; println!("  [{group}]"); }
        let label = match sym.member { Some(m) => format!("{}.{}", sym.class, m), None => sym.class.to_string() };
        if *ok { ok_n += 1; println!("    ok      {label}"); }
        else if sym.critical { miss_crit += 1; println!("    MISSING {label}  ({})", sym.why); }
        else { miss_warn += 1; println!("    warn    {label}  ({})", sym.why); }
    }
    println!("\n{ok_n} ok, {miss_crit} missing (critical), {miss_warn} missing (non-critical)");
    if miss_crit > 0 {
        eprintln!("\ncritical SSF2 symbols moved — patching is blocked. update SSF2_MANIFEST + the inject_* helpers.");
        std::process::exit(1);
    }
    Ok(())
}

pub fn run_cli(args: &[String]) -> Result<()> {
    // args == everything after the `ssf2` word.
    let sub = args.first().map(|s| s.as_str()).unwrap_or("help");
    let rest = if args.len() > 1 { &args[1..] } else { &[] };
    match sub {
        "manifest" => cmd_manifest(rest),
        "doctor" => cmd_doctor(rest).and_then(|()| cmd_doctor_injections(rest)),
        "stats" => cmd_stats(rest),
        "scale" => cmd_scale(rest),
        "identify" => cmd_identify(rest),
        "findsound" => cmd_findsound(rest),
        "stage" => cmd_stage(rest),
        "patch" => cmd_patch(rest),
        "install" => cmd_install(rest).map(|_| ()),
        "selftest" => cmd_selftest(rest),
        "loadtest" => cmd_loadtest(rest),
        "autojump" => cmd_autojump(rest),
        // Read a class out of SSF2's own code. The stage class is decompiled as part of a
        // conversion, but the ENGINE classes it hands its clips to are what decide what those
        // clips do at runtime (a stage hands its lighting sheets to one), and without reading them a port can only guess at behaviour the timeline
        // does not carry.
        "decompile" | "class" => cmd_decompile(rest),
        // what a package is made of, as the converter's source model reads it
        "model" => cmd_model(rest),
        // a package's classes, compiled by the new converter's compiler
        "compile" => cmd_compile(rest),
        "tags" => cmd_tags(rest),
        "session" => crate::ssf2_bridge::session(rest),
        "tell" => crate::ssf2_bridge::tell(rest),
        "log" => crate::ssf2_bridge::log(rest),
        "send" => crate::ssf2_bridge::send(rest),
        "jumpcapture" => crate::ssf2_bridge::jumpcapture(rest),
        "launch" | "boot" | "run" => cmd_launch(rest),
        "help" | "-h" | "--help" => { help(); Ok(()) }
        other => {
            eprintln!("peptide ssf2: unknown subcommand {other:?}\n");
            help();
            std::process::exit(2);
        }
    }
}


/// Where a sound named by id actually lives.
///
/// A fighter can name a clip its own file does not carry, and the manifest's packages do not
/// account for all of them. This walks every `.ssf` beside the game and says which one holds it,
/// so a missing sound is a fact rather than a guess.
fn cmd_findsound(args: &[String]) -> Result<()> {
    let want: std::collections::BTreeSet<String> = args.iter().cloned().collect();
    if want.is_empty() { anyhow::bail!("usage: peptide ssf2 findsound <sound-id> [<sound-id>…]") }
    let dir = std::env::var("PEPTIDE_SSF2_DATA").map(std::path::PathBuf::from)
        .ok()
        .or_else(|| std::env::var("PEPTIDE_SSF2_APP").ok()
            .map(|a| std::path::PathBuf::from(a).join("Contents/Resources/data")))
        .filter(|p| p.is_dir())
        .ok_or_else(|| anyhow::anyhow!("set $PEPTIDE_SSF2_DATA or $PEPTIDE_SSF2_APP"))?;
    let mut files: Vec<_> = std::fs::read_dir(&dir)?.filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "ssf")).collect();
    files.sort();
    // the game's own movie counts as a place to look
    if let Some(up) = dir.parent() { let m = up.join("SSF2.swf"); if m.is_file() { files.push(m); } }
    eprintln!("looking through {} files for {}", files.len(),
        want.iter().cloned().collect::<Vec<_>>().join(", "));
    let mut found: std::collections::BTreeSet<String> = Default::default();
    for f in &files {
        let Ok(raw) = std::fs::read(f) else { continue };
        let data = if f.extension().is_some_and(|e| e == "swf") { raw }
                   else { match ssf2_converter::ssf::decompress(&raw) { Ok(d) => d, Err(_) => continue } };
        let mut names: Vec<String> = ssf2_converter::sound_extractor::sound_names_in_movie(&data)
            .into_iter().filter(|n| want.contains(n)).collect();
        // a clip is asked for across packages by its exported name, same as a sound is
        names.extend(ssf2_converter::sound_extractor::symbol_names_in_movie(&data)
            .into_iter().filter(|n| want.contains(n)));
        names.sort();
        names.dedup();
        if names.is_empty() { continue }
        println!("{}: {}", f.file_name().unwrap_or_default().to_string_lossy(), names.join(", "));
        found.extend(names);
    }
    let missing: Vec<_> = want.difference(&found).cloned().collect();
    if !missing.is_empty() { println!("nowhere: {}", missing.join(", ")); }
    Ok(())
}
