use super::*;
use crate::types::{TypeParamId, TypeRef};

fn codegen() -> VbcCodegen {
    let ast = verum_fast_parser::Parser::new(
        "type Payload is { value: Int }; type Owner<A, B> is { a: A, b: B };",
    )
    .parse_module()
    .expect("parse");
    let mut codegen = VbcCodegen::with_config(super::super::CodegenConfig::new("callable_owner"));
    codegen.compile_module(&ast).expect("compile");
    codegen
}

#[test]
fn callable_owner_requires_the_whole_declared_arity() {
    let codegen = codegen();
    for owner in ["Owner", "Owner<Payload>", "Owner<Payload, Bool, Int>"] {
        assert!(
            codegen.callable_owner_substitution(owner).is_none(),
            "{owner}"
        );
    }
}

#[test]
fn callable_owner_rejects_unknown_types_and_unconsumed_syntax() {
    let codegen = codegen();
    for owner in ["Owner<Payload, Missing>", "Owner<Payload, Bool> trailing"] {
        assert!(
            codegen.callable_owner_substitution(owner).is_none(),
            "{owner}"
        );
    }
}

#[test]
fn callable_owner_structural_arguments_keep_declared_slot_ids() {
    let codegen = codegen();
    let subst = codegen
        .callable_owner_substitution("Owner<(Payload, Bool), fn(Payload, Bool) -> Payload>")
        .expect("complete owner");
    let payload = TypeRef::Concrete(codegen.nominal_type_id("Payload").expect("payload"));
    let boolean = TypeRef::Concrete(crate::types::TypeId::BOOL);
    assert_eq!(
        subst.get(TypeParamId(0)),
        Some(&TypeRef::Tuple(vec![payload.clone(), boolean.clone()]))
    );
    assert_eq!(
        subst.get(TypeParamId(1)),
        Some(&TypeRef::Function {
            params: vec![payload.clone(), boolean],
            return_type: Box::new(payload),
            contexts: Default::default()
        })
    );
}
