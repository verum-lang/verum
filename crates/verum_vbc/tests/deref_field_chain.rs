//! T1545: inference of a field chain must follow the same declared Deref
//! target as field emission. Dynamic by-name lookup must not hide lost facts.
#![cfg(feature = "codegen")]

use verum_common::List;
use verum_fast_parser::Parser;
use verum_vbc::bytecode::decode_instruction;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::instruction::Instruction;
use verum_vbc::module::VbcModule;

const RECORDS: &str = r#"
type Config is { first: Int, second: Int, label: Int };
type Inner is { head: Int, config: Config };
// A same-named foreign field makes type loss observable, not lucky slot agreement.
type Foreign is { label: Int, a: Int, b: Int, c: Int };
"#;
const GUARD: &str = r#"
type Deref is protocol { type Target; fn deref(&self) -> &Self.Target; };
type Guard<Noise, Item> is { noise: Noise, value: Item };
implement<Noise, Item> Deref for Guard<Noise, Item> {
    type Target = Item;
    fn deref(&self) -> &Item { &self.value }
}
"#;
fn compile(source: &str) -> VbcModule {
    let ast = Parser::new(source).parse_module().expect("source parses");
    VbcCodegen::with_config(CodegenConfig::new("field_chain"))
        .compile_module(&ast)
        .expect("source compiles")
}
fn body(module: &VbcModule) -> List<Instruction> {
    let f = module
        .functions
        .iter()
        .find(|f| {
            module
                .get_string(f.name)
                .is_some_and(|n| n == "probe" || n.ends_with(".probe"))
        })
        .expect("probe body");
    let mut pc = f.bytecode_offset as usize;
    let end = pc + f.bytecode_length as usize;
    let mut result = List::new();
    while pc < end {
        result.push(decode_instruction(&module.bytecode, &mut pc).expect("decode"));
    }
    result
}
fn exact_projection(module: &VbcModule, last: u32) {
    let operations = body(module);
    assert!(
        !operations
            .iter()
            .any(|op| matches!(op, Instruction::GetFieldNamed { .. })),
        "lost owner forced dynamic field lookup: {operations:?}"
    );
    let slots: List<_> = operations
        .iter()
        .filter_map(|op| match op {
            Instruction::GetF { field_idx, .. } => Some(*field_idx),
            _ => None,
        })
        .collect();
    assert_eq!(
        slots.last().copied(),
        Some(last),
        "wrong final field: {operations:?}"
    );
}
#[test]
fn ordinary_nested_fields_keep_nonzero_declared_slot() {
    exact_projection(
        &compile(&format!(
            "{RECORDS} fn probe(x: &Inner) -> Int {{ x.config.label }}"
        )),
        2,
    );
}
#[test]
fn transparent_carrier_nested_fields_keep_nonzero_declared_slot() {
    exact_projection(
        &compile(&format!(
            "{RECORDS} type Holder is {{ padding: Int, inner: Shared<Inner> }}; fn probe(x: &Holder) -> Int {{ x.inner.config.label }}"
        )),
        2,
    );
}
#[test]
fn user_deref_nested_fields_use_declared_target_parameter() {
    exact_projection(
        &compile(&format!(
            "{RECORDS} {GUARD} fn probe(x: Guard<Int, Inner>) -> Int {{ x.config.label }}"
        )),
        2,
    );
}
#[test]
fn declared_generic_field_is_instantiated_before_next_projection() {
    exact_projection(
        &compile(&format!(
            "{RECORDS} type Holder<T> is {{ padding: Int, inner: T }}; fn probe(x: Holder<Inner>) -> Int {{ x.inner.config.label }}"
        )),
        2,
    );
}

#[test]
fn wrapper_own_field_wins_over_deref_target() {
    let module = compile(&format!(
        r#"{RECORDS}
        type Deref is protocol {{ type Target; fn deref(&self) -> &Self.Target; }};
        type OwnConfig is {{ label: Int, other: Int }};
        type Guard<T> is {{ config: OwnConfig, value: T }};
        implement<T> Deref for Guard<T> {{ type Target = T; fn deref(&self) -> &T {{ &self.value }} }}
        fn probe(x: Guard<Inner>) -> Int {{ x.config.label }}
    "#
    ));
    exact_projection(&module, 0);
    assert!(
        !body(&module)
            .iter()
            .any(|op| matches!(op, Instruction::CallM { .. })),
        "own fields must not invoke Deref"
    );
}

fn archive_consumers(source: &str) -> List<VbcModule> {
    let mut producers = List::new();
    for (owner, fields, inner) in [
        (
            "alpha",
            "first: Int, second: Int, label: Int",
            "head: Int, config: alpha.Config",
        ),
        (
            "beta",
            "first: Int, label: Int",
            "config: beta.Config, head: Int",
        ),
    ] {
        let source =
            format!("module {owner}; type Config is {{ {fields} }}; type Inner is {{ {inner} }};");
        let ast = Parser::new(&source)
            .parse_module()
            .expect("producer parses");
        producers.push(
            VbcCodegen::with_config(CodegenConfig::new(owner))
                .compile_module(&ast)
                .expect("producer"),
        );
    }
    consume_archive_types(&producers, source)
}

fn consume_archive_types(producers: &[VbcModule], source: &str) -> List<VbcModule> {
    let mut result = List::new();
    for order in [[0, 1], [1, 0]] {
        let mut cg = VbcCodegen::with_config(CodegenConfig::new("consumer"));
        // Distinct pools must be read through archive descriptor authority.
        for i in 0..400 {
            cg.ctx_mut().intern_string_raw(&format!("foreign_{i}"));
        }
        for i in order {
            cg.import_archive_module_types(&producers[i]);
        }
        let ast = Parser::new(source).parse_module().expect("consumer parses");
        cg.collect_unit_declarations(&[&ast])
            .expect("consumer declarations");
        result.push(cg.compile_function_bodies(&ast).expect("consumer"));
    }
    result
}

#[test]
fn archive_field_chains_keep_sibling_owners_in_both_orders() {
    for module in archive_consumers(
        "fn probe(a: Shared<alpha.Inner>, b: Shared<beta.Inner>) -> Int { a.config.label + b.config.label }",
    ) {
        let operations = body(&module);
        assert!(
            !operations
                .iter()
                .any(|op| matches!(op, Instruction::GetFieldNamed { .. })),
            "{operations:?}"
        );
        let slots: List<_> = operations
            .iter()
            .filter_map(|op| match op {
                Instruction::GetF { field_idx, .. } => Some(*field_idx),
                _ => None,
            })
            .collect();
        assert_eq!(slots.as_slice(), &[1, 2, 0, 1], "{operations:?}");
    }
}

#[test]
fn missing_qualified_target_does_not_borrow_ancestor_fields() {
    for module in
        archive_consumers("fn probe(x: Shared<alpha.child.Inner>) -> Int { x.config.label }")
    {
        let operations = body(&module);
        assert!(
            operations
                .iter()
                .any(|op| matches!(op, Instruction::GetFieldNamed { .. })),
            "missing exact owner must remain unresolved: {operations:?}"
        );
    }
}

#[test]
fn declared_deref_field_chain_executes_without_dynamic_field_lookup() {
    let module = compile(&format!(
        r#"{RECORDS} {GUARD}
        fn probe() -> Int {{
            let x: Guard<Int, Inner> = Guard {{ noise: 91, value: Inner {{
                head: 83, config: Config {{ first: 71, second: 61, label: 17 }}
            }} }};
            x.config.label
        }}
    "#
    ));
    exact_projection(&module, 2);
    let entry = module
        .functions
        .iter()
        .find(|f| {
            module
                .get_string(f.name)
                .is_some_and(|n| n == "probe" || n.ends_with(".probe"))
        })
        .expect("probe")
        .id;
    let value = verum_vbc::interpreter::Interpreter::new(std::sync::Arc::new(module))
        .execute_function(entry)
        .expect("execute");
    assert!(value.is_int(), "expected Int, got {value:?}");
    assert_eq!(value.as_i64(), 17);
}

#[test]
fn qualified_field_segment_is_not_an_owner_parameter() {
    exact_projection(
        &compile(
            r#"
        module alpha;
        type Item is { padding: Int, label: Int };
        type Config is { label: Int, padding: Int };
        type Holder<Item> is { value: alpha.Item, generic: Item };
        fn probe(x: Holder<Config>) -> Int { x.value.label }
    "#,
        ),
        1,
    );
}

#[test]
fn generic_field_preserves_unicode_nominal_identity() {
    exact_projection(
        &compile(
            r#"
        type Данные is { padding: Int, label: Int };
        type Other is { label: Int };
        type Holder<T> is { value: Данные, generic: T };
        fn probe(x: Holder<Int>) -> Int { x.value.label }
    "#,
        ),
        1,
    );
}

#[test]
fn tuple_owner_argument_remains_one_type() {
    exact_projection(
        &compile(
            r#"
        type Config is { padding: Int, label: Int };
        type Other is { label: Int };
        type Holder<T, U> is { value: U, other: T };
        fn probe(x: Holder<(Int, Text), Config>) -> Int { x.value.label }
    "#,
        ),
        1,
    );
}

#[test]
fn function_owner_argument_remains_one_type() {
    exact_projection(
        &compile(
            r#"
        type Config is { padding: Int, label: Int };
        type Other is { label: Int };
        type Holder<T, U> is { value: U, other: T };
        fn probe(x: Holder<fn(Int, Text) -> Int, Config>) -> Int { x.value.label }
    "#,
        ),
        1,
    );
}

fn inferred_field(source: &str, expression: &str) -> Option<String> {
    let ast = Parser::new(source).parse_module().expect("source parses");
    let mut cg = VbcCodegen::with_config(CodegenConfig::new("scoped_field"));
    cg.collect_unit_declarations(&[&ast]).expect("declarations");
    cg.compile_function_bodies(&ast).expect("bodies");
    // The parameter's type was populated by source compilation, not injected.
    let expr = Parser::new(expression)
        .parse_expr()
        .expect("field expression");
    let inferred = cg.infer_expr_type_name(&expr);
    assert_eq!(inferred, cg.extract_expr_type_name(&expr));
    inferred
}

#[test]
fn rank_two_field_preserves_local_binder_and_substitutes_outer_parameter() {
    assert_eq!(
        inferred_field(
            r#"
        type Holder<T, U> is { callback: fn<T>(T, U) -> T };
        fn probe(x: Holder<Int, Text>) { let carried = x.callback; }
    "#,
            "x.callback"
        ),
        Some("fn<T>(T, Text) -> T".into())
    );
}

#[test]
fn rank_two_field_does_not_capture_actual_argument() {
    assert_eq!(
        inferred_field(
            r#"
        type Holder<A> is { callback: fn<B>(A) -> B };
        fn probe<B>(x: Holder<B>) { let carried = x.callback; }
    "#,
            "x.callback"
        ),
        None
    );
}

#[test]
fn incomplete_field_owner_arguments_remain_unknown() {
    assert_eq!(
        inferred_field(
            r#"
        type Holder<T, U> is { value: U };
        fn probe(x: Holder<Int>) { let carried = x.value; }
    "#,
            "x.value"
        ),
        None
    );
}

#[test]
fn nested_qualified_field_arguments_are_still_substituted() {
    exact_projection(
        &compile(
            r#"
        module alpha;
        type Config is { padding: Int, label: Int };
        type Other is { label: Int };
        type Wrap<T> is { value: T };
        type Holder<Item> is { value: alpha.Wrap<Item> };
        fn probe(x: Holder<Config>) -> Int { x.value.value.label }
    "#,
        ),
        1,
    );
}

#[test]
fn field_owner_const_argument_keeps_its_declared_position() {
    exact_projection(
        &compile(
            r#"
        type Config is { padding: Int, label: Int };
        type Other is { label: Int };
        type Buffer<T, const N: Int> is { head: T };
        type Holder<T, const N: Int> is { value: Buffer<T, N> };
        fn probe(x: Holder<Config, 3>) -> Int { x.value.head.label }
    "#,
        ),
        1,
    );
}

#[test]
fn archived_generic_fields_preserve_nominal_segments_in_both_orders() {
    let mut producers = List::new();
    for (module, fields) in [
        ("alpha", "padding: Int, label: Int"),
        ("beta", "first: Int, second: Int, label: Int"),
    ] {
        let source = format!(
            r#"
            module {module};
            type Item is {{ {fields} }};
            type Config is {{ label: Int }};
            type Holder<Item> is {{ value: {module}.Item, generic: Item }};
        "#
        );
        let ast = Parser::new(&source)
            .parse_module()
            .expect("producer parses");
        producers.push(
            VbcCodegen::with_config(CodegenConfig::new(module))
                .compile_module(&ast)
                .expect("producer compiles"),
        );
    }
    for module in consume_archive_types(
        &producers,
        r#"
        fn probe(a: alpha.Holder<alpha.Config>, b: beta.Holder<beta.Config>) -> Int {
            a.value.label + b.value.label
        }
    "#,
    ) {
        let operations = body(&module);
        assert!(
            !operations
                .iter()
                .any(|op| matches!(op, Instruction::GetFieldNamed { .. })),
            "{operations:?}"
        );
        let slots: Vec<_> = operations
            .iter()
            .filter_map(|op| match op {
                Instruction::GetF { field_idx, .. } => Some(*field_idx),
                _ => None,
            })
            .collect();
        assert_eq!(slots, [0, 1, 0, 2], "{operations:?}");
    }
}
