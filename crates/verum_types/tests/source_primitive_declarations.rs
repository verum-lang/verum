use verum_ast::{FileId, ItemKind};
use verum_fast_parser::FastParser;
use verum_modules::{
    ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module,
};
use verum_types::TypeChecker;
fn ast(source: &str, id: u32) -> verum_ast::Module {
    FastParser::new()
        .parse_module_str(source, FileId::new(id))
        .unwrap()
}
fn errors(provider: &str, consumer: &str) -> Vec<String> {
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_module_path("example.main");
    checker.set_current_cog("example");
    let mut registry = ModuleRegistry::new();
    let source = ast(provider, 1);
    let id = ModuleId::new(1);
    let path = ModulePath::from_str("example.types");
    let mut info = ModuleInfo::new(
        id,
        path.clone(),
        source.clone(),
        source.file_id,
        provider.into(),
    );
    info.exports = extract_exports_from_module(&source, id, &path).unwrap();
    registry.register(info);
    checker.set_module_registry_direct(registry.clone());
    let consumer = ast(consumer, 2);
    let mut errors = Vec::new();
    for item in &consumer.items {
        let result = if let ItemKind::Mount(mount) = &item.kind {
            checker.process_import(mount, "example.main", &registry)
        } else {
            checker.check_item(item)
        };
        if let Err(error) = result {
            errors.push(error.to_string());
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .iter()
            .map(ToString::to_string),
    );
    errors.extend(checker.diagnostic_sources().iter().map(ToString::to_string));
    errors
}
fn accepts(provider: &str, consumer: &str) {
    let errors = errors(provider, consumer);
    assert!(errors.is_empty(), "{provider}\n{consumer}\n{errors:#?}");
}
fn refuses(provider: &str, consumer: &str) {
    let errors = errors(provider, consumer);
    assert!(!errors.is_empty(), "accepted {consumer}");
}
#[test]
fn imported_actual_bottom_alias_preserves_bottom() {
    accepts(
        "public type Never is !;",
        "mount example.types.Never; fn eliminate<A>(x:Never)->A{x}",
    );
}
#[test]
fn renamed_bottom_alias_preserves_bottom() {
    accepts(
        "public type Never is !;",
        "mount example.types.Never as Empty; fn eliminate<A>(x:Empty)->A{x}",
    );
}
#[test]
fn bottom_alias_cannot_be_constructed() {
    refuses(
        "public type Never is !;",
        "mount example.types.Never; fn forge()->Never{Never{}}",
    );
}
#[test]
fn record_named_never_remains_a_record() {
    accepts(
        "public type Never is { public value:Int };",
        "mount example.types.Never; fn make()->Never{Never{value:42}}",
    );
}
#[test]
fn record_name_cannot_authorize_bottom_elimination() {
    refuses(
        "public type Never is { public value:Int };",
        "mount example.types.Never; fn forge<A>(x:Never)->A{x}",
    );
}
#[test]
fn primitive_named_alias_keeps_declared_target() {
    accepts(
        "public type Byte is UInt8;",
        "mount example.types.Byte; fn identity(x:Byte)->Byte{x}",
    );
}
#[test]
fn primitive_value_annotations_keep_their_meaning() {
    for name in ["Unit", "Bool", "Int", "Text"] {
        accepts(
            &format!("public type {name} is {{ public value:Int }};"),
            &format!(
                "mount example.types.{name}; fn unit()->(){{()}} fn integer()->Int{{42}} fn truth()->Bool{{true}}"
            ),
        );
    }
}
#[test]
fn private_alias_cannot_be_mounted() {
    refuses(
        "type Never is !;",
        "mount example.types.Never; fn eliminate<A>(x:Never)->A{x}",
    );
}

#[test]
fn renamed_primitive_spellings_retain_their_actual_records() {
    for name in [
        "Int", "Float", "Bool", "Char", "Text", "Unit", "Never", "Byte",
    ] {
        accepts(
            &format!("public type {name} is {{ public value:Int }};"),
            &format!("mount example.types.{name} as Data; fn make()->Data{{Data{{value:42}}}}"),
        );
    }
}

#[test]
fn qualified_primitive_spellings_retain_their_actual_records() {
    for name in [
        "Int", "Float", "Bool", "Char", "Text", "Unit", "Never", "Byte",
    ] {
        accepts(
            &format!("public type {name} is {{ public value:Int }};"),
            &format!("fn make()->example.types.{name}{{example.types.{name}{{value:42}}}}"),
        );
    }
}

#[test]
fn imported_scalar_methods_follow_the_declared_receiver() {
    for (declared, used) in [
        ("UInt64", "UInt64"), ("u64", "UInt64"), ("UInt64", "u64"),
        ("UInt16", "u16"), ("Int32", "i32"), ("Float32", "f32"),
        ("Bool", "Bool"),
    ] {
        let provider = format!("implement {declared} {{ public fn marked(self) -> Bool {{ true }} }}");
        accepts(&provider, &format!("mount example.types.*; fn use_it(x: {used}) -> Bool {{ x.marked() }}"));
        refuses(&provider, &format!("mount example.types.*; fn use_it(x: {used}) -> Int {{ x.marked() }}"));
        refuses(&provider, "mount example.types.*; fn wrong_receiver(x: Text) -> Bool { x.marked() }");
        refuses(&provider, &format!("fn not_imported(x: {used}) -> Bool {{ x.marked() }}"));
    }
}

#[test]
fn imported_scalar_method_can_use_a_source_alias() {
    accepts(
        "type Word is UInt64; implement Word { public fn marked(self) -> Bool { true } }",
        "mount example.types.*; fn use_it(x: UInt64) -> Bool { x.marked() }",
    );
}

#[test]
fn primitive_spelling_does_not_attach_a_record_method_to_a_scalar() {
    refuses(
        "public type UInt64 is { public value: Int }; implement UInt64 { public fn marked(self) -> Bool { true } }",
        "mount example.types.*; fn use_it(x: u64) -> Bool { x.marked() }",
    );
}
