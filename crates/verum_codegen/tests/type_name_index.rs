//! The native phase index preserves descriptor identity and exact lookup order.
use verum_codegen::llvm::context::TypeNameIndex;
use verum_vbc::{
    module::VbcModule,
    types::{StringId, TypeDescriptor, TypeId},
};

fn push(module: &mut VbcModule, id: u32, name: &str) {
    let name = module.strings.intern(name);
    module.types.push(TypeDescriptor {
        id: TypeId(id),
        name,
        ..Default::default()
    });
}

#[test]
fn exact_names_preserve_first_ids_sparse_ids_and_first_named_descriptors() {
    let mut module = VbcModule::new("index".into());
    push(&mut module, 4001, "alpha.Cell");
    push(&mut module, 1, "beta.Cell");
    push(&mut module, 4001, "hidden.DuplicateId");
    push(&mut module, 8900, "alpha.Cell");
    module.types.push(TypeDescriptor {
        id: TypeId(9900),
        name: StringId(u32::MAX),
        ..Default::default()
    });
    push(&mut module, 9900, "hidden.InvalidFirstName");
    let index = TypeNameIndex::build(&module);
    for (name, expected) in [
        ("alpha.Cell", Some(0)),
        ("beta.Cell", Some(1)),
        ("hidden.DuplicateId", None),
        ("hidden.InvalidFirstName", None),
        ("Cell", None),
        ("ALPHA.Cell", None),
        ("missing.Cell", None),
    ] {
        assert_eq!(index.find(name), expected, "{name}");
        let previous = module
            .types
            .iter()
            .position(|descriptor| module.get_type_name(descriptor.id).as_deref() == Some(name));
        assert_eq!(index.find(name), previous, "legacy selection: {name}");
    }
}

#[test]
fn a_new_lowering_input_rebuilds_its_own_descriptor_order() {
    let mut first = VbcModule::new("first".into());
    push(&mut first, 4001, "alpha.Cell");
    push(&mut first, 7, "beta.Cell");
    let first_index = TypeNameIndex::build(&first);
    let mut second = VbcModule::new("second".into());
    push(&mut second, 7, "beta.Cell");
    push(&mut second, 4001, "alpha.Cell");
    let second_index = TypeNameIndex::build(&second);
    assert_eq!(first_index.find("alpha.Cell"), Some(0));
    assert_eq!(second_index.find("alpha.Cell"), Some(1));
    assert_eq!(
        second.types[second_index.find("alpha.Cell").unwrap()].id,
        TypeId(4001)
    );
    assert_eq!(first_index.find("beta.Cell"), Some(1));
    assert_eq!(second_index.find("beta.Cell"), Some(0));
}
