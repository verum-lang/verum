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
        let graph = SymbolGraph::build(&archive);
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
        assert_exact_beta_closure(&SymbolGraph::build(&archive), &archive);
    }
}

#[test]
fn encoded_sidecar_roundtrip_keeps_the_declaring_owner_and_edges() {
    let archive = sibling_archive(false);
    let encoded = SymbolGraph::scan_and_encode(&archive);
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
        let graph = SymbolGraph::build(&archive);
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
    let graph = SymbolGraph::build(&archive);
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
    let graph = SymbolGraph::build(&archive);
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
