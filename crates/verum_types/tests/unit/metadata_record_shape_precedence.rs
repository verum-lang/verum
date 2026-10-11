//! A legacy descriptor's bare key cannot replace a local declaration's arity.
use super::*;
use crate::core_metadata::{
    CoreMetadata, FieldDescriptor, GenericParam, TypeDescriptor, TypeDescriptorKind,
};
use verum_common::ResourceDiscipline;
use verum_fast_parser::Parser;

fn checker(owner: &str) -> TypeChecker {
    let descriptor = TypeDescriptor {
        name: "Packet".into(),
        module_path: owner.into(),
        origin_module_path: None,
        generic_params: [GenericParam {
            name: "Foreign".into(),
            bounds: List::new(),
            default: None,
            type_bounds: List::new(),
            pid: Some(0),
        }]
        .into_iter()
        .collect(),
        kind: TypeDescriptorKind::Record {
            fields: [FieldDescriptor {
                name: "foreign".into(),
                ty: "Foreign".into(),
                is_public: true,
                declared_visibility: None, // This control deliberately models a legacy descriptor.
            }]
            .into_iter()
            .collect(),
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
    let mut metadata = CoreMetadata::default();
    metadata.types.insert("Packet".into(), descriptor.clone());
    metadata
        .types
        .insert(TypeChecker::metadata_declaring_key(&descriptor), descriptor);
    let mut checker = TypeChecker::new_with_core(Shared::new(metadata).into_arc());
    checker.register_primitives();
    checker
}

fn declare_local(checker: &mut TypeChecker, source: &str) {
    let module = Parser::new(source).parse_module().expect("source grammar");
    let verum_ast::ItemKind::Type(declaration) = &module.items[0].kind else {
        panic!("type declaration")
    };
    checker
        .register_type_declaration(declaration)
        .expect("local registration");
    assert_eq!(
        checker.type_generics_count.get(&Text::from("Packet")),
        Some(&0)
    );
}

#[test]
fn lazy_legacy_record_keeps_each_local_declarations_parameter_authority() {
    for owner in ["", "cog"] {
        for source in [
            "type Packet is { body: Int };",
            "type Packet is Empty | Filled(Int);",
            "type Packet is ();",
        ] {
            let mut checker = checker(owner);
            declare_local(&mut checker, source);
            let parameters = checker.lookup_type_for_testing("__type_params_Packet");
            checker.ensure_stdlib_type_loaded(&"Packet".into(), &mut Default::default());
            assert_eq!(
                checker.type_generics_count.get(&Text::from("Packet")),
                Some(&0),
                "owner={owner:?},source={source}"
            );
            assert_eq!(
                checker.lookup_type_for_testing("__type_params_Packet"),
                parameters,
                "owner={owner:?},source={source}: metadata filled or replaced local parameters"
            );
        }
    }
}

#[test]
fn unclaimed_legacy_and_qualified_records_keep_their_declared_parameters() {
    for owner in ["", "cog", "archive.owner"] {
        let mut checker = checker(owner);
        checker.ensure_stdlib_type_loaded(&"Packet".into(), &mut Default::default());
        let key: Text = if owner.is_empty() || owner == "cog" {
            "Packet".into()
        } else {
            format!("{owner}.Packet").into()
        };
        assert_eq!(checker.recorded_type_arity(key.as_str()), Some(1));
        let fields = checker
            .resolve_to_record_type(&Type::Generic {
                name: key,
                args: [Type::Bool].into_iter().collect(),
            })
            .expect("record shape");
        assert_eq!(fields.get("foreign"), Some(&Type::Bool));
    }
}

#[test]
fn qualified_metadata_leaves_a_local_bare_arity_independent() {
    let mut checker = checker("archive.owner");
    declare_local(&mut checker, "type Packet is { body: Int };");
    checker.ensure_stdlib_type_loaded(&"Packet".into(), &mut Default::default());
    assert_eq!(
        checker.type_generics_count.get(&Text::from("Packet")),
        Some(&0)
    );
    assert_eq!(checker.recorded_type_arity("Packet"), Some(0));
    assert_eq!(checker.recorded_type_arity("archive.owner.Packet"), Some(1));
    let fields = checker
        .resolve_to_record_type(&Type::Generic {
            name: "archive.owner.Packet".into(),
            args: [Type::Bool].into_iter().collect(),
        })
        .expect("qualified metadata fields");
    assert_eq!(fields.get("foreign"), Some(&Type::Bool));
}
