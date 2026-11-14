//! T0691: declaration names must survive the archive-to-symbol-graph boundary.
//! Synthetic archives distinguish this naming contract from TLS root selection
//! and consumer string-pool remapping. CI selects these through compiler --lib.

use super::*;
use std::borrow::Cow;
use verum_common::{List, Maybe, Text};
use verum_vbc::archive::{ArchiveBuilder, read_archive, write_archive};
use verum_vbc::instruction::{Reg, RegRange};
use verum_vbc::module::{FunctionDescriptor, FunctionId, XMOD_CALL_ID_BAND_BASE};

fn add_function(module: &mut VbcModule, name: &str, owner: Maybe<&str>, callee: Maybe<&str>) {
    let id = FunctionId(module.functions.len().try_into().unwrap());
    let mut body = List::new();
    if let Some(callee) = callee {
        let external_id = XMOD_CALL_ID_BAND_BASE + id.0;
        let callee_name = module.strings.intern(callee);
        module
            .external_function_names
            .push((FunctionId(external_id), callee_name));
        body.push(Instruction::Call {
            dst: Reg(0),
            func_id: external_id,
            args: RegRange::new(Reg(0), 0),
        });
    }
    body.push(Instruction::RetV);
    let offset = module.bytecode.len();
    verum_vbc::bytecode::encode_instructions(&body, &mut module.bytecode);
    module.functions.push(FunctionDescriptor {
        id,
        name: module.strings.intern(name),
        origin_module: owner.map(|owner| module.strings.intern(owner)),
        bytecode_offset: offset.try_into().unwrap(),
        bytecode_length: (module.bytecode.len() - offset).try_into().unwrap(),
        register_count: 1,
        ..Default::default()
    });
}

fn module(entry: &str, name: &str, owner: Maybe<&str>, callee: Maybe<&str>) -> VbcModule {
    let mut module = VbcModule::new(entry.into());
    add_function(&mut module, name, owner, callee);
    module
}

fn archive(modules: &[VbcModule]) -> VbcArchive {
    let mut builder = ArchiveBuilder::new();
    for module in modules {
        builder.add_module(&module.name, module, &[]).unwrap();
    }
    let mut bytes = List::new();
    write_archive(&builder.finish(), &mut bytes).unwrap();
    read_archive(std::io::Cursor::new(bytes.as_slice())).unwrap()
}

fn sibling_archive(reverse: bool) -> VbcArchive {
    let mut modules: List<_> = [
        module(
            "alpha",
            "Counter.new",
            Some("alpha.owner"),
            Some("alpha.dep.finish"),
        ),
        module(
            "beta",
            "Counter.new",
            Some("beta.owner"),
            Some("beta.dep.finish"),
        ),
        module("alpha_tail", "finish", Some("alpha.dep"), None),
        module("beta_tail", "finish", Some("beta.dep"), None),
        module("client", "run", None, Some("beta.owner.Counter.new")),
    ]
    .into_iter()
    .collect();
    if reverse {
        modules.reverse();
    }
    archive(&modules)
}

fn assert_exact_beta_closure(graph: &SymbolGraph, archive: &VbcArchive) {
    let seeds = ["client.run".to_owned()].into_iter().collect();
    let (reached, entries) = graph.reachable(&seeds, &Default::default());
    assert!(reached.contains("beta.owner.Counter.new"), "{reached:?}");
    assert!(
        reached.contains("beta.dep.finish"),
        "the declared node must carry its callees: {reached:?}"
    );
    assert!(
        !reached.contains("alpha.owner.Counter.new"),
        "a same-leaf sibling is not the callee"
    );
    assert!(
        !reached.contains("alpha.dep.finish"),
        "a sibling's edges must not replace the callee's"
    );
    let mut names: List<Text> = entries
        .into_iter()
        .map(|index| archive.index[index as usize].name.clone().into())
        .collect();
    names.sort();
    let expected: List<Text> = ["beta", "beta_tail", "client"]
        .into_iter()
        .map(Text::from)
        .collect();
    assert_eq!(
        names, expected,
        "qualified calls must preserve exact entry reachability"
    );
}

#[test]
fn scan_retains_declaring_owner_and_existing_descriptor_spellings() {
    for reverse in [false, true] {
        let archive = sibling_archive(reverse);
        let graph = SymbolGraph::build(&archive).expect("valid archive");
        for (owner, entry) in [
            ("alpha.owner.Counter.new", "alpha"),
            ("beta.owner.Counter.new", "beta"),
        ] {
            assert!(graph.has_symbol(owner), "missing declaration {owner}");
            assert_eq!(graph.defining_entry(owner, &archive), Some(entry));
        }
        for name in ["Counter.new", "alpha.Counter.new", "beta.Counter.new"] {
            assert!(
                graph.has_symbol(name),
                "existing spelling disappeared: {name}"
            );
        }
        for absent in [
            "gamma.owner.Counter.new",
            "beta.owner_extra.Counter.new",
            "beta.other.Counter.new",
        ] {
            assert!(
                !graph.has_symbol(absent),
                "undeclared owner acquired a symbol: {absent}"
            );
        }
        assert_eq!(
            graph.baked.leaf_match_count("new"),
            2,
            "owner aliases must not enlarge bare-leaf fanout"
        );
    }
}

#[test]
fn exact_declaring_owner_carries_its_transitive_call_edges_in_both_orders() {
    for reverse in [false, true] {
        let archive = sibling_archive(reverse);
        assert_exact_beta_closure(&SymbolGraph::build(&archive).expect("valid archive"), &archive);
    }
}

#[test]
fn encoded_sidecar_roundtrip_keeps_the_declaring_owner_and_edges() {
    let archive = sibling_archive(false);
    let encoded = SymbolGraph::scan_and_encode(&archive).expect("valid archive");
    let decoded = crate::symbol_graph_baked::BakedSymbolGraph::from_bytes(Cow::Owned(
        encoded.as_bytes().to_owned(),
    ))
    .unwrap();
    let graph = SymbolGraph { baked: decoded };
    assert_eq!(
        graph.defining_entry("beta.owner.Counter.new", &archive),
        Some("beta")
    );
    assert_exact_beta_closure(&graph, &archive);
}

#[test]
fn two_declarers_in_one_umbrella_keep_distinct_edges() {
    for reverse in [false, true] {
        let mut umbrella = VbcModule::new("beta".into());
        let mut owners = ["beta.owner", "beta.sibling"];
        if reverse {
            owners.reverse();
        }
        for owner in owners {
            add_function(
                &mut umbrella,
                "Counter.new",
                Some(owner),
                Some(&format!("{owner}.finish")),
            );
        }
        let archive = archive(&[umbrella]);
        let graph = SymbolGraph::build(&archive).expect("valid archive");
        for owner in owners {
            let name = format!("{owner}.Counter.new");
            let index = graph
                .baked
                .function_index(&name)
                .expect("each declaration must have its own node");
            let callees: List<Text> = graph.baked.callees(index).map(Text::from).collect();
            let expected: List<Text> = [Text::from(format!("{owner}.finish"))]
                .into_iter()
                .collect();
            assert_eq!(callees, expected);
        }
    }
}

#[test]
fn absent_origin_keeps_legacy_entry_qualification() {
    let archive = archive(&[module("legacy", "Counter.new", None, None)]);
    let graph = SymbolGraph::build(&archive).expect("valid archive");
    assert!(graph.has_symbol("Counter.new"));
    assert_eq!(
        graph.defining_entry("legacy.Counter.new", &archive),
        Some("legacy")
    );
    assert!(!graph.has_symbol("legacy.owner.Counter.new"));
}

#[test]
fn promoted_names_preserve_overlapping_module_segments_once() {
    let archive = archive(&[
        module("beta", "owner.Counter.new", Some("beta.owner"), None),
        module(
            "gamma",
            "gamma.owner.Counter.new",
            Some("gamma.owner"),
            None,
        ),
    ]);
    let graph = SymbolGraph::build(&archive).expect("valid archive");
    assert_eq!(
        graph.defining_entry("beta.owner.Counter.new", &archive),
        Some("beta")
    );
    assert_eq!(
        graph.defining_entry("gamma.owner.Counter.new", &archive),
        Some("gamma")
    );
    assert!(!graph.has_symbol("beta.owner.owner.Counter.new"));
    assert!(!graph.has_symbol("gamma.owner.gamma.owner.Counter.new"));
}


#[test]
fn constructor_and_type_annotation_keep_both_dependency_rows() {
    use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
    let source = verum_fast_parser::Parser::new(
        "module pkg.owner;
         fn allowed(value:Int)->Bool {value>0}
         type Holder is (Int{allowed(it)}, Int);",
    ).parse_module().unwrap();
    let mut module = VbcCodegen::with_config(CodegenConfig::new("pkg.owner"))
        .compile_module(&source).unwrap();
    // The graph also accepts an executable row at a constructor's exact name.
    // Keep the nominal metadata from its real source producer, not a seed.
    add_function(&mut module, "Holder", Some("pkg.owner"), Some("pkg.owner.finish"));
    add_function(&mut module, "allowed", Some("pkg.decoy"), None);
    let graph = SymbolGraph::build(&archive(&[module])).unwrap();
    let index = graph.baked.function_index("pkg.owner.Holder").unwrap();
    let dependencies: List<_> = graph.baked.callees(index).collect();
    assert!(dependencies.contains(&"pkg.owner.finish"), "{dependencies:?}");
    assert!(dependencies.contains(&"pkg.owner.allowed"), "{dependencies:?}");
    assert!(!dependencies.contains(&"pkg.decoy.allowed"), "{dependencies:?}");
}

#[test]
fn graph_without_declaration_dependencies_is_not_reused() {
    let archive = sibling_archive(false);
    let graph = SymbolGraph::scan_and_encode(&archive).unwrap();
    let mut bytes = graph.as_bytes().to_owned();
    bytes[4..8].copy_from_slice(&2u32.to_le_bytes());
    assert!(crate::symbol_graph_baked::BakedSymbolGraph::from_bytes(Cow::Owned(bytes)).is_none());
}

#[test]
fn refinement_value_binder_does_not_reach_homonymous_owner_function() {
    use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
    let source = verum_fast_parser::Parser::new(
        "module pkg.owner;
         fn unrelated()->Int {73}
         fn it()->Int {unrelated()}
         fn allowed(value:Int)->Bool {value>0}
         type Holder is {value:Int{allowed(it)}};",
    )
    .parse_module()
    .unwrap();
    let module = VbcCodegen::with_config(CodegenConfig::new("pkg.owner"))
        .compile_module(&source)
        .unwrap();
    let archive = archive(&[module]);
    let graph = SymbolGraph::build(&archive).unwrap();
    let index = graph.baked.function_index("pkg.owner.Holder").unwrap();
    let dependencies: List<_> = graph.baked.callees(index).collect();
    assert!(
        dependencies.contains(&"pkg.owner.allowed"),
        "{dependencies:?}"
    );
    assert!(
        !dependencies.contains(&"pkg.owner.it"),
        "the refinement's value binder is not a module callable: {dependencies:?}"
    );
    let seeds = ["pkg.owner.Holder".to_owned()].into_iter().collect();
    let (reached, _) = graph.reachable(&seeds, &Default::default());
    assert!(!reached.contains("pkg.owner.it"), "{reached:?}");
    assert!(!reached.contains("pkg.owner.unrelated"), "{reached:?}");
}

fn annotation_dependencies(declaration: &str) -> List<Text> {
    use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
    let source = format!("module pkg.owner; fn allowed(value:Int)->Bool {{value>0}} {declaration}");
    let parsed = verum_fast_parser::Parser::new(&source)
        .parse_module()
        .unwrap();
    let module = VbcCodegen::with_config(CodegenConfig::new("pkg.owner"))
        .compile_module(&parsed)
        .unwrap();
    let archive = archive(&[module]);
    let graph = SymbolGraph::build(&archive).unwrap();
    let index = graph.baked.function_index("pkg.owner.Holder").unwrap();
    graph.baked.callees(index).map(Text::from).collect()
}

#[test]
fn declaration_telescope_and_method_parameters_are_not_module_dependencies() {
    for (source, bound) in [
        ("type Holder<T> is {value:T, valid:Int{allowed(it)}};", "T"),
        ("type Holder<N:meta Int> is {value:Int{allowed(N)}};", "N"),
        (
            "type Holder(limit:Int) is {value:Int{allowed(limit)}};",
            "limit",
        ),
        (
            "type Holder is protocol {fn invoke(value:Int)->Int{allowed(value)};};",
            "value",
        ),
        ("type Holder is {callback:fn<T>(T)->Int{allowed(it)}};", "T"),
        (
            "type Holder is {carrier:Type, value:carrier, valid:Int{allowed(it)}};",
            "carrier",
        ),
    ] {
        let dependencies = annotation_dependencies(source);
        assert!(
            dependencies.contains(&"pkg.owner.allowed".into()),
            "{source}: {dependencies:?}"
        );
        assert!(
            !dependencies.contains(&format!("pkg.owner.{bound}").into()),
            "{source}: {dependencies:?}"
        );
    }
}

#[test]
fn scalar_record_field_does_not_hide_a_module_predicate() {
    let dependencies =
        annotation_dependencies("type Holder is {allowed:Int, value:Int{allowed(it)}};");
    assert!(
        dependencies.contains(&"pkg.owner.allowed".into()),
        "{dependencies:?}"
    );
}

#[test]
fn expression_binders_are_local_to_their_expression_scopes() {
    for source in [
        "type Holder is Int where |value| allowed(value);",
        "type Holder is Int{{let value=it;allowed(value)}};",
        "type Holder is Int{{const value:Int=73;allowed(value)}};",
        "type Holder is Int{( |value:Int| allowed(value) )(it)};",
        "type Holder is Int{match it {value=>allowed(value)}};",
        "type Holder is Int{if let value=it {allowed(value)} else {false}};",
        "type Holder is Int{forall value:Int. allowed(value)};",
        "type Holder is Int{[allowed(value) for value in [it]].len()>0};",
        "type Holder is Int{{for value in [it] {allowed(value);} true}};",
    ] {
        let dependencies = annotation_dependencies(source);
        assert!(
            dependencies.contains(&"pkg.owner.allowed".into()),
            "{source}: {dependencies:?}"
        );
        assert!(
            !dependencies.contains(&"pkg.owner.value".into()),
            "{source}: {dependencies:?}"
        );
        assert!(
            !dependencies.contains(&"pkg.owner.it".into()),
            "{source}: {dependencies:?}"
        );
    }
}

#[test]
fn bound_spelling_does_not_erase_free_uses_in_initializers_or_siblings() {
    for source in [
        "type Holder is Int{{let allowed=allowed(it);allowed}};",
        "type Holder is (Int where |allowed| allowed>0, Int{allowed(it)});",
        "type Holder is Int{if let allowed=it {allowed>0} else {allowed(it)}};",
        "type Holder is Int{match it {allowed if allowed>0=>true, _=>allowed(it)}};",
        "type Holder is {callback:fn<allowed>(allowed)->Bool, value:Int{allowed(it)}};",
    ] {
        let dependencies = annotation_dependencies(source);
        assert!(
            dependencies.contains(&"pkg.owner.allowed".into()),
            "{source}: {dependencies:?}"
        );
    }
}

#[test]
fn qualified_and_relative_helpers_keep_their_defining_owner() {
    for (predicate, expected) in [
        ("pkg.rules.allowed(it)", "pkg.rules.allowed"),
        ("super.rules.allowed(it)", "pkg.rules.allowed"),
    ] {
        let dependencies = annotation_dependencies(&format!("type Holder is Int{{{predicate}}};"));
        assert!(
            dependencies.contains(&expected.into()),
            "{predicate}: {dependencies:?}"
        );
        assert!(
            !dependencies.contains(&"pkg.owner.allowed".into()),
            "{predicate}: {dependencies:?}"
        );
    }
}

#[test]
fn sidecar_with_unscoped_annotation_paths_is_rebuilt() {
    let archive = sibling_archive(false);
    let graph = SymbolGraph::scan_and_encode(&archive).unwrap();
    let mut bytes = graph.as_bytes().to_owned();
    bytes[4..8].copy_from_slice(&3u32.to_le_bytes());
    assert!(crate::symbol_graph_baked::BakedSymbolGraph::from_bytes(Cow::Owned(bytes)).is_none());
}

#[test]
fn alternative_pattern_bindings_do_not_hide_other_alternatives_view_calls() {
    for source in [
        "type Holder is Int{match it {(outside -> value) | (value -> value)=>allowed(value)}};",
        "type Holder is Int{match it {(value -> value) | (outside -> value)=>allowed(value)}};",
    ] {
        let dependencies = annotation_dependencies(source);
        for helper in ["allowed", "outside", "value"] {
            assert!(
                dependencies.contains(&format!("pkg.owner.{helper}").into()),
                "{source}: {dependencies:?}"
            );
        }
    }
}

#[test]
fn annotation_graph_uses_the_existing_source_carrier_budget() {
    use verum_ast::{Expr, ExprKind, TypeDeclBody, TypeKind};
    use verum_common::Heap;
    use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
    let source = verum_fast_parser::Parser::new(
        "module pkg.owner; fn allowed(value:Int)->Bool {value>0} type Holder is Int{allowed(it)};",
    )
    .parse_module()
    .unwrap();
    let original = VbcCodegen::with_config(CodegenConfig::new("pkg.owner"))
        .compile_module(&source)
        .unwrap();
    for depth in [8, 192] {
        let mut module = original.clone();
        let source = module
            .types
            .iter_mut()
            .find_map(|ty| {
                ty.source_declaration
                    .as_mut()
                    .filter(|source| source.name.name == "Holder")
            })
            .unwrap();
        let TypeDeclBody::Alias(ty) = &mut source.body else {
            panic!("refined alias");
        };
        let TypeKind::Refined { predicate, .. } = &mut ty.kind else {
            panic!("retained predicate");
        };
        let mut expression = predicate.expr.clone();
        for _ in 0..depth {
            expression = Expr::new(ExprKind::Paren(Heap::new(expression)), predicate.span);
        }
        predicate.expr = expression;
        let mut builder = ArchiveBuilder::new();
        let encoded = builder.add_module(&module.name, &module, &[]);
        if depth == 8 {
            encoded.unwrap();
            let graph = SymbolGraph::build(&builder.finish()).unwrap();
            let index = graph.baked.function_index("pkg.owner.Holder").unwrap();
            let dependencies: List<_> = graph.baked.callees(index).collect();
            assert!(
                dependencies.contains(&"pkg.owner.allowed"),
                "{dependencies:?}"
            );
            assert!(!dependencies.contains(&"pkg.owner.it"), "{dependencies:?}");
        } else {
            let error = encoded
                .expect_err("oversized source must be refused before scoped graph traversal");
            assert!(error.to_string().contains("structural budget"), "{error}");
        }
    }
}

#[test]
fn implicit_refinement_spelling_is_free_outside_a_refinement_scope() {
    let dependencies =
        annotation_dependencies("type Holder is protocol {fn read(value:Int=it())->Int;};");
    assert!(
        dependencies.contains(&"pkg.owner.it".into()),
        "{dependencies:?}"
    );
}

#[test]
fn value_binders_with_universe_annotations_keep_the_type_namespace() {
    let cases = [
        "let Carrier:Type=witness; let inner:Carrier=value; inner",
        "let Carrier:Type=witness else {return value;}; let inner:Carrier=value; inner",
        "let read = |Carrier:Type| {let inner:Carrier=value;inner}; read(witness)",
        "let _ = [{let inner:Carrier=value;inner} for token in [true] let Carrier:Type=witness]; value",
        "try {value} recover |Carrier:Type| {let inner:Carrier=value;inner}",
    ];
    for body in cases {
        for has_type in [true, false] {
            let prefix = if has_type { "type Carrier is Int;" } else { "" };
            let source = format!("{prefix} fn inspect(witness:Type,value:Int)->Int {{{body}}}");
            let parsed = verum_fast_parser::Parser::new(&source)
                .parse_module()
                .unwrap();
            let mut checker = verum_types::TypeChecker::new();
            checker.register_primitives();
            checker.set_universe_poly_enabled(true);
            let mut errors = checker.prepare_module_declarations(&parsed);
            for item in &parsed.items {
                if let Err(error) = checker.check_item(item) {
                    errors.push(error);
                }
            }
            errors.extend(checker.take_deferred_errors());
            let diagnostics: List<_> = checker
                .diagnostics()
                .iter()
                .filter(|diagnostic| diagnostic.is_error())
                .collect();
            if has_type {
                assert!(
                    errors.is_empty() && diagnostics.is_empty(),
                    "{source}: {errors:?} {diagnostics:?}"
                );
            } else {
                assert!(errors.iter().any(|error| matches!(error, verum_types::TypeError::TypeNotFound {name,..} if name == "Carrier")) || diagnostics.iter().any(|error| error.code() == Some("E101") && error.message() == "type not found: Carrier"), "{source}: {errors:?} {diagnostics:?}");
            }
        }
    }
    for source in [
        "type Holder is protocol {fn read(Carrier:Type,value:Carrier)->Int;};",
        "type Holder is Int{{let Carrier:Type=witness;let value:Carrier=it;allowed(value)}};",
        "type Holder is Int{{let Carrier:Type=witness else {return false;};let value:Carrier=it;allowed(value)}};",
        "type Holder is Int{(|Carrier:Type| {let value:Carrier=it;allowed(value)})(witness)};",
        "type Holder is Int{[allowed(value) for token in [true] let Carrier:Type=witness let value:Carrier=it].len()>0};",
        "type Holder is Int{try {true} recover |Carrier:Type| {let value:Carrier=it;allowed(value)}};",
    ] {
        let dependencies = annotation_dependencies(&format!("type Carrier is Int; {source}"));
        assert!(
            dependencies.contains(&"pkg.owner.Carrier".into()),
            "{source}: {dependencies:?}"
        );
    }
}

#[test]
fn executable_local_functions_keep_free_paths_but_are_not_admitted_as_metadata() {
    use verum_ast::{ItemKind, source_signature::source_type_declaration};
    use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
    let seed = verum_fast_parser::Parser::new("module pkg.owner; type Holder is {value:Int};")
        .parse_module()
        .unwrap();
    let original = VbcCodegen::with_config(CodegenConfig::new("pkg.owner"))
        .compile_module(&seed)
        .unwrap();
    for (declaration, free_local) in [
        (
            "type Holder is Int{{fn local(value:Int)->Bool {allowed(value)} local(it)}};",
            false,
        ),
        (
            "type Holder is Int{{local(it);fn local(value:Int)->Bool {allowed(value)} local(it)}};",
            true,
        ),
        (
            "type Holder is {value:Bool={fn local(value:Int)->Bool {allowed(value)} local(73)}};",
            false,
        ),
    ] {
        let parsed = verum_fast_parser::Parser::new(declaration)
            .parse_module()
            .unwrap();
        let ItemKind::Type(declaration) = &parsed.items[0].kind else {
            panic!("nominal declaration");
        };
        // Exercise the executable visitor independently: it must not become a
        // signature-only traversal if metadata later admits this AST shape.
        let paths = crate::source_annotation_paths::free_paths(declaration);
        assert!(paths.contains(&"allowed".into()), "{paths:?}");
        assert_eq!(paths.contains(&"local".into()), free_local, "{paths:?}");
        assert!(!paths.contains(&"value".into()), "{paths:?}");
        // Current production explicitly refuses this unsupported carrier.
        // Both source capture and wire emission preserve that refusal.
        let error = source_type_declaration(declaration).unwrap_err();
        assert!(error.contains("nested function bodies"), "{error}");
        let mut module = original.clone();
        module
            .types
            .iter_mut()
            .find(|ty| {
                ty.source_declaration
                    .as_ref()
                    .is_some_and(|source| source.name.name == "Holder")
            })
            .unwrap()
            .source_declaration = Some(declaration.clone());
        let mut builder = ArchiveBuilder::new();
        let error = builder.add_module(&module.name, &module, &[]).unwrap_err();
        assert!(
            error.to_string().contains("nested function bodies"),
            "{error}"
        );
    }
    // Executable closures are supported and retain the actual helper edge.
    let dependencies =
        annotation_dependencies("type Holder is Int{(|value:Int| allowed(value))(it)};");
    assert!(
        dependencies.contains(&"pkg.owner.allowed".into()),
        "{dependencies:?}"
    );
    assert!(
        !dependencies.contains(&"pkg.owner.value".into()),
        "{dependencies:?}"
    );
}
