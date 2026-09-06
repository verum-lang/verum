//! A94 — a PRIVATE stdlib constant must not be published into a user
//! program's namespace.
//!
//! `register_stdlib_consts_from_metadata` publishes every `is_const`
//! descriptor in `CoreMetadata` under its BARE name so that user code
//! writing `let s = SSO_CAPACITY;` resolves through `env.lookup`.  It
//! had nothing to filter on, so `core/hash/crypto/sha512.vr`'s
//! `const K: [UInt64; 80]` — declared without `public` — answered to
//! `K` in a program that mounts nothing, together with 301 other
//! private-only names measured across `core/`.
//!
//! The two assertions below are each other's control: the same
//! metadata, the same loop, one descriptor public and one not.  A
//! filter that dropped everything would fail the first; the pre-fix
//! behaviour fails the second.

use std::sync::Arc;
use verum_common::{List, Maybe, Text};
use verum_types::TypeChecker;
use verum_types::core_metadata::{CoreMetadata, FunctionDescriptor};

fn const_descriptor(name: &str, is_public: bool) -> FunctionDescriptor {
    FunctionDescriptor {
        is_public,
        name: Text::from(name),
        module_path: Text::from("test.module"),
        origin_module_path: Maybe::None,
        generic_params: List::new(),
        params: List::new(),
        return_type: Text::from("Int"),
        contexts: List::new(),
        is_async: false,
        is_unsafe: false,
        intrinsic_id: Maybe::None,
        parent_type: Maybe::None,
        impl_generic_names: List::new(),
        is_const: true,
        decl_span: Maybe::None,
    }
}

fn checker_with_two_consts() -> TypeChecker {
    let mut metadata = CoreMetadata::default();
    metadata
        .functions
        .insert(Text::from("PUBLIC_CONST"), const_descriptor("PUBLIC_CONST", true));
    metadata
        .functions
        .insert(Text::from("PRIVATE_CONST"), const_descriptor("PRIVATE_CONST", false));
    TypeChecker::new_with_core_eager(Arc::new(metadata))
}

#[test]
fn a_public_stdlib_const_is_published_under_its_bare_name() {
    let checker = checker_with_two_consts();
    assert!(
        checker
            .lookup_qualified_name_for_testing("PUBLIC_CONST")
            .is_some(),
        "a public const must stay reachable — without this control a filter \
         that dropped every constant would look like a fix"
    );
}

#[test]
fn a_private_stdlib_const_is_not_published_at_all() {
    let checker = checker_with_two_consts();
    assert!(
        checker
            .lookup_qualified_name_for_testing("PRIVATE_CONST")
            .is_none(),
        "a const declared without `public` must not answer to its bare name \
         in a program that never mounted it"
    );
}

#[test]
fn a_descriptor_baked_before_the_field_deserialises_as_public() {
    // Wire compat: the stale-archive default has to be the permissive
    // one.  `false` would hide the whole stdlib from every program
    // compiled against an older `.vbca` — a silent, total failure.
    let fd = const_descriptor("SOME_CONST", true);
    let mut json: serde_json::Value =
        serde_json::to_value(&fd).expect("serialize");
    json.as_object_mut()
        .expect("descriptor serialises as an object")
        .remove("is_public")
        .expect("the field is present before removal");
    let back: FunctionDescriptor =
        serde_json::from_value(json).expect("deserialize without is_public");
    assert!(
        back.is_public,
        "a descriptor with no `is_public` field must read as PUBLIC"
    );
}
