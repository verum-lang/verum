//! T1720: a type declaration's own parameter is an alias target, not a marker.
//! These controls use the production parser and both source registration paths.

use verum_ast::{FileId, ItemKind, Module, TypeDecl, decl::TypeDeclBody, ty::TypeKind};
use verum_common::{List, Text};
use verum_fast_parser::FastParser;
use verum_types::{Type, TypeChecker};

fn parse(source: &str) -> Module {
    FastParser::new()
        .parse_module_str(source, FileId::new(0))
        .expect("fixture follows the production grammar")
}

fn declaration<'a>(module: &'a Module, name: &str) -> &'a TypeDecl {
    module
        .items
        .iter()
        .find_map(|item| match &item.kind {
            ItemKind::Type(declaration) if declaration.name.name.as_str() == name => {
                Some(declaration)
            }
            _ => None,
        })
        .expect("fixture declaration exists")
}

fn assert_alias(module: &Module, name: &str, target: &str) {
    let body = &declaration(module, name).body;
    let TypeDeclBody::Alias(ty) = body else {
        panic!("{name} must be a declared-parameter alias, got {body:?}");
    };
    let TypeKind::Path(path) = &ty.kind else {
        panic!("{name} alias target must retain a path: {ty:?}");
    };
    assert_eq!(path.to_string(), target);
}

#[derive(Clone, Copy, Debug)]
enum Registration {
    SinglePass,
    TwoPass,
}

const REGISTRATIONS: [Registration; 2] = [Registration::SinglePass, Registration::TwoPass];

fn registered(source: &str, registration: Registration) -> (Module, TypeChecker) {
    let module = parse(source);
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_module_path("alias_owner");
    match registration {
        Registration::SinglePass => {
            for item in &module.items {
                if let ItemKind::Type(declaration) = &item.kind {
                    checker
                        .register_type_declaration(declaration)
                        .expect("single-pass declaration registration");
                }
            }
        }
        Registration::TwoPass => {
            checker.register_all_type_names(&module.items);
            for result in checker.resolve_all_type_definitions(&module.items) {
                result.expect("two-pass declaration resolution");
            }
        }
    }
    (module, checker)
}

fn annotation(checker: &mut TypeChecker, source_type: &str) -> Type {
    let module = parse(&format!("fn probe(value: {source_type}) {{}}"));
    let ItemKind::Function(function) = &module.items[0].kind else {
        panic!("fixture function");
    };
    let verum_ast::decl::FunctionParamKind::Regular { ty, .. } = &function.params[0].kind else {
        panic!("fixture typed parameter");
    };
    checker.ast_to_type(ty).expect("annotation resolves")
}

fn assert_packet(ty: &Type) {
    let Type::Named { path, args } = ty else {
        panic!("alias must preserve the nominal record owner, got {ty:?}");
    };
    let segments: List<Text> = path
        .segments
        .iter()
        .map(|segment| match segment {
            verum_ast::ty::PathSegment::Name(name) => name.name.clone(),
            _ => panic!("nominal owner must contain resolved declaration names: {path:?}"),
        })
        .collect();
    assert_eq!(segments.len(), 2);
    assert_eq!(segments[0].as_str(), "alias_owner");
    assert_eq!(segments[1].as_str(), "Packet");
    assert!(args.is_empty(), "Packet has no generic arguments");
}

#[test]
fn bare_declared_parameter_is_an_alias_in_the_parsed_ast() {
    assert_alias(&parse("type Identity<T> is T;"), "Identity", "T");
}

#[test]
fn selected_parameter_need_not_be_the_first_parameter() {
    assert_alias(
        &parse("type Select<Unused, Chosen> is Chosen;"),
        "Select",
        "Chosen",
    );
}

#[test]
fn other_type_parameter_kinds_are_alias_targets_in_the_parsed_ast() {
    for (source, target) in [
        ("type Identity<T: Type> is T;", "T"),
        ("type Identity<F<_>> is F;", "F"),
    ] {
        assert_alias(&parse(source), "Identity", target);
    }
}

#[test]
fn a_parameter_does_not_turn_another_declarations_marker_into_an_alias() {
    for source in [
        "type Identity<T> is T; type Marker is T;",
        "type Marker is T; type Identity<T> is T;",
    ] {
        let module = parse(source);
        assert!(matches!(
            declaration(&module, "Marker").body,
            TypeDeclBody::Variant(_)
        ));
    }
}

#[test]
fn unknown_and_explicit_markers_keep_their_variant_meaning() {
    for source in [
        "type Marker<T> is Missing;",
        "type Marker<T> is | T;",
        "type Marker<T> is | Missing;",
        "type Marker<T> is T(T);",
        "type Marker<T> is T | Other;",
    ] {
        let module = parse(source);
        assert!(
            matches!(
                declaration(&module, "Marker").body,
                TypeDeclBody::Variant(_)
            ),
            "marker syntax must remain a variant: {source}"
        );
    }
}

#[test]
fn a_known_concrete_type_remains_an_alias_target() {
    let module = parse("type Packet is { public number: Int }; type Alias is Packet;");
    assert_alias(&module, "Alias", "Packet");
}

#[test]
fn declared_parameter_shadows_an_ambient_type_in_both_source_orders() {
    for source in [
        "type T is { public unrelated: Bool }; type Identity<T> is T;",
        "type Identity<T> is T; type T is { public unrelated: Bool };",
    ] {
        for registration in REGISTRATIONS {
            let (module, mut checker) = registered(source, registration);
            assert_alias(&module, "Identity", "T");
            assert_eq!(annotation(&mut checker, "Identity<Int>"), Type::Int);
        }
    }
}

#[test]
fn both_registration_paths_publish_the_alias_template() {
    for registration in REGISTRATIONS {
        let (_, mut checker) = registered("type Identity<T> is T;", registration);
        let target = checker
            .context_mut()
            .type_aliases
            .get(&Text::from("alias_owner.Identity"))
            .cloned()
            .expect("exact declared alias target must be registered");
        let Type::Named { path, args } = target else {
            panic!(
                "named declaration parameter, not a fresh variant slot: {registration:?}: {target:?}"
            );
        };
        assert_eq!(path.to_string(), "T");
        assert!(args.is_empty());
    }
}

#[test]
fn generic_alias_substitution_preserves_the_exact_nominal_owner() {
    let source = "type Packet is { public number: Int }; type Identity<T> is T;";
    for registration in REGISTRATIONS {
        let (_, mut checker) = registered(source, registration);
        assert_packet(&annotation(&mut checker, "Packet"));
        for applied in ["Identity<Packet>", "Identity<Identity<Packet>>"] {
            assert_packet(&annotation(&mut checker, applied));
        }
    }
}

#[test]
fn unused_and_reordered_parameters_keep_the_selected_argument() {
    let source = "type Packet is { public number: Int }; type Select<Unused, Chosen> is Chosen;";
    for registration in REGISTRATIONS {
        let (_, mut checker) = registered(source, registration);
        assert_packet(&annotation(&mut checker, "Select<Bool, Packet>"));
        assert_eq!(annotation(&mut checker, "Select<Packet, Int>"), Type::Int);
    }
}

fn field_errors(return_type: &str, registration: Registration) -> List<Text> {
    let source = format!(
        "type Packet is {{ public number: Int }}; type Identity<T> is T; fn probe(value: &Identity<Packet>) -> {return_type} {{ value.number }}"
    );
    let (module, mut checker) = registered(&source, registration);
    let mut errors = List::new();
    for item in &module.items {
        if let ItemKind::Function(function) = &item.kind {
            if let Err(error) = checker.register_function_signature(function) {
                errors.push(format!("{error:?}").into());
            }
        }
    }
    for item in &module.items {
        if matches!(&item.kind, ItemKind::Function(_)) {
            if let Err(error) = checker.check_item(item) {
                errors.push(format!("{error:?}").into());
            }
        }
    }
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
    errors
}

#[test]
fn alias_field_reads_use_the_declared_field_type() {
    for registration in REGISTRATIONS {
        let errors = field_errors("Int", registration);
        assert!(errors.is_empty(), "{registration:?}: {errors:?}");
    }
}

#[test]
fn alias_field_reads_cannot_manufacture_a_requested_result_type() {
    for registration in REGISTRATIONS {
        let errors = field_errors("Bool", registration);
        assert!(
            errors.iter().any(|error| error.contains("Mismatch")),
            "{registration:?}: the Int field cannot satisfy Bool: {errors:?}"
        );
    }
}
