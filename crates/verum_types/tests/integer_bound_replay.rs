use std::{fmt::Write, path::Path, process::Command};

use verum_common::Text;
use verum_types::refinement_entailment::{BoundImplication, IntegerComparison, Relation};

#[test]
#[ignore = "requires the pinned Lean toolchain; run explicitly with --ignored"]
fn native_integer_bound_rules_correspond_to_the_proved_lean_model() {
    let model = include_str!(
        "../../../verification/external/lean/VerumExternalReplay/IntegerBoundEntailment.lean"
    );
    let mut source = Text::from(model);
    source.push_str("\nopen Verum.IntegerBoundEntailment\n");
    let relations = [
        (Relation::Eq, "eq"),
        (Relation::Ne, "ne"),
        (Relation::Lt, "lt"),
        (Relation::Le, "le"),
        (Relation::Gt, "gt"),
        (Relation::Ge, "ge"),
    ];
    let bounds = [
        i128::MIN,
        i128::MIN + 1,
        -3,
        -1,
        0,
        1,
        3,
        i128::MAX - 1,
        i128::MAX,
    ];
    for (source_relation, source_name) in relations {
        for (target_relation, target_name) in relations {
            for a in bounds {
                for b in bounds {
                    let accepted = BoundImplication {
                        source: IntegerComparison {
                            relation: source_relation,
                            bound: a,
                        },
                        target: IntegerComparison {
                            relation: target_relation,
                            bound: b,
                        },
                    }
                    .check();
                    // Check refusals as well as acceptances against the exact
                    // model conditions. General soundness for all mathematical
                    // bounds and values is proved in the included Lean module.
                    writeln!(source,
                        "example : {} (Accepts .{source_name} ({a}) .{target_name} ({b})) := by\n  unfold Accepts\n  decide",
                        if accepted { "" } else { "¬" }).unwrap();
                }
            }
        }
    }
    // Exercise the source projection as well as the arithmetic rule. Names
    // belong to distinct declaration frames; each alias must denote that
    // frame's subject before the native decision is replayed over Lean Int.
    use verum_ast::{BinOp, Expr, ExprKind, Ident, Literal, Span};
    use verum_common::Heap;
    use verum_types::{
        Type,
        refinement::{RefinementBinding, RefinementPredicate},
    };
    fn predicate(owner: &str, spelling: &str, bound: i128) -> RefinementPredicate {
        let span = Span::dummy();
        let mut predicate = RefinementPredicate::inline(
            Expr::new(
                ExprKind::Binary {
                    op: BinOp::Gt,
                    left: Heap::new(Expr::ident(Ident::new(spelling, span))),
                    right: Heap::new(Expr::literal(Literal::int(bound, span))),
                },
                span,
            ),
            span,
        );
        predicate.binding = RefinementBinding::ValueParameter(owner.into());
        predicate
    }
    for source_name in ["n", "it", "self"] {
        for target_name in ["m", "it", "self"] {
            for a in [-3, -1, 0, 1, 3] {
                for b in [-3, -1, 0, 1, 3] {
                    let accepted = verum_types::refinement_entailment::entails(
                        &Type::Int,
                        &predicate("n", source_name, a),
                        &Type::Int,
                        &predicate("m", target_name, b),
                    );
                    assert_eq!(
                        accepted,
                        a >= b,
                        "{source_name} > {a} => {target_name} > {b}"
                    );
                    writeln!(source,
                        "example : {} (Accepts .gt ({a}) .gt ({b})) := by\n  unfold Accepts\n  decide",
                        if accepted { "" } else { "¬" }).unwrap();
                }
            }
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("IntegerBounds.lean");
    std::fs::write(&path, source.as_bytes()).unwrap();
    let result = Command::new(std::env::var_os("VERUM_LEAN").unwrap_or_else(|| "lean".into()))
        .arg(&path)
        .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../verification/external/lean"))
        .output()
        .expect("the pinned Lean toolchain must be installed");
    let stdout = std::string::String::from_utf8_lossy(&result.stdout);
    let stderr = std::string::String::from_utf8_lossy(&result.stderr);
    assert!(result.status.success(), "{stdout}\n{stderr}");
    assert!(
        stdout.contains("Verum.IntegerBoundEntailment.sound"),
        "{stdout}"
    );
    for forbidden in ["sorryAx", "Lean.ofReduceBool", "warning:", "error:"] {
        assert!(
            !stdout.contains(forbidden) && !stderr.contains(forbidden),
            "{stdout}\n{stderr}"
        );
    }
}
