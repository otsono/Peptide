//! The decompiler, on bytecode assembled by hand: each case is a shape the compiler emits, and
//! what it must read back as.

use swf::avm2::types::*;
use swf::avm2::write::Writer;

use super::ast;
use super::decode::{Pool, Slots};

/// an instruction, or a label a branch can name
enum A {
    Op(Op),
    Label(&'static str),
    Jump(&'static str),
    IfTrue(&'static str),
    IfFalse(&'static str),
    IfLt(&'static str),
    IfStrictNe(&'static str),
    Switch(&'static str, Vec<&'static str>),
}

fn size(op: &Op) -> usize {
    let mut v = Vec::new();
    Writer::new(&mut v).write_op(op).unwrap();
    v.len()
}

/// assemble: branch offsets resolved from labels (a branch's offset counts from the instruction
/// after it; a switch's from its own start)
fn assemble(code: Vec<A>) -> Vec<u8> {
    let placeholder = |a: &A| -> Option<Op> {
        Some(match a {
            A::Op(o) => o.clone(),
            A::Label(_) => return None,
            A::Jump(_) => Op::Jump { offset: 0 },
            A::IfTrue(_) => Op::IfTrue { offset: 0 },
            A::IfFalse(_) => Op::IfFalse { offset: 0 },
            A::IfLt(_) => Op::IfLt { offset: 0 },
            A::IfStrictNe(_) => Op::IfStrictNe { offset: 0 },
            A::Switch(_, cases) => Op::LookupSwitch(Box::new(LookupSwitch { default_offset: 0, case_offsets: vec![0; cases.len()].into_boxed_slice() })),
        })
    };
    let mut at = 0usize;
    let mut labels = std::collections::BTreeMap::new();
    let mut starts = Vec::new();
    for a in &code {
        starts.push(at);
        if let A::Label(l) = a { labels.insert(*l, at); }
        if let Some(op) = placeholder(a) { at += size(&op); }
    }
    let mut out = Vec::new();
    for (i, a) in code.iter().enumerate() {
        let Some(op) = placeholder(a) else { continue };
        let next = starts[i] + size(&op);
        let rel = |l: &str| labels[l] as i32 - next as i32;
        let op = match a {
            A::Jump(l) => Op::Jump { offset: rel(l) },
            A::IfTrue(l) => Op::IfTrue { offset: rel(l) },
            A::IfFalse(l) => Op::IfFalse { offset: rel(l) },
            A::IfLt(l) => Op::IfLt { offset: rel(l) },
            A::IfStrictNe(l) => Op::IfStrictNe { offset: rel(l) },
            A::Switch(d, cases) => {
                let own = |l: &str| labels[l] as i32 - starts[i] as i32;
                Op::LookupSwitch(Box::new(LookupSwitch { default_offset: own(d), case_offsets: cases.iter().map(|c| own(c)).collect() }))
            }
            _ => op,
        };
        Writer::new(&mut out).write_op(&op).unwrap();
    }
    out
}

const NAMES: &[&str] = &["f", "g", "h", "x", "y", "list", "splice", "length"];

/// a one-method package: `params` parameters, the names above as public multinames 1..
fn decompile(params: usize, code: Vec<A>) -> String {
    let mut strings: Vec<Vec<u8>> = vec![b"".to_vec()];
    strings.extend(NAMES.iter().map(|n| n.as_bytes().to_vec()));
    let pool = ConstantPool {
        ints: vec![], uints: vec![], doubles: vec![],
        strings,
        namespaces: vec![Namespace::Package(Index::new(1))],
        namespace_sets: vec![],
        multinames: NAMES.iter().enumerate().map(|(i, _)| Multiname::QName { namespace: Index::new(1), name: Index::new(i as u32 + 2) }).collect(),
    };
    let method = Method {
        name: Index::new(0),
        params: (0..params).map(|_| MethodParam { name: None, kind: Index::new(0), default_value: None }).collect(),
        return_type: Index::new(0), flags: MethodFlags::empty(), body: Some(Index::new(0)),
    };
    let body = MethodBody {
        method: Index::new(0), max_stack: 16, num_locals: 8, init_scope_depth: 0, max_scope_depth: 1,
        code: assemble(code), exceptions: vec![], traits: vec![],
    };
    let abc = AbcFile {
        major_version: 46, minor_version: 16, constant_pool: pool, methods: vec![method], metadata: vec![],
        instances: vec![], classes: vec![], scripts: vec![], method_bodies: vec![body],
    };
    let pool = Pool { abc: &abc };
    let f = super::function(&pool, 0, &Slots::default());
    let mut out = String::new();
    ast::print_stmts(&mut out, &f.body, 0);
    out
}

fn mn(name: &str) -> Index<Multiname> { Index::new(NAMES.iter().position(|n| *n == name).unwrap() as u32 + 1) }
fn op(o: Op) -> A { A::Op(o) }
fn local(i: u32) -> A { op(Op::GetLocal { index: i }) }
fn byte(v: u8) -> A { op(Op::PushByte { value: v }) }
fn call_void(name: &str, n: u32) -> Vec<A> {
    vec![op(Op::FindPropStrict { index: mn(name) }), op(Op::CallPropVoid { index: mn(name), num_args: n })]
}

#[test]
fn a_ternary_value() {
    // return a ? 1 : 2
    let got = decompile(1, vec![
        local(1), A::IfFalse("else"), byte(1), A::Jump("end"),
        A::Label("else"), byte(2),
        A::Label("end"), op(Op::ReturnValue),
    ]);
    assert_eq!(got, "return r1 ? 1 : 2;\n");
}

#[test]
fn a_short_circuit_value() {
    // return a && b;  return a || b
    let and = decompile(2, vec![
        local(1), op(Op::Dup), A::IfFalse("end"), op(Op::Pop), local(2),
        A::Label("end"), op(Op::ReturnValue),
    ]);
    assert_eq!(and, "return r1 && r2;\n");
    let or = decompile(2, vec![
        local(1), op(Op::Dup), A::IfTrue("end"), op(Op::Pop), local(2),
        A::Label("end"), op(Op::ReturnValue),
    ]);
    assert_eq!(or, "return r1 || r2;\n");
}

#[test]
fn a_nested_short_circuit_value() {
    // return ((a || b) && c) || (d && e): each operator's value flows on, and the whole one is
    // returned (a left side kept twice and the right dropped is the shape to guard against)
    let got = decompile(5, vec![
        local(1), op(Op::Dup), A::IfTrue("ab"), op(Op::Pop), local(2),
        A::Label("ab"), op(Op::Dup), A::IfFalse("abc"), op(Op::Pop), local(3),
        A::Label("abc"), op(Op::Dup), A::IfTrue("end"), op(Op::Pop),
        local(4), op(Op::Dup), A::IfFalse("end"), op(Op::Pop), local(5),
        A::Label("end"), op(Op::ReturnValue),
    ]);
    assert_eq!(got, "return ((r1 || r2) && r3) || (r4 && r5);\n");
}

#[test]
fn a_compound_condition() {
    // if (a && b) f();   if (a || b) g();
    let mut code = vec![local(1), A::IfFalse("skip"), local(2), A::IfFalse("skip")];
    code.extend(call_void("f", 0));
    code.extend([A::Label("skip")]);
    code.extend([local(1), A::IfTrue("do"), local(2), A::IfFalse("done"), A::Label("do")]);
    code.extend(call_void("g", 0));
    code.extend([A::Label("done"), op(Op::ReturnVoid)]);
    let got = decompile(2, code);
    assert_eq!(got, "if (r1 && r2) {\n\tf();\n}\nif (r1 || r2) {\n\tg();\n}\nreturn;\n");
}

#[test]
fn a_value_is_computed_before_a_test_that_calls() {
    // x = x + (a ? 1 : 0): the read of x happens before the test, which could change it
    let got = decompile(1, vec![
        op(Op::GetLocal { index: 0 }), op(Op::GetProperty { index: mn("x") }),
        local(1), A::IfFalse("zero"), byte(1), A::Jump("add"),
        A::Label("zero"), byte(0),
        A::Label("add"), op(Op::Add), A::Op(Op::GetLocal { index: 0 }), op(Op::Swap),
        op(Op::SetProperty { index: mn("x") }), op(Op::ReturnVoid),
    ]);
    assert_eq!(got, "this.x = this.x + (r1 ? 1 : 0);\nreturn;\n");
}

#[test]
fn a_for_loop_whose_continue_runs_the_step() {
    // for (i = 0; i < 10; i++) { if (a) continue; f(); }
    let mut code = vec![byte(0), op(Op::SetLocal { index: 2 }), A::Jump("test"),
                        A::Label("body"), op(Op::Label), local(1), A::IfFalse("work"), A::Jump("step"), A::Label("work")];
    code.extend(call_void("f", 0));
    code.extend([A::Label("step"), op(Op::IncLocal { index: 2 }),
                 A::Label("test"), local(2), byte(10), A::IfLt("body"), op(Op::ReturnVoid)]);
    let got = decompile(1, code);
    // the continue path keeps the step: the loop runs the step on every way round
    assert_eq!(got, "r2 = 0;\nwhile (r2 < 10) {\n\tif (!r1) {\n\t\tf();\n\t}\n\tr2 = r2 + 1;\n}\nreturn;\n");
}

#[test]
fn a_do_while() {
    // do { f(); } while (a)
    let mut code = vec![A::Label("top"), op(Op::Label)];
    code.extend(call_void("f", 0));
    code.extend([local(1), A::IfTrue("top"), op(Op::ReturnVoid)]);
    let got = decompile(1, code);
    assert_eq!(got, "do {\n\tf();\n} while (r1);\nreturn;\n");
}

#[test]
fn a_switch_on_a_value() {
    // switch (a) { case 1: f(); break; case 2: g(); break; default: h(); }
    let mut code = vec![local(1), op(Op::SetLocal { index: 2 }),
        byte(1), local(2), A::IfStrictNe("n1"), byte(0), A::Jump("go"),
        A::Label("n1"), byte(2), local(2), A::IfStrictNe("n2"), byte(1), A::Jump("go"),
        A::Label("n2"), byte(2),
        A::Label("go"), A::Switch("d", vec!["c0", "c1", "d"]),
        A::Label("c0")];
    code.extend(call_void("f", 0));
    code.extend([A::Jump("end"), A::Label("c1")]);
    code.extend(call_void("g", 0));
    code.extend([A::Jump("end"), A::Label("d")]);
    code.extend(call_void("h", 0));
    code.extend([A::Label("end"), op(Op::ReturnVoid)]);
    let got = decompile(1, code);
    assert!(got.contains("switch (r2) {\ncase 1:\n\tf();\ncase 2:\n\tg();\ndefault:\n\th();\n}"), "{got}");
}

#[test]
fn a_post_decrement_keeps_the_old_value() {
    // list.splice(i--, 1)
    let got = decompile(1, vec![
        op(Op::FindPropStrict { index: mn("list") }), op(Op::GetProperty { index: mn("list") }),
        local(1), op(Op::Dup), op(Op::Decrement), op(Op::SetLocal { index: 1 }), byte(1),
        op(Op::CallPropVoid { index: mn("splice"), num_args: 2 }), op(Op::ReturnVoid),
    ]);
    // the argument is r1 as it was before the decrement
    assert!(got.contains("list.splice(") && !got.contains("list.splice(r1 - 1"), "{got}");
    let lines: Vec<&str> = got.lines().collect();
    let dec = lines.iter().position(|l| l.contains("= Number(r1) - 1")).expect("the decrement");
    let call = lines.iter().position(|l| l.contains("splice(")).expect("the call");
    assert!(dec < call, "{got}");
}

#[test]
fn nothing_is_unreadable_in_these() {
    let got = decompile(1, vec![local(1), A::IfFalse("e"), byte(1), A::Jump("x"), A::Label("e"), byte(2), A::Label("x"), op(Op::ReturnValue)]);
    assert!(!got.contains("unreadable") && !got.contains("$j"), "{got}");
}
