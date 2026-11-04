#![cfg(feature = "codegen")]
//! Unrelated registrations cannot alter a selected glob import.
use std::collections::HashMap;
use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen, context::FunctionInfo};
use verum_vbc::module::FunctionId;
use verum_vbc::types::{TypeId, TypeRef};

fn imported(unrelated: usize, reverse: bool) {
    let mut entries = vec![
        ("core.selected.picked".to_string(), 21),
        ("core.selected.z.extra".to_string(), 22),
        ("core.selectedish.foreign".to_string(), 23),
        ("core.selected.shadow".to_string(), 24),
        ("shadow".to_string(), 25),
    ];
    for n in 0..unrelated {
        entries.push((format!("core.unrelated.group{n}.other{n}"), 100 + n as u32));
    }
    if reverse {
        entries.reverse();
    }
    let registry: HashMap<_, _> = entries
        .into_iter()
        .map(|(name, id)| {
            (
                name,
                FunctionInfo {
                    id: FunctionId(id),
                    return_type: Some(TypeRef::Concrete(TypeId::INT)),
                    ..Default::default()
                },
            )
        })
        .collect();
    let mut compiler = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    compiler.import_functions(&registry);
    let ast = Parser::new("module consumer; mount core.selected.*; fn probe()->Int { picked() }")
        .parse_module()
        .unwrap();
    let start = std::time::Instant::now();
    compiler.collect_unit_declarations(&[&ast]).unwrap();
    let elapsed = start.elapsed();
    let exports = compiler.export_functions();
    assert_eq!(exports.get("picked").unwrap().id, FunctionId(21));
    assert_eq!(exports.get("extra").unwrap().id, FunctionId(22));
    assert_eq!(exports.get("shadow").unwrap().id, FunctionId(25));
    assert!(!exports.contains_key("foreign"));
    eprintln!(
        "glob: unrelated={unrelated} reverse={reverse} elapsed_us={}",
        elapsed.as_micros()
    );
}
#[test]
fn selected_exports_are_stable_under_foreign_growth_and_registration_order() {
    for size in [0, 1000, 20000] {
        for reverse in [false, true] {
            imported(size, reverse);
        }
    }
}
