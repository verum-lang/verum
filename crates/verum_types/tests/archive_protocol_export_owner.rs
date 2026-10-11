//! T1732: archive protocol exports retain their declaration alongside methods.
//! The real producer emits both type and protocol descriptors for one protocol.

use std::sync::Arc;
use verum_ast::{FileId, ItemKind, Visibility};
use verum_common::{List, ResourceDiscipline, Text};
use verum_fast_parser::FastParser;
use verum_modules::{
    ExportKind, ExportTable, ExportedItem, ModuleId, ModuleInfo, ModulePath, ModuleRegistry,
    extract_exports_from_module,
};
use verum_types::core_metadata::{
    CoreMetadata, MethodSignature, ProtocolDescriptor, ReceiverKind, TypeDescriptor,
    TypeDescriptorKind,
};
use verum_types::{TypeChecker, TypeError};

const REQUESTED: &str = "core.protocol_alpha";
const FOREIGN: &str = "core.protocol_beta";

fn check(
    owner: Option<&str>,
    eager: bool,
    decoded: bool,
    protocol_surface: bool,
) -> (List<TypeError>, List<Text>) {
    let mut metadata = CoreMetadata::default();
    if let Some(owner) = owner {
        let method = MethodSignature {
            name: "inspect".into(),
            receiver: ReceiverKind::SelfRef,
            params: List::new(),
            return_type: "Int".into(),
            contexts: List::new(),
            is_async: false,
        };
        let methods = List::from_iter([method]);
        let descriptor = TypeDescriptor {
            name: "Inspect".into(),
            module_path: "core.bundle".into(),
            origin_module_path: Some(owner.into()),
            generic_params: List::new(),
            kind: TypeDescriptorKind::Protocol {
                super_protocols: List::new(),
                associated_types: List::new(),
                required_methods: methods.clone(),
                default_methods: List::new(),
            },
            size: None,
            alignment: None,
            methods: List::new(),
            implements: List::new(),
            decl_span: None,
            is_public: true,
            is_transparent_wrapper: false,
            resource_discipline: ResourceDiscipline::Unrestricted,
        };
        // Mirrors archive_metadata's common type registration, in addition
        // to the protocol method catalog produced by its Protocol arm.
        for key in [Text::from("Inspect"), format!("{owner}.Inspect").into()] {
            metadata.types.insert(key.clone(), descriptor.clone());
            metadata.type_declaration_order.push(key);
        }
        metadata.protocols.insert(
            "Inspect".into(),
            ProtocolDescriptor {
                name: "Inspect".into(),
                module_path: "core.bundle".into(),
                origin_module_path: Some(owner.into()),
                generic_params: List::new(),
                super_protocols: List::new(),
                associated_types: List::new(),
                required_methods: methods,
                default_methods: List::new(),
                decl_span: None,
            },
        );
    }
    let metadata = if decoded {
        serde_json::from_slice(&serde_json::to_vec(&metadata).expect("encode metadata"))
            .expect("decode metadata")
    } else {
        metadata
    };
    let mut checker = if eager {
        TypeChecker::new_with_core_eager(Arc::new(metadata))
    } else {
        TypeChecker::new_with_core(Arc::new(metadata))
    };
    checker.register_primitives();
    checker.set_current_module_path("consumer");
    let parser = FastParser::new();
    let file = FileId::new(0);
    let declared = parser
        .parse_module_str(
            "public type Inspect is protocol { fn inspect(&self) -> Int; };",
            file,
        )
        .expect("protocol grammar");
    let empty = parser.parse_module_str("", file).expect("archive AST");
    let id = ModuleId::new(0);
    let path = ModulePath::from_str(REQUESTED);
    let mut info = ModuleInfo::new(id, path.clone(), empty, file, "".into());
    info.exports = extract_exports_from_module(&declared, id, &path).expect("parsed exports");
    assert_eq!(
        info.exports.get(&"Inspect".into()).expect("protocol export").kind,
        ExportKind::Type,
    );
    if protocol_surface {
        // Parsed `type ... is protocol` exports Type. The ordinary archive
        // loader additionally creates Protocol exports from its method
        // catalog; preserve both consumer routes without changing the AST.
        info.exports = ExportTable::new();
        info.exports.add_export(ExportedItem::new(
            "Inspect", ExportKind::Protocol, Visibility::Public, id, declared.span,
        )).expect("archive protocol export");
    }
    let mut registry = ModuleRegistry::new();
    registry.register(info);
    checker.set_module_registry_direct(registry.clone());
    let module = parser
        .parse_module_str(
            "mount core.protocol_alpha.{Inspect};
             fn read<T: Inspect>(value: &T) -> Int { value.inspect() }",
            FileId::new(1),
        )
        .expect("consumer grammar");
    checker.register_stdlib_types_for_module(&module);
    let mut errors = List::new();
    for item in &module.items {
        if let ItemKind::Mount(import) = &item.kind {
            if let Err(error) = checker.process_import(import, "consumer", &registry) {
                errors.push(error);
            }
        }
    }
    for item in &module.items {
        if let ItemKind::Function(function) = &item.kind {
            if let Err(error) = checker.register_function_signature(function) {
                errors.push(error);
            }
        }
    }
    for item in &module.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(error);
        }
    }
    errors.extend(checker.take_deferred_errors());
    let diagnostics = checker.diagnostic_sources().iter()
        .map(|error| format!("{error:?}").into()).collect();
    (errors, diagnostics)
}

#[test]
fn archive_protocol_declaration_keeps_its_method_contract() {
    for eager in [false, true] {
        for decoded in [false, true] {
            for protocol_surface in [false, true] {
                let (errors, diagnostics) = check(Some(REQUESTED), eager, decoded, protocol_surface);
                assert!(errors.is_empty() && diagnostics.is_empty(),
                    "eager={eager} decoded={decoded} protocol_surface={protocol_surface}: {errors:?} {diagnostics:?}");
            }
        }
    }
}

#[test]
fn archive_protocol_export_cannot_borrow_another_owner() {
    for owner in [None, Some(FOREIGN)] {
        for eager in [false, true] {
            for decoded in [false, true] {
                for protocol_surface in [false, true] {
                    let (errors, diagnostics) = check(owner, eager, decoded, protocol_surface);
                    assert!(errors.iter().any(|error| matches!(error,
                        TypeError::ImportItemNotFound { item_name, module_path, .. }
                            if item_name.as_str() == "Inspect" && module_path.as_str() == REQUESTED
                    )), "owner={owner:?} eager={eager} decoded={decoded} protocol_surface={protocol_surface}: {errors:?} {diagnostics:?}");
                }
            }
        }
    }
}
