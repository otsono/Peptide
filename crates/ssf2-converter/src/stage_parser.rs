//! stage_parser — extract a geometry model from an SSF2 stage `.ssf`.
//!
//! SSF2 stages are SWF resources: a root `stageMC` MovieClip holds a `terrain`
//! container (the collision geometry), named boundary clips (`deathBoundary`,
//! `camBoundary`, `smashBallBoundary`), and per-player `pN_Start` / `pN_Spawn`
//! beacon instances. We walk the placement tree accumulating the FULL affine
//! matrix down each branch, then read each instance's WORLD axis-aligned bounding
//! box (the referenced shape's local bounds transformed by the compounded matrix).
//!
//! Output is in **Fraymakers stage coordinates**: SSF2 world coords minus the
//! `stageMC` origin (FrayTools and SSF2 share a y-down pixel space at 1:1 scale —
//! the battlefield floor is ~520px wide in both). The FM stage emitter consumes
//! this model directly. Read-only on the input file.
//!
//! Reuses [`crate::ssf::decompress`] (the DAT-archive aware path; the raw `swf`
//! decompressor fails on the install's DATs).

use anyhow::{bail, Context, Result};
use std::collections::BTreeMap;
use std::path::Path;

/// A 2D affine matrix (SWF convention: x' = a·x + c·y + tx, y' = b·x + d·y + ty).
#[derive(Clone, Copy, Debug)]
struct Mat { a: f64, b: f64, c: f64, d: f64, tx: f64, ty: f64 }

/// The document root, seen at the fighters' scale. A fighter lives in the stage clip's own space,
/// so where the document places that clip SCALED (the target tests at 0.942) everything the root
/// draws is that much smaller than the fighters see it; every walk from the root starts with the
/// scale divided out, so art and collision come out in the fighters' units like the rest.
fn root_mat(tags: &[swf::Tag], sym_names: &BTreeMap<u16, String>) -> Mat {
    for t in tags {
        let swf::Tag::PlaceObject(po) = t else { continue };
        let id = match &po.action {
            swf::PlaceObjectAction::Place(i) | swf::PlaceObjectAction::Replace(i) => *i,
            swf::PlaceObjectAction::Modify => continue,
        };
        let named = po.name.as_ref().map(|n| n.to_str_lossy(encoding_rs::WINDOWS_1252)).as_deref() == Some("stageMC");
        let sym = sym_names.get(&id).map(|n| n.rsplit('.').next().unwrap_or(n).to_ascii_lowercase()).unwrap_or_default();
        if !named && !sym.starts_with("stage_") { continue }
        let Some(m) = po.matrix.as_ref() else { return Mat::id() };
        let (a, b, c, d) = (m.a.to_f64(), m.b.to_f64(), m.c.to_f64(), m.d.to_f64());
        let k = ((a * a + b * b).sqrt() + (c * c + d * d).sqrt()) / 2.0;
        if k <= 0.0 || (k - 1.0).abs() < 1e-3 { return Mat::id() }
        return Mat { a: 1.0 / k, b: 0.0, c: 0.0, d: 1.0 / k, tx: 0.0, ty: 0.0 };
    }
    Mat::id()
}
impl Mat {
    fn id() -> Mat { Mat { a: 1.0, b: 0.0, c: 0.0, d: 1.0, tx: 0.0, ty: 0.0 } }
    /// `self ∘ other` — apply `other` first, then `self`.
    fn mul(&self, o: &Mat) -> Mat {
        Mat {
            a: self.a * o.a + self.c * o.b,
            b: self.b * o.a + self.d * o.b,
            c: self.a * o.c + self.c * o.d,
            d: self.b * o.c + self.d * o.d,
            tx: self.a * o.tx + self.c * o.ty + self.tx,
            ty: self.b * o.tx + self.d * o.ty + self.ty,
        }
    }
    fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        (self.a * x + self.c * y + self.tx, self.b * x + self.d * y + self.ty)
    }
    /// `+1.0` if this matrix preserves x-orientation, `-1.0` if mirrored (scaleX < 0).
    fn x_sign(&self) -> f64 { if self.a < 0.0 { -1.0 } else { 1.0 } }
    /// This placement in the form the emitter draws from -- the SAME conversion the character art
    /// path uses, so a shape on a backdrop is placed like a shape on a character.
    fn placement(&self) -> crate::fm_placement::WorldPlacement {
        crate::fm_placement::WorldPlacement::from_affine(self.a, self.b, self.c, self.d, self.tx, self.ty)
    }
    /// `(flip_x, flip_y)` for an AXIS-ALIGNED placement (a mirrored decorative element). A
    /// negative scale on an axis means the art is drawn reversed there; the raster path composites
    /// onto an axis-aligned AABB and so must mirror the tile to match. Rotated/skewed matrices (b/c
    /// nonzero) report `(false, false)` — a resize+blit can't represent a true affine warp, and
    /// background decals are effectively never rotated.
    fn flips(&self) -> (bool, bool) {
        let axis_aligned = self.b.abs() < 1e-3 && self.c.abs() < 1e-3;
        (axis_aligned && self.a < 0.0, axis_aligned && self.d < 0.0)
    }
}

/// An axis-aligned box in FM stage coordinates (x/y = top-left, w/h = size).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Rect { pub x: f64, pub y: f64, pub w: f64, pub h: f64 }
impl Rect {
    pub fn left(&self) -> f64 { self.x }
    pub fn top(&self) -> f64 { self.y }
    pub fn right(&self) -> f64 { self.x + self.w }
    pub fn bottom(&self) -> f64 { self.y + self.h }
}

/// A collision platform (the main floor or a soft platform).
// Default so a stage can be built from scratch in the emitter tests (see StageModel).
#[derive(Clone, Debug, Default)]
pub struct Platform {
    /// World AABB in FM coords.
    pub rect: Rect,
    /// `true` for a one-way (drop-through) soft platform; `false` for solid terrain.
    pub drop_through: bool,
    /// The sideways push the piece declares (`x_influence`), in the source's px a frame: the game
    /// moves a fighter standing on it by that much (desk's lamp slopes slide a fighter down).
    pub x_influence: Option<f64>,
    /// The walkable TOP surface as a polyline in FM coords (left to right), when the terrain
    /// is curved/sloped. `None` for a flat platform (use `rect.top()` as a level line).
    pub profile: Option<Vec<(f64, f64)>>,
    /// `true` if this platform needs a drawn grey surface (a hand-declared platform whose visual
    /// isn't in the stage art, e.g. bowserscastle's columns over lava). Parsed terrain platforms
    /// already have their art in the background, so they stay invisible collision.
    pub visible: bool,
    /// `true` for a standable floor that must not anchor the stage (a molten lake the fighter
    /// lands ON while its hazard hitbox burns them). Excluded from main-floor selection and
    /// engine-layer alignment; still real collision.
    /// The name the stage's own code addresses this platform by, when it addresses one. Butter
    /// building keeps a table of its layouts and reaches each platform by name to make it solid or
    /// let fighters through, so a named platform is emitted as an object rather than baked into the
    /// stage's own collision.
    pub name: Option<String>,
    /// The solid's own outline, every edge typed by which side of it is solid (`FLOOR`,
    /// `CEILING`, `LEFT_WALL`, `RIGHT_WALL`), in FM coords. A hollow level (corridors inside one
    /// polygon) has surfaces all through it that no top line and box can carry. Empty when the
    /// rect and profile say everything.
    pub outline: Vec<(&'static str, Vec<(f64, f64)>)>,
    /// Where the plane's timeline puts this surface at each of its labels, as an offset from
    /// its boot placement in FM px, or nothing when the label does not place it at all (mushroom
    /// kingdom 3 shows one terrain section per phase). Empty when every label places it where
    /// it started.
    pub placements: Vec<(String, Option<(f64, f64)>)>,
    /// Which ends are ledges: a ledge beacon that declares this piece's own `syncPlatform` key
    /// stands at that end (gangplank galleon's platform and its two beacons share a key).
    pub ledges: (bool, bool),
    /// The ground friction and walk acceleration the piece declares for whoever stands on it
    /// (`decel_friction`, `accel_friction`; source px a frame squared): race to the finish's ice.
    pub friction: Option<(f64, f64)>,
    /// The source declared this a platform (or named it one): a surface a fighter lands on, drawn
    /// as its planks, never a block with walls (lunar core's pit bridges boxed the whole pit shut).
    pub is_platform: bool,
}

/// A stage hazard, emitted as a Fraymakers custom game object (a damaging hitbox volume the
/// stage spawns). Position is in FM coords; `w`/`h` size the hitbox; the rest are FM
/// HitboxStats values. `interval`/`active` drive a simple on/off pulse (0 interval = always on).
#[derive(Clone, Debug)]
pub struct Hazard {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub damage: f64,
    pub knockback: f64,
    pub angle: f64,
    /// Per-hitbox launch directions from the class's `getAttackStats` (the Thwomp's two boxes =
    /// 135 left / 45 right). Empty falls back to `angle`. Data-driven, so the emitter angles each
    /// HIT_BOX by what the SSF2 source declares instead of a hardcoded pair.
    pub hitbox_dirs: Vec<f64>,
    /// Animation labels the hazard class plays via `forceAttack(...)` — the code-referenced handle
    /// to its art clip (the clip whose frame labels match these), so the art comes from what the
    /// script animates, not a library keyword match. Empty falls back to the keyword path.
    pub anim_labels: Vec<String>,
    /// The engine-spawned faller cycle (entrance delay, landed wait, spawn period, column xs),
    /// stepped from the enemy + stage classes' own code. None for non-spawned hazards.
    pub faller: Option<crate::abc_parser::FallerCycle>,
    /// Behavior values stepped out of the hazard class's update()/initialize() — drives the emitted
    /// CGO script (shake amplitude, rise/fall speeds, self-platform, dust, sounds) from the hazard's
    /// own code rather than hardcoded template constants.
    pub behavior: crate::abc_parser::EnemyBehavior,
    /// The hazard class's update()/initialize() reconstructed as FM hscript via the character
    /// decompile→translate pipeline (gated reconstruction path; needs a field-state + FrameTimer
    /// pass before it runs). None when the class has no lifecycle methods.
    pub reconstructed_script: Option<String>,
    /// The SSF2 enemy class this hazard came from (`spawnEnemy(<Class>)`), e.g.
    /// `BowsersCastleLava` — used for readable emitted content ids. None for detected-only
    /// hazards with no class reference.
    pub class_name: Option<String>,
    /// The class's `getOwnStats` maxYSpeed (the SSF2 engine's fall-speed cap) — the terminal
    /// velocity the reconstruction's kinematics integrator honors. None = no cap declared.
    pub max_y_speed: Option<f64>,
    /// The name the stage's own code addresses this object by, when the object was PLACED in the
    /// tree rather than spawned from code (`getPlatform("left_plat")`). None for a spawned object.
    pub instance_name: Option<String>,
    /// Another placement of the same class carries the same name (target test black mage's ten
    /// targets are each placed as `stance`): the name reaches the first of them, and does not tell
    /// this one's content apart.
    pub shared_name: bool,
    /// WHERE THE TIMELINE PUTS A PLACED OBJECT AT EACH OF ITS LABELS: (label, x, y) in FM
    /// coordinates. A stage that lays its objects out by playing a label on a plane (palace of
    /// twilight's `layout01`..) moves the placed clips with the timeline, and the objects the port
    /// makes for them follow the same labels. Empty for an object nothing places.
    pub placements: Vec<(String, f64, f64)>,
    /// The clip the stage hands this object to draw with (`setStance`), by the name the tree gives
    /// it. None when the object owns its own art.
    pub art_clip: Option<String>,
    /// Which shared package the class came from, when the stage package does not define it: the
    /// art it names is in that package too.
    pub external_pkg: Option<usize>,
    /// The clip the class names for itself (`getOwnStats().linkage_id`).
    pub linkage_id: Option<String>,
    /// The bounce a placed platform clip declares (speed in the source's units per frame, and the
    /// label it plays): the game's platform layer sends a landing fighter up at it.
    pub bounce: Option<(f64, String)>,
    /// Which sides of the body are ledges: a ledge beacon that declares the object's own
    /// `syncPlatform` key stands at that edge and moves with it.
    pub ledges: (bool, bool),
    /// The legs the game's own platform class runs this clip through, when it declares them.
    pub dynamic: Option<DynamicDecl>,
    /// The picture a collision clip names for itself in the foreground (`self.foreground = "x"`).
    pub fg_name: Option<String>,
    /// The clip the class names to draw IN FRONT of the fighters (`setForeground("x")`), rendered
    /// from the package the class came from. Empty when the name is the object's own art, which
    /// needs no second piece: the object itself moves forward.
    pub fg_anims: Vec<ClipAnim>,
    /// Whether the class asks to be drawn ahead of the fighters at all.
    pub draws_in_front: bool,
    /// The class says a hit does not send it flying (`canReceiveKnockback: false`).
    pub no_knockback: bool,
    /// The class says it survives the death bounds (`surviveDeathBounds: true`): the source
    /// starts it outside them on purpose, an enemy walking in from off-stage.
    pub survives_bounds: bool,
    /// The projectiles its class declares, by the name its code fires them under.
    pub projectiles: std::collections::BTreeMap<String, crate::abc_parser::ProjectileData>,
    /// The hit itself, exactly as the class declares it, box by box. The damage, launch and angle
    /// are read out of here into their own fields for the geometry work; this is kept whole so the
    /// TIMING the class states -- how long the freeze is, how long the victim is held -- goes out
    /// as stated rather than as a number the port picked.
    pub attack_stats: Vec<std::collections::BTreeMap<String, f64>>,
    /// Which animations the class declares its hit for. Empty when it names none, and then the hit
    /// rides whatever the object draws.
    pub attack_anims: std::collections::BTreeSet<String>,
    /// The clips the class hands over as the SHAPE of the body it gives itself, by the name it
    /// names them with, in the object's own space. A class that picks its shape at runtime names
    /// several; one that always has the same body names one.
    pub platform_shapes: Vec<(String, (f64, f64, f64, f64))>,
    /// The faces of a PLACED body, in the port's units relative to the object's centre: the
    /// collision clip's own outline (a solid's every edge, typed; a soft platform's floor runs).
    /// Empty for a body that is only its declared box.
    pub body_faces: Vec<(&'static str, Vec<(f64, f64)>)>,
    /// A named child of the object's own clip whose rectangle its code reads at run time
    /// (`getMC().hitBox.getRect(parent)`): the child's box in the clip's own space, measured here,
    /// since this engine has no display tree to ask. Unscaled clip pixels.
    pub child_rects: Vec<(String, (f64, f64, f64, f64))>,
    /// The role its placement declared (`platform`, `terrain`). Empty for a spawned object. A
    /// placed object with a role is standable: the game builds its class over that collision clip,
    /// so the port carries a body of the same size.
    pub declared_kind: String,
    /// Period in frames of the on/off pulse (0 = always active).
    pub interval: u32,
    /// Frames the hitbox stays active within each `interval` (ignored when interval is 0).
    pub active: u32,
    /// Movement amplitude (px) and period (frames) for the pattern.
    pub range: f64,
    pub period: u32,
    /// Frames between native-hitbox re-arms (so a lingering fighter keeps taking hits).
    /// = the SSF2 attack's `refreshRate`, frame-doubled.
    pub rehit: u32,
    /// FM knockbackGrowth (= the SSF2 attack's `kbConstant`, per the shared key map).
    pub kb_growth: f64,
    /// Display label (for the entity/layer names).
    pub label: String,
    /// The rasterized SSF2 hazard sprite (the real art), if recovered from the placement tree.
    /// `None` falls back to a generated placeholder box in the emitter.
    pub art: Option<StageArt>,
    /// The hazard's labelled sub-animations (e.g. a Thwomp's `entrance`/`fall`/`idle`), recovered
    /// from its SSF2 animation clip's frame labels. When present, the emitter builds a
    /// multi-animation custom game object whose Substate machine switches between them; empty falls
    /// back to the single-animation [`Hazard::art`].
    pub anims: Vec<ClipAnim>,
    /// The clip's SSF2 `CollisonBox`/`attackBox` shapes, clip-local and FM-scaled (one per box; the
    /// Thwomp's `fall` carries a left + right pair). The multi-animation emitter rides these on its
    /// damaging animation as HIT_BOX layers, so the hit volume is the real SSF2 attackBox geometry
    /// (recovered like the lava's) instead of the art canvas or a guessed size.
    pub attack_boxes: Vec<Rect>,
    /// The clip holds a body box (`hitBox`) of its own and so hurts no one by touch (see
    /// `body_only`)
    pub body_box: bool,
}

/// One labelled animation of an SSF2 movieclip (a frame-label segment, e.g. `idle`/`fall`), as a
/// sequence of rasterized frames. Universal: used for multi-animation hazard CGOs and any other
/// stage clip whose code switches animations by label. Frame lengths are SSF2 (30fps) units; the
/// emitter applies the same 30->60fps doubling as the rest of the stage.
#[derive(Clone, Debug)]
pub struct ClipAnim {
    /// FM animation name (the SSF2 frame label, camelCased + prefixed by the emitter).
    pub label: String,
    /// The animation's frames, in order (one rasterized image per source frame).
    pub frames: Vec<StageArt>,
    /// 0-based frame the SWF timeline stop()s on, if the sub-clip HOLDS instead of looping (its
    /// Flash-generated `_fla.` timeline class has a stopping frame script). `frames` is already
    /// truncated to this; the emitter maps it to AnimationEndType.NONE. None = the clip loops.
    pub stop_frame: Option<usize>,
    /// `setYSpeed` actions the sub-clip's frame scripts drive (1-based source frame, 30fps speed)
    /// — the thwomp entrance's descend/hover/rise bob. Empty for clips without such scripts.
    pub frame_velocities: Vec<(u32, f64)>,
    /// The driver clip's own frame scripts, by the label each sits under, the frames past it,
    /// and the script's code: what the clip asks of the stage as it plays (phase 8's waterfall
    /// has the stage remove it at its last frame). The same for every animation of one clip.
    pub clip_scripts: Vec<(String, u32, String)>,
    /// The source LOOPS this one and says so: its own frame script sends playback back to the
    /// label it is on. That is a declaration, not an absence, so it outranks what the class
    /// driving the object would otherwise imply.
    pub loops: bool,
    /// When the driving clip picks its segment at random (`"loop_" + randomInteger(1, 4)`), the
    /// labels it picks between. Carried on each of them so the emitter can make the same choice.
    pub pick_between: Vec<String>,
}

impl Hazard {
    /// Re-arm cadence for the native hitbox, in frames (min 1).
    pub fn rehit(&self) -> u32 { self.rehit.max(1) }
}

/// Anchor points the placement walk records: the stage MC's world registration (the FM-space
/// origin) and the terrain MC's world registration (the GAME-space origin — the coordinate
/// space SSF2's AS3 spawn calls and runtime X/Y use).
#[derive(Default)]
struct WalkAnchors {
    origin: Option<(f64, f64)>,
    terrain: Option<(f64, f64)>,
    /// the terrain clip's frame labels: the sections a stage's code jumps its planes between
    terrain_labels: Vec<(String, u16)>,
    /// how many frames the terrain clip's own timeline runs
    terrain_frames: usize,
    /// the terrain clip itself
    terrain_id: Option<u16>,
}

/// A player beacon (match-start "entrance" or respawn point) in FM coords.
#[derive(Clone, Debug)]
pub struct SpawnPoint {
    /// 0-based player index (p1 -> 0 …).
    pub index: usize,
    pub x: f64,
    pub y: f64,
    /// `true` if the beacon faces left (SSF2 scaleX < 0).
    pub face_left: bool,
}

/// The parsed SSF2 stage geometry, in FM stage coordinates.
///
/// `Default` exists so a stage can be built from scratch — the emitter tests construct one
/// directly instead of parsing a real SSF2 file, which keeps official content out of the
/// default test gate. `scale` defaults to 0.0 under `Default`, so a synthetic model must set
/// it to a real multiplier (the parser always does).
/// A stage's particle weather: many copies of one small clip drifting across the camera, with the
/// particle already rasterised. Neither timeline art nor a hazard, so nothing else in the pipeline
/// picks it up -- see `abc_parser::extract_stage_weather`.
#[derive(Clone, Debug)]
pub struct Weather {
    /// The particle, rendered once. Every copy draws the same picture.
    pub art: StageArt,
    /// The particle's own animation when its clip has one (an ember flickers through its frames
    /// as it rises), in the clip's own coordinates; empty when the clip is one picture.
    pub frames: Vec<StageArt>,
    pub count: u32,
    /// The area particles are scattered over and wrap within, in FM units.
    pub width: f64,
    pub height: f64,
    /// How the source's own weather class moves one particle per frame.
    pub motion: Option<crate::abc_parser::WeatherMotion>,
    /// The stage field this weather is kept in. A stage can keep several and only this one is the
    /// field this port built, so only its strength means anything here.
    pub field: String,
    /// The library name of the particle's clip (the class the stage's code makes each one from).
    pub linkage: String,
}

#[derive(Clone, Debug, Default)]
pub struct StageModel {
    /// The file this was read from, so what is emitted can go back for anything the model does not
    /// carry in full (its sounds, which are audio data rather than geometry).
    pub source: std::path::PathBuf,
    /// Regions the stage's own code asks for by name (`getCollisionBoundary("safezone")`), in FM
    /// coords. The placement IS the region, so the stage can be given the same answer it asked for.
    pub named_boxes: std::collections::BTreeMap<String, Rect>,
    /// Where each named clip's own origin sits (its placement point), in this stage's coordinates:
    /// the point the source's code moves the clip by (`sprite.y = plat.getY()`).
    pub named_regs: std::collections::BTreeMap<String, (f64, f64)>,
    /// every library clip's extent in its own space (source pixels), by its library name: what
    /// the code makes a surface the shape of (`createSelfPlatformWithMC`)
    pub clip_bounds: std::collections::BTreeMap<String, Rect>,
    /// every clip's extent by its character id (a named child's box, which its code reads)
    pub sprite_bounds: std::collections::BTreeMap<u16, Rect>,
    /// the camera backgrounds the package declares, by library name, in the order it declares
    /// them: what the stage's code reaches by index
    pub cam_backgrounds: Vec<String>,
    /// What a named clip's own timeline does as it plays: (instance name, label, frames past that
    /// label, the frame script's code). A plane that switches its sections by label writes their
    /// `fallthrough` from these; the stage script runs each as the clip's pieces reach the frame.
    pub clip_frame_scripts: Vec<(String, String, u32, String)>,
    /// Clips that hide themselves on their own first frame: the stage shows them at a moment of
    /// its own choosing (crateria's acid warning sign), so the port starts them hidden too.
    pub hidden_clips: std::collections::BTreeSet<String>,
    /// Every name the package gives one of its own symbols. The stage's code hands these around
    /// as values -- the class of the splash to make, the class of the raindrop to fall -- and a
    /// name is all they are here, so a read of one has something to answer with.
    pub symbol_names: std::collections::BTreeSet<String>,
    /// Particle weather, one entry per field the stage hands to the weather layer. A stage can run
    /// more than one at a time and turn them on and off separately.
    pub weathers: Vec<Weather>,
    /// What this stage's own code puts behind the hazards switch.
    pub hazard_gate: crate::abc_parser::HazardGate,
    /// Content id (from `Main.id`, fallback file stem). The emitter may suffix this
    /// (`<id>ssf2`) so it can't shadow a built-in stage; `display_name` stays clean.
    pub id: String,
    /// The stage class's own behaviour, decompiled and translated. Carried to the emitter rather
    /// than used here, because turning a spawn into content needs the FINAL id -- which is only
    /// settled after parsing (the `ssf2` suffix).
    pub stage_script: Option<String>,
    /// the terrain clip's frame labels (frame, in source frames): the sections the stage's code
    /// jumps its planes between; the stage's own painted art carries them as frame labels
    pub terrain_labels: Vec<(String, u16)>,
    /// how many frames the terrain clip's own timeline runs, whatever the painted art came to
    pub terrain_frames: usize,
    /// Whether the terrain plane's OWN timeline moves anything drawn: a plane that only walks its
    /// warning boxes and markers over its frames (sand ocean) paints one picture, whatever loops
    /// inside its clips; one that moves its art (a rising pillar) is animation of its own.
    pub terrain_art_moves: bool,
    /// objects the package declares that this port leaves out, as (class, base, the clip it names),
    /// so a check against the source reads them as left out rather than missing
    pub deferred: Vec<(String, String, Option<String>)>,
    /// Human display name for the stage-select screen (override map, else the SSF2 id
    /// title-cased).
    pub display_name: String,
    /// Source series (for the description), if known from the override map.
    pub series: Option<String>,
    /// The stage's original SSF2 soundtrack track ids (`bgm_*`), preserved in the
    /// description. Not playable in FM (the audio isn't shipped), recorded for the author.
    pub ssf2_music: Vec<String>,
    /// FM bgm resource ids the converted stage actually references (override map, else the
    /// configured default). Must be real public FM resources so the match can start.
    pub fm_music: Vec<String>,
    /// Collision platforms (floor + soft platforms), top surfaces are `rect.top()`.
    pub platforms: Vec<Platform>,
    /// Blast zone (KO boundary).
    pub death_box: Option<Rect>,
    /// Hard camera bounds.
    pub camera_box: Option<Rect>,
    /// Match-start beacons (SSF2 `pN_Start`), ordered by player index.
    pub entrances: Vec<SpawnPoint>,
    /// Respawn beacons (SSF2 `pN_Spawn`), ordered by player index.
    pub respawns: Vec<SpawnPoint>,
    /// Grabbable ledge x-positions `(left, right)` from the SSF2 `ledge_mc` instances,
    /// in FM coords. Used as the main floor's left/right endpoints.
    pub ledges: Option<(f64, f64)>,
    /// Stage hazards, each emitted as a Fraymakers custom game object the stage spawns. SSF2
    /// hazards are bespoke per-stage AS3 (lava, thwomps, …) so they aren't auto-ported; these
    /// are declared opt-in in `mappings/stage/metadata.jsonc` and become real, author-editable
    /// FM custom-game-object hazards (a damaging hitbox volume).
    pub hazards: Vec<Hazard>,
    /// Thwomp-style hazard target columns as FM x coords, converted from the GAME-space values
    /// in `metadata.jsonc` (read 1:1 from the stage class's `update` disasm). Empty when the
    /// stage has none; the emitter drives the falling-hazard cycle over them.
    pub sink_columns: Vec<f64>,
    /// Rendered stage art, split into depth layers (the painted backdrop, the main
    /// stage at character depth, and a foreground that draws in front of fighters).
    pub art: StageArtSet,
    /// Non-fatal validation notes: required SSF2 stage linkages that were missing from
    /// the source (e.g. no collision floor, no spawn beacons). Surfaced to the user.
    pub warnings: Vec<String>,
    /// SSF2 -> Fraymakers spatial scale (the `size_multiplier` knob, default 1.3). Fraymakers
    /// space is SSF2 space scaled up by this factor (characters are rendered at this scale,
    /// so the stage geometry + art must match it). Geometry coords are already scaled; the
    /// emitter renders the art IMAGE layers at this scale.
    pub scale: f64,
    /// The source-space point the emitted stage is measured from: FM = (source - origin) * scale.
    /// Ported stage code carries the source's own screen coordinates, so anything it states as a
    /// constant has to come through here to land where the art did.
    pub origin: (f64, f64),
    /// The terrain clip's registration relative to the stage clip. The stage's own AS3 states a
    /// spawn in TERRAIN-local coordinates, so this is what lines those up with everything measured
    /// off the art -- see `stage_emit::entity_origin`.
    pub terrain_off: Option<(f64, f64)>,
}

/// The stage art split by depth so the emitter can layer it around the characters
/// and parallax-scroll the background.
#[derive(Clone, Debug, Default)]
pub struct StageArtSet {
    /// Painted backdrop (SSF2 `<id>_bg` / `background`) as ORDERED per-element layers, back-to-
    /// front. SSF2 authors each animated backdrop element (lava, torches, embers, podoboos, ...)
    /// as its own movieclip on its own loop, so each becomes its own FM layer/animation rather
    /// than one baked composite. A layer with one frame is static (drawn once); more than one =
    /// that element animates and the emitter loops its frames at its own pace. The whole list
    /// moves 1:1 with the world (it carries the surface fighters stand on).
    pub background: Vec<BgLayer>,
    /// Camera-relative parallax layers (the SSF2 `<id>_bg` backdrop + the `_cambg` layers that
    /// `SSF2Stage.getCameraBackgrounds` returns), back-to-front. Each is its OWN plane with its
    /// OWN pan rate (SSF2 auto-derives it from the layer size). Empty for the 109/110 corpus
    /// stages with no camera backgrounds.
    pub parallax: Vec<ParallaxLayer>,
    /// The main stage art (terrain / platforms / props) at character depth. More than
    /// one frame when the source has animated clips (the emitter loops them).
    pub stage_frames: Vec<StageArt>,
    /// The elements of that same plane that MOVE, split out the way [`Self::background`] is, so
    /// each keeps its own loop. The plane is painted as one picture and a picture can only carry
    /// one timeline, so without this an element only animated when everything around it animated
    /// in step. Empty when nothing on the plane moves.
    pub stage_elements: Vec<BgLayer>,
    /// Art that draws in front of the fighters (SSF2 `foreground`), as a possibly-animated,
    /// semi-transparent overlay (bowserscastle's shimmering lava-glow sheet + lightmask tint). One
    /// frame = static, more = the emitter loops it. Empty = no foreground.
    pub foreground: Vec<StageArt>,
    /// The SAME foreground plane as [`Self::foreground`], but split into per-element layers the way
    /// [`Self::background`] is, so each object keeps its OWN loop instead of being frozen into one
    /// composite. Populated whenever the grouping finds elements; the flat `foreground` is then
    /// left empty so the two can't both draw. The plane is one image only as a fallback.
    pub foreground_elements: Vec<BgLayer>,
    /// Clips the stage's CODE makes from the library at run time (`getMCByLinkageName`), each
    /// rasterised on its own timeline in its own local space (frames in FM px relative to the
    /// clip's registration). Never placed at boot: the script makes them when the source does.
    pub runtime_clips: Vec<BgLayer>,
    /// Symbol names of placed clips whose animated segments are ALL event-driven (reached only by a
    /// script jump on a game event, never from the clip's own timeline: a goal light that flashes
    /// when a goal is scored). The port keeps the idle it can reach and drops the rest, so the
    /// clip's full source length is not reproduced passively. Recorded so a cross-engine check
    /// reads it as deferred rather than as a missing animation.
    pub deferred_event_clips: Vec<String>,
    /// The stage's SHADOW MASK, if it declares one: the shape that says where a fighter's shadow
    /// falls. SSF2 keeps it as a clip in its own plane and never draws it; Fraymakers takes the
    /// same idea as a mask sprite on a shadow layer. Dropped as scaffolding, the port simply has
    /// no shadows where the source has them.
    pub shadow_mask: Option<StageArt>,
    /// Where the stage's own light sits, in FM coords, when it parks a `light_source` clip in a
    /// plane it never draws. The shadows a stage casts are aimed from here.
    pub light_source: Option<(f64, f64)>,
    /// OPAQUE foreground occluders that draw in front of the fighter but BEHIND the semi-transparent
    /// `foreground` tint: the near face of a standable structure SSF2 split off as a separate piece
    /// (bowscastle's bridge parapet) so a fighter standing on the deck is occluded from the front.
    /// Rendered at full alpha (a translucent occluder would show the fighter through solid brick).
    pub foreground_occluders: Vec<StageArt>,
    /// The foreground plane's own opacity, as authored: the product of the colour-transform alpha
    /// multipliers on the SSF2 placement chain (a translucent plane is made by setting alpha on a
    /// PARENT movieclip). Read from the source per stage; it is NOT a fixed convention, so the
    /// emitter must use this rather than assume a value.
    pub foreground_alpha: f64,
    /// `true` when [`Self::foreground`] holds a SOLVED blend overlay rather than ordinary
    /// semi-transparent foreground art. Such an overlay already carries the exact per-pixel alpha
    /// the blend needs, so the emitter must ship it at full layer alpha — halving it (the ~0.5 the
    /// authored foreground sheet wants) halves the blend itself and the tint reads as missing.
    pub foreground_is_blend_overlay: bool,
}

impl StageArtSet {
    /// `true` if no layer rasterized (e.g. a stage with only bitmap fills we can't
    /// decode) — the emitter then falls back to a geometry placeholder.
    pub fn is_empty(&self) -> bool {
        self.background.is_empty() && self.parallax.is_empty()
            && self.stage_frames.is_empty() && self.foreground.is_empty()
    }
}

/// How a camera-background layer scrolls (mirrors SSF2 `VcamBGSettings.mode` =
/// Fraymakers `ParallaxMode`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParallaxMode {
    /// Straight parallax pan at `x_pan`/`y_pan` (`bg = camera * multiplier`). SSF2's `_cambg`
    /// discrete layers use this (the `autoPanMultiplier` feeds the pan).
    Pan,
    /// Anchored to the camera bounds, tiling to fill (SSF2 makes ±width copies). For a
    /// repeating/wrapping backdrop.
    Bounds,
}

/// One camera-background parallax layer: the composited art + its per-layer scroll mode and
/// pan rate (SSF2 auto-derives the rate from the layer's pixel size; see `parallax_pan`).
#[derive(Clone, Debug)]
pub struct ParallaxLayer {
    pub art: StageArt,
    pub mode: ParallaxMode,
    /// Fraction of the camera's movement this layer scrolls by (0 = screen-fixed).
    pub x_pan: f64,
    pub y_pan: f64,
}

/// One backdrop element as its own animated layer (the SSF2 movieclip model: each animated
/// backdrop object is separate, on its own loop). `name` is the SSF2 symbol id (e.g.
/// `bowsers_torches_lit_bg`) used to name the FM layer; `frames` is 1 (static) or the
/// element's RLE'd animation loop.
#[derive(Clone, Debug)]
pub struct BgLayer {
    pub name: String,
    /// The NAMED CLIP this layer is part of -- the nearest named ancestor, which is what a plane's
    /// child is called in SSF2's display tree and therefore what the stage class addresses
    /// (`getForeground().clocktown_rainsplash_fg`). One clip is one object however much is inside
    /// it: fifty-three splashes nested under that name are one rainsplash, not fifty-three, and
    /// the still plate and the parts moving over it are LAYERS of the clip rather than separate
    /// objects. Keying on the innermost placement instead splits the object the source named.
    pub clip: String,
    /// Every instance name over the element, outermost first: the stage's code reaches a piece
    /// through any of them (`getBackground().movingplatLsprite.sprite`), so the emitter binds the
    /// element under each.
    pub named_path: Vec<String>,
    pub frames: Vec<StageArt>,
    /// The opacity the SOURCE authored for this element (the lowest its placements carry). SSF2
    /// makes an overlay subtle -- or hides it entirely, leaving it for code to raise -- by setting
    /// alpha on the clip, so an element spawned at full strength paints over the whole stage.
    pub authored_alpha: f64,
    /// `true` when the SOURCE animates this object's opacity. Recorded before the fade is baked
    /// into the pictures, because after that nothing in the frames says it ever faded -- and a
    /// fading object is a fade/flash sheet, not the backdrop the picture should be anchored on.
    pub fades: bool,
    /// Set when this element came out of a CAMERA BACKGROUND: the fraction of the camera's
    /// movement it scrolls by. An animated one cannot be a flattened parallax plate (that is what
    /// froze clocktown's clock), so it stays an element and carries its layer's pan instead.
    pub pan: Option<(f64, f64)>,
    /// When the STAGE puts this object on screen, in emitted frames: `(first, last)` per run.
    /// Empty means always. See [`split_loop_and_presence`].
    pub presence: Vec<(usize, usize)>,
    /// The element's label-delimited segments, when its clip's timeline is a state machine rather
    /// than one loop. Empty for an ordinary looping element, which is the common case. When this is
    /// populated the emitter must NOT play `frames` end to end: only the reachable segments are ever
    /// shown in SSF2, and they are chained by [`BgSegment::next`], not by running off the end.
    pub segments: Vec<BgSegment>,
    /// The source draws this element BEHIND the art that got baked into the stage picture.
    /// A promoted element is reparented into a container, and the containers sit either side of
    /// the baked art, so this is what decides which one. Fire in a brazier is the case that needs
    /// it: the basket is baked stage art and the flame belongs behind its lattice, not on top.
    pub behind_base: bool,
    /// The clip hides itself on its own first frame: it is drawn only once the stage plays a
    /// label on it (a caution sign, a spirit), and starts unseen.
    pub starts_hidden: bool,
}

/// One label-delimited stretch of a backdrop element's own timeline.
#[derive(Clone, Debug)]
pub struct BgSegment {
    /// The SSF2 frame label naming this segment (`idle`, `wait`, `lose`).
    pub label: String,
    /// Its rasterized frames, in order, at SSF2 (30fps) rate.
    pub frames: Vec<StageArt>,
    /// What the timeline does when the segment ends. `None` = nothing scripted, so playback simply
    /// runs on into the next segment (the last segment wrapping to the first).
    pub next: Option<crate::abc_parser::LabelJump>,
}

/// A composited stage-art image ready to drop in as an IMAGE layer / parallax bg.
#[derive(Clone, Debug)]
pub struct StageArt {
    /// PNG bytes (RGBA).
    pub png: Vec<u8>,
    /// Top-left of the image in FM stage coords.
    pub x: f64,
    pub y: f64,
    /// Pixel dimensions (for parallax `originalBGWidth/Height`).
    pub w: u32,
    pub h: u32,
    /// Frames to display this image before advancing (FM 60fps frames). For an animated layer
    /// this is the run length of identical source frames times the 30->60fps doubling, so the
    /// loop matches the SSF2 duration and held frames read as pauses. 1 for a static layer.
    pub hold: u32,
    /// Orientation for this frame, when the element is ONE shape the source moves or turns rather
    /// than a composite redrawn each frame. The art is then emitted once and placed by these --
    /// the same thing the character path does, and the difference between a rotating clock hand
    /// costing one bitmap and costing one per angle.
    pub rotation: f64,
    pub scale_x: f64,
    pub scale_y: f64,
    /// Where the art TURNS about, in image pixels from its top-left. A shape rotates about its own
    /// registration point, which is not the corner of the raster it was cropped to -- and a piece
    /// turned about the wrong point swings out of place instead of spinning where it sits.
    pub pivot: (f64, f64),
    /// The opacity the SOURCE gives this frame. SSF2 fades a backdrop between its day and night
    /// skies, and flashes the screen white, by animating alpha -- so a port that carries only
    /// pixels loses the transition and shows a hard cut.
    pub alpha: f64,
    /// The frame's COLOUR, as a per-channel multiply and offset (rgb only -- alpha is carried
    /// above and applying it twice would darken the fade). SSF2 slides a stage from day to night
    /// with this, so art baked without it holds its painted colours and changes only when a CEL
    /// does: a hard step where the source slides. The engine takes exactly these terms.
    pub tint: [f64; 6],
}

impl Default for StageArt {
    fn default() -> Self {
        StageArt { png: Vec::new(), x: 0.0, y: 0.0, w: 0, h: 0, hold: 1,
                   rotation: 0.0, scale_x: 1.0, scale_y: 1.0, alpha: 1.0, pivot: (0.0, 0.0),
                   tint: TINT_ID }
    }
}

impl StageModel {
    /// The main (solid) floor: the widest non-drop-through platform, if any.
    pub fn main_floor(&self) -> Option<&Platform> {
        // a hazard floor (a standable molten lake) never anchors the stage, however wide.
        self.platforms.iter().filter(|p| !p.drop_through)
            .max_by(|a, b| a.rect.w.total_cmp(&b.rect.w))
    }
}

/// One placed shape instance discovered during the tree walk.
#[derive(Clone)]
struct Instance {
    /// What the nearest enclosing clip SAYS it is, off its own frame script (`self.type =
    /// "platform"`): the fact the collision reads, ahead of any name.
    kind: Option<String>,
    /// The enclosing clip says nobody drops through it.
    no_drop: bool,
    /// The sideways push the enclosing clip declares (`self.x_influence`), in the source's px a frame.
    x_influence: Option<f64>,
    /// The key the clip itself declares it moves in step with (`self.syncPlatform`).
    sync_key: Option<String>,
    /// The ground friction and walk acceleration the enclosing clip declares (source px a frame squared).
    friction: Option<(f64, f64)>,
    /// The placed `DefineShape` id (so its art can be rasterized later).
    shape_id: u16,
    /// PlaceObject instance name (e.g. `deathBoundary`), if any.
    inst_name: Option<String>,
    /// SWF SymbolClass name of the nearest named sprite ancestor (e.g.
    /// `battlefield_fla.battlefield_TerrainMC_5`). Fla-prefixed + auto-numbered, so it's a
    /// weak cross-stage signal; prefer [`Instance::plane`] for art and linkage suffixes for
    /// markers.
    sym_name: String,
    /// The stage-root plane this instance descends from, by INSTANCE name (the stable AS3
    /// slot: `terrain` / `background` / `foreground` / `shadowMask` / `reflectionMask`), or
    /// an extra root instance (`*_cambg` parallax, `stance`, decorations). The robust art
    /// classifier (instance names generalize across stages; symbol names do not).
    plane: Option<String>,
    /// World AABB in raw SSF2 coords.
    aabb: Rect,
    /// World center (raw SSF2 coords).
    cx: f64,
    cy: f64,
    /// `-1.0` if mirrored along x (facing left).
    x_sign: f64,
    /// Axis flips of this leaf's world placement (`scaleX`/`scaleY` < 0). The art is composited
    /// onto an axis-aligned AABB, so a flipped placement must mirror the rasterized tile to render
    /// the source orientation. Without this, a mirrored multi-frame element (e.g. a wall torch
    /// whose flame frames are cropped bitmaps) draws each frame un-mirrored and its content slides
    /// frame to frame (a false left/right wiggle).
    flip: (bool, bool),
    /// The hazard kind this shape descends from, if any. SSF2 hazards (lava/thwomp/…) are named
    /// sprite containers whose leaf shapes carry deeper auto-named symbols, so the hazard signal
    /// lives on an ancestor and is propagated down the subtree (like [`Instance::moving`]).
    /// World position of the nearest NAMED MovieClip ancestor (the placement point of this leaf's
    /// owning instance). Repeated placements of one symbol (e.g. 16 torch-ember emitters) share a
    /// `sym_name` but get DISTINCT anchors, so the art grouping can split them into one object per
    /// placement at its own position instead of merging them into one union-bounds image.
    inst_anchor: (f64, f64),
    /// The display FILTERS this leaf renders under, gathered down the subtree. Flash filters a
    /// clip after it composites; art arrives here one shape at a time, so a chain on an ancestor
    /// runs on each shape beneath it.
    filters: Vec<swf::Filter>,
    /// The instance names over this leaf, outermost first (what the stage's code reaches it by).
    named_path: Vec<String>,
    /// The placement's SWF blend mode, inherited down the subtree (`None` = Normal). The
    /// compositor emulates the modes it can (HardLight) when baking.
    blend: Option<swf::BlendMode>,
    /// Hash of the placement-depth chain from the root to this leaf's owning clip. Two sibling
    /// clips placed at the SAME anchor (same symbol, same origin) still differ here, so the art
    /// grouping splits them into independent elements that each loop on their OWN period (a
    /// merged capture cuts every child whose cycle doesn't divide the parent's).
    inst_path: u64,
    /// This leaf's world placement, in the shared form (`fm_placement`). An element whose frames
    /// are ONE shape being moved or turned is emitted as that shape once plus these per frame,
    /// instead of a fresh raster per angle.
    place: crate::fm_placement::WorldPlacement,
    /// This leaf's OWN placement depth inside its owning clip. Sibling shapes in one clip share
    /// `inst_path` (they belong to the same clip), so this is what tells them apart -- the static
    /// backdrop plate from the clock hands animating over it.
    leaf_depth: u16,
    /// Sprite definition id of the clip that named this leaf (the one supplying [`Self::sym_name`]).
    /// The element's own timeline lives there, so it's what carries the frame LABELS: a clip whose
    /// labels split it into segments is a state machine, not a loop, and must not be played end to
    /// end. `None` for leaves with no named clip ancestor.
    inst_clip: Option<u16>,
    /// `true` when a clip on this leaf's chain hides itself on its own first frame: the source
    /// never draws it (a terrain plate), whatever name it was placed under.
    hidden_by_class: bool,
    /// Placed with `visible = false` (or under a placement that was): the source's collision
    /// plates are drawn by nothing, and a container that holds such plates draws its own shapes.
    hidden_placed: bool,
    /// `true` when a clip on this leaf's chain declares a collision role: what it draws may be
    /// art, but its timeline is not what makes it so.
    under_collision: bool,
    /// Product of the colour-transform alpha multipliers down this leaf's placement chain. SSF2
    /// authors a translucent plane by setting alpha on a PARENT movieclip, so the value belongs to
    /// the source, not to the plane: it varies per stage and must never be assumed.
    alpha: f64,
    /// The colour transform down this leaf's placement chain, composed the same way alpha is. SSF2
    /// slides a whole stage from day to night with it, so a port that drops it holds the art at its
    /// painted colours and only changes when a CEL does, which is a step where the source slides.
    tint: crate::swf_timeline::Tint,
    /// How often this leaf's picture comes round: the lengths of every clip with a timeline down
    /// its placement chain, named or not, as a least common multiple (1 for a still chain). An
    /// unnamed clip that slides a piece back and forth is the piece's loop even when the nearest
    /// NAMED clip is the whole stage.
    chain_len: usize,
}

/// Parse the SSF2 stage at `path` into a [`StageModel`], rendering its art (read-only).
pub fn parse_stage(path: &Path) -> Result<StageModel> {
    parse_stage_opts(path, true)
}

/// Parse the SSF2 stage at `path`. `render_art` toggles the (relatively expensive)
/// art rasterization: the converter needs it, but a geometry-only pass (e.g. the
/// coverage test, or `--info`) skips decoding + compositing every shape/bitmap.
pub fn parse_stage_opts(path: &Path, render_art_flag: bool) -> Result<StageModel> {
    let raw = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let swf_data = ssf_decompress(&raw, path)?;
    let buf = swf::decompress_swf(&swf_data[..]).context("decompress SWF")?;
    let swf = swf::parse_swf(&buf).context("parse SWF")?;

    let meta = stage_package_metadata(&swf_data);
    let id = meta.as_ref().and_then(|m| m.id.clone()).unwrap_or_else(|| {
        path.file_stem().and_then(|s| s.to_str()).unwrap_or("stage").to_string()
    });
    let cam_backgrounds = meta.as_ref().map(|m| m.camera_backgrounds.clone()).unwrap_or_default();
    let ssf2_music = meta.map(|m| m.music).unwrap_or_default();
    // the stage's AS3 plane assignments + spawned actors (authoritative; heuristic is the fallback).
    let abc_model = stage_abc_model(&swf_data);
    if std::env::var("PEPTIDE_STAGE_DEBUG").is_ok() {
        match &abc_model {
            Some(m) => {
                eprintln!("[as3] doc_class={} planes={} actors={}", m.doc_class, m.planes.len(), m.actors.len());
                if let Ok(path) = std::env::var("PEPTIDE_DUMP_STAGE_SCRIPT") {
                    let _ = std::fs::write(&path, m.reconstructed_script.clone().unwrap_or_default());
                    eprintln!("[as3] stage script -> {path}");
                }
                for (clip, plane) in &m.planes { eprintln!("  plane {:<10?} <- {clip}", plane); }
                for a in &m.actors {
                    eprintln!("  actor {} : {} @ (x={:?}, y={:?})", a.class_name, a.base, a.x, a.y);
                    eprintln!("      hitboxes={} own_stats={:?} anims={:?} faller={} script={}",
                        a.attack_hitboxes.len(), a.own_stats, a.anim_labels,
                        a.faller.as_ref().map(|f| format!("{f:?}")).unwrap_or("none".into()),
                        a.reconstructed_script.is_some());
                    eprintln!("      behavior={:?}", a.behavior);
                }
            }
            None => eprintln!("[as3] no SSF2Stage subclass found (heuristic fallback)"),
        }
    }
    // display name + FM music: the override map keyed by the SSF2 id, else title-case +
    // the default bgm. The original SSF2 soundtrack (ssf2_music) is preserved separately.
    let smeta = crate::mappings::stage_metadata();
    let entry = smeta.stages.get(&id);
    let display_name = entry.and_then(|e| e.name.clone()).unwrap_or_else(|| title_case(&id));
    let series = entry.and_then(|e| e.series.clone());
    let fm_music = match entry.map(|e| &e.music) {
        Some(m) if !m.is_empty() => m.clone(),
        _ => vec![smeta.default_music.clone()],
    };
    // SymbolClass id -> name; DefineShape bounds; DefineSprite tag lists.
    let mut sym_names: BTreeMap<u16, String> = BTreeMap::new();
    let mut shape_bounds: BTreeMap<u16, (f64, f64, f64, f64)> = BTreeMap::new();
    let mut sprites: BTreeMap<u16, &Vec<swf::Tag>> = BTreeMap::new();
    for tag in &swf.tags {
        match tag {
            swf::Tag::SymbolClass(links) => {
                for link in links {
                    let n = link.class_name.to_str_lossy(encoding_rs::WINDOWS_1252).to_string();
                    if !n.is_empty() { sym_names.insert(link.id, n); }
                }
            }
            swf::Tag::DefineShape(s) => {
                let b = &s.shape_bounds;
                shape_bounds.insert(s.id, (
                    b.x_min.get() as f64 / 20.0, b.y_min.get() as f64 / 20.0,
                    b.x_max.get() as f64 / 20.0, b.y_max.get() as f64 / 20.0,
                ));
            }
            swf::Tag::DefineSprite(s) => {
                sprites.insert(s.id, &s.tags);
            }
            _ => {}
        }
    }
    // DefineShape registry, for rasterizing stage art.
    let mut shape_defs: BTreeMap<u16, &swf::Shape> = BTreeMap::new();
    for tag in &swf.tags {
        if let swf::Tag::DefineShape(s) = tag { shape_defs.insert(s.id, s); }
    }
    // debug (`PEPTIDE_DUMP_SHAPE=<ids>`): a shape's bounds + fill styles, gradients in full.
    if let Ok(ids) = std::env::var("PEPTIDE_PATH_DUMP") {
        for id in ids.split(',').filter_map(|x| x.trim().parse::<u16>().ok()) {
            let Some(sh) = shape_defs.get(&id) else { continue };
            let (mut moves, mut edges, mut styles) = (0, 0, 0);
            let mut xs: Vec<f64> = Vec::new();
            let (mut cx, mut cy) = (0.0f64, 0.0f64);
            for r in &sh.shape {
                match r {
                    swf::ShapeRecord::StyleChange(sc) => {
                        styles += 1;
                        if let Some(mv) = sc.move_to { moves += 1; cx = mv.x.get() as f64; cy = mv.y.get() as f64; xs.push(cx / 20.0); }
                        let _ = cy;
                    }
                    swf::ShapeRecord::StraightEdge { delta } => { edges += 1; cx += delta.dx.get() as f64; xs.push(cx / 20.0); }
                    swf::ShapeRecord::CurvedEdge { control_delta, anchor_delta } => {
                        edges += 1; cx += control_delta.dx.get() as f64 + anchor_delta.dx.get() as f64; xs.push(cx / 20.0);
                    }
                }
            }
            let (mn, mx) = (xs.iter().cloned().fold(f64::MAX, f64::min), xs.iter().cloned().fold(f64::MIN, f64::max));
            eprintln!("[path] shape {id}: {styles} style-changes ({moves} moveTo), {edges} edges, x span {mn:.0}..{mx:.0}px");
        }
    }
    if let Ok(ids) = std::env::var("PEPTIDE_DUMP_SHAPE") {
        for id in ids.split(',').filter_map(|x| x.trim().parse::<u16>().ok()) {
            let Some(sh) = shape_defs.get(&id) else { continue };
            let b = &sh.shape_bounds;
            eprintln!("[shape {id}] bounds px=({:.1},{:.1})..({:.1},{:.1})",
                b.x_min.to_pixels(), b.y_min.to_pixels(), b.x_max.to_pixels(), b.y_max.to_pixels());
            for (i, f) in sh.styles.fill_styles.iter().enumerate() {
                match f {
                    swf::FillStyle::Color(c) => eprintln!("  fill{i}: Color({},{},{},{})", c.r, c.g, c.b, c.a),
                    swf::FillStyle::LinearGradient(g) | swf::FillStyle::RadialGradient(g)
                    | swf::FillStyle::FocalGradient { gradient: g, .. } => {
                        let kind = match f { swf::FillStyle::LinearGradient(_) => "Linear", swf::FillStyle::RadialGradient(_) => "Radial", _ => "Focal" };
                        eprintln!("  fill{i}: {kind}Gradient matrix=[a={} b={} c={} d={} tx={:.1}px ty={:.1}px] spread={:?}",
                            g.matrix.a.to_f64(), g.matrix.b.to_f64(), g.matrix.c.to_f64(), g.matrix.d.to_f64(),
                            g.matrix.tx.to_pixels(), g.matrix.ty.to_pixels(), g.spread);
                        for r in &g.records {
                            eprintln!("    stop ratio={} rgba=({},{},{},{})", r.ratio, r.color.r, r.color.g, r.color.b, r.color.a);
                        }
                    }
                    swf::FillStyle::Bitmap { id: bid, .. } => eprintln!("  fill{i}: Bitmap({bid})"),
                }
            }
        }
    }
    // Bitmap registry (id -> (w, h, RGBA)) for shapes whose fill is a bitmap (stage
    // backgrounds are usually bitmaps, which `vector_raster` can't fill). Only decoded
    // when rendering art (decoding every stage's bitmaps is the slow part).
    let mut bitmaps: BTreeMap<u16, (u32, u32, Vec<u8>)> = BTreeMap::new();
    let mut unsupported_bitmaps: Vec<(u16, &str, String)> = Vec::new();
    if render_art_flag {
        // the JPEG tables the file shares between its pictures, if it keeps any
        let jpeg_tables: Vec<u8> = swf.tags.iter().find_map(|t| match t {
            swf::Tag::JpegTables(d) => Some(d.to_vec()),
            _ => None,
        }).unwrap_or_default();
        for tag in &swf.tags {
            match tag {
                swf::Tag::DefineBitsLossless(b) => {
                    match crate::image_extractor::decode_lossless(b) {
                        Ok(rgba) => { bitmaps.insert(b.id, (b.width as u32, b.height as u32, rgba)); }
                        // a decode that fails silently is art that vanishes: the shape filled with
                        // it rasterises empty and the object is simply absent from the stage.
                        Err(e) => unsupported_bitmaps.push((b.id, "DefineBitsLossless", e.to_string())),
                    }
                }
                swf::Tag::DefineBitsJpeg3(j) => {
                    match crate::image_extractor::decode_jpeg3(j) {
                        Ok((w, h, rgba)) => { bitmaps.insert(j.id, (w, h, rgba)); }
                        Err(e) => unsupported_bitmaps.push((j.id, "DefineBitsJpeg3", e.to_string())),
                    }
                }
                // A picture the file stores as a JPEG, with its tables either in the tag or shared
                // (see `decode_jpeg_with_tables`). Undecoded, the shape filled with it rasterises
                // empty and the object is simply absent from the stage.
                swf::Tag::DefineBitsJpeg2 { id, jpeg_data } => {
                    match crate::image_extractor::decode_jpeg_with_tables(&jpeg_tables, jpeg_data) {
                        Ok((w, h, rgba)) => { bitmaps.insert(*id, (w, h, rgba)); }
                        Err(e) => unsupported_bitmaps.push((*id, "DefineBitsJpeg2", e.to_string())),
                    }
                }
                swf::Tag::DefineBits { id, jpeg_data } => {
                    match crate::image_extractor::decode_jpeg_with_tables(&jpeg_tables, jpeg_data) {
                        Ok((w, h, rgba)) => { bitmaps.insert(*id, (w, h, rgba)); }
                        Err(e) => unsupported_bitmaps.push((*id, "DefineBits", e.to_string())),
                    }
                }
                _ => {}
            }
        }
    }
    if std::env::var("PEPTIDE_STAGE_DEBUG").is_ok() {
        let want = std::env::var("PEPTIDE_BMP_STATS").unwrap_or_default();
        for (id, (w, h, rgba)) in &bitmaps {
            if !want.split(',').any(|x| x.trim().parse::<u16>() == Ok(*id)) { continue; }
            let n = rgba.len() / 4;
            let opaque = (0..n).filter(|i| rgba[i * 4 + 3] > 8).count();
            eprintln!("[bmp-stat] bitmap {id}: {w}x{h} non-transparent={:.1}%",
                100.0 * opaque as f64 / n.max(1) as f64);
        }
    }
    if let Ok(want) = std::env::var("PEPTIDE_BMP_DUMP") {
        for (id, (w, h, rgba)) in &bitmaps {
            if !want.split(',').any(|x| x.trim().parse::<u16>() == Ok(*id)) { continue; }
            if let Some(img) = image::RgbaImage::from_raw(*w, *h, rgba.clone()) {
                let _ = img.save(format!("/tmp/bmp{id}.png"));
                eprintln!("[bmp-dump] wrote /tmp/bmp{id}.png ({w}x{h})");
            }
        }
    }
    if !unsupported_bitmaps.is_empty() {
        log::warn!("stage art: {} bitmap(s) use a tag this build can't decode {:?} — every shape \
            filled with one renders EMPTY", unsupported_bitmaps.len(), unsupported_bitmaps);
        if std::env::var("PEPTIDE_STAGE_DEBUG").is_ok() {
            eprintln!("[bmp-skip] undecodable: {unsupported_bitmaps:?}");
        }
    }

    for (id, sh) in placed_picture_shapes(&swf.tags, &bitmaps).into_iter().chain(morph_start_shapes(&swf.tags)) {
        let b = &sh.shape_bounds;
        shape_bounds.insert(id, (b.x_min.to_pixels(), b.y_min.to_pixels(), b.x_max.to_pixels(), b.y_max.to_pixels()));
        shape_defs.insert(id, sh);
    }

    // PEPTIDE_STAGE_DEBUG dumps the SymbolClass linkage table; PEPTIDE_STAGE_TREE (in `walk`)
    // dumps the placement tree with each instance's resolved plane. Both gated, for stage RE.
    if std::env::var("PEPTIDE_STAGE_DEBUG").is_ok() {
        eprintln!("=== SymbolClass linkage table ({} entries) for {} ===", sym_names.len(), id);
        for (cid, n) in &sym_names { eprintln!("  id={cid} linkage={n:?}"); }
    }

    // the AS3-derived plane map (empty if the stage has no parseable SSF2Stage subclass → the
    // name heuristic in plane_tag takes over).
    let mut planes: PlaneMap = abc_model.as_ref().map(|m| m.planes.clone()).unwrap_or_default();
    tag_declared_cambg(&mut planes, &cam_backgrounds);
    let planes = planes;

    // What each clip in the tree SAYS it is, and which class the stage builds over it. A stage
    // places objects as well as spawning them, and the placement is the declaration: it carries the
    // class, the geometry and the name the stage's own code addresses the object by.
    let clip_roles: BTreeMap<String, crate::abc_parser::ClipRole> = crate::swf_parser::parse(&swf_data)
        .map(|sw| sw.abc_blocks.iter().filter_map(|b| crate::abc_parser::parse(b).ok())
            .flat_map(|abc| crate::abc_parser::declared_clip_roles(&abc))
            .collect())
        .unwrap_or_default();

    // Collect every placed shape instance (with world AABB) and find the stageMC origin.
    let mut instances: Vec<Instance> = Vec::new();
    let mut anchors = WalkAnchors::default();
    // `PEPTIDE_FIND_SYM=needle` lists the package's symbol names holding the needle: which
    // package ships a clip a class names (`setForeground("x")`) is a question this answers
    if let Ok(needle) = std::env::var("PEPTIDE_FIND_SYM") {
        for (id, n) in &sym_names { if n.to_ascii_lowercase().contains(&needle.to_ascii_lowercase()) { eprintln!("[sym] #{id} {n} sprite={}", sprites.contains_key(id)); } }
        // ...and every package the game's manifest names, for a clip this one only refers to
        for (pkg, data) in crate::shared_packages::every_package() {
            let Ok(sw) = crate::swf_parser::parse(&data) else { continue };
            for (id, n) in &sw.symbols {
                if n.to_ascii_lowercase().contains(&needle.to_ascii_lowercase()) { eprintln!("[sym] {pkg}: #{id} {n}"); }
            }
        }
    }
    let (class_sites, named_clips, hidden_clips, named_boxes, named_regs) = collect_class_sites(&swf.tags, &sym_names, &sprites, &shape_bounds, &clip_roles);
    let mut extent_memo = BTreeMap::new();
    let sprite_bounds: BTreeMap<u16, Rect> = sprites.keys()
        .filter_map(|id| clip_extent_memo(*id, &sprites, &shape_bounds, &mut extent_memo).map(|r| (*id, r))).collect();
    let clip_bounds: BTreeMap<String, Rect> = sym_names.iter()
        .filter_map(|(id, n)| sprite_bounds.get(id).map(|r| (n.clone(), *r))).collect();
    if std::env::var("PEPTIDE_PLAT_DEBUG").is_ok() {
        eprintln!("[hidden-clips] {:?}", hidden_clips);
    }
    // the frame scripts of every clip the tree places under a name, by the label and offset
    // the clip reaches them at: what the clip's own timeline does to its children as it plays
    let clip_frame_scripts: Vec<(String, String, u32, String)> = {
        let abcs: Vec<crate::abc_parser::AbcFile> = crate::swf_parser::parse(&swf_data)
            .map(|sw| sw.abc_blocks.iter().filter_map(|b| crate::abc_parser::parse(b).ok()).collect())
            .unwrap_or_default();
        let mut out = Vec::new();
        for (name, cid) in &named_clips {
            let Some(class) = sym_names.get(cid) else { continue };
            let Some(tags) = sprites.get(cid) else { continue };
            let labels = crate::sprite_parser::extract_frame_labels_from_tags(tags);
            if std::env::var("PEPTIDE_STAGE_DEBUG").is_ok() {
                let n: usize = abcs.iter().map(|a| crate::abc_parser::extract_frame_scripts(a, class).len()).sum();
                eprintln!("[clip-scripts] {name} -> {class} ({n} frame scripts, {} labels)", labels.len());
            }
            for abc in &abcs {
                for (frame, code) in crate::abc_parser::extract_frame_scripts(abc, class) {
                    // a frame script is named from 1, a label's frame counted from 0
                    let index = frame.saturating_sub(1);
                    let Some((label, at)) = labels.iter().filter(|(_, f)| u32::from(*f) <= index)
                        .max_by_key(|(_, f)| *f).map(|(l, f)| (l.clone(), u32::from(*f))) else { continue };
                    out.push((name.clone(), label, index - at, code));
                }
            }
        }
        out
    };
    let walk_abcs: Vec<crate::abc_parser::AbcFile> = crate::swf_parser::parse(&swf_data)
        .map(|sw| sw.abc_blocks.iter().filter_map(|b| crate::abc_parser::parse(b).ok()).collect())
        .unwrap_or_default();
    walk(&swf.tags, root_mat(&swf.tags, &sym_names), &sym_names, &shape_bounds, &sprites, &planes, &clip_roles, 0, None, None, false, false, None, None, &mut instances, &mut anchors, false, None, None, None, &hidden_clips, usize::MAX, &walk_abcs);
    if std::env::var("PEPTIDE_STAGE_DEBUG").is_ok() {
        eprintln!("[hidden-clips] {hidden_clips:?}");
    }
    if std::env::var("PEPTIDE_STAGE_TREE").is_ok() {
        for s in &class_sites {
            eprintln!("[class-site] {} kind={} name={:?} @({:.0},{:.0}) {:.0}x{:.0}",
                s.class_api, s.kind, s.inst_name, s.aabb.x, s.aabb.y, s.aabb.w, s.aabb.h);
        }
    }

    let (ox, oy) = anchors.origin.unwrap_or((275.0, 200.0)); // SWF stage center fallback
    // GAME-space origin: the terrain MC's registration relative to the stage MC. The stage's
    // own AS3 (spawnEnemy coords) and the engine's runtime X/Y are terrain-local, so
    // FM = (game + terrain_off) × scale. Verified live on bowserscastle: terrain at
    // stageMC-local (338.85, 381.7); ledge anchors confirm pure translation (scale 1.0).
    let terrain_off = anchors.terrain.map(|(tx, ty)| (tx - ox, ty - oy));
    // Fraymakers space = SSF2 space scaled up by `size_multiplier` (the same knob the
    // character converter scales sprites by, default 1.3), so the stage matches the
    // scaled-up fighters and the art fills the FM camera the way it did in SSF2.
    let scale = crate::mappings::character_stats().scaling.size_multiplier;
    let to_fm = |r: &Rect| Rect {
        x: (r.x - ox) * scale, y: (r.y - oy) * scale, w: r.w * scale, h: r.h * scale,
    };

    // The name a platform answers to, when the stage gave it one. The walk carries the most
    // specific identity it saw down to each shape, so a clip placed under a name arrives with that
    // name -- which is exactly the handle the stage's own code reaches it by.
    let platform_name = |inst: &Instance| -> Option<String> {
        let n = inst.inst_name.clone().unwrap_or_else(|| inst.sym_name.clone());
        (!n.is_empty() && !n.contains('.') && !n.starts_with("instance")).then_some(n)
    };
    // --- platforms: `*platform*` = drop-through soft platform; otherwise any terrain /
    // collision shape (SSF2 stages name these inconsistently: TerrainMC, terrain_mc,
    // ChunkTerrain, CollisonBox [sic], ground) is a solid floor. Check `platform` first so
    // `terrainGround_platform` (both words) is classified drop-through.
    // (every shape of a terrain clip collides in the source, hidden plate or visible drawing:
    // pokemon colosseum's fighters stand on the bleacher steps of its painted stands. measured
    // live, so the visible shapes stay collision however many hidden plates sit beside them.)
    let mut platforms: Vec<Platform> = Vec::new();
    // A LEDGE BEACON THAT DECLARES A KEY (`syncPlatform`) stands at an end of whatever else
    // declares that key: a piece (gangplank galleon's platform), or an object placed under the
    // key as its instance name (sand ocean's big blue), or a piece inside an object's clip
    // (crateria's platform). the beacon's side is a ledge on that thing.
    let synced_beacons: Vec<(&str, String)> = instances.iter().filter_map(|i| {
        let local = i.sym_name.rsplit('.').next().unwrap_or(&i.sym_name);
        let role = clip_roles.get(local)?;
        let key = role.sync_platform.clone()?;
        let side = match role.kind.as_str() {
            "l_ledge" => "l",
            "r_ledge" => "r",
            _ => {
                let n = i.sym_name.to_ascii_lowercase();
                if n.contains("ledge_mc_left") { "l" } else if n.contains("ledge_mc_right") { "r" } else { return None }
            }
        };
        Some((side, key))
    }).collect();
    let sync_ledges = |key: Option<&str>| -> (bool, bool) {
        match key {
            Some(k) => (synced_beacons.iter().any(|(sd, kk)| *sd == "l" && kk == k), synced_beacons.iter().any(|(sd, kk)| *sd == "r" && kk == k)),
            None => (false, false),
        }
    };
    for inst in &instances {
        let sn = inst.sym_name.to_ascii_lowercase();
        // `back`/`fore`ground are ART, not collision — exclude them (they contain the
        // substring "ground", which would otherwise read as a floor).
        let is_art_bg = sn.contains("background") || sn.contains("foreground")
            // and exclude clearly-behind art PLANES: a background/backdrop/parallax shape named
            // with "ground"/"platform" (e.g. homeruncontest's `hrc_groundloop` field texture)
            // is scenery, not collision. (Foreground stays eligible — some stages put a
            // standable moving platform in the foreground plane.)
            || matches!(inst.plane.as_deref(), Some("background" | "backdrop" | "cambg"));
        // A boundary marker is never collision, wherever it sits. Shipped stages nest the blast
        // and camera boxes INSIDE terrain, so the name carried down to their shapes contains
        // "terrain" and they would otherwise read as one enormous floor spanning the whole blast
        // zone -- swallowing the real floor in the process, since it is the widest thing there.
        // The instance name matters as much as the linkage here: a boundary is placed by NAME
        // (`deathBoundary`) over a plain unnamed shape, so the linkage carried down is the
        // parent's, not its own.
        let inst_label = inst.inst_name.as_deref().unwrap_or_default().to_ascii_lowercase();
        if sn.contains("boundary") || inst_label.contains("boundary") { continue; }
        if std::env::var("PEPTIDE_PLAT_DEBUG").is_ok() {
            eprintln!("[plat-src] sn={sn:?} inst={inst_label:?} kind={:?} hidden={} aabb={:?}", inst.kind, inst.hidden_placed, inst.aabb);
        }
        // what the clip declares itself to be comes first: a platform placed under any name
        // (`start1`) is still the platform its class says it is
        let declared = inst.kind.as_deref().map(str::to_ascii_lowercase);
        let soft = declared.as_deref() == Some("platform")
            || (declared.is_none() && !is_art_bg && sn.contains("platform"));
        let solid = !soft && (declared.as_deref() == Some("terrain")
            || (declared.is_none() && !is_art_bg && ["terrain", "collison", "collision", "ground"].iter().any(|m| sn.contains(m))));
        if soft {
            // A PLATFORM IS ITS PLANKS. The clip's shape says where a fighter lands: each floor run
            // of its outline is a surface of its own, at its own height and span (lunar core's pit
            // bridges are four planks over a pit; boxed by their bounds they closed the pit and
            // stood a fighter over its bottom). A shape with no floor run keeps its box's top.
            let runs: Vec<Vec<(f64, f64)>> = shape_defs.get(&inst.shape_id)
                .map(|sh| terrain_outline(sh, &inst.aabb, inst.flip, ox, oy, scale)).unwrap_or_default()
                .into_iter().filter(|(t, _)| *t == "FLOOR").map(|(_, pts)| pts).collect();
            if runs.is_empty() {
                platforms.push(Platform { rect: to_fm(&inst.aabb), drop_through: !inst.no_drop, profile: None, visible: false, name: platform_name(inst), x_influence: inst.x_influence, outline: Vec::new(), placements: Vec::new(), ledges: sync_ledges(inst.sync_key.as_deref()), friction: inst.friction, is_platform: true });
            } else {
                let full = to_fm(&inst.aabb);
                for run in runs {
                    let (x0, x1) = (run.iter().map(|p| p.0).fold(f64::MAX, f64::min), run.iter().map(|p| p.0).fold(f64::MIN, f64::max));
                    let top = run.iter().map(|p| p.1).fold(f64::MAX, f64::min);
                    platforms.push(Platform {
                        rect: Rect { x: x0, y: top, w: (x1 - x0).max(1.0), h: (full.bottom() - top).max(1.0) },
                        drop_through: !inst.no_drop, profile: Some(run), visible: false, name: platform_name(inst), x_influence: inst.x_influence, outline: Vec::new(), placements: Vec::new(), ledges: sync_ledges(inst.sync_key.as_deref()), friction: inst.friction, is_platform: true,
                    });
                }
            }
        } else if solid {
            // the solid's outline, edge by edge: the whole of what the source collides with
            let outline = shape_defs.get(&inst.shape_id)
                .map(|s| terrain_outline(s, &inst.aabb, inst.flip, ox, oy, scale)).unwrap_or_default();
            if std::env::var("PEPTIDE_PLAT_DEBUG").is_ok() {
                for (ty, run) in &outline {
                    eprintln!("[edge] {ty} {} pts from ({:.0},{:.0}) to ({:.0},{:.0})", run.len(), run[0].0, run[0].1, run[run.len() - 1].0, run[run.len() - 1].1);
                }
            }
            if !outline.is_empty() {
                platforms.push(Platform { rect: to_fm(&inst.aabb), drop_through: false, profile: None, visible: false, name: platform_name(inst), x_influence: inst.x_influence, outline, placements: Vec::new(), ledges: sync_ledges(inst.sync_key.as_deref()), friction: inst.friction, is_platform: false });
                continue;
            }
            // Solid terrain is traced rather than boxed: it can be curved or sloped, and it can be
            // SEVERAL pieces in one clip, each of which is terrain a fighter stands on.
            let bands = shape_defs.get(&inst.shape_id)
                .map(|s| floor_bands(s, &inst.aabb, ox, oy, scale)).unwrap_or_default();
            if std::env::var("PEPTIDE_PLAT_DEBUG").is_ok() {
                for (pf, bottom, left, right) in &bands {
                    eprintln!("[band] x[{left:.0},{right:.0}] bottom={bottom:.0} pts={} top={:.0}",
                        pf.len(), pf.iter().map(|p| p.1).fold(f64::MAX, f64::min));
                }
            }
            if bands.len() > 1 {
                for (profile, bottom, left, right) in bands {
                    let top = profile.iter().map(|p| p.1).fold(f64::MAX, f64::min);
                    platforms.push(Platform {
                        rect: Rect { x: left, y: top, w: (right - left).max(1.0), h: (bottom - top).max(1.0) },
                        drop_through: false, profile: Some(profile), visible: false, name: platform_name(inst), x_influence: inst.x_influence, outline: Vec::new(), placements: Vec::new(), ledges: sync_ledges(inst.sync_key.as_deref()), friction: inst.friction, is_platform: false
                    });
                }
            } else {
                let profile = shape_defs.get(&inst.shape_id)
                    .and_then(|s| floor_profile(s, &inst.aabb, ox, oy, scale));
                platforms.push(Platform { rect: to_fm(&inst.aabb), drop_through: false, profile, visible: false, name: platform_name(inst), x_influence: inst.x_influence, outline: Vec::new(), placements: Vec::new(), ledges: sync_ledges(inst.sync_key.as_deref()), friction: inst.friction, is_platform: false });
            }
        }
    }
    // Dedupe near-coincident collision platforms: a moving-platform container MC (e.g.
    // `tos_movingplatform`) and the collision child inside it both match the platform/terrain
    // naming, so the same platform gets emitted twice (overlapping rects a few px apart). Drop
    // a platform when another of the same kind covers most of it; keep the larger. Distinct
    // platforms (battlefield's three soft platforms, stacked floors) don't overlap, so they stay.
    dedupe_platforms(&mut platforms);

    // --- boundaries: identified by the carried name (the boundary clip is placed with
    // a PlaceObject name like `deathBoundary`, which the walk carries down to its shape).
    let mut death_box = None;
    let mut camera_box = None;
    for inst in &instances {
        let label = inst.inst_name.as_deref().unwrap_or(&inst.sym_name).to_ascii_lowercase();
        if label.contains("deathboundary") { death_box = Some(to_fm(&inst.aabb)); }
        else if label.contains("camboundary") { camera_box = Some(to_fm(&inst.aabb)); }
    }

    // AS3-sourced hazards: the stage's own `spawnEnemy(<Class>)` calls (e.g. bowserscastle's
    // BowsersCastleLava + Thwomp). these win over the placement-tree heuristic when present.
    // Each actor's hazard clip (the SymbolClass whose linkage carries the kind keyword) supplies
    // its own collision box so the hitbox lands EXACTLY where SSF2 puts it (terrain-space).
    // the actor's clip: BY WHAT THE CLASS NAMES ITSELF first (`getOwnStats().linkage_id`), which
    // needs no vocabulary and so misses nothing, and only then by the kind keyword for a class that
    // declares no linkage.
    let clip_by_name = |want: &str| -> Option<Rect> {
        let want = want.to_ascii_lowercase();
        sym_names.iter()
            .filter(|(id, name)| name.to_ascii_lowercase() == want && sprites.contains_key(id))
            .find_map(|(id, _)| clip_attack_box(*id, &sprites, &sym_names, &shape_bounds))
    };
    // The clip an object hits with is the one the class NAMES ITSELF (`getOwnStats().linkage_id`),
    // and failing that the one carrying its class name. Both are the package stating which art
    // belongs to which class, so neither needs a vocabulary and neither can miss content whose
    // author picked words we do not know.
    let actor_box = |actor: &crate::stage_abc::SpawnedActor| -> Option<Rect> {
        // a borrowed class names its clip the same way; the clip is just in the package it came from
        if let Some(art) = actor.external_pkg.and_then(pkg_art) {
            for want in [actor.linkage_id.as_deref().unwrap_or(""), &actor.class_name] {
                if want.is_empty() { continue }
                let want = want.to_ascii_lowercase();
                if let Some(b) = art.sym_names.iter()
                    .filter(|(id, n)| n.to_ascii_lowercase() == want && art.sprites.contains_key(id))
                    .find_map(|(id, _)| clip_attack_box(*id, &art.sprites, &art.sym_names, &art.shape_bounds))
                { return Some(b) }
            }
            return None
        }
        if let Some(b) = actor.linkage_id.as_deref().and_then(clip_by_name) { return Some(b); }
        if let Some(b) = clip_by_name(&actor.class_name) { return Some(b); }
        // no hit box in it: the clip's whole extent is the body (a rotating platform, a zone
        // whose region its own code sets, an apple that falls). the object still ships and
        // runs its code; nothing is dropped for declaring no size.
        let want = actor.linkage_id.clone().unwrap_or_else(|| actor.class_name.clone()).to_ascii_lowercase();
        sym_names.iter()
            .filter(|(id, name)| name.to_ascii_lowercase() == want && sprites.contains_key(id))
            .find_map(|(id, _)| clip_extent(*id, &sprites, &shape_bounds))
    };
    // ONE path, and it has no vocabulary to miss: size, position, hit params, animations and
    // script all come from what the class declares about itself.
    // ...and an object nothing makes is not an object of this stage: a class no code spawns and
    // the tree does not place is a base other classes extend, or one the shipped stage never
    // uses (casino night's bumper base, bowser's castle's unused platform). it is not stood up
    // parked at boot to run code that was never meant to run alone.
    let code_spawns = |m: &crate::stage_abc::StageAbcModel, c: &str| -> bool {
        let clean: String = c.chars().filter(|ch| ch.is_ascii_alphanumeric()).collect();
        // a helper the stage builds carries its spawns already lowered to a fired projectile
        // (RTTFDamageZoneHandler's `spawnEnemy(DamageZone)`), named as the lowering names it
        let lowered = format!("getContent(\"{}Projectile\"", clean.to_ascii_lowercase());
        let forms = [format!("new {c}("), format!("spawnEnemy({c}"), format!("spawnEnemy(\"{c}\""), format!("spawnItem({c}"), format!("getContent(\"{id}{c}\""), format!("getContent(\"{id}{clean}\""), format!("fireProjectile(\"{c}\""), lowered];
        std::iter::once(m.reconstructed_script.as_deref().unwrap_or(""))
            .chain(m.actors.iter().filter_map(|a| a.reconstructed_script.as_deref()))
            .any(|src| forms.iter().any(|f| src.contains(f.as_str())))
    };
    let placed_classes_in_tree: std::collections::BTreeSet<&str> = class_sites.iter().map(|s| s.class_api.as_str()).collect();
    let as3_hazards: Vec<Hazard> = abc_model.as_ref().map(|m| m.actors.iter()
        .filter_map(|a| object_from_declarations(a, scale, terrain_off, death_box.as_ref(), actor_box(a).as_ref()))
        .filter(|(hz, parked)| {
            let c = hz.class_name.as_deref().unwrap_or("");
            !*parked || placed_classes_in_tree.contains(c) || code_spawns(m, c)
        })
        .map(|(hz, _)| hz)
        .collect()).unwrap_or_default();
    // PLACED objects: a class the tree names over a collision clip. The clip states the class, the
    // size and the position, and the placement name is the handle the stage's own code reaches it
    // by, so every one of those comes from the source and none of it is a category.
    let sync_key_of_clip = |clip: u16| -> Option<String> {
        fn find(tags: &[swf::Tag], sprites: &BTreeMap<u16, &Vec<swf::Tag>>, sym_names: &BTreeMap<u16, String>,
                roles: &BTreeMap<String, crate::abc_parser::ClipRole>, depth: usize) -> Option<String> {
            if depth > 3 { return None }
            for t in tags {
                let swf::Tag::PlaceObject(po) = t else { continue };
                let id = match &po.action {
                    swf::PlaceObjectAction::Place(i) | swf::PlaceObjectAction::Replace(i) => *i,
                    swf::PlaceObjectAction::Modify => continue,
                };
                if let Some(k) = sym_names.get(&id).and_then(|n| roles.get(n.rsplit('.').next().unwrap_or(n))).and_then(|r| r.sync_platform.clone()) { return Some(k) }
                if let Some(child) = sprites.get(&id) {
                    if let Some(k) = find(child, sprites, sym_names, roles, depth + 1) { return Some(k) }
                }
            }
            None
        }
        // the clip itself first (a placed collision clip declares its own key), then what it holds
        if let Some(k) = sym_names.get(&clip).and_then(|n| clip_roles.get(n.rsplit('.').next().unwrap_or(n))).and_then(|r| r.sync_platform.clone()) { return Some(k) }
        find(sprites.get(&clip)?, &sprites, &sym_names, &clip_roles, 0)
    };
    let placed_objects: Vec<Hazard> = abc_model.as_ref().map(|m| class_sites.iter().enumerate()
        .filter_map(|(si, site)| {
            let actor = m.actors.iter().find(|a| a.class_name == site.class_api);
            // a target the game builds itself runs the runtime's target, on the clip it was placed as
            let target = actor.is_none() && site.kind.eq_ignore_ascii_case("target");
            if actor.is_none() && site.bounce.is_none() && !target { return None }
            let r = to_fm(&site.aabb);
            let mut hz = Hazard {
                x: r.x + r.w / 2.0, y: r.y + r.h / 2.0, w: r.w, h: r.h,
                damage: 0.0, knockback: 0.0, angle: 0.0,
                interval: 0, active: 20, range: 0.0, period: 120, rehit: 30, kb_growth: 0.0,
                label: site.inst_name.clone().unwrap_or_else(|| if site.class_api.is_empty() { format!("bounce{si}") } else { site.class_api.clone() }),
                art: None, anims: vec![], attack_boxes: vec![], body_box: false, hitbox_dirs: vec![], anim_labels: vec![],
                faller: None, behavior: crate::abc_parser::EnemyBehavior::default(),
                reconstructed_script: None, class_name: None, max_y_speed: None,
                instance_name: site.inst_name.clone(), declared_kind: site.kind.clone(), art_clip: None, placements: Vec::new(),
                external_pkg: None, linkage_id: None, fg_anims: vec![], draws_in_front: false, platform_shapes: vec![], body_faces: vec![], child_rects: vec![], no_knockback: false, survives_bounds: false, projectiles: Default::default(), attack_stats: vec![], attack_anims: Default::default(), bounce: None, ledges: (false, false), dynamic: None, fg_name: None, shared_name: false,
            };
            if let Some(actor) = actor { apply_enemy_stats(&mut hz, actor); }
            if target {
                hz.class_name = Some("PeptideTarget".into());
                // its animations are its `stance`'s labels (`still`, `break`, ...) when it places one
                let stance = sprites.get(&site.clip).and_then(|tags| tags.iter().find_map(|t| match t {
                    swf::Tag::PlaceObject(po) if po.name.as_ref().map(|n| n.to_str_lossy(encoding_rs::WINDOWS_1252)).as_deref() == Some("stance") =>
                        match &po.action { swf::PlaceObjectAction::Place(i) | swf::PlaceObjectAction::Replace(i) => Some(*i), _ => None },
                    _ => None,
                }));
                hz.art_clip = sym_names.get(&stance.unwrap_or(site.clip)).cloned();
                // played as the clip places it: the stance runs its frames and splits at its labels
                hz.anims = extract_labeled_clip_anims(site.clip, &sprites, &sym_names, &shape_defs, &bitmaps, &planes, &[], 0.0, 0.0, &["hitBox"]);
                // the clip itself carries no label (its one section is the extractor's own name for
                // it): each animation is its stance's section, by the stance's label
                for a in &mut hz.anims {
                    if let Some((_, rest)) = a.label.split_once('_') { a.label = rest.to_string(); }
                }
            }
            hz.body_box = body_only(site.clip, &sprites, &sym_names, &shape_bounds);
            hz.bounce = site.bounce.clone();
            hz.dynamic = site.dynamic.clone();
            hz.fg_name = site.foreground.clone();
            let key = sync_key_of_clip(site.clip).or_else(|| site.inst_name.clone().filter(|n| synced_beacons.iter().any(|(_, k)| k == n)));
            if let Some(key) = key {
                hz.ledges = sync_ledges(Some(&key));
                if hz.ledges.0 || hz.ledges.1 {
                    log::info!("stage object '{}' carries ledges (left {}, right {}): beacons synced to its key '{key}'", hz.label, hz.ledges.0, hz.ledges.1);
                }
            }
            if let Some(n) = site.inst_name.as_deref() {
                hz.placements = placed_at_labels(&swf.tags, &sprites, n).into_iter()
                    .filter_map(|(l, at)| at.map(|(dx, dy)| (l, hz.x + dx * scale, hz.y + dy * scale))).collect();
            }
            // WHAT A PLACED OBJECT DRAWS IS THE CLIP IT WAS PLACED AS, LESS ITS COLLISION. The
            // class over it names no art of its own (it is the clip's class), so the clip's
            // timeline is the object's animation. A collision shape is never drawn, the same as
            // the stage's own terrain: a clip that holds its collision as a `ground` child draws
            // the rest of itself (the saucer over the plate); a clip that IS the collision draws
            // nothing of its own, and what the source shows there is placed elsewhere.
            let clip_sym = sym_names.get(&site.clip).map(|n| n.rsplit('.').next().unwrap_or(n).to_string()).unwrap_or_default();
            let starts_hidden = hidden_clips.contains(&clip_sym)
                || site.inst_name.as_deref().is_some_and(|n| hidden_clips.contains(n));
            if hz.anims.is_empty() && !starts_hidden && site.ground.is_some() {
                // the collision child is left out of the picture whoever hides it (its own frame
                // script, or the clip's over it): a plate drawn in the file as a tinted polygon is
                // still collision, and the source never shows it
                let anims = extract_labeled_clip_anims(site.clip, &sprites, &sym_names, &shape_defs, &bitmaps, &planes, &[], 0.0, 0.0, &["ground"]);
                if !anims.is_empty() {
                    hz.anims = anims;
                    hz.art_clip = sym_names.get(&site.clip).cloned();
                    log::info!("stage object '{}' draws with the clip it was placed as ('{}')",
                        hz.label, hz.art_clip.as_deref().unwrap_or("?"));
                }
            }
            // A clip declared as collision IS a body: the game builds the class over that shape, so
            // the port carries the same box unless the class makes its own. only a role the format
            // declares as collision collides as one: a clip declared as an enemy or a target is a
            // thing to hit (konoha's lanterns stood fighters on their tops)
            let surface = ["platform", "terrain", "collision"].iter().any(|k| site.kind.eq_ignore_ascii_case(k));
            if hz.behavior.self_platform.is_none() && surface {
                // the plate the clip collides with: the child it holds as its `ground` when it has
                // one, else the whole clip. relative to the object's centre, in the source's units.
                let body = site.ground.unwrap_or(site.aabb);
                let (cx, cy) = (site.aabb.x + site.aabb.w / 2.0, site.aabb.y + site.aabb.h / 2.0);
                hz.behavior.self_platform = Some((body.x - cx, body.y - cy, body.w, body.h));
                // ...and the shape itself, not the box round it: the source collides with the
                // clip's outline, so a sloped cliff is a slope to stand on, and a soft platform is
                // its floor and nothing else (a fighter jumps up through it).
                let soft = site.kind.eq_ignore_ascii_case("platform");
                let same = |a: &Rect, b: &Rect| (a.x - b.x).abs() < 0.5 && (a.y - b.y).abs() < 0.5
                    && (a.w - b.w).abs() < 0.5 && (a.h - b.h).abs() < 0.5;
                if let Some(inst) = instances.iter().find(|i| same(&i.aabb, &body)) {
                    let outline = shape_defs.get(&inst.shape_id)
                        .map(|sh| terrain_outline(sh, &inst.aabb, inst.flip, ox, oy, scale)).unwrap_or_default();
                    hz.body_faces = outline.into_iter()
                        .filter(|(ty, _)| !soft || *ty == "FLOOR")
                        .map(|(ty, run)| (ty, run.into_iter().map(|(x, y)| (x - hz.x, y - hz.y)).collect()))
                        .collect();
                }
            }
            Some(hz)
        })
        .collect()).unwrap_or_default();
    // What the package declares and this port does NOT ship, said once and only when it is true:
    // an object the tree places is shipped from its placement, so it is not a gap.
    let mut deferred: Vec<(String, String, Option<String>)> = Vec::new();
    if let Some(m) = abc_model.as_ref() {
        for a in &m.actors {
            if as3_hazards.iter().chain(placed_objects.iter())
                .any(|h| h.class_name.as_deref() == Some(a.class_name.as_str())) { continue }
            deferred.push((a.class_name.clone(), a.base.clone(), a.linkage_id.clone()));
            if a.base == crate::ssf2_objects::ITEM_BASE {
                log::warn!(
                    "stage object '{}' is an {}: deferred, because Fraymakers has no item system \
                     to port onto and one has to be built before a pickup can ship",
                    a.class_name, a.base);
            } else {
                log::warn!(
                    "stage object '{}' ({}) is declared by the package but not shipped: no code \
                     spawns it and the tree does not place it (a base class, or one this stage \
                     never uses)",
                    a.class_name, a.base);
            }
        }
    }
    // an object whose own spawn point lies outside the blast zone exists only if it survives it
    // (yoshi's fly guy starts above the top edge and flies in): the source keeps it, so this
    // engine's destroy-at-bounds is turned off for it, as for one that declares it survives
    let mut as3_hazards = as3_hazards;
    let mut placed_objects = placed_objects;
    if let Some(db) = death_box.as_ref() {
        for hz in as3_hazards.iter_mut().chain(placed_objects.iter_mut()) {
            let inside = hz.x >= db.x && hz.x <= db.x + db.w && hz.y >= db.y && hz.y <= db.y + db.h;
            if !inside { hz.survives_bounds = true; }
        }
    }
    let placed_classes: Vec<Option<String>> = placed_objects.iter().map(|p| p.class_name.clone()).collect();
    let as3_hazards: Vec<Hazard> = as3_hazards.into_iter()
        // a class the tree PLACES is emitted from its placement, so the spawned-object path does
        // not emit it a second time
        .filter(|h| !placed_classes.contains(&h.class_name))
        .chain(placed_objects)
        .collect();
    let hazards: Vec<Hazard> = if !as3_hazards.is_empty() {
        // Art is NOT borrowed by kind: the clip an object animates is found further down from the
        // labels its own class plays (`forceAttack`), which is the source stating whose art is
        // whose and needs no vocabulary.
        let bound = death_box.or(camera_box);
        // ...nor is one the stage's own code reaches by its placement name: the code puts it where
        // it goes (target test 4 parks its targets off the stage and moves each along a path)
        let stage_code = abc_model.as_ref().and_then(|m| m.reconstructed_script.as_deref()).unwrap_or("");
        let reached = |h: &Hazard| h.instance_name.as_deref().filter(|n| !n.is_empty()).is_some_and(|n| {
            let quoted = format!("\"{n}\"");
            stage_code.contains(&quoted) || regex::Regex::new(&format!(r"\.{}\b", regex::escape(n))).is_ok_and(|r| r.is_match(stage_code))
        });
        as3_hazards.into_iter()
        .filter(|h| h.reconstructed_script.is_some() || reached(h) || match &bound {
            // An object that positions ITSELF is never judged on where it was parked: a Thwomp
            // drops onto random columns, a HyruleTornado sweeps across, a rock falls from off
            // screen, and none of them operate where the converter left them. The question is
            // whether the object has code that moves it -- which the class answers -- not which
            // motion category it was sorted into. Only a genuinely placed object (a lava sheet) is
            // reachability-checked, so a phantom off-screen clip still does not ship.
            Some(b) => h.x >= b.x - 60.0 && h.x <= b.x + b.w + 60.0
                    && h.y >= b.y - 60.0 && h.y <= b.y + b.h + 60.0,
            None => true,
        }).collect()
    } else {
        // A package that declares no objects HAS no objects. Art on its own hurts nobody: a clip is
        // scenery until a class says otherwise, and the class is the only thing that can say it.
        // Guessing from the placement tree meant reading symbol names for words like "lava", which
        // invented hazards for content that never declared any and missed every one whose author
        // picked a word we did not know.
        Vec::new()
    };

    // --- hazard ANIMATIONS: recover each hazard's labelled sub-animations from its SSF2 clip so
    // the emitter can build a multi-animation Substate CGO (a Thwomp's entrance/fall/idle, not one
    // frozen pose). The source clip is the SymbolClass whose linkage matches the hazard's class
    // keyword and which carries frame labels; universal across hazards.
    // ART THE STAGE HANDS AN OBJECT. An SSF2 object does not always own its clip: the stage looks
    // one up and gives it over (`getPlatform("left_plat").setStance(getMidground().plat1)`). That
    // line is the source saying whose art is whose, so it is read rather than searched for -- the
    // label-matching search below cannot find a clip the class never names.
    let mut hazards = hazards;
    {
        let key = |h: &Hazard| (h.class_name.clone(), h.instance_name.clone().filter(|n| !n.is_empty()));
        let mut count: BTreeMap<(Option<String>, Option<String>), usize> = BTreeMap::new();
        for h in &hazards { if h.instance_name.is_some() { *count.entry(key(h)).or_default() += 1; } }
        for h in hazards.iter_mut() { h.shared_name = count.get(&key(h)).is_some_and(|&n| n > 1); }
    }
    let mut object_art: std::collections::BTreeSet<String> = Default::default();
    {
        let script = abc_model.as_ref().and_then(|m| m.reconstructed_script.as_deref()).unwrap_or("");
        // which variable holds which placed object, then which clip that variable is handed
        let holds = crate::re!(r#"(\w+)\s*=\s*(?:match|SSF2API)\.getPlatform\("([^"]+)"\)"#);
        let stance = crate::re!(r#"(\w+)\.setStance\(.*\.(\w+)\)"#);
        let by_var: BTreeMap<String, String> = holds.captures_iter(script)
            .map(|c| (c[1].to_string(), c[2].to_string())).collect();
        for c in stance.captures_iter(script) {
            let (Some(place), Some(&cid)) = (by_var.get(&c[1]), named_clips.get(&c[2])) else { continue };
            let Some(hz) = hazards.iter_mut().find(|h| h.instance_name.as_deref() == Some(place.as_str()))
                else { continue };
            let anims = extract_labeled_clip_anims(cid, &sprites, &sym_names, &shape_defs, &bitmaps, &planes, &[], 0.0, 0.0, &[]);
            if !anims.is_empty() { hz.anims = anims; }
            hz.art_clip = Some(c[2].to_string());
            object_art.insert(c[2].to_string());
            log::info!("stage object '{place}' draws with the clip the stage hands it ('{}')", &c[2]);
        }
    }
    if render_art_flag {
        // the ABC blocks, for hold-vs-loop: a sub-clip that stop()s carries a Flash timeline class.
        let hz_abcs: Vec<crate::abc_parser::AbcFile> = crate::swf_parser::parse(&swf_data)
            .map(|s| s.abc_blocks.iter().filter_map(|b| crate::abc_parser::parse(b).ok()).collect())
            .unwrap_or_default();
        if let Ok(want) = std::env::var("PEPTIDE_DUMP_CLASS") {
            for abc in &hz_abcs { crate::abc_parser::dump_class(abc, &want); }
        }
        // a hazard's art clip is ENGINE-DRIVEN, not authored scenery: a clip placed inside another
        // SPRITE's timeline (the stage's background/terrain/foreground MCs) is scenery and must not
        // be adopted (bowserscastle's Bowser spectator, placed in the background MC, shares the lava
        // class's idle/wait/lose labels and would otherwise become the "art" of an invisible-region
        // hazard, doubling him onto the lava CGO). ROOT-timeline placements don't disqualify — an
        // engine-spawned hazard parks its clip at root frame 0 (the thwomp's below-stage rest spot).
        let scenery_ids: std::collections::BTreeSet<u16> = {
            let mut s = std::collections::BTreeSet::new();
            for tags in sprites.values() {
                for t in tags.iter() {
                    if let swf::Tag::PlaceObject(po) = t {
                        if let swf::PlaceObjectAction::Place(id) | swf::PlaceObjectAction::Replace(id) = po.action { s.insert(id); }
                    }
                }
            }
            s
        };
        // One clip belongs to ONE hazard. Two classes that animate the same label set -- and a stage
        // whose hazards all play `rise`/`idle`/`die` will have several -- otherwise tie on the label
        // count and every one of them adopts the same clip: clocktown's falling rocks came out
        // wearing tingle's balloon. A clip already taken is skipped, and a clip whose own linkage
        // name echoes the class wins outright, which is the source's own way of saying whose art it
        // is.
        let fg_re = crate::re!(r#"setForeground\(\s*"([^"]+)"\s*\)"#);
        // the clip a class hands over as the shape of its own body
        let sp_re = crate::re!(r#"createSelfPlatformWithMC\(\s*(?:[\w.]+\.)?getMCByLinkageName\(\s*"([^"]+)"\s*\)"#);
        let sp_prefix_re = crate::re!(r#"createSelfPlatformWithMC\(\s*(?:[\w.]+\.)?getMCByLinkageName\(\s*"([^"]+)"\s*\+"#);
        // a rectangle a class reads off a named child of its own clip
        let child_re = crate::re!(r"\.(?:getMC|getViewRootContainer)\(\)\.(\w+)\)?\.(?:getRect|getBounds)\(");
        let mut claimed: std::collections::BTreeSet<u16> = Default::default();
        for hz in hazards.iter_mut() {
        // ART THE OBJECT DRAWS IN FRONT. A class can name a clip to be drawn ahead of the
        // fighters (`setForeground("x")`) -- the front lip of a platform, which a fighter
        // standing on it is partly behind. It is a clip like any other, named by the class
        // itself, so it is read and rendered the way its body art is. A name that resolves to
        // the art the object already draws adds nothing: the object itself moves forward.
        let fg_named: Option<String> = hz.reconstructed_script.as_deref().and_then(|src| fg_re.captures(src).map(|c| c[1].to_string()));
        if let Some(fg) = fg_named {
            let c = [fg.clone(), fg.clone()];
            let want = c[1].to_ascii_lowercase();
            let find = |names: &BTreeMap<u16, String>, sp: &BTreeMap<u16, &Vec<swf::Tag>>| {
                names.iter().find(|(id, n)| clip_name_is(n, &want) && sp.contains_key(id)).map(|(id, _)| *id)
            };
            let (mut fid, mut anims) = match hz.external_pkg.and_then(pkg_art) {
                Some(art) => (find(&art.sym_names, &art.sprites), None),
                None => {
                    let id = find(&sym_names, &sprites).or_else(|| named_clips.iter()
                        .find(|(n, id)| clip_name_is(n, &want) && sprites.contains_key(id))
                        .map(|(_, id)| *id));
                    (id, id.map(|id| extract_labeled_clip_anims(
                        id, &sprites, &sym_names, &shape_defs, &bitmaps, &planes, &hz_abcs, 0.0, 0.0, &[])))
                }
            };
            // A PICTURE THE GAME'S OWN LIBRARY HOLDS. A platform can name a clip no package ships
            // (flat zone's `targettestPlatform`, the target test platform the game draws for every
            // stage that asks); the packages the manifest loads alongside carry it, so they are
            // searched, and the clip is read out of whichever holds it as a local one would be.
            if fid.is_none() {
                for k in 0..crate::shared_packages::packages().len() {
                    let Some(art) = pkg_art(k) else { continue };
                    if let Some(id) = find(&art.sym_names, &art.sprites) {
                        let a = extract_labeled_clip_anims(id, &art.sprites, &art.sym_names, &art.shape_defs, &art.bitmaps, &planes, &art.abcs, 0.0, 0.0, &[]);
                        if !a.is_empty() {
                            log::info!("stage object '{}' draws '{}' out of shared package {k}", hz.label, &c[1]);
                            fid = Some(id); anims = Some(a);
                            break;
                        }
                    }
                }
            }
            match anims {
                Some(a) if !a.is_empty() => {
                    log::info!("stage object '{}' also draws '{}' in front of the fighters", hz.label, &c[1]);
                    hz.fg_anims = a;
                }
                _ if fid.is_some() && Some(&want) == hz.linkage_id.as_ref().map(|s| s.to_ascii_lowercase()).as_ref() =>
                    log::info!("stage object '{}' draws its own art in front of the fighters", hz.label),
                _ => log::info!("stage object '{}' draws in front of the fighters; the clip it \
                                 names ('{}') is not one this package holds on its own, so what \
                                 moves forward is the object", hz.label, &c[1]),
            }
            hz.draws_in_front = true;
        }
            // THE SHAPE IT GIVES ITSELF. A class can hand a clip over as the outline of the body it
            // stands on rather than as art, and name a different one for each form it takes. Each
            // is looked up and measured in its own space, so the body is the size the source drew.
            if std::env::var("PEPTIDE_PLAT_DEBUG").is_ok() {
                let src = hz.reconstructed_script.as_deref().unwrap_or("");
                eprintln!("[sp] {} script={} has_call={} has_mc={}", hz.label, src.len(),
                    src.contains("createSelfPlatformWithMC"), src.contains("getMCByLinkageName"));
            }
            for c in sp_re.captures_iter(hz.reconstructed_script.as_deref().unwrap_or("")) {
                let name = c[1].to_string();
                if hz.platform_shapes.iter().any(|(n, _)| *n == name) { continue }
                let want = name.to_ascii_lowercase();
                let ext = match hz.external_pkg.and_then(pkg_art) {
                    Some(art) => art.sym_names.iter()
                        .find(|(id, n)| clip_name_is(n, &want) && art.sprites.contains_key(id))
                        .and_then(|(id, _)| clip_extent(*id, &art.sprites, &art.shape_bounds)),
                    None => sym_names.iter()
                        .find(|(id, n)| clip_name_is(n, &want) && sprites.contains_key(id))
                        .map(|(id, _)| *id)
                        .or_else(|| named_clips.iter()
                            .find(|(n, id)| clip_name_is(n, &want) && sprites.contains_key(id))
                            .map(|(_, id)| *id))
                        .and_then(|id| clip_extent(id, &sprites, &shape_bounds)),
                };
                match ext {
                    Some(r) => hz.platform_shapes.push((name, (r.x, r.y, r.w, r.h))),
                    None => log::warn!("stage object '{}' shapes a body from '{name}', and no \
                                        package holds a clip by that name", hz.label),
                }
            }
            // ...and a clip named from a literal PREFIX and a value the class picks at run time
            // (sand ocean's vehicle: `"terrain_" + vehicleName`, one of eleven): every clip the
            // package holds under that prefix is measured, and the object picks by name as it runs
            for c in sp_prefix_re.captures_iter(hz.reconstructed_script.as_deref().unwrap_or("")) {
                let prefix = c[1].to_ascii_lowercase();
                let candidates: Vec<(String, u16)> = sym_names.iter()
                    .filter(|(id, n)| sprites.contains_key(id) && n.rsplit('.').next().unwrap_or(n).to_ascii_lowercase().starts_with(&prefix))
                    .map(|(id, n)| (n.rsplit('.').next().unwrap_or(n).to_string(), *id)).collect();
                if candidates.is_empty() {
                    log::warn!("stage object '{}' shapes a body from a clip named '{prefix}…', and the package holds none under that prefix", hz.label);
                }
                for (name, id) in candidates {
                    if hz.platform_shapes.iter().any(|(n, _)| *n == name) { continue }
                    if let Some(r) = clip_extent(id, &sprites, &shape_bounds) {
                        hz.platform_shapes.push((name, (r.x, r.y, r.w, r.h)));
                    }
                }
            }
            // A RECTANGLE THE CLASS READS OFF ITS OWN CLIP. `getMC().hitBox.getRect(parent)` asks
            // the display tree for a child's box; the child is a fact of the clip, so it is
            // measured here and the script gets the number.
            for c in child_re.captures_iter(hz.reconstructed_script.as_deref().unwrap_or("")) {
                let name = c[1].to_string();
                if hz.child_rects.iter().any(|(n, _)| *n == name) { continue }
                // the object's clip: the linkage its class declares, else the instance the tree
                // placed it as (a platform object is placed, not spawned), else the art it draws
                let want = hz.linkage_id.as_deref().map(|s| s.to_ascii_lowercase()).unwrap_or_default();
                let by_linkage = |names: &BTreeMap<u16, String>, sp: &BTreeMap<u16, &Vec<swf::Tag>>| {
                    (!want.is_empty()).then(|| names.iter()
                        .find(|(id, n)| clip_name_is(n, &want) && sp.contains_key(id)).map(|(id, _)| *id)).flatten()
                };
                let r = match hz.external_pkg.and_then(pkg_art) {
                    Some(art) => by_linkage(&art.sym_names, &art.sprites)
                        .and_then(|id| clip_child_rect(id, &name, &art.sprites, &art.shape_bounds)),
                    None => by_linkage(&sym_names, &sprites)
                        .or_else(|| hz.instance_name.as_deref().and_then(|n| named_clips.get(n).copied()))
                        .or_else(|| hz.art_clip.as_deref().and_then(|a| {
                            let a = a.to_ascii_lowercase();
                            sym_names.iter().find(|(id, n)| clip_name_is(n, &a) && sprites.contains_key(id)).map(|(id, _)| *id)
                        }))
                        .and_then(|id| clip_child_rect(id, &name, &sprites, &shape_bounds)),
                };
                match r {
                    Some(r) => hz.child_rects.push((name, (r.x, r.y, r.w, r.h))),
                    None => log::warn!("stage object '{}' reads the box of a child '{name}' its clip does not carry", hz.label),
                }
            }
            // a class that shapes its body from a clip HAS a body, whatever else it declared
            if hz.behavior.self_platform.is_none() {
                if let Some((_, b)) = hz.platform_shapes.first() { hz.behavior.self_platform = Some(*b); }
            }
            if !hz.platform_shapes.is_empty() {
                log::info!("stage object '{}' shapes its body from {} clip(s) it names",
                    hz.label, hz.platform_shapes.len());
            }
            // A borrowed class draws with the clip it names, out of the package it came from. There
            // is nothing to search for: the class states its linkage and the package holds it.
            if let Some((pi, art)) = hz.external_pkg.and_then(|pi| pkg_art(pi).map(|a| (pi, a))) {
                let want = hz.linkage_id.clone().unwrap_or_else(|| hz.label.clone()).to_ascii_lowercase();
                let cid = art.sym_names.iter()
                    .find(|(id, n)| n.to_ascii_lowercase() == want && art.sprites.contains_key(id))
                    .map(|(id, _)| *id);
                if let Some(cid) = cid {
                    hz.body_box = body_only(cid, &art.sprites, &art.sym_names, &art.shape_bounds);
                    hz.attack_boxes = clip_attack_boxes(cid, &art.sprites, &art.sym_names, &art.shape_bounds)
                        .into_iter()
                        .map(|r| Rect { x: r.x * scale, y: r.y * scale, w: r.w * scale, h: r.h * scale })
                        .collect();
                    let anims = extract_labeled_clip_anims(cid, &art.sprites, &art.sym_names,
                        &art.shape_defs, &art.bitmaps, &planes, &art.abcs, 0.0, 0.0, &[]);
                    if !anims.is_empty() {
                        log::info!("stage object '{}' draws with '{want}' from the shared package \
                                    ({} animations)", hz.label, anims.len());
                        hz.anims = anims;
                    }
                } else {
                    log::warn!("stage object '{}' names the clip '{want}' but package {pi} does not \
                                hold it", hz.label);
                }
                continue
            }
            // CODE-DRIVEN art clip: the hazard CLASS plays its art via `forceAttack("<label>")`, so
            // the art clip is the one whose frame labels carry those labels (the thing the script
            // actually animates) — not a library symbol grabbed by keyword. Pick the clip matching
            // the MOST of the class's forceAttack labels. Only when the class declared none (or none
            // match) do we fall back to the old linkage-keyword heuristic.
            // the class's own name, for the tie-break: a clip whose linkage echoes it is its art
            let own = hz.class_name.clone().unwrap_or_else(|| hz.label.clone()).to_ascii_lowercase();
            let anim_labels = hz.anim_labels.clone();
            if std::env::var("PEPTIDE_STAGE_DEBUG").is_ok() {
                let cands: Vec<String> = sprites.keys().filter_map(|id| {
                    let labels = crate::sprite_parser::extract_frame_labels_from_tags(sprites[id]);
                    let hits = anim_labels.iter().filter(|l| labels.iter().any(|(fl, _)| fl.eq_ignore_ascii_case(l))).count();
                    (hits > 0).then(|| format!("{id}={:?} hits={hits} scenery={} claimed={}", sym_names.get(id), scenery_ids.contains(id), claimed.contains(id)))
                }).collect();
                eprintln!("[hz-clip-search] {} labels={anim_labels:?} cands={cands:?}", hz.label);
            }
            // THE CLIP THE CLASS NAMES FOR ITSELF comes first: the source states it, so there is
            // nothing to search for, and it holds whether the same clip is also placed on the
            // timeline or already worn by another object of the same class (emerald cave's five
            // wooden panels all draw with the one `ecave_wood` their class declares)
            let declared = hz.linkage_id.as_deref().map(|s| s.to_ascii_lowercase()).filter(|w| !w.is_empty())
                .and_then(|want| sym_names.iter()
                    .find(|(id, n)| clip_name_is(n, &want) && sprites.contains_key(id)).map(|(id, _)| *id));
            // an object that ALREADY draws with a clip (the stance the stage handed it, or the
            // clip it was placed as) keeps it: central highway's platforms wear the floating
            // platform the stage hands them, not the base clip their class declares
            let declared = if hz.art_clip.is_some() && !hz.anims.is_empty() { None } else { declared };
            let keeps_its_clip = hz.art_clip.is_some() && !hz.anims.is_empty();
            let clip = declared.or_else(|| (!anim_labels.is_empty() && !keeps_its_clip).then(|| {
                sprites.keys()
                    .filter(|id| !scenery_ids.contains(id) && !claimed.contains(id))
                    .filter_map(|id| {
                        let labels = crate::sprite_parser::extract_frame_labels_from_tags(sprites[id]);
                        let hits = anim_labels.iter()
                            .filter(|l| labels.iter().any(|(fl, _)| fl.eq_ignore_ascii_case(l))).count();
                        // a name the class shares outranks a label count it merely ties on
                        let named = sym_names.get(id).is_some_and(|n| {
                            let n = n.to_ascii_lowercase();
                            own.split(|c: char| !c.is_ascii_alphanumeric())
                                .filter(|w| w.len() > 3).any(|w| n.contains(w))
                        });
                        (hits > 0).then_some((*id, hits, named))
                    })
                    .max_by_key(|(_, n, named)| (*named, *n)).map(|(id, ..)| id)
            }).flatten()).or_else(|| {
                if keeps_its_clip { return None; }
                let kw = hz.label.to_ascii_lowercase();
                sym_names.iter()
                    .filter(|(id, name)| !scenery_ids.contains(id)
                        && name.to_ascii_lowercase().contains(&kw) && sprites.contains_key(id))
                    .map(|(id, _)| (*id, crate::sprite_parser::extract_frame_labels_from_tags(sprites[id]).len()))
                    .filter(|(_, n)| *n > 0)
                    .max_by_key(|(_, n)| *n)
                    .map(|(id, _)| id)
            });
            if let Some(cid) = clip {
                claimed.insert(cid);
                // recover the clip's real SSF2 attackBox shapes (clip-local), FM-scaled to ride the
                // damaging animation as HIT_BOXes — the data-driven hit volume, same recovery the
                // lava uses. The Thwomp's `fall` yields its left + right pair.
                hz.body_box = body_only(cid, &sprites, &sym_names, &shape_bounds);
                hz.attack_boxes = clip_attack_boxes(cid, &sprites, &sym_names, &shape_bounds).into_iter()
                    .map(|r| Rect { x: r.x * scale, y: r.y * scale, w: r.w * scale, h: r.h * scale })
                    .collect();
                // origin 0,0: the frames stay in the clip's LOCAL space, so the emitter centres them
                // on the CGO (which is spawned at setX/setY) rather than the stage origin.
                let anims = extract_labeled_clip_anims(cid, &sprites, &sym_names, &shape_defs, &bitmaps, &planes, &hz_abcs, 0.0, 0.0, &[]);
                // only adopt a multi-animation CGO when it's genuinely animated (>1 label or any
                // multi-frame segment); a single static label is just the placeholder art.
                if anims.len() > 1 || anims.iter().any(|a| a.frames.len() > 1) {
                    hz.anims = anims;
                }
                if std::env::var("PEPTIDE_STAGE_DEBUG").is_ok() {
                    eprintln!("[hz-anims] {} clip={cid}: {:?}", hz.label,
                        hz.anims.iter().map(|a| (a.label.clone(), a.frames.len(), a.frames.first().map(|f| (f.w, f.h)))).collect::<Vec<_>>());
                }
            }
        }
    }

    // --- spawns: pN_Start -> entrances, pN_Spawn -> respawns (by symbol-class name).
    let mut entrances: Vec<SpawnPoint> = Vec::new();
    let mut respawns: Vec<SpawnPoint> = Vec::new();
    for inst in &instances {
        let sn = inst.sym_name.to_ascii_lowercase();
        if let Some(idx) = player_index(&sn, "_start_") {
            entrances.push(SpawnPoint { index: idx, x: (inst.cx - ox) * scale, y: (inst.cy - oy) * scale, face_left: inst.x_sign < 0.0 });
        } else if let Some(idx) = player_index(&sn, "_spawn_") {
            respawns.push(SpawnPoint { index: idx, x: (inst.cx - ox) * scale, y: (inst.cy - oy) * scale, face_left: inst.x_sign < 0.0 });
        }
    }
    entrances.sort_by_key(|s| s.index);
    respawns.sort_by_key(|s| s.index);
    // Some stages label their fighter starts with a team/role prefix rather than `pN_Start`
    // (sandbag basketball/soccer place `red1_Start`, `blue1_Start`, ...), so no `pN_Start`
    // entrance is found. A fighter still appears on the stage -- at the respawn points, which the
    // engine drops it from -- so where nothing marked a match-start, the respawns ARE the
    // entrances. A stage that has its own `pN_Start` keeps them; this only fills an empty list.
    if entrances.is_empty() && !respawns.is_empty() {
        entrances = respawns.clone();
    }

    if std::env::var("PEPTIDE_STAGE_DEBUG").is_ok() {
        for p in &platforms {
            eprintln!("[plat] top={:.0} x[{:.0},{:.0}] drop={} name={:?}",
                p.rect.top(), p.rect.left(), p.rect.right(), p.drop_through, p.name);
        }
    }
    // A SURFACE INSIDE A HAZARD IS NOT A SURFACE. SSF2 collides with a terrain clip's SHAPE; this
    // port carries its outline as a rectangle, and for a molten lake that rectangle is a lid over
    // the whole pit -- a floor at the height of the bridges, spanning the stage, that the source
    // does not have. Where a stage's own damaging object covers that rectangle, the rectangle is
    // the hazard's body rather than somewhere to stand.
    // A SURFACE THAT IS AN OBJECT'S BODY RIDES THE OBJECT. A collision clip placed under a class
    // ships as that object, with its box as the self-platform the object carries wherever its
    // code takes it. The same clip also came through the collision walk, and a static copy left
    // at the boot placement is a floor the source never has once the object moves: a fighter
    // stands in mid air on it after the platform has floated off.
    platforms.retain(|p| {
        let same = |x: f64, y: f64, w: f64, h: f64| (x - (p.rect.x + p.rect.w / 2.0)).abs() < 2.0
            && (y - (p.rect.y + p.rect.h / 2.0)).abs() < 2.0 && (w - p.rect.w).abs() < 2.0 && (h - p.rect.h).abs() < 2.0;
        let is_body = hazards.iter().filter_map(|h| h.behavior.self_platform.map(|b| (h, b))).any(|(h, (bx, by, bw, bh))| {
            if std::env::var("PEPTIDE_PLAT_DEBUG").is_ok() {
                eprintln!("[body?] surface {:?} rect=({:.1},{:.1},{:.1},{:.1}) vs {} at ({:.1},{:.1}) {}x{} body=({:.1},{:.1},{:.1},{:.1})",
                    p.name, p.rect.x, p.rect.y, p.rect.w, p.rect.h, h.label, h.x, h.y, h.w, h.h, bx * scale, by * scale, bw * scale, bh * scale);
            }
            // the clip's whole box, or the plate inside it the object collides with
            same(h.x, h.y, h.w, h.h)
                || same(h.x + (bx + bw / 2.0) * scale, h.y + (by + bh / 2.0) * scale, bw * scale, bh * scale)
                // ...or a surface as wide as the plate, at its x, inside the object's own box: the
                // plate parsed off the clip's collision child (peach's castle's rising platforms
                // held their fighters on the copy, 7 px over the deck, and never felt anyone)
                || ((h.x + (bx + bw / 2.0) * scale - (p.rect.x + p.rect.w / 2.0)).abs() < 2.0
                    && (bw * scale - p.rect.w).abs() < 2.0
                    && (p.rect.y + p.rect.h / 2.0 - h.y).abs() < h.h / 2.0 + 2.0)
                // ...or the clip's own box read as a surface, as wide as the object and centred
                // in it (desk's stapler: its art shape is 3 px shorter than the clip's whole box)
                || ((h.x - (p.rect.x + p.rect.w / 2.0)).abs() < 2.0 && (h.w - p.rect.w).abs() < 2.0
                    && (p.rect.y + p.rect.h / 2.0 - h.y).abs() < h.h / 2.0 + 2.0)
        });
        if is_body {
            log::info!("surface {:?} at y={:.0} is a placed object's body: its collision rides the object",
                p.name, p.rect.top());
        }
        !is_body
    });
    platforms.retain(|p| {
        let area = (p.rect.w * p.rect.h).max(1.0);
        let swallowed = hazards.iter().filter(|h| h.damage > 0.0).any(|h| {
            let (hl, hr) = (h.x - h.w / 2.0, h.x + h.w / 2.0);
            let (ht, hb) = (h.y - h.h / 2.0, h.y + h.h / 2.0);
            let ox = (p.rect.right().min(hr) - p.rect.left().max(hl)).max(0.0);
            let oy = (p.rect.bottom().min(hb) - p.rect.top().max(ht)).max(0.0);
            ox * oy > area * 0.4
        });
        if swallowed {
            log::info!("terrain at y={:.0} spanning {:.0} is inside a damaging hazard: the pit, not a floor",
                p.rect.top(), p.rect.w);
        }
        !swallowed
    });

    // A SURFACE THE PLANE'S TIMELINE MOVES OR DROPS BY LABEL. The source shows one terrain
    // section per phase of its stage and slides the next one in at the transition, by placing each
    // named section on some labels and not others, or elsewhere; the port's collision is built once,
    // so what each label does to a named surface is kept for the stage script to apply.
    for p in platforms.iter_mut() {
        let Some(n) = p.name.as_deref() else { continue };
        let at: Vec<(String, Option<(f64, f64)>)> = placed_at_labels(&swf.tags, &sprites, n).into_iter()
            .map(|(l, o)| (l, o.map(|(dx, dy)| (dx * scale, dy * scale)))).collect();
        let varies = at.iter().any(|(_, o)| o.is_none_or(|(dx, dy)| dx.abs() > 0.5 || dy.abs() > 0.5));
        if std::env::var("PEPTIDE_STAGE_DEBUG").is_ok() {
            eprintln!("[plat-labels] {n}: varies={varies} {at:?}");
        }
        if varies { p.placements = at; }
    }

    // --- ledges: a stage marks each standable deck's grabbable edges with a pair of beacons, and
    // each says which side it is (`type = "l_ledge"` / `"r_ledge"`) on its own frame 1. FM grabs
    // ledges at the floor's endpoints, so the first pair gives those.
    let ledge_side = |i: &Instance| -> Option<&'static str> {
        let local = i.sym_name.rsplit('.').next().unwrap_or(&i.sym_name);
        match clip_roles.get(local).map(|r| r.kind.as_str()) {
            Some("l_ledge") => Some("l"),
            Some("r_ledge") => Some("r"),
            // a stage whose beacons declare nothing is read by what it called them
            None => {
                let n = i.sym_name.to_ascii_lowercase();
                if n.contains("ledge_mc_left") { Some("l") }
                else if n.contains("ledge_mc_right") { Some("r") }
                else { None }
            }
            _ => None,
        }
    };
    let mut ledge_marks: Vec<(&'static str, f64, f64)> = instances.iter()
        .filter_map(|i| ledge_side(i).map(|s| (s, (i.cx - ox) * scale, (i.cy - oy) * scale)))
        .collect();
    ledge_marks.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    if std::env::var("PEPTIDE_STAGE_DEBUG").is_ok() {
        eprintln!("[ledge-marks] {ledge_marks:?}");
    }
    let ledges = match (ledge_marks.iter().find(|m| m.0 == "l"), ledge_marks.iter().find(|m| m.0 == "r")) {
        (Some(l), Some(r)) => Some((l.1.min(r.1), l.1.max(r.1))),
        _ => None,
    };

    // A PAIR OF LEDGE BEACONS AT ONE HEIGHT IS A DECK. The beacons say where a deck ends and how
    // high it is, which is the whole of a flat standable surface -- bowser's castle marks its two
    // bridges this way and has no collision clip for either, so read any other way the stage has
    // nothing over its lava at all. Only where the stage has no surface there already: every stage
    // marks its main floor the same way, and that one is usually real terrain as well.
    // ...and ONLY for a stage that has no standable surface at all. Every stage marks its floor's
    // edges this way, so reading a deck out of every pair would add surfaces to stages that already
    // have the real thing parsed -- inventing floors the source does not have. Where the parse
    // found nothing to stand on, the beacons are all there is.
    let has_floor = platforms.iter().any(|p| !p.drop_through);
    if !has_floor {
        let mut by_height: BTreeMap<i64, Vec<(&'static str, f64)>> = BTreeMap::new();
        for (side, x, y) in &ledge_marks {
            by_height.entry((y / 4.0).round() as i64).or_default().push((side, *x));
        }
        for (hk, mut marks) in by_height {
            let y = hk as f64 * 4.0;
            marks.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
            let mut i = 0;
            while i + 1 < marks.len() {
                let (a, b) = (marks[i], marks[i + 1]);
                i += 2;
                if !(a.0 == "l" && b.0 == "r") { continue }
                let (left, right) = (a.1, b.1);
                if right - left < 20.0 { continue }
                let covered = platforms.iter().any(|p| (p.rect.top() - y).abs() < 24.0
                    && p.rect.left() < right - 10.0 && p.rect.right() > left + 10.0);
                if covered { continue }
                log::info!("stage deck from its ledge pair: x[{left:.0},{right:.0}] top y={y:.0}");
                if std::env::var("PEPTIDE_STAGE_DEBUG").is_ok() {
                    eprintln!("[deck] x[{left:.0},{right:.0}] y={y:.0}");
                }
                platforms.push(Platform {
                    rect: Rect { x: left, y, w: right - left, h: 64.0 },
                    drop_through: false, profile: None, visible: true, name: None, x_influence: None, ledges: (false, false), friction: None,
                outline: Vec::new(), placements: Vec::new(), is_platform: false });
            }
        }
    }

    let platforms = platforms;
    // a stage whose every surface is a placed object's body has no static collision of its own
    // (steel diver's decks all ride their objects); only a stage with neither is not a stage
    if platforms.is_empty() && !hazards.iter().any(|h| h.behavior.self_platform.is_some() || !h.body_faces.is_empty()) {
        bail!("no collision geometry found in {} (not a recognised SSF2 stage?)", path.display());
    }

    // --- art: rasterize the stage's shapes, split into background / stage / foreground.
    // The PNGs stay native-resolution; placement is scaled here and the emitter renders the
    // IMAGE layers at `scale`, matching the geometry.
    // the main solid floor's top y (FM coords), so an engine-added standable bg layer (e.g.
    // bowserscastle's brick bridge) aligns its surface to where fighters actually stand.
    // hazard floors (a standable molten lake) never anchor, however wide.
    let main_floor_y = platforms.iter().filter(|p| !p.drop_through)
        .max_by(|a, b| a.rect.w.total_cmp(&b.rect.w)).map(|p| p.rect.y);
    // the timeline classes, for the frame-script jumps that chain a segmented clip's labels.
    let art_abcs: Vec<crate::abc_parser::AbcFile> = if render_art_flag {
        crate::swf_parser::parse(&swf_data)
            .map(|s| s.abc_blocks.iter().filter_map(|b| crate::abc_parser::parse(b).ok()).collect())
            .unwrap_or_default()
    } else { Vec::new() };
    if let Ok(want) = std::env::var("PEPTIDE_OWNER") {
        let want: u16 = want.parse().unwrap_or(0);
        for (sid, tags) in &sprites {
            for frame in build_frames(tags) {
                for (cid, _, name, _, depth, _, _, _, _, _) in frame {
                    if cid == want {
                        eprintln!("OWNER sprite {sid} ({:?}) places shape {want} at depth {depth} name={name:?}",
                            sym_names.get(sid));
                    }
                }
            }
        }
    }
    if std::env::var("PEPTIDE_FILLALPHA").is_ok() {
        let mut hist: std::collections::BTreeMap<u8, usize> = Default::default();
        for sh in shape_defs.values() {
            for f in &sh.styles.fill_styles {
                if let swf::FillStyle::Color(c) = f { *hist.entry(c.a).or_default() += 1; }
            }
        }
        eprintln!("FILLALPHA histogram (alpha -> count): {:?}", hist);
    }
    if let Ok(w) = std::env::var("PEPTIDE_FILLS") {
        let w: u16 = w.parse().unwrap_or(0);
        if let Some(sh) = shape_defs.get(&w) {
            eprintln!("FILLS shape {w}: {} fill styles, {} line styles",
                sh.styles.fill_styles.len(), sh.styles.line_styles.len());
            for (n, f) in sh.styles.fill_styles.iter().enumerate() {
                match f {
                    swf::FillStyle::Color(c) => eprintln!("  fill {n}: colour ({},{},{},{})", c.r, c.g, c.b, c.a),
                    swf::FillStyle::Bitmap { id, .. } => eprintln!("  fill {n}: bitmap {id}"),
                    other => eprintln!("  fill {n}: {:?}", std::mem::discriminant(other)),
                }
            }
        }
    }
    if std::env::var("PEPTIDE_REDSHAPES").is_ok() {
        for (id, sh) in &shape_defs {
            for f in &sh.styles.fill_styles {
                if let swf::FillStyle::Color(c) = f {
                    if c.r > c.g + 40 && c.r > c.b + 40 {
                        let b = &sh.shape_bounds;
                        eprintln!("REDSHAPE id={id} rgba=({},{},{},{}) box={}x{}",
                            c.r, c.g, c.b, c.a,
                            (b.x_max - b.x_min).to_pixels() as i64,
                            (b.y_max - b.y_min).to_pixels() as i64);
                    }
                }
            }
        }
    }
    if std::env::var("PEPTIDE_SYMS").is_ok() {
        let mut names: Vec<&String> = sym_names.values().collect();
        names.sort();
        for n in names { eprintln!("SYM {n}"); }
    }
    // the clips the stage's own code shows: every name it writes a visibility to. a piece that
    // starts hidden is drawn only when one of these names it (crateria's acid warning sign).
    let shown_clips: std::collections::BTreeSet<String> = {
        let src = abc_model.as_ref().and_then(|m| m.reconstructed_script.as_deref()).unwrap_or("");
        let re = crate::re!(r"([A-Za-z_]\w*)\.(?:visible\s*=|setVisible\s*\()");
        // ...and through the handle the class keeps for a plane's child: `m_cautionMC =
        // getBackground().caution`, then `m_cautionMC.visible = true`, shows `caution`
        let alias_re = crate::re!(r"([A-Za-z_]\w*)\s*=\s*(?:self\.|match\.getStage\(\)\.)?get(?:Background|Foreground|Midground)\(\)\.([A-Za-z_]\w*)");
        let aliases: BTreeMap<String, String> = alias_re.captures_iter(src)
            .map(|c| (c[1].to_string(), c[2].to_string())).collect();
        re.captures_iter(src).flat_map(|c| {
            let v = c[1].to_string();
            let mut out = vec![v.clone()];
            if let Some(n) = aliases.get(&v) { out.push(n.clone()); }
            out
        }).collect()
    };
    // the clips whose class declares a collision role: never art, whatever their timeline
    let collision_syms: std::collections::BTreeSet<String> = clip_roles.iter()
        .filter(|(_, r)| !r.kind.is_empty()).map(|(k, _)| k.clone()).collect();
    let mut art = if render_art_flag {
        render_art_layers(&swf.tags, &sprites, &sym_names, &shape_defs, &bitmaps, ox, oy, scale, &planes, main_floor_y, &art_abcs, &cam_backgrounds, &object_art, &hidden_clips, &shown_clips, &collision_syms)
    } else {
        StageArtSet::default()
    };

    // --- validate: the SSF2 source should carry the standard stage linkages, and the
    // parse should have recovered the playable bits. Non-fatal notes for the user.
    let mut warnings: Vec<String> = Vec::new();
    // a floor is a parsed surface or a placed body's deck; a stage with neither drops its fighters
    if platforms.is_empty() && !hazards.iter().any(|h| h.behavior.self_platform.is_some() || !h.body_faces.is_empty()) {
        warnings.push("no standable surface parsed (no terrain and no placed body with a deck)".into());
    }
    if death_box.is_none() { warnings.push("blast zone not parsed (no deathBoundary)".into()); }
    if camera_box.is_none() { warnings.push("camera bounds not parsed (no camBoundary)".into()); }
    if entrances.is_empty() { warnings.push("no match-start entrances parsed (pN_Start)".into()); }
    if !platforms.iter().any(|p| !p.drop_through) { warnings.push("no solid floor parsed".into()); }

    // thwomp-style target columns: the int-array literal the stage class's own spawn code steps
    // through (GAME x values) -> FM x via the terrain origin.
    let sink_columns: Vec<f64> = match terrain_off {
        Some(t) => {
            let from_code: Vec<f64> = hazards.iter()
                .filter_map(|h| h.faller.as_ref())
                .find(|f| !f.columns.is_empty())
                .map(|f| f.columns.iter().map(|x| (x + t.0) * scale).collect())
                .unwrap_or_default();
            if !from_code.is_empty() && std::env::var("PEPTIDE_STAGE_DEBUG").is_ok() {
                eprintln!("[sink-columns] from CODE: {from_code:?}");
            }
            from_code
        }
        None => Vec::new(),
    };

    // ── the hazards switch ────────────────────────────────────────────────────
    // Which parts of the stage go quiet when a match turns hazards off is decided by the STAGE, not
    // by the engine, so it is read out of the stage's own code (see `extract_hazard_gate`).
    let hazard_gate = crate::swf_parser::parse(&swf_data).ok().and_then(|sw| {
        sw.abc_blocks.iter().filter_map(|b| crate::abc_parser::parse(b).ok())
            .map(|abc| crate::abc_parser::extract_hazard_gate(&abc, &id))
            .find(|g| g.checked)
    }).unwrap_or_default();
    if std::env::var("PEPTIDE_STAGE_DEBUG").is_ok() {
        eprintln!("[hazard-gate] {hazard_gate:?}");
    }

    // ── particle weather ──────────────────────────────────────────────────────
    // Stepped out of the stage's own initialize (see `extract_stage_weather`), then the particle
    // clip it names is rendered once. It is not on the timeline and it is not a hazard, so this is
    // the only pass that sees it.
    // A stage can run more than one field at once (crateria falls rain AND acid), so each one the
    // class hands to the weather layer is built. Reading only the first left the other as a stub
    // that answers every ask with nothing, and crateria's acid never fell.
    let weather_decls: Vec<crate::abc_parser::StageWeather> = crate::swf_parser::parse(&swf_data).ok()
        .map(|sw| sw.abc_blocks.iter().filter_map(|b| crate::abc_parser::parse(b).ok())
            .map(|abc| crate::abc_parser::extract_stage_weather(&abc, &id))
            .find(|v| !v.is_empty()).unwrap_or_default())
        .unwrap_or_default();
    let weathers: Vec<Weather> = weather_decls.into_iter().filter_map(|w| {
        (|| -> Option<Weather> {
        let dbg = std::env::var("PEPTIDE_STAGE_DEBUG").is_ok();
        if dbg { eprintln!("[weather] {w:?}"); }
        let cid = sym_names.iter().find(|(_, n)| n.as_str() == w.art).map(|(cid, _)| *cid);
        if dbg { eprintln!("[weather] linkage {:?} -> char {cid:?}", w.art); }
        let cid = cid?;
        // the clip holds one shape: the particle
        let shape_id: Option<u16> = sprites.get(&cid).and_then(|tags| tags.iter().find_map(|t| match t {
            swf::Tag::PlaceObject(po) => match po.action {
                swf::PlaceObjectAction::Place(c) => Some(c),
                _ => None,
            },
            _ => None,
        }));
        let shape_id = shape_id?;
        if dbg { eprintln!("[weather] clip {cid} -> shape {shape_id:?}"); }
        let sh = shape_defs.get(&shape_id)?;
        // The particle is usually a BITMAP fill (SSF2 art nearly always is), which the vector
        // rasteriser declines by design, so try that path first and fall back to vectors.
        let tw = ((sh.shape_bounds.x_max - sh.shape_bounds.x_min).to_pixels().ceil() as u32).max(1);
        let th = ((sh.shape_bounds.y_max - sh.shape_bounds.y_min).to_pixels().ceil() as u32).max(1);
        let (rgba, w_px, h_px) = match bitmap_fill_layers(sh, &bitmaps, tw, th) {
            Some(img) => { let (w, h) = (img.width(), img.height()); (img.into_raw(), w, h) }
            None => {
                let r = crate::vector_raster::rasterize_shape(
                    &sh.shape_bounds, &sh.styles.fill_styles, &sh.styles.line_styles, &sh.shape)?;
                (r.rgba, r.width, r.height)
            }
        };
        if dbg { eprintln!("[weather] particle {w_px}x{h_px}"); }
        // A drop is drawn as a STREAK, and a streak leans the way the drop travels: it trails
        // behind. So the art already says which way the source's weather blows. If the drift says
        // the other way, the art was authored for the mirror of it (SSF2 declares a direction and
        // flips the drop to suit), and used as-is the rain falls visibly backwards -- leaning into
        // its own motion instead of away from it.
        let rgba = {
            let img = image::RgbaImage::from_raw(w_px, h_px, rgba.clone());
            let lean = img.as_ref().map(|im| {
                let mean = |ys: std::ops::Range<u32>| {
                    let px: Vec<u32> = ys.flat_map(|y| (0..w_px).filter(move |&x| im.get_pixel(x, y)[3] > 60))
                        .collect();
                    if px.is_empty() { None } else { Some(px.iter().sum::<u32>() as f64 / px.len() as f64) }
                };
                match (mean(0..h_px / 3), mean(2 * h_px / 3..h_px)) {
                    (Some(t), Some(b)) => t - b,   // >0: the top is to the RIGHT of the bottom
                    _ => 0.0,
                }
            }).unwrap_or(0.0);
            // travelling right trails its top to the LEFT, so lean and drift disagree in sign
            let wants_mirror = lean * w.motion.as_ref().map(|m| m.dx.base).unwrap_or(0.0) > 0.0 && lean.abs() > 0.5;
            match (wants_mirror, img) {
                (true, Some(im)) => {
                    log::info!("stage weather: the drop is drawn leaning against its own drift; \
                                mirrored so it trails the way it travels");
                    image::imageops::flip_horizontal(&im).into_raw()
                }
                _ => rgba,
            }
        };
        let mut png = Vec::new();
        {
            use image::ImageEncoder;
            image::codecs::png::PngEncoder::new(&mut png)
                .write_image(&rgba, w_px, h_px, image::ExtendedColorType::Rgba8).ok()?;
        }
        // the particle's own animation, when the clip plays one
        let anims = extract_labeled_clip_anims(cid, &sprites, &sym_names, &shape_defs, &bitmaps, &planes, &[], 0.0, 0.0, &[]);
        if dbg {
            let shown = sprites.get(&cid).map(|tg| tg.iter().filter(|x| matches!(x, swf::Tag::ShowFrame)).count()).unwrap_or(0);
            eprintln!("[weather] clip {cid} shows {shown} frames; animations read: {:?}",
                anims.iter().map(|a| (a.label.clone(), a.frames.len())).collect::<Vec<_>>());
        }
        let frames: Vec<StageArt> = anims.into_iter().next().map(|a| a.frames).filter(|f| f.len() > 1).unwrap_or_default();
        Some(Weather {
            frames,
            art: StageArt { png, x: 0.0, y: 0.0, w: w_px, h: h_px, hold: 1, rotation: 0.0, scale_x: 1.0, scale_y: 1.0 , alpha: 1.0 , pivot: (0.0, 0.0), tint: TINT_ID },
            count: w.count,
            width: w.width * scale,
            height: w.height * scale,
            motion: w.motion,
            field: w.field,
            linkage: w.art.clone(),
        })
        })()
    }).collect();
    for w in &weathers {
        log::info!("stage weather '{}': {} particles of {}x{} over {:.0}x{:.0}",
                   w.field, w.count, w.art.w, w.art.h, w.width, w.height);
    }

    let stage_script = abc_model.as_ref().and_then(|m| m.reconstructed_script.clone());
    // CLIPS THE STAGE'S CODE MAKES FROM THE LIBRARY AT RUN TIME. A stage that asks for a clip by
    // its linkage (`getMCByLinkageName("fairyglade_popolopoi")`), puts it in a plane and moves
    // it by hand is placing art the tree never carries; each such clip is rasterised on its
    // own timeline and shipped as a vfx the script makes on demand
    if render_art_flag {
        let re = crate::re!(r#"getMCByLinkageName\(\s*"([^"]+)"\s*\)"#);
        let mut names: std::collections::BTreeSet<String> = Default::default();
        for txt in stage_script.iter().cloned().chain(hazards.iter().filter_map(|h| h.reconstructed_script.clone())) {
            for c in re.captures_iter(&txt) { names.insert(c[1].to_string()); }
        }
        for name in names {
            let want = name.to_ascii_lowercase();
            let Some(cid) = sym_names.iter().find(|(id, n)| clip_name_is(n, &want) && sprites.contains_key(id)).map(|(id, _)| *id) else {
                log::warn!("the stage's code makes a clip '{name}' by linkage and the package carries none by that name");
                continue;
            };
            // the clip's OWN timeline, frame by frame, over as long as it and the clips inside it
            // take to come round together (a butterfly turns over its own 111 frames)
            let mut frames: Vec<StageArt> = {
                let sprite_frames: BTreeMap<u16, Vec<Vec<PlacedChild>>> = sprites.iter().map(|(id, tags)| (*id, build_frames(tags))).collect();
                let labelled = labelled_clips(&sprites);
                let no_phase: std::collections::HashMap<(u16, (i64, i64)), usize> = Default::default();
                let clip = sprite_frames.get(&cid).cloned().unwrap_or_default();
                fn gcd(a: usize, b: usize) -> usize { if b == 0 { a } else { gcd(b, a % b) } }
                let nested: Vec<usize> = clip.iter().flat_map(|f| f.iter())
                    .filter_map(|c| sprite_frames.get(&c.0).map(|fr| fr.len())).filter(|n| *n > 1).collect();
                let together = nested.iter().fold(clip.len().max(1), |acc, n| acc / gcd(acc, *n) * n);
                let span = if together <= 720 { together } else { nested.iter().copied().max().unwrap_or(1).max(clip.len()) };
                let per_frame: Vec<Vec<Instance>> = (0..span).map(|g| {
                    let mut out = Vec::new();
                    if let Some(children) = clip.get(g % clip.len().max(1)) {
                        walk_frame(children, Mat::id(), g, None, None, &planes, &sym_names, &shape_defs, &sprite_frames, &mut out, 0, (0.0, 0.0), 0, None, &no_phase, Some(cid), 1.0, crate::swf_timeline::Tint::ID, &[], &labelled, &std::collections::BTreeSet::new(), false, &std::collections::BTreeSet::new(), false, false, &[], 1);
                    }
                    out.retain(|i| shape_defs.contains_key(&i.shape_id));
                    out
                }).collect();
                let all: Vec<&Instance> = per_frame.iter().flatten().collect();
                let bounds = (!all.is_empty()).then(|| (
                    all.iter().map(|i| i.aabb.left()).fold(f64::MAX, f64::min),
                    all.iter().map(|i| i.aabb.top()).fold(f64::MAX, f64::min),
                    all.iter().map(|i| i.aabb.right()).fold(f64::MIN, f64::max),
                    all.iter().map(|i| i.aabb.bottom()).fold(f64::MIN, f64::max),
                ));
                let mut out: Vec<StageArt> = Vec::new();
                for f in per_frame.iter().filter_map(|insts| composite_layer(&insts.iter().collect::<Vec<_>>(), &shape_defs, &bitmaps, 0.0, 0.0, bounds)) {
                    match out.last_mut() {
                        Some(prev) if prev.png == f.png && prev.x == f.x && prev.y == f.y && prev.rotation == f.rotation
                            && prev.scale_x == f.scale_x && prev.scale_y == f.scale_y => prev.hold += 1,
                        _ => out.push(f),
                    }
                }
                out
            };
            if frames.is_empty() {
                log::info!("the stage's code makes a clip '{name}' by linkage and it draws nothing");
                continue;
            }
            for f in frames.iter_mut() { f.x *= scale; f.y *= scale; }
            log::info!("the stage's code makes a clip '{name}' by linkage: shipped as a vfx of {} frame(s) it makes on demand", frames.len());
            art.runtime_clips.push(BgLayer {
                named_path: Vec::new(), name: name.clone(), clip: name.clone(), frames,
                authored_alpha: 1.0, fades: false, pan: None, presence: Vec::new(),
                segments: Vec::new(), starts_hidden: false, behind_base: false,
            });
        }
    }
    // the named regions in this stage's own coordinates, like everything else it is handed
    let named_boxes: BTreeMap<String, Rect> = named_boxes.iter().map(|(n, r)| (n.clone(), to_fm(r))).collect();
    let named_regs: BTreeMap<String, (f64, f64)> = named_regs.iter().map(|(n, (x, y))| (n.clone(), ((x - ox) * scale, (y - oy) * scale))).collect();
    let terrain_labels = anchors.terrain_labels.clone();
    let terrain_frames = anchors.terrain_frames;
    // does the plane's own timeline move anything drawn? each of its frames is walked with the
    // clips inside held on their first frame, and the drawn placements compared with frame 0's
    let terrain_art_moves = anchors.terrain_id.filter(|_| terrain_frames > 1).is_some_and(|tid| {
        let sprite_frames: BTreeMap<u16, Vec<Vec<PlacedChild>>> = sprites.iter().map(|(id, tags)| (*id, build_frames(tags))).collect();
        let labelled = labelled_clips(&sprites);
        let no_phase: std::collections::HashMap<(u16, (i64, i64)), usize> = Default::default();
        let Some(frames) = sprite_frames.get(&tid) else { return false };
        let drawn = |g: usize| -> Vec<(u16, i64, i64, i64, i64)> {
            let mut out = Vec::new();
            if let Some(children) = frames.get(g) {
                walk_frame(children, Mat::id(), 0, Some("terrain"), Some("terrain"), &planes, &sym_names, &shape_defs, &sprite_frames, &mut out, 0, (0.0, 0.0), 0, None, &no_phase, Some(tid), 1.0, crate::swf_timeline::Tint::ID, &[], &labelled, &hidden_clips, false, &collision_syms, false, false, &[], 1);
            }
            let mut v: Vec<(u16, i64, i64, i64, i64)> = out.iter()
                .filter(|i| shape_defs.contains_key(&i.shape_id) && !i.hidden_by_class && !i.under_collision && art_kind(i) == ArtKind::Stage
                        && !i.sym_name.to_ascii_lowercase().contains("boundary") && !i.inst_name.as_deref().unwrap_or("").to_ascii_lowercase().contains("boundary"))
                .map(|i| (i.shape_id, i.aabb.x.round() as i64, i.aabb.y.round() as i64, i.aabb.w.round() as i64, i.aabb.h.round() as i64)).collect();
            v.sort();
            v
        };
        let first = drawn(0);
        (1..frames.len()).any(|g| drawn(g) != first)
    });
    let mut model = StageModel { source: path.to_path_buf(), origin: (ox, oy), terrain_off, terrain_labels, terrain_frames, terrain_art_moves, deferred, id, display_name, series, ssf2_music, fm_music, platforms, death_box, camera_box, entrances, respawns, ledges, hazards, sink_columns, art, warnings, scale, weathers, hazard_gate, stage_script, symbol_names: sym_names.values().cloned().collect(), hidden_clips, named_boxes, named_regs, clip_bounds, sprite_bounds,
        cam_backgrounds: cam_backgrounds.iter().map(|b| b.linkage_id.clone()).collect(), clip_frame_scripts };
    // a placed clip whose animation is entirely event-driven joins the deferred list, so the plan
    // says its animation was left out on purpose (the idle it can reach is still kept and drawn)
    for sym in std::mem::take(&mut model.art.deferred_event_clips) {
        model.deferred.push(("EventDrivenAnimation".to_string(), String::new(), Some(sym)));
    }
    extend_art_to_death_bounds(&mut model);
    Ok(model)
}

/// SSF2 fills the space between a painted hazard surface and the blast zone with runtime engine
/// effects (bowserscastle mounts an animated lava sheet + a `lavafade` gradient below the floor;
/// neither lives in the stage file), so an art layer that stops mid-hazard shows a hard edge with
/// the backdrop behind it in FM. Data-driven repair: any art layer whose bottom edge lands INSIDE
/// a static full-width hazard's declared region is that hazard's visible surface cut short —
/// extend it by repeating its bottom row (opaque down to the death bounds, then fading out), the
/// same visual role the runtime fade plays.
fn extend_art_to_death_bounds(model: &mut StageModel) {
    use image::{GenericImageView, RgbaImage};
    let Some(db) = model.death_box else { return };
    let scale = model.scale;
    let stage_w = model.platforms.iter().map(|p| p.rect.w).fold(0.0, f64::max);
    // static, stage-spanning hazard regions (the lava pool), FM coords centered on (x, y).
    let regions: Vec<Rect> = model.hazards.iter()
        .filter(|h| h.w >= stage_w)
        .map(|h| Rect { x: h.x - h.w / 2.0, y: h.y - h.h / 2.0, w: h.w, h: h.h })
        .collect();
    if regions.is_empty() { return; }
    let fade_rows = 100u32;
    let extend = |art: &mut StageArt| {
        let bottom = art.y + art.h as f64 * scale;
        let inside = regions.iter().any(|r| bottom > r.top() + 4.0 && bottom < r.bottom() - 4.0
            && art.x < r.right() && art.x + art.w as f64 * scale > r.left());
        if !inside { return; }
        let gap_raw = ((db.bottom() - bottom) / scale).ceil();
        let opaque = gap_raw.clamp(0.0, 600.0) as u32;
        let add = opaque + fade_rows;
        let Ok(img) = image::load_from_memory(&art.png) else { return };
        let (w, h) = img.dimensions();
        let img = img.to_rgba8();
        let mut out = RgbaImage::new(w, h + add);
        image::imageops::overlay(&mut out, &img, 0, 0);
        for dy in 0..add {
            let f = if dy < opaque { 1.0 } else { 1.0 - (dy - opaque + 1) as f64 / fade_rows as f64 };
            for x in 0..w {
                let mut p = *img.get_pixel(x, h - 1);
                p[3] = (p[3] as f64 * f) as u8;
                out.put_pixel(x, h + dy, p);
            }
        }
        let mut png = Vec::new();
        {
            use image::ImageEncoder;
            if image::codecs::png::PngEncoder::new(&mut png)
                .write_image(out.as_raw(), w, h + add, image::ExtendedColorType::Rgba8).is_err() { return; }
        }
        art.png = png;
        art.h += add;
    };
    for a in model.art.foreground.iter_mut() { extend(a); }
    for l in model.art.background.iter_mut() {
        for a in l.frames.iter_mut() { extend(a); }
    }
}


/// Depth layer an art instance belongs to (back-to-front). `Backdrop` is the document-root
/// `<id>_bg` scene (the far sky), which composites WITH `Parallax` (the `_cambg` layers) into
/// one camera-relative background when parallax is present, so the rays/sun draw in front of
/// the sky instead of being occluded by it.
#[derive(Clone, Copy, PartialEq)]
enum ArtKind { Backdrop, Parallax, Background, Stage, Foreground }

/// Classify a *visual* art instance by its stage PLANE — the instance name of the stage-root
/// child it descends from (the stable AS3 slot: `terrain`/`background`/`foreground`/mask, or
/// an extra root instance). Instance names generalize across stages; fla-prefixed symbol
/// names do not. `*_cambg` = a camera-background parallax layer (SSF2
/// `SSF2Stage.getCameraBackgrounds`); `foreground`/`*_fg` = in front of fighters;
/// `background` = the fixed backdrop; decorations (clouds, lensflare, …) sit at stage depth.
/// Collision (`terrain`) and masks never reach here (filtered by [`is_non_art_plane`]).
fn art_kind(inst: &Instance) -> ArtKind {
    let plane = inst.plane.as_deref().unwrap_or("").to_ascii_lowercase();
    if plane.contains("cambg") {
        ArtKind::Parallax
    } else if plane == "foreground" || plane.contains("_fg") {
        ArtKind::Foreground
    } else if plane == "backdrop" {
        ArtKind::Backdrop
    } else if plane == "background" {
        ArtKind::Background
    } else {
        ArtKind::Stage
    }
}

/// Planes that are not visual art: collision (`terrain`), the shadow/reflection masks, and
/// spawn-pose beacons (`stance`). The plane is already normalized by [`plane_tag`].
fn is_non_art_plane(plane: Option<&str>) -> bool {
    // `terrain` is NOT here. The container holds collision geometry, and the collision pieces
    // say so in their own linkage (`TerrainMC`, `platform`, `CollisonBox`, `ledge_mc`, ...) --
    // which `is_non_art` already reads. Excluding the whole plane instead threw out everything
    // else placed in it, and stages place OBJECTS there: a target test's targets, rainbow
    // route's mirrors, the palace's pillars. All of them draw, all of them move, and none of
    // them reached the art at all.
    matches!(plane, Some("mask") | Some("stance"))
}

/// The normalized stage plane an instance establishes for its subtree, from its INSTANCE
/// name (the AS3 slot vocabulary + the `_cambg` parallax / `_fg` foreground conventions),
/// or `None` to inherit the parent's plane. The one symbol-name exception: the document-
/// root backdrop container carries no instance name but the explicit linkage `<id>_bg`, so
/// an unnamed `*_bg` instance tags `background`. Instance/linkage signals generalize across
/// stages; fla-prefixed timeline symbol names do not.
fn plane_tag(inst_name: Option<&str>, sym: &str, planes: &PlaneMap) -> Option<&'static str> {
    let n = inst_name.unwrap_or("");
    // AS3-authoritative: if the stage's own `initialize` assigned this instance (or its symbol)
    // to a plane, that wins over the name heuristic (e.g. `bowsers_lightmask` -> foreground, not
    // the `mask` the heuristic would infer).
    if let Some(p) = planes.get(n).or_else(|| planes.get(sym)) {
        return Some(p.code());
    }
    let nl = n.to_ascii_lowercase();
    if nl.contains("cambg") { Some("cambg") }
    else if nl == "foreground" || nl.contains("_fg") { Some("foreground") }
    else if nl == "background" { Some("background") }
    else if nl == "terrain" { Some("terrain") }
    else if nl.contains("mask") { Some("mask") }
    else if nl == "stance" { Some("stance") }
    // the document-root backdrop container (`<id>_bg`, and `*_bg` sky props inside it) carries
    // no instance name. it sits BEHIND the stageMC `background` plane and holds the `_cambg`
    // parallax layers, so it gets its own `backdrop` tag (composited with the parallax when
    // present, otherwise drawn as the farthest fixed layer).
    else if n.is_empty() && sym.to_ascii_lowercase().ends_with("_bg") { Some("backdrop") }
    else { None }
}

// ── SSF2 stage linkage vocabulary ──
// A shipped SSF2 stage carries a consistent set of named symbols (linkage ids). Art
// classification and validation key off these names. Two families are NOT art:
//  - COLLISION masks: `*TerrainMC*`, `terrainGround_platform*`, `CollisonBox*` [sic],
//    `ledge_mc_*`. SSF2 renders them invisibly; Peptide maps them to FM collision boxes /
//    line segments. Drawing them as art double-draws a flat silhouette over the backdrop.
//  - SCAFFOLDING: `pN_Start`/`pN_Spawn` beacons, the death/camera `boundary_clip`,
//    off-screen `warningbounds_*`, the `itemGen_mc`, `shadowMask`/`reflectionMask`, the
//    `light_source_mc`, the `smashball` spawn. All non-visual.

/// Tag the clips a stage DECLARES as camera backgrounds.
///
/// SSF2 stages do not mark their parallax on the timeline: the package metadata names the clip by
/// linkage (`camera.backgrounds[].linkage_id`) and gives its pan rate, and the engine scrolls it.
/// Reading only the timeline finds a `_cambg` instance name on the handful of stages that use one
/// and nothing at all on the rest, so clocktown's whole town, clock and moon converted as a fixed
/// backdrop. The declaration is the source of truth, so take it.
fn tag_declared_cambg(planes: &mut PlaneMap, backgrounds: &[crate::abc_parser::CameraBackground]) {
    use crate::stage_abc::StagePlane;
    for bg in backgrounds {
        // A BACKGROUND THE STAGE DECLARES AS A CAMERA BACKGROUND IS ONE. Its pan rate says how much
        // of the camera's movement it takes, and ZERO is the most camera-locked of all: the piece
        // sits still in the view and covers it whatever the camera does. Read as a world-fixed
        // backdrop instead, a screen-sized piece covers only the patch of world it was placed on
        // and the rest of the view is empty -- crystal smash's whole backdrop went black that way.
        if bg.linkage_id.is_empty() { continue }
        planes.insert(bg.linkage_id.clone(), StagePlane::CameraBg);
    }
}

/// SSF2 collision-geometry linkage (invisible in SSF2; becomes FM collision, never art).
fn is_collision_linkage(label: &str) -> bool {
    let l = label.to_ascii_lowercase();
    l.contains("terrainmc") || l.contains("terrain_mc") || l.contains("platform")
        || l.contains("collison") || l.contains("collision") || l.contains("ledge_mc")
        // the region a thing HITS with is the same family as the region it collides with, and
        // just as invisible: an object's own art sits beside it under the stance, not in it
        || l.contains("hitbox") || l.contains("attackbox") || l.contains("hurtbox")
}

/// SSF2 non-visual scaffolding linkage (spawn/boundary/warning/itemgen/mask/light/smashball).
fn is_scaffold_linkage(label: &str) -> bool {
    let l = label.to_ascii_lowercase();
    ["_start_", "_spawn_", "boundary", "warningbounds", "warning", "itemgen",
     "shadowmask", "reflectionmask", "light_source", "smashball"]
        .iter().any(|m| l.contains(m))
}

/// `true` if a placed instance is collision geometry or scaffolding (not stage art).
fn is_non_art(label: &str) -> bool {
    is_collision_linkage(label) || is_scaffold_linkage(label)
}

/// A damaging stage-hazard type, recognised from an SSF2 linkage name. Carries the FM hitbox
/// defaults (damage / knockback / angle / motion) the auto-detected hazard emits — tunable per
/// The collision box placed inside a hazard clip (the SSF2 `attackBox`: a `CollisonBox` child
/// sprite whose inner shape is the box), as a CLIP-LOCAL rect — the placement matrix applied to
/// the inner shape's bounds. Searched across all the clip's frames, one level of nesting deep
/// (SSF2 hazards put the box under a `stance` wrapper). Static data, live-verified: the
/// bowserscastle lava box computed here matches the running engine's attackBox exactly.
/// EVERY distinct `CollisonBox`/`attackBox` shape under a hazard clip (clip-local rects), in
/// depth-first placement order, deduped by rounded geometry (a box repeats across frames). A clip
/// can carry MORE than one — the Thwomp's `fall` has a left + right attackBox — so the multi-box
/// hazard emit can reproduce both instead of guessing.
/// The whole extent of a clip, in its own space.
///
/// A class can hand a clip over as the SHAPE of something rather than as art: "give me a body this
/// size". That is the clip's own outline, every piece of it placed through its own matrices, not
/// the collision box a hitbox clip carries.
/// The box of a named child inside a clip, in the clip's own space: the child's extent through
/// the placement the clip gives it.
fn clip_child_rect(
    clip_id: u16, child: &str,
    sprites: &BTreeMap<u16, &Vec<swf::Tag>>,
    shape_bounds: &BTreeMap<u16, (f64, f64, f64, f64)>,
) -> Option<Rect> {
    let tags = sprites.get(&clip_id)?;
    for frame in build_frames(tags) {
        for (id, local, name, _, _, _, _, _, _, _) in &frame {
            if !name.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(child)) { continue }
            let own = if let Some(b) = shape_bounds.get(id) {
                Rect { x: b.0, y: b.1, w: b.2 - b.0, h: b.3 - b.1 }
            } else {
                clip_extent(*id, sprites, shape_bounds)?
            };
            let c = [local.apply(own.x, own.y), local.apply(own.x + own.w, own.y),
                     local.apply(own.x + own.w, own.y + own.h), local.apply(own.x, own.y + own.h)];
            let xmn = c.iter().map(|p| p.0).fold(f64::MAX, f64::min);
            let xmx = c.iter().map(|p| p.0).fold(f64::MIN, f64::max);
            let ymn = c.iter().map(|p| p.1).fold(f64::MAX, f64::min);
            let ymx = c.iter().map(|p| p.1).fold(f64::MIN, f64::max);
            return Some(Rect { x: xmn, y: ymn, w: xmx - xmn, h: ymx - ymn });
        }
    }
    None
}

fn clip_extent(
    clip_id: u16,
    sprites: &BTreeMap<u16, &Vec<swf::Tag>>,
    shape_bounds: &BTreeMap<u16, (f64, f64, f64, f64)>,
) -> Option<Rect> {
    clip_extent_memo(clip_id, sprites, shape_bounds, &mut BTreeMap::new())
}

/// A clip's extent over all its frames, in its own space: its shapes' corners and its child clips'
/// extents, each placed by its matrix. Each clip's own extent is worked out once (`memo`), since a
/// stage nests long timelines in long timelines and walking every frame of every child again for
/// every placement of it never ends.
fn clip_extent_memo(
    clip_id: u16,
    sprites: &BTreeMap<u16, &Vec<swf::Tag>>,
    shape_bounds: &BTreeMap<u16, (f64, f64, f64, f64)>,
    memo: &mut BTreeMap<u16, Option<Rect>>,
) -> Option<Rect> {
    if let Some(r) = memo.get(&clip_id) { return *r }
    // a clip that holds itself (deep or direct) counts as nothing while it is being measured
    memo.insert(clip_id, None);
    let tags = sprites.get(&clip_id)?;
    let mut pts: Vec<(f64, f64)> = Vec::new();
    let mut seen: std::collections::HashSet<(u16, [u64; 6])> = std::collections::HashSet::new();
    for frame in build_frames(tags) {
        for (id, local, _, _, _, _, _, _, _, _) in &frame {
            let key = (*id, [local.a.to_bits(), local.b.to_bits(), local.c.to_bits(), local.d.to_bits(), local.tx.to_bits(), local.ty.to_bits()]);
            if !seen.insert(key) { continue }
            let b = shape_bounds.get(id).copied()
                .or_else(|| if sprites.contains_key(id) {
                    clip_extent_memo(*id, sprites, shape_bounds, memo).map(|r| (r.x, r.y, r.x + r.w, r.y + r.h))
                } else { None });
            if let Some((x0, y0, x1, y1)) = b {
                for p in [local.apply(x0, y0), local.apply(x1, y0), local.apply(x1, y1), local.apply(x0, y1)] {
                    pts.push(p);
                }
            }
        }
    }
    let r = if pts.is_empty() { None } else {
        let xmn = pts.iter().map(|p| p.0).fold(f64::MAX, f64::min);
        let xmx = pts.iter().map(|p| p.0).fold(f64::MIN, f64::max);
        let ymn = pts.iter().map(|p| p.1).fold(f64::MAX, f64::min);
        let ymx = pts.iter().map(|p| p.1).fold(f64::MIN, f64::max);
        Some(Rect { x: xmn, y: ymn, w: xmx - xmn, h: ymx - ymn })
    };
    memo.insert(clip_id, r);
    r
}

fn clip_attack_boxes(
    clip_id: u16,
    sprites: &BTreeMap<u16, &Vec<swf::Tag>>,
    sym_names: &BTreeMap<u16, String>,
    shape_bounds: &BTreeMap<u16, (f64, f64, f64, f64)>,
) -> Vec<Rect> {
    clip_boxes(clip_id, sprites, sym_names, shape_bounds, true)
}

/// Whether a clip holds a body box of its own (a child it names `hitBox`) and nothing that hits: the
/// shape it is hit on, which deals nothing. every hazard that hurts carries an `attackBox` (clock town's meteor, bonzai
/// bill, bombfactory's blast); one with only a `hitBox` is a thing to hit (konoha's lanterns,
/// peach's castle's buttons), and a fighter dropped through the lanterns lands untouched
fn body_only(
    clip_id: u16,
    sprites: &BTreeMap<u16, &Vec<swf::Tag>>,
    sym_names: &BTreeMap<u16, String>,
    shape_bounds: &BTreeMap<u16, (f64, f64, f64, f64)>,
) -> bool {
    !clip_boxes(clip_id, sprites, sym_names, shape_bounds, false).is_empty()
        && clip_boxes(clip_id, sprites, sym_names, shape_bounds, true).is_empty()
}

/// The boxes under a clip: the ones that hit (`attack`), else its body boxes (`hitBox…`).
fn clip_boxes(
    clip_id: u16,
    sprites: &BTreeMap<u16, &Vec<swf::Tag>>,
    sym_names: &BTreeMap<u16, String>,
    shape_bounds: &BTreeMap<u16, (f64, f64, f64, f64)>,
    attack: bool,
) -> Vec<Rect> {
    #[allow(clippy::too_many_arguments)]
    fn walk(
        tags: &[swf::Tag], mat: Mat, depth: usize,
        sprites: &BTreeMap<u16, &Vec<swf::Tag>>,
        sym_names: &BTreeMap<u16, String>,
        shape_bounds: &BTreeMap<u16, (f64, f64, f64, f64)>,
        attack: bool,
        out: &mut Vec<Rect>,
    ) {
        if depth > 3 { return; }
        for frame in build_frames(tags) {
            for (id, local, name, _, _, _, _, _, _, _) in &frame {
                let m = mat.mul(local);
                let collision = sym_names.get(id).is_some_and(|s| {
                    let l = s.to_ascii_lowercase();
                    l.contains("collison") || l.contains("collision")
                });
                // a box's name is its role: `hitBox…` is the body the object is hit on; an
                // `attackBox`, a `customBox` its class tests itself or an unnamed collision clip hits
                let role = match name.as_deref() {
                    Some(n) if n.starts_with("hitBox") => Some(false),
                    _ if collision || name.as_deref() == Some("attackBox") => Some(true),
                    _ => None,
                };
                if role == Some(attack) {
                    // the box sprite's inner shape, through its own placement matrix.
                    if let Some(btags) = sprites.get(id) {
                        for inner in build_frames(btags) {
                            for (sid, smat, _, _, _, _, _, _, _, _) in &inner {
                                if let Some((x0, y0, x1, y1)) = shape_bounds.get(sid) {
                                    let mm = m.mul(smat);
                                    let c = [mm.apply(*x0, *y0), mm.apply(*x1, *y0), mm.apply(*x1, *y1), mm.apply(*x0, *y1)];
                                    let xmn = c.iter().map(|p| p.0).fold(f64::MAX, f64::min);
                                    let xmx = c.iter().map(|p| p.0).fold(f64::MIN, f64::max);
                                    let ymn = c.iter().map(|p| p.1).fold(f64::MAX, f64::min);
                                    let ymx = c.iter().map(|p| p.1).fold(f64::MIN, f64::max);
                                    out.push(Rect { x: xmn, y: ymn, w: xmx - xmn, h: ymx - ymn });
                                }
                            }
                        }
                    }
                } else if role.is_none() {
                    if let Some(child) = sprites.get(id) {
                        walk(child, m, depth + 1, sprites, sym_names, shape_bounds, attack, out);
                    }
                }
            }
        }
    }
    let mut raw = Vec::new();
    if let Some(tags) = sprites.get(&clip_id) {
        walk(tags, Mat::id(), 0, sprites, sym_names, shape_bounds, attack, &mut raw);
    }
    // dedup by rounded geometry, preserving first-seen (placement) order.
    let mut seen = std::collections::BTreeSet::new();
    let boxes: Vec<Rect> = raw.into_iter()
        .filter(|r| seen.insert((r.x.round() as i64, r.y.round() as i64, r.w.round() as i64, r.h.round() as i64)))
        .collect();
    if attack && std::env::var("PEPTIDE_BOX_DEBUG").is_ok() {
        eprintln!("[boxes] clip={clip_id}: {}", boxes.iter().map(|r| format!("({:.0},{:.0} {:.0}x{:.0})", r.x, r.y, r.w, r.h)).collect::<Vec<_>>().join(" "));
    }
    boxes
}

/// The first attackBox under a hazard clip (the lava's single wide band). See [`clip_attack_boxes`].
fn clip_attack_box(
    clip_id: u16,
    sprites: &BTreeMap<u16, &Vec<swf::Tag>>,
    sym_names: &BTreeMap<u16, String>,
    shape_bounds: &BTreeMap<u16, (f64, f64, f64, f64)>,
) -> Option<Rect> {
    clip_attack_boxes(clip_id, sprites, sym_names, shape_bounds).into_iter().next()
}

/// Build a stage object out of what its class DECLARES, with no taxonomy in the way.
///
/// The older path asks "is this one of the hazards we have a word for", which is the wrong
/// question twice over: it cannot see an object nobody named, and the word it matches carries a
/// bundle of assumed stats that the class has already stated for itself. Here the class is the
/// only source -- `getOwnStats` gives the body, `getAttackStats` gives the hit (or gives nothing,
/// and then the object simply does not hurt anyone, which is how SSF2 says harmless), the
/// `forceAttack` labels name the art, and the reconstructed script is the behaviour.
///
/// Position is deliberately NOT invented. An object whose class sets no literal coordinates is
/// placed by its own code at runtime, so it is parked above the death boundary until that code
/// runs -- out of the match rather than at the world origin, where a hitbox would sit on top of
/// the spawn points.
/// Does this symbol name the clip the source asked for?
///
/// A clip authored on a timeline exports under the file's package and with the frame it was cut at
/// stuck on the end (`stage_fla.thePiece_30`), while the code that asks for it says the plain name.
/// Both spellings are the same clip, so the package and the trailing cut are dropped before the
/// comparison.
/// A name reduced to its letters and digits, lowercased -- the two sides of a comparison are
/// spelled differently by the tree and by the package, and only what they have in common matters.
fn plain_name(t: &str) -> String {
    t.rsplit('.').next().unwrap_or(t).chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

fn clip_name_is(symbol: &str, want_lower: &str) -> bool {
    let leaf = symbol.rsplit('.').next().unwrap_or(symbol);
    let trimmed = match leaf.rfind('_') {
        Some(i) if leaf[i + 1..].chars().all(|c| c.is_ascii_digit()) && i > 0 => &leaf[..i],
        _ => leaf,
    };
    leaf.eq_ignore_ascii_case(want_lower) || trimmed.eq_ignore_ascii_case(want_lower)
}

/// The art tables of a package that is not the stage's own.
///
/// A class the stage borrows brings its art with it: same clips, same shapes, same pictures, just
/// in another file. These are the same four tables the stage builds over its own tags, so
/// everything downstream reads a borrowed clip exactly as it reads a local one.
struct PkgArt {
    sym_names: BTreeMap<u16, String>,
    shape_bounds: BTreeMap<u16, (f64, f64, f64, f64)>,
    sprites: BTreeMap<u16, &'static Vec<swf::Tag<'static>>>,
    shape_defs: BTreeMap<u16, &'static swf::Shape>,
    bitmaps: BTreeMap<u16, (u32, u32, Vec<u8>)>,
    abcs: Vec<crate::abc_parser::AbcFile>,
}

/// Read one shared package's art tables, once per package.
fn pkg_art(index: usize) -> Option<&'static PkgArt> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<BTreeMap<usize, &'static PkgArt>>> =
        std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(a) = cache.lock().ok()?.get(&index) { return Some(a) }
    let pkg = crate::shared_packages::packages().get(index)?;
    // the tags outlive the call: the package holds the movie they are read out of
    let tags: &'static [swf::Tag<'static>] = Box::leak(pkg.tags().into_boxed_slice());
    let mut art = PkgArt {
        sym_names: BTreeMap::new(), shape_bounds: BTreeMap::new(), sprites: BTreeMap::new(),
        shape_defs: BTreeMap::new(), bitmaps: BTreeMap::new(),
        abcs: crate::swf_parser::parse(&pkg.swf).map(|s| s.abc_blocks.iter()
            .filter_map(|b| crate::abc_parser::parse(b).ok()).collect()).unwrap_or_default(),
    };
    let jpeg_tables: Vec<u8> = tags.iter().find_map(|t| match t {
        swf::Tag::JpegTables(d) => Some(d.to_vec()), _ => None,
    }).unwrap_or_default();
    for tag in tags {
        match tag {
            swf::Tag::SymbolClass(links) => {
                for link in links {
                    let n = link.class_name.to_str_lossy(encoding_rs::WINDOWS_1252).to_string();
                    if !n.is_empty() { art.sym_names.insert(link.id, n); }
                }
            }
            swf::Tag::DefineShape(sh) => {
                let b = &sh.shape_bounds;
                art.shape_bounds.insert(sh.id, (
                    b.x_min.get() as f64 / 20.0, b.y_min.get() as f64 / 20.0,
                    b.x_max.get() as f64 / 20.0, b.y_max.get() as f64 / 20.0));
                art.shape_defs.insert(sh.id, sh);
            }
            swf::Tag::DefineSprite(sp) => { art.sprites.insert(sp.id, &sp.tags); }
            swf::Tag::DefineBitsLossless(b) => {
                if let Ok(rgba) = crate::image_extractor::decode_lossless(b) {
                    art.bitmaps.insert(b.id, (b.width as u32, b.height as u32, rgba));
                }
            }
            swf::Tag::DefineBitsJpeg3(j) => {
                if let Ok((w, h, rgba)) = crate::image_extractor::decode_jpeg3(j) {
                    art.bitmaps.insert(j.id, (w, h, rgba));
                }
            }
            swf::Tag::DefineBitsJpeg2 { id, jpeg_data } | swf::Tag::DefineBits { id, jpeg_data } => {
                if let Ok((w, h, rgba)) = crate::image_extractor::decode_jpeg_with_tables(&jpeg_tables, jpeg_data) {
                    art.bitmaps.insert(*id, (w, h, rgba));
                }
            }
            _ => {}
        }
    }
    for (id, sh) in placed_picture_shapes(tags, &art.bitmaps).into_iter().chain(morph_start_shapes(tags)) {
        let b = &sh.shape_bounds;
        art.shape_bounds.insert(id, (b.x_min.to_pixels(), b.y_min.to_pixels(), b.x_max.to_pixels(), b.y_max.to_pixels()));
        art.shape_defs.insert(id, sh);
    }
    let art: &'static PkgArt = Box::leak(Box::new(art));
    cache.lock().ok()?.insert(index, art);
    Some(art)
}

fn object_from_declarations(
    actor: &crate::stage_abc::SpawnedActor, scale: f64, terrain_off: Option<(f64, f64)>,
    death_box: Option<&Rect>, clip_box: Option<&Rect>,
) -> Option<(Hazard, bool)> {
    // an item runs its own class as a stage object on the runtime path (its code walks it, turns
    // it, sends it flying); the text path has nothing to run it on
    if actor.base == crate::ssf2_objects::ITEM_BASE && std::env::var_os("PEPTIDE_STAGE_TEXT").is_some() { return None; }
    let (mut w, mut h) = (
        actor.own_stats.get("width").copied().unwrap_or(0.0) * scale,
        actor.own_stats.get("height").copied().unwrap_or(0.0) * scale,
    );
    // A class can declare no body of its own and still be something a fighter runs into: a ghost
    // whose whole point is its attack (bomb factory's explosion is 0x0 and hits for 35). Where it
    // says no size, its own clip's attack box is the size -- the same box SSF2 hits with.
    if !(w > 0.0 && h > 0.0) {
        if let Some(b) = clip_box {
            w = b.w * scale;
            h = b.h * scale;
        }
    }
    // a class that states no size and draws nothing measurable still exists: its code is what
    // it is (a zone that sizes itself at runtime), so it ships at a point and runs
    if !(w > 0.0 && h > 0.0) { w = 1.0; h = 1.0; }
    // WHERE it goes, when the source says so. The class's own setX/setY literals are terrain-local
    // game coords and its clip's collision box says where the box sits inside it, so the three
    // together place the hitbox exactly as SSF2 does. Missing any of them is not a licence to
    // guess: an object placed by its own code at runtime parks above the death boundary until that
    // code runs, out of the match rather than on top of the spawn points.
    let placed = match (terrain_off, actor.x, actor.y, clip_box) {
        (Some(t), Some(ax), Some(ay), Some(b)) => Some((
            (ax + b.x + b.w / 2.0 + t.0) * scale,
            (ay + b.y + b.h / 2.0 + t.1) * scale,
        )),
        _ => None,
    };
    let (x, y) = placed.unwrap_or_else(||
        (0.0, death_box.map(|b| b.y - h * 2.0).unwrap_or(-h * 4.0)));
    let mut hz = Hazard {
        x, y, w, h,
        // no declared hit is not a missing value; it is the class saying this object is harmless
        damage: 0.0, knockback: 0.0, angle: 0.0,
        interval: 0, active: 20,
        range: 0.0, period: 120, rehit: 30, kb_growth: 0.0,
        label: actor.class_name.clone(),
        art: None, anims: vec![], attack_boxes: vec![], body_box: false, hitbox_dirs: vec![], anim_labels: vec![],
        faller: None, behavior: crate::abc_parser::EnemyBehavior::default(),
        reconstructed_script: None, class_name: None, max_y_speed: None,
        instance_name: None, declared_kind: String::new(), art_clip: None, placements: Vec::new(),
        external_pkg: actor.external_pkg, linkage_id: actor.linkage_id.clone(), fg_anims: vec![], draws_in_front: false, platform_shapes: vec![], body_faces: vec![], child_rects: vec![], no_knockback: false, survives_bounds: false, projectiles: Default::default(), attack_stats: vec![], attack_anims: Default::default(), bounce: None, ledges: (false, false), dynamic: None, fg_name: None, shared_name: false,
    };
    apply_enemy_stats(&mut hz, actor);
    Some((hz, placed.is_none()))
}

/// Override a hazard's hit params + animation set with the spawned class's OWN declarations (the
/// authoritative SSF2 source) — `getAttackStats` (SSF2's key map: `power`→baseKnockback,
/// `kbConstant`→knockbackGrowth, `direction`→angle, `damage`→damage; per-box dirs kept for the dual
/// emit) and the `forceAttack` animation labels. A hazard whose class declares neither keeps its
/// per-kind default.
fn apply_enemy_stats(hz: &mut Hazard, actor: &crate::stage_abc::SpawnedActor) {
    hz.anim_labels = actor.anim_labels.clone();
    hz.behavior = actor.behavior.clone();
    hz.reconstructed_script = actor.reconstructed_script.clone();
    hz.faller = actor.faller.clone();
    hz.max_y_speed = actor.own_stats.get("maxYSpeed").copied();
    // the source's own word on whether a hit moves it
    hz.no_knockback = actor.own_flags.get("canReceiveKnockback") == Some(&false);
    hz.survives_bounds = actor.own_flags.get("surviveDeathBounds") == Some(&true);
    hz.projectiles = actor.projectiles.clone();
    hz.class_name = Some(actor.class_name.clone());
    hz.attack_stats = actor.attack_hitboxes.clone();
    hz.attack_anims = actor.attack_by_anim.keys().cloned().collect();
    // HOW OFTEN THE HIT COMES ROUND, as the class states it: a fighter standing in a hazard is hit
    // again on the source's own cadence rather than on a number the port kept as a default. SSF2
    // counts at thirty a second and this engine at sixty.
    if let Some(r) = actor.attack_rates.values().cloned().fold(None, |a: Option<f64>, b| Some(a.map_or(b, |x| x.min(b)))) {
        if r > 0.0 { hz.rehit = (r * 2.0).round() as u32; }
    }
    if let Some(h0) = actor.attack_hitboxes.first() {
        if let Some(&d) = h0.get("damage") { hz.damage = d; }
        // `weightKB` is fixed knockback, same as a fighter's: it is the base, and it does not
        // grow with the victim's damage
        let fixed = crate::mappings::HitboxStatsMapping::is_fixed_knockback(h0);
        if let Some(&p) = h0.get("power") { hz.knockback = p; }
        if let Some(&w) = h0.get("weightKB") { hz.knockback = hz.knockback.max(w); }
        if let Some(&k) = h0.get("kbConstant") { hz.kb_growth = if fixed { 0.0 } else { k }; }
        if let Some(&dir) = h0.get("direction") { hz.angle = dir; }
        hz.hitbox_dirs = actor.attack_hitboxes.iter().filter_map(|h| h.get("direction").copied()).collect();
    }
}

/// One placed child in a sprite's timeline frame: `(character id, local matrix, name)`.
/// One placed child on a timeline frame.
/// The `Option<u16>` = the placement's `ratio` (the Flash IDE writes one on a sprite placed
/// as a GRAPHIC symbol for its frame-phase sync; a real MovieClip placement has none) — such a
/// child shows ONE fixed frame and never self-animates or runs its frame scripts.
/// (character id, local matrix, instance name, graphic frame, placement depth). The depth is the
/// SWF display-list slot — stable per child across frames, so it discriminates same-anchor
/// siblings (a clip whose children all sit at the clip origin, like a multi-emitter bubble set).
/// A placed child: `(id, matrix, name, pinned-frame, depth, blend, alpha, visible)`. `alpha` is the
/// placement's own colour-transform alpha multiplier (1.0 when it has none) — SSF2 authors a
/// translucent plane by setting alpha on the PARENT movieclip, so it has to be read from the
/// source and carried down rather than assumed. `visible` is the placement's own visibility flag:
/// a clip Flash is told to hide is still ON the display list, so a walk that ignores the flag
/// draws it -- clocktown's storm sheet is a screen-filling grey rectangle that spends most of the
/// day hidden.
type PlacedChild = (u16, Mat, Option<String>, Option<u16>, u16, Option<swf::BlendMode>, f64, bool, crate::swf_timeline::Tint, Vec<swf::Filter>);

/// instance/symbol name -> AS3-assigned render plane (from `stage_abc::extract_stage`). empty when
/// the stage has no parseable SSF2Stage subclass, in which case `plane_tag` falls back to heuristics.
type PlaneMap = BTreeMap<String, crate::stage_abc::StagePlane>;

/// Build a sprite/root timeline in this walk's tuple form, from the SHARED timeline reader
/// ([`crate::swf_timeline`]). The reader is what knows Flash's display-list semantics -- including
/// the parts a placement carries beyond its matrix -- so both content types get them from one
/// place; this only reshapes the result into the tuple the walk below threads around.
fn build_frames(tags: &[swf::Tag]) -> Vec<Vec<PlacedChild>> {
    crate::swf_timeline::frames(tags).into_iter().map(|frame| {
        frame.into_iter().map(|p| {
            let m = Mat { a: p.matrix.a, b: p.matrix.b, c: p.matrix.c, d: p.matrix.d,
                          tx: p.matrix.tx, ty: p.matrix.ty };
            (p.char_id, m, p.name, p.ratio, p.depth, p.blend, p.alpha, p.visible,
             p.tint.unwrap_or(crate::swf_timeline::Tint::ID), p.filters)
        }).collect()
    }).collect()
}

fn collect_clip_positions(
    children: &[PlacedChild], parent: Mat,
    sprite_frames: &BTreeMap<u16, Vec<Vec<PlacedChild>>>, out: &mut Vec<(u16, (i64, i64))>,
) {
    if out.len() > 100_000 { return; }
    for (id, local, _, _, _, _, _, _, _, _) in children {
        let world = parent.mul(local);
        if let Some(frames) = sprite_frames.get(id) {
            out.push((*id, (world.tx.round() as i64, world.ty.round() as i64)));
            if let Some(f) = frames.first() {
                collect_clip_positions(f, world, sprite_frames, out);
            }
        }
    }
}

/// An ancestor's filter chain and this placement's own, in the order Flash would run them: the
/// inner object is filtered first, then what it sits inside.
fn chain(outer: &[swf::Filter], own: &[swf::Filter]) -> Vec<swf::Filter> {
    if outer.is_empty() { return own.to_vec(); }
    if own.is_empty() { return outer.to_vec(); }
    own.iter().chain(outer.iter()).cloned().collect()
}

/// shape instances with world AABBs. Mirrors [`walk`] but frame-aware.
#[allow(clippy::too_many_arguments)]
fn walk_frame(
    children: &[PlacedChild], parent: Mat, global_frame: usize, carried_sym: Option<&str>,
    plane: Option<&str>, planes: &PlaneMap,
    sym_names: &BTreeMap<u16, String>, shape_defs: &BTreeMap<u16, &swf::Shape>,
    sprite_frames: &BTreeMap<u16, Vec<Vec<PlacedChild>>>, out: &mut Vec<Instance>, rec: usize,
    inst_anchor: (f64, f64), inst_path: u64, blend: Option<swf::BlendMode>,
    phase_rank: &std::collections::HashMap<(u16, (i64, i64)), usize>, inst_clip: Option<u16>,
    alpha: f64, ptint_parent: crate::swf_timeline::Tint, filters: &[swf::Filter],
    labelled: &std::collections::BTreeSet<u16>,
    hidden_syms: &std::collections::BTreeSet<String>, hidden: bool,
    collision_syms: &std::collections::BTreeSet<String>, collision: bool, only_ground: bool,
    // the INSTANCE names over this subtree, outermost first: every one is a name the stage's
    // code can reach a piece by (`getBackground().movingplatLsprite.sprite`)
    named: &[String],
    chain_len: usize,
) {
    if rec > 8 { return; }
    for (id, local, name, pinned, pdepth, pblend, palpha, pvisible, ptint, pfilters) in children {
        let local_sym = sym_names.get(id).map(|n| n.rsplit('.').next().unwrap_or(n).to_string()).unwrap_or_default();
        // A CLIP THAT HOLDS ITS COLLISION AS A `ground` CHILD draws the rest of itself: the plate
        // is the collision and everything beside it is art (pokemon colosseum's bug terrain runs
        // 132 frames of it beside its plate; as a collision clip it was never drawn)
        let my_collision = if only_ground { name.as_deref() == Some("ground") } else { collision || collision_syms.contains(&local_sym) };
        // a clip whose own class hides it on its first frame takes its whole subtree with it
        let my_hidden = hidden || sym_names.get(id)
            .map(|n| n.rsplit('.').next().unwrap_or(n))
            .is_some_and(|l| hidden_syms.contains(l))
            // or the clip over it hides this child by name on its first frame
            || match (carried_sym, name) {
                (Some(p), Some(n)) => hidden_syms.contains(&format!("{}::{n}", p.rsplit('.').next().unwrap_or(p))),
                _ => false,
            };
        // hidden by the timeline: still on the display list, drawn by nothing. Its whole subtree
        // goes with it.
        if !*pvisible { continue; }
        let world = parent.mul(local);
        let sym = sym_names.get(id).cloned().unwrap_or_default();
        // an instance establishes a plane for its subtree: the stage's AS3 plane map (authoritative)
        // first, then the instance-name heuristic, otherwise it inherits the parent's plane.
        let my_plane = plane_tag(name.as_deref(), &sym, planes).or(plane);
        if let Some(s) = shape_defs.get(id) {
            let b = &s.shape_bounds;
            let (x0, y0, x1, y1) = (b.x_min.get() as f64/20.0, b.y_min.get() as f64/20.0, b.x_max.get() as f64/20.0, b.y_max.get() as f64/20.0);
            let cs = [world.apply(x0, y0), world.apply(x1, y0), world.apply(x1, y1), world.apply(x0, y1)];
            let xmn = cs.iter().map(|c| c.0).fold(f64::MAX, f64::min);
            let xmx = cs.iter().map(|c| c.0).fold(f64::MIN, f64::max);
            let ymn = cs.iter().map(|c| c.1).fold(f64::MAX, f64::min);
            let ymx = cs.iter().map(|c| c.1).fold(f64::MIN, f64::max);
            out.push(Instance {
                kind: None, no_drop: false, x_influence: None, sync_key: None, friction: None, // art path: what a clip is drawn as, never what it collides as
                shape_id: *id, inst_name: name.clone(), sym_name: carried_sym.unwrap_or("").to_string(),
                plane: my_plane.map(str::to_string),
                aabb: Rect { x: xmn, y: ymn, w: xmx - xmn, h: ymx - ymn },
                cx: world.tx, cy: world.ty, x_sign: world.x_sign(), flip: world.flips(),
                inst_anchor,
                inst_path,
                place: world.placement(),
                leaf_depth: *pdepth,
                blend: pblend.or(blend),
                inst_clip,
                chain_len,
                hidden_by_class: my_hidden,
                hidden_placed: false,
                under_collision: my_collision,
                alpha: alpha * palpha,
                tint: ptint.under(&ptint_parent),
                filters: chain(filters, pfilters),
                named_path: named.to_vec(),
            });
        }
        if let Some(frames) = sprite_frames.get(id) {
            let next = name.as_deref().or(if sym.is_empty() { carried_sym } else { Some(&sym) }).map(|s| s.to_string());
            if let Ok(pat) = std::env::var("PEPTIDE_WALK_TRACE") {
                if next.as_deref().is_some_and(|n| n.to_ascii_lowercase().contains(&pat.to_ascii_lowercase())) {
                    eprintln!("[walk-trace] clip id={id} pdepth={pdepth} path={inst_path} name={:?} sym={:?} plane={:?} frames={} at=({:.0},{:.0})",
                        name, sym, my_plane, frames.len(), world.tx, world.ty);
                }
            }
            // EVERY movieclip placement is its own instance: its world position becomes the anchor
            // for its subtree, so each repeated clip (the 16 ember emitters, the 8 wall torches --
            // even when unnamed) splits into its OWN object at its own position, instead of being
            // merged into the parent element. each then animates + stabilizes independently (a torch
            // can't drift left/right because the whole row's average shifts -- it's its own clip).
            let child_anchor = (world.tx, world.ty);
            let child_path = inst_path.wrapping_mul(31).wrapping_add(*pdepth as u64 + 1);
            // even desync: each clip's start frame from its position-rank via the golden ratio (a
            // low-discrepancy sequence), so a row of repeated clips is spread EVENLY across its loop
            // and their per-frame leans cancel (the row reads as still). see `phase_rank` above.
            let rank = phase_rank.get(&(*id, (world.tx.round() as i64, world.ty.round() as i64))).copied().unwrap_or(0);
            let phase = ((rank as f64 * 0.618_033_988_75).fract() * frames.len() as f64) as usize;
            // A `ratio` on a placement does NOT pin a sprite to one frame. Ratio is only
            // meaningful for MorphShape characters; Flash ignores it on a sprite, so the sprite
            // runs its own timeline as normal. Treating it as a pin froze animations the source
            // plays: bowserscastle's thwomp places its `idle` sub-clip with ratio=2, and SSF2
            // runs that clip through all 52 of its frames (read live: `thwomp_idle_41 frame=51/52
            // 'loop'`) while we emitted a single static frame — so the pose never changed during
            // the landing wait the way the original does.
            let f = &frames[(global_frame + phase) % frames.len()];
            let _ = pinned;
            // the clip id tracks `next`: whichever clip supplies the carried symbol name is the one
            // whose timeline (and so whose frame labels) the grouped element corresponds to.
            // ...unless the clip above is a STATE MACHINE and this one is a still picture: what a
            // labelled clip places per section is that clip's segments, whatever the piece is
            // called. castle siege's sky is a named child of a backdrop that holds a section and
            // plays a transition when the stage says, and owned by its own name it ran as one
            // loop through every section. a child with a timeline of its own (the clouds, 720
            // frames of sliding) keeps its own clock, as it does in Flash while the parent holds.
            let still = sprite_frames.get(id).is_none_or(|f| f.len() <= 1);
            let child_clip = if name.is_some() || !sym.is_empty() {
                if still && inst_clip.is_some_and(|c| labelled.contains(&c)) && !labelled.contains(id) { inst_clip } else { Some(*id) }
            } else { inst_clip };
            let child_only_ground = my_collision && f.iter().any(|(_, _, n, ..)| n.as_deref() == Some("ground"));
            let child_named: Vec<String> = match name { Some(n) => named.iter().cloned().chain(std::iter::once(n.clone())).collect(), None => named.to_vec() };
            let child_chain = if frames.len() > 1 {
                fn gcd(a: usize, b: usize) -> usize { if b == 0 { a } else { gcd(b, a % b) } }
                (chain_len / gcd(chain_len, frames.len()) * frames.len()).min(1 << 20)
            } else { chain_len };
            walk_frame(f, world, global_frame, next.as_deref(), my_plane, planes, sym_names, shape_defs, sprite_frames, out, rec + 1, child_anchor, child_path, pblend.or(blend), phase_rank, child_clip, alpha * palpha, ptint.under(&ptint_parent), &chain(filters, pfilters), labelled, hidden_syms, my_hidden, collision_syms, my_collision, child_only_ground, &child_named, child_chain);
        }
    }
}

/// Hard-light `mask` into `target` where the two overlap, in place.
///
/// WHY THIS IS BAKED AND NOT SET AT RUNTIME
/// ---------------------------------------
/// Fraymakers exposes no blend modes to a mod. The renderer underneath has them (`blendMode` is a
/// real field on every Heaps display object), but the only handle a stage Script gets is a
/// `ContainerApi`, whose entire surface is addChild/addChildAt/removeChild + state — and the whole
/// scripting API's visual vocabulary is scale, alpha and visibility. There is no setBlendMode,
/// setTint, setFilter or setShader anywhere in it. Reaching the field needs engine injection,
/// which is fine for Peptide's own testing and useless for a converted stage, because that stage
/// runs on the STOCK engine.
///
/// So the blend has to happen before the art ships. For STATIC art that is exact rather than an
/// approximation: hard-light of a fixed mask over fixed pixels is a constant image, so it can be
/// computed once and emitted as the art itself.
///
/// THE LIMIT, which no amount of baking fixes: only what is baked gets tinted. SSF2 draws its mask
/// over the LIVE scene, so fighters, hazards and anything else moving underneath are tinted there
/// and cannot be here. That divergence is permanent until the engine exposes a blend mode.
///
/// Both images are in the same scaled stage space (1 unit = 1 pixel), so their overlap is just the
/// difference of their top-left corners.
/// Mean per-pixel spread between a mask's colour channels, over its visible pixels. 0 means the
/// mask is greyscale, which is what decides whether HardLight can be reproduced by ALPHA LAYERS
/// instead of baked: the black/white decomposition needs one alpha per pixel, so it is exact for a
/// greyscale mask and cannot express a mask whose channels disagree.
/// Pad every frame of an element to one canvas size, keeping each frame's content CENTRED.
///
/// A frame drawn as its own crop carries its position in `x`/`y`; growing the canvas around the
/// content and moving the origin back by half the growth leaves the content exactly where it was,
/// and leaves a rotation about the image centre unchanged. Frames that already share the size are
/// untouched.
fn pad_frames_to_one_canvas(frames: &mut [StageArt], scale: f64) {
    let (Some(w), Some(h)) = (frames.iter().map(|f| f.w).max(), frames.iter().map(|f| f.h).max()) else { return };
    if frames.iter().all(|f| f.w == w && f.h == h) { return; }
    for f in frames.iter_mut() {
        if f.w == w && f.h == h { continue; }
        let Ok(img) = image::load_from_memory(&f.png) else { continue };
        let src = img.to_rgba8();
        let (dx, dy) = ((w - f.w) / 2, (h - f.h) / 2);
        let mut canvas = image::RgbaImage::new(w, h);
        image::imageops::replace(&mut canvas, &src, dx as i64, dy as i64);
        let mut png = Vec::new();
        {
            use image::ImageEncoder;
            if image::codecs::png::PngEncoder::new(&mut png)
                .write_image(canvas.as_raw(), w, h, image::ExtendedColorType::Rgba8).is_err() { continue; }
        }
        f.png = png;
        f.x -= dx as f64 * scale;
        f.y -= dy as f64 * scale;
        f.w = w;
        f.h = h;
    }
}

fn mask_chroma(mask: &StageArt) -> f64 {
    let Ok(img) = image::load_from_memory(&mask.png).map(|i| i.to_rgba8()) else { return -1.0 };
    let (mut sum, mut n) = (0.0f64, 0u64);
    for p in img.pixels() {
        if p[3] < 8 { continue; }
        let (r, g, b) = (p[0] as f64, p[1] as f64, p[2] as f64);
        let mx = r.max(g).max(b);
        let mn = r.min(g).min(b);
        sum += mx - mn;
        n += 1;
    }
    if n == 0 { 0.0 } else { sum / n as f64 }
}

/// Turn a blended placement into an ordinary ALPHA overlay that reproduces the blend over the art
/// we already know, while staying translucent so whatever MOVES underneath is tinted too.
///
/// Baking the blend into the art is exact, but it can only ever tint what was baked: SSF2 draws
/// its mask over the LIVE scene, so the fighters running around under it are tinted there and
/// would not be here. That is the difference people actually see.
///
/// A layer can't reproduce a chromatic HardLight exactly. Normal blending gives
/// `out_c = b_c(1-a) + C_c*a`, so the slope `(1-a)` is SHARED across channels and only the offset
/// is per-channel; stacking more alpha layers just multiplies the slopes and they stay shared.
/// HardLight with a coloured mask needs a PER-CHANNEL slope, which no arrangement of alpha layers
/// has. (For a greyscale mask it is exactly expressible: black at `1-2t` is the multiply half,
/// white at `2t-1` is the screen half — hence `mask_chroma`, which decides which case a stage is.)
///
/// So instead of approximating the blend in the abstract, solve it against the base we KNOW:
/// per pixel, find `(a, C)` with `b(1-a) + C*a = HardLight(b, t)`, taking the SMALLEST alpha that
/// keeps `C` in gamut. Over the static stage art that is exact. Over a fighter it is a tint rather
/// than a true hard-light, but it is present, positioned and moves with them — which baking can
/// never manage.
fn fit_blend_overlay(mask: &StageArt, base: &StageArt) -> Option<StageArt> {
    let m = image::load_from_memory(&mask.png).ok()?.to_rgba8();
    let b_img = image::load_from_memory(&base.png).ok()?.to_rgba8();
    let hl = |b: f64, t: f64| if t <= 0.5 { 2.0 * b * t } else { 1.0 - 2.0 * (1.0 - b) * (1.0 - t) };
    let (dx, dy) = ((mask.x - base.x).round() as i64, (mask.y - base.y).round() as i64);
    let (bw, bh) = (b_img.width() as i64, b_img.height() as i64);
    let mut out = image::RgbaImage::new(m.width(), m.height());
    for (mx, my, sp) in m.enumerate_pixels() {
        let sa = sp[3] as f64 / 255.0;
        if sa <= 0.0 { continue; }
        let (cx, cy) = (dx + mx as i64, dy + my as i64);
        // base pixel under this mask pixel; outside the art (or transparent there) we have nothing
        // to solve against, so carry the mask through at its own alpha and let it tint whatever
        // passes underneath at runtime.
        let base_px = if cx >= 0 && cy >= 0 && cx < bw && cy < bh {
            let p = b_img.get_pixel(cx as u32, cy as u32);
            (p[3] > 0).then(|| [p[0] as f64 / 255.0, p[1] as f64 / 255.0, p[2] as f64 / 255.0])
        } else { None };
        let t = [sp[0] as f64 / 255.0, sp[1] as f64 / 255.0, sp[2] as f64 / 255.0];
        let Some(bp) = base_px else {
            out.put_pixel(mx, my, image::Rgba([sp[0], sp[1], sp[2], sp[3]]));
            continue;
        };
        // Carry the MASK'S OWN alpha and let the colour be the hard-lit result. Substituting
        // a = sa and C = hl(b,t) into `b(1-a) + C*a` returns the target exactly, so the static art
        // is as accurate as baking — and because the alpha is the source's rather than the minimum
        // that stays in gamut, everything else underneath gets the coverage SSF2 intended.
        //
        // Solving for the SMALLEST in-gamut alpha also reproduces the static art exactly, but it
        // is the wrong objective: it maximises transparency, so moving objects barely change.
        // Measured on bowserscastle, it shifted Mario's overalls R/B 0.315 -> 0.327 where SSF2
        // reads 0.426 — a tenth of the intended strength.
        let mut px = [0u8; 4];
        for c in 0..3 {
            px[c] = (hl(bp[c], t[c]).clamp(0.0, 1.0) * 255.0).round() as u8;
        }
        px[3] = sp[3];
        out.put_pixel(mx, my, image::Rgba(px));
    }
    let mut png = Vec::new();
    use image::ImageEncoder;
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(out.as_raw(), out.width(), out.height(), image::ExtendedColorType::Rgba8).ok()?;
    Some(StageArt { png, x: mask.x, y: mask.y, w: mask.w, h: mask.h, hold: 1, rotation: 0.0, scale_x: 1.0, scale_y: 1.0 , alpha: 1.0 , pivot: (0.0, 0.0), tint: TINT_ID })
}

/// Paint a shape's bitmap FILL into a `tw` x `th` tile by mapping through the fill's own matrix,
/// which is what says where the bitmap sits and whether it repeats.
///
/// Stretching the bitmap across the shape's box instead (the obvious shortcut) is only ever right
/// when the bitmap happens to be one image sized to that box. It is wrong for a fill placed at an
/// offset, and badly wrong for a small REPEATING tile: the art lands outside the path, and once the
/// path mask is applied almost nothing survives. bowserscastle showed both failure modes — its
/// front chandeliers are a 29x38 tile spread over 807x113 (the sheet came out blank, so two of the
/// four chandeliers were missing), and its foreground lava is a 1305x124 texture stretched over a
/// 1305x284 box, which showed the wrong slice of lava and left a hard horizontal seam where the
/// sheet met the stage.
///
/// Maps destination pixel -> shape-local twips -> (inverse fill matrix) -> bitmap twips -> pixel.
/// Paint EVERY bitmap fill in a shape, each through its own matrix and clipped to its own
/// sub-paths, and composite them into one `tw` x `th` tile.
///
/// A shape is not limited to one bitmap fill. `StyleChange` records can carry `new_styles`, a fresh
/// fill table, so a single shape can place many different bitmaps at many different matrices --
/// bowserscastle's sconce sheet is one shape spanning 807px with 6 sconce regions, each with its
/// own fill. Treating the shape as "one bitmap, one matrix" paints only whichever region that fill
/// happens to cover and silently drops the rest, which is why the stage was missing sconces.
///
/// Each fill is isolated by rasterising the shape with THAT fill white and every other fill fully
/// transparent, giving a per-fill mask; the bitmap is then mapped through its own matrix and
/// multiplied by that mask.
pub(crate) fn bitmap_fill_layers(
    shape: &swf::Shape, bitmaps: &BTreeMap<u16, (u32, u32, Vec<u8>)>, tw: u32, th: u32,
) -> Option<image::RgbaImage> {
    let clear = || swf::FillStyle::Color(swf::Color { r: 0, g: 0, b: 0, a: 0 });
    let white = || swf::FillStyle::Color(swf::Color { r: 255, g: 255, b: 255, a: 255 });
    // every bitmap fill, tagged by which table (0 = the shape's own, then each new_styles in order)
    let mut wanted: Vec<(usize, usize, u16, swf::Matrix, bool)> = Vec::new();
    let mut scan = |t: usize, fs: &[swf::FillStyle]| {
        for (i, f) in fs.iter().enumerate() {
            if let swf::FillStyle::Bitmap { id, matrix, is_repeating, .. } = f {
                if bitmaps.contains_key(id) { wanted.push((t, i, *id, *matrix, *is_repeating)); }
            }
        }
    };
    scan(0, &shape.styles.fill_styles);
    let mut t = 0usize;
    for r in &shape.shape {
        if let swf::ShapeRecord::StyleChange(sc) = r {
            if let Some(ns) = &sc.new_styles { t += 1; scan(t, &ns.fill_styles); }
        }
    }
    if wanted.is_empty() { return None; }

    let mut out = image::RgbaImage::new(tw, th);
    for (table, idx, bid, mat, repeating) in &wanted {
        let pick = |t: usize, fs: &[swf::FillStyle]| -> Vec<swf::FillStyle> {
            fs.iter().enumerate()
                .map(|(i, _)| if t == *table && i == *idx { white() } else { clear() })
                .collect()
        };
        let top = pick(0, &shape.styles.fill_styles);
        let mut t2 = 0usize;
        let recs: Vec<swf::ShapeRecord> = shape.shape.iter().map(|r| match r {
            swf::ShapeRecord::StyleChange(sc) if sc.new_styles.is_some() => {
                let mut sc = sc.clone();
                t2 += 1;
                if let Some(ns) = sc.new_styles.as_mut() { ns.fill_styles = pick(t2, &ns.fill_styles); }
                swf::ShapeRecord::StyleChange(sc)
            }
            other => other.clone(),
        }).collect();
        let Some(mask) = crate::vector_raster::rasterize_shape(
            &shape.shape_bounds, &top, &shape.styles.line_styles, &recs,
        ).and_then(|r| image::RgbaImage::from_raw(r.width, r.height, r.rgba)) else { continue };
        let mask = resize_to(&mask, tw, th);
        let (w, h, rgba) = &bitmaps[bid];
        let Some(src) = image::RgbaImage::from_raw(*w, *h, rgba.clone()) else { continue };
        let art = bitmap_fill_tile(&src, &shape.shape_bounds, mat, *repeating, tw, th)
            .unwrap_or_else(|| resize_to(&src, tw, th));
        for ((o, a), m) in out.pixels_mut().zip(art.pixels()).zip(mask.pixels()) {
            let sa = (a[3] as u32 * m[3] as u32 / 255) as u8;
            if sa == 0 { continue; }
            // src-over onto whatever earlier fills already painted
            let da = o[3] as u32;
            let na = sa as u32 + da * (255 - sa as u32) / 255;
            if na == 0 { continue; }
            for c in 0..3 {
                o[c] = (((a[c] as u32 * sa as u32) + (o[c] as u32 * da * (255 - sa as u32) / 255)) / na) as u8;
            }
            o[3] = na as u8;
        }
    }
    Some(out)
}

fn bitmap_fill_tile(
    bmp: &image::RgbaImage, bounds: &swf::Rectangle<swf::Twips>, m: &swf::Matrix, repeating: bool,
    tw: u32, th: u32,
) -> Option<image::RgbaImage> {
    let (a, b, c, d) = (m.a.to_f64(), m.b.to_f64(), m.c.to_f64(), m.d.to_f64());
    let det = a * d - b * c;
    if !det.is_finite() || det.abs() < 1e-9 { return None; }
    let (tx, ty) = (m.tx.get() as f64, m.ty.get() as f64);
    let (x0, y0) = (bounds.x_min.get() as f64, bounds.y_min.get() as f64);
    let (bw, bh) = (bounds.width().get() as f64, bounds.height().get() as f64);
    if bw <= 0.0 || bh <= 0.0 { return None; }
    let (iw, ih) = (bmp.width() as i64, bmp.height() as i64);
    if iw == 0 || ih == 0 { return None; }
    if std::env::var("PEPTIDE_FILL_DEBUG").is_ok() {
        let corner = |lx: f64, ly: f64| {
            let (px, py) = (lx - tx, ly - ty);
            ((d * px - c * py) / det, (a * py - b * px) / det)
        };
        eprintln!("[fill] bmp {iw}x{ih} bounds twips ({x0},{y0}) {bw}x{bh} repeating={repeating} \
            mat a={a:.4} b={b:.4} c={c:.4} d={d:.4} tx={tx} ty={ty} -> topleft={:?} botright={:?}",
            corner(x0, y0), corner(x0 + bw, y0 + bh));
    }
    let mut out = image::RgbaImage::new(tw, th);
    for dy in 0..th {
        for dx in 0..tw {
            // destination pixel -> shape-local twips (pixel centres)
            let lx = x0 + (dx as f64 + 0.5) / tw as f64 * bw;
            let ly = y0 + (dy as f64 + 0.5) / th as f64 * bh;
            // inverse fill matrix -> bitmap twips -> bitmap pixels (20 twips per pixel)
            let (px, py) = (lx - tx, ly - ty);
            // The inverse already lands in bitmap PIXELS: SSF2's fill matrices are
            // `a = d = 20, tx/ty = the shape's origin`, i.e. they map bitmap pixels onto shape
            // twips. Dividing by 20 again (treating the source as twips) shrinks the sampled
            // region by 20x and reads a tiny corner of the image. Checked against the lava sheet,
            // where the shape is 26100 twips wide and the bitmap 1305px: 26100/20 = 1305, exactly
            // 1:1, and its 124px band lines up with the path mask.
            let sx = (d * px - c * py) / det;
            let sy = (a * py - b * px) / det;
            let (mut ix, mut iy) = (sx.floor() as i64, sy.floor() as i64);
            if repeating {
                ix = ix.rem_euclid(iw);
                iy = iy.rem_euclid(ih);
            } else if ix < 0 || iy < 0 || ix >= iw || iy >= ih {
                continue;                       // outside a clamped fill: leave it transparent
            }
            out.put_pixel(dx, dy, *bmp.get_pixel(ix as u32, iy as u32));
        }
    }
    Some(out)
}

fn bake_hardlight(target: &mut StageArt, mask: &StageArt) {
    let (Ok(mut base), Ok(top)) = (
        image::load_from_memory(&target.png).map(|i| i.to_rgba8()),
        image::load_from_memory(&mask.png).map(|i| i.to_rgba8()),
    ) else { return };
    let hl = |b: f64, t: f64| if t <= 0.5 { 2.0 * b * t } else { 1.0 - 2.0 * (1.0 - b) * (1.0 - t) };
    let (dx, dy) = ((mask.x - target.x).round() as i64, (mask.y - target.y).round() as i64);
    let (bw, bh) = (base.width() as i64, base.height() as i64);
    for (mx, my, sp) in top.enumerate_pixels() {
        let (cx, cy) = (dx + mx as i64, dy + my as i64);
        if cx < 0 || cy < 0 || cx >= bw || cy >= bh { continue; }
        let sa = sp[3] as f64 / 255.0;
        if sa <= 0.0 { continue; }
        let dp = base.get_pixel_mut(cx as u32, cy as u32);
        // only tint pixels the target actually painted: hard-lighting transparent pixels would
        // stamp the mask onto empty space, which is the artefact the old canvas-local emulation
        // produced whenever it ran over an unpainted region.
        let da = dp[3] as f64 / 255.0;
        if da <= 0.0 { continue; }
        for c in 0..3 {
            let b = dp[c] as f64 / 255.0;
            let t = sp[c] as f64 / 255.0;
            dp[c] = ((b * (1.0 - sa) + hl(b, t) * sa).clamp(0.0, 1.0) * 255.0) as u8;
        }
    }
    let mut png = Vec::new();
    use image::ImageEncoder;
    if image::codecs::png::PngEncoder::new(&mut png)
        .write_image(base.as_raw(), base.width(), base.height(), image::ExtendedColorType::Rgba8)
        .is_ok()
    {
        target.png = png;
    }
}

/// Split a backdrop element's rasterized frames into its clip's label-delimited segments, keeping
/// only the ones SSF2 can actually reach during play.
///
/// A clip carrying more than one frame label is a state machine, not a loop: its frame scripts jump
/// between labels, so playing the timeline end to end shows animations the source never shows
/// (bowserscastle's crowd has a `lose` reaction that nothing on the timeline reaches). Returns empty
/// for the ordinary single-label element, which is the common case.
#[allow(clippy::too_many_arguments)]
fn clip_segments(
    inst: Option<&&Instance>,
    per_frame: &[StageArt],
    sprites: &BTreeMap<u16, &Vec<swf::Tag>>,
    sym_names: &BTreeMap<u16, String>,
    sprite_frames: &BTreeMap<u16, Vec<Vec<PlacedChild>>>,
    abcs: &[crate::abc_parser::AbcFile],
    phase_rank: &std::collections::HashMap<(u16, (i64, i64)), usize>,
    deferred_event_clips: &std::cell::RefCell<Vec<String>>,
) -> Vec<BgSegment> {
    use crate::abc_parser::LabelJump;
    let Some(cid) = inst.and_then(|i| i.inst_clip) else { return Vec::new() };
    let Some(tags) = sprites.get(&cid) else { return Vec::new() };
    let mut labels = crate::sprite_parser::extract_frame_labels_from_tags(tags);
    // a clip with no labels but a script that stops it is a held picture, not a loop: jungle
    // island's digit clips stop on their second frame and are moved by frame number from
    // outside, which nothing here does, so they hold their first digit
    if labels.is_empty() {
        let stops = sym_names.get(&cid)
            .map(|class| abcs.iter().flat_map(|a| crate::abc_parser::extract_label_jumps(a, class)).any(|(_, j)| matches!(j, LabelJump::Stop)))
            .unwrap_or(false);
        if stops { labels.push(("_0".to_string(), 0)); }
    }
    if labels.len() < 2 && !(labels.len() == 1 && labels[0].0 == "_0") { return Vec::new(); }
    let n = sprite_frames.get(&cid).map(|f| f.len()).unwrap_or(0);
    // a lap of the clip has to fit in the sample window, or there is nothing to slice
    if n < 2 || per_frame.len() < n { return Vec::new(); }
    // the walk starts each clip at a phase offset (repeated placements are spread across their loop
    // so their per-frame leans cancel), so sample i holds clip frame (i + phase) % n. Undo it, or
    // the segment boundaries land in the wrong place.
    let anchor = inst.map(|i| i.inst_anchor).unwrap_or((0.0, 0.0));
    let rank = phase_rank.get(&(cid, (anchor.0.round() as i64, anchor.1.round() as i64))).copied().unwrap_or(0);
    let phase = ((rank as f64 * 0.618_033_988_75).fract() * n as f64) as usize % n;
    let at = |f: usize| per_frame[(f + n - phase) % n].clone();

    let jumps: Vec<(u32, LabelJump)> = sym_names.get(&cid)
        .map(|class| abcs.iter().flat_map(|a| crate::abc_parser::extract_label_jumps(a, class)).collect())
        .unwrap_or_default();
    let mut segs: Vec<BgSegment> = Vec::new();
    for (li, (label, start)) in labels.iter().enumerate() {
        let s = *start as usize;
        let e = labels.get(li + 1).map(|(_, f)| *f as usize).unwrap_or(n);
        if e <= s || e > n { continue; }
        // AS3 frame numbers are 1-based, so the script on the segment's last frame is `frame{e}`.
        // THE FIRST SCRIPTED FRAME INSIDE THE SEGMENT IS WHERE IT ENDS: a plane that holds a
        // section stops on the section's first frame, and what the source shows of that section
        // is that one frame, not the run up to the next label. castle siege's planes stop on
        // `top` and wait for the stage's code to play the transition.
        let (end, next) = jumps.iter()
            .find(|(f, _)| (*f as usize) > s && (*f as usize) <= e)
            .map(|(f, j)| (*f as usize, Some(j.clone())))
            .unwrap_or((e, None));
        // a move by frame lands in whichever label's span holds that frame
        let next = match next {
            Some(LabelJump::ToFrame(f)) => {
                let f0 = (f as usize).saturating_sub(1);
                labels.iter().rev().find(|(_, st)| (*st as usize) <= f0)
                    .map(|(l, _)| LabelJump::Always(l.clone()))
            }
            other => other,
        };
        segs.push(BgSegment { label: label.clone(), frames: (s..end).map(at).collect(), next });
    }
    // A MOVE THAT HALTS ON ITS TARGET halts the target on its first frame, whatever that frame's
    // own script says: castle siege's sky is sent to `transition1_loop` and stops there, and
    // played through, the loop it was parked on ran as a loop. The target keeps one frame.
    let held: Vec<String> = segs.iter()
        .filter_map(|s| match &s.next { Some(LabelJump::Hold(t)) => Some(t.clone()), _ => None })
        .collect();
    for s in &mut segs {
        if held.contains(&s.label) && !matches!(s.next, Some(LabelJump::Stop)) {
            s.frames.truncate(1);
            s.next = Some(LabelJump::Stop);
        }
    }
    if segs.len() < 2 { return Vec::new(); }

    // Reachability from the clip's entry (its first label), and from every label the package's
    // own code names: a segment the timeline never runs into is still played when the stage asks
    // for it by name (castle siege's transitions), and only a segment nobody reaches is dead --
    // it exists for an event nothing fires.
    let named: std::collections::BTreeSet<&str> = abcs.iter()
        .flat_map(|a| a.strings.iter().map(|s| &**s)).collect();
    // a name the code BUILDS counts as named: `kind + "_transform"` (both halves in the pool),
    // `"min_" + n` (a pool string and a number), and a frame-number jump lands on the label
    // that frame got (`_N`), so those are live too
    let names_it = |label: &str| -> bool {
        if named.contains(label) { return true; }
        if label.strip_prefix('_').is_some_and(|d| !d.is_empty() && d.chars().all(|c| c.is_ascii_digit())) { return true; }
        (1..label.len()).any(|k| {
            let (a, b) = (&label[..k], &label[k..]);
            named.contains(a) && (named.contains(b) || b.chars().all(|c| c.is_ascii_digit()))
        })
    };
    let mut live = vec![false; segs.len()];
    let mut stack: Vec<usize> = std::iter::once(0usize)
        .chain(segs.iter().enumerate().filter(|(_, s)| names_it(s.label.as_str())).map(|(i, _)| i))
        .collect();
    while let Some(i) = stack.pop() {
        if live[i] { continue; }
        live[i] = true;
        let mut targets: Vec<usize> = Vec::new();
        match &segs[i].next {
            Some(LabelJump::Always(t)) | Some(LabelJump::Hold(t)) => targets.extend(segs.iter().position(|s| &s.label == t)),
            // a chance jump can also NOT fire, in which case playback runs on into the next segment
            Some(LabelJump::Chance { target, .. }) => {
                targets.extend(segs.iter().position(|s| &s.label == target));
                targets.push(i + 1);
            }
            // the playhead halts here: nothing follows on its own
            Some(LabelJump::Stop) => {}
            // a move by frame is resolved to its label as the segments are built
            Some(LabelJump::ToFrame(_)) => {}
            // nothing scripted: run on, wrapping at the end of the timeline
            None => targets.push(if i + 1 < segs.len() { i + 1 } else { 0 }),
        }
        stack.extend(targets.into_iter().filter(|&t| t < segs.len()));
    }
    let dropped: Vec<&str> = segs.iter().zip(&live).filter(|(_, &l)| !l).map(|(s, _)| s.label.as_str()).collect();
    let live_count = live.iter().filter(|&&l| l).count();
    if !dropped.is_empty() {
        log::warn!("stage art: clip '{}' segments {:?} are unreachable from its timeline \
            (event-driven); not ported",
            sym_names.get(&cid).map(String::as_str).unwrap_or("?"), dropped);
        // the only thing the timeline can reach is the idle base and every animated segment is
        // event-driven: the port keeps the idle and drops the rest, so the clip's full source
        // length is not reproduced passively. record the clip's symbol name (what the live tree
        // reports as its class) so a cross-engine check reads it as deferred, not missing.
        if live_count <= 1 {
            if let Some(sym) = sym_names.get(&cid) {
                deferred_event_clips.borrow_mut().push(sym.clone());
            }
        }
    }
    let mut out: Vec<BgSegment> = segs.into_iter().zip(live).filter(|(_, l)| *l).map(|(s, _)| s).collect();
    // a jump to a segment that got pruned would dangle; treat it as "run on" instead.
    let names: Vec<String> = out.iter().map(|s| s.label.clone()).collect();
    for s in &mut out {
        let target = match &s.next {
            Some(LabelJump::Always(t)) | Some(LabelJump::Hold(t)) | Some(LabelJump::Chance { target: t, .. }) => Some(t.clone()),
            Some(LabelJump::Stop) | Some(LabelJump::ToFrame(_)) | None => None,
        };
        if target.is_some_and(|t| !names.contains(&t)) { s.next = None; }
    }
    out
}

/// Rasterize the stage's art into background / stage / foreground layers. The stage layer
/// is frame-aware: if the source has animated clips it renders multiple frames (the
/// emitter loops them). Background/foreground are single-frame (frame 0).
#[allow(clippy::too_many_arguments)]
fn render_art_layers(
    root_tags: &[swf::Tag],
    sprites: &BTreeMap<u16, &Vec<swf::Tag>>,
    sym_names: &BTreeMap<u16, String>,
    shape_defs: &BTreeMap<u16, &swf::Shape>,
    bitmaps: &BTreeMap<u16, (u32, u32, Vec<u8>)>,
    ox: f64, oy: f64, scale: f64,
    planes: &PlaneMap,
    surface_y_fm: Option<f64>,
    abcs: &[crate::abc_parser::AbcFile],
    cam_backgrounds: &[crate::abc_parser::CameraBackground],
    // clips an OBJECT draws with: the stage hands one to a class it built, and from then on that
    // clip is the object's -- painted into the stage as well, it would be drawn twice, and the copy
    // in the painting would stay put while the object it belongs to fades and moves.
    object_art: &std::collections::BTreeSet<String>,
    // clips that hide themselves on their first frame, and the clips the stage's code shows. a
    // piece that starts hidden and that no code ever shows is not art: the source never draws
    // it. castle siege's terrain sections are such pieces, and painted in they sat over the castle.
    hidden_clips: &std::collections::BTreeSet<String>,
    shown_clips: &std::collections::BTreeSet<String>,
    collision_syms: &std::collections::BTreeSet<String>,
) -> StageArtSet {
    let root_m = root_mat(root_tags, sym_names);
    // symbol names of clips whose animated segments are all event-driven (filled as the segment
    // reader prunes each clip; see `clip_segments`)
    let deferred_event_clips: std::cell::RefCell<Vec<String>> = std::cell::RefCell::new(Vec::new());
    // per-sprite + root frame timelines.
    let mut sprite_frames: BTreeMap<u16, Vec<Vec<PlacedChild>>> = BTreeMap::new();
    for (id, tags) in sprites { sprite_frames.insert(*id, build_frames(tags)); }
    let root_frames = build_frames(root_tags);
    let labelled = labelled_clips(sprites);

    if let Ok(want) = std::env::var("PEPTIDE_CLIP_ANIMS") {
        for (id, name) in sym_names.iter() {
            if !name.to_ascii_lowercase().contains(&want.to_ascii_lowercase()) { continue; }
            if let Some(tags) = sprites.get(id) {
                let labels = crate::sprite_parser::extract_frame_labels_from_tags(tags);
                let frames = build_frames(tags);
                eprintln!("[clip] {name} id={id}: {} frames; labels={labels:?}", frames.len());
            }
        }
    }

    // full animation length = longest timeline. Port EVERY source frame 1:1 (no sampling): each
    // SSF2 frame becomes one keyframe of length 1, and the shared 30fps->60fps doubling
    // (entity_gen::double_keyframe_lengths, exactly the character port's tool) stretches each to
    // length 2. Identical consecutive frames RLE into one longer hold. Shorter-looping elements
    // repeat within the global timeline, matching SSF2's per-clip frame advancement.
    let full_len = sprite_frames.values().map(|f| f.len())
        .chain(std::iter::once(root_frames.len())).max().unwrap_or(1);
    let n_samples = full_len;
    let sample_frame = |i: usize| -> usize { i };

    // The ROOT timeline usually has just ONE content frame (the backdrop + stageMC placements);
    // many SSF2 stages reserve frame 0 as an empty preloader and put the content on frame 1.
    // Sampling the root by `g % root_len` can then miss the content frame entirely (when the
    // global stride and root length are both even, every sample lands on the empty frame 0),
    // dropping the whole backdrop. So anchor the root at its RICHEST frame (the content layout);
    // the sprites INSIDE still cycle by the global frame, so animation is preserved.
    let root_idx = root_frames.iter().enumerate().max_by_key(|(_, f)| f.len()).map(|(i, _)| i).unwrap_or(0);

    // EVEN DESYNC: SSF2 starts each repeated decorative clip (the row of wall torches/lamps, the
    // embers) on a different frame so the row doesn't flicker in unison. rank every clip by world
    // position, then give each a low-discrepancy (golden-ratio) start phase. ranking + golden ratio
    // spreads a row of repeated clips EVENLY across its loop, so their per-frame leans cancel and
    // the row reads as still -- unlike syncing them (they all lean together) or a linear position
    // offset (adjacent clips one frame apart = a gradient that travels across the row as a wave).
    // ONLY a repeated clip is spread out. Starting every clip on a different frame desynchronises
    // the pieces of one thing as well: clocktown's clock is a stack of clips at different spots, so
    // each got a different start and the ring lagged the face it belongs to -- they turn together in
    // the source and could never line up here. A clip placed once keeps frame 0 and stays in step
    // with everything around it; a symbol placed SEVERAL times is the row this exists for, and its
    // copies are spread across their loop as before.
    let phase_rank = {
        let mut pos: Vec<(u16, (i64, i64))> = Vec::new();
        collect_clip_positions(&root_frames[root_idx], root_m, &sprite_frames, &mut pos);
        pos.sort_unstable();
        pos.dedup();
        let mut by_sym: BTreeMap<u16, Vec<(i64, i64)>> = BTreeMap::new();
        for (id, p) in pos { by_sym.entry(id).or_default().push(p); }
        let mut m: std::collections::HashMap<(u16, (i64, i64)), usize> = Default::default();
        for (id, places) in by_sym {
            if places.len() < 2 { continue; }
            for (r, p) in places.into_iter().enumerate() { m.insert((id, p), r); }
        }
        m
    };

    // instances at a given global frame, classified + composited per layer.
    let frame_instances = |g: usize| -> Vec<Instance> {
        let root = &root_frames[root_idx];
        let mut out = Vec::new();
        walk_frame(root, root_m, g, None, None, planes, sym_names, shape_defs, &sprite_frames, &mut out, 0, (0.0, 0.0), 0, None, &phase_rank, None, 1.0, crate::swf_timeline::Tint::ID, &[], &labelled, hidden_clips, false, collision_syms, false, false, &[], 1);
        // exclude non-art PLANES (terrain/masks/spawns, by instance name) and any stray
        // collision/scaffolding markers (by linkage suffix) that slipped into an art plane.
        if std::env::var("PEPTIDE_STAGE_DEBUG").is_ok() {
            for i in out.iter().filter(|i| !shape_defs.contains_key(&i.shape_id)) {
                eprintln!("[no-shapedef] id={} plane={:?} aabb=({:.0},{:.0} {:.0}x{:.0})",
                    i.shape_id, i.plane, i.aabb.x, i.aabb.y, i.aabb.w, i.aabb.h);
            }
        }
        // hidden by its own class (carried down the chain by the walk), unless the stage shows it
        // later by name. the instance NAME alone decides nothing: a terrain plate placed as `cliff`
        // hides itself, and the cliff's art placed as `cliff` in another plane does not.
        let shown = |i: &Instance| shown_clips.contains(&i.sym_name)
            || i.inst_name.as_deref().is_some_and(|n| shown_clips.contains(n));
        // a clip with a timeline of its own is art whatever it was named: butter building's
        // `seg2platforms` is the layout's picture, six frames the stage steps through by label,
        // and a collision plate is one frame of one shape
        // ...unless the clip DECLARES itself collision: what says it is terrain is terrain,
        // however many frames it plays (galaxy tours' starship deck has a timeline of its own)
        let animated = |i: &Instance| !i.under_collision
            && i.inst_clip.and_then(|c| sprite_frames.get(&c)).is_some_and(|f| f.len() > 1);
        if std::env::var("PEPTIDE_STAGE_DEBUG").is_ok() {
            for i in out.iter().filter(|i| animated(i) && (is_non_art(i.inst_name.as_deref().unwrap_or("")) || is_non_art(&i.sym_name))) {
                eprintln!("[art-by-timeline] inst={:?} sym={:?} clip={:?} under_collision={}", i.inst_name, i.sym_name,
                    i.inst_clip.and_then(|c| sym_names.get(&c)), i.under_collision);
            }
        }
        out.retain(|i| shape_defs.contains_key(&i.shape_id)
            && !is_non_art_plane(i.plane.as_deref())
            && (!i.hidden_by_class || shown(i) || i.inst_clip.is_some_and(|c| labelled.contains(&c)))
            && !object_art.contains(&i.sym_name)
            && !i.inst_name.as_deref().is_some_and(|n| object_art.contains(n))
            // scaffolding (a boundary, a spawn marker) stays out whatever timeline it sits in
            && !is_scaffold_linkage(i.inst_name.as_deref().unwrap_or("")) && !is_scaffold_linkage(&i.sym_name)
            && (animated(i) || (!is_non_art(i.inst_name.as_deref().unwrap_or("")) && !is_non_art(&i.sym_name))));
        if std::env::var("PEPTIDE_STAGE_DEBUG").is_ok() {
            let names: Vec<String> = out.iter().map(|i| format!("{}:{}/{}/{}", if art_kind(i) == ArtKind::Stage { "Stage" } else { "Other" }, i.inst_name.as_deref().unwrap_or("-"), i.sym_name, i.shape_id)).collect();
            eprintln!("[art-frame] g={g} n={} {}", out.len(), names.join(" "));
        }
        out
    };
    // The shadow mask is dropped by that filter along with the rest of the scaffolding -- it is
    // not art, and SSF2 never draws it. But it is the SHAPE that says where a fighter's shadow
    // falls, which is exactly what the engine wants as a mask sprite, so it is picked up here from
    // the unfiltered walk before the filter takes it.
    let mask_instances = |g: usize| -> Vec<Instance> {
        let root = &root_frames[root_idx];
        let mut out = Vec::new();
        walk_frame(root, root_m, g, None, None, planes, sym_names, shape_defs, &sprite_frames,
                   &mut out, 0, (0.0, 0.0), 0, None, &phase_rank, None, 1.0, crate::swf_timeline::Tint::ID, &[], &labelled, hidden_clips, false, collision_syms, false, false, &[], 1);
        out.retain(|i| shape_defs.contains_key(&i.shape_id) && {
            let n = i.sym_name.to_ascii_lowercase();
            let l = i.inst_name.as_deref().unwrap_or_default().to_ascii_lowercase();
            n.contains("shadowmask") || l.contains("shadowmask")
        });
        out
    };
    let composite = |insts: &[Instance], kinds: &[ArtKind]| -> Option<StageArt> {
        let group: Vec<&Instance> = insts.iter().filter(|i| kinds.contains(&art_kind(i))).collect();
        // PNG stays native-resolution; only the placement is scaled (the emitter renders the
        // IMAGE layer at `scale`), so the art and geometry share one scale with no upscaling.
        composite_layer(&group, shape_defs, bitmaps, ox, oy, None).map(|mut a| {
            a.x *= scale; a.y *= scale; a
        })
    };

    // sample every frame's instance set + the composited image.
    let sampled: Vec<(Vec<Instance>, Option<StageArt>)> = (0..n_samples).map(|i| {
        let insts = frame_instances(sample_frame(i));
        let img = composite(&insts, &[ArtKind::Stage]);
        (insts, img)
    }).collect();
    let stage_counts: Vec<usize> = sampled.iter()
        .map(|(insts, _)| insts.iter().filter(|x| art_kind(x) == ArtKind::Stage).count()).collect();
    let max_count = stage_counts.iter().copied().max().unwrap_or(0);

    // background / foreground / parallax from the richest frame — the one with the most ART of
    // ANY plane (an animated stage's frame 0 is often empty; many stages put ALL their art in
    // the backdrop/background plane, so counting only Stage-plane art would leave base_idx on
    // an empty frame and drop the whole backdrop — half the corpus rendered as a placeholder).
    let total_counts: Vec<usize> = sampled.iter().map(|(insts, _)| insts.len()).collect();
    let base_idx = total_counts.iter().enumerate().max_by_key(|(_, c)| **c).map(|(i, _)| i).unwrap_or(0);
    let base_insts = &sampled[base_idx].0;
    if let Ok(want) = std::env::var("PEPTIDE_FRAME_SCAN") {
        let ids: Vec<u16> = want.split(',').filter_map(|x| x.trim().parse().ok()).collect();
        let mut seen: std::collections::BTreeMap<(u16, i64), Vec<usize>> = Default::default();
        for (fi, (insts, _)) in sampled.iter().enumerate() {
            for i in insts.iter().filter(|i| ids.contains(&i.shape_id)) {
                seen.entry((i.shape_id, (i.cx / 25.0).round() as i64 * 25)).or_default().push(fi);
            }
        }
        for ((sid, x), frames) in &seen {
            eprintln!("[frame-scan] shape {sid} near x={x}: on {} of {} frames (base={}, first={:?})",
                frames.len(), sampled.len(), frames.contains(&base_idx), &frames[..frames.len().min(4)]);
        }
    }
    if std::env::var("PEPTIDE_STAGE_DEBUG").is_ok() {
        eprintln!("=== art instances (richest frame {base_idx}, {} insts) ===", base_insts.len());
        for i in base_insts {
            let k = match art_kind(i) { ArtKind::Backdrop => "BKDRP", ArtKind::Parallax => "PARLX", ArtKind::Background => "BG", ArtKind::Stage => "STAGE", ArtKind::Foreground => "FG" };
            eprintln!("  [{k:5}] plane={:?} sym={:?} name={:?} shape={} aabb=({:.0},{:.0} {:.0}x{:.0})",
                i.plane, i.sym_name, i.inst_name, i.shape_id, i.aabb.x, i.aabb.y, i.aabb.w, i.aabb.h);
        }
    }
    // The planes are ported in the ORDER AND PLANE THE SOURCE PUTS THEM IN. SSF2 names a plane
    // `foreground` to say "in front of the fighters", so that is where it goes; second-guessing
    // that -- folding a foreground behind them when it looked like a re-drawn copy of the
    // structure -- overrode the one statement the source actually makes about depth, and let
    // fighters stand through the deck lip that is there to cover them at the ledge.
    // composite an explicit instance group (back-to-front = walk order), scaled like `composite`.
    let composite_grp = |group: Vec<&Instance>, bounds: Option<(f64, f64, f64, f64)>| -> Option<StageArt> {
        composite_layer(&group, shape_defs, bitmaps, ox, oy, bounds).map(|mut a| { a.x *= scale; a.y *= scale; a })
    };
    // union raw bounds of an instance set (None if empty) — the fixed canvas for an animated layer.
    let union_bounds = |insts: &[&Instance]| -> Option<(f64, f64, f64, f64)> {
        (!insts.is_empty()).then(|| (
            insts.iter().map(|i| i.aabb.left()).fold(f64::MAX, f64::min),
            insts.iter().map(|i| i.aabb.top()).fold(f64::MAX, f64::min),
            insts.iter().map(|i| i.aabb.right()).fold(f64::MIN, f64::max),
            insts.iter().map(|i| i.aabb.bottom()).fold(f64::MIN, f64::max),
        ))
    };

    // one SSF2 source frame = one hold unit (the 30fps->60fps doubling is applied uniformly by the
    // emitter, exactly like the character port). Consecutive identical frames RLE into one longer
    // hold so a held source frame reads as a pause.
    let base_hold = 1u32;
    // A frame is the picture AND where it is put. Comparing pictures alone reads an element that
    // is one cel being moved or turned as repeating immediately, and truncating to that period
    // throws the whole motion away: a turning piece comes back as a still, which is the form this
    // path exists to produce.
    let same_frame = |a: &StageArt, b: &StageArt| {
        a.png == b.png && a.x == b.x && a.y == b.y && a.rotation == b.rotation
            && a.scale_x == b.scale_x && a.scale_y == b.scale_y && a.alpha == b.alpha
            // ...and the same COLOUR. The source slides a stage from day to night by recolouring
            // the same picture in the same place, so a comparison that skips colour reads those
            // frames as one frame held: the slide is folded away, and the loop finder then reads
            // the whole day as repeating on whatever short cycle the ART repeats on and truncates
            // to it. Same reason position and rotation are here.
            && a.tint.iter().zip(b.tint).all(|(x, y)| (x - y).abs() < 1e-4)
    };
    let rle = |frames: Vec<StageArt>| -> Vec<StageArt> {
        if frames.len() <= 1 { return frames; }
        let mut out: Vec<StageArt> = Vec::new();
        for mut f in frames {
            f.hold = base_hold;
            match out.last_mut() {
                // same picture in the same PLACE. matching on the picture alone folds a piece that
                // is one cel being moved or turned into a single held frame, which is that motion
                // deleted.
                Some(prev) if same_frame(prev, &f) => prev.hold += base_hold,
                _ => out.push(f),
            }
        }
        out
    };
    // The smallest period at which a captured frame sequence repeats. Each backdrop ELEMENT loops
    // on its OWN movieclip clock, but we sample every element over `n_samples` (the longest clip on
    // the stage), so a short element loop comes back repeated N times. Truncating to one period
    // gives the element's REAL loop length, so its emitted VFX loops at its own rate instead of the
    // stage's (a 6-frame ember stays 6 frames, not 142). Returns the full length if it never
    // repeats within the sample window.
    let loop_period = |frames: &[StageArt]| -> usize {
        let n = frames.len();
        let p = (1..n).find(|&p| (0..n - p).all(|i| same_frame(&frames[i], &frames[i + p]))).unwrap_or(n);
        // A shorter loop only holds if it FITS the stage's timeline a whole number of times. An
        // element that repeats every 4480 frames of a 7440-frame day, played on its own loop,
        // drifts out of step with the day a little more each lap -- so clocktown's rain splashes,
        // which the source draws only while it rains, end up splashing at every hour of it. A
        // period that divides the timeline stays in phase (a six-frame ember still costs six
        // frames); one that does not keeps the whole thing.
        // A period that does not divide the sample window is still the element's real loop: the
        // window is the longest clip on the STAGE and has nothing to do with this element. Read
        // live off SSF2, clocktown's rain splash runs a 14-frame loop and its torch embers a
        // 23-frame one inside a 3720-frame day, and 3720 divides neither.
        //
        // What this gives up is phase against the stage clock, which only matters for a schedule
        // played against that clock; those wrap with the element now (`len` in m_alpha).
        let _ = n.is_multiple_of(p);
        p
    };
    // ...and a period the element's OWN clip does not come round on is not its loop: a 600-frame
    // clip whose look repeats every 16 restarts that look at 600 in the source (600 is not a
    // multiple of 16), so the clip's own length is the loop, within the window
    // ...the clip the pieces were cut from, or the pieces' own symbol when that is the longer
    // timeline (a 23-frame clip nested in a one-frame section is the element that loops)
    let sym_ids: BTreeMap<&str, u16> = sym_names.iter().map(|(id, n)| (n.as_str(), *id)).collect();
    // a clip's own timeline length, by its name: a period it does not come round on is not its loop
    let clip_len = |id: u16| -> Option<usize> { sprite_frames.get(&id).map(|f| f.len()) };
    let own_clip_len = |insts: &[&Instance]| -> Option<usize> {
        insts.iter().filter_map(|i| i.inst_clip).filter_map(|c| sprite_frames.get(&c)).map(|f| f.len())
            .chain(insts.iter().filter_map(|i| sym_ids.get(i.sym_name.as_str())).filter_map(|id| sprite_frames.get(id)).map(|f| f.len()))
            .max()
    };
    let fit_own = |p: usize, n: usize, own: Option<usize>| -> usize {
        let r = match own { Some(o) if o > p && o % p != 0 => o.min(n), _ => p };
        if std::env::var("PEPTIDE_STAGE_DEBUG").is_ok() { eprintln!("[period] detected={p} window={n} own={own:?} -> {r}"); }
        r
    };
    // Group a set of bg-plane instances into ORDERED per-element layers (the SSF2 movieclip
    // model: each animated backdrop object is its own clip). `member` selects bg-plane art;
    // each distinct `sym_name` becomes one layer in first-appearance (back-to-front) draw order.
    // An element that changes across the sampled frames keeps its own animation loop; a static
    // one collapses to its single base-frame image.
    // `one_canvas`: after building, pad every frame of an element to a single canvas size. A
    // CAMERA BACKGROUND needs it -- the engine sizes a background layer once
    // (`originalBGWidth`/`Height`) and draws every frame at that size, so frames that are each
    // their own crop come out stretched and shifted against one another. Padding (rather than
    // re-rasterising onto union bounds) keeps the one-cel-plus-rotation form the rings use, which
    // is the difference between a 12MB stage and an 87MB one.
    let seg = |inst: &Instance, fr: &[StageArt]| clip_segments(Some(&inst), fr, sprites, sym_names, &sprite_frames, abcs, &phase_rank, &deferred_event_clips);
    let group_bg_layers = |member: &dyn Fn(&Instance) -> bool, one_canvas: bool| -> Vec<BgLayer> {
        // one layer per distinct PLACEMENT (sym_name + instance anchor), not per symbol: repeated
        // placements of one symbol (16 torch-ember emitters at 16 wall positions) become 16 objects
        // each at its own position, instead of one merged union-bounds image (which both desyncs
        // their independent loops and wastes a giant mostly-empty texture). a single-instance symbol
        // is one group, same as before.
        // A clip's own shapes are split further, by the DEPTH each sits on, but ONLY where that
        // pays: a depth whose content changes over time becomes its own element, and every static
        // depth stays merged into one composite. Flash draws a backdrop as many shapes on many
        // depths, so splitting them all would turn one bitmap into dozens of entities; merging
        // them all is what made clocktown emit its whole 739x436 town once per frame, 623 times,
        // because a clock hand two hundred pixels away kept changing. Splitting only what moves
        // keeps a large bitmap large -- as it is in the source -- and gives the moving parts their
        // own small ones.
        //
        // A blend is the exception: a hard-light mask is baked by compositing it OVER the art
        // underneath, so those depths have to stay in one group or the effect is lost.
        let group_blended: std::collections::BTreeSet<(String, i64, i64, u64)> = sampled.iter()
            .flat_map(|(insts, _)| insts.iter())
            .filter(|i| member(i) && i.blend.is_some())
            .map(|i| (i.sym_name.clone(), i.inst_anchor.0.round() as i64, i.inst_anchor.1.round() as i64, i.inst_path))
            .collect();
        // Which (clip, depth) pairs change across the sample: a different shape on that depth, or
        // the same shape moved. Read off the placements, so nothing is rasterised to find out.
        let mut depth_seen: std::collections::BTreeMap<(String, i64, i64, u64, u16), std::collections::BTreeSet<(u16, i64, i64)>> = Default::default();
        for (insts, _) in sampled.iter() {
            for i in insts.iter().filter(|i| member(i)) {
                depth_seen
                    .entry((i.sym_name.clone(), i.inst_anchor.0.round() as i64, i.inst_anchor.1.round() as i64, i.inst_path, i.leaf_depth))
                    .or_default()
                    .insert((i.shape_id, i.aabb.x.round() as i64, i.aabb.y.round() as i64));
            }
        }
        // Splitting only pays where there is a STILL PLATE to save: a group that is part static
        // and part moving re-rasterises the whole thing every frame because of the moving part,
        // which is how clocktown emitted its 739x436 town 623 times over a clock hand. A group
        // where everything moves has no plate to preserve, and splitting it just trades one
        // shared canvas for several union-of-path canvases that are mostly empty -- measured on
        // bowserscastle's bubbles, which doubled the stage's art that way. So: split only a group
        // that has both.
        let mut group_static: std::collections::BTreeMap<(String, i64, i64, u64), (bool, bool)> = Default::default();
        for ((sn, ax, ay, ap, _), shapes) in depth_seen.iter() {
            let e = group_static.entry((sn.clone(), *ax, *ay, *ap)).or_insert((false, false));
            if shapes.len() > 1 { e.1 = true; } else { e.0 = true; }
        }
        let depth_animates = |i: &Instance| -> bool {
            let gk = (i.sym_name.clone(), i.inst_anchor.0.round() as i64, i.inst_anchor.1.round() as i64, i.inst_path);
            if group_blended.contains(&gk) { return false; }
            // nothing still to protect, or nothing moving to separate -> leave the group whole
            if !group_static.get(&gk).is_some_and(|(has_still, has_moving)| *has_still && *has_moving) {
                return false;
            }
            depth_seen.get(&(gk.0, gk.1, gk.2, gk.3, i.leaf_depth)).is_some_and(|v| v.len() > 1)
        };
        // key: the placement, plus the depth when that depth animates (None = the merged still part)
        let akey = |i: &Instance| (i.sym_name.clone(), i.inst_anchor.0.round() as i64, i.inst_anchor.1.round() as i64, i.inst_path,
                                   if depth_animates(i) { Some(i.leaf_depth) } else { None });
        let mut order: Vec<(String, i64, i64, u64, Option<u16>)> = Vec::new();
        let mut seen: std::collections::BTreeSet<(String, i64, i64, u64, Option<u16>)> = std::collections::BTreeSet::new();
        for i in base_insts.iter().filter(|i| member(i)) {
            let k = akey(i);
            if seen.insert(k.clone()) { order.push(k); }
        }
        let mut built: Vec<((String, i64, i64, u64), BgLayer)> = order.iter().filter_map(|(sname, ax, ay, apath, adepth)| {
            let matches = |i: &Instance| member(i) && &i.sym_name == sname
                && i.inst_anchor.0.round() as i64 == *ax && i.inst_anchor.1.round() as i64 == *ay
                && i.inst_path == *apath
                && (if depth_animates(i) { Some(i.leaf_depth) } else { None }) == *adepth;
            // fixed canvas = the union of THIS placement's bounds across ALL frames, so the layer
            // stays put while its content animates (no wiggle as the flame bbox flickers).
            let all: Vec<&Instance> = sampled.iter()
                .flat_map(|(insts, _)| insts.iter().filter(|i| matches(i))).collect();
            // the opacity the source authored for this element: the lowest any of its placements
            // carries (SSF2 sets it on the clip, so every leaf under it inherits the same value)
            let elem_alpha = all.iter().map(|i| i.alpha).fold(1.0_f64, f64::min);
            let bounds = union_bounds(&all);
            // IN FRONT OF THE BAKED ART, OR BEHIND IT? The source answers, on the frames where
            // this element and a piece of baked art are both drawn and actually overlap: whatever
            // is later in the display list is in front. Counted rather than taken from one frame,
            // so a single odd frame does not decide the stack.
            let (mut vote_behind, mut vote_front) = (0usize, 0usize);
            for (insts, _) in sampled.iter() {
                let mine: Vec<usize> = insts.iter().enumerate()
                    .filter(|(_, i)| matches(i)).map(|(n, _)| n).collect();
                if mine.is_empty() { continue; }
                for (n, other) in insts.iter().enumerate() {
                    if matches(other) || !member(other) { continue; }
                    let hit = |a: &Rect, b: &Rect| a.x < b.x + b.w && b.x < a.x + a.w
                                                   && a.y < b.y + b.h && b.y < a.y + a.h;
                    let overlaps = mine.iter().any(|&m| hit(&insts[m].aabb, &other.aabb));
                    if !overlaps { continue; }
                    let first = mine[0];
                    if n > first { vote_behind += 1; } else { vote_front += 1; }
                }
            }
            let behind_base = vote_behind > vote_front;
            // a frame where the element is INVISIBLE (a bubble between pops, a podoboo between
            // leaps) is a real frame of its loop: emit it as a blank, not a gap. dropping it
            // both broke the animated test (collapsing the element to one static frame) and
            // compressed the quiet stretches out of the loop.
            let blank = || bounds.map(|(l, t, _, _)| StageArt {
                png: blank_png(), x: (l - ox) * scale, y: (t - oy) * scale, w: 1, h: 1, hold: 1, rotation: 0.0, scale_x: 1.0, scale_y: 1.0 , alpha: 1.0 , pivot: (0.0, 0.0), tint: TINT_ID });
            let per_frame: Vec<StageArt> = sampled.iter().filter_map(|(insts, _)| {
                let grp: Vec<&Instance> = insts.iter().filter(|i| matches(i)).collect();
                composite_grp(grp, bounds).or_else(blank)
            }).collect();
            let animated = per_frame.len() == n_samples
                && per_frame.windows(2).any(|w| w[0].png != w[1].png);
            // a label-segmented clip is a state machine: its emitted art is the segments SSF2 can
            // reach, chained by their jumps, NOT the whole timeline played end to end.
            let segments = if animated {
                clip_segments(all.first(), &per_frame, sprites, sym_names, &sprite_frames, abcs, &phase_rank, &deferred_event_clips)
            } else { Vec::new() };
            if !segments.is_empty() {
                // run-length each segment SEPARATELY: collapsing across a boundary (the last frame
                // of one segment matching the first of the next) would merge them into one keyframe
                // and the emitter could no longer tell where a segment starts.
                let mut segments = segments;
                for s in &mut segments { s.frames = rle(std::mem::take(&mut s.frames)); }
                let frames = segments.iter().flat_map(|s| s.frames.iter().cloned()).collect();
                return Some(((sname.clone(), *ax, *ay, *apath), BgLayer { named_path: all.first().map(|i| i.named_path.clone()).unwrap_or_default(), behind_base, presence: Vec::new(), name: sname.clone(), clip: sname.clone(), frames, segments, pan: None, fades: false, authored_alpha: elem_alpha , starts_hidden: all.iter().any(|i| i.hidden_by_class) }));
            }
            // ONE shape the source moves or turns is emitted as that shape ONCE, placed by a
            // transform each frame -- the same thing the character path does with an arm. Baking
            // instead gives a fresh raster per angle: clocktown's clock rings arrived as five
            // elements of 200-460 frames with every frame unique, ~67MB for art the source stores
            // as five bitmaps. The rasteriser fits a shape to its axis-aligned bounds, so the art
            // is taken from the frame where the shape sits squarest (the least rotated), and the
            // rest is rotation.
            if animated {
                // an ELEMENT is one layer, so it takes the placement path only when the object is
                // drawn once at a time. Several at once is split into one object per slot by the
                // track path, which emits a layer each.
                let slots = frame_slots(&sampled, &matches);
                let pf: Vec<Option<&Instance>> = slots.iter()
                    .map(|f| if f.len() == 1 { f.first().copied() } else { None }).collect();
                let single = slots.iter().all(|f| f.len() <= 1);
                if let Some(fs) = single.then(|| single_shape_frames(&pf, ox, oy, scale, shape_defs, bitmaps)).flatten() {
                    let p = fit_own(loop_period(&fs), fs.len(), own_clip_len(&all));
                    return Some(((sname.clone(), *ax, *ay, *apath),
                                 BgLayer { named_path: all.first().map(|i| i.named_path.clone()).unwrap_or_default(), behind_base, presence: Vec::new(), name: sname.clone(), clip: sname.clone(), pan: None, fades: false, authored_alpha: elem_alpha, frames: rle(fs[..p].to_vec()), segments: Vec::new() , starts_hidden: all.iter().any(|i| i.hidden_by_class) }));
                }
            }
            // Whether the SOURCE places this element on each frame -- the group being empty is the
            // stage not putting it there, which is a different thing from its cel being blank.
            let placed_on: Vec<bool> = sampled.iter()
                .map(|(insts, _)| insts.iter().any(&matches)).collect();
            let runs: Vec<(usize, usize)> = {
                let mut v: Vec<(usize, usize)> = Vec::new();
                for (f, on) in placed_on.iter().enumerate() {
                    if !on { continue; }
                    match v.last_mut() { Some(r) if r.1 + 1 == f => r.1 = f, _ => v.push((f, f)) }
                }
                v
            };
            let bursty = runs.len() >= 3 && runs.iter().all(|(a, b)| b - a < 120);
            let frames = if animated && bursty {
                // put down over and over: what plays each time is the clip's own few frames, so
                // that is the art, and the container it lives in is what comes and goes
                let (a, b) = *runs.iter().max_by_key(|(a, b)| b - a).unwrap();
                rle(per_frame[a..=b.min(per_frame.len() - 1)].to_vec())
            } else if animated {
                // truncate to the element's OWN loop period (it was sampled over the stage's
                // longest clip), so its VFX loops at its own length, not the whole stage's.
                let p = fit_own(loop_period(&per_frame), per_frame.len(), own_clip_len(&all));
                rle(per_frame[..p].to_vec())
            } else {
                let grp: Vec<&Instance> = base_insts.iter().filter(|i| matches(i)).collect();
                composite_grp(grp, bounds).into_iter().collect()
            };
            let presence: Vec<(usize, usize)> = if bursty {
                runs.iter().map(|(a, b)| (a * FRAME_RATIO_USIZE, (b + 1) * FRAME_RATIO_USIZE)).collect()
            } else { Vec::new() };
            if std::env::var("PEPTIDE_STAGE_DEBUG").is_ok() {
                eprintln!("[bg-group] {sname} at ({ax},{ay}) path={apath} depth={adepth:?}: {} instances over {} samples, animated={animated}, bursty={bursty}, frames={}",
                    all.len(), n_samples, frames.len());
            }
            (!frames.is_empty()).then(|| ((sname.clone(), *ax, *ay, *apath),
                                          BgLayer { named_path: all.first().map(|i| i.named_path.clone()).unwrap_or_default(), behind_base, presence, name: sname.clone(), clip: sname.clone(), frames, segments: Vec::new(), pan: None, fades: false, authored_alpha: elem_alpha , starts_hidden: all.iter().any(|i| i.hidden_by_class) }))
        }).collect();

        // Parts of ONE source clip run off ONE timeline. Split into separate elements they each
        // collapse to their own loop length, and from the first restart they are out of phase with
        // each other for good -- which is what scattered clocktown's clock, five rings of 11 to 16
        // frames that should turn together. So every element from a group is padded (by repeating
        // its own loop) to the group's longest, and they stay in step. Repeats cost nothing: the
        // emitter writes one image per distinct picture and points the extra frames at it.
        let mut longest: std::collections::BTreeMap<(String, i64, i64, u64), usize> = Default::default();
        for (k, l) in built.iter() {
            let n = l.frames.iter().map(|f| f.hold.max(1) as usize).sum::<usize>();
            let e = longest.entry(k.clone()).or_insert(0);
            *e = (*e).max(n);
        }
        for (k, l) in built.iter_mut() {
            if l.frames.len() < 2 || !l.segments.is_empty() { continue; }
            let want = longest.get(k).copied().unwrap_or(0);
            let own: usize = l.frames.iter().map(|f| f.hold.max(1) as usize).sum();
            if own == 0 || own >= want { continue; }
            let cycle = l.frames.clone();
            let mut total = own;
            while total < want {
                for f in &cycle {
                    if total >= want { break; }
                    l.frames.push(f.clone());
                    total += f.hold.max(1) as usize;
                }
            }
            // a cycle that does not divide the group's length leaves a remainder, and a remainder
            // is a drift of that many frames every time round. Hold the last frame for it.
            if total < want {
                if let Some(last) = l.frames.last_mut() { last.hold += (want - total) as u32; }
            }
        }
        let mut out: Vec<BgLayer> = built.into_iter().map(|(_, l)| l).collect();
        if one_canvas { for l in out.iter_mut() { pad_frames_to_one_canvas(&mut l.frames, scale); } }
        // what the plane actually came out as, which is the question every "did this thing keep
        // moving" hunt ends up asking
        if std::env::var("PEPTIDE_STAGE_DEBUG").is_ok() {
            for l in out.iter() {
                eprintln!("[group ] {:<28} frames={}", l.name, l.frames.len());
            }
        }
        out
    };

    // foreground = the genuine in-front props (non-overlapping foreground), sampled PER FRAME so
    // an animated SSF2 foreground (bowserscastle's shimmering lava-glow sheet over the floor)
    // actually animates instead of freezing on one frame. static foregrounds collapse to one frame.
    // A blended placement is NOT emitted as art of its own: it is baked into the art beneath it
    // (see `bake_hardlight` for why the engine leaves no other option). Splitting it out as its
    // own element is what made bowserscastle's lightmask vanish — alone on an empty canvas the
    // old emulation had nothing to blend against and fell back to plain alpha.
    let is_blended = |i: &Instance| i.blend == Some(swf::BlendMode::HardLight);
    // the plane's authored opacity: the LOWEST alpha any of its placements carries (SSF2 sets it
    // on a parent clip, so every leaf under that clip inherits the same value). 1.0 when nothing
    // in the plane is translucent.
    let authored = base_insts.iter()
        .filter(|i| art_kind(i) == ArtKind::Foreground)
        .map(|i| i.alpha)
        .fold(f64::INFINITY, f64::min);
    // A plane with no colour transform anywhere reads as fully opaque, and drawing this sheet
    // opaque over the backdrop's own lava puts a hard seam at its top edge (the rasterised path is
    // binary 0/255 — the source art carries no soft edge to inherit). SSF2 makes the plane
    // translucent outside the SWF placement data, so when the source says nothing, fall back to the
    // long-standing translucent default rather than to opaque.
    const FG_ALPHA_FALLBACK: f64 = 0.5;
    let foreground_alpha = match authored.is_finite() && authored < 1.0 {
        true => authored.clamp(0.0, 1.0),
        false => FG_ALPHA_FALLBACK,
    };
    if std::env::var("PEPTIDE_STAGE_DEBUG").is_ok() {
        let alphas: Vec<String> = base_insts.iter().filter(|i| art_kind(i) == ArtKind::Foreground)
            .map(|i| format!("{:.2}", i.alpha)).collect();
        eprintln!("[fg-alpha] plane opacity {foreground_alpha:.2} from placements {alphas:?}");
    }
    let fg_member = |i: &Instance| art_kind(i) == ArtKind::Foreground && !is_blended(i);
    let fg_all: Vec<&Instance> = sampled.iter().flat_map(|(insts, _)| insts.iter().filter(|i| fg_member(i))).collect();
    let fg_bounds = union_bounds(&fg_all);
    let fg_per_frame: Vec<StageArt> = sampled.iter()
        .filter_map(|(insts, _)| composite_grp(insts.iter().filter(|i| fg_member(i)).collect(), fg_bounds))
        .collect();
    let fg_animated = fg_per_frame.len() == n_samples
        && fg_per_frame.windows(2).any(|w| w[0].png != w[1].png);
    let foreground_composite: Vec<StageArt> = if fg_animated {
        rle(fg_per_frame)
    } else {
        composite_grp(base_insts.iter().filter(|i| fg_member(i)).collect(), fg_bounds).into_iter().collect()
    };
    // The foreground plane gets the SAME per-element split the background does. Compositing the
    // whole plane into one image is what froze it: SSF2 authors each in-front prop as its own
    // movieclip on its own loop (exactly like the backdrop), and flattening them into a single
    // picture leaves one master clock for all of them, so every one that animates independently
    // stops animating. The composite stays as the fallback for a plane the grouping can't split.
    //
    // Blended placements are no longer a reason to keep the plane composited: they are baked into
    // the art beneath instead of shipped as a layer (see `bake_hardlight`), so nothing here needs
    // a shared canvas any more and every unblended element can have its own clock.
    // The in-front plane is ported as the source's TIMELINE too, for the same reason the camera
    // background is: its objects are moved and FADED, and an element flattened per frame loses the
    // fade. Clocktown's ambient-lighting sheet is a screen-sized tint whose opacity follows the
    // time of day -- drawn at a fixed strength it washes out the whole stage.
    let foreground_elements = {
        // a piece whose clips come round together only after several laps of the stage's longest
        // timeline (meteo voyage's arwings fly a 3173-frame path and flicker on a 6-frame clip of
        // their own: the source's picture repeats every 19038) is followed that far, by walking the
        // extra frames for the in-front pieces alone; every other piece keeps one lap
        let lap = sampled.len();
        let longest = sampled.iter().flat_map(|(insts, _)| insts.iter().filter(|i| fg_member(i)).map(|i| i.chain_len)).max().unwrap_or(1);
        let reach = if longest > lap { longest.min(lap * 8) } else { lap };
        let mut followed: Vec<(Vec<Instance>, Option<StageArt>)> = Vec::new();
        let tracks = if reach > lap {
            followed.extend(sampled.iter().map(|(insts, _)| (insts.iter().filter(|i| fg_member(i)).cloned().collect::<Vec<_>>(), None)));
            followed.extend((lap..reach).map(|g| (frame_instances(sample_frame(g)).into_iter().filter(|i| fg_member(i)).collect::<Vec<_>>(), None)));
            timeline_tracks_lapped(&followed, &fg_member, ox, oy, scale, shape_defs, bitmaps, Some(&seg), &clip_len, lap)
        } else {
            timeline_tracks(&sampled, &fg_member, ox, oy, scale, shape_defs, bitmaps, Some(&seg), &clip_len)
        };
        if tracks.is_empty() { group_bg_layers(&fg_member, false) } else { tracks }
    };
    let foreground = if foreground_elements.is_empty() { foreground_composite } else { Vec::new() };
    // when the backdrop carries `_cambg` parallax layers, each backdrop/cambg LAYER (grouped
    // by symbol) becomes its own camera-relative plane with its own auto-derived pan rate (so
    // the sky, sun rays, trees, ... scroll at different rates, reading as depth — and the rays
    // draw in front of the sky, not occluded by it). The stageMC `background` plane (the
    // island fighters stand near) stays a fixed near-layer in front. Without parallax, backdrop
    // + background fold into one fixed bg. The folded structure-foreground draws last (in front
    // of the background structure, still behind fighters).
        // A CAMERA BACKGROUND DOES NOT HAVE TO BE ON THE STAGE'S TIMELINE. The package names the
        // clip and the game makes one from the library; nothing has to place it, and several
        // stages place none of theirs -- flat zone's whole backdrop is declared and never put down,
        // so reading only the placement tree shipped the stage with no backdrop at all. Each
        // declared name that the walk did not already find is looked up in the library and read
        // straight from its own timeline.
    let declared_cam: Vec<BgLayer> = {
        let mut out: Vec<BgLayer> = Vec::new();
        for bg in cam_backgrounds.iter() {
                if bg.linkage_id.is_empty() { continue }
                let want = bg.linkage_id.to_ascii_lowercase();
                // ALWAYS from the library, even when the tree also puts the clip down. The game
                // makes a camera background from the library clip and never from the placement, so
                // whatever transform the tree carries is not part of it: fourside places its
                // backdrop at a size of its own, and read through that its moon came out nearly
                // three times the size it is against the city.
                let Some(cid) = sym_names.iter()
                    .find(|(id, n)| clip_name_is(n, &want) && sprites.contains_key(id))
                    .map(|(id, _)| *id) else {
                        log::warn!("stage declares the camera background '{}' and no clip by that \
                                    name is in the package", bg.linkage_id);
                        continue
                    };
                // ITS OWN TIMELINE, AS OBJECTS. Drawing a picture per frame bakes the motion into
                // pixels: castle siege's sky is one still image and its clouds are three shapes
                // being slid across it, and composited that came to 720 full-width pictures --
                // 957MB, which no package can carry. Read as the source keeps it, it is four
                // pictures and a list of where they go, which is what the timeline walk already
                // does for a background the tree places. A clip named by the package rather than
                // put down by the tree never reached that walk, so it is walked here.
                let by_object = sprite_frames.get(&cid).map(|clip| {
                    // OVER AS LONG AS IT ACTUALLY RUNS. The clip's own frame count is not its
                    // length when the motion is in a clip it PLACES: Sector Z's backdrop is one
                    // outer frame holding a nested clip that runs for 2599, and sampling the outer
                    // count reads a single still object and hands the whole thing back to be drawn
                    // a picture at a time. The nested clip runs on Flash's clock whatever the
                    // parent does, so the span to walk is the longest one placed here, and the
                    // outer frame that holds it repeats under it.
                    // ...and every clip in it repeats on its own period, so the span walked is
                    // the smallest one they all come round on together: yoshi's island's backdrop
                    // runs 35 frames with a 45-frame clip inside, and sampled over 45 the 35-frame
                    // pieces restarted a third of the way through their loop at every turn. Past
                    // a cap the longest wins, as before, and the shorter loops carry the seam.
                    fn gcd(a: usize, b: usize) -> usize { if b == 0 { a } else { gcd(b, a % b) } }
                    let nested: Vec<usize> = clip.iter().flat_map(|f| f.iter())
                        .filter_map(|c| sprite_frames.get(&c.0).map(|fr| fr.len()))
                        .filter(|n| *n > 1).collect();
                    let longest = nested.iter().copied().max().unwrap_or(1).max(clip.len());
                    let together = nested.iter().fold(clip.len().max(1), |acc, n| acc / gcd(acc, *n) * n);
                    let span = if together <= 720 { together } else { longest };
                    let sampled_clip: Vec<(Vec<Instance>, Option<StageArt>)> = (0..span)
                        .map(|g| {
                            let mut insts = Vec::new();
                            // owned by the backdrop clip itself, whose timeline carries the
                            // labels and frame scripts that hold and switch its sections
                            walk_frame(&clip[g % clip.len()], Mat::id(), g, Some(&bg.linkage_id), Some("cambg"), planes,
                                sym_names, shape_defs, &sprite_frames, &mut insts, 0, (0.0, 0.0),
                                0, None, &phase_rank, Some(cid), 1.0,
                                crate::swf_timeline::Tint::ID, &[], &labelled, &std::collections::BTreeSet::new(), false, &std::collections::BTreeSet::new(), false, false, &[], clip.len().max(1));
                            insts.retain(|i| shape_defs.contains_key(&i.shape_id));
                            (insts, None)
                        })
                        .collect();
                    // its sections too: a backdrop whose timeline is a state machine (castle
                    // siege's sky holds its section and plays a transition when the stage says)
                    // is read the way the placed planes are, so its pieces hold and jump the same
                    timeline_tracks(&sampled_clip, &|_| true, ox, oy, scale, shape_defs, bitmaps, Some(&seg), &clip_len)
                }).unwrap_or_default();
                // ONLY WHERE THERE IS MOTION TO KEEP OUT OF THE PIXELS. A backdrop that is ONE
                // still object is the same picture either way, and reading it as an object measures
                // its height off its own bounds rather than the canvas it was drawn on -- a couple
                // of percent different, which is a couple of percent of backdrop scale for no gain.
                // Bowsers castle's is one object on one frame, and stays a picture.
                //
                // Motion is what makes the difference, and it comes from either direction: SEVERAL
                // objects means one is moving against the others, and a clip of MANY FRAMES is
                // moving whether it holds one object or twenty. Sector Z's is a single object over
                // 2599 frames, and a picture per frame of it is 4096x623 nearly 2400 times: 1.6GB,
                // which the publish step cannot hold in memory, so the stage never ships at all.
                let animates = sprite_frames.get(&cid).is_some_and(|c| c.len() > 1);
                if by_object.len() > 1 || (animates && !by_object.is_empty()) {
                    log::info!("camera background '{}' read from the library as {} objects rather \
                        than one picture a frame", bg.linkage_id, by_object.len());
                    for mut t in by_object {
                        t.name = bg.linkage_id.clone();
                        t.clip = bg.linkage_id.clone();
                        out.push(t);
                    }
                    continue
                }
                let frames = extract_labeled_clip_anims(cid, sprites, sym_names, shape_defs,
                    bitmaps, planes, &[], 0.0, 0.0, &[]);
                let art: Vec<StageArt> = frames.into_iter().next().map(|a| a.frames).unwrap_or_default();
                if art.is_empty() {
                    // the package names it and the clip draws nothing: flat zone declares a
                    // backdrop whose clip is empty, and that is the stage, not a gap in the port
                    log::info!("camera background '{}' is declared and its clip draws nothing",
                        bg.linkage_id);
                    continue
                }
                log::info!("camera background '{}' comes from the library, not the tree ({} frames, \
                    read as {} object(s))", bg.linkage_id, art.len(), by_object.len());
                out.push(BgLayer {
                    named_path: Vec::new(),
                    name: bg.linkage_id.clone(), clip: bg.linkage_id.clone(), frames: art,
                    authored_alpha: 1.0, fades: false, pan: None, presence: Vec::new(),
                    segments: Vec::new(), starts_hidden: false, behind_base: false,
                });
        }
        out
    };
    let has_parallax = base_insts.iter().any(|i| art_kind(i) == ArtKind::Parallax)
        || !declared_cam.is_empty();
    let (background, parallax): (Vec<BgLayer>, Vec<ParallaxLayer>) = if has_parallax {
        // A camera background is split the same way every other plane is: into ELEMENTS. The
        // still ones become the camera-background plates the engine scrolls (that is what a
        // parallax layer is for); the animated ones stay elements and are promoted like any
        // other animated backdrop object, because flattening one into a plate freezes it --
        // which is what turned clocktown's clock, its day/night sky and its torches into a
        // single still picture. They carry the layer's declared pan so they scroll with it.
        // A DECLARED BACKGROUND IS READ FROM THE LIBRARY, so the tree's copy of it is not drawn
        // twice: the game makes a camera background from the library clip and never from the
        // placement, and a piece whose owning clip is a declared background is that copy. castle
        // siege's sky came out twice, once from each, a plate's width apart.
        // ...for a background whose timeline is a state machine, which is where the two copies
        // collided (each read in its own space, the pieces of one landed a plate's width from the
        // other). a background that loops is still read from the tree as well: galaxy tours'
        // random backdrop is six pictures on six frames, and the library read keeps one of them.
        let declared_owner = |i: &Instance| i.inst_clip.filter(|c| labelled.contains(c))
            .and_then(|c| sym_names.get(&c)).is_some_and(|n| {
                cam_backgrounds.iter().any(|b| !b.linkage_id.is_empty() && clip_name_is(n, &b.linkage_id.to_ascii_lowercase()))
            });
        let cam_member = |i: &Instance| matches!(art_kind(i), ArtKind::Backdrop | ArtKind::Parallax) && !declared_owner(i);
        // the layer carries the clip's name in the walk's own spelling, so both sides are read down
        // to their letters before they are compared
        let plain = |t: &str| t.chars().filter(|c| c.is_ascii_alphanumeric())
            .flat_map(|c| c.to_lowercase()).collect::<String>();
        if std::env::var("PEPTIDE_STAGE_DEBUG").is_ok() {
            for b in cam_backgrounds.iter() {
                eprintln!("[cambg-decl] {:?} x={:?} y={:?} auto={}", b.linkage_id, b.x_pan, b.y_pan, b.auto_pan);
            }
        }
        // WHICH DECLARATION IS THIS LAYER'S. The package declares a rate per background under the
        // library name it exports it as -- the stage's own id with the piece's name on the end
        // (`huecomundoBG3`) -- while the walk knows the piece by the name the tree placed it under
        // plus the ids of what it is made of (`bg3_8_1`). Compared head-first neither is a prefix
        // of the other, so every layer fell through to the FIRST declaration and a stage that
        // declares a different rate per layer scrolled all of them at its farthest layer's rate.
        // Read from the tail, and with the walk's trailing ids dropped, the two names meet.
        let tail = |t: &str| -> String {
            let mut s = t;
            while let Some(i) = s.rfind('_') {
                if s[i + 1..].chars().all(|c| c.is_ascii_digit()) && !s[i + 1..].is_empty() { s = &s[..i]; }
                else { break }
            }
            plain(s)
        };
        // The whole name first, and only then the name with the file's own cut taken off it. A
        // declaration names a library clip exactly, and two of them can differ by nothing but a
        // digit (`fs_building4`, `fs_building6`): trimming first hands both the same rate.
        let declared_pan = |name: &str| -> Option<(f64, f64)> {
            let whole = plain(name);
            let n = tail(name);
            // ...and only where it STATES a rate. A declaration that names the piece and says
            // nothing else leaves the rate to the engine, which is not the same as saying zero:
            // read as zero, battlefield's jungle stopped scrolling with the camera entirely.
            cam_backgrounds.iter()
                .find(|b| plain(&b.linkage_id) == whole)
                .or_else(|| cam_backgrounds.iter()
                    .find(|b| { let l = tail(&b.linkage_id); n == l || l.ends_with(&n) || n.ends_with(&l) }))
                .filter(|b| !b.auto_pan)
                .and_then(|b| Some((b.x_pan?, b.y_pan?)))
        };
        let (plates, mut moving) = (Vec::new(), Vec::new());
        // A camera background is ported as the source's own TIMELINE (see `timeline_tracks`):
        // one image per shape, moved / turned / faded by keyframes, which is both how the source
        // draws it and the only form that can carry its fades and its flash.
        let cam_layers = {
            // A camera background is assembled in the SOURCE's own units, positions included: the
            // camera sizes the whole layer from `originalBGWidth` x the multiplier and draws its
            // pictures at scale 1, so the stage scale belongs to the layer, not inside it. Scaling
            // only the POSITIONS spreads a clip's parts a third further apart than they are drawn.
            let tracks = timeline_tracks(&sampled, &cam_member, ox, oy, scale, shape_defs, bitmaps, Some(&seg), &clip_len);
            let mut walked = if tracks.is_empty() { group_bg_layers(&cam_member, true) } else { tracks };
            // THE DECLARATION IS THE BACKGROUND, in its own order. The package lists its camera
            // backgrounds back to front and gives each one its rate; the timeline walk finds
            // whatever happens to be placed, in whatever order the tree draws it. Led by the walk,
            // crateria's wall -- declared last, so nearest -- came out fifth of eight and drew at
            // the depth of a mountain. So the declared list is laid down first and in its order,
            // and what the walk found of the same clip gives way to it (its copy carries the tree's
            // own transform, which the game never applies to a camera background).
            let covers = |l: &BgLayer, d: &BgLayer| {
                let (a, b) = (plain_name(&l.name), plain_name(&d.name));
                a == b || a.ends_with(&b) || b.ends_with(&a)
                    || { let c = plain_name(&l.clip); c == b || c.ends_with(&b) || b.ends_with(&c) }
            };
            walked.retain(|l| !declared_cam.iter().any(|d| covers(l, d)));
            let mut v = declared_cam.clone();
            // anything on a camera-background plane the package never declared still belongs to the
            // backdrop; it goes in front of what was declared, which is where the tree drew it
            v.extend(walked);
            v
        };
        // The rate this background scrolls at, for the layers the package does not name one for.
        // ONE rate for the whole background, taken from its biggest picture: the estimate is a
        // function of size, so per-piece it would scroll the parts of one thing at as many speeds
        // as they have sizes and slide them apart as the camera moves.
        let widest = cam_layers.iter().flat_map(|l| l.frames.iter())
            .max_by_key(|f| f.w as u64 * f.h as u64).map(|f| (f.w, f.h));
        let fallback = widest.map(|(w, h)| parallax_pan(w, h))
            .filter(|(x, y)| *x != 0.0 || *y != 0.0);
        for mut l in cam_layers {
            // THE RATE THE SOURCE DECLARES IS THE RATE, zero included: a background that takes none
            // of the camera's movement is the one pinned hardest to the view, and it covers the
            // view wherever the camera goes. Estimated only where the package declares nothing.
            let pan = declared_pan(&l.name).or(fallback);
            // EVERY object of this background scrolls with the background, whether or not it moves
            // on its own. Giving a still one its own plate hands it a pan of its own -- and with no
            // declared rate that is estimated from the PICTURE'S SIZE, so the parts of one thing
            // scroll at as many speeds as they have sizes and slide apart as the camera moves.
            // Clocktown's clock is concentric rings of different radius: emitted this way its rings
            // drift off its face and the sky shows through the gap between them.
            l.pan = pan;
            moving.push(l);
        }
        // the fixed near-background (the stageMC `background` plane) keeps its own elements
        // The near background is ported as the source's TIMELINE, the same as the plane in front of
        // it and the camera background behind it. Those planes are placed BY the stage's own
        // timeline -- clocktown's rain splashes are instances it puts down for the frames it rains
        // and takes away again -- so an object cut loose onto a clock of its own keeps splashing
        // through the dry hours. The composite stays as the fallback for a plane this cannot split.
        let near_member = |i: &Instance| art_kind(i) == ArtKind::Background;
        let mut bg = moving;
        let near_tracks = timeline_tracks(&sampled, &near_member, ox, oy, scale, shape_defs, bitmaps, Some(&seg), &clip_len);
        if near_tracks.is_empty() { bg.extend(group_bg_layers(&near_member, false)); }
        else { bg.extend(near_tracks); }
        (bg, plates)
    } else {
        // No parallax: each backdrop/background ELEMENT (lava, torches, embers, podoboos, ...)
        // becomes its own layer on its own loop, in back-to-front draw order, so an animated SSF2
        // backdrop element actually animates without forcing every other element onto one shared
        // composite timeline. The folded structure-foreground (e.g. bowserscastle's stone walkway)
        // rides along as its own element, drawn after the backdrop it sits on.
        let bg_member = |i: &Instance| matches!(art_kind(i), ArtKind::Backdrop | ArtKind::Background);
        (group_bg_layers(&bg_member, false), Vec::new())
    };

    // Emit a multi-frame stage animation only when the samples form a CLEAN animation:
    // every frame well-populated (no blinking on/off) and at least two distinct images.
    // Otherwise (static, or a chaotic/long SSF2 timeline) use the single richest frame.
    // AN ELEMENT ON THE STAGE'S OWN PLANE ANIMATES TOO (see below for why the plane then skips it).
    let (stage_elements, moved_keys): (Vec<BgLayer>, std::collections::BTreeSet<(u64, u16)>) = {
        // the same test the painted plane applies: a collision piece or a marker is never art,
        // however it moves. castle siege's terrain sections travel during a transition, and they
        // came out as drawn objects, so the stage flickered between its art and its collision.
        let member = |i: &Instance| art_kind(i) == ArtKind::Stage
            && !is_non_art(i.inst_name.as_deref().unwrap_or("")) && !is_non_art(&i.sym_name);
        let mut keys = Vec::new();
        let tracks = timeline_tracks_keyed(&sampled, &member, ox, oy, scale, shape_defs, bitmaps, &mut keys, Some(&seg), &clip_len, None);
        let (mut kept, mut moved) = (Vec::new(), std::collections::BTreeSet::new());
        for (l, k) in tracks.into_iter().zip(keys) {
            // what moves is an object of its own; so is a piece the source HIDES until the stage
            // shows it by name, and one the stage plays by label: painting those into the plane
            // draws them before their time, and a plane whose pieces change by phase came out
            // as hundreds of full-size pictures (galaxy tours' terrains, 458 megapixels).
            if l.frames.len() > 1 || l.starts_hidden || !l.segments.is_empty() { moved.insert(k); kept.push(l); }
        }
        (kept, moved)
    };
    // WHAT WAS SPLIT OUT IS NOT PAINTED IN as well. An element on the stage's own plane that moves
    // comes out as its own object below, and the plane was still composited from everything --
    // so the moving piece existed twice, and the painted copy dragged the whole plane along with
    // it. Mushroom kingdom 3's plane came out 1050 pictures on that account, which is more assets
    // than the publish step can carry, so the stage never shipped.
    // (one entry per sample: a plane whose every piece was split out paints NOTHING, and must not
    // fall back to the full composite, which has the split pieces in it)
    let split = !moved_keys.is_empty();
    let plane_only: Vec<Option<StageArt>> = if !split {
        Vec::new()
    } else {
        sampled.iter().map(|(insts, _)| {
            let kept: Vec<Instance> = insts.iter()
                .filter(|i| !moved_keys.contains(&(i.inst_path, i.leaf_depth)))
                .cloned().collect();
            if std::env::var("PEPTIDE_STAGE_DEBUG").is_ok() {
                let st: Vec<String> = kept.iter().filter(|i| art_kind(i) == ArtKind::Stage)
                    .map(|i| format!("{}/{}/{:x}:{}", i.inst_name.as_deref().unwrap_or("-"), i.sym_name, i.inst_path, i.leaf_depth)).collect();
                eprintln!("[plane-only] stage kept={} moved_keys={} {}", st.len(), moved_keys.len(), st.join(" "));
            }
            composite(&kept, &[ArtKind::Stage])
        }).collect()
    };
    let imgs: Vec<&StageArt> = if !split {
        sampled.iter().filter_map(|(_, img)| img.as_ref()).collect()
    } else {
        plane_only.iter().flatten().collect()
    };
    let clean = max_count > 0
        && stage_counts.iter().all(|&c| c * 5 >= max_count * 4)   // >=80% of max in every frame
        && imgs.len() == n_samples
        && imgs.windows(2).any(|w| w[0].png != w[1].png);          // some motion
    // AN ELEMENT ON THE STAGE'S OWN PLANE ANIMATES TOO. The plane is painted as one picture, so
    // an element on it only kept its animation when the WHOLE plane animated in step -- ten
    // targets each on its own loop read as chaos, the plane fell back to a single frame, and every
    // one of them stopped moving. The same split the backdrop and the foreground already get:
    // whatever moves comes out as its own element on its own loop, and the rest stays painted.
    let stage_frames: Vec<StageArt> = if clean {
        // truncate to the animation's OWN loop period (sampled over the stage's longest clip), so a
        // stage with a long looping timeline emits ONE loop, not the whole multi-thousand-frame
        // sweep (gangplankgalleon's sea animation was 2651 frames). loop_period returns the full
        // length if it never repeats, so a genuinely non-looping stage is unaffected.
        let frames: Vec<StageArt> = imgs.into_iter().cloned().collect();
        let p = loop_period(&frames);
        frames[..p].to_vec()
    } else if split {
        plane_only[base_idx].clone().into_iter().collect()
    } else {
        sampled[base_idx].1.clone().into_iter().collect()
    };

    // engine-added background layers: orphan library bitmaps the SSF2 engine instantiates as named
    // bg children (e.g. bowserscastle's standable brick bridge). Placed centred on the backdrop;
    // appended frontmost-of-background (in front of the painted lava, behind the fighters).
    let backdrop_bounds = {
        let bg: Vec<&Instance> = base_insts.iter()
            .filter(|i| matches!(art_kind(i), ArtKind::Backdrop | ArtKind::Background))
            .collect();
        (!bg.is_empty()).then(|| (
            bg.iter().map(|i| i.aabb.x).fold(f64::MAX, f64::min),
            bg.iter().map(|i| i.aabb.y).fold(f64::MAX, f64::min),
            bg.iter().map(|i| i.aabb.x + i.aabb.w).fold(f64::MIN, f64::max),
            bg.iter().map(|i| i.aabb.y + i.aabb.h).fold(f64::MIN, f64::max),
        ))
    };
    let mut background = background;
    // engine-added orphan bitmaps: standable bg layers (the bridge deck) PLUS any foreground
    // occluder pieces SSF2 splits off them (the near parapet that draws in front of the fighter).
    let (eng_bg, foreground_occluders) = engine_added_bg_layers(root_tags, shape_defs, bitmaps, sprites, base_insts, backdrop_bounds, surface_y_fm, ox, oy, scale);
    background.extend(eng_bg);

    // background is already grouped into per-element layers (each RLE'd at the 30->60fps doubling);
    // the stage plane still needs the same RLE pass so a held source frame reads as a break.
    // ── bake the blended placements into everything beneath them ──────────────
    // The mask is composited onto the art it sits over and then dropped, so the shipped stage
    // carries the blend already applied. Exact for static art; anything that MOVES under the mask
    // (fighters, hazards) is tinted in SSF2 and cannot be here, because the engine exposes no
    // runtime blend mode. See `bake_hardlight`.
    let (mut background, mut stage_frames) = (background, stage_frames);
    let (mut foreground, mut foreground_elements) = (foreground, foreground_elements);
    let mut foreground_occluders = foreground_occluders;
    let mut foreground_is_blend_overlay = false;
    let blend_insts: Vec<&Instance> = base_insts.iter()
        .filter(|i| art_kind(i) == ArtKind::Foreground && is_blended(i)).collect();
    if !blend_insts.is_empty() {
        let n_blend = blend_insts.len();
        let bounds = union_bounds(&blend_insts);
        if let Some(mask) = composite_grp(blend_insts, bounds) {
            // Solve the blend against the art beneath it and ship the result as a normal alpha
            // overlay in the FOREGROUND, which is where SSF2 draws it: over everything, fighters
            // included. Baking it into the art instead is exact but tints only what was baked, and
            // the layer people actually watch has characters running around inside it.
            let beneath: Vec<&Instance> = base_insts.iter().filter(|i| !is_blended(i)).collect();
            let base_art = composite_grp(beneath, bounds);
            if std::env::var("PEPTIDE_STAGE_DEBUG").is_ok() {
                match &base_art {
                    None => eprintln!("[blend-base] no art beneath the mask to solve against"),
                    Some(b) => {
                        let cov = image::load_from_memory(&b.png).map(|i| {
                            let im = i.to_rgba8();
                            let n = im.pixels().len().max(1);
                            100.0 * im.pixels().filter(|p| p[3] > 0).count() as f64 / n as f64
                        }).unwrap_or(-1.0);
                        eprintln!("[blend-base] base {}x{} at ({:.1},{:.1}) covers {cov:.1}% \
                            (mask {}x{} at ({:.1},{:.1}))", b.w, b.h, b.x, b.y, mask.w, mask.h,
                            mask.x, mask.y);
                    }
                }
            }
            let fitted = base_art.and_then(|base| fit_blend_overlay(&mask, &base));
            let mode = match fitted {
                Some(overlay) => {
                    foreground.push(overlay);
                    foreground_is_blend_overlay = true;
                    "overlay (moving objects tinted too)"
                }
                // no art underneath to solve against: fall back to compositing the blend into
                // whatever IS there, which is exact for that art and tints nothing else.
                None => {
                    for a in stage_frames.iter_mut() { bake_hardlight(a, &mask); }
                    for l in background.iter_mut() {
                        for a in l.frames.iter_mut() { bake_hardlight(a, &mask); }
                    }
                    for l in foreground_elements.iter_mut() {
                        for a in l.frames.iter_mut() { bake_hardlight(a, &mask); }
                    }
                    for a in foreground_occluders.iter_mut() { bake_hardlight(a, &mask); }
                    "baked into the art (static only)"
                }
            };
            log::info!("stage art: {n_blend} blended placement(s) -> {mode}; the engine exposes no \
                runtime blend mode, so a chromatic mask can only be approximated over moving art");
            if std::env::var("PEPTIDE_STAGE_DEBUG").is_ok() {
                eprintln!("[blend-bake] {n_blend} placement(s); mask {}x{} at ({:.1},{:.1}) \
                    -> {mode}; mask chroma {:.1}/255 (0 = greyscale)",
                    mask.w, mask.h, mask.x, mask.y, mask_chroma(&mask));
            }
        }
    }
    // The shadow mask is not art the stage draws -- SSF2 never shows it -- but it is not
    // scaffolding to throw away either: it is the SHAPE that says where a fighter's shadow falls,
    // and Fraymakers takes exactly that as a mask sprite on a shadow layer. So it is rasterised
    // like anything else and carried out separately from what gets drawn.
    // The stage's OWN light. SSF2 parks a `light_source` clip in the plane it never draws, which is
    // where the shadows it casts are aimed from -- so the light does not have to be guessed at from
    // the stage's outline. Read like the shadow mask: out of a plane that carries no art.
    let light_source = {
        let root = &root_frames[root_idx];
        let mut out = Vec::new();
        walk_frame(root, root_m, base_idx, None, None, planes, sym_names, shape_defs,
                   &sprite_frames, &mut out, 0, (0.0, 0.0), 0, None, &phase_rank, None, 1.0,
                   crate::swf_timeline::Tint::ID, &[], &labelled, &std::collections::BTreeSet::new(), false, &std::collections::BTreeSet::new(), false, false, &[], 1);
        out.iter()
            .find(|i| i.sym_name.to_ascii_lowercase().contains("light_source"))
            .map(|i| (((i.cx - ox) * scale), ((i.cy - oy) * scale)))
    };
    if let Some((x, y)) = light_source {
        log::info!("stage light source: ported from the source's own light clip at ({x:.0}, {y:.0})");
    }
    let shadow_mask = {
        let mi = mask_instances(base_idx);
        let grp: Vec<&Instance> = mi.iter().collect();
        if grp.is_empty() { None } else {
            composite_layer(&grp, shape_defs, bitmaps, ox, oy, None).map(|mut a| {
                a.x *= scale; a.y *= scale; a
            })
        }
    };
    if shadow_mask.is_some() { log::info!("stage shadow mask: ported as a shadow layer"); }
    // A decoration the source authors INVISIBLE is one something else switches on. SSF2 registers
    // its ambient sheets, lights and weather with a quality system and that system raises their
    // alpha at the quality the game is running; the registration is a call FM has no counterpart
    // for, so the port drops it -- and anything waiting on it stays at the alpha it was authored
    // with, which is zero. Emitting that is emitting nothing: clocktown's five ambient lighting
    // sheets, the whole day-to-night wash and the red one the moon crashes under, all rasterised
    // correctly and all fully transparent.
    //
    // So a piece that is invisible for its WHOLE timeline is shown. It cannot be right as it
    // stands (nothing that is never drawn is worth emitting), and the state the source's own
    // default quality puts it in is on.
    // Gated: four measurements say clocktown's plates carry no strength in the file at all, so
    // showing them may be a difference FROM the source rather than toward it. The day/night wash
    // comes from the backdrop's white plate carried through its tint ramp, which the converter now
    // reproduces, so this rule may simply be double-tinting what is already right.
    let wake_hidden = std::env::var("PEPTIDE_WAKE_HIDDEN").is_ok();
    let mut woke = 0usize;
    for layer in background.iter_mut().chain(foreground_elements.iter_mut()) {
        let drawn: Vec<&mut StageArt> = layer.frames.iter_mut().filter(|f| f.w > 1 || f.h > 1).collect();
        if !wake_hidden || drawn.is_empty() || drawn.iter().any(|f| f.alpha > 0.001) { continue; }
        woke += 1;
        for f in drawn { f.alpha = 1.0; }
    }
    if woke > 0 {
        log::warn!("{woke} decoration(s) are authored fully transparent and switched on by a \
                    quality system FM has no counterpart for; shown at full opacity, which is \
                    what the source's own default quality does");
    }
    StageArtSet { background, parallax, stage_elements, stage_frames: rle(stage_frames), foreground,
        foreground_elements, foreground_occluders, foreground_is_blend_overlay, foreground_alpha,
        shadow_mask, light_source, runtime_clips: Vec::new(),
        deferred_event_clips: deferred_event_clips.into_inner() }
}

/// Engine-added background layers: bitmaps in the stage library that NO shape fill, PlaceObject,
/// or timeline references. The SSF2 engine instantiates these at runtime as named background
/// children (e.g. bowserscastle's standable `bowsers_bridge_bg` brick walkway) — they live in the
/// stage's library but never in any movieclip, so the placement walk structurally can't see them.
/// We detect the orphan bitmaps directly and emit each distinct one as its own background layer,
/// centered on the backdrop at its standable height. Same-size orphans are one layer's animation
/// frames; tiny orphans (animation swap-frames of a placed bitmap) are skipped. Generalizes to any
/// stage built this way. `backdrop` is the raw-coord union bounds of the placed background art.
#[allow(clippy::too_many_arguments)]
fn engine_added_bg_layers(
    root_tags: &[swf::Tag],
    shape_defs: &BTreeMap<u16, &swf::Shape>,
    bitmaps: &BTreeMap<u16, (u32, u32, Vec<u8>)>,
    sprites: &BTreeMap<u16, &Vec<swf::Tag>>,
    instances: &[Instance],
    backdrop: Option<(f64, f64, f64, f64)>,
    surface_y_fm: Option<f64>,
    ox: f64, oy: f64, scale: f64,
) -> (Vec<BgLayer>, Vec<StageArt>) {
    use std::collections::{BTreeMap as Map, BTreeSet};
    let Some((bx0, by0, bx1, by1)) = backdrop else { return (Vec::new(), Vec::new()) };
    // every character id referenced by a shape fill / sprite placement / root placement, and the
    // pixel dims of every referenced bitmap (so a placed clip's animation swap-frame is excluded).
    let mut referenced: BTreeSet<u16> = BTreeSet::new();
    let mut ref_dims: BTreeSet<(u32, u32)> = BTreeSet::new();
    let note = |id: u16, bitmaps: &BTreeMap<u16, (u32, u32, Vec<u8>)>, refd: &mut BTreeSet<u16>, dims: &mut BTreeSet<(u32, u32)>| {
        refd.insert(id);
        if let Some((w, h, _)) = bitmaps.get(&id) { dims.insert((*w, *h)); }
    };
    for s in shape_defs.values() {
        for f in &s.styles.fill_styles {
            if let swf::FillStyle::Bitmap { id, .. } = f { note(*id, bitmaps, &mut referenced, &mut ref_dims); }
        }
    }
    for tags in sprites.values() {
        for frame in build_frames(tags) { for (cid, _, _, _, _, _, _, _, _, _) in frame { note(cid, bitmaps, &mut referenced, &mut ref_dims); } }
    }
    for t in root_tags {
        if let swf::Tag::PlaceObject(po) = t {
            if let swf::PlaceObjectAction::Place(id) | swf::PlaceObjectAction::Replace(id) = po.action { note(id, bitmaps, &mut referenced, &mut ref_dims); }
        }
    }
    // orphan + large + not a swap-frame of a placed bitmap; group equal-size orphans (frames).
    let mut by_dim: Map<(u32, u32), Vec<u16>> = Map::new();
    for (id, (w, h, _)) in bitmaps.iter() {
        if !referenced.contains(id) && w * h > 50_000 && !ref_dims.contains(&(*w, *h)) {
            by_dim.entry((*w, *h)).or_default().push(*id);
        }
    }
    if let Ok(ids) = std::env::var("PEPTIDE_DUMP_BMPS") {
        for id in ids.split(',').filter_map(|s| s.trim().parse::<u16>().ok()) {
            if let Some((w, h, rgba)) = bitmaps.get(&id) {
                if let Some(im) = image::RgbaImage::from_raw(*w, *h, rgba.clone()) {
                    let _ = im.save(format!("/tmp/bmp_{id}.png"));
                }
            }
        }
    }
    if std::env::var("PEPTIDE_ORPHAN_DEBUG").is_ok() {
        eprintln!("=== bitmap inventory ({} total) ===", bitmaps.len());
        for (id, (w, h, _)) in bitmaps.iter() {
            let orphan = !referenced.contains(id);
            let swap = ref_dims.contains(&(*w, *h));
            eprintln!("  id={id} {w}x{h} ({} px)  orphan={orphan} swapframe-dim={swap}{}",
                w * h, if orphan && w * h > 50_000 && !swap { "  <- ENGINE LAYER" } else { "" });
            if std::env::var("PEPTIDE_ORPHAN_DUMP").is_ok() && orphan && w * h > 50_000 && !swap {
                if let Some(im) = image::RgbaImage::from_raw(*w, *h, bitmaps[id].2.clone()) {
                    let _ = im.save(format!("/tmp/orphan_{id}.png"));
                }
            }
        }
    }
    // horizontal centre of the backdrop, in FM coords (the emitter scales the IMAGE by `scale`).
    let center_x_fm = ((bx0 + bx1) / 2.0 - ox) * scale;
    // the TRUE placement of an engine-bound orphan: SSF2 authors a placed shape whose bitmap fill
    // is a PLACEHOLDER (id 0xFFFF) the engine binds the orphan into at runtime (bowserscastle's
    // bridge: the background-plane shape hosts the deck piece, the foreground-plane shape the
    // parapet — at DIFFERENT y's, which is why "pixel-align the pair" mis-stacked them). Recover
    // the placeholder region by rasterizing a mask of just that fill and mapping its bbox through
    // the placed instance's world AABB; the region's dims must match the orphan's.
    // bbox of the subpaths painted by a GIVEN bitmap fill, walked straight off the shape records
    // (same conventions as `rasterize_shape`: bounds-relative px, absolute moveTo, delta edges,
    // mid-shape new-style arrays reset the fill indices). These "orphan" bitmaps are in fact
    // referenced — by fills inside mid-shape new-styles groups, which the top-level `referenced`
    // scan (and the composite's single-bitmap render path) don't see. The region IS the bitmap's
    // authored placement.
    fn bitmap_fill_bbox(shape: &swf::Shape, want_id: u16) -> Option<(f64, f64, f64, f64)> {
        let is_want = |fills: &[swf::FillStyle], idx: usize|
            idx > 0 && matches!(fills.get(idx - 1), Some(swf::FillStyle::Bitmap { id, .. }) if *id == want_id);
        let min_x = shape.shape_bounds.x_min.to_pixels();
        let min_y = shape.shape_bounds.y_min.to_pixels();
        let mut fills: &[swf::FillStyle] = &shape.styles.fill_styles;
        let (mut fs0, mut fs1) = (0usize, 0usize);
        let (mut px, mut py) = (-min_x, -min_y);
        let mut bb: Option<(f64, f64, f64, f64)> = None;
        fn extend(bb: &mut Option<(f64, f64, f64, f64)>, x: f64, y: f64) {
            *bb = Some(match *bb {
                None => (x, y, x, y),
                Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
            });
        }
        for rec in &shape.shape {
            match rec {
                swf::ShapeRecord::StyleChange(sc) => {
                    if let Some(ns) = &sc.new_styles { fills = &ns.fill_styles; fs0 = 0; fs1 = 0; }
                    if let Some(f) = sc.fill_style_0 { fs0 = f as usize; }
                    if let Some(f) = sc.fill_style_1 { fs1 = f as usize; }
                    if let Some(mv) = &sc.move_to { px = mv.x.to_pixels() - min_x; py = mv.y.to_pixels() - min_y; }
                    if is_want(fills, fs0) || is_want(fills, fs1) { extend(&mut bb, px, py); }
                }
                swf::ShapeRecord::StraightEdge { delta } => {
                    px += delta.dx.to_pixels(); py += delta.dy.to_pixels();
                    if is_want(fills, fs0) || is_want(fills, fs1) { extend(&mut bb, px, py); }
                }
                swf::ShapeRecord::CurvedEdge { control_delta, anchor_delta } => {
                    let cx = px + control_delta.dx.to_pixels(); let cy = py + control_delta.dy.to_pixels();
                    px = cx + anchor_delta.dx.to_pixels(); py = cy + anchor_delta.dy.to_pixels();
                    if is_want(fills, fs0) || is_want(fills, fs1) { extend(&mut bb, cx, cy); extend(&mut bb, px, py); }
                }
            }
        }
        bb
    }
    // the authored FM placement of bitmap `id`: the fill region painting it in any placed shape,
    // mapped through that instance's world AABB.
    let bitmap_host = |id: u16| -> Option<(f64, f64)> {
        for inst in instances {
            let Some(shape) = shape_defs.get(&inst.shape_id) else { continue };
            let Some((x0, y0, _x1, _y1)) = bitmap_fill_bbox(shape, id) else { continue };
            let shape_w = (shape.shape_bounds.x_max - shape.shape_bounds.x_min).to_pixels();
            let shape_h = (shape.shape_bounds.y_max - shape.shape_bounds.y_min).to_pixels();
            if shape_w <= 0.0 || shape_h <= 0.0 { continue; }
            let (fx, fy) = (
                (inst.aabb.x + x0 * (inst.aabb.w / shape_w) - ox) * scale,
                (inst.aabb.y + y0 * (inst.aabb.h / shape_h) - oy) * scale,
            );
            if std::env::var("PEPTIDE_STAGE_DEBUG").is_ok() {
                eprintln!("[bmp-host] bitmap {id}: shape {} plane {:?} region@({x0:.0},{y0:.0}) -> FM ({fx:.1},{fy:.1})",
                    inst.shape_id, inst.plane);
            }
            return Some((fx, fy));
        }
        None
    };
    // opaque coverage of bitmap `id`'s row `y` (fraction of the width that's solid).
    let row_cov = |id: &u16, y: u32| -> f64 {
        let (bw, bh, rgba) = &bitmaps[id];
        if y >= *bh { return 0.0; }
        let op = (0..*bw).filter(|&x| rgba[((y * *bw + x) * 4 + 3) as usize] > 40).count();
        op as f64 / *bw as f64
    };
    // the topmost row of bitmap `id` opaque across most of the width (its standable surface).
    let surf_row = |id: &u16| -> u32 {
        let (_, bh, _) = &bitmaps[id];
        (0..*bh).find(|&y| row_cov(id, y) > 0.5).unwrap_or(0)
    };
    let to_art = |id: &u16, x_fm: f64, y_fm: f64| -> Option<StageArt> {
        let (bw, bh, rgba) = &bitmaps[id];
        let im = image::RgbaImage::from_raw(*bw, *bh, rgba.clone())?;
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(im).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
        Some(StageArt { png, x: x_fm, y: y_fm, w: *bw, h: *bh, hold: 1, rotation: 0.0, scale_x: 1.0, scale_y: 1.0 , alpha: 1.0 , pivot: (0.0, 0.0), tint: TINT_ID })
    };
    let mut bg_layers: Vec<BgLayer> = Vec::new();
    let mut fg_arts: Vec<StageArt> = Vec::new();
    for (li, ((w, _h), ids)) in by_dim.into_iter().enumerate() {
        let x_fm = center_x_fm - w as f64 * scale / 2.0;
        // DETECT the walkway's standable surface: the topmost row opaque across most of the width
        // (the flat brick deck; the banner/throne above it are narrow). The BACKGROUND piece is the
        // one solid at that deck row (fighters stand ON it); align THAT to the main collision floor.
        // Fall back to the backdrop's vertical centre when there's no floor.
        // Pick the deck row from whichever same-size orphan has the highest (topmost) solid deck.
        let deck_row = ids.iter().map(&surf_row).min().unwrap_or(0);
        // SSF2 splits a standable structure (e.g. bowscastle's bridge) into a background piece
        // (solid deck, drawn behind the fighter) and a foreground piece (the near parapet, deck cut
        // OUT so the fighter shows through, drawn in front to occlude their feet). Both are orphan
        // bitmaps of the same size. Classify each by its coverage at the deck row: solid -> behind,
        // cut-out -> in front. Animation-frame groups (all solid) keep one background layer.
        let probe = (deck_row + 4).min(bitmaps[&ids[0]].1.saturating_sub(1));
        let mut bg_id = ids[0];
        let mut bg_cov = row_cov(&bg_id, probe);
        for id in &ids {
            let c = row_cov(id, probe);
            if c > bg_cov { bg_cov = c; bg_id = *id; }
        }
        // background = the deck-solid piece, at its AUTHORED fill-region placement when a placed
        // shape paints it (the authoritative position), else anchored to the collision floor.
        let (bg_x, bg_y) = bitmap_host(bg_id).unwrap_or_else(|| {
            let y_fm = match surface_y_fm {
                Some(s) => s - surf_row(&bg_id) as f64 * scale,
                None => ((by0 + by1) / 2.0 - oy) * scale,
            };
            (x_fm, y_fm)
        });
        if let Some(art) = to_art(&bg_id, bg_x, bg_y) {
            bg_layers.push(BgLayer { named_path: Vec::new(), behind_base: false, presence: Vec::new(), name: format!("engineLayer{li}"), clip: format!("engineLayer{li}"), frames: vec![art], segments: Vec::new(), pan: None, fades: false, authored_alpha: 1.0, starts_hidden: false });
        }
        // foreground = any same-size sibling whose deck row is clearly CUT OUT (a near parapet that
        // must draw IN FRONT of the fighter), at ITS OWN authored placement, falling back to the
        // background piece's.
        for id in &ids {
            if *id != bg_id && row_cov(id, probe) < 0.25 && bg_cov > 0.4 {
                let (fx, fy) = bitmap_host(*id).unwrap_or((bg_x, bg_y));
                if let Some(art) = to_art(id, fx, fy) { fg_arts.push(art); }
            }
        }
    }
    (bg_layers, fg_arts)
}

/// spanning their union bounds. `None` if nothing rasterized.
/// A 1x1 fully transparent PNG: the canvas for an element frame where nothing is visible.
/// The bytes of the shared one-pixel transparent picture, for the emitter to ship once.
pub fn blank_png_bytes() -> Vec<u8> { blank_png() }

fn blank_png() -> Vec<u8> {
    use image::ImageEncoder;
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(&[0u8, 0, 0, 0], 1, 1, image::ExtendedColorType::Rgba8)
        .expect("encode 1x1 png");
    png
}

/// `imageops::resize` that skips the filter when the target size already matches.
///
/// The image crate runs the full sampling filter regardless of whether the dimensions
/// change, and stage compositing calls it two to three times per placed instance — so
/// every 1:1 placement was paying for a resample that produces its input. Compositing is
/// about half of a stage parse and this is the hottest call in it.
fn resize_to(img: &image::RgbaImage, tw: u32, th: u32) -> image::RgbaImage {
    if img.width() == tw && img.height() == th {
        return img.clone();
    }
    image::imageops::resize(img, tw, th, image::imageops::FilterType::Triangle)
}


/// An element that is a FEW shapes being moved or turned, as art-once-per-shape plus placements.
///
/// SSF2 stores a turning clock ring as a bitmap and a matrix per frame; rasterising each frame
/// separately stores it as one bitmap PER ANGLE. This detects the case and emits what the source
/// has -- each distinct shape rasterised once, with the per-frame orientation carried on the
/// frames, which is the same shape the character path emits for a rotating limb. Frames sharing a
/// shape share the art, so the writer's dedupe collapses them to one file.
///
/// Returns None for anything else (several shapes at once on a frame, a shape whose content
/// changes, or more distinct shapes than this can pay for), which the compositor still bakes.
#[allow(clippy::too_many_arguments)]
/// This object's instances per frame, one list per sampled frame, in the order the walk lists them.
///
/// SEVERAL at once is not a failure: a clip drawing many of a thing at the same time (a burst of
/// rain splashes, a torch's embers) is many objects, and slot `k` of each frame is one of them.
/// Taking a slot each and placing it is how those come out as the objects they are, instead of
/// being drawn together into one picture per frame.
fn frame_slots<'a>(
    sampled: &'a [(Vec<Instance>, Option<StageArt>)],
    matches: &dyn Fn(&Instance) -> bool,
) -> Vec<Vec<&'a Instance>> {
    sampled.iter().map(|(insts, _)| insts.iter().filter(|i| matches(i)).collect()).collect()
}

/// How long an absence has to be before it counts as the STAGE taking a clip away. A clip blinking
/// through its own animation is absent for a frame or two at a time -- an empty cel, or a piece
/// placed on its own cycling timeline -- and reading that as the stage both chops the clip's motion
/// down to whatever showed in one blink and switches it on and off a few hundred times a day. An
/// absence the stage means is measured in seconds.
const REAL_GAP: usize = 60;

/// A colour transform that changes nothing.
pub const TINT_ID: [f64; 6] = [1.0, 1.0, 1.0, 0.0, 0.0, 0.0];

/// The rgb half of a placement's colour transform, in the order the engine's shader takes.
fn tint_of(t: &crate::swf_timeline::Tint) -> [f64; 6] {
    [t.mul[0], t.mul[1], t.mul[2], t.add[0], t.add[1], t.add[2]]
}

fn single_shape_frames(
    // this object's instance in each sampled frame (`None` where the timeline does not draw it).
    // Handed in rather than matched here, so a caller holding SEVERAL of a thing at once can split
    // them into one of these per slot and place each, instead of drawing them into one picture.
    per_frame: &[Option<&Instance>],
    ox: f64, oy: f64, scale: f64,
    shape_defs: &BTreeMap<u16, &swf::Shape>,
    bitmaps: &BTreeMap<u16, (u32, u32, Vec<u8>)>,
) -> Option<Vec<StageArt>> {
    /// A ceiling on distinct cels, so a pathological element cannot emit hundreds of rasters.
    /// Clocktown's clock rings are ten cels that also turn.
    const MAX_SHAPES: usize = 32;
    let all: Vec<&Instance> = per_frame.iter().flatten().copied().collect();
    if all.is_empty() { return None; }
    let shapes: std::collections::BTreeSet<u16> = all.iter().map(|i| i.shape_id).collect();
    let dbg = std::env::var("PEPTIDE_PLACE_DEBUG").is_ok();
    if dbg { eprintln!("[place] sym={:?} shapes={} ({:?})", all[0].sym_name, shapes.len(), shapes.iter().take(6).collect::<Vec<_>>()); }
    if shapes.len() > MAX_SHAPES { if dbg { eprintln!("[place]   reject: too many shapes"); } return None; }
    // A piece that never moves is built the same way, from its own picture at its own placement.
    // Sending it to the compositor instead draws it onto union bounds shared with whatever else is
    // in the group, which both moves its origin and pads it out with the empty space between them.
    let shown: Vec<&Instance> = all;
    // Rasterise each shape UPRIGHT, in the box it fills with its turn taken out.
    //
    // The compositor fits a shape to whatever box it is handed and never turns it, so handing it a
    // frame's own box squeezes the art into a footprint the source never draws -- a turned box is
    // wider and shorter than the piece in it. Taking the least-rotated frame only makes that
    // squeeze small; it removes it only at exact right angles. The turn belongs in the placement,
    // so the picture has to be the piece as drawn, at the size the source scales it to.
    let mut art_for: BTreeMap<u16, (StageArt, f64, f64, (f64, f64, f64, f64))> = BTreeMap::new();
    for sid in shapes {
        // any frame serves now that the bake is upright, so take the one drawn BIGGEST: every
        // other frame scales that picture down, which is sharper than scaling one up
        let best = shown.iter().copied().filter(|i| i.shape_id == sid)
            .max_by(|a, b| (a.place.sx.abs() * a.place.sy.abs())
                .partial_cmp(&(b.place.sx.abs() * b.place.sy.abs())).unwrap_or(std::cmp::Ordering::Equal))?;
        let sb = &shape_defs.get(&sid)?.shape_bounds;
        let (bw, bh) = ((sb.x_max - sb.x_min).to_pixels(), (sb.y_max - sb.y_min).to_pixels());
        // Both corners of the shape's bounds. Which one becomes the picture's TOP-LEFT depends on
        // whether the placement mirrors that axis: a mirrored placement draws the shape's far edge
        // on the near side.
        let bounds = (sb.x_min.to_pixels(), sb.y_min.to_pixels(), sb.x_max.to_pixels(), sb.y_max.to_pixels());
        let (uw, uh) = (bw * best.place.sx.abs(), bh * best.place.sy.abs());
        if !(uw > 0.0 && uh > 0.0) { if dbg { eprintln!("[place]   reject: shape has no upright size"); } return None; }
        let mut upright = (*best).clone();
        upright.aabb = Rect { x: best.cx - uw / 2.0, y: best.cy - uh / 2.0, w: uw, h: uh };
        upright.place.rotation = 0.0;
        // ...and UNMIRRORED. One picture serves every frame of this shape, so it cannot carry a
        // mirror belonging to the single frame it was baked from: every other frame would then be
        // transformed as if its own mirror were in the pixels, and a frame that disagrees comes out
        // a half turn wrong against a picture that never changed. The mirror is per-frame, so it
        // rides in the per-frame transform, and the picture stays the shape as drawn.
        upright.flip = (false, false);
        let art = match composite_layer(&[&upright], shape_defs, bitmaps, ox, oy, None) {
            Some(a) => a,
            None => { if dbg { eprintln!("[place]   reject: shape {sid} would not rasterise upright"); } return None; }
        };
        art_for.insert(sid, (art, bw, bh, bounds));
    }
    // one turning point for the whole clip, solved from the widest pair of angles this piece is
    // drawn at (the wider the pair, the better conditioned the solve)

    let anchor = shown[0];
    Some(per_frame.iter().map(|slot| match slot {
        Some(i) => {
            let (art, bw, bh, bnd) = &art_for[&i.shape_id];
            // Read the turn and the scale straight out of THIS frame's matrix. Reasoning from the
            // flip flags instead asks the picture what the frame is doing: the flags say what the
            // compositor did to the pixels, which it did once, for one frame, so every frame that
            // mirrors differently gets a half turn that is not in its matrix -- the same picture,
            // 180 degrees out, snapping back and forth mid-turn.
            //
            // A placement is a turn and a scale with at most a reflection in it, so it separates
            // exactly: the first column gives the angle and the width, and the second column, read
            // along that angle, gives a height whose SIGN is the reflection. Keeping the reflection
            // in that sign is what lets one unmirrored picture serve every frame.
            let (ct, st) = {
                let m = (i.place.a * i.place.a + i.place.b * i.place.b).sqrt();
                if m > 0.0 { (i.place.a / m, i.place.b / m) } else { (1.0, 0.0) }
            };
            let p_col = (i.place.a * i.place.a + i.place.b * i.place.b).sqrt();
            let q_col = i.place.d * ct - i.place.c * st;
            // Scale so the picture covers EXACTLY what the source draws: the shape's own bounds
            // under this frame's scale. Deriving it from the baked raster instead carries that
            // raster's rounding into every frame, and a piece drawn a fraction small leaves a
            // hairline against whatever it is supposed to meet.
            let (sx, sy) = (bw * p_col / (art.w as f64).max(1.0),
                            bh * q_col / (art.h as f64).max(1.0));
            // The picture's top-left is the shape's own near corner, always -- there is no mirror in
            // the pixels to pick a different one.
            let (lcx, lcy) = (bnd.0, bnd.1);
            let cx = (i.place.a * lcx + i.place.c * lcy + i.place.tx - ox) * scale;
            let cy = (i.place.b * lcx + i.place.d * lcy + i.place.ty - oy) * scale;
            StageArt {
                png: art.png.clone(),
                x: cx, y: cy,
                w: art.w, h: art.h, hold: 1,
                // ABSOLUTE: the picture is baked upright, so the frame's own angle is the whole
                // turn. Taking it relative to the frame the art came from (as if those pixels
                // carried that much turn) leans every frame by that frame's angle.
                rotation: crate::fm_placement::normalise_degrees(st.atan2(ct).to_degrees()),
                scale_x: sx, scale_y: sy, alpha: i.alpha, tint: tint_of(&i.tint),
                pivot: (0.0, 0.0) }
        }
        None => StageArt {
            png: blank_png(),
            x: (anchor.aabb.left() - ox) * scale,
            y: (anchor.aabb.top() - oy) * scale,
            w: 1, h: 1, hold: 1, rotation: 0.0, scale_x: 1.0, scale_y: 1.0, alpha: 1.0 , pivot: (0.0, 0.0), tint: TINT_ID },
    }).collect())
}

fn composite_layer(
    art: &[&Instance],
    shape_defs: &BTreeMap<u16, &swf::Shape>,
    bitmaps: &BTreeMap<u16, (u32, u32, Vec<u8>)>,
    ox: f64, oy: f64,
    fixed_bounds: Option<(f64, f64, f64, f64)>,
) -> Option<StageArt> {
    use image::{imageops, RgbaImage};

    // The bitmap id a shape fills with, if any (stage backgrounds are bitmap-filled
    // rects that `vector_raster` skips; we blit the bitmap into the shape's AABB).
    let shape_bitmap = |sid: u16| -> Option<(u16, swf::Matrix, bool)> {
        let shape = shape_defs.get(&sid)?;
        shape.styles.fill_styles.iter().find_map(|f| match f {
            // the fill MATRIX maps bitmap space onto the shape; without it the bitmap can only be
            // stretched to the shape's box, which is wrong for every fill that tiles or is placed
            // at an offset (see `bitmap_fill_tile`).
            swf::FillStyle::Bitmap { id, matrix, is_repeating, .. } if bitmaps.contains_key(id) =>
                Some((*id, *matrix, *is_repeating)),
            _ => None,
        })
    };

    if art.is_empty() { return None; }

    // union bounds (raw world coords) -> canvas. cap to keep the PNG sane. `fixed_bounds` (the
    // union across ALL frames of an animated layer) keeps every frame on one canvas so the layer
    // doesn't slide as a flickering element's per-frame bbox shifts (the "wiggle").
    let (min_x, min_y, max_x, max_y) = fixed_bounds.unwrap_or_else(|| (
        art.iter().map(|i| i.aabb.left()).fold(f64::MAX, f64::min),
        art.iter().map(|i| i.aabb.top()).fold(f64::MAX, f64::min),
        art.iter().map(|i| i.aabb.right()).fold(f64::MIN, f64::max),
        art.iter().map(|i| i.aabb.bottom()).fold(f64::MIN, f64::max),
    ));
    let cw = ((max_x - min_x).ceil() as u32).clamp(1, 4096);
    let ch = ((max_y - min_y).ceil() as u32).clamp(1, 4096);
    let mut canvas = RgbaImage::new(cw, ch);

    // CEIL, the same way the canvas is measured. Rounding the tile while the canvas is ceiled
    // leaves the art up to a pixel short of its own raster, and a piece drawn a pixel small opens a
    // hairline where it meets the piece around it -- which is a ring of sky between a clock's face
    // and the band that closes on it.
    let tw_th = |i: &Instance| (
        (i.aabb.w.ceil() as u32).clamp(1, 4096),
        (i.aabb.h.ceil() as u32).clamp(1, 4096),
    );
    // Tile production (resize + mask rasterize + alpha multiply + flip) is independent per
    // placed instance and is where a stage parse spends most of its time, so it runs across
    // all cores here. Compositing STAYS serial and in list order below: overlay and the
    // hard-light path both read-modify-write the shared canvas, so their order is the layer's
    // draw order and must not change. Output is therefore identical to the serial version.
    struct Tile {
        img: image::RgbaImage,
        ox: i64,
        oy: i64,
        blend: Option<swf::BlendMode>,
    }

    let make_tile = |inst: &&Instance| -> Option<Tile> {
        let (tw, th) = tw_th(inst);
        if let Ok(want) = std::env::var("PEPTIDE_SHAPE_TRACE") {
            if want == "all" || want.split(',').any(|x| x.trim().parse::<u16>() == Ok(inst.shape_id)) {
                eprintln!("[shape-inst] shape {} at world=({:.0},{:.0}) aabb={:.0}x{:.0} \
                    flip={:?} x_sign={} plane={:?}", inst.shape_id, inst.cx, inst.cy,
                    inst.aabb.w, inst.aabb.h, inst.flip, inst.x_sign, inst.plane);
            }
        }
        // The placed tile at the AABB size: a bitmap fill clipped to the shape OUTLINE, else
        // the vector rasterization.
        let scaled: Option<RgbaImage> = if shape_bitmap(inst.shape_id).is_some() {
            // every bitmap fill in the shape, each through its own matrix and sub-paths
            let shape = shape_defs.get(&inst.shape_id).unwrap();
            let bmp = bitmap_fill_layers(shape, bitmaps, tw, th)?;
            Some(bmp)
        } else {
            let shape = shape_defs.get(&inst.shape_id).unwrap();
            crate::vector_raster::rasterize_shape(
                &shape.shape_bounds, &shape.styles.fill_styles, &shape.styles.line_styles, &shape.shape,
            ).and_then(|r| RgbaImage::from_raw(r.width, r.height, r.rgba))
                .map(|t| resize_to(&t, tw, th))
        };
        let mut scaled = scaled?;
        if scaled.width() == 0 || scaled.height() == 0 { return None; }
        // The tile is rasterized in source orientation and placed onto the axis-aligned AABB; a
        // mirrored placement (scaleX/scaleY < 0) must flip the tile so it lands the same way it
        // does in SSF2. For a single static shape this is invisible (the AABB is unchanged), but a
        // mirrored MULTI-FRAME element whose frames are off-center crops (a wall torch's flame
        // bitmaps) needs it: un-flipped, each frame's content sits at a different spot in its AABB
        // and the element appears to slide left/right instead of animating in place.
        let (fx, fy) = inst.flip;
        if fx { scaled = imageops::flip_horizontal(&scaled); }
        if fy { scaled = imageops::flip_vertical(&scaled); }
        let mut ox_px = (inst.aabb.left() - min_x).round() as i64;
        let mut oy_px = (inst.aabb.top() - min_y).round() as i64;
        // A filter spreads OUTSIDE the shape it is on, so the tile grows and its corner moves.
        // Flash states the spread in the art's own pixels, and this tile is already at its placed
        // size, so the spread is scaled the same way the art was.
        if !inst.filters.is_empty() {
            let shape = shape_defs.get(&inst.shape_id).unwrap();
            let b = &shape.shape_bounds;
            let (sw, sh) = ((b.x_max.get() - b.x_min.get()) as f64 / 20.0,
                            (b.y_max.get() - b.y_min.get()) as f64 / 20.0);
            let sx = if sw > 0.0 { scaled.width() as f64 / sw } else { 1.0 };
            let sy = if sh > 0.0 { scaled.height() as f64 / sh } else { 1.0 };
            let (img, dx, dy) = crate::swf_filter::apply(&scaled, &inst.filters, sx, sy);
            scaled = img; ox_px += dx; oy_px += dy;
        }
        Some(Tile { img: scaled, ox: ox_px, oy: oy_px, blend: inst.blend })
    };

    // fan out over contiguous chunks so results come back already in list order
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(art.len()).max(1);
    let chunk = art.len().div_ceil(threads);
    let tiles: Vec<Option<Tile>> = std::thread::scope(|scope| {
        let handles: Vec<_> = art
            .chunks(chunk)
            .map(|part| scope.spawn(|| part.iter().map(&make_tile).collect::<Vec<_>>()))
            .collect();
        handles.into_iter().flat_map(|h| h.join().unwrap()).collect()
    });

    let mut drew = false;
    for tile in tiles.into_iter().flatten() {
        let Tile { img: scaled, ox: ox_px, oy: oy_px, blend } = tile;
        if blend == Some(swf::BlendMode::HardLight) {
            // SSF2 draws this placement with HardLight (bowserscastle's lightmask gradient over
            // the lava). FM stage layers have no blend modes, so emulate in the bake: hard-light
            // into pixels this composite already painted; where the canvas is empty (the layer
            // sits over the live scene) fall back to normal alpha as the approximation.
            let hl = |b: f64, t: f64| if t <= 0.5 { 2.0 * b * t } else { 1.0 - 2.0 * (1.0 - b) * (1.0 - t) };
            for (x, y, sp) in scaled.enumerate_pixels() {
                let (cx, cy) = (ox_px + x as i64, oy_px + y as i64);
                if cx < 0 || cy < 0 || cx >= cw as i64 || cy >= ch as i64 { continue; }
                let dp = canvas.get_pixel_mut(cx as u32, cy as u32);
                let sa = sp[3] as f64 / 255.0;
                if sa <= 0.0 { continue; }
                let da = dp[3] as f64 / 255.0;
                for ch_i in 0..3 {
                    let b = dp[ch_i] as f64 / 255.0;
                    let t = sp[ch_i] as f64 / 255.0;
                    // blend result over painted base, plain alpha over unpainted (da lerps them)
                    let blended = hl(b, t) * da + t * (1.0 - da);
                    dp[ch_i] = ((b * da * (1.0 - sa) + blended * sa).min(1.0) * 255.0
                        / (da * (1.0 - sa) + sa).max(1e-6)) as u8;
                }
                dp[3] = (((da * (1.0 - sa) + sa) * 255.0).min(255.0)) as u8;
            }
        } else {
            imageops::overlay(&mut canvas, &scaled, ox_px, oy_px);
        }
        drew = true;
    }
    if !drew { return None; }

    // A PICTURE TOO BIG FOR A TEXTURE IS SHIPPED SMALLER AND DRAWN BACK UP. Past 2048 px on a
    // side the engine draws a frame at a crawl (pokemon colosseum's foreground shade, eleven
    // frames of 2670 x 2209, ran the match at three frames a second), so such a frame shrinks
    // by the smallest whole factor that fits and carries that factor as its placement scale:
    // the same picture over the same box, at a size the engine draws at speed.
    const MAX_TEXTURE_PX: u32 = 2048;
    let factor = (cw.max(ch) as f64 / MAX_TEXTURE_PX as f64).ceil().max(1.0) as u32;
    let (canvas, cw, ch) = if factor > 1 {
        let (nw, nh) = ((cw / factor).max(1), (ch / factor).max(1));
        log::info!("a {cw}x{ch} frame ships at {nw}x{nh}, drawn {factor}x");
        (imageops::resize(&canvas, nw, nh, imageops::FilterType::Triangle), nw, nh)
    } else { (canvas, cw, ch) };
    let mut png = Vec::new();
    {
        use image::ImageEncoder;
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(canvas.as_raw(), cw, ch, image::ExtendedColorType::Rgba8).ok()?;
    }
    let f = factor as f64;
    Some(StageArt { png, x: min_x - ox, y: min_y - oy, w: cw, h: ch, hold: 1, rotation: 0.0, scale_x: f, scale_y: f , alpha: 1.0 , pivot: (0.0, 0.0), tint: TINT_ID })
}

/// Extract a clip's labelled sub-animations as rasterized frame sequences. Each frame label in the
/// clip's own timeline (e.g. a Thwomp's `entrance`/`fall`/`idle`) becomes one [`ClipAnim`]: the clip
/// is held on that label's frame while the nested sub-clip plays through its full timeline,
/// composited per frame on ONE fixed canvas (the union across the sub-animation, so it doesn't
/// wiggle). Universal — any multi-animation stage clip (hazards, animated props) uses this.
#[allow(clippy::too_many_arguments)]
/// The clips whose timeline is a state machine: two or more frame labels.
fn labelled_clips(sprites: &BTreeMap<u16, &Vec<swf::Tag>>) -> std::collections::BTreeSet<u16> {
    sprites.iter()
        .filter(|(_, t)| crate::sprite_parser::extract_frame_labels_from_tags(t).len() >= 2)
        .map(|(id, _)| *id).collect()
}

fn extract_labeled_clip_anims(
    clip_id: u16,
    sprites: &BTreeMap<u16, &Vec<swf::Tag>>,
    sym_names: &BTreeMap<u16, String>,
    shape_defs: &BTreeMap<u16, &swf::Shape>,
    bitmaps: &BTreeMap<u16, (u32, u32, Vec<u8>)>,
    planes: &PlaneMap,
    abcs: &[crate::abc_parser::AbcFile],
    ox: f64, oy: f64,
    skip: &[&str],
) -> Vec<ClipAnim> {
    let Some(tags) = sprites.get(&clip_id) else { return Vec::new() };
    if let Ok(ids) = std::env::var("PEPTIDE_CLIP_TAGS") {
        for id in ids.split(',').filter_map(|s| s.trim().parse::<u16>().ok()) {
            let Some(t) = sprites.get(&id) else { continue };
            eprintln!("[clip-tags] sprite {id} raw timeline:");
            let mut frame = 0usize;
            for tag in t.iter() {
                match tag {
                    swf::Tag::PlaceObject(po) => {
                        let act = match &po.action {
                            swf::PlaceObjectAction::Place(i) => format!("Place({i})"),
                            swf::PlaceObjectAction::Replace(i) => format!("Replace({i})"),
                            swf::PlaceObjectAction::Modify => "Modify".into(),
                        };
                        let a = po.color_transform.map(|ct| {
                            (f32::from(ct.a_multiply) as f64, f32::from(ct.a_add) as f64)
                        });
                        let m = po.matrix.map(|m| format!("sx={:.3} sy={:.3} tx={:.1} ty={:.1}",
                            m.a.to_f32(), m.d.to_f32(), m.tx.to_pixels(), m.ty.to_pixels()));
                        eprintln!("    f{frame} depth={} {act} mat={:?} ratio={:?} alpha={:?} blend={:?}",
                            po.depth, m, po.ratio, a, po.blend_mode);
                    }
                    swf::Tag::RemoveObject(r) => eprintln!("    f{frame} depth={} REMOVE", r.depth),
                    swf::Tag::FrameLabel(l) => eprintln!("    f{frame} LABEL {:?}", l.label.to_str_lossy(encoding_rs::WINDOWS_1252)),
                    swf::Tag::ShowFrame => { frame += 1; }
                    _ => {}
                }
            }
            eprintln!("    total frames {frame}");
        }
    }
    // A clip that carries no labels is not a clip with no animation: it is ONE animation, the
    // whole timeline, which is how a piece of art that just plays is authored. Read as nothing, a
    // class that names such a clip has nothing to draw.
    let mut labels = crate::sprite_parser::extract_frame_labels_from_tags(tags);
    if labels.is_empty() { labels.push(("active".to_string(), 0)); }
    let mut sprite_frames: BTreeMap<u16, Vec<Vec<PlacedChild>>> = BTreeMap::new();
    for (id, t) in sprites { sprite_frames.insert(*id, build_frames(t)); }
    // the children left out by name: a body's collision child is never in the picture
    let clip_frames: Vec<Vec<PlacedChild>> = build_frames(tags).into_iter()
        .map(|fr| fr.into_iter().filter(|c| !skip.contains(&c.2.as_deref().unwrap_or(""))).collect())
        .collect();
    let no_phase: std::collections::HashMap<(u16, (i64, i64)), usize> = std::collections::HashMap::new();
    let labelled = labelled_clips(sprites);
    let mut anims = Vec::new();
    for (label, frame) in &labels {
        let f = *frame as usize;
        let Some(children) = clip_frames.get(f) else { continue };
        // sub-animation length = the longest nested MOVIECLIP placed on this label's frame. A
        // placement's `ratio` does NOT exclude it: ratio only means anything for a MorphShape, and
        // Flash runs a sprite's own timeline regardless (see `walk_frame`).
        let sub_len = children.iter()
            .filter_map(|(id, _, _, _, _, _, _, _, _, _)| sprite_frames.get(id).map(|fr| fr.len()))
            .max().unwrap_or(1).max(1);
        // hold-vs-loop comes from the DRIVING nested clip's own timeline: a sub-clip that holds
        // carries a Flash-generated `_fla.` class (frame scripts) bound in SymbolClass. `stop()`
        // freezes where it fires; `gotoAndStop(target)` plays through then freezes at the target
        // (a label in the SUB-clip's own timeline, or a 1-based frame number).
        let driver_id = children.iter()
            .filter_map(|(id, _, _, _, _, _, _, _, _, _)| sprite_frames.get(id).map(|fr| (*id, fr.len())))
            .max_by_key(|(_, n)| *n).map(|(id, _)| id);
        let hold = driver_id
            .and_then(|id| sym_names.get(&id))
            .and_then(|class| abcs.iter().find_map(|a| crate::abc_parser::extract_timeline_hold(a, class)));
        let frame_velocities: Vec<(u32, f64)> = driver_id
            .and_then(|id| sym_names.get(&id))
            .map(|class| abcs.iter().flat_map(|a| crate::abc_parser::extract_frame_velocities(a, class)).collect())
            .unwrap_or_default();
        // the clip's frame scripts by label and offset (a script is named from 1, a label's
        // frame counted from 0), for what they ask of the stage
        let clip_scripts: Vec<(String, u32, String)> = driver_id
            .and_then(|id| sym_names.get(&id).map(|class| (id, class)))
            .map(|(id, class)| {
                let labels = sprites.get(&id).map(|t| crate::sprite_parser::extract_frame_labels_from_tags(t)).unwrap_or_default();
                abcs.iter().flat_map(|a| crate::abc_parser::extract_frame_scripts(a, class)).filter_map(|(frame, code)| {
                    let index = frame.saturating_sub(1);
                    let (label, at) = labels.iter().filter(|(_, f)| u32::from(*f) <= index)
                        .max_by_key(|(_, f)| *f).map(|(l, f)| (l.clone(), u32::from(*f)))?;
                    Some((label, index - at, code))
                }).collect()
            })
            .unwrap_or_default();
        // resolve to (play-until frame, freeze-at frame), both 0-based sub-frame indices.
        let hold_frames: Option<(usize, usize)> = hold.as_ref().and_then(|h| match h {
            crate::abc_parser::TimelineHold::StopAt(n) => {
                let f = (*n as usize).saturating_sub(1);
                Some((f, f))
            }
            crate::abc_parser::TimelineHold::GotoStop(target, n) => {
                let until = (*n as usize).saturating_sub(1);
                let at = if let Ok(num) = target.parse::<usize>() { num.saturating_sub(1) } else {
                    let sub_labels = driver_id.and_then(|id| sprites.get(&id))
                        .map(|t| crate::sprite_parser::extract_frame_labels_from_tags(t))
                        .unwrap_or_default();
                    sub_labels.iter().find(|(l, _)| l.eq_ignore_ascii_case(target))
                        .map(|(_, f)| *f as usize)?
                };
                Some((until, at))
            }
        });
        if std::env::var("PEPTIDE_STAGE_DEBUG").is_ok() {
            eprintln!("[hz-hold] label={label:?} driver={driver_id:?} class={:?} hold={hold:?} -> {hold_frames:?}",
                driver_id.and_then(|id| sym_names.get(&id)));
        }
        // walk every sub-frame, then composite all frames onto one fixed union canvas (no wiggle).
        // walk_frame itself pins graphic placements to their ratio-selected frame.
        let per_frame: Vec<Vec<Instance>> = (0..sub_len).map(|g| {
            let mut out = Vec::new();
            walk_frame(children, Mat::id(), g, None, None, planes, sym_names, shape_defs, &sprite_frames, &mut out, 0, (0.0, 0.0), 0, None, &no_phase, None, 1.0, crate::swf_timeline::Tint::ID, &[], &labelled, &std::collections::BTreeSet::new(), false, &std::collections::BTreeSet::new(), false, false, &[], 1);
            out.retain(|i| shape_defs.contains_key(&i.shape_id));
            out
        }).collect();
        let all: Vec<&Instance> = per_frame.iter().flatten().collect();
        let bounds = (!all.is_empty()).then(|| (
            all.iter().map(|i| i.aabb.left()).fold(f64::MAX, f64::min),
            all.iter().map(|i| i.aabb.top()).fold(f64::MAX, f64::min),
            all.iter().map(|i| i.aabb.right()).fold(f64::MIN, f64::max),
            all.iter().map(|i| i.aabb.bottom()).fold(f64::MIN, f64::max),
        ));
        let mut frames: Vec<StageArt> = per_frame.iter()
            .filter_map(|insts| composite_layer(&insts.iter().collect::<Vec<_>>(), shape_defs, bitmaps, ox, oy, bounds))
            .collect();
        // arrange the frames so FM's play-once-and-hold (endType NONE) freezes on the SAME frame
        // SSF2 freezes on: truncate at the play-until frame, and when the freeze target differs
        // (gotoAndStop to a label) append that frame as the final held one.
        // on the runtime path the clip's own frame scripts run as it plays, holds and all: the frames
        // are kept whole and split at its labels below
        let runtime_path = std::env::var("PEPTIDE_STAGE_TEXT").is_err();
        let stop_frame = if runtime_path { None } else { hold_frames }.and_then(|(until, at)| {
            if until + 1 < frames.len() { frames.truncate(until + 1); }
            if at != until {
                let held = frames.get(at).or(frames.first()).cloned()?;
                frames.push(held);
            }
            Some(frames.len() - 1)
        });
        // A label that draws NOTHING is still a state the class plays: bomb factory's block is
        // hidden on "dead" before it is started and again after it breaks. Dropped, its state had
        // to be guessed an animation and the block wore a lit bomb while it was supposed to be
        // gone. Kept with no frames, it is a state that shows nothing -- which is what it is.
        // ONE LABEL PER ANIMATION, in the driving clip as well as on the outside.
        //
        // A clip splits into animations at its frame labels, and a driving sub-clip's labels are
        // the same statement one level down. Clocktown's meteor is four eight-frame tumbles --
        // `loop_1` `loop_2` `loop_3` `loop_4` -- and its own frame scripts send playback back to
        // whichever it was given, so each is a loop in its own right. Flattened into the
        // thirty-three frames they sit in, a meteor plays all four tumbles in a row and every
        // meteor plays the same one.
        let self_played = driver_id.and_then(|id| sym_names.get(&id)).and_then(|class|
            abcs.iter().find_map(|a| crate::abc_parser::extract_self_played_labels(a, class)));
        let driver_labels: Vec<(String, u16)> = driver_id.and_then(|id| sprites.get(&id))
            .map(|t| crate::sprite_parser::extract_frame_labels_from_tags(t)).unwrap_or_default();
        let split = self_played.as_ref().filter(|_| driver_labels.len() > 1 && stop_frame.is_none());
        // ...and on the runtime path every driving clip with labels splits at them, whoever plays
        // them: the runtime takes each label's section as the animation that shows it (mushroom
        // kingdom 3's piranha plant attacks and dies in sections of its stance)
        let none = crate::abc_parser::SelfPlayedLabels::default();
        let split_by_runtime = split.is_none() && runtime_path && driver_labels.len() > 1;
        let split = split.or_else(|| split_by_runtime.then_some(&none));
        if let Some(sp) = split {
            for (i, (name, start)) in driver_labels.iter().enumerate() {
                let from = *start as usize;
                let to = driver_labels.get(i + 1).map(|(_, f)| *f as usize).unwrap_or(frames.len());
                if from >= frames.len() || to <= from { continue; }
                // on the runtime path a section is named under the label that places its clip: two sections' clips may share labels (saffron city's pokemon each run
                // `entrance` .. `end`), and one entity animation stands for one of them
                let named = if runtime_path { format!("{label}_{name}") } else { name.clone() };
                anims.push(ClipAnim {
                    label: named,
                    frames: frames[from..to.min(frames.len())].to_vec(),
                    stop_frame: None,
                    frame_velocities: frame_velocities.clone(),
                    clip_scripts: clip_scripts.clone(),
                    loops: true,
                    pick_between: sp.pick_between.clone(),
                });
            }
            continue;
        }
        anims.push(ClipAnim { label: label.clone(), frames, stop_frame, frame_velocities, clip_scripts: clip_scripts.clone(),
                              loops: false, pick_between: Vec::new() });
    }
    anims
}

/// Trace a solid terrain shape's walkable TOP surface as a polyline in FM coords (left to
/// right). Rasterizes the shape and scans each column for the topmost opaque pixel, then maps
/// to world via the placed AABB and simplifies. `None` if the shape won't rasterize, is
/// essentially flat (so the level `rect.top()` line suffices), or maps to < 2 points. Maps
/// through the AABB, so it's exact for translated/scaled terrain; rotated terrain would skew
/// (rare for stages).
/// Every standable surface a terrain shape carries, top edge and underside, in FM coords.
///
/// One terrain clip is not one floor. A shape can hold several pieces stacked over each other --
/// crystal smash draws its whole structure as a single clip, an upper deck over a lower one over
/// the ground -- and SSF2 collides with the shape, so every piece of it is terrain. Read as one
/// surface (the highest opaque pixel in each column) everything under the top piece disappears and
/// the fighters spawn INSIDE a solid body with no floor under them, which is what crystal smash
/// did: they fell through the stage on the first frame, forever.
///
/// So each column is cut into its runs of opaque pixels, and runs that touch across columns are one
/// piece. A shape with a single piece gives a single band, which is the old behaviour exactly.
fn floor_bands(shape: &swf::Shape, aabb: &Rect, ox: f64, oy: f64, scale: f64)
    -> Vec<(Vec<(f64, f64)>, f64, f64, f64)>
{
    use image::{imageops, RgbaImage};
    // a terrain can be wider than the raster cap (one clip holding a whole level): the mask is
    // made at the scale that fits, and every column maps back through the placed size
    let sb = &shape.shape_bounds;
    let (sw, sh) = ((sb.x_max - sb.x_min).to_pixels().max(1.0), (sb.y_max - sb.y_min).to_pixels().max(1.0));
    let k = (4000.0 / sw).min(4000.0 / sh).min(1.0);
    let Some(raster) = crate::vector_raster::rasterize_shape_scaled(
        &shape.shape_bounds, &shape.styles.fill_styles, &shape.styles.line_styles, &shape.shape, k,
    ) else { return Vec::new() };
    let Some(mask) = RgbaImage::from_raw(raster.width, raster.height, raster.rgba) else { return Vec::new() };
    let w = ((aabb.w * k).round() as u32).clamp(1, 4096);
    let h = ((aabb.h * k).round() as u32).clamp(1, 4096);
    if w < 2 || h < 2 { return Vec::new() }
    let mask = imageops::resize(&mask, w, h, imageops::FilterType::Triangle);
    let (px, py) = (aabb.w / w as f64, aabb.h / h as f64);
    let step = (w / 96).max(1);
    // a band being built: its top points so far, its lowest underside, and the column it last grew in
    struct Band { pts: Vec<(u32, u32)>, bottom: u32, last_col: u32, last: (u32, u32) }
    let mut open: Vec<Band> = Vec::new();
    let mut done: Vec<Band> = Vec::new();
    let mut cx = 0u32;
    loop {
        let col = cx.min(w - 1);
        // the runs of solid pixels down this column
        let mut runs: Vec<(u32, u32)> = Vec::new();
        let mut start: Option<u32> = None;
        for y in 0..h {
            let solid = mask.get_pixel(col, y)[3] > 40;
            match (solid, start) {
                (true, None) => start = Some(y),
                (false, Some(s)) => { runs.push((s, y - 1)); start = None }
                _ => {}
            }
        }
        if let Some(s) = start { runs.push((s, h - 1)); }
        let mut grown = vec![false; open.len()];
        for (top, bot) in runs {
            // the piece this run continues: one whose last run overlaps it vertically
            let hit = open.iter().position(|b| !(bot < b.last.0 || top > b.last.1));
            match hit {
                Some(i) => {
                    grown[i] = true;
                    open[i].pts.push((col, top));
                    open[i].bottom = open[i].bottom.max(bot);
                    open[i].last_col = col;
                    open[i].last = (top, bot);
                }
                None => open.push(Band { pts: vec![(col, top)], bottom: bot, last_col: col, last: (top, bot) }),
            }
        }
        // a piece that did not carry into this column has ended
        let mut i = 0;
        while i < open.len() {
            if !grown.get(i).copied().unwrap_or(true) && open[i].last_col != col {
                done.push(open.remove(i));
            } else { i += 1 }
        }
        if col == w - 1 { break }
        cx += step;
    }
    done.extend(open);
    let mut out = Vec::new();
    for b in done {
        if b.pts.len() < 2 { continue }
        let pts: Vec<(f64, f64)> = b.pts.iter().map(|&(c, y)| (
            (aabb.left() + c as f64 * px - ox) * scale,
            (aabb.top() + y as f64 * py - oy) * scale,
        )).collect();
        let left = pts.first().map(|p| p.0).unwrap_or(0.0);
        let right = pts.last().map(|p| p.0).unwrap_or(0.0);
        let bottom = (aabb.top() + b.bottom as f64 * py - oy) * scale;
        out.push((rdp_simplify(&pts, 2.0), bottom, left, right));
    }
    out
}

fn floor_profile(shape: &swf::Shape, aabb: &Rect, ox: f64, oy: f64, scale: f64) -> Option<Vec<(f64, f64)>> {
    use image::{imageops, RgbaImage};
    let raster = crate::vector_raster::rasterize_shape(
        &shape.shape_bounds, &shape.styles.fill_styles, &shape.styles.line_styles, &shape.shape,
    )?;
    let mask = RgbaImage::from_raw(raster.width, raster.height, raster.rgba)?;
    let w = (aabb.w.round() as u32).clamp(1, 4096);
    let h = (aabb.h.round() as u32).clamp(1, 4096);
    if w < 2 || h < 2 { return None; }
    let mask = imageops::resize(&mask, w, h, imageops::FilterType::Triangle);
    let step = (w / 96).max(1); // ~96 columns sampled across the span
    let mut pts: Vec<(f64, f64)> = Vec::new();
    let mut cx = 0u32;
    loop {
        let col = cx.min(w - 1);
        if let Some(ty) = (0..h).find(|&y| mask.get_pixel(col, y)[3] > 40) {
            pts.push((
                (aabb.left() + col as f64 - ox) * scale,
                (aabb.top() + ty as f64 - oy) * scale,
            ));
        }
        if col == w - 1 { break; }
        cx += step;
    }
    if pts.len() < 2 { return None; }
    let simplified = rdp_simplify(&pts, 2.0);
    // a (near-)flat profile adds nothing over the level floor line.
    let ys: Vec<f64> = simplified.iter().map(|p| p.1).collect();
    let span = ys.iter().cloned().fold(f64::MIN, f64::max) - ys.iter().cloned().fold(f64::MAX, f64::min);
    if span < 6.0 { return None; }
    Some(simplified)
}

/// Ramer-Douglas-Peucker polyline simplification (perpendicular-distance epsilon in px).
fn rdp_simplify(pts: &[(f64, f64)], eps: f64) -> Vec<(f64, f64)> {
    if pts.len() < 3 { return pts.to_vec(); }
    let (a, b) = (pts[0], pts[pts.len() - 1]);
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = (dx * dx + dy * dy).sqrt().max(1e-9);
    let (mut max_d, mut idx) = (0.0_f64, 0);
    for (i, p) in pts.iter().enumerate().take(pts.len() - 1).skip(1) {
        let d = ((p.0 - a.0) * dy - (p.1 - a.1) * dx).abs() / len;
        if d > max_d { max_d = d; idx = i; }
    }
    if max_d > eps {
        let mut left = rdp_simplify(&pts[..=idx], eps);
        let right = rdp_simplify(&pts[idx..], eps);
        left.pop();
        left.extend(right);
        left
    } else {
        vec![a, b]
    }
}

/// The rate a camera background pans at when the package states none for it.
///
/// A declaration that names the piece and says nothing else leaves the rate to the game, and this
/// is what the game works out: how far the layer travels across the view, over how far the view and
/// the layer together can move. `(size - view) / (2 * (size + view))`, and a layer no bigger than
/// the view does not travel at all.
///
/// The number is the fraction of the camera's movement the layer shows ON SCREEN, which is what
/// Fraymakers' own multiplier means as well (measured: a layer emitted at 0.137 moved 17px while
/// the world moved 117). Battlefield, whose declaration is a linkage id and nothing else, moves its
/// jungle 32px against 406 in SSF2 -- 0.079, which is what this gives for an 882x496 backdrop.
fn parallax_pan(w: u32, h: u32) -> (f64, f64) {
    let x = ((w as f64 - 640.0) / (2.0 * (w as f64 + 640.0))).clamp(0.0, 0.5);
    let y = ((h as f64 - 360.0) / (2.0 * (h as f64 + 360.0))).clamp(0.0, 0.5);
    (x, y)
}

/// Decompress an `.ssf` to SWF bytes via the DAT-archive aware path.
fn ssf_decompress(raw: &[u8], path: &Path) -> Result<Vec<u8>> {
    crate::ssf::decompress(raw).with_context(|| format!("decompress {}", path.display()))
}

/// Read the stage DAT's `Main` package metadata (`id`, `guid`, `music`) from the first
/// ABC block that carries an `id` (the content id the engine knows the stage by).
/// the stage's AS3 layer assignments + spawned actors (the document class extending `SSF2Stage`).
/// `None` if the stage has no parseable SSF2Stage subclass (the heuristic path then takes over).
fn stage_abc_model(swf_data: &[u8]) -> Option<crate::stage_abc::StageAbcModel> {
    let swf = crate::swf_parser::parse(swf_data).ok()?;
    let blocks: Vec<crate::abc_parser::AbcFile> = swf.abc_blocks.iter()
        .filter_map(|b| crate::abc_parser::parse(b).ok()).collect();
    let mut model = blocks.iter().find_map(crate::stage_abc::extract_stage)?;
    // Resolve each spawned hazard's get*() declarations across ALL abc blocks, not just the stage's.
    // The enemy class is usually compiled into the same block as the stage (extract_stage already
    // filled it from there), but a stage CAN split it into another block — and we follow the
    // `spawnEnemy(<Class>)` reference (`a.class_name`) to find it wherever it lives, rather than
    // assuming co-location or scanning for SSF2Enemy subclasses by hand.
    for a in &mut model.actors {
        if a.attack_hitboxes.is_empty() {
            for abc in &blocks {
                let hb = crate::abc_parser::extract_attack_stats_for(abc, &a.class_name);
                if !hb.is_empty() { a.attack_hitboxes = hb; break; }
            }
        }
        if a.own_stats.is_empty() {
            for abc in &blocks {
                let os = crate::abc_parser::extract_own_stats_for(abc, &a.class_name);
                if !os.is_empty() { a.own_stats = os; break; }
            }
        }
        if a.anim_labels.is_empty() {
            for abc in &blocks {
                let al = crate::abc_parser::extract_force_attack_labels(abc, &a.class_name);
                if !al.is_empty() { a.anim_labels = al; break; }
            }
        }
        if a.faller.is_none() {
            for abc in &blocks {
                if let Some(fc) = crate::abc_parser::extract_faller_cycle(abc, &a.class_name) {
                    a.faller = Some(fc); break;
                }
            }
        }
        if a.behavior.shake.is_none() && a.behavior.self_platform.is_none() {
            for abc in &blocks {
                let b = crate::abc_parser::extract_enemy_behavior(abc, &a.class_name);
                if b.shake.is_some() || b.self_platform.is_some() || b.rise_yspeed.is_some() { a.behavior = b; break; }
            }
        }
        if a.reconstructed_script.is_none() {
            for abc in &blocks {
                if let Some(s) = crate::abc_parser::reconstruct_enemy_script(abc, &a.class_name) { a.reconstructed_script = Some(s); break; }
            }
        }
    }
    // A class the stage asks for that no block here defines is one the game had loaded alongside.
    // The shared packages are searched for it, and it ports out of whichever one answers exactly as
    // it would have from the stage's own file: same declarations, same reconstruction.
    let external: Vec<String> = model.external_classes.clone();
    for name in external {
        if model.actors.iter().any(|a| a.class_name == name) { continue }
        let mut found = None;
        for (pi, pkg) in crate::shared_packages::packages().iter().enumerate() {
            let Ok(swf) = crate::swf_parser::parse(&pkg.swf) else { continue };
            for b in &swf.abc_blocks {
                let Ok(abc) = crate::abc_parser::parse(b) else { continue };
                let Some(class) = abc.classes.iter().find(|c| c.name == name) else { continue };
                found = Some((pi, pkg.id.clone(), class.super_name.clone(), abc));
                break;
            }
            if found.is_some() { break }
        }
        let Some((pi, pkg_id, base, abc)) = found else {
            log::warn!("stage object '{name}' is named by the stage's own code but no package \
                        defines it: the spawn has nothing behind it");
            continue
        };
        log::info!("stage object '{name}' comes from the shared '{pkg_id}' package");
        model.actors.push(crate::stage_abc::SpawnedActor {
            class_name: name.clone(), base, x: None, y: None,
            attack_hitboxes: crate::abc_parser::extract_attack_stats_for(&abc, &name),
            attack_by_anim: crate::abc_parser::extract_attack_stats_by_anim(&abc, &name),
            attack_rates: crate::abc_parser::extract_attack_rates_by_anim(&abc, &name),
            own_stats: crate::abc_parser::extract_own_stats_for(&abc, &name),
            own_flags: crate::abc_parser::extract_own_flags_for(&abc, &name),
            projectiles: crate::abc_parser::extract_projectiles_for(&abc, &name),
            linkage_id: crate::abc_parser::extract_own_linkage_for(&abc, &name),
            anim_labels: crate::abc_parser::extract_force_attack_labels(&abc, &name),
            behavior: crate::abc_parser::extract_enemy_behavior(&abc, &name),
            reconstructed_script: crate::abc_parser::reconstruct_enemy_script(&abc, &name),
            faller: crate::abc_parser::extract_faller_cycle(&abc, &name),
            external_pkg: Some(pi),
        });
    }
    Some(model)
}

fn stage_package_metadata(swf_data: &[u8]) -> Option<crate::abc_parser::MainPackageMetadata> {
    let swf = crate::swf_parser::parse(swf_data).ok()?;
    for abc_bytes in &swf.abc_blocks {
        if let Ok(abc) = crate::abc_parser::parse(abc_bytes) {
            if let Some(md) = crate::abc_parser::extract_main_package_metadata(&abc) {
                if md.id.is_some() { return Some(md); }
            }
        }
    }
    None
}

/// Drop collision platforms that are near-duplicates of another (a moving-platform container
/// MC and its collision child both match the platform/terrain naming → the same platform twice).
/// A platform is removed when another platform of the SAME kind covers >=70% of its area; the
/// larger of an overlapping pair survives. Genuinely separate platforms don't overlap, so they
/// are untouched. A removed platform's `moving` flag is OR'd onto the survivor.
fn dedupe_platforms(platforms: &mut Vec<Platform>) {
    let area = |r: &Rect| (r.w * r.h).max(1.0);
    let covered = |a: &Rect, b: &Rect| -> f64 {
        let ix = (a.right().min(b.right()) - a.left().max(b.left())).max(0.0);
        let iy = (a.bottom().min(b.bottom()) - a.top().max(b.top())).max(0.0);
        (ix * iy) / area(a) // fraction of `a` inside `b`
    };
    let mut keep = vec![true; platforms.len()];
    for i in 0..platforms.len() {
        if !keep[i] { continue; }
        for j in 0..platforms.len() {
            if i == j || !keep[j] || platforms[i].drop_through != platforms[j].drop_through { continue; }
            // two surfaces the stage names differently are two surfaces, however they overlap:
            // galaxy tours stacks every section's ground in one clip and turns each on by name
            if let (Some(a), Some(b)) = (&platforms[i].name, &platforms[j].name) {
                if a != b { continue; }
            }
            // j is the smaller (or equal, broken by index) -> drop j into i.
            let (bigger, smaller) = if area(&platforms[i].rect) >= area(&platforms[j].rect) { (i, j) } else { continue };
            // a duplicate of a surface stands where it stands: the same top edge. a no-drop
            // platform laid over a terrain's box is a second floor, not the same one again
            // (crateria's fighters start on one, a hundred and fifty px above the ground)
            if (platforms[smaller].rect.top() - platforms[bigger].rect.top()).abs() > 4.0 { continue; }
            // a solid with its outline traced is exact, and its box says nothing about what is
            // under a surface laid over it: crateria's no-drop platform sits inside its terrain's
            // box, a hundred and fifty px above the ground the outline puts there
            if !platforms[bigger].outline.is_empty() || !platforms[smaller].outline.is_empty() { continue; }
            if covered(&platforms[smaller].rect, &platforms[bigger].rect) >= 0.7 {
                keep[smaller] = false;
            }
        }
    }
    let mut idx = 0;
    platforms.retain(|_| { let k = keep[idx]; idx += 1; k });
}

/// Title-case an SSF2 lowercase-concatenated id for a display name: capitalize the first
/// letter and any after an underscore/space ("battlefield" -> "Battlefield"). Multi-word
/// ids that aren't underscore-separated stay one word; add an override in the metadata map.
fn title_case(id: &str) -> String {
    let mut out = String::with_capacity(id.len());
    let mut cap = true;
    for c in id.chars() {
        if c == '_' || c == '-' { out.push(' '); cap = true; }
        else if cap { out.extend(c.to_uppercase()); cap = false; }
        else { out.push(c); }
    }
    out
}

/// Extract a 0-based player index from a symbol name like
/// `battlefield_fla.p1_Start_9` given the marker `"_start_"` / `"_spawn_"`.
/// Looks for `pN` immediately before the marker.
fn player_index(sym_lower: &str, marker: &str) -> Option<usize> {
    let pos = sym_lower.find(marker)?;
    // walk back over the `pN` token preceding the marker
    let prefix = &sym_lower[..pos];
    let digits: String = prefix.chars().rev().take_while(|c| c.is_ascii_digit()).collect::<String>().chars().rev().collect();
    if digits.is_empty() { return None; }
    // the char before the digits must be 'p'
    let p_idx = prefix.len() - digits.len();
    if prefix.as_bytes().get(p_idx.wrapping_sub(1)) != Some(&b'p') { return None; }
    digits.parse::<usize>().ok().and_then(|n| n.checked_sub(1))
}

/// Recurse the placement tree, recording each placed shape's world AABB and the
/// `stageMC` origin (the root instance whose name is `stageMC` or whose symbol
/// class starts with `stage_`).
#[allow(clippy::too_many_arguments)]
/// One entry of a timeline's display list at a frame: what sits at a depth once every
/// placement, modification and removal up to that frame has been applied.
struct PlacedAt {
    depth: u16,
    id: u16,
    matrix: Option<swf::Matrix>,
    name: Option<String>,
    visible: bool,
}

/// The display list a timeline shows at `frame` (0-based; past the end = its final state).
/// A later frame placing over a depth is the clip's next phase, not more of the same frame:
/// pokemon colosseum's terrain runs 779 frames of art, and reading every frame's placement as
/// one display list made nine hundred collision shapes of it.
fn display_list_at(tags: &[swf::Tag], frame: usize) -> Vec<PlacedAt> {
    let mut at: BTreeMap<u16, PlacedAt> = BTreeMap::new();
    let mut f = 0usize;
    for tag in tags {
        match tag {
            swf::Tag::PlaceObject(po) => {
                let name = po.name.as_ref().map(|n| n.to_str_lossy(encoding_rs::WINDOWS_1252).to_string());
                match &po.action {
                    swf::PlaceObjectAction::Place(id) | swf::PlaceObjectAction::Replace(id) => {
                        at.insert(po.depth, PlacedAt {
                            depth: po.depth, id: *id, matrix: po.matrix, name,
                            visible: po.is_visible.unwrap_or(true),
                        });
                    }
                    swf::PlaceObjectAction::Modify => {
                        if let Some(e) = at.get_mut(&po.depth) {
                            if po.matrix.is_some() { e.matrix = po.matrix; }
                            if name.is_some() { e.name = name; }
                            if let Some(v) = po.is_visible { e.visible = v; }
                        }
                    }
                }
            }
            swf::Tag::RemoveObject(r) => { at.remove(&r.depth); }
            swf::Tag::ShowFrame => {
                if f == frame { break; }
                f += 1;
            }
            _ => {}
        }
    }
    at.into_values().collect()
}

/// The frame a clip's own script holds on, 0-based: `stop()` at N, or `gotoAndStop(target)`
/// resolved through its labels. None when the clip plays on (its first frame is then read).
fn hold_frame_of(tags: &[swf::Tag], class: Option<&str>, abcs: &[crate::abc_parser::AbcFile]) -> Option<usize> {
    let class = class?;
    let hold = abcs.iter().find_map(|a| crate::abc_parser::extract_timeline_hold(a, class))?;
    match hold {
        crate::abc_parser::TimelineHold::StopAt(n) => Some((n as usize).saturating_sub(1)),
        crate::abc_parser::TimelineHold::GotoStop(target, _) => {
            if let Ok(k) = target.trim().parse::<usize>() { return Some(k.saturating_sub(1)); }
            crate::sprite_parser::extract_frame_labels_from_tags(tags).iter()
                .find(|(l, _)| l.eq_ignore_ascii_case(&target))
                .map(|(_, f)| *f as usize)
        }
    }
}

fn walk<'a>(
    tags: &'a [swf::Tag], parent: Mat,
    sym_names: &BTreeMap<u16, String>,
    shape_bounds: &BTreeMap<u16, (f64, f64, f64, f64)>,
    sprites: &BTreeMap<u16, &'a Vec<swf::Tag>>,
    planes: &PlaneMap,
    roles: &BTreeMap<String, crate::abc_parser::ClipRole>,
    rec: usize, carried_sym: Option<&str>, carried_kind: Option<&str>, carried_no_drop: bool, only_ground: bool, owner: Option<&str>, plane: Option<&str>,
    out: &mut Vec<Instance>, anchors: &mut WalkAnchors, placed_hidden: bool,
    carried_push: Option<f64>,
    carried_sync: Option<&str>,
    carried_friction: Option<(f64, f64)>,
    hidden_syms: &std::collections::BTreeSet<String>, frame: usize, abcs: &[crate::abc_parser::AbcFile],
) {
    if rec > 8 { return; }
    // the display list at the frame this clip holds on (the root: its final state)
    let entries = display_list_at(tags, frame);
    if std::env::var("PEPTIDE_PLAT_DEBUG").is_ok() {
        let frames = tags.iter().filter(|t| matches!(t, swf::Tag::ShowFrame)).count();
        eprintln!("[dl] {}sym={:?} frame={} of {frames} entries={}", "  ".repeat(rec), carried_sym.unwrap_or(""), if frame == usize::MAX { -1 } else { frame as i64 }, entries.len());
    }
    for po in &entries {
        let id = po.id;
        let placement_hidden = placed_hidden || !po.visible;
        let local = po.matrix.as_ref().map(|m| Mat {
            a: m.a.to_f64(), b: m.b.to_f64(), c: m.c.to_f64(), d: m.d.to_f64(),
            tx: m.tx.get() as f64 / 20.0, ty: m.ty.get() as f64 / 20.0,
        }).unwrap_or(Mat::id());
        let world = parent.mul(&local);
        let inst_name = po.name.clone();
        let sym = sym_names.get(&id).cloned().unwrap_or_default();
        let my_plane = plane_tag(inst_name.as_deref(), &sym, planes).or(plane);
        // the role this clip's class declares, else the one it sits inside
        let local_sym = sym.rsplit('.').next().unwrap_or(&sym);
        // ...or hidden by its own class on its first frame (`self.visible = false`), or by the
        // clip over it hiding this child by name: pokemon colosseum's terrain plate is hidden
        // that way, and it is the plate; the drawing around it is not collision
        let here_hidden = placement_hidden || hidden_syms.contains(local_sym)
            || match (carried_sym, inst_name.as_deref()) {
                (Some(p), Some(n)) => hidden_syms.contains(&format!("{}::{n}", p.rsplit('.').next().unwrap_or(p))),
                _ => false,
            };
        // A CLIP THAT HOLDS ITS COLLISION AS A CHILD collides with that child alone: the role it
        // declares reaches its `ground` and nothing else under it, which is art (central
        // highway's pillars are drawn a hundred px taller than the plate fighters stand on).
        let is_ground_child = only_ground && inst_name.as_deref() == Some("ground");
        let carried_kind = if only_ground && !is_ground_child { None } else { carried_kind };
        // ...and the plate answers to the name of the clip that holds it, which is the name the
        // stage's code asks for it by (hylian skies swaps its `lightTerrain` and `darkTerrain`)
        let inst_name = if is_ground_child { owner.map(str::to_string).or(inst_name) } else { inst_name };
        let my_kind = roles.get(local_sym).filter(|r| !r.kind.is_empty()).map(|r| r.kind.as_str()).or(carried_kind);
        let my_no_drop = roles.get(local_sym).is_some_and(|r| r.no_drop_through) || carried_no_drop;
        let my_push = roles.get(local_sym).and_then(|r| r.x_influence.as_deref()).and_then(|s| s.parse::<f64>().ok()).or(carried_push);
        let my_sync: Option<String> = roles.get(local_sym).and_then(|r| r.sync_platform.clone()).or(carried_sync.map(str::to_string));
        let my_friction: Option<(f64, f64)> = roles.get(local_sym).and_then(|r| r.decel_friction.map(|d| (d, r.accel_friction.unwrap_or(d)))).or(carried_friction);
        if std::env::var("PEPTIDE_STAGE_TREE").is_ok() {
            let kind = if sprites.contains_key(&id) { "MC" } else if shape_bounds.contains_key(&id) { "shape" } else { "?" };
            // frames tells a MOVING object from a still one, which is the whole question
            // when auditing a stage: stage art is rasterized into one composite sprite, so
            // a 1-frame clip having no separate Fraymakers object is correct, while a
            // multi-frame clip baked into that composite has silently lost its animation.
            let frames = sprites.get(&id).map(|t| build_frames(t).len()).unwrap_or(1);
            eprintln!("{}d{} {kind}#{id} inst={:?} sym={:?} plane={:?} @({:.0},{:.0}) frames={}",
                "  ".repeat(rec), po.depth, inst_name.as_deref().unwrap_or(""), sym, my_plane.unwrap_or(""), world.tx, world.ty, frames);
        }

        // record the stage origin from the root stageMC instance
        if anchors.origin.is_none()
            && (inst_name.as_deref() == Some("stageMC") || sym.to_ascii_lowercase().starts_with("stage_"))
        {
            anchors.origin = Some((world.tx, world.ty));
        }
        // record the terrain MC's REGISTRATION (matrix translate) — the origin of the GAME
        // coordinate space: the stage's own AS3 (spawnEnemy/setX) and the engine's runtime
        // X/Y both use terrain-local coords (verified live: pure translation, scale 1).
        if anchors.terrain.is_none() && inst_name.as_deref() == Some("terrain") {
            anchors.terrain = Some((world.tx, world.ty));
            anchors.terrain_id = Some(id);
            anchors.terrain_labels = sprites.get(&id)
                .map(|tg| crate::sprite_parser::extract_frame_labels_from_tags(tg)).unwrap_or_default();
            anchors.terrain_frames = sprites.get(&id)
                .map(|tg| tg.iter().filter(|x| matches!(x, swf::Tag::ShowFrame)).count()).unwrap_or(0);
        }

        if let Some((x0, y0, x1, y1)) = shape_bounds.get(&id) {
            let corners = [world.apply(*x0, *y0), world.apply(*x1, *y0), world.apply(*x1, *y1), world.apply(*x0, *y1)];
            let xmn = corners.iter().map(|c| c.0).fold(f64::MAX, f64::min);
            let xmx = corners.iter().map(|c| c.0).fold(f64::MIN, f64::max);
            let ymn = corners.iter().map(|c| c.1).fold(f64::MAX, f64::min);
            let ymx = corners.iter().map(|c| c.1).fold(f64::MIN, f64::max);
            out.push(Instance {
                kind: my_kind.map(str::to_string),
                no_drop: my_no_drop,
                x_influence: my_push,
                sync_key: my_sync.clone(),
                friction: my_friction,
                shape_id: id,
                hidden_by_class: false,
                hidden_placed: here_hidden,
                under_collision: false,
                inst_name: inst_name.clone(),
                sym_name: carried_sym.unwrap_or("").to_string(),
                plane: my_plane.map(str::to_string),
                aabb: Rect { x: xmn, y: ymn, w: xmx - xmn, h: ymx - ymn },
                cx: world.tx, cy: world.ty,
                x_sign: world.x_sign(), flip: world.flips(),
                inst_anchor: (0.0, 0.0), inst_path: 0, place: Default::default(), leaf_depth: 0, blend: None, inst_clip: None, chain_len: 1, alpha: 1.0, // geometry walk feeds collision/hazards, not art grouping
                tint: crate::swf_timeline::Tint::ID, // geometry walk: collision, never drawn
                named_path: Vec::new(),
                filters: Vec::new(),
            });
        }
        if let Some(child) = sprites.get(&id) {
            // carry the most specific identity: instance name wins, else symbol class,
            // else inherit the parent's carried symbol.
            let next = inst_name.as_deref().or(if sym.is_empty() { carried_sym } else { Some(&sym) });
            let next = next.map(|s| s.to_string());
            let child_has_ground = child.iter().any(|t| matches!(t, swf::Tag::PlaceObject(cp)
                if cp.name.as_ref().map(|n| n.to_str_lossy(encoding_rs::WINDOWS_1252)).as_deref() == Some("ground")));
            let child_owner = if child_has_ground { inst_name.as_deref() } else { owner };
            let child_frame = hold_frame_of(child, sym_names.get(&id).map(|x| x.rsplit('.').next().unwrap_or(x)), abcs).unwrap_or(0);
            walk(child, world, sym_names, shape_bounds, sprites, planes, roles, rec + 1, next.as_deref(), my_kind, my_no_drop, child_has_ground, child_owner, my_plane, out, anchors, here_hidden, my_push, my_sync.as_deref(), my_friction, hidden_syms, child_frame, abcs);
        }
    }
}

// ── the source's TIMELINE, ported as transforms ─────────────────────────────

/// Port a plane as its source TIMELINE rather than as a picture per frame.
///
/// Flash draws a backdrop as objects on depths: each object shows one shape at a time and is moved,
/// turned, scaled and faded by the timeline. Rasterising the composite every frame throws all of
/// that away and pays for it twice -- clocktown's day/night backdrop came out as hundreds of copies
/// of the same town, and its clock, its sky fades and its white flash came out as hard cuts, because
/// a raster cannot carry a tween.
///
/// So: one image per distinct SHAPE, and a keyframe per change carrying the transform and the alpha
/// the source authored. That is the same thing the character path does with a limb, and it is what
/// the engine's own keyframes are for. The measurement on clocktown's camera background: 16 objects,
/// 97 distinct shapes, 7131 keyframes -- against 623 copies of a 739x437 town.
const FRAME_RATIO_USIZE: usize = crate::entity_gen::FRAME_RATIO as usize;

type Segmenter<'a> = &'a dyn Fn(&Instance, &[StageArt]) -> Vec<BgSegment>;

/// consecutive frames that draw the same thing, held: one keyframe each
fn rle_frames(frames: Vec<StageArt>) -> Vec<StageArt> {
    let mut out: Vec<StageArt> = Vec::new();
    for f in frames {
        match out.last_mut() {
            Some(prev) if prev.png == f.png && prev.x == f.x && prev.y == f.y
                && prev.rotation == f.rotation && prev.scale_x == f.scale_x
                && prev.scale_y == f.scale_y && prev.alpha == f.alpha && prev.tint == f.tint => prev.hold += 1,
            _ => out.push(f),
        }
    }
    out
}

fn timeline_tracks(
    sampled: &[(Vec<Instance>, Option<StageArt>)],
    member: &dyn Fn(&Instance) -> bool,
    ox: f64, oy: f64, scale: f64,
    shape_defs: &BTreeMap<u16, &swf::Shape>,
    bitmaps: &BTreeMap<u16, (u32, u32, Vec<u8>)>,
    segmenter: Option<Segmenter<'_>>,
    own_len: &dyn Fn(u16) -> Option<usize>,
) -> Vec<BgLayer> {
    timeline_tracks_keyed(sampled, member, ox, oy, scale, shape_defs, bitmaps, &mut Vec::new(), segmenter, own_len, None)
}

/// As `timeline_tracks`, over samples that run past one lap of the stage (`lap` frames): a piece
/// whose clips come round together within the lap keeps the lap, and one that takes longer is
/// followed as far as its clips' common length
#[allow(clippy::too_many_arguments)]
fn timeline_tracks_lapped(
    sampled: &[(Vec<Instance>, Option<StageArt>)],
    member: &dyn Fn(&Instance) -> bool,
    ox: f64, oy: f64, scale: f64,
    shape_defs: &BTreeMap<u16, &swf::Shape>,
    bitmaps: &BTreeMap<u16, (u32, u32, Vec<u8>)>,
    segmenter: Option<Segmenter<'_>>,
    own_len: &dyn Fn(u16) -> Option<usize>,
    lap: usize,
) -> Vec<BgLayer> {
    timeline_tracks_keyed(sampled, member, ox, oy, scale, shape_defs, bitmaps, &mut Vec::new(), segmenter, own_len, Some(lap))
}

/// As `timeline_tracks`, and says WHICH placement each track came from.
///
/// A caller that both splits objects out of a plane and paints what is left needs to know what it
/// split, or it paints them again underneath: the object then exists twice, and the painted copy
/// keeps the whole plane animating in step with it.
#[allow(clippy::too_many_arguments)]
fn timeline_tracks_keyed(
    sampled: &[(Vec<Instance>, Option<StageArt>)],
    member: &dyn Fn(&Instance) -> bool,
    ox: f64, oy: f64, scale: f64,
    shape_defs: &BTreeMap<u16, &swf::Shape>,
    bitmaps: &BTreeMap<u16, (u32, u32, Vec<u8>)>,
    keys_out: &mut Vec<(u64, u16)>,
    segmenter: Option<Segmenter<'_>>,
    own_len: &dyn Fn(u16) -> Option<usize>,
    lap: Option<usize>,
) -> Vec<BgLayer> {
    // A Flash OBJECT is a placement chain plus a depth: that pair is what persists across frames
    // while its shape and transform change.
    type Key = (u64, u16);
    let mut name_of: BTreeMap<Key, String> = BTreeMap::new();
    let mut clip_of: BTreeMap<Key, u16> = BTreeMap::new();
    let mut chain_of: BTreeMap<Key, usize> = BTreeMap::new();
    // Draw ORDER, not order of appearance. Within one frame the walk lists instances back to
    // front, so an object's mean position in that list is where it belongs in the stack. Ordering
    // by first appearance instead puts anything that shows up late in the timeline (clocktown's
    // storm sky arrives on the third day) in FRONT of everything, which reads as the backdrop
    // being replaced by a flat colour.
    let mut rank: BTreeMap<Key, (f64, usize)> = BTreeMap::new();
    // ONLY THE FRAMES WHERE A THING IS ACTUALLY SEEN decide where it sits in the stack. An object
    // that is faded out to nothing is still placed, and counting those frames answers a question
    // about where it was PUT rather than what covers what. It matters because an object can be
    // re-placed at a different depth partway through the day: clocktown draws its clock face
    // behind its neighbour for the hours it is invisible and in front of it for the cross-fade
    // that makes it appear, and one static order has to pick the one the eye ever sees.
    let seen = |i: &Instance| member(i) && i.alpha > 0.02;
    if std::env::var("PEPTIDE_TRACK_DEBUG").is_ok() {
        // what a member is never seen at: an object placed at no opacity is an object this
        // walk never picks up, and it says so rather than leaving it out in silence
        let mut faint: BTreeMap<String, (usize, f64)> = BTreeMap::new();
        for (insts, _) in sampled.iter() {
            for i in insts.iter().filter(|i| member(i) && i.alpha <= 0.02) {
                let e = faint.entry(i.sym_name.clone()).or_insert((0, 0.0));
                e.0 += 1; e.1 = e.1.max(i.alpha);
            }
        }
        for (n, (c, a)) in faint {
            eprintln!("[track] {n:?}: {c} placements at opacity {a:.3} or less, never seen");
        }
    }
    for (insts, _) in sampled.iter() {
        let members: Vec<&Instance> = insts.iter().filter(|i| seen(i)).collect();
        if members.is_empty() { continue; }
        let last = (members.len().max(2) - 1) as f64;
        for (pos, i) in members.iter().enumerate() {
            let k = (i.inst_path, i.leaf_depth);
            name_of.entry(k).or_insert_with(|| i.sym_name.clone());
            if let Some(c) = i.inst_clip { clip_of.entry(k).or_insert(c); }
            let c = chain_of.entry(k).or_insert(1);
            *c = (*c).max(i.chain_len);
            let e = rank.entry(k).or_insert((0.0, 0));
            e.0 += pos as f64 / last;
            e.1 += 1;
        }
    }
    // WHO IS IN FRONT OF WHOM, decided on the frames where both are actually drawn. A mean
    // position over the whole timeline compares objects that are never on screen together, and the
    // answer it gives is about when they appear rather than how deep they are: clocktown's white
    // flash exists only at the end of the day, so against a clock that is up all day its mean puts
    // it behind, and the clock draws over a flash that covers the sky.
    let mut before: BTreeMap<(Key, Key), i64> = BTreeMap::new();
    for (insts, _) in sampled.iter() {
        let members: Vec<&Instance> = insts.iter().filter(|i| seen(i)).collect();
        for (a, ia) in members.iter().enumerate() {
            for ib in members.iter().skip(a + 1) {
                let (ka, kb) = ((ia.inst_path, ia.leaf_depth), (ib.inst_path, ib.leaf_depth));
                if ka == kb { continue; }
                let (lo, hi, sign) = if ka < kb { (ka, kb, 1) } else { (kb, ka, -1) };
                *before.entry((lo, hi)).or_insert(0) += sign;
            }
        }
    }
    let order: Vec<Key> = rank.keys().copied().collect();
    let mean = |k: &Key| rank[k].0 / rank[k].1.max(1) as f64;
    // THE SOURCE'S OWN ORDER, kept as an order rather than scored into one.
    //
    // Each frame lists what it draws back to front, so every pair that shares a frame states which
    // of the two is in front. Counting those statements per object and sorting on the count reads
    // as popularity: an object on screen all day is in front of dozens of things, and one that is
    // only up for the length of a cross-fade is in front of one. The short-lived one then sorts
    // behind the thing it is supposed to be covering -- which is a cross-fade playing backwards,
    // the incoming picture sitting on top of the outgoing one at full opacity and the change
    // arriving in a single frame instead of over the fade. Clocktown's clock face did exactly
    // that while the backdrop behind it faded correctly.
    //
    // So the pairs are resolved as what they are: edges. A topological walk honours every one it
    // can, and where the source never draws two things together (nothing to honour) the old
    // heuristics decide -- mean draw position, then depth.
    let mut edges: BTreeMap<Key, Vec<Key>> = BTreeMap::new();   // back -> fronts
    let mut indeg: BTreeMap<Key, usize> = order.iter().map(|k| (*k, 0)).collect();
    for ((lo, hi), v) in &before {
        if *v == 0 { continue; }
        let (back, front) = if *v > 0 { (lo, hi) } else { (hi, lo) };
        edges.entry(*back).or_default().push(*front);
        *indeg.entry(*front).or_insert(0) += 1;
    }
    let mut sorted: Vec<Key> = Vec::with_capacity(order.len());
    let mut left: Vec<Key> = order.clone();
    while !left.is_empty() {
        // everything nothing is still behind, furthest back first
        let mut ready: Vec<Key> = left.iter().copied().filter(|k| indeg[k] == 0).collect();
        if ready.is_empty() {
            // the statements contradict each other (they can, across frames): take the one the
            // source draws furthest back on average and carry on rather than stall
            ready = vec![*left.iter().min_by(|a, b|
                mean(a).partial_cmp(&mean(b)).unwrap_or(std::cmp::Ordering::Equal)
                    .then(a.1.cmp(&b.1))).expect("left is not empty")];
        }
        ready.sort_by(|a, b| mean(a).partial_cmp(&mean(b))
            .unwrap_or(std::cmp::Ordering::Equal).then(a.1.cmp(&b.1)));
        let k = ready[0];
        left.retain(|x| *x != k);
        indeg.insert(k, usize::MAX); // taken
        for f in edges.get(&k).cloned().unwrap_or_default() {
            if let Some(d) = indeg.get_mut(&f) { if *d != usize::MAX { *d -= 1; } }
        }
        sorted.push(k);
    }
    let order = sorted;

    // WHICH SIDE OF THE BAKED PICTURE an object belongs on. The stage's still art is flattened
    // into one layer, and a promoted object is reparented into a container either in front of it
    // or behind it. The source decides: a still that OVERLAPS this object and is drawn after it is
    // art this object belongs under. Clocktown's braziers are the case -- the basket is a still,
    // the flame is drawn beneath it, and in front of the baked layer the flame covers the lattice
    // that should be silhouetted against it.
    let mut obj_box: BTreeMap<Key, Rect> = BTreeMap::new();
    let mut obj_shapes: BTreeMap<Key, std::collections::BTreeSet<u16>> = BTreeMap::new();
    // a track whose clip hides itself on its first frame starts unseen
    let mut obj_hidden: BTreeMap<Key, bool> = BTreeMap::new();
    let mut named_of: BTreeMap<Key, Vec<String>> = BTreeMap::new();
    for (insts, _) in sampled.iter() {
        for i in insts.iter().filter(|i| member(i)) {
            let k = (i.inst_path, i.leaf_depth);
            obj_shapes.entry(k).or_default().insert(i.shape_id);
            *obj_hidden.entry(k).or_default() |= i.hidden_by_class;
            named_of.entry(k).or_insert_with(|| i.named_path.clone());
            obj_box.entry(k)
                .and_modify(|r| {
                    let (x0, y0) = (r.x.min(i.aabb.x), r.y.min(i.aabb.y));
                    let (x1, y1) = ((r.x + r.w).max(i.aabb.x + i.aabb.w), (r.y + r.h).max(i.aabb.y + i.aabb.h));
                    *r = Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
                })
                .or_insert(i.aabb);
        }
    }
    let at: BTreeMap<Key, usize> = order.iter().enumerate().map(|(n, k)| (*k, n)).collect();
    let overlaps = |a: &Rect, b: &Rect| a.x < b.x + b.w && b.x < a.x + a.w
                                     && a.y < b.y + b.h && b.y < a.y + a.h;
    let behind_base: BTreeMap<Key, bool> = order.iter().map(|k| {
        let (mut under, mut over) = (0usize, 0usize);
        if let (Some(mine), Some(&me)) = (obj_box.get(k), at.get(k)) {
            for o in order.iter() {
                if o == k { continue; }
                // only STILLS: an object that animates is promoted too, and two promoted objects
                // sit in the same container -- their order there is already the stack order.
                if obj_shapes.get(o).map(|s| s.len()) != Some(1) { continue; }
                match (obj_box.get(o), at.get(o)) {
                    (Some(ob), Some(&pos)) if overlaps(mine, ob) => {
                        if pos > me { under += 1 } else { over += 1 }
                    }
                    _ => {}
                }
            }
        }
        (*k, under > over)
    }).collect();

    // Presence per object, so the ones that are really ONE object can be put back together.
    let presence: BTreeMap<Key, Vec<Option<&Instance>>> = order.iter().map(|k| {
        (*k, sampled.iter()
            .map(|(insts, _)| insts.iter().find(|i| member(i) && (i.inst_path, i.leaf_depth) == *k))
            .collect::<Vec<_>>())
    }).collect();

    // One object RE-PLACED is not two objects. A clip that puts its child on a different depth
    // (or re-places the subtree, which changes the path it is keyed by) hands us two keys that are
    // never on screen at the same time and draw the same shapes -- the second takes over exactly
    // where the first stops. Emitted apart, each is blank for the frames the other covers, so the
    // piece disappears and comes back mid-turn, and each half works out its own turning point.
    // Detected by the SHAPE of the presence (disjoint in time, same art, same clip), never by who
    // the object is.
    let mut merged: Vec<Vec<Key>> = Vec::new();
    let mut taken: std::collections::BTreeSet<Key> = Default::default();
    for k in order.iter() {
        if taken.contains(k) { continue; }
        let mut group = vec![*k];
        taken.insert(*k);
        // the frames this group already occupies. A candidate has to be disjoint from ALL of
        // them, not just from the object that started the group: tested against the leader only,
        // two pieces that are each disjoint from IT but overlap EACH OTHER both join, and the
        // group can only ever draw one object per frame -- so one of them stops being drawn.
        // Clocktown's clock face lost a ring that way and wore the colour of the one underneath.
        let mut occupied: Vec<bool> = presence[k].iter().map(|s| s.is_some()).collect();
        for o in order.iter() {
            if taken.contains(o) { continue; }
            if name_of.get(k) != name_of.get(o) { continue; }
            // Never on screen together AND standing in the same place. Disjoint alone is not
            // enough: a whole plane can share one clip name -- every camera background here is
            // `clocktown_bg` -- so any two pieces that happen not to overlap in time would be
            // welded into one object that jumps between them, which is a clock that snaps to
            // another piece's frame partway through its turn. Two appearances of one object are
            // in the same place; two different objects are not.
            let disjoint = presence[o].iter().zip(&occupied)
                .all(|(a, busy)| a.is_none() || !*busy);
            let same_spot = match (presence[k].iter().flatten().next(),
                                   presence[o].iter().flatten().next()) {
                (Some(x), Some(y)) => (x.cx - y.cx).abs() < 8.0 && (x.cy - y.cy).abs() < 8.0,
                _ => false,
            };
            if disjoint && same_spot {
                for (slot, inst) in occupied.iter_mut().zip(&presence[o]) {
                    if inst.is_some() { *slot = true; }
                }
                group.push(*o);
                taken.insert(*o);
            }
        }
        merged.push(group);
    }

    // WHERE A MERGED OBJECT SITS IN THE STACK is decided for the OBJECT, not for one of its
    // placements.
    //
    // A group is one object the source re-places, and a re-placement can move it to a different
    // depth. Ordered by whichever placement came first in the stack, it sits where it was while
    // nobody could see it: clocktown parks its clock face behind its neighbour for the hours it is
    // faded out and moves it in front for the cross-fade that brings it back. Ordered by the
    // parked placement, the incoming face draws OVER the outgoing one at full opacity and the
    // change lands in a single frame instead of over the fade -- the face snapping while the rest
    // of the backdrop dissolves.
    //
    // So the pairwise evidence is gathered again with every placement folded into its object, and
    // only from frames where both objects are actually SEEN, since that is the only time their
    // order shows. Then the same topological walk as before, over objects.
    let gid_of: BTreeMap<Key, usize> = merged.iter().enumerate()
        .flat_map(|(g, ks)| ks.iter().map(move |k| (*k, g))).collect();
    let ng = merged.len();
    let mut gbefore: BTreeMap<(usize, usize), i64> = BTreeMap::new();
    let mut grank: Vec<(f64, usize)> = vec![(0.0, 0); ng];
    for (insts, _) in sampled.iter() {
        let members: Vec<usize> = insts.iter().filter(|i| seen(i))
            .filter_map(|i| gid_of.get(&(i.inst_path, i.leaf_depth)).copied()).collect();
        if members.is_empty() { continue; }
        let last = (members.len().max(2) - 1) as f64;
        for (pos, g) in members.iter().enumerate() {
            grank[*g].0 += pos as f64 / last;
            grank[*g].1 += 1;
        }
        for (a, ga) in members.iter().enumerate() {
            for gb in members.iter().skip(a + 1) {
                if ga == gb { continue; }
                let (lo, hi, sign) = if ga < gb { (*ga, *gb, 1) } else { (*gb, *ga, -1) };
                *gbefore.entry((lo, hi)).or_insert(0) += sign;
            }
        }
    }
    let gmean = |g: usize| grank[g].0 / grank[g].1.max(1) as f64;
    let mut gedges: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    let mut gindeg: Vec<usize> = vec![0; ng];
    for ((lo, hi), v) in &gbefore {
        if *v == 0 { continue; }
        let (back, front) = if *v > 0 { (*lo, *hi) } else { (*hi, *lo) };
        gedges.entry(back).or_default().push(front);
        gindeg[front] += 1;
    }
    let mut gsorted: Vec<usize> = Vec::with_capacity(ng);
    let mut gleft: Vec<usize> = (0..ng).collect();
    let mut gtaken = vec![false; ng];
    while !gleft.is_empty() {
        let mut ready: Vec<usize> = gleft.iter().copied().filter(|g| gindeg[*g] == 0).collect();
        if ready.is_empty() {
            ready = vec![*gleft.iter().min_by(|a, b| gmean(**a).partial_cmp(&gmean(**b))
                .unwrap_or(std::cmp::Ordering::Equal).then(a.cmp(b))).expect("gleft is not empty")];
        }
        ready.sort_by(|a, b| gmean(*a).partial_cmp(&gmean(*b))
            .unwrap_or(std::cmp::Ordering::Equal).then(a.cmp(b)));
        let g = ready[0];
        gleft.retain(|x| *x != g);
        gtaken[g] = true;
        for f in gedges.get(&g).cloned().unwrap_or_default() {
            if !gtaken[f] && gindeg[f] > 0 { gindeg[f] -= 1; }
        }
        gsorted.push(g);
    }
    let merged: Vec<Vec<Key>> = gsorted.into_iter().map(|g| merged[g].clone()).collect();

    let mut out: Vec<BgLayer> = Vec::new();
    for group in merged {
        let key = group[0];
        // this object's instance in each frame (None where the timeline does not show it)
        // past one lap only for a piece whose clips take longer to come round
        let span = match lap {
            Some(l) if chain_of.get(&key).copied().unwrap_or(1) <= l => l.min(sampled.len()),
            Some(_) => chain_of.get(&key).copied().unwrap_or(1).min(sampled.len()),
            None => sampled.len(),
        };
        let per_frame: Vec<Option<&Instance>> = (0..span)
            .map(|f| group.iter().find_map(|k| presence[k][f]))
            .collect();
        let shown: Vec<&Instance> = per_frame.iter().flatten().copied().collect();
        if shown.is_empty() {
            if std::env::var("PEPTIDE_TRACK_DEBUG").is_ok() {
                eprintln!("[track] {:?} key={key:?}: never shown, skipped", name_of.get(&key));
            }
            continue;
        }
        let placed_on: Vec<bool> = per_frame.iter().map(|s| s.is_some()).collect();

        // Keyed by (shape, angle STEP). A piece is rasterised at each angle it is actually drawn
        // at, rounded to a step, and placed by its own box with no rotation of ours. That is how
        // the source draws it, so it lands where the source lands it. The angles are shared and
        // deduplicated, so a turning ring would cost a handful of pictures, not one a frame.
        //
        // EVERY object is built the way the source builds it: its own picture, placed. Not only the
        // turning ones -- a piece that is moved, or scaled, or simply sits still is the same thing
        // with a duller transform, and drawing those into a fresh picture per frame is redrawing
        // art the source stores once.
        //
        // A clip that draws SEVERAL of a thing at once (a burst of rain splashes, a torch's
        // embers) is several objects, not one picture: slot `k` of every frame is one of them, and
        // each slot is placed on its own. Only what still declines falls through to the bake.
        let matches = |i: &Instance| member(i) && group.contains(&(i.inst_path, i.leaf_depth));
        let slots = frame_slots(sampled, &matches);
        let widest = slots.iter().map(|f| f.len()).max().unwrap_or(0);
        // Which of the several is which, held to the SAME slot across frames. Position in the
        // frame's list cannot do it: the moment one of them is not drawn, every later one shifts
        // down a place and a slot changes identity mid-animation, so a piece swaps for another and
        // part of the object blinks out. Where the shapes tell them apart (a thing built of
        // distinct pieces, like a clock face) that is what keys them; where they do not (a burst of
        // one particle drawn many times) order is all there is and is used.
        let by_shape = slots.iter().all(|f| {
            let mut ids: Vec<u16> = f.iter().map(|i| i.shape_id).collect();
            ids.sort_unstable();
            let n = ids.len();
            ids.dedup();
            ids.len() == n
        });
        // ...but a shape is not automatically its own OBJECT. A piece that changes cel over the day
        // -- clocktown's clock face has a day one and a night one -- draws each cel at the same
        // place and never both at once, so keying by shape stacks it as two objects with two
        // independent schedules: one of them switches early and the face snaps ahead of the pieces
        // around it. Cels that never share a frame and stand in the same place are ONE thing
        // changing, so they share a slot; shapes drawn together stay apart, which is what the
        // keying is for.
        let keys: Vec<Vec<u16>> = if by_shape {
            let mut v: Vec<u16> = slots.iter().flatten().map(|i| i.shape_id).collect();
            v.sort_unstable();
            v.dedup();
            // where each shape sits, and which shapes it is ever drawn alongside
            let mut at: std::collections::BTreeMap<u16, (f64, f64)> = Default::default();
            // counted, not just noted: a piece CROSSFADING to its next cel draws both for the few
            // frames of the fade, so "never together" would keep the two apart and stack them
            // again. Drawn together throughout is two pieces; drawn together in passing is one
            // piece changing.
            let mut together: std::collections::BTreeMap<(u16, u16), usize> = Default::default();
            let mut seen: std::collections::BTreeMap<u16, usize> = Default::default();
            for f in slots.iter() {
                for i in f.iter() {
                    at.entry(i.shape_id).or_insert((i.cx, i.cy));
                    *seen.entry(i.shape_id).or_default() += 1;
                }
                for a in f.iter() {
                    for b in f.iter() {
                        if a.shape_id != b.shape_id {
                            *together.entry((a.shape_id.min(b.shape_id), a.shape_id.max(b.shape_id)))
                                .or_default() += 1;
                        }
                    }
                }
            }
            let mut groups: Vec<Vec<u16>> = Vec::new();
            for id in v {
                let slot = groups.iter_mut().find(|g| g.iter().all(|o| {
                    let pair = (*o.min(&id), *o.max(&id));
                    let overlap = together.get(&pair).copied().unwrap_or(0);
                    let life = seen.get(o).copied().unwrap_or(0).min(seen.get(&id).copied().unwrap_or(0));
                    overlap * 10 < life.max(1)
                        && at.get(o).zip(at.get(&id)).is_some_and(|(p, q)| {
                            (p.0 - q.0).abs() < 8.0 && (p.1 - q.1).abs() < 8.0
                        })
                }));
                match slot { Some(g) => g.push(id), None => groups.push(vec![id]) }
            }
            groups
        } else { Vec::new() };
        let n_slots = if by_shape { keys.len() } else { widest };
        let mut extra: Vec<Vec<StageArt>> = Vec::new();
        // each slot's OWN placement, so the object made from it can be told when to be up. a slot
        // is a separate object and the stage puts it down on its own frames.
        let mut slot_placed: Vec<Vec<bool>> = Vec::new();
        let placed = (0..n_slots).map(|k| {
            let pf: Vec<Option<&Instance>> = slots.iter().map(|f| if by_shape {
                f.iter().find(|i| keys[k].contains(&i.shape_id)).copied()
            } else {
                f.get(k).copied()
            }).collect();
            slot_placed.push(pf.iter().map(|s| s.is_some()).collect());
            single_shape_frames(&pf, ox, oy, scale, shape_defs, bitmaps)
        }).collect::<Option<Vec<_>>>().and_then(|mut v| {
            if v.is_empty() { return None; }
            extra = v.split_off(1);
            v.pop()
        });
        let slot_presence = |on: &[bool]| -> Vec<(usize, usize)> {
            let mut p: Vec<(usize, usize)> = Vec::new();
            for (f, s) in on.iter().enumerate() {
                if !s { continue; }
                let (a, b) = (f * FRAME_RATIO_USIZE, (f + 1) * FRAME_RATIO_USIZE);
                match p.last_mut() { Some(r) if r.1 == a => r.1 = b, _ => p.push((a, b)) }
            }
            // short absences belong INSIDE the object, as its own animation, not in a schedule the
            // stage runs against it
            let mut m: Vec<(usize, usize)> = Vec::new();
            for r in p {
                match m.last_mut() {
                    Some(q) if r.0 - q.1 <= REAL_GAP * FRAME_RATIO_USIZE => q.1 = r.1,
                    _ => m.push(r),
                }
            }
            if m.len() == 1 && m[0].0 == 0 { m.clear(); }
            m
        };

        // value = the picture, plus the BOX it was rasterised from. A frame that draws the piece
        // smaller reuses the same picture scaled down to its own box, so size follows the source
        // frame by frame instead of snapping to whatever the biggest instance in the bucket was.
        // value = the picture, the scale it was baked at, and WHERE ITS CORNER SITS relative to the
        // placement's own origin. That offset is what puts the picture back: a frame draws the same
        // art from its own origin, so the corner is the origin plus the offset, scaled. Taken as
        // "centred on the placement" instead, every piece landed half its own size up and left --
        // which is where bowser's castle's Bowser was, floating off his throne.
        let mut cel: BTreeMap<(u16, i32), (StageArt, f64, f64, f64, f64)> = BTreeMap::new();
        /// Degrees per rasterised step. Fine enough that a turn reads as smooth, coarse enough
        /// that a full revolution is a couple of dozen pictures instead of a few hundred -- and a
        /// piece drawn in several variants over a day/night cycle pays that per variant, so the
        /// step is what keeps a turning clock from being most of the package.
        const ANGLE_STEP: f64 = 10.0;
        let step_of = |i: &Instance| -> i32 {
            (crate::fm_placement::normalise_degrees(i.place.rotation) / ANGLE_STEP).round() as i32
        };
        // one raster per (shape, angle) actually shown, taken from the frame where the piece is
        // drawn BIGGEST at that angle (every other frame then scales down, which is sharper)
        let mut want: BTreeMap<(u16, i32), &Instance> = BTreeMap::new();
        for i in shown.iter().copied() {
            let key = (i.shape_id, step_of(i));
            let better = want.get(&key).map(|b| {
                (i.place.sx.abs() * i.place.sy.abs()) > (b.place.sx.abs() * b.place.sy.abs())
            }).unwrap_or(true);
            if better { want.insert(key, i); }
        }
        for (key, best) in want {
            if let Some(art) = composite_layer(&[best], shape_defs, bitmaps, ox, oy, None) {
                cel.insert(key, (art, best.aabb.x - best.cx, best.aabb.y - best.cy,
                    best.place.sx.abs().max(f64::EPSILON), best.place.sy.abs().max(f64::EPSILON)));
            }
        }
        if placed.is_none() && cel.is_empty() {
            if std::env::var("PEPTIDE_TRACK_DEBUG").is_ok() {
                eprintln!("[track] {:?} key={key:?}: no picture rasterised, skipped", name_of.get(&key));
            }
            continue;
        }

        if std::env::var("PEPTIDE_WASH").is_ok() && shown.len() > 40 {
            // what a WHITE plate would come out as under each tint: mul*255 + add
            let mut seen: std::collections::BTreeSet<(i64, i64, i64)> = Default::default();
            for i in shown.iter() {
                let c = |m: f64, a: f64| ((255.0 * m + a).clamp(0.0, 255.0)) as i64;
                seen.insert((c(i.tint.mul[0], i.tint.add[0]),
                             c(i.tint.mul[1], i.tint.add[1]),
                             c(i.tint.mul[2], i.tint.add[2])));
            }
            if seen.len() > 3 {
                let reds: Vec<&(i64,i64,i64)> = seen.iter()
                    .filter(|(r,g,b)| *r > g + 30 && *r > b + 30).collect();
                eprintln!("WASH {:?} distinct={} reds={} sample={:?}",
                    name_of.get(&key), seen.len(), reds.len(),
                    seen.iter().take(6).collect::<Vec<_>>());
                if !reds.is_empty() { eprintln!("   RED WASHES: {:?}", &reds[..reds.len().min(8)]); }
            }
        }
        if std::env::var("PEPTIDE_TINTCLIP").is_ok() {
            let mut seen: std::collections::BTreeSet<String> = Default::default();
            let mut n_tinted = 0usize;
            for i in shown.iter() {
                if !i.tint.is_id() { n_tinted += 1; }
                seen.insert(format!("{:?}/{:?}", i.tint.mul.map(|m| (m * 100.0) as i64),
                                                i.tint.add.map(|a| a as i64)));
            }
            if seen.len() > 1 || n_tinted > 0 {
                eprintln!("TINTCLIP {:?} frames={} tinted={} distinct_tints={}",
                    name_of.get(&key), shown.len(), n_tinted, seen.len());
            }
        }
        if std::env::var("PEPTIDE_AUDIT").is_ok() {
            if let Some(pf) = placed.as_ref() {
                let s0: Vec<Option<&Instance>> = slots.iter().map(|f| if by_shape {
                    f.iter().find(|i| keys[0].contains(&i.shape_id)).copied()
                } else { f.first().copied() }).collect();
                let (mut worst_pos, mut worst_rot, mut n) = (0.0f64, 0.0f64, 0usize);
                for (slot, out) in s0.iter().zip(pf.iter()) {
                    let Some(i) = slot else { continue };
                    let Some(sb) = shape_defs.get(&i.shape_id).map(|s| &s.shape_bounds) else { continue };
                    n += 1;
                    // the same mirror the emitter removes: a mirrored placement's pixels already
                    // carry it, so the angle expected here is the one with the mirror taken out
                    let src_rot = crate::fm_placement::normalise_degrees(
                        i.place.b.atan2(i.place.a).to_degrees());
                    let d = (src_rot - out.rotation).abs();
                    worst_rot = worst_rot.max(d.min(360.0 - d));
                    let (x0, y0) = (sb.x_min.to_pixels(), sb.y_min.to_pixels());
                    let (x1, y1) = (sb.x_max.to_pixels(), sb.y_max.to_pixels());
                    let (ct, st) = (out.rotation.to_radians().cos(), out.rotation.to_radians().sin());
                    // what the EMITTER draws: it composes the stage scale onto the frame's own,
                    // so a size read straight off the frame is a whole size_multiplier short of
                    // the thing that reaches the screen
                    let (w, h) = (out.w as f64 * out.scale_x * scale, out.h as f64 * out.scale_y * scale);
                    for (u, v) in [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)] {
                        let ours = (out.x + ct * u - st * v, out.y + st * u + ct * v);
                        let mut near = f64::MAX;
                        for (lu, lv) in [(x0, y0), (x1, y0), (x0, y1), (x1, y1)] {
                            let src = ((i.place.a * lu + i.place.c * lv + i.place.tx - ox) * scale,
                                       (i.place.b * lu + i.place.d * lv + i.place.ty - oy) * scale);
                            near = near.min(((ours.0 - src.0).powi(2) + (ours.1 - src.1).powi(2)).sqrt());
                        }
                        worst_pos = worst_pos.max(near);
                    }
                }
                if n > 0 {
                    let i0 = s0.iter().flatten().next();
                    eprintln!("AUDIT frames={n} worst_pos={worst_pos:.2}px worst_rot={worst_rot:.4}deg \
                        clip={:?} flip={:?} sx={:.3} sy={:.3}",
                        name_of.get(&key), i0.map(|i| i.flip), i0.map(|i| i.place.sx).unwrap_or(0.0),
                        i0.map(|i| i.place.sy).unwrap_or(0.0));
                }
            }
        }
        let anchor = shown[0];
        if std::env::var("PEPTIDE_TRACK_DEBUG").is_ok() {
            eprintln!("[track] {:?} placed={} anchor.aabb=({:.1},{:.1} {:.1}x{:.1}) cx={:.1} cy={:.1}",
                name_of.get(&key), placed.is_some(), anchor.aabb.x, anchor.aabb.y,
                anchor.aabb.w, anchor.aabb.h, anchor.cx, anchor.cy);
        }
        let frames: Vec<StageArt> = match placed {
            Some(f) => f,
            None => per_frame.into_iter().map(|slot| match slot.and_then(|i| cel.get(&(i.shape_id, step_of(i))).map(|c| (i, c))) {
            Some((i, (art, cel_dx, cel_dy, cel_sx, cel_sy))) => {
                // How much bigger this frame draws the piece than the frame the picture was baked
                // from. It comes from the placement's OWN scale, never from the bounding box: a
                // turning piece's box swells toward 45 degrees while the piece itself never changes
                // size, so sizing by the box renders a rotation as a zoom.
                let (sx, sy) = (i.place.sx.abs() / cel_sx, i.place.sy.abs() / cel_sy);
                // centred on the frame's box rather than pinned to its corner, so the leftover
                // angle inside a rasterised step spreads evenly instead of sliding the piece
                StageArt {
                    png: art.png.clone(),
                    x: (i.cx + cel_dx * sx - ox) * scale,
                    y: (i.cy + cel_dy * sy - oy) * scale,
                    w: art.w, h: art.h, hold: 1,
                    rotation: 0.0,
                    scale_x: sx, scale_y: sy,
                    alpha: i.alpha,
                    pivot: (0.0, 0.0),
                    tint: tint_of(&i.tint),
                }
            },
            // not shown this frame: the object is still there, just invisible
            None => StageArt {
                png: blank_png(),
                x: (anchor.aabb.left() - ox) * scale,
                y: (anchor.aabb.top() - oy) * scale,
                w: 1, h: 1, hold: 1, rotation: 0.0, scale_x: 1.0, scale_y: 1.0, alpha: 0.0, pivot: (0.0, 0.0), tint: TINT_ID },
            }).collect(),
        };
        // A clip the stage puts down over and over is playing ITS OWN timeline each time, not one
        // long animation with gaps: clocktown drops a rain splash 265 times, and what plays on each
        // is the same handful of frames. So the object's art is ONE of those placements -- its own
        // loop, which the vfx repeats forever inside its container -- and the container is what the
        // stage puts up and takes away.
        //
        // Only for a clip placed MANY times in SHORT bursts. One long placement is the object just
        // being there, and a piece that turns or travels across it has no loop to lift out at all:
        // cutting one leaves a sequence that means nothing, which is what left a ring adrift and a
        // clock face missing when this was guessed from the pictures instead.
        let runs: Vec<(usize, usize)> = {
            let mut v: Vec<(usize, usize)> = Vec::new();
            for (f, on) in placed_on.iter().enumerate() {
                if !on { continue; }
                match v.last_mut() { Some(r) if r.1 + 1 == f => r.1 = f, _ => v.push((f, f)) }
            }
            v
        };
        // A GAP the stage means is long. A clip blinking through its own animation is absent for a
        // frame or two at a time -- that is the cel being empty, not the stage taking the clip
        // away -- and reading those as placement both chops the clip's animation down to whatever
        // showed in one blink (a splash frozen on a single frame) and switches it on and off every
        // few frames (a torch that flickers). Only an absence measured in seconds is the stage.
        let long_gaps = runs.windows(2).any(|w| w[1].0 - w[0].1 > REAL_GAP)
            || runs.first().is_some_and(|r| r.0 > REAL_GAP);
        const MANY: usize = 3;
        const BRIEF: usize = 120;
        // The art is NOT cut down to one placement. A vfx loops on its own clock, so a piece cut to
        // a single burst is caught wherever that loop happens to be when the stage shows it -- a
        // splash coming in halfway through its own splash. Keeping the whole timeline puts every
        // cel at the moment the source draws it, and the schedule below only decides whether the
        // object is on screen at all.
        let cut_to_one_run = false;
        let _ = (MANY, BRIEF);
        let frames: Vec<StageArt> = if cut_to_one_run {
            let (a, b) = *runs.iter().max_by_key(|(a, b)| b - a).unwrap();
            frames[a..=b.min(frames.len() - 1)].to_vec()
        } else { frames };
        // A clip the stage keeps up is playing ITS OWN loop, and the source keeps that loop's
        // phase when the stage's timeline comes round: the clip is not put down again, it just
        // goes on. Baked over the stage's whole timeline it would start over with the stage
        // instead, and hold twice the frames it has. The loop is read off the pictures: the
        // shortest period the rasterised frames repeat at, over as much of the timeline as they
        // can be checked against (bowser's castle runs 142 frames and its podoboos 82, so 60 of
        // the second period are there to check). A frame the clip's own cel leaves empty is a
        // frame of that loop like any other, which is why this is judged on the stage's gaps
        // rather than the pictures' -- a podoboo is up for a few frames of its arc and gone for
        // the rest, all inside its own loop.
        let same = |a: &StageArt, b: &StageArt| a.png == b.png && a.x == b.x && a.y == b.y
            && a.rotation == b.rotation && a.scale_x == b.scale_x && a.scale_y == b.scale_y
            && a.alpha == b.alpha && a.tint == b.tint;
        // the shortest period a slot's frames repeat at, and whether a frame with something in it
        // was among those checked: a slot up for a few frames of its arc can pass any period that
        // keeps its blank frames aligned, which says nothing
        let period_of = |frames: &[StageArt]| -> Option<(usize, bool)> {
            let n = frames.len();
            if long_gaps || n < 4 { return None; }
            (2..n).find(|&p| {
                let overlap = n - p;
                overlap >= (p / 4).max(8) && (0..overlap).all(|i| same(&frames[i], &frames[i + p]))
            }).map(|p| (p, (0..n - p).any(|i| frames[i].alpha > 0.0 && frames[i].w > 1)))
        };
        let track_name = format!("{:?}", name_of.get(&key));
        // A CLIP THE STAGE DRIVES ACROSS LABELS is a state machine, not a loop: castle siege
        // jumps every plane to `transition1` when the siege moves on. Its art is the clip's
        // label-delimited segments, each an animation of its own that the stage's code can ask
        // for by name, and the loop below is not looked for in it.
        let segments: Vec<BgSegment> = segmenter.map(|f| f(anchor, &frames)).unwrap_or_default();
        if !segments.is_empty() {
            let mut segments = segments;
            for s in &mut segments { s.frames = rle_frames(std::mem::take(&mut s.frames)); }
            let frames: Vec<StageArt> = segments.iter().flat_map(|s| s.frames.iter().cloned()).collect();
            let clip = name_of.get(&key).cloned().unwrap_or_default();
            let name = format!("{clip}_{}_{}", key.0, key.1);
            let fades = frames.iter().any(|f| f.alpha < 1.0);
            if std::env::var("PEPTIDE_TRACK_DEBUG").is_ok() {
                eprintln!("[track] {track_name} is a state machine over {:?}", segments.iter().map(|s| s.label.as_str()).collect::<Vec<_>>());
            }
            keys_out.push(key);
            out.push(BgLayer { named_path: named_of.get(&key).cloned().unwrap_or_default(), starts_hidden: obj_hidden.get(&key).copied().unwrap_or(false), behind_base: behind_base.get(&key).copied().unwrap_or(false), presence: Vec::new(), name, clip: clip.clone(), frames, segments, pan: None, fades, authored_alpha: 1.0 });
            continue;
        }
        // the clip's period is one thing for every slot it draws: taken from the slots that show
        // it, and applied to the ones that only show blank frames where they could be checked
        let mut votes: std::collections::BTreeMap<usize, usize> = Default::default();
        let mut found: Vec<Option<(usize, bool)>> = Vec::new();
        found.push(period_of(&frames));
        for fr in extra.iter() { found.push(period_of(fr)); }
        for (p, sure) in found.iter().flatten() {
            if *sure { *votes.entry(*p).or_default() += 1; }
        }
        // ...and a period the clip's own timeline does not come round on is a stretch of it that
        // happens to look the same (konoha's hinata ends on the picture it starts on, and a 265
        // repeat fitted a 268-frame clip): the clip's own length is its loop, where it was sampled
        // (the clips down its chain come round together, named or not: target test's sliding
        // platform is an unnamed 105-frame clip that starts and ends on the same 25 frames, and an
        // 80 fitted it)
        let owns = [chain_of.get(&key).copied().filter(|&c| c > 1), clip_of.get(&key).and_then(|c| own_len(*c))];
        let clip_period = votes.iter().max_by_key(|(_, c)| **c).map(|(p, _)| *p)
            .map(|p| owns.iter().flatten().copied().find(|&o| {
                // only where the sampled frames themselves repeat at that length too (vacuously
                // so when the sample is exactly one lap of it)
                o > p && o % p != 0 && o <= frames.len()
                    && (0..frames.len() - o).all(|i| same(&frames[i], &frames[i + o]))
            }).unwrap_or(p));
        let own_loop = |frames: Vec<StageArt>, what: &str| -> Vec<StageArt> {
            let Some(p) = clip_period else {
                if std::env::var("PEPTIDE_TRACK_DEBUG").is_ok() && !long_gaps && frames.len() >= 4 {
                    // where the best candidate period breaks, for reading why a clip stayed whole
                    let n = frames.len();
                    let distinct = frames.windows(2).filter(|w| !same(&w[0], &w[1])).count();
                    let best = (2..n).filter(|&p| n - p >= 8).min_by_key(|&p| {
                        (0..n - p).filter(|&i| !same(&frames[i], &frames[i + p])).count()
                    });
                    let detail = best.map(|p| {
                        let bad: Vec<usize> = (0..n - p).filter(|&i| !same(&frames[i], &frames[i + p])).collect();
                        let first = bad.first().map(|&i| {
                            let (a, b) = (&frames[i], &frames[i + p]);
                            format!("first break at {i}: png {} pos ({:.2},{:.2})->({:.2},{:.2}) scale ({:.3},{:.3})->({:.3},{:.3}) alpha {}->{} tint {}",
                                if a.png == b.png { "same" } else { "differs" }, a.x, a.y, b.x, b.y,
                                a.scale_x, a.scale_y, b.scale_x, b.scale_y, a.alpha, b.alpha,
                                if a.tint == b.tint { "same" } else { "differs" })
                        }).unwrap_or_default();
                        format!("nearest period {p} breaks {} of {} times; {first}", bad.len(), n - p)
                    }).unwrap_or_default();
                    eprintln!("[track] {what} no loop in {n} frames ({distinct} changes); {detail}");
                }
                return frames;
            };
            let n = frames.len();
            if p >= n || (0..n - p).any(|i| !same(&frames[i], &frames[i + p])) { return frames; }
            if std::env::var("PEPTIDE_TRACK_DEBUG").is_ok() {
                eprintln!("[track] {what} loops every {p} of {n} frames: kept as its own loop");
            }
            frames[..p].to_vec()
        };
        let frames = own_loop(frames, &track_name);
        let mut collapsed: Vec<StageArt> = Vec::new();
        for f in frames {
            match collapsed.last_mut() {
                // ...and the same COLOUR. A piece whose scheme changes without its picture
                // changing is the same picture in a different colour, and collapsing the two
                // holds the old colour until the piece happens to change art for some other
                // reason -- so pieces that swap art on the turn changed on time while pieces
                // that only change colour lagged behind them.
                Some(prev) if prev.png == f.png && prev.x == f.x && prev.y == f.y
                    && prev.rotation == f.rotation && prev.scale_x == f.scale_x
                    && prev.scale_y == f.scale_y && prev.alpha == f.alpha
                    && prev.tint == f.tint => prev.hold += 1,
                _ => collapsed.push(f),
            }
        }
        // the fade stays on the KEYFRAME (`alpha`), which is how the source expresses it and what
        // keeps a crossfade smooth; baking it into the pictures costs one copy per level and turns
        // a dissolve between two skies into hundreds of megabytes.
        let fades = collapsed.iter().any(|f| f.alpha < 1.0);

        // NOT padded to a shared canvas: these are layers INSIDE one background animation, and
        // only the background itself is sized by the engine. Padding here would move each frame's
        // origin (to keep its content centred) and pull the objects out of alignment with each
        // other.
        // the track's own name stays unique (one per object in the plane), but the CLIP it belongs
        // to is the named ancestor -- so the fifty-three splashes the source keeps under one
        // `clocktown_rainsplash_fg` come back together as that one object
        let clip = name_of.get(&key).cloned().unwrap_or_default();
        let name = format!("{clip}_{}_{}", key.0, key.1);
        // WHEN the source places this object, read off the timeline rather than guessed from what
        // got rasterised. A frame the stage does not place it on is a different thing from a frame
        // whose cel happens to be empty, and only the placement list can tell them apart.
        let mut presence: Vec<(usize, usize)> = Vec::new();
        for (f, slot) in placed_on.iter().enumerate() {
            if !slot { continue; }
            let (a, b) = (f * FRAME_RATIO_USIZE, (f + 1) * FRAME_RATIO_USIZE);
            match presence.last_mut() {
                Some(r) if r.1 == a => r.1 = b,
                _ => presence.push((a, b)),
            }
        }
        // The same rule the runs are judged by, applied to the SCHEDULE the stage actually gets.
        // The gate above is measured on the rasterised runs while the schedule is built from the
        // placement list, so an object could pass the gate and still be handed a schedule that
        // blinks: a piece inside a nested clip is placed on its OWN timeline, which cycles, so
        // sampling it against the whole stage leaves a couple of frames on, a couple off, forever.
        // Switching an object on and off at that rate is not a stage taking a clip away, it is the
        // clip's own animation read as if it were one -- and it costs the object its motion, since
        // it is only ever up for a blink at a time. An absence the STAGE means is measured in
        // seconds; anything shorter belongs inside the object, so close it up.
        let mut merged: Vec<(usize, usize)> = Vec::new();
        for r in presence {
            match merged.last_mut() {
                Some(p) if r.0 - p.1 <= REAL_GAP * FRAME_RATIO_USIZE => p.1 = r.1,
                _ => merged.push(r),
            }
        }
        let mut presence = merged;
        // Shown throughout is no schedule at all -- but ONLY if the art was left whole. An object
        // whose frames were cut down to one placement has to be told when to be up, or it holds
        // that fragment on screen for the rest of the match: clocktown's lighting sheets came out
        // as three frames of a flash, frozen in the plane the fighters stand in.
        // Slots split BY SHAPE are the cels of one animation, not separate objects: each is drawn
        // only on the frames its own shape is up, so its coming and going IS the animation. Switch
        // them on and off for that and a torch flame blinks a couple of hundred times a day.
        if by_shape || !long_gaps
            || (presence.len() == 1 && presence[0].0 == 0 && !cut_to_one_run) {
            presence.clear();
        }
        keys_out.push(key);
        out.push(BgLayer { named_path: named_of.get(&key).cloned().unwrap_or_default(), starts_hidden: obj_hidden.get(&key).copied().unwrap_or(false), behind_base: behind_base.get(&key).copied().unwrap_or(false), presence, name, clip: clip.clone(), frames: collapsed, segments: Vec::new(), pan: None, fades, authored_alpha: 1.0 });
        // the other slots of a clip that draws several at once: each is its own object, in the
        // same clip, so they stay one thing to the stage's code while each is placed on its own
        for (n, frames) in extra.into_iter().enumerate() {
            let frames = own_loop(frames, &format!("{track_name} slot {}", n + 1));
            let mut c: Vec<StageArt> = Vec::new();
            for f in frames {
                match c.last_mut() {
                    Some(prev) if prev.png == f.png && prev.x == f.x && prev.y == f.y
                        && prev.rotation == f.rotation && prev.scale_x == f.scale_x
                        && prev.scale_y == f.scale_y && prev.alpha == f.alpha => prev.hold += 1,
                    _ => c.push(f),
                }
            }
            if c.is_empty() { continue; }
            let fades = c.iter().any(|f| f.alpha < 1.0);
            // slot n+1's own schedule: it is its own object and the stage places it on its own
            // frames. Without one it holds whatever it was cut down to for the whole match.
            let p = if by_shape { Vec::new() } else {
                slot_placed.get(n + 1).map(|on| slot_presence(on)).unwrap_or_default()
            };
            if std::env::var("PEPTIDE_STAGE_DEBUG").is_ok() {
                eprintln!("[track ] {clip}_{}_{}_{} frames={}", key.0, key.1, n + 1, c.len());
            }
            keys_out.push(key);
            out.push(BgLayer { named_path: named_of.get(&key).cloned().unwrap_or_default(), starts_hidden: obj_hidden.get(&key).copied().unwrap_or(false), behind_base: behind_base.get(&key).copied().unwrap_or(false), presence: p, name: format!("{clip}_{}_{}_{}", key.0, key.1, n + 1),
                clip: clip.clone(), frames: c, segments: Vec::new(), pan: None, fades, authored_alpha: 1.0 });
        }
    }
    out
}

/// Where a stage puts a clip that names its own driving class.
///
/// SSF2 stages do not spawn every object from code: a clip placed in the tree says what it is
/// (`self.type = "platform"`) and which class runs it (`self.classAPI = …`), and the game builds
/// that class over that clip's geometry. So the placement is the object's declaration -- its
/// position, its size, and the NAME the stage's own code addresses it by (`getPlatform("left_plat")`).
#[derive(Debug, Clone)]
pub(crate) struct ClassSite {
    /// The class the clip names.
    pub class_api: String,
    /// The role the clip declares (`platform`, `terrain`, …).
    pub kind: String,
    /// The placement's instance name, which is the handle the stage's code uses.
    pub inst_name: Option<String>,
    /// World AABB of everything under the clip, in raw SSF2 coords.
    pub aabb: Rect,
    /// The box of the child the clip holds as its collision (`ground`), world coords, when the
    /// clip has one: a platform draws a saucer and collides with a hidden plate inside it.
    pub ground: Option<Rect>,
    /// The placed clip's own definition: what the object draws, when its class names nothing else.
    pub clip: u16,
    /// The bounce the clip declares (speed, label), when it names no class of its own: the game's
    /// platform layer runs it, so the port stands the placement up as an object that does.
    pub bounce: Option<(f64, String)>,
    /// The movements the clip declares for the game's own platform class to run, when it names
    /// one (`className`, `movement1..N`, `startIndex`, `fallthrough`).
    pub dynamic: Option<DynamicDecl>,
    /// The picture the collision clip names for itself in the foreground (`self.foreground`).
    pub foreground: Option<String>,
}

/// A platform the game's own class moves by declared legs: each leg runs for `move_time` source
/// frames at a speed reached by `accel` a frame and left by `decel` a frame, then waits
/// `wait_time`; the legs cycle from `start_index`.
#[derive(Clone, Debug, PartialEq)]
pub struct DynamicDecl {
    pub legs: Vec<DynamicLeg>,
    pub start_index: u32,
    pub fallthrough: Option<bool>,
}
#[derive(Clone, Debug, PartialEq, Default)]
pub struct DynamicLeg {
    pub move_time: f64, pub wait_time: f64,
    pub x_speed: f64, pub y_speed: f64,
    pub x_accel: f64, pub x_decel: f64, pub y_accel: f64, pub y_decel: f64,
}
impl DynamicDecl {
    /// the declaration's literals (`{ moveTime: 265, waitTime: 10, xSpeed: -2.4, ... }`), read key by key
    pub fn from_role(role: &crate::abc_parser::ClipRole) -> Option<DynamicDecl> {
        if role.movements.is_empty() { return None }
        let kv = crate::re!(r"(\w+)\s*:\s*(-?\d+(?:\.\d+)?)");
        let legs: Vec<DynamicLeg> = role.movements.iter().map(|m| {
            let mut leg = DynamicLeg::default();
            for c in kv.captures_iter(m) {
                let v: f64 = c[2].parse().unwrap_or(0.0);
                match &c[1] {
                    "moveTime" => leg.move_time = v, "waitTime" => leg.wait_time = v,
                    "xSpeed" => leg.x_speed = v, "ySpeed" => leg.y_speed = v,
                    "xAccel" => leg.x_accel = v, "xDecel" => leg.x_decel = v,
                    "yAccel" => leg.y_accel = v, "yDecel" => leg.y_decel = v,
                    _ => {}
                }
            }
            leg
        }).collect();
        Some(DynamicDecl { legs, start_index: role.start_index, fallthrough: role.fallthrough })
    }
}

/// Find every placement whose clip declares a `classAPI`, with the geometry under it.
/// Where a timeline puts the instance named `inst_name` at each of its frame labels, as the
/// step from its first placement (source units). The timeline is whichever sprite (or the root)
/// places that name; a label names the frame whose display list is shown next.
fn placed_at_labels(root: &[swf::Tag], sprites: &BTreeMap<u16, &Vec<swf::Tag>>, inst_name: &str) -> Vec<(String, Option<(f64, f64)>)> {
    let places_it = |tags: &[swf::Tag]| tags.iter().any(|t| matches!(t, swf::Tag::PlaceObject(po)
        if po.name.as_ref().is_some_and(|n| n.to_str_lossy(encoding_rs::WINDOWS_1252) == inst_name)));
    let tags: &[swf::Tag] = if places_it(root) { root } else {
        match sprites.values().find(|t| places_it(t)) { Some(t) => t, None => return Vec::new() }
    };
    let mut at: BTreeMap<u16, (Option<String>, f64, f64)> = BTreeMap::new();
    let mut pending: Vec<String> = Vec::new();
    let mut first: Option<(f64, f64)> = None;
    let mut out = Vec::new();
    for tag in tags {
        match tag {
            swf::Tag::PlaceObject(po) => {
                let e = at.entry(po.depth).or_insert((None, 0.0, 0.0));
                if let Some(n) = &po.name { e.0 = Some(n.to_str_lossy(encoding_rs::WINDOWS_1252).to_string()); }
                if !matches!(po.action, swf::PlaceObjectAction::Modify) && po.name.is_none() { e.0 = None; }
                if let Some(m) = &po.matrix { e.1 = m.tx.get() as f64 / 20.0; e.2 = m.ty.get() as f64 / 20.0; }
            }
            swf::Tag::RemoveObject(r) => { at.remove(&r.depth); }
            swf::Tag::FrameLabel(l) => pending.push(l.label.to_str_lossy(encoding_rs::WINDOWS_1252).to_string()),
            swf::Tag::ShowFrame => {
                let here = at.values().find(|(n, ..)| n.as_deref() == Some(inst_name)).map(|(_, x, y)| (*x, *y));
                if first.is_none() { first = here; }
                // a label the timeline reaches without the instance on its list places it nowhere
                let rel = match (here, first) { (Some((x, y)), Some((fx, fy))) => Some((x - fx, y - fy)), _ => None };
                for l in pending.drain(..) { out.push((l, rel)); }
            }
            _ => {}
        }
    }
    out
}

fn collect_class_sites(
    tags: &[swf::Tag],
    sym_names: &BTreeMap<u16, String>,
    sprites: &BTreeMap<u16, &Vec<swf::Tag>>,
    shape_bounds: &BTreeMap<u16, (f64, f64, f64, f64)>,
    roles: &BTreeMap<String, crate::abc_parser::ClipRole>,
) -> (Vec<ClassSite>, BTreeMap<String, u16>, std::collections::BTreeSet<String>, BTreeMap<String, Rect>, BTreeMap<String, (f64, f64)>) {
    /// Union of the world AABBs of every shape under a clip.
    fn subtree_bounds(
        tags: &[swf::Tag], parent: Mat, rec: usize,
        sprites: &BTreeMap<u16, &Vec<swf::Tag>>,
        shape_bounds: &BTreeMap<u16, (f64, f64, f64, f64)>,
        acc: &mut Option<(f64, f64, f64, f64)>,
    ) {
        if rec > 6 { return }
        for tag in tags {
            let swf::Tag::PlaceObject(po) = tag else { continue };
            let id = match &po.action {
                swf::PlaceObjectAction::Place(i) | swf::PlaceObjectAction::Replace(i) => *i,
                swf::PlaceObjectAction::Modify => continue,
            };
            let local = po.matrix.as_ref().map(|m| Mat {
                a: m.a.to_f64(), b: m.b.to_f64(), c: m.c.to_f64(), d: m.d.to_f64(),
                tx: m.tx.get() as f64 / 20.0, ty: m.ty.get() as f64 / 20.0,
            }).unwrap_or(Mat::id());
            let world = parent.mul(&local);
            if let Some((x0, y0, x1, y1)) = shape_bounds.get(&id) {
                for (cx, cy) in [world.apply(*x0, *y0), world.apply(*x1, *y0), world.apply(*x1, *y1), world.apply(*x0, *y1)] {
                    *acc = Some(match *acc {
                        None => (cx, cy, cx, cy),
                        Some((a, b, c, d)) => (a.min(cx), b.min(cy), c.max(cx), d.max(cy)),
                    });
                }
            }
            if let Some(child) = sprites.get(&id) {
                subtree_bounds(child, world, rec + 1, sprites, shape_bounds, acc);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn walk_sites(
        tags: &[swf::Tag], parent: Mat, rec: usize,
        sym_names: &BTreeMap<u16, String>,
        sprites: &BTreeMap<u16, &Vec<swf::Tag>>,
        shape_bounds: &BTreeMap<u16, (f64, f64, f64, f64)>,
        roles: &BTreeMap<String, crate::abc_parser::ClipRole>,
        out: &mut Vec<ClassSite>,
        named: &mut BTreeMap<String, u16>,
        hidden: &mut std::collections::BTreeSet<String>,
        boxes: &mut BTreeMap<String, Rect>,
        regs: &mut BTreeMap<String, (f64, f64)>,
    ) {
        if rec > 8 { return }
        for tag in tags {
            let swf::Tag::PlaceObject(po) = tag else { continue };
            let id = match &po.action {
                swf::PlaceObjectAction::Place(i) | swf::PlaceObjectAction::Replace(i) => *i,
                swf::PlaceObjectAction::Modify => continue,
            };
            let local = po.matrix.as_ref().map(|m| Mat {
                a: m.a.to_f64(), b: m.b.to_f64(), c: m.c.to_f64(), d: m.d.to_f64(),
                tx: m.tx.get() as f64 / 20.0, ty: m.ty.get() as f64 / 20.0,
            }).unwrap_or(Mat::id());
            let world = parent.mul(&local);
            let sym = sym_names.get(&id).cloned().unwrap_or_default();
            let local_sym = sym.rsplit('.').next().unwrap_or(&sym).to_string();
            // every clip the tree gives a name to, so a class that is handed one by name
            // (`setStance(getMidground().plat1)`) can be given the real art
            let inst = po.name.as_ref().map(|n| n.to_str_lossy(encoding_rs::WINDOWS_1252).to_string());
            if let Some(n) = inst.clone() {
                named.entry(n.clone()).or_insert(id);
                regs.entry(n.clone()).or_insert((world.tx, world.ty));
                // ...and where it is: a stage asks the match for a region by name (crateria's acid
                // spares whoever stands in its `safezone`), and the placement is that region.
                let mut acc = None;
                if let Some(child) = sprites.get(&id) {
                    subtree_bounds(child, world, 0, sprites, shape_bounds, &mut acc);
                }
                if let Some((x0, y0, x1, y1)) = acc {
                    boxes.entry(n).or_insert(Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 });
                }
            }
            // a clip that hides ITSELF on its first frame is one the stage only shows at a moment
            // of its own choosing; it is filed under the name the art walk will carry it by.
            if roles.get(&local_sym).is_some_and(|r| r.starts_hidden) {
                hidden.insert(local_sym.clone());
                if let Some(n) = inst.clone() { hidden.insert(n); }
            }
            // ...and the children a clip hides on its first frame, filed as `Class::child`
            for c in roles.get(&local_sym).map(|r| r.hidden_children.iter()).into_iter().flatten() {
                hidden.insert(format!("{local_sym}::{c}"));
            }
            if let Some(role) = roles.get(&local_sym) {
                // a clip that names a class is an object of that class; one that only declares a
                // bounce is an object too, of nobody's class, standing in for the game's own bounce
                // the legs a clip declares for the game's own platform class are kept on the site for the
                // record: measured live, flat zone plus's game leaves such a plate still, so nothing
                // stands a clip up on that declaration alone
                let dynamic = DynamicDecl::from_role(role);
                // ...and one declared a target is the game's own target (it names no class: the game
                // builds its target over the clip, in every mode)
                if let Some(api) = role.class_api.clone().or_else(|| role.bounce.as_ref().map(|_| String::new()))
                    .or_else(|| role.kind.eq_ignore_ascii_case("target").then(String::new)) {
                    let mut acc = None;
                    if let Some(child) = sprites.get(&id) {
                        subtree_bounds(child, world, 0, sprites, shape_bounds, &mut acc);
                    }
                    let aabb = acc.map(|(x0, y0, x1, y1)| Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 })
                        .unwrap_or(Rect { x: world.tx, y: world.ty, w: 0.0, h: 0.0 });
                    let inst_name = po.name.as_ref()
                        .map(|n| n.to_str_lossy(encoding_rs::WINDOWS_1252).to_string());
                    // the child the clip holds as its collision, by the member the platform api
                    // reads it through: the clip's own class declares `ground` for it, and what is
                    // under that name is the plate fighters stand on (the rest of the clip is art)
                    let mut ground = None;
                    for ct in sprites.get(&id).map(|c| c.iter()).into_iter().flatten() {
                        let swf::Tag::PlaceObject(cp) = ct else { continue };
                        if cp.name.as_ref().map(|n| n.to_str_lossy(encoding_rs::WINDOWS_1252)).as_deref() != Some("ground") { continue }
                        let cid = match &cp.action {
                            swf::PlaceObjectAction::Place(i) | swf::PlaceObjectAction::Replace(i) => *i,
                            swf::PlaceObjectAction::Modify => continue,
                        };
                        let cl = cp.matrix.as_ref().map(|m| Mat {
                            a: m.a.to_f64(), b: m.b.to_f64(), c: m.c.to_f64(), d: m.d.to_f64(),
                            tx: m.tx.get() as f64 / 20.0, ty: m.ty.get() as f64 / 20.0,
                        }).unwrap_or(Mat::id());
                        let cw = world.mul(&cl);
                        let mut acc: Option<(f64, f64, f64, f64)> = None;
                        if let Some((x0, y0, x1, y1)) = shape_bounds.get(&cid) {
                            for (cx, cy) in [cw.apply(*x0, *y0), cw.apply(*x1, *y0), cw.apply(*x1, *y1), cw.apply(*x0, *y1)] {
                                acc = Some(match acc {
                                    None => (cx, cy, cx, cy),
                                    Some((a, b, c, d)) => (a.min(cx), b.min(cy), c.max(cx), d.max(cy)),
                                });
                            }
                        }
                        if let Some(cc) = sprites.get(&cid) {
                            subtree_bounds(cc, cw, 0, sprites, shape_bounds, &mut acc);
                        }
                        ground = acc.map(|(x0, y0, x1, y1)| Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 });
                    }
                    let already = out.iter().any(|s| s.class_api == api
                        && (s.aabb.x - aabb.x).abs() < 0.5 && (s.aabb.y - aabb.y).abs() < 0.5);
                    if !already {
                        out.push(ClassSite { class_api: api, kind: role.kind.clone(), inst_name, aabb, ground, clip: id,
                                             bounce: role.bounce.as_ref().map(|(s, l)| (s.parse::<f64>().unwrap_or(0.0), l.clone())),
                                             dynamic: dynamic.clone(), foreground: role.foreground.clone() });
                    }
                }
            }
            if let Some(child) = sprites.get(&id) {
                walk_sites(child, world, rec + 1, sym_names, sprites, shape_bounds, roles, out, named, hidden, boxes, regs);
            }
        }
    }

    let mut out = Vec::new();
    let mut named = BTreeMap::new();
    let mut hidden = std::collections::BTreeSet::new();
    let mut boxes = BTreeMap::new();
    let mut regs = BTreeMap::new();
    walk_sites(tags, root_mat(tags, sym_names), 0, sym_names, sprites, shape_bounds, roles, &mut out, &mut named, &mut hidden, &mut boxes, &mut regs);
    (out, named, hidden, boxes, regs)
}


/// A timeline can place a picture directly, with no shape in between: the player then makes a
/// Bitmap child of it (which is what a stage script finds by class and paints into). Every walk
/// here reads shapes and clips, so such a placement is given a shape of its own under the
/// picture's id: a rectangle of the picture's size, painted with the picture at scale 1. The
/// shapes outlive the tables that borrow them (a stage is read once per run).
fn placed_picture_shapes(
    root: &[swf::Tag<'_>], bitmaps: &BTreeMap<u16, (u32, u32, Vec<u8>)>,
) -> Vec<(u16, &'static swf::Shape)> {
    let mut placed: std::collections::BTreeSet<u16> = Default::default();
    let mut note = |tags: &[swf::Tag<'_>]| {
        for t in tags {
            if let swf::Tag::PlaceObject(po) = t {
                if let swf::PlaceObjectAction::Place(id) | swf::PlaceObjectAction::Replace(id) = po.action {
                    if bitmaps.contains_key(&id) { placed.insert(id); }
                }
            }
        }
    };
    note(root);
    for t in root {
        if let swf::Tag::DefineSprite(sp) = t { note(&sp.tags); }
    }
    placed.into_iter().filter_map(|id| {
        let (w, h, _) = bitmaps.get(&id)?;
        let tw = swf::Twips::from_pixels(*w as f64);
        let th = swf::Twips::from_pixels(*h as f64);
        let zero = swf::Twips::new(0);
        let bounds = swf::Rectangle { x_min: zero, y_min: zero, x_max: tw, y_max: th };
        // a bitmap fill's matrix maps picture pixels to twips: twenty per pixel
        let matrix = swf::Matrix { a: swf::Fixed16::from_f32(20.0), d: swf::Fixed16::from_f32(20.0), ..swf::Matrix::IDENTITY };
        let styles = swf::ShapeStyles {
            fill_styles: vec![swf::FillStyle::Bitmap { id, matrix, is_smoothed: false, is_repeating: false }],
            line_styles: Vec::new(),
        };
        let edge = |dx: swf::Twips, dy: swf::Twips| swf::ShapeRecord::StraightEdge { delta: swf::PointDelta { dx, dy } };
        let shape = vec![
            swf::ShapeRecord::StyleChange(Box::new(swf::StyleChangeData {
                move_to: Some(swf::Point { x: zero, y: zero }), fill_style_0: None, fill_style_1: Some(1), line_style: None, new_styles: None,
            })),
            edge(tw, zero), edge(zero, th), edge(-tw, zero), edge(zero, -th),
        ];
        let sh: &'static swf::Shape = Box::leak(Box::new(swf::Shape {
            version: 3, id, shape_bounds: bounds.clone(), edge_bounds: bounds, flags: swf::ShapeFlag::empty(), styles, shape,
        }));
        Some((id, sh))
    }).collect()
}

/// A morph shape is placed as itself too. It is read at its start state: the tween between its
/// two states is not carried (a placement's ratio is not read), so the piece draws as it is on
/// its first frame rather than not at all.
fn morph_start_shapes(root: &[swf::Tag<'_>]) -> Vec<(u16, &'static swf::Shape)> {
    root.iter().filter_map(|t| {
        let swf::Tag::DefineMorphShape(m) = t else { return None };
        let sh: &'static swf::Shape = Box::leak(Box::new(swf::Shape {
            version: 3, id: m.id, shape_bounds: m.start.shape_bounds.clone(), edge_bounds: m.start.edge_bounds.clone(),
            flags: swf::ShapeFlag::empty(),
            styles: swf::ShapeStyles { fill_styles: m.start.fill_styles.clone(), line_styles: m.start.line_styles.clone() },
            shape: m.start.shape.clone(),
        }));
        Some((m.id, sh))
    }).collect()
}

/// Every edge of a solid terrain's outline, typed by which side of it is solid, in FM coords.
///
/// A line-based engine collides with edges, not with area: a level drawn as one hollow polygon
/// (corridors inside an outer wall) has floors, ceilings and walls all through it, and the top
/// surface of the whole polygon is its ceiling. So the polygon itself is traced, and the format
/// says which side of every edge is filled: an edge carries a fill on its left, on its right, or
/// on both (a seam between two fills, which is no surface). Each edge is turned to run with the
/// solid on its right, then the direction of travel types it: rightward is a floor, leftward a
/// ceiling, downward the body's right face and upward its left, the same convention the emitter's
/// solid body uses. The work is done in world space so a mirrored placement types its edges the
/// right way round.
fn terrain_outline(shape: &swf::Shape, aabb: &Rect, flip: (bool, bool), ox: f64, oy: f64, scale: f64)
    -> Vec<(&'static str, Vec<(f64, f64)>)>
{
    let sb = &shape.shape_bounds;
    let (min_x, min_y) = (sb.x_min.to_pixels(), sb.y_min.to_pixels());
    let (sw, sh) = ((sb.x_max - sb.x_min).to_pixels().max(1.0), (sb.y_max - sb.y_min).to_pixels().max(1.0));
    let (kx, ky) = (aabb.w / sw, aabb.h / sh);
    let to_fm = |x: f64, y: f64| -> (f64, f64) {
        let wx = if flip.0 { aabb.right() - (x - min_x) * kx } else { aabb.left() + (x - min_x) * kx };
        let wy = if flip.1 { aabb.bottom() - (y - min_y) * ky } else { aabb.top() + (y - min_y) * ky };
        ((wx - ox) * scale, (wy - oy) * scale)
    };
    // every edge with a fill on exactly one side, turned so the solid is on its right, as a
    // flattened polyline in FM px
    let mut edges: Vec<Vec<(f64, f64)>> = Vec::new();
    let (mut cx, mut cy) = (0.0f64, 0.0f64);
    let (mut fs0, mut fs1) = (0u32, 0u32);
    let mut push = |pts: Vec<(f64, f64)>, fs0: u32, fs1: u32| {
        if std::env::var("PEPTIDE_DUMP_OUTLINE").is_ok() {
            let (a, b) = (pts[0], pts[pts.len() - 1]);
            eprintln!("[shape-edge] fs0={fs0} fs1={fs1} ({:.0},{:.0}) -> ({:.0},{:.0})", a.0, a.1, b.0, b.1);
        }
        match (fs0 != 0, fs1 != 0) {
            (false, true) => edges.push(pts),
            (true, false) => { let mut r = pts; r.reverse(); edges.push(r) }
            _ => {}
        }
    };
    for r in &shape.shape {
        match r {
            swf::ShapeRecord::StyleChange(sc) => {
                if let Some(mv) = sc.move_to { cx = mv.x.to_pixels(); cy = mv.y.to_pixels(); }
                if sc.new_styles.is_some() { fs0 = 0; fs1 = 0; }
                if let Some(f) = sc.fill_style_0 { fs0 = f; }
                if let Some(f) = sc.fill_style_1 { fs1 = f; }
            }
            swf::ShapeRecord::StraightEdge { delta } => {
                let (x0, y0) = (cx, cy);
                cx += delta.dx.to_pixels(); cy += delta.dy.to_pixels();
                push(vec![to_fm(x0, y0), to_fm(cx, cy)], fs0, fs1);
            }
            swf::ShapeRecord::CurvedEdge { control_delta, anchor_delta } => {
                let (x0, y0) = (cx, cy);
                let (qx, qy) = (cx + control_delta.dx.to_pixels(), cy + control_delta.dy.to_pixels());
                let (x1, y1) = (qx + anchor_delta.dx.to_pixels(), qy + anchor_delta.dy.to_pixels());
                let pts: Vec<(f64, f64)> = (0..=6).map(|i| {
                    let t = i as f64 / 6.0;
                    let x = (1.0 - t) * (1.0 - t) * x0 + 2.0 * (1.0 - t) * t * qx + t * t * x1;
                    let y = (1.0 - t) * (1.0 - t) * y0 + 2.0 * (1.0 - t) * t * qy + t * t * y1;
                    to_fm(x, y)
                }).collect();
                push(pts, fs0, fs1);
                cx = x1; cy = y1;
            }
        }
    }
    if std::env::var("PEPTIDE_DUMP_OUTLINE").is_ok() {
        for e in &edges {
            let (a, b) = (e[0], e[e.len() - 1]);
            eprintln!("[outline-edge] ({:.0},{:.0}) -> ({:.0},{:.0}) pts={}", a.0, a.1, b.0, b.1, e.len());
        }
    }
    // the solid was on the right in shape space; a mirrored placement puts it on the left
    if flip.0 != flip.1 { for e in &mut edges { e.reverse(); } }

    let steep = 75.0f64.to_radians().tan();
    let kind = |a: (f64, f64), b: (f64, f64)| -> &'static str {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        if dy.abs() > dx.abs() * steep { if dy > 0.0 { "RIGHT_WALL" } else { "LEFT_WALL" } }
        else if dx > 0.0 { "FLOOR" } else { "CEILING" }
    };
    let near = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).abs() < 0.5 && (a.1 - b.1).abs() < 0.5;
    // chain edges that meet end to end into whole contours, WHATEVER their kinds: the kinds are
    // read off the simplified contour, so a curve flattened into a dozen tiny edges does not
    // become a dozen runs alternating between floor and wall (pokemon colosseum's bowl came out
    // as a thousand structures, and the engine crawled at three frames a second)
    let mut contours: Vec<Vec<(f64, f64)>> = Vec::new();
    for e in edges {
        let (a, b) = (e[0], e[e.len() - 1]);
        let joined = contours.iter_mut().rev().take(8).find(|pts| near(pts[pts.len() - 1], a) || near(pts[0], b));
        match joined {
            Some(pts) if near(pts[pts.len() - 1], a) => pts.extend_from_slice(&e[1..]),
            Some(pts) => { let mut head = e[..e.len() - 1].to_vec(); head.append(pts); *pts = head; }
            None => contours.push(e),
        }
    }
    // (a closed contour the size of a bleacher step is still a step: pokemon colosseum's fighters
    // stand on its painted stands, measured live, so nothing small is dropped here)
    let mut runs: Vec<(&'static str, Vec<(f64, f64)>)> = Vec::new();
    for c in contours {
        // a CLOSED contour is simplified as two halves: its chord is a point, and against that
        // the simplifier keeps nothing (jungle island's floor vanished and its raster bands took
        // over, a band's top being fifty px above the true edge)
        let c = if c.len() > 3 && near(c[0], c[c.len() - 1]) {
            let mid = c.len() / 2;
            let mut a = rdp_simplify(&c[..=mid], 1.5);
            let b = rdp_simplify(&c[mid..], 1.5);
            a.extend_from_slice(&b[1..]);
            a
        } else {
            rdp_simplify(&c, 1.5)
        };
        let mut cur: Option<(&'static str, Vec<(f64, f64)>)> = None;
        for w in c.windows(2) {
            let ty = kind(w[0], w[1]);
            match &mut cur {
                Some((t, pts)) if *t == ty => pts.push(w[1]),
                _ => {
                    if let Some(r) = cur.take() { runs.push(r); }
                    cur = Some((ty, vec![w[0], w[1]]));
                }
            }
        }
        if let Some(r) = cur.take() { runs.push(r); }
    }
    runs.into_iter().filter(|(_, pts)| {
        let span = (pts[pts.len() - 1].0 - pts[0].0).abs().max((pts[pts.len() - 1].1 - pts[0].1).abs());
        pts.len() >= 2 && span >= 2.0
    }).collect()
}
