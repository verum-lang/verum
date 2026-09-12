//! `find_function_by_name` answers the same under its indices as under
//! the full scans they replaced — measured INSIDE one binary.
//!
//! The lookup has two arms. The EXACT arm has been served by
//! `fn_idx_by_name` for a while; the `.name` SUFFIX arm was still
//! walking every descriptor and calling `get_string` — a hash lookup —
//! on each. `VbcModule::resolve_external_bands` calls this once per
//! external band entry, so the pair is quadratic in module size, and
//! that is where a 30 s sample of a channel spec's compile lands.
//!
//! An index that answers DIFFERENTLY from the scan it replaced is a
//! silent change of meaning, and comparing two BUILDS cannot separate
//! it from everything else in the tree. `VERUM_NO_FN_INDEX=1` forces
//! both arms back to the scan, which makes the comparison possible in
//! one process — this file is that comparison.
//!
//! The cases are chosen so a wrong answer is a DIFFERENT id, never a
//! shared `None`: bodied and bodyless entries are interleaved, and the
//! decoys share a last segment with the subject without matching its
//! suffix.

use verum_vbc::module::{FunctionDescriptor, VbcModule};

/// `bodied` entries get a non-zero `bytecode_length`, which is what
/// `find_function_by_name`'s tie-break ranks on.
fn module(entries: &[(&str, bool)]) -> VbcModule {
    let mut m = VbcModule::new("probe".to_string());
    for (name, bodied) in entries {
        let sid = m.intern_string(name);
        let mut desc = FunctionDescriptor::new(sid);
        if *bodied {
            desc.bytecode_length = 8;
        }
        m.functions.push(desc);
    }
    m
}

/// Every name worth asking about in `entries`, plus the ones that must
/// miss.
fn probes() -> Vec<&'static str> {
    vec![
        "main",
        "UInt8.to_hex",
        "to_hex",
        "Channel.send",
        "send",
        "base.primitives.UInt8.to_hex",
        "Nope.absent",
        "absent",
        "",
        ".",
        "a.b.c.d",
    ]
}

fn answers(m: &VbcModule) -> Vec<(String, Option<u32>)> {
    probes()
        .into_iter()
        .map(|p| (p.to_string(), m.find_function_by_name(p).map(|f| f.0)))
        .collect()
}

/// The whole point: same module, same questions, index vs scan.
///
/// Serialised through one process-wide env var, so this test owns the
/// variable for its duration and restores it — the suite runs threads.
#[test]
fn the_indexed_answers_equal_the_scanned_ones() {
    let m = module(&[
        // A decoy sharing the last segment but NOT the suffix: keyed
        // under `to_hex` in the index, must be rejected by `ends_with`.
        ("other.Int16.to_hex", true),
        // Bodyless first, bodied later — the tie-break must pick the
        // bodied one, and it is the index order that decides.
        ("base.primitives.UInt8.to_hex", false),
        ("deep.base.primitives.UInt8.to_hex", true),
        ("main", true),
        // Exact and suffix candidates for the same simple name.
        ("Channel.send", false),
        ("core.async.channel.Channel.send", true),
        ("net.socket.send", true),
        // A bare name that is also somebody's last segment.
        ("absent_decoy.absent", true),
    ]);

    let indexed = answers(&m);

    let prev = std::env::var_os("VERUM_NO_FN_INDEX");
    unsafe { std::env::set_var("VERUM_NO_FN_INDEX", "1") };
    let scanned = answers(&m);
    match prev {
        Some(v) => unsafe { std::env::set_var("VERUM_NO_FN_INDEX", v) },
        None => unsafe { std::env::remove_var("VERUM_NO_FN_INDEX") },
    }

    for ((name, a), (_, b)) in indexed.iter().zip(scanned.iter()) {
        assert_eq!(
            a, b,
            "`{name}`: the index answers {a:?} where the scan answers \
             {b:?} — an index that disagrees with the scan it replaced \
             is a silent change of meaning, not a speed-up",
        );
    }

    // A guard on the guard: if every probe missed, the comparison above
    // would be vacuously true and would stay green through any defect.
    let hits = indexed.iter().filter(|(_, a)| a.is_some()).count();
    assert!(
        hits >= 4,
        "the probe set must actually resolve things — only {hits} of \
         {} found a function, so the equality above proves nothing",
        indexed.len()
    );
}

/// The suffix arm's candidate set is keyed on the LAST SEGMENT, which
/// is only sound if it is a SUPERSET of the matches. This pins the
/// reasoning directly rather than through the lookup.
#[test]
fn a_suffix_match_always_shares_the_probes_last_segment() {
    let m = module(&[
        ("a.b.Type.method", true),
        ("Type.method", true),
        ("x.method", true),
        ("method", true),
        ("other.Type.notmethod", true),
    ]);
    for probe in ["Type.method", "method", "a.b.Type.method"] {
        let suffix = format!(".{probe}");
        let seg = probe.rsplit('.').next().unwrap();
        for (idx, desc) in m.functions.iter().enumerate() {
            let fname = m.get_string(desc.name).unwrap();
            if fname.ends_with(&suffix) {
                assert!(
                    fname.rsplit('.').next() == Some(seg),
                    "`{fname}` (#{idx}) ends with `{suffix}` yet its \
                     last segment is not `{seg}` — the index would \
                     never offer it as a candidate and the match would \
                     be lost",
                );
            }
        }
    }
}

/// `add_function` after the index is materialised must reach BOTH
/// indices. A stale index answers a subset, which is the silent half of
/// a wrong answer.
#[test]
fn a_function_added_after_the_index_is_still_found() {
    let mut m = module(&[("core.thing.run", true)]);
    // Materialise both indices.
    assert!(m.find_function_by_name("thing.run").is_some());
    assert!(m.find_function_by_name("core.thing.run").is_some());

    let sid = m.intern_string("late.module.added_fn");
    let mut desc = FunctionDescriptor::new(sid);
    desc.bytecode_length = 8;
    let id = m.add_function(desc);

    assert_eq!(
        m.find_function_by_name("late.module.added_fn").map(|f| f.0),
        Some(id.0),
        "exact lookup lost a function added after the index was built",
    );
    assert_eq!(
        m.find_function_by_name("module.added_fn").map(|f| f.0),
        Some(id.0),
        "suffix lookup lost a function added after the index was built",
    );
}
