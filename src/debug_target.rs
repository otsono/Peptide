//! debug_target — the OOP seam that lets ONE command vocabulary drive BOTH
//! engines. `interpreter::parse` turns a human line into an engine-agnostic
//! `Command`; a `DebugTarget` executes it. `FraymakersTarget` speaks the HashLink
//! socket/wire protocol; `Ssf2Target` (src/ssf2_target.rs) speaks AVM2 reflection
//! over file-IPC. `run_command` is the single dispatcher used by the session
//! layer, so `spawn sandbag`, `e match.getCharacters()[0]...`, `hold down+special`,
//! `seq …` etc. behave identically regardless of which engine is attached.

use anyhow::Result;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use crate::interpreter::{self, Command, SpawnArgs};

/// A debugger backend: executes the engine-agnostic [`Command`] set. Each method
/// returns the engine's textual reply (or `()` for a fire-and-forget shutdown).
///
/// FEATURE SURFACE: a host-facing feature (matchStatus, char icons, and anything
/// added later) is modelled here as a trait method whose DEFAULT implementation
/// just evaluates the engine helper of the same name (`matchStatus()`,
/// `iconFeed(slot)`, …). Both backends implement `eval`, so a new feature
/// automatically reaches BOTH engines the moment each one's `eval` knows the
/// expression — Fraymakers via `commands.hsx`, SSF2 via `ssf2_target`'s evaluator.
/// An engine that genuinely can't do a feature overrides the method to say so
/// (e.g. SSF2 has no stock-icon pipeline → `char_icon` returns `None`).
pub trait DebugTarget {
    fn eval(&mut self, expr: &str) -> Result<String>;
    /// the script run inside one entity's own scope (`s` the stage, a digit a character slot).
    /// the default sends the console eval with `self` bound to that entity, which is what an
    /// engine without per-entity interpreters can offer; an engine with them overrides this.
    fn eval_in(&mut self, target: &str, expr: &str) -> Result<String> {
        self.eval(&bind_self(target, expr))
    }
    /// A value a fighter keeps by name between its moves (a stored charge), read or set. The
    /// default asks the content's own api on the console's character binding; an engine that
    /// keeps the value somewhere else overrides this.
    fn var(&mut self, idx: usize, name: &str, value: Option<f64>) -> Result<String> {
        match value {
            Some(v) => { self.eval(&format!("p{idx}.setGlobalVariable(\"{name}\", {v})"))?; self.eval(&format!("p{idx}.getGlobalVariable(\"{name}\")")) }
            None => self.eval(&format!("p{idx}.getGlobalVariable(\"{name}\")")),
        }
    }
    fn spawn(&mut self, args: &SpawnArgs) -> Result<String>;
    fn hold(&mut self, mask: u32) -> Result<String>;
    /// engine frames per tick. an engine that runs at its own rate only says so.
    fn speed(&mut self, n: u32) -> Result<String>;
    fn seq(&mut self, masks: &[u32]) -> Result<String>;
    fn console(&mut self) -> Result<String>;
    fn add_character(&mut self) -> Result<String>;
    fn exit(&mut self) -> Result<()>;
    fn load(&mut self) -> Result<String>;

    /// The per-character status feed (`MATCHSTATUS:<id>|<dmg>|<anim>;…`) the host
    /// polls into the matchStatus widget. Default: eval the engine's `matchStatus()`
    /// helper. `None` when there's no live match (empty feed).
    fn match_status(&mut self) -> Result<Option<String>> {
        Ok(non_empty(strip_eval(self.eval("matchStatus()")?)))
    }

    /// A character's stock icon for match `slot` (`ICON:<slot>:<hex>;<palette>`),
    /// host-requested on demand. Default: eval `iconFeed(slot)`. Engines without a
    /// stock-icon pipeline override this to return `None` (the widget keeps a glyph).
    fn char_icon(&mut self, slot: u32) -> Result<Option<String>> {
        Ok(non_empty(strip_eval(self.eval(&format!("iconFeed({slot})"))?)))
    }

    /// Drive the camera: set its mode, park it, zoom it, or report it.
    ///
    /// Both engines have the same controls under different names, which is why this is a trait
    /// method and not two dialects of eval: the point of it is to put the SAME view on both
    /// engines so a converted stage can be compared with its source frame for frame.
    fn camera(&mut self, _mode: Option<&str>, _pos: Option<(f64, f64)>, _zoom: Option<f64>) -> Result<String> {
        Ok("cam: no camera control on this engine yet".into())
    }

    /// Dump the live object tree to `depth` — the stage-triage ground truth (every
    /// live object's name/class/position/size/frame). SSF2 walks its display list
    /// via reflection; an engine without a tree walk declares the gap.
    fn tree(&mut self, _depth: u32) -> Result<String> {
        Ok("tree: no live object-tree walk on this engine yet (SSF2-only)".into())
    }

    /// Screenshot the live frame to `path` (PNG), from inside the engine.
    ///
    /// The tree walk proves an object EXISTS, is visible, sized and positioned. None of that is
    /// evidence that a pixel of it reaches the screen, and the two get confused easily. Reading
    /// the window host-side is not a substitute: a headless session's window has no capturable
    /// surface, so the engine has to photograph itself. An engine that can't declares the gap.
    ///
    /// WHAT THIS CAPTURES, AND WHY IT IS NOT WHAT A PLAYER SEES
    /// -------------------------------------------------------
    /// This photographs the DISPLAY ROOT, not the camera. On BOTH engines that means the frame
    /// can contain things the game would never show: art outside the camera bounds, the full
    /// extent of a stage the camera only ever pans across, and regions the camera crops away.
    /// It also carries no camera zoom, so two engines' frames are not comparable by default --
    /// the same stage photographs at different apparent scales purely because their cameras were
    /// never consulted.
    ///
    /// So a shot is evidence about what was DRAWN, not about what is VISIBLE in play. Before
    /// comparing frames across engines, or before treating a frame as "what the player sees",
    /// set the camera position/mode on both sides first and capture that. Otherwise the
    /// comparison silently measures framing differences instead of art differences.
    fn shot(&mut self, _path: &std::path::Path) -> Result<String> {
        Ok("shot: no in-engine screenshot on this engine yet".into())
    }

    /// Character `idx`'s live animation clock. Default: eval the engine's `animFeed(i)`
    /// helper. `None` when that character doesn't exist (or there's no live match).
    ///
    /// This is the primitive every frame-accurate operation is built on. Waiting on
    /// wall-clock time is not a valid substitute: an engine's frame rate varies with
    /// load, so a sleep long enough to be safe is mostly idle and a sleep short enough
    /// to be quick silently observes a half-played animation. Read the clock instead.
    fn char_anim(&mut self, idx: usize) -> Result<Option<AnimState>> {
        Ok(non_empty(strip_eval(self.eval(&format!("animFeed({idx})"))?))
            .and_then(|s| AnimState::parse(&s)))
    }

    /// Play a NAMED move, named as Fraymakers names its animations.
    ///
    /// The point of this being a trait method rather than two dialects of eval: an input does
    /// not mean the same thing on both engines. A held direction plus attack is a forward SMASH
    /// on one and a forward TILT on the other, because one of them tells them apart by how far
    /// the stick is pushed and an injected bitmask can only say "held" -- and some moves no
    /// input reaches at all. So comparing what a move DOES has to start by naming the move.
    fn play_move(&mut self, idx: usize, name: &str) -> Result<String> {
        // the engine's own state machine, named as it names its states: playing the animation
        // alone would draw the move without the state that makes it one
        let r = self.eval(&format!("p{idx}.toState(CState.{})", name.to_ascii_uppercase()))?;
        Ok(format!("MOVE:p{idx} {name} {}", strip_eval(r).trim()))
    }

    /// A whole hit WINDOW at the engine's own frame rate, when the engine can record one.
    ///
    /// Default: none, and the caller samples with one round trip per frame instead. An engine
    /// that pushes per-frame telemetry overrides this, because polling cannot see a launch it
    /// only holds for a frame or two: SSF2 answers a read every ~14 of its frames, so the
    /// hitstop can begin and end between two samples and the knockback is simply never in the
    /// data. The recorded stream has every frame in it.
    fn hit_trace(&mut self, _idx: usize, _frames: u32, _masks: &[u32])
        -> Result<Option<Vec<HitState>>> { Ok(None) }

    /// One character's live HIT STATE (damage taken, velocity, state name).
    ///
    /// Default: the engine's own `hitFeed` helper, one round trip per sample. SSF2 has no
    /// interpreter to hold a helper, so it overrides this and composes the same record from
    /// reflection reads.
    fn char_hit(&mut self, idx: usize) -> Result<Option<HitState>> {
        Ok(non_empty(strip_eval(self.eval(&format!("hitFeed({idx})"))?))
            .and_then(|s| HitState::parse(&s)))
    }

    /// Arm/disarm the engine's own per-frame recorder, clearing its buffer.
    ///
    /// Default: declare the gap. Fraymakers doesn't need one — it PUSHES per-frame `ANIM:`
    /// telemetry over its socket and its hscript eval is fast enough that `await` can poll
    /// the clock directly. SSF2 has neither, so it implements this with injected bytecode.
    fn record(&mut self, _on: bool) -> Result<String> {
        Ok("record: this engine streams per-frame telemetry already — use `await` \
            (no host-side recorder needed)".into())
    }

    /// Read back the recorded frame history and stop recording. See [`Self::record`].
    ///
    /// `raw` dumps one line per frame (with position) instead of the run-length summary. The
    /// summary answers how long a move lasted; the raw stream answers how far it moved each
    /// frame, which is the only way to derive a speed multiplier from observation.
    fn frame_trace(&mut self, _raw: bool, _all: bool) -> Result<String> {
        Ok("trace: this engine has no host-side frame buffer — its per-frame ANIM \
            telemetry is already in the session log, and `await` reports frame counts"
            .into())
    }
}

/// Where the next decoded frame lands. Set when a `shot` goes out, read when the engine's hex
/// reply arrives — the two happen on different threads, so the path can't just be a local.
static SHOT_PATH: std::sync::Mutex<Option<std::path::PathBuf>> = std::sync::Mutex::new(None);

/// Remember where the frame that's about to come back should be written.
pub fn set_shot_path(p: &std::path::Path) {
    *SHOT_PATH.lock().unwrap() = Some(p.to_path_buf());
}

/// Decode a `SHOT:` hex payload to the requested PNG path and describe what happened. Shared by
/// the session daemon and the trait path so both report a frame the same way.
pub fn write_shot(hex: &str) -> String {
    let hex = hex.trim();
    let path = SHOT_PATH.lock().unwrap().clone()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp/peptide-shot.png"));
    if hex.is_empty() {
        return "SHOT: the engine returned an empty frame".into();
    }
    let bytes: std::result::Result<Vec<u8>, _> = (0..hex.len() / 2)
        .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16)).collect();
    match bytes {
        Err(e) => format!("SHOT: frame hex is malformed: {e}"),
        Ok(b) => match std::fs::write(&path, &b) {
            Err(e) => format!("SHOT: could not write {}: {e}", path.display()),
            Ok(()) => format!("SHOT: wrote {} ({} bytes)", path.display(), b.len()),
        },
    }
}

/// One sample of a character's animation clock: which animation, how far into it, how
/// long it is, and where the character is. `frame`/`total` are `-1` when the engine
/// couldn't report them.
#[derive(Clone, Debug, PartialEq)]
pub struct AnimState {
    pub anim: String,
    pub frame: i32,
    pub total: i32,
    pub x: f64,
    pub y: f64,
    pub state: String,
}

impl AnimState {
    /// Parse the shared `<anim>|<frame>|<total>|<x>|<y>|<state>` wire shape both
    /// backends emit. Returns `None` if it isn't that shape, so a garbled or
    /// no-live-match reply reads as "no sample" rather than a bogus one.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        // an engine reply can carry a leading tag or trailing noise; take the last
        // line that actually has the field count we need.
        let line = s.lines().rev().find(|l| l.matches('|').count() >= 5)?;
        let f: Vec<&str> = line.rsplitn(6, '|').collect();
        // rsplitn yields reversed; re-order to (anim, frame, total, x, y, state)
        let (state, y, x, total, frame, anim) = (f[0], f[1], f[2], f[3], f[4], f[5]);
        Some(AnimState {
            anim: anim.trim().trim_start_matches("E:").trim().to_string(),
            frame: frame.trim().parse().unwrap_or(-1),
            total: total.trim().parse().unwrap_or(-1),
            x: x.trim().parse().unwrap_or(0.0),
            y: y.trim().parse().unwrap_or(0.0),
            state: state.trim().to_string(),
        })
    }
    /// `true` once the clock has reached the end of the animation.
    pub fn at_end(&self) -> bool { self.total > 0 && self.frame >= self.total - 1 }
}

/// One frame's worth of a character's HIT STATE: what it has taken, where it is, and how fast the
/// engine says it is going.
///
/// The engines keep no record of a hit once it is over, so a hit is measured while it happens.
/// Two readings of the same launch:
///
/// * the VELOCITY the engine itself calculated. This is the knockback, stated rather than
///   inferred: whatever the engine worked out from the hitbox's base and growth is the number
///   sitting in the victim's velocity. It reads zero for as long as the hitstop freeze lasts, so
///   it is taken from the first frame it is non-zero after the hit.
/// * how far the victim MOVED, sampled from position. This is what the launch did after
///   friction, gravity and DI got at it, which is a different question and worth keeping as a
///   cross-check, and it is the only reading available if an engine will not state its velocity.
#[derive(Clone, Debug, PartialEq)]
pub struct HitState {
    pub damage: f64,
    pub x: f64,
    pub y: f64,
    pub state: String,
    /// The engine's own velocity, when it states one. Absent for a feed that carries no
    /// velocity fields, in which case only the position reading is reported.
    pub vel: Option<(f64, f64)>,
}

impl HitState {
    /// Parse the shared `<damage>|<x>|<y>|<state>` wire shape, optionally followed by
    /// `|<vx>|<vy>`, the same way `AnimState` does: the last line carrying the field count, so a
    /// tagged or noisy reply still reads. The velocity is optional so an engine that does not
    /// state one still reports its hits.
    pub fn parse(s: &str) -> Option<Self> {
        let line = s.trim().lines().rev().find(|l| l.matches('|').count() >= 3)?;
        let num = |t: &str| t.trim().trim_start_matches("E:").trim().parse::<f64>().ok();
        let f: Vec<&str> = line.split('|').collect();
        let (damage, x, y) = (f[0], f.get(1).copied().unwrap_or(""), f.get(2).copied().unwrap_or(""));
        let state = f.get(3).copied().unwrap_or("");
        let vel = match (f.get(4).and_then(|t| num(t)), f.get(5).and_then(|t| num(t))) {
            (Some(vx), Some(vy)) => Some((vx, vy)),
            _ => None,
        };
        Some(HitState {
            damage: num(damage)?,
            x: num(x).unwrap_or(0.0),
            y: num(y).unwrap_or(0.0),
            state: state.trim().to_string(),
            vel,
        })
    }

    /// The IMPULSE between two frames: how much the engine changed this character's velocity.
    ///
    /// A launch is a jump in velocity, not a velocity, and that distinction is the whole
    /// measurement. Reading the velocity itself catches whatever else is acting -- a victim
    /// falling at terminal speed reads far faster than the hit that struck it, and one standing
    /// still reads zero for the whole hitstop. The CHANGE isolates the hit: gravity adds its
    /// small constant every frame, and the frame knockback lands on dwarfs it.
    ///
    /// Returned as magnitude and degrees, the way `moved_from` returns a move, so the two read
    /// the same (0 = right, counter-clockwise, matching how SSF2 states a hit's `direction`).
    /// SSF2 counts Y downward and Fraymakers does too, so neither needs flipping here.
    pub fn impulse_from(&self, prev: &HitState) -> Option<(f64, f64, f64, f64)> {
        let ((vx, vy), (px, py)) = (self.vel?, prev.vel?);
        let (dx, dy) = (vx - px, vy - py);
        let mag = (dx * dx + dy * dy).sqrt();
        let deg = (-dy).atan2(dx).to_degrees();
        Some((mag, if deg < 0.0 { deg + 360.0 } else { deg + 0.0 }, dx, dy))
    }
    /// How far this moved from `prev`, and which way: magnitude, then degrees with 0 = right and
    /// angles counter-clockwise, which is how SSF2 states a hit's `direction` so the two read the
    /// same way. One sample apart, so on Fraymakers this is per-frame; see the sampling note on
    /// SSF2's own `char_hit`.
    pub fn moved_from(&self, prev: &HitState) -> (f64, f64) {
        let (dx, dy) = (self.x - prev.x, self.y - prev.y);
        let mag = (dx * dx + dy * dy).sqrt();
        let deg = (-dy).atan2(dx).to_degrees();
        // + 0.0 so a straight-right launch reads 0, not the "-0" a negative zero prints as
        (mag, if deg < 0.0 { deg + 360.0 } else { deg + 0.0 })
    }
}

/// Watch one character for HITS and report each outcome.
///
/// A hit leaves no record behind, so it is caught as it happens: sample every frame and watch
/// the damage. Where damage went UP, a hit landed, and the launch it gave is the FIRST movement
/// in the samples that follow. It is read there rather than off the hit's own frame because both
/// engines freeze the victim for the hitstop: on the frame the damage registers the velocity is
/// still zero, and the launch appears once the freeze ends. Taking the first movement rather
/// than the fastest keeps it the hit's own impulse -- wait any longer and gravity has added more
/// speed than the hit did.
///
/// The sampling goes through the shared vocabulary (`damage._damage`,
/// `physics.currentVelocityX`), which SSF2 answers from its own fields, so this measures the
/// same quantity on either engine. The sampler is passed in because the session daemon owns its
/// own reply channel; [`watch_hits`] is the plain target wrapper.
/// How many samples after a hit the launch is looked for. Long enough to outlast the hitstop,
/// short enough that a fall can't accumulate more speed than the hit gave.
const LAUNCH_WINDOW: usize = 20;
// the least a launch is: a hit frame stating less states none (a throw's let-go clears it)
const LAUNCH_FLOOR: f64 = 0.05;

pub fn watch_hits_with(
    idx: usize,
    frames: u32,
    raw: bool,
    mut sample: impl FnMut(Option<u32>) -> Option<HitState>,
    masks: &[u32],
) -> String {
    let Some(first) = sample(None) else {
        return format!("HITS:p{idx} nothing to watch (no live character)");
    };
    let mut samples: Vec<HitState> = vec![first];
    for i in 0..frames {
        // one input frame per sample, so the move happens INSIDE the window being watched
        let Some(s) = sample(masks.get(i as usize).copied()) else { break };
        samples.push(s);
    }
    // damage only ever goes UP on a hit; a drop is a reset or a fresh stock, which is not one
    if raw {
        // every sample, so a hit can be read frame by frame instead of through the summary
        let mut out = format!(
            "HITS:p{idx} {} samples (sample: damage x y moved vx vy state)\n", samples.len());
        for (i, s) in samples.iter().enumerate() {
            let moved = if i == 0 { 0.0 } else { s.moved_from(&samples[i - 1]).0 };
            let (vx, vy) = s.vel.unwrap_or((f64::NAN, f64::NAN));
            out.push_str(&format!(
                "  {i:>3} {:>6.1} {:>9.2} {:>9.2} {moved:>7.2} {vx:>7.2} {vy:>7.2} {}\n",
                s.damage, s.x, s.y, s.state));
        }
        return out.trim_end().to_string();
    }
    let landed: Vec<usize> = (1..samples.len())
        .filter(|&i| samples[i].damage > samples[i - 1].damage + 1e-6)
        .collect();
    let last = samples.last().map(|s| s.damage).unwrap_or(0.0);
    if landed.is_empty() {
        return format!("HITS:p{idx} none in {} samples (damage held at {last})", samples.len());
    }
    // how many samples the watch took: the two engines do not always take every one asked for, and
    // a hit after the shorter watch ended is not one the other engine was watched for
    let mut out = format!("HITS:p{idx} {} hit(s) in {} samples\n", landed.len(), samples.len());
    let mut total = 0.0;
    for (n, &i) in landed.iter().enumerate() {
        let dmg = samples[i].damage - samples[i - 1].damage;
        total += dmg;
        // The launch is the BIGGEST single-sample move shortly after the hit. It cannot be read
        // off the hit's own frame (hitstop freezes the victim there) nor off the first movement
        // (during hitstop the victim still drifts, and that drift is not the hit). Once the
        // launch does land it decays every frame after, so the largest move in a short window IS
        // its first frame. The window is short so a long fall can't out-grow a real launch, and
        // it stops at the next hit so one hit's launch is never read off another's.
        // (and a sample short of the next hit: an engine can set that hit's launch a sample before
        // its damage shows, and a hit that launches nothing then read the next one's)
        let stop = landed.get(n + 1).map(|&j| j.saturating_sub(1).max(i + 1))
            .unwrap_or(samples.len()).min(i + LAUNCH_WINDOW);
        let (at, (mag, deg)) = (i.max(1)..stop)
            .map(|k| (k, samples[k].moved_from(&samples[k - 1])))
            .max_by(|a, b| a.1 .0.total_cmp(&b.1 .0))
            .unwrap_or((i, (0.0, 0.0)));
        // THE KNOCKBACK, as the engine calculated it: the first velocity it states once the
        // hitstop freeze lets go. Nothing has acted on it yet at that point, so it is the hit's
        // own impulse rather than what survived friction and gravity.
        // The hit's impulse is read ON THE FRAME THE DAMAGE LANDS. Both engines set the launch
        // there and then hold it through the hitstop freeze, so that one frame is the knockback
        // the hit calculated, before any decay, gravity or DI has touched it.
        //
        // Nothing later will do: a victim landing a few frames on stops dead, and stopping dead
        // is a bigger velocity change than a light hit ever is. If an engine turns up that
        // applies its launch after the freeze instead, the first frame that moves is taken.
        // (a throw's let-go clears the launch on that frame and the runtime puts it back on the
        // next: a hit frame that states next to nothing is passed over for the next few, else
        // zelda's forward throw read a launch of 0)
        let at_hit = samples[i].impulse_from(&samples[i - 1]).map(|v| (i, v))
            .filter(|(_, (m, ..))| *m > LAUNCH_FLOOR);
        // (a few samples on at most: further on, a later hit's launch is what moves it)
        let engine = at_hit.or_else(|| (i + 1..stop.min(i + 3))
            .filter_map(|k| samples[k].impulse_from(&samples[k - 1]).map(|v| (k, v)))
            .find(|(_, (m, ..))| *m > LAUNCH_FLOOR));
        out.push_str(&format!(
            "  +{dmg:.1} damage at sample {i} -> launch {mag:.2}/sample at {deg:.0} deg \
             (sample {at}), state {}\n",
            samples[i].state));
        if let Some((k, (vmag, vdeg, dx, dy))) = engine {
            out.push_str(&format!(
                "     engine knockback {vmag:.3}/frame at {vdeg:.0} deg  (dvx {dx:.3}, dvy {dy:.3}, \
                 sample {k})\n"));
        }
    }
    out.push_str(&format!("  total {total:.1} damage, ending at {last:.1}"));
    out
}

/// [`watch_hits_with`] over a target's own sampler, so it reads the same on either engine.
pub fn watch_hits(
    target: &mut dyn DebugTarget, idx: usize, frames: u32, raw: bool, masks: &[u32],
) -> Result<String> {
    // An engine that can record the window gives every frame of it; polling gives whatever the
    // round trips happened to land on. Same report either way.
    if let Some(trace) = target.hit_trace(idx, frames, masks)? {
        let mut it = trace.into_iter();
        return Ok(watch_hits_with(idx, frames, raw, |_| it.next(), &[]));
    }
    Ok(watch_hits_with(idx, frames, raw, |m| {
        if let Some(mask) = m { let _ = target.hold(mask); }
        target.char_hit(idx).ok().flatten()
    }, masks))
}

/// Why [`await_animation`] stopped watching.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AnimOutcome {
    /// Played to its last frame.
    Completed,
    /// The clock wrapped back — a looping animation, which never "finishes".
    Looped,
    /// The engine moved on to a different animation.
    Changed,
    /// The clock stopped advancing without reaching the end (a paused or dead handler).
    Stalled,
    /// Ran out of budget while still advancing.
    BudgetExhausted,
    /// No character / no readable clock.
    NoSample,
}

/// The result of watching one animation play: how it ended, the last sample taken, and
/// how many distinct frames were actually observed advancing.
#[derive(Clone, Debug)]
pub struct AnimWatch {
    pub outcome: AnimOutcome,
    pub last: Option<AnimState>,
    pub frames_seen: u32,
}

/// Watch character `idx`'s animation until it finishes, loops, changes, or stalls —
/// polling the engine's own frame counter, never a wall-clock delay.
///
/// `budget_frames` bounds the watch so a looping or wedged animation can't hang the
/// caller; it is a CEILING, not a pace. `stall_polls` is how many consecutive samples
/// may show the same frame before we call it stalled — the engine is genuinely
/// between frames sometimes, so one repeat is not evidence of anything.
///
/// Goes through `char_anim`, so it works on Fraymakers and SSF2 without a branch.
pub fn await_animation(
    target: &mut dyn DebugTarget,
    idx: usize,
    budget_frames: u32,
    stall_polls: u32,
) -> Result<AnimWatch> {
    watch_animation(|| target.char_anim(idx), budget_frames, stall_polls)
}

/// Catch the FIRST frame the driven fighter reaches animation `label`, in one round trip: arm
/// the per-frame recorder, let the engine run `budget` frames on its own clock, then scan the
/// window host-side. A label that never comes is reported at the end of the budget rather than
/// waited on -- the frame-precise counterpart to `await_animation`, which plays a move OUT. It
/// reads the same `record`/`frame_trace` telemetry the DebugTarget seam gives both engines, so
/// it behaves the same on Fraymakers and SSF2.
pub fn catch_frame(target: &mut dyn DebugTarget, label: &str, budget: u32) -> Result<String> {
    target.record(true)?;                        // clear the window and arm it
    crate::session::frame_wait(budget as u64, Duration::from_millis(500 + 200 * budget as u64));
    let raw = target.frame_trace(true, false)?;         // one line per frame, read back in one round trip
    for line in raw.lines() {
        // each record line is `  <i>\t<label>|<LABEL>|<x>|<y>|<frame>`; the header has no tab-index
        let Some((idx, rec)) = line.split_once('\t') else { continue };
        if idx.trim().parse::<u32>().is_err() { continue; }
        let lab = rec.split('|').next().unwrap_or("").trim();
        if lab.eq_ignore_ascii_case(label) {
            return Ok(format!("CATCH:{label} at frame {} ({})", idx.trim(), rec.trim()));
        }
    }
    Ok(format!("CATCH:{label} not seen in {budget} frames"))
}

/// Poll until a character is actually readable, or `tries` samples have come back empty.
///
/// A match doesn't exist the instant `spawn` returns: the engine still has to construct
/// the characters, and how long that takes depends on what content it had to load. Every
/// driver here used to paper over that with a fixed delay or by grepping the log for a
/// hopeful line, which costs the first observation of every run — the Fraymakers scan
/// opened with `NoSample` and silently threw away its first driven state.
pub fn await_live(
    mut sample: impl FnMut() -> Result<Option<AnimState>>,
    tries: u32,
) -> Result<Option<AnimState>> {
    for _ in 0..tries.max(1) {
        if let Some(s) = sample()? { return Ok(Some(s)); }
    }
    Ok(None)
}

/// The frame-watching state machine itself, over any source of samples.
///
/// Split out from [`await_animation`] because the two live drivers reach the engine
/// differently: the `DebugTarget` path calls `char_anim` and gets the reply back
/// inline, while `bridge::serve` hands its socket reader to a printing thread and has
/// to pick replies off a channel. Only the sampling differs; the decision about when
/// an animation is done must not.
/// How long the clock may sit on the same frame before the watch calls it stalled.
/// Generous enough to cover a slow frame on either engine (SSF2 33ms, Fraymakers 16ms).
const STALL_AFTER: Duration = Duration::from_millis(400);

pub fn watch_animation(
    mut sample: impl FnMut() -> Result<Option<AnimState>>,
    budget_frames: u32,
    _stall_polls: u32,
) -> Result<AnimWatch> {
    let Some(first) = sample()? else {
        return Ok(AnimWatch { outcome: AnimOutcome::NoSample, last: None, frames_seen: 0 });
    };
    let start_anim = first.anim.clone();
    let mut prev = first;
    let mut frames_seen = 0u32;
    // "Stalled" is measured as TIME SINCE THE CLOCK LAST MOVED, not as a count of
    // identical samples. A poll count is the wrong unit: sampling faster than the engine
    // renders naturally returns the same frame several times in a row, so a fixed count
    // reports a perfectly healthy animation as stalled — and how often that happens
    // depends on the sampler's speed, so the same code silently behaved differently on
    // the two engines (Fraymakers polls at roughly one sample per frame and squeaked by;
    // SSF2's evals are sub-millisecond against a 33ms frame, so it tripped instantly and
    // every SSF2 watch came back Stalled with frames_seen=0).
    let mut last_advance = Instant::now();
    let mut polls = 0u32;
    // Backstop only. A fast sampler takes many polls per rendered frame, so this can't be
    // budget*2 any more — it has to be loose enough not to cut a healthy animation short.
    let max_polls = budget_frames.max(1) * 200;
    loop {
        polls += 1;
        if polls > max_polls {
            return Ok(AnimWatch { outcome: AnimOutcome::BudgetExhausted, last: Some(prev), frames_seen });
        }
        let Some(cur) = sample()? else {
            return Ok(AnimWatch { outcome: AnimOutcome::NoSample, last: Some(prev), frames_seen });
        };
        if cur.anim != start_anim {
            return Ok(AnimWatch { outcome: AnimOutcome::Changed, last: Some(cur), frames_seen });
        }
        if cur.frame < prev.frame {
            return Ok(AnimWatch { outcome: AnimOutcome::Looped, last: Some(cur), frames_seen });
        }
        if cur.frame > prev.frame {
            frames_seen += cur.frame.saturating_sub(prev.frame).unsigned_abs();
            last_advance = Instant::now();
        } else if last_advance.elapsed() >= STALL_AFTER {
            let outcome = if cur.at_end() { AnimOutcome::Completed } else { AnimOutcome::Stalled };
            return Ok(AnimWatch { outcome, last: Some(cur), frames_seen });
        }
        if cur.at_end() {
            return Ok(AnimWatch { outcome: AnimOutcome::Completed, last: Some(cur), frames_seen });
        }
        prev = cur;
    }
}

/// Frames the Fraymakers engine has PUSHED since the last `record`.
///
/// The exact counterpart of `ssf2_bridge::FRAMES`. Both engines now emit `FRAME:` per frame
/// and the host buffers it, so `trace` reads the same shape from either one and a timing
/// comparison isn't comparing two different measurements.
static FM_FRAMES: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Route one engine-pushed frame line into the buffer (bounded, so a session left
/// recording can't grow without limit: 20k frames is ~5.5 minutes at 60fps).
pub fn push_frame(state: &str) {
    let mut f = FM_FRAMES.lock().unwrap();
    if f.len() < 20_000 { f.push(state.to_string()); }
}
pub fn fm_frames_clear() { FM_FRAMES.lock().unwrap().clear(); }
pub fn fm_frames_take() -> Vec<String> { FM_FRAMES.lock().unwrap().clone() }

/// The ONE rendering of each host-computed result, shared by every driver and both
/// engines. These strings previously existed in three files (bridge.rs's serve loop, the
/// Fraymakers target, the SSF2 target) and could drift apart — which is exactly how `tree`
/// ended up answering "SSF2-only" through one path and working through another.
/// One object on a stage, as BOTH engines describe it.
///
/// The stage comparison only works if the two engines report the same facts in the same
/// shape. They don't produce them the same way — SSF2 walks a display list and formats
/// host-side, Fraymakers builds its rows in injected bytecode — so this is the contract that
/// keeps them diffable: whatever a target can measure goes in these fields, and anything it
/// genuinely cannot measure is `None` rather than a filler value that would read as data.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StageNode {
    /// Nesting depth, so a tree and a flat layer listing print alike.
    pub depth: usize,
    /// Runtime class. The engines' names differ; the SHAPE of the row is what's shared.
    pub class: String,
    /// Instance name, or `None` when the object is anonymous (common in Fraymakers, where
    /// reparented vfx sprites are wrapped in unnamed containers).
    pub name: Option<String>,
    pub x: f64,
    pub y: f64,
    /// Pixel size, when the engine exposes it (SSF2 does; Fraymakers doesn't per-object).
    pub size: Option<(f64, f64)>,
    pub visible: bool,
    /// Opacity, when the engine states one: a clip placed at none is a placeholder.
    pub alpha: Option<f64>,
    /// Animation clock: current frame and length. This is the "does it animate / does it
    /// loop" axis, and it's the field most worth having on both sides.
    pub frame: Option<(i64, i64)>,
    /// Current animation label, when the engine names one.
    pub label: Option<String>,
    /// Which named stage layer the object sits in, when known.
    pub layer: Option<String>,
    /// Where the object ACTUALLY renders, in absolute space.
    ///
    /// A display child's own x/y is local to its container, so it can read (0,0) while the
    /// thing is drawn anywhere at all — that is exactly how a placement bug hid here once.
    /// The absolute pair is the figure comparable to an SSF2 world position.
    pub abs: Option<(f64, f64)>,
}

/// Render a `StageNode` the SAME way for either engine, so two logs diff field by field.
///
/// Absent facts are OMITTED rather than filled: a missing frame clock must not be
/// indistinguishable from `frame=0/0`, or a comparison would silently read "no animation"
/// where the truth is "not measured here".
pub fn fmt_stage_node(n: &StageNode) -> String {
    let mut out = format!("{}{} ", "  ".repeat(n.depth), n.class);
    out.push_str(&match &n.name {
        Some(nm) => format!("{nm:?} "),
        None => String::new(),
    });
    out.push_str(&format!("@({:.1},{:.1})", n.x, n.y));
    if let Some((w, h)) = n.size {
        out.push_str(&format!(" {w:.0}x{h:.0}"));
    }
    if !n.visible {
        out.push_str(" HIDDEN");
    }
    if let Some(a) = n.alpha.filter(|a| *a < 0.999) {
        out.push_str(&format!(" alpha={a:.2}"));
    }
    if let Some((f, t)) = n.frame {
        out.push_str(&format!(" frame={f}/{t}"));
    }
    if let Some(l) = &n.label {
        out.push_str(&format!(" '{l}'"));
    }
    if let Some(l) = &n.layer {
        out.push_str(&format!(" layer={l}"));
    }
    if let Some((ax, ay)) = n.abs {
        out.push_str(&format!(" abs=({ax:.1},{ay:.1})"));
    }
    out
}

pub fn fmt_record(on: bool) -> String {
    if on { "record: window opened (engine pushes FRAME: telemetry)".into() }
    else { "record: the recorder always pushes — `trace` reads the window".into() }
}

/// `TRACE:` header plus one `<animation> xN` row per run. `ends[i]`, when present, is the
/// position the run finished at (SSF2 carries it in the stream; Fraymakers doesn't).
/// One line per recorded frame, verbatim. The RLE summary says how many frames a move lasted;
/// this says where the entity was on each of them, which is the only way to derive a speed
/// multiplier from observation rather than assuming one.
pub fn fmt_trace_raw(frames: &[String]) -> String {
    let mut out = format!("TRACERAW:{} frames\n", frames.len());
    for (i, f) in frames.iter().enumerate() { out.push_str(&format!("  {i}\t{f}\n")); }
    out
}

pub fn fmt_trace(total: usize, runs: &[(String, usize)], ends: &[Option<String>]) -> String {
    let mut out = format!("TRACE:{total} frames, {} animations\n", runs.len());
    for (i, (label, n)) in runs.iter().enumerate() {
        let end = ends.get(i).and_then(|e| e.as_deref()).unwrap_or("");
        out.push_str(&format!("  {label} x{n}{end}\n"));
    }
    out
}

pub fn fmt_await(w: &AnimWatch, idx: usize) -> String {
    match &w.last {
        Some(s) => format!(
            "AWAIT:{:?} anim={} frame={}/{} frames_seen={} pos=({:.1},{:.1})",
            w.outcome, s.anim, s.frame, s.total, w.frames_seen, s.x, s.y),
        None => format!("AWAIT:{:?} (no live character p{idx})", w.outcome),
    }
}

pub fn fmt_ready(live: Option<&AnimState>, idx: usize, tries: u32) -> String {
    match live {
        Some(s) => format!("MATCHREADY:p{idx} anim={} pos=({:.1},{:.1})", s.anim, s.x, s.y),
        None => format!("MATCHREADY:none (p{idx} never became readable in {tries} tries)"),
    }
}

/// Run-length-encode a per-frame stream into `"<anim> xN"` records — the comparable unit
/// across engines. Shared so Fraymakers and SSF2 can't report it differently.
pub fn rle_frames(frames: &[String]) -> Vec<(String, usize)> {
    let mut runs: Vec<(String, usize)> = Vec::new();
    for f in frames {
        let label = f.split('|').next().unwrap_or(f).trim().to_string();
        if label.is_empty() { continue; }
        match runs.last_mut() {
            Some((l, n)) if *l == label => *n += 1,
            _ => runs.push((label, 1)),
        }
    }
    runs
}

/// Drop a leading `E:` (the Fraymakers eval-reply wrapper) so feed payloads are
/// uniform regardless of which backend produced them.
fn strip_eval(s: String) -> String {
    s.strip_prefix("E:").map(str::to_string).unwrap_or(s)
}
fn non_empty(s: String) -> Option<String> {
    if s.trim().is_empty() { None } else { Some(s) }
}

/// Parse `line` and execute it on `target`. Returns `Some(reply)` to show the
/// user, or `None` for a no-op (empty line). The single front door shared by the
/// Fraymakers and SSF2 session loops — identical syntax, identical routing.
pub fn run_command(target: &mut dyn DebugTarget, line: &str) -> Result<Option<String>> {
    Ok(match interpreter::parse(line) {
        Command::Help => Some(interpreter::help_text()),
        Command::Client(s) => if s.trim().is_empty() { None } else { Some(s) },
        Command::Spawn(a) => Some(target.spawn(&a)?),
        Command::Eval(e) => Some(target.eval(&e)?),
        Command::EvalIn { target: t, expr } => Some(target.eval_in(&t, &expr)?),
        Command::Var { idx, name, value } => Some(format!("GVAR:p{idx} {name} = {}", strip_eval(target.var(idx, &name, value)?).trim())),
        Command::Hold(m) => Some(target.hold(m)?),
        Command::Speed(n) => Some(target.speed(n)?),
        Command::Seq(s) => Some(target.seq(&s)?),
        // Scenario = set up the scene (eval), then play p0's input timeline (seq).
        // Same DebugTarget seam, so it works identically on Fraymakers and SSF2.
        Command::Scenario { setup, masks } => {
            let mut out = target.eval(&setup)?;
            if !masks.is_empty() {
                let s = target.seq(&masks)?;
                if !s.trim().is_empty() {
                    if !out.trim().is_empty() { out.push('\n'); }
                    out.push_str(&s);
                }
            }
            Some(out)
        }
        Command::Await { idx, budget } => {
            let w = await_animation(target, idx, budget, 3)?;
            Some(fmt_await(&w, idx))
        }
        Command::Catch { label, budget } => Some(catch_frame(target, &label, budget)?),
        Command::Hits { idx, frames, raw, masks } => Some(watch_hits(target, idx, frames, raw, &masks)?),
        Command::AwaitMatch { idx, tries } => {
            let live = await_live(|| target.char_anim(idx), tries)?;
            Some(fmt_ready(live.as_ref(), idx, tries))
        }
        Command::Move { idx, name } => Some(target.play_move(idx, &name)?),
        Command::Info => {
            // Enumerate the players that exist rather than assuming two. `char_anim`
            // already reports animation/position/state and answers None for an absent
            // slot, so this is correct for any roster size on either engine.
            let mut out = String::new();
            for idx in 0..4 {
                let Some(s) = target.char_anim(idx)? else { continue };
                let dmg = strip_eval(target.eval(&format!("p{idx}.damage._damage"))
                    .unwrap_or_default());
                let dmg = dmg.trim();
                // The per-frame sampler drops `state` to keep a poll cheap; `info` is a
                // one-shot readout, so it can afford the extra read rather than print "?".
                let state = if s.state == "?" || s.state.is_empty() {
                    let v = strip_eval(target.eval(&format!("p{idx}.getStateName()"))
                        .unwrap_or_default());
                    let v = v.trim().to_string();
                    if v.is_empty() { "?".into() } else { v }
                } else { s.state.clone() };
                out.push_str(&format!(
                    "p{idx}: anim={} frame={}/{} pos=({:.1},{:.1}) state={} damage={}\n",
                    s.anim, s.frame, s.total, s.x, s.y, state,
                    if dmg.is_empty() { "?" } else { dmg }));
            }
            Some(if out.is_empty() { "info: no live characters".into() } else { out })
        }
        Command::Tick { n } => Some(crate::session::frame_wait_reply(n)),
        Command::Clock => Some(format!("CLOCK:{} frames", crate::session::frames_seen())),
        Command::Record { on } => Some(target.record(on)?),
        Command::Trace { raw, all } => Some(target.frame_trace(raw, all)?),
        Command::Console => Some(target.console()?),
        Command::Tree(depth) => Some(target.tree(depth)?),
        Command::Camera { ref mode, pos, zoom } => Some(target.camera(mode.as_deref(), pos, zoom)?),
        Command::Shot(ref p) => Some(target.shot(p)?),
        Command::AddCharacter => Some(target.add_character()?),
        Command::Exit => { target.exit()?; Some("exit".into()) }
        Command::Load => Some(target.load()?),
    })
}

// ─────────────────────────── Fraymakers backend ───────────────────────────

/// Drives the live Fraymakers engine over its loopback TCP socket. Each
/// `Command` is encoded to the wire via `interpreter::command_to_wire` (so the
/// protocol stays in one place) and the reply is drained synchronously.
pub struct FraymakersTarget {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
}

impl FraymakersTarget {
    pub fn new(reader: BufReader<TcpStream>, writer: TcpStream) -> Self {
        // Short read timeout so draining replies returns once the engine goes quiet.
        let _ = reader.get_ref().set_read_timeout(Some(Duration::from_millis(120)));
        FraymakersTarget { reader, writer }
    }

    /// Send a wire line (which may itself be multi-line for `seq`) and drain the
    /// engine's reply until it goes quiet (or a hard cap elapses).
    fn send_wire(&mut self, wire: &str) -> Result<String> {
        if !wire.is_empty() {
            let line = if wire.ends_with('\n') { wire.to_string() } else { format!("{wire}\n") };
            self.writer.write_all(line.as_bytes())?;
            self.writer.flush()?;
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut out = String::new();
        loop {
            let mut buf = String::new();
            match self.reader.read_line(&mut buf) {
                Ok(0) => break, // EOF
                Ok(_) => {
                    let t = buf.trim();
                    if !t.is_empty() && t != "READY" {
                        if let Some(g) = interpreter::gloss(t) { out.push_str(&g); }
                        else { out.push_str(t); }
                        out.push('\n');
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut => {
                    if !out.is_empty() { break; } // quiet after some output
                    if Instant::now() >= deadline { break; }
                }
                Err(_) => break,
            }
            if Instant::now() >= deadline { break; }
        }
        Ok(out.trim_end().to_string())
    }

    fn run(&mut self, cmd: &Command) -> Result<String> {
        match interpreter::command_to_wire(cmd) {
            interpreter::Translated::Wire(w) => self.send_wire(&w),
            interpreter::Translated::Client(c) => Ok(c.trim_end().to_string()),
        }
    }
}

/// Render the engine's raw `tree` reply, turning its `N\t…` field rows into the SHARED
/// stage-node format.
///
/// The injected walk emits fields rather than prose precisely so this formatting lives in one
/// place for both engines. Non-node lines (group headers, entity rows) pass through untouched.
pub fn fmt_fm_tree(raw: &str) -> String {
    let mut out = String::new();
    for line in raw.lines() {
        let Some(fields) = line.trim_start().strip_prefix("N\t") else {
            out.push_str(line);
            out.push('\n');
            continue;
        };
        let f: Vec<&str> = fields.split('\t').collect();
        let get = |i: usize| f.get(i).copied().unwrap_or("");
        let num = |v: &str| v.parse::<f64>().unwrap_or(0.0);
        let name = get(1);
        let node = StageNode {
            depth: 3,
            class: get(0).to_string(),
            name: (!name.is_empty()).then(|| name.to_string()),
            x: num(get(2)),
            y: num(get(3)),
            // pixel size, when the object has a dimensions component (fields 9/10)
            size: (!get(8).is_empty() && get(8) != "null")
                .then(|| (num(get(8)), num(get(9)))),
            visible: get(4) != "0",
            // The animation clock, when the object has one. Absent fields stay None: a
            // display child genuinely has no clock, and that must not read as frame 0.
            frame: (!get(5).is_empty() && get(5) != "null")
                .then(|| (num(get(5)) as i64, num(get(6)) as i64)),
            label: (!get(7).is_empty() && get(7) != "null").then(|| get(7).to_string()),
            layer: (!get(10).is_empty() && get(10) != "null").then(|| get(10).to_string()),
            abs: (!get(11).is_empty() && get(11) != "null")
                .then(|| (num(get(11)), num(get(12)))),
            alpha: None,
        };
        out.push_str(&fmt_stage_node(&node));
        out.push('\n');
    }
    out
}

/// the console name an entity target answers to: `match` for the stage, `p<slot>` for a fighter
pub fn self_root(target: &str) -> String {
    if target == "s" { "match".to_string() } else { format!("p{target}") }
}

/// the script with every whole-word `self` replaced by the entity's console root
pub fn bind_self(target: &str, expr: &str) -> String {
    let root = self_root(target);
    let mut bound = String::with_capacity(expr.len() + 8);
    let bytes = expr.as_bytes();
    let mut i = 0;
    while i < expr.len() {
        let word_start = i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_');
        let word_end = i + 4 >= expr.len() || !(bytes[i + 4].is_ascii_alphanumeric() || bytes[i + 4] == b'_');
        if word_start && expr[i..].starts_with("self") && word_end {
            bound.push_str(&root);
            i += 4;
        } else {
            let ch = expr[i..].chars().next().unwrap_or(' ');
            bound.push(ch);
            i += ch.len_utf8();
        }
    }
    bound
}

impl DebugTarget for FraymakersTarget {

    /// The port keeps a fighter's named values on its runtime object, reached inside the
    /// fighter's own script.
    fn var(&mut self, idx: usize, name: &str, value: Option<f64>) -> Result<String> {
        self.eval_in(&idx.to_string(), &fm_var_expr(name, value))
    }

    /// Open a recording window: drop whatever the engine has pushed so far.
    /// Symmetric with `Ssf2Target::record` — both engines push per-frame telemetry and the
    /// host decides what counts as a window.
    fn record(&mut self, on: bool) -> Result<String> {
        if on { fm_frames_clear(); }
        Ok(fmt_record(on))
    }

    /// Close the window: run-length-encode every frame the engine pushed since `record`.
    /// Identical output shape to the SSF2 side, via the shared `rle_frames`, so a timing
    /// comparison is reading the same measurement from both engines.
    fn frame_trace(&mut self, raw: bool, _all: bool) -> Result<String> {
        let frames = fm_frames_take();
        if raw { return Ok(fmt_trace_raw(&frames)); }
        let runs = rle_frames(&frames);
        Ok(fmt_trace(frames.len(), &runs, &[]))
    }

    fn eval(&mut self, expr: &str) -> Result<String> { self.run(&Command::Eval(expr.to_string())) }
    /// this engine keeps an interpreter per hscript script, so the script really runs in the
    /// entity's own. a built-in fighter's script is compiled into the engine and has no
    /// interpreter to enter (and no script locals to reach), so it gets what the other engine
    /// gets: the console's eval with `self` bound to that entity, every api call landing on the
    /// real fighter. the reply says which it was.
    fn eval_in(&mut self, target: &str, expr: &str) -> Result<String> {
        let reply = self.run(&Command::EvalIn { target: target.to_string(), expr: expr.to_string() })?;
        if !reply.contains("script is compiled") {
            return Ok(reply);
        }
        let bound = bind_self(target, expr);
        let via = self.eval(&bound)?;
        Ok(format!("{via}  (compiled script: run on the console with self = {})", self_root(target)))
    }
    fn spawn(&mut self, a: &SpawnArgs) -> Result<String> { self.run(&Command::Spawn(a.clone())) }
    fn hold(&mut self, mask: u32) -> Result<String> { self.run(&Command::Hold(mask)) }
    fn speed(&mut self, n: u32) -> Result<String> { self.run(&Command::Speed(n)) }
    fn seq(&mut self, masks: &[u32]) -> Result<String> { self.run(&Command::Seq(masks.to_vec())) }
    fn console(&mut self) -> Result<String> { self.run(&Command::Console) }
    fn add_character(&mut self) -> Result<String> { self.run(&Command::AddCharacter) }
    fn exit(&mut self) -> Result<()> { let _ = self.run(&Command::Exit)?; Ok(()) }
    fn load(&mut self) -> Result<String> { self.run(&Command::Load) }
    // Fraymakers DOES walk its live object tree (the `w` verb). Without this it inherited
    // the trait default, which declares the feature SSF2-only — so `tree` answered
    // "not supported" through the session/GUI path (run_command -> target.tree) while the
    // same command worked through the serve path (command_to_wire -> "w"). Two drivers,
    // opposite answers, same command.
    // Rendered through the SHARED stage-node formatter: the injected walk emits fields, not
    // prose, so both engines' rows are produced by one piece of code and stay diffable.
    fn tree(&mut self, depth: u32) -> Result<String> {
        let raw = self.run(&Command::Tree(depth))?;
        Ok(fmt_fm_tree(&raw))
    }

    fn camera(&mut self, mode: Option<&str>, pos: Option<(f64, f64)>, zoom: Option<f64>) -> Result<String> {
        let out = self.run(&Command::Camera { mode: mode.map(str::to_string), pos, zoom })?;
        Ok(strip_eval(out).trim().to_string())
    }

    // The engine encodes the PNG and hands it back as hex on the SAME socket the commands ride
    // (the stock-icon path's trick). No engine-side file write, and nothing depends on the host
    // being able to see the window — which, headless, it cannot.
    fn shot(&mut self, path: &std::path::Path) -> Result<String> {
        let raw = self.run(&Command::Shot(path.to_path_buf()))?;
        let hex: String = raw.lines()
            .find_map(|l| l.trim().strip_prefix("SHOT:").map(str::to_string))
            .ok_or_else(|| anyhow::anyhow!("engine returned no frame: {}", raw.trim()))?;
        set_shot_path(path);
        Ok(write_shot(&hex))
    }
}

/// The script that reads or sets a fighter's named value on Fraymakers, run inside the
/// fighter's own script (`e@pN`): the runtime keeps them on the object its code works with.
pub fn fm_var_expr(name: &str, value: Option<f64>) -> String {
    match value {
        Some(v) => format!("(function(){{var o=Objects.view(self);o.__vars.set(\"{name}\",{v});return o.__vars.get(\"{name}\");}})()"),
        None => format!("Objects.view(self).__vars.get(\"{name}\")"),
    }
}

#[cfg(test)]
mod hit_tests {
    use super::*;

    fn s(damage: f64, x: f64, y: f64) -> HitState {
        HitState { damage, x, y, state: "HURT".into(), vel: None }
    }

    #[test]
    fn parses_the_shared_wire_shape() {
        let h = HitState::parse("E:9|-83.5|73|HURT_MEDIUM").expect("parses");
        assert_eq!((h.damage, h.x, h.y), (9.0, -83.5, 73.0));
        assert_eq!(h.state, "HURT_MEDIUM");
        assert!(HitState::parse("").is_none());
    }

    #[test]
    fn reads_the_launch_past_the_hitstop() {
        // the shape a real hit has: drift, damage steps up, the victim is FROZEN for the
        // hitstop, then the launch lands and decays. the launch is the 14.0, not the drift and
        // not the zero on the hit's own frame.
        let mut frames = vec![s(0.0, 0.0, 0.0), s(0.0, 0.5, 0.0)];
        frames.push(s(9.0, 1.0, 0.0)); // the hit
        for i in 0..4 { frames.push(s(9.0, 1.0 + f64::from(i) * 0.0, 0.0)); } // hitstop
        frames.push(s(9.0, 15.0, 0.0)); // launch
        frames.push(s(9.0, 28.0, 0.0));
        let mut it = frames.into_iter();
        let out = watch_hits_with(1, 20, false, |_| it.next(), &[]);
        assert!(out.contains("1 hit(s)"), "{out}");
        assert!(out.contains("+9.0 damage"), "{out}");
        assert!(out.contains("launch 14.00/sample at 0 deg"), "{out}");
    }

    #[test]
    fn a_damage_drop_is_not_a_hit() {
        // a reset or a fresh stock takes damage DOWN, which is not a hit landing
        let mut it = vec![s(40.0, 0.0, 0.0), s(0.0, 0.0, 0.0), s(0.0, 0.0, 0.0)].into_iter();
        let out = watch_hits_with(0, 10, false, |_| it.next(), &[]);
        assert!(out.contains("none in 3 samples"), "{out}");
    }
}
