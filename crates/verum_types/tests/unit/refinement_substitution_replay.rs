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
            let actual = checker
                .substitute_all_in_expr(&original, &replacements)
                .expect("checked substitution");
            writeln!(source,
                "example (algebra : Algebra Value) (env : String → Value) :\n  eval algebra env {} = eval algebra (fun name => eval algebra env (replacements{index} name)) {} := by rfl",
                term(&actual), term(&original)).unwrap();
        }
    }
    assert_lean(&source, "Verum.RefinementSubstitution.sound");
}

fn assert_lean(source: &str, theorem: &str) {
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
    assert!(stdout.contains(theorem), "{stdout}");
    for forbidden in ["sorryAx", "Lean.ofReduceBool", "warning:", "error:"] {
        assert!(
            !stdout.contains(forbidden) && !stderr.contains(forbidden),
            "{stdout}\n{stderr}"
        );
    }
}

fn scoped_term(expr: &Expr, locals: &[Text]) -> Text {
    scoped_term_with_aliases(expr, locals, &[])
}

fn scoped_term_with_aliases(expr: &Expr, locals: &[Text], root_aliases: &[Text]) -> Text {
    use verum_ast::{PatternKind, StmtKind};
    match &expr.kind {
        ExprKind::Path(path) if path.segments.len() == 1 => {
            let name = path.to_string();
            if let Some(index) = locals.iter().rev().position(|bound| bound.as_str() == name) {
                format!("(.bound ⟨{index}, by decide⟩)").into()
            } else if root_aliases.iter().any(|alias| alias.as_str() == name) {
                format!(
                    "(.bound ⟨{}, by decide⟩)",
                    locals.len().checked_sub(1).expect("root binder")
                )
                .into()
            } else {
                format!("(.var {name:?})").into()
            }
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
            scoped_term_with_aliases(left, locals, root_aliases),
            scoped_term_with_aliases(right, locals, root_aliases)
        )
        .into(),
        ExprKind::Paren(inner) => scoped_term_with_aliases(inner, locals, root_aliases),
        ExprKind::Block(block) => {
            let mut locals: List<_> = locals.iter().cloned().collect();
            let mut initializers = List::new();
            for statement in &block.stmts {
                let StmtKind::Let {
                    pattern,
                    value: Some(value),
                    ..
                } = &statement.kind
                else {
                    panic!("outside the replay's immutable-let fragment: {statement:?}")
                };
                let PatternKind::Ident {
                    name,
                    mutable: false,
                    subpattern: None,
                    by_ref: false,
                } = &pattern.kind
                else {
                    panic!("outside the replay's simple-binding fragment: {pattern:?}")
                };
                initializers.push(scoped_term_with_aliases(value, &locals, root_aliases));
                locals.push(name.name.clone());
            }
            let mut body = scoped_term_with_aliases(
                block.expr.as_ref().expect("block tail"),
                &locals,
                root_aliases,
            );
            for initial in initializers.iter().rev() {
                body = format!("(.letE {initial} {body})").into();
            }
            body
        }
        _ => panic!("outside the replay's arithmetic/let fragment: {expr:?}"),
    }
}

#[test]
#[ignore = "requires the pinned Lean toolchain; run explicitly with --ignored"]
fn native_lexical_substitution_replays_exactly_against_the_scoped_model() {
    let checker = RefinementChecker::new(RefinementConfig::default());
    let mut source = Text::from(include_str!(
        "../../../../verification/external/lean/VerumExternalReplay/RefinementSubstitution.lean"
    ));
    source.push_str(
        "\nopen Verum.RefinementSubstitution (Algebra)\nopen Verum.RefinementSubstitution.Scoped\n",
    );
    for (index, argument) in ["7", "outside", "it + outside", "__refinement_local_0"]
        .iter()
        .enumerate()
    {
        let value = parse(argument);
        writeln!(source, "def scopedReplacement{index} (name : String) : Term 0 := if name == \"it\" then {} else .var name", scoped_term(&value, &[])).unwrap();
        for (case, input) in [
            "{let it = it; it}",
            "{let outside = 0; it}",
            "{let outside = 0; {let outside = 1; it + outside} + outside}",
            "{let it = 3; it}",
            "{let outside = it; it + outside}",
            "{let outside = it; let it = outside + it; it}",
            "{let __refinement_local_0 = 2; it + __refinement_local_0}",
            "{let outside = {let it = it; it}; it + outside}",
        ]
        .iter()
        .enumerate()
        {
            let original = parse(input);
            let actual = checker
                .substitute_all_in_expr(&original, &[("it".into(), &value)])
                .expect("checked substitution");
            writeln!(source, "def original{index}_{case} : Term 0 := {}\ndef actual{index}_{case} : Term 0 := {}", scoped_term(&original, &[]), scoped_term(&actual, &[])).unwrap();
            writeln!(source, "theorem projection{index}_{case} : actual{index}_{case} = substitute scopedReplacement{index} original{index}_{case} := by rfl").unwrap();
            writeln!(source, "example (algebra : Algebra Value) (env : String → Value) :\n  eval algebra env Fin.elim0 actual{index}_{case} = eval algebra (fun name => eval algebra env Fin.elim0 (scopedReplacement{index} name)) Fin.elim0 original{index}_{case} := by\n  rw [projection{index}_{case}]\n  exact sound _ _ _ _ _").unwrap();
        }
    }
    assert_lean(&source, "Verum.RefinementSubstitution.Scoped.sound");
}

#[test]
#[ignore = "requires the pinned Lean toolchain; run explicitly with --ignored"]
fn native_predicate_transport_and_subject_instantiation_replay_as_distinct_scopes() {
    use super::{RefinementBinding, RefinementPredicate};
    let checker = RefinementChecker::new(RefinementConfig::default());
    let mut source = Text::from(include_str!(
        "../../../../verification/external/lean/VerumExternalReplay/RefinementSubstitution.lean"
    ));
    source.push_str(
        "\nopen Verum.RefinementSubstitution (Algebra)\nopen Verum.RefinementSubstitution.Scoped\n",
    );
    let mut index = 0;
    for (binding, body) in [
        (RefinementBinding::Inline, "it + self + a"),
        (
            RefinementBinding::ValueParameter("n".into()),
            "n + it + self + a",
        ),
        (RefinementBinding::Lambda("subject".into()), "subject + a"),
        (
            RefinementBinding::Sigma("subject".into()),
            "{let outside = 2; subject + a + outside}",
        ),
    ] {
        let expr = parse(body);
        let mut predicate = RefinementPredicate::inline(expr.clone(), expr.span);
        predicate.binding = binding;
        let aliases: List<_> = predicate.subject_names().map(Text::from).collect();
        for argument in ["it", "self", "subject", "it + self + outside"] {
            let value = parse(argument);
            let transported = predicate
                .substitute_parameters(&[("a".into(), &value)])
                .expect("free parameter transport");
            let target = parse("it + subject + 7");
            let vc = checker
                .generate_vc(&target, &transported)
                .expect("subject instantiation");
            let transported_aliases: List<_> =
                transported.subject_names().map(Text::from).collect();
            writeln!(source, "def original{index} : Term 1 := {}\ndef actual{index} : Term 1 := {}\ndef argument{index} : Term 0 := {}\ndef target{index} : Term 0 := {}\ndef vc{index} : Term 0 := {}",
                scoped_term_with_aliases(&predicate.predicate, &[predicate.bound_variable()], &aliases),
                scoped_term_with_aliases(&transported.predicate, &[transported.bound_variable()], &transported_aliases),
                scoped_term(&value, &[]), scoped_term(&target, &[]), scoped_term(&vc.condition, &[])).unwrap();
            writeln!(source, "def replacement{index} (name : String) : Term 0 := if name == \"a\" then argument{index} else .var name\ntheorem projection{index} : actual{index} = substitute replacement{index} original{index} := by rfl").unwrap();
            writeln!(source, "theorem instantiated{index} (algebra : Algebra Value) (env : String → Value) :\n  eval algebra env Fin.elim0 vc{index} = eval algebra env (extend (eval algebra env Fin.elim0 target{index}) Fin.elim0) actual{index} := by rfl").unwrap();
            writeln!(source, "example (algebra : Algebra Value) (env : String → Value) :\n  eval algebra env Fin.elim0 vc{index} = eval algebra (fun name => eval algebra env Fin.elim0 (replacement{index} name)) (extend (eval algebra env Fin.elim0 target{index}) Fin.elim0) original{index} := by\n  rw [instantiated{index}, projection{index}]\n  exact sound _ _ _ _ _").unwrap();
            index += 1;
        }
    }
    assert_lean(&source, "Verum.RefinementSubstitution.Scoped.sound");
}
