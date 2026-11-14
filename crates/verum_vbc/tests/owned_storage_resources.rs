//! Runtime containers keep the resource constraints of their owned storage.
#![cfg(feature = "codegen")]

use verum_ast::{ItemKind, Module};
use verum_common::{List, ResourceDiscipline as D, Text};
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    deserialize::deserialize_module,
    module::VbcModule,
    serialize::serialize_module,
    types::{TypeDescriptor, TypeRef},
};

fn selected(source: &str, names: &[&str]) -> Module {
    let mut ast = Parser::new(source).parse_module().unwrap();
    ast.items.retain(|item| match &item.kind {
        ItemKind::Module(_) => true,
        ItemKind::Type(declaration) => names.contains(&declaration.name.name.as_str()),
        _ => false,
    });
    ast
}

fn compile(source: &str, list_owner: &str) -> VbcModule {
    let list = include_str!("../../../core/collections/list.vr").replace(
        "module core.collections.list;",
        &format!("module {list_owner};"),
    );
    let sources = [
        selected(
            include_str!("../../../core/base/protocols.vr"),
            &["PartialEq", "Eq", "Hash", "Hasher", "DefaultHasher"],
        ),
        selected(&list, &["List"]),
        selected(
            include_str!("../../../core/collections/map.vr"),
            &["Map", "Slot"],
        ),
        selected(
            include_str!("../../../core/base/memory.vr"),
            &["Heap", "Shared", "SharedInner"],
        ),
        Parser::new(source).parse_module().unwrap(),
    ];
    let roots: List<_> = sources.iter().collect();
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("app"));
    codegen.collect_unit_declarations(&roots).unwrap();
    codegen.resolve_pending_imports();
    codegen
        .compile_unit_items(&roots, ItemFailurePolicy::Strict)
        .unwrap();
    codegen.finalize_module_from_state().unwrap()
}

fn descriptor<'a>(module: &'a VbcModule, owner: &str, name: &str) -> &'a TypeDescriptor {
    module
        .types
        .iter()
        .find(|descriptor| {
            descriptor
                .origin_module
                .and_then(|id| module.get_string(id))
                == Some(owner)
                && module
                    .get_string(descriptor.name)
                    .is_some_and(|actual| actual == name || actual == format!("{owner}.{name}"))
        })
        .unwrap_or_else(|| panic!("missing exact {owner}.{name}"))
}

fn discipline(module: &VbcModule, name: &str) -> D {
    module.resource_discipline(&TypeRef::Concrete(descriptor(module, "app", name).id))
}

#[test]
fn actual_container_declarations_keep_owned_parameters_and_reference_boundaries() {
    let module = compile(
        r#"
        module app;
        mount core.collections.list.List;
        mount core.collections.map.Map;
        mount core.base.memory.{Heap, Shared};
        type affine Ticket is { id: Int };
        type OwnedList is List<Ticket>;
        type OwnedKeys is Map<Ticket, Int>;
        type OwnedValues is Map<Int, Ticket>;
        type BorrowedList is List<&Ticket>;
        type SharedList is List<Shared<Ticket>>;
        type Scalars is List<Int>;
        type Service is protocol { fn read(&self) -> Int; };
        type DynamicHandle is Shared<dyn Service>;
        type DynamicMap is Map<Int, Shared<dyn Service>>;
        type DynamicOwned is List<dyn Service>;
    "#,
        "core.collections.list",
    );
    let decoded = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    for module in [&module, &decoded] {
        for name in ["OwnedList", "OwnedKeys", "OwnedValues"] {
            assert_eq!(discipline(module, name), D::Affine, "{name}");
        }
        for name in [
            "BorrowedList",
            "SharedList",
            "Scalars",
            "DynamicHandle",
            "DynamicMap",
        ] {
            assert_eq!(discipline(module, name), D::Unrestricted, "{name}");
        }
        assert_eq!(discipline(module, "DynamicOwned"), D::Unknown);
        for (name, accepted) in [
            ("OwnedList", false),
            ("BorrowedList", true),
            ("DynamicMap", true),
            ("DynamicOwned", false),
        ] {
            let capture = Some(TypeRef::Concrete(descriptor(module, "app", name).id));
            assert_eq!(
                module
                    .validate_closure_captures(Some(&[capture]), 1, Clone::clone)
                    .is_ok(),
                accepted,
                "capture {name}"
            );
        }
    }
}

#[test]
fn same_named_foreign_record_retains_only_its_declared_fields() {
    let module = compile(
        r#"
        module app;
        mount foreign.storage.List;
        type affine Ticket is { id: Int };
        type BorrowedStorage is List<Ticket>;
    "#,
        "foreign.storage",
    );
    assert_eq!(discipline(&module, "BorrowedStorage"), D::Unrestricted);
}

#[test]
fn recursive_container_values_join_all_alternatives() {
    let module = compile(
        r#"
        module app;
        mount core.collections.list.List;
        mount core.collections.map.Map;
        mount core.base.memory.Heap;
        type Tree is Scalar(Int) | Children(List<Tree>) | Object(Map<Int, Tree>);
        type OwnedTree is Scalar(Int) | Children(List<OwnedTree>) | Owned(Heap<Int>);
        type DirectCycle is { storage: List<DirectCycle>, direct: DirectCycle };
        type MixedCycle is { storage: List<Bridge>, direct: Bridge };
        type Bridge is { back: MixedCycle };
        type Growing<T> is { children: List<Growing<(T, T)>> };
        type Unbounded is Growing<Int>;
    "#,
        "core.collections.list",
    );
    let decoded = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    for module in [&module, &decoded] {
        assert_eq!(discipline(module, "Tree"), D::Unrestricted);
        assert_eq!(discipline(module, "OwnedTree"), D::Affine);
        for name in ["DirectCycle", "MixedCycle", "Unbounded"] {
            assert_eq!(discipline(module, name), D::Unknown, "{name}");
        }
    }
}

#[test]
fn deep_owned_declaration_chain_uses_the_graph_budget() {
    let mut source = Text::from("module app; type affine End is (); ");
    let mut previous = Text::from("End");
    for index in 0..600 {
        let name: Text = format!("Node{index}").into();
        source.push_str(&format!("type {name} is {{ next: {previous} }}; "));
        previous = name;
    }
    let module = compile(&source, "core.collections.list");
    assert_eq!(discipline(&module, "Node599"), D::Affine);
}
