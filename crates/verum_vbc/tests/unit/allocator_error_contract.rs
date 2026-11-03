use super::*;
use crate::{
    interpreter::heap,
    module::{FunctionDescriptor, VbcModule},
    types::{FieldDescriptor, TypeDescriptor, TypeKind, TypeRef, VariantDescriptor, VariantKind},
};
use std::sync::Arc;

#[test]
fn allocation_failure_uses_the_intrinsic_result_owner_and_declared_tag() {
    let mut module = VbcModule::new("allocation_client".into());
    let name = module.intern_string("AllocError");
    let actual = module.intern_string("StorageFailure");
    let intrinsic = module.intern_string("cbgr_alloc");
    let variant = module.intern_string("InvalidSize");
    let field = module.intern_string("size");
    let owner = TypeId(3017);
    module.types.push(TypeDescriptor {
        name,
        id: TypeId(3016),
        ..Default::default()
    });
    module.types.push(TypeDescriptor {
        name: actual,
        id: owner,
        kind: TypeKind::Sum,
        variants: smallvec::smallvec![VariantDescriptor {
            name: variant,
            tag: 7,
            kind: VariantKind::Record,
            fields: smallvec::smallvec![FieldDescriptor {
                name: field,
                type_ref: TypeRef::Concrete(TypeId::INT),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    });
    module.add_function(FunctionDescriptor {
        intrinsic_name: Some(intrinsic),
        return_type: TypeRef::Instantiated {
            base: TypeId::RESULT,
            args: vec![TypeRef::Concrete(TypeId::INT), TypeRef::Concrete(owner)],
        },
        ..Default::default()
    });
    let mut state = InterpreterState::new(Arc::new(module));
    let operation = AllocatingOperation::Intrinsic("cbgr_alloc");
    let result = make_alloc_err(&mut state, operation, AllocationFailure::InvalidSize, -8).unwrap();
    // Both objects were allocated by this call and remain live in state's heap.
    unsafe {
        let outer = result.as_ptr::<u8>();
        assert_eq!(
            (*outer.cast::<heap::ObjectHeader>()).type_id,
            TypeId::RESULT
        );
        assert_eq!(
            heap::variant_tag(outer),
            verum_common::well_known_types::result_error_tag()
        );
        let payload = (*heap::variant_payload_ptr(outer, 0)).as_ptr::<u8>();
        assert_eq!((*payload.cast::<heap::ObjectHeader>()).type_id, owner);
        assert_eq!(heap::variant_tag(payload), 7);
        assert_eq!((*heap::variant_payload_ptr(payload, 0)).as_i64(), -8);
    }
    assert!(make_alloc_err(&mut state, operation, AllocationFailure::OutOfMemory, 64).is_err());
}
