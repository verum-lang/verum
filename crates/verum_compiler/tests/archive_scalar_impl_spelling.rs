//! T1547: source-declared scalar impl targets survive the public archive metadata API.

use verum_compiler::archive_metadata::archive_to_core_metadata;
use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};

#[test]
fn specialized_nominal_receiver_keeps_the_declaring_type_in_metadata() {
    let owner = "range_probe";
    let ast = Parser::new(
        "module range_probe;
        public type Counter is protocol { fn value(&self)->Int {0} };
        public type Cursor<A> is { item:A };
        implement Counter for Cursor<Int> { fn value(&self)->Int {41} }",
    )
    .parse_module()
    .unwrap();
    let module = VbcCodegen::with_config(CodegenConfig::new(owner))
        .compile_module(&ast)
        .unwrap();
    let mut archive = verum_vbc::archive::ArchiveBuilder::stdlib();
    archive.add_module(owner, &module, &[]).unwrap();
    let metadata = archive_to_core_metadata(&archive.finish()).expect("valid specialized receiver");
    let implementation = metadata
        .implementations
        .iter()
        .find(|item| item.protocol.as_str() == "Counter")
        .expect("Counter implementation");
    let target = metadata
        .types
        .get(&implementation.target_type)
        .expect("Cursor declaration");
    assert_eq!(target.name.as_str(), "Cursor");
    assert_eq!(
        target
            .origin_module_path
            .as_ref()
            .unwrap_or(&target.module_path)
            .as_str(),
        owner
    );
}

fn check(first: &str, second: &str) {
    let source = format!(
        r#"
        module scalar_alias_probe;
        type FirstOnly is protocol {{}};
        type SecondOnly is protocol {{}};
        implement FirstOnly for {first} {{}}
        implement SecondOnly for {second} {{}}
    "#
    );
    let ast = Parser::new(&source).parse_module().unwrap();
    let module = VbcCodegen::with_config(CodegenConfig::new("scalar_alias_probe"))
        .compile_module(&ast)
        .unwrap();
    let module = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    )
    .unwrap();
    let mut archive = verum_vbc::archive::ArchiveBuilder::stdlib();
    archive
        .add_module("scalar_alias_probe", &module, &[])
        .unwrap();
    let metadata = archive_to_core_metadata(&archive.finish()).expect("valid archive metadata");
    let mut actual: Vec<_> = metadata
        .implementations
        .iter()
        .map(|implementation| {
            (
                implementation.target_type.as_str().to_owned(),
                implementation.protocol.as_str().to_owned(),
            )
        })
        .collect();
    actual.sort();
    let mut expected = vec![
        (first.to_owned(), "FirstOnly".into()),
        (second.to_owned(), "SecondOnly".into()),
    ];
    expected.sort();
    assert_eq!(actual, expected);
}

#[test]
fn source_scalar_alias_associations_survive_archive_metadata_in_both_orders() {
    for (first, second) in [
        ("Byte", "UInt8"),
        ("UInt8", "Byte"),
        ("USize", "ISize"),
        ("ISize", "USize"),
    ] {
        check(first, second);
    }
}

#[test]
fn source_distinct_scalar_associations_remain_independent() {
    check("Bool", "Int");
    check("Int", "Bool");
}

#[test]
fn scalar_method_ownership_is_independent_of_implementation_module() {
    let modules = [
        ("extensions.first", "public fn encode_first(self) -> [Byte; 3] { [1, 2, 3] }"),
        ("extensions.second", "public fn encode_second(self) -> Bool { true }"),
    ];
    for entries in [modules, [modules[1], modules[0]]] {
        let mut archive = verum_vbc::archive::ArchiveBuilder::stdlib();
        for (name, method) in entries {
            let source = format!("module {name}; implement UInt64 {{ {method} }}");
            let ast = Parser::new(&source).parse_module().unwrap();
            let module = VbcCodegen::with_config(CodegenConfig::new(name)).compile_module(&ast).unwrap();
            archive.add_module(name, &module, &[]).unwrap();
        }
        let metadata = archive_to_core_metadata(&archive.finish()).unwrap();
        let scalar = metadata.types.get(&"UInt64".into()).unwrap();
        assert!(scalar.module_path.is_empty() && scalar.origin_module_path.is_none());
        for (method, origin, result) in [
            // This isolated module has no core.base.Byte declaration. Its wire
            // element is the ABI UInt8 and the semantic return retains its extent.
            ("encode_first", "extensions.first", "[UInt8; 3]"),
            ("encode_second", "extensions.second", "Bool"),
        ] {
            assert!(scalar.methods.iter().any(|name| name.as_str() == method));
            let function = metadata.functions.get(&format!("UInt64.{method}").into()).unwrap();
            assert_eq!(function.parent_type.as_ref().unwrap().as_str(), "UInt64");
            assert_eq!(function.origin_module_path.as_ref().unwrap_or(&function.module_path).as_str(), origin);
            assert_eq!(function.return_type.as_str(), result);
            assert!(!metadata.functions.contains_key(&format!("{origin}.UInt64.{method}").into()));
        }
        assert!(!scalar.methods.iter().any(|name| name.as_str() == "to_be_bytes"));
    }
}

fn compiled_module(name: &str, body: &str) -> verum_vbc::module::VbcModule {
    let source = format!("module {name}; {body}");
    let ast = Parser::new(&source).parse_module().unwrap();
    VbcCodegen::with_config(CodegenConfig::new(name)).compile_module(&ast).unwrap()
}

#[test]
fn scalar_and_nominal_homonym_keep_their_exact_method_slots_in_both_orders() {
    for (nominal, scalar) in [("aaa.names", "zzz.ext"), ("zzz.names", "aaa.ext")] {
        let record = compiled_module(nominal,
            "public type UInt64 is { n: Int }; implement UInt64 {
                public fn same(self) -> Bool { true }
                public fn record_only(self) -> Bool { true }
            }");
        let extension = compiled_module(scalar,
            "implement UInt64 { public fn same(self) -> Int { 7 } }");
        let mut archive = verum_vbc::archive::ArchiveBuilder::stdlib();
        archive.add_module(nominal, &record, &[]).unwrap();
        archive.add_module(scalar, &extension, &[]).unwrap();
        let meta = archive_to_core_metadata(&archive.finish()).unwrap();
        let owner = format!("{nominal}.UInt64");
        assert!(!meta.functions.contains_key(&"UInt64.record_only".into()));
        assert!(meta.functions.contains_key(&format!("{owner}.record_only").into()));
        for (name, result) in [("UInt64", "Int"), (owner.as_str(), "Bool")] {
            let ty = meta.types.get(&name.into()).unwrap();
            let method = meta.functions.get(&format!("{name}.same").into()).unwrap();
            assert!(ty.methods.iter().any(|name| name.as_str() == "same"));
            assert_eq!(method.parent_type.as_ref().unwrap().as_str(), name);
            assert_eq!(method.return_type.as_str(), result);
        }
    }
}

#[test]
fn contradictory_parents_cannot_publish_a_scalar_method_through_its_stored_name() {
    use verum_vbc::types::{TypeDescriptor, TypeId, TypeKind};
    for invalid in ["unknown", "wrong-id", "wrong-descriptor", "conflicting", "conflicting-reverse", "free"] {
        let mut bad = compiled_module("aaa.decoy",
            "implement UInt64 { public fn same(self) -> Bool { true } }");
        let function = bad.functions.iter_mut().find(|f| bad.strings.get(f.name) == Some("UInt64.same")).unwrap();
        match invalid {
            "unknown" => function.parent_type = Some(TypeId(32991)),
            "wrong-id" => function.parent_type = Some(TypeId::BOOL),
            "free" => function.parent_type = None,
            _ => {
                bad.types.retain(|ty| ty.id != TypeId::U64);
                let wrong = TypeDescriptor {
                    id: TypeId::U64, kind: TypeKind::Primitive,
                    name: bad.strings.intern("WrongName"), ..Default::default()
                };
                let right = TypeDescriptor {
                    id: TypeId::U64, kind: TypeKind::Primitive,
                    name: bad.strings.intern("UInt64"), ..Default::default()
                };
                match invalid {
                    "conflicting" => bad.types.extend([right, wrong]),
                    "conflicting-reverse" => bad.types.extend([wrong, right]),
                    _ => bad.types.push(wrong),
                }
            }
        }
        let mut archive = verum_vbc::archive::ArchiveBuilder::stdlib();
        archive.add_module("aaa.decoy", &bad, &[]).unwrap();
        let alone = archive_to_core_metadata(&archive.finish()).unwrap();
        assert!(!alone.functions.contains_key(&"UInt64.same".into()), "{invalid}");

        let real = compiled_module("zzz.extensions", "implement UInt64 { public fn same(self) -> Int { 7 } }");
        let mut archive = verum_vbc::archive::ArchiveBuilder::stdlib();
        archive.add_module("aaa.decoy", &bad, &[]).unwrap();
        archive.add_module("zzz.extensions", &real, &[]).unwrap();
        let meta = archive_to_core_metadata(&archive.finish()).unwrap();
        let method = meta.functions.get(&"UInt64.same".into()).unwrap();
        assert_eq!(method.parent_type.as_ref().unwrap().as_str(), "UInt64");
        assert_eq!(method.return_type.as_str(), "Int");
    }
}

#[test]
fn scalar_methods_keep_the_declared_alias_when_the_abi_parent_is_shared() {
    use verum_vbc::types::{TypeDescriptor, TypeId, TypeKind};
    for (declared, abi, id) in [("Int64", "Int", TypeId::I64), ("Float64", "Float", TypeId::F64),
        ("ISize", "USize", TypeId::ISIZE), ("Byte", "UInt8", TypeId::U8)] {
        let mut module = compiled_module("ext", &format!("implement {declared} {{ public fn flag(self) -> Bool {{ true }} }}"));
        module.types.retain(|ty| ty.id != id);
        module.types.push(TypeDescriptor { id, kind: TypeKind::Primitive, name: module.strings.intern(abi), ..Default::default() });
        let mut archive = verum_vbc::archive::ArchiveBuilder::stdlib();
        archive.add_module("ext", &module, &[]).unwrap();
        let meta = archive_to_core_metadata(&archive.finish()).unwrap();
        let function = meta.functions.get(&format!("{declared}.flag").into()).unwrap();
        assert_eq!(function.parent_type.as_ref().unwrap().as_str(), declared);
        assert!(!meta.functions.contains_key(&format!("{abi}.flag").into()));
    }
}

#[test]
fn a_module_qualified_nominal_declaration_outranks_an_entry_alias() {
    use verum_types::core_metadata::TypeDescriptorKind;
    let carrier = compiled_module("implementation",
        "type Marker is protocol {}; implement Marker for UInt64 {}");
    for (owner, bundle) in [("aaa.names", "zzz.bundle"), ("zzz.names", "aaa.bundle")] {
        let record = compiled_module(owner,
            "public type UInt64 is { n: Int }; implement UInt64 { public fn read(self) -> Bool { true } }");
        let mut archive = verum_vbc::archive::ArchiveBuilder::stdlib();
        // Entry names can cover a different file owner. Its scalar carrier
        // contributes a compatibility alias at the nominal declaration's path.
        archive.add_module(owner, &carrier, &[]).unwrap();
        archive.add_module(bundle, &record, &[]).unwrap();
        let meta = archive_to_core_metadata(&archive.finish()).unwrap();
        let nominal = meta.types.get(&format!("{owner}.UInt64").into()).unwrap();
        assert!(matches!(nominal.kind, TypeDescriptorKind::Record { .. }));
        assert_eq!(nominal.origin_module_path.as_ref().unwrap_or(&nominal.module_path).as_str(), owner);
        assert!(nominal.methods.iter().any(|name| name.as_str() == "read"));
    }
}


#[test]
fn a_nominal_byte_method_cannot_rewrite_its_uint8_result_to_the_receiver() {
    let module = compiled_module("forms",
        "public type Byte is { n: Int }; implement Byte { public fn item(self) -> UInt8 { 1 } }");
    let mut archive = verum_vbc::archive::ArchiveBuilder::stdlib();
    archive.add_module("forms", &module, &[]).unwrap();
    let meta = archive_to_core_metadata(&archive.finish()).unwrap();
    let method = meta.functions.get(&"forms.Byte.item".into()).unwrap();
    assert_eq!(method.parent_type.as_ref().unwrap().as_str(), "forms.Byte");
    assert_eq!(method.return_type.as_str(), "UInt8");
}
