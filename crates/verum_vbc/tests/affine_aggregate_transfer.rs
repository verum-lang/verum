//! T1805: an owning aggregate receives the source obligation before cleanup.
#![cfg(feature = "codegen")]
use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{
    VbcModule,
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    deserialize::deserialize_module,
    interpreter::Interpreter,
    serialize::serialize_module,
};

const DECLARATIONS: &str = r#"
type Count is { value: Int };
type affine Handle is { counter: &mut Count, digit: Int };
implement Drop for Handle {
    fn drop(&mut self) { self.counter.value = self.counter.value * 10 + self.digit; }
}
type Holder is { handle: Handle };
type Stream is Plain(Handle) | Empty;
implement Drop for Stream {
    fn drop(&mut self) {
        match &mut self { Stream.Plain(value) => value.drop(), Stream.Empty => {} }
    }
}
type Named is Held { handle: Handle } | Empty;
implement Drop for Named {
    fn drop(&mut self) {
        match &mut self { Named.Held { handle } => handle.drop(), Named.Empty => {} }
    }
}
"#;

fn compile(body: &str, bootstrap: bool) -> VbcModule {
    let ast = Parser::new(&format!("{DECLARATIONS}\n{body}"))
        .parse_module()
        .expect("source grammar");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("aggregate_transfer"));
    if bootstrap {
        codegen.collect_unit_declarations(&[&ast]).unwrap();
        codegen
            .compile_unit_items(&[&ast], ItemFailurePolicy::Strict)
            .unwrap();
        codegen.finalize_module_from_state().unwrap()
    } else {
        codegen.compile_module(&ast).unwrap()
    }
}
fn run(body: &str, bootstrap: bool) -> i64 {
    let mut module = compile(body, bootstrap);
    // Ownership must survive transport and removal of optional observations.
    for function in &mut module.functions {
        function.value_uses = None;
    }
    let module = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    let probe = module
        .find_function_by_name("aggregate_transfer.probe")
        .unwrap();
    Interpreter::new(Arc::new(module))
        .execute_function(probe)
        .unwrap()
        .as_i64()
}

#[test]
fn returned_aggregate_keeps_socket_like_resource_alive_until_owner_cleanup() {
    for (ty, constructor) in [
        ("Holder", "Holder { handle }"),
        ("Holder", "Holder { handle: (handle) }"),
        ("Holder", "Holder(handle)"),
        ("Stream", "Stream.Plain(handle)"),
        ("Stream", "Plain(handle)"),
        ("Named", "Named.Held { handle }"),
        ("Named", "Named.Held { handle: (handle) }"),
    ] {
        for annotation in ["", ": Handle"] {
            let source = format!(
                r#"
fn open(counter: &mut Count) -> {ty} {{
    let handle{annotation} = Handle {{ counter, digit: 7 }};
    let owner: {ty} = {constructor};
    owner
}}
fn probe() -> Int {{
    let mut counter = Count {{ value: 0 }};
    {{ let owner: {ty} = open(&mut counter); if counter.value != 0 {{ return 99; }} }}
    counter.value
}}
"#
            );
            for bootstrap in [false, true] {
                assert_eq!(
                    run(&source, bootstrap),
                    7,
                    "{ty}, {constructor}, {annotation}, bootstrap={bootstrap}"
                );
            }
        }
    }
}

#[test]
fn aggregate_transfer_clears_only_the_executed_branch() {
    for flag in ["true", "false"] {
        let source = format!(
            r#"
fn probe() -> Int {{
    let mut counter = Count {{ value: 0 }};
    {{
        let handle: Handle = Handle {{ counter: &mut counter, digit: 3 }};
        if {flag} {{ let owner: Holder = Holder {{ handle }}; }}
    }}
    counter.value
}}
"#
        );
        for bootstrap in [false, true] {
            assert_eq!(run(&source, bootstrap), 3);
        }
    }
}

#[test]
fn explicit_return_can_transfer_into_a_fresh_aggregate() {
    for (ty, constructor) in [
        ("Holder", "Holder { handle }"),
        ("Stream", "Stream.Plain(handle)"),
    ] {
        let source = format!(
            r#"
fn open(counter: &mut Count) -> {ty} {{
    let before: Handle = Handle {{ counter, digit: 1 }};
    let handle: Handle = Handle {{ counter, digit: 5 }};
    let after: Handle = Handle {{ counter, digit: 2 }};
    return {constructor};
}}
fn probe() -> Int {{
    let mut counter = Count {{ value: 0 }};
    {{ let owner: {ty} = open(&mut counter); if counter.value != 21 {{ return 99; }} }}
    counter.value
}}
"#
        );
        for bootstrap in [false, true] {
            assert_eq!(run(&source, bootstrap), 215);
        }
    }
}

#[test]
fn borrowed_fields_never_acquire_the_referents_cleanup() {
    let source = r#"
type Borrower is { handle: &Handle };
fn probe() -> Int {
    let mut counter = Count { value: 0 };
    {
        let handle: Handle = Handle { counter: &mut counter, digit: 6 };
        { let borrower = Borrower { handle: &handle }; }
        if counter.value != 0 { return 99; }
    }
    counter.value
}
"#;
    for bootstrap in [false, true] {
        assert_eq!(run(source, bootstrap), 6);
    }
}

#[test]
fn each_variant_argument_transfers_once() {
    let source = r#"
type Pair is Both(Handle, Handle);
implement Drop for Pair {
    fn drop(&mut self) {
        match &mut self { Pair.Both(left, right) => { left.drop(); right.drop(); } }
    }
}
fn open(counter: &mut Count) -> Pair {
    let first = Handle { counter, digit: 1 };
    let second = Handle { counter, digit: 2 };
    Pair.Both(first, second)
}
fn probe() -> Int {
    let mut counter = Count { value: 0 };
    { let pair: Pair = open(&mut counter); if counter.value != 0 { return 99; } }
    counter.value
}
"#;
    for bootstrap in [false, true] {
        assert_eq!(run(source, bootstrap), 12);
    }
}

#[test]
fn ordinary_record_fields_still_copy_the_source_value() {
    let source = r#"
type Ordinary is { counter: &mut Count };
implement Drop for Ordinary { fn drop(&mut self) { self.counter.value += 1; } }
type PlainOwner is { handle: Ordinary };
fn probe() -> Int {
    let mut counter = Count { value: 0 };
    {
        let handle = Ordinary { counter: &mut counter };
        { let owner = PlainOwner { handle }; }
        if counter.value != 1 { return 99; }
    }
    counter.value
}
"#;
    for bootstrap in [false, true] {
        assert_eq!(run(source, bootstrap), 2);
    }
}
