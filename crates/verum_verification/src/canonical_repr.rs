//! Canonical byte-level representation for AST nodes that participate
//! in the closure-hash incremental verification cache (#79 / #88 /
//! #89 hardening pass).
//!
//! ## The problem this module solves
//!
//! [`closure_cache::ClosureFingerprint`] is hashed from
//! `signature_payload`, `body_payload`, and `citations` byte slices.
//! Before this module those payloads were produced via
//! `format!("{:?}", thm.requires)` — Rust's [`std::fmt::Debug`] is
//! **explicitly not stable across compiler versions** (the compiler
//! reserves the right to change Debug output between releases), so
//! every `cargo update` of the rustc toolchain could silently
//! invalidate the entire on-disk cache, defeating the purpose of
//! `KERNEL_VERSION`-based invalidation.
//!
//! [`CanonicalRepr`] replaces that with an explicitly-stable
//! representation:
//!
//!   * Serialise via `serde_json` (which is API-stable).
//!   * Recursively sort every JSON object's keys (so AST struct field
//!     order at the source level is irrelevant — only field *names*
//!     and value types matter).
//!   * Emit canonical bytes (UTF-8 JSON with sorted keys) suitable
//!     for blake3 hashing.
//!
//! ## Schema-stability contract
//!
//! These canonical bytes are stable so long as:
//!
//!   1. The set of fields on each AST node is unchanged.
//!   2. Field *names* are unchanged (renaming a field is a schema
//!      break — bump `KERNEL_VERSION` or run `verum cache-closure
//!      clear`).
//!   3. Enum variant names are unchanged (variant rename is a schema
//!      break).
//!   4. The mapping from Rust `Vec<T>` → JSON array preserves order
//!      (this is intrinsic to serde_json).
//!
//! Adding a new optional field with a default is *not* a schema break
//! provided the existing serialiser still emits the same bytes when
//! the field has its default value.  Reordering fields in source is
//! never a schema break (we sort).
//!
//! ## Why not `bincode` or `postcard`?
//!
//! `serde_json` is already a workspace dependency and is the format
//! the rest of `closure_cache` uses for on-disk entries.  Sorted-keys
//! JSON is human-readable, which makes cache-debugging
//! (`cat target/.verum_cache/closure-hashes/X.json`) tractable.
//! Performance is not a bottleneck — fingerprint cost is dominated by
//! blake3, not the JSON round-trip.

use blake3::Hasher;
use serde::Serialize;
use serde_json::Value;
use verum_ast::decl::TheoremDecl;
use verum_common::Text;

// =============================================================================
// CanonicalRepr — the trait
// =============================================================================

/// Produce a deterministic byte-level representation for fingerprinting.
///
/// Default impl: serialise via serde JSON, recursively sort object
/// keys, emit UTF-8 bytes.  Override only if the default would
/// produce a non-canonical form (e.g. if `Serialize` for a node
/// includes timestamps or random IDs — which AST nodes don't).
pub trait CanonicalRepr {
    fn canonical_repr(&self) -> Vec<u8>;

    /// Convenience: blake3 hash of the canonical bytes, hex-encoded.
    fn canonical_blake3_hex(&self) -> Text {
        let bytes = self.canonical_repr();
        let mut h = Hasher::new();
        h.update(&bytes);
        let digest = h.finalize();
        let mut hex = String::with_capacity(64);
        for b in digest.as_bytes() {
            hex.push_str(&format!("{:02x}", b));
        }
        Text::from(hex)
    }
}

/// Blanket impl for any `Serialize` value.  Concrete AST nodes use
/// this transparently — no `impl CanonicalRepr for Expr {}` boilerplate
/// required.
impl<T: Serialize> CanonicalRepr for T {
    fn canonical_repr(&self) -> Vec<u8> {
        canonical_json_bytes(self)
    }
}

// =============================================================================
// canonical_json_bytes — recursive key-sort JSON encoder
// =============================================================================

/// Serialise `value` to canonical JSON bytes.  Object keys are
/// recursively sorted lexicographically; arrays preserve source
/// order.  This is the **single point of byte-level determinism**
/// for the cache fingerprint.
pub fn canonical_json_bytes<T: Serialize>(value: &T) -> Vec<u8> {
    // First convert to a Value tree so we can rewrite map keys.
    let v: Value = serde_json::to_value(value)
        .expect("Serialize impl must not fail on AST nodes");
    let canon = canonicalise(v);
    // serde_json's `to_writer` with a Value preserves the BTreeMap-
    // backed object order — which after `canonicalise` is sorted.
    serde_json::to_vec(&canon).expect("re-serialise sorted Value cannot fail")
}

/// Recursively rewrite every `Value::Object` so its keys are sorted
/// lexicographically.  Other variants (Null / Bool / Number / String /
/// Array) are passed through.
fn canonicalise(v: Value) -> Value {
    match v {
        Value::Object(map) => {
            // `serde_json::Map` is backed by a `BTreeMap` when the
            // `preserve_order` feature is **off** (the default), in
            // which case keys are already sorted.  But we don't rely
            // on that — we re-insert into a fresh BTreeMap to be
            // independent of the workspace's serde_json features.
            let mut sorted: std::collections::BTreeMap<String, Value> =
                std::collections::BTreeMap::new();
            for (k, val) in map {
                sorted.insert(k, canonicalise(val));
            }
            let mut out = serde_json::Map::with_capacity(sorted.len());
            for (k, val) in sorted {
                out.insert(k, val);
            }
            Value::Object(out)
        }
        Value::Array(arr) => Value::Array(arr.into_iter().map(canonicalise).collect()),
        leaf => leaf,
    }
}

// =============================================================================
// Theorem-level helpers — the three payload buckets used by the cache
// =============================================================================

/// Canonical bytes of a theorem's *signature*: name + requires +
/// ensures + proposition + return_type + generics + params.  Two
/// theorems with the same signature bytes are interchangeable from
/// the kernel's standpoint as far as obligation shape is concerned.
///
/// Excludes proof body (separate payload), attributes (separate
/// payload), and span (location is not part of meaning).
pub fn theorem_signature_bytes(thm: &TheoremDecl) -> Vec<u8> {
    // Use a small typed helper struct so the JSON shape is explicit
    // and stable.  Field names are part of the schema-stability
    // contract.
    #[derive(Serialize)]
    struct SignatureView<'a> {
        name: &'a str,
        generics: &'a verum_common::List<verum_ast::ty::GenericParam>,
        params: &'a verum_common::List<verum_ast::decl::FunctionParam>,
        return_type: &'a verum_common::Maybe<verum_ast::ty::Type>,
        requires: &'a verum_common::List<verum_ast::expr::Expr>,
        ensures: &'a verum_common::List<verum_ast::expr::Expr>,
        proposition: &'a verum_ast::expr::Expr,
        generic_where_clause: &'a verum_common::Maybe<verum_ast::ty::WhereClause>,
        meta_where_clause: &'a verum_common::Maybe<verum_ast::ty::WhereClause>,
    }
    let view = SignatureView {
        name: thm.name.name.as_str(),
        generics: &thm.generics,
        params: &thm.params,
        return_type: &thm.return_type,
        requires: &thm.requires,
        ensures: &thm.ensures,
        proposition: &thm.proposition,
        generic_where_clause: &thm.generic_where_clause,
        meta_where_clause: &thm.meta_where_clause,
    };
    canonical_json_bytes(&view)
}

/// Canonical bytes of a theorem's *proof body* (or the absence
/// thereof).  This is the ProofBody alone — span / attributes are
/// elsewhere.
pub fn theorem_body_bytes(thm: &TheoremDecl) -> Vec<u8> {
    canonical_json_bytes(&thm.proof)
}

/// Sorted, deduplicated list of `@framework("system", "citation")`
/// citation references reachable through this theorem's attributes.
/// Returns the canonical list of strings (one per citation, in the
/// form `"system:citation"`) — caller passes the result into
/// [`closure_cache::ClosureFingerprint::compute`].
pub fn theorem_citations(thm: &TheoremDecl) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for attr in thm.attributes.iter() {
        if !attr.is_named("framework") {
            continue;
        }
        // We don't inspect the AttributeArgs structure here — its
        // canonical bytes are what counts.  A canonical-bytes string
        // collision across two distinct framework attribute payloads
        // would already require a blake3 collision.
        let bytes = canonical_json_bytes(attr);
        // Use the canonical bytes as the citation token; the cache
        // fingerprint then sorts + dedups them.  (Hex-encoded so the
        // citation list is plain ASCII for cache-file readability.)
        let mut hex = String::with_capacity(64);
        for b in blake3::hash(&bytes).as_bytes() {
            hex.push_str(&format!("{:02x}", b));
        }
        out.push(format!("framework:{}", hex));
    }
    out.sort_unstable();
    out.dedup();
    out
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalise_sorts_object_keys() {
        let v: Value =
            serde_json::from_str(r#"{"z": 1, "a": 2, "m": {"y": 3, "b": 4}}"#).unwrap();
        let bytes = serde_json::to_vec(&canonicalise(v)).unwrap();
        let s = std::str::from_utf8(&bytes).unwrap();
        // Outer keys sorted: a, m, z.
        let pos_a = s.find("\"a\"").unwrap();
        let pos_m = s.find("\"m\"").unwrap();
        let pos_z = s.find("\"z\"").unwrap();
        assert!(pos_a < pos_m);
        assert!(pos_m < pos_z);
        // Inner keys sorted: b, y.
        let pos_b = s.find("\"b\"").unwrap();
        let pos_y = s.find("\"y\"").unwrap();
        assert!(pos_b < pos_y);
    }

    #[test]
    fn canonicalise_preserves_array_order() {
        let v: Value = serde_json::from_str(r#"[3, 1, 2]"#).unwrap();
        let bytes = serde_json::to_vec(&canonicalise(v)).unwrap();
        assert_eq!(bytes, b"[3,1,2]");
    }

    #[test]
    fn canonical_repr_blanket_impl_works_for_serde_value() {
        let v: Value = serde_json::from_str(r#"{"b": 2, "a": 1}"#).unwrap();
        let bytes = v.canonical_repr();
        // Sorted: `{"a":1,"b":2}` — stable byte sequence.
        assert_eq!(bytes, br#"{"a":1,"b":2}"#);
    }

    #[test]
    fn canonical_blake3_hex_is_64_hex_chars() {
        let v: Value = serde_json::from_str(r#"{"x": 1}"#).unwrap();
        let h = v.canonical_blake3_hex();
        assert_eq!(h.as_str().len(), 64);
        assert!(h.as_str().chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn canonical_blake3_is_deterministic_under_key_reorder() {
        let v1: Value = serde_json::from_str(r#"{"a": 1, "b": 2}"#).unwrap();
        let v2: Value = serde_json::from_str(r#"{"b": 2, "a": 1}"#).unwrap();
        // Same canonical hash even though the source key order differs.
        assert_eq!(v1.canonical_blake3_hex(), v2.canonical_blake3_hex());
    }

    #[test]
    fn canonical_blake3_changes_when_value_changes() {
        let v1: Value = serde_json::from_str(r#"{"x": 1}"#).unwrap();
        let v2: Value = serde_json::from_str(r#"{"x": 2}"#).unwrap();
        assert_ne!(v1.canonical_blake3_hex(), v2.canonical_blake3_hex());
    }

    #[test]
    fn canonical_blake3_distinguishes_object_from_array() {
        let obj: Value = serde_json::from_str(r#"{"0": 1}"#).unwrap();
        let arr: Value = serde_json::from_str(r#"[1]"#).unwrap();
        assert_ne!(obj.canonical_blake3_hex(), arr.canonical_blake3_hex());
    }

    #[test]
    fn nested_objects_recursively_sorted() {
        let v: Value = serde_json::from_str(
            r#"{"outer": {"z": 1, "a": {"q": 1, "b": 2}}}"#,
        )
        .unwrap();
        let bytes = serde_json::to_vec(&canonicalise(v)).unwrap();
        let s = std::str::from_utf8(&bytes).unwrap();
        // Innermost keys sorted: b, q.
        let pos_b = s.find("\"b\"").unwrap();
        let pos_q = s.find("\"q\"").unwrap();
        assert!(pos_b < pos_q);
    }

    // ----- Property: stability across repeated canonicalisation -----

    #[test]
    fn canonicalise_is_idempotent() {
        let v: Value = serde_json::from_str(
            r#"{"z": 1, "a": [{"y": 1, "b": 2}, {"d": 3}]}"#,
        )
        .unwrap();
        let once = canonicalise(v.clone());
        let twice = canonicalise(once.clone());
        // Bytes-equivalent.
        let b1 = serde_json::to_vec(&once).unwrap();
        let b2 = serde_json::to_vec(&twice).unwrap();
        assert_eq!(b1, b2);
    }
}
