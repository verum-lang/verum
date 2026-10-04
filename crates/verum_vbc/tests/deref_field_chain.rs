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
