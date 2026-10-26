//! A stage's boot, as data: what the stage puts down when the match starts, in the order it puts
//! it down. The converter works these tables out once; the stage script carries them as literals
//! (`DATA.stage.boot`) and `runtime/stage/boot.hx` does the making.
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
    pub fn is_empty(&self) -> bool {
        !self.spawns_at_start() && !self.has_placed() && self.weather.is_empty()
    }

    /// The tables as a script literal: `DATA.stage.boot`.
    pub fn literal(&self) -> String {
        format!("var DATA = {{ stage: {{ boot: {} }} }};\n", self.boot())
    }

    /// The boot itself, as an object literal.
    pub fn boot(&self) -> String {
        // one row a line, and no comma after the last: not every parser takes a trailing one
        let rows = |rows: Vec<String>| -> String {
            if rows.is_empty() { "[]".to_string() } else { format!("[\n\t\t{}\n\t]", rows.join(",\n\t\t")) }
        };
        let structures = self.structures.iter()
            .map(|s| format!("{{ id: {}, name: {} }}", q(&s.id), opt_q(&s.name))).collect();
        let objects = self.objects.iter().map(|b| {
            let deck = match b.deck {
                Some((l, t, r)) => format!("{{ l: {l:.1}, t: {t:.1}, r: {r:.1} }}"),
                None => "null".into(),
            };
            format!("{{ i: {}, id: {}, x: {:.1}, y: {:.1}, front: {}, fg: {}, name: {}, gated: {}, placed: {}, deck: {} }}",
                b.index, q(&b.id), b.x, b.y, b.front, opt_q(&b.fg), opt_q(&b.name), b.gated, b.placed, deck)
        }).collect();
        let elements = self.elements.iter().map(|e| {
            let shown = if e.shown.is_empty() { "null".to_string() } else {
                format!("[{}]", e.shown.iter().map(|(a, b)| format!("[{a}, {b}]")).collect::<Vec<_>>().join(", "))
            };
            let (tint, tlen) = match e.tint { Some((k, len)) => (k.to_string(), len.to_string()), None => ("null".into(), "0".into()) };
            let alpha = e.alpha.map(|a| format!("{a:.3}")).unwrap_or_else(|| "null".into());
            let ride = match e.ride {
                Some((i, dx, dy, still)) => format!("{{ o: {i}, dx: {dx:.4}, dy: {dy:.4}, still: {still} }}"),
                None => "null".into(),
            };
            let follow = match &e.follow {
                Some(f) => format!("{{ n: {}, at: [{}] }}", q(&f.name), f.labels.iter()
                    .map(|(l, d)| format!("{{ l: {}, d: [{}] }}", q(l),
                        d.iter().map(|(x, y)| format!("{x:.1}, {y:.1}")).collect::<Vec<_>>().join(", ")))
                    .collect::<Vec<_>>().join(", ")),
                None => "null".into(),
            };
            let sections = match &e.sections {
                Some((anims, lens)) => format!("{{ anims: [{}], lens: [{}] }}",
                    anims.iter().map(|a| q(a)).collect::<Vec<_>>().join(", "),
                    lens.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(", ")),
                None => "null".into(),
            };
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
        }).collect();
        let tints = self.tints.iter().map(|t| format!("[{}]", t.iter()
            .map(|(at, c)| format!("[{at}, {:.4}, {:.4}, {:.4}, {:.1}, {:.1}, {:.1}]", c[0], c[1], c[2], c[3], c[4], c[5]))
            .collect::<Vec<_>>().join(", "))).collect();
        let groups = self.groups.iter().map(|g| format!("{{ n: {}, src: {}, parts: [{}], labels: [{}], ws: [{}], plane: {} }}",
            q(&g.name), q(&g.source),
            g.parts.iter().map(|(i, _)| i.to_string()).collect::<Vec<_>>().join(", "),
            g.parts.iter().map(|(_, ls)| format!("[{}]", ls.iter().map(|l| q(l)).collect::<Vec<_>>().join(", ")))
                .collect::<Vec<_>>().join(", "),
            g.widths.iter().map(|w| format!("{w:.2}")).collect::<Vec<_>>().join(", "),
            q(&g.plane))).collect();
        let cam_bg = self.cam_bg.iter().map(|c| match c {
            BootCamBg::Fade { index, len, keys } => format!("{{ i: {index}, len: {len}, k: [{}], a: null }}", keys.join(", ")),
            BootCamBg::Alpha { index, alpha } => format!("{{ i: {index}, len: 0, k: null, a: {alpha} }}"),
        }).collect();
        let weather = self.weather.iter().map(|w| {
            let area = w.area.map(|a| format!("{a:.1}")).unwrap_or_else(|| "null".into());
            let scale = w.scale.as_ref().map(draw_literal).unwrap_or_else(|| "null".into());
            format!("{{ field: {}, id: {}, hw: {}.0, hh: {}.0, count: {}, cap: {}, area: {area}, dy: {}, dx: {}, scale: {scale} }}",
                w.field, q(&w.id), w.hw, w.hh, w.count, w.cap, draw_literal(&w.dy), draw_literal(&w.dx))
        }).collect();
        let mut o = String::new();
        let _ = writeln!(o, "{{");
        let _ = writeln!(o, "\tobjectsDeclared: {},", self.objects_declared);
        let _ = writeln!(o, "\tstructures: {},", rows(structures));
        let _ = writeln!(o, "\tobjects: {},", rows(objects));
        let _ = writeln!(o, "\telements: {},", rows(elements));
        let _ = writeln!(o, "\ttints: {},", rows(tints));
        let _ = writeln!(o, "\tgroups: {},", rows(groups));
        let _ = writeln!(o, "\tcamBg: {},", rows(cam_bg));
        let _ = writeln!(o, "\tweather: {}", rows(weather));
        o.push('}');
        o
    }
}

/// The runtime's stage boot (`runtime/stage/boot.hx`), as the stage script carries it.
pub fn boot_runtime() -> anyhow::Result<String> {
    let dir = crate::compile::Runtime::locate()
        .ok_or_else(|| anyhow::anyhow!("the runtime's files are not where the converter looks for them"))?;
    let path = dir.join("stage").join("boot.hx");
    std::fs::read_to_string(&path).map_err(|e| anyhow::anyhow!("read {}: {e}", path.display()))
}
