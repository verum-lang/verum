use super::apply_function_signatures;
use crate::codegen::{CodegenConfig, VbcCodegen};
use std::cell::Cell;
use verum_common::{List, Map, Text};
use verum_fast_parser::Parser;

#[test]
fn signature_batch_matches_ordered_replay_with_one_live_registry_visit() {
    let source = Parser::new(
        "module replay; fn first()->Int { 7 } fn second()->Bool { true } fn untouched()->Text { \"x\" }",
    ).parse_module().expect("source");
    let mut cg = VbcCodegen::with_config(CodegenConfig::new("replay"));
    cg.compile_module(&source).expect("source signatures");
    let exports = cg.export_functions();
    let declarations = [
        exports["replay.first"].clone(),
        exports["replay.second"].clone(),
        exports["replay.untouched"].clone(),
    ];
    let mut original = Map::new();
    for index in 0..4096 {
        original.insert(
            Text::from(format!("alias_{index}")),
            declarations[index % 3].clone(),
        );
    }
    let mut updates = List::new();
    for index in 0..512 {
        let declaration = &declarations[index % 2];
        let result = declaration.return_type.clone().expect("declared return");
        // Optional archive yield facts exercise both setting and clearing the
        // carrier, independently of source generator inference.
        let yielded = (index % 4 < 2).then(|| result.clone());
        updates.push((declaration.id, (result, yielded)));
    }
    let mut expected = original.clone();
    let mut old_visits = 0;
    // The previous production algorithm is the reference, including repeated
    // IDs and a later None replacing an earlier Some yield fact.
    for (id, (result, yielded)) in &updates {
        for info in expected.values_mut() {
            old_visits += 1;
            if info.id == *id {
                info.return_type = Some(result.clone());
                info.yield_type = yielded.clone();
            }
        }
    }
    let mut signatures = Map::new();
    for (id, signature) in updates.iter() {
        signatures.insert(*id, signature.clone());
    }
    let visits = Cell::new(0);
    // Count the actual production helper's iterator, not a copied batch model.
    apply_function_signatures(
        original
            .values_mut()
            .inspect(|_| visits.set(visits.get() + 1)),
        &signatures,
    );
    for (alias, before) in &expected {
        let after = &original[alias];
        assert_eq!(after.id, before.id, "{alias}");
        assert_eq!(after.return_type, before.return_type, "{alias}");
        assert_eq!(after.yield_type, before.yield_type, "{alias}");
    }
    assert_eq!(old_visits, original.len() * updates.len());
    assert_eq!(visits.get(), original.len());
    eprintln!(
        "signature replay: sites={}, aliases={}, old visits={old_visits}, batch visits={}",
        updates.len(),
        original.len(),
        visits.get()
    );

    let empty_visits = Cell::new(0);
    apply_function_signatures(
        original
            .values_mut()
            .inspect(|_| empty_visits.set(empty_visits.get() + 1)),
        &Map::new(),
    );
    assert_eq!(
        empty_visits.get(),
        0,
        "no selected sites must not scan the registry"
    );
}
