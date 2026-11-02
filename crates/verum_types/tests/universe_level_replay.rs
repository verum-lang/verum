//! Replay actual native normal forms and order decisions with independent Lean
//! semantics. No native success flag is treated as a proof of the proposition.

use std::{fmt::Write, path::Path, process::Command};

use verum_common::{List, Text};
use verum_types::universe_level::{NormalLevel, UniverseLevel};

enum Work<'a> {
    Visit(&'a UniverseLevel),
    Successor,
    Maximum,
}

fn emit_formula(output: &mut Text, level: &UniverseLevel, next: &mut usize) -> usize {
    let mut work = List::from_single(Work::Visit(level));
    let mut values = List::new();
    while let Some(item) = work.pop() {
        let expression = match item {
            Work::Visit(UniverseLevel::Concrete(value)) => format!(".concrete {value}"),
            Work::Visit(UniverseLevel::Variable(id)) => format!(".var {id}"),
            Work::Visit(UniverseLevel::Meta(_)) => panic!("unresolved inference cannot be exported as a universal proof"),
            Work::Visit(UniverseLevel::Succ(inner)) => {
                work.push(Work::Successor);
                work.push(Work::Visit(inner));
                continue;
            }
            Work::Visit(UniverseLevel::Max(left, right)) => {
                work.push(Work::Maximum);
                work.push(Work::Visit(right));
                work.push(Work::Visit(left));
                continue;
            }
            Work::Successor => format!(".succ e{}", values.pop().unwrap()),
            Work::Maximum => {
                let right = values.pop().unwrap();
                let left = values.pop().unwrap();
                format!(".max e{left} e{right}")
            }
        };
        writeln!(output, "private def e{next} : Formula := {expression}").unwrap();
        values.push(*next);
        *next += 1;
    }
    values.pop().unwrap()
}

fn emit_normal(output: &mut Text, name: &str, level: &NormalLevel) {
    write!(
        output,
        "private def {name} : NormalLevel := ⟨{}, [",
        level.constant()
    )
    .unwrap();
    for (index, (variable, offset)) in level.terms().iter().enumerate() {
        if index != 0 {
            output.push_str(", ");
        }
        write!(output, "({variable}, {offset})").unwrap();
    }
    output.push_str("]⟩\n");
}

fn replay_case(
    case: usize,
    source: &UniverseLevel,
    target: &UniverseLevel,
    strict: bool,
) -> Option<Text> {
    let left = source.normalize()?;
    let right = target.normalize()?;
    let accepted = if strict {
        source.is_less_than(target)
    } else {
        source.is_less_or_equal(target)
    };
    let mut output = Text::new();
    writeln!(output, "\nnamespace Verum.UniverseLevelsReplay.Case{case}").unwrap();
    output.push_str("open Verum.UniverseLevels\n");
    let mut next = 0;
    let source_id = emit_formula(&mut output, source, &mut next);
    let target_id = emit_formula(&mut output, target, &mut next);
    emit_normal(&mut output, "left", &left);
    emit_normal(&mut output, "right", &right);
    writeln!(
        output,
        "theorem sourceNormalizes : Normalizes e{source_id} left = true := by decide\n\
         theorem targetNormalizes : Normalizes e{target_id} right = true := by decide\n\
         theorem leftMinimum : minimum left = {} := by decide\n\
         theorem rightMinimum : minimum right = {} := by decide",
        left.minimum(),
        right.minimum()
    )
    .unwrap();
    let left_term = if strict { "(successor left)" } else { "left" };
    writeln!(
        output,
        "theorem orderAgrees : Below {left_term} right = {accepted} := by decide"
    )
    .unwrap();
    if accepted {
        let relation = if strict { "<" } else { "≤" };
        let theorem = if strict { "checked_lt" } else { "checked_le" };
        writeln!(
            output,
            "theorem verifiedQuery : ∀ valuation, denote valuation e{source_id} {relation} \
             denote valuation e{target_id} :=\n  \
             {theorem} sourceNormalizes targetNormalizes orderAgrees\n\
             #print axioms verifiedQuery"
        )
        .unwrap();
    }
    writeln!(
        output,
        "#print axioms sourceNormalizes\n#print axioms orderAgrees\n\
         end Verum.UniverseLevelsReplay.Case{case}"
    )
    .unwrap();
    Some(output)
}

fn lean(path: &Path) -> std::process::Output {
    Command::new(std::env::var_os("VERUM_LEAN").unwrap_or_else(|| "lean".into()))
        .arg(path)
        .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../verification/external/lean"))
        .output()
        .expect("Lean must be installed for this explicitly requested independent replay")
}

#[test]
fn replay_uses_actual_structural_levels_and_native_normal_forms() {
    let variable = UniverseLevel::variable(4_000_000_000);
    let overflow = UniverseLevel::Concrete(u32::MAX).succ();
    let source = UniverseLevel::Max(Box::new(variable.succ()), Box::new(overflow));
    let target = source.succ();
    let replay = replay_case(0, &source, &target, true).unwrap();
    assert!(replay.contains(".var 4000000000"));
    assert!(replay.contains(".concrete 4294967295"));
    assert!(replay.contains(".succ e"));
    assert!(replay.contains(".max e"));
    assert!(replay.contains("⟨4294967296, [(4000000000, 1)]⟩"));
    assert!(replay.contains("checked_lt sourceNormalizes targetNormalizes orderAgrees"));
    assert!(replay.contains("Below (successor left) right = true"));
}

#[test]
#[ignore = "requires the pinned Lean toolchain; run explicitly with --ignored"]
fn independent_lean_checks_native_normalization_and_universal_order() {
    let a = UniverseLevel::variable(17);
    let b = UniverseLevel::variable(4_000_000_000);
    let a_succ = a.succ();
    let b_succ = b.succ();
    let maximum = a.max(&b);
    let distributed_successor = a_succ.max(&b_succ);
    let repeated = UniverseLevel::Max(
        Box::new(a_succ.clone()),
        Box::new(UniverseLevel::Max(
            Box::new(a.clone()),
            Box::new(a_succ.succ()),
        )),
    );
    let overflow = UniverseLevel::Concrete(u32::MAX).succ();
    let cases = [
        (a.clone(), a.clone(), false, true),
        (a.clone(), a_succ.clone(), true, true),
        (a.clone(), b.clone(), false, false),
        (b.clone(), a.clone(), false, false),
        (a_succ.clone(), a.clone(), false, false),
        (a.max(&b_succ), distributed_successor.clone(), false, true),
        (maximum.succ(), distributed_successor.clone(), false, true),
        (distributed_successor, maximum.succ(), false, true),
        (UniverseLevel::Concrete(1), a_succ.clone(), false, true),
        (UniverseLevel::Concrete(2), a_succ.clone(), false, false),
        (overflow.clone(), overflow.succ(), true, true),
        (repeated, a_succ.succ(), false, true),
        (
            UniverseLevel::Concrete(0),
            UniverseLevel::Concrete(0),
            true,
            false,
        ),
    ];
    let mut text = Text::from(include_str!(
        "../../../verification/external/lean/VerumExternalReplay/UniverseLevels.lean"
    ));
    text.push_str(
        "\n#print axioms Verum.UniverseLevels.normalize_sound\n\
         #print axioms Verum.UniverseLevels.below_sound\n\
         #print axioms Verum.UniverseLevels.checked_le\n\
         #print axioms Verum.UniverseLevels.checked_lt\n",
    );
    for (index, (source, target, strict, expected)) in cases.iter().enumerate() {
        let actual = if *strict {
            source.is_less_than(target)
        } else {
            source.is_less_or_equal(target)
        };
        assert_eq!(
            actual, *expected,
            "unexpected native decision for case {index}"
        );
        text.push_str(&replay_case(index, source, target, *strict).unwrap());
    }
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("UniverseLevelsReplay.lean");
    // Inline the current canonical model so a stale compiled library cannot
    // satisfy this independent check.
    std::fs::write(&path, text.as_bytes()).unwrap();
    let output = lean(&path);
    let stdout = std::string::String::from_utf8_lossy(&output.stdout);
    let stderr = std::string::String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stdout}\n{stderr}");
    assert!(stdout.contains("Verum.UniverseLevelsReplay.Case10.verifiedQuery"));
    for forbidden in ["sorryAx", "Lean.ofReduceBool", "warning:", "error:"] {
        assert!(
            !stdout.contains(forbidden) && !stderr.contains(forbidden),
            "{stdout}\n{stderr}"
        );
    }

    let changed_witness = text.replacen(
        "private def left : NormalLevel := ⟨0, [(17, 0)]⟩",
        "private def left : NormalLevel := ⟨0, [(18, 0)]⟩",
        1,
    );
    assert_ne!(changed_witness, text);
    std::fs::write(&path, changed_witness.as_bytes()).unwrap();
    assert!(
        !lean(&path).status.success(),
        "Lean accepted an unrelated variable in the normalization witness"
    );

    let changed_target = text.replacen(
        "denote valuation e0 ≤ denote valuation e1",
        "denote valuation e0 ≤ denote valuation (.concrete 0)",
        1,
    );
    assert_ne!(changed_target, text);
    std::fs::write(&path, changed_target.as_bytes()).unwrap();
    assert!(
        !lean(&path).status.success(),
        "Lean accepted a changed final target"
    );
}
