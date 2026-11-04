use super::*;
use verum_fast_parser::Parser;
fn ast(source: &str) -> Module {
    Parser::new(source).parse_module().unwrap()
}
#[test]
fn representation_hints_are_ambiguous_across_file_owners_in_either_order() {
    for opaque in [
        "public type Cell is { value:Int };",
        "public type Cell is (Int, Int);",
    ] {
        let transparent = ast("public type Cell is (Int);");
        let opaque = ast(opaque);
        for reverse in [false, true] {
            let mut sources = vec![("fixture.alpha", &transparent), ("fixture.beta", &opaque)];
            if reverse {
                sources.reverse();
            }
            assert_eq!(
                source_type_names(sources).ambiguous,
                HashSet::from(["Cell".to_owned()])
            );
        }
    }
}
#[test]
fn repeated_same_owner_does_not_create_false_ambiguity() {
    let one = ast("public type Token is (Int);");
    let other = ast("public type Other is (Int);");
    assert!(
        source_type_names([
            ("fixture.one", &one),
            ("fixture.one", &one),
            ("fixture.two", &other)
        ])
        .ambiguous
        .is_empty()
    );
}
#[test]
fn same_leaf_aliases_retain_each_files_qualified_key_and_target() {
    let alpha = ast("public type Left is {}; public type Alias is Left;");
    let beta = ast("public type Right is {}; public type Alias is Right;");
    for reverse in [false, true] {
        let mut sources = vec![("fixture.alpha", &alpha), ("fixture.beta", &beta)];
        if reverse {
            sources.reverse();
        }
        let names = source_type_names(sources);
        let aliases: std::collections::BTreeMap<_, _> = names
            .aliases
            .into_iter()
            .map(|a| (a.qualified_name, a.target))
            .collect();
        assert_eq!(
            aliases,
            std::collections::BTreeMap::from([
                ("fixture.alpha.Alias".to_owned(), "Left".to_owned()),
                ("fixture.beta.Alias".to_owned(), "Right".to_owned()),
            ])
        );
        assert_eq!(names.ambiguous, HashSet::from(["Alias".to_owned()]));
    }
}
#[test]
fn sibling_declaration_cannot_turn_an_unmounted_marker_into_an_alias() {
    let alpha = ast("public type OnlyHere is {};");
    let beta = ast("public type Marker is OnlyHere;");
    for sources in [
        [("fixture.alpha", &alpha), ("fixture.beta", &beta)],
        [("fixture.beta", &beta), ("fixture.alpha", &alpha)],
    ] {
        assert!(source_type_names(sources).aliases.is_empty());
    }
}
#[test]
fn explicit_mount_admits_the_declaring_files_alias() {
    let file = ast("mount fixture.alpha.OnlyHere; public type Alias is OnlyHere;");
    let names = source_type_names([("fixture.beta", &file)]);
    assert_eq!(names.aliases.len(), 1);
    assert_eq!(names.aliases[0].qualified_name, "fixture.beta.Alias");
    assert_eq!(names.aliases[0].local_name, "Alias");
    assert_eq!(names.aliases[0].target, "OnlyHere");
}

#[test]
fn ambiguous_short_aliases_are_evicted_and_cannot_return_on_completion() {
    let alpha = ast("public type Left is {}; public type Alias is Left;");
    let beta = ast("public type Right is {}; public type Alias is Right;");
    for reverse in [false, true] {
        let mut sources = vec![("fixture.alpha", &alpha), ("fixture.beta", &beta)];
        if reverse {
            sources.reverse();
        }
        let names = source_type_names(sources);
        let mut registry = HashMap::from([("Alias".to_owned(), "Unrelated".to_owned())]);
        merge_type_aliases(
            &mut registry,
            names.aliases.iter().flat_map(|alias| {
                [
                    (alias.qualified_name.clone(), alias.target.clone()),
                    (alias.local_name.clone(), alias.target.clone()),
                ]
            }),
            &names.ambiguous,
        );
        assert!(!registry.contains_key("Alias"));
        assert_eq!(registry["fixture.alpha.Alias"], "Left");
        assert_eq!(registry["fixture.beta.Alias"], "Right");
        // Both completion orders try to restore their bare convenience name.
        // Qualified declaration entries must survive the same merge policy.
        let mut completed = vec![
            ("fixture.alpha.Alias", "Left"),
            ("fixture.beta.Alias", "Right"),
        ];
        if reverse {
            completed.reverse();
        }
        for (key, target) in completed {
            merge_type_aliases(
                &mut registry,
                [
                    ("Alias".to_owned(), target.to_owned()),
                    (key.to_owned(), target.to_owned()),
                ],
                &names.ambiguous,
            );
            assert!(!registry.contains_key("Alias"));
        }
        assert_eq!(registry.len(), 2);
        assert_eq!(registry["fixture.alpha.Alias"], "Left");
        assert_eq!(registry["fixture.beta.Alias"], "Right");
    }
}
#[test]
fn unambiguous_short_alias_compatibility_keeps_existing_precedence() {
    let source = ast("public type Target is {}; public type Unique is Target;");
    let names = source_type_names([("fixture.only", &source)]);
    let mut registry = HashMap::new();
    merge_type_aliases(
        &mut registry,
        names.aliases.iter().flat_map(|alias| {
            [
                (alias.qualified_name.clone(), alias.target.clone()),
                (alias.local_name.clone(), alias.target.clone()),
            ]
        }),
        &names.ambiguous,
    );
    assert_eq!(registry["Unique"], "Target");
    assert_eq!(registry["fixture.only.Unique"], "Target");
    merge_type_aliases(
        &mut registry,
        [("Unique".to_owned(), "Later".to_owned())],
        &names.ambiguous,
    );
    assert_eq!(registry["Unique"], "Target");
}
