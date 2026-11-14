//! Complete selected declarations for the actual scalar Hash/default contract.
//! This is a focused dependency closure, not whole-standard-library admission.
use verum_common::Text;

pub fn canonical_hash_source() -> Text {
    let protocols = include_str!("../../../../core/base/protocols.vr");
    let start = protocols
        .find("public type Hash is protocol")
        .expect("Hash source");
    let end = protocols
        .find("// Clone Protocol")
        .expect("following protocol");
    let mut source = Text::from(&protocols[start..end]);
    let primitives = include_str!("../../../../core/base/primitives.vr");
    for declaration in ["implement Hash for Int {", "implement Hash for () {"] {
        let start = primitives.find(declaration).expect("primitive Hash source");
        let end = start + primitives[start..].find("\n}\n").expect("impl end") + 3;
        source.push_str(&primitives[start..end]);
    }
    source.push_str(" implement Int { ");
    for signature in [
        "public fn wrapping_mul(self, rhs: Int)",
        "public fn rotate_left(self, n: Int)",
    ] {
        let start = primitives
            .find(signature)
            .expect("hasher primitive dependency");
        let end = start + primitives[start..].find("\n    }").expect("method end") + 6;
        source.push_str(&primitives[start..end]);
    }
    source.push_str(" }");

    // DefaultHasher.write calls the actual slice extension. Keep the whole
    // selected method and its registered primitive, not a test replacement.
    let slices = include_str!("../../../../core/collections/slice.vr");
    let start = slices
        .find("implement<T> [T] {")
        .expect("slice implementation");
    let end = start
        + slices[start..]
            .find("    /// Check if slice is empty.")
            .expect("next method");
    source.push_str(&slices[start..end]);
    source.push_str(" } ");
    let memory = include_str!("../../../../core/intrinsics/memory.vr");
    let start = memory
        .find("public fn slice_len<T>(")
        .expect("registered slice primitive");
    let end = start + memory[start..].find('\n').expect("intrinsic end");
    source.push_str(&memory[start..end]);
    source
}
