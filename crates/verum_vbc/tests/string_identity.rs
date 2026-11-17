//! Empty metadata and real source spellings occupy disjoint string identities.
#![cfg(feature = "codegen")]

use verum_common::Text;
use verum_fast_parser::Parser;
use verum_vbc::{
    VbcModule,
    codegen::{CodegenConfig, CodegenContext, VbcCodegen},
    deserialize::{deserialize_module, deserialize_module_validated},
    interpreter::Interpreter,
    module::StringTable,
    serialize::serialize_module,
    types::{StringId, TypeDescriptor, TypeRef},
};

fn compile(owner: &str, text: &str) -> VbcModule {
    VbcCodegen::with_config(CodegenConfig::new(owner))
        .compile_module(&Parser::new(text).parse_module().unwrap())
        .unwrap()
}

fn wire(module: &VbcModule) -> VbcModule {
    deserialize_module_validated(&serialize_module(module).unwrap()).unwrap()
}

fn descriptor<'a>(module: &'a VbcModule, name: &str) -> &'a TypeDescriptor {
    module
        .types
        .iter()
        .find(|ty| module.get_string(ty.name) == Some(name))
        .unwrap_or_else(|| panic!("missing type {name}"))
}

fn check_fields(module: &VbcModule) {
    assert_eq!(
        module.get_string(StringId(module.header.module_name_offset)),
        Some(module.name.as_str())
    );
    let leaf = descriptor(module, "Leaf");
    let wrapper = descriptor(module, "Wrapper");
    assert_eq!(wrapper.fields[0].type_ref, TypeRef::Concrete(leaf.id));
    for field in [
        &wrapper.fields[0],
        &descriptor(module, "Choice").variants[0].fields[0],
    ] {
        assert_ne!(field.type_name, StringId::EMPTY);
        assert_eq!(module.get_string(field.type_name), Some("Leaf"));
        assert_eq!(field.refinement_src, StringId::EMPTY);
        assert_eq!(field.refinement_binding, StringId::EMPTY);
        assert_eq!(module.get_string(field.refinement_src), Some(""));
    }
    assert_eq!(module.get_string(leaf.fields[0].name), Some("value"));
    assert_eq!(module.get_string(StringId::EMPTY), Some(""));
}

#[test]
fn codegen_preserves_the_empty_identity_across_reset() {
    let mut context = CodegenContext::new();
    for name in ["Leaf", "different"] {
        assert_eq!(context.intern_string_raw(""), StringId::EMPTY.0);
        let real = context.intern_string_raw(name);
        assert_ne!(real, StringId::EMPTY.0);
        assert_eq!(context.strings[real as usize], name);
        context.reset();
    }
}

#[test]
fn string_table_constructors_reserve_empty_and_serde_preserves_offsets() {
    for mut table in [StringTable::new(), StringTable::default()] {
        assert_eq!(table.get(StringId::EMPTY), Some(""));
        assert_eq!(table.intern(""), StringId::EMPTY);
        let first = table.intern("Leaf");
        let unicode = table.intern("λ");
        let reordered = table.intern("Before");
        assert_ne!(first, StringId::EMPTY);
        assert_eq!(table.intern("Leaf"), first);
        let decoded: StringTable =
            serde_json::from_value(serde_json::to_value(&table).unwrap()).unwrap();
        assert_eq!(decoded.get(StringId::EMPTY), Some(""));
        assert_eq!(decoded.get(first), Some("Leaf"));
        assert_eq!(decoded.get(unicode), Some("λ"));
        assert_eq!(decoded.get(reordered), Some("Before"));
        assert_eq!(decoded.serialized_size(), table.serialized_size());
    }
}

#[test]
fn first_source_name_and_module_name_survive_source_wire_and_import() {
    // A spelling that is also the module name previously acquired wire ID 0,
    // independently of the source-index collision at the first declaration.
    for owner in ["alpha", "Leaf"] {
        let source: Text = format!(
            "module {owner}; type Leaf is {{ value: Int }}; \
             public type Wrapper is {{ nested: Leaf }}; \
             public type Choice is Present {{ leaf: Leaf }} | Missing; \
             fn probe() -> Int {{ let item = Wrapper {{ nested: Leaf {{ value: 41 }} }}; item.nested.value + 1 }}"
        ).into();
        let produced = compile(owner, &source);
        for dependency in [produced.clone(), wire(&produced)] {
            check_fields(&dependency);
            let entry = dependency
                .find_function_by_name(&format!("{owner}.probe"))
                .expect("exact declaring module owns the runtime entry");
            let result = Interpreter::try_new_validated(dependency.clone().into())
                .unwrap()
                .execute_function(entry)
                .unwrap();
            assert_eq!(result.try_as_i64(), Some(42));

            let consumer = Parser::new(&format!(
                "module consumer; mount {owner}.{{Wrapper, Choice}}; \
                 fn accept(item: Wrapper, choice: Choice) -> Int {{ 7 }}"
            ))
            .parse_module()
            .unwrap();
            let mut codegen = VbcCodegen::with_config(CodegenConfig::new("consumer"));
            for number in 0..400 {
                codegen
                    .ctx_mut()
                    .intern_string_raw(&format!("foreign_{number}"));
            }
            codegen
                .import_bootstrap_nominal_dependencies(&[&consumer], &[&dependency])
                .unwrap();
            codegen.collect_unit_declarations(&[&consumer]).unwrap();
            let imported = codegen.compile_function_bodies(&consumer).unwrap();
            check_fields(&imported);
            check_fields(&wire(&imported));
        }
    }
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn reject_table(bytes: &[u8]) {
    let error = deserialize_module(bytes).unwrap_err();
    assert!(
        matches!(&error, verum_vbc::VbcError::Deserialization(message)
        if message.contains("string table")),
        "{error}"
    );
}

#[test]
fn wire_rejects_missing_sentinel_in_current_and_old_archives() {
    let original = serialize_module(&VbcModule::new("name".into())).unwrap();
    let start = u32_at(&original, 0x28);
    let size = u32_at(&original, 0x2c);
    for minor in [verum_vbc::format::VERSION_MINOR, 61] {
        let mut bytes = original.clone();
        bytes[6..8].copy_from_slice(&minor.to_le_bytes());
        // Exclude the empty entry without relocating any remaining section.
        bytes[0x28..0x2c].copy_from_slice(&(start + 4).to_le_bytes());
        bytes[0x2c..0x30].copy_from_slice(&(size - 4).to_le_bytes());
        bytes[0x0c..0x10].copy_from_slice(&0u32.to_le_bytes());
        reject_table(&bytes);
    }
    let mut empty = original;
    empty[0x2c..0x30].copy_from_slice(&0u32.to_le_bytes());
    reject_table(&empty);
}

#[test]
fn wire_rejects_duplicate_strings_instead_of_reindexing_them() {
    let mut module = VbcModule::new("name".into());
    let duplicate = module.intern_string("same");
    let mut bytes = serialize_module(&module).unwrap();
    let start = u32_at(&bytes, 0x28) as usize;
    let entry = start + duplicate.0 as usize + 4;
    bytes[entry..entry + 4].copy_from_slice(b"name");
    reject_table(&bytes);
}

#[test]
fn serde_rejects_ambiguous_ids_and_inconsistent_extents() {
    for value in [
        serde_json::json!({"index": {"name": 0}, "next_offset": 8}),
        serde_json::json!({"index": {"": 0, "name": 0}, "next_offset": 12}),
        serde_json::json!({"index": {"": 0, "name": 9}, "next_offset": 17}),
        serde_json::json!({"index": {"": 0, "name": 4}, "next_offset": 11}),
    ] {
        let error = serde_json::from_value::<StringTable>(value).unwrap_err();
        assert!(error.to_string().contains("string table"), "{error}");
    }
}
