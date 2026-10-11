//! Text replacement must reach the borrowed Value slot, not its temporary register.
#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_ast::{
    ItemKind,
    decl::{ImplItemKind, ImplKind},
    ty::TypeKind,
};
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::interpreter::Interpreter;

const HELPERS: &str = r#"
type Fields is { padding: Int, word: Text, untouched: Text };
fn append(source: &mut Text) { source.push_str(" suffix"); }
fn forward(source: &mut Text) { append(source); }
fn append_lambda(source: &mut Text) { source.push_byte(206); source.push_byte(187); }
"#;

fn check(body: &str, expected: &str) {
    let source = format!("module text_field_mutation; {HELPERS}\nfn probe() -> Text {{ {body} }}");
    let consumer = Parser::new(&source)
        .parse_module()
        .expect("fixture grammar");
    // Receiver ownership comes from the real declaration, not the method name.
    // Register the unchanged canonical record and actual method signatures.
    // Compile only consumer bodies: these controls exercise runtime intercepts,
    // not the complete stdlib method-body dependency closure.
    let mut ast = Parser::new(include_str!("../../../core/text/text.vr"))
        .parse_module()
        .expect("actual Text grammar");
    let methods = [
        "push",
        "push_byte",
        "push_str",
        "truncate",
        "clear",
        "pop",
        "reserve",
        "capacity",
    ];
    ast.items.retain_mut(|item| match &mut item.kind {
        ItemKind::Module(_) => true,
        ItemKind::Type(declaration) => declaration.name.name.as_str() == "Text",
        ItemKind::Impl(declaration) => {
            if !matches!(&declaration.kind, ImplKind::Inherent(ty)
                if matches!(&ty.kind, TypeKind::Text))
            {
                return false;
            }
            declaration.items.retain(|item| {
                matches!(&item.kind, ImplItemKind::Function(function)
                if methods.contains(&function.name.name.as_str()))
            });
            assert_eq!(
                declaration.items.len(),
                methods.len(),
                "complete method selection"
            );
            true
        }
        _ => false,
    });
    assert_eq!(
        ast.items.len(),
        3,
        "canonical module, record and inherent impl"
    );
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("text_field_mutation"));
    codegen.register_builtin_variants();
    codegen
        .collect_unit_declarations(&[&ast, &consumer])
        .expect("actual declaration registration");
    let functions = codegen.export_functions();
    for name in methods {
        let info = functions
            .get(&format!("Text.{name}"))
            .expect("exact registered source signature");
        assert_eq!(
            info.takes_self_mut_ref,
            name != "capacity",
            "Text.{name} receiver ownership"
        );
    }
    codegen
        .compile_module_items(&consumer)
        .expect("strict consumer bodies");
    let module = codegen.finalize_module().expect("consumer module");
    assert!(
        module
            .types
            .iter()
            .any(|ty| module.get_string(ty.name) == Some("Text")
                && ty.origin_module.and_then(|id| module.get_string(id)) == Some("core.text.text")
                && ty.kind == verum_vbc::types::TypeKind::Record),
        "exact source-owned Text record"
    );
    let wire = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).expect("serialize"),
    )
    .expect("decode wire");
    let mut failures: List<Text> = List::new();
    for (route, module) in [("source", module), ("wire", wire)] {
        let entry = module
            .find_function_by_name("text_field_mutation.probe")
            .expect("probe");
        let mut interpreter = Interpreter::new(Arc::new(module));
        match interpreter.execute_function(entry) {
            Ok(value) => {
                let actual = interpreter.read_text(value);
                if actual.as_deref() != Some(expected) {
                    failures.push(format!("{route}: expected {expected:?}, got {actual:?}").into());
                }
            }
            Err(error) => failures.push(format!("{route}: {error}").into()),
        }
    }
    assert!(failures.is_empty(), "{failures:?}");
}

#[test]
fn direct_local_text_append() {
    check(
        r#"let mut word = "seed"; word.push_str(" suffix"); word"#,
        "seed suffix",
    );
}

#[test]
fn forwarded_local_text_append() {
    check(
        r#"let mut word = "seed"; forward(&mut word); word"#,
        "seed suffix",
    );
}

#[test]
fn direct_small_text_field_append() {
    check(
        r#"
        let mut fields = Fields { padding: 17, word: "seed", untouched: "sentinel" };
        fields.word.push_str(" suffix");
        assert(fields.padding == 17); assert(fields.untouched == "sentinel");
        fields.word
    "#,
        "seed suffix",
    );
}

#[test]
fn direct_heap_text_field_append() {
    check(
        r#"
        let mut fields = Fields { padding: 17, word: "registry source", untouched: "sentinel" };
        fields.word.push_str(" λ");
        assert(fields.padding == 17); assert(fields.untouched == "sentinel");
        fields.word
    "#,
        "registry source λ",
    );
}

#[test]
fn mutable_field_argument_updates_owner() {
    check(
        r#"
        let mut fields = Fields { padding: 17, word: "seed", untouched: "sentinel" };
        append(&mut fields.word); fields.word
    "#,
        "seed suffix",
    );
}

#[test]
fn forwarded_mutable_field_argument_updates_owner() {
    check(
        r#"
        let mut fields = Fields { padding: 17, word: "seed", untouched: "sentinel" };
        forward(&mut fields.word); fields.word
    "#,
        "seed suffix",
    );
}

#[test]
fn local_bytewise_unicode_append() {
    check(
        r#"let mut word = "seed"; append_lambda(&mut word); word"#,
        "seedλ",
    );
}

#[test]
fn field_bytewise_unicode_append() {
    check(
        r#"
        let mut fields = Fields { padding: 17, word: "seed", untouched: "sentinel" };
        append_lambda(&mut fields.word); fields.word
    "#,
        "seedλ",
    );
}

#[test]
fn field_truncate_updates_owner() {
    check(
        r#"
        let mut fields = Fields { padding: 17, word: "seedλ", untouched: "sentinel" };
        fields.word.truncate(4); fields.word
    "#,
        "seed",
    );
}

#[test]
fn field_pop_updates_owner_and_returns_character() {
    check(
        r#"
        let mut fields = Fields { padding: 17, word: "seedλ", untouched: "sentinel" };
        match fields.word.pop() { Maybe.Some(ch) => assert(ch == 'λ'), Maybe.None => assert(false), }
        fields.word
    "#,
        "seed",
    );
}

#[test]
fn field_clear_updates_owner() {
    check(
        r#"
        let mut fields = Fields { padding: 17, word: "registry source", untouched: "sentinel" };
        fields.word.clear(); fields.word
    "#,
        "",
    );
}

#[test]
fn reused_mutable_field_alias_keeps_origin() {
    check(
        r#"
        let mut fields = Fields { padding: 17, word: "seed", untouched: "sentinel" };
        let source = &mut fields.word;
        forward(source); forward(source);
        assert(fields.padding == 17); assert(fields.untouched == "sentinel");
        fields.word
    "#,
        "seed suffix suffix",
    );
}

#[test]
fn field_reserve_retains_bytes_and_exposes_capacity() {
    check(
        r#"
        let mut fields = Fields { padding: 17, word: "seedλ", untouched: "sentinel" };
        fields.word.reserve(32);
        assert(fields.word.capacity() >= 38);
        assert(fields.padding == 17); assert(fields.untouched == "sentinel");
        fields.word
    "#,
        "seedλ",
    );
}

#[test]
fn field_reserve_then_append_updates_same_origin() {
    check(
        r#"
        let mut fields = Fields { padding: 17, word: "seed", untouched: "sentinel" };
        fields.word.reserve(32);
        append_lambda(&mut fields.word);
        fields.word
    "#,
        "seedλ",
    );
}
