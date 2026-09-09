//! `@test_case` rows must run on the tier `verum test` uses by DEFAULT
//! (T1340).
//!
//! Discovery expanded the table correctly — `verum test --list` printed
//! `probe::add_table[0]` … `[3]` — and Tier 0 ran all four green. Tier 1,
//! the default, could not compile a single one: the synthetic `main` the
//! AOT path generates called the function with an empty argument list.
//!
//! ```text
//! error<E102>: Function requires at least 3 arguments, got 0
//!              (calling `add_table`)
//!   --> …/target/test/test_probe_<hash>.merged.vr:27:5
//!    │
//! 26 │ public fn main() -> Int {
//! 27 │     add_table();
//! ```
//!
//! So a reader who copied the worked example from
//! `website:docs/tooling/testing.md` and ran `verum test` got four
//! compile errors inside a file they never wrote, and the tier that
//! worked was the one they had to know to ask for.
//!
//! This gate runs the page's example through the real binary with no
//! tier flag — the reader's command — and requires four passes.

use std::fs;
use std::process::Command;

/// The website's example, verbatim.
const PROBE: &str = r#"
@test
@test_case(0, 0, 0)
@test_case(1, 2, 3)
@test_case(-5, 5, 0)
@test_case(100, -50, 50)
fn add_table(a: Int, b: Int, expected: Int) {
    assert_eq(a + b, expected);
}
"#;

const MANIFEST: &str = "[cog]\nname = \"t1340_probe\"\nversion = \"0.1.0\"\n";

/// One directory PER TEST. The first version keyed the fixture on the
/// process id alone, so both tests shared a directory and the one that
/// finished first deleted it under the other — `found 0` merged
/// sources, from a fixture that had been removed rather than from a
/// defect. Cargo runs integration tests in parallel threads of ONE
/// process, so a pid is not a unique name.
fn fixture(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("t1340_probe_{}_{}", std::process::id(), tag));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("tests")).expect("fixture dirs");
    fs::write(dir.join("Verum.toml"), MANIFEST).expect("manifest");
    fs::write(dir.join("tests/probe.vr"), PROBE).expect("probe source");
    dir
}

#[test]
fn the_four_rows_pass_with_no_tier_flag() {
    let dir = fixture("rows_pass");
    let out = Command::new(env!("CARGO_BIN_EXE_verum"))
        .args(["test", "--filter", "add_table"])
        .current_dir(&dir)
        .output()
        .expect("`verum test` must run");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    assert!(
        !text.contains("got 0"),
        "the synthetic main dropped the @test_case row's arguments — the \
         wrapper is calling the function with none (T1340).\n{}",
        text
    );
    assert!(
        text.contains("4 passed") && text.contains("0 failed"),
        "the four @test_case rows must pass on the DEFAULT tier — this is \
         the command a reader of the docs runs.\n{}",
        text
    );

    let _ = fs::remove_dir_all(&dir);
}

/// Each row must compile to its OWN binary. All four share a file and a
/// function name, so the merge/binary key has to include the arguments
/// or the four parallel workers write over one another — the exact
/// collision `unique_merged_stem` was introduced to prevent, reached
/// through a door that did not exist when it was written.
#[test]
fn each_row_gets_its_own_merged_source() {
    let dir = fixture("own_merged");
    let _ = Command::new(env!("CARGO_BIN_EXE_verum"))
        .args(["test", "--filter", "add_table"])
        .current_dir(&dir)
        .output()
        .expect("`verum test` must run");

    let merged: Vec<_> = fs::read_dir(dir.join("target/test"))
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .filter(|n| n.ends_with(".merged.vr"))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    assert!(
        merged.len() >= 4,
        "expected one merged source per @test_case row, found {}: {:?}",
        merged.len(),
        merged
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------
// The renderer's edges. `grammar/verum.ebnf:212` says
// `float_lit = decimal_lit , '.' , decimal_lit , [ exponent ]` — the dot
// with digits on both sides is required and the exponent is only legal
// after one. Rust's `{:?}` gives `1e300` and `inf`, neither of which the
// Verum lexer reads, and the result would land in a generated file the
// author never opens.
// ---------------------------------------------------------------------

use verum_cli::commands::property::TreeValue;
use verum_cli::commands::test::{case_args_to_verum_literals, tree_value_to_verum_literal};

fn int(v: i64) -> TreeValue {
    TreeValue::Int { value: v, lo: i64::MIN, hi: i64::MAX }
}

#[test]
fn a_float_literal_always_carries_a_dot() {
    assert_eq!(tree_value_to_verum_literal(&TreeValue::Float(1.0)).as_deref(), Some("1.0"));
    assert_eq!(tree_value_to_verum_literal(&TreeValue::Float(-0.5)).as_deref(), Some("-0.5"));
    let big = tree_value_to_verum_literal(&TreeValue::Float(1e300)).expect("finite renders");
    assert!(
        big.contains('.') && big.contains('e'),
        "an exponent literal needs a mantissa with a dot to satisfy \
         `decimal_lit '.' decimal_lit [exponent]`; got {:?}",
        big
    );
    let small = tree_value_to_verum_literal(&TreeValue::Float(1.5e-320)).expect("finite renders");
    assert!(small.contains('.'), "got {:?}", small);
}

#[test]
fn a_non_finite_float_is_refused_rather_than_emitted() {
    for v in [f64::INFINITY, f64::NEG_INFINITY, f64::NAN] {
        assert!(
            tree_value_to_verum_literal(&TreeValue::Float(v)).is_none(),
            "`inf` / `NaN` are not Verum literals — emitting one would put \
             an unlexable token in the generated main"
        );
    }
}

#[test]
fn an_absent_row_renders_an_empty_argument_list() {
    assert_eq!(case_args_to_verum_literals(None).as_deref(), Some(""));
}

#[test]
fn a_row_renders_in_order_with_negatives_intact() {
    let row = [int(-5), int(5), int(0)];
    assert_eq!(case_args_to_verum_literals(Some(&row)).as_deref(), Some("-5, 5, 0"));
}

#[test]
fn one_unrenderable_argument_refuses_the_whole_row() {
    let row = [int(1), TreeValue::Float(f64::NAN)];
    assert!(
        case_args_to_verum_literals(Some(&row)).is_none(),
        "a row must be all-or-nothing: half a row rendered into a call is \
         worse than no synthesis, because it compiles into the wrong test"
    );
}
