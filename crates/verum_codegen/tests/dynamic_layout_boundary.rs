//! Semantic object constraints cannot invent the hidden value's allocation ABI.
use verum_codegen::llvm::types::TypeLowering;
use verum_common::{Heap, List};
use verum_llvm::context::Context;
use verum_vbc::types::{
    CbgrTier, DynamicProtocolType, Mutability, ProtocolApplication, ProtocolId, TypeRef,
};

#[test]
fn owning_dynamic_layout_requires_an_actual_descriptor() {
    let context = Context::create();
    let lowering = TypeLowering::new(&context);
    let object = TypeRef::Dynamic(DynamicProtocolType {
        bounds: List::from_iter([ProtocolApplication {
            protocol: ProtocolId(2048),
            args: Default::default(),
        }]),
        bindings: List::new(),
    });
    let error = lowering.lower_type_ref(&object).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("actual owning runtime descriptor"),
        "{error}"
    );
    for tier in [CbgrTier::Tier0, CbgrTier::Tier1, CbgrTier::Tier2] {
        let borrowed = TypeRef::Reference {
            inner: Heap::new(object.clone()),
            mutability: Mutability::Immutable,
            tier,
        };
        assert!(
            lowering
                .lower_type_ref(&borrowed)
                .unwrap()
                .is_pointer_type()
        );
    }
}
