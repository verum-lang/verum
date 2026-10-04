use super::*;
use verum_vbc::module::{FunctionDescriptor, FunctionId};
use verum_vbc::types::{TypeDescriptor, TypeId};

fn owner(name: &str) -> (String, VbcModule) {
    let mut module = VbcModule::new(name.to_owned());
    for method in ["drop", "clone"] {
        let id = FunctionId(module.functions.len() as u32);
        let function = FunctionDescriptor {
            id,
            name: module.strings.intern(&format!("Token.{method}")),
            origin_module: Some(module.strings.intern(&format!("{name}.memory"))),
            instructions: Some(vec![Instruction::RetV]),
            ..Default::default()
        };
        module.functions.push(function);
    }
    (name.to_owned(), module)
}
fn consumer(target: &str) -> (String, VbcModule) {
    let mut module = VbcModule::new("consumer".to_owned());
    let first = verum_vbc::module::XMOD_CALL_ID_BAND_BASE;
    for (offset, method) in ["drop", "clone"].iter().enumerate() {
        let name = module.strings.intern(&format!("{target}.Token.{method}"));
        module
            .external_function_names
            .push((FunctionId(first + offset as u32), name));
    }
    module.types.push(TypeDescriptor {
        id: TypeId(19),
        name: module.strings.intern("Token"),
        drop_fn: Some(first),
        clone_fn: Some(first + 1),
        ..Default::default()
    });
    ("consumer".to_owned(), module)
}
#[test]
fn named_external_glue_is_kept_by_exact_declaring_file_identity() {
    for reversed in [false, true] {
        let mut decoded = vec![owner("alpha"), owner("beta"), consumer("alpha.memory")];
        if reversed {
            decoded.reverse();
        }
        let keep =
            compute_merge_keep_sets(&decoded, &HashMap::new(), &HashSet::new(), &HashSet::new());
        assert_eq!(keep["alpha"], HashSet::from([0, 1]));
        assert!(
            keep["beta"].is_empty(),
            "same-leaf sibling must not satisfy alpha's glue"
        );
    }
}
#[test]
fn missing_qualified_glue_does_not_borrow_an_ancestor_or_sibling() {
    let decoded = vec![
        owner("alpha"),
        owner("beta"),
        consumer("alpha.child.memory"),
    ];
    let keep = compute_merge_keep_sets(&decoded, &HashMap::new(), &HashSet::new(), &HashSet::new());
    assert!(keep["alpha"].is_empty());
    assert!(keep["beta"].is_empty());
}
