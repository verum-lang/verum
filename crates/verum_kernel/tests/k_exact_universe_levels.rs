//! T1768: the concrete carrier must never identify different universe levels.
use verum_kernel::proof_checker::{Certificate, Context, Level, Term, infer};
use verum_kernel::proof_checker_nbe;

fn successor(level: Level) -> Level {
    Level::Succ(Box::new(level))
}
fn exact(level: &Level) -> u64 {
    exact_at(level, 7, 11)
}
fn exact_at(level: &Level, u: u64, v: u64) -> u64 {
    match level {
        Level::Concrete(n) => u64::from(*n),
        Level::Var(name) => match name.as_str() {
            "u" => u,
            "v" => v,
            _ => panic!("undeclared oracle variable"),
        },
        Level::Succ(inner) => exact_at(inner, u, v) + 1,
        Level::Offset(inner, by) => exact_at(inner, u, v) + u64::from(*by),
        Level::Max(a, b) => exact_at(a, u, v).max(exact_at(b, u, v)),
    }
}

#[test]
fn successor_above_concrete_carrier_never_collapses() {
    let top = Level::Concrete(u32::MAX);
    let larger = successor(top.clone());
    assert_ne!(larger.clone().normalize(), top);
    assert_eq!(larger.as_concrete(), None);
    assert!(infer(&Context::new(), &Term::universe(u32::MAX)).is_err());
}

#[test]
fn both_kernels_reject_forged_successor_claims_and_preserve_top_boundary() {
    let mut level = Level::Concrete(u32::MAX);
    for _ in 0..4 {
        level = successor(level);
        let cert = Certificate {
            term: Term::universe(u32::MAX - 1),
            claimed_type: Term::universe_at(level.clone()),
            metadata: Default::default(),
        };
        assert!(cert.verify().is_err(), "{level:?}");
        assert!(
            proof_checker_nbe::verify_certificate(&cert).is_err(),
            "{level:?}"
        );
        let decoded: Certificate =
            serde_json::from_str(&serde_json::to_string(&cert).unwrap()).unwrap();
        assert!(decoded.verify().is_err());
    }
    let valid = Certificate {
        term: Term::universe(u32::MAX - 1),
        claimed_type: Term::universe(u32::MAX),
        metadata: Default::default(),
    };
    valid.verify().unwrap();
    proof_checker_nbe::verify_certificate(&valid).unwrap();
}

#[test]
fn normalization_and_lifts_agree_with_independent_integer_interpretation() {
    let seeds = [
        Level::Concrete(0),
        Level::Concrete(3),
        Level::Concrete(u32::MAX - 1),
        Level::Concrete(u32::MAX),
        Level::var("u"),
        successor(Level::Concrete(u32::MAX)),
    ];
    for a in &seeds {
        for b in &seeds {
            let level = successor(Level::Max(Box::new(a.clone()), Box::new(b.clone())));
            assert_eq!(exact(&level.clone().normalize()), exact(&level));
            for by in 0..4 {
                assert_eq!(
                    exact(&level.clone().shifted_by(by)),
                    exact(&level) + u64::from(by)
                );
            }
        }
        for by in 0..4 {
            assert_eq!(exact(&a.clone().shifted_by(by)), exact(a) + u64::from(by));
        }
    }
}

#[test]
fn maximum_offsets_are_exact_and_compact_for_numeric_and_symbolic_bases() {
    for base in [Level::Concrete(u32::MAX), Level::var("u")] {
        let shifted = base.clone().shifted_by(u32::MAX);
        assert_eq!(exact(&shifted), exact(&base) + u64::from(u32::MAX));
        let json = serde_json::to_string(&shifted).unwrap();
        assert!(
            json.len() < 120,
            "a numerical lift must not expand into billions of nodes"
        );
        let restored: Level = serde_json::from_str(&json).unwrap();
        assert_eq!(exact(&restored), exact(&shifted));
        let twice = shifted.clone().shifted_by(u32::MAX);
        assert_eq!(exact(&twice), exact(&base) + 2 * u64::from(u32::MAX));
        assert!(serde_json::to_string(&twice).unwrap().len() < 180);
        let common = Level::Max(
            Box::new(shifted),
            Box::new(Level::var("v").shifted_by(u32::MAX)),
        );
        assert_eq!(exact(&common.clone().normalize()), exact(&common));
        assert!(serde_json::to_string(&common.normalize()).unwrap().len() < 240);
    }
}

#[test]
fn offset_normalization_is_exact_and_idempotent_under_distinct_valuations() {
    let nested = |name: &str| {
        Level::Offset(
            Box::new(Level::Offset(Box::new(Level::var(name)), u32::MAX)),
            u32::MAX,
        )
    };
    let seeds = [
        Level::var("u"),
        Level::var("v"),
        nested("u"),
        nested("v"),
        Level::Concrete(u32::MAX),
        Level::Offset(Box::new(Level::var("u")), 3),
    ];
    for a in &seeds {
        for b in &seeds {
            let raw = Level::Max(Box::new(a.clone()), Box::new(b.clone()));
            let normalized = raw.clone().normalize();
            assert_eq!(normalized.clone().normalize(), normalized, "{raw:?}");
            for (u, v) in [
                (0, 17),
                (17, 0),
                (u64::from(u32::MAX), u64::from(u32::MAX) + 1),
            ] {
                assert_eq!(exact_at(&normalized, u, v), exact_at(&raw, u, v));
                for by in [0, 1, 7, u32::MAX] {
                    let shifted = raw.clone().shifted_by(by);
                    assert_eq!(
                        exact_at(&shifted, u, v),
                        exact_at(&raw, u, v) + u64::from(by)
                    );
                }
            }
        }
    }
}
