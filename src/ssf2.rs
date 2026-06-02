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

use anyhow::{anyhow, bail, Result};
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
        .unwrap_or_else(|| PathBuf::from("/Applications/SSF2.app"))
}

/// Copy the SSF2 app bundle to a writable staging location for patching. Prefers
/// the SAME folder as the real app (sibling `SSF2-patched.app`); if that folder
/// can't be written, falls back to a per-platform cache dir, then the temp dir.
/// Returns the absolute path of the staged copy. Errors only if every candidate
/// location fails (e.g. no writable disk at all).
fn stage_patched_bundle(app: &Path) -> Result<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(parent) = app.parent() {
        candidates.push(parent.join("SSF2-patched.app"));
    }
    candidates.push(
        dirs::cache_dir()
            .map(|d| d.join("peptide"))
            .unwrap_or_else(|| std::env::temp_dir().join("peptide"))
            .join("SSF2-patched.app"),
    );
    candidates.push(std::env::temp_dir().join("peptide").join("SSF2-patched.app"));

    let mut last = String::from("no candidate locations");
    for dst in &candidates {
        if let Some(parent) = dst.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                last = format!("cannot create {}: {e}", parent.display());
                continue;
            }
        }
        let _ = std::fs::remove_dir_all(dst);
        match std::process::Command::new("cp").arg("-R").arg(app).arg(dst).status() {
            Ok(st) if st.success() => return Ok(dst.clone()),
            Ok(_) => last = format!("cp -R into {} failed", dst.display()),
            Err(e) => last = format!("cp -R into {} failed: {e}", dst.display()),
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

/// `peptide ssf2 install [--heartbeat]` — build a patched copy of SSF2.app under
/// ./build with the bridge payload injected, ad-hoc resigned so it boots.
fn cmd_install(args: &[String]) -> Result<PathBuf> {
    let heartbeat = args.iter().any(|a| a == "--heartbeat");
    let app = ssf2_app();
    let src_swf = app.join("Contents/Resources/SSF2.swf");
    if !src_swf.exists() { bail!("SSF2.swf not found at {}", src_swf.display()); }
    // stage the copy next to the real app (CWD-independent), with safe fallbacks.
    let dst_app = stage_patched_bundle(&app)?;
    // patch the swf in place inside the copy
    let dst_swf = dst_app.join("Contents/Resources/SSF2.swf");
    let marker = "/tmp/peptide_ssf2_marker.txt";
    let echo = args.iter().any(|a| a == "--echo");
    if echo {
        ssf2_converter::abc_inject::patch_file_with(&src_swf, &dst_swf, |abc| {
            ssf2_converter::abc_inject::inject_command_channel(
                abc, SSF2_DOC_CLASS, "/tmp/peptide_ssf2_cmd.txt", "/tmp/peptide_ssf2_resp.txt")
        })?;
    } else if heartbeat {
        ssf2_converter::abc_inject::patch_file_with(&src_swf, &dst_swf, |abc| {
            ssf2_converter::abc_inject::inject_enterframe_heartbeat(abc, SSF2_DOC_CLASS, marker)
        })?;
    } else {
        ssf2_converter::abc_inject::patch_file(&src_swf, &dst_swf, SSF2_DOC_CLASS, marker, "alive")?;
    }
    // strip quarantine + ad-hoc resign so Gatekeeper/AIR accepts the modified bundle
    let _ = std::process::Command::new("xattr").arg("-cr").arg(&dst_app).status();
    let _ = std::process::Command::new("codesign").args(["--force","--deep","--sign","-"]).arg(&dst_app).status();
    println!("installed patched bridge: {}", dst_app.display());
    Ok(dst_app)
}

/// Install a patched SSF2.app carrying the command bridge and return the bundle
/// path. The bridge is an async `flash.net.Socket` that dials into our loopback
/// server on `port` (127.0.0.1) — the host must already be listening on it.
pub fn install_patched(port: u16) -> Result<PathBuf> {
    let app = ssf2_app();
    let src_swf = app.join("Contents/Resources/SSF2.swf");
    if !src_swf.exists() { bail!("SSF2.swf not found at {}", src_swf.display()); }
    // Stage the patched copy NEXT TO the real SSF2 app (keeps it in the same
    // resources/signing context), falling back to a safe per-platform cache dir
    // when that folder isn't writable (e.g. /Applications without admin rights, or
    // the GUI's working directory not being the project root). Absolute paths only,
    // so this works regardless of the process CWD.
    let dst_app = stage_patched_bundle(&app)?;
    let dst_swf = dst_app.join("Contents/Resources/SSF2.swf");
    ssf2_converter::abc_inject::patch_file_with(&src_swf, &dst_swf, |abc| {
        ssf2_converter::abc_inject::inject_socket_bridge(abc, SSF2_DOC_CLASS, "127.0.0.1", port)?;
        // a second ENTER_FRAME listener that logs Characters[0] physics each frame
        ssf2_converter::abc_inject::inject_jump_probe(
            abc, SSF2_DOC_CLASS, crate::ssf2_bridge::TRAJ_PATH, 0)
    })?;
    let _ = std::process::Command::new("xattr").arg("-cr").arg(&dst_app).status();
    let _ = std::process::Command::new("codesign").args(["--force","--deep","--sign","-"]).arg(&dst_app).status();
    Ok(dst_app)
}

/// `peptide ssf2 selftest` — install, boot, confirm injected code executed
/// (the marker file appears). Proves the code-execution bridge end-to-end.
fn cmd_selftest(args: &[String]) -> Result<()> {
    let echo = args.iter().any(|a| a == "--echo");
    // echo mode: the engine reads the cmd file every frame, so it must exist before boot.
    if echo {
        std::fs::write("/tmp/peptide_ssf2_cmd.txt", b"")?;
        let _ = std::fs::remove_file("/tmp/peptide_ssf2_resp.txt");
    }
    let dst_app = cmd_install(args)?;
    let marker = Path::new("/tmp/peptide_ssf2_marker.txt");
    let _ = std::fs::remove_file(marker);
    let exe = dst_app.join("Contents/MacOS/SSF2");
    println!("booting patched SSF2…");
    let mut child = std::process::Command::new(&exe)
        .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
        .spawn()?;

    if echo {
        let resp = Path::new("/tmp/peptide_ssf2_resp.txt");
        // give it a moment to install the listener, then send a command
        std::thread::sleep(std::time::Duration::from_millis(3000));
        let token = "PING-12345";
        std::fs::write("/tmp/peptide_ssf2_cmd.txt", token.as_bytes())?;
        let mut got = String::new();
        for _ in 0..20 {
            std::thread::sleep(std::time::Duration::from_millis(300));
            if let Ok(s) = std::fs::read_to_string(resp) { if s == token { got = s; break; } got = s; }
        }
        let _ = child.kill();
        let _ = std::process::Command::new("pkill").args(["-f","SSF2-patched"]).status();
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
        let _ = std::process::Command::new("pkill").args(["-f","SSF2-patched"]).status();
        bail!("FAIL: marker never appeared — injected code did not run (check the patch)");
    }
    let first = std::fs::read_to_string(marker).unwrap_or_default();
    let result = if heartbeat {
        // confirm the value CHANGES across ~1.2s → the handler fires every frame
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
    let _ = std::process::Command::new("pkill").args(["-f","SSF2-patched"]).status();
    result
}

/// `peptide ssf2 loadtest` — install ONLY a timer-driven load test (no per-frame
/// bridge), boot, and report whether the resource content parses (LIB/STATS).
fn cmd_loadtest(args: &[String]) -> Result<()> {
    let ch = args.iter().rev().find(|a| !a.starts_with("--")).cloned().unwrap_or_else(|| "sandbag".into());
    let app = ssf2_app();
    let src_swf = app.join("Contents/Resources/SSF2.swf");
    let dst_app = PathBuf::from("build/SSF2-patched.app");
    let _ = std::fs::remove_dir_all(&dst_app);
    if !std::process::Command::new("cp").arg("-R").arg(&app).arg(&dst_app).status()?.success() { bail!("cp failed"); }
    let dst_swf = dst_app.join("Contents/Resources/SSF2.swf");
    let marker = "/tmp/peptide_ssf2_loadtest.txt";
    let _ = std::fs::remove_file(marker);
    ssf2_converter::abc_inject::patch_file_with(&src_swf, &dst_swf, |abc| {
        ssf2_converter::abc_inject::inject_load_test(abc, SSF2_DOC_CLASS, marker, &ch, "battlefield")
    })?;
    let _ = std::process::Command::new("xattr").arg("-cr").arg(&dst_app).status();
    let _ = std::process::Command::new("codesign").args(["--force","--deep","--sign","-"]).arg(&dst_app).status();
    let exe = dst_app.join("Contents/MacOS/SSF2");
    println!("booting timer-driven load test (char={ch}, no per-frame bridge)…");
    let mut child = std::process::Command::new(&exe).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn()?;
    let mp = Path::new(marker);
    for i in 0..30 {
        std::thread::sleep(std::time::Duration::from_millis(1000));
        if let Ok(s) = std::fs::read_to_string(mp) {
            println!("  t{}s: {}", i+1, s.trim());
            if s.contains("[object") && !s.contains("LIB:null") { break; } // content parsed
        }
    }
    let final_s = std::fs::read_to_string(mp).unwrap_or_default();
    let _ = child.kill(); let _ = std::process::Command::new("pkill").args(["-f","SSF2-patched"]).status();
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
    let src_swf = app.join("Contents/Resources/SSF2.swf");
    let dst_app = PathBuf::from("build/SSF2-patched.app");
    let _ = std::fs::remove_dir_all(&dst_app);
    if !std::process::Command::new("cp").arg("-R").arg(&app).arg(&dst_app).status()?.success() { bail!("cp failed"); }
    let dst_swf = dst_app.join("Contents/Resources/SSF2.swf");
    let traj = crate::ssf2_bridge::TRAJ_PATH;
    let _ = std::fs::remove_file(traj);
    let chc = ch.clone();
    ssf2_converter::abc_inject::patch_file_with(&src_swf, &dst_swf, |abc| {
        ssf2_converter::abc_inject::inject_autospawn(abc, SSF2_DOC_CLASS, &chc, "battlefield", 3000)?;
        ssf2_converter::abc_inject::inject_jump_probe(abc, SSF2_DOC_CLASS, traj, 0)
    })?;
    let _ = std::process::Command::new("xattr").arg("-cr").arg(&dst_app).status();
    let _ = std::process::Command::new("codesign").args(["--force","--deep","--sign","-"]).arg(&dst_app).status();
    let exe = dst_app.join("Contents/MacOS/SSF2");
    println!("booting timer-driven autospawn ({ch} on battlefield) — load→startMatch→jump, capturing…");
    let mut child = std::process::Command::new(&exe).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn()?;
    let tp = Path::new(traj);
    let mut last_lines = 0;
    for i in 0..30 {
        std::thread::sleep(std::time::Duration::from_millis(1000));
        if let Ok(s) = std::fs::read_to_string(tp) {
            let n = s.lines().count();
            if n != last_lines { println!("  t{}s: trajectory has {} samples", i+1, n); last_lines = n; }
            if n > 40 { break; }
        }
    }
    let traj_data = std::fs::read_to_string(tp).unwrap_or_default();
    let _ = child.kill(); let _ = std::process::Command::new("pkill").args(["-f","SSF2-patched"]).status();
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
        std::process::Command::new("open").arg(&app).spawn()
            .map_err(|e| anyhow!("open {}: {e}", app.display()))?;
    }
    #[cfg(not(target_os = "macos"))]
    {
        bail!("ssf2 launch is only wired for macOS standalone right now");
    }
    Ok(())
}

fn help() {
    print!("\
peptide ssf2 — SSF2 engine integration (physics model + scaling + quickboot)

USAGE:
  peptide ssf2 stats  <file.ssf> [char] [--size-mult N] [--ssf2-fps N] [--fm-fps N]
        Raw SSF2 constants → simulated ground-truth motion → derived Fraymakers stats.
  peptide ssf2 scale  <file.ssf> [char] [--size-mult N]
        Compact raw → derived scaling table (velocity_scale / accel_scale).
  peptide ssf2 launch
        Quickboot the standalone SSF2.app for manual observation.

  ── runtime code-execution bridge (AVM2 injection; see docs/ssf2-runtime-bridge.md) ──
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

pub fn run_cli(args: &[String]) -> Result<()> {
    // args == everything after the `ssf2` word.
    let sub = args.first().map(|s| s.as_str()).unwrap_or("help");
    let rest = if args.len() > 1 { &args[1..] } else { &[] };
    match sub {
        "stats" => cmd_stats(rest),
        "scale" => cmd_scale(rest),
        "patch" => cmd_patch(rest),
        "install" => cmd_install(rest).map(|_| ()),
        "selftest" => cmd_selftest(rest),
        "loadtest" => cmd_loadtest(rest),
        "autojump" => cmd_autojump(rest),
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
