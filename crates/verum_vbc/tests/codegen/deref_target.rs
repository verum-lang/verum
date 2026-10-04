#[test]
fn deref_target_renders_carried_reference_slice_tuple_and_array_arguments() {
    use super::VbcCodegen;
    use crate::codegen::CodegenConfig;
    use crate::types::{CbgrTier, Mutability, TypeId, TypeParamId, TypeRef};
    use verum_common::{Maybe, Text};
    let ast = verum_fast_parser::Parser::new(
        r#"
type Deref is protocol { type Target; fn deref(&self) -> &Self.Target; };
type Guard<T> is { value: T };
implement<T> Deref for Guard<T> { type Target = T; fn deref(&self) -> &T { &self.value } }
"#,
    )
    .parse_module()
    .expect("parse");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("deref_target_render"));
    codegen.compile_module(&ast).expect("compile");
    let id = codegen.type_name_to_id["Guard"];
    let descriptor = codegen
        .types
        .iter_mut()
        .find(|t| t.id == id)
        .expect("Guard");
    // Source collection currently erases references nested in associated
    // generic arguments. Pin the complete archive descriptor independently.
    descriptor.protocols[0].associated_types[0].1 = TypeRef::Instantiated {
        base: TypeId::LIST,
        args: vec![TypeRef::Tuple(vec![
            TypeRef::Reference {
                inner: Box::new(TypeRef::Generic(TypeParamId(0))),
                mutability: Mutability::Mutable,
                tier: CbgrTier::Tier0,
            },
            TypeRef::Slice(Box::new(TypeRef::Generic(TypeParamId(0)))),
            TypeRef::Array {
                element: Box::new(TypeRef::Generic(TypeParamId(0))),
                length: 3,
            },
        ])],
    };
    assert_eq!(
        codegen.user_deref_target_type_name("Guard<Int>"),
        Maybe::Some(Text::from("List<(&mut Int, [Int], [Int; 3])>"))
    );
}

