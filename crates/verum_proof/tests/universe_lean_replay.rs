//! Compact universe arithmetic must have the same meaning in both checkers.
use std::process::Command;
use verum_proof::proof_checker::{Certificate, Level, Term};

fn offset(base: Level, by: u32) -> Level {
    Level::Offset(Box::new(base), by)
}
fn maximum(a: Level, b: Level) -> Level {
    Level::Max(Box::new(a), Box::new(b))
}
fn successor(level: Level) -> Level {
    Level::Succ(Box::new(level))
}
fn comparisons() -> Vec<(&'static str, Level, Level, bool)> {
    let u = Level::var("u");
    let v = Level::var("v");
    let big = u32::MAX;
    vec![
        ("zero-offset", offset(u.clone(), 0), u.clone(), true),
        (
            "successor",
            offset(u.clone(), 1),
            successor(u.clone()),
            true,
        ),
        (
            "small-composition",
            offset(offset(u.clone(), 2), 3),
            offset(u.clone(), 5),
            true,
        ),
        (
            "large-offset",
            offset(u.clone(), big),
            offset(u.clone(), big),
            true,
        ),
        (
            "common-large-prefix",
            maximum(offset(u.clone(), big), offset(v.clone(), big)),
            offset(maximum(u.clone(), v.clone()), big),
            true,
        ),
        (
            "nested-wide-prefix",
            maximum(
                offset(offset(u.clone(), big), big),
                offset(offset(v.clone(), big), big),
            ),
            offset(offset(maximum(u.clone(), v.clone()), big), big),
            true,
        ),
        (
            "concrete-prefix",
            maximum(Level::Concrete(big), offset(u.clone(), big)),
            offset(u.clone(), big),
            true,
        ),
        (
            "closed-beyond-wire-width",
            offset(Level::Concrete(big), 1),
            successor(Level::Concrete(big)),
            true,
        ),
        (
            "concrete-boundary",
            offset(Level::Concrete(big - 3), 1),
            Level::Concrete(big - 2),
            true,
        ),
        (
            "max-order",
            maximum(maximum(v.clone(), u.clone()), Level::var("w")),
            maximum(Level::var("w"), maximum(u.clone(), v.clone())),
            true,
        ),
        (
            "wrong-large-offset",
            offset(u.clone(), big),
            offset(u.clone(), big - 1),
            false,
        ),
        ("wrong-variable", offset(u.clone(), 2), offset(v, 2), false),
        // A parameter shifted by one dominates the same unshifted parameter.
        (
            "same-parameter-dominance",
            maximum(u.clone(), successor(u.clone())),
            successor(u),
            true,
        ),
        (
            "symbolic-offset-chunk-associativity",
            offset(offset(Level::var("u"), big), 1),
            offset(offset(Level::var("u"), 1), big),
            true,
        ),
        (
            "closed-offset-chunk-associativity",
            offset(offset(Level::Concrete(big), big), 1),
            offset(offset(Level::Concrete(big), 1), big),
            true,
        ),
        (
            "distributed-asymmetric-wide-shift",
            maximum(
                offset(Level::var("u"), big),
                offset(offset(Level::var("v"), big), 3),
            ),
            offset(maximum(Level::var("u"), offset(Level::var("v"), 3)), big),
            true,
        ),
        (
            "wide-dominated-constant",
            maximum(
                offset(Level::Concrete(big), 1),
                offset(offset(Level::var("v"), 1), big),
            ),
            offset(offset(Level::var("v"), big), 1),
            true,
        ),
        (
            "wide-undominated-constant",
            maximum(
                offset(Level::Concrete(big), 2),
                offset(offset(Level::var("v"), 1), big),
            ),
            offset(offset(Level::var("v"), big), 1),
            false,
        ),
        (
            "different-parameter-separated-above-large-floor",
            maximum(
                offset(Level::var("u"), big),
                offset(offset(Level::var("v"), big), 1),
            ),
            offset(offset(Level::var("v"), big), 1),
            false,
        ),
        (
            "nearby-wide-sums-stay-distinct",
            offset(offset(Level::var("u"), big), big),
            offset(offset(Level::var("u"), big - 1), big),
            false,
        ),
        (
            "concrete-boundary-equality",
            offset(Level::Concrete(big - 1), 1),
            Level::Concrete(big),
            true,
        ),
    ]
}

fn universe_inputs() -> Vec<(&'static str, Level)> {
    let big = u32::MAX;
    vec![
        ("below-boundary", Level::Concrete(big - 1)),
        ("boundary", Level::Concrete(big)),
        ("above-boundary", offset(Level::Concrete(big), big)),
        (
            "closed-maximum",
            maximum(Level::Concrete(1), Level::Concrete(2)),
        ),
        (
            "symbolic-floor",
            maximum(Level::var("u"), Level::Concrete(big)),
        ),
        ("zero-offset", offset(Level::var("u"), 0)),
        (
            "dominated-parameter",
            maximum(Level::var("v"), successor(Level::var("v"))),
        ),
        (
            "wide-maximum",
            maximum(
                offset(Level::var("u"), big),
                offset(offset(Level::var("v"), big), 3),
            ),
        ),
    ]
}

fn cases() -> Vec<(String, Certificate, bool)> {
    let mut cases: Vec<_> = comparisons()
        .into_iter()
        .map(|(id, left, right, accepted)| {
            let domain = Term::universe_at(right);
            (
                id.to_string(),
                Certificate {
                    term: Term::lam(Term::universe_at(left), Term::var(0)),
                    claimed_type: Term::pi(domain.clone(), domain),
                    metadata: Default::default(),
                },
                accepted,
            )
        })
        .collect();
    for (name, level) in universe_inputs() {
        let term = Term::universe_at(level.clone());
        let raised = successor(level.clone());
        for (claim, expected, accepted) in [
            ("raw-successor", raised.clone(), true),
            ("canonical-successor", raised.clone().normalize(), true),
            ("self", level, false),
            ("two-above", successor(raised), false),
        ] {
            cases.push((
                format!("{name}-{claim}"),
                Certificate {
                    term: term.clone(),
                    claimed_type: Term::universe_at(expected),
                    metadata: Default::default(),
                },
                accepted,
            ));
        }
    }
    cases
}

fn native_universe_types(level: &Level) -> [Level; 2] {
    use verum_proof::{proof_checker as sub, proof_checker_nbe as nbe};
    let term = Term::universe_at(level.clone());
    [
        sub::infer(&sub::Context::new(), &term),
        nbe::infer(&nbe::NbeContext::new(), &term),
    ]
    .map(|inferred| match inferred {
        Ok(Term::Universe(output)) => output,
        other => panic!("expected a universe type for {level:?}, received {other:?}"),
    })
}

#[test]
fn public_inference_preserves_the_raw_universe_expression() {
    for (name, level) in universe_inputs() {
        let expected = match &level {
            Level::Concrete(n) if *n < u32::MAX => Level::Concrete(n + 1),
            _ => successor(level.clone()),
        };
        for output in native_universe_types(&level) {
            assert_eq!(output, expected, "{name}");
        }
    }
}

#[test]
fn native_compact_universe_controls() {
    for (id, certificate, accepted) in cases() {
        for result in [
            certificate.verify(),
            verum_proof::proof_checker_nbe::verify_certificate(&certificate),
        ] {
            if accepted {
                assert_eq!(result, Ok(()), "{id}");
            } else {
                assert!(
                    matches!(
                        result,
                        Err(verum_proof::proof_checker::CheckError::TypeMismatch { .. })
                    ),
                    "{id}: {result:?}"
                );
            }
        }
    }
}

#[test]
#[ignore = "requires VERUM_LEAN_REPLAY pointing to the independent checker"]
fn independent_compact_universes_and_wire_bounds() {
    let replay = std::env::var_os("VERUM_LEAN_REPLAY").expect("build verum_replay_checker first");
    let cases = cases();
    let rows: Vec<_> = cases.iter().map(|(id, certificate, _)| {
        serde_json::json!({"id":id,"term":certificate.term,"claimed_type":certificate.claimed_type})
    }).collect();
    let path =
        std::env::temp_dir().join(format!("verum-universe-replay-{}.json", std::process::id()));
    let battery = serde_json::json!({"schema_version":1,"certificates":rows});
    std::fs::write(&path, serde_json::to_vec(&battery).unwrap()).unwrap();
    let output = Command::new(&replay).arg(&path).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let decoded: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let verdicts = decoded["verdicts"].as_array().unwrap();
    assert_eq!(verdicts.len(), cases.len());
    for (verdict, (id, _, expected)) in verdicts.iter().zip(cases) {
        assert_eq!(verdict["id"], id);
        assert_eq!(verdict["ok"], expected, "{id}: {verdict}");
        if !expected {
            assert_eq!(verdict["error"], "TypeMismatch", "{id}: {verdict}");
        }
    }
    for invalid in [
        serde_json::json!(4294967296_u64),
        serde_json::json!({"Concrete":4294967296_u64}),
        serde_json::json!({"Offset":[{"Var":"u"},4294967296_u64]}),
        serde_json::json!({"Offset":[{"Var":"u"},-1]}),
        serde_json::json!({"Offset":[{"Var":"u"},2,3]}),
        serde_json::json!({"Var":"u","Concrete":0}),
    ] {
        assert!(serde_json::from_value::<Level>(invalid.clone()).is_err());
        let mut malformed = battery.clone();
        malformed["certificates"][0]["term"] = serde_json::json!({"Universe":invalid});
        std::fs::write(&path, serde_json::to_vec(&malformed).unwrap()).unwrap();
        let rejected = Command::new(&replay).arg(&path).output().unwrap();
        assert_eq!(rejected.status.code(), Some(4), "invalid level: {invalid}");
    }
    std::fs::remove_file(path).unwrap();
}

#[test]
fn normalization_consumes_skewed_maxima_and_emits_a_balanced_tree() {
    let mut tree = Level::var("root");
    for index in 0..16_384 {
        tree = maximum(tree, Level::var(format!("parameter-{index}")));
    }
    let result = tree.normalize();
    let mut pending = vec![(&result, 0)];
    let mut variables = 0;
    while let Some((node, depth)) = pending.pop() {
        assert!(depth <= 15, "maximum should be balanced");
        match node {
            Level::Max(left, right) => {
                pending.push((left, depth + 1));
                pending.push((right, depth + 1));
            }
            Level::Var(_) => variables += 1,
            _ => panic!("all input atoms are unshifted parameters"),
        }
    }
    assert_eq!(variables, 16_385);
}

// Encode the observed data. Lean independently checks equality under every
// valuation, and rejects a successor substituted for each result.
fn lean_level(level: &Level) -> String {
    match level {
        Level::Concrete(n) => format!("(.concrete {n})"),
        Level::Var(name) => {
            let scalars: Vec<_> = name
                .chars()
                .map(|c| format!("Char.ofNat {}", u32::from(c)))
                .collect();
            format!("(.var (String.ofList [{}]))", scalars.join(", "))
        }
        Level::Succ(base) => format!("(.succ {})", lean_level(base)),
        Level::Offset(base, n) => format!("(.offset {} {n})", lean_level(base)),
        Level::Max(left, right) => format!("(.max {} {})", lean_level(left), lean_level(right)),
    }
}

#[test]
#[ignore = "requires Lean and the freshly built reference checker library"]
fn lean_checks_compiled_native_results_at_every_valuation() {
    use std::{fmt::Write as _, path::PathBuf};
    let replay =
        PathBuf::from(std::env::var_os("VERUM_LEAN_REPLAY").expect("build verum_replay_checker"));
    let library = replay.parent().unwrap().parent().unwrap().join("lib/lean");
    let mut source = include_str!(
        "../../../verification/external/lean/VerumExternalReplay/UniverseLevelEquality.lean"
    )
    .to_string();
    source.push_str(
        "
namespace VerumKernel
private def infersRawUniverse (input expected : Level) : Bool :=
  match infer {} (.universe input) with
  | .ok (.universe actual) => decide (actual = expected)
  | _ => false
",
    );
    for (id, left, right, expected) in comparisons() {
        let source_left = lean_level(&left);
        let source_right = lean_level(&right);
        let mut raw = vec![
            (source_left.clone(), left.clone().normalize()),
            (source_right.clone(), right.clone().normalize()),
            (
                format!("(.max {source_left} {source_right})"),
                left.clone().max_with(right.clone()),
            ),
            (
                format!("(.offset {source_left} {})", u32::MAX),
                left.clone().shifted_by(u32::MAX),
            ),
            (format!("(.succ {source_right})"), right.successor()),
        ];
        writeln!(source, "-- {id}").unwrap();
        for (input, expression) in [(&left, &source_left), (&right, &source_right)] {
            let [substitution, nbe] = native_universe_types(input);
            let result = lean_level(&substitution);
            // Compare the independently compiled reference inference structurally
            // with the native public result, before any level conversion.
            writeln!(
                source,
                "/-- info: true -/\n#guard_msgs in\n#eval! infersRawUniverse {expression} {result}"
            )
            .unwrap();
            assert_eq!(substitution, nbe, "{id}");
            raw.push((format!("(.succ {expression})"), substitution));
            raw.push((format!("(.succ {expression})"), nbe));
        }
        for (spec, actual) in raw {
            let output = lean_level(&actual);
            writeln!(source, "example (ρ : String → Nat) : Level.denote ρ {spec} = Level.denote ρ {output} := (Level.eq_iff_denote _ _).mp (by decide) ρ").unwrap();
            writeln!(
                source,
                "example : Level.eq {spec} (.succ {output}) = false := by decide"
            )
            .unwrap();
        }
        let accepted = left.clone().normalize() == right.clone().normalize();
        assert_eq!(accepted, expected, "{id}");
        writeln!(
            source,
            "example : Level.eq {source_left} {source_right} = {accepted} := by decide"
        )
        .unwrap();
    }
    source.push_str("end VerumKernel\n");
    let output_dir = std::env::var_os("VERUM_UNIVERSE_WITNESS_OUTPUT").map(PathBuf::from);
    let dir = output_dir.clone().unwrap_or_else(std::env::temp_dir);
    std::fs::create_dir_all(&dir).unwrap();
    let witness = dir.join(format!(
        "NativeUniverseWitnesses{}.lean",
        std::process::id()
    ));
    std::fs::write(&witness, source).unwrap();
    let result = Command::new(std::env::var_os("VERUM_LEAN").unwrap_or_else(|| "lean".into()))
        .env("LEAN_PATH", library)
        .arg(&witness)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "Lean rejected {}:\n{}\n{}",
        witness.display(),
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    if output_dir.is_none() {
        std::fs::remove_file(witness).unwrap();
    }
}
