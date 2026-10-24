//! Each script carries only the runtime it uses.
//!
//! The runtime's files are split into definitions, one per top-level statement (`var X = ...`,
//! `X.y = ...`, `function f`). A definition is kept when something kept names it, starting from
//! what the object itself carries (its classes, its data, its entry and the code on its
//! keyframes): a global by its name, a module's function (`X.y`) once `X` is kept and `X.y` is
//! written, or `.y` is asked of anything while `X` is handed around as a value (the runtime's
//! kind module, a class in the data). Names are matched as words in the code, strings included
//! (comments are not), so a doubt keeps a definition rather than dropping it. A class's `build`
//! is kept whole with its class: objects cross scripts (another script calls a method on this
//! script's object), so what this script's own code calls says nothing about what is called.

use std::collections::BTreeSet;

/// One definition: its text, the names that keep it, and what it names.
struct Unit {
    text: String,
    /// a global it defines (kept when the name appears)
    global: Option<String>,
    /// (owner, member): kept when the owner is kept and `.member` appears (a class's `build`
    /// among them, whole: every member it gives an object stays)
    member: Option<(String, String)>,
    /// kept with its owner, whatever is named
    with: Option<String>,
}

/// A text without its line comments (strings kept whole).
pub fn code_text(text: &str) -> String { code_only(text) }

/// A text without its line comments (strings kept whole): what a comment mentions keeps nothing.
fn code_only(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let mut q: Option<char> = None;
        let mut prev = ' ';
        let mut chars = line.char_indices().peekable();
        let mut end = line.len();
        while let Some((i, c)) = chars.next() {
            match q {
                Some(open) => { if c == open && prev != '\\' { q = None } }
                None => match c {
                    '"' | '\'' => q = Some(c),
                    '/' if chars.peek().map(|p| p.1) == Some('/') => { end = i; break }
                    _ => {}
                },
            }
            prev = if prev == '\\' && c == '\\' { ' ' } else { c };
        }
        out.push_str(&line[..end]);
        out.push('\n');
    }
    out
}

/// What kept code names.
#[derive(Default)]
struct Seen {
    /// every word
    names: BTreeSet<String>,
    /// words that follow a `.`
    members: BTreeSet<String>,
    /// `a.b` as written
    pairs: BTreeSet<String>,
    /// words used as a value: not followed by a `.` (a module handed around, asked by `.y` later)
    values: BTreeSet<String>,
}

/// Words (identifiers) in a text's code.
fn words(text: &str, seen: &mut Seen) {
    let text = &code_only(text);
    let b = text.as_bytes();
    let mut i = 0;
    let mut last: Option<(usize, usize)> = None;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_alphabetic() || c == b'_' {
            let s = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') { i += 1; }
            let w = &text[s..i];
            if s > 0 && b[s - 1] == b'.' {
                seen.members.insert(w.to_string());
                if let Some((a, e)) = last { if e + 1 == s { seen.pairs.insert(format!("{}.{w}", &text[a..e])); } }
            }
            // a definition's own name is not a use of it
            let defined = text[..s].ends_with("var ") || text[..s].ends_with("function ");
            if !defined && (i >= b.len() || b[i] != b'.') { seen.values.insert(w.to_string()); }
            seen.names.insert(w.to_string());
            last = Some((s, i));
        } else { i += 1; }
    }
}

/// How much a line opens or closes, outside strings and line comments.
fn depth_change(line: &str) -> i32 {
    let mut d = 0;
    let mut q: Option<char> = None;
    let mut prev = ' ';
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match q {
            Some(open) => { if c == open && prev != '\\' { q = None } }
            None => match c {
                '"' | '\'' => q = Some(c),
                '/' if chars.peek() == Some(&'/') => break,
                '{' | '(' | '[' => d += 1,
                '}' | ')' | ']' => d -= 1,
                _ => {}
            },
        }
        prev = if prev == '\\' && c == '\\' { ' ' } else { c };
    }
    d
}

/// How many statements end on a line at its top level (outside brackets and strings).
fn statements(line: &str) -> usize {
    let mut n = 0;
    let mut d = 0i32;
    let mut q: Option<char> = None;
    let mut prev = ' ';
    for c in line.chars() {
        match q {
            Some(open) => { if c == open && prev != '\\' { q = None } }
            None => match c {
                '"' | '\'' => q = Some(c),
                '{' | '(' | '[' => d += 1,
                '}' | ')' | ']' => d -= 1,
                ';' if d == 0 => n += 1,
                _ => {}
            },
        }
        prev = c;
    }
    n
}

fn ident_prefix(s: &str) -> String {
    s.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect()
}

/// A file's definitions, in order.
fn units(text: &str) -> Vec<Unit> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out: Vec<Unit> = Vec::new();
    let mut pending = String::new();
    let mut i = 0;
    while i < lines.len() {
        let l = lines[i];
        // comments and blank lines ride with what follows them
        if l.trim().is_empty() || l.starts_with("//") || l.starts_with(char::is_whitespace) && l.trim_start().starts_with("//") && out.is_empty() {
            pending.push_str(l);
            pending.push('\n');
            i += 1;
            continue;
        }
        // a top-level statement: to where its brackets close
        let start = i;
        let mut depth = depth_change(l);
        i += 1;
        while depth > 0 && i < lines.len() { depth += depth_change(lines[i]); i += 1; }
        let block = &lines[start..i];
        let head = block[0];
        let text: String = format!("{}{}", std::mem::take(&mut pending), block.iter().map(|x| format!("{x}\n")).collect::<String>());
        let unit = if let Some(rest) = head.strip_prefix("var ") {
            Unit { text, global: Some(ident_prefix(rest)), member: None, with: None }
        } else if let Some(rest) = head.strip_prefix("function ") {
            Unit { text, global: Some(ident_prefix(rest)), member: None, with: None }
        } else {
            let owner = ident_prefix(head);
            let rest = &head[owner.len()..];
            match rest.strip_prefix('.') {
                // a module's function or value, one to a statement
                Some(r) if r[ident_prefix(r).len()..].trim_start().starts_with('=') && statements(head) <= 1 =>
                    Unit { text, global: None, member: Some((owner, ident_prefix(r))), with: None },
                // several statements on one line (a table of values): kept with its owner
                _ => Unit { text, global: None, member: None, with: Some(owner) },
            }
        };
        out.push(unit);
    }
    if !pending.is_empty() {
        out.push(Unit { text: pending, global: None, member: None, with: Some(String::new()) });
    }
    out
}

/// The runtime a script needs: `files` (path, text) in order, pruned to what `seed` reaches.
pub fn shake(files: &[(&str, &str)], seed: &str) -> String {
    let parsed: Vec<(&str, Vec<Unit>)> = files.iter().map(|(p, t)| (*p, units(t))).collect();
    let mut seen = Seen::default();
    words(seed, &mut seen);
    let mut kept: Vec<Vec<bool>> = parsed.iter().map(|(_, us)| vec![false; us.len()]).collect();
    // an owner kept: a global kept, or a class whose build is kept
    let owner_kept = |o: &str, names: &BTreeSet<String>| names.contains(o);
    loop {
        let mut grew = false;
        for (fi, (_, us)) in parsed.iter().enumerate() {
            for (ui, u) in us.iter().enumerate() {
                if kept[fi][ui] { continue }
                let keep = match (&u.global, &u.member, &u.with) {
                    (Some(g), _, _) => seen.names.contains(g),
                    (_, Some((o, m)), _) => owner_kept(o, &seen.names)
                        && (seen.pairs.contains(&format!("{o}.{m}")) || (seen.values.contains(o) && seen.members.contains(m))),
                    (_, _, Some(w)) => w.is_empty() || owner_kept(w, &seen.names),
                    _ => true,
                };
                if keep {
                    kept[fi][ui] = true;
                    words(&u.text, &mut seen);
                    grew = true;
                }
            }
        }
        if !grew { break }
    }
    let mut out = String::new();
    for (fi, (path, us)) in parsed.iter().enumerate() {
        // a file's lead comment and trailing lines count only when something of it is kept
        let any = us.iter().enumerate().any(|(ui, u)| kept[fi][ui] && (u.global.is_some() || u.member.is_some()));
        if !any { continue }
        out.push_str(&format!("// ---- runtime: runtime/{path}\n"));
        for (ui, u) in us.iter().enumerate() {
            if kept[fi][ui] { out.push_str(&u.text); }
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::shake;

    #[test]
    fn keeps_what_is_named_and_drops_the_rest() {
        let a = "// a module\nvar A = {};\nA.used = function() { return B.x(); };\nA.unused = function() { return C.y(); };\n";
        let b = "var B = {};\nB.x = function() { return 1; };\n";
        let c = "var C = {};\nC.y = function() { return 2; };\n";
        let k = "// class K\nvar K = { name: \"K\" };\nK.build = function(me) {\n\tme.n = 0;\n\tme.called = function() { return A.used(); };\n\tme.never = function() {\n\t\treturn 3;\n\t};\n};\n";
        let out = shake(&[("a.hx", a), ("b.hx", b), ("c.hx", c), ("k.hx", k)], "var o = {}; K.build(o); o.called();");
        assert!(out.contains("A.used") && out.contains("B.x") && out.contains("me.called") && out.contains("me.never"));
        assert!(!out.contains("A.unused") && !out.contains("C.y"));
        assert!(out.contains("me.n = 0;") && out.contains("K.build = function(me) {"));
    }
}
