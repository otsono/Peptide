//! A stage's boot: what the stage puts down when the match starts, in the order it puts it down.
//! The converter works these tables out once and writes them into the stage script as calls, one
//! per piece with the row it makes; `runtime/stage/boot.hx` is how each kind of piece is made.
//!
//! Numbers are written at the precision each one has always been written at, so the engine gets
//! the same values it got when the boot was generated text.

use std::fmt::Write as _;

/// Where an element's view is parented.
#[derive(Clone, Debug, PartialEq)]
pub enum BootAt {
    /// one of the stage's containers: "behind", "background" or "foreground"
    Plane(&'static str),
    /// the engine's container for camera background entry `n`
    Camera(usize),
    /// the container that pans at this rate against the camera
    Pan(f64, f64),
}

/// A moving-platform structure, filed under the name the stage's code drives it by.
#[derive(Clone, Debug)]
pub struct BootStructure {
    pub id: String,
    pub name: Option<String>,
}

/// One declared object the boot makes, by its index in the stage's list of objects.
#[derive(Clone, Debug)]
pub struct BootObject {
    pub index: usize,
    pub id: String,
    pub x: f64,
    pub y: f64,
    /// drawn ahead of the fighters
    pub front: bool,
    /// the separate piece it draws in front, riding it
    pub fg: Option<String>,
    /// the name the tree placed it under
    pub name: Option<String>,
    /// made only while hazards are on
    pub gated: bool,
    /// a placed body: made before the fighters are
    pub placed: bool,
    /// a placed body with an entrance over it: (left, top, right) of its deck
    pub deck: Option<(f64, f64, f64)>,
}

/// A surface whose plane moves it by label, riding an element's picture: per label, the
/// offsets per source frame.
#[derive(Clone, Debug)]
pub struct BootFollow {
    pub name: String,
    pub labels: Vec<(String, Vec<(f64, f64)>)>,
}

/// One backdrop element, by its index in the element list.
#[derive(Clone, Debug)]
pub struct BootElement {
    pub index: usize,
    pub id: String,
    /// an object whose script switches its sections (else a looping effect)
    pub segmented: bool,
    pub x: f64,
    pub y: f64,
    pub at: BootAt,
    /// when the stage has it on screen: runs of [from, to) on the stage's clock
    pub shown: Vec<(usize, usize)>,
    /// its colour schedule (an index into the tint tables) and its length
    pub tint: Option<(usize, usize)>,
    /// its own opacity, when under 1
    pub alpha: Option<f64>,
    /// starts hidden
    pub hidden: bool,
    /// the object it rides: (object index, dx, dy, whether its picture never changes)
    pub ride: Option<(usize, f64, f64, bool)>,
    pub follow: Option<BootFollow>,
    /// every instance name over it, outermost first (what the stage's code reaches it through)
    pub path: Vec<String>,
    /// the labels it plays as animations of its own
    pub labels: Vec<String>,
    /// a sectioned element the engine plays by itself (its sections chain in its animation
    /// stats): the animation each label is, and each section's length on the engine's clock
    pub sections: Option<(Vec<String>, Vec<usize>)>,
    /// a piece that carries a named surface: the surface, and the structures it is (its art on
    /// the first)
    pub carry: Option<(String, Vec<String>)>,
    /// a sectioned piece drawn as effects the stage swaps (one in a camera background): which
    /// section follows each, -1 where it holds its last frame
    pub chain: Option<Vec<i64>>,
}

/// A named clip, bound to the elements that came out of it.
#[derive(Clone, Debug)]
pub struct BootGroup {
    /// the handle the stage's code knows it by
    pub name: String,
    /// the source's own name for the clip
    pub source: String,
    /// its pieces (element indices) and the labels each carries
    pub parts: Vec<(usize, Vec<String>)>,
    pub widths: Vec<f64>,
    pub plane: String,
}

/// A camera background's opacity: a schedule of keys, or one constant.
#[derive(Clone, Debug)]
pub enum BootCamBg {
    Fade { index: usize, len: usize, keys: Vec<String> },
    Alpha { index: usize, alpha: String },
}

/// `base + rand * U(0,1)`, as the hundredths draw the script language offers.
#[derive(Clone, Copy, Debug)]
pub struct BootDraw {
    pub lo: f64,
    pub span: i64,
}

/// One weather field's particles.
#[derive(Clone, Debug)]
pub struct BootWeather {
    /// the field's number (its `m_weather` list)
    pub field: usize,
    pub id: String,
    pub hw: i64,
    pub hh: i64,
    pub count: u32,
    pub cap: u32,
    /// the source's area the count is a density over, when it states one
    pub area: Option<f64>,
    pub dy: BootDraw,
    pub dx: BootDraw,
    pub scale: Option<BootDraw>,
}

#[derive(Clone, Debug, Default)]
pub struct StageTables {
    /// how many objects the stage declares: the length of the handle list
    pub objects_declared: usize,
    pub structures: Vec<BootStructure>,
    pub objects: Vec<BootObject>,
    pub elements: Vec<BootElement>,
    /// the distinct colour schedules: (frame, rgb multipliers, rgb offsets) keys
    pub tints: Vec<Vec<(usize, [f64; 6])>>,
    pub groups: Vec<BootGroup>,
    pub cam_bg: Vec<BootCamBg>,
    pub weather: Vec<BootWeather>,
}

fn q(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn opt_q(s: &Option<String>) -> String {
    s.as_deref().map(q).unwrap_or_else(|| "null".into())
}

fn at_literal(at: &BootAt) -> String {
    match at {
        BootAt::Plane(p) => format!("{{ plane: {} }}", q(p)),
        BootAt::Camera(n) => format!("{{ plane: \"camera\", n: {n} }}"),
        BootAt::Pan(px, py) => format!("{{ pan: true, px: {px:.4}, py: {py:.4} }}"),
    }
}

fn draw_literal(d: &BootDraw) -> String {
    format!("{{ lo: {:.3}, span: {} }}", d.lo, d.span)
}

impl StageTables {
    /// anything made while the match starts (not counting the placed bodies, made before it)
    pub fn spawns_at_start(&self) -> bool {
        !self.structures.is_empty()
            || self.objects.iter().any(|o| !o.placed || o.deck.is_some())
            || !self.elements.is_empty()
            || !self.cam_bg.is_empty()
    }
    pub fn has_placed(&self) -> bool { self.objects.iter().any(|o| o.placed) }
    pub fn has_follow(&self) -> bool { self.elements.iter().any(|e| e.follow.is_some()) }
    /// something rides an object: a piece drawn in front of one, or a picture placed in its clip
    pub fn has_riders(&self) -> bool {
        self.objects.iter().any(|o| o.front && o.fg.is_some()) || self.elements.iter().any(|e| e.ride.is_some())
    }
    pub fn has_shown(&self) -> bool { self.elements.iter().any(|e| !e.shown.is_empty()) }
    pub fn has_tint(&self) -> bool { self.elements.iter().any(|e| e.tint.is_some()) }
    pub fn has_pan(&self) -> bool { self.elements.iter().any(|e| matches!(e.at, BootAt::Pan(..))) }
    pub fn has_fade(&self) -> bool { self.cam_bg.iter().any(|c| matches!(c, BootCamBg::Fade { .. })) }
    pub fn is_empty(&self) -> bool {
        !self.spawns_at_start() && !self.has_placed() && self.weather.is_empty()
    }

    /// The calls that make the bodies the tree placed (before the fighters exist), one statement
    /// a line at `indent`.
    pub fn placed_calls(&self, indent: &str, owner: &str) -> String {
        let mut o = self.sized(indent);
        for b in self.objects.iter().filter(|b| b.placed) {
            let _ = writeln!(o, "{indent}StageBoot.object({}, {owner});", object_row(b));
        }
        o
    }

    /// The calls that make the rest once the match is live: structures, the objects (what the
    /// source puts behind the hazards switch only while `hazards` holds), then the backdrop
    /// elements; then each named clip, handed to `bind` as the expression that makes it, and the
    /// camera backgrounds' opacity.
    pub fn spawn_calls(&self, indent: &str, owner: &str, chars: &str, hazards: &str,
                       bind: impl Fn(&BootGroup, String) -> String) -> String {
        let mut o = self.sized(indent);
        for s in &self.structures {
            let _ = writeln!(o, "{indent}StageBoot.structure({}, {});", q(&s.id), opt_q(&s.name));
        }
        for b in &self.objects {
            if b.placed {
                if let Some((l, t, r)) = b.deck {
                    let _ = writeln!(o, "{indent}StageBoot.deck({{ i: {}, deck: {{ l: {l:.1}, t: {t:.1}, r: {r:.1} }} }}, {chars});", b.index);
                }
            } else if !b.gated {
                let _ = writeln!(o, "{indent}StageBoot.object({}, {owner});", object_row(b));
            }
        }
        let gated: Vec<&BootObject> = self.objects.iter().filter(|b| !b.placed && b.gated).collect();
        if !gated.is_empty() {
            let _ = writeln!(o, "{indent}if ({hazards}) {{");
            for b in gated { let _ = writeln!(o, "{indent}\tStageBoot.object({}, {owner});", object_row(b)); }
            let _ = writeln!(o, "{indent}}}");
        }
        let grouped = |i: usize| self.groups.iter().any(|g| g.parts.iter().any(|(k, _)| *k == i));
        for e in &self.elements {
            let row = self.element_row(e);
            let call = if e.chain.is_some() { format!("StageBoot.chained({row})") }
                else if e.carry.is_some() { format!("StageBoot.carry({row})") }
                else if e.ride.is_some_and(|r| r.3) && !grouped(e.index) { format!("StageBoot.still({row}, {owner})") }
                else if e.segmented { format!("StageBoot.segment({row}, {owner})") }
                else { format!("StageBoot.effect({row})") };
            let _ = writeln!(o, "{indent}{call};");
        }
        for g in &self.groups {
            let made = format!("StageBoot.group({{ n: {}, src: {}, parts: [{}], labels: [{}], ws: [{}], plane: {} }})",
                q(&g.name), q(&g.source),
                g.parts.iter().map(|(i, _)| i.to_string()).collect::<Vec<_>>().join(", "),
                g.parts.iter().map(|(_, ls)| format!("[{}]", ls.iter().map(|l| q(l)).collect::<Vec<_>>().join(", ")))
                    .collect::<Vec<_>>().join(", "),
                g.widths.iter().map(|w| format!("{w:.2}")).collect::<Vec<_>>().join(", "),
                q(&g.plane));
            let _ = writeln!(o, "{indent}{}", bind(g, made));
        }
        for c in &self.cam_bg {
            let _ = match c {
                BootCamBg::Fade { index, len, keys } => writeln!(o, "{indent}StageBoot.camFade({index}, {len}, [{}]);", keys.join(", ")),
                BootCamBg::Alpha { index, alpha } => writeln!(o, "{indent}StageBoot.camAlpha({index}, {alpha});"),
            };
        }
        o
    }

    /// Room for every handle the boot files by index.
    fn sized(&self, indent: &str) -> String {
        format!("{indent}StageBoot.sized({}, {});\n", self.objects_declared, self.elements.iter().map(|e| e.index + 1).max().unwrap_or(0))
    }

    /// One element as the row its call makes.
    fn element_row(&self, e: &BootElement) -> String {
        let shown = if e.shown.is_empty() { "null".to_string() } else {
            format!("[{}]", e.shown.iter().map(|(a, b)| format!("[{a}, {b}]")).collect::<Vec<_>>().join(", "))
        };
        let (tint, tlen) = match e.tint.and_then(|(k, len)| Some((self.tints.get(k)?, len))) {
            Some((t, len)) => (format!("[{}]", t.iter()
                .map(|(at, c)| format!("[{at}, {:.4}, {:.4}, {:.4}, {:.1}, {:.1}, {:.1}]", c[0], c[1], c[2], c[3], c[4], c[5]))
                .collect::<Vec<_>>().join(", ")), len.to_string()),
            None => ("null".into(), "0".into()),
        };
        let alpha = e.alpha.map(|a| format!("{a:.3}")).unwrap_or_else(|| "null".into());
        let ride = match e.ride {
            Some((i, dx, dy, _)) => format!("{{ o: {i}, dx: {dx:.4}, dy: {dy:.4} }}"),
            None => "null".into(),
        };
        let follow = match &e.follow {
            Some(f) => format!("{{ n: {}, at: [{}] }}", q(&f.name), f.labels.iter()
                .map(|(l, d)| format!("{{ l: {}, d: [{}] }}", q(l),
                    d.iter().map(|(x, y)| format!("{x:.1}, {y:.1}")).collect::<Vec<_>>().join(", ")))
                .collect::<Vec<_>>().join(", ")),
            None => "null".into(),
        };
        let sections = sections_literal(e);
        let carry = match &e.carry {
            Some((n, ids)) => format!("{{ n: {}, ids: [{}] }}", q(n), ids.iter().map(|c| q(c)).collect::<Vec<_>>().join(", ")),
            None => "null".into(),
        };
        let chain = match &e.chain {
            Some(c) => format!("[{}]", c.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(", ")),
            None => "null".into(),
        };
        format!("{{ i: {}, id: {}, seg: {}, x: {}, y: {}, at: {}, shown: {shown}, tint: {tint}, tlen: {tlen}, alpha: {alpha}, hidden: {}, ride: {ride}, follow: {follow}, sections: {sections}, carry: {carry}, chain: {chain} }}",
            e.index, q(&e.id), e.segmented, e.x, e.y, at_literal(&e.at), e.hidden)
    }
}

/// An element's sections, as the planes play them by label: the animation each is and its length.
pub fn sections_literal(e: &BootElement) -> String {
    match &e.sections {
        Some((anims, lens)) => format!("{{ anims: [{}], lens: [{}] }}",
            anims.iter().map(|a| q(a)).collect::<Vec<_>>().join(", "),
            lens.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(", ")),
        None => "null".into(),
    }
}

/// One declared object as the row its call makes.
fn object_row(b: &BootObject) -> String {
    format!("{{ i: {}, id: {}, x: {:.1}, y: {:.1}, front: {}, fg: {}, name: {} }}",
        b.index, q(&b.id), b.x, b.y, b.front, opt_q(&b.fg), opt_q(&b.name))
}

/// One weather field as the row `StageBoot.weather` scatters.
pub fn weather_row(w: &BootWeather) -> String {
    let area = w.area.map(|a| format!("{a:.1}")).unwrap_or_else(|| "null".into());
    let scale = w.scale.as_ref().map(draw_literal).unwrap_or_else(|| "null".into());
    format!("{{ id: {}, hw: {}.0, hh: {}.0, count: {}, cap: {}, area: {area}, dy: {}, dx: {}, scale: {scale} }}",
        q(&w.id), w.hw, w.hh, w.count, w.cap, draw_literal(&w.dy), draw_literal(&w.dx))
}

/// The runtime's stage boot (`runtime/stage/boot.hx`), as the stage script carries it.
pub fn boot_runtime() -> anyhow::Result<String> {
    let dir = crate::compile::Runtime::locate()
        .ok_or_else(|| anyhow::anyhow!("the runtime's files are not where the converter looks for them"))?;
    let path = dir.join("stage").join("boot.hx");
    std::fs::read_to_string(&path).map_err(|e| anyhow::anyhow!("read {}: {e}", path.display()))
}
