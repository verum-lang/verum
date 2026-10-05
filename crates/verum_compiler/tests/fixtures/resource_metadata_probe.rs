//! T1594 source -> VBC archive -> serialized metadata -> checker controls.
use std::sync::Arc;
use verum_ast::ItemKind;
use verum_common::{ResourceDiscipline as D, Text};
use verum_fast_parser::Parser;
use verum_types::{
    TypeChecker,
    core_metadata::{CoreMetadata, TypeDescriptorKind},
};
use verum_vbc::{archive::ArchiveBuilder, codegen::VbcCodegen};

fn metadata(convert: fn(&verum_vbc::archive::VbcArchive) -> CoreMetadata) -> CoreMetadata {
    metadata_with_modes(convert, false)
}

fn metadata_with_modes(
    convert: fn(&verum_vbc::archive::VbcArchive) -> CoreMetadata,
    swap: bool,
) -> CoreMetadata {
    let mut archive = ArchiveBuilder::stdlib();
    for (owner, body) in [
        (
            "alpha",
            "public type affine Token is { id: Int }; public type Alias is Token; public type Borrowed is { token: &Token }; public type linear Once is { id: Int }; @must_consume public type Ack is { id: Int };",
        ),
        (
            "beta",
            "public type Token is { id: Int }; public type Alias is Token;",
        ),
    ] {
        let body = if swap {
            if owner == "alpha" {
                body.replace("type affine Token", "type Token")
            } else {
                body.replace("type Token", "type affine Token")
            }
        } else {
            body.to_owned()
        };
        let ast = Parser::new(&format!("module {owner}; {body}"))
            .parse_module()
            .unwrap();
        let module = VbcCodegen::new().compile_module(&ast).unwrap();
        archive.add_module(owner, &module, &[]).unwrap();
    }
    let metadata = convert(&archive.finish());
    bincode::deserialize(&bincode::serialize(&metadata).unwrap()).unwrap()
}

fn errors(checker: &mut TypeChecker, source: &str) -> Vec<String> {
    checker.set_current_module_path("consumer");
    let ast = Parser::new(source).parse_module().unwrap();
    checker.register_stdlib_types_for_module(&ast);
    let mut errors = Vec::new();
    for item in &ast.items {
        if let ItemKind::Function(decl) = &item.kind {
            if let Err(error) = checker.register_function_signature(decl) {
                errors.push(format!("{error:?}"));
            }
        }
    }
    for item in &ast.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(format!("{error:?}"));
        }
    }
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|error| format!("{error:?}")),
    );
    errors.extend(
        checker
            .diagnostic_sources()
            .iter()
            .map(|error| format!("{error:?}")),
    );
    errors
}

pub fn exact_owners(convert: fn(&verum_vbc::archive::VbcArchive) -> CoreMetadata) {
    let metadata = Arc::new(metadata(convert));
    for eager in [false, true] {
        let mut checker = if eager {
            TypeChecker::new_with_core_eager(metadata.clone())
        } else {
            TypeChecker::new_with_core(metadata.clone())
        };
        for name in ["alpha.Token", "beta.Token", "alpha.Once", "alpha.Ack"] {
            checker.ensure_stdlib_type_loaded(&Text::from(name), &mut Vec::new());
        }
        assert_eq!(
            checker.declared_resource_discipline("alpha.Token"),
            D::Affine,
            "eager={eager}"
        );
        assert_eq!(
            checker.declared_resource_discipline("beta.Token"),
            D::Unrestricted,
            "eager={eager}"
        );
        assert_eq!(
            checker.declared_resource_discipline("alpha.Once"),
            D::Linear
        );
        assert_eq!(checker.declared_resource_discipline("alpha.Ack"), D::Linear);
        assert_eq!(
            checker.declared_resource_discipline("missing.Token"),
            D::Unknown
        );
    }
}

pub fn imported_consumption(convert: fn(&verum_vbc::archive::VbcArchive) -> CoreMetadata) {
    for swap in [false, true] {
        let metadata = Arc::new(metadata_with_modes(convert, swap));
        for eager in [false, true] {
            for name in ["alpha.Token", "beta.Token", "alpha.Alias", "beta.Alias"] {
                let moved = name.starts_with("alpha.") != swap;
                let mut checker = if eager {
                    TypeChecker::new_with_core_eager(metadata.clone())
                } else {
                    TypeChecker::new_with_core(metadata.clone())
                };
                let errors = errors(
                    &mut checker,
                    &format!(
                        "fn consume(value: {name}) {{}} fn probe(value: {name}) {{ consume(value); consume(value); }}"
                    ),
                );
                assert_eq!(
                    errors.iter().any(|error| error.contains("MovedValueUsed")),
                    moved,
                    "swap={swap}, eager={eager}, {name}: {errors:?}"
                );
                assert!(
                    errors.iter().all(|error| error.contains("MovedValueUsed")),
                    "{errors:?}"
                );
            }
        }
    }
}

pub fn borrowed_field(convert: fn(&verum_vbc::archive::VbcArchive) -> CoreMetadata) {
    let metadata = metadata(convert);
    let descriptor = metadata.types.get(&Text::from("alpha.Borrowed")).unwrap();
    let TypeDescriptorKind::Record { fields } = &descriptor.kind else {
        panic!("record")
    };
    assert!(
        fields[0].ty.as_str().starts_with('&'),
        "borrow erased: {:?}",
        fields[0].ty
    );
}

pub fn promoted_owner(convert: fn(&verum_vbc::archive::VbcArchive) -> CoreMetadata) {
    let mut metadata = metadata(convert);
    for descriptor in metadata.types.values_mut() {
        if descriptor.module_path.as_str() == "alpha" && descriptor.name.as_str() == "Token" {
            descriptor.name = Text::from("alpha.Token");
        }
    }
    let checker = TypeChecker::new_with_core_eager(Arc::new(metadata));
    assert_eq!(
        checker.declared_resource_discipline("alpha.Token"),
        D::Affine
    );
    assert_eq!(
        checker.declared_resource_discipline("alpha.alpha.Token"),
        D::Unknown
    );
}

pub fn generic_aliases(convert: fn(&verum_vbc::archive::VbcArchive) -> CoreMetadata) {
    let mut archive = ArchiveBuilder::stdlib();
    for (owner, qualifier) in [("alpha", "affine "), ("beta", "")] {
        let source = format!(
            "module {owner}; public type {qualifier}Token<T> is {{ value: T }}; public type Alias<T> is Token<T>;"
        );
        let module = VbcCodegen::new()
            .compile_module(&Parser::new(&source).parse_module().unwrap())
            .unwrap();
        archive.add_module(owner, &module, &[]).unwrap();
    }
    let metadata = Arc::new(convert(&archive.finish()));
    for eager in [false, true] {
        for (name, moved) in [
            ("alpha.Alias", true),
            ("beta.Alias", false),
            ("alpha.Token", true),
            ("beta.Token", false),
        ] {
            let mut checker = if eager {
                TypeChecker::new_with_core_eager(metadata.clone())
            } else {
                TypeChecker::new_with_core(metadata.clone())
            };
            let errors = errors(
                &mut checker,
                &format!(
                    "fn consume(value: {name}<Int>) {{}} fn probe(value: {name}<Int>) {{ consume(value); consume(value); }}"
                ),
            );
            assert_eq!(
                errors.iter().any(|error| error.contains("MovedValueUsed")),
                moved,
                "eager={eager}, {name}: {errors:?}"
            );
            assert!(
                errors.iter().all(|error| error.contains("MovedValueUsed")),
                "{errors:?}"
            );
        }
    }
}
