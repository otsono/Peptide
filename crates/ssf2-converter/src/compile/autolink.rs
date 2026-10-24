//! A clip's autolink as the engine's own: a move whose code starts the source's autolink (a hit
//! pulls the one struck toward a point beside the fighter) is the engine's autolink angle on its
//! hitboxes, toward an autolink point the animation carries. This reads, from the clip's frame
//! scripts in frame order, where its code starts, moves and stops the autolink and the point it
//! names each time: a point written out (`new Point(x, y)`), or a field of the clip's own that its
//! code set to one before. A point it works out any other way is the engine's default, the
//! hitbox's centre.

use crate::model::ast::*;
use crate::model::ClassModel;

/// how the engine takes the source's autolink (`runtime/autolink.jsonc`)
#[derive(Debug)]
pub struct Table {
    /// what a hitbox states while the autolink is on, by field
    pub box_fields: Vec<(String, String)>,
    pub point_scaled: bool,
}

pub fn table() -> &'static Table {
    static T: std::sync::OnceLock<Table> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let json: serde_json::Value = super::runtime::Runtime::locate()
            .and_then(|d| std::fs::read_to_string(d.join("autolink.jsonc")).ok())
            .and_then(|t| serde_json::from_str(&crate::mappings::strip_jsonc(&t)).ok()).unwrap_or_default();
        Table {
            box_fields: json["box"].as_object().map(|o| o.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect()).unwrap_or_default(),
            point_scaled: json["point_scaled"].as_bool().unwrap_or(false),
        }
    })
}

/// what a clip's code does to the autolink on one of its frames (1-based)
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// the autolink starts (or restarts) toward this point
    Start(f64, f64),
    /// it pulls toward a new point
    Point(f64, f64),
    Stop,
}

#[derive(Clone, Debug, Default)]
pub struct Autolink {
    /// (frame, step), in frame order
    pub steps: Vec<(u32, Step)>,
    /// what its code handed the source's autolink that this engine's has no way to take (a rule
    /// that reshapes the pull, the most power)
    pub dropped: Vec<String>,
}

const START: &[&str] = &["setupAutolinkAngle", "startAutolink"];
const MOVE: &[&str] = &["updateAutolinkTarget", "setAutolinkPoint"];
const STOP: &[&str] = &["stopAutolinkAngle", "stopAutolink"];
const RULE: &[&str] = &["updateAutolinkFunction", "setAutolinkRule"];

/// the clip's autolink, when its code has one and every point it names is known
pub fn of_class(class: &ClassModel) -> Option<Autolink> {
    let mut frames: Vec<(u32, &str)> = class.frame_scripts.iter().map(|fs| (fs.frame + 1, fs.method.as_str())).collect();
    frames.sort();
    let mut out = Autolink::default();
    let mut fields: std::collections::BTreeMap<String, (f64, f64)> = Default::default();
    let mut any = false;
    for (frame, m) in frames {
        let Some(method) = class.methods.iter().find(|x| x.name == m) else { continue };
        walk(&method.function.body, &mut |s| {
            match s {
                Stmt::Assign(p, v) => {
                    if let Some(f) = own_field_place(p) {
                        match point_of(v, &fields) { Some(pt) => { fields.insert(f, pt); } None => { fields.remove(&f); } }
                    } else if let Place::Member(o, axis) = p {
                        if let (Some(f), Some(n)) = (own_field(o), num(v)) {
                            if let Some(pt) = fields.get_mut(&f) {
                                match axis.as_str() { "x" => pt.0 = n, "y" => pt.1 = n, _ => {} }
                            }
                        }
                    }
                }
                Stmt::Expr(Expr::Call(c, args)) => {
                    let Expr::Member(_, name) = &**c else { return };
                    let name = name.as_str();
                    if START.contains(&name) || MOVE.contains(&name) {
                        any = true;
                        // a point worked out as the code runs has no place in the animation: the
                        // engine's own, the hitbox's centre (NaN marks it)
                        let (x, y) = args.first().and_then(|a| point_of(a, &fields)).unwrap_or((f64::NAN, f64::NAN));
                        let step = if START.contains(&name) { Step::Start(x, y) } else { Step::Point(x, y) };
                        out.steps.push((frame, step));
                        if START.contains(&name) {
                            if args.get(1).is_some_and(|a| !matches!(a, Expr::Null | Expr::Undefined)) { out.dropped.push(format!("frame {frame}: the rule that reshapes the pull")); }
                            if args.get(2).is_some_and(|a| !matches!(a, Expr::Null | Expr::Undefined)) { out.dropped.push(format!("frame {frame}: the most power")); }
                        }
                    } else if STOP.contains(&name) {
                        any = true;
                        out.steps.push((frame, Step::Stop));
                    } else if RULE.contains(&name) {
                        any = true;
                        out.dropped.push(format!("frame {frame}: the rule that reshapes the pull"));
                    }
                }
                _ => {}
            }
        });
    }
    if !any || !out.steps.iter().any(|(_, s)| matches!(s, Step::Start(..))) { return None }
    Some(out)
}

/// every statement, in the order the code runs them when each branch is taken
fn walk(stmts: &[Stmt], f: &mut impl FnMut(&Stmt)) {
    for s in stmts {
        f(s);
        match s {
            Stmt::If(_, a, b) => { walk(a, f); walk(b, f); }
            Stmt::Loop(b) | Stmt::While(_, b) | Stmt::DoWhile(b, _) => walk(b, f),
            Stmt::Switch(_, cases, d) => { for (_, b) in cases { walk(b, f) } walk(d, f); }
            Stmt::Try(b, cs) => { walk(b, f); for c in cs { walk(&c.body, f) } }
            _ => {}
        }
    }
}

fn own_field(e: &Expr) -> Option<String> {
    match e {
        Expr::Member(o, n) if matches!(**o, Expr::This) => Some(n.clone()),
        Expr::Name(n) => Some(n.clone()),
        _ => None,
    }
}

fn own_field_place(p: &Place) -> Option<String> {
    match p {
        Place::Member(Expr::This, n) => Some(n.clone()),
        Place::Name(n) => Some(n.clone()),
        _ => None,
    }
}

fn num(e: &Expr) -> Option<f64> {
    match e {
        Expr::Num(v) => Some(*v),
        Expr::Int(v) => Some(*v as f64),
        Expr::Unary(UnOp::Neg, x) => num(x).map(|v| -v),
        Expr::Coerce(_, x) | Expr::Convert(_, x) => num(x),
        _ => None,
    }
}

/// a point written out, or a field of the clip's own holding one
fn point_of(e: &Expr, fields: &std::collections::BTreeMap<String, (f64, f64)>) -> Option<(f64, f64)> {
    match e {
        Expr::New(c, args) if matches!(&**c, Expr::Name(n) if super::short(n) == "Point") => match args.as_slice() {
            [] => Some((0.0, 0.0)),
            [x] => Some((num(x)?, 0.0)),
            [x, y, ..] => Some((num(x)?, num(y)?)),
        },
        Expr::Coerce(_, x) | Expr::Convert(_, x) => point_of(x, fields),
        _ => own_field(e).and_then(|f| fields.get(&f).copied()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(name: &str, args: Vec<Expr>) -> Stmt {
        Stmt::Expr(Expr::Call(Box::new(Expr::Member(Box::new(Expr::Member(Box::new(Expr::This), "self".into())), name.into())), args))
    }
    fn point(x: i64, y: i64) -> Expr { Expr::New(Box::new(Expr::Name("flash.geom:Point".into())), vec![Expr::Int(x), Expr::Int(y)]) }

    #[test]
    fn a_point_held_in_a_field_is_followed_through_the_frames() {
        let steps = |frames: Vec<(u32, Vec<Stmt>)>| {
            let func = |body: Vec<Stmt>| Function { params: vec![], rest: false, return_type: None, local_names: vec![], activation: vec![], body };
            let mut c = ClassModel { name: "clip".into(), super_name: "MovieClip".into(), block: 0, symbol: None, fields: vec![], methods: vec![],
                constructor: func(vec![]), static_init: func(vec![]), frame_scripts: vec![], unreadable: vec![] };
            for (f, body) in frames {
                let name = format!("frame{f}");
                c.frame_scripts.push(crate::model::FrameScript { frame: f - 1, method: name.clone() });
                c.methods.push(crate::model::MethodModel { name, kind: crate::model::MethodKind::Method, is_static: false, function: func(body) });
            }
            of_class(&c)
        };
        let field = || Place::Member(Expr::This, "target".into());
        let read = || Expr::Member(Box::new(Expr::This), "target".into());
        let got = steps(vec![
            (1, vec![Stmt::Assign(field(), point(0, -16)), call("setupAutolinkAngle", vec![read()])]),
            (7, vec![Stmt::Assign(Place::Member(read(), "y".into()), Expr::Int(-21)), call("updateAutolinkTarget", vec![read()])]),
            (12, vec![call("stopAutolinkAngle", vec![])]),
        ]).unwrap();
        assert_eq!(got.steps, vec![(1, Step::Start(0.0, -16.0)), (7, Step::Point(0.0, -21.0)), (12, Step::Stop)]);
        // a point worked out as it runs is the engine's own (unplaced)
        let got = steps(vec![(1, vec![call("setupAutolinkAngle", vec![Expr::Local(1)])])]).unwrap();
        assert!(matches!(got.steps[0], (1, Step::Start(x, _)) if x.is_nan()));
    }
}
