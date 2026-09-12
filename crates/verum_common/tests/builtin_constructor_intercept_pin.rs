//! Pins the membership of `WellKnownType::has_builtin_constructor_intercept`.
//!
//! A member of this set is a type whose heap REPRESENTATION the
//! interpreter substitutes for the stdlib's own. That substitution has
//! two halves — where the object is BUILT and where it is READ — and
//! both halves must be present for the same type, or one of them reads
//! a shape it was not written for.
//!
//! `Channel` is the case that taught it. `TypeId::CHANNEL` is stamped by
//! NAME, so `core/async/channel.vr`'s eight-field `Channel<T>` carried
//! the well-known id by being called `Channel`. The interpreter kept a
//! five-slot `[len, cap, head, buffer_ptr, closed]` channel behind that
//! id, with the METHODS keyed on the type id and the CONSTRUCTOR keyed
//! on a receiver-name string. Only the first key ever matched, so every
//! Tier-0 channel was built by the stdlib and read by the builtin:
//! `send` read slot 4 (`data`) as `closed` and `recv` read slot 3
//! (`tail`, an Int) as the buffer pointer. `Channel.new(1)` followed by
//! one `send` was enough to reach a wrong answer, an interpreter panic
//! or a SIGSEGV, depending only on what the misread slot held.
//!
//! So `Channel` is NOT in this set: Tier 0 runs the stdlib body for the
//! constructor and the methods alike. Tier 1 keeps its own
//! `verum_chan_*` runtime and intercepts BOTH halves at the LLVM layer,
//! which is the property Tier 0 lacked.
//!
//! See `docs/architecture/intrinsic-dispatch-contract.md` section 9.
//!
//! ADDING A TYPE HERE means committing to the other half as well: an
//! interpreter method intercept keyed on that type's id, reading the
//! layout this constructor writes. REMOVING one means the stdlib body
//! must be able to run — which for `Channel` was checked by pasting
//! its body into a probe under a different type name, where no
//! well-known id is stamped and no intercept fires, and watching it
//! round-trip its values.

use verum_common::well_known_types::WellKnownType as W;

/// Every variant, partitioned. The list is exhaustive on purpose: a new
/// `WellKnownType` variant has to be classified here rather than
/// silently defaulting.
const INTERCEPTED: &[W] = &[W::List, W::Map, W::Set, W::Deque];

#[test]
fn the_constructor_intercept_set_is_exactly_the_four_collections() {
    for w in INTERCEPTED {
        assert!(
            w.has_builtin_constructor_intercept(),
            "{:?} is listed as intercepted but the predicate says otherwise",
            w
        );
    }

    // The types most likely to be added back by mistake, each with the
    // reason it is out.
    let out: &[(W, &str)] = &[
        (
            W::Channel,
            "Tier 0 has no builtin channel — `core/async/channel.vr` owns \
             the constructor AND the methods; re-adding it re-opens the \
             slot-3/slot-4 misread",
        ),
        (
            W::Shared,
            "the `Shared.new` interception was retired (T1159); the \
             compiled stdlib body builds it",
        ),
        (
            W::Heap,
            "`Heap.new` substitutes a CBGR cell, and the stdlib body \
             cannot read that cell as its declared record (T1189) — the \
             mismatch is reported, not papered over",
        ),
        (W::Text, "Text has no `new` constructor to intercept"),
    ];
    for (w, why) in out {
        assert!(
            !w.has_builtin_constructor_intercept(),
            "{:?} must NOT be constructor-intercepted: {}",
            w,
            why
        );
    }
}

#[test]
fn the_name_form_agrees_with_the_variant_form() {
    // `name_has_builtin_constructor_intercept` is what codegen calls;
    // the two spellings answering differently is how the halves drift.
    for (name, expected) in [
        ("List", true),
        ("Map", true),
        ("Set", true),
        ("Deque", true),
        ("Channel", false),
        ("Shared", false),
        ("Heap", false),
        ("NotAWellKnownTypeAtAll", false),
    ] {
        assert_eq!(
            W::name_has_builtin_constructor_intercept(name),
            expected,
            "`{}`: the name form and the variant form must agree",
            name
        );
    }
}

/// The constructor intercept is not the only half-a-table in this file.
/// Two more predicates decide, per type NAME, whether codegen reaches
/// for a builtin representation — and for `Channel` both of them used
/// to say yes while nothing on the other side could answer.
#[test]
fn channel_claims_no_builtin_length_anywhere() {
    // `len_type_hint` non-zero is what forces `.len()` and `.is_empty()`
    // into the `Len` opcode instead of the declared body. Tier 1's
    // `lower_len` carries arms for 1..=5 only, so Channel's old 6 was
    // read by no one and cost the stdlib body its call.
    assert_eq!(
        W::Channel.len_type_hint(),
        0,
        "a channel has no builtin length at either tier — `ch.len()` must \
         reach `Channel.len`, whose slot 0 is an `AtomicInt` and not an i64"
    );
    for (w, hint) in [
        (W::List, 1u8),
        (W::Map, 2),
        (W::Set, 3),
        (W::Deque, 4),
        (W::Text, 5),
    ] {
        assert_eq!(w.len_type_hint(), hint, "{:?}'s Len hint moved", w);
    }

    // `is_builtin_method_type` gates the `Type.is_empty` lookup: listed
    // here means "don't look, emit Len".
    assert!(
        !verum_common::well_known_types::type_names::is_builtin_method_type("Channel"),
        "listing Channel here makes codegen skip `Channel.is_empty` and \
         emit `Len` against the stdlib record"
    );
    for name in ["List", "Map", "Set", "Deque", "Text", "Heap", "Shared"] {
        assert!(
            verum_common::well_known_types::type_names::is_builtin_method_type(name),
            "{} dropped out of the builtin-method set",
            name
        );
    }
}
