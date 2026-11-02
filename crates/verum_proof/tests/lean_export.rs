use serde_json::{Value, json};
use verum_proof::lean_export::source;

const FIBER: &str = include_str!("fixtures/decoder_fiber.json");

fn renamed() -> String {
    FIBER
        .replace("level_0", "x0")
        .replace("level_1", "u v")
        .replace("level_2", "end -- injected")
}

#[cfg(feature = "process")]
fn mismatched_endpoint() -> Value {
    let mut judgment: Value = serde_json::from_str(FIBER).unwrap();
    for (pointer, binder) in [
        ("/statement", "Pi"),
        ("/certificate/claimed_type", "Pi"),
        ("/certificate/term", "Lam"),
    ] {
        let mut ty = judgment.pointer_mut(pointer).unwrap();
        for _ in 0..8 {
            ty = &mut ty[binder][1];
        }
        let id = &mut ty[binder][0]["Id"];
        assert_ne!(id["rhs"], id["lhs"]);
        id["rhs"] = id["lhs"].clone();
    }
    judgment
}

#[test]
fn universe_parameters_belong_to_the_statement() {
    let lean = source(FIBER.as_bytes()).unwrap();
    assert_eq!(source(renamed().as_bytes()).unwrap(), lean);
    assert!(lean.contains("universe w0 w1 w2\n"));
    let mut extra: Value = serde_json::from_str(FIBER).unwrap();
    extra["certificate"]["term"]["Lam"][0] = json!({"Universe":{"Var":"foreign"}});
    assert!(
        source(extra.to_string().as_bytes())
            .unwrap_err()
            .contains("statement scope")
    );
    extra["certificate"]["claimed_type"] = json!({"Universe":0});
    assert!(
        source(extra.to_string().as_bytes())
            .unwrap_err()
            .contains("different formal statement")
    );
}

#[cfg(feature = "process")]
#[test]
#[ignore = "requires VERUM_LEAN_BIN pointing to the pinned Lean toolchain"]
fn kernel_accepts_polymorphic_transport_and_rejects_a_changed_endpoint() {
    let lean =
        std::env::var_os("VERUM_LEAN_BIN").expect("explicit pinned Lean executable required");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Certificate.lean");
    for (wire, valid) in [
        (FIBER.to_owned(), true),
        (renamed(), true),
        (mismatched_endpoint().to_string(), false),
    ] {
        std::fs::write(&path, source(wire.as_bytes()).unwrap()).unwrap();
        let output = std::process::Command::new(&lean)
            .arg("--json")
            .arg(&path)
            .output()
            .unwrap();
        let messages: Vec<Value> = String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect();
        assert_eq!(output.status.success(), valid, "{messages:?}");
        if valid {
            assert_eq!(messages.len(), 1, "{messages:?}");
            assert_eq!(
                messages[0]["data"],
                "'VerumCertificate.checked' does not depend on any axioms"
            );
        } else {
            assert!(
                messages.iter().any(|m| m["severity"] == "error"
                    && m["data"]
                        .as_str()
                        .is_some_and(|s| s.starts_with("Application type mismatch")
                            || s.starts_with("Type mismatch"))),
                "{messages:?}"
            );
        }
    }
}
