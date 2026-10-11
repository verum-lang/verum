//! Prepared T1713 source controls; the preparing agent has not executed them.
//! Every type/protocol/impl is parsed from source. No Heap spelling, seeded
//! field maps, synthetic protocol implementations or no-op optional receiver
//! can establish the positive path. Setup must pass before the probe runs.

use verum_ast::{FileId, ItemKind, Module};
use verum_common::{List, Text};
use verum_fast_parser::FastParser;
use verum_modules::{
    ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module,
};
use verum_types::TypeChecker;

const OWNER: &str = "core.field_protocol";
const DECLARATIONS: &str = r#"
    public type Packet is { public visible: Int, hidden: Int };
"#;

const DEREF_TYPES: &str = r#"
    type Deref is protocol {
        type Target;
        fn deref(&self) -> &Self.Target;
    };
    type DerefMut is protocol extends Deref {
        fn deref_mut(&mut self) -> &mut Self.Target;
    };
    type FieldGuard<Noise, Item> is { noise: Noise, value: Item };
"#;
const DEREF_IMPLS: &str = r#"
    implement<Noise, Item> Deref for FieldGuard<Noise, Item> {
        type Target = Item;
        fn deref(&self) -> &Item { &self.value }
    }
    implement<Noise, Item> DerefMut for FieldGuard<Noise, Item> {
        fn deref_mut(&mut self) -> &mut Item { &mut self.value }
    }
"#;

const MAYBE: &str = "type Maybe<T> is None | Some(T);";

// Fallible is deliberately neither Result nor Maybe. Its Output is supplied
// by the parsed Try implementation, not the well-known-name fallback.
const TRY: &str = r#"
    type ControlFlow<B, C> is Break(B) | Continue(C);
    type Try is protocol {
        type Output;
        type Residual;
        fn from_output(output: Self.Output) -> Self;
        fn branch(self) -> ControlFlow<Self.Residual, Self.Output>;
    };
    type Fallible<T> is Failed(Int) | Passed(T);
    implement<T> Try for Fallible<T> {
        type Output = T;
        type Residual = Int;
        fn from_output(output: T) -> Fallible<T> { Passed(output) }
        fn branch(self) -> ControlFlow<Int, T> {
            match self {
                Failed(error) => Break(error),
                Passed(value) => Continue(value),
            }
        }
    }
"#;

fn parse(source: &str, file: u32) -> Module {
    FastParser::new()
        .parse_module_str(source, FileId::new(file))
        .expect("fixture follows the production grammar")
}

fn collect_diagnostics(checker: &mut TypeChecker, errors: &mut List<Text>) {
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|error| Text::from(format!("{error:?}"))),
    );
    errors.extend(
        checker
            .diagnostic_sources()
            .iter()
            .map(|error| Text::from(format!("{error:?}"))),
    );
}

fn check(helpers: &str, probe: &str) -> List<Text> {
    let source = format!("mount {OWNER}.{{Packet}}; {helpers} {probe}");
    let mut registry = ModuleRegistry::new();
    for (index, (owner, text)) in [(OWNER, DECLARATIONS), ("consumer", source.as_str())]
        .into_iter()
        .enumerate()
    {
        let file = FileId::new(index as u32);
        let id = ModuleId::new(index as u32);
        let ast = parse(text, index as u32);
        let path = ModulePath::from_str(owner);
        let mut info = ModuleInfo::new(id, path.clone(), ast.clone(), file, text.into());
        info.exports = extract_exports_from_module(&ast, id, &path).expect("fixture exports");
        registry.register(info);
    }
    let ast = parse(&source, 1);
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_module_path("consumer");
    checker.set_module_registry_direct(registry.clone());
    checker.register_stdlib_types_for_module(&ast);
    let mut setup_errors = List::new();
    for item in &ast.items {
        if let ItemKind::Mount(import) = &item.kind {
            if let Err(error) = checker.process_import(import, "consumer", &registry) {
                setup_errors.push(Text::from(format!("{error:?}")));
            }
        }
    }
    for item in &ast.items {
        let result = match &item.kind {
            ItemKind::Type(declaration) => checker.register_type_declaration(declaration),
            ItemKind::Impl(declaration) => checker.register_impl_block(declaration),
            ItemKind::Function(declaration) => checker.register_function_signature(declaration),
            _ => continue,
        };
        if let Err(error) = result {
            setup_errors.push(Text::from(format!("{error:?}")));
        }
    }
    for item in &ast.items {
        if matches!(&item.kind, ItemKind::Function(declaration) if declaration.name.name.as_str() == "probe")
        {
            continue;
        }
        if let Err(error) = checker.check_item(item) {
            setup_errors.push(Text::from(format!("{error:?}")));
        }
    }
    collect_diagnostics(&mut checker, &mut setup_errors);
    assert!(
        setup_errors.is_empty(),
        "the declared protocol/impl/owner setup must pass independently: {setup_errors:?}"
    );
    let probe = ast
        .items
        .iter()
        .find(|item| matches!(&item.kind, ItemKind::Function(declaration) if declaration.name.name.as_str() == "probe"))
        .expect("exactly named probe body");
    let mut errors = List::new();
    if let Err(error) = checker.check_item(probe) {
        errors.push(Text::from(format!("{error:?}")));
    }
    collect_diagnostics(&mut checker, &mut errors);
    errors
}

fn assert_public(helpers: &str, probe: &str) {
    let errors = check(helpers, probe);
    assert!(
        errors.is_empty(),
        "declared public target field: {errors:?}"
    );
}

fn assert_private(helpers: &str, probe: &str) {
    let errors = check(helpers, probe);
    let is_target_refusal = |error: &Text| {
        error.contains("VisibilityError")
            && error.contains("name: Text { inner: \"hidden\" }")
            && error.contains("visibility: Text { inner: \"Private\" }")
            && error.contains("module_path: Text { inner: \"core.field_protocol\" }")
    };
    assert!(
        !errors.is_empty() && errors.iter().all(is_target_refusal),
        "refuse the target's private field, not an unknown wrapper/protocol: {errors:?}"
    );
}

#[test]
fn declared_deref_public_target_read_is_allowed() {
    assert_public(
        &format!("{DEREF_TYPES} {DEREF_IMPLS}"),
        "fn probe(value: FieldGuard<Bool, Packet>) -> Int { value.visible }",
    );
}

#[test]
fn declared_deref_private_target_read_is_refused() {
    assert_private(
        &format!("{DEREF_TYPES} {DEREF_IMPLS}"),
        "fn probe(value: FieldGuard<Bool, Packet>) -> Int { value.hidden }",
    );
}

#[test]
fn declared_deref_mut_public_target_write_is_allowed() {
    assert_public(
        &format!("{DEREF_TYPES} {DEREF_IMPLS}"),
        "fn probe(value: &mut FieldGuard<Bool, Packet>) { value.visible = 7; }",
    );
}

#[test]
fn declared_deref_mut_private_target_write_is_refused() {
    assert_private(
        &format!("{DEREF_TYPES} {DEREF_IMPLS}"),
        "fn probe(value: &mut FieldGuard<Bool, Packet>) { value.hidden = 7; }",
    );
}

#[test]
fn two_declared_deref_hops_keep_public_target_field() {
    assert_public(
        &format!("{DEREF_TYPES} {DEREF_IMPLS}"),
        "fn probe(value: FieldGuard<Int, FieldGuard<Bool, Packet>>) -> Int { value.visible }",
    );
}

#[test]
fn two_declared_deref_hops_keep_private_target_owner() {
    assert_private(
        &format!("{DEREF_TYPES} {DEREF_IMPLS}"),
        "fn probe(value: FieldGuard<Int, FieldGuard<Bool, Packet>>) -> Int { value.hidden }",
    );
}

#[test]
fn own_field_wins_before_declared_deref_target() {
    let helpers = format!(
        r#"{DEREF_TYPES} {DEREF_IMPLS}
        type ShadowGuard<Item> is {{ hidden: Bool, value: Item }};
        implement<Item> Deref for ShadowGuard<Item> {{
            type Target = Item;
            fn deref(&self) -> &Item {{ &self.value }}
        }}
        "#
    );
    assert_public(
        &helpers,
        "fn probe(value: ShadowGuard<Packet>) -> Bool { value.hidden }",
    );
}

#[test]
fn wrapper_without_deref_does_not_expose_inner_fields() {
    let errors = check(
        DEREF_TYPES,
        "fn probe(value: FieldGuard<Bool, Packet>) -> Int { value.visible }",
    );
    assert!(
        errors.iter().any(|error| error.contains("visible")),
        "the positive path must require the parsed Deref implementation: {errors:?}"
    );
}

#[test]
fn declared_maybe_public_inner_field_is_allowed() {
    assert_public(
        MAYBE,
        "fn probe(value: Maybe<Packet>) -> Maybe<Int> { value?.visible }",
    );
}

#[test]
fn declared_maybe_private_inner_field_is_refused() {
    assert_private(
        MAYBE,
        "fn probe(value: Maybe<Packet>) -> Maybe<Int> { value?.hidden }",
    );
}

#[test]
fn declared_maybe_public_reference_inner_field_is_allowed() {
    assert_public(
        MAYBE,
        "fn probe(value: Maybe<&Packet>) -> Maybe<Int> { value?.visible }",
    );
}

#[test]
fn declared_maybe_private_reference_inner_field_is_refused() {
    assert_private(
        MAYBE,
        "fn probe(value: Maybe<&Packet>) -> Maybe<Int> { value?.hidden }",
    );
}

#[test]
fn declared_try_public_output_field_is_allowed() {
    assert_public(
        TRY,
        "fn probe(value: Fallible<Packet>) -> Int { try { value?.visible } recover { _ => 0 } }",
    );
}

#[test]
fn declared_try_private_output_field_is_refused() {
    assert_private(
        TRY,
        "fn probe(value: Fallible<Packet>) -> Int { try { value?.hidden } recover { _ => 0 } }",
    );
}

#[test]
fn declared_try_public_reference_output_field_is_allowed() {
    assert_public(
        TRY,
        "fn probe(value: Fallible<&Packet>) -> Int { try { value?.visible } recover { _ => 0 } }",
    );
}

#[test]
fn declared_try_private_reference_output_field_is_refused() {
    assert_private(
        TRY,
        "fn probe(value: Fallible<&Packet>) -> Int { try { value?.hidden } recover { _ => 0 } }",
    );
}
