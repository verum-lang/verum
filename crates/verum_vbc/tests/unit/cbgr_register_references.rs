//! Register reference authority is independent of retired environment settings.
use super::*;

/// The soundness property at the authority boundary: no user `Int` — of any
/// value or sign — is classified as a register-ref. This is the exact
/// function (and the exact seed value) from the T0367 acceptance.
#[test]
fn is_cbgr_ref_rejects_user_ints() {
    assert!(!is_cbgr_ref(&Value::from_i64(-131008744790)));
    // The full collision family -(k·2^32 + m) - 1 the legacy encoding
    // decoded to a small abs-index, plus assorted user ints and both
    // inline boundaries.
    for k in 0..64i64 {
        for m in [0i64, 1, 2, 1647571, 12242261, (1 << 24) - 1] {
            let bad = -(k * (1i64 << 32) + m) - 1;
            assert!(
                !is_cbgr_ref(&Value::from_i64(bad)),
                "user Int {} (k={}, m={}) misclassified as a register-ref",
                bad,
                k,
                m
            );
        }
    }
    for &i in &[0i64, 1, -1, i64::MIN, i64::MAX, -4296614867] {
        assert!(!is_cbgr_ref(&Value::from_i64(i)), "user Int {} misread", i);
    }
    // Other Value kinds are not register-refs either.
    assert!(!is_cbgr_ref(&Value::unit()));
    assert!(!is_cbgr_ref(&Value::from_bool(true)));
    assert!(!is_cbgr_ref(&Value::from_f64(1.5)));
}

/// A genuinely-encoded register-ref round-trips through the authority:
/// encode → is_cbgr_ref true → decode → same (abs, gen); mutability
/// preserved and strippable.
#[test]
fn encoded_regref_roundtrips() {
    let cases = [
        (0u32, 1u32, false),
        (7, 3, true),
        (1023, CBGR_NO_CHECK_GENERATION, false),
        (12242261, 42, true), // the T0367 abs — a *legitimate* ref here
        ((1 << 24) - 1, (1 << 22) - 1, true),
    ];
    for (abs, generation, mutable) in cases {
        let enc = if mutable {
            encode_cbgr_ref_mut(abs, generation)
        } else {
            encode_cbgr_ref(abs, generation)
        };
        assert!(is_cbgr_ref(&enc), "encoded regref not classified (abs={})", abs);
        assert!(!enc.is_int(), "a tagged regref must not be an Int");
        let (dabs, dgen) = decode_cbgr_ref(enc);
        assert_eq!(dabs, abs, "abs round-trip");
        assert_eq!(dgen, generation & ((1 << 22) - 1), "gen round-trip mod 2^22");
        assert_eq!(is_cbgr_ref_mutable(enc), mutable, "mutability round-trip");

        let shared = strip_cbgr_ref_mutability(enc);
        assert!(is_cbgr_ref(&shared));
        assert!(!is_cbgr_ref_mutable(shared), "strip must clear mutability");
        let (sabs, sgen) = decode_cbgr_ref(shared);
        assert_eq!((sabs, sgen), (abs, generation & ((1 << 22) - 1)));
    }
}

#[test]
fn obsolete_environment_setting_cannot_restore_integer_references() {
    // Use fresh processes: environment caches must not make the result depend
    // on test order, and no process-global environment mutation is needed.
    for setting in ["1", "true", "0", ""] {
        for test in ["is_cbgr_ref_rejects_user_ints", "encoded_regref_roundtrips"] {
            let name = format!("interpreter::dispatch_table::handlers::cbgr_helpers::tests::{test}");
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", &name, "--nocapture"])
                .env("VERUM_CBGR_LEGACY_INT_REFS", setting)
                .output()
                .expect("execute isolated reference authority control");
            let stdout = verum_common::Text::from_utf8_lossy(&output.stdout);
            let stderr = verum_common::Text::from_utf8_lossy(&output.stderr);
            assert!(output.status.success(), "setting={setting:?}, test={test}\n{stdout}\n{stderr}");
            assert!(stdout.contains("1 passed; 0 failed"), "control did not run: {stdout}");
        }
    }
}
