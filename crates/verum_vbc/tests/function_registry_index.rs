//! T2114: indexed suffix lookup must preserve the old key filter under mutation.
#![cfg(feature = "codegen")]

use std::panic::{AssertUnwindSafe, catch_unwind};
use verum_common::List;
use verum_vbc::codegen::context::{CodegenContext, FunctionInfo, FunctionRegistry};
use verum_vbc::module::FunctionId;

fn info(id: u32, arity: usize) -> FunctionInfo {
    FunctionInfo {
        id: FunctionId(id),
        param_count: arity,
        ..Default::default()
    }
}

fn assert_forms(registry: &FunctionRegistry, dotted: &str, colon: &str) {
    // This is the exact pre-index predicate, deliberately independent of the
    // index's data structure and tokenization of qualified names.
    let dotted_suffix = format!(".{dotted}");
    let colon_suffix = format!("::{colon}");
    let mut expected: List<_> = registry
        .keys()
        .filter(|key| key.ends_with(&dotted_suffix) || key.ends_with(&colon_suffix))
        .cloned()
        .collect();
    expected.sort();
    expected.dedup();
    let actual = registry.qualified_suffix_candidates_with_forms(dotted, colon);
    assert_eq!(
        actual.as_slice(),
        expected.as_slice(),
        "forms {dotted:?}, {colon:?}"
    );
}

fn assert_index(registry: &FunctionRegistry) {
    // Include absent and removed-key suffixes so phantom index entries cannot
    // hide merely because the corresponding map key has disappeared.
    for suffix in [
        "",
        "f",
        "f#2",
        "f#3",
        "beta.f",
        "beta::f",
        "δ",
        "β.δ",
        ".",
        "::",
        ".f",
        ":f",
        "missing",
        "only_bare",
        "entry.f",
    ] {
        assert_forms(registry, suffix, suffix);
        let candidates = registry.qualified_suffix_candidates(suffix);
        assert_eq!(
            candidates,
            registry.qualified_suffix_candidates_with_forms(suffix, suffix)
        );
    }
    // Every separator position contributes a query, including overlapping
    // colon separators and UTF-8 prefixes.
    for key in registry.keys() {
        for (index, character) in key.char_indices() {
            if character == '.' {
                let suffix = &key[index + 1..];
                assert_forms(registry, suffix, suffix);
            }
            if key[index..].starts_with("::") {
                let suffix = &key[index + 2..];
                assert_forms(registry, suffix, suffix);
            }
        }
    }
}

#[test]
fn suffix_candidates_preserve_literal_separator_and_overload_semantics() {
    let mut registry = FunctionRegistry::new();
    assert_index(&registry);
    for (index, name) in [
        "only_bare",
        "f",
        "alpha.f",
        "alpha::f",
        "alpha.beta.f",
        "alpha::beta::f",
        "alpha.beta::f",
        "alpha::beta.f",
        ".f",
        "::f",
        "alpha..f",
        "alpha:::f",
        "alpha.f#2",
        "alpha::f#2",
        "alpha.beta.f#3",
        "модуль.δ",
        "東京::β.δ",
        "::δ",
        ".",
        "::",
        "alpha.",
        "alpha::",
        "alpha..",
        "alpha::::",
        "notaf",
        "",
    ]
    .into_iter()
    .enumerate()
    {
        registry.insert(name.into(), info(index as u32, index % 4));
        assert_index(&registry);
    }
    assert!(registry.qualified_suffix_candidates("only_bare").is_empty());
    assert!(
        !registry
            .qualified_suffix_candidates("f")
            .iter()
            .any(|key| key.contains('#'))
    );
    let names: List<_> = registry.keys().cloned().collect();
    for name in names {
        registry.remove(name.as_str()).unwrap();
        assert_index(&registry);
    }
}

#[test]
fn distinct_separator_forms_do_not_admit_extra_mixed_spellings() {
    let registry: FunctionRegistry = [
        "module.part.call",
        "module::part::call",
        "module.part::call",
        "module::part.call",
        "root.module.part.call",
        "root::module::part::call",
        "root.module::part::call",
        "root::module.part.call",
        "root.module.part::call",
        "root::module::part.call",
    ]
    .into_iter()
    .enumerate()
    .map(|(id, name)| (name.into(), info(id as u32, 0)))
    .collect();
    assert_index(&registry);
    for (dotted, colon) in [
        ("module.part.call", "module::part::call"),
        ("part.call", "part::call"),
        ("part::call", "part.call"),
        ("", "part::call"),
    ] {
        assert_forms(&registry, dotted, colon);
    }
    let candidates =
        registry.qualified_suffix_candidates_with_forms("module.part.call", "module::part::call");
    assert_eq!(
        candidates,
        ["root.module.part.call", "root::module::part::call"]
    );
}

#[test]
fn entry_mutations_and_equal_size_replacements_keep_names_current() {
    let mut registry = FunctionRegistry::default();
    registry.insert("primary.f".into(), info(1, 1));
    assert_index(&registry);
    registry.insert("primary.f".into(), info(2, 2));
    assert_index(&registry);
    assert_eq!(registry.get("primary.f").unwrap().id, FunctionId(2));
    assert!(registry.remove("missing.f").is_none());
    assert_index(&registry);

    assert_eq!(registry.entry("unused.f".into()).key(), "unused.f");
    assert_index(&registry);
    assert!(!registry.contains_key("unused.f"));
    drop(registry.entry("primary.f".into()));
    assert_index(&registry);
    registry
        .entry("primary.f".into())
        .or_insert_with(|| panic!("occupied entry factory ran"));
    assert_index(&registry);
    registry.entry("inserted.f".into()).or_insert(info(3, 3));
    assert_index(&registry);
    registry
        .entry("factory.f".into())
        .or_insert_with(|| info(4, 4));
    assert_index(&registry);
    registry
        .entry("keyed.entry.f".into())
        .or_insert_with_key(|key| {
            assert_eq!(key, "keyed.entry.f");
            info(5, 5)
        });
    assert_index(&registry);
    registry.entry("default.f".into()).or_default().param_count = 6;
    assert_index(&registry);
    registry
        .entry("primary.f".into())
        .and_modify(|value| value.param_count = 9)
        .or_insert(info(99, 99));
    assert_eq!(registry.get("primary.f").unwrap().param_count, 9);
    assert_index(&registry);
    registry
        .entry("vacant.entry.f".into())
        .and_modify(|_| panic!("vacant modify ran"))
        .or_insert(info(6, 6));
    assert_index(&registry);

    registry.clear();
    assert_index(&registry);
    for name in ["left.f", "center.f", "right.f"] {
        registry.insert(name.into(), info(1, 1));
        assert_index(&registry);
    }
    for (old, new) in [
        ("left.f", "north.f"),
        ("center.f", "middle.f"),
        ("right.f", "south.f"),
    ] {
        let size = registry.len();
        let value = registry.remove(old).unwrap();
        assert_index(&registry);
        registry.insert(new.into(), value);
        assert_eq!(registry.len(), size);
        assert_index(&registry);
    }
    registry.clear();
    assert_index(&registry);
    registry.clear();
    assert_index(&registry);
}

#[test]
fn collection_construction_clone_retain_and_extend_preserve_index() {
    let map: std::collections::HashMap<_, _> = [
        ("keep.f".to_owned(), info(1, 1)),
        ("drop.f".to_owned(), info(2, 2)),
    ]
    .into_iter()
    .collect();
    let mut registry = FunctionRegistry::from(map);
    assert_index(&registry);
    let clone = registry.clone();
    assert_index(&clone);
    registry.retain(|key, value| {
        value.param_count += 1;
        !key.starts_with("drop")
    });
    assert_index(&registry);
    assert_index(&clone);
    assert!(clone.contains_key("drop.f"));
    assert_eq!(registry.get("keep.f").unwrap().param_count, 2);
    assert_eq!(clone.get("keep.f").unwrap().param_count, 1);
    registry.extend([
        ("keep.f".into(), info(3, 3)),
        ("added::f".into(), info(4, 4)),
    ]);
    assert_index(&registry);
    let from_iter: FunctionRegistry = [
        ("same.f".into(), info(5, 5)),
        ("same.f".into(), info(6, 6)),
        ("other::f".into(), info(7, 7)),
    ]
    .into_iter()
    .collect();
    assert_index(&from_iter);
    assert_eq!(from_iter.get("same.f").unwrap().id, FunctionId(6));
    let readonly: &std::collections::HashMap<_, _> = &from_iter;
    assert_eq!(readonly.len(), 2);
    let mut count = 0;
    for (name, value) in &from_iter {
        assert_eq!(from_iter.get(name).unwrap().id, value.id);
        count += 1;
    }
    assert_eq!(count, from_iter.len());
}

#[test]
fn arity_lookup_reads_live_values_after_every_mutable_access_path() {
    let mut context = CodegenContext::new();
    for (name, value) in [
        ("scope.f", info(10, 1)),
        ("scope.f#2", info(20, 2)),
        ("scope.f#3", info(30, 3)),
    ] {
        context.functions.insert(name.into(), value);
        assert_index(&context.functions);
    }
    assert_eq!(
        context.lookup_function_with_arity("scope.f", 2).unwrap().id,
        FunctionId(20)
    );
    let original_candidates = context.functions.qualified_suffix_candidates("f");
    {
        let value = context.functions.get_mut("scope.f").unwrap();
        value.id = FunctionId(11);
        value.param_count = 2;
        value.return_type_name = Some("Changed".into());
    }
    assert_index(&context.functions);
    let selected = context.lookup_function_with_arity("scope.f", 2).unwrap();
    assert_eq!(selected.id, FunctionId(11));
    assert_eq!(selected.return_type_name.as_deref(), Some("Changed"));
    for value in context.functions.values_mut() {
        if value.id == FunctionId(11) {
            value.id = FunctionId(12);
            value.param_count = 3;
        }
    }
    assert_index(&context.functions);
    assert_eq!(
        context.lookup_function_with_arity("scope.f", 2).unwrap().id,
        FunctionId(20)
    );
    assert_eq!(
        context.lookup_function_with_arity("scope.f", 3).unwrap().id,
        FunctionId(12)
    );
    {
        let value = context.functions.entry("scope.f".into()).or_default();
        value.id = FunctionId(13);
        value.param_count = 2;
    }
    assert_index(&context.functions);
    assert_eq!(
        context.lookup_function_with_arity("scope.f", 2).unwrap().id,
        FunctionId(13)
    );
    context
        .functions
        .entry("scope.f".into())
        .and_modify(|value| value.param_count = 1)
        .or_default();
    assert_index(&context.functions);
    assert_eq!(
        context.lookup_function_with_arity("scope.f", 2).unwrap().id,
        FunctionId(20)
    );
    assert_eq!(
        context.functions.qualified_suffix_candidates("f"),
        original_candidates
    );
}

#[test]
fn panicking_entry_factories_do_not_leave_phantom_names() {
    let mut registry = FunctionRegistry::new();
    registry.insert("present.f".into(), info(1, 1));
    assert_index(&registry);
    assert!(
        catch_unwind(AssertUnwindSafe(|| {
            registry
                .entry("panic.f".into())
                .or_insert_with(|| panic!("factory failed"));
        }))
        .is_err()
    );
    assert!(!registry.contains_key("panic.f"));
    assert_index(&registry);
    assert!(
        catch_unwind(AssertUnwindSafe(|| {
            registry
                .entry("panic.keyed.f".into())
                .or_insert_with_key(|_| panic!("keyed factory failed"));
        }))
        .is_err()
    );
    assert!(!registry.contains_key("panic.keyed.f"));
    assert_index(&registry);
    assert!(
        catch_unwind(AssertUnwindSafe(|| {
            registry
                .entry("present.f".into())
                .and_modify(|value| {
                    value.param_count = 8;
                    panic!("value mutation failed");
                })
                .or_default();
        }))
        .is_err()
    );
    assert_eq!(registry.get("present.f").unwrap().param_count, 8);
    assert_index(&registry);
    registry.entry("panic.f".into()).or_insert(info(2, 2));
    assert_index(&registry);
}

#[test]
fn lookup_by_id_uses_the_resolved_name_only_while_it_matches() {
    let mut ctx = CodegenContext::new();
    let original = FunctionId(71);
    ctx.functions.insert("first".into(), info(original.0, 1));
    assert_eq!(ctx.lookup_function_by_id(original).unwrap().param_count, 1);
    ctx.lookup_function("first").unwrap();
    ctx.functions.insert("alias".into(), info(original.0, 1));
    ctx.functions.insert("first".into(), info(72, 3));
    assert_eq!(ctx.lookup_function_by_id(original).unwrap().id, original);
    ctx.functions.remove("alias");
    assert!(ctx.lookup_function_by_id(original).is_none());
    assert_eq!(ctx.lookup_function_by_id(FunctionId(72)).unwrap().param_count, 3);
}
