use super::{RefinementChecker, RefinementConfig};
use std::{fmt::Write, path::Path, process::Command};
use verum_ast::{BinOp, Expr, ExprKind, LiteralKind};
use verum_common::{List, Text};
use verum_fast_parser::Parser;

fn parse(source: &str) -> Expr {
    Parser::new(source)
        .parse_expr()
        .expect("expression grammar")
}

fn term(expr: &Expr) -> Text {
    match &expr.kind {
        ExprKind::Path(path) if path.segments.len() == 1 => {
            format!("(.var {:?})", path.to_string()).into()
        }
        ExprKind::Literal(literal) => match &literal.kind {
            LiteralKind::Int(value) => format!("(.int ({}))", value.value).into(),
            _ => panic!("outside the replay's integer-literal fragment: {expr:?}"),
        },
        ExprKind::Binary {
            op: op @ (BinOp::Add | BinOp::Sub),
            left,
            right,
        } => format!(
            "(.{} {} {})",
            if *op == BinOp::Add { "add" } else { "sub" },
            term(left),
            term(right)
        )
        .into(),
        ExprKind::Paren(inner) => term(inner),
        _ => panic!("outside the replay's binder-free fragment: {expr:?}"),
    }
}

#[test]
#[ignore = "requires the pinned Lean toolchain; run explicitly with --ignored"]
fn native_simultaneous_substitution_preserves_every_environment_and_operator_interpretation() {
    let checker = RefinementChecker::new(RefinementConfig::default());
    let mut source = Text::from(include_str!(
        "../../../../verification/external/lean/VerumExternalReplay/RefinementSubstitution.lean"
    ));
    source.push_str("\nopen Verum.RefinementSubstitution\n");
    for (index, bindings) in [
        [("a", "b"), ("b", "0")],
        [("a", "b + 1"), ("b", "a - 1")],
        [("it", "self + 1"), ("self", "it - 1")],
        [("a", "__refinement_subject_0"), ("b", "a")],
    ]
    .iter()
    .enumerate()
    {
        let values: List<_> = bindings
            .iter()
            .map(|(name, value)| (Text::from(*name), parse(value)))
            .collect();
        let replacements: List<_> = values
            .iter()
            .map(|(name, value)| (name.clone(), value))
            .collect();
        let mut function = Text::from(".var name");
        for (name, value) in values.iter().rev() {
            function = format!(
                "if name == {:?} then {} else {function}",
                name.as_str(),
                term(value)
            )
            .into();
        }
        writeln!(
            source,
            "def replacements{index} (name : String) : Term := {function}"
        )
        .unwrap();
        for input in [
            "a",
            "b",
            "a + b",
            "(a - b) + (b - a)",
            "it",
            "self",
            "it + self",
            "__refinement_subject_0 + a",
        ] {
            let original = parse(input);
            let actual = checker.substitute_all_in_expr(&original, &replacements);
            writeln!(source,
                "example (algebra : Algebra Value) (env : String → Value) :\n  eval algebra env {} = eval algebra (fun name => eval algebra env (replacements{index} name)) {} := by rfl",
                term(&actual), term(&original)).unwrap();
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("NativeSubstitution.lean");
    std::fs::write(&path, source.as_bytes()).unwrap();
    let output = Command::new(std::env::var_os("VERUM_LEAN").unwrap_or_else(|| "lean".into()))
        .arg(&path)
        .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../verification/external/lean"))
        .output()
        .expect("the pinned Lean toolchain must be installed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stdout}\n{stderr}");
    assert!(
        stdout.contains("Verum.RefinementSubstitution.sound"),
        "{stdout}"
    );
    for forbidden in ["sorryAx", "Lean.ofReduceBool", "warning:", "error:"] {
        assert!(
            !stdout.contains(forbidden) && !stderr.contains(forbidden),
            "{stdout}\n{stderr}"
        );
    }
}
