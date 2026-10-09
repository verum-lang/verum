//! Alias expansion uses the declaration's parameter slots, including archive templates.
use super::*;
use crate::core_metadata::{CoreMetadata, GenericParam, TypeDescriptor, TypeDescriptorKind};
use crate::infer::parse_descriptor_type_string as parse_type;
use verum_common::ResourceDiscipline;
use verum_fast_parser::Parser;

fn metadata_checker(eager: bool) -> TypeChecker {
    let mut metadata = CoreMetadata::default();
    let descriptor = TypeDescriptor {
        name: "Pair".into(),
        module_path: "alpha".into(),
        origin_module_path: None,
        generic_params: ["First", "Second"].into_iter().map(|name| GenericParam {
            name: name.into(),
            bounds: List::new(),
            default: None,
            type_bounds: List::new(),
            pid: None,
        }).collect(),
        kind: TypeDescriptorKind::Record { fields: List::new() },
        size: None,
        alignment: None,
        methods: List::new(),
        implements: List::new(),
        decl_span: None,
        is_public: true,
        is_transparent_wrapper: false,
        resource_discipline: ResourceDiscipline::Unrestricted,
    };
    metadata.types.insert("Pair".into(), descriptor.clone());
    metadata.types.insert("alpha.Pair".into(), descriptor);
    let metadata = Shared::new(metadata).into_arc();
    let mut checker = if eager {
        TypeChecker::new_with_core_eager(metadata)
    } else {
        TypeChecker::new_with_core(metadata)
    };
    checker.ensure_stdlib_type_loaded(&"Pair".into(), &mut List::new().into());
    checker.ensure_stdlib_type_loaded(&"alpha.Pair".into(), &mut List::new().into());
    checker
}

#[test]
fn alias_arguments_archive_template_keeps_distinct_slots_and_uses() {
    for eager in [false, true] {
        let checker = metadata_checker(eager);
        for spelling in ["Pair", "alpha.Pair"] {
            for (first, second) in [(Type::Text, Type::Bool), (Type::Bool, Type::Int)] {
                let mut application = parse_type(spelling);
                let Type::Named { args, .. } = &mut application else { panic!("named type") };
                args.extend([first.clone(), second.clone()]);
                let expected = Type::Generic {
                    name: "alpha.Pair".into(),
                    args: List::from_iter([first, second]),
                };
                assert_eq!(checker.expand_type_alias(&application), Some(expected),
                    "{spelling}, eager={eager}");
            }
        }
    }
}

#[test]
fn alias_arguments_source_declaration_preserves_reordered_and_unused_slots() {
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_module_path("alpha");
    let module = Parser::new("type Select<Unused, Last, First> is fn(First, Last)->First;")
        .parse_module().expect("source grammar");
    for item in &module.items {
        checker.check_item(item).expect("source declaration");
    }
    for spelling in ["Select", "alpha.Select"] {
        let application = parse_type(&format!("{spelling}<Int, Bool, Text>"));
        let expected = Type::function(List::from_iter([Type::Text, Type::Bool]), Type::Text);
        assert_eq!(checker.expand_type_alias(&application), Some(expected), "{spelling}");
    }
}

#[test]
fn alias_arguments_missing_qualified_owner_does_not_borrow_bare_template() {
    for eager in [false, true] {
        let checker = metadata_checker(eager);
        assert_eq!(checker.expand_type_alias(&parse_type("beta.Pair<Text, Bool>")), None);
    }
}
