//! T1629: declaration-owned Self must not alias the global inference counter.
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::{infer::TypeChecker, ty::TypeVar};

fn errors(source: &str) -> List<Text> {
    let module = Parser::new(source)
        .parse_module()
        .expect("valid test source");
    let mut checker = TypeChecker::new();
    let mut errors = List::new();
    for item in &module.items {
        if let verum_ast::ItemKind::Type(declaration) = &item.kind {
            if let Err(error) = checker.register_type_declaration(declaration) {
                errors.push(Text::from(format!("{error:?}")));
            }
        }
    }
    for item in &module.items {
        if let Err(error) = checker.check_item(item) {
            errors.push(Text::from(format!("{error:?}")));
        }
    }
    errors.extend(
        checker
            .diagnostics()
            .iter()
            .filter(|d| d.is_error())
            .map(|d| Text::from(format!("{d:?}"))),
    );
    errors
}

#[test]
fn default_self_is_independent_of_prior_type_variable_allocation() {
    const CHILD: &str = "VERUM_T1629_CHILD";
    if let Ok(case) = std::env::var(CHILD) {
        assert_eq!(
            TypeVar::peek_counter(),
            0,
            "control requires a fresh process"
        );
        let (warmup, case) = case.split_once(':').unwrap();
        for _ in 0..warmup.parse::<usize>().unwrap() {
            TypeVar::fresh();
        }
        let (source, expected) = match case {
            "inferred" => (
                "fn d<T>()->Int where T: Default { let x = T.default(); 1 }",
                "clean",
            ),
            "annotated" => (
                "fn d<T>()->Int where T: Default { let x: T = T.default(); 1 }",
                "clean",
            ),
            "return" => (
                "fn d<T>()->T where T: Default { let x = T.default(); x }",
                "clean",
            ),
            "cross_method" => (
                r#"
type M<T> is None | Some(T);
implement<T> M<T> {
    public fn d(self)->T where T: Default { T.default() }
    public fn take(&mut self)->M<T> { let old = *self; *self = None; old }
}
"#,
                "clean",
            ),
            "unbound" => (
                "type Default is protocol { fn default()->Self; }; fn d()->Int { let x = Default.default(); 1 }",
                "ambiguous",
            ),
            "missing" => (
                "fn d<T>()->Int where T: Default { let x = T.missing(); 1 }",
                "missing",
            ),
            _ => panic!("unknown child control"),
        };
        let observed = errors(source);
        match expected {
            "clean" => assert!(observed.is_empty(), "{case}: all errors = {observed:?}"),
            "ambiguous" => assert!(
                observed
                    .iter()
                    .any(|e| e.contains("E404") || e.contains("Ambiguous")),
                "{case}: expected real ambiguity, got {observed:?}"
            ),
            "missing" => assert!(
                !observed.is_empty(),
                "{case}: unknown member must be rejected, got {observed:?}"
            ),
            _ => unreachable!(),
        }
        return;
    }
    let executable = std::env::current_exe().unwrap();
    let mut failures = List::new();
    for warmup in [0, 1, 32] {
        for case in [
            "inferred",
            "annotated",
            "return",
            "cross_method",
            "unbound",
            "missing",
        ] {
            let output = std::process::Command::new(&executable)
                .args([
                    "--exact",
                    "default_self_is_independent_of_prior_type_variable_allocation",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env(CHILD, format!("{warmup}:{case}"))
                .env("RUST_MIN_STACK", "16777216")
                .output()
                .expect("run isolated checker control");
            if !output.status.success() {
                failures.push(Text::from(format!(
                    "warmup={warmup} case={case}\n{}\n{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                )));
            }
        }
    }
    assert!(failures.is_empty(), "fresh-process failures: {failures:#?}");
}
