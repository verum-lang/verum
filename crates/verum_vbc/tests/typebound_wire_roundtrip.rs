//! TYPEBOUND-CARRY (v2.11, T0701) wire pin: `TypeParamDescriptor.
//! type_bounds` must survive serialize→deserialize byte-for-byte.
//!
//! The metadata sidecar is built from `archive.load_module` — a wire
//! round-trip — so a codec that drops this field silently severs the
//! fn-bound linkage (`F: fn(I.Item) -> B`) that impl-level
//! existentials like `B` are extracted from (measured pre-v2.11:
//! reader saw bounds_parsed=0 with the writer landed, and
//! `m.next()` judged `Some(_)` E404 on a fully concrete receiver).

use verum_vbc::TypeRef;

#[test]
fn type_bounds_survive_wire_roundtrip() {
    let mut module = verum_vbc::module::VbcModule::new("tb_pin".to_string());

    let name_f = module.intern_string("F");
    let name_fn = module.intern_string("adapter_fn");
    let bound = TypeRef::Function {
        params: vec![TypeRef::Generic(verum_vbc::types::TypeParamId(0))],
        return_type: Box::new(TypeRef::Generic(verum_vbc::types::TypeParamId(2))),
        contexts: smallvec::SmallVec::new(),
    };
    let mut fd = verum_vbc::module::FunctionDescriptor::new(name_fn);
    fd.type_params.push(verum_vbc::types::TypeParamDescriptor {
        name: name_f,
        id: verum_vbc::types::TypeParamId(1),
        bounds: smallvec::SmallVec::new(),
        default: None,
        variance: verum_vbc::types::Variance::Invariant,
        type_bounds: {
            let mut v: smallvec::SmallVec<[TypeRef; 1]> = smallvec::SmallVec::new();
            v.push(bound.clone());
            v
        },
    });
    module.add_function(fd);

    let bytes = verum_vbc::serialize::serialize_module(&module)
        .expect("serialize");
    let decoded = verum_vbc::deserialize::deserialize_module(&bytes)
        .expect("deserialize");

    let dfd = decoded
        .functions
        .iter()
        .find(|f| decoded.strings.get(f.name) == Some("adapter_fn"))
        .expect("descriptor present");
    let tp = dfd
        .type_params
        .iter()
        .find(|tp| decoded.strings.get(tp.name) == Some("F"))
        .expect("type param F present");
    assert_eq!(
        tp.type_bounds.len(),
        1,
        "fn-bound dropped at the wire — sidecar linkage severed"
    );
    assert_eq!(
        tp.type_bounds[0], bound,
        "fn-bound structure mutated in transit"
    );
}
