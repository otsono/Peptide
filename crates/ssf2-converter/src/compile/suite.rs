//! The language suite: small functions whose AS3 result is known, compiled the way content is,
//! for the engine's own interpreter to run with the runtime's `AS3` layer
//! (`tools/tests/lang_suite.py`). Each case is a rule AS3 states; the expected value is what AS3
//! prints for it with `String(...)`. A case that fails is a place the compiled code does not mean
//! what the source meant.

use crate::model::ast::*;

use super::lower::quote;
use super::Runtime;

fn b(e: Expr) -> Box<Expr> { Box::new(e) }
fn int(i: i64) -> Expr { Expr::Int(i) }
fn num(n: f64) -> Expr { Expr::Num(n) }
fn st(s: &str) -> Expr { Expr::Str(s.into()) }
fn reg(r: u32) -> Expr { Expr::Local(r) }
fn bin(op: BinOp, x: Expr, y: Expr) -> Expr { Expr::Binary(op, b(x), b(y)) }
fn and(x: Expr, y: Expr) -> Expr { Expr::And(b(x), b(y)) }
fn or(x: Expr, y: Expr) -> Expr { Expr::Or(b(x), b(y)) }
fn not(x: Expr) -> Expr { Expr::Unary(UnOp::Not, b(x)) }
fn conv(c: Conv, x: Expr) -> Expr { Expr::Convert(c, b(x)) }
fn name(n: &str) -> Expr { Expr::Name(n.into()) }
fn call(f: Expr, args: Vec<Expr>) -> Expr { Expr::Call(b(f), args) }
fn method(o: Expr, m: &str, args: Vec<Expr>) -> Expr { Expr::Call(b(Expr::Member(b(o), m.into())), args) }
fn index(o: Expr, k: Expr) -> Expr { Expr::Index(b(o), b(k)) }
fn set(r: u32, v: Expr) -> Stmt { Stmt::Assign(Place::Local(r), v) }
fn ret(v: Expr) -> Stmt { Stmt::Return(Some(v)) }
fn iff(c: Expr, a: Vec<Stmt>, e: Vec<Stmt>) -> Stmt { Stmt::If(c, a, e) }
fn test_value(c: Expr) -> Vec<Stmt> { vec![iff(c, vec![ret(st("t"))], vec![ret(st("f"))])] }

/// (name, the body of a function of no arguments, what AS3 gives)
fn cases() -> Vec<(&'static str, Vec<Stmt>, &'static str)> {
    let r = |v: Expr| vec![ret(v)];
    vec![
        // a test is truthiness
        ("if 0", test_value(int(0)), "f"),
        ("if 1", test_value(int(1)), "t"),
        ("if empty string", test_value(st("")), "f"),
        ("if string", test_value(st("a")), "t"),
        ("if null", test_value(Expr::Null), "f"),
        ("if NaN", test_value(num(f64::NAN)), "f"),
        ("if object", test_value(Expr::Object(vec![])), "t"),
        ("not 0", r(not(int(0))), "true"),
        ("not string", r(not(st("a"))), "false"),
        // && and || give back an operand
        ("1 && 2", r(and(int(1), int(2))), "2"),
        ("0 && 2", r(and(int(0), int(2))), "0"),
        ("0 || 5", r(or(int(0), int(5))), "5"),
        ("a || b", r(or(st("a"), st("b"))), "a"),
        ("null || x", r(or(Expr::Null, st("x"))), "x"),
        ("&& in a test", test_value(and(int(1), st(""))), "f"),
        ("|| in a test", test_value(or(int(0), st("a"))), "t"),
        ("&& reads its left side once", vec![
            set(1, int(0)),
            Stmt::Expr(and(Expr::Assign(Box::new(Place::Local(1)), b(bin(BinOp::Add, reg(1), int(1)))), int(7))),
            ret(reg(1)),
        ], "1"),
        // comparisons
        ("null > -1", r(bin(BinOp::Gt, Expr::Null, int(-1))), "true"),
        ("NaN < 1", r(bin(BinOp::Lt, num(f64::NAN), int(1))), "false"),
        ("not (NaN < 1)", r(not(bin(BinOp::Lt, num(f64::NAN), int(1)))), "true"),
        ("10 < 9 as strings", r(bin(BinOp::Lt, st("10"), st("9"))), "true"),
        ("2 <= 2", r(bin(BinOp::Le, num(2.5), num(2.5))), "true"),
        ("null == undefined", r(bin(BinOp::Eq, Expr::Null, Expr::Undefined)), "true"),
        // numbers and conversions
        ("-1 >>> 0", r(bin(BinOp::UShr, int(-1), int(0))), "4294967295"),
        ("16 >>> 2", r(bin(BinOp::UShr, int(16), int(2))), "4"),
        ("int(-1.5)", r(conv(Conv::Int, num(-1.5))), "-1"),
        ("int(\"12\")", r(conv(Conv::Int, st("12"))), "12"),
        ("int(2147483648)", r(conv(Conv::Int, num(2147483648.0))), "-2147483648"),
        ("int(NaN)", r(conv(Conv::Int, num(f64::NAN))), "0"),
        ("uint(-1)", r(conv(Conv::Uint, int(-1))), "4294967295"),
        ("Number(null)", r(conv(Conv::Number, Expr::Null)), "0"),
        ("Number(\"\")", r(conv(Conv::Number, st(""))), "0"),
        ("Number(\"2.5\")", r(conv(Conv::Number, st("2.5"))), "2.5"),
        ("String(null)", r(conv(Conv::String, Expr::Null)), "null"),
        ("String(1.0)", r(conv(Conv::String, num(1.0))), "1"),
        ("Boolean(\"\")", r(conv(Conv::Boolean, st(""))), "false"),
        ("int() call", r(call(name("int"), vec![num(3.7)])), "3"),
        ("isNaN", r(call(name("isNaN"), vec![num(f64::NAN)])), "true"),
        ("1 / 2", r(bin(BinOp::Div, int(1), int(2))), "0.5"),
        ("-7 % 3", r(bin(BinOp::Mod, int(-7), int(3))), "-1"),
        ("a + 1.5", r(bin(BinOp::Add, st("a"), num(1.5))), "a1.5"),
        ("1 + \"2\"", r(bin(BinOp::Add, int(1), st("2"))), "12"),
        ("int.MAX_VALUE", r(Expr::Member(b(name("int")), "MAX_VALUE".into())), "2147483647"),
        ("Number.MAX_VALUE > 1e300", r(bin(BinOp::Gt, Expr::Member(b(name("Number")), "MAX_VALUE".into()), num(1e300))), "true"),
        ("NaN by name is NaN", r(call(name("isNaN"), vec![name("NaN")])), "true"),
        ("Math.max of three", r(method(name("Math"), "max", vec![int(1), int(5), int(3)])), "5"),
        ("Math.floor(Math.PI * 1000)", r(method(name("Math"), "floor", vec![bin(BinOp::Mul, Expr::Member(b(name("Math")), "PI".into()), int(1000))])), "3141"),
        // arrays and keys
        ("a[-1]", r(index(Expr::Array(vec![int(1), int(2)]), int(-1))), "null"),
        ("a[length - 1] on empty", vec![set(1, Expr::Array(vec![])), ret(index(reg(1), bin(BinOp::Sub, Expr::Member(b(reg(1)), "length".into()), int(1))))], "null"),
        ("a[1]", r(index(Expr::Array(vec![int(5), int(6)]), int(1))), "6"),
        // a whole-number key read in place on an array only: anything else takes the general read
        ("a[2] past the end", r(index(Expr::Array(vec![int(5), int(6)]), int(2))), "null"),
        // a loop that only counts is given its count at once: it ends where counting would, and
        // one already past its bound is left where it is
        ("a counting loop ends at its bound", vec![
            set(1, int(0)),
            Stmt::While(bin(BinOp::Lt, reg(1), int(5)), vec![set(1, bin(BinOp::Add, reg(1), int(1)))]),
            ret(reg(1)),
        ], "5"),
        ("a counting loop past its bound", vec![
            set(1, int(7)),
            Stmt::While(bin(BinOp::Lt, reg(1), int(5)), vec![set(1, bin(BinOp::Add, reg(1), int(1)))]),
            ret(reg(1)),
        ], "7"),
        ("an object read by a whole number", vec![
            set(1, Expr::Object(vec![])), set(2, int(0)),
            Stmt::Assign(Place::Index(reg(1), reg(2)), int(7)),
            ret(index(reg(1), int(0))),
        ], "7"),
        ("put then get by key", vec![
            set(1, Expr::Object(vec![])), set(2, st("k")),
            Stmt::Assign(Place::Index(reg(1), reg(2)), int(9)),
            ret(index(reg(1), reg(2))),
        ], "9"),
        ("key in object", vec![
            set(1, Expr::Object(vec![])), set(2, st("k")),
            Stmt::Assign(Place::Index(reg(1), reg(2)), int(9)),
            ret(bin(BinOp::In, reg(2), reg(1))),
        ], "true"),
        ("literal key is a field", vec![
            set(1, Expr::Object(vec![(st("a"), int(1))])),
            ret(index(reg(1), st("a"))),
        ], "1"),
        ("a literal read by a key worked out as it runs", vec![
            set(1, Expr::Object(vec![(st("a"), int(1)), (st("b"), int(2))])), set(2, bin(BinOp::Add, st("b"), st(""))),
            ret(index(reg(1), reg(2))),
        ], "2"),
        ("a literal's keys enumerate", vec![
            set(1, Expr::Object(vec![(st("a"), int(1)), (st("b"), int(2))])), set(2, int(0)), set(3, st("")),
            Stmt::While(Expr::HasNext2 { object: 1, index: 2 }, vec![
                set(3, bin(BinOp::Add, reg(3), Expr::NextName(b(reg(1)), b(reg(2))))),
            ]),
            ret(reg(3)),
        ], "ab"),
        ("a literal written by a key", vec![
            set(1, Expr::Object(vec![(st("a"), int(1))])), set(2, st("a")),
            Stmt::Assign(Place::Index(reg(1), reg(2)), int(5)),
            ret(Expr::Member(b(reg(1)), "a".into())),
        ], "5"),
        ("a number in base 2", r(method(num(5.0), "toString", vec![int(2)])), "101"),
        ("a number's text", r(method(num(2.5), "toString", vec![])), "2.5"),
        ("toFixed", r(method(num(1.23456), "toFixed", vec![int(2)])), "1.23"),
        ("a string's slice from the end", r(method(st("abcdef"), "slice", vec![int(-2)])), "ef"),
        ("reverse gives the array back", r(method(method(Expr::Array(vec![int(1), int(2)]), "reverse", vec![]), "join", vec![st("")])), "21"),
        ("replace takes the first", r(method(st("aXbX"), "replace", vec![st("X"), st("-")])), "a-bX"),
        ("numeric sort", r(method(method(Expr::Array(vec![int(10), int(9), int(100)]), "sort", vec![int(16)]), "join", vec![st(",")])), "9,10,100"),
        ("push gives the length", r(method(Expr::Array(vec![int(1)]), "push", vec![int(2)])), "2"),
        ("new Array(3)", r(Expr::Member(b(Expr::New(b(name("Array")), vec![int(3)])), "length".into())), "3"),
        // statements
        ("while with continue", vec![
            set(1, int(0)), set(2, int(0)),
            Stmt::While(bin(BinOp::Lt, reg(1), int(5)), vec![
                set(1, bin(BinOp::Add, reg(1), int(1))),
                iff(bin(BinOp::Eq, reg(1), int(2)), vec![Stmt::Continue], vec![]),
                set(2, bin(BinOp::Add, reg(2), reg(1))),
            ]),
            ret(reg(2)),
        ], "13"),
        ("do while", vec![
            set(1, int(0)),
            Stmt::DoWhile(vec![set(1, bin(BinOp::Add, reg(1), int(1)))], bin(BinOp::Lt, reg(1), int(3))),
            ret(reg(1)),
        ], "3"),
        ("switch is strict", vec![
            Stmt::Switch(st("1"), vec![(vec![int(1)], vec![ret(st("number"))])], vec![ret(st("none"))]),
        ], "none"),
        ("switch on strings", vec![
            Stmt::Switch(st("y"), vec![(vec![st("x")], vec![ret(int(1))]), (vec![st("y")], vec![ret(int(2))])], vec![ret(int(3))]),
        ], "2"),
        ("try catch", vec![
            Stmt::Try(vec![Stmt::Throw(st("boom"))], vec![Catch { index: 0, var: Some("e".into()), type_name: None,
                body: vec![ret(bin(BinOp::Add, st("caught "), Expr::Caught(0)))] }]),
            ret(st("no")),
        ], "caught boom"),
        ("closures made in a loop share its variable", vec![
            set(1, Expr::Array(vec![])), set(2, int(0)),
            Stmt::While(bin(BinOp::Lt, reg(2), int(3)), vec![
                Stmt::Assign(Place::Name("j".into()), reg(2)),
                Stmt::Expr(method(reg(1), "push", vec![Expr::Function(Box::new(Function {
                    params: vec![], rest: false, return_type: None, local_names: vec![], activation: vec![],
                    body: vec![ret(name("j"))] }))])),
                set(2, bin(BinOp::Add, reg(2), int(1))),
            ]),
            ret(bin(BinOp::Add, bin(BinOp::Add, call(index(reg(1), int(0)), vec![]), st(",")), call(index(reg(1), int(2)), vec![]))),
        ], "2,2"),
        ("ternary", r(Expr::Cond(b(int(0)), b(st("t")), b(st("f")))), "f"),
        ("for in over an array", vec![
            set(1, Expr::Array(vec![st("a"), st("b")])), set(2, int(0)), set(3, st("")),
            Stmt::While(Expr::HasNext2 { object: 1, index: 2 }, vec![
                set(3, bin(BinOp::Add, reg(3), Expr::NextValue(b(reg(1)), b(reg(2))))),
            ]),
            ret(reg(3)),
        ], "ab"),
    ]
}

/// The suite's cases as script text (`AS3_SUITE`); `tools/tests/lang_suite_runner.hx` runs them.
pub fn language_suite(runtime: &Runtime) -> String {
    let mut out = String::from("// the language suite (generated by the compiler; see src/compile/suite.rs)\nvar AS3_SUITE = [];\n");
    for (n, body, want) in cases() {
        // `j` is the one captured variable a case uses: it lives in the function's activation
        let activation = if n.starts_with("closures") { vec![(1, "j".to_string())] } else { vec![] };
        let f = Function { params: vec![], rest: false, return_type: None, local_names: vec![], activation, body };
        let (text, problems) = super::compile_function(&f, runtime);
        assert!(problems.is_empty(), "{n}: {problems:?}");
        out.push_str(&format!("AS3_SUITE.push({{ name: {}, want: {}, run: {text} }});\n", quote(n), quote(want)));
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_case_compiles() {
        let rt = super::Runtime::empty();
        let text = super::language_suite(&rt);
        assert!(text.contains("AS3_SUITE.push"));
    }
}
