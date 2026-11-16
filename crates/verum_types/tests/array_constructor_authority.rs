//! T1726: array construction requires a supported declaration-owned operation.

use std::sync::Arc;
use verum_ast::{FileId, ItemKind, Module};
use verum_common::{List, ResourceDiscipline, Text};
use verum_fast_parser::VerumParser;
use verum_modules::{ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module};
use verum_types::core_metadata::{CoreMetadata, GenericParam, TypeDescriptor, TypeDescriptorKind};
use verum_types::TypeChecker;

const LIST_OWNER: &str = "core.collections.list";
const LIST_SOURCE: &str = include_str!("../../../core/collections/list.vr");

#[derive(Clone, Copy, Debug)]
enum Input { Source, LazyMetadata, EagerMetadata }

fn parse(source: &str, file: FileId) -> Module {
    VerumParser::new().parse_module_str(source, file).expect("source grammar")
}

fn list_interface() -> Module {
    let mut module = parse(LIST_SOURCE, FileId::new(0));
    // Isolate the actual declaration, without replacing its fields or telescope
    // with a test-designed collection. This gate checks construction admission;
    // full List method-body admission remains the compiler's stdlib gate.
    module.items.retain(|item| matches!(&item.kind,
        ItemKind::Type(declaration) if declaration.name.name == "List"));
    assert_eq!(module.items.len(), 1);
    module
}

fn metadata(module: &Module) -> CoreMetadata {
    let ItemKind::Type(declaration) = &module.items[0].kind else { panic!("List declaration") };
    let mut metadata = CoreMetadata::default();
    let key: Text = format!("{LIST_OWNER}.List").into();
    let descriptor = TypeDescriptor {
        source_declaration: verum_ast::source_signature::source_type_declaration(declaration)
            .expect("retained List source declaration"),
        name: "List".into(), module_path: LIST_OWNER.into(), origin_module_path: None,
        generic_params: List::from_iter([GenericParam {
            name: "T".into(), bounds: List::new(), default: None,
            type_bounds: List::new(), pid: Some(0),
        }]),
        // The retained source declaration owns the field policy and types.
        kind: TypeDescriptorKind::Record { fields: List::new() },
        size: None, alignment: None, methods: List::new(), implements: List::new(),
        decl_span: None, is_public: true, is_transparent_wrapper: false,
        resource_discipline: ResourceDiscipline::Unknown,
    };
    metadata.types.insert(key.clone(), descriptor);
    metadata.type_declaration_order.push(key);
    serde_json::from_slice(&serde_json::to_vec(&metadata).expect("encode source metadata"))
        .expect("decode source metadata")
}

fn errors(source: &str, input: Input) -> List<Text> {
    let interface = list_interface();
    let eager_metadata = matches!(input, Input::EagerMetadata)
        .then(|| Arc::new(metadata(&interface)));
    let mut checker = match input {
        Input::Source | Input::EagerMetadata => TypeChecker::with_minimal_context(),
        Input::LazyMetadata => TypeChecker::new_with_core(Arc::new(metadata(&interface))),
    };
    checker.register_primitives();
    let mut registry = ModuleRegistry::new();
    for (owner, body, ast) in [
        (LIST_OWNER, LIST_SOURCE, match input {
            Input::Source => interface,
            _ => parse("", FileId::new(0)),
        }),
        ("consumer", source, parse(source, FileId::new(1))),
    ] {
        let id = registry.allocate_id();
        let path = ModulePath::from_str(owner);
        let exports = extract_exports_from_module(&ast, id, &path).expect("source exports");
        let mut info = ModuleInfo::new(id, path, ast.clone(), ast.file_id, body.into());
        info.exports = exports;
        registry.register(info);
    }
    let module = registry.get_by_path("consumer").expect("consumer source");
    checker.set_module_registry_direct(registry.clone());
    // Source admission captures observed registry dependencies. Configure the
    // registry before eager admission, just as the compiler does before lazy
    // admission; adding the owner afterward invalidates the captured absence.
    if let Some(metadata) = eager_metadata {
        checker.load_core_eager(metadata);
    }
    checker.set_current_module_path("consumer");
    let mut errors: List<Text> = checker.prepare_module_declarations(&module.ast)
        .into_iter().map(|error| format!("{error:?}").into()).collect();
    for item in &module.ast.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(format!("{error:?}").into());
        }
    }
    errors.extend(checker.take_deferred_errors().into_iter()
        .map(|error| Text::from(format!("{error:?}"))));
    errors.extend(checker.diagnostic_sources().iter()
        .map(|error| Text::from(format!("{error:?}"))));
    errors
}

fn allowed(source: &str) {
    for input in [Input::Source, Input::LazyMetadata, Input::EagerMetadata] {
        let errors = errors(source, input);
        assert!(errors.is_empty(), "{input:?}: {source}\n{errors:?}");
    }
}

fn mismatch(source: &str) {
    for input in [Input::Source, Input::LazyMetadata, Input::EagerMetadata] {
        let errors = errors(source, input);
        assert!(errors.iter().any(|error| error.contains("Mismatch")),
            "{input:?}: expected a construction/element mismatch: {source}\n{errors:?}");
    }
}

#[test]
fn actual_list_declaration_accepts_literals_through_source_and_metadata() {
    for value in ["[]", "[1, 2]", "[1; 2]"] {
        allowed(&format!("mount {LIST_OWNER}.List; fn probe() -> List<Int> {{ {value} }}"));
    }
}

#[test]
fn renamed_list_and_transparent_alias_keep_constructor_authority() {
    allowed(&format!("mount {LIST_OWNER}.List as Buffer; type Values<T> is Buffer<T>; fn probe() -> Values<Int> {{ [1, 2] }}"));
}

#[test]
fn a_nominal_type_parameter_does_not_supply_an_array_constructor() {
    for name in ["Storage", "Set", "Shared", "List"] {
        for value in ["[]", "[1, 2]", "[1; 2]"] {
            mismatch(&format!("public type {name}<T> is {{ public item: T }}; fn probe() -> {name}<Int> {{ {value} }}"));
        }
    }
}

#[test]
fn a_forged_marker_protocol_does_not_supply_an_array_constructor() {
    mismatch("public type ArrayCoercible is protocol {}; public type Storage<T> is { public item: T }; implement<T> ArrayCoercible for Storage<T> {} fn probe() -> Storage<Int> { [1, 2] }");
}

#[test]
fn imported_list_does_not_authorize_a_same_named_local_record() {
    mismatch(&format!("mount {LIST_OWNER}.List as Actual; public type List<T> is {{ public item: T }}; fn probe() -> List<Int> {{ [1, 2] }}"));
}

#[test]
fn list_construction_still_checks_each_element() {
    mismatch(&format!("mount {LIST_OWNER}.List; fn probe() -> List<Bool> {{ [1, 2] }}"));
}

#[test]
fn structural_arrays_keep_their_extent_and_element_contract() {
    allowed("fn probe() -> [Int; 2] { [1, 2] }");
    mismatch("fn probe() -> [Int; 2] { [1] }");
    mismatch("fn probe() -> [Bool; 2] { [1, 2] }");
}

#[test]
fn explicit_nominal_construction_remains_available() {
    allowed("public type Storage<T> is { public item: T }; fn probe() -> Storage<Int> { Storage { item: 7 } }");
}
