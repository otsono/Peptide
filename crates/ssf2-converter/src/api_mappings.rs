/// SSF2 → Fraymakers API mapping table.
///
/// SSF2 (ActionScript 3) uses a MovieClip timeline model with SSF2API/SSF2Character
/// wrappers. Fraymakers uses Haxe with Entity/Character/GameObject class hierarchy.
///
/// This module provides:
///   1. Method-level mappings (SSF2 call → FM equivalent)
///   2. Property mappings (SSF2 property access → FM getter/setter)
///   3. Constant mappings (SSF2 state constants → FM CState/etc)
///   4. Pattern-level transformations (multi-statement → single FM call)

use std::collections::BTreeMap;

// ─── SSF2 "self.self" Pattern ──────────────────────────────────────────────────
// In decompiled SSF2 sub-MC code, "self.self" refers to the character instance.
// In Fraymakers, the Script.hx `self` already IS the character/entity.
// So "self.self.endAttack()" → "self.endAnimation()"
//
// This is handled at the text level in the post-processor.

/// Remove SSF2 readiness guard if-blocks that are always-true in Fraymakers.
///
/// SSF2 wraps initialization code with guards like:
///   `if (SSF2API.isReady()) { ... }`
///   `if (self && SSF2API.isReady()) { ... }`
///   `if (SSF2API.isReady() && self) { ... }`
///
/// In Fraymakers Script.hx, `self` is always valid and the API is always ready,
/// so these guards are unnecessary. This function removes the if-wrapper and
/// A line with its comments and string contents removed, leaving only what the parser
/// reads as code. Braces inside a note or a message are not structure.
pub(crate) fn code_only(line: &str) -> String {
    let b = line.as_bytes();
    let mut out = String::with_capacity(line.len());
    let mut i = 0usize;
    let mut in_str: Option<u8> = None;
    while i < b.len() {
        let c = b[i];
        if let Some(q) = in_str {
            if c == b'\\' { i += 2; continue }
            if c == q { in_str = None }
            i += 1;
            continue;
        }
        if c == b'/' && i + 1 < b.len() && b[i + 1] == b'/' { break }
        if c == b'/' && i + 1 < b.len() && b[i + 1] == b'*' {
            i += 2;
            while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') { i += 1 }
            i = (i + 2).min(b.len());
            continue;
        }
        if c == b'"' || c == b'\'' { in_str = Some(c); i += 1; continue }
        out.push(c as char);
        i += 1;
    }
    out
}

/// dedents the body by one level, effectively inlining the body.
pub fn remove_readiness_guards(code: &str) -> String {
    let lines: Vec<&str> = code.lines().collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        if is_isready_guard(trimmed) {
            // Count braces to find the matching close-brace of this if block
            let mut depth: i32 = 0;
            let mut found_open = false;
            let mut body_start = i;
            let mut j = i;

            'scan: while j < lines.len() {
                // Only the braces that are CODE. By the time this runs, earlier passes have
                // struck statements out as comments, and a struck `if (...) {` leaves an
                // opening brace sitting in a comment. Counted, it hides the guard's real
                // closing brace one line early and leaves a `}` with nothing to close.
                for ch in code_only(lines[j]).chars() {
                    match ch {
                        '{' => {
                            depth += 1;
                            if !found_open {
                                found_open = true;
                                body_start = j;
                            }
                        }
                        '}' => {
                            depth -= 1;
                            if found_open && depth == 0 {
                                break 'scan;
                            }
                        }
                        _ => {}
                    }
                }
                j += 1;
            }

            // The closing line has to be a closer and nothing else. A guard that ends
            // `} else {` carries the else's OPENING brace on the same line, and dropping
            // the line whole takes that with it, leaving the else's body and its closing
            // brace behind with nothing to belong to.
            let closer_stands_alone = lines.get(j)
                .is_some_and(|l| code_only(l).trim() == "}");
            if found_open && depth == 0 && closer_stands_alone {
                // Emit body lines (body_start+1 .. j), each dedented by one tab
                // Add a comment so the reader knows a guard was stripped
                out.push("// [FM] isReady guard removed — always true in Fraymakers".to_string());
                for body_line in &lines[body_start + 1..j] {
                    out.push(strip_one_tab(body_line).to_string());
                }
                i = j + 1; // skip the closing `}` line
                continue;
            } else {
                // Couldn't parse — emit original
                out.push(line.to_string());
                i += 1;
                continue;
            }
        }

        out.push(line.to_string());
        i += 1;
    }

    // Re-join, preserving trailing newline if original had one
    let mut joined = out.join("\n");
    if code.ends_with('\n') {
        joined.push('\n');
    }
    joined
}

/// Returns true if a trimmed line is a SSF2 readiness guard if-statement: one whose condition
/// is the readiness check and nothing else (`self` beside it is as constant here). A
/// condition that says more (`pos == null && SSF2API.isReady()`, dracula's castle's platform
/// reading its rest position ONCE) is not a guard to drop: the readiness half reads as true
/// and the rest of the test stands.
fn is_isready_guard(trimmed: &str) -> bool {
    // Must start with `if (` or `if(`
    if !trimmed.starts_with("if (") && !trimmed.starts_with("if(") {
        return false;
    }
    // Must contain SSF2API.isReady() — the canonical readiness check
    if !trimmed.contains("SSF2API.isReady()") { return false; }
    let Some(open) = trimmed.find('(') else { return false };
    let Some(close) = trimmed.rfind(')') else { return false };
    if close <= open { return false; }
    // `self`, `self.self` and `parent` (a script's own object, the fighter it drives and the
    // clip it hangs off, every one of them always there) are as constant here as readiness is,
    // and so is a `true` an earlier mapping already put there
    let cond = crate::re!(r"\btrue\b").replace_all(&trimmed[open + 1..close], "")
        .replace("__truthy(SSF2API.isReady())", "")
        .replace("SSF2API.isReady()", "")
        .replace("__truthy(self.self)", "")
        .replace("__truthy(self)", "")
        .replace("__truthy(parent)", "")
        .replace("__truthy()", "")
        .replace("&&", "")
        .replace("self.self", "")
        .replace("self", "")
        .replace("parent", "")
        .replace([' ', '(', ')'], "");
    cond.is_empty()
}

/// Remove `if (… isForcedCrash …) { … }` blocks entirely. SSF2's `isForcedCrash`
/// (a training-mode forced-crash flag) has no Fraymakers concept, so the whole guarded
/// block is obsolete in FM. Leaves a one-line marker in place of the deleted block.
/// (Runs before comment_out_unknown_calls so the block goes away cleanly instead of
/// being dead-commented.)
pub fn remove_forced_crash_guards(code: &str) -> String {
    if !code.contains("isForcedCrash") {
        return code.to_string();
    }
    let lines: Vec<&str> = code.lines().collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut i = 0;
    while i < lines.len() {
        let trimmed = lines[i].trim();
        let is_guard = (trimmed.starts_with("if (") || trimmed.starts_with("if(")
            || trimmed.starts_with("} else if"))
            && trimmed.contains("isForcedCrash")
            && trimmed.ends_with('{');
        if is_guard {
            // brace-match to the block's close, then skip the whole block.
            let mut depth = 0i32;
            let mut found_open = false;
            let mut j = i;
            'scan: while j < lines.len() {
                for ch in lines[j].chars() {
                    match ch {
                        '{' => { depth += 1; found_open = true; }
                        '}' => { depth -= 1; if found_open && depth == 0 { break 'scan; } }
                        _ => {}
                    }
                }
                j += 1;
            }
            if found_open && depth == 0 {
                let indent = &lines[i][..lines[i].len() - lines[i].trim_start().len()];
                out.push(format!("{}// [FM] isForcedCrash guard removed (no forced-crash in Fraymakers)", indent));
                i = j + 1;
                continue;
            }
        }
        out.push(lines[i].to_string());
        i += 1;
    }
    let mut joined = out.join("\n");
    if code.ends_with('\n') { joined.push('\n'); }
    joined
}

/// Strip one tab (or 4 spaces) from the start of a line.
fn strip_one_tab(line: &str) -> &str {
    line.strip_prefix('\t')
        .or_else(|| line.strip_prefix("    "))
        .unwrap_or(line)
}

/// Apply text-level SSF2→FM API translations to decompiled Haxe code.
/// This is a post-processing step run on the output of the decompiler.
///
/// Pipeline order:
///   1. `remove_readiness_guards`      — strip `SSF2API.isReady()` guards
///   2. `double_frame_counts`           — 30→60 fps scaling on frame-typed
///                                        fields BEFORE any rename or split,
///                                        so SSF2 field names still match
///   3. `apply_call_splits`             — fan out SSF2 umbrella calls
///                                        (e.g. updateAttackStats) to their
///                                        FM target methods using
///                                        `call_splits` from commands.jsonc
///   4. literal `replacements`          — text find→replace pairs (ordered;
///                                        SSF2 field names like `hitStun:`
///                                        get renamed to FM `hitstop:` here)
///   5. `regex_replacements`            — regex-based renames
///   6. `strip_last_frame_end_animation` — drop redundant terminal calls
///   7. `comment_out_unknown_calls`     — `ssf2_only` markers + log unknowns
/// SSF2 asks a fighter what it is doing by reading the label its timeline is parked on.
/// This engine has no such property -- it answers the same question with `getAnimation()`, and
/// it answers in ITS OWN animation names. So the read comes across, and so do the labels the
/// read is compared against, or every one of those comparisons is a fighter being asked a
/// question in a language it does not speak: the answer is never the one expected, and code
/// that follows a character while it swings settles for following it forever.
fn resolve_animation_label_reads(code: &str) -> String {
    // The label is a property of the CLIP, so whatever stands immediately before it is how the
    // source reached that clip -- and this engine asks the fighter directly. So the accessor
    // goes with the property it was there to reach; left standing it is a call on nothing
    // ("Invalid function null") and the fighter is never asked at all.
    let asked = {
        let mut out = String::with_capacity(code.len());
        let mut rest = code;
        loop {
            // the whole name and not the start of a longer one: `currentLabels` is the plural,
            // a list of every label there is, and rewriting the singular out of the middle of it
            // leaves an `s` where an expression should be -- the script then does not PARSE, so
            // nothing in it runs and every variable it declares is unknown to its own frames.
            let hit = ["currentFrameLabel", "currentLabel"].iter()
                .filter_map(|p| {
                    let needle = format!(".{p}");
                    rest.match_indices(&needle)
                        .find(|(i, _)| {
                            rest[i + needle.len()..].chars().next()
                                .map(|c| !c.is_alphanumeric() && c != '_')
                                .unwrap_or(true)
                        })
                        .map(|(i, _)| (i, p.len() + 1))
                })
                .min_by_key(|(i, _)| *i);
            let Some((at, len)) = hit else { break };
            let b = rest.as_bytes();
            // step back over the accessor that reached the clip, call-shaped or plain
            let mut cut = at;
            if cut > 0 && b[cut - 1] == b')' {
                let mut depth = 0i32;
                let mut i = cut;
                while i > 0 {
                    i -= 1;
                    match b[i] {
                        b')' => depth += 1,
                        b'(' => { depth -= 1; if depth == 0 { break } }
                        _ => {}
                    }
                }
                while i > 0 && (b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_') { i -= 1 }
                if i > 0 && b[i - 1] == b'.' { cut = i - 1 }
            }
            // the clip's OWN label (`self.currentLabel`, the clip class asking its own timeline)
            // is a label inside the animation, answered off the label table; reached through
            // the fighter (`self.self...`) or an accessor it is the fighter's root label, which
            // is the animation. chibirobo's blaster ends "unless in `fire`", a label inside its
            // neutral special: against the animation's name it ended at once.
            let own_clip = cut == at && rest[..at].ends_with("self") && !rest[..at].ends_with("self.self")
                && !rest[..at].ends_with("_self");
            if own_clip {
                out.push_str(&rest[..at - 4]);
                out.push_str("_currentLabel(self)");
            } else {
                out.push_str(&rest[..cut]);
                out.push_str(".getAnimation()");
            }
            rest = &rest[at + len..];
        }
        out.push_str(rest);
        out
    };

    // The literal on the other side of the comparison is an SSF2 label; this engine answers
    // with its own name for the same move, so the literal is translated the same way the
    // animation itself was when the entity was built.
    let anims = &crate::mappings::character_animations().ssf2_to_fm;
    let mut out = String::with_capacity(asked.len());
    let mut rest = asked.as_str();
    const READ: &str = "getAnimation()";
    while let Some(i) = rest.find(READ) {
        let end = i + READ.len();
        out.push_str(&rest[..end]);
        let after = &rest[end..];
        let b = after.as_bytes();
        let mut j = 0;
        while j < b.len() && (b[j] == b' ' || b[j] == b')') {
            j += 1;
        }
        if !(after[j..].starts_with("==") || after[j..].starts_with("!=")) {
            rest = after;
            continue;
        }
        let mut k = j + 2;
        while k < b.len() && (b[k] == b' ' || b[k] == b'(') {
            k += 1;
        }
        if !after[k..].starts_with('"') {
            rest = after;
            continue;
        }
        let lit_start = k + 1;
        let Some(rel) = after[lit_start..].find('"') else {
            rest = after;
            continue;
        };
        match anims.get(&after[lit_start..lit_start + rel]) {
            Some(fm) => {
                out.push_str(&after[..lit_start]);
                out.push_str(fm);
                rest = &after[lit_start + rel..];
            }
            None => rest = after,
        }
    }
    out.push_str(rest);
    out
}

/// A list the source builds by name is a list here.
///
/// `new Array()` and `new Vector.<int>()` both mean the same empty list, and neither name means
/// anything to this engine -- asked for one, it answers with nothing, and the first `push` on it
/// takes the frame. The brackets are what a list is called here.
fn a_list_by_any_name(code: &str) -> String {
    use regex::Regex;
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| {
        // THE WHOLE NAME, QUALIFIER AND ALL. Replacing only the tail of `self.Vector.Vector.new(5)`
        // leaves `self.[]`, which parses nowhere -- and an unparseable script is dropped whole, so
        // the object and everything it spawns goes with it. Whatever the list is reached through,
        // it is still just a list.
        regex::Regex::new(r"\b(?:[A-Za-z_]\w*\.)*(?:new\s+)?(?:Array|Vector)\s*(?:\.\s*new)?\s*\((?:[^()]|\([^()]*\))*\)").expect("regex")
    });
    re.replace_all(code, "[]").into_owned()
}

/// A pair of coordinates the source builds by name is a pair of names here.
///
/// `new Point()` names a type this engine does not have, so the ask answers with nothing and the
/// first read of `.x` off it takes the frame. A point is two numbers with names, which is a thing
/// the port can make, and everything the stage's own code does with one -- reading the halves,
/// writing them, moving by an amount -- works on that.
///
/// NOT the engine's own point: `makePoint` belongs to the family that allocates an object's
/// state, so it can only be called while the object is still being set up. A stage makes points
/// whenever it likes, and asking for one later is refused outright. Where the ENGINE is handed a
/// point -- a camera target -- it still wants its own, and that boundary is not crossed yet.
fn a_point_is_two_names(code: &str) -> String {
    use regex::Regex;
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r"\b(?:new\s+)?Point\s*(?:\.\s*new)?\s*\(((?:[^()]|\([^()]*\))*)\)").expect("regex")
    });
    re.replace_all(code, |c: &regex::Captures| {
        // split where the commas BETWEEN the two halves are: either half can be worked out by a
        // call of its own, and a plain split comes apart inside it
        let args = crate::stage_emit::top_level_items(&c[1]);
        let x = args.first().filter(|a| !a.is_empty()).map_or("0", |a| a.as_str());
        let y = args.get(1).filter(|a| !a.is_empty()).map_or("0", |a| a.as_str());
        format!("{{ x: {x}, y: {y} }}")
    }).into_owned()
}

/// The rest of the source language's own containers, as the shapes they are.
///
/// `Rectangle` is four numbers with names, `Object` and `Dictionary` are somewhere to put names.
/// None of them are types this engine has, and all of them are types the SOURCE LANGUAGE has --
/// a fixed vocabulary that comes with the format being read, not content anybody wrote. So they
/// are carried across as what they are, and everything done with them afterwards still works.
/// A list built and filled in one breath, put back together.
///
/// The source writes `new <String>["chansey", ...]`, and the compiler builds the list once, keeps
/// a copy on hand for each value it drops in, and only then hands it to the field. Rebuilt from
/// the bytecode that comes out as five fills of a list nothing keeps, followed by the field being
/// given an empty one -- so the field ends up empty and the hazard has no names to pick from.
/// They were always the same list, so they are written as one again.
fn a_list_built_and_filled(code: &str) -> String {
    use regex::Regex;
    static FILL: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    static TAKE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    if !code.contains("[][") { return code.to_string() }
    let fill = FILL.get_or_init(|| Regex::new(r"^\s*\[\]\[(\d+)\]\s*=\s*(.+);\s*$").expect("fill regex"));
    let take = TAKE.get_or_init(|| Regex::new(r"^(\s*)(.+?)\s*=\s*\[\];\s*$").expect("take regex"));
    let lines: Vec<&str> = code.lines().collect();
    let mut out = String::with_capacity(code.len());
    let mut i = 0usize;
    while i < lines.len() {
        let mut j = i;
        let mut vals: Vec<(usize, String)> = Vec::new();
        while j < lines.len() {
            let Some(c) = fill.captures(lines[j]) else { break };
            vals.push((c[1].parse().unwrap_or(0), c[2].to_string()));
            j += 1;
        }
        // the fills only mean anything if the line after them is the list being handed over
        if !vals.is_empty() {
            if let Some(c) = lines.get(j).and_then(|l| take.captures(l)) {
                let n = vals.iter().map(|(k, _)| *k + 1).max().unwrap_or(0);
                let mut items = vec!["null".to_string(); n];
                for (k, v) in &vals { if *k < n { items[*k] = v.clone() } }
                out.push_str(&format!("{}{} = [{}];\n", &c[1], &c[2], items.join(", ")));
                i = j + 1;
                continue;
            }
        }
        out.push_str(lines[i]);
        out.push('\n');
        i += 1;
    }
    out
}

/// Moving a pair of coordinates by an amount.
///
/// `offset` is what the source language calls it, and it belongs to the type rather than to the
/// engine -- which has no name for it, so the ask lands on nothing and takes the frame. A pair of
/// names moved by an amount is each name moved by its part of it, which is what it always was.
/// The clock, under the name the target keeps it.
///
/// A stage that shows the time reads the real one: `new Date()`, then `.hours` and `.minutes`.
/// The target has a clock of its own, asked with calls rather than read as names -- so the same
/// question, spelled the way this side spells it. Desk's wall clock runs on it.
/// A copy of a pair of coordinates.
///
/// `clone` is the source language's, and it belongs to the type rather than to the engine, which
/// has no name for it -- so the copy comes back as nothing and whatever was going to be done with
/// it is done to nothing. Saffron city's platforms keep a route of points and copy each one as it
/// is handed over; without the copy the route filled up with nothing and the platform read a half
/// off it every frame.
///
/// A pair copied is its two names copied, which is what the source asked for.
fn a_copy_of_a_pair(code: &str) -> String {
    use regex::Regex;
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    if !code.contains(".clone()") { return code.to_string() }
    let re = RE.get_or_init(|| {
        regex::Regex::new(r"([\w.\[\]]+)\.clone\(\)").expect("regex")
    });
    // a rectangle copied is its four names copied; a pair has no width and height, and copying
    // nothing into them is nothing
    re.replace_all(code, "{ x: ${1}.x, y: ${1}.y, width: ${1}.width, height: ${1}.height }").into_owned()
}

fn the_clock_as_the_target_keeps_it(code: &str) -> String {
    use regex::Regex;
    static NEW: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    static PART: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    if !code.contains("Date") { return code.to_string() }
    let newd = NEW.get_or_init(|| {
        regex::Regex::new(r"\b(?:new\s+)?Date\s*(?:\.\s*new)?\s*\([^()]*\)").expect("regex")
    });
    let out = newd.replace_all(code, "Date.now()").into_owned();
    let part = PART.get_or_init(|| {
        regex::Regex::new(r"\.(hours|minutes|seconds|date|month|day)\b").expect("regex")
    });
    part.replace_all(&out, |c: &regex::Captures| {
        let (a, b) = c[1].split_at(1);
        format!(".get{}{}()", a.to_uppercase(), b)
    }).into_owned()
}

fn a_pair_moved_by_an_amount(code: &str) -> String {
    use regex::Regex;
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    if !code.contains(".offset(") { return code.to_string() }
    let re = RE.get_or_init(|| {
        regex::Regex::new(r"(?m)^(\s*)([\w.\[\]]+)\.offset\(((?:[^()]|\([^()]*\))*)\);\s*$").expect("regex")
    });
    re.replace_all(code, |c: &regex::Captures| {
        let (indent, recv, args) = (&c[1], &c[2], &c[3]);
        // the two halves of the amount, split at the comma that is not inside a call
        let mut depth = 0i32;
        let split = args.char_indices().find(|(_, ch)| {
            match ch { '(' => depth += 1, ')' => depth -= 1, _ => {} }
            *ch == ',' && depth == 0
        }).map(|(i, _)| i);
        let Some(i) = split else { return c[0].to_string() };
        let (dx, dy) = (args[..i].trim(), args[i + 1..].trim());
        format!("{indent}{recv}.x = {recv}.x + ({dx}); {recv}.y = {recv}.y + ({dy});")
    }).into_owned()
}

fn the_language_own_containers(code: &str) -> String {
    use regex::Regex;
    static RECT: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    static BAG: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let rect = RECT.get_or_init(|| {
        regex::Regex::new(r"\b(?:new\s+)?Rectangle\s*(?:\.\s*new)?\s*\(((?:[^()]|\([^()]*\))*)\)").expect("regex")
    });
    let out = rect.replace_all(code, |c: &regex::Captures| {
        let a = crate::stage_emit::top_level_items(&c[1]);
        let g = |i: usize| a.get(i).filter(|v| !v.is_empty()).map_or("0", |v| v.as_str());
        format!("{{ x: {}, y: {}, width: {}, height: {} }}", g(0), g(1), g(2), g(3))
    }).into_owned();
    let bag = BAG.get_or_init(|| {
        regex::Regex::new(r"\b(?:new\s+)?(?:Object|Dictionary)\s*(?:\.\s*new)?\s*\((?:[^()]|\([^()]*\))*\)").expect("regex")
    });
    bag.replace_all(&out, "{}").into_owned()
}

/// A HITBOX'S STATS HANDED OVER AS A VALUE. The source builds one field by field or reads it out of
/// its own data (lucario's aura sphere picks its hit sound by damage; wario keeps the fart's in a
/// variable) and hands the object over, so the renames and scales that convert a written-out one
/// never see it, and the engine refused the whole write for one source name. It is converted when
/// it is applied, from the same tables: the measured fields, then the plain renames, then this
/// engine's own names as they are. Anything else is left out.
pub fn hitbox_fields_helper() -> String {
    let map = crate::mappings::character_hitbox_stats();
    let cmds = crate::mappings::api_commands();
    let ratio = crate::entity_gen::FRAME_RATIO as f64;
    let mut body = String::from("function __hbFields(o) {\n\tif (o == null) { return o; }\n\tvar r = {};\n");
    let mut done: std::collections::BTreeSet<String> = Default::default();
    for f in &map.fields {
        // a fixed launch is worked out against the victim; the stated number alone is not it
        for k in f.ssf2_keys.iter().filter(|k| k.as_str() != "weightKB") {
            // a multiplier, not frames: the engine's own answer stands (as for a written-out one)
            if f.fm_field == "hitstun" { done.insert(k.clone()); continue }
            let mut v = format!("o.{k}");
            if f.isframe { v = format!("(({v}) + {}) * {}", f.plus_frames, ratio); }
            if let Some(sc) = f.scale { v = format!("({v}) * {sc}"); }
            body.push_str(&format!("\tif (o.{k} != null) {{ r.{} = {v}; }}\n", f.fm_field));
            done.insert(k.clone());
        }
    }
    for r in &cmds.replacements {
        let (Some(from), Some(to)) = (r.from.strip_suffix(':'), r.to.strip_suffix(':')) else { continue };
        if from.contains(|c: char| !c.is_ascii_alphanumeric() && c != '_') || done.contains(from) { continue }
        // the attacker's lag is the same kind of multiplier
        if from.ends_with("HitLag") { continue }
        if !map.fm_stats.iter().any(|s| s == to) { continue }
        body.push_str(&format!("\tif (o.{from} != null) {{ r.{to} = o.{from}; }}\n"));
        done.insert(from.to_string());
    }
    for r in &cmds.replacements {
        // an element flag: `burn: true` is the element
        let Some(flag) = r.from.strip_suffix(": true") else { continue };
        let Some(el) = r.to.strip_prefix("element: ") else { continue };
        body.push_str(&format!("\tif (o.{flag} == true) {{ r.element = {el}; }}\n"));
    }
    for s in &map.fm_stats {
        if done.contains(s) { continue }
        body.push_str(&format!("\tif (r.{s} == null && o.{s} != null) {{ r.{s} = o.{s}; }}\n"));
    }
    body.push_str("\treturn r;\n}\n");
    body
}

/// A HIT'S STATS READ BACK BY THE SOURCE'S NAMES (marth's counter reads the `power` and `kbConstant`
/// of the hit it caught): this engine's event carries its own stats, under its own names and in its
/// own units, so the read is answered in the source's, from the same table that converts a write
/// (the inverse of each field's scale and frame count). The name goes as a string, so no pass that
/// renames members can touch it.
pub fn hitbox_reads_helper() -> String {
    let map = crate::mappings::character_hitbox_stats();
    let cmds = crate::mappings::api_commands();
    let ratio = crate::entity_gen::FRAME_RATIO as f64;
    let mut body = String::from("function __hbRead(h, k) {\n\tif (h == null) { return null; }\n");
    let mut done: std::collections::BTreeSet<String> = Default::default();
    for f in &map.fields {
        for k in f.ssf2_keys.iter().filter(|k| k.as_str() != "weightKB") {
            if f.fm_field == "hitstun" { continue }
            let mut v = format!("h.{}", f.fm_field);
            if let Some(sc) = f.scale { v = format!("({v}) / {sc}"); }
            if f.isframe { v = format!("({v}) / {ratio} - {}", f.plus_frames); }
            // the source's own number back, not the float noise of undoing the scale: a
            // knockback of 30 reads back as 29.99999, and a whole-number field keeps 29
            if f.scale.is_some() || f.isframe { v = format!("Math.round(({v}) * 1000000) / 1000000"); }
            body.push_str(&format!("\tif (k == \"{k}\") {{ return h.{} == null ? null : {v}; }}\n", f.fm_field));
            done.insert(k.clone());
        }
    }
    for r in &cmds.replacements {
        let (Some(from), Some(to)) = (r.from.strip_suffix(':'), r.to.strip_suffix(':')) else { continue };
        if from.contains(|c: char| !c.is_ascii_alphanumeric() && c != '_') || done.contains(from) || from.ends_with("HitLag") { continue }
        if !map.fm_stats.iter().any(|s| s == to) { continue }
        body.push_str(&format!("\tif (k == \"{from}\") {{ return h.{to}; }}\n"));
        done.insert(from.to_string());
    }
    body.push_str("\treturn null;\n}\n");
    body
}

/// The source's names read off a hit's stats (`<x>.hitboxStats.power`), answered through `__hbRead`.
pub fn hitbox_stat_reads(code: &str) -> String {
    let map = crate::mappings::character_hitbox_stats();
    let cmds = crate::mappings::api_commands();
    let mut keys: std::collections::BTreeSet<String> = map.fields.iter()
        .filter(|f| f.fm_field != "hitstun").flat_map(|f| f.ssf2_keys.iter().cloned()).filter(|k| k != "weightKB").collect();
    for r in &cmds.replacements {
        if let (Some(from), Some(_)) = (r.from.strip_suffix(':'), r.to.strip_suffix(':')) {
            if from.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !from.ends_with("HitLag") { keys.insert(from.to_string()); }
        }
    }
    // a name both engines use for the same thing reads as it is
    keys.retain(|k| !map.fm_stats.iter().any(|s| s == k));
    if keys.is_empty() { return code.to_string() }
    let alt = keys.iter().map(|k| regex::escape(k)).collect::<Vec<_>>().join("|");
    let re = crate::dyn_re(&format!(r"((?:[A-Za-z_]\w*(?:\([^()]*\))?\.)*hitboxStats)\.({alt})\b(\s*[^\s=]|\s*==|\s*$)"));
    re.replace_all(code, "__hbRead(${1}, \"${2}\")${3}").into_owned()
}

/// A WRITE TO SOMETHING ANOTHER SCRIPT PUBLISHES (`hydrant.get().exports.rightDir = v`): a script
/// keeps its fields as its own variables and publishes a setter for each, so writing the published
/// name leaves the variable it reads untouched (pacman's hydrant never learned which way it faced).
/// Through the setter when there is one, else as written.
pub fn published_writes_through_setters(code: &str) -> String {
    let re = crate::re!(r"(?m)^(\s*)((?:[A-Za-z_]\w*(?:\(\))?\.)*?[A-Za-z_]\w*(?:\(\))?)\.exports\.([A-Za-z_]\w*)\s*=\s*([^=;][^;]*);\s*$");
    re.replace_all(code, |c: &regex::Captures| {
        let (ind, recv, name, v) = (&c[1], &c[2], &c[3], c[4].trim());
        // a script publishing its own is publishing, not writing into someone else
        if name.starts_with("__") || name.starts_with("set_") || recv == "self" { return c[0].to_string() }
        format!("{ind}{{ var __w = {recv}; if (__w != null && __w.exports != null) {{ if (__w.exports.set_{name} != null) {{ __w.exports.set_{name}({v}); }} else {{ __w.exports.{name} = {v}; }} }} }}")
    }).into_owned()
}

/// ...read across everything that shares the script's scope (its frames too): a frame cancels
/// by function what the script started, and the other way round.
pub fn timers_cancelled_by_function_in(code: &str, all: &str) -> String {
    let rm = crate::re!(r"((?:[A-Za-z_]\w*(?:\(\))?\.)*[A-Za-z_]\w*(?:\(\))?)\.removeTimer\(\s*([A-Za-z_][\w.]*)\s*\)");
    // the helpers themselves take a timer off by its number, and are left as they are: any line
    // that defines one of this converter's own functions
    let is_helper = |l: &str| l.trim_start().starts_with("function __");
    // ...and a removal already routed through the record helper (`__rmTimer(name)`)
    let own_rm = crate::re!(r"(^|[^\w.])__rmTimer\(\s*([A-Za-z_][\w.]*)\s*\)");
    let body: String = all.lines().filter(|l| !is_helper(l)).collect::<Vec<_>>().join("\n");
    let mut by_fn: std::collections::BTreeSet<String> = Default::default();
    let names: Vec<String> = rm.captures_iter(&body).map(|c| c[2].to_string())
        .chain(own_rm.captures_iter(&body).map(|c| c[2].to_string())).collect();
    for f in &names {
        let f = f.as_str();
        // a kept number (`x_timer.get()` reads as a call, and is left out by the pattern) or a
        // variable named for one is a number already -- unless the script defines a function by
        // that name, which is what it is then, whatever it is called (`timerDone`)
        let is_fn = crate::dyn_re(&format!(r"\bfunction\s+{}\s*\(", regex::escape(f))).is_match(&body);
        if !is_fn && (f.ends_with("_timer") || f.ends_with("Timer") || f.contains("timer")) { continue }
        by_fn.insert(f.to_string());
    }
    if by_fn.is_empty() { return code.to_string() }
    let add = crate::re!(r"((?:[A-Za-z_]\w*(?:\(\))?\.)*[A-Za-z_]\w*(?:\(\))?)\.addTimer\(\s*([^,()]+),\s*([^,()]+),\s*([A-Za-z_][\w.]*)\s*([,)])");
    code.lines().map(|l| {
        if is_helper(l) { return l.to_string() }
        let l = rm.replace_all(l, |c: &regex::Captures| {
            if by_fn.contains(&c[2]) { format!("__removeTimerF({}, {})", &c[1], &c[2]) } else { c[0].to_string() }
        }).into_owned();
        let l = own_rm.replace_all(&l, |c: &regex::Captures| {
            if by_fn.contains(&c[2]) { format!("{}__removeTimerF(self, {})", &c[1], &c[2]) } else { c[0].to_string() }
        }).into_owned();
        let l = add.replace_all(&l, |c: &regex::Captures| {
            if by_fn.contains(&c[4]) { format!("__addTimerF({}, {}, {}, {}{}", &c[1], c[2].trim(), c[3].trim(), &c[4], &c[5]) } else { c[0].to_string() }
        }).into_owned();
        // ...and one the script started as its own (see source_timers_last_to_the_state_change)
        let own = crate::re!(r"(^|[^\w.])__srcTimer\(\s*([^,()]+),\s*([^,()]+),\s*([A-Za-z_][\w.]*)\s*([,)])");
        own.replace_all(&l, |c: &regex::Captures| {
            if by_fn.contains(&c[4]) { format!("{}__addTimerF(self, {}, {}, {}{}", &c[1], c[2].trim(), c[3].trim(), &c[4], &c[5]) } else { c[0].to_string() }
        }).into_owned()
    }).collect::<Vec<_>>().join("\n")
}

/// A PROJECTILE'S OWN END tells what asked to be told of it first. Its update stops once it dies
/// (and a projectile sent to die is gone at once), so the asks were never run: megaman's crash bomb
/// resets his count of bombs out when it goes, and he could throw one only once.
pub fn own_end_tells_first(code: &str) -> String {
    let re = crate::re!(r"(?m)^(\s*)(self\.kill\(\)|self\.toState\(PState\.DESTROYING\));");
    let code = re.replace_all(code, "${1}__takeProjDestroyedAsks(true); __unlistenOthers(); ${2};").into_owned();
    // taken off at once, it takes its timers with it (a dying one keeps them: its state change ends
    // the ones that end with the state)
    code.replace("__unlistenOthers(); self.kill();", "__unlistenOthers(); __endSrcTimers(); self.kill();")
}

/// ...and a PROJECTILE's are also taken off when it ends (`__listenOther`, `__unlistenOthers`): a
/// projectile's script goes with it, so a handler it left on its fighter ran into nothing the next
/// time the fighter was hit (fox's up special, black mage's warp).
pub fn listeners_on_others_persist_until_end(code: &str) -> String { listeners_on_others(code, true) }

fn listeners_on_others(code: &str, track: bool) -> String {
    // the helpers that do the tracking listen (and unlisten) the same way, and read as listeners
    // to track they called themselves forever (falco's side special took the engine down): their
    // own lines are left be
    let held: Vec<&str> = code.lines().filter(|l| {
        let t = l.trim_start();
        t.starts_with("function __listenOther(") || t.starts_with("function __unlistenOther(") || t.starts_with("function __unlistenOthers(")
    }).collect();
    if !held.is_empty() {
        let mut out = code.to_string();
        for (i, h) in held.iter().enumerate() { out = out.replacen(h, &format!("/*__LISTEN_OTHER_HELPER_{i}__*/"), 1); }
        let mut out = listeners_on_others(&out, track);
        for (i, h) in held.iter().enumerate() { out = out.replacen(&format!("/*__LISTEN_OTHER_HELPER_{i}__*/"), h, 1); }
        return out;
    }
    let re = crate::re!(r"((?:[A-Za-z_]\w*(?:\(\))?\.)*[A-Za-z_]\w*(?:\(\))?)\.addEventListener\(\s*([\w.]+)\s*,\s*([A-Za-z_][\w.]*|function\s*\([^()]*\)\s*\{[^{}]*\})\s*(,\s*\{[^{}]*\})?\s*\)");
    let out = re.replace_all(code, |c: &regex::Captures| {
        let recv = &c[1];
        if recv == "match" { return c[0].to_string() }
        // the object's OWN listener lasts until its code takes it off, as the source's does; this
        // engine drops one that does not ask to persist at the object's next animation change
        // (megaman's buster listened for its hit in its first animation and hit four times from
        // its second, flying on)
        if recv.split('.').all(|r| r == "self") {
            if c.get(4).is_some() || !track { return c[0].to_string() }
            return format!("self.addEventListener({}, {}, {{ persistent: true }})", &c[2], &c[3]);
        }
        // one that already states its options keeps them, and is still one to take off at the end
        if c.get(4).is_some() && !track { return c[0].to_string() }
        if track { format!("__listenOther({recv}, {}, {})", &c[2], &c[3]) }
        else { format!("{recv}.addEventListener({}, {}, {{ persistent: true }})", &c[2], &c[3]) }
    }).into_owned();
    if !track { return out }
    // and taken off through the same record (the handler went on wrapped, so the bare function
    // is not what the other object holds)
    let off = crate::re!(r"((?:[A-Za-z_]\w*(?:\(\))?\.)*[A-Za-z_]\w*(?:\(\))?)\.removeEventListener\(\s*([\w.]+)\s*,\s*([A-Za-z_][\w.]*)\s*\)");
    off.replace_all(&out, |c: &regex::Captures| {
        let recv = &c[1];
        if recv.split('.').all(|r| r == "self") || recv == "match" { return c[0].to_string() }
        format!("__unlistenOther({recv}, {}, {})", &c[2], &c[3])
    }).into_owned()
}

/// `updateHitboxStats(i, value)` with a value that is not written out: converted as it is applied.
pub fn hitbox_values_converted(code: &str) -> String {
    let re = crate::re!(r"\.updateHitboxStats\(\s*([^,()]+(?:\([^()]*\))?)\s*,\s*");
    let mut out = String::with_capacity(code.len());
    let mut pos = 0;
    for c in re.captures_iter(code) {
        let m = c.get(0).unwrap();
        let arg_start = m.end();
        if code[arg_start..].starts_with('{') || code[arg_start..].starts_with("__hbFields(") { continue }
        // the argument, to its close
        let b = code.as_bytes();
        let (mut depth, mut i) = (0i32, arg_start);
        while i < b.len() {
            match b[i] { b'(' | b'[' | b'{' => depth += 1, b')' | b']' | b'}' => { if depth == 0 { break } depth -= 1 } b',' if depth == 0 => break, _ => {} }
            i += 1;
        }
        if i >= b.len() || b[i] != b')' { continue }
        let arg = code[arg_start..i].trim();
        if arg.is_empty() || arg == "null" { continue }
        out.push_str(&code[pos..arg_start]);
        out.push_str(&format!("__hbFields({arg})"));
        pos = i;
    }
    out.push_str(&code[pos..]);
    out
}

/// A handler the source hands over by naming one of the ENGINE's methods on itself
/// (`self.resetMovement` as a landing handler, `self.refreshAttackID` to a timer). Only a call to such a method is translated, so the
/// bare name read back nothing here, and the landing threw on a null handler every time it fired
/// (captain falcon); handed to a timer, the engine itself fell over the first time it fired
/// (link's side special). The handler becomes a function that makes the translated call, one per
/// name (`__apiFn`), so taking it off again takes off the same function it put on.
/// A name whose call translates to itself, or only to a strike, is left alone.
pub fn functions_defined_in(code: &str) -> std::collections::HashSet<String> {
    // and its variables: a frame handing one over hands whatever the script keeps in it
    crate::re!(r"\b(?:function|var)\s+([A-Za-z_]\w*)").captures_iter(code).map(|c| c[1].to_string()).collect()
}

///
/// Run on the finished text, where the script's own functions are known (`own`): a name the
/// script defines is its own method even when a call of it would translate to the engine's
/// (captain falcon's `checkGround`).
/// A timer the script takes off by what `__srcTimer` handed it (a record, see the helper) goes
/// through the record; a raw engine id still reaches the engine. Run on the finished text, past
/// every pass that writes a removal; the helpers' own lines are theirs.
pub fn timer_removals_through_records(code: &str) -> String {
    if !code.contains("self.removeTimer(") { return code.to_string() }
    code.lines().map(|l| {
        if l.trim_start().starts_with("function __") { l.to_string() } else { l.replace("self.removeTimer(", "__rmTimer(") }
    }).collect::<Vec<_>>().join("\n") + if code.ends_with('\n') { "\n" } else { "" }
}

pub fn handed_api_methods(code: &str, own: &std::collections::HashSet<String>) -> String {
    let re = crate::re!(r"((?:__onGroundTouch|__offGroundTouch|removeTimer)\(\s*|(?:add|remove)EventListener\(\s*[^,()]+,\s*|(?:addTimer|__srcTimer)\(\s*[^,()]+,\s*[^,()]+,\s*)(self\.)?([A-Za-z_]\w*)(\s*[,)])");
    re.replace_all(code, |c: &regex::Captures| {
        let name = &c[3];
        if own.contains(name) { return c[0].to_string(); }
        // named bare, it is a method the class inherits from the source engine's base -- unless
        // the script has something of that name itself (a local, a parameter, a slot)
        if c.get(2).is_none() {
            let declared = crate::dyn_re(&format!(r"\b(?:var|function)\s+{0}\b|\bfunction\s*\w*\s*\([^)]*\b{0}\b[^)]*\)|\b{0}\s*=[^=]", regex::escape(name)));
            if declared.is_match(code) || name == "null" || name == "true" || name == "false" { return c[0].to_string(); }
        }
        let call = format!("self.{name}();");
        // a question asked of the translation, not a line of the port: it leaves no count behind
        let kept = conversion_log().lock().unwrap().clone();
        let t = translate_ssf2_to_fm(&call);
        *conversion_log().lock().unwrap() = kept;
        // one this engine has too (`self.flip`) is still wrapped when named through self: handed
        // over bare, the engine calls its own method with the event it passes every listener.
        // named bare, a call that translates to itself says nothing about what the name is
        if t.trim() == call && c.get(2).is_none() { return c[0].to_string(); }
        // a call struck as unknown is no answer: the script's own methods read that way too,
        // until they are pointed at the script's own functions
        let body: String = t.lines().filter(|l| !l.trim_start().starts_with("//"))
            .map(|l| l.trim()).collect::<Vec<_>>().join(" ");
        if body.is_empty() { return c[0].to_string(); }
        format!("{}__apiFn(\"{name}\", function(?e) {{ {} }}){}", &c[1], body, &c[4])
    }).into_owned()
}

pub fn translate_ssf2_to_fm(code: &str) -> String {
    let code = &the_character_behind_self(code);
    let code = &label_jumps_on_others(code);
    let code = &colour_settings_the_engine_can_show(code);
    let code = &the_stage_from_outside(code);
    // asked of an object's kind, a method by a worked-out name, a hitbox stat read back: answered
    // in the script (see the helpers) before the passes below could strike them as unanswerable
    let code = &rewrite_stat_accessors(&rewrite_dynamic_calls(&rewrite_get_type(&rewrite_attack_box_stat_reads(code))));
    let mut result = a_copy_of_a_pair(&the_clock_as_the_target_keeps_it(&a_pair_moved_by_an_amount(&the_language_own_containers(
        &a_point_is_two_names(&a_list_by_any_name(&remove_readiness_guards(code)))))));
    // a speed the script states, before any spawn line built here adds one in engine units
    result = speeds_cross_the_boundary(&result);

    // A backdrop's timeline position becomes the stage's own. This runs FIRST, before the pass
    // that comments out calls with no equivalent: `currentFrame` genuinely has none in general,
    // and the whole statement is struck out when it survives to that point -- so a condition that
    // decides the time of day, or whether the moon has fallen, silently stops existing. Rewriting
    // it here means there is no `currentFrame` left for that pass to find.
    // An animation a call NAMES has to be the name the entity carries, or the engine is asked
    // for something that does not exist and the character stops being drawn.
    // who `self` is decides what a request for the stage means, so answer that first
    result = rewrite_stage_self(&result);
    // a jump to a label first, so the call is one the engine has before its argument is resolved
    result = rewrite_play_label_calls(&result);
    result = rewrite_anim_name_args(&result);
    // before anything strikes it: a cast is nothing here, and struck it takes the
    // assignment it was part of with it
    // before the tests are wrapped, while a conjunct still reads the same in both places
    result = restore_lost_conjuncts(&result);
    result = unwrap_type_casts(&result);
    result = rewrite_backdrop_frame_reads(&result);
    // Asking what a fighter is doing, before the pass that strikes out what it cannot place.
    result = resolve_animation_label_reads(&result);
    result = rewrite_quality_settings(&result);
    result = rewrite_stage_bounds(&result);

    // 30→60fps frame doubling on SSF2-named frame fields (hitStun, hitLag,
    // refreshRate, etc.). Runs BEFORE call_splits and the rename pass so
    // SSF2 field names still match `frame_params` entries. Previously this
    // ran only in entity_gen; moved here so Script.hx + ext-methods get it
    // too (otherwise frame fields inside updateHitboxStats calls in
    // Script.hx kept SSF2 30fps values).
    result = an_effect_taken_off_the_list_ends_itself(&result);
    result = inline_named_stat_objects(&result);
    result = double_frame_counts(&result);

    // Fan out SSF2 umbrella calls into one or more FM calls per the
    // `call_splits` table. Runs BEFORE the rename pass so the rename
    // pass can finish field-name conversions on the emitted FM calls.
    result = apply_call_splits(&result);

    // Literal SSF2 → Fraymakers find/replace pairs. Order matters.
    for r in &crate::mappings::api_commands().replacements {
        result = result.replace(&r.from, &r.to);
    }
    result = others_ended_through_their_own_end(&result);
    result = slices_of_strings_too(&result);

    // A knockback the script sets at runtime converts like one in a stats table. Runs here,
    // after the renames above, so the fields carry their Fraymakers names.
    result = scale_knockback_writes(&result);
    result = resolve_fixed_knockback_writes(&result);
    result = drop_hitbox_fields_without_counterpart(&result);
    result = drop_character_stats_without_counterpart(&result);
    result = rename_projectile_flag_writes(&result);
    result = projectile_lifetime_writes(&result);
    result = drop_projectile_stats_without_counterpart(&result);
    result = answer_unkept_projectile_stat_asks(&result);
    result = renumber_hitbox_ids(&result);
    result = resolve_character_stat_reads(&result);
    // a transition handed over as a callback has to be a function, not a member
    result = wrap_transition_callbacks(&result);
    result = normalize_hitstun_writes(&result);

    // Calls rewritten by their parsed arguments (see `CallRewrite`)
    result = apply_call_rewrites(&result);

    // Regex replacements — arg-dropping, arg-aware dispatch, etc.
    let stage_content = STAGE_CONTENT.with(|c| c.get());
    for re in regex_replacement_cache() {
        // a stage's spawn by class makes one of the stage's own objects: left for the stage
        // emitter, which gives it its content and its maker (bomb factory's blocks were fired as
        // projectiles owned by the stage, which the engine refuses)
        if stage_content && (re.regex.as_str().contains("spawnEnemy") || re.regex.as_str().contains("spawnItem")) { continue }
        let next = re.regex.replace_all(&result, re.replacement.as_str());
        if matches!(&next, std::borrow::Cow::Owned(_)) {
            result = next.into_owned();
        }
    }

    // Translate the SSF2 "infinite timer" sentinel. Runs AFTER the literal
    // replacements above have renamed createTimer → addTimer, so a single
    // pass covers both spellings.
    result = fix_timer_infinite_repeat(result);

    // Context-aware self.attachEffect(...) → match.createVfx(...) rewrite.
    // Needs the per-character effect→primary-animation map (set up by
    // haxe_gen::generate via `with_effect_animations`); a static regex
    // can't pick the right animation name for effects that split into
    // multiple FrameLabel-derived animations.
    result = rewrite_attach_effect_calls(&result);
    // and the ones whose effect is named only once it runs
    result = rewrite_attach_effect_by_runtime_name(&result);
    // a state number of the source's, a shift in its pixels, and the hitbox writes a read-back
    // has to know about
    result = rewrite_numeric_state_compares(&result);
    result = scale_move_args(&result);
    result = note_hitbox_writes(&result);

    // SSF2 playSound("id") → Fraymakers AudioClip.play(...). Must run before
    // comment_out_unknown_calls so the rewritten calls aren't flagged.
    result = rewrite_play_sound_calls(&result);
    result = a_stop_with_no_name_stops_the_last_sound(&result);

    // SSF2 self.fireProjectile("name", x, y) → match.createProjectile(...) + position.
    // Must run before comment_out_unknown_calls so the spawn isn't commented out.
    result = rewrite_fire_projectile_calls(&result);
    // right after the spawn rewrite, while what it made is still named on the line above
    result = resolve_current_projectile(&result);

    // SSF2 RECV.addEffectToList(effect, owner) → keep just the effect-spawning first
    // arg (createVfx already spawns it). Balanced-scan so the closing paren and owner
    // arg are removed cleanly instead of orphaned.
    result = rewrite_add_effect_to_list(&result);

    result = strip_last_frame_end_animation(&result);
    // `if (… isForcedCrash …) { … }` blocks are obsolete in FM — delete them
    // (before comment_out_unknown_calls would dead-comment them).
    result = remove_forced_crash_guards(&result);
    // SSF2 hitTestGround(x, y) point test → FM line-segment hitTestStructuresWithLineSegment.
    result = rewrite_hit_test_ground(&result);
    // anything can be asked to say what it is, but only through the standard library
    result = say_what_a_value_is(&result);
    // ...and not where it is being subtracted, multiplied or divided
    result = arithmetic_is_never_text(&result);
    // ...and a subtraction inside a sentence had brackets round it before the decompiler
    result = subtraction_inside_a_sentence(&result);
    // a reach one step up the display list is a reach at the running object itself
    result = resolve_display_parent(&result);
    // work an object does, put on the match by the renames above, goes back to the object
    result = send_object_calls_to_the_object(&result);
    // AS3 self.Number(x) cast → Std.parseFloat(Std.string(x)).
    result = rewrite_number_cast(&result);
    // a match option this engine lacks reads as off: a bit-field test gets the object whose
    // string is "0", and any other use (rest area subtracts one from its difficulty) the number
    result = option_off_as_number(&result);
    // SSF2Utils.decel(v, amount) → inline Math min/max (FM has no Math.decel).
    result = rewrite_decel_calls(&result);
    // RECV.angleControl(speed, angle) → the two velocity writes it makes, before the pass below
    // strikes it as unmapped.
    result = rewrite_angle_control_calls(&result);
    // RECV.getCurrentAttackFrame() → the fighter's animation, and the attack names compared with it
    // → the animation names this engine uses; before the pass below strikes the call as unmapped.
    result = rewrite_current_attack_frame(&result);
    // a projectile-collision listener → a per-frame body-overlap check off the other entity's ECB,
    // and isReversed() → false, before the pass below strikes them; together these catch a returning
    // projectile at its thrower.
    result = rewrite_proj_collide_listeners(&result);
    result = rewrite_is_reversed(&result);
    // Any SSF2Event.* / SSF2Utils.* still present has no Fraymakers equivalent
    // in the tables above; neutralize it so the line can't reference an
    // undefined symbol and crash the script at runtime.
    // after the renames, so a uid is spelled the way this engine spells it
    result = rewrite_current_projectile(&result);
    // a global whose name is only known as it runs has no slot to be, so it goes to the
    // store every entity can reach
    result = rewrite_runtime_named_globals(&result);
    result = unwrap_calls_with_no_answer(&result);
    result = rewrite_vfx_sent_behind(&result);
    result = rewrite_grab_calls(&result);
    result = neutralize_unmapped_ssf2(&result);
    // `.currentLabel` / `.currentFrame` are Flash MovieClip TIMELINE properties with
    // no Fraymakers equivalent. They survive because getMC()→getViewRootContainer()
    // maps to a real FM call, so the line stays live and reads a property the FM
    // container doesn't have (null access at runtime). Neutralize block-aware.
    result = neutralize_flash_timeline_props(&result);
    // Comment out statements the decompiler couldn't fully reconstruct, where the
    // leftover `/* ? */` / `/* class */` marker would make INVALID Haxe (a `.x`
    // receiver after a comment, or `= ;` empty RHS) that the engine rejects —
    // killing the whole script. Commenting keeps the gap visible and the rest of
    // the script alive. See docs/STATUS.md "frame-script translation".
    result = neutralize_decompiler_gaps(&result);
    result = comment_out_unknown_calls(&result);
    // A condition the target cannot answer is not a reason to drop what the source does when the
    // answer is no.
    // after the passes that strike what a test was reading, since that is what leaves the
    // `null` on one side of it
    result = resolve_tests_with_a_lost_side(&result);
    result = resolve_unavailable_conditions(&result);
    // the camera here answers a CameraApi, which has no view container to reach into: the source
    // read the camera's own container to work out screen bounds, and that reach has no counterpart.
    // strike the statement so the local it fills is left null and the guard below wraps its use.
    result = strike_camera_container_reach(&result);
    // `self.camera` is the source's cached reference to the engine camera; here the camera is always
    // match.getCamera(), so a fighter has no `camera` field to hold it (and writing one throws,
    // killing the animation). Drop the redundant cache write and resolve every other `self.camera`
    // to match.getCamera().
    result = resolve_camera_reference(&result);
    // a local whose one value-giving assignment was struck is left holding null, and the
    // statements that reach through it (`_v3.exports.x.get()`) throw every frame. guard them so
    // they only run when the value is there, which it never is once its source was struck.
    result = guard_reads_of_a_struck_definition(&result);
    // Asking a fighter what it is doing is asking the FIGHTER, not the container it draws
    // itself in. SSF2 reads the label off the clip, so the read arrives here wearing the
    // clip accessor the rename pass has just turned into a real call; the question belongs
    // one level up.
    result = result.replace(".getViewRootContainer().getAnimation()", ".getAnimation()");
    // a WRITE of a thing's own position reads its position through a getter here (parent.y and
    // getViewRootContainer().y both resolve to getX()/getY()), and a getter on the left of `=` does
    // not parse. this runs LAST because the passes that produce those getters (resolve_display_parent
    // among them) run after the find/replace table where the getter-to-setter rule lives, so it
    // would otherwise never see them. a getter assigned to is the setter.
    colour_settings_on_the_object(&source_timers_last_to_the_state_change(&stage_rects_in_source_units(&hitbox_stat_reads(&frame_jumps_on_others(&fix_getter_lhs_assignments(&result))))))
}

/// A TIMER THE SOURCE STARTS lasts until its object changes state (or it is taken off, or asked to
/// persist), and runs on while the clip moves from label to label. This engine drops one at every
/// animation change, and a clip's labels are separate animations here, so a timer died at its first
/// label boundary (a boomerang stopped slowing the moment it went from `start` to `loop`). Started
/// through `__srcTimer`: kept by the engine, and taken off by the script at the state change.
fn source_timers_last_to_the_state_change(code: &str) -> String {
    if IN_STAGE.with(|c| c.get()) || STAGE_CONTENT.with(|c| c.get()) || !code.contains("self.addTimer(") {
        return code.to_string()
    }
    crate::re!(r"\bself\.addTimer\(").replace_all(code, "__srcTimer(").into_owned()
}

/// A colour setting goes on the object, not on the view the source reached it through (the
/// source's clip reads as the object's view container by the time the call is translated).
fn colour_settings_on_the_object(code: &str) -> String {
    if !code.contains("__colorFilters(") { return code.to_string() }
    crate::re!(r"__colorFilters\(([^,()]*(?:\(\))?[^,()]*?)\.getViewRootContainer\(\)\s*,").replace_all(code, "__colorFilters($1,").into_owned()
}

/// A jump to a frame NUMBER of another object's animation (sonic's spring puts the foe it bounces
/// on the third frame of its landing): the number is the source's frame, and this engine plays two
/// for each of those. The script's own jumps go through its frame table instead.
fn frame_jumps_on_others(code: &str) -> String {
    let re = crate::re!(r"(^|[^\w.])((?:[a-z_]\w*)(?:\.get\(\))?(?:\.[a-z_]\w*(?:\.get\(\))?)*)\.playAnimation\(\s*(\d+)\s*\)");
    let ratio = crate::entity_gen::FRAME_RATIO as i64;
    re.replace_all(code, |c: &regex::Captures| {
        let recv = &c[2];
        if recv.split('.').all(|r| r == "self") { return c[0].to_string() }
        let n: i64 = c[3].parse().unwrap_or(1);
        format!("{}{recv}.playFrame({})", &c[1], (n - 1) * ratio + 1)
    }).into_owned()
}

/// `x.getX() = v;` / `x.getY() = v;` -> `x.setX(v);` / `x.setY(v);`. A position WRITE that was
/// lowered like a read leaves a getter on the left of `=`, which is not valid; the setter is what
/// the source meant. Only the getter immediately before the `=` is the target (a getter on the
/// right, `self.getY() = self.getY() + 5`, is a read and stays).
fn fix_getter_lhs_assignments(code: &str) -> String {
    use std::sync::LazyLock;
    static RE: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"([A-Za-z_][\w.()\[\]]*)\.get([XY])\(\)\s*=\s*([^=;][^;]*);").unwrap()
    });
    RE.replace_all(code, "$1.set$2($3);").into_owned()
}

/// Keep the branch that RUNS when a condition asks about something Fraymakers does not have.
///
/// SSF2 branches on state the engine has no counterpart for -- whether the character is metal, for
/// one -- and the call is struck out, which takes the whole `if` with it and the ELSE branch too.
/// mario landed in silence because of it: the metal landing sound and the ordinary one were both
/// inside the same dead block, and the ordinary one is the one that always plays here.
///
/// The engine never grants what it does not have, so the condition is constantly false and the
/// else branch is what the port should run (constantly true, and the then branch, when negated).
/// Only a condition that is ENTIRELY the unavailable call is resolved -- anything mixed with a
/// term that still means something is left alone rather than guessed at.
fn resolve_unavailable_conditions(code: &str) -> String {
    let lines: Vec<&str> = code.lines().collect();
    let dead = |l: &str| l.trim_start().starts_with("// [SSF2-dead]");
    let revive = |l: &str| -> String {
        let indent = &l[..l.len() - l.trim_start().len()];
        let body = l.trim_start().trim_start_matches("// [SSF2-dead]").trim_start();
        format!("{indent}{body}")
    };
    // net brace delta of a marked line, reading the code the marker hides
    let net = |l: &str| -> i32 {
        let body = l.trim_start().trim_start_matches("// [SSF2-dead]");
        let body = match body.find("] ") { Some(i) if body.starts_with(" [SSF2-only:") => &body[i + 2..], _ => body };
        body.chars().map(|c| match c { '{' => 1, '}' => -1, _ => 0 }).sum()
    };
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut i = 0usize;
    while i < lines.len() {
        let line = lines[i];
        let t = line.trim_start();
        // the opener: a struck call standing as the whole condition of an `if`
        let cond = t.strip_prefix("// [SSF2-only: ").and_then(|r| r.split_once("] "))
            .map(|(_, rest)| rest)
            .filter(|rest| rest.starts_with("if (") && rest.trim_end().ends_with('{'));
        let Some(cond) = cond else { out.push(line.to_string()); i += 1; continue };
        // the condition must be ONE call and nothing else: no && / || / comparison mixed in
        let inner = &cond["if (".len()..cond.trim_end().len() - 1];
        let inner = inner.trim().trim_end_matches('{').trim().trim_end_matches(')').trim();
        let negated = inner.starts_with('!');
        let bare = inner.trim_start_matches('!').trim();
        let mixed = ["&&", "||", "==", "!=", "<", ">"].iter().any(|op| bare.contains(op));
        if mixed {
            out.push(line.to_string()); i += 1; continue;
        }
        // walk the block: then-branch, an optional `} else {`, else-branch, closing `}`
        let mut depth = net(line);
        let (mut else_at, mut end_at) = (None, None);
        for (j, l) in lines.iter().enumerate().skip(i + 1) {
            let before = depth;
            depth += net(l);
            if before == 1 && depth == 1 && l.contains("else") && l.trim_start().contains('}') {
                else_at = Some(j);
            }
            if depth <= 0 { end_at = Some(j); break; }
        }
        let (Some(end), true) = (end_at, lines[i + 1..end_at.unwrap_or(i + 1)].iter().all(|l| dead(l)))
        else { out.push(line.to_string()); i += 1; continue };
        // the branch that runs: the else when the answer is always no, the then when negated
        let keep: Vec<usize> = match (negated, else_at) {
            (false, Some(e)) => (e + 1..end).collect(),
            (true, Some(e)) => (i + 1..e).collect(),
            (true, None) => (i + 1..end).collect(),
            (false, None) => Vec::new(),
        };
        // the call's answer is always no: a negated test of it is always yes
        let tag = if negated { "always true here" } else { "always false here" };
        out.push(format!("{}// [{tag}] {}",
            &line[..line.len() - line.trim_start().len()], cond.trim_end_matches('{').trim()));
        for k in keep { out.push(revive(lines[k])); }
        i = end + 1;
    }
    let mut joined = out.join("\n");
    if code.ends_with('\n') { joined.push('\n'); }
    joined
}

/// Markers a decompiler-unreconstructable expression renders as.
const GAP_MARKERS: &[&str] = &["/* ? */", "/* class */"];

/// True when `line` uses a gap marker in a position that is INVALID Haxe:
///   - as a receiver:        `/* ? */.foo`  →  `.foo` (parse error)
///   - as a whole RHS value: `x = /* ? */;` →  `x = ;` (parse error)
/// `f(/* ? */)` (empty arg) and `return /* ? */;` (→ `return ;`) stay valid, so
/// they are NOT matched.
/// Remove `/* … */` spans and a trailing `//` comment, returning the trimmed
/// remaining code. Used to test what the parser actually sees on a line.
fn strip_comments(line: &str) -> String {
    let b = line.as_bytes();
    let mut s = String::new();
    let mut i = 0;
    while i < b.len() {
        if i + 1 < b.len() && b[i] == b'/' && b[i + 1] == b'*' {
            i += 2;
            while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') { i += 1; }
            i += 2;
            continue;
        }
        if i + 1 < b.len() && b[i] == b'/' && b[i + 1] == b'/' { break; }
        s.push(b[i] as char);
        i += 1;
    }
    s.trim().to_string()
}

fn line_has_invalid_gap(line: &str) -> bool {
    let stripped = strip_comments(line);
    // A line whose real code starts with `.` is a dangling method-chain
    // continuation — the decompiler renders chains inline, so this only happens
    // when the chain's base was an unrecoverable expr on the previous line
    // (`/* ? */` / `/* class */`). `.foo()` at statement start is a parse error.
    if stripped.starts_with('.') {
        return true;
    }
    // `.{` (a block-expression attached to a `.` access) is never valid Haxe —
    // the projectile/inline-block rewrite can emit `getOwner().{ var _p = … }`.
    // The whole statement is unparseable, so comment it out.
    if stripped.contains(".{") {
        return true;
    }
    for m in GAP_MARKERS {
        let mut from = 0;
        while let Some(rel) = line[from..].find(m) {
            let start = from + rel;
            let after = start + m.len();
            // receiver: marker directly followed by `.`
            let next = line[after..].trim_start();
            if next.starts_with('.') {
                return true;
            }
            // empty RHS: marker preceded by `=` (not `==`/`!=`/`<=`…) and the
            // value ends right after (`;`/`)`/`,`/end).
            let before = line[..start].trim_end();
            let assign = before.ends_with('=')
                && !before.ends_with("==") && !before.ends_with("!=")
                && !before.ends_with("<=") && !before.ends_with(">=");
            if assign && (next.is_empty() || next.starts_with([';', ')', ','])) {
                return true;
            }
            // an OPERAND of an operator: `x = /* ? */ >= (…)`. Nothing can be
            // compared against a comment, and the statement will not parse.
            const OPS: &[&str] = &[
                ">=", "<=", "==", "!=", "&&", "||", ">>", "<<",
                ">", "<", "+", "-", "*", "/", "%", "&", "|", "^",
            ];
            if OPS.iter().any(|o| next.starts_with(o)) {
                return true;
            }
            if OPS.iter().any(|o| before.ends_with(o)) && !assign {
                return true;
            }
            from = after;
        }
    }
    false
}

/// Comment out lines whose decompiler gap marker is in an invalid position
/// (`line_has_invalid_gap`), block-aware so an opener's body/brace go with it.
pub fn neutralize_decompiler_gaps(code: &str) -> String {
    // Cheap skip when there's nothing to do: no gap markers, no `.{`
    // block-as-property, and no line that could begin with a dangling `.`.
    // A CALL WITH NO NAME. The decompiler could not recover which method was being called and
    // wrote the dot and the arguments with nothing between them. That does not parse, and a
    // script that does not parse takes every variable it declares with it -- kirby lost all of
    // them to one line.
    let code = &{
        static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        let re = RE.get_or_init(|| regex::Regex::new(r"[A-Za-z_0-9\)]\.\(").unwrap());
        code.lines().map(|line| {
            let live = line.split("//").next().unwrap_or(line);
            if re.is_match(live) && !line.trim_start().starts_with("//") {
                let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
                format!("{indent}// [gap: the decompiler recovered no name for this call] {}", line.trim())
            } else {
                line.to_string()
            }
        }).collect::<Vec<_>>().join("\n")
    };
    if !GAP_MARKERS.iter().any(|m| code.contains(m))
        && !code.contains(".{")
        && !code.lines().any(|l| l.trim_start().starts_with('.'))
    {
        return code.to_string();
    }
    fn net_braces(line: &str) -> i32 {
        let live = line.split("//").next().unwrap_or(line);
        live.matches('{').count() as i32 - live.matches('}').count() as i32
    }
    let lines: Vec<&str> = code.lines().collect();
    let mut dead = vec![false; lines.len()];
    let mut i = 0;
    while i < lines.len() {
        let trimmed = lines[i].trim_start();
        if !trimmed.starts_with("//") && line_has_invalid_gap(lines[i]) {
            dead[i] = true;
            let mut depth = net_braces(lines[i]);
            let mut j = i + 1;
            while depth > 0 && j < lines.len() {
                depth += net_braces(lines[j]);
                dead[j] = true;
                j += 1;
            }
            i = j.max(i + 1);
            continue;
        }
        i += 1;
    }
    let trailing_nl = code.ends_with('\n');
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if dead[idx] && !trimmed.starts_with("//") {
            let indent = &line[..line.len() - trimmed.len()];
            out.push(format!("{}// [gap: decompiler couldn't recover] {}", indent, trimmed));
        } else {
            out.push(line.to_string());
        }
    }
    let mut joined = out.join("\n");
    if trailing_nl { joined.push('\n'); }
    joined
}

/// SSF2 namespaces with no Fraymakers symbol that, if they survive translation,
/// crash the script (`Unknown variable: SSF2Event` / `SSF2Utils`). Anything
/// mapped by the tables above is already renamed by the time this runs.
const UNMAPPED_SSF2_PREFIXES: &[&str] = &["SSF2Event.", "SSF2Utils."];

fn line_has_unmapped_ssf2(line: &str) -> bool {
    UNMAPPED_SSF2_PREFIXES.iter().any(|p| line.contains(p))
}

/// Comment out lines that still reference a bare unmapped `SSF2*.` symbol after
/// the rename passes. Those have no Fraymakers equivalent, so the engine would
/// hit `Unknown variable` and drop the whole script; commenting the line (or the
/// whole block, for inline-closure handlers) keeps the rest of the handler alive.
pub fn neutralize_unmapped_ssf2(code: &str) -> String {
    if !UNMAPPED_SSF2_PREFIXES.iter().any(|p| code.contains(p)) {
        return code.to_string();
    }
    // Net `{` minus `}` on a line, ignoring an inline `//` comment tail (string
    // contents are not scanned — event-listener lines don't carry `{`-in-strings).
    fn net_braces(line: &str) -> i32 {
        let live = line.split("//").next().unwrap_or(line);
        live.matches('{').count() as i32 - live.matches('}').count() as i32
    }

    let lines: Vec<&str> = code.lines().collect();
    let mut dead = vec![false; lines.len()];
    let mut i = 0;
    while i < lines.len() {
        let trimmed = lines[i].trim_start();
        if line_has_unmapped_ssf2(lines[i]) && !trimmed.starts_with("//") {
            dead[i] = true;
            // If the listener line opens a block (e.g. an inline `function() {`
            // handler), comment through the matching close so the orphaned body
            // and brace don't leak as live code.
            let mut depth = net_braces(lines[i]);
            let mut j = i + 1;
            while depth > 0 && j < lines.len() {
                depth += net_braces(lines[j]);
                dead[j] = true;
                j += 1;
            }
            i = j.max(i + 1);
            continue;
        }
        i += 1;
    }

    let trailing_nl = code.ends_with('\n');
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if dead[idx] && !trimmed.starts_with("//") {
            let indent = &line[..line.len() - trimmed.len()];
            // Tag the opener line with the symbol name; continuation lines just dead.
            let prefix = UNMAPPED_SSF2_PREFIXES.iter().find(|p| line.contains(**p));
            if let Some(p) = prefix {
                let sym = line.split(*p).nth(1)
                    .map(|r| r.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect::<String>())
                    .unwrap_or_default();
                out.push(format!("{}// [SSF2-only: {}] {}", indent, sym, trimmed));
            } else {
                out.push(format!("{}// [SSF2-dead] {}", indent, trimmed));
            }
        } else {
            out.push(line.to_string());
        }
    }
    let mut joined = out.join("\n");
    if trailing_nl { joined.push('\n'); }
    joined
}

/// Flash MovieClip timeline tokens: `currentLabel` / `currentFrame`, whether read
/// as a property (`getViewRootContainer().currentLabel`) or as the bare frame-script
/// playhead identifier (`curFrame.get() == currentFrame`). Neither has a Fraymakers
/// equivalent, so a live use is a runtime null-access / Unknown-variable. Comment the
/// statement (and, if it opens a block like `if (...currentLabel...) {`, the whole
/// block) so the script stays alive. balance_commented_blocks fixes any orphaned close.
/// Properties the source reads off something this engine does not keep: the clip's own timeline
/// position, and the match settings SSF2 keeps its special-mode flags in. A read of one is a read
/// of nothing, and the statement it feeds goes wrong quietly rather than saying so.
const FLASH_TIMELINE_PROPS: &[&str] = &["currentLabel", "currentFrame"];

/// True if `needle` appears in `hay` as a whole identifier (not a substring of a
/// longer name like `getCurrentFrame` or `currentFrameRate`).
fn contains_word(hay: &str, needle: &str) -> bool {
    let b = hay.as_bytes();
    let n = needle.as_bytes();
    let is_id = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut from = 0;
    while let Some(rel) = hay[from..].find(needle) {
        let s = from + rel;
        let before_ok = s == 0 || !is_id(b[s - 1]);
        let e = s + n.len();
        let after_ok = e >= b.len() || !is_id(b[e]);
        if before_ok && after_ok { return true; }
        from = s + 1;
    }
    false
}

pub fn neutralize_flash_timeline_props(code: &str) -> String {
    if !FLASH_TIMELINE_PROPS.iter().any(|p| contains_word(code, p)) {
        return code.to_string();
    }
    fn net_braces(line: &str) -> i32 {
        let live = line.split("//").next().unwrap_or(line);
        live.matches('{').count() as i32 - live.matches('}').count() as i32
    }
    let lines: Vec<&str> = code.lines().collect();
    let mut dead = vec![false; lines.len()];
    let mut i = 0;
    while i < lines.len() {
        let trimmed = lines[i].trim_start();
        let hit = FLASH_TIMELINE_PROPS.iter().any(|p| contains_word(lines[i], p));
        if hit && !trimmed.starts_with("//") {
            dead[i] = true;
            let mut depth = net_braces(lines[i]);
            let mut j = i + 1;
            while depth > 0 && j < lines.len() {
                depth += net_braces(lines[j]);
                dead[j] = true;
                j += 1;
            }
            i = j.max(i + 1);
            continue;
        }
        i += 1;
    }
    let trailing_nl = code.ends_with('\n');
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if dead[idx] && !trimmed.starts_with("//") {
            let indent = &line[..line.len() - trimmed.len()];
            let sym = FLASH_TIMELINE_PROPS.iter().find(|p| contains_word(line, p))
                .copied().unwrap_or("currentLabel");
            out.push(format!("{}// [SSF2-only: {}] {}", indent, sym, trimmed));
        } else {
            out.push(line.to_string());
        }
    }
    let mut joined = out.join("\n");
    if trailing_nl { joined.push('\n'); }
    joined
}

/// Placeholder audio content id (a silent .wav emitted per character by
/// haxe_gen) used for SSF2 sounds with no extracted asset and no GlobalSfx
/// match — paired with a visible TODO so the modder can swap in a real sound.
pub const PLACEHOLDER_SOUND_ID: &str = "_ssf2_placeholder";

thread_local! {
    /// Per-character set of extracted audio content ids (the `id` of each
    /// emitted `<name>.wav.meta`). Installed by `AvailableSoundsGuard` at the
    /// top of `haxe_gen::generate`; read by `rewrite_play_sound_calls` to
    /// decide getContent("id") vs. placeholder. Empty outside a guard scope.
    static AVAILABLE_SOUNDS: std::cell::RefCell<std::collections::BTreeSet<String>>
        = const { std::cell::RefCell::new(std::collections::BTreeSet::new()) };
}

thread_local! {
    /// Each projectile's declared launch, keyed by its SSF2 name: `(xspeed, yspeed)`, already
    /// scaled. Installed for a generation pass by `ProjectileLaunchGuard`, read where a
    /// `fireProjectile` call is rewritten. Empty outside a guard scope, and then a spawn just
    /// places the projectile, as it did before.
    static PROJECTILE_LAUNCH: std::cell::RefCell<std::collections::BTreeMap<String, (f64, f64)>>
        = const { std::cell::RefCell::new(std::collections::BTreeMap::new()) };
    /// Where each projectile leaves its summoner when the call does not say: the declaration's
    /// `xoffset`/`yoffset`, already scaled to engine pixels. Birdo fires its egg with
    /// `fireProjectile("egg")` alone, and the egg leaves its mouth because the declaration says
    /// 41 source pixels up; placed at its feet it hit the floor on its first frame.
    static PROJECTILE_OFFSET: std::cell::RefCell<std::collections::BTreeMap<String, (f64, f64)>>
        = const { std::cell::RefCell::new(std::collections::BTreeMap::new()) };
}

/// RAII guard installing the per-character projectile launch speeds.
///
/// The LAUNCH belongs to the spawn, not to the projectile: Fraymakers ignores a velocity a
/// projectile gives itself in `initialize` (measured live -- the fireball sat where it spawned),
/// and SSF2 states it at the spawn too, where the move says which way and how fast.
pub struct ProjectileLaunchGuard;

impl ProjectileLaunchGuard {
    pub fn install(speeds: std::collections::BTreeMap<String, (f64, f64)>) -> Self {
        PROJECTILE_LAUNCH.with(|c| { *c.borrow_mut() = speeds; });
        ProjectileLaunchGuard
    }
    /// The launch and the declared spawn offsets together, both keyed by the SSF2 name.
    pub fn install_with_offsets(speeds: std::collections::BTreeMap<String, (f64, f64)>,
                                offsets: std::collections::BTreeMap<String, (f64, f64)>) -> Self {
        PROJECTILE_LAUNCH.with(|c| { *c.borrow_mut() = speeds; });
        PROJECTILE_OFFSET.with(|c| { *c.borrow_mut() = offsets; });
        ProjectileLaunchGuard
    }
    /// Every declared launch and offset of a set of projectile declarations, scaled the way the
    /// engine expects: speeds through the velocity scale, offsets through the size scale.
    ///
    /// `raw_speeds`: hand the launch over in the source's own units. A stage object's script
    /// goes through a pass that converts every speed literal it finds, the spawn's included, so
    /// a launch scaled here would be scaled twice there.
    pub fn from_declarations(data: &std::collections::BTreeMap<String, crate::abc_parser::ProjectileData>, raw_speeds: bool) -> Self {
        let size = crate::mappings::character_stats().scaling.size_multiplier;
        let vs = crate::mappings::character_stats().scaling.velocity_scale();
        let speeds = data.iter().map(|(name, d)| {
            let (x, y, _) = crate::haxe_gen::synth_projectile_launch(Some(d));
            (name.clone(), if raw_speeds { (x / vs, y / vs) } else { (x, y) })
        }).collect();
        let offsets = data.iter().filter_map(|(name, d)| {
            let pick = |keys: &[&str]| keys.iter().find_map(|k| d.stats.get(*k).copied());
            let x = pick(&["xoffset", "xOffset", "x_offset"]);
            let y = pick(&["yoffset", "yOffset", "y_offset"]);
            (x.is_some() || y.is_some()).then(|| (name.clone(), (x.unwrap_or(0.0) * size, y.unwrap_or(0.0) * size)))
        }).collect();
        Self::install_with_offsets(speeds, offsets)
    }
}

impl Drop for ProjectileLaunchGuard {
    fn drop(&mut self) {
        PROJECTILE_LAUNCH.with(|c| { c.borrow_mut().clear(); });
        PROJECTILE_OFFSET.with(|c| { c.borrow_mut().clear(); });
    }
}

/// RAII guard installing the per-character extracted-sound id set for the
/// duration of a generation pass (mirrors `EffectAnimGuard`).
pub struct AvailableSoundsGuard;

impl AvailableSoundsGuard {
    pub fn install(ids: std::collections::BTreeSet<String>) -> Self {
        AVAILABLE_SOUNDS.with(|c| { *c.borrow_mut() = ids; });
        AvailableSoundsGuard
    }
}

impl Drop for AvailableSoundsGuard {
    fn drop(&mut self) {
        AVAILABLE_SOUNDS.with(|c| { c.borrow_mut().clear(); });
    }
}

/// Build the Fraymakers replacement for a single `playSound` call, given the
/// extracted first string-literal id (the sound name). Resolution order:
///   1. global_sound_map  → `AudioClip.play(GlobalSfx.<CONST>)`
///   2. extracted asset    → `AudioClip.play(self.getResource().getContent("id"))`
///   3. neither            → placeholder asset + visible `/* TODO … */`
/// The first argument of a call, as the source wrote it: the text up to the first comma that is
/// not inside brackets or a string.
fn first_argument(args: &str) -> String {
    let (mut depth, mut quote) = (0i32, false);
    for (i, c) in args.char_indices() {
        match c {
            '"' => quote = !quote,
            '(' | '[' | '{' if !quote => depth += 1,
            ')' | ']' | '}' if !quote => depth -= 1,
            ',' if !quote && depth == 0 => return args[..i].trim().to_string(),
            _ => {}
        }
    }
    args.trim().to_string()
}

/// A sound the source can stop later: the handle the engine gives back is kept under the name the
/// source started it by, and a stop by that name stops that handle.
pub fn sound_handle_helpers() -> String {
    "// a sound that can be stopped later: the handle the engine gives back, kept under the name\n\
// the source started it by\n\
var __sndNames = [];\n\
var __sndClips = [];\n\
var __sndLast = null;\n\
function __sound(n, clip) {\n\
\tvar __i = __sndNames.indexOf(n);\n\
\tif (n == null) { __i = (__sndNames.length > 0 && __sndNames[__sndNames.length - 1] == null) ? __sndNames.length - 1 : -1; }\n\
\tif (__i < 0) { __sndNames.push(n); __sndClips.push(clip); } else { __sndClips[__i] = clip; }\n\
\t__sndLast = clip;\n\
\treturn clip;\n\
}\n\
function __stopSound(?n) {\n\
\tvar c = null;\n\
\tif (n == null) { c = __sndLast; __sndLast = null; }\n\
\telse { var __i = __sndNames.indexOf(n); if (__i < 0) { return; } c = __sndClips[__i]; __sndClips[__i] = null; }\n\
\tif (c != null && c.stop != null) { c.stop(); }\n\
}\n".to_string()
}

fn build_sound_call(id: Option<&str>) -> String {
    let cfg = crate::mappings::api_commands();
    let t = &crate::mappings::script_templates().global.playsound;
    let req = crate::mappings::require_template;
    let id = match id {
        Some(i) if !i.is_empty() => i,
        _ => return req("global.playsound.placeholder_no_static_id", &t.placeholder_no_static_id)
            .replace("{{placeholder_id}}", PLACEHOLDER_SOUND_ID),
    };
    // the source's own sound first, wherever it was found: this engine's stand-in is only for a
    // sound no package carries
    let have = AVAILABLE_SOUNDS.with(|s| s.borrow().contains(id));
    if have {
        return req("global.playsound.asset", &t.asset).replace("{{ssf2_id}}", id);
    }
    if let Some(c) = cfg.global_sound_map.get(id) {
        return req("global.playsound.global", &t.global).replace("{{global_sfx}}", c);
    }
    req("global.playsound.placeholder_unmapped", &t.placeholder_unmapped)
        .replace("{{placeholder_id}}", PLACEHOLDER_SOUND_ID)
        .replace("{{ssf2_id}}", id)
}

/// Rewrite every `[receiver.]playSound( "id" [, …] )` call to its Fraymakers
/// `AudioClip.play(...)` equivalent. Uses balanced-paren scanning (SSF2
/// playSound args can contain nested calls, e.g. position objects), extracts
/// the first string-literal argument, and drops the rest (FM AudioClip.play
/// takes the content + an optional options object we don't synthesise).
/// Every projectile name the source fires by, in the order it fires them. What a fighter
/// spawns is stated in its own code, so this is what "the projectiles this character needs"
/// means -- not what its file happens to carry.
pub fn projectile_names_fired_in(code: &str) -> Vec<String> {
    // every spelling the source spawns by, since a fighter picks one and a copy ability picks
    // another, and a name that only one of them recognises is a move that fires nothing.
    // ...and a thing the code makes as an ITEM (`generateItem("bobomb", ...)`): a clip with its
    // own states (idle, held, tossed), pulled from wherever the game keeps it, that is a
    // projectile of this character's here, riding its holder until it is thrown
    const CALLS: [&str; 4] = ["fireProjectile(\"", "spawnProjectile(\"", "generateItem(\"", "generateCharacterItem(\""];
    let mut out: Vec<String> = Vec::new();
    for call in CALLS {
        let mut rest = code;
        while let Some(at) = rest.find(call) {
            rest = &rest[at + call.len()..];
            if let Some(end) = rest.find('"') {
                let name = &rest[..end];
                if !name.is_empty() && !out.iter().any(|n| n == name) {
                    out.push(name.to_string());
                }
            }
        }
    }
    // ...and a thing spawned by its CLASS (`spawnEnemy(PacManHydrant)`): the class is what the
    // source says exists, and the object it makes is a projectile of this character's here
    // ...and an item generated under a name chosen as the code runs (`generateItem(itemSelect)`,
    // peach's turnips and the items she pulls): the names it can take are the literals the
    // code lists, pushed onto or written into the list it picks from
    if ["generateItem(", "generateCharacterItem("].iter().any(|c| code.contains(c))
        && crate::re!(r#"generate(?:Character)?Item\(\s*[^"\s)]"#).is_match(code) {
        for c in crate::re!(r#"(?:\.push\(|[\[,]\s*|=\s*)"([A-Za-z_][\w]*)""#).captures_iter(code) {
            let name = c[1].to_string();
            if !out.contains(&name) { out.push(name); }
        }
    }
    for by_class in ["spawnEnemy(", "spawnItem("] {
        let mut rest = code;
        while let Some(at) = rest.find(by_class) {
            rest = &rest[at + by_class.len()..];
            let name: String = rest.trim_start().chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            if !name.is_empty() && !out.contains(&name) { out.push(name); }
        }
    }
    out
}

/// The projectiles a summoner FIRES AND THEN KEEPS, by their SSF2 names.
///
/// The source's idiom is plain: fire one, then ask the engine for what was just fired and
/// put it somewhere. What it puts away it goes on steering -- chibirobo's telly vision sits
/// on the stage for the whole match and is moved from the character's own code. A projectile
/// nobody keeps flies off and has to die on its own, which is what the default handlers are
/// for; giving them to a kept one kills it the first time it touches the floor.
pub fn projectiles_the_summoner_keeps(code: &str) -> std::collections::BTreeSet<String> {
    let mut kept = std::collections::BTreeSet::new();
    let mut last: Option<String> = None;
    for line in code.lines() {
        if let Some(at) = line.find("fireProjectile(\"") {
            let rest = &line[at + "fireProjectile(\"".len()..];
            if let Some(end) = rest.find('"') {
                let name = rest[..end].to_string();
                // stored as it is fired (`proj = fireProjectile("aurasphere")`): kept, and
                // steered from then on (lucario charges its sphere at its feet; given the
                // handlers of a shot nobody keeps it died on the floor the frame it appeared)
                if line[..at].contains('=') { kept.insert(name.clone()); }
                last = Some(name);
            }
        }
        // ...and then puts what it fired somewhere that outlives the call
        if line.contains("getCurrentProjectile()") {
            if let Some(name) = last.take() { kept.insert(name); }
        }
        // ...or stores it as it makes it: `hydrant = spawnEnemy(PacManHydrant)`, a thing spawned
        // by its class and steered from then on (pacman's hydrant, rayman's plum)
        if let Some(at) = line.find("spawnEnemy(") {
            let stored = line[..at].contains('=');
            let name: String = line[at + "spawnEnemy(".len()..].trim_start().chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            if stored && !name.is_empty() { kept.insert(name); }
        }
    }
    kept
}

/// Point "the projectile I just fired" at the one that was just made.
///
/// The source fires a projectile and then asks the engine for it, to keep hold of it: the
/// thing it fired is the thing it goes on to steer, and it also uses HAVING one as the sign
/// not to fire another. Struck as unportable, that second part is what breaks -- chibirobo
/// asks every frame whether it has a telly vision, never records the one it made, and makes
/// another, and another, for the whole match.
///
/// Nothing has to be tracked to answer it: the spawn rewrite above already keeps what it
/// made in a local, so the ask right after it is that local.
fn resolve_current_projectile(code: &str) -> String {
    if !code.contains("getCurrentProjectile") { return code.to_string() }
    let mut out: Vec<String> = Vec::new();
    let mut last: Option<String> = None;
    for line in code.lines() {
        // a function of its own starts over: what the last one fired is not in scope here
        if line.starts_with("function ") { last = None }
        if let Some((lhs, _)) = line.split_once('=') {
            if line.contains("match.createProjectile(") {
                let name = lhs.trim().trim_start_matches("var ").trim();
                if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                    last = Some(name.to_string());
                }
            }
        }
        match &last {
            Some(v) if line.contains("getCurrentProjectile()") => {
                let mut fixed = line.to_string();
                for recv in ["self.getCurrentProjectile()", "getCurrentProjectile()"] {
                    fixed = fixed.replace(recv, v);
                }
                out.push(fixed);
            }
            _ => out.push(line.to_string()),
        }
    }
    let mut joined = out.join("\n");
    if code.ends_with('\n') { joined.push('\n'); }
    joined
}

pub fn rewrite_fire_projectile_calls(code: &str) -> String {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| {
        // Consume the WHOLE dotted/method receiver chain before fireProjectile
        // (`self.`, `self.getOwner().`, …) so the block replaces the entire
        // `<receiver>.fireProjectile(...)` call. Leaving a dangling `receiver.`
        // before the emitted `{ … }` produces invalid `receiver.{ … }`. The
        // block always creates the projectile on `match` with `self` as owner.
        // Each arg may contain ONE level of parens (a method call like self.getXSpeed()).
        regex::Regex::new(
            // Each arg may nest calls two deep, and the source may pass more of them than the
            // two offsets this needs -- a fourth argument left the whole spawn struck.
            // Each arg may nest calls two deep: isaac aims its hand with
            // `self.flipX(self.getXSpeed())`, and one level of parens left that spawn struck --
            // so the move fired nothing, and the frames that steer what it fired steered null.
            r#"((?:[A-Za-z_]\w*(?:\([^()]*\))?\.)*)fireProjectile\(\s*"([^"]+)"\s*(?:,\s*((?:[^,()]|\((?:[^()]|\([^()]*\))*\))+?)\s*)?(?:,\s*((?:[^,()]|\((?:[^()]|\([^()]*\))*\))+?)\s*)?(?:,\s*((?:[^,()]|\((?:[^()]|\([^()]*\))*\))+?)\s*)?(?:,\s*(?:[^,()]|\((?:[^()]|\([^()]*\))*\))+?\s*)*\)"#,
        )
        .expect("fireProjectile regex")
    });
    let out = re.replace_all(code, |caps: &regex::Captures| {
        if code[..caps.get(0).unwrap().start()].trim_end().ends_with("function") { return caps[0].to_string() }
        let content_id = crate::projectile_gen::projectile_content_id(&caps[2]);
        // fired by whoever the source asks (a projectile firing the next of its kind asks the
        // fighter), so it is theirs and placed off them
        let who = fire_receiver(&caps[1]);
        // the offset the call gives, else the one the declaration gives, else none
        let declared = PROJECTILE_OFFSET.with(|c| c.borrow().get(&caps[2]).copied());
        // an offset the call states is in the source's pixels and crosses through `__sp` (the
        // size scale: pikachu's cloud sits 220 above it there, and 220 here is a bolt that lands
        // three source frames early); a declared one was scaled when it was read
        let off = |i: usize| {
            caps.get(i)
                .map(|m| m.as_str().trim())
                .filter(|s| !s.is_empty())
                .map(|s| if s == "0" { s.to_string() } else { format!("__sp({s})") })
                .or_else(|| declared.map(|(x, y)| crate::haxe_gen::fmt_num(if i == 3 { x } else { y })))
                .unwrap_or_else(|| "0".to_string())
        };
        // The launch the projectile's own package declares, given at the spawn: `flipX` so it
        // goes the way the fighter faces, which is what SSF2's own call does.
        let launch = PROJECTILE_LAUNCH.with(|c| c.borrow().get(&caps[2]).copied());
        let (xs, ys) = match launch {
            Some((x, y)) if x != 0.0 || y != 0.0 => (format!("_lw.flipX({})", crate::haxe_gen::fmt_num(x)), crate::haxe_gen::fmt_num(y)),
            _ => ("null".to_string(), "null".to_string()),
        };
        let rel_x = format!("{who}.getX() + {who}.flipX({})", off(3));
        let rel_y = format!("{who}.getY() + ({})", off(4));
        let (px, py) = spawn_place(caps.get(3).map(|m| m.as_str()), caps.get(4).map(|m| m.as_str()),
            caps.get(5).map(|m| m.as_str()), &rel_x, &rel_y);
        format!(
            // Placed only if there IS one. Asked for content the package does not ship, the
            // engine hands back nothing, and every setter after that throws -- so a spawn of
            // something missing took the rest of the frame with it instead of just not
            // happening. wario's final-smash afterimage is one of those.
            // THE LAUNCH COMES FIRST, as the source's engine gives it before the projectile's own
            // code runs, and that code may change it (link's arrow sets its speed from how long
            // the bow was drawn). the projectile takes it up from its owner as it starts; one that
            // does not is given it here, after.
            "{{ var _lw = {who}; var _lx = {px}; var _ly = {py}; \
if (_lw != null && _lw.exports != null) {{ _lw.exports.__launch = {{ x: _lx, y: _ly, xs: {xs}, ys: {ys} }}; }} \
var _proj = match.createProjectile(self.getResource().getContent(\"{content_id}\"), _lw); \
if (_proj != null && _lw != null && _lw.exports != null && _lw.exports.__launch != null) {{ var _l = _lw.exports.__launch; _lw.exports.__launch = null; \
_proj.setX(_l.x); _proj.setY(_l.y); if (_l.xs != null) {{ _proj.setXSpeed(_l.xs); _proj.setYSpeed(_l.ys); }} }} _proj; }}",
        )
    })
    .into_owned();
    fire_projectile_by_runtime_name(&out)
}

/// Whether the mapping tables already say what a call by this name becomes (a rename, a rewrite,
/// a split, or that it has no counterpart): such a name is theirs, not something to translate.
pub fn the_mappings_handle(name: &str) -> bool {
    let cfg = crate::mappings::api_commands();
    let call = format!("{name}(");
    cfg.replacements.iter().any(|r| r.from.contains(&call))
        || cfg.regex_replacements.iter().any(|r| r.pattern.contains(name))
        || cfg.call_rewrites.iter().any(|r| r.call == name)
        || cfg.call_splits.contains_key(name)
        || cfg.ssf2_only.iter().any(|e| e.name == name)
        || cfg.translated_in_code.iter().any(|n| n == name)
}

/// Who a fire is asked of, as the expression that names them (`self` when the source asks itself).
fn fire_receiver(chain: &str) -> String {
    let r = chain.trim_end_matches('.');
    if r.is_empty() || r == "self" { "self".to_string() } else { r.to_string() }
}

/// Where a fired projectile goes. The call's fourth argument asks for the two numbers to be the
/// place itself rather than an offset from the fighter, unless both are 0 (then it spawns where it
/// would anyway): megaman's wave fires its next wave at a place it worked out.
fn spawn_place(x: Option<&str>, y: Option<&str>, abs: Option<&str>, rel_x: &str, rel_y: &str) -> (String, String) {
    let (x, y) = (x.map(str::trim).unwrap_or("0"), y.map(str::trim).unwrap_or("0"));
    let both_zero = (x == "0" || x.is_empty()) && (y == "0" || y.is_empty());
    match abs.map(str::trim) {
        None | Some("false") | Some("null") | Some("0") | Some("") => (rel_x.to_string(), rel_y.to_string()),
        _ if both_zero => (rel_x.to_string(), rel_y.to_string()),
        Some("true") => (format!("__sp({x})"), format!("__sp({y})")),
        Some(a) => (
            format!("(__truthy({a}) && (({x}) != 0 || ({y}) != 0) ? __sp({x}) : {rel_x})"),
            format!("(__truthy({a}) && (({x}) != 0 || ({y}) != 0) ? __sp({y}) : {rel_y})"),
        ),
    }
}

/// A PROJECTILE FIRED BY A NAME WORKED OUT AS IT RUNS. bowser picks his fire breath by costume
/// (`fireProjectile(projId.get(), 40, -23)`), dedede and kirby by a global, and a call whose name
/// is not a literal was struck: the move fired nothing. The name is looked up when it runs among
/// the projectiles this fighter declares (`__fireNamed`, see `fire_named_helpers`), with the same
/// offsets and launch a literal spawn gets; a name it does not declare spawns nothing.
fn fire_projectile_by_runtime_name(code: &str) -> String {
    let re = crate::re!(r#"((?:[A-Za-z_]\w*(?:\([^()]*\))?\.)*)fireProjectile\(\s*((?:[^",()]|\((?:[^()]|\([^()]*\))*\))+?)\s*(?:,\s*((?:[^,()]|\((?:[^()]|\([^()]*\))*\))+?)\s*)?(?:,\s*((?:[^,()]|\((?:[^()]|\([^()]*\))*\))+?)\s*)?(?:,\s*((?:[^,()]|\((?:[^()]|\([^()]*\))*\))+?)\s*)?(?:,\s*(?:[^,()]|\((?:[^()]|\([^()]*\))*\))+?\s*)*\)"#);
    re.replace_all(code, |caps: &regex::Captures| {
        // a class's own method of that name is a definition, not a fire (pichu's, which adds to
        // what the engine's fire made)
        if code[..caps.get(0).unwrap().start()].trim_end().ends_with("function") { return caps[0].to_string() }
        let arg = |i: usize| caps.get(i).map(|m| m.as_str().trim().to_string()).filter(|s| !s.is_empty())
            .unwrap_or_else(|| "null".to_string());
        let abs = caps.get(5).map(|m| m.as_str().trim().to_string()).unwrap_or_else(|| "false".to_string());
        format!("__fireNamed({}, {}, {}, {}, {})", caps[2].trim(), arg(3), arg(4), abs, fire_receiver(&caps[1]))
    }).into_owned()
}

/// The helper a fire by a runtime name goes through, and the table of what this fighter declares:
/// each name folded the way the source folds it (case and underscores do not count), its content,
/// its declared offset and its launch.
pub fn fire_named_helpers() -> String {
    let launch = PROJECTILE_LAUNCH.with(|c| c.borrow().clone());
    let offset = PROJECTILE_OFFSET.with(|c| c.borrow().clone());
    let names: std::collections::BTreeSet<String> = launch.keys().chain(offset.keys()).cloned().collect();
    let fold = |n: &str| n.replace('_', "").to_lowercase();
    let list = |f: &dyn Fn(&String) -> String| names.iter().map(f).collect::<Vec<_>>().join(", ");
    let num = crate::haxe_gen::fmt_num;
    format!("// a projectile fired by a name worked out as it runs: one this fighter declares, found by\n\
// the name folded the way the source folds it; a name it does not declare spawns nothing\n\
var __pnNames = [{}];\n\
var __pnContent = [{}];\n\
var __pnOffX = [{}];\n\
var __pnOffY = [{}];\n\
var __pnLX = [{}];\n\
var __pnLY = [{}];\n\
function __fireNamed(n, ox, oy, ?abs, ?who) {{\n\
\tvar w = who != null ? who : self;\n\
\tif (n == null || Std.string(n) != n) {{ return null; }}\n\
\tvar __i = __pnNames.indexOf(Std.string(n).split(\"_\").join(\"\").toLowerCase());\n\
\tif (__i < 0) {{ return null; }}\n\
\t// where it goes and how it is launched, worked out first and left on its owner: the projectile\n\
\t// takes them up as it starts, before its own code runs (which may change them)\n\
\tvar lx = 0; var ly = 0;\n\
\t// a place the call states outright, where it asks for one (and says more than 0, 0)\n\
\tif (__truthy(abs) && ((ox != null && ox != 0) || (oy != null && oy != 0))) {{ lx = __sp(ox); ly = __sp(oy); }} else {{\n\
\t\tlx = w.getX() + w.flipX(ox != null ? __sp(ox) : __pnOffX[__i]);\n\
\t\tly = w.getY() + (oy != null ? __sp(oy) : __pnOffY[__i]); }}\n\
\tvar launched = __pnLX[__i] != 0 || __pnLY[__i] != 0;\n\
\tif (w.exports != null) {{ w.exports.__launch = {{ x: lx, y: ly, xs: launched ? w.flipX(__pnLX[__i]) : null, ys: launched ? __pnLY[__i] : null }}; }}\n\
\tvar _proj = match.createProjectile(self.getResource().getContent(__pnContent[__i]), w);\n\
\tif (_proj != null && w.exports != null && w.exports.__launch != null) {{\n\
\t\tvar l = w.exports.__launch; w.exports.__launch = null;\n\
\t\t_proj.setX(l.x); _proj.setY(l.y);\n\
\t\tif (l.xs != null) {{ _proj.setXSpeed(l.xs); _proj.setYSpeed(l.ys); }}\n\
\t}}\n\
\treturn _proj;\n\
}}\n",
        list(&|n| format!("\"{}\"", fold(n))),
        list(&|n| format!("\"{}\"", crate::projectile_gen::projectile_content_id(n))),
        list(&|n| num(offset.get(n).map(|o| o.0).unwrap_or(0.0))),
        list(&|n| num(offset.get(n).map(|o| o.1).unwrap_or(0.0))),
        list(&|n| num(launch.get(n).map(|o| o.0).unwrap_or(0.0))),
        list(&|n| num(launch.get(n).map(|o| o.1).unwrap_or(0.0))))
}

/// `spawnEnemy(SomeClass)` -> `match.createCustomGameObject(getContent("<id>SomeClass"), null)`.
///
/// SSF2 spawns a stage's objects by handing the engine a CLASS; Fraymakers spawns one by handing
/// `match` a piece of CONTENT. Those are the same act with different currency, and the converter
/// already ships each of those classes as content -- so the call has an exact equivalent and used
/// to be marked as having none. While it was, every spawn a stage's own code performs was
/// commented out, which is why a reconstructed stage was inert: it kept its timers, its random
/// rolls and its coordinates, and dropped the line that makes the thing exist.
///
/// `content_id` maps the SSF2 class name to the emitted content id, since only the caller knows
/// what the package ended up calling it.
pub fn rewrite_spawn_enemy_calls(code: &str, content_id: &dyn Fn(&str) -> String) -> String {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| {
        // the whole receiver chain goes with it: `match.`/`self.` before spawnEnemy would
        // otherwise be left dangling in front of the replacement.
        // the class either way: a stage names the class itself, or names it as a string, and both
        // are the stage saying which of its own objects to make.
        regex::Regex::new(r#"(?:[A-Za-z_]\w*(?:\([^()]*\))?\.)*spawnEnemy\(\s*"?([A-Za-z_]\w*)"?\s*\)"#).expect("regex")
    });
    re.replace_all(code, |caps: &regex::Captures| {
        format!("match.createCustomGameObject(self.getResource().getContent(\"{}\"), null)",
            content_id(&caps[1]))
    }).into_owned()
}

/// The stage's bounds, reached the way Fraymakers hands them over.
///
/// SSF2's `getDeathBounds()` / `getCamBounds()` give the rectangle itself; Fraymakers gives the
/// collision AREA the rectangle belongs to, one call further in. Read straight, `.x`/`.y`/`.width`
/// /`.height` come back null off the area object and the arithmetic around them collapses to
/// whatever constant it was added to -- which is a stage putting its hazards at the origin
/// instead of above the blast zone, and a spawn gate that never opens.
///
/// Applied wherever the call appears, since a stage asks ITSELF for its bounds (`self.get...`)
/// as often as it asks the api, and both lower onto the same Fraymakers method.
pub fn rewrite_stage_bounds(code: &str) -> String {
    let mut out = code.to_string();
    for name in ["getDeathBounds()", "getCameraBounds()", "getCamBounds()"] {
        let mut result = String::with_capacity(out.len());
        let mut rest = out.as_str();
        while let Some(i) = rest.find(name) {
            let end = i + name.len();
            result.push_str(&rest[..end]);
            if !rest[end..].starts_with(".getRectangle()") {
                result.push_str(".getRectangle()");
            }
            rest = &rest[end..];
        }
        result.push_str(rest);
        out = result;
    }
    out
}

/// The quality settings a stage asks about, answered at full quality.
///
/// SSF2's are the PLAYER's -- `stage_effects` is disabled/simple/enabled, `ambient_lighting`
/// likewise, `weather` is on or off -- and a stage asks, then skips what it chooses to. Fraymakers
/// has no such scale (its graphics settings are independent toggles), and the port is of the stage
/// at FULL quality, which is the state its art and timings were converted from. So the query
/// answers that, rather than having no answer: left unmapped, every statement asking it was struck
/// out, and clocktown's rain went with it.
pub fn rewrite_quality_settings(code: &str) -> String {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r"(?:[A-Za-z_]\w*(?:\([^()]*\))?\.)*getQualitySettings\(\)").expect("regex")
    });
    re.replace_all(code,
        "{ weather: true, stage_effects: \"enabled\", ambient_lighting: \"enabled\" }").into_owned()
}

/// A backdrop's timeline position, read the way Fraymakers offers it.
///
/// SSF2 asks a camera background's clip where it is (`getCameraBackgrounds()[0].mc.currentFrame`)
/// and branches on the number -- clocktown decides the time of day, whether the moon has fallen and
/// whether rocks should be falling entirely from where that timeline has got to. Fraymakers puts
/// the same timeline on the stage itself, so the stage's own frame is the same clock.
///
/// Returned in SSF2 frame units. The port runs at twice the rate, so the raw frame is twice the
/// source's; dividing keeps every frame NUMBER the source compares against meaningful, instead of
/// rewriting each of those literals and hoping none were missed.
pub fn rewrite_backdrop_frame_reads(code: &str) -> String {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r"[A-Za-z_][\w.()\[\]]*getCameraBackgrounds\(\)\[\d+\][\w.]*\.currentFrame").expect("regex")
    });
    re.replace_all(code, format!("Std.int(self.getCurrentFrame() / {})",
        crate::entity_gen::FRAME_RATIO).as_str()).into_owned()
}

/// A STOP THAT NAMES NO SOUND stops the last one this script started. A script of its own that
/// happens to carry the name is its own: a line that declares one is left alone.
pub fn a_stop_with_no_name_stops_the_last_sound(code: &str) -> String {
    let re = crate::re!(r"(^|[^.\w])(?:self\.|match\.)?stopSound\(\s*\)");
    code.lines().map(|line| {
        if line.trim_start().starts_with("function ") { return line.to_string() }
        re.replace_all(line, "${1}__stopSound()").into_owned()
    }).collect::<Vec<_>>().join("\n")
}

pub fn rewrite_play_sound_calls(code: &str) -> String {
    const NEEDLE: &str = "playSound(";
    let bytes = code.as_bytes();
    let mut out = String::with_capacity(code.len());
    let mut cursor = 0usize;
    while let Some(rel) = code[cursor..].find(NEEDLE) {
        let pos = cursor + rel; // index of 'p' in playSound(
        // Walk back over the dotted receiver (self. / self.self. / parent. …).
        let mut recv_start = pos;
        while recv_start > 0 {
            let c = bytes[recv_start - 1] as char;
            if c.is_alphanumeric() || c == '_' || c == '.' { recv_start -= 1; } else { break; }
        }
        // Guard: make sure this is really `playSound` and not a longer ident
        // ending in playSound (e.g. `myplaySound`). The char before the
        // receiver-run (or the run itself) must not glue another identifier.
        // recv_start..pos is the receiver; the token right before NEEDLE is
        // "playSound" only if recv_start==pos or recv_start ends with '.'.
        let receiver_ok = recv_start == pos || bytes[pos - 1] as char == '.';
        if !receiver_ok {
            out.push_str(&code[cursor..pos + NEEDLE.len()]);
            cursor = pos + NEEDLE.len();
            continue;
        }
        // Find the matching close paren.
        let open = pos + NEEDLE.len() - 1; // index of '('
        let mut depth = 0i32;
        let mut i = open;
        let mut close = None;
        while i < bytes.len() {
            match bytes[i] as char {
                '(' => depth += 1,
                ')' => { depth -= 1; if depth == 0 { close = Some(i); break; } }
                _ => {}
            }
            i += 1;
        }
        let close = match close {
            Some(e) => e,
            None => { out.push_str(&code[cursor..]); cursor = code.len(); break; }
        };
        // Extract first string-literal id from the args.
        let args = &code[open + 1..close];
        let id = args.find('"').and_then(|s| args[s + 1..].find('"').map(|e| &args[s + 1..s + 1 + e]));
        // Emit text up to the receiver, then the replacement.
        out.push_str(&code[cursor..recv_start]);
        // WHAT THE SOURCE STOPS LATER. A sound the source starts can be stopped by name, and the
        // engine stops one by the handle it gave back when it started. So the handle is kept under
        // whatever the source named the sound by -- a literal or a slot it reads -- and a stop by
        // that name stops that handle.
        let named = first_argument(args);
        out.push_str(&format!("__sound({}, {})",
                              if named.is_empty() { "null".to_string() } else { named },
                              build_sound_call(id)));
        cursor = close + 1;
    }
    out.push_str(&code[cursor..]);
    out
}

thread_local! {
    /// Per-character map of `<effect_name>` → first/primary animation name
    /// in its emitted `.entity` file. Set at the top of `haxe_gen::generate`
    /// via `with_effect_animations` and cleared on return. Used only by
    /// `rewrite_attach_effect_calls`. Empty map = no effects discovered
    /// (or we're outside a `with_effect_animations` scope) → all calls
    /// fall through to the `ssf2_only` marker.
    static EFFECT_PRIMARY_ANIMS: std::cell::RefCell<BTreeMap<String, String>>
        = const { std::cell::RefCell::new(BTreeMap::new()) };
}

thread_local! {
    /// SSF2 animation name -> Fraymakers animation name, for the character being generated.
    /// Installed by `AnimNameGuard`; read where a call names an animation.
    static ANIM_NAMES: std::cell::RefCell<BTreeMap<String, String>>
        = const { std::cell::RefCell::new(BTreeMap::new()) };
    /// A frame label that STARTS an emitted animation -> that animation. Installed alongside
    /// `ANIM_NAMES`; read where a call jumps to a label, since a label at the head of its own
    /// animation is that animation and cannot be reached as a label any more.
    static LABEL_ANIMS: std::cell::RefCell<BTreeMap<String, String>>
        = const { std::cell::RefCell::new(BTreeMap::new()) };
}

thread_local! {
    /// True while translating code whose `self` IS the stage. Installed by `SelfIsStageGuard`.
    static SELF_IS_STAGE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// True while translating a fighter's or a projectile's script, whose positions cross the
    /// size scale through `__sp`/`__isp` (see `SpeedUnits::ApiLayer`). A stage object's script
    /// carries its own position boundary and its own passes read its `getX()` calls, so it is
    /// left alone. Installed by `PositionsCrossGuard`.
    static POSITIONS_CROSS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// RAII guard marking the code being translated as a fighter's or a projectile's, whose
/// positions cross the size scale.
pub struct PositionsCrossGuard;

impl PositionsCrossGuard {
    pub fn install() -> Self {
        POSITIONS_CROSS.with(|c| c.set(true));
        PositionsCrossGuard
    }
}

impl Drop for PositionsCrossGuard {
    fn drop(&mut self) { POSITIONS_CROSS.with(|c| c.set(false)); }
}

thread_local! {
    /// Code of a stage or of an object it spawns is being translated: its spawns by class are the
    /// stage emitter's to make (custom game objects), not a fighter's projectile fire.
    static STAGE_CONTENT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Marks the translation it lives through as a stage's (see [`STAGE_CONTENT`]).
pub struct StageContentGuard(bool);

impl StageContentGuard {
    pub fn install() -> Self { StageContentGuard(STAGE_CONTENT.with(|c| c.replace(true))) }
}

impl Drop for StageContentGuard {
    fn drop(&mut self) { let was = self.0; STAGE_CONTENT.with(|c| c.set(was)); }
}

/// RAII guard marking the code being translated as a STAGE's own.
///
/// The same call means two things depending on who runs it. A stage asking for the stage is asking
/// for ITSELF, and `self` is the answer; anybody else is asking for something no script can reach,
/// and there is nothing to translate it to.
pub struct SelfIsStageGuard;

impl SelfIsStageGuard {
    pub fn install() -> Self {
        SELF_IS_STAGE.with(|c| c.set(true));
        SelfIsStageGuard
    }
}

impl Drop for SelfIsStageGuard {
    fn drop(&mut self) { SELF_IS_STAGE.with(|c| c.set(false)); }
}

/// A stage asking for the stage is asking for itself.
fn rewrite_stage_self(code: &str) -> String {
    if !SELF_IS_STAGE.with(|c| c.get()) { return code.to_string() }
    let code = code.replace("SSF2API.getStage()", "self");
    // ...and the stage's own blast and camera boxes are the regions it was drawn with, kept by
    // name in the script (butter building walks its warning edges against the blast box)
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| regex::Regex::new(
        r"\b(?:self|match|SSF2API)\.get(Death|Cam|Camera)Bounds\(\)(?:\.getRectangle\(\))?").expect("regex"));
    re.replace_all(&code, |c: &regex::Captures| {
        let n = if &c[1] == "Death" { "deathBoundary" } else { "camBoundary" };
        format!("__objectBoundary(\"{n}\")")
    }).into_owned()
}

/// Rewrite a jump-to-label into the call Fraymakers actually has.
///
/// SSF2 chains a combo by jumping its timeline to a label: mario's jab reads the button and jumps
/// to `hit2`. Fraymakers has ONE jab state and separate animations, so the label that started the
/// next stage of the combo IS an animation now, and the way to reach it is `playAnimation`. A
/// label still sitting INSIDE an animation stays a label and gets `playFrameLabel`. Neither is
/// `playLabel`, which nothing answers to: left as it stood, the call died with "Invalid function
/// null" and the combo stopped at its first hit.
pub fn rewrite_play_label_calls(code: &str) -> String {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r#"\.playLabel\(\s*"([^"]+)"\s*\)"#).expect("regex")
    });
    LABEL_ANIMS.with(|c| {
        let labels = c.borrow();
        ANIM_NAMES.with(|a| {
            let anims = a.borrow();
            re.replace_all(code, |caps: &regex::Captures| {
                let name = &caps[1];
                // ONLY when it names something the entity actually has. Asked for an animation it
                // does not carry, the engine leaves the object with no animation at all and dies
                // on the next frame it draws it (GameCamera, null access) -- sandbag's crouch exit
                // is emitted as `crouch_out` and the label is `uncrouch`, so the call went out
                // naming nothing and the match ended the moment he stood up. A name that resolves
                // to nothing is left as it was, for the pass that strikes out what has no
                // counterpart: unplayed is what it was before, and it does not take the match
                // with it.
                match labels.get(name).or_else(|| anims.get(name)) {
                    Some(anim) => format!(".playAnimation(\"{anim}\")"),
                    None => format!(".playLabel(\"{name}\")"),
                }
            }).into_owned()
        })
    })
}

/// RAII guard installing the character's SSF2 -> Fraymakers animation names.
///
/// A script that says `playFrameLabel("collapse")` is naming an SSF2 frame label, and the entity
/// carries that animation under the name the converter gave it (`crash_collapse`). Left
/// untranslated the engine is asked for an animation that does not exist, plays nothing, and the
/// character stops being drawn from that moment on -- which is what sandbag did, a few seconds
/// into every match, once its stand timer ran out.
pub struct AnimNameGuard;

impl AnimNameGuard {
    pub fn install(map: BTreeMap<String, String>) -> Self {
        Self::install_with_labels(map, BTreeMap::new())
    }

    /// `labels` maps a frame label that STARTS an emitted animation to that animation.
    pub fn install_with_labels(map: BTreeMap<String, String>, labels: BTreeMap<String, String>) -> Self {
        ANIM_NAMES.with(|c| { *c.borrow_mut() = map; });
        LABEL_ANIMS.with(|c| { *c.borrow_mut() = labels; });
        AnimNameGuard
    }
}

impl Drop for AnimNameGuard {
    fn drop(&mut self) {
        ANIM_NAMES.with(|c| { c.borrow_mut().clear(); });
        LABEL_ANIMS.with(|c| { c.borrow_mut().clear(); });
    }
}

/// Rewrite the animation a call names into the name the entity actually carries.
///
/// The map is the character's own animation names. A label the entity carries under a longer name
/// (SSF2's `collapse` is emitted as `crash_collapse`) resolves when exactly one animation ends
/// with it; more than one candidate means the label is ambiguous and is left alone rather than
/// guessed at. Only exact string literals are touched -- an animation named at runtime cannot be
/// resolved here.
pub fn rewrite_anim_name_args(code: &str) -> String {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| {
        // capture the receiver too, so an unresolvable cross-animation label jump can be rerouted
        // through _jumpLabel (which resolves by SOURCE CLIP, the one thing that disambiguates a
        // label like "attack" that many split animations carry)
        regex::Regex::new(r#"([A-Za-z_][\w.()\[\]]*)\.(playFrameLabel|toState|playAnimation)\(\s*"([^"]+)"\s*\)"#).expect("regex")
    });
    ANIM_NAMES.with(|c| {
        let names = c.borrow();
        if names.is_empty() { return code.to_string() }
        re.replace_all(code, |caps: &regex::Captures| {
            let recv_raw = &caps[1];
            // the receiver char class allows parens (a call receiver), so when an enclosing
            // null-guard has already wrapped this receiver (`(x == null ? null : x.countMC)`), the
            // match grabs the guard's trailing `)` too, and rebuilding the call around that orphan
            // splits it (`_jumpLabel(x.countMC), "two", null)` -- a parse error that takes the whole
            // script's scope down. balance the receiver: trim the unmatched trailing `)` and put
            // them back AFTER the rewritten call, so the guard stays whole around it.
            let mut depth = 0i32; let mut trim = 0usize;
            for ch in recv_raw.chars() { if ch == '(' { depth += 1 } else if ch == ')' { depth -= 1 } }
            if depth < 0 {
                let mut need = -depth;
                for ch in recv_raw.chars().rev() {
                    if ch == ')' { trim += 1; need -= 1; if need == 0 { break } }
                    else if ch != ' ' { break }
                }
            }
            let (recv_s, tail) = recv_raw.split_at(recv_raw.len() - trim);
            let recv = recv_s;
            // if the kept receiver is STILL not bracket-balanced, the match began in the middle of a
            // bigger expression (waluigi's `map.get()[__iterName(_v4, _v3)].vine.playFrameLabel(...)`:
            // the char class cannot cross the `, ` inside the index, so the match starts at `_v3)].vine`).
            // rerouting that fragment would weld `_jumpLabel(_v3)].vine, ...` into the enclosing index
            // and break the whole script, so leave the call untouched -- a valid, if unrerouted, form.
            let balanced = |s: &str| {
                let (mut p, mut b) = (0i32, 0i32);
                for ch in s.chars() { match ch { '('=>p+=1, ')'=>p-=1, '['=>b+=1, ']'=>b-=1, _=>{} } if p < 0 || b < 0 { return false } }
                p == 0 && b == 0
            };
            if !balanced(recv) { return caps[0].to_string(); }
            let call = &caps[2];
            let name = &caps[3];
            let wrap = |s: String| format!("{s}{tail}");
            if names.contains_key(name) { return wrap(format!("{recv}.{call}(\"{name}\")")) }
            // a label of the animation being emitted is a frame of it, under its own name
            if call == "playFrameLabel" && CURRENT_LABELS.with(|c| c.borrow().contains(name)) {
                return wrap(format!("{recv}.{call}(\"{name}\")"));
            }
            // a label that STARTS an animation is that animation: SSF2's jab jumps to `hit2`,
            // which the split emitted as `jab2`, and the engine knows it by no other name
            if let Some(anim) = LABEL_ANIMS.with(|l| l.borrow().get(name).cloned()) {
                return wrap(format!("{recv}.{call}(\"{anim}\")"));
            }
            let suffix = format!("_{name}");
            let mut hits = names.keys().filter(|k| k.ends_with(&suffix));
            wrap(match (hits.next(), hits.next()) {
                (Some(only), None) => format!("{recv}.{call}(\"{only}\")"),
                // an ambiguous cross-clip label (many animations end `_attack` once charge specials
                // are split): a same-clip label jump the engine cannot name is resolved at runtime by
                // _jumpLabel off the label tables, keyed on the clip the playing animation came from
                _ if call == "playFrameLabel" => format!("_jumpLabel({recv}, \"{name}\", null)"),
                _ => format!("{recv}.{call}(\"{name}\")"),
            })
        }).into_owned()
    })
}

/// ...and in a projectile's code, where its own `forceAttack` is a state of its own: only the one
/// it belongs to (or another it holds) asked for one of the fighter's moves.
pub fn rewrite_force_attack_on_others(code: &str, ssf2_to_fm: &BTreeMap<String, String>) -> String {
    force_attack_moves(code, ssf2_to_fm, true)
}

fn force_attack_moves(code: &str, ssf2_to_fm: &BTreeMap<String, String>, others_only: bool) -> String {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    // the commented form the decompile-time strike leaves: `// [SSF2-only: forceAttack]
    // self.forceAttack("<move>", ...)`. only the first arg (the move) matters here.
    // whoever is asked (the fighter, or the one a projectile belongs to, asked for one of this
    // fighter's moves), the move, and where in it to start: a label, or a frame of the source's
    let re = RE.get_or_init(|| regex::Regex::new(
        r#"^(\s*)// \[SSF2-only: forceAttack\] ((?:[A-Za-z_]\w*(?:\(\))?\.)*)forceAttack\(\s*"([^"]+)"\s*(?:,\s*("[^"]*"|\d+|null)\s*)?(?:,[^\n]*)?\);?\s*$"#).expect("regex"));
    let mut out: Vec<String> = Vec::new();
    for line in code.lines() {
        if let Some(caps) = re.captures(line) {
            let indent = &caps[1];
            let recv = caps[2].trim_end_matches('.');
            let who = if recv.is_empty() || recv == "self" { "self" } else { recv };
            if others_only && who == "self" { out.push(line.to_string()); continue }
            let mv = &caps[3];
            if let Some(fm) = ssf2_to_fm.get(mv).filter(|s| !s.is_empty()) {
                // drop the generic note the strike wrote above the call: it described the gap that
                // is now filled, and a stale reason next to a live line reads as a mistake.
                if out.last().map(|l: &String| l.trim_start().starts_with("// SSF2 buffers a raw attack input")).unwrap_or(false) {
                    out.pop();
                }
                let start = match caps.get(4).map(|m| m.as_str()) {
                    Some(l) if l.starts_with('"') && who == "self" => format!(" _jumpLabel(self, {l}, null);"),
                    Some(l) if l.starts_with('"') => format!(" if ({who}.exports != null) {{ {who}.exports.__askLabel = {l}; }}"),
                    Some(n) if n.parse::<i64>().is_ok() => format!(" {who}.playFrame({});", (n.parse::<i64>().unwrap() - 1) * 2 + 1),
                    _ => String::new(),
                };
                out.push(format!("{indent}{who}.playAnimation(\"{fm}\");{start} // SSF2 forceAttack(\"{mv}\")"));
                continue;
            }
        }
        out.push(line.to_string());
    }
    // `lines()` drops a trailing newline; keep the file's original ending.
    let mut joined = out.join("\n");
    if code.ends_with('\n') { joined.push('\n'); }
    joined
}

pub fn strike_unplayable_own_animations(code: &str, names: &[String]) -> String {
    if names.is_empty() { return code.to_string() }
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r#"(?m)^([ 	]*)self\.(playAnimation|playFrameLabel)\(\s*"([^"]+)"\s*\)\s*;"#).expect("regex")
    });
    // a projectile's labels are its animations: a jump to one is playing it (the engine's label
    // jump looks for a frame label the entity does not carry, and falco's afterimage looping back
    // to its `loop` took the engine down)
    let out = re.replace_all(code, |caps: &regex::Captures| {
        let (indent, call, name) = (&caps[1], &caps[2], &caps[3]);
        if names.iter().any(|n| n == name) {
            return if call == "playFrameLabel" { format!("{indent}__playOwn(\"{name}\");") } else { caps[0].to_string() }
        }
        format!("{indent}// [it has no animation of this name] self.{call}(\"{name}\")")
    }).into_owned();
    // and the same ask wherever else it sits: the source often checks something first, which
    // puts the whole thing on one line, and a comment cannot be hung on part of a line
    static ANY: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let any = ANY.get_or_init(|| {
        regex::Regex::new(r#"self\.(playAnimation|playFrameLabel)\(\s*"([^"]+)"\s*\)"#).expect("regex")
    });
    any.replace_all(&out, |caps: &regex::Captures| {
        let name = &caps[2];
        if names.iter().any(|n| n == name) {
            return if &caps[1] == "playFrameLabel" { format!("__playOwn(\"{name}\")") } else { caps[0].to_string() }
        }
        format!("null /* [it has no animation of this name: {name}] */")
    }).into_owned()
}

pub fn strike_unplayable_owner_animations(code: &str) -> String {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r#"(?m)^([ 	]*)((?:character|char|owner)(?:\.get\(\))?)\.(playAnimation|playFrameLabel)\(\s*"([^"]+)"\s*\)\s*;"#).expect("regex")
    });
    ANIM_NAMES.with(|a| {
        let anims = a.borrow();
        if anims.is_empty() { return code.to_string() }
        let out = re.replace_all(code, |caps: &regex::Captures| {
            let (indent, call, name) = (&caps[1], &caps[3], &caps[4]);
            if anims.contains_key(name) { return caps[0].to_string() }
            format!("{indent}// [the character has no animation of this name] {}.{call}(\"{name}\")",
                &caps[2])
        }).into_owned();
        static ANY: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        let any = ANY.get_or_init(|| {
            regex::Regex::new(r#"(?:character|char|owner)(?:\.get\(\))?\.(playAnimation|playFrameLabel)\(\s*"([^"]+)"\s*\)"#).expect("regex")
        });
        any.replace_all(&out, |caps: &regex::Captures| {
            let name = &caps[2];
            if anims.contains_key(name) { return caps[0].to_string() }
            format!("null /* [the character has no animation of this name: {name}] */")
        }).into_owned()
    })
}

thread_local! {
    /// the labels the character's animations carry: a jump to one of these is a jump inside
    /// an animation, and stands whether or not it names an animation. the whole set, since
    /// the passes that strike and rename run again over the finished frame scripts, after
    /// the animation they belong to is no longer in hand
    static CURRENT_LABELS: std::cell::RefCell<std::collections::BTreeSet<String>> = Default::default();
}

/// name the labels the character's animations carry (an empty list forgets them)
pub fn set_current_labels(labels: &[String]) {
    CURRENT_LABELS.with(|c| {
        let mut c = c.borrow_mut();
        if labels.is_empty() { c.clear(); }
        for l in labels { c.insert(l.clone()); }
    });
}

/// RAII guard that installs an effect→primary-animation map on
/// construction and clears it on drop. Use at the top of a
/// per-character generation pass so any `translate_ssf2_to_fm` call
/// made on this thread (Character.entity frame scripts, Script.hx,
/// per-attack scripts) gets context-aware attachEffect rewriting.
pub struct EffectAnimGuard;

impl EffectAnimGuard {
    pub fn install(map: BTreeMap<String, String>) -> Self {
        EFFECT_PRIMARY_ANIMS.with(|cell| { *cell.borrow_mut() = map; });
        EffectAnimGuard
    }
}

impl Drop for EffectAnimGuard {
    fn drop(&mut self) {
        EFFECT_PRIMARY_ANIMS.with(|cell| { cell.borrow_mut().clear(); });
    }
}


/// Whether an `attachEffect("name")` can actually be spawned in the converted output: either the
/// name maps to one of the engine's own global VFX, or this character emitted an effect entity
/// for it.
///
/// Anything else is a SHARED SSF2 effect that lives in some other character's file (sandbag's
/// `effect_land` is defined in bandanadee's), and referencing one is not harmless. `getContent`
/// does not validate: it builds `private::<resource>.<id>` from whatever it is handed, so a
/// missing id yields a DANGLING reference, and a VfxStats built on one renders the resource's
/// first animation instead of nothing. For a character that is `revival` -- so a move with an
/// unconvertible effect drew the character on its revival platform for a frame or two, in
/// whatever move happened to spawn it.
pub fn effect_is_spawnable(name: &str) -> bool {
    if crate::mappings::api_commands().global_vfx_map.contains_key(name) { return true; }
    EFFECT_PRIMARY_ANIMS.with(|cell| cell.borrow().contains_key(name))
}

/// The note left where an unspawnable effect was asked for.
fn unspawnable_note(name: &str) -> String {
    format!("attachEffect(\"{name}\") not spawned: no GlobalVfx mapping and no local effect entity \
(an SSF2 shared effect defined in another character's file). Add a global_vfx_map entry in \
commands.jsonc, or supply a {name}.entity")
}

/// Rewrite `self.attachEffect("name")` and `self.attachEffect("name", { props })`
/// to `match.createVfx(new VfxStats({ spriteContent: …, animation: "<primary>", <translated props> }), self)`.
///
/// Animation-name resolution:
///   - If `name` is in the thread_local map (a local effect we extracted),
///     use that effect's primary animation name — picks the right name
///     even when the entity has multiple FrameLabel-derived animations.
///   - If `name` is not in the map, fall back to `"active"`. Most
///     unknown references are FM-global effects (`global_dust_blast`,
///     `global_spark`, `itempickup_effect`, …) whose entities live in
///     the engine's standard library; `"active"` is the project-wide
///     default animation name for VFX entities we emit, so any local
///     fallback also lands on a valid animation. Worst case the
///     animation is wrong → runtime warning; never worse than the
///     original SSF2 call which we couldn't translate at all.
///
/// Per-prop translation:
///   - The 2-arg form parses the `{…}` block with paren-aware comma
///     splitting (so `Random.getFloat(0, 1)` is one value, not two).
///     Each `key: value` is routed through the `attach_effect_props`
///     mapping table in commands.jsonc, which supports direct renames,
///     1→N expansions (e.g. `parentLock` → `relativeWith` +
///     `resizeWith` + `flipWith`), and explicit `todo` notes for keys
///     with no clean FM equivalent (`syncHitStun`, `loop`, `behind`, …).
///   - Unmapped props emit a `// TODO: <key>: <value> — note` line
///     above the call, preserving the original value alongside the
///     reason. The call line itself never carries free-form comments.
/// An OVERLAY is the same attachment, drawn over the character rather than under it. Fraymakers
/// spawns a vfx at the entity and does not offer the choice, so it goes the same way: attached at
/// the character, which is the part that matters, rather than struck out for the part that cannot
/// be said.
/// The effects this character ships, by the name the source calls them and the animation each
/// one starts on.
pub fn local_effect_table() -> Vec<(String, String)> {
    EFFECT_PRIMARY_ANIMS.with(|cell| {
        cell.borrow().iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    })
}

/// AN EFFECT NAMED BY SOMETHING WORKED OUT AS IT RUNS. The name is checked before an effect is
/// spawned, because `getContent` does not check: handed a name it does not have, it builds a
/// reference to nothing and the vfx draws the resource's FIRST animation instead -- which for a
/// character is its revival, on its platform, in the middle of a move. So a call that names its
/// effect with a variable was struck, and the slot it would have filled stayed empty: pacman
/// attaches every one of its effects through one helper, and read that empty slot afterwards.
///
/// The check can happen while it runs instead. The names this character ships go in a table, and
/// a name that is not in it spawns nothing -- the same answer as before, without drawing the
/// wrong thing.
pub fn rewrite_attach_effect_by_runtime_name(code: &str) -> String {
    // whatever the name is worked out from (a variable, or a name and a number put together:
    // peach's float picks one of two sparkles as it goes); a literal name was rewritten already
    let mut out = String::with_capacity(code.len());
    let mut rest = code;
    loop {
        let found = ["self.attachEffectOverlay(", "self.attachEffect("].iter()
            .filter_map(|n| rest.find(n).map(|i| (i, n.len()))).min();
        let Some((at, len)) = found else { break };
        let open = at + len - 1;
        let Some(close) = matching_paren(rest, open) else { out.push_str(&rest[..at + len]); rest = &rest[at + len..]; continue };
        let args = split_top_level(&rest[open + 1..close]);
        let name = args.first().map(|a| a.trim()).unwrap_or("");
        let literal = name.starts_with('"') && name.ends_with('"') && name.matches('"').count() == 2;
        if name.is_empty() || literal || args.len() > 2 {
            out.push_str(&rest[..close + 1]); rest = &rest[close + 1..]; continue
        }
        let opts = args.get(1).map(|a| a.trim().to_string()).unwrap_or_else(|| "{}".to_string());
        out.push_str(&rest[..at]);
        out.push_str(&format!("_attachEffect({name}, {opts})"));
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    out
}

/// Whether code names an effect by something worked out as it runs (which `_attachEffect` answers),
/// however the call is spelled: on `self`, or bare as a clip's timeline writes it.
pub fn names_effect_at_runtime(code: &str) -> bool {
    if code.contains("_attachEffect(") { return true }
    let on_self = crate::re!(r"(^|[^\w.])(attachEffect(?:Overlay)?\()").replace_all(code, "${1}self.${2}");
    rewrite_attach_effect_by_runtime_name(&on_self).contains("_attachEffect(")
}

/// The `)` that closes the `(` at `open`, strings and nesting counted.
fn matching_paren(s: &str, open: usize) -> Option<usize> {
    let b = s.as_bytes();
    let (mut depth, mut i, mut q) = (0i32, open, None::<u8>);
    while i < b.len() {
        let c = b[i];
        if let Some(qc) = q {
            if c == b'\\' { i += 2; continue }
            if c == qc { q = None }
        }
        else if c == b'"' || c == b'\'' { q = Some(c) }
        else if c == b'(' || c == b'{' || c == b'[' { depth += 1 }
        else if c == b')' || c == b'}' || c == b']' { depth -= 1; if depth == 0 { return (c == b')').then_some(i) } }
        i += 1;
    }
    None
}


pub fn rewrite_attach_effect_calls(code: &str) -> String {
    static RE_LINE_2ARG: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    static RE_ANY_2ARG: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    static RE_1ARG: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    // Two 2-arg patterns. The first is line-anchored (`(?m)^([ \t]*)…`)
    // and captures the line's indent so the emitted `// TODO:` lines
    // can mirror it. The second is the mid-line fallback (assignments
    // like `self.effect = self.attachEffect(…)` or calls that follow
    // an inline comment); it can't safely inject TODO lines without
    // breaking the surrounding expression, so unmapped props are
    // dropped silently in that case. The two passes run in order, so
    // the line-anchored form is preferred whenever it can match.
    // WHO THE EFFECT IS ATTACHED TO is whatever the call is made on: the fighter itself, another
    // object it names, or the match, which puts the effect at a place on the stage rather than on
    // anything. The engine takes the same three: the object as the vfx's owner, or no owner at
    // all, which is what a place on the stage is.
    // the receiver stands on its own: a hop through another script's exports is one script
    // calling another, not an object to hand the engine, and is left where it is
    const RECV: &str = r#"(self|match|(?:[A-Za-z_]\w*\.)*[A-Za-z_]\w*(?:\.get\(\))?)"#;
    // mid-line, the receiver has to stand on its own: a hop through another script's exports is
    // one script calling another, not an object to hand the engine, and is left where it is
    const LEAD: &str = r#"(?m)(^|[^.\w])"#;
    let re_line_2arg = RE_LINE_2ARG.get_or_init(|| {
        regex::Regex::new(&format!(r#"(?m)^([ \t]*){RECV}\.attachEffect(?:Overlay)?\(\s*"([^"]*)"\s*,\s*\{{([^{{}}]*)\}}\s*\)"#)).expect("regex")
    });
    let re_any_2arg = RE_ANY_2ARG.get_or_init(|| {
        regex::Regex::new(&format!(r#"{LEAD}{RECV}\.attachEffect(?:Overlay)?\(\s*"([^"]*)"\s*,\s*\{{([^{{}}]*)\}}\s*\)"#)).expect("regex")
    });
    let re_1arg = RE_1ARG.get_or_init(|| {
        regex::Regex::new(&format!(r#"{LEAD}{RECV}\.attachEffect(?:Overlay)?\(\s*"([^"]*)"\s*\)"#)).expect("regex")
    });
    // what the engine is handed as the vfx's owner: nothing when the effect was put on the match,
    // which is a place on the stage and nobody's
    let owned_by = |recv: &str| -> String {
        if recv == "match" { String::new() } else { format!(", {recv}") }
    };
    // a hop through another script's exports is one script calling another, not an object
    let a_script_asking_another = |recv: &str| recv.ends_with(".exports");

    // Build the translated `new VfxStats({…})` literal from the parsed
    // props. Returns the bag body (no surrounding `{}`) plus a list of
    // TODO notes for unmapped props.
    let build_bag = |name: &str, props_block: &str| -> (String, Vec<String>) {
        let mut fm_fields: Vec<(String, String)> = Vec::new();
        let mut todo_lines: Vec<String> = Vec::new();
        for (key, value) in parse_prop_bag(props_block) {
            translate_attach_effect_prop(&key, &value, &mut fm_fields, &mut todo_lines);
        }
        drop_redundant_flip(&mut fm_fields);
        let substituted = crate::mappings::api_commands().global_vfx_map.contains_key(name);
        let positions = POSITIONS_CROSS.with(|c| c.get());
        for (k, v) in fm_fields.iter_mut() {
            // A SIGN ON THE SCALE IS THE SOURCE ART'S: the source draws its dust upside down and
            // flips it back with `scaleY: -0.5`, and that flip put this engine's own dust (a
            // different drawing, authored upright) on its head. When the effect is this engine's
            // asset, the size carries over and the flip does not.
            if substituted && (k == "scaleX" || k == "scaleY") {
                let t = v.trim();
                *v = match t.parse::<f64>() {
                    Ok(n) => crate::haxe_gen::fmt_num(n.abs()),
                    Err(_) => format!("Math.abs({t})"),
                };
            }
            // where the effect sits is said in the source's units, off the fighter's origin: it
            // crosses the boundary the way every other position does
            if positions && (k == "x" || k == "y") {
                let t = v.trim();
                let zero = t == "0" || t == "0.0";
                if !zero && !t.starts_with("__sp(") { *v = format!("__sp({t})"); }
            }
        }
        let mut bag = build_vfx_head(name);
        for (k, v) in &fm_fields {
            bag.push_str(&format!(", {}: {}", k, v));
        }
        (bag, todo_lines)
    };

    // Pass 1: line-anchored 2-arg form — emit TODOs above the call.
    let after_line_2arg = re_line_2arg.replace_all(code, |caps: &regex::Captures| {
        let indent = &caps[1];
        if a_script_asking_another(&caps[2]) { return caps[0].to_string() }
        let owner = owned_by(&caps[2]);
        let name = &caps[3];
        let props_block = &caps[4];
        let (bag, todo_lines) = build_bag(name, props_block);

        let mut out = String::new();
        for note in &todo_lines {
            out.push_str(indent);
            out.push_str("// TODO: ");
            out.push_str(note);
            out.push('\n');
        }
        out.push_str(indent);
        if effect_is_spawnable(name) {
            out.push_str(&format!(
                "match.createVfx(new VfxStats({{ {bag} }}){owner})"
            ));
        } else {
            // Not spawnable: leave the intent visible and spawn NOTHING. Emitting the call anyway
            // draws the entity's first animation (see effect_is_spawnable).
            out.push_str(&format!("// TODO: {}", unspawnable_note(name)));
        }
        out
    });

    // Pass 2: mid-line 2-arg fallback — translate inline. Embedded
    // calls can't host above-line TODOs (would break the surrounding
    // expression), so any unmapped props ride along as a trailing
    // `/* TODO: … */` block-comment instead. That keeps the warning
    // visible without altering line topology.
    let after_any_2arg = re_any_2arg.replace_all(&after_line_2arg, |caps: &regex::Captures| {
        let lead = &caps[1];
        if a_script_asking_another(&caps[2]) { return caps[0].to_string() }
        let owner = owned_by(&caps[2]);
        let name = &caps[3];
        let props_block = &caps[4];
        let (bag, todos) = build_bag(name, props_block);
        if !effect_is_spawnable(name) {
            return format!("{lead}null /* TODO: {} */", unspawnable_note(name));
        }
        let call = format!("{lead}match.createVfx(new VfxStats({{ {bag} }}){owner})");
        if todos.is_empty() {
            call
        } else {
            format!("{} /* TODO: {} */", call, todos.join(" | TODO: "))
        }
    });

    // 1-arg form — no props to translate; the head alone is the bag.
    let after_1arg = re_1arg.replace_all(&after_any_2arg, |caps: &regex::Captures| {
        let lead = &caps[1];
        if a_script_asking_another(&caps[2]) { return caps[0].to_string() }
        let owner = owned_by(&caps[2]);
        let name = &caps[3];
        if !effect_is_spawnable(name) {
            return format!("{lead}null /* TODO: {} */", unspawnable_note(name));
        }
        let head = build_vfx_head(name);
        format!("{lead}match.createVfx(new VfxStats({{ {head} }}){owner})")
    });
    after_1arg.into_owned()
}

/// Build the `spriteContent: …, animation: …` head of the VfxStats bag
/// for an effect named `name`. Shape selection:
///   - If `name` matches `global_vfx_map`, emit the Fraymakers global
///     shape:
///       `spriteContent: "global::vfx.vfx", animation: GlobalVfx.<C>`
///     The constant is unquoted because it's an accessor on the
///     `GlobalVfx` class, not a string literal — the engine resolves
///     the underlying animation name at runtime.
///   - Otherwise emit the per-character shape that resolves `name`
///     against the local effect entity we generated:
///       `spriteContent: self.getResource().getContent("name"),
///        animation: "<primary or active fallback>"`
///     where the animation name comes from `EFFECT_PRIMARY_ANIMS` (set
///     up per-character by `haxe_gen::generate`), or `"active"` when
///     the name isn't a known local effect.
pub fn build_vfx_head(name: &str) -> String {
    let cfg = crate::mappings::api_commands();
    if let Some(constant) = cfg.global_vfx_map.get(name) {
        format!(
            "spriteContent: \"global::vfx.vfx\", animation: GlobalVfx.{c}",
            c = constant,
        )
    } else {
        let anim = EFFECT_PRIMARY_ANIMS.with(|cell| {
            cell.borrow().get(name).cloned().unwrap_or_else(|| "active".to_string())
        });
        format!(
            "spriteContent: self.getResource().getContent(\"{n}\"), animation: \"{a}\"",
            n = name, a = anim,
        )
    }
}

/// Split a `{ key1: val1, key2: val2, … }` body (without the braces) into
/// `(key, value)` pairs, respecting parentheses so values like
/// `Random.getFloat(0, 1)` aren't split on the inner comma. Whitespace
/// around keys and values is trimmed.
fn parse_prop_bag(body: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut depth: i32 = 0;
    let mut cur = String::new();
    for ch in body.chars() {
        match ch {
            '(' | '[' => { depth += 1; cur.push(ch); }
            ')' | ']' => { depth -= 1; cur.push(ch); }
            ',' if depth == 0 => {
                if let Some((k, v)) = cur.split_once(':') {
                    let k = k.trim().to_string();
                    let v = v.trim().to_string();
                    if !k.is_empty() && !v.is_empty() {
                        out.push((k, v));
                    }
                }
                cur.clear();
            }
            _ => cur.push(ch),
        }
    }
    if !cur.trim().is_empty() {
        if let Some((k, v)) = cur.split_once(':') {
            let k = k.trim().to_string();
            let v = v.trim().to_string();
            if !k.is_empty() && !v.is_empty() {
                out.push((k, v));
            }
        }
    }
    out
}

/// Route one SSF2 prop through `attach_effect_props` in commands.jsonc.
/// On a successful mapping, push `(fm_field, value)` pairs into
/// `fm_fields`. On an explicit `todo` or an unknown key, push a
/// `// TODO:` note onto `todo_lines`. Both arrays are caller-owned so
/// the rewrite can emit them in source order.

/// Drop the source's own horizontal flip from an effect offset when the engine is already doing it.
///
/// SSF2 does not mirror an attached effect with its owner, so its scripts flip the offset by hand
/// (`x: self.flipX(78)`). Fraymakers DOES mirror it, via the `flipWith` that `parentLock` expands
/// to -- measured in-engine: with `flipWith: true` and a raw `x: 78`, the effect lands at +78
/// facing right and -78 facing left. Porting both flips means they cancel, and every effect with a
/// sideways offset appears on the wrong side of a character facing left.
///
/// So the flip is removed from the VALUE exactly when `flipWith` is set, and kept when it is not.
fn drop_redundant_flip(fm_fields: &mut [(String, String)]) {
    let engine_flips = fm_fields.iter().any(|(k, v)| k == "flipWith" && v.trim() != "false");
    if !engine_flips { return; }
    for (k, v) in fm_fields.iter_mut() {
        if k != "x" { continue; }
        if let Some(inner) = strip_call(v.trim(), "self.flipX(") {
            *v = inner;
        }
    }
}

/// `self.flipX(<expr>)` -> `<expr>`, respecting nesting. None when the text is not that call.
fn strip_call(text: &str, open: &str) -> Option<String> {
    let rest = text.strip_prefix(open)?;
    let mut depth = 1usize;
    for (i, ch) in rest.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    // only a bare call, not `self.flipX(1) + 2`
                    return rest[i + 1..].trim().is_empty().then(|| rest[..i].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

fn translate_attach_effect_prop(
    key: &str,
    value: &str,
    fm_fields: &mut Vec<(String, String)>,
    todo_lines: &mut Vec<String>,
) {
    use crate::mappings::AttachEffectPropMapping as M;
    let cfg = crate::mappings::api_commands();
    match cfg.attach_effect_props.get(key) {
        Some(M::Simple(fm)) => {
            fm_fields.push((fm.clone(), value.to_string()));
        }
        Some(M::Detailed { target, expand_to, todo }) => {
            if !expand_to.is_empty() {
                for fm in expand_to {
                    fm_fields.push((fm.clone(), value.to_string()));
                }
            } else if let Some(fm) = target {
                fm_fields.push((fm.clone(), value.to_string()));
            }
            if let Some(note) = todo {
                // Even routed entries can carry a porter-facing caveat
                // (matches call_splits's todo semantics).
                todo_lines.push(format!("{}: {} — {}", key, value, note));
            } else if expand_to.is_empty() && target.is_none() {
                // Entry exists but routes nowhere → treat as unmapped.
                todo_lines.push(format!("{}: {} — entry present but no target/expand_to", key, value));
            }
        }
        None => {
            todo_lines.push(format!("{}: {} — no mapping in attach_effect_props", key, value));
        }
    }
}

/// Strip `self.endAnimation()` calls that appear alone on a line inside the
/// last-numbered frame function of each animation group.
///
/// Pattern: a function named `<anim>__frame<N>` where no higher-numbered frame
/// function exists for that animation in the same script. In that case endAnimation
/// is redundant — FM already ends the animation when the final frame completes.
pub fn strip_last_frame_end_animation(code: &str) -> String {
    // Collect all (anim_prefix, max_frame_num) seen across all functions
    let mut max_frames: std::collections::BTreeMap<String, u32> = std::collections::BTreeMap::new();
    for (prefix, frame_num) in iter_frame_fns(code) {
        let entry = max_frames.entry(prefix.to_string()).or_insert(0);
        if frame_num > *entry {
            *entry = frame_num;
        }
    }

    // Now process line by line: when inside a last-frame function and we see
    // a line that is ONLY `self.endAnimation();`, replace with a comment.
    let lines: Vec<&str> = code.lines().collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut in_last_frame = false;

    for line in &lines {
        let trimmed = line.trim();
        // Check if this line opens a last-frame function
        if let Some((prefix, frame_num)) = parse_frame_fn_header(trimmed) {
            in_last_frame = max_frames.get(&prefix).copied() == Some(frame_num);
        } else if trimmed == "}" {
            // Leaving any function
            in_last_frame = false;
        }

        if in_last_frame && trimmed == "self.endAnimation();" {
            // Strip it — FM animation ends naturally on the last frame
            out.push(format!(
                "{}// [FM] endAnimation removed — redundant on last frame",
                &line[..line.len() - trimmed.len()]
            ));
        } else {
            out.push(line.to_string());
        }
    }

    let mut joined = out.join("\n");
    if code.ends_with('\n') { joined.push('\n'); }
    joined
}

// Simple &str-based frame function pattern matching (no regex dep).

/// Iterate `function <prefix>__frame<N>(…)` headers in a block of code,
/// yielding `(prefix, N)` for each.
fn iter_frame_fns(code: &str) -> impl Iterator<Item = (String, u32)> + '_ {
    code.lines().filter_map(|line| parse_frame_fn_header(line.trim()))
}

/// Parse a single trimmed line. Returns `Some((prefix, N))` if the line
/// is a `function <prefix>__frame<N>(…)` header, `None` otherwise.
fn parse_frame_fn_header(trimmed: &str) -> Option<(String, u32)> {
    let rest = trimmed.strip_prefix("function ")?;
    let paren = rest.find('(')?;
    let name = &rest[..paren];
    let frame_pos = name.rfind("__frame")?;
    let prefix = name[..frame_pos].to_string();
    let frame_num: u32 = name[frame_pos + "__frame".len()..].parse().ok()?;
    Some((prefix, frame_num))
}

/// Fix `setIntangibility` pairs across the full assembled Script.hx.
///
/// SSF2 pattern:
///   `anim__frame1`:  self.setIntangibility(true);
///   `anim__frame20`: self.setIntangibility(false);
///
/// FM equivalent: a single `applyGlobalBodyStatus(BodyStatus.INTANGIBLE, N)` call
/// where N = (false_frame - true_frame). The false call is removed entirely.
///
/// This is a full-script pass (not per-frame-body) because we need to look
/// across multiple frame functions to compute the duration.
/// Wrap a state transition handed over as an event CALLBACK.
///
/// A transition is normally CALLED (`self.toHeavyLand()`) and the tables rewrite it to the
/// Fraymakers state. Handed to an event listener it is a bare REFERENCE with no parentheses, so
/// no rewrite matched and what reached the engine was a member that does not exist -- a null
/// callback. mario's up special listens for the floor with one, so the move never resolved its
/// landing.
///
/// A callback has to be a function, so the mapped call goes inside one.
fn wrap_transition_callbacks(code: &str) -> String {
    let mut out = code.to_string();
    for r in &crate::mappings::api_commands().replacements {
        let Some(name) = r.from.strip_prefix('.').and_then(|n| n.strip_suffix("()")) else { continue };
        if !r.to.contains(".toState(") || name.is_empty() { continue }
        // the call without the table's trailing note, which has no place inside a closure
        let call = r.to.split("/*").next().unwrap_or(&r.to).trim().to_string();
        let needle = format!(".{name}");
        let mut scan = 0usize;
        while let Some(i) = out[scan..].find(&needle) {
            let at = scan + i;
            let after = at + needle.len();
            let is_ref = out[after..].chars().next().is_none_or(|c| c != '(')
                && !out[after..].starts_with(|c: char| c.is_alphanumeric() || c == '_');
            // the receiver: the identifier immediately before the dot
            let start = out[..at].rfind(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.'))
                .map(|k| k + 1).unwrap_or(0);
            if is_ref && start < at {
                let recv = &out[start..at];
                let wrapped = format!("function() {{ {recv}{call}; }}");
                out = format!("{}{}{}", &out[..start], wrapped, &out[after..]);
                scan = start + wrapped.len();
            } else {
                scan = after;
            }
        }
    }
    out
}

/// Where a field's value ends inside an object literal: the next `,` or `}` that
/// is not nested inside a call, a collection, a string or a comment. A value is
/// often a call with arguments of its own, and cutting at its first comma leaves
/// half a call behind.
fn value_end(after: &str) -> usize {
    let b = after.as_bytes();
    let mut depth: i32 = 0;
    let mut in_str: Option<u8> = None;
    let mut i = 0usize;
    while i < b.len() {
        let c = b[i];
        if let Some(q) = in_str {
            if c == b'\\' { i += 2; continue }
            if c == q { in_str = None }
        } else if c == b'/' && i + 1 < b.len() && b[i + 1] == b'/' {
            while i < b.len() && b[i] != b'\n' { i += 1 }
            continue;
        } else if c == b'/' && i + 1 < b.len() && b[i + 1] == b'*' {
            i += 2;
            while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') { i += 1 }
            i = (i + 2).min(b.len());
            continue;
        } else {
            match c {
                b'"' | b'\'' => in_str = Some(c),
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' => depth -= 1,
                b'}' if depth == 0 => return i,
                b'}' => depth -= 1,
                b',' if depth == 0 => return i,
                _ => {}
            }
        }
        i += 1;
    }
    after.len()
}

/// Every stat name written in a runtime hitbox update in this code.
fn hitbox_stat_names_written(code: &str) -> Vec<String> {
    stat_names_written(code, "updateHitboxStats(")
}

/// Every field name written in an object handed to `call` in this code.
fn stat_names_written(code: &str, call: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let mut scan = 0usize;
    while let Some(rel) = code[scan..].find(call) {
        let at = scan + rel;
        let open = at + call.len();
        scan = open;
        // only an object handed to THIS call: a call handed a name (`updateHitboxStats(1, stats)`)
        // writes nothing this can read, and the next brace in the code belongs to something else
        // (a point two lines on, whose `x` and `y` were then struck out of every literal)
        let Some(close) = call_close(&code[open..]) else { continue };
        let args = &code[open..open + close];
        let Some(rel_brace) = split_top_level(args).iter().find(|a| a.trim_start().starts_with('{'))
            .and_then(|a| args.find(a.trim_start())) else { continue };
        let body_start = open + rel_brace + 1;
        let end = body_start + value_end_of_object(&code[body_start..]);
        for part in split_top_level(&code[body_start..end]) {
            if let Some((k, _)) = part.split_once(':') {
                let k = k.trim();
                if !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                    && !names.iter().any(|n| n == k)
                {
                    names.push(k.to_string());
                }
            }
        }
    }
    names
}

/// Length of an object literal's body, from just after its `{` to its matching `}`.
fn value_end_of_object(after: &str) -> usize {
    let b = after.as_bytes();
    let mut i = 0usize;
    loop {
        let step = value_end(&after[i..]);
        i += step;
        if i >= b.len() { return b.len() }
        if b[i] == b'}' { return i }
        i += 1; // step over the separating comma
    }
}

/// Split an object literal's body on its top-level commas.
fn split_top_level(body: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut i = 0usize;
    while i < body.len() {
        let step = value_end(&body[i..]);
        parts.push(&body[i..i + step]);
        i += step + 1;
    }
    parts
}

/// Drop a hitbox field the engine has no counterpart for from a RUNTIME write.
///
/// The engine rejects the WHOLE object for one name it does not know ("Invalid hitbox stat"), so a
/// source-only field takes the entire update overboard with it: mario's forward aerial states a
/// camera shake on its hitbox and lost its damage, angle and knockback along with it. Each of
/// these was asked of the engine BY NAME and refused, so they are dropped rather than guessed at.
/// Where the thing being read starts, walking back from the call that reads it.
///
/// A receiver is a chain, and a link in it can be a call of its own: the whole of
/// `self.getOwner().` belongs to the read that follows it, and stopping at the `)`
/// leaves the front half sitting next to whatever replaces the read.
/// SSF2 grabs from the other side: the FOE is the receiver and the grabber is handed in, by
/// uid. this engine states it the way it reads -- the grabber attempts, and the foe is what it
/// attempts on -- so the two swap places and the uid becomes the object it names.
///
/// fourteen of these across ten characters, and each one is a grab that throws where it stands:
/// kirby inhales and the rest of the handler never runs.
/// A vfx is not a game object, and this engine will not pretend otherwise: handing one to
/// `sendBehind` throws where it stands. But the thing the source wanted -- draw this behind me
/// -- is something a vfx states for itself, on the stats it is made with. So the request moves
/// onto the vfx and the call it could never satisfy goes away.
///
/// sixteen of these across five characters, each one an effect that threw instead of drawing.
/// "the projectile I have out right now". SSF2 asks the fighter and gets an answer; this engine
/// keeps one list, of every projectile in the match, so the same question is answered by walking
/// it back to the newest one that belongs to whoever asked.
///
/// eighty-nine of these across eighteen characters, every one a call on nothing: blackmage warps
/// and the six frames that steer the warp all die on the same missing answer.
fn rewrite_current_projectile(code: &str) -> String {
    const CALL: &str = ".getCurrentProjectile()";
    let mut out = String::with_capacity(code.len());
    let mut rest = code;
    while let Some(at) = rest.find(CALL) {
        let start = receiver_start(rest, at);
        let who = &rest[start..at];
        if who.is_empty() {
            out.push_str(&rest[..at + CALL.len()]);
        } else {
            out.push_str(&rest[..start]);
            out.push_str(&format!("_currentProjectile({who})"));
        }
        rest = &rest[at + CALL.len()..];
    }
    out.push_str(rest);
    out
}

/// A call the target has no answer for, wrapped around something it still has to do. SSF2 asks
/// for an effect to be drawn in the character's own colours; here a vfx takes the palette of the
/// sprite it is made from, so there is nothing to ask -- but the effect still has to be MADE,
/// and leaving the request in place is a call on nothing that takes the effect down with it.
///
/// nineteen of these, and falco's phantasm is three of them: the blur never drew.
/// A name the source uses for a method, turned into a state wrapper because something else in
/// the class used it for a field. The wrapper's value is not a function, so calling it is a call
/// on nothing -- and it is the CALL that says which one the source meant.
///
/// eighty-five of these; forty-eight are one question, "am I facing right", asked in the middle
/// of moves that then stop.
/// A global whose NAME is worked out as it runs. The literal ones become persistent state, one
/// slot per name, but a name built at runtime ("dashX" + n) has no slot to be. This engine keeps
/// one shared object on the match and takes literal fields on it, and arrays work, so the pair of
/// them is a keyed store every entity can reach -- which is what "global" meant.
///
/// fox writes a trail of positions this way and its forward special reads them back; without it
/// the character wrote nowhere and the projectile read nothing.
/// Walking a pair of arrays, which is what a table read by a runtime name has to be here.
/// Walking the match's list back to the newest projectile someone owns.
/// Playing a label only when what was handed over IS one.
/// A jump to a label from a method, which runs under whatever animation is playing: inside it
/// when the animation carries the label, else to the animation the label starts (link's bow
/// jumps to `attack` inside its own clip; its charged smash leaves the charge for the piece
/// emitted under that name).
/// The clip's frame in the SOURCE's units. The source asks its fighter which frame its pose
/// clip is on and compares the number to frames it counted at its own rate (sonic's homing
/// attack releases on frame 26); this engine counts twice as many frames for the same pose, and
/// numbers them from one. Read here, every frame the source compares against keeps its meaning.
pub(crate) const SRC_FRAME: &str = "\
function __srcFrame() {
\treturn Std.int((self.getCurrentFrame() - 1) / 2) + 1;
}
function __srcTotalFrames() {
\treturn Std.int((self.getTotalFrames() + 1) / 2);
}
";

/// `<recv>.getCurrentFrame()` on the fighter is a read in the source's units (see `SRC_FRAME`).
pub fn rewrite_current_frame_reads(code: &str) -> String {
    static BARE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let bare = BARE.get_or_init(|| regex::Regex::new(r"(^|[^\w.])currentFrame\b").expect("bare currentFrame regex"));
    let out = code.replace("self.self.getCurrentFrame()", "__srcFrame()").replace("self.getCurrentFrame()", "__srcFrame()");
    // the bare identifier inside a clip's own frame script is the clip's own frame (fox's
    // reflector holds `curFrame` and compares it to `currentFrame` on its loop's first frame)
    bare.replace_all(&out, "${1}__srcFrame()").into_owned()
}

/// A number compared against nothing compares false, as the source's `undefined` does. A field
/// the fighter has not written yet reads as null here, and a comparison on it decided the other
/// way: samus's charge shot read a null charge as already full and stowed itself.
pub(crate) const NULL_COMPARE: &str = "\
function __ge(a, b) { return a != null && b != null && a >= b; }
function __gt(a, b) { return a != null && b != null && a > b; }
function __le(a, b) { return a != null && b != null && a <= b; }
function __lt(a, b) { return a != null && b != null && a < b; }
function __cb(f) { return f != null ? f : function() {}; }
// a whole-number field keeps the whole part, cut toward zero, and nothing or not-a-number is 0.
// what the port keeps where the source kept a number (a sound's handle for its id) goes in as is
function __int(v) { if (v == null) { return 0; } if (!Std.isOfType(v, Float)) { return v; } if (v - v != 0) { return 0; } return Std.int(v); }

// a slice of a string as much as of a list, as the source takes one (negative ends count back)
function __slice(o, ?a, ?b) { if (o == null) { return null; } if (!Std.isOfType(o, String)) { return o.slice(a, b); } var n = o.length; var st = (a == null) ? 0 : ((a < 0) ? n + a : a); var en = (b == null) ? n : ((b < 0) ? n + b : b); if (st < 0) { st = 0; } if (en > n) { en = n; } return (en > st) ? o.substr(st, en - st) : \"\"; }
// what the source asks as taking hits and knockback: a fighter does, a thing it made does not
// an effect put on the list its fighter clears (the fighter's own `addEffectToList` takes it
// when there is one, and the list is how the source takes its effects off when the move ends)
function __addFx(v) { return v; }
// the one it holds jumps to a hurt pose of its own by label: asked of its script, which takes it
// in its own clip and does nothing when the clip has no such label (a fighter that publishes
// nothing has no labels to ask for)
function __grabbedHurtFrame(l) { if (self.getGrabbedFoe == null) { return; } var g = self.getGrabbedFoe(); if (g != null && g.exports != null && g.exports.__linkageId != null) { g.exports.__askLabel = l; } }
// the fighter drawn in front of the one it holds, or behind it
function __swapWithGrabbed(?front) { if (self.getGrabbedFoe == null) { return; } var g = self.getGrabbedFoe(); if (g == null) { return; } if (front == true) { self.bringInFront(g); } else { self.sendBehind(g); } }
// a colour setting the engine can show: an adjustment is a filter (the source's -100..100 are
// fractions here, its hue is degrees as here), a fade is the alpha; null or false takes them off
var __cfT = [];
var __cfF = [];
function __colorFilters(t, o) { if (t == null) { return; } var i = __cfT.indexOf(t); if (i >= 0) { if (__cfF[i] != null) { t.removeFilter(__cfF[i]); } __cfT.splice(i, 1); __cfF.splice(i, 1); } if (o == null || o == false) { t.setAlpha(1); return; } if (o.alphaMultiplier != null) { t.setAlpha(o.alphaMultiplier); } if (o.hue != null || o.saturation != null || o.brightness != null || o.contrast != null) { var h = new HsbcColorFilter(); if (o.hue != null) { h.hue = o.hue; } if (o.saturation != null) { h.saturation = o.saturation / 100; } if (o.brightness != null) { h.brightness = o.brightness / 100; } if (o.contrast != null) { h.contrast = o.contrast / 100; } t.addFilter(h); __cfT.push(t); __cfF.push(h); } }
// a timer the source starts: kept by the engine across animation changes, taken off here at the
// state change unless the source asked it to persist
// (each remembers the animation it was started under: the state change that ends a state lands
// after the next state's first frame has run, and the timers that frame starts are the new
// state's own, not the old one's to take)
// WHEN A TIMER FIRES. the source's timer of N frames runs before the update of its Nth frame; this
// engine's runs after an update (measured: booked during a frame's update or scripts, a delay of
// d fires after d - 1 more updates; booked from a timer's callback, after d). so a source timer
// is a record fired off a due list that every update drains first: booked from a script, its
// first hop is one engine frame short of its doubled delay, which lands after the update before
// the one the source fires in; and each fire, from inside the drain, books the next a whole
// period on. removed by the record, whichever hop is booked (samus's missile took its launch
// speed as its homing speed a source frame late, and spent its whole flight two units slow;
// link's boomerang slowed every engine frame)
// (where the timer is booked decides the first hop: at spawn the object's first update is still
// to come in this frame, so one frame less; from a script, one frame short of the doubled delay;
// from inside the drain, a whole period)
var __srcTimerIds = [];
var __srcTimerAll = [];
var __srcDue = [];
var __srcSpawning = true;
var __srcInDrain = false;
function __srcHop(r, d) { if (d < 1) { __srcDue.push(r); return; } r.id = self.addTimer(d, 1, function() { r.id = null; __srcDue.push(r); }, r.o); }
// (one timer per callback, as the source keeps them: a frame that starts its timer again each
// time it plays replaces the running one there, and stacked them here, so link's boomerang
// slowed twice as fast)
function __srcTimer(a, b, f, ?o) { var keep = o != null && o.persistent == true; if (o == null) { o = { persistent: true }; } else { o.persistent = true; } var j = 0; while (j < __srcTimerAll.length) { if (__srcTimerAll[j].f == f) { __rmTimer(__srcTimerAll[j]); } else { j = j + 1; } } var r = { f: f, a: a, b: b, o: o, id: null, dead: false, anim: self.getAnimation() }; __srcHop(r, __srcInDrain ? a : (__srcSpawning ? a - 2 : a - 1)); if (!keep) { __srcTimerIds.push(r); } __srcTimerAll.push(r); return r; }
function __drainSrcTimers() { __srcSpawning = false; if (__srcDue.length == 0) { return; } var l = __srcDue.splice(0, __srcDue.length); var i = 0; __srcInDrain = true; while (i < l.length) { var r = l[i]; i = i + 1; if (r.dead) { continue; } if (r.b != -1) { r.b = r.b - 1; } if (r.b == 0) { r.dead = true; __srcTimerIds.remove(r); __srcTimerAll.remove(r); } else { __srcHop(r, r.a); } r.f(); } __srcInDrain = false; }
function __rmTimer(x) { if (x == null) { return false; } if (Std.isOfType(x, Int) || Std.isOfType(x, Float)) { return self.removeTimer(x); } var rec = false; try { rec = x.dead != null; } catch (err:Dynamic) { rec = false; } if (!rec) { return __removeTimerF(self, x); } if (x.dead == true) { return false; } x.dead = true; if (x.id != null) { self.removeTimer(x.id); x.id = null; } __srcTimerIds.remove(x); __srcTimerAll.remove(x); return true; }
function __flushSrcTimers(?e) { var now = self.getAnimation(); var l = []; var i = 0; while (i < __srcTimerIds.length) { if (__srcTimerIds[i].anim != now) { l.push(__srcTimerIds[i]); } i = i + 1; } i = 0; while (i < l.length) { __rmTimer(l[i]); i = i + 1; } }
// ...and every one of them when the object is taken off at once: a timer kept past its object's end
// is ticked on nothing
function __endSrcTimers() { var l = __srcTimerAll.splice(0, __srcTimerAll.length); __srcTimerIds.splice(0, __srcTimerIds.length); __srcDue.splice(0, __srcDue.length); var i = 0; while (i < l.length) { l[i].dead = true; if (l[i].id != null) { self.removeTimer(l[i].id); l[i].id = null; } i = i + 1; } }
// trading places in the draw order with a thing beside the fighter
var __behindVfx = [];\nfunction __swapWithSelf(o) { if (o == null || o == self) { return; } if (o.getType() != EntityType.VFX) { self.swapDepths(o); return; } var k = __behindVfx.indexOf(o); if (k >= 0) { __behindVfx.splice(k, 1); stage.getCharactersFrontContainer().addChild(o.getViewRootContainer()); } else { __behindVfx.push(o); stage.getCharactersBackContainer().addChild(o.getViewRootContainer()); } }
// the id a fighter's stats state it by, as its script publishes it (a thing that is no fighter
// of this package states none)
function __statsNameOf(o) { if (o != null && o.exports != null && o.exports.__statsName != null) { return o.exports.__statsName; } return \"\"; }
// turned to face the other way, by the way it faces (a projectile's api has no flip of its own)
function __flip(o) { if (o == null) { return; } if (o.isFacingRight()) { o.faceLeft(); } else { o.faceRight(); } }
function __isCharacter(o) { return o != null && o.getType != null && o.getType() == EntityType.CHARACTER; }
// another object ended as the source asks it: a converted projectile ends itself in its own update
// (it publishes the id it was made from), anything else is killed
function __endOther(o) { if (o == null) { return; } if (o.exports != null && o.exports.__linkageId != null) { o.exports.__askEnd = true; } else if (o.kill != null) { o.kill(); } }
// a collection the source keys by name, as its arrays and objects allow: pairs kept beside it here,
// found by the collection itself
var __maps = [];
function __mapOf(o) { var i = 0; while (i < __maps.length) { if (__maps[i].o == o) { return __maps[i]; } i = i + 1; } var m = { o: o, k: [], v: [] }; __maps.push(m); return m; }
function __mapSet(o, k, v) { if (o == null) { return v; } var m = __mapOf(o); var i = m.k.indexOf(k); if (i >= 0) { m.v[i] = v; } else { m.k.push(k); m.v.push(v); } return v; }
function __mapGet(o, k) { if (o == null) { return null; } var m = __mapOf(o); var i = m.k.indexOf(k); return i >= 0 ? m.v[i] : null; }
// a rectangle of this engine's, in the source's units, the way the script keeps its positions
// an area's rectangle is its own, placed at the area's origin: world coordinates are the two added
function __srcRect(a) { if (a == null) { return null; } var r = a.getRectangle(); if (r == null) { return null; } var x = __isp(a.getX() + r.x); var y = __isp(a.getY() + r.y); var w = __isp(r.width); var h = __isp(r.height); return { x: x, y: y, width: w, height: h, left: x, top: y, right: x + w, bottom: y + h }; }
// what a call nobody answers was handed, evaluated for what making it does, and answered with nothing
function __nothing(a) { return null; }
// a timer taken off by the function it runs, as the source does it: this engine takes one off by the
// number it handed back when it was made, so the number is kept against the function
var __tmF = [];
var __tmI = [];
// what this has put on other objects, taken off when it ends (a projectile's script goes with it)
var __others = [];
// (the handler goes on wrapped: the engine can take this object off with nothing of its own run
// first, and a handler left on another object then runs in a scope that is gone, on every event
// for the rest of the match; the wrapper takes itself off the first time it finds that)
function __listenOther(o, e, f) { if (o == null) { return; } var r = { o: o, e: e, f: f, w: null }; r.w = function(ev) { var gone = true; try { gone = self.isDisposed(); } catch (err:Dynamic) { gone = true; } if (gone) { if (r.o != null && !r.o.isDisposed()) { r.o.removeEventListener(r.e, r.w); } return; } f(ev); }; __others.push(r); o.addEventListener(e, r.w, { persistent: true }); }
function __unlistenOther(o, e, f) { var i = 0; while (i < __others.length) { var r = __others[i]; if (r.o == o && r.e == e && r.f == f) { __others.splice(i, 1); if (r.o != null && !r.o.isDisposed()) { r.o.removeEventListener(r.e, r.w); } return; } i = i + 1; } if (o != null && !o.isDisposed()) { o.removeEventListener(e, f); } }
function __unlistenOthers() { var l = __others.splice(0, __others.length); var i = 0; while (i < l.length) { var r = l[i]; if (r.o != null && !r.o.isDisposed()) { r.o.removeEventListener(r.e, r.w); } i = i + 1; } }
function __addTimerF(o, a, b, f, ?opt) { var id = o == self ? __srcTimer(a, b, f, opt) : (opt == null ? o.addTimer(a, b, f) : o.addTimer(a, b, f, opt)); __tmF.push(f); __tmI.push(id); return id; }
function __removeTimerF(o, x) { var i = __tmF.indexOf(x); if (i >= 0) { var id = __tmI[i]; __tmF.splice(i, 1); __tmI.splice(i, 1); return o == self ? __rmTimer(id) : o.removeTimer(id); } if (!(Std.isOfType(x, Int) || Std.isOfType(x, Float))) { return false; } return o == self ? __rmTimer(x) : o.removeTimer(x); }
// the angle from one point to another, in degrees within one turn, the way the source works it
// out (its vertical term included as it is written there), from anything with an x and a y
function __angleBetween(a, b) { var d = Math.atan2(-b.y - a.y, b.x - a.x) * 180 / Math.PI; while (d < 0) { d = d + 360; } while (d >= 360) { d = d - 360; } return d; }
// the one function standing in for a handler named after one of the source engine's methods
var __apiFns = [];
function __apiFn(n, f) { var i = 0; while (i < __apiFns.length) { if (__apiFns[i] == n) { return __apiFns[i + 1]; } i = i + 2; } __apiFns.push(n); __apiFns.push(f); return f; }
// what an object says it is: the id its declaration carries, published by every script here
function __linkageOf(o) { if (o == null) { return null; } if (o == self) { return __ownLinkage; } if (o.exports != null && o.exports.__linkageId != null) { return o.exports.__linkageId; } return null; }
";

/// `<field>.get() >= <operand>` and its kin read through the helpers above, so a field that
/// holds nothing compares false rather than however the engine orders null against a number.
pub fn guard_null_comparisons(code: &str) -> String {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    // the fighter's own fields, before they become persistent reads (`self.curCharge` here,
    // `curCharge.get()` once the script is assembled)
    // each operand carries any trailing array index with it (`holdLengths[getAuraLevel()]`), so the
    // whole indexed value goes inside the __ge/__lt helper -- without this the index was left
    // outside (`__ge(a, holdLengths)[getAuraLevel()]`), indexing the bool the helper returns and
    // crashing with "cast bool to Array" (lucario's aura-charge frame compare).
    // and a right-hand operand keeps its whole member chain: `self.t < self.teammates.length` left
    // `.length` outside, reading the length of a bool (bomberman's team-bomb loop)
    let re = RE.get_or_init(|| regex::Regex::new(
        r"(self\.[A-Za-z_]\w*(?:\[[^\]]*\])*)\s*(>=|<=|>|<)\s*(self\.[A-Za-z_]\w*(?:\.[A-Za-z_]\w*|\(\)|\[[^\]]*\])*|[A-Za-z_][\w.]*(?:\(\))?(?:\[[^\]]*\])*|-?\d+(?:\.\d+)?)"
    ).expect("null compare regex"));
    re.replace_all(code, |c: &regex::Captures| {
        // an operand that goes on past what was taken (a call with arguments) is left as written
        // rather than cut in two
        if matches!(code[c.get(0).unwrap().end()..].chars().next(), Some('(' | '.' | '[')) {
            return c[0].to_string();
        }
        let f = match &c[2] { ">=" => "__ge", "<=" => "__le", ">" => "__gt", _ => "__lt" };
        format!("{f}({}, {})", &c[1], &c[3])
    }).into_owned()
}

/// `<recv>.stancePlayFrame("L")` in a method, for a label some animation carries: resolved as
/// the animation plays. Runs before the translation, which would read
/// every such jump as a change of animation.
pub fn rewrite_clip_label_jumps(code: &str) -> String {
    const CALL: &str = ".stancePlayFrame(\"";
    let mut out = String::with_capacity(code.len());
    let mut rest = code;
    while let Some(at) = rest.find(CALL) {
        let open = at + CALL.len();
        let Some(e) = rest[open..].find('"') else { break };
        let label = &rest[open..open + e];
        let after = &rest[open + e + 1..];
        let known = CURRENT_LABELS.with(|c| c.borrow().contains(label));
        if !known || !after.starts_with(')') {
            out.push_str(&rest[..open]);
            rest = &rest[open..];
            continue;
        }
        let start = receiver_start(rest, at);
        let who = &rest[start..at];
        let fallback = LABEL_ANIMS.with(|l| l.borrow().get(label).cloned());
        out.push_str(&rest[..start]);
        out.push_str(&format!("_jumpLabel({who}, \"{label}\", {})",
            fallback.map(|f| format!("\"{f}\"")).unwrap_or_else(|| "null".to_string())));
        rest = &after[1..];
    }
    out.push_str(rest);
    out
}

/// `<recv>.getAttackStat("K")` reads the attack table. Runs before the
/// translation, which has no answer for the call and strikes it.
pub fn rewrite_attack_stat_reads(code: &str) -> String {
    const CALL: &str = ".getAttackStat(\"";
    let mut out = String::with_capacity(code.len());
    let mut rest = code;
    while let Some(at) = rest.find(CALL) {
        let open = at + CALL.len();
        let Some(e) = rest[open..].find('"') else { break };
        let key = &rest[open..open + e];
        let after = &rest[open + e + 1..];
        if !after.starts_with(')') {
            out.push_str(&rest[..open]);
            rest = &rest[open..];
            continue;
        }
        let start = receiver_start(rest, at);
        let who = &rest[start..at];
        let who = if who == "self.self" { "self" } else { who };
        out.push_str(&rest[..start]);
        out.push_str(&format!("_attackStat({who}, \"{key}\")"));
        rest = &after[1..];
    }
    out.push_str(rest);
    out
}

pub(crate) const CURRENT_PROJECTILE: &str = "\
function _currentProjectile(who) {
\tvar all = match.getProjectiles();
\tif (all == null) { return null; }
\tvar __i = all.length - 1;
\twhile (__i >= 0) {
\t\tif (all[__i].getOwner() == who) { return all[__i]; }
\t\t__i = __i - 1;
\t}
\treturn null;
}
";

pub(crate) const TABLE_PICK: &str = "\
function _pick(keys, vals, k) {
\tvar __i = 0;
\twhile (__i < keys.length) {
\t\tif (keys[__i] == k) { return vals[__i]; }
\t\t__i = __i + 1;
\t}
\treturn null;
}
";

pub(crate) const GLOBAL_STORE: &str = "\
function _globalSlot(k) {
\tif (match.globals.peptideKeys == null) {
\t\tmatch.globals.peptideKeys = [];
\t\tmatch.globals.peptideVals = [];
\t}
\tvar __i = 0;
\twhile (__i < match.globals.peptideKeys.length) {
\t\tif (match.globals.peptideKeys[__i] == k) { return __i; }
\t\t__i = __i + 1;
\t}
\tmatch.globals.peptideKeys.push(k);
\tmatch.globals.peptideVals.push(null);
\treturn match.globals.peptideKeys.length - 1;
}

function _setGlobal(k, v) {
\tvar __i = _globalSlot(k);
\tmatch.globals.peptideVals[__i] = v;
}

function _getGlobal(k) {
\tvar __i = _globalSlot(k);
\treturn match.globals.peptideVals[__i];
}
";

fn rewrite_runtime_named_globals(code: &str) -> String {
    let mut out = code.to_string();
    for (call, to) in [(".setGlobalVariable(", "_setGlobal("), (".getGlobalVariable(", "_getGlobal(")] {
        let mut acc = String::with_capacity(out.len());
        let mut rest = out.as_str();
        while let Some(at) = rest.find(call) {
            let after = &rest[at + call.len()..];
            // a name spelled out in full still gets a slot of its own; this is only for the
            // ones that are not known until they are worked out.
            let spelled_out = after.starts_with('"')
                && after[1..].find('"').is_some_and(|e| {
                    let next = after[1 + e + 1..].trim_start();
                    next.starts_with(',') || next.starts_with(')')
                });
            if spelled_out {
                acc.push_str(&rest[..at + call.len()]);
                rest = after;
                continue;
            }
            let start = receiver_start(rest, at);
            acc.push_str(&rest[..start]);
            acc.push_str(to);
            rest = after;
        }
        acc.push_str(rest);
        out = acc;
    }
    out
}

/// WRITING ANOTHER OBJECT'S FIELD UNDER A NAME THIS CHARACTER ALREADY KEEPS. In the source the
/// character hands a value to the thing it fired by setting that thing's field, and the thing
/// reads its own. Here the thing reads the SUMMONER's -- that is how a name crosses now -- so
/// the value has to be in the summoner's slot, and writing it into the other object's leaves the
/// slot the reader actually reads untouched. Which is also a write to a field the other object
/// never declared, so it fails where it stands.
/// READING THE SUMMONER'S STATE AS A PLAIN FIELD. The source is standing on the object that has
/// it, so it just names it -- `char.dotCount` -- and the wrapper forms are only what the writing
/// side happens to look like. A plain read reaches nothing, and the arithmetic built on it is
/// quietly wrong or throws: pacman counts the dots it has out this way.
pub fn read_owner_state_through_exports(code: &str, owner_vars: &[String]) -> String {
    let mut out = code.to_string();
    for v in owner_vars {
        let needle = format!(".{v}");
        let mut from = 0usize;
        while let Some(rel) = out[from..].find(&needle) {
            let at = from + rel;
            let after = &out[at + needle.len()..];
            let next_is_name = after.chars().next().is_some_and(|c| c.is_alphanumeric() || c == '_');
            let is_call = after.starts_with('(');
            let wrapped = after.starts_with(".get(") || after.starts_with(".set(");
            // a summoner slot already reached through `.exports` but NOT wrapped: an earlier pass
            // put the `.exports` on without the `.get()` an owner var needs to read as a value
            // (fox's side-special projectile: `character.exports.dashLim - 1`, an ApiVarObject the
            // engine cannot cast to a number). Add the `.get()` in a value read -- the char after
            // the name is an operator or a terminator, never a lone `=` (that is a write, already
            // lowered to `.set()` before this runs).
            if out[..at].ends_with(".exports") && !next_is_name && !is_call && !wrapped {
                let rest = after.trim_start();
                let value_read = rest.is_empty()
                    || rest.starts_with("==") || rest.starts_with("!=")
                    || rest.starts_with([')', ',', ';', ']', '}',
                        '-', '+', '*', '/', '%', '<', '>', '&', '|', '?', ':']);
                if value_read {
                    let ins = format!(".{v}.get()");
                    out = format!("{}{}{}", &out[..at], ins, after);
                    from = at + ins.len();
                    continue;
                }
                from = at + needle.len();
                continue;
            }
            // and never what an EVENT carries: `arg0.data.foe` is the event's, whatever the
            // character happens to call a slot of its own.
            let already = out[..at].ends_with(".exports")
                || out[..at].ends_with("self")
                || out[..at].ends_with(".data")
                || out[..at].ends_with("data");
            if next_is_name || already || is_call || wrapped {
                from = at + needle.len();
                continue;
            }
            // only where the source is reaching THROUGH something at all
            let before_ok = out[..at].chars().last().is_some_and(|c| c.is_alphanumeric() || c == '_' || c == ')');
            if !before_ok {
                from = at + needle.len();
                continue;
            }
            let ins = format!(".exports.{v}.get()");
            out = format!("{}{}{}", &out[..at], ins, &out[at + needle.len()..]);
            from = at + ins.len();
        }
    }
    out
}

/// READING THE SUMMONER'S GETTER AS A PLAIN FIELD, which is how the source reads one: the
/// summoner published the getter as a function, so the read is a call through it. A read
/// already a call, or already through the exports, is left alone; so is a name that is a
/// field of something else (`arg0.data.foe`).
pub fn read_owner_getter_through_exports(code: &str, getter: &str) -> String {
    let needle = format!(".{getter}");
    let mut out = String::with_capacity(code.len());
    let mut rest = code;
    while let Some(at) = rest.find(&needle) {
        let after = &rest[at + needle.len()..];
        let next_is_name = after.chars().next().is_some_and(|c| c.is_alphanumeric() || c == '_');
        let is_call = after.trim_start().starts_with('(');
        let before = &rest[..at];
        let through_exports = before.ends_with(".exports");
        let on_event = before.ends_with(".data") || before.ends_with("data");
        let on_self = before.ends_with("self") && !before.ends_with(".self");
        let receiver_ok = before.chars().last().is_some_and(|c| c.is_alphanumeric() || c == '_' || c == ')');
        out.push_str(before);
        if next_is_name || is_call || through_exports || on_event || on_self || !receiver_ok {
            out.push_str(&needle);
        } else {
            out.push_str(&format!(".exports.{getter}()"));
        }
        rest = after;
    }
    out.push_str(rest);
    out
}

/// Names the things a character summons WRITE ON IT. In the source they are fields on the one
/// object both share, so a projectile setting one is setting the character's own state -- and
/// here the character has to be keeping it, or there is nothing to write into and nothing for
/// the character to read back.
pub fn names_written_on_the_owner(projectile_code: &str) -> Vec<String> {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| {
        // set as a field on whoever summoned it, or by name through the call the source uses for
        // state it shares -- the second is the form most of them arrive in.
        // set as a field on whoever summoned it, or named through the call the source uses for
        // state it shares -- read or written, since a read means the summoner is expected to be
        // keeping it just as much as a write does.
        regex::Regex::new(r#"\b(?:character|char|owner)\.(\w+)\s*=|[gs]etGlobalVariable\("(\w+)"|\b(?:character|char|owner)\.(\w+)([^(\w]|$)"#).expect("regex")
    });
    let mut v: Vec<String> = Vec::new();
    for c in re.captures_iter(projectile_code) {
        let n = c.get(1).or_else(|| c.get(2)).or_else(|| c.get(3)).map(|m| m.as_str().to_string());
        if let Some(n) = n {
            if n != "exports" && !v.contains(&n) { v.push(n); }
        }
    }
    v
}

/// A CAST IS NOTHING HERE. The source names the type it expects so its own compiler will let the
/// call through; this engine's scripts are not typed that way and the value passes through
/// unchanged. Struck instead, the cast took the assignment it was part of with it -- zelda's
/// din's fire never learned who cast it, and every frame that asks what she is doing asks null.
fn unwrap_type_casts(code: &str) -> String {
    const CALL: &str = "SSF2Utils.cast(";
    let mut out = String::with_capacity(code.len());
    let mut rest = code;
    while let Some(at) = rest.find(CALL) {
        let open = at + CALL.len();
        let b = rest.as_bytes();
        let (mut i, mut depth) = (open, 1i32);
        while i < b.len() && depth > 0 {
            match b[i] { b'(' => depth += 1, b')' => depth -= 1, _ => {} }
            i += 1;
        }
        if depth != 0 { out.push_str(&rest[..open]); rest = &rest[open..]; continue }
        let close = i - 1;
        let inner = &rest[open..close];
        // the value is the first thing it names; the rest is the type it was naming for
        let (mut j, mut d2) = (0usize, 0i32);
        let ib = inner.as_bytes();
        while j < ib.len() {
            match ib[j] {
                b'(' | b'[' | b'{' => d2 += 1,
                b')' | b']' | b'}' => d2 -= 1,
                b',' if d2 == 0 => break,
                _ => {}
            }
            j += 1;
        }
        out.push_str(&rest[..at]);
        out.push_str(inner[..j].trim());
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    out
}

/// A CONJUNCT THE DECOMPILER LEFT STANDING ALONE. `a && b && c` comes back as an `if` per
/// conjunct whose body is that conjunct again -- `if (a) { a; }` -- which is code nobody writes
/// and which does nothing, so the test that was meant to GUARD what follows guards nothing.
/// blackmage asks whether its warp still exists that way and reads a slot the guard was there to
/// protect.
///
/// What `&&` does is leave early when a conjunct is false, so that is what it becomes.
/// A TEST WHOSE LEFT-HAND SIDE IS GONE. Something the target cannot answer was struck out of a
/// comparison and left `null` behind, so `null != "isaac_move_proj"` is always true and the
/// branch it guards always runs -- isaac clears the handles on the hand it just fired and then
/// reaches through them.
///
/// A test that cannot be answered is not answered: the branch does not run, which is the same
/// choice made everywhere else a condition has nothing to read.
pub fn resolve_tests_with_a_lost_side(code: &str) -> String {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r#"(?:\bnull\s*[!=]=\s*"[^"]*"|"[^"]*"\s*[!=]=\s*null\b)"#).expect("regex")
    });
    re.replace_all(code, "false").into_owned()
}

fn restore_lost_conjuncts(code: &str) -> String {
    let lines: Vec<&str> = code.lines().collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut i = 0usize;
    while i < lines.len() {
        let head = lines[i].trim();
        let is_if = head.starts_with("if (") && head.ends_with(") {");
        if is_if && i + 2 < lines.len() {
            let cond = &head[4..head.len() - 3];
            let body = lines[i + 1].trim().trim_end_matches(';');
            // the same test, allowing for one side having been wrapped as a truth check
            // already and the other not
            let norm = |t: &str| t.replace("__truthy(", "")
                .chars().filter(|c| !c.is_whitespace() && *c != ')').collect::<String>();
            if !cond.is_empty() && norm(body) == norm(cond) && lines[i + 2].trim() == "}" {
                let indent: String = lines[i].chars().take_while(|c| c.is_whitespace()).collect();
                out.push(format!("{indent}// [a conjunct of the test below, standing alone]"));
                out.push(format!("{indent}if (!__truthy({cond})) {{ return null; }}"));
                i += 3;
                continue;
            }
        }
        out.push(lines[i].to_string());
        i += 1;
    }
    out.join("\n")
}

fn unwrap_calls_with_no_answer(code: &str) -> String {
    const CALLS: [&str; 1] = [".applyPalette("];
    let mut out = code.to_string();
    for call in CALLS {
        let mut acc = String::with_capacity(out.len());
        let mut rest = out.as_str();
        while let Some(at) = rest.find(call) {
            let open = at + call.len();
            let b = rest.as_bytes();
            let mut depth = 1i32;
            let mut i = open;
            while i < b.len() && depth > 0 {
                match b[i] { b'(' => depth += 1, b')' => depth -= 1, _ => {} }
                i += 1;
            }
            if depth != 0 { acc.push_str(&rest[..open]); rest = &rest[open..]; continue }
            let close = i - 1;
            let start = receiver_start(rest, at);
            acc.push_str(&rest[..start]);
            acc.push_str(&rest[open..close]);
            rest = &rest[close + 1..];
        }
        acc.push_str(rest);
        out = acc;
    }
    out
}

fn rewrite_vfx_sent_behind(code: &str) -> String {
    const CALL: &str = ".sendBehind(";
    const MADE: &str = "match.createVfx(new VfxStats({";
    let mut out = String::with_capacity(code.len());
    let mut rest = code;
    while let Some(at) = rest.find(CALL) {
        let open = at + CALL.len();
        let b = rest.as_bytes();
        let mut depth = 1i32;
        let mut i = open;
        while i < b.len() && depth > 0 {
            match b[i] { b'(' => depth += 1, b')' => depth -= 1, _ => {} }
            i += 1;
        }
        if depth != 0 { out.push_str(&rest[..open]); rest = &rest[open..]; continue }
        let close = i - 1;
        let inner = &rest[open..close];
        match inner.find(MADE) {
            Some(m) if !inner[..m].contains(';') => {
                let start = receiver_start(rest, at);
                out.push_str(&rest[..start]);
                let brace = m + MADE.len();
                out.push_str(&inner[..brace]);
                out.push_str(" layer: VfxLayer.CHARACTERS_BACK,");
                out.push_str(&inner[brace..]);
            }
            _ => out.push_str(&rest[..close + 1]),
        }
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    out
}

fn rewrite_grab_calls(code: &str) -> String {
    const CALL: &str = ".grab(";
    let mut out = String::with_capacity(code.len());
    let mut rest = code;
    while let Some(at) = rest.find(CALL) {
        // the matching close paren, so a nested call inside the args does not end it early
        let open = at + CALL.len();
        let b = rest.as_bytes();
        let mut depth = 1i32;
        let mut i = open;
        while i < b.len() && depth > 0 {
            match b[i] { b'(' => depth += 1, b')' => depth -= 1, _ => {} }
            i += 1;
        }
        if depth != 0 {
            out.push_str(&rest[..open]);
            rest = &rest[open..];
            continue;
        }
        let close = i - 1;
        let start = receiver_start(rest, at);
        let foe = &rest[start..at];
        let args = &rest[at + CALL.len()..close];
        // whoever is named first is who grabs; a uid names the object it belongs to, and a
        // grab that names nobody is the script's own character grabbing.
        let first = args.split(',').next().unwrap_or("").trim();
        let grabber = first.strip_suffix(".getUid()").unwrap_or({
            if first.is_empty() || first.starts_with('-') || first.parse::<f64>().is_ok() {
                "self"
            } else {
                first
            }
        });
        if foe.is_empty() || grabber.is_empty() {
            out.push_str(&rest[..close + 1]);
        } else {
            out.push_str(&rest[..start]);
            out.push_str(&format!("{grabber}.attemptGrab({foe})"));
        }
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    out
}

/// A SPEED A SCRIPT STATES OR READS crosses the engine boundary. Its stats already do: a
/// declared walk speed is scaled for the doubled frame rate and the sprite size, so a script
/// that sets its own speed (a hop, a bolt's fall) in source pixels per source frame ran about
/// one and a half times too fast against everything the stats drive, and reached what it was
/// falling towards frames early. A write goes through `__sv` (source per-frame speed to engine
/// per-frame speed) and a read comes back through `__isv`, so the script's own arithmetic stays
/// in the units its author wrote it in, comparisons included. Zero needs no conversion, and a
/// value already wrapped is left alone.
pub(crate) fn speeds_cross_the_boundary(code: &str) -> String {
    let re = crate::re!(r"\.get([XY])Speed\(\)");
    let mut res = String::with_capacity(code.len() + 64);
    let mut cursor = 0usize;
    for m in re.captures_iter(code) {
        let whole = m.get(0).unwrap();
        if whole.start() < cursor { continue; }
        let rs = receiver_start(code, whole.start());
        if rs == whole.start() || rs < cursor { continue; }
        let before = &code[..rs];
        res.push_str(&code[cursor..rs]);
        if before.ends_with("__isv(") {
            res.push_str(&code[rs..whole.end()]);
        } else {
            res.push_str(&format!("__isv({})", &code[rs..whole.end()]));
        }
        cursor = whole.end();
    }
    res.push_str(&code[cursor..]);
    // a position the script reads is in this engine's pixels and comes back in the source's
    // (`__isp`), the units the script's own arithmetic is in; a position it sets crosses the
    // other way (`__sp`, below). a nudge of 3 source pixels is 3.9 here
    let positions = POSITIONS_CROSS.with(|c| c.get());
    let pre = crate::re!(r"\.get([XY])\(\)");
    let out = if !positions { res } else {
        let code = res;
        let mut res = String::with_capacity(code.len() + 64);
        let mut cursor = 0usize;
        for m in pre.captures_iter(&code) {
            let whole = m.get(0).unwrap();
            if whole.start() < cursor { continue; }
            let rs = receiver_start(&code, whole.start());
            if rs == whole.start() || rs < cursor { continue; }
            res.push_str(&code[cursor..rs]);
            if code[..rs].ends_with("__isp(") {
                res.push_str(&code[rs..whole.end()]);
            } else {
                res.push_str(&format!("__isp({})", &code[rs..whole.end()]));
            }
            cursor = whole.end();
        }
        res.push_str(&code[cursor..]);
        res
    };
    let next_write = |s: &str| -> Option<usize> {
        [s.find(".setXSpeed("), s.find(".setYSpeed("), s.find(".setX("), s.find(".setY(")]
            .into_iter().flatten().min()
    };
    let mut res = String::with_capacity(out.len() + 64);
    let mut rest: &str = &out;
    while let Some(at) = next_write(rest) {
        let open = at + rest[at..].find('(').unwrap() + 1;
        let (mut depth, mut end) = (1usize, open);
        for (i, ch) in rest[open..].char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => { depth -= 1; if depth == 0 { end = open + i; break; } }
                _ => {}
            }
        }
        if depth != 0 { break; }
        let arg = &rest[open..end];
        let t = arg.trim();
        let is_y = rest[at..open].contains("setYSpeed");
        let is_pos = !rest[at..open].contains("Speed");
        if is_pos && !positions {
            // a stage object's own position boundary is its own
            res.push_str(&rest[..open]);
            rest = &rest[open..];
            continue;
        }
        let zero = t == "0" || t == "0.0";
        // a y write is a step ahead of an x write: the source adds a frame of gravity to a
        // speed a script sets before it moves on it, this engine moves on it first. an x write
        // of zero on the floor holds there (the source's walk does not accelerate from a scripted
        // zero), so every x write goes through its own wrapper, zero included
        let done = t.starts_with("__sv") || t.starts_with("__sp(") || t.is_empty() || (zero && (is_y || is_pos));
        // ...on the object's OWN body: a speed written into another object (a projectile aimed by
        // its maker) crosses plainly, since the step of gravity and the floor hold are this body's
        // (ness's fire took a frame of ness's gravity as a constant sink and died on the floor)
        let recv = &rest[receiver_start(rest, at)..at];
        let own = recv.split('.').all(|r| r.is_empty() || r == "self");
        let wrap = if is_pos { "__sp" } else if !own { "__sv" } else if is_y { "__svy" } else { "__svx" };
        res.push_str(&rest[..open]);
        if done { res.push_str(arg); } else { res.push_str(&format!("{wrap}({arg})")); }
        res.push(')');
        rest = &rest[end + 1..];
    }
    res.push_str(rest);
    res
}

/// The two helpers `speeds_cross_the_boundary` leaves a script needing, for its preamble, and
/// the two a short-circuit used as a value needs (the source answers the operand it settled on;
/// the host's operators answer a bool).
pub(crate) fn speed_boundary_helpers() -> String {
    speed_boundary_helpers_for(SpeedUnits::ApiLayer)
}

/// The fighter's boundary helpers, for the integration test that pins where the scale lives.
pub fn speed_boundary_helpers_for_test() -> String { speed_boundary_helpers() }

/// What a script's stated speed is worth to this engine. A script speaks the source's units
/// throughout (its literals, its locals, what it reads back), and the boundary converts both
/// ways: a write by the velocity scale (the source steps 30 times a second in a world the
/// conversion makes `size_multiplier` bigger), a read by its inverse. Measured with
/// `tools/trajectory.py`: ganondorf writes -23 and both engines record -21.4 (one step of its
/// gravity taken off); game and watch writes -25 and records -23.7; simon writes 28.34.
#[derive(Clone, Copy)]
pub(crate) enum SpeedUnits {
    /// A fighter's or a projectile's script: positions cross by the size scale too.
    ApiLayer,
    /// A stage object's script: its positions are already the stage's.
    Scaled,
}

pub(crate) fn speed_boundary_helpers_for(units: SpeedUnits) -> String {
    let vs = crate::mappings::character_stats().scaling.velocity_scale();
    // a position a script states, in the source's pixels: the size scale here
    let sp = match units {
        SpeedUnits::ApiLayer => crate::mappings::character_stats().scaling.size_multiplier,
        SpeedUnits::Scaled => 1.0,
    };
    let gstep = match units {
        SpeedUnits::ApiLayer => "",
        SpeedUnits::Scaled => "function __gStep() { return 0; }\nfunction __xWrite(v) { return __sv(v); }\n",
    };
    format!("// a speed the script states, as this engine takes it (see api_mappings::SpeedUnits)\n\
             function __sv(v) {{ return v * {vs:.4}; }}\nfunction __isv(v) {{ return v / {vs:.4}; }}\n\
             // a y speed a script sets: the source's engine adds one frame of gravity to it before it moves\n\
             // (simon's -20 moves -18.35 on its first frame there), this engine moves on the value as set\n\
             function __svy(v) {{ return __sv(v) + __gStep(); }}\n\
             function __svx(v) {{ return __xWrite(v); }}\n\
             function __sp(v) {{ return v * {sp:.4}; }}\nfunction __isp(v) {{ return v / {sp:.4}; }}\n\
             // a short-circuit used as a value answers the operand it settled on\n\
             function __or(a, b) {{ return __truthy(a) ? a : b; }}\nfunction __and(a, b) {{ return __truthy(a) ? b : a; }}\n\
             // a match option this engine does not have is off: it reads as 0 however it is asked\n\
             function __optionOffStr(name) {{ return {{ toString: function(radix) {{ return \"0\"; }} }}; }}\n\
             // reverse() answers the array in the source and nothing here (no apostrophe in this comment: one before a helper hid it from the loader)\n\
             function __arrayReversed(a) {{ if (a == null) {{ return null; }} var out = []; var __i = a.length - 1; while (__i >= 0) {{ out.push(a[__i]); __i = __i - 1; }} return out; }}\n{gstep}")
}

/// The clip-clock and for-each helpers a FIGHTER's or a PROJECTILE's script needs (a stage
/// object's preamble defines its own, parts-aware versions): the clock of a clip the script
/// holds, as the source reads it, and a for-each walked by index.
pub(crate) fn entity_clock_helpers() -> String {
    let vs = crate::mappings::character_stats().scaling.velocity_scale();
    format!("// air_ease, as the source's engine runs it: a CAP on the downward speed while it is in force\n\
     // (-1 is none). measured with tools/trajectory.py: game and watch declares 3.5 on its up special and\n\
     // its fall holds at exactly 3.5 with its own gravity up to there; ganondorf writes 2 on its second\n\
     // frame and -1 six frames later, and its fall reaches its ordinary terminal; sora declares 0\n\
     // on its side special and hangs in the air (its record holds a vertical speed of exactly 0\n\
     // for the twelve frames before its dash, and again after its rise, until the frame that\n\
     // writes -1), so 0 is a cap of nothing and only -1 is none. an animation starts\n\
     // with the value its attack declares, and a frame script's write replaces it. the value is in the\n\
     // source's units (it is a stat, not a speed the script writes), so it scales here\n\
     var __airEaseV = -1;\n\
     var __airEaseAnim = null;\n\
     function __airEase(o) {{ __airEaseV = o.hold; __airEaseAnim = self.getAnimation(); }}\n\
     function __applyAirEase() {{\n\
     \tif (__airEaseAnim != self.getAnimation()) {{ __airEaseAnim = self.getAnimation(); __airEaseV = __declaredAirEase(); __startMovement(); }}\n\
     \tif (__airEaseV != null && __airEaseV >= 0) {{ var cap = __airEaseV * {vs:.4}; if (self.getYVelocity() > cap) {{ self.setYVelocity(cap); }} }}\n\
     }}\n\
     // resetMovement, as the source runs it: an attack that declares it starts from a standstill, both\n\
     // speeds put to zero as it begins (mario's up special took seven frames of fall speed into its\n\
     // climb without it). a piece cut from the same attack is not a new start\n\
     var __movementBase = null;\n\
     function __startMovement() {{ var b = __declaredAttackBase(); if (b != null && b != __movementBase) {{ if (__truthy(_attackStat(self, \"resetMovement\"))) {{ self.setXVelocity(0); self.setYVelocity(0); }} __declaredMotionStats(); }} __movementBase = b; }}\n\
     // rotate, as the source runs it: while it is in force the art turns to face the way the\n\
     // fighter moves, measured from the way it faces (this engine's own auto-rotation reads its\n\
     // art as pointing up, and turned sora's arc a quarter turn off). an animation starts with\n\
     // what its attack declares, a frame script's write replaces it, and a turn that ends puts\n\
     // the art back upright\n\
     var __rotateV = false;\n\
     var __rotateAnim = null;\n\
     function __rotate(o) {{ __rotateV = __truthy(o.on); __rotateAnim = self.getAnimation(); if (!__rotateV) {{ self.setRotation(0); }} }}\n\
     function __applyRotate() {{\n\
     \tif (__rotateAnim != self.getAnimation()) {{ __rotateAnim = self.getAnimation(); if (__rotateV) {{ self.setRotation(0); }} __rotateV = __declaredRotate(); }}\n\
     \tif (__rotateV) {{ var vx = self.getXVelocity(); var vy = self.getYVelocity(); if (vx != 0 || vy != 0) {{ self.setRotation(Math.atan2(vy, self.isFacingRight() ? vx : -vx) * 180 / Math.PI); }} }}\n\
     }}\n\
     // the clock of a clip the script holds (a vfx, a projectile, a part), as the source reads it:\n\
     // one it published through its exports, else what the engine answers for it\n\
     function __currentFrame(o) {{ if (o == null) {{ return 0; }} if (o.exports != null && o.exports.__frame != null) {{ return o.exports.__frame; }} return (o.getCurrentFrame != null) ? o.getCurrentFrame() : 0; }}\n\
     function __currentLabel(o) {{ if (o == null) {{ return \"\"; }} if (o.exports != null && o.exports.__label != null) {{ return o.exports.__label; }} return \"\"; }}\n\
     function __totalFrames(o) {{ if (o == null) {{ return 0; }} if (o.exports != null && o.exports.__total != null) {{ return o.exports.__total; }} return (o.getTotalFrames != null) ? o.getTotalFrames() : 0; }}\n\
     // a for-each walks a list by index: the next entry (1-based, 0 when there is none), and the entry under it\n\
     function __iterNext(o, i) {{ if (o == null || o.length == null) {{ return 0; }} return i < o.length ? i + 1 : 0; }}\n\
     function __iterValue(o, i) {{ return (o == null || i < 1) ? null : o[i - 1]; }}\n\
     function __iterName(o, i) {{ return i - 1; }}\n")
}

/// A projectile WRITING its summoner's state, the other half of reading it. In the source both
/// are fields on one object so it just assigns; here the summoner's state is its own, and what
/// it publishes is a wrapper, so the write goes through the wrapper the same way the summoner
/// writes it itself. Assigning over the published wrapper instead would leave the summoner
/// reading its own, which nobody had written to.
pub(crate) fn write_through_published_state(code: &str, name: &str) -> String {
    let needle = format!(".{name} = ");
    let mut out = String::with_capacity(code.len());
    let mut rest = code;
    while let Some(at) = rest.find(&needle) {
        let start = receiver_start(rest, at);
        let rhs_at = at + needle.len();
        // the end of the statement, not the first `;` inside whatever it assigns
        let b = rest.as_bytes();
        let (mut i, mut depth) = (rhs_at, 0i32);
        while i < b.len() {
            match b[i] {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth -= 1,
                b';' if depth <= 0 => break,
                _ => {}
            }
            i += 1;
        }
        if i >= b.len() {
            out.push_str(&rest[..rhs_at]);
            rest = &rest[rhs_at..];
            continue;
        }
        // a write that already reaches through the exports (another pass put it there) keeps one
        let receiver = rest[start..at].strip_suffix(".exports").unwrap_or(&rest[start..at]);
        out.push_str(&rest[..start]);
        out.push_str(&format!("{receiver}.exports.{name}.set({})", &rest[rhs_at..i]));
        rest = &rest[i..];
    }
    out.push_str(rest);
    out
}

/// A projectile's move to another of its own states (`forceAttack("pull")` on a projectile):
/// the label is one of its clip's, so it is that state, entered by name. Reads the commented
/// form the strike leaves, the same way the fighter's rewrite does.
pub fn rewrite_projectile_force_attack(code: &str, lstates: &BTreeMap<String, String>) -> String {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| regex::Regex::new(
        r#"^(\s*)// \[SSF2-only: forceAttack\] (?:self\.)?forceAttack\(\s*"([^"]+)"[^\n]*$"#).expect("regex"));
    let mut out: Vec<String> = Vec::new();
    for line in code.lines() {
        if let Some(caps) = re.captures(line) {
            let (indent, label) = (&caps[1], &caps[2]);
            if let Some(lname) = lstates.get(label) {
                if out.last().map(|l: &String| l.trim_start().starts_with("// SSF2 buffers a raw attack input")).unwrap_or(false) {
                    out.pop();
                }
                out.push(format!("{indent}Common.toLocalState(LState.{lname}); // SSF2 forceAttack(\"{label}\")"));
                continue;
            }
        }
        out.push(line.to_string());
    }
    let mut joined = out.join("\n");
    if code.ends_with('\n') { joined.push('\n'); }
    joined
}

/// A projectile's `self.destroy` (the call, or the handler handed to a listener by name) is
/// its own end here. The name only: a handler CALLED `destroyed` keeps its name (a plain
/// replace turned it into `_destroySelfed`, which nothing declares).
pub fn rename_self_destroy(code: &str) -> String {
    let mut out = String::with_capacity(code.len());
    let mut rest = code;
    while let Some(at) = rest.find("self.destroy") {
        let end = at + "self.destroy".len();
        let next = rest[end..].chars().next();
        out.push_str(&rest[..at]);
        if next.is_some_and(|c| c.is_alphanumeric() || c == '_') {
            out.push_str("self.destroy");
        } else {
            out.push_str("_destroySelf");
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// A PROJECTILE'S BOX IS ONE BOX. The source's attack boxes belong to the projectile, so a write
/// to one (the buster's damage falling off as it flies) holds through every label its clip plays;
/// here each label is an animation with a hitbox of its own, so a write made in one was gone at
/// the next, and at the next replay of the same one (a looping label restarts its animation, and
/// a restart puts the declared stats back). The writes are kept and put back at every change of
/// animation and every restart.
pub const HB_CARRY: &str = "\
var __hbCarried = [];\n\
var __hbCarryAnim = null;\n\
var __hbCarryFrame = 0;\n\
function __hbCarry(i, o) { if (o == null) { return; } while (__hbCarried.length <= i) { __hbCarried.push(null); } if (__hbCarried[i] == null) { __hbCarried[i] = []; } __hbCarried[i].push(o); __hbCarryAnim = self.getAnimation(); __hbCarryFrame = self.getCurrentFrame(); self.updateHitboxStats(i, o); }\n\
// (put back when the animation changes or starts over, not every update: a write every update\n\
// cleared the box's record of what it had hit, and ness's thunder and fire struck again and died)\n\
function __hbCarryUpdate() { var a = self.getAnimation(); var f = self.getCurrentFrame(); var again = a != __hbCarryAnim || f < __hbCarryFrame; __hbCarryAnim = a; __hbCarryFrame = f; if (!again) { return; } var i = 0; while (i < __hbCarried.length) { var l = __hbCarried[i]; if (l != null) { var j = 0; while (j < l.length) { self.updateHitboxStats(i, l[j]); j = j + 1; } } i = i + 1; } }\n";

/// `self.updateHitboxStats(i, o)` in a projectile's code, kept across its animations (see `HB_CARRY`).
pub fn hitbox_writes_carried(code: &str) -> String {
    if !code.contains("self.updateHitboxStats(") { return code.to_string() }
    code.lines().map(|l| {
        if l.trim_start().starts_with("function __") { l.to_string() } else { l.replace("self.updateHitboxStats(", "__hbCarry(") }
    }).collect::<Vec<_>>().join("\n") + if code.ends_with('\n') { "\n" } else { "" }
}

/// Who a fighter is, by the id its stats state: what every script this converter makes publishes.
pub const STATS_NAME_OF: &str = "function __statsNameOf(o) { if (o != null && o.exports != null && o.exports.__statsName != null) { return o.exports.__statsName; } return \"\"; }\n";

/// What a script that is not the fighter's does to ask the fighter for a label jump.
pub const ASK_LABEL: &str = "\
// a label the summoner is asked to jump to: signalled, since a call across scripts loses the\n\
// caller's own helpers, and the summoner takes it up on its next frame\n\
function __askLabel(o, l) { if (o != null && o.exports != null) { o.exports.__askLabel = l; } }\n";

/// `<who>.getStanceMC().self.stancePlayFrame("L")` in code that is not `who`'s own: the label
/// is `who`'s, jumped to in whatever move it is in. Asked of the other script rather than called
/// into it (see `ASK_LABEL`).
pub fn rewrite_owner_label_jumps(code: &str) -> String {
    const CALL: &str = ".getStanceMC().self.stancePlayFrame(\"";
    let mut out = String::with_capacity(code.len());
    let mut rest = code;
    while let Some(at) = rest.find(CALL) {
        let open = at + CALL.len();
        let Some(e) = rest[open..].find('"') else { break };
        let label = &rest[open..open + e];
        let after = &rest[open + e + 1..];
        if !after.starts_with(')') {
            out.push_str(&rest[..open]);
            rest = &rest[open..];
            continue;
        }
        let start = receiver_start(rest, at);
        let who = &rest[start..at];
        out.push_str(&rest[..start]);
        out.push_str(&format!("__askLabel({who}, \"{label}\")"));
        rest = &after[1..];
    }
    out.push_str(rest);
    out
}

/// A move asked for by the source's name (`getAnimation() == "b"`), answered by this engine
/// with the name the move has here. Only the names the map carries change; one it does not
/// (another fighter's copy of this move) is left as written.
pub fn rename_animation_literals(code: &str, ssf2_to_fm: &BTreeMap<String, String>) -> String {
    if ssf2_to_fm.is_empty() { return code.to_string() }
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| regex::Regex::new(
        r#"(getAnimation\(\)\s*[!=]=\s*)"([^"]+)""#).expect("regex"));
    let out = re.replace_all(code, |caps: &regex::Captures| {
        match ssf2_to_fm.get(&caps[2]).filter(|s| !s.is_empty()) {
            Some(fm) => format!("{}\"{}\"", &caps[1], fm),
            None => caps[0].to_string(),
        }
    }).into_owned();
    // ...and a compare made through a variable the read was put in first (`_v1 = char.getAnimation();
    // ... _v1 == "b"`): ness's flash asked whether ness was still in his special by the source's
    // name, never was, and gave up its charge every tick
    let held: std::collections::BTreeSet<String> = crate::re!(r"(?m)^\s*(?:var\s+)?([A-Za-z_]\w*)\s*=\s*[\w.()]*getAnimation\(\);")
        .captures_iter(&out).map(|c| c[1].to_string()).collect();
    let mut out = out;
    for v in held {
        let cmp = crate::dyn_re(&format!(r#"(\b{0}\s*[!=]=\s*)"([^"]+)"|"([^"]+)"(\s*[!=]=\s*{0}\b)"#, regex::escape(&v)));
        out = cmp.replace_all(&out, |c: &regex::Captures| {
            let lit = c.get(2).or(c.get(3)).map(|m| m.as_str()).unwrap_or("");
            match ssf2_to_fm.get(lit).filter(|s| !s.is_empty()) {
                Some(fm) if c.get(2).is_some() => format!("{}\"{}\"", &c[1], fm),
                Some(fm) => format!("\"{}\"{}", fm, &c[4]),
                None => c[0].to_string(),
            }
        }).into_owned();
    }
    out
}

thread_local! {
    /// The source's character states by number, as the file being converted declares them
    /// (its CState class): what a script's `getState() == 14` is asking about.
    static SOURCE_STATE_NAMES: std::cell::RefCell<BTreeMap<i64, String>> = const { std::cell::RefCell::new(BTreeMap::new()) };
    /// Whether the script being translated reads its own hitbox stats back at runtime, so every
    /// write to them is also noted for the read (see `note_hitbox_writes`).
    static MIRROR_HITBOX_WRITES: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Install the source's state numbering for the conversion at hand (empty when the file
/// declares none: numeric compares are then left as written).
pub fn set_source_state_names(m: BTreeMap<i64, String>) {
    SOURCE_STATE_NAMES.with(|c| *c.borrow_mut() = m);
}

/// Whether hitbox writes are mirrored for a later read (a script that asks
/// `getAttackBoxStat` gets them; nothing else pays for it).
pub fn set_mirror_hitbox_writes(on: bool) {
    MIRROR_HITBOX_WRITES.with(|c| c.set(on));
}

/// `<recv>.getType()` answers a kind number here and a name in the source ("SSF2Character"),
/// and the source's scripts compare and slice that name. Answered by the name through
/// `__typeName` (see `runtime_type_helpers`).
pub fn rewrite_get_type(code: &str) -> String {
    const CALL: &str = ".getType()";
    let mut out = String::with_capacity(code.len());
    let mut rest = code;
    while let Some(at) = rest.find(CALL) {
        let start = receiver_start(rest, at);
        let who = &rest[start..at];
        if who.is_empty() || rest[..start].ends_with("__typeName(") {
            out.push_str(&rest[..at + CALL.len()]);
            rest = &rest[at + CALL.len()..];
            continue;
        }
        out.push_str(&rest[..start]);
        out.push_str(&format!("__typeName({who})"));
        rest = &rest[at + CALL.len()..];
    }
    out.push_str(rest);
    out
}

/// `<recv>[<name>](<args>)`: a method called by a name worked out at runtime (the source builds
/// `"get" + kind + "Stat"` from an object's type). This engine's scripts cannot index a method
/// by name, so the call goes through `__callByName`, which dispatches the names the api has.
pub fn rewrite_dynamic_calls(code: &str) -> String {
    let b = code.as_bytes();
    let mut out = String::with_capacity(code.len());
    let mut cursor = 0usize;
    let mut i = 0usize;
    while i + 1 < b.len() {
        if !(b[i] == b']' && b[i + 1] == b'(') { i += 1; continue }
        // the bracket this closes
        let mut depth = 0i32;
        let mut open = None;
        let mut j = i;
        while j > 0 {
            j -= 1;
            if b[j] == b']' { depth += 1 }
            else if b[j] == b'[' { if depth == 0 { open = Some(j); break } depth -= 1 }
        }
        let Some(open) = open else { i += 1; continue };
        if open < cursor { i += 1; continue }
        let key = code[open + 1..i].trim();
        let numeric = key.parse::<f64>().is_ok();
        if key.is_empty() || numeric { i += 1; continue }
        let recv_start = receiver_start(code, open);
        let recv = code[recv_start..open].trim();
        if recv.is_empty() { i += 1; continue }
        // a list read by an expression is a list, not a method: the source writes a string
        // (or a name holding one) for a method it addresses by name
        if !(key.starts_with('"') || key.chars().all(|c| c.is_alphanumeric() || c == '_')) { i += 1; continue }
        let Some(close) = call_close(&code[i + 2..]) else { i += 1; continue };
        let args_end = i + 2 + close;
        let args = code[i + 2..args_end].trim();
        out.push_str(&code[cursor..recv_start]);
        out.push_str(&format!("__callByName({recv}, {key}, [{args}])"));
        cursor = args_end + 1;
        i = cursor;
    }
    out.push_str(&code[cursor..]);
    out
}

/// `<recv>.getWidth()` and the other accessors the source answers from a stat the object
/// declares (`stat_accessors`): answered from the declared stat here (`__declaredStat`), the
/// object's kind worked out when it is asked.
pub fn rewrite_stat_accessors(code: &str) -> String {
    let mut out = code.to_string();
    for (method, stat) in &crate::mappings::api_commands().stat_accessors {
        let call = format!(".{method}()");
        let mut next = String::with_capacity(out.len());
        let mut rest = out.as_str();
        while let Some(at) = rest.find(&call) {
            let start = receiver_start(rest, at);
            let who = &rest[start..at];
            if who.is_empty() {
                next.push_str(&rest[..at + call.len()]);
                rest = &rest[at + call.len()..];
                continue;
            }
            next.push_str(&rest[..start]);
            next.push_str(&format!("__declaredStat({who}, \"{stat}\")"));
            rest = &rest[at + call.len()..];
        }
        next.push_str(rest);
        out = next;
    }
    out
}

/// `<recv>.getAttackBoxStat(<id>, <key>)`: a hitbox stat read back while the move runs. Answered
/// from the move's declared hitboxes and the writes the script made since (`__hbStat`).
pub fn rewrite_attack_box_stat_reads(code: &str) -> String {
    const CALL: &str = ".getAttackBoxStat(";
    let mut out = String::with_capacity(code.len());
    let mut rest = code;
    while let Some(at) = rest.find(CALL) {
        let start = receiver_start(rest, at);
        let open = at + CALL.len();
        let Some(close) = call_close(&rest[open..]) else { out.push_str(&rest[..open]); rest = &rest[open..]; continue };
        let end = open + close;
        let inner = &rest[open..end];
        let Some(comma) = top_level_comma(inner) else { out.push_str(&rest[..open]); rest = &rest[open..]; continue };
        let id = inner[..comma].trim();
        let key = inner[comma + 1..].trim();
        // the source counts its boxes from one, this engine from zero
        let idx = match id.parse::<i64>() { Ok(n) => (n - 1).to_string(), Err(_) => format!("({id}) - 1") };
        out.push_str(&rest[..start]);
        out.push_str(&format!("__hbStat({idx}, {key})"));
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

/// Where the call whose `(` precedes `after` closes: the index in `after` of its `)`, past
/// nested brackets and strings. None when it never closes.
fn call_close(after: &str) -> Option<usize> {
    let b = after.as_bytes();
    let mut depth = 0i32;
    let mut in_str: Option<u8> = None;
    let mut i = 0usize;
    while i < b.len() {
        let c = b[i];
        if let Some(q) = in_str {
            if c == b'\\' { i += 2; continue }
            if c == q { in_str = None }
        } else {
            match c {
                b'"' | b'\'' => in_str = Some(c),
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => { if depth == 0 { return Some(i) } depth -= 1 }
                _ => {}
            }
        }
        i += 1;
    }
    None
}

/// The first comma at depth zero of an argument list, if any.
fn top_level_comma(s: &str) -> Option<usize> {
    let mut depth = 0i32;
    let mut in_str: Option<char> = None;
    for (i, c) in s.char_indices() {
        if let Some(q) = in_str { if c == q { in_str = None } continue }
        match c {
            '"' | '\'' => in_str = Some(c),
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => return Some(i),
            _ => {}
        }
    }
    None
}

/// `<recv>.getState() == N` (or `!=`): a state number of the source's, compared by a script.
/// The file names its states (`set_source_state_names`); the name is asked of this engine's
/// states through `__srcStateIs`. A number the file does not name is left as written.
pub fn rewrite_numeric_state_compares(code: &str) -> String {
    const CALL: &str = ".getState()";
    let names = SOURCE_STATE_NAMES.with(|c| c.borrow().clone());
    if !code.contains(CALL) { return code.to_string() }
    // a state the source names that this engine covers under other names (`CState.INJURED` is
    // its three hurt states): asked through the same helper as a numbered one
    let by_other_name = |n: &str| crate::mappings::api_commands().source_state_names.get(n)
        .is_some_and(|fm| !fm.is_empty() && (fm.len() != 1 || fm[0] != n));
    if names.is_empty() && !code.contains("CState.") { return code.to_string() }
    let mut out = String::with_capacity(code.len());
    let mut rest = code;
    while let Some(at) = rest.find(CALL) {
        let after = &rest[at + CALL.len()..];
        let t = after.trim_start();
        let (neg, t2) = if let Some(r) = t.strip_prefix("==") { (false, r) } else if let Some(r) = t.strip_prefix("!=") { (true, r) } else {
            out.push_str(&rest[..at + CALL.len()]); rest = after; continue
        };
        let t2 = t2.trim_start();
        if let Some(named) = t2.strip_prefix("CState.") {
            let id: String = named.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
            if by_other_name(&id) {
                let start = receiver_start(rest, at);
                let who = &rest[start..at];
                let consumed = after.len() - t2.len() + "CState.".len() + id.len();
                out.push_str(&rest[..start]);
                out.push_str(&format!("{}__srcStateIs({who}, \"{id}\")", if neg { "!" } else { "" }));
                rest = &after[consumed..];
                continue
            }
            out.push_str(&rest[..at + CALL.len()]); rest = after; continue
        }
        let digits: String = t2.chars().take_while(|c| c.is_ascii_digit()).collect();
        let Some(name) = digits.parse::<i64>().ok().and_then(|n| names.get(&n)) else {
            out.push_str(&rest[..at + CALL.len()]); rest = after; continue
        };
        let start = receiver_start(rest, at);
        let who = &rest[start..at];
        let consumed = after.len() - t2.len() + digits.len();
        out.push_str(&rest[..start]);
        out.push_str(&format!("{}__srcStateIs({who}, \"{name}\")", if neg { "!" } else { "" }));
        rest = &after[consumed..];
    }
    out.push_str(rest);
    out
}

/// `.move(dx, dy)`: a shift stated in the source's pixels crosses the boundary like every
/// other position (see `speeds_cross_the_boundary`).
pub fn scale_move_args(code: &str) -> String {
    const CALL: &str = ".move(";
    if !POSITIONS_CROSS.with(|c| c.get()) || !code.contains(CALL) { return code.to_string() }
    let mut out = String::with_capacity(code.len());
    let mut rest = code;
    while let Some(at) = rest.find(CALL) {
        let open = at + CALL.len();
        let Some(close) = call_close(&rest[open..]) else { out.push_str(&rest[..open]); rest = &rest[open..]; continue };
        let end = open + close;
        let inner = &rest[open..end];
        let Some(comma) = top_level_comma(inner) else { out.push_str(&rest[..open]); rest = &rest[open..]; continue };
        let wrap = |v: &str| -> String {
            let t = v.trim();
            if t == "0" || t == "0.0" || t.starts_with("__sp(") { t.to_string() } else { format!("__sp({t})") }
        };
        out.push_str(&rest[..open]);
        out.push_str(&format!("{}, {}", wrap(&inner[..comma]), wrap(&inner[comma + 1..])));
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// After a literal `updateHitboxStats(i, { k: v, … });`, the fields it wrote are noted for a
/// read-back (`__hbNote`), so `__hbStat` answers what the move has NOW and not only what it
/// declared. Only in a script that reads them back (`set_mirror_hitbox_writes`).
pub fn note_hitbox_writes(code: &str) -> String {
    const CALL: &str = ".updateHitboxStats(";
    if !MIRROR_HITBOX_WRITES.with(|c| c.get()) || !code.contains(CALL) { return code.to_string() }
    let mut out = String::with_capacity(code.len());
    let mut rest = code;
    while let Some(at) = rest.find(CALL) {
        let open = at + CALL.len();
        let Some(close) = call_close(&rest[open..]) else { out.push_str(&rest[..open]); rest = &rest[open..]; continue };
        let end = open + close;
        let inner = &rest[open..end];
        let close_ok = rest[end..].starts_with(");");
        let Some(comma) = top_level_comma(inner) else { out.push_str(&rest[..open]); rest = &rest[open..]; continue };
        let id = inner[..comma].trim().to_string();
        let bag = inner[comma + 1..].trim();
        if !(close_ok && bag.starts_with('{') && bag.ends_with('}')) { out.push_str(&rest[..open]); rest = &rest[open..]; continue }
        let mut notes = String::new();
        for (k, v) in parse_prop_bag(&bag[1..bag.len() - 1]) {
            if v.parse::<f64>().is_ok() || v == "true" || v == "false" {
                notes.push_str(&format!(" __hbNote({id}, \"{k}\", {v});"));
            }
        }
        out.push_str(&rest[..end + 2]);
        out.push_str(&notes);
        rest = &rest[end + 2..];
    }
    out.push_str(rest);
    out
}

pub fn attack_complete_helpers() -> String {
    "// the end of an attack: kept with the animation it was asked during, answered on the frame\n\
// the fighter is in another one\n\
var __acAsks = [];\n\
var __acAnims = [];\n\
function __onAttackComplete(f, ?_opts) { if (f == null) { return; } __acAsks.push(f); __acAnims.push(self.getAnimation()); }\n\
function __offAttackComplete(f, ?_opts) { var __i = __acAsks.indexOf(f); if (__i >= 0) { __acAsks.splice(__i, 1); __acAnims.splice(__i, 1); } }\n\
function __acSame(a, b) { if (a == null || b == null) { return false; } return a == b || a.indexOf(b) == 0 || b.indexOf(a) == 0; }\n\
function __takeAttackCompleteAsks() {\n\
\tif (__acAsks.length == 0) { return; }\n\
\tvar a = self.getAnimation();\n\
\tvar __i = 0;\n\
\twhile (__i < __acAsks.length) {\n\
\t\tif (!__acSame(a, __acAnims[__i])) { var f = __acAsks[__i]; __acAsks.splice(__i, 1); __acAnims.splice(__i, 1); f(); }\n\
\t\telse { __i = __i + 1; }\n\
\t}\n\
}\n".to_string()
}

/// WHAT AN OBJECT ASKS TO BE TOLD WHEN IT IS DESTROYED. The source announces an object's
/// destruction and the object's own code hangs its ending on it (a trap sends itself to its last
/// label, a bomb plays its blast). Nothing here announces it, so the asks are kept and run on the
/// frame the object enters the state it dies in, once.
/// A TIMELINE THE SOURCE HOLDS. The source stops a clip's timeline and it shows that frame until
/// something plays it again, while everything else about it carries on -- it keeps moving, and its
/// own code keeps running. This object can be told the same (measured: a held object's frame stays
/// where it is and it travels at its speed all the while), so a hold becomes a hold, and anything
/// that plays it again lets it go.
pub fn a_held_timeline_holds_here(code: &str) -> String {
    let re = crate::re!(r"(?m)^([ \t]*)(?:self\.)*stop\(\s*\)\s*;");
    re.replace_all(code, "${1}__holdFrame();").into_owned()
}

/// ...and where the clip that stops has a single frame of its own, there is no timeline to hold:
/// it shows the same picture whatever it is told, and what moves inside it is the clip placed
/// within, which the source leaves running.
pub fn a_one_frame_clip_has_nothing_to_hold(code: &str) -> String {
    let re = crate::re!(r"(?m)^([ \t]*)(?:self\.)*stop\(\s*\)\s*;");
    re.replace_all(code, "${1}// [nothing to hold: this clip has one frame of its own]").into_owned()
}

/// The hold itself, and the letting go: an object that is playing something again is not holding a
/// frame, and one on its way out plays its ending rather than standing still forever.
pub fn frame_hold_helpers() -> String {
    "// a timeline held where it is: the frame stays, the object carries on. it is let go when\n\
// something plays it again, and when it is on its way out, which it has to play to finish\n\
var __heldFrame = false;\n\
var __heldAnim = null;\n\
function __holdFrame() { if (!__heldFrame && self.pause != null) { __heldFrame = true; __heldAnim = self.getAnimation(); self.pause(); } }\n\
function __releaseFrame() { if (__heldFrame && self.resume != null) { __heldFrame = false; self.resume(); } }\n\
function __takeFrameHold() {\n\
\tif (!__heldFrame) { return; }\n\
\tif (self.getState() == PState.DESTROYING || self.getAnimation() != __heldAnim) { __releaseFrame(); }\n\
}\n".to_string()
}

pub fn proj_destroyed_helpers() -> String {
    "// what this asks to be told when it is destroyed: run on the frame it enters the state it\n\
// dies in, once\n\
var __pdAsks = [];\n\
var __pdFired = false;\n\
function __onProjDestroyed(f, ?_opts) { if (f != null && __pdAsks.indexOf(f) < 0) { __pdAsks.push(f); } }\n\
function __offProjDestroyed(f, ?_opts) { var __i = __pdAsks.indexOf(f); if (__i >= 0) { __pdAsks.splice(__i, 1); } }\n\
function __takeProjDestroyedAsks(?now) {\n\
\tif (__pdFired || __pdAsks.length == 0) { return; }\n\
\tif (now != true && self.getState() != PState.DESTROYING) { return; }\n\
\t__pdFired = true;\n\
\tvar l = __pdAsks; __pdAsks = []; var __i = 0;\n\
\twhile (__i < l.length) { l[__i](); __i = __i + 1; }\n\
}\n".to_string()
}

/// WHAT SOMETHING ELSE ASKS TO BE TOLD WHEN A THING IT MADE IS DESTROYED. The ask belongs to
/// whoever made it -- the handler is that script's own function, and a function handed to another
/// script loses its helpers -- so the thing is watched from here instead, and the handler runs on
/// the frame it is gone.
pub fn watched_destroyed_helpers() -> String {
    "// something this made, watched for its end: the handler is this script's own, so it is run\n\
// here on the frame the thing is gone\n\
var __pdWatch = [];\n\
function __onProjDestroyedOf(o, f, ?_opts) { if (o != null && f != null) { __pdWatch.push({ o: o, f: f }); } }\n\
function __offProjDestroyedOf(o, f, ?_opts) {\n\
\tvar __i = 0;\n\
\twhile (__i < __pdWatch.length) { if (__pdWatch[__i].o == o && __pdWatch[__i].f == f) { __pdWatch.splice(__i, 1); } else { __i = __i + 1; } }\n\
}\n\
function __takeWatchedDestroyed() {\n\
\tvar __i = 0;\n\
\twhile (__i < __pdWatch.length) {\n\
\t\tvar w = __pdWatch[__i];\n\
\t\tif (w.o == null || w.o.isDisposed()) { __pdWatch.splice(__i, 1); w.f(); }\n\
\t\telse { __i = __i + 1; }\n\
\t}\n\
}\n".to_string()
}

pub fn object_match_helpers(text: &str) -> String {
    let mut out = String::new();
    out.push_str(&if text.contains("__onProjDestroyed(") || text.contains("__offProjDestroyed(") {
        proj_destroyed_helpers()
    } else {
        "function __takeProjDestroyedAsks(?now) {}\n".to_string()
    });
    if text.contains("__sound(") || text.contains("__stopSound(") { out.push_str(&sound_handle_helpers()); }
    if text.contains("__fireNamed(") { out.push_str(&fire_named_helpers()); }
    out.push_str(&if text.contains("__onProjDestroyedOf(") || text.contains("__offProjDestroyedOf(") {
        watched_destroyed_helpers()
    } else {
        "function __takeWatchedDestroyed() {}\n".to_string()
    });
    out.push_str(&if text.contains("__holdFrame(") {
        frame_hold_helpers()
    } else {
        "function __takeFrameHold() {}\n".to_string()
    });
    // an object's own end-of-attack asks, when its code makes any: otherwise the frame that runs
    // them has nothing to run
    out.push_str(&if text.contains("__onAttackComplete(") || text.contains("__offAttackComplete(") {
        attack_complete_helpers()
    } else {
        "function __takeAttackCompleteAsks() {}\n".to_string()
    });
    if text.contains("__askAttackEnabled(") {
        out.push_str("// an attack another object turns on or off on the fighter holding it: asked, and taken up on\n\
// the fighter's next frame (a call into another script loses its helpers)\n\
function __askAttackEnabled(o, on, n, ?frames) { if (o != null && o.exports != null && o.exports.__askAttack != null) { o.exports.__askAttack.push({ on: on, n: n, frames: frames }); } }\n");
    }
    if text.contains("__globalVar(") || text.contains("__setGlobalVar(") {
        out.push_str("// a match global: this engine keeps none, so one this script did not set reads as unset,\n\
// which is what a versus match had in the source too\n\
var __globalNames = [];\n\
var __globalValues = [];\n\
function __globalVar(n) { var __i = 0; while (__i < __globalNames.length) { if (__globalNames[__i] == n) { return __globalValues[__i]; } __i = __i + 1; } return null; }\n\
function __setGlobalVar(n, v) { var __i = 0; while (__i < __globalNames.length) { if (__globalNames[__i] == n) { __globalValues[__i] = v; return; } __i = __i + 1; } __globalNames.push(n); __globalValues.push(v); }\n");
    }
    if text.contains("__platformBetween(") || text.contains("__groundBetween(") {
        out.push_str("// THE GROUND BETWEEN TWO POINTS (source units), as the source's terrain query answers it:\n\
// the first surface the segment crosses. a terrain body's surfaces count unless `terrains` is\n\
// false; a drop-through floor counts unless `platforms` is false or `ignoreFallthrough` is set.\n\
// this engine answers one structure type per ask, nearest first\n\
function __platformBetween(a, b, ?o) {\n\
\tif (a == null || b == null || a.x == null || b.x == null) { return null; }\n\
\tvar terrains = (o == null || o.terrains == null) ? true : __truthy(o.terrains);\n\
\tvar platforms = (o == null || o.platforms == null) ? true : __truthy(o.platforms);\n\
\tvar ignoreFall = o != null && __truthy(o.ignoreFallthrough);\n\
\tvar p1 = Point.create(__sp(a.x), __sp(a.y));\n\
\tvar p2 = Point.create(__sp(b.x), __sp(b.y));\n\
\tvar kinds = [StructureType.FLOOR, StructureType.CEILING, StructureType.LEFT_WALL, StructureType.RIGHT_WALL];\n\
\tvar __k = 0;\n\
\twhile (__k < kinds.length) {\n\
\t\tvar hits = self.hitTestStructuresWithLineSegment(p1, p2, Point.create(0, 0), { structureType: kinds[__k] });\n\
\t\tif (hits != null) {\n\
\t\t\tvar __h = 0;\n\
\t\t\twhile (__h < hits.length) {\n\
\t\t\t\tvar st = hits[__h];\n\
\t\t\t\tvar drop = st.getStructureStat(\"dropThrough\") == true;\n\
\t\t\t\tif ((drop && platforms && !ignoreFall) || (!drop && terrains)) { return st; }\n\
\t\t\t\t__h = __h + 1;\n\
\t\t\t}\n\
\t\t}\n\
\t\t__k = __k + 1;\n\
\t}\n\
\treturn null;\n\
}\n\
function __groundBetween(a, b) { return __platformBetween(a, b, null) != null; }\n");
    }
    if text.contains("__nextUid(") {
        out.push_str("// a fresh attack id, so a fighter is hit once per swing\nvar __uid = 0;\nfunction __nextUid() { __uid = __uid + 1; return __uid; }\n");
    }
    if text.contains("__srcLen(") {
        out.push_str("// a length in this engine's px, in the source's units\nfunction __srcLen(v) { return __isp(v); }\n");
    }
    if text.contains("__objectBoundary(") {
        out.push_str("// the stage's blast or camera box, as the rectangle the source's code reads (x, y, w, h)\n\
function __objectBoundary(n) {\n\
\t// nothing hands a fighter the stage here, so its boxes are not there to read: the source's\n\
\t// own code takes an absent stage the same way, and the ask that wanted one is struck where\n\
\t// it stands rather than answered with a box nobody measured\n\
\treturn { x: 0, y: 0, w: 0, h: 0, width: 0, height: 0 };\n\
}\n");
    }
    out
}

pub fn runtime_type_helpers() -> String {
    let cmds = crate::mappings::api_commands();
    let stats = crate::mappings::character_stats();
    let mut out = String::new();
    // types: kind here -> the source's name
    out.push_str("// what the source answers when an object is asked its type, by the kind it is here\n");
    // an object that cannot say (a hit's foe that is a surface or a record rather than an
    // entity; the ask throws rather than answering null) has no kind the source would name
    out.push_str("function __typeName(o) {\n\tif (o == null) { return \"\"; }\n\tvar t = null;\n\ttry { t = o.getType(); } catch (err:Dynamic) { return \"\"; }\n");
    for (kind, name) in &cmds.entity_type_names {
        out.push_str(&format!("\tif (t == EntityType.{kind}) {{ return \"{name}\"; }}\n"));
    }
    out.push_str("\treturn \"\";\n}\n");
    // the common prefix of the source's type names is what its scripts slice off to build a
    // method name from the rest ("get" + "Character" + "Stat")
    let names: Vec<&String> = cmds.entity_type_names.values().collect();
    let prefix_len = names.first().map(|f| {
        let mut n = f.len();
        for other in &names { n = n.min(f.chars().zip(other.chars()).take_while(|(a, b)| a == b).count()); }
        n
    }).unwrap_or(0);
    out.push_str("// a method called by a name worked out at runtime: the ones the api has\n");
    out.push_str("function __callByName(o, name, args) {\n\tif (o == null || name == null) { return null; }\n");
    let mut kinds: Vec<(String, String)> = Vec::new();
    for (kind, name) in &cmds.entity_type_names {
        let word = &name[prefix_len..];
        if word.is_empty() { continue }
        kinds.push((kind.clone(), word.to_string()));
        out.push_str(&format!("\tif (name == \"get{word}Stat\") {{ return __objStat(o, \"{word}\", args[0]); }}\n"));
    }
    out.push_str("\treturn null;\n}\n");
    out.push_str("// a stat read off another object, by the name the source uses for it\n");
    out.push_str("function __objStat(o, kind, name) {\n\tvar n = __statName(name);\n\tvar v = null;\n");
    for (_, word) in &kinds {
        out.push_str(&format!("\tif (kind == \"{word}\" && o.get{word}Stat != null) {{ v = o.get{word}Stat(n); }}\n"));
    }
    out.push_str("\tif (v == null) { v = __statDefault(name); } else { v = __statUnscale(o, kind, n, v); }\n\treturn v;\n}\n");
    // an accessor the source answers from a declared stat, on whatever kind of object is asked
    out.push_str(&format!("// a declared stat read through the accessor the source answers it by\n\
function __declaredStat(o, name) {{\n\tvar t = __typeName(o);\n\tif (t == \"\") {{ return __statDefault(name); }}\n\treturn __objStat(o, t.substr({prefix_len}), name);\n}}\n"));
    // units: a stat this engine holds scaled comes back in the source's units. a body is stated
    // in world units by everything but a character, whose own scale does that (the ecb block)
    let character_kind = cmds.entity_type_names.get("CHARACTER").map(|n| n[prefix_len..].to_string()).unwrap_or_default();
    out.push_str("function __statUnscale(o, kind, n, v) {\n\tif (v == null) { return null; }\n");
    let sc = &stats.scaling;
    for (unit, names) in &stats.runtime_read_units {
        let (factor, guard) = match unit.as_str() {
            "accel" => (sc.accel_scale(), String::new()),
            "speed" | "launch" => (sc.velocity_scale(), String::new()),
            "size" => (sc.size_multiplier, format!("kind != \"{character_kind}\" && ")),
            _ => continue,
        };
        if factor == 0.0 { continue }
        let test: Vec<String> = names.iter().map(|n| format!("n == \"{n}\"")).collect();
        if unit == "launch" {
            // a launch speed was converted with a step of gravity taken off; the read puts it back
            out.push_str(&format!("\tif ({}) {{ var __g = (o != null && o.getCharacterStat != null) ? o.getCharacterStat(\"gravity\") : 0; return (v + 1.5 * __g) / {factor:.4}; }}\n", test.join(" || ")));
        } else {
            out.push_str(&format!("\tif ({guard}({})) {{ return v / {factor:.4}; }}\n", test.join(" || ")));
        }
    }
    out.push_str("\treturn v;\n}\n");
    // names: the source's stat name -> this engine's
    let known: std::collections::BTreeSet<&str> = stats.fm_runtime_stats.iter().map(|s| s.as_str()).collect();
    out.push_str("function __statName(n) {\n");
    for (src, fm) in &stats.fm_runtime_read_names {
        if known.contains(fm.as_str()) { out.push_str(&format!("\tif (n == \"{src}\") {{ return \"{fm}\"; }}\n")); }
    }
    for aliases in stats.field_keys.values() {
        let Some(fm) = aliases.iter().find(|a| known.contains(a.as_str())) else { continue };
        for src in aliases {
            if src == fm || known.contains(src.as_str()) { continue }
            out.push_str(&format!("\tif (n == \"{src}\") {{ return \"{fm}\"; }}\n"));
        }
    }
    out.push_str("\treturn n;\n}\n");
    out.push_str("function __statDefault(n) {\n");
    for (name, v) in &stats.runtime_read_defaults {
        out.push_str(&format!("\tif (n == \"{name}\") {{ return {v}; }}\n"));
    }
    out.push_str("\treturn null;\n}\n");
    // states: the source's name -> the states here it covers
    out.push_str("// whether an object is in one of the source's states, asked by the source's name\n");
    out.push_str("function __srcStateIs(o, name) {\n\tif (o == null || o.getState == null) { return false; }\n\tvar s = o.getState();\n");
    for (src, fm) in &cmds.source_state_names {
        if fm.is_empty() { continue }
        let test: Vec<String> = fm.iter().map(|f| format!("s == CState.{f}")).collect();
        out.push_str(&format!("\tif (name == \"{src}\") {{ return {}; }}\n", test.join(" || ")));
    }
    out.push_str("\treturn false;\n}\n");
    // the launch the source's api works out, in this engine's knockback units
    let kb_scale = crate::mappings::character_hitbox_stats().fields.iter()
        .find(|f| f.fm_field == "baseKnockback").and_then(|f| f.scale).unwrap_or(1.0);
    out.push_str(&format!("// the launch the source's api works out for a hit (growth, base, the weight to use instead of\n\
// the victim's own, the damage dealt, the victim's damage, its weight, the type flag), in this\n\
// engine's knockback units: the same factor its hitbox knockbacks cross with\n\
function __calculateKnockback(growth, base, weightKB, dmg, victimDmg, weight, flag) {{\n\
\tif (growth == null) {{ growth = 0; }}\n\tif (base == null) {{ base = 0; }}\n\tif (dmg == null) {{ dmg = 0; }}\n\tif (victimDmg == null) {{ victimDmg = 0; }}\n\
\tvar w = (weightKB != null && weightKB != 0) ? weightKB : ((weight == null) ? 100 : weight);\n\
\tvar total = victimDmg + dmg;\n\
\tvar inner = total * 0.1 + dmg * total * 0.05;\n\
\tvar kb = ((inner * 1.4 * (200 / (w + 100))) + 18) * (growth * 0.01) + base;\n\
\tif (__truthy(flag)) {{ kb = kb * 1.2; }}\n\
\treturn kb * {kb_scale};\n\
}}\n"));
    out
}

/// What a script that reads its hitbox stats back needs: the notes of what it wrote
/// (`__hbNote`), the read (`__hbStat`) over those notes and the declared table
/// (`_hbAnim`/`_hbIdx`/`_hbKey`/`_hbVal`, emitted by the caller in the source's units), and
/// the key and unit translation between the two, from the hitbox mapping.
pub fn hitbox_read_helpers() -> String {
    let fields = &crate::mappings::character_hitbox_stats().fields;
    let mut out = String::new();
    out.push_str("// the hitbox stats the move wrote since it started, for a read-back (cleared as the animation changes)\n\
var __hbNoteA = [];\nvar __hbNoteI = [];\nvar __hbNoteK = [];\nvar __hbNoteV = [];\nvar __hbNoteAnim = null;\n\
function __hbNote(i, k, v) {\n\tvar a = self.getAnimation();\n\tif (a != __hbNoteAnim) { __hbNoteA = []; __hbNoteI = []; __hbNoteK = []; __hbNoteV = []; __hbNoteAnim = a; }\n\t__hbNoteA.push(a); __hbNoteI.push(i); __hbNoteK.push(k); __hbNoteV.push(v);\n}\n");
    out.push_str("// the source's key for a hitbox stat, as this engine names it\nfunction __hbFmKey(k) {\n");
    for f in fields {
        for k in &f.ssf2_keys {
            if k != &f.fm_field { out.push_str(&format!("\tif (k == \"{k}\") {{ return \"{}\"; }}\n", f.fm_field)); }
        }
    }
    out.push_str("\treturn k;\n}\n");
    out.push_str("// a value this engine holds, back in the source's units\nfunction __hbUnscale(k, v) {\n\tif (v == null) { return null; }\n");
    for f in fields {
        let expr = if f.isframe { format!("v / 2 - {}", f.plus_frames) } else if let Some(sc) = f.scale { format!("v / {sc}") } else { continue };
        for k in &f.ssf2_keys {
            out.push_str(&format!("\tif (k == \"{k}\") {{ return {expr}; }}\n"));
        }
    }
    out.push_str("\treturn v;\n}\n");
    out.push_str("// a hitbox stat of the move playing: what the script wrote since, else what the move declares\n\
function __hbStat(__i, key) {\n\tvar a = self.getAnimation();\n\tvar fk = __hbFmKey(key);\n\
\tvar __j = __hbNoteA.length - 1;\n\twhile (__j >= 0) { if (__hbNoteA[__j] == a && __hbNoteI[__j] == __i && __hbNoteK[__j] == fk) { return __hbUnscale(key, __hbNoteV[__j]); } __j = __j - 1; }\n\
\t__j = 0;\n\twhile (__j < _hbAnim.length) { if (_hbAnim[__j] == a && _hbIdx[__j] == __i && _hbKey[__j] == key) { return _hbVal[__j]; } __j = __j + 1; }\n\
\t__j = 0;\n\twhile (__j < _hbAnim.length) { if (_hbIdx[__j] == __i && _hbKey[__j] == key && a != null && a.indexOf(_hbAnim[__j]) == 0) { return _hbVal[__j]; } __j = __j + 1; }\n\
\treturn null;\n}\n");
    out
}

/// Where the expression that owns a call begins.
pub(crate) fn receiver_start_public(code: &str, at: usize) -> usize { receiver_start(code, at) }

fn receiver_start(code: &str, at: usize) -> usize {
    let b = code.as_bytes();
    let mut start = at;
    loop {
        while start > 0 {
            let c = b[start - 1];
            if c.is_ascii_alphanumeric() || c == b'_' || c == b'.' { start -= 1 } else { break }
        }
        // AN INDEX IS PART OF THE RECEIVER TOO. Stepping back over a call's parentheses but not
        // over an index's brackets stops the walk in the middle of `players2[p].getStat(...)`, so
        // the replacement lands after `players2[p]` and leaves it welded to a number -- which
        // parses nowhere, and the whole script goes. Both are brackets around an argument.
        let (open, close) = match b.get(start.wrapping_sub(1)) {
            Some(b')') if start > 0 => (b'(', b')'),
            Some(b']') if start > 0 => (b'[', b']'),
            _ => return start,
        };
        let mut depth = 0i32;
        let mut i = start;
        while i > 0 {
            i -= 1;
            if b[i] == close { depth += 1 }
            else if b[i] == open {
                depth -= 1;
                if depth == 0 { break }
            }
        }
        if depth != 0 { return start }
        start = i;
    }
}

/// Number a hitbox the way the target numbers them.
///
/// The source names its boxes `attackBox`, `attackBox2`, `attackBox3` and asks for them by
/// the number in that name, so its first box is 1. The engine names them `hitbox0`,
/// `hitbox1`, `hitbox2` and asks by the number in THAT name, so its first box is 0. Passed
/// straight through, every update lands on the box after the one it was meant for, and an
/// update meant for the last box lands on no box at all: mario's tornado sets its launch
/// angle on box 1 and the move keeps the angle it was declared with, which is why it
/// throws a victim away instead of holding them in it.
fn renumber_hitbox_ids(code: &str) -> String {
    const CALL: &str = ".updateHitboxStats(";
    if !code.contains(CALL) { return code.to_string() }
    let mut out = String::with_capacity(code.len());
    let mut rest = code;
    while let Some(at) = rest.find(CALL) {
        let (head, tail) = rest.split_at(at + CALL.len());
        out.push_str(head);
        let id: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect();
        match id.parse::<i64>() {
            // the source counts from one, the engine from zero
            Ok(n) => {
                out.push_str(&(n - 1).to_string());
                rest = &tail[id.len()..];
            }
            // worked out at runtime: say the same thing about whatever it comes to
            Err(_) => {
                let end = value_end(tail);
                out.push_str(&format!("({}) - 1", tail[..end].trim()));
                rest = &tail[end..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// A stat READ by name while the match runs has to answer with something, so unlike a
/// write it cannot simply be dropped. What it should answer with is this engine's own
/// stat: the code around the read goes on to do engine-side arithmetic with it, so the
/// engine's value is the one that belongs there.
///
/// Only where the mapping's own name for it IS one this engine has. The table names a
/// stat the way each side says it, and not every name on it is the engine's; taking one
/// that is not would swap a refusal for a wrong number, which is worse.
fn resolve_character_stat_reads(code: &str) -> String {
    // an object's stat read under the source's name for it (`weight1`): the same aliases name it
    let code = &if code.contains("getGameObjectStat(\"") {
        let stats = crate::mappings::character_stats();
        let mut out = code.to_string();
        for aliases in stats.field_keys.values() {
            let Some(fm) = aliases.iter().find(|a| stats.fm_runtime_stats.iter().any(|k| k == *a)) else { continue };
            for source in aliases.iter().filter(|a| *a != fm) {
                out = out.replace(&format!("getGameObjectStat(\"{source}\")"), &format!("getGameObjectStat(\"{fm}\")"));
            }
        }
        out
    } else { code.to_string() };
    if !code.contains("getCharacterStat(") { return code.to_string() }
    let stats = crate::mappings::character_stats();
    let known: std::collections::BTreeSet<&str> =
        stats.fm_runtime_stats.iter().map(|s| s.as_str()).collect();
    if known.is_empty() { return code.to_string() }

    let mut out = code.to_string();
    let rename = |out: &mut String, source: &str, fm: &str| {
        let from = format!("getCharacterStat(\"{source}\")");
        if !out.contains(&from) { return }
        log::info!("stat '{source}' read while running is this engine's '{fm}'");
        *out = out.replace(&from, &format!("getCharacterStat(\"{fm}\")"));
    };
    // the ones stated outright, each only if the engine really has it
    for (source, fm) in &stats.fm_runtime_read_names {
        if !known.contains(fm.as_str()) {
            log::warn!("'{fm}' is not a stat this engine has, so '{source}' is left alone");
            continue;
        }
        rename(&mut out, source, fm);
    }
    // and the ones the mapping already names both ways
    for aliases in stats.field_keys.values() {
        let Some(fm) = aliases.iter().find(|a| known.contains(a.as_str())) else { continue };
        let fm = fm.clone();
        for source in aliases {
            if source == &fm || known.contains(source.as_str()) { continue }
            rename(&mut out, source, &fm);
        }
    }

    // A NAME THIS ENGINE DOES NOT HAVE AT ALL. The read still has to answer, and the
    // engine refuses the name, taking the handler with it. Zero is what "this engine
    // does not have that" comes to, and it is the answer the source's own guards around
    // the read are already written to handle.
    let mut scan = 0usize;
    while let Some(rel) = out[scan..].find("getCharacterStat(\"") {
        let at = scan + rel;
        let name_at = at + "getCharacterStat(\"".len();
        let Some(close) = out[name_at..].find('"') else { break };
        let name = out[name_at..name_at + close].to_string();
        let end = name_at + close + 2; // past the quote and the `)`
        if known.contains(name.as_str()) || end > out.len() {
            scan = name_at;
            continue;
        }
        // the whole read goes, receiver and all: `self.getCharacterStat("x")` answers a
        // number, and leaving the `self.` in front of one is not an expression
        let start = receiver_start(&out, at);
        // WHO AN OBJECT IS. Its declared id is a stat in the source and the code compares
        // against it (a sphere asks whether lucario or kirby's copy fired it); this engine keeps
        // no such stat, but every script of this package knows its own and publishes it
        // ...and the id its stats state it by, which each fighter's script publishes the same way
        if name == "statsName" {
            let recv = out[start..at].trim_end_matches('.');
            let recv = if recv.is_empty() { "self" } else { recv };
            let call = format!("__statsNameOf({recv})");
            out = format!("{}{call}{}", &out[..start], &out[end..]);
            scan = start + call.len();
            continue;
        }
        if name == "linkage_id" {
            let recv = out[start..at].trim_end_matches('.');
            let recv = if recv.is_empty() { "self" } else { recv };
            let call = format!("__linkageOf({recv})");
            out = format!("{}{call}{}", &out[..start], &out[end..]);
            scan = start + call.len();
            continue;
        }
        // a stat this engine keeps under another shape (a list where the source keeps one
        // number, a joined string where it keeps a list): the mapping says how to read it
        if let Some(expr) = crate::mappings::character_stats().runtime_read_exprs.get(&name) {
            let recv = out[start..at].trim_end_matches('.');
            let recv = if recv.is_empty() { "self" } else { recv };
            let call = format!("({})", expr.replace("{recv}", recv)
                .replace("{velocity_scale}", &format!("{:.4}", crate::mappings::character_stats().scaling.velocity_scale())));
            out = format!("{}{call}{}", &out[..start], &out[end..]);
            scan = start + call.len();
            continue;
        }
        log::warn!("stat '{name}' read while running is not one this engine has, so it \
            answers 0 (the engine refuses the name and takes the handler with it)");
        // a NAME reads as an empty string, never a number: the source lower-cases and compares
        // it, which a number cannot survive (kingdom1's power-up picks a fighter by name)
        let none = if matches!(name.as_str(), "displayName" | "statsName" | "name") { "\"\"" } else { "0" };
        out = format!("{}{none} /* the engine has no {name} */{}", &out[..start], &out[end..]);
        scan = start;
    }
    strike_writes_onto_an_absent_read(&out)
}

/// Strike the field writes that hang off a read the engine could not answer.
///
/// The source reads something the engine does not have and then fills it in field by
/// field -- pit builds its whole voice table that way. The read answers a number, and a
/// field written onto a number stops the handler. There is nothing to write it onto, so
/// the writes go too, and the read keeps its answer for whatever else asks.
fn strike_writes_onto_an_absent_read(code: &str) -> String {
    if !code.contains("/* the engine has no ") { return code.to_string() }
    let mut out: Vec<String> = Vec::new();
    let mut absent: std::collections::BTreeSet<String> = Default::default();
    for line in code.lines() {
        let t = line.trim_start();
        // `x = 0 /* the engine has no y */;` -- from here on, x holds that answer
        if let Some((lhs, rhs)) = t.split_once('=') {
            let name = lhs.trim().trim_start_matches("var ").trim();
            if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                if rhs.contains("/* the engine has no ") {
                    absent.insert(name.to_string());
                } else {
                    absent.remove(name);
                }
            }
        }
        let hit = absent.iter().find(|n| {
            t.starts_with(&format!("{n}.")) && t[n.len() + 1..].contains('=')
        });
        match hit {
            Some(n) => {
                let indent = &line[..line.len() - t.len()];
                out.push(format!("{indent}// [nothing to write it onto] {t}"));
                let _ = n;
            }
            None => out.push(line.to_string()),
        }
    }
    let mut joined = out.join("\n");
    if code.ends_with('\n') { joined.push('\n'); }
    joined
}

/// Strike a reach into the camera's view container.
///
/// `getViewRootContainer()` is a method an ENTITY has, for the container it draws itself in. The
/// source also asks the CAMERA for one (`match.getCamera().getViewRootContainer()`) to work out the
/// screen edges, but the Fraymakers camera (CameraApi) has no such container, so the call resolves
/// to null and throws every frame the line runs. There is no counterpart, so the whole statement is
/// struck; the local it was filling is then left null, and `guard_reads_of_a_struck_definition`
/// wraps the code that reached through it.
fn strike_camera_container_reach(code: &str) -> String {
    const NEEDLE: &str = ".getCamera().getViewRootContainer()";
    if !code.contains(NEEDLE) { return code.to_string() }
    let mut out: Vec<String> = Vec::with_capacity(code.lines().count());
    for line in code.lines() {
        let t = line.trim_start();
        if t.contains(NEEDLE) && !t.starts_with("//") {
            let indent = &line[..line.len() - t.len()];
            out.push(format!("{indent}// [no counterpart: getViewRootContainer on the camera] {t}"));
        } else {
            out.push(line.to_string());
        }
    }
    let mut joined = out.join("\n");
    if code.ends_with('\n') { joined.push('\n'); }
    joined
}

/// Resolve `self.camera` to the engine camera.
///
/// SSF2 caches the camera in a clip field (`self.camera = <the camera>`) and reads it back as
/// `self.camera.foo()`. Fraymakers has one camera, `match.getCamera()`, and a fighter has no
/// `camera` field -- writing one throws and takes the whole animation down (nine characters cache it
/// in a frame script). So a line that only assigns `self.camera` is dropped (the cache is redundant),
/// and every other `self.camera` becomes `match.getCamera()`. A line already struck is left alone.
fn resolve_camera_reference(code: &str) -> String {
    if !code.contains("self.camera") { return code.to_string() }
    let mut out: Vec<String> = Vec::with_capacity(code.lines().count());
    let assign = crate::re!(r"^\s*self\.camera\s*=\s*[^=].*;?\s*$");
    // only the WHOLE field `self.camera`, not a longer name that starts with it (hoisted out of the
    // per-line loop)
    let field = crate::re!(r"self\.camera\b");
    for line in code.lines() {
        let t = line.trim_start();
        if t.starts_with("//") {
            out.push(line.to_string());
            continue;
        }
        if assign.is_match(line) {
            let indent = &line[..line.len() - t.len()];
            out.push(format!("{indent}// [camera is match.getCamera() here] {t}"));
        } else {
            // only the WHOLE field `self.camera`, not a longer name that starts with it: a plain
            // substring replace turned tails's `self.cameraProjs[0]` into `match.getCamera()Projs[0]`,
            // an unparseable token that took the whole script's scope down. the word boundary after
            // `camera` leaves `cameraProjs` (and any other `camera*` field) alone.
            out.push(field.replace_all(line, "match.getCamera()").into_owned());
        }
    }
    let mut joined = out.join("\n");
    if code.ends_with('\n') { joined.push('\n'); }
    joined
}

/// Guard a read that reaches through a local left null by a struck assignment.
///
/// The decompiler gives each function scratch locals (`_v1`.._vN`), declares them `var _v3 = null`
/// and then assigns the real value. When that assignment is the one thing with no counterpart here
/// (`_v3 = match.getStage().getMidground()`), the pass above strikes it out and `_v3` stays null --
/// so the later `if (... _v3.exports.x.get() ...)` throws a null access EVERY frame the handler runs
/// (simon's cross boomerang spammed it for the projectile's whole life, which tripped the next
/// move's dispatch read on the feed).
///
/// The fix is per function: a local whose ONLY value-giving assignment was struck (every live
/// assignment to it is `= null`) is null wherever it is read, so each statement that reaches through
/// it is wrapped in `if (name != null) { ... }`. If the local really is always null the guarded
/// statement is dead, exactly as striking it would be, but the wrap keeps the braces balanced
/// through a block and an `else`; and if the read is ever reached with a value after all, the guard
/// runs it rather than dropping it. Reads already guarded on the same name are left alone.
fn guard_reads_of_a_struck_definition(code: &str) -> String {
    let lines: Vec<&str> = code.lines().collect();
    // the value-giving RHS of a struck assignment, and the live assignments, are read per function
    let is_ident = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_alphanumeric() || c == '_')
        && !s.chars().next().unwrap().is_ascii_digit();
    // `// [marker] name = ...` or `// [marker] var name = ...` -> the struck name
    let struck_name = |t: &str| -> Option<String> {
        let t = t.strip_prefix("//")?.trim_start();
        if !t.starts_with('[') { return None }
        let after = t[t.find("] ")? + 2..].trim_start();
        let after = after.strip_prefix("var ").unwrap_or(after);
        let (lhs, rhs) = after.split_once('=')?;
        let lhs = lhs.trim();
        // an assignment, not `==`, and a plain local (not `a.b = `)
        if rhs.starts_with('=') || !is_ident(lhs) { return None }
        Some(lhs.to_string())
    };
    // `name = rhs` / `var name = rhs` on a LIVE line -> (name, rhs is non-null)
    let live_assign = |t: &str| -> Option<(String, bool)> {
        if t.starts_with("//") { return None }
        let a = t.strip_prefix("var ").unwrap_or(t);
        let (lhs, rhs) = a.split_once('=')?;
        let (lhs, rhs) = (lhs.trim(), rhs.trim_start());
        if rhs.starts_with('=') || !is_ident(lhs) { return None }
        let val = rhs.trim().trim_end_matches(';').trim();
        Some((lhs.to_string(), val != "null"))
    };
    // function extents at top level, by brace depth
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut i = 0usize;
    while i < lines.len() {
        let line = lines[i];
        if !line.trim_start().starts_with("function ") {
            out.push(line.to_string());
            i += 1;
            continue;
        }
        // the function body: from here to the line that closes it
        let mut depth = 0i32;
        let mut started = false;
        let mut end = i;
        for (j, l) in lines.iter().enumerate().skip(i) {
            for c in l.chars() {
                if c == '{' { depth += 1; started = true; }
                else if c == '}' { depth -= 1; }
            }
            if started && depth <= 0 { end = j; break; }
            end = j;
        }
        let body = &lines[i..=end];
        let mut struck: std::collections::BTreeSet<String> = Default::default();
        let mut live_val: std::collections::BTreeSet<String> = Default::default();
        for l in body {
            let t = l.trim_start();
            if let Some(n) = struck_name(t) { struck.insert(n); }
            if let Some((n, non_null)) = live_assign(t) {
                if non_null { live_val.insert(n); }
            }
        }
        let builtin = |n: &str| matches!(n, "self" | "match" | "character");
        let poison: Vec<String> = struck.into_iter()
            .filter(|n| !live_val.contains(n) && !builtin(n)).collect();
        if poison.is_empty() {
            out.extend(body.iter().map(|l| l.to_string()));
            i = end + 1;
            continue;
        }
        // walk the body statement by statement; wrap one that reaches through a poison local
        let mut k = 0usize;
        while k < body.len() {
            let l = body[k];
            let t = l.trim_start();
            let hit = if t.starts_with("//") {
                None
            } else {
                poison.iter().find(|n| reaches_through(t, n))
            };
            let Some(name) = hit else { out.push(l.to_string()); k += 1; continue };
            // already guarded on this same name: leave it
            if t.contains(&format!("{name} != null")) || t.contains(&format!("{name} == null")) {
                out.push(l.to_string());
                k += 1;
                continue;
            }
            // the statement's extent: to where its braces balance and it ends on `;` or `}`
            let mut d = 0i32;
            let mut span_end = k;
            for (m, s) in body.iter().enumerate().skip(k) {
                for c in s.chars() {
                    if c == '{' { d += 1 } else if c == '}' { d -= 1 }
                }
                let tt = s.trim_end();
                if d <= 0 && (tt.ends_with(';') || tt.ends_with('}')) { span_end = m; break; }
                span_end = m;
            }
            let indent: String = l.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
            out.push(format!("{indent}if ({name} != null) {{"));
            for s in &body[k..=span_end] {
                out.push(format!("\t{s}"));
            }
            out.push(format!("{indent}}}"));
            k = span_end + 1;
        }
        i = end + 1;
    }
    let mut joined = out.join("\n");
    if code.ends_with('\n') { joined.push('\n'); }
    joined
}

/// True when a statement reaches through `name` as an object: `name.` or `name[`, as a whole token
/// (not a suffix of a longer identifier). A bare `name` compared or passed by value cannot throw, so
/// it does not count.
fn reaches_through(stmt: &str, name: &str) -> bool {
    let b = stmt.as_bytes();
    let mut from = 0;
    while let Some(rel) = stmt[from..].find(name) {
        let at = from + rel;
        let before_ok = at == 0 || !(b[at - 1].is_ascii_alphanumeric() || b[at - 1] == b'_');
        let after = at + name.len();
        let after_ok = after < b.len() && (b[after] == b'.' || b[after] == b'[');
        // not itself a field access `foo.name.` -- the `name` before a `.` we key on must be the
        // object, so a preceding `.` means it is someone's field, already reached safely
        let not_field = at == 0 || b[at - 1] != b'.';
        if before_ok && after_ok && not_field {
            return true;
        }
        from = at + name.len();
    }
    false
}

/// The same rule for a CHARACTER stat written while the match runs: the engine refuses
/// the whole write for one name it does not know, taking every stat beside it along.
fn drop_character_stats_without_counterpart(code: &str) -> String {
    let known: std::collections::BTreeSet<&str> = crate::mappings::character_stats()
        .fm_runtime_stats.iter().map(|s| s.as_str()).collect();
    if known.is_empty() { return code.to_string() }
    let mut drop_names: Vec<String> = Vec::new();
    for name in stat_names_written(code, "updateCharacterStats(") {
        if !known.contains(name.as_str()) {
            log::warn!("character stat '{name}' is not one this engine has, so it is dropped \
                from the write that carries it (the engine refuses the whole write for it)");
            drop_names.push(name);
        }
    }
    drop_fields_named(code, &drop_names)
}

/// The same for a projectile's own tuning. A write that carries a stat the engine does not have
/// is refused whole, so everything else in that write is lost with it: blackmage steers its warp
/// by asking for a direction this engine has no name for, and the ask took the steering with it.
/// ASKING FOR A STAT THIS ENGINE DOES NOT KEEP. The write side is filtered already; the read
/// side was not, and the engine refuses the name rather than answering nothing -- so a condition
/// that merely wanted to know whether a projectile hangs in the air takes the move down with it.
/// The answer for a name it has never kept is nothing, which is what the source's test would
/// have made of it anyway.
fn answer_unkept_projectile_stat_asks(code: &str) -> String {
    let known: std::collections::BTreeSet<&str> = crate::mappings::projectile_tables()
        .fm_stats.iter().map(|s| s.as_str()).collect();
    if known.is_empty() { return code.to_string() }
    const CALL: &str = ".getProjectileStat(\"";
    let mut out = String::with_capacity(code.len());
    let mut rest = code;
    while let Some(at) = rest.find(CALL) {
        let open = at + CALL.len();
        let Some(q) = rest[open..].find('"') else { break };
        let name = &rest[open..open + q];
        let after = &rest[open + q + 1..];
        let Some(close) = after.find(')') else { break };
        let start_recv = receiver_start(rest, at);
        let own_ask = self_is_projectile() && rest[start_recv..at].split('.').all(|r| r.is_empty() || r == "self");
        if known.contains(name) {
            out.push_str(&rest[..open + q + 1 + close + 1]);
        } else if name == "time_max" && own_ask {
            // its own lifetime, which its script keeps (in the source's frames)
            out.push_str(&rest[..start_recv]);
            out.push_str("__lifetimeStat()");
        } else if name == "time_max" && !rest[start_recv..at].trim_end_matches('.').is_empty() {
            // another one's, through what it publishes
            let r = rest[start_recv..at].trim_end_matches('.');
            out.push_str(&rest[..start_recv]);
            out.push_str(&format!("(({r} != null && {r}.exports != null && {r}.exports.__lifetimeStat != null) ? {r}.exports.__lifetimeStat() : null)"));
        } else if name == "linkage_id" {
            // who the object is: its declared id, which every script here publishes
            let start = receiver_start(rest, at);
            let recv = rest[start..at].trim_end_matches('.');
            let recv = if recv.is_empty() { "self" } else { recv };
            out.push_str(&rest[..start]);
            out.push_str(&format!("__linkageOf({recv})"));
        } else {
            log::warn!("projectile stat '{name}' is not one this engine keeps, so asking for it \
                answers nothing rather than being refused");
            let start = receiver_start(rest, at);
            out.push_str(&rest[..start]);
            out.push_str("null");
        }
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    out
}

/// A yes/no stat a script writes at runtime, under this engine's name for it, asking this
/// engine's question: the two ask opposite ones about the death bounds (survive them, or be
/// destroyed by them), so the value turns over with the name. The stats table already carries
/// the declared value this way; a write on a frame (pikachu's bolt gives up surviving them
/// once it has fallen a while) was dropped as a name this engine does not keep.
fn rename_projectile_flag_writes(code: &str) -> String {
    let flags = &crate::mappings::projectile_tables().flag_map;
    let renamed: Vec<&crate::mappings::FlagMap> = flags.iter().filter(|f| f.ssf2 != f.fm || f.invert).collect();
    if renamed.is_empty() { return code.to_string() }
    const CALL: &str = "updateProjectileStats(";
    let mut out = String::with_capacity(code.len());
    let mut scan = 0usize;
    while let Some(rel) = code[scan..].find(CALL) {
        let at = scan + rel;
        let Some(brace) = code[at..].find('{') else { break };
        let body_start = at + brace + 1;
        let end = body_start + value_end_of_object(&code[body_start..]);
        out.push_str(&code[scan..body_start]);
        let parts: Vec<String> = split_top_level(&code[body_start..end]).into_iter().map(|part| {
            let Some((k, v)) = part.split_once(':') else { return part.to_string() };
            let key = k.trim();
            let Some(f) = renamed.iter().find(|f| f.ssf2 == key) else { return part.to_string() };
            let lead = &k[..k.len() - k.trim_start().len()];
            let value = v.trim();
            let value = if !f.invert { value.to_string() }
                else if value == "true" { "false".to_string() }
                else if value == "false" { "true".to_string() }
                else { format!("!__truthy({value})") };
            format!("{lead}{}: {value}", f.fm)
        }).collect();
        out.push_str(&parts.join(","));
        out.push_str(&code[end..end + 1]);
        scan = end + 1;
    }
    out.push_str(&code[scan..]);
    out
}

/// A projectile setting a stat its SCRIPT keeps as it runs (kirby's hammer flame lives as long as
/// it was charged; samus's missile hands its decaying speed back as its homing speed): the stat is
/// kept by the script, not the engine, so the write goes there too, through `__setStat` (the
/// lifetime through its own setter). The keys are the table's `handled_in_script`.
fn projectile_lifetime_writes(code: &str) -> String {
    let script_kept: Vec<String> = crate::mappings::projectile_tables().handled_in_script.iter()
        .map(|h| h.ssf2.clone()).filter(|k| k != "time_max").collect();
    const CALL: &str = "updateProjectileStats(";
    let mut out = String::with_capacity(code.len());
    let mut scan = 0usize;
    while let Some(rel) = code[scan..].find(CALL) {
        let at = scan + rel;
        let start = receiver_start(code, at);
        let recv = code[start..at].trim_end_matches('.');
        let own = self_is_projectile() && recv.split('.').all(|r| r.is_empty() || r == "self");
        let Some(brace) = code[at + CALL.len()..].find('{').filter(|b| code[at + CALL.len()..at + CALL.len() + b].trim().is_empty()) else {
            out.push_str(&code[scan..at + CALL.len()]);
            scan = at + CALL.len();
            continue;
        };
        let body_start = at + CALL.len() + brace + 1;
        let end = body_start + value_end_of_object(&code[body_start..]);
        let life = split_top_level(&code[body_start..end]).into_iter().find_map(|part| {
            let (k, v) = part.split_once(':')?;
            (k.trim() == "time_max").then(|| v.trim().to_string())
        });
        let kept: Vec<(String, String)> = split_top_level(&code[body_start..end]).into_iter().filter_map(|part| {
            let (k, v) = part.split_once(':')?;
            let k = k.trim();
            script_kept.iter().any(|s| s == k).then(|| (k.to_string(), v.trim().to_string()))
        }).collect();
        out.push_str(&code[scan..start]);
        match (own, life) {
            (true, Some(v)) => out.push_str(&format!("__setLifetime({v}); ")),
            // one it holds: through what that one publishes
            (false, Some(v)) if !recv.is_empty() && recv != "self" => out.push_str(&format!(
                "if ({recv} != null && {recv}.exports != null && {recv}.exports.__setLifetime != null) {{ {recv}.exports.__setLifetime({v}); }} ")),
            _ => {}
        }
        for (k, v) in kept {
            if own {
                out.push_str(&format!("__setStat(\"{k}\", {v}); "));
            } else if !recv.is_empty() && recv != "self" {
                out.push_str(&format!(
                    "if ({recv} != null && {recv}.exports != null && {recv}.exports.__setStat != null) {{ {recv}.exports.__setStat(\"{k}\", {v}); }} "));
            }
        }
        out.push_str(&code[start..end]);
        scan = end;
    }
    out.push_str(&code[scan..]);
    out
}

fn drop_projectile_stats_without_counterpart(code: &str) -> String {
    let known: std::collections::BTreeSet<&str> = crate::mappings::projectile_tables()
        .fm_stats.iter().map(|s| s.as_str()).collect();
    if known.is_empty() { return code.to_string() }
    let mut drop_names: Vec<String> = Vec::new();
    for name in stat_names_written(code, "updateProjectileStats(") {
        if !known.contains(name.as_str()) {
            log::warn!("projectile stat '{name}' is not one this engine has, so it is dropped \
                from the write that carries it (the engine refuses the whole write for it)");
            drop_names.push(name);
        }
    }
    drop_fields_named(code, &drop_names)
}

fn drop_hitbox_fields_without_counterpart(code: &str) -> String {
    let map = crate::mappings::character_hitbox_stats();
    // What the engine HAS is the question, not what the source might send: a stat
    // nobody has listed yet still has to fall out on its own. The named list stays
    // as the record of which ones were looked at and why they map to nothing.
    let known: std::collections::BTreeSet<&str> =
        map.fm_stats.iter().map(|s| s.as_str()).collect();
    let mut drop_names: Vec<String> = map.no_counterpart.clone();
    if !known.is_empty() {
        for name in hitbox_stat_names_written(code) {
            if !known.contains(name.as_str()) && !drop_names.contains(&name) {
                log::warn!("hitbox stat '{name}' is not one this engine has, so it is dropped \
                    from the write that carries it (the engine refuses the whole write for it)");
                drop_names.push(name);
            }
        }
    }
    drop_fields_named(code, &drop_names)
}

/// Remove each named field, and its whole value, from the object literals in `code`.
fn drop_fields_named(code: &str, drop_names: &[String]) -> String {
    let mut out = code.to_string();
    for field in drop_names {
        let needle = format!("{field}:");
        let mut from = 0usize;
        while let Some(rel) = out[from..].find(&needle) {
            let at = from + rel;
            // the name has to START here: `Effect:` sits inside `hitEffectOverride:`,
            // and cutting from the middle of a name leaves the front of it behind as
            // a stray word the script cannot parse.
            let starts = at == 0 || {
                let prev = out.as_bytes()[at - 1];
                !(prev.is_ascii_alphanumeric() || prev == b'_')
            };
            if !starts {
                from = at + 1;
                continue;
            }
            // only inside a hitbox object literal
            let Some(open) = out[..at].rfind('{') else { break };
            let after = &out[at + needle.len()..];
            let cut = value_end(after);
            let end = at + needle.len() + cut;
            let head = out[..at].trim_end().trim_end_matches(',');
            let tail = out[end..].trim_start().trim_start_matches(',');
            let sep = if head.ends_with('{') || tail.starts_with('}') { " " } else { ", " };
            out = format!("{head}{sep}{tail}");
            from = open;
        }
    }
    out.replace("{ }", "{}")
}

/// Apply the FIXED-knockback rule to a hitbox object written at RUNTIME.
///
/// A source hitbox states fixed knockback with `weightKB` -- a launch that does not grow with the
/// victim's damage. Fraymakers has no such field, and the static table already reads the rule the
/// measured way: fixed knockback carries no growth, and `weightKB` is not a field it emits.
///
/// A runtime write went out with `weightKB` still on it, and the engine rejects the whole object
/// for the one name it does not know ("Invalid hitbox stat") -- so mario's up special threw away
/// its entire knockback update over a field that was set to zero. The same rule applies here: the
/// field goes, and when it states a real fixed launch the growth goes to zero with it.
fn resolve_fixed_knockback_writes(code: &str) -> String {
    let mut out = code.to_string();
    while let Some(at) = out.find("weightKB:") {
        // the object this field belongs to
        let Some(open) = out[..at].rfind('{') else { break };
        let mut depth = 0i32;
        let mut close = out.len();
        for (i, c) in out[open..].char_indices() {
            match c { '{' => depth += 1, '}' => { depth -= 1; if depth == 0 { close = open + i; break } }, _ => {} }
        }
        let obj = out[open..=close.min(out.len() - 1)].to_string();
        // read and strip the field
        let rel = at - open;
        let after = &obj[rel + "weightKB:".len()..];
        let val: f64 = after.trim_start()
            .split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
            .next().unwrap_or("0").parse().unwrap_or(0.0);
        let end_rel = rel + "weightKB:".len() + after.len() - after.trim_start().len()
            + after.trim_start().find([',', '}']).unwrap_or(0);
        let mut new_obj = String::with_capacity(obj.len());
        new_obj.push_str(&obj[..rel]);
        new_obj.push_str(obj[end_rel..].trim_start_matches(", ").trim_start_matches(','));
        // a stated fixed launch does not grow
        if val > 0.0 {
            if let Some(g) = new_obj.find("knockbackGrowth:") {
                let tail = &new_obj[g + "knockbackGrowth:".len()..];
                let cut = tail.find([',', '}']).unwrap_or(tail.len());
                new_obj = format!("{}knockbackGrowth: 0{}", &new_obj[..g], &tail[cut..]);
            } else if let Some(b) = new_obj.rfind('}') {
                new_obj = format!("{}, knockbackGrowth: 0 {}", new_obj[..b].trim_end().trim_end_matches(','), &new_obj[b..]);
            }
        }
        // tidy the comma left where the field used to sit, at either end
        let new_obj = new_obj.replace("{ ,", "{").replace("{,", "{")
            .replace(", }", " }").replace(",}", "}");
        out = format!("{}{}{}", &out[..open], new_obj, &out[close + 1..]);
    }
    out
}

// A held item: a projectile of the character's made by `generateItem`, riding the fighter's
// item box (published each frame for the item to follow) until `tossItem` throws it. The
// item's own script takes the hold and the throw up as state asks on its exports, in its own
// scope (see the projectile templates' `__itemUpdate`). The item box table is handed to the
// script through this slot, since the boxes are read where the script is not assembled.
thread_local! { static GENERATED_ITEMS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) }; }
/// The content ids of the items the fighter's code generates, for what an item pulled with no
/// declaration of its own takes as physics.
pub fn set_generated_items(ids: Vec<String>) { GENERATED_ITEMS.with(|c| *c.borrow_mut() = ids); }
pub fn is_generated_item(id: &str) -> bool {
    // the id as a content id (`bobombProjectile`) or as the entity's own (`bobomb`)
    let folded = crate::projectile_gen::projectile_content_id(id);
    GENERATED_ITEMS.with(|c| c.borrow().iter().any(|i| i == id || *i == folded))
}

/// Comment out entire statements containing SSF2-only method calls that have
/// no Fraymakers equivalent. The list of names comes from
/// `commands.jsonc :: ssf2_only` (see crate::mappings).
///
/// Each matched line is replaced with `// [SSF2-only: NAME] <original>` so it
/// won't compile but stays readable. Lines that already start with `//` are
/// left alone (idempotent).
///
/// Side-effect: when log level is at least `debug`, any `.NAME(` call whose
/// name doesn't appear in ANY section of commands.jsonc (replacements,
/// passthrough_fm_apis, ssf2_only, or frame_params) is logged once globally.
/// This is what makes `passthrough_fm_apis` functional — listing a name
/// there suppresses it from this "unknown call" stream. Run with
/// `RUST_LOG=debug` to surface new SSF2 calls that need attention.
/// Split SSF2 umbrella calls (currently `updateAttackStats`) into one or
/// more grouped FM calls per the `call_splits` table in commands.jsonc.
///
/// Source pattern: `<receiver>.<method>({ k1: v1, k2: v2, … })`. The
/// receiver may be any chain (`self`, `match.getCamera()`,
/// `self.lightningBolts[…]`) — it's preserved verbatim on every emitted
/// call. Calls whose argument isn't an object literal are left alone.
///
/// Per-field semantics (drawn from CallSplit.fields):
///   - mapping `"updateAnimationStats.leaveGroundCancel"`  →
///     groups under target `updateAnimationStats` with FM field
///     `leaveGroundCancel: <value>`
///   - source fields absent from the mapping become a `// TODO: …` line
///   - all fields sharing a target method merge into ONE call with
///     `, `-joined pairs, no trailing comma
///   - source line indentation propagates to every emitted line
/// `x.slice(a, b)` is the source's for a string as much as for a list, and a string here has no
/// such method (sonic's autolink read a type name that way and its hit handler died on it), so it
/// is asked of a helper that does either.
fn slices_of_strings_too(code: &str) -> String {
    const CALL: &str = ".slice(";
    if !code.contains(CALL) { return code.to_string() }
    let mut out = String::with_capacity(code.len());
    let mut rest = code;
    while let Some(at) = rest.find(CALL) {
        let start = receiver_start(rest, at);
        if start == at { out.push_str(&rest[..at + CALL.len()]); rest = &rest[at + CALL.len()..]; continue }
        let who = &rest[start..at];
        out.push_str(&rest[..start]);
        out.push_str(&format!("__slice({who}, "));
        rest = &rest[at + CALL.len()..];
    }
    out.push_str(rest);
    out
}

/// The source ends another object by asking it to (`proj.destroy()`), and its own end runs: what it
/// was asked to tell of its end, and its listeners taken off. Here `kill` on another object disposes
/// it with none of that, so a projectile another script ends (an absorb, a fighter putting away
/// what it made) is asked on its exports and ends itself in its own update, the way it ends at the
/// death bounds; anything else is killed as before. A stage has its own preamble and is left be.
fn others_ended_through_their_own_end(code: &str) -> String {
    if IN_STAGE.with(|c| c.get()) || !code.contains(".kill()") { return code.to_string() }
    crate::re!(r"(^|[^\w.])([A-Za-z_]\w*(?:\.\w+|\(\)|\[[^\]\[]*\])*)\.kill\(\)")
        .replace_all(code, |c: &regex::Captures| {
            if &c[2] == "self" { c[0].to_string() } else { format!("{}__endOther({})", &c[1], &c[2]) }
        }).into_owned()
}

/// The source ends a thing it made by taking it off the display list, and what it takes off is
/// itself (`hatEffect.parent.removeChild(hatEffect)`). Here that is the thing ending its own life,
/// which is what `kill` is. Only when it IS itself: a list that hands back something else is
/// taking that off, and nothing here says what to do with it.
fn an_effect_taken_off_the_list_ends_itself(code: &str) -> String {
    if !code.contains(".parent.removeChild(") { return code.to_string() }
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| regex::Regex::new(
        r"(?m)^([ 	]*)([A-Za-z_][\w.\[\]()]*)\.parent\.removeChild\(\s*([^;]*?)\s*\)\s*;").expect("regex"));
    re.replace_all(code, |c: &regex::Captures| {
        let (indent, recv, arg) = (&c[1], &c[2], &c[3]);
        // only what CAN end itself: a slot the source filled with something else (a struck call
        // leaves one holding a plain bag) has no such life to end, and asking took goku's whole
        // script down with it
        if recv == arg { format!("{indent}if ({recv} != null && {recv}.kill != null) {{ {recv}.kill(); }}") }
        else { c[0].to_string() }
    }).into_owned()
}

/// A split call handed a NAME whose object literal was written just before (`newStats = {
/// xSpeedDecay: -3.8 }; updateAttackStats(newStats);`) is handed that literal: the split reads
/// the fields it fans out from the literal, and a name leaves it nothing to read (peach's side
/// special never slowed). Only the last assignment ahead of the call, in the same code.
fn inline_named_stat_objects(code: &str) -> String {
    let cfg = crate::mappings::api_commands();
    let mut out = code.to_string();
    for method in cfg.call_splits.keys() {
        let needle = format!(".{method}(");
        let mut from = 0usize;
        while let Some(rel) = out[from..].find(&needle) {
            let open = from + rel + needle.len();
            from = open;
            let Some(close) = call_close(&out[open..]) else { continue };
            let arg = out[open..open + close].trim().to_string();
            if arg.is_empty() || !arg.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.') { continue }
            let assign = format!("{arg} = {{");
            let Some(at) = out[..open].rfind(&assign) else { continue };
            // the assignment has to start its statement
            let before = out[..at].trim_end();
            if !(before.is_empty() || before.ends_with(';') || before.ends_with('{') || before.ends_with('}')) { continue }
            let body_start = at + assign.len();
            let body_end = body_start + value_end_of_object(&out[body_start..]);
            if body_end >= open { continue }
            let literal = format!("{{{}}}", &out[body_start..body_end]);
            out.replace_range(open..open + close, &literal);
            from = open + literal.len();
        }
    }
    out
}

pub fn apply_call_splits(code: &str) -> String {
    let cfg = crate::mappings::api_commands();
    if cfg.call_splits.is_empty() { return code.to_string(); }
    let mut current = code.to_string();
    for (source_method, split) in &cfg.call_splits {
        current = apply_one_call_split(&current, source_method, split);
    }
    current
}

fn apply_one_call_split(code: &str, method: &str, split: &crate::mappings::CallSplit) -> String {
    let needle = format!(".{}(", method);
    let bytes = code.as_bytes();
    let mut result = String::with_capacity(code.len());
    let mut cursor = 0usize;

    while cursor < bytes.len() {
        let Some(rel) = code[cursor..].find(&needle) else {
            result.push_str(&code[cursor..]);
            break;
        };
        let dot_pos = cursor + rel;
        // Don't replace inside a single-line comment — if the line up to
        // dot_pos contains a `//` (outside of a string), skip this match.
        if line_is_commented_at(code, dot_pos) {
            result.push_str(&code[cursor..=dot_pos]);
            cursor = dot_pos + 1;
            continue;
        }
        let paren_open = dot_pos + needle.len() - 1;
        let receiver_start = find_receiver_start(code, dot_pos);
        let receiver = &code[receiver_start..dot_pos];

        let Some(paren_close) = find_matching_close(code, paren_open) else {
            // Unclosed call — give up on this match, advance past `.`.
            result.push_str(&code[cursor..=dot_pos]);
            cursor = dot_pos + 1;
            continue;
        };

        let args = code[paren_open + 1..paren_close].trim();
        if !args.starts_with('{') || !args.ends_with('}') {
            // Not an object-literal argument — leave the call alone.
            result.push_str(&code[cursor..=paren_close]);
            cursor = paren_close + 1;
            continue;
        }
        let body = &args[1..args.len() - 1];
        let fields = parse_object_fields(body);

        let line_start = code[..receiver_start].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let indent: String = code[line_start..receiver_start].chars()
            .take_while(|c| c.is_whitespace() && *c != '\n').collect();

        // Consume trailing `;` and any spaces between `)` and `;`.
        let mut full_end = paren_close + 1;
        while full_end < bytes.len() && bytes[full_end] == b' ' { full_end += 1; }
        if full_end < bytes.len() && bytes[full_end] == b';' { full_end += 1; }

        let rendered = render_call_split(receiver, &fields, split, &indent);
        // When the render is empty (every field was skipped) AND the only
        // text between cursor and the call is leading whitespace on this
        // line, also consume the trailing newline so the line vanishes
        // cleanly instead of leaving a blank-with-indent.
        if rendered.is_empty()
            && code[..receiver_start].rfind('\n').map(|i| i + 1).unwrap_or(0) >= cursor
            && code[cursor..receiver_start].chars().all(|c| c.is_whitespace() && c != '\n')
        {
            // Skip up to and including the next '\n' after the call.
            let mut skip_end = full_end;
            while skip_end < bytes.len() && bytes[skip_end] != b'\n' {
                if !(bytes[skip_end] as char).is_whitespace() { break; }
                skip_end += 1;
            }
            if skip_end < bytes.len() && bytes[skip_end] == b'\n' {
                skip_end += 1;
                cursor = skip_end;
                continue;
            }
        }
        result.push_str(&code[cursor..receiver_start]);
        result.push_str(&rendered);
        cursor = full_end;
    }
    a_list_built_and_filled(&result)
}

/// Per-field decision when resolving a CallSplit field against a source.
enum FieldOutcome {
    /// Route into the named target method's grouped call. `todo_note`,
    /// if present, also emits an inline `// TODO:` comment for the user
    /// to verify the value.
    Route { target_method: String, fm_field: String, value: String, todo_note: Option<String> },
    /// Emit only as a TODO comment line — no routed call.
    Todo { src_name: String, src_value: String, note: Option<String> },
    /// Drop the field entirely (no emission, no TODO) — used by
    /// `skip_if_value` matches.
    Skip,
}

fn render_call_split(
    receiver: &str,
    source_fields: &[(String, String)],
    split: &crate::mappings::CallSplit,
    indent: &str,
) -> String {
    use crate::mappings::CallSplitFieldMapping as M;
    use std::collections::BTreeMap;

    let parse_target = |s: &str| -> Option<(String, String)> {
        s.rfind('.').map(|d| (s[..d].to_string(), s[d + 1..].to_string()))
    };

    let mut outcomes: Vec<FieldOutcome> = Vec::with_capacity(source_fields.len());
    for (field_name, value) in source_fields {
        let trimmed = value.trim();
        let outcome = match split.fields.get(field_name) {
            None => FieldOutcome::Todo {
                src_name: field_name.clone(),
                src_value: value.clone(),
                note: None,
            },
            Some(M::Simple(s)) => match parse_target(s) {
                Some((tm, ff)) => FieldOutcome::Route {
                    target_method: tm, fm_field: ff, value: value.clone(), todo_note: None,
                },
                None => FieldOutcome::Todo {
                    src_name: field_name.clone(),
                    src_value: value.clone(),
                    note: Some("malformed mapping in commands.jsonc".to_string()),
                },
            },
            Some(M::Detailed { target, value_map, skip_if_value, todo, unit }) => {
                let converted = unit.as_deref().map(|u| convert_split_unit(u, trimmed));
                let (value, trimmed, todo) = match &converted {
                    Some(Ok(v)) => (v.as_str(), v.as_str(), todo.clone()),
                    Some(Err(note)) => (value.as_str(), trimmed, Some(note.clone())),
                    None => (value.as_str(), trimmed, todo.clone()),
                };
                let todo = &todo;
                if let Some(sv) = skip_if_value {
                    if trimmed == sv.trim() {
                        FieldOutcome::Skip
                    } else {
                        resolve_detailed(field_name, value, trimmed, target, value_map, todo, &parse_target)
                    }
                } else {
                    resolve_detailed(field_name, value, trimmed, target, value_map, todo, &parse_target)
                }
            }
        };
        // a projectile asking itself for what only a fighter has: the source's engine keeps it on
        // the projectile and nothing reads it there; this engine refuses it outright
        let outcome = match outcome {
            FieldOutcome::Route { ref target_method, ref fm_field, .. }
                if self_is_projectile() && receiver.split('.').all(|r| r.is_empty() || r == "self")
                    && crate::mappings::api_commands().projectile_self.call_split_drops
                        .iter().any(|d| *d == format!("{target_method}.{fm_field}")) => FieldOutcome::Skip,
            o => o,
        };
        outcomes.push(outcome);
    }

    // Group Route outcomes by target method (preserve first-seen order),
    // collect TODO lines (from both Todo outcomes and Route todo_notes).
    let mut order: Vec<String> = Vec::new();
    let mut groups: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    let mut todo_lines: Vec<String> = Vec::new();

    for o in outcomes {
        match o {
            FieldOutcome::Route { target_method, fm_field, value, todo_note } => {
                if !groups.contains_key(&target_method) {
                    order.push(target_method.clone());
                }
                if let Some(n) = &todo_note {
                    todo_lines.push(format!("// TODO: {}: {}", fm_field, n));
                }
                groups.entry(target_method).or_default().push((fm_field, value));
            }
            FieldOutcome::Todo { src_name, src_value, note } => {
                let n = note.as_deref().unwrap_or("no FM mapping in call_splits");
                todo_lines.push(format!("// TODO: {}: {} — {}", src_name, src_value, n));
            }
            FieldOutcome::Skip => {}
        }
    }

    let mut lines: Vec<String> = Vec::new();
    lines.extend(todo_lines);
    for target_method in &order {
        let pairs = groups.get(target_method).unwrap();
        // a holder of the fighter's own script (`__attackControl`, ...) is only its own: written
        // through another receiver (dedede's inhale writes `it.character`), control goes to the
        // engine's own field and the rest has nothing to answer it there
        if target_method.starts_with("__") && !receiver.split('.').all(|r| r.is_empty() || r == "self") {
            if target_method == "__attackControl" {
                if let Some((_, v)) = pairs.iter().find(|(f, _)| f == "on") {
                    lines.push(format!("{receiver}.updateAnimationStats({{ allowMovement: {v} }});"));
                }
            } else {
                let joined: Vec<String> = pairs.iter().map(|(f, v)| format!("{}: {}", f, v)).collect();
                lines.push(format!("// [only the fighter's own script holds this] {receiver}.{target_method}({{ {} }});", joined.join(", ")));
            }
            continue;
        }
        let joined: Vec<String> = pairs.iter().map(|(f, v)| format!("{}: {}", f, v)).collect();
        lines.push(format!("{}.{}({{ {} }});", receiver, target_method, joined.join(", ")));
    }

    if lines.is_empty() {
        // Everything skipped (or source object was empty). Drop the
        // source line entirely so the output has no stray no-op.
        return String::new();
    }
    lines.join(&format!("\n{}", indent))
}

/// Resolve a Detailed field mapping when `skip_if_value` didn't apply.
/// Handles the value_map lookup + target routing.
/// A split field's value in this engine's units (see `CallSplitFieldMapping::Detailed::unit`).
fn convert_split_unit(unit: &str, value: &str) -> Result<String, String> {
    let Ok(v) = value.parse::<f64>() else { return Err(format!("{unit} value is not a plain number; convert by hand")) };
    let sc = &crate::mappings::character_stats().scaling;
    let out = match unit {
        "speed" => v * sc.velocity_scale(),
        "accel" => v * sc.accel_scale(),
        "decay" if v < 0.0 => -v * sc.accel_scale(),
        "decay" => return Err("a decay the source states as positive is not a per-frame slowdown; convert by hand".into()),
        other => return Err(format!("unknown unit {other}")),
    };
    Ok(format!("{out:.4}"))
}

fn resolve_detailed(
    field_name: &str,
    raw_value: &str,
    trimmed_value: &str,
    target: &Option<String>,
    value_map: &std::collections::BTreeMap<String, String>,
    todo: &Option<String>,
    parse_target: &dyn Fn(&str) -> Option<(String, String)>,
) -> FieldOutcome {
    if !value_map.is_empty() {
        match value_map.get(trimmed_value) {
            Some(mapped_value) => match target.as_deref().and_then(parse_target) {
                Some((tm, ff)) => FieldOutcome::Route {
                    target_method: tm, fm_field: ff, value: mapped_value.clone(),
                    todo_note: todo.clone(),
                },
                None => FieldOutcome::Todo {
                    src_name: field_name.to_string(),
                    src_value: raw_value.to_string(),
                    note: todo.clone().or_else(|| Some("value_map matched but no target defined".to_string())),
                },
            },
            None => FieldOutcome::Todo {
                src_name: field_name.to_string(),
                src_value: raw_value.to_string(),
                note: todo.clone().or_else(|| Some("value not in value_map".to_string())),
            },
        }
    } else if let Some(t) = target {
        match parse_target(t) {
            Some((tm, ff)) => FieldOutcome::Route {
                target_method: tm, fm_field: ff, value: raw_value.to_string(),
                todo_note: todo.clone(),
            },
            None => FieldOutcome::Todo {
                src_name: field_name.to_string(),
                src_value: raw_value.to_string(),
                note: todo.clone().or_else(|| Some("malformed target".to_string())),
            },
        }
    } else {
        FieldOutcome::Todo {
            src_name: field_name.to_string(),
            src_value: raw_value.to_string(),
            note: todo.clone(),
        }
    }
}

/// Parse the inside of an object literal (everything between `{` and `}`,
/// exclusive) into `(field_name, value_expr)` pairs. Tracks brace/paren/
/// bracket depth + string state so nested objects, function calls in
/// values, and quoted commas don't break field splitting.
fn parse_object_fields(body: &str) -> Vec<(String, String)> {
    let chars: Vec<char> = body.chars().collect();
    let mut out: Vec<(String, String)> = Vec::new();
    let mut i = 0usize;
    while i < chars.len() {
        while i < chars.len() && (chars[i].is_whitespace() || chars[i] == ',') { i += 1; }
        if i >= chars.len() { break; }
        // Quoted key form: `"foo bar": value`. Accept the same set of
        // characters as inside any string literal — read until the closing
        // quote (honouring backslash escapes), then proceed to the `:`
        // separator as with bare identifier keys.
        let field_name: String = if chars[i] == '"' || chars[i] == '\'' {
            let quote = chars[i];
            i += 1;
            let mut name = String::new();
            while i < chars.len() && chars[i] != quote {
                if chars[i] == '\\' {
                    i += 1;
                    if i >= chars.len() { break; }
                    name.push(chars[i]);
                    i += 1;
                    continue;
                }
                name.push(chars[i]);
                i += 1;
            }
            if i < chars.len() && chars[i] == quote { i += 1; }
            name
        } else {
            let name_start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') { i += 1; }
            chars[name_start..i].iter().collect()
        };
        if field_name.is_empty() { break; }
        while i < chars.len() && chars[i].is_whitespace() { i += 1; }
        if i >= chars.len() || chars[i] != ':' { break; }
        i += 1;
        while i < chars.len() && chars[i].is_whitespace() { i += 1; }
        let val_start = i;
        let mut depth = 0i32;
        let mut in_str: Option<char> = None;
        while i < chars.len() {
            let c = chars[i];
            if let Some(q) = in_str {
                if c == '\\' { i += 1; if i < chars.len() { i += 1; } continue; }
                if c == q { in_str = None; }
            } else {
                match c {
                    '"' | '\'' => in_str = Some(c),
                    '{' | '[' | '(' => depth += 1,
                    '}' | ']' | ')' => depth -= 1,
                    ',' if depth == 0 => break,
                    _ => {}
                }
            }
            i += 1;
        }
        let value: String = chars[val_start..i].iter().collect::<String>().trim().to_string();
        if !value.is_empty() {
            out.push((field_name, value));
        }
    }
    out
}

/// Walk backward from a `.<method>(` position to find the start of the
/// receiver expression. Accepts identifier chars, `.`, balanced `[…]`
/// and `(…)` (for method-chain / index expressions).
fn find_receiver_start(code: &str, dot_pos: usize) -> usize {
    let bytes = code.as_bytes();
    let mut i = dot_pos;
    loop {
        if i == 0 { return 0; }
        let prev = bytes[i - 1];
        if prev.is_ascii_alphanumeric() || prev == b'_' || prev == b'.' {
            i -= 1;
        } else if prev == b']' {
            match find_matching_open(code, i - 1, b'[', b']') {
                Some(open) => i = open,
                None => return i,
            }
        } else if prev == b')' {
            match find_matching_open(code, i - 1, b'(', b')') {
                Some(open) => i = open,
                None => return i,
            }
        } else {
            return i;
        }
    }
}

/// Walk forward from an opening bracket/paren/brace to find the matching
/// close. Tracks string state so quoted delimiters don't disturb the count.
/// SSF2 `RECV.addEffectToList(effect, owner)` registers an already-built effect on an
/// entity's effect list, the one its `removeAllEffects` clears. The effect goes through
/// `__addFx`, which hands it to the fighter's own `addEffectToList` where it defines one and
/// otherwise keeps it as it is; the receiver and the owner arg are dropped. A plain
/// string strip of `.addEffectToList(` would leave that `)` (and `, owner`) dangling,
/// which is a parse error the engine swallows silently. #13.
fn rewrite_add_effect_to_list(code: &str) -> String {
    // Match the call name with OR without a receiver: `recv.addEffectToList(` and a
    // bare `addEffectToList(` (the latter appears when SSF2's receiver was stripped
    // upstream) both leave a dangling `)` + owner arg if not unwrapped.
    const NEEDLE: &str = "addEffectToList(";
    let mut out = String::with_capacity(code.len());
    let mut cursor = 0;
    while let Some(rel) = code[cursor..].find(NEEDLE) {
        let name_start = cursor + rel;
        let prev = code[..name_start].chars().last();
        // Skip if it's part of a longer identifier (e.g. removeAddEffectToList).
        if prev.map(|c| c.is_alphanumeric() || c == '_').unwrap_or(false) {
            out.push_str(&code[cursor..name_start + NEEDLE.len()]);
            cursor = name_start + NEEDLE.len();
            continue;
        }
        // Skip a function DEFINITION: `function addEffectToList(arg0) {` is a decl,
        // not a call. Rewriting it strips the parens and produces invalid hscript
        // (`function /* addEffectToList */ arg0 {`), which fails to parse and takes
        // the whole Script.hx scope down with it.
        let before = code[..name_start].trim_end();
        let is_fn_def = before.strip_suffix("function")
            .map(|b| b.is_empty() || !b.chars().last().is_some_and(|c| c.is_alphanumeric() || c == '_'))
            .unwrap_or(false);
        if is_fn_def {
            out.push_str(&code[cursor..name_start + NEEDLE.len()]);
            cursor = name_start + NEEDLE.len();
            continue;
        }
        let paren_open = name_start + NEEDLE.len() - 1; // index of '('
        let close = match find_matching_close(code, paren_open) {
            Some(c) => c,
            None => {
                out.push_str(&code[cursor..=name_start]);
                cursor = name_start + 1;
                continue;
            }
        };
        // With a `.`, drop the whole receiver chain; a bare call keeps text up to the name.
        let cut_start = if prev == Some('.') {
            find_receiver_start(code, name_start - 1)
        } else {
            name_start
        };
        let arg1 = first_top_level_arg(&code[paren_open + 1..close]).trim();
        out.push_str(&code[cursor..cut_start]);
        out.push_str("__addFx(");
        out.push_str(arg1);
        out.push(')');
        cursor = close + 1;
    }
    out.push_str(&code[cursor..]);
    out
}

/// `__optionOff("x")` used as a number is 0; used through `toString(radix)` it stays the object.
fn option_off_as_number(code: &str) -> String {
    let kept = crate::re!(r#"__optionOff\("([^"]+)"\)\.toString\("#).replace_all(code, "__optionOffStr(\"$1\").toString(").into_owned();
    crate::re!(r#"__optionOff\("[^"]+"\)"#).replace_all(&kept, "0").into_owned()
}

/// SSF2 `[recv.]hitTestGround(x, y[, opts])` → FM
/// `hitTestStructuresWithLineSegment(new Point(x, y), new Point(x, y), null, null)`.
/// SSF2 tests a single point and returns the platform-or-null; FM tests a line
/// segment and returns an Array, so:
///   - the point becomes a degenerate (x,y)→(x,y) segment (TODO: a short vertical
///     span may be more faithful for ground detection),
///   - the optional SSF2 options object is dropped (FM
///     StructureColliderSearchOptions differs), and
///   - an immediately-following `!= null` / `== null` is rewritten to `.length > 0`
///     / `.length == 0` (an Array is never null, so a raw null-check would be
///     silently always-true). A bare boolean use gets `.length > 0`.
/// Balanced-scan so nested-paren / object args don't break the split.
fn rewrite_hit_test_ground(code: &str) -> String {
    const NEEDLE: &str = "hitTestGround(";
    if !code.contains(NEEDLE) {
        return code.to_string();
    }
    let mut out = String::with_capacity(code.len());
    let mut cursor = 0;
    while let Some(rel) = code[cursor..].find(NEEDLE) {
        let start = cursor + rel;
        let paren_open = start + NEEDLE.len() - 1;
        let close = match find_matching_close(code, paren_open) {
            Some(c) => c,
            None => { out.push_str(&code[cursor..=start]); cursor = start + 1; continue; }
        };
        let args = &code[paren_open + 1..close];
        let a1 = first_top_level_arg(args).trim();
        let rest = args[first_top_level_arg(args).len()..].trim_start_matches([',', ' ']);
        let a2 = first_top_level_arg(rest).trim();
        if a1.is_empty() || a2.is_empty() {
            out.push_str(&code[cursor..=start]); cursor = start + 1; continue;
        }
        // Look at what immediately follows the call for a null comparison.
        let after = code[close + 1..].trim_start();
        // the engine answers with nothing when nothing was hit, so every form of the test has to
        // survive that answer as well as an empty one
        let (found, consume) = if let Some(r) = after.strip_prefix("!= null") {
            (true, code[close + 1..].len() - r.len())
        } else if let Some(r) = after.strip_prefix("== null") {
            (false, code[close + 1..].len() - r.len())
        } else {
            (true, 0)
        };
        out.push_str(&code[cursor..start]);
        // the source asked its api object; here the asking is an object's own work, so drop
        // whatever receiver the renames left in front and let the running object do it
        while out.ends_with('.') {
            out.pop();
            while out.chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == '_') { out.pop(); }
            if out.ends_with("()") { out.pop(); out.pop(); }
        }
        // a point has no length to cross anything with, so the segment spans the pixel it names
        let call = format!(
            "self.hitTestStructuresWithLineSegment(new Point({a1}, ({a2}) - 1), new Point({a1}, ({a2}) + 1), null, null)"
        );
        let test = if found {
            format!("({call} != null && {call}.length > 0)")
        } else {
            format!("({call} == null || {call}.length == 0)")
        };
        out.push_str(&format!(
            "{test} /*TODO: SSF2 point ground-test -> FM line segment; verify segment & options*/"
        ));
        cursor = close + 1 + consume;
    }
    out.push_str(&code[cursor..]);
    out
}

/// AS3 `self.Number(x)` (a global cast the decompiler mis-attached to `self`, so it
/// resolves to null at runtime) → Haxe `Std.parseFloat(Std.string(x))`, which converts
/// strings AND already-numeric values to Float. `Std` is in character-script scope
/// (live-probed). Balanced-scan so nested-paren args don't break the wrap.
fn rewrite_number_cast(code: &str) -> String {
    const NEEDLE: &str = "self.Number(";
    if !code.contains(NEEDLE) {
        return code.to_string();
    }
    let mut out = String::with_capacity(code.len());
    let mut cursor = 0;
    while let Some(rel) = code[cursor..].find(NEEDLE) {
        let start = cursor + rel;
        let paren_open = start + NEEDLE.len() - 1;
        let close = match find_matching_close(code, paren_open) {
            Some(c) => c,
            None => { out.push_str(&code[cursor..=start]); cursor = start + 1; continue; }
        };
        let inner = code[paren_open + 1..close].trim();
        out.push_str(&code[cursor..start]);
        out.push_str(&format!("Std.parseFloat(Std.string({inner}))"));
        cursor = close + 1;
    }
    out.push_str(&code[cursor..]);
    out
}

/// SSF2Utils.decel(value, amount) → inline decay-toward-zero. SSF2's helper
/// reduces |value| by amount, clamped at 0 (never crosses sign). Fraymakers'
/// Math has no `decel`, so emit the equivalent ternary using Math.min/max
/// (both present in FM). Balanced-scan so nested-paren args (e.g.
/// Math.abs(Math.calculateXVelocity(...))) split correctly.
/// `RECV.angleControl(speed, angle)` -> set the projectile's velocity to `speed` at `angle`. SSF2's
/// angleControl drives a projectile along an aimed heading (black mage's meteor drops on release,
/// game and watch/kirby/ness/pikachu/chibi/tails aim a shot); there is no single Fraymakers call, so
/// it becomes the two velocity writes the source's version makes, through the engine's own trig
/// helper. the angle is SSF2 degrees (0 right, 90 up); the speed is a source speed, so it scales.
/// Without this the call was struck and the projectile kept whatever velocity it had, so a meteor
/// that should drop just hovered.
fn rewrite_angle_control_calls(code: &str) -> String {
    use std::sync::LazyLock;
    static RE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"([A-Za-z_][\w.()\[\]]*)\.angleControl\(").unwrap());
    let vs = crate::mappings::character_stats().scaling.velocity_scale();
    let mut out = String::with_capacity(code.len());
    let mut cursor = 0;
    while let Some(caps) = RE.captures(&code[cursor..]) {
        let whole = caps.get(0).unwrap();
        let recv = caps.get(1).unwrap().as_str().to_string();
        let mstart = cursor + whole.start();
        let paren_open = cursor + whole.end() - 1;
        let close = match find_matching_close(code, paren_open) {
            Some(c) => c,
            None => { out.push_str(&code[cursor..=mstart]); cursor = mstart + 1; continue; }
        };
        let args = &code[paren_open + 1..close];
        let speed = first_top_level_arg(args).trim().to_string();
        let angle = args[first_top_level_arg(args).len()..].trim_start_matches([',', ' ']).trim();
        if angle.is_empty() {
            out.push_str(&code[cursor..=mstart]); cursor = mstart + 1; continue;
        }
        // a magnitude the decompiler could not recover comes through as the literal null. the
        // source's engine ignores a velocity write with an undefined magnitude (its setter guards
        // NaN), leaving whatever launch the spawn already handed the projectile. writing it here
        // instead computes null*scale = 0 and freezes the shot, so drop the write and keep the
        // launch. general to any projectile whose aimed-heading speed is lost, not one by name.
        if speed == "null" {
            out.push_str(&code[cursor..mstart]);
            out.push_str("{ /* angleControl magnitude unrecovered: keep the spawn launch */ }");
            cursor = close + 1;
            continue;
        }
        out.push_str(&code[cursor..mstart]);
        // Math.calculateXVelocity(mag, deg) = mag*cos(deg); calculateYVelocity(mag, deg) =
        // mag*sin(deg). the source's angle counts 90 as UP and writes ySpeed = -mag*sin(deg), so
        // the y is negated to land in this engine's screen-down frame.
        out.push_str(&format!(
            "{{ var _ac = {recv}; if (_ac != null) {{ var _asp = ({speed}) * {vs:.4}; \
             _ac.setXVelocity(Math.calculateXVelocity(_asp, ({angle}))); \
             _ac.setYVelocity(-Math.calculateYVelocity(_asp, ({angle}))); }} }}"));
        cursor = close + 1;
    }
    out.push_str(&code[cursor..]);
    out
}

/// `RECV.getCurrentAttackFrame()` -> `RECV.getAnimation()`, and the SSF2 attack names it is compared
/// against -> the animation names this engine carries (`"b_down"` -> `"special_down"`). SSF2 asks
/// what attack a fighter is running to decide what a projectile riding it does (pit's orbitars stick
/// to pit while pit runs its down special, else they end); here that is the fighter's animation.
/// Without this the call was struck, so the whole test went dead and the projectile neither followed
/// nor ended.
fn rewrite_current_attack_frame(code: &str) -> String {
    use std::sync::LazyLock;
    // a comparison of the call against an attack-name literal, either order
    static CMP_R: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r#"([A-Za-z_][\w.()\[\]]*)\.getCurrentAttackFrame\(\)(\s*[=!]=\s*)"([A-Za-z_]+)""#).unwrap()
    });
    static CMP_L: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r#""([A-Za-z_]+)"(\s*[=!]=\s*)([A-Za-z_][\w.()\[\]]*)\.getCurrentAttackFrame\(\)"#).unwrap()
    });
    let out = CMP_R.replace_all(code, |c: &regex::Captures| {
        format!("{}.getAnimation(){}\"{}\"", &c[1], &c[2], crate::abc_parser::normalize_attack_name(&c[3]))
    });
    let out = CMP_L.replace_all(&out, |c: &regex::Captures| {
        format!("\"{}\"{}{}.getAnimation()", crate::abc_parser::normalize_attack_name(&c[1]), &c[2], &c[3])
    });
    // any remaining bare call (not a name comparison) is just the current animation
    out.replace(".getCurrentAttackFrame()", ".getAnimation()")
}

/// `RECV.addEventListener(SSF2Event.PROJ_COLLIDE, HANDLER)` -> a per-frame check that fires HANDLER
/// when this projectile enters another entity's body. SSF2's PROJ_COLLIDE is the projectile touching
/// a character or projectile; Fraymakers has no such event, and hitTestEntity ignores the owner, so
/// the overlap is read off the other entity's real environmental collision box (its ECB extents) as
/// a point-in-box test of this projectile's position, guarded for an entity that has no ECB. it fires
/// on ENTER, not every frame: the projectile spawns inside its thrower, so that overlap is seeded and
/// only a fresh one (it flew out and came back) fires. link's boomerang is caught returning this way;
/// bomberman, kirby, pacman, simon, waluigi read collisions the same.
fn rewrite_proj_collide_listeners(code: &str) -> String {
    use std::sync::LazyLock;
    static RE: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"([A-Za-z_][\w.()\[\]]*)\.addEventListener\(SSF2Event\.PROJ_COLLIDE,\s*([A-Za-z_][\w.]*)\)").unwrap()
    });
    let out = RE.replace_all(code, |c: &regex::Captures| {
        let (recv, handler) = (&c[1], &c[2]);
        format!(
            "{{ var _pcS = {recv}; var _pcH = {handler}; \
             var _pcHW = 12.0; if (_pcS.getEcbRightHipX != null) {{ var _h = _pcS.getEcbRightHipX(); if (_h != null && _h > 0) {{ _pcHW = _h; }} }} \
             var _pcIn = function(_pcO) {{ \
               if (_pcO == null || _pcO == _pcS || _pcO.isDisposed() || _pcO.getEcbLeftHipX == null) {{ return false; }} \
               var _oL = _pcO.getX() + _pcO.getEcbLeftHipX(); var _oR = _pcO.getX() + _pcO.getEcbRightHipX(); \
               var _oT = _pcO.getY() + _pcO.getEcbHeadY(); var _oB = _pcO.getY() + _pcO.getEcbFootY(); \
               return (_pcS.getX() - _pcHW < _oR) && (_pcS.getX() + _pcHW > _oL) && (_pcS.getY() - _pcHW < _oB) && (_pcS.getY() + _pcHW > _oT); }}; \
             var _pcScan = function(_pcSink) {{ \
               var _pcC = match.getCharacters(); var _pci = 0; while (_pci < _pcC.length) {{ if (_pcIn(_pcC[_pci])) {{ _pcSink(_pcC[_pci]); }} _pci = _pci + 1; }} \
               var _pcP = match.getProjectiles(); _pci = 0; while (_pci < _pcP.length) {{ if (_pcIn(_pcP[_pci])) {{ _pcSink(_pcP[_pci]); }} _pci = _pci + 1; }} }}; \
             var _pcPrev = []; _pcScan(function(_pcO) {{ _pcPrev.push(_pcO); }}); \
             var _pcId = _pcS.addTimer(1, -1, function(?_pct) {{ if (_pcS.isDisposed()) {{ return; }} var _pcNow = []; \
               _pcScan(function(_pcO) {{ _pcNow.push(_pcO); \
                 var _pcWas = false; var _pcj = 0; while (_pcj < _pcPrev.length) {{ if (_pcPrev[_pcj] == _pcO) {{ _pcWas = true; }} _pcj = _pcj + 1; }} \
                 if (!_pcWas) {{ _pcH({{ data: {{ caller: _pcS, self: _pcS, foe: _pcO }} }}); }} }}); \
               _pcPrev = _pcNow; }}, {{ persistent: true }}); \
             if (_pcS.exports.__pcOn == null) {{ _pcS.exports.__pcOn = []; }} _pcS.exports.__pcOn.push({{ h: _pcH, id: _pcId }}); }}")
    }).into_owned();
    // ...and the listener taken off the way the source takes it off. the watch lasts until then
    // (a listener outlives its clip's labels), and it keeps going through its object's end the way
    // the engine's own ending timer does, so the handler can end the object from inside it
    static OFF: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"([A-Za-z_][\w.()\[\]]*)\.removeEventListener\(SSF2Event\.PROJ_COLLIDE,\s*([A-Za-z_][\w.]*)\)").unwrap()
    });
    OFF.replace_all(&out, |c: &regex::Captures| {
        let (recv, handler) = (&c[1], &c[2]);
        format!("{{ var _pcL = {recv}.exports.__pcOn; if (_pcL != null) {{ var _pcK = 0; \
                 while (_pcK < _pcL.length) {{ if (_pcL[_pcK].h == {handler}) {{ {recv}.removeTimer(_pcL[_pcK].id); _pcL.splice(_pcK, 1); }} else {{ _pcK = _pcK + 1; }} }} }} }}")
    }).into_owned()
}

/// `RECV.isReversed()` -> `false`. SSF2 asks whether a projectile was reflected back at its owner;
/// Fraymakers has a reflection listener but no boolean query, and the common case (and the one the
/// callers guard for, `!isReversed()`) is that it was not, so it reads as not-reflected.
fn rewrite_is_reversed(code: &str) -> String {
    use std::sync::LazyLock;
    // receiver is a plain name chain (no parens): this call's whole text is REPLACED by false, so a
    // receiver that reached across an enclosing `(` (as in `__truthy(self.isReversed())`) would drop
    // that paren and unbalance the expression.
    static RE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"[A-Za-z_][\w.]*\.isReversed\(\)").unwrap());
    RE.replace_all(code, "false").into_owned()
}

fn rewrite_decel_calls(code: &str) -> String {
    const NEEDLE: &str = "SSF2Utils.decel(";
    let mut out = String::with_capacity(code.len());
    let mut cursor = 0;
    while let Some(rel) = code[cursor..].find(NEEDLE) {
        let start = cursor + rel;
        let paren_open = start + NEEDLE.len() - 1;
        let close = match find_matching_close(code, paren_open) {
            Some(c) => c,
            None => { out.push_str(&code[cursor..=start]); cursor = start + 1; continue; }
        };
        let args = &code[paren_open + 1..close];
        let a = first_top_level_arg(args).trim();
        let b = args[first_top_level_arg(args).len()..].trim_start_matches([',', ' ']).trim();
        if b.is_empty() {
            // not the 2-arg form — leave untouched
            out.push_str(&code[cursor..=start]); cursor = start + 1; continue;
        }
        out.push_str(&code[cursor..start]);
        out.push_str(&format!("(({a}) < 0 ? Math.min(0, ({a}) + ({b})) : Math.max(0, ({a}) - ({b})))"));
        cursor = close + 1;
    }
    out.push_str(&code[cursor..]);
    out
}

/// Substring of a comma-separated arg list up to the first top-level (depth-0,
/// outside string literals) comma, or the whole string if there is none.
fn first_top_level_arg(args: &str) -> &str {
    let b = args.as_bytes();
    let mut depth = 0i32;
    let mut in_str: Option<u8> = None;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        match in_str {
            Some(q) => {
                if c == b'\\' { i += 2; continue; }
                if c == q { in_str = None; }
            }
            None => match c {
                b'"' | b'\'' => in_str = Some(c),
                b'(' | b'{' | b'[' => depth += 1,
                b')' | b'}' | b']' => depth -= 1,
                b',' if depth == 0 => return &args[..i],
                _ => {}
            },
        }
        i += 1;
    }
    args
}

fn find_matching_close(code: &str, open_pos: usize) -> Option<usize> {
    let bytes = code.as_bytes();
    let mut depth = 1i32;
    let mut i = open_pos + 1;
    let mut in_str: Option<u8> = None;
    while i < bytes.len() {
        let c = bytes[i];
        if let Some(q) = in_str {
            if c == b'\\' { i += 2; continue; }
            if c == q { in_str = None; }
        } else {
            // Skip comments — a `'`/`(`/`)` inside them is text, not code (e.g. an
            // inserted `/* TODO: …don't… */` would otherwise open a phantom string).
            if c == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
                i += 2;
                while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') { i += 1; }
                i += 2;
                continue;
            }
            if c == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                while i < bytes.len() && bytes[i] != b'\n' { i += 1; }
                continue;
            }
            match c {
                b'"' | b'\'' => in_str = Some(c),
                b'(' | b'{' | b'[' => depth += 1,
                b')' | b'}' | b']' => {
                    depth -= 1;
                    if depth == 0 { return Some(i); }
                }
                _ => {}
            }
        }
        i += 1;
    }
    None
}

fn find_matching_open(code: &str, close_pos: usize, open: u8, close: u8) -> Option<usize> {
    let bytes = code.as_bytes();
    let mut depth = 1i32;
    let mut i = close_pos;
    while i > 0 {
        i -= 1;
        if bytes[i] == close { depth += 1; }
        else if bytes[i] == open {
            depth -= 1;
            if depth == 0 { return Some(i); }
        }
    }
    None
}

/// True if `pos` sits after a `//` on the same line (i.e. inside a
/// single-line comment). Used to skip call_splits matches that landed
/// inside comments.
fn line_is_commented_at(code: &str, pos: usize) -> bool {
    let line_start = code[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let line_prefix = &code[line_start..pos];
    let mut in_str: Option<char> = None;
    let chars: Vec<char> = line_prefix.chars().collect();
    let mut j = 0;
    while j < chars.len() {
        let c = chars[j];
        if let Some(q) = in_str {
            if c == '\\' { j += 1; if j < chars.len() { j += 1; } continue; }
            if c == q { in_str = None; }
        } else {
            match c {
                '"' | '\'' => in_str = Some(c),
                '/' if j + 1 < chars.len() && chars[j + 1] == '/' => return true,
                _ => {}
            }
        }
        j += 1;
    }
    false
}

/// SSF2 ext-var names that collide with identifiers already in scope in a Fraymakers
/// script and therefore must NOT be wrapped as persistent state. Wrapping one emits e.g.
/// `var self = self.makeObject(null)`, which shadows the real reference for the rest of the
/// file — silently breaking every `self.foo()` / `match.foo()` call (this is what killed
/// sandbag's down-special: its `dashCheck` ran against a null `self`). These names are
/// dropped from the wrapper declarations and left untouched by the persistent-state pass,
/// so references resolve to the genuine engine-provided `self` / `match`.
/// Names the engine has already bound in the scope a script runs in. Declaring one of these
/// does not shadow it for our own code alone: the engine's OWN scripts read them out of that
/// scope, so a null wrapper under a name it expects breaks code that is nothing to do with us.
/// `camera` is how that shows up -- the engine reaches for it when a fighter dies, finds a slot
/// this character declared, and the error is reported against the engine's script.
pub const RESERVED_EXT_VARS: &[&str] = &["self", "match", "camera"];

/// A write to a field declared `int` (or `uint`) keeps only the whole part in the source, cut
/// toward zero: marth's counter computes `damage * 1.2` into one and hits for the whole number.
/// `__int` cuts the same way (the engine's own whole-number store rounds down instead, which
/// differs below zero, and `Std.int` makes not-a-number the lowest int where the source makes it
/// 0). A whole-number literal is left as it is.
pub fn coerce_int_field_writes(code: &str, declared: &[(String, String)]) -> String {
    let mut out = code.to_string();
    for (name, ty) in declared {
        if ty != "int" && ty != "uint" { continue }
        if !out.contains(&format!("self.{name}")) { continue }
        out = coerce_one_int_field(&out, name);
    }
    out
}

fn coerce_one_int_field(code: &str, name: &str) -> String {
    let re = crate::dyn_re(&format!(r"(^|[^\w.])self\.{}\s*([-+*/]?)=([^=])", regex::escape(name)));
    let mut out = String::with_capacity(code.len() + 16);
    let mut cursor = 0usize;
    while let Some(c) = re.captures(&code[cursor..]) {
        let m = c.get(0).unwrap();
        let op = c.get(2).map(|g| g.as_str()).unwrap_or("");
        let rhs_start = cursor + c.get(3).unwrap().start();
        // the value runs to the statement's end: the first `;` outside brackets and strings
        let bytes = code.as_bytes();
        let (mut q, mut depth, mut quote) = (rhs_start, 0i32, None::<u8>);
        let mut end = None;
        while q < bytes.len() {
            let ch = bytes[q];
            if let Some(qc) = quote {
                if ch == b'\\' { q += 2; continue; }
                if ch == qc { quote = None; }
            } else {
                match ch {
                    b'"' | b'\'' => quote = Some(ch),
                    b'(' | b'[' | b'{' => depth += 1,
                    b')' | b']' | b'}' => { if depth == 0 { end = Some(q); break; } depth -= 1; }
                    b';' | b'\n' if depth == 0 => { end = Some(q); break; }
                    _ => {}
                }
            }
            q += 1;
        }
        let end = end.unwrap_or(bytes.len());
        let value = code[rhs_start..end].trim();
        let lhs_start = cursor + m.start() + c.get(1).unwrap().len();
        out.push_str(&code[cursor..lhs_start]);
        let whole = value.parse::<i64>().is_ok() || value.starts_with("__int(");
        if value.is_empty() || (op.is_empty() && whole) {
            out.push_str(&code[lhs_start..end]);
        } else if op.is_empty() && value == "null" {
            // nothing, stored in a whole-number field, is zero
            out.push_str(&format!("self.{name} = 0"));
        } else if op.is_empty() {
            out.push_str(&format!("self.{name} = __int({value})"));
        } else {
            out.push_str(&format!("self.{name} = __int(self.{name} {op} ({value}))"));
        }
        cursor = end;
    }
    out.push_str(&code[cursor..]);
    out
}

/// Translate SSF2's "repeat forever" timer sentinel to Fraymakers' spelling.
///
/// SSF2's `createTimer`/`addTimer(delay, repeatCount, callback)` follows the
/// AS3 `flash.utils.Timer` convention where a non-positive `repeatCount` (`0`
/// is how SSF2 character scripts spell it) means "repeat indefinitely".
/// Fraymakers reserves `-1` for an infinite timer; a literal `0` there fires
/// zero/one time and then silently stops. A polling timer copied verbatim —
/// e.g. sandbag's down-special `addTimer(2, 0, dashCheck)`, which keeps
/// reading held controls until a direction is pressed and only then removes
/// itself — therefore never polls, so the move applies no momentum and ignores
/// button presses. Rewrite the `0` repeat count to `-1`.
///
/// A `0`-repeat timer is meaningless in Fraymakers (you would not register a
/// timer to fire zero times), so every `0` we see here is an SSF2 infinite
/// sentinel — the rewrite is unconditional and safe. Positive counts and an
/// already-correct `-1` are left untouched.
fn fix_timer_infinite_repeat(code: String) -> String {
    use regex::Regex;
    // addTimer( <delay> , 0 , …  →  addTimer( <delay> , -1 , …
    // `[^,()]+` matches a simple delay arg (number or identifier), mirroring
    // `neutralize_getter_callbacks`; it deliberately skips delays that contain
    // a nested call (rare for a timer delay).
    static RE_REPEAT: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let re = RE_REPEAT.get_or_init(|| Regex::new(r"(addTimer\(\s*[^,()]+,\s*)0(\s*,)").unwrap());
    let code = re.replace_all(&code, "${1}-1${2}").into_owned();
    // A timer's last argument is an OPTIONS OBJECT on both sides -- SSF2 content builds one with
    // the same `persistent` / `hitStunPause` fields -- but content also passes a bare boolean
    // there. Handed to Fraymakers as it stands the call dies on the spot ("Can't cast bool"), and
    // it takes the timer with it: mario's jab lost the poll that reads the button for jab2 and the
    // one that chains into it. A boolean in the options slot is the persistence flag, and false
    // (which is what content passes) means the same thing either way -- no options.
    static RE_OPTS: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let opts = RE_OPTS.get_or_init(|| Regex::new(r"(addTimer\([^()]*,\s*)(true|false)(\s*\))").unwrap());
    opts.replace_all(&code, "${1}{ persistent: ${2} }${3}").into_owned()
}

/// Take the text back off a value that is being done ARITHMETIC to.
///
/// The source asks a value to say what it is and then adds it to a sentence; a decompiler that
/// loses the brackets around a subtraction leaves the asking on the wrong side of it, and
/// `x - Std.string(y)` is a subtraction of a sentence, which the engine refuses. Nothing is ever
/// concatenated with a minus, a times or a divide, so the text is not wanted there whatever the
/// brackets looked like.
/// Put back the brackets around a subtraction inside a sentence.
///
/// `"at: " + x - offset` cannot mean what it says: nothing is subtracted FROM a sentence, so the
/// source wrote `"at: " + (x - offset)` and the decompiler flattened it. Reading it as written
/// asks the engine to take a number off a string, which it refuses -- and this is usually a
/// trace, so it refuses once per frame forever. Only where a sentence is actually being built:
/// a line with no text in it is arithmetic and means exactly what it says.
pub fn subtraction_inside_a_sentence(code: &str) -> String {
    use regex::Regex;
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(
        r#"\+ ([A-Za-z_][\w.]*(?:\([^()]*\))?(?:\[[^\]]*\])?) - ([A-Za-z_][\w.]*(?:\([^()]*\))?(?:\[[^\]]*\])?)"#)
        .expect("sentence subtraction regex"));
    let mut out = String::with_capacity(code.len());
    for l in code.lines() {
        if l.contains('"') && l.contains(" - ") {
            out.push_str(&re.replace_all(l, "+ ($1 - $2)"));
        } else {
            out.push_str(l);
        }
        out.push('\n');
    }
    if !code.ends_with('\n') { while out.ends_with('\n') { out.pop(); } }
    out
}

pub fn arithmetic_is_never_text(code: &str) -> String {
    use regex::Regex;
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(
        r"([-*/])(\s*)Std\.string\(([^()]*(?:\([^()]*\)[^()]*)*)\)").expect("arith text regex"));
    re.replace_all(code, "$1$2$3").into_owned()
}

/// Asking a value to say what it is. The source lets everything answer that; here only some
/// things carry the name, and a number or a flag asked for it answers with nothing, which is
/// then called. The standard library will say what anything is, so ask it instead.
pub fn say_what_a_value_is(code: &str) -> String {
    const NEEDLE: &str = ".toString()";
    if !code.contains(NEEDLE) { return code.to_string() }
    let mut out = String::with_capacity(code.len());
    let mut cursor = 0;
    while let Some(rel) = code[cursor..].find(NEEDLE) {
        let dot = cursor + rel;
        // walk back over the expression the name is asked of, brackets and all
        let b = code.as_bytes();
        let mut i = dot;
        let mut depth = 0usize;
        while i > 0 {
            let c = b[i - 1];
            match c {
                b')' | b']' => depth += 1,
                b'(' | b'[' => { if depth == 0 { break } depth -= 1 }
                _ if depth == 0 && !(c.is_ascii_alphanumeric() || c == b'_' || c == b'.') => break,
                _ => {}
            }
            i -= 1;
        }
        if i == dot {
            out.push_str(&code[cursor..dot + NEEDLE.len()]);
            cursor = dot + NEEDLE.len();
            continue;
        }
        out.push_str(&code[cursor..i]);
        out.push_str(&format!("Std.string({})", &code[i..dot]));
        cursor = dot + NEEDLE.len();
    }
    out.push_str(&code[cursor..]);
    out
}

/// What holds the running object's art. The source reaches one step up the display list to
/// move the whole thing; here the object IS the whole thing, so the reach resolves to itself.
/// Only position and scale carry over -- the rest of what a display list answers to has no
/// counterpart, and a line built on it is struck rather than left to fail.
pub fn resolve_display_parent(code: &str) -> String {
    use regex::Regex;
    if !code.contains("parent.") { return code.to_string() }
    static SET: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    static GET: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let set = SET.get_or_init(|| Regex::new(r"\bparent\.([xy])\s*=\s*([^;\n]+)").unwrap());
    let mut out = set.replace_all(code, |c: &regex::Captures| {
        let axis = if &c[1] == "x" { "X" } else { "Y" };
        format!("self.set{axis}({})", c[2].trim())
    }).into_owned();
    let get = GET.get_or_init(|| Regex::new(r"\bparent\.([xy])\b").unwrap());
    out = get.replace_all(&out, |c: &regex::Captures| {
        let axis = if &c[1] == "x" { "X" } else { "Y" };
        format!("self.get{axis}()")
    }).into_owned();
    // scale is the object's own as well
    out = crate::re!(r"\bparent\.(setScale[XY])\b").replace_all(&out, "self.$1").into_owned();
    out
}

/// Whether this engine answers to a name itself. A summoner and this engine can both have one,
/// and when they do the name is the engine's here: routing it through what the summoner
/// published asks something that never published it, and the ask is refused.
pub fn the_engine_answers_to(name: &str) -> bool {
    let api = crate::mappings::fm_api();
    api.methods.iter().chain(api.game_object_methods.iter())
        .chain(api.match_methods.iter()).chain(api.standard_methods.iter())
        .any(|m| m == name)
}

/// A call put on the match that the match has no name for, but an object does. The source's own
/// api object stands for both, so translating it to the match sends object work to the wrong
/// place; the object doing the asking is the one running the script, so send it there.
pub fn send_object_calls_to_the_object(code: &str) -> String {
    use regex::Regex;
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let api = crate::mappings::fm_api();
    if api.match_methods.is_empty() || api.game_object_methods.is_empty() { return code.to_string() }
    let re = RE.get_or_init(|| Regex::new(r"\bmatch\.(\w+)\s*\(").unwrap());
    let mut out = String::with_capacity(code.len());
    let mut cursor = 0;
    for c in re.captures_iter(code) {
        let name = &c[1];
        let m = c.get(0).unwrap();
        let misplaced = !api.match_methods.iter().any(|s| s == name)
            && api.game_object_methods.iter().any(|s| s == name);
        out.push_str(&code[cursor..m.start()]);
        if misplaced { out.push_str(&format!("self.{name}(")); } else { out.push_str(m.as_str()); }
        cursor = m.end();
    }
    out.push_str(&code[cursor..]);
    out
}

/// Net `{` minus `}` on a single line, ignoring braces inside string literals and
/// after a `//` line comment. Drives block-aware comment-out so a multi-line dead
/// block (condition + body + close) is fully neutralized, not just its first line.
fn net_brace_delta(line: &str) -> i32 {
    let b = line.as_bytes();
    let mut depth = 0i32;
    let mut in_str: Option<u8> = None;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        match in_str {
            Some(q) => {
                if c == b'\\' { i += 2; continue; }
                if c == q { in_str = None; }
            }
            None => match c {
                b'/' if i + 1 < b.len() && b[i + 1] == b'/' => break,
                b'"' | b'\'' => in_str = Some(c),
                b'{' => depth += 1,
                b'}' => depth -= 1,
                _ => {}
            },
        }
        i += 1;
    }
    depth
}

pub fn comment_out_unknown_calls(code: &str) -> String {
    let cfg = crate::mappings::api_commands();
    // Build the `.NAME(` match strings from the JSON ssf2_only list.
    // a STAGE's reaches through its own planes and to "the stage" are resolved later, by the
    // stage emitter (a plane child becomes the object it turned into, the match's stage is the
    // script itself); struck here, the resolved line and the whole block under it stayed dead
    // (race to the finish's regions, every castle siege transition reached through getStage)
    let stage_resolves = |name: &str| SELF_IS_STAGE.with(|c| c.get())
        && matches!(name, "getMidground" | "getStage");
    let markers: Vec<(String, &str)> = cfg.ssf2_only.iter()
        .filter(|e| !stage_resolves(&e.name))
        .map(|e| (format!(".{}(", e.name), e.name.as_str()))
        .collect();
    // WHY a line is a comment, said where the line is. A struck call reads as an oversight
    // otherwise, and the reason is already written down next to the call in the mapping: an audit
    // of what a port does not carry should not have to go and look it up.
    let why = |name: &str| -> Option<String> {
        let n = cfg.ssf2_only.iter().find(|e| e.name == name)?.note.trim().to_string();
        let n = n.trim_end_matches("ssf2_only.").trim().trim_end_matches('.').to_string();
        (!n.is_empty()).then_some(n)
    };

    // Names defined as local functions in THIS script are NOT SSF2-only — a call
    // to a real local function is valid and must not be commented out (e.g. mario
    // defines `function jumpToContinue(...)` and also uses it as a COLLIDE_FLOOR
    // listener; commenting its direct call broke the landing/continue transition).
    let local_fns: std::collections::HashSet<String> = {
        static RE_FNDEF: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        let re = RE_FNDEF.get_or_init(|| regex::Regex::new(r"(?m)^\s*function\s+(\w+)\s*\(").unwrap());
        re.captures_iter(code).map(|c| c[1].to_string()).collect()
    };

    let lines: Vec<&str> = code.lines().collect();
    let mut out = Vec::with_capacity(lines.len());
    let mut ssf2_hits: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();
        if trimmed.starts_with("//") { out.push(line.to_string()); i += 1; continue; }
        // a call on one of the port's OWN handles (`__clip.stop()`, the sound the play handed
        // back) is the port's, whatever the name: only a call on something else is struck
        let own_receiver = |m: &str| line.match_indices(m).all(|(at, _)| {
            let before = &line[..at];
            let ident_start = before.rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).map(|i| i + 1).unwrap_or(0);
            before[ident_start..].starts_with("__")
        });
        let hit = markers.iter().find(|(m, name)| line.contains(m) && !local_fns.contains(*name) && !own_receiver(m));
        if let Some((_, name)) = hit {
            let indent = &line[..line.len() - line.trim_start().len()];
            *ssf2_hits.entry((*name).to_string()).or_insert(0) += 1;
            // If this statement OPENS a `{` block (e.g. `if (self.isCPU()) {`),
            // the block body was guarded by a now-unsupported call, so it is dead in
            // FM. Comment the whole block through its matching `}` — leaving the body
            // or the close brace live would produce unconditional code or an orphaned
            // `}` (a parse error the engine swallows silently). #13.
            let mut depth = net_brace_delta(line);
            if let Some(w) = why(name) { out.push(format!("{indent}// {w}")); }
            out.push(format!("{}// [SSF2-only: {}] {}", indent, name, trimmed));
            i += 1;
            while depth > 0 && i < lines.len() {
                let cont = lines[i];
                let cont_indent = &cont[..cont.len() - cont.trim_start().len()];
                let cont_trimmed = cont.trim();
                if cont_trimmed.starts_with("//") {
                    out.push(cont.to_string());
                } else {
                    out.push(format!("{}// [SSF2-dead] {}", cont_indent, cont_trimmed));
                }
                depth += net_brace_delta(cont);
                i += 1;
            }
        } else {
            out.push(line.to_string());
            i += 1;
        }
    }
    let mut joined = out.join("\n");
    if code.ends_with('\n') { joined.push('\n'); }

    // Always accrue counts into the conversion log (the JSON file is written
    // unconditionally at the end of the run). The debug-level log line is
    // separately gated inside log_unknown_calls.
    if !ssf2_hits.is_empty() {
        let mut log = conversion_log().lock().unwrap();
        for (name, n) in ssf2_hits {
            *log.ssf2_only.entry(name).or_insert(0) += n;
        }
    }
    log_unknown_calls(&joined, cfg);
    joined
}

/// Un-comment `// [SSF2-only: NAME] <stmt>` lines where NAME is actually defined
/// as a local `function NAME(...)` in the ASSEMBLED script. comment_out_unknown_calls
/// runs per-method, so it can't see a sibling function definition and wrongly flags a
/// valid call to a real local function (e.g. mario's `jumpToContinue`, also used as a
/// COLLIDE_FLOOR listener). This whole-file post-pass restores those calls.
pub fn uncomment_local_fn_calls(code: &str) -> String {
    static RE_DEF: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let def_re = RE_DEF.get_or_init(|| regex::Regex::new(r"(?m)^\s*function\s+(\w+)\s*\(").unwrap());
    let locals: std::collections::HashSet<String> =
        def_re.captures_iter(code).map(|c| c[1].to_string()).collect();
    uncomment_calls_to(code, &locals)
}

/// A call struck as `[SSF2-only: NAME]` restored where NAME is one of the given functions: a
/// frame script runs in its script's scope, so a call to a function the script defines answers
/// there even though the frame defines nothing.
pub fn uncomment_calls_to(code: &str, locals: &std::collections::HashSet<String>) -> String {
    uncomment_calls_to_with(code, locals, &Default::default())
}

/// [`uncomment_calls_to`] where the line may also call names of the scope it runs in (`also`: the
/// fighter's own functions, which a frame calls by name) and the port's own helpers (`_`-named):
/// sonic's frames set his foot blur part from one of his functions and stayed struck for it
pub fn uncomment_calls_to_with(code: &str, locals: &std::collections::HashSet<String>, also: &std::collections::HashSet<String>) -> String {
    if locals.is_empty() { return code.to_string(); }
    static RE_MARKED: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let line_re = RE_MARKED.get_or_init(|| regex::Regex::new(r"^(\s*)// \[SSF2-only: (\w+)\] (.*)$").unwrap());
    let mut restored = 0usize;
    let mut joined = code.lines().map(|line| {
        if let Some(c) = line_re.captures(line) {
            // the name has to be the thing the line names, not a field of something else (a part
            // called `top` is not the `top` of a rectangle the line reads), and the line must ask
            // nothing else: a line that also calls something the engine cannot answer stays struck
            // for that, not restored for this
            let own = crate::dyn_re(&format!(r"(^|[^.\w])(self\.)?{}\b", regex::escape(&c[2])));
            let only_its_own = || {
                static CALL: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
                let call = CALL.get_or_init(|| regex::Regex::new(r"(?:(\w+)\s*\.\s*)?(\w+)\s*\(").expect("regex"));
                call.captures_iter(&c[3]).all(|m| {
                    let (recv, name) = (m.get(1).map(|r| r.as_str()), &m[2]);
                    // its own name, a call on it, another name of this script's, or a slot read
                    name == &c[2] || recv == Some(&c[2]) || locals.contains(name)
                        || name == "get" || name == "set"
                        || (recv.is_none() && (also.contains(name) || (!also.is_empty() && name.starts_with('_'))))
                })
            };
            // the block's own shape (`} else {`) carries no call and belongs with the lines it
            // holds: struck on its own it hands the else's body to the if (ryu's trails then
            // showed for one costume and both branches ran for the other)
            let structure = matches!(c[3].trim(), "}" | "{" | "} else {" | "else {" | "} else" );
            if locals.contains(&c[2]) && (structure || (own.is_match(&c[3]) && only_its_own())) {
                restored += 1;
                return format!("{}{}", &c[1], &c[3]);
            }
        }
        line.to_string()
    }).collect::<Vec<_>>().join("\n");
    if code.ends_with('\n') { joined.push('\n'); }
    if restored > 0 {
        log::info!("uncomment_local_fn_calls: restored {} call(s) to locally-defined functions", restored);
    }
    joined
}

/// Comment out any `}` whose matching `{` lives on a commented-out line. When a
/// translator neutralizes a block-opening statement (`if (self.getMC()...) {` becomes
/// `// [SSF2-only: getMC] if (...) {`), the opening brace is now inside a comment but
/// the matching `}` is still live code, so the script no longer parses and the engine
/// drops the whole handler silently. This whole-file post-pass walks braces in source
/// order on a stack tagged by whether each opener was dead, and comments any live close
/// that matches a dead open. Iterates to a fixpoint to absorb nested cascades. #13.
/// Give back a closing brace that was struck out from under a LIVE block.
///
/// Striking a line takes its brackets with it. Where the line that OPENED the block was struck
/// too, that is right and `balance_commented_blocks` handles the other direction. Where the
/// opener is still live, the block never closes, the script runs off the end of the file, and the
/// engine -- which drops a script it cannot parse WHOLE -- throws away every object that stage
/// was going to make. Six stages ended that way.
///
/// A REPAIR, not a rule: it looks at the finished script, and if the braces already balance it
/// does nothing at all. So it cannot disturb a script that was fine.
pub fn restore_struck_closers(code: &str) -> String {
    // WHERE it is owed, not how many. Counting the file over and handing the closers back to the
    // first struck line that happens to close something puts the brace in the wrong block: DAT61
    // got one inside a dead if/else, which ended the function early and left the function's own
    // `}` as a token nothing expected -- so the script did not parse, and the stage lost every
    // object it makes. The file balanced, which is why counting could not see it.
    //
    // A block belongs to whoever OPENED it. Walking the braces in order and remembering, for each
    // open, whether the line that opened it was live, says exactly which closers are owed: the
    // ones where a LIVE block is closed by a line that was struck. Those, and no others.
    let dead_line = |l: &str| l.trim_start().starts_with("//");
    // the braces that stand for structure: on a struck line, the ones the marker hides; on a live
    // one, the ones outside strings and before any trailing comment
    fn structural(line: &str, dead: bool) -> Vec<bool> {
        let b = line.as_bytes();
        let mut v = Vec::new();
        if dead {
            for &c in b { if c == b'{' { v.push(true) } else if c == b'}' { v.push(false) } }
            return v;
        }
        let mut in_str: Option<u8> = None;
        let mut i = 0;
        while i < b.len() {
            let c = b[i];
            match in_str {
                Some(q) => {
                    if c == b'\\' { i += 2; continue }
                    if c == q { in_str = None }
                }
                None => match c {
                    b'/' if i + 1 < b.len() && b[i + 1] == b'/' => break,
                    b'"' | b'\'' => in_str = Some(c),
                    b'{' => v.push(true),
                    b'}' => v.push(false),
                    _ => {}
                },
            }
            i += 1;
        }
        v
    }
    let mut opened_live: Vec<bool> = Vec::new();
    let mut owed_at: std::collections::BTreeMap<usize, usize> = std::collections::BTreeMap::new();
    for (idx, line) in code.lines().enumerate() {
        let dead = dead_line(line);
        for is_open in structural(line, dead) {
            if is_open { opened_live.push(!dead); continue }
            // a close with nothing open above it is someone else's problem, not this one's
            let Some(was_live) = opened_live.pop() else { continue };
            if was_live && dead { *owed_at.entry(idx).or_insert(0) += 1; }
        }
    }
    if owed_at.is_empty() { return code.to_string() }
    let mut out = String::with_capacity(code.len());
    for (idx, line) in code.lines().enumerate() {
        if let Some(&give) = owed_at.get(&idx) {
            let indent = &line[..line.len() - line.trim_start().len()];
            out.push_str(&format!("{indent}{} {}\n", "}".repeat(give), line.trim()));
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    if !code.ends_with('\n') { while out.ends_with('\n') { out.pop(); } }
    out
}

pub fn balance_commented_blocks(code: &str) -> String {
    fn is_dead(line: &str) -> bool {
        line.trim_start().starts_with("//")
    }
    // Indices of `{` / `}` on a line that are structural. For a dead (commented) line,
    // every brace in the raw text counts (the original block markers live in the
    // comment). For a live line, skip braces inside strings or after an inline `//`.
    fn braces(line: &str, dead: bool) -> Vec<(usize, bool)> {
        let b = line.as_bytes();
        let mut v = Vec::new();
        if dead {
            for (i, &c) in b.iter().enumerate() {
                if c == b'{' { v.push((i, true)); } else if c == b'}' { v.push((i, false)); }
            }
            return v;
        }
        let mut in_str: Option<u8> = None;
        let mut i = 0;
        while i < b.len() {
            let c = b[i];
            match in_str {
                Some(q) => {
                    if c == b'\\' { i += 2; continue; }
                    if c == q { in_str = None; }
                }
                None => match c {
                    b'/' if i + 1 < b.len() && b[i + 1] == b'/' => break,
                    b'"' | b'\'' => in_str = Some(c),
                    b'{' => v.push((i, true)),
                    b'}' => v.push((i, false)),
                    _ => {}
                },
            }
            i += 1;
        }
        v
    }

    let net = |line: &str| -> i32 {
        braces(line, is_dead(line)).iter().map(|(_, o)| if *o { 1 } else { -1 }).sum()
    };

    let trailing_nl = code.ends_with('\n');
    let mut lines: Vec<String> = code.lines().map(|s| s.to_string()).collect();
    // Local forward scan per commented opener — robust to imbalance elsewhere in the
    // file (a global stack mispairs once any earlier block is uneven). For each dead
    // line that nets a positive `{`, walk forward tracking depth until it returns to
    // the opener's base level; if the line that closes it is live, that `}` is the
    // orphan, so comment it.
    for _ in 0..16 {
        let mut to_comment: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
        for idx in 0..lines.len() {
            if !is_dead(&lines[idx]) {
                continue;
            }
            let open_delta = net(&lines[idx]);
            if open_delta <= 0 {
                continue; // not a net block-opener
            }
            let mut depth = open_delta;
            for (j, line) in lines.iter().enumerate().skip(idx + 1) {
                depth += net(line);
                if depth <= 0 {
                    if !is_dead(line) {
                        to_comment.insert(j);
                    }
                    break;
                }
            }
        }
        if to_comment.is_empty() {
            break;
        }
        for j in to_comment {
            let line = &lines[j];
            let indent = &line[..line.len() - line.trim_start().len()];
            lines[j] = format!("{}// [SSF2-dead] {}", indent, line.trim());
        }
    }
    let mut joined = lines.join("\n");
    if trailing_nl {
        joined.push('\n');
    }
    joined
}

/// The top-level arguments of a call whose `(` is at `open`, and the index just past its `)`.
/// Brackets of every kind nest and strings are skipped whole, so `f(g(a, b), "x,)")` has two.
pub(crate) fn call_args_at(code: &str, open: usize) -> Option<(Vec<String>, usize)> {
    let b = code.as_bytes();
    if b.get(open) != Some(&b'(') { return None }
    let mut depth = 0i32;
    let mut i = open;
    let mut args: Vec<String> = Vec::new();
    let mut start = open + 1;
    while i < b.len() {
        let c = b[i];
        match c {
            b'"' | b'\'' => {
                i += 1;
                while i < b.len() && b[i] != c { if b[i] == b'\\' { i += 1; } i += 1; }
            }
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth -= 1;
                if depth == 0 {
                    let last = code[start..i].trim();
                    if !(args.is_empty() && last.is_empty()) { args.push(last.to_string()); }
                    return Some((args, i + 1));
                }
            }
            b',' if depth == 1 => { args.push(code[start..i].trim().to_string()); start = i + 1; }
            _ => {}
        }
        i += 1;
    }
    None
}

/// The value of field `name` in an object literal (`{ damage: x, flinch: false }`), if it is one.
fn object_literal_field(arg: &str, name: &str) -> Option<String> {
    let t = arg.trim();
    if !t.starts_with('{') || !t.ends_with('}') { return None }
    let inner = &t[1..t.len() - 1];
    let wrapped = format!("({inner})");
    let (parts, _) = call_args_at(&wrapped, 0)?;
    parts.iter().find_map(|p| {
        let (k, v) = p.split_once(':')?;
        (k.trim().trim_matches('"') == name).then(|| v.trim().to_string())
    })
}

fn apply_call_rewrites(code: &str) -> String {
    let rules = &crate::mappings::api_commands().call_rewrites;
    if rules.is_empty() { return code.to_string() }
    let mut out = code.to_string();
    for rule in rules {
        if !out.contains(&format!("{}(", rule.call)) { continue }
        out = apply_call_rewrite(&out, rule);
    }
    out
}

fn apply_call_rewrite(code: &str, rule: &crate::mappings::CallRewrite) -> String {
    let needle = format!("{}(", rule.call);
    let field_ref = crate::re!(r"\{(\d+)\.(\w+)\}");
    let b = code.as_bytes();
    let mut out = String::with_capacity(code.len());
    let mut copied = 0usize;
    let mut from = 0usize;
    while let Some(rel) = code[from..].find(&needle) {
        let at = from + rel;
        let open = at + rule.call.len();
        from = at + 1;
        let prev = if at == 0 { None } else { Some(b[at - 1]) };
        let (span_start, recv) = match rule.receiver.as_str() {
            "none" => {
                if prev.is_some_and(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'.' || c == b'$') { continue }
                (at, String::new())
            }
            kind => {
                if prev != Some(b'.') { continue }
                let rs = receiver_start(code, at - 1);
                let recv = code[rs..at - 1].to_string();
                if recv.is_empty() || (kind == "self" && recv != "self") || (kind == "other" && recv == "self") { continue }
                (rs, recv)
            }
        };
        if span_start < copied { continue }
        let Some((args, end)) = call_args_at(code, open) else { continue };
        if rule.args.is_some_and(|n| n != args.len()) { continue }
        // a call nested in an argument is rewritten too
        let args: Vec<String> = args.iter().map(|a| apply_call_rewrite(a, rule)).collect();
        let mut text = rule.template.replace("{recv}", &recv).replace("{args}", &args.join(", "));
        let mut ok = true;
        for c in field_ref.captures_iter(&rule.template) {
            let i: usize = c[1].parse().unwrap_or(0);
            match args.get(i.wrapping_sub(1)).and_then(|a| object_literal_field(a, &c[2])) {
                Some(v) => text = text.replace(&c[0], &v),
                None => { ok = false; break }
            }
        }
        if !ok { continue }
        for (i, a) in args.iter().enumerate() { text = text.replace(&format!("{{{}}}", i + 1), a); }
        out.push_str(&code[copied..span_start]);
        out.push_str(&text);
        copied = end;
        from = end;
    }
    out.push_str(&code[copied..]);
    out
}

/// One regex_replacements entry, pre-compiled from the JSONC config.
struct CompiledRegexReplacement {
    regex: regex::Regex,
    replacement: String,
}

/// Pre-compile every `regex_replacements` entry once per process. Bad patterns
/// log a warning and are silently skipped, so a typo in commands.jsonc can't
/// break the conversion.
fn regex_replacement_cache() -> &'static [CompiledRegexReplacement] {
    static CACHE: std::sync::OnceLock<Vec<CompiledRegexReplacement>> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| {
        let cfg = crate::mappings::api_commands();
        let mut out = Vec::with_capacity(cfg.regex_replacements.len());
        for r in &cfg.regex_replacements {
            match regex::Regex::new(&r.pattern) {
                Ok(re) => out.push(CompiledRegexReplacement {
                    regex: re,
                    replacement: r.replacement.clone(),
                }),
                Err(e) => log::warn!(
                    "[api_mappings] regex_replacements pattern failed to compile — \
                     pattern={:?} note={:?} error={}",
                    r.pattern, r.note, e
                ),
            }
        }
        out
    })
}

/// Per-character bookkeeping written to `conversion_log.json` at the end of a
/// run. `unknown` are calls that didn't appear in ANY commands.jsonc section
/// (genuine gaps); `ssf2_only` are calls we know lack a Fraymakers equivalent
/// and have intentionally commented out. The "already logged" set keeps each
/// unknown name's debug-log line down to one occurrence per process.
#[derive(Debug, Default, Clone)]
pub struct ConversionLog {
    pub unknown: std::collections::BTreeMap<String, usize>,
    pub ssf2_only: std::collections::BTreeMap<String, usize>,
    pub already_logged: std::collections::BTreeSet<String>,
    /// Helper names this character's own script defines (see [`discount_local_fn_calls`]).
    /// Filtered out of `unknown` at snapshot time, so it doesn't matter whether the
    /// defining file or a later frame-script pass accrued the count.
    pub local_fns: std::collections::BTreeSet<String>,
}

fn conversion_log() -> &'static std::sync::Mutex<ConversionLog> {
    static LOG: std::sync::OnceLock<std::sync::Mutex<ConversionLog>> = std::sync::OnceLock::new();
    LOG.get_or_init(|| std::sync::Mutex::new(ConversionLog::default()))
}

/// Reset the conversion log — call at the start of each character so per-
/// character counts and the once-per-name dedup start from a clean slate.
pub fn reset_conversion_log() {
    *conversion_log().lock().unwrap() = ConversionLog::default();
}

/// Snapshot the current log (does not reset). Used at the end of a run to
/// write `conversion_log.json` next to the exported character. Calls to the
/// character's own helpers are dropped from `unknown` here (see
/// [`discount_local_fn_calls`]) so the stream means "no FM equivalent".
pub fn snapshot_conversion_log() -> ConversionLog {
    let mut snap = conversion_log().lock().unwrap().clone();
    let locals = std::mem::take(&mut snap.local_fns);
    snap.unknown.retain(|name, _| !locals.contains(name));
    snap.local_fns = locals;
    snap
}

/// Walk `code` for `.NAME(` call sites and log any name that isn't covered
/// by any of the five commands.jsonc sections. Locally-defined helper names
/// still land here (this pass runs per-method, so it can't see a sibling
/// definition); [`discount_local_fn_calls`] subtracts them once the whole
/// script is assembled.
fn log_unknown_calls(code: &str, cfg: &crate::mappings::ApiCommands) {
    // Build the union of all known names across every section.
    let mut known: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    // Call-shaped identifiers (`NAME(`) in a mapping's source and target text. A regex
    // pattern escapes its call paren, so `NAME\(` counts as a call site too — without
    // that, every name a regex mapping EMITS (`Common.setAutocancel($1)`) reads back as
    // an unmapped gap.
    let extract = |s: &str, out: &mut std::collections::BTreeSet<String>| {
        let bytes = s.as_bytes();
        let mut i = 0usize;
        while i < bytes.len() {
            // start of identifier?
            if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
                let start = i;
                while i < bytes.len()
                    && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_')
                { i += 1; }
                let call = bytes.get(i) == Some(&b'(')
                    || (bytes.get(i) == Some(&b'\\') && bytes.get(i + 1) == Some(&b'('));
                if call {
                    out.insert(String::from_utf8_lossy(&bytes[start..i]).into_owned());
                }
            } else {
                i += 1;
            }
        }
    };
    for r in &cfg.replacements {
        extract(&r.from, &mut known);
        extract(&r.to,   &mut known);
    }
    for r in &cfg.regex_replacements {
        extract(&r.pattern,     &mut known);
        extract(&r.replacement, &mut known);
    }
    for p in &cfg.passthrough_fm_apis { known.insert(p.name.clone()); }
    for b in &cfg.haxe_builtins       { known.insert(b.name.clone()); }
    for o in &cfg.ssf2_only           { known.insert(o.name.clone()); }
    for f in &cfg.frame_params        { known.insert(f.name.clone()); }

    let bytes = code.as_bytes();
    let mut i = 0usize;
    let debug = log::log_enabled!(log::Level::Debug);
    let mut log_state = conversion_log().lock().unwrap();
    while i < bytes.len() {
        if bytes[i] == b'.'
            && i + 1 < bytes.len()
            && (bytes[i + 1].is_ascii_alphabetic() || bytes[i + 1] == b'_')
        {
            let start = i + 1;
            let mut j = start;
            while j < bytes.len()
                && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_')
            { j += 1; }
            if j < bytes.len() && bytes[j] == b'(' {
                let name = String::from_utf8_lossy(&bytes[start..j]).into_owned();
                if !known.contains(&name) {
                    *log_state.unknown.entry(name.clone()).or_insert(0) += 1;
                    if debug && log_state.already_logged.insert(name.clone()) {
                        log::debug!(
                            "[api_mappings] unknown call '{}' — add to replacements / \
                             passthrough_fm_apis / ssf2_only in commands.jsonc as appropriate",
                            name
                        );
                    }
                }
                i = j;
                continue;
            }
        }
        i += 1;
    }
}

// ─── 30fps → 60fps frame-count scaling ───────────────────────────────────────
//
// SSF2 plays at 30fps, Fraymakers at 60fps. Numeric arguments in decompiled
// frame-script Haxe that represent frame counts must be doubled so the script
// runs at the same real time. Only specific, known frame-count fields are
// touched — speeds, positions, IDs, angles, repeat counts and multipliers are
// deliberately left alone.

/// Double the first non-negative integer literal that follows each occurrence
/// of `marker` (after optional spaces/tabs). Values `>= skip_at` are treated as
/// sentinels and left unchanged; non-literal arguments (expressions, negatives)
/// are skipped because no digits immediately follow the marker.
fn double_int_after_marker(code: &str, marker: &str, skip_at: i64, keep_one: bool, plus: i64) -> String {
    let mut out = String::with_capacity(code.len() + 16);
    let bytes = code.as_bytes();
    let mut i = 0usize;
    while i < code.len() {
        if code[i..].starts_with(marker) {
            out.push_str(marker);
            let mut j = i + marker.len();
            while j < code.len() && (bytes[j] == b' ' || bytes[j] == b'\t') {
                out.push(bytes[j] as char);
                j += 1;
            }
            let start = j;
            while j < code.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j > start {
                match code[start..j].parse::<i64>() {
                    Ok(1) if keep_one => out.push_str(&code[start..j]),
                    Ok(n) if n < skip_at => out.push_str(&((n + plus) * 2).to_string()),
                    _ => out.push_str(&code[start..j]),
                }
            }
            i = j;
        } else {
            let ch = code[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

/// Double the integer literal at positional argument `arg_idx` of every
/// `fn_name(...)` call. Arguments before the target are walked over with
/// bracket-depth tracking, so commas inside nested calls/arrays/objects don't
/// miscount. Non-literal and negative arguments are left unchanged; a literal
/// `>= skip_at` is treated as a sentinel and left unchanged.
fn double_call_arg(code: &str, fn_name: &str, arg_idx: usize, skip_at: i64, keep_one: bool) -> String {
    let marker = format!("{}(", fn_name);
    let bytes = code.as_bytes();
    let mut out = String::with_capacity(code.len() + 16);
    let mut cursor = 0usize;
    while let Some(rel) = code[cursor..].find(&marker) {
        let arg0 = cursor + rel + marker.len(); // start of argument 0
        // Walk to the start of argument `arg_idx`, depth-tracking brackets.
        let mut j = arg0;
        let mut depth = 0i32;
        let mut cur = 0usize;
        let mut bailed = false;
        while cur < arg_idx && j < code.len() {
            match bytes[j] {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => {
                    if depth == 0 { bailed = true; break; } // call closed early
                    depth -= 1;
                }
                b',' if depth == 0 => cur += 1,
                _ => {}
            }
            j += 1;
        }
        // Copy verbatim up to the target argument's start.
        out.push_str(&code[cursor..j]);
        cursor = j;
        if bailed || cur < arg_idx { continue; } // argument not present
        // Skip leading whitespace, then double the integer literal if present.
        let mut k = j;
        while k < code.len() && (bytes[k] == b' ' || bytes[k] == b'\t') { k += 1; }
        out.push_str(&code[j..k]);
        let start = k;
        while k < code.len() && bytes[k].is_ascii_digit() { k += 1; }
        if k > start {
            match code[start..k].parse::<i64>() {
                Ok(1) if keep_one => out.push_str(&code[start..k]),
                Ok(n) if n < skip_at => out.push_str(&(n * 2).to_string()),
                _ => out.push_str(&code[start..k]),
            }
        }
        cursor = k;
    }
    out.push_str(&code[cursor..]);
    out
}

/// Double every command parameter flagged `isframe` in mappings/commands.json
/// — `createTimer` delays, `stancePlayFrame` indices, and object-literal
/// fields like `hitStun` / `refreshRate` / `chargetime_max`. This is the
/// single, data-driven path for frame-count scaling in decompiled frame-script
/// Haxe; which parameters are doubled is controlled entirely by the JSON.
pub fn double_frame_counts(code: &str) -> String {
    let mut out = code.to_string();
    for p in &crate::mappings::api_commands().frame_params {
        if !p.isframe { continue; }
        let skip_at = p.sentinel.unwrap_or(i64::MAX);
        out = match p.kind.as_str() {
            "field" => double_int_after_marker(&out, &format!("{}:", p.name), skip_at, p.keep_one, p.plus),
            "call"  => double_call_arg(&out, &p.name, p.arg, skip_at, p.keep_one),
            _ => out, // unknown kind — leave untouched
        };
    }
    out
}

/// A hitstun a script writes is only a duration if it IS one.
///
/// SSF2 states `hitLag` as a multiplier around one (-1.0, -1.1, -1.23, 0.9), not a count of
/// frames, and the rename pass turns that key into Fraymakers' `hitstun`. Written through
/// verbatim it asks the engine to hold a victim for a negative fraction of a frame. Fraymakers
/// has no hitstun multiplier and works its hitstun out from the knockback, which is what SSF2
/// does too, so anything that is not a positive whole number of frames becomes the "no override"
/// sentinel and the engine's own answer stands.
pub fn normalize_hitstun_writes(code: &str) -> String {
    let re = crate::re!(r"(?m)\bhitstun\s*:\s*(-?[0-9]+(?:\.[0-9]+)?)");
    re.replace_all(code, |c: &regex::Captures| {
        let v: f64 = c[1].parse().unwrap_or(-1.0);
        if v > 0.0 && v.fract() == 0.0 { format!("hitstun: {}", &c[1]) }
        else { "hitstun: -1".to_string() }
    }).into_owned()
}

/// A knockback a script WRITES carries SSF2 units, so it converts like every other knockback.
///
/// The two engines launch a victim at different speeds for the same numbers, and the factors are
/// measured per field in `mappings/character/hitbox_stats.jsonc`. A stats table gets them at
/// emit; a script that sets knockback at runtime (`updateHitboxStats(1, { baseKnockback:
/// -speed * 5 })`) has to get them too, or that move alone launches in SSF2 units.
///
/// Knockback is linear in these fields, so a computed value scales exactly like a literal one.
/// Runs AFTER the rename pass, so the field names here are already the Fraymakers ones.
pub fn scale_knockback_writes(code: &str) -> String {
    let cfg = crate::mappings::character_hitbox_stats();
    let mut out = code.to_string();
    for field in ["baseKnockback", "knockbackGrowth"] {
        let factor = cfg.scale_for(field);
        if (factor - 1.0).abs() < f64::EPSILON { continue; }
        // The value ends at the next `,` or `}` that is not nested inside a call of its
        // own: lucario's growth is a `Math.pow(x, 2)`, and stopping at that comma wraps
        // half a call and leaves the rest of it outside.
        let needle = format!("{field}:");
        let mut scan = 0usize;
        let mut next = String::with_capacity(out.len());
        while let Some(rel) = out[scan..].find(&needle) {
            let at = scan + rel;
            let starts = at == 0 || {
                let p = out.as_bytes()[at - 1];
                !(p.is_ascii_alphanumeric() || p == b'_')
            };
            let val_at = at + needle.len();
            if !starts {
                next.push_str(&out[scan..val_at]);
                scan = val_at;
                continue;
            }
            let raw = &out[val_at..val_at + value_end(&out[val_at..])];
            let expr = raw.trim();
            let lead = &raw[..raw.len() - raw.trim_start().len()];
            let tail = &raw[lead.len() + expr.len()..];
            next.push_str(&out[scan..at]);
            match expr.parse::<f64>() {
                // a literal converts to a literal, the way the stats tables do
                Ok(v) => next.push_str(&format!(
                    "{field}:{lead}{}{tail}", crate::haxe_gen::fmt_num(v * factor))),
                // anything computed keeps its expression and carries the factor
                Err(_) => next.push_str(&format!("{field}:{lead}({expr}) * {factor}{tail}")),
            }
            scan = val_at + raw.len();
        }
        next.push_str(&out[scan..]);
        out = next;
    }
    out
}

#[cfg(test)]
mod inline_named_stat_objects_tests {
    #[test]
    fn a_split_call_handed_a_name_takes_the_literal_written_before_it() {
        let src = "self.newStats = { xSpeedDecay: -3.8, xSpeedDecayAir: -3.8 };\nself.self.updateAttackStats(self.newStats);\n";
        let out = super::translate_ssf2_to_fm(src);
        assert!(!out.contains("SSF2-only: updateAttackStats"), "{out}");
        assert!(out.contains("updateAnimationStats"), "{out}");
    }
}

#[cfg(test)]
mod stat_names_written_tests {
    #[test]
    fn a_call_handed_a_name_writes_no_fields_and_a_later_point_keeps_its_halves() {
        let src = "self.updateAttackBoxStats(1, self.jabFinishStats);\nif (__truthy(SSF2API.getPlatformBetweenPoints(new Point(self.getX() + 18, self.getY() - 2), new Point(self.getX() + 18, self.getY() + 15), { ignoreFallthrough: false }))) {\n\tself.safeMove(18, 0);\n}\n";
        let out = super::translate_ssf2_to_fm(src);
        assert!(out.contains("__platformBetween({ x: self.getX() + 18, y: self.getY() - 2 }"), "{out}");
        assert!(super::hitbox_stat_names_written("self.updateHitboxStats(0, stats); var p = { x: 1, y: 2 };").is_empty());
        assert_eq!(super::hitbox_stat_names_written("self.updateHitboxStats(0, { damage: 3, angle: f(a, b) });"), vec!["damage", "angle"]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(call: &str, receiver: &str, args: Option<usize>, template: &str) -> crate::mappings::CallRewrite {
        crate::mappings::CallRewrite { call: call.into(), receiver: receiver.into(), args, template: template.into(), note: String::new() }
    }

    #[test]
    fn call_args_are_read_with_brackets_and_strings_balanced() {
        let code = r#"f(g(a, b), "x,)", [1, 2], { k: h(3) })"#;
        let (args, end) = call_args_at(code, 1).expect("args");
        assert_eq!(args, vec!["g(a, b)", r#""x,)""#, "[1, 2]", "{ k: h(3) }"]);
        assert_eq!(end, code.len());
        assert_eq!(call_args_at("f()", 1).expect("empty").0, Vec::<String>::new());
    }

    #[test]
    fn a_call_rewrite_keeps_an_indexed_receiver_and_nested_arguments_whole() {
        let r = rule("setScale", "", Some(2), "{recv}.setScaleX({1}); {recv}.setScaleY({2})");
        assert_eq!(apply_call_rewrite("p.get()[0].setScale(a(b(c)), d);", &r),
                   "p.get()[0].setScaleX(a(b(c))); p.get()[0].setScaleY(d);");
        // the wrong number of arguments is not this rewrite's call
        assert_eq!(apply_call_rewrite("x.setScale(1);", &r), "x.setScale(1);");
    }

    #[test]
    fn a_call_rewrite_reads_a_field_of_an_object_literal_or_leaves_the_call() {
        let r = rule("takeDamage", "", None, "{recv}.addDamage({1.damage})");
        assert_eq!(apply_call_rewrite("foe.takeDamage({ flinch: f(1, 2), damage: g(3) }, self)", &r), "foe.addDamage(g(3))");
        assert_eq!(apply_call_rewrite("foe.takeDamage(stats, self)", &r), "foe.takeDamage(stats, self)");
    }

    #[test]
    fn a_self_rewrite_does_not_touch_another_receiver_and_a_bare_one_needs_no_receiver() {
        let only_self = rule("recover", "self", Some(1), "{recv}.addDamage(-({1}))");
        assert_eq!(apply_call_rewrite("other.recover(2); self.recover(3)", &only_self), "other.recover(2); self.addDamage(-(3))");
        let bare = rule("heal", "none", Some(1), "__heal({1})");
        assert_eq!(apply_call_rewrite("heal(4); x.heal(5)", &bare), "__heal(4); x.heal(5)");
    }
    #[test]
    fn a_readiness_guard_with_more_in_it_keeps_the_rest_of_its_test() {
        // a guard that is readiness alone goes; one that also asks something else keeps that
        // ask, with readiness read as true by the call mapping later
        let alone = "\tif (SSF2API.isReady()) {\n\t\tgo();\n\t}\n";
        assert_eq!(remove_readiness_guards(alone), "// [FM] isReady guard removed — always true in Fraymakers\n\tgo();\n");
        let with_self = "\tif (__truthy(self) && SSF2API.isReady()) {\n\t\tgo();\n\t}\n";
        assert_eq!(remove_readiness_guards(with_self), "// [FM] isReady guard removed — always true in Fraymakers\n\tgo();\n");
        let with_handle = "\tif (__truthy(SSF2API.isReady()) && __truthy(self.self)) {\n\t\tgo();\n\t}\n";
        assert_eq!(remove_readiness_guards(with_handle), "// [FM] isReady guard removed — always true in Fraymakers\n\tgo();\n");
        let with_parent = "\tif ((__truthy(parent) && __truthy(SSF2API.isReady())) && __truthy(self.self)) {\n\t\tgo();\n\t}\n";
        assert_eq!(remove_readiness_guards(with_parent), "// [FM] isReady guard removed — always true in Fraymakers\n\tgo();\n");
        let with_true = "\tif ((true && __truthy(SSF2API.isReady())) && __truthy(self)) {\n\t\tgo();\n\t}\n";
        assert_eq!(remove_readiness_guards(with_true), "// [FM] isReady guard removed — always true in Fraymakers\n\tgo();\n");
        let more = "\tif ((self.pos == null) && __truthy(SSF2API.isReady())) {\n\t\tself.pos = [self.getX(), self.getY()];\n\t}\n";
        assert_eq!(remove_readiness_guards(more), more);
    }
    #[test]
    fn a_stated_position_crosses_the_size_scale_for_a_fighter_only() {
        let code = "self.setX(self.getX() + self.flipX(3)); foe.setY(0);";
        // a stage object's script keeps its own position boundary
        assert_eq!(speeds_cross_the_boundary(code), code);
        let _g = PositionsCrossGuard::install();
        assert_eq!(speeds_cross_the_boundary(code), "self.setX(__sp(__isp(self.getX()) + self.flipX(3))); foe.setY(0);");
    }

    #[test]
    fn a_stated_speed_crosses_the_boundary_and_a_read_comes_back() {
        let out = speeds_cross_the_boundary("self.setYSpeed(26); self.setXSpeed(self.getXSpeed() / 2); self.setXSpeed(0); if (self.getYSpeed() > 0) {}");
        // a y write carries the gravity step, an x write (a zero included: it may hold) its own wrapper
        assert_eq!(out, "self.setYSpeed(__svy(26)); self.setXSpeed(__svx(__isv(self.getXSpeed()) / 2)); self.setXSpeed(__svx(0)); if (__isv(self.getYSpeed()) > 0) {}");
        assert_eq!(speeds_cross_the_boundary("p.setXSpeed(__sv(3))"), "p.setXSpeed(__sv(3))");
    }

    #[test]
    fn a_write_through_exports_keeps_one_exports_hop() {
        let once = write_through_published_state("character.curThunder = (thunder);", "curThunder");
        assert_eq!(once, "character.exports.curThunder.set((thunder));");
        let already = write_through_published_state("character.exports.curThunder = (thunder);", "curThunder");
        assert_eq!(already, "character.exports.curThunder.set((thunder));");
    }


    #[test]
    fn a_scaled_knockback_keeps_a_call_in_its_value_whole() {
        // The value is a call with an argument list of its own; stopping at that
        // comma wraps half the call and leaves the rest outside it.
        let src = "self.updateHitboxStats(1, { knockbackGrowth: (84 - Math.pow(a, 2)), angle: 82 });";
        let out = scale_knockback_writes(src);
        assert!(out.contains("Math.pow(a, 2)"), "{out}");
        assert!(out.contains("angle: 82"), "{out}");
        let factor = crate::mappings::character_hitbox_stats().scale_for("knockbackGrowth");
        assert!(out.contains(&format!("* {factor}")), "{out}");
    }

    #[test]
    fn a_runtime_hitbox_write_loses_the_field_the_engine_does_not_know() {
        let out = resolve_fixed_knockback_writes(
            "self.updateHitboxStats(1, { baseKnockback: 34.875, knockbackGrowth: 76.4, angle: 60, damage: 3, weightKB: 0 });");
        assert!(!out.contains("weightKB"), "the field the engine rejects is gone");
        assert!(out.contains("baseKnockback: 34.875"), "and the rest of the write survives");
        assert!(out.contains("knockbackGrowth: 76.4"), "a zero states no fixed launch, so growth stands");
        assert!(!out.contains(", }") && !out.contains(",}"), "and no comma is left dangling: {out}");
    }

    #[test]
    fn a_stated_fixed_launch_does_not_grow() {
        let out = resolve_fixed_knockback_writes(
            "self.updateHitboxStats(1, { baseKnockback: 10, knockbackGrowth: 50, weightKB: 12 });");
        assert!(!out.contains("weightKB"));
        assert!(out.contains("knockbackGrowth: 0"), "fixed knockback carries no growth: {out}");
    }

    #[test]
    fn fire_projectile_becomes_create_projectile_spawn() {
        // SSF2's fireProjectile must spawn a real FM projectile, not get
        // commented out (which left the neutral special spawning nothing).
        let out = translate_ssf2_to_fm("self.fireProjectile(\"mario_fireball\", 22, -20);\n");
        assert!(out.contains("match.createProjectile"), "no createProjectile: {out}");
        assert!(out.contains("getContent(\"mariofireballProjectile\")"),
            "content id wrong (want mariofireballProjectile): {out}");
        // a stated offset is in the source's pixels: it crosses through the size scale
        assert!(out.contains("self.flipX(__sp(22))"), "x offset/facing not applied: {out}");
        assert!(out.contains("self.getY() + (__sp(-20))"), "y offset not applied: {out}");
        assert!(!out.contains("[SSF2-only: fireProjectile]"),
            "spawn was still commented out: {out}");
        // No-offset form defaults to 0,0.
        let bare = translate_ssf2_to_fm("self.fireProjectile(\"mario_fireball\");\n");
        assert!(bare.contains("self.flipX(0)") && bare.contains("self.getY() + (0)"),
            "bare fireProjectile didn't default offsets: {bare}");
        // Expression args (a method call, e.g. zelda's airneedle spawning ahead of motion)
        // must match too — they contain parens, which a naive [^,()] arg pattern would miss.
        let expr = translate_ssf2_to_fm(
            "self.fireProjectile(\"airneedle\", self.getXSpeed(), self.getYSpeed());\n");
        assert!(expr.contains("match.createProjectile"), "expr-arg form not rewritten: {expr}");
        // a speed read comes back in the source's units, the units every offset here is in,
        // and the offset crosses back through the size scale
        assert!(expr.contains("self.flipX(__sp(__isv(self.getXSpeed())))"),
            "expr x-offset not carried: {expr}");
        assert!(expr.contains("self.getY() + (__sp(__isv(self.getYSpeed())))"),
            "expr y-offset not carried: {expr}");
        assert!(!expr.contains("[SSF2-only: fireProjectile]"),
            "expr-arg fireProjectile left commented: {expr}");
        // A non-`self` receiver chain (zelda: self.getOwner().fireProjectile) must be
        // consumed WHOLE — otherwise the dangling `receiver.` glues onto the emitted
        // block as `receiver.{ … }`, which is invalid Haxe.
        let owner = translate_ssf2_to_fm(
            "proj = self.getOwner().fireProjectile(\"cameraPanProjectile\", 0, 0);\n");
        assert!(owner.contains("match.createProjectile"), "owner-receiver not rewritten: {owner}");
        assert!(!owner.contains(".{"), "dangling receiver produced .{{: {owner}");
        assert!(owner.trim_end().ends_with("_proj; };") || owner.contains("_proj; }"),
            "block doesn't return _proj (wrong value in assignment position): {owner}");
    }

    #[test]
    fn test_global_variable_rewrite() {
        // SSF2 get/setGlobalVariable -> Fraymakers makeX wrapper .get()/.set().
        let out = translate_ssf2_to_fm(concat!(
            "self.setGlobalVariable(\"canStartRise\", true);\n",
            "if (self.getGlobalVariable(\"canStartRise\")) { return; }\n",
            "self.setGlobalVariable(\"standtime\", self.getGlobalVariable(\"standtime\") - 1);\n",
        ));
        assert!(out.contains("canStartRise.set(true)"), "set not rewritten: {out}");
        assert!(out.contains("canStartRise.get()"), "get not rewritten: {out}");
        assert!(out.contains("standtime.set(standtime.get() - 1)"), "nested get/set not rewritten: {out}");
        assert!(!out.contains("GlobalVariable"), "raw GlobalVariable call left in output: {out}");
        assert!(!out.contains("AnimationStatsMetadata"), "old wrong mapping still present: {out}");
    }

    #[test]
    fn test_uncomment_local_fn_calls() {
        // A call commented [SSF2-only: NAME] is restored iff NAME is a local function.
        let code = concat!(
            "function jumpToContinue(arg0) {\n",
            "\tself.removeEventListener(EntityEvent.COLLIDE_FLOOR, jumpToContinue);\n",
            "}\n",
            "function setLandingLag(arg0) {\n",
            "\t\t\t// [SSF2-only: jumpToContinue] jumpToContinue();\n",
            "\t\t\t// [SSF2-only: tossItem] self.tossItem(270);\n",
            "}\n",
        );
        let out = uncomment_local_fn_calls(code);
        // jumpToContinue is defined locally -> restored (indent preserved, marker gone)
        assert!(out.contains("\t\t\tjumpToContinue();"), "local-fn call not restored: {out}");
        assert!(!out.contains("[SSF2-only: jumpToContinue]"), "marker not removed");
        // tossItem is NOT defined locally -> stays commented
        assert!(out.contains("[SSF2-only: tossItem] self.tossItem(270)"), "non-local wrongly restored");
    }

    #[test]
    fn a_value_holding_a_comma_is_one_value() {
        // A stat's value is often a call with arguments of its own; cutting at its
        // first comma leaves half a call behind.
        assert_eq!(value_end("self.getAttackBoxStat(a, b), rest"), 27);
        assert_eq!(value_end("1 }"), 2);
        assert_eq!(value_end("[1, 2], x"), 6);
    }

    #[test]
    fn a_dropped_name_does_not_cut_into_a_longer_one() {
        // `stun` is not a stat this engine has; `hitstun` is. Searching for the
        // short one inside the long one leaves `hit,` behind and the script stops
        // parsing at it.
        let src = "self.updateHitboxStats(1, { hitstun: -1, stun: 4, damage: 3 });";
        let out = drop_hitbox_fields_without_counterpart(src);
        assert!(out.contains("hitstun: -1"), "{out}");
        assert!(!out.contains("stun: 4"), "{out}");
        assert!(out.contains("damage: 3"), "{out}");
    }

    #[test]
    fn a_stat_the_engine_does_not_have_is_dropped_with_its_whole_value() {
        let src = "self.updateHitboxStats(1, { damage: 5, madeUpStat: self.f(a, b), angle: 20 });";
        let out = drop_hitbox_fields_without_counterpart(src);
        assert!(!out.contains("madeUpStat"), "{out}");
        assert!(!out.contains("self.f("), "the value went with it: {out}");
        assert!(out.contains("damage: 5"), "{out}");
        assert!(out.contains("angle: 20"), "{out}");
    }

    #[test]
    fn stats_the_engine_has_are_left_alone() {
        let src = "self.updateHitboxStats(1, { damage: 5, angle: 20, shieldable: false });";
        assert_eq!(drop_hitbox_fields_without_counterpart(src), src);
    }

    #[test]
    fn rewrite_add_effect_to_list_unwraps_balanced() {
        let code = "self.addEffectToList(match.createVfx(new VfxStats({ x: 1 }), self), self);";
        let out = rewrite_add_effect_to_list(code);
        assert_eq!(
            out,
            "__addFx(match.createVfx(new VfxStats({ x: 1 }), self));"
        );
        // balanced parens after the rewrite
        assert_eq!(out.matches('(').count(), out.matches(')').count());
    }

    #[test]
    fn number_cast_rewritten_balanced() {
        let out = rewrite_number_cast("cardNumber.set(self.Number(child.get().name));");
        assert!(out.contains("Std.parseFloat(Std.string(child.get().name))"), "wrong: {out}");
        assert!(!out.contains("self.Number("), "old cast left: {out}");
        assert_eq!(out.matches('(').count(), out.matches(')').count(), "unbalanced: {out}");
    }

    #[test]
    fn rewrite_add_effect_to_list_bare_call_unwrapped() {
        // No receiver (the SSF2 receiver was stripped upstream) — must still unwrap so
        // the wrapper's `)` + owner arg don't dangle (marth/pichu finalsmash).
        let code = "\taddEffectToList(match.createVfx(new VfxStats({ x: 1 }), self), self);";
        let out = rewrite_add_effect_to_list(code);
        assert!(out.contains("match.createVfx(new VfxStats({ x: 1 }), self)"), "effect lost: {out}");
        assert!(!out.contains("addEffectToList("), "wrapper not removed: {out}");
        assert_eq!(out.matches('(').count(), out.matches(')').count(), "unbalanced: {out}");
    }

    #[test]
    fn rewrite_add_effect_to_list_leaves_fn_definition_intact() {
        // `function addEffectToList(arg0) {` is the SSF2 ext-method DEFINITION, not a
        // call. It must pass through untouched — mangling it to
        // `function /* addEffectToList */ arg0 {` is invalid hscript and crashes the
        // whole Script.hx scope (zelda:stand rand/repeats Unknown-variable cascade).
        let code = "function addEffectToList(arg0) {\n\teffects.get().push(arg0);\n}";
        let out = rewrite_add_effect_to_list(code);
        assert!(out.contains("function addEffectToList(arg0) {"), "definition mangled: {out}");
        assert_eq!(out.matches('(').count(), out.matches(')').count(), "unbalanced: {out}");
    }

    #[test]
    fn neutralize_flash_timeline_props_comments_block() {
        // getMC()→getViewRootContainer() leaves a live `.currentLabel` read on a
        // container that has no such property. The whole `if (...) {...}` must be
        // commented block-aware so it can't null-access at runtime.
        let code = "if (self.getViewRootContainer().currentLabel != \"b\") {\n\tEngine.log(\"x\");\n}\nself.toState(CState.STAND);";
        let out = neutralize_flash_timeline_props(code);
        assert!(!out.lines().any(|l| {
            let t = l.trim_start();
            !t.starts_with("//") && t.contains(".currentLabel")
        }), "live currentLabel survived: {out}");
        // The trailing live statement is untouched.
        assert!(out.contains("self.toState(CState.STAND);"), "ate live code: {out}");
        // Block body got swept too (no orphaned Engine.log left live).
        assert!(out.lines().filter(|l| !l.trim_start().starts_with("//")).all(|l| !l.contains("Engine.log")),
            "block body leaked: {out}");
    }

    #[test]
    fn neutralize_flash_timeline_props_bare_identifier_and_word_boundary() {
        // Bare `currentFrame` playhead must be neutralized…
        let bare = "if (curFrame.get() == currentFrame) {\n\tx();\n}";
        let out = neutralize_flash_timeline_props(bare);
        assert!(out.lines().all(|l| l.trim_start().starts_with("//") || !contains_word(l, "currentFrame")),
            "bare currentFrame survived: {out}");
        // …but a longer identifier that merely contains it must NOT be touched.
        let safe = "var n = self.getCurrentFrameRate();";
        assert_eq!(neutralize_flash_timeline_props(safe), safe, "word-boundary false positive");
    }

    #[test]
    fn balance_commented_blocks_fixes_orphaned_close() {
        let code = concat!(
            "function destroy(arg0) {\n",
            "\t// [SSF2-only: getMC] if (self.getMC().currentLabel != \"x\") {\n",
            "\t\t// [SSF2-only: getMC] self.getMC().currentLabel == \"x\";\n",
            "\t}\n",
            "\t// [SSF2-only: getStanceMC] if (self.getStanceMC() != \"a\") {\n",
            "\t\tself.playLabel(\"attack\");\n",
            "\t}\n",
            "}\n",
        );
        let out = balance_commented_blocks(code);
        // the two `}` whose openers are commented must now be commented too; the
        // function's own braces stay live and the whole thing balances.
        let live: i32 = out.lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .map(net_brace_delta)
            .sum();
        assert_eq!(live, 0, "live braces unbalanced after fix:\n{out}");
        // only the function body's own close should remain live.
        let live_closes = out.lines()
            .filter(|l| !l.trim_start().starts_with("//") && l.trim() == "}")
            .count();
        assert_eq!(live_closes, 1, "expected 1 live `}}` (function end), got {live_closes}:\n{out}");
    }

    #[test]
    fn guard_reads_of_a_struck_definition_wraps_a_null_local_deref() {
        // a scratch local declared null, its real value struck, then reached through in a block
        // with an else -- the guard must wrap the whole if/else and keep braces balanced.
        let code = concat!(
            "function update() {\n",
            "\tvar _v3 = null;\n",
            "\t// [SSF2-only: getStage] _v3 = match.getStage().getMidground();\n",
            "\tif ((_v4 < (_v2.x - _v3.x))) {\n",
            "\t\toffscreenTimer.tick();\n",
            "\t} else {\n",
            "\t\toffscreenTimer.reset();\n",
            "\t}\n",
            "\treturn;\n",
            "}\n",
        );
        let out = guard_reads_of_a_struck_definition(code);
        assert!(out.contains("if (_v3 != null) {"), "deref not guarded:\n{out}");
        // braces on live lines still balance
        let live: i32 = out.lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .map(net_brace_delta).sum();
        assert_eq!(live, 0, "unbalanced after guard:\n{out}");
        // the plain `return;` (no poison deref) is left untouched, not guarded
        assert!(out.lines().any(|l| l.trim() == "return;"), "unrelated stmt disturbed:\n{out}");
    }

    #[test]
    fn guard_reads_of_a_struck_definition_leaves_a_live_value_alone() {
        // same shape but the local gets a real value on a live line: nothing to guard
        let code = concat!(
            "function update() {\n",
            "\tvar _v3 = null;\n",
            "\t_v3 = self.getViewRootContainer();\n",
            "\tfoo(_v3.x);\n",
            "}\n",
        );
        let out = guard_reads_of_a_struck_definition(code);
        assert!(!out.contains("if (_v3 != null)"), "guarded a live value:\n{out}");
    }

    #[test]
    fn test_basic_translations() {
        let input = concat!(
            "self.self.endAttack();\n",
            "SSF2API.print(\"hello\");\n",
            "self.self.updateAttackBoxStats(1, { damage: 5, direction: 45, power: 10 });\n",
            "self.self.refreshAttackID();",
        );

        let output = translate_ssf2_to_fm(input);
        // a bare endAttack on the fighter ends where it is (the __endAttack helper of the preamble)
        assert!(output.contains("__endAttack()"), "endAttack not translated");
        assert!(output.contains("Engine.log(\"hello\")"), "print not translated");
        assert!(output.contains("self.updateHitboxStats"), "updateAttackBoxStats not translated");
        assert!(output.contains("angle: 45"), "direction: not renamed to angle:");
        // renamed AND converted: a knockback a script writes carries SSF2 units like any other
        let want = crate::haxe_gen::fmt_num(
            10.0 * crate::mappings::character_hitbox_stats().scale_for("baseKnockback"));
        assert!(output.contains(&format!("baseKnockback: {want}")),
            "power: not renamed and converted (want baseKnockback: {want}): {output}");
        assert!(output.contains("self.reactivateHitboxes()"), "refreshAttackID not translated");
    }

    #[test]
    fn test_timer_infinite_repeat_translated() {
        // (started as the script's own timer, `__srcTimer`, which lasts until the state changes)
        // SSF2's 0 = "repeat forever" must become Fraymakers' -1; a source `addTimer` delay is a
        // frame count like `createTimer`'s and doubles the same way (2 -> 4).
        let out = translate_ssf2_to_fm("self.addTimer(2, 0, dashCheck);");
        assert!(out.contains("__srcTimer(4, -1, dashCheck)"), "0 repeat not rewritten to -1: {out}");
        // SSF2 createTimer is renamed to addTimer and its delay is 30→60fps
        // doubled (52 → 104), then the repeat sentinel is fixed.
        let out2 = translate_ssf2_to_fm("self.createTimer(52, 0, loopSound2);");
        assert!(out2.contains("__srcTimer(104, -1, loopSound2)"), "createTimer 0 not rewritten: {out2}");
        // Positive counts and an existing -1 are untouched; a 0 delay stays.
        let out3 = translate_ssf2_to_fm("self.addTimer(0, 1, enable);\nself.addTimer(2, -1, poll);");
        assert!(out3.contains("__srcTimer(0, 1, enable)"), "positive count or 0 delay altered: {out3}");
        assert!(out3.contains("__srcTimer(4, -1, poll)"), "existing -1 altered: {out3}");
    }

    #[test]
    fn test_timer_delay_one_kept_per_frame() {
        // A per-frame SSF2 timer (createTimer delay 1) fires once a SOURCE frame, which is
        // every other engine frame: its checks were written for thirty a second, and run sixty
        // times a second a one-frame tap read as held across two of them.
        let out = translate_ssf2_to_fm("self.createTimer(1, 0, dashCheck);");
        assert!(out.contains("__srcTimer(2, -1, dashCheck)"), "delay 1 should become 2: {out}");
        // Larger delays still scale 30→60fps.
        let out2 = translate_ssf2_to_fm("self.createTimer(3, 0, poll);");
        assert!(out2.contains("__srcTimer(6, -1, poll)"), "delay 3 should double to 6: {out2}");
    }

    #[test]
    fn test_isready_guard_removed() {
        let input = concat!(
            "function a__frame0() {\n",
            "\tif (SSF2API.isReady()) {\n",
            "\t\tself.looped = false;\n",
            "\t\tself.playsound = SSF2API.random();\n",
            "\t}\n",
            "\treturn;\n",
            "}",
        );
        let output = translate_ssf2_to_fm(input);
        // The if-block wrapper should be gone
        assert!(!output.contains("SSF2API.isReady()"), "guard not removed");
        assert!(!output.contains("if ("), "if block should be stripped");
        // Body should be inlined (one tab dedented)
        assert!(output.contains("self.looped = false;"), "body not inlined");
        assert!(output.contains("Random.getFloat"), "random not translated");
    }

    #[test]
    fn test_self_and_isready_guard_removed() {
        let input = concat!(
            "\tif (self && SSF2API.isReady()) {\n",
            "\t\tself.x = 10;\n",
            "\t}",
        );
        let output = translate_ssf2_to_fm(input);
        assert!(!output.contains("SSF2API.isReady()"), "guard not removed");
        assert!(output.contains("self.x = 10;"), "body not inlined");
    }

    #[test]
    fn test_nested_isready_guard() {
        // Guard with nested braces inside
        let input = concat!(
            "\tif (SSF2API.isReady()) {\n",
            "\t\tif (x > 0) {\n",
            "\t\t\tself.y = 1;\n",
            "\t\t}\n",
            "\t}",
        );
        let output = translate_ssf2_to_fm(input);
        assert!(!output.contains("SSF2API.isReady()"), "outer guard not removed");
        // inner if should survive
        assert!(output.contains("if (x > 0)"), "inner if should survive");
        assert!(output.contains("self.y = 1;"), "body not inlined");
    }

    #[test]
    fn mapped_events_are_renamed() {
        // High-frequency events with a confirmed FM equivalent.
        let out = translate_ssf2_to_fm("self.addEventListener(SSF2Event.GROUND_LEAVE, h);");
        assert!(out.contains("GameObjectEvent.LEFT_GROUND"), "GROUND_LEAVE not mapped: {out}");
        assert!(!out.contains("SSF2Event."), "raw SSF2Event leaked: {out}");
        // ATTACK_HIT must not corrupt the longer ATTACK_HIT_SHIELD prefix.
        let shield = translate_ssf2_to_fm("self.addEventListener(SSF2Event.ATTACK_HIT_SHIELD, h);");
        assert!(shield.contains("GameObjectEvent.SHIELD_HIT_DEALT"), "ATTACK_HIT_SHIELD mismapped: {shield}");
        assert!(!shield.contains("HIT_DEALT_SHIELD"), "prefix corruption: {shield}");
    }

    #[test]
    fn unmapped_event_is_neutralized_not_leaked() {
        // An event with no FM equivalent must be commented, never left raw.
        let out = translate_ssf2_to_fm("self.addEventListener(SSF2Event.ITEM_HURT, h);");
        assert!(!out.lines().any(|l| !l.trim_start().starts_with("//") && l.contains("SSF2Event.")),
            "raw SSF2Event leaked live: {out}");
        assert!(out.contains("[SSF2-only: ITEM_HURT]"), "no marker: {out}");
    }

    #[test]
    fn the_end_of_an_attack_is_kept_as_an_ask() {
        // The source announces an attack ending; here the ask is kept and answered when the
        // fighter is in another animation, so the rest of the move still runs.
        let out = translate_ssf2_to_fm("self.addEventListener(SSF2Event.CHAR_ATTACK_COMPLETE, h);");
        assert!(out.contains("__onAttackComplete(h)"), "the ask was not kept: {out}");
        let off = translate_ssf2_to_fm("self.removeEventListener(SSF2Event.CHAR_ATTACK_COMPLETE, h);");
        assert!(off.contains("__offAttackComplete(h)"), "the ask cannot be taken back: {off}");
    }

    #[test]
    fn being_destroyed_is_kept_as_an_ask_and_watched_when_it_is_another_thing() {
        // an object asking about its own end keeps the ask; a script asking about the end of
        // something it made watches that thing, because the handler is the asker's own function
        let own = translate_ssf2_to_fm("self.addEventListener(SSF2Event.PROJ_DESTROYED, h);");
        assert!(own.contains("__onProjDestroyed(h)"), "its own end was not kept: {own}");
        let other = translate_ssf2_to_fm("proj.get().addEventListener(SSF2Event.PROJ_DESTROYED, h);");
        assert!(other.contains("__onProjDestroyedOf(proj.get(), h)"), "the thing is not watched: {other}");
        let off = translate_ssf2_to_fm("proj.get().removeEventListener(SSF2Event.PROJ_DESTROYED, h);");
        assert!(off.contains("__offProjDestroyedOf(proj.get(), h)"), "the watch cannot be taken back: {off}");
    }

    #[test]
    fn a_sound_is_kept_under_the_name_it_was_started_by() {
        let out = translate_ssf2_to_fm("self.playSound(\"brawl_swing_s\");");
        assert!(out.contains("__sound(\"brawl_swing_s\""), "the handle was not kept: {out}");
        let stop = translate_ssf2_to_fm("self.stopSound(held);");
        assert!(stop.contains("__clip.stop()"), "the sound cannot be stopped: {stop}");
        // ...and a script's own function of that name is its own
        let mine = a_stop_with_no_name_stops_the_last_sound("function stopSound() {\n\tx();\n}");
        assert!(mine.contains("function stopSound()"), "a declaration was rewritten: {mine}");
    }

    #[test]
    fn who_an_object_is_comes_from_what_its_script_publishes() {
        let out = translate_ssf2_to_fm("if (foe.getLinkageID() == \"x\") { y(); }");
        assert!(out.contains("__linkageOf(foe)"), "the id was not read: {out}");
    }

    #[test]
    fn an_effect_is_attached_to_whatever_the_call_was_made_on() {
        let own = translate_ssf2_to_fm("self.attachEffect(\"global_dust_blast\");");
        assert!(own.contains("), self)"), "the fighter is not its owner: {own}");
        let there = translate_ssf2_to_fm("match.attachEffect(\"global_dust_blast\");");
        assert!(there.contains("}))") && !there.contains("), match)"),
            "a place on the stage was given an owner: {there}");
        let its = translate_ssf2_to_fm("foe.attachEffect(\"global_dust_blast\");");
        assert!(its.contains("), foe)"), "the thing it was put on is not its owner: {its}");
    }

    /// NONE OF THESE NAMES ARE IN THE CORPUS. A fabricated fighter, a fabricated move, a
    /// fabricated projectile: nothing here is `mario` or `pikachu`, so a pass that answered by
    /// recognising one of the 50 built characters could not have produced this. What proves the
    /// mechanism generalizes is that it reads STRUCTURE (a receiver, a call shape, an event name)
    /// and never a name from a table.
    #[test]
    fn today_s_mechanisms_hold_on_a_character_that_does_not_exist() {
        let code = "\
function quorbSlamStart(?arg0) {\n\
\tquorbTrail.get().addEventListener(SSF2Event.PROJ_DESTROYED, quorbCleanup);\n\
\tself.addEventListener(SSF2Event.CHAR_ATTACK_COMPLETE, function() { self.toState(CState.FALL_SPECIAL); });\n\
\tzhamvorSpark.attachEffect(\"global_spark\", { x: 4, y: -2 });\n\
\tzhamvorSfx.set(self.playSound(\"quorb_slam_impact\"));\n\
\tif (self.zhamvorSfx.getLinkageID() == \"quorb_slam_impact_clip\") { self.stopSound(zhamvorSfx.get()); }\n\
}\n";
        let out = translate_ssf2_to_fm(code);
        // an object made up for this test is watched from here, not answered by name
        assert!(out.contains("__onProjDestroyedOf(quorbTrail.get(), quorbCleanup)"),
            "a fabricated projectile's end was not watched: {out}");
        // the end of an attack, on a fighter this converter has never seen, is still kept
        assert!(out.contains("__onAttackComplete(function()"),
            "a fabricated attack's ending was not kept: {out}");
        // an effect on a fabricated object gets that object as its owner
        assert!(out.contains("zhamvorSpark)"), "a fabricated receiver lost its ownership: {out}");
        // a sound this converter has never heard of is kept under its own (fabricated) name
        assert!(out.contains("__sound(\"quorb_slam_impact\""),
            "a fabricated sound name was not kept: {out}");
        // an id asked of a fabricated receiver still reads off what that script publishes
        assert!(out.contains("__linkageOf("), "a fabricated linkage check fell through: {out}");
    }

    #[test]
    fn unmapped_event_inline_closure_block_is_fully_commented() {
        // Inline-closure handler: the whole block must be commented so the body
        // and closing brace don't leak as orphaned live code.
        let input = "self.addEventListener(SSF2Event.ITEM_HURT, function() {\n\tself.x = 1;\n});";
        let out = neutralize_unmapped_ssf2(input);
        for l in out.lines() {
            assert!(l.trim_start().starts_with("//"), "line left live: {l:?}");
        }
    }

    #[test]
    fn ssf2utils_decel_inlined_with_nested_args() {
        let input = "self.setXSpeed(SSF2Utils.decel(self.getXSpeed(), Math.abs(Math.calculateXVelocity(a.get(), b.get()))));";
        let out = translate_ssf2_to_fm(input);
        assert!(!out.contains("SSF2Utils.decel"), "decel not rewritten: {out}");
        assert!(out.contains("Math.min(0,") && out.contains("Math.max(0,"), "no min/max form: {out}");
        assert!(out.contains("Math.calculateXVelocity(a.get(), b.get())"), "nested arg corrupted: {out}");
    }

    #[test]
    fn forced_crash_guard_block_removed() {
        let input = "before();\nif (!self.isForcedCrash()) {\n\tself.setIntangibility(true);\n}\nafter();";
        let out = remove_forced_crash_guards(input);
        assert!(!out.contains("isForcedCrash") || out.contains("guard removed"),
            "isForcedCrash not removed: {out}");
        assert!(!out.contains("setIntangibility(true)"), "guarded body not removed: {out}");
        assert!(out.contains("before();") && out.contains("after();"), "surrounding code lost: {out}");
    }

    #[test]
    fn object_work_put_on_the_match_goes_to_the_object() {
        let out = send_object_calls_to_the_object("match.isOnFloor();\nmatch.createVfx(a);\n");
        assert!(out.contains("self.isOnFloor()"), "object work left on the match: {out}");
        assert!(out.contains("match.createVfx(a)"), "the match's own work was moved: {out}");
    }

    #[test]
    fn a_reach_up_the_display_list_is_a_reach_at_itself() {
        let out = resolve_display_parent("parent.x = 5;\nvar a = parent.y;\nparent.setScaleX(2);");
        assert!(out.contains("self.setX(5)"), "write not resolved: {out}");
        assert!(out.contains("self.getY()"), "read not resolved: {out}");
        assert!(out.contains("self.setScaleX(2)"), "scale not resolved: {out}");
        assert!(!out.contains("parent."), "a reach was left standing: {out}");
    }

    #[test]
    fn anything_can_say_what_it_is() {
        let out = say_what_a_value_is("Engine.log(self.isOnFloor().toString());");
        assert!(out.contains("Std.string(self.isOnFloor())"), "not asked of the library: {out}");
        assert!(!out.contains(".toString()"), "the old ask was left: {out}");
        let idx = say_what_a_value_is("a = arr[i].toString();");
        assert!(idx.contains("Std.string(arr[i])"), "an indexed value mishandled: {idx}");
    }

    #[test]
    fn hit_test_ground_to_line_segment_with_null_check() {
        let neq = rewrite_hit_test_ground("if (self.hitTestGround(a, b) != null) {");
        assert!(neq.contains("new Point(a, (b) - 1), new Point(a, (b) + 1)"),
            "point not spanned into a segment: {neq}");
        assert!(neq.contains("!= null &&") && neq.contains(".length > 0"),
            "!= null not rewritten to a null-safe length test: {neq}");
        assert!(!neq.contains("hitTestGround"), "old call left: {neq}");
        assert!(!neq.contains("self.self."), "receiver kept as well as replaced: {neq}");
        let eq = rewrite_hit_test_ground("x = self.hitTestGround(a, b) == null;");
        assert!(eq.contains("== null ||") && eq.contains(".length == 0"), "== null not rewritten: {eq}");
        // A receiver the renames put on the match is dropped, not stacked.
        let mt = rewrite_hit_test_ground("if (match.hitTestGround(a, b) != null) {");
        assert!(!mt.contains("match."), "match receiver kept: {mt}");
        // The 3rd options arg is dropped; nested-object arg must not break the split.
        let opts = rewrite_hit_test_ground("self.hitTestGround(p.x, p.y - 5, { platforms: false })");
        assert!(opts.contains("new Point(p.x, (p.y - 5) - 1)") && opts.contains(".length > 0"),
            "options form mishandled: {opts}");
    }

    #[test]
    fn ssf2utils_cast_is_neutralized() {
        let input = "if (!SSF2Utils.cast(arg0.data.grabber, SSF2Character)) {\n\tself.x = 1;\n}";
        let out = translate_ssf2_to_fm(input);
        assert!(!out.lines().any(|l| !l.trim_start().starts_with("//") && l.contains("SSF2Utils.")),
            "raw SSF2Utils leaked: {out}");
    }

    #[test]
    fn invalid_gap_receiver_is_commented() {
        // `/* ? */.standtime.set(...)` is invalid Haxe; the line must be commented.
        let out = neutralize_decompiler_gaps("\t/* ? */.standtime.set(standtime.get() - 1);");
        assert!(out.trim_start().starts_with("//"), "invalid receiver not commented: {out}");
    }

    #[test]
    fn empty_rhs_gap_is_commented() {
        let out = neutralize_decompiler_gaps("\tObject.SSF2Utils = /* class */;");
        assert!(out.trim_start().starts_with("//"), "empty RHS not commented: {out}");
    }

    #[test]
    fn valid_gap_positions_are_preserved() {
        // `f(/* ? */)` (empty arg) and `return /* ? */;` (→ `return ;`) are valid.
        let keep = "\tself.doThing(/* ? */);\n\treturn /* ? */;";
        let out = neutralize_decompiler_gaps(keep);
        assert!(!out.lines().any(|l| l.trim_start().starts_with("//")),
            "valid gap position wrongly commented: {out}");
    }

    #[test]
    fn dot_brace_block_property_is_commented() {
        // The projectile/inline-block rewrite can emit `getOwner().{ var _p = … }`
        // — `.{` is invalid Haxe (and its inner `return` would also choke), so the
        // whole line must be commented even with no other gap marker present.
        let input = "\tproj.set(self.getOwner().{ var _p = make(); _p.setX(0); });";
        let out = neutralize_decompiler_gaps(input);
        assert!(out.trim_start().starts_with("//"), "dot-brace block left live: {out}");
    }

    #[test]
    fn multiline_leading_dot_continuation_is_commented() {
        // The marker ends one line (in a // comment), the chain continues with a
        // leading `.` on the next — that `.foo()` is a parse error and must be commented.
        let input = "\t/* ? */// TODO note\n\t.self.updateAnimationStats({ landType: LandType.NORMAL });";
        let out = neutralize_decompiler_gaps(input);
        assert!(!out.lines().any(|l| !l.trim_start().starts_with("//") && l.trim_start().starts_with('.')),
            "leading-dot continuation left live: {out}");
    }
}

/// A call on nothing, inside an expression, is nothing.
///
/// When a reach into the display tree has no counterpart here it reads as `null`, and the call that
/// was made on it is left as `null(args)`. As a whole statement that is a statement with nothing to
/// do; inside an expression it is one VALUE the port does not have, and blanking the statement
/// throws away the rest of the line with it.
pub fn drop_calls_on_nothing(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(at) = rest.find("null(") {
        out.push_str(&rest[..at]);
        let after = &rest[at + "null(".len()..];
        let mut depth = 1i32;
        let mut end = after.len();
        for (i, ch) in after.char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => { depth -= 1; if depth == 0 { end = i; break } }
                _ => {}
            }
        }
        out.push_str("null");
        rest = &after[(end + 1).min(after.len())..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod runtime_passes_tests {
    use super::*;

    #[test]
    fn an_objects_type_is_asked_by_the_sources_name() {
        assert_eq!(rewrite_get_type("_v3 = arg0.data.foe.getType().slice(4);"),
                   "_v3 = __typeName(arg0.data.foe).slice(4);");
        assert_eq!(rewrite_get_type("if (self.getType() == \"SSF2Character\") {}"),
                   "if (__typeName(self) == \"SSF2Character\") {}");
        // already answered: left alone
        let done = "__typeName(o) == \"x\"";
        assert_eq!(rewrite_get_type(done), done);
    }

    #[test]
    fn a_declared_stat_is_read_through_the_accessor_the_source_answers_it_by() {
        assert_eq!(rewrite_stat_accessors("t.y = t.y - (character.getHeight() * 1.5);"),
                   "t.y = t.y - (__declaredStat(character, \"height\") * 1.5);");
        assert_eq!(rewrite_stat_accessors("x + self.getWidth()"),
                   "x + __declaredStat(self, \"width\")");
        assert_eq!(rewrite_stat_accessors("players[p].getWidth()"),
                   "__declaredStat(players[p], \"width\")");
        // a bare name is nobody's: left alone
        assert_eq!(rewrite_stat_accessors("getWidth()"), "getWidth()");
    }

    #[test]
    fn a_method_called_by_a_worked_out_name_goes_through_the_dispatcher() {
        assert_eq!(rewrite_dynamic_calls("if (__truthy(_v2[_v4](\"canReceiveKnockback\"))) {}"),
                   "if (__truthy(__callByName(_v2, _v4, [\"canReceiveKnockback\"]))) {}");
        assert_eq!(rewrite_dynamic_calls("x = o[\"getItemStat\"](a, b(c));"),
                   "x = __callByName(o, \"getItemStat\", [a, b(c)]);");
        // a list indexed by a number is a list
        let list = "fns[0](x); arr[i + 1](y);";
        assert_eq!(rewrite_dynamic_calls(list), list);
    }

    #[test]
    fn a_hitbox_stat_read_back_is_answered_from_the_script() {
        assert_eq!(rewrite_attack_box_stat_reads("kb = f(self.getAttackBoxStat(1, \"damage\"), 2);"),
                   "kb = f(__hbStat(0, \"damage\"), 2);");
        assert_eq!(rewrite_attack_box_stat_reads("self.getAttackBoxStat(n, k)"),
                   "__hbStat((n) - 1, k)");
    }

    #[test]
    fn a_source_state_number_is_asked_by_name() {
        let mut m = BTreeMap::new();
        m.insert(14, "INJURED".to_string());
        m.insert(26, "CRASH_LAND".to_string());
        set_source_state_names(m);
        assert_eq!(rewrite_numeric_state_compares("if ((arg0.getState() == 14) || (arg0.getState() != 26)) {}"),
                   "if ((__srcStateIs(arg0, \"INJURED\")) || (!__srcStateIs(arg0, \"CRASH_LAND\"))) {}");
        // a number the file does not name stays a number
        assert_eq!(rewrite_numeric_state_compares("a.getState() == 99"), "a.getState() == 99");
        set_source_state_names(BTreeMap::new());
    }

    #[test]
    fn a_shift_in_source_pixels_crosses_the_boundary() {
        let _g = PositionsCrossGuard::install();
        assert_eq!(scale_move_args("foe.move(0, -25); foe.move(a, __sp(b));"),
                   "foe.move(0, __sp(-25)); foe.move(__sp(a), __sp(b));");
    }

    #[test]
    fn hitbox_writes_are_noted_for_the_read_back() {
        set_mirror_hitbox_writes(true);
        let out = note_hitbox_writes("self.updateHitboxStats(0, { angle: 50, damage: 7, hitSoundOverride: \"x\" });\n");
        assert_eq!(out, "self.updateHitboxStats(0, { angle: 50, damage: 7, hitSoundOverride: \"x\" }); __hbNote(0, \"angle\", 50); __hbNote(0, \"damage\", 7);\n");
        set_mirror_hitbox_writes(false);
        let plain = "self.updateHitboxStats(0, { angle: 50 });";
        assert_eq!(note_hitbox_writes(plain), plain);
    }

    #[test]
    fn a_projectiles_force_attack_is_a_move_to_its_own_state() {
        let mut m = BTreeMap::new();
        m.insert("pull".to_string(), "PULL".to_string());
        let code = "\t// SSF2 buffers a raw attack input\n\t// [SSF2-only: forceAttack] self.forceAttack(\"pull\");\n";
        assert_eq!(rewrite_projectile_force_attack(code, &m),
                   "\tCommon.toLocalState(LState.PULL); // SSF2 forceAttack(\"pull\")\n");
        let other = "\t// [SSF2-only: forceAttack] self.forceAttack(\"b\");\n";
        assert_eq!(rewrite_projectile_force_attack(other, &m), other);
    }

    #[test]
    fn a_summoners_move_is_asked_by_this_engines_name() {
        let mut m = BTreeMap::new();
        m.insert("b".to_string(), "special_neutral".to_string());
        assert_eq!(rename_animation_literals("if (c.getAnimation() == \"b\" || c.getAnimation() != \"kirby_sora\") {}", &m),
                   "if (c.getAnimation() == \"special_neutral\" || c.getAnimation() != \"kirby_sora\") {}");
    }

    #[test]
    fn a_handler_named_destroyed_keeps_its_name() {
        assert_eq!(rename_self_destroy("self.addEventListener(E, self.destroyed); self.destroy();"),
                   "self.addEventListener(E, self.destroyed); _destroySelf();");
    }

    #[test]
    fn a_label_asked_of_the_summoner_is_signalled() {
        assert_eq!(rewrite_owner_label_jumps("self.character.getStanceMC().self.stancePlayFrame(\"finish\");"),
                   "__askLabel(self.character, \"finish\");");
    }
}


thread_local! {
    static SELF_IS_PROJECTILE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Translate as the code of a projectile (`self` is the projectile) for the length of `f`.
pub fn as_projectile<T>(f: impl FnOnce() -> T) -> T {
    let was = SELF_IS_PROJECTILE.with(|c| c.replace(true));
    let out = f();
    SELF_IS_PROJECTILE.with(|c| c.set(was));
    out
}

fn self_is_projectile() -> bool { SELF_IS_PROJECTILE.with(|c| c.get()) }

/// COLOUR SETTINGS the engine can show: a hue, saturation, brightness or contrast adjustment is a
/// filter here, and a fade is the object's alpha; clearing them takes both off. A setting that
/// also multiplies or offsets the colour channels has nothing here to show it with, and is left for
/// the pass that strikes what has no counterpart (half of it would draw a different colour).
fn colour_settings_the_engine_can_show(code: &str) -> String {
    if IN_STAGE.with(|c| c.get()) || STAGE_CONTENT.with(|c| c.get()) || !code.contains("setColorFilters(") {
        return code.to_string()
    }
    const SHOWN: &[&str] = &["hue", "saturation", "brightness", "contrast", "alphaMultiplier"];
    let key = crate::re!(r"([A-Za-z_]\w*)\s*:");
    let mut out = String::with_capacity(code.len());
    let mut rest = code;
    while let Some(at) = rest.find("setColorFilters(") {
        let open = at + "setColorFilters".len();
        let Some(close) = matching_paren(rest, open) else { out.push_str(&rest[..open + 1]); rest = &rest[open + 1..]; continue };
        let args = split_top_level(&rest[open + 1..close]);
        let (target, setting) = match args.as_slice() {
            [o] => ("self".to_string(), o.trim().to_string()),
            [t, o] => (t.trim().trim_end_matches(".getViewRootContainer()").to_string(), o.trim().to_string()),
            _ => (String::new(), String::new()),
        };
        let clear = matches!(setting.as_str(), "null" | "false");
        let shown = setting.starts_with('{') && setting.ends_with('}') && {
            let keys = key.captures_iter(&setting).map(|c| c[1].to_string()).collect::<Vec<_>>();
            !keys.is_empty() && keys.iter().all(|k| SHOWN.contains(&k.as_str()))
        };
        if target.is_empty() || !(clear || shown) {
            out.push_str(&rest[..close + 1]); rest = &rest[close + 1..]; continue
        }
        // the whole call, receiver and all (`SSF2Utils.`, `self.`)
        let head = &rest[..at];
        let cut = head.trim_end_matches(|c: char| c.is_alphanumeric() || c == '_' || c == '.');
        out.push_str(cut);
        out.push_str(&format!("__colorFilters({target}, {})", if clear { "null" } else { &setting }));
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    out
}

/// A JUMP TO A LABEL ON ANOTHER ENTITY'S CLIP (`char.stancePlayFrame("attack")`: ness's flash sends
/// ness on to the attack part of his special) is asked of that entity's script, which knows its
/// own labels; read as an animation of that name it was struck as one it does not have. The
/// entity's own jumps are left to the label passes.
fn label_jumps_on_others(code: &str) -> String {
    if !["stancePlayFrame(", "playFrameLabel(", "gotoAndPlay(", "gotoAndStop("].iter().any(|k| code.contains(k)) { return code.to_string() }
    // a stage's jumps are its own emitter's: a clip it plays is a plane child it drew, not an
    // entity with a script to ask (bowser's castle's podoboos stopped stepping when asked)
    if IN_STAGE.with(|c| c.get()) || SELF_IS_STAGE.with(|c| c.get()) { return code.to_string() }
    // a receiver is a chain of names, calls and index reads (`dotArray.get()[i.get()]`)
    crate::re!(r#"((?:[A-Za-z_]\w*(?:\(\))?(?:\[[^\]\[]*\])*\.)*[A-Za-z_]\w*(?:\(\))?(?:\[[^\]\[]*\])*)\.(?:stancePlayFrame|playFrameLabel|gotoAndPlay|gotoAndStop)\(\s*("[^"]+")\s*\)"#)
        .replace_all(code, |c: &regex::Captures| {
            // the entity's stance clip is the entity, as far as its labels go
            let recv = c[1].trim_end_matches(".getStanceMC()");
            if recv.split('.').all(|r| r == "self") { return c[0].to_string() }
            // the fighter's script resolves the label against its own table; a projectile can
            // only ask the entity for it
            if self_is_projectile() { return format!("__askLabel({recv}, {})", &c[2]) }
            format!("_jumpLabel({recv}, {}, null)", &c[2])
        }).into_owned()
}

/// THE FIGHTER BEHIND `self`, asked of the api by the object or by its uid: the fighter itself in
/// its own code, and the one that made it in a projectile's. A stage's `self` is no fighter's, and
/// is left as it is.
fn the_character_behind_self(code: &str) -> String {
    if IN_STAGE.with(|c| c.get()) || !code.contains("getCharacter(") { return code.to_string() }
    let who = if self_is_projectile() { "self.getOwner()" } else { "self" };
    crate::re!(r"\b(?:SSF2API|match)\.getCharacter\(\s*self(?:\.self)?(?:\.get(?:Uid|UID|ID)\(\))?\s*\)")
        .replace_all(code, who).into_owned()
}

thread_local! {
    static IN_STAGE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Translating a stage's own scripts for as long as this lives.
pub struct StageScope(bool);
impl StageScope {
    pub fn enter() -> StageScope { StageScope(IN_STAGE.with(|c| c.replace(true))) }
}
impl Drop for StageScope {
    fn drop(&mut self) { let was = self.0; IN_STAGE.with(|c| c.set(was)); }
}

/// THE STAGE, from a fighter's or a projectile's script: the global `stage`, which every script
/// here has (measured live: a fighter's script reads the death and camera bounds off it). Asked of
/// the match, which has no such method, the call died where it ran; struck as the source's own
/// call, so did every line that only wanted the stage's bounds.
fn the_stage_from_outside(code: &str) -> String {
    // a stage and the objects it makes reach it their own way (the object is handed its stage)
    if IN_STAGE.with(|c| c.get()) || STAGE_CONTENT.with(|c| c.get()) { return code.to_string() }
    code.replace("match.getStage()", "stage").replace("SSF2API.getStage()", "stage")
}

/// A rectangle off the stage is in this engine's units, and the script compares it with its own
/// positions, which it keeps in the source's (`__isp`): converted the same way. Run once the
/// source's names for the bounds have become this engine's.
fn stage_rects_in_source_units(code: &str) -> String {
    if IN_STAGE.with(|c| c.get()) || STAGE_CONTENT.with(|c| c.get()) { return code.to_string() }
    crate::re!(r"(^|[^\w.])stage\.(getCameraBounds|getDeathBounds)\(\)\.getRectangle\(\)")
        .replace_all(code, "${1}__srcRect(stage.${2}())").into_owned()
}

#[cfg(test)]
mod listen_other_tests {
    use super::{listeners_on_others_persist_until_end, others_ended_through_their_own_end};

    #[test]
    fn the_tracking_helper_does_not_call_itself() {
        let helper = "function __listenOther(o, e, f) { if (o == null) { return; } __others.push({ o: o, e: e, f: f }); o.addEventListener(e, f, { persistent: true }); }";
        let off = "function __unlistenOther(o, e, f) { o.removeEventListener(e, f); }";
        let code = format!("{helper}\n{off}\ncharacter.addEventListener(GameObjectEvent.HIT_RECEIVED, remove);\ncharacter.removeEventListener(GameObjectEvent.HIT_RECEIVED, remove);\nself.removeEventListener(GameObjectEvent.HIT_RECEIVED, remove);\n");
        let out = listeners_on_others_persist_until_end(&code);
        assert!(out.contains(helper), "the helper was rewritten: {out}");
        assert!(out.contains(off), "the unlisten helper was rewritten: {out}");
        assert!(out.contains("__listenOther(character, GameObjectEvent.HIT_RECEIVED, remove)"), "{out}");
        assert!(out.contains("__unlistenOther(character, GameObjectEvent.HIT_RECEIVED, remove)"), "{out}");
        assert!(out.contains("self.removeEventListener(GameObjectEvent.HIT_RECEIVED, remove)"), "its own stays: {out}");
    }

    #[test]
    fn another_object_is_asked_to_end_and_self_is_not() {
        let out = others_ended_through_their_own_end("proj.get().kill();\nself.kill();\nif (x != null) { a.b[0].kill(); }");
        assert!(out.contains("__endOther(proj.get());"), "{out}");
        assert!(out.contains("self.kill();"), "{out}");
        assert!(out.contains("__endOther(a.b[0]);"), "{out}");
    }
}

#[cfg(test)]
mod declared_type_tests {
    use super::*;

    fn int(n: &str) -> Vec<(String, String)> { vec![(n.to_string(), "int".to_string())] }

    #[test]
    fn a_write_to_an_int_field_keeps_the_whole_part() {
        let out = coerce_int_field_writes("self.damage = arg0.data.damage * 1.2;\n", &int("damage"));
        assert_eq!(out, "self.damage = __int(arg0.data.damage * 1.2);\n");
    }

    #[test]
    fn literals_comparisons_and_other_fields_are_left_alone() {
        let code = "self.damage = 7;\nif (self.damage == 3) {}\nself.damageX = 1.5;\nself.other = 1.5;\n";
        assert_eq!(coerce_int_field_writes(code, &int("damage")), code);
    }

    #[test]
    fn compound_writes_and_nothing_are_coerced_too() {
        let out = coerce_int_field_writes("self.n += a / 2;\nself.n = null;\n", &int("n"));
        assert_eq!(out, "self.n = __int(self.n + (a / 2));\nself.n = 0;\n");
    }

    #[test]
    fn a_value_with_brackets_runs_to_the_statement_end() {
        let out = coerce_int_field_writes("{ self.p = f(a, (b; c)); }", &int("p"));
        assert_eq!(out, "{ self.p = __int(f(a, (b; c))); }");
    }

    #[test]
    fn an_effect_named_as_it_runs_is_seen_however_the_call_is_spelled() {
        assert!(names_effect_at_runtime("attachEffect(\"fx_\" + n, { y: 2 });"));
        assert!(names_effect_at_runtime("self.attachEffect(self.EFFECTS[i]);"));
        assert!(!names_effect_at_runtime("self.attachEffect(\"fx_hit\");"));
        assert!(!names_effect_at_runtime("self.myattachEffect(n);"));
    }

    #[test]
    fn the_fighter_behind_self_is_the_fighter_or_its_projectiles_owner() {
        assert_eq!(the_character_behind_self("match.getCharacter(self).endAnimation();"), "self.endAnimation();");
        assert_eq!(the_character_behind_self("c = SSF2API.getCharacter(self.getUid());"), "c = self;");
        let p = as_projectile(|| the_character_behind_self("match.getCharacter(self).x;"));
        assert_eq!(p, "self.getOwner().x;");
        assert_eq!(the_character_behind_self("match.getCharacter(other)"), "match.getCharacter(other)");
    }

    #[test]
    fn colour_settings_the_engine_can_show_become_filters_and_the_rest_stay() {
        assert_eq!(colour_settings_the_engine_can_show("SSF2Utils.setColorFilters(p.getViewRootContainer(), { brightness: -10, hue: 60 });"),
            "__colorFilters(p, { brightness: -10, hue: 60 });");
        assert_eq!(colour_settings_the_engine_can_show("self.setColorFilters(null);"), "__colorFilters(self, null);");
        let mixed = "self.setColorFilters({ hue: -60, redMultiplier: -1 });";
        assert_eq!(colour_settings_the_engine_can_show(mixed), mixed);
    }

    #[test]
    fn an_animation_compared_through_a_variable_is_renamed_too() {
        let map: BTreeMap<String, String> = [("b".to_string(), "special_neutral".to_string())].into_iter().collect();
        let code = "\t_v1 = char.getAnimation();\n\tif ((_v1 == \"b\") || (\"b\" == _v1)) { x = 1; }\n\tif (_v1 == \"nope\") {}\n";
        let out = rename_animation_literals(code, &map);
        assert!(out.contains("(_v1 == \"special_neutral\")") && out.contains("(\"special_neutral\" == _v1)"), "{out}");
        assert!(out.contains("_v1 == \"nope\""), "{out}");
    }

    #[test]
    fn a_label_jump_on_another_entity_is_asked_of_its_script() {
        assert_eq!(label_jumps_on_others("self.char.stancePlayFrame(\"attack\");"), "_jumpLabel(self.char, \"attack\", null);");
        assert_eq!(label_jumps_on_others("self.self.stancePlayFrame(\"loop\");"), "self.self.stancePlayFrame(\"loop\");");
        assert_eq!(label_jumps_on_others("proj.get().playFrameLabel(\"boom\");"), "_jumpLabel(proj.get(), \"boom\", null);");
        assert_eq!(label_jumps_on_others("self.proj.getStanceMC().gotoAndStop(\"boom\");"), "_jumpLabel(self.proj, \"boom\", null);");
        assert_eq!(label_jumps_on_others("dotArray.get()[i.get()].getStanceMC().gotoAndStop(\"wait\");"), "_jumpLabel(dotArray.get()[i.get()], \"wait\", null);");
        assert_eq!(label_jumps_on_others("self.playFrameLabel(\"stand\");"), "self.playFrameLabel(\"stand\");");
    }

}
