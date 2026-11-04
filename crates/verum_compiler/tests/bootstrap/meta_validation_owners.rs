//! Bootstrap meta lookup follows each source owner, not its archive group.
use super::*;
use crate::Session;
use verum_common::Text;
use verum_fast_parser::Parser;

const CHILD: &str = "VERUM_TEST_META_VALIDATION_OWNERS_CHILD";

#[test]
fn meta_validation_uses_each_file_owner_and_reports_its_real_failure() {
    if std::env::var_os(CHILD).is_none() {
        // Configure the child process only. No process-global environment
        // mutation races with the other tests in this executable.
        let thread = std::thread::current();
        let test_name = thread.name().expect("libtest supplies the exact test name");
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test_name, "--nocapture", "--test-threads=1"])
            .env(CHILD, "1")
            .env("VERUM_BAKE_VALIDATION", "verify")
            .env_remove("VERUM_META_ENGINE")
            .output()
            .expect("launch isolated meta-validation test");
        assert!(
            output.status.success(),
            "child status {:?}\nstdout:\n{}\nstderr:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("T2281_META_OWNERS_CHECKED"),
            "child must execute the controls: {stdout}"
        );
        return;
    }

    assert_eq!(
        std::env::var("VERUM_BAKE_VALIDATION").as_deref(),
        Ok("verify")
    );
    const GROUP: &str = "fixture.archive_bundle";
    const GOOD: &str = "fixture.lexical.good";
    const BAD: &str = "fixture.lexical.bad";
    let good = Parser::new(
        "meta fn helper(value: Int) -> Int { value + 41 } meta fn probe() -> Int { helper(1) }",
    )
    .parse_module()
    .unwrap();
    let bad = Parser::new("meta fn helper(value: Int) -> Int { value / (value - value) } meta fn probe() -> Int { helper(1) }").parse_module().unwrap();
    // The group has a colliding helper which must never replace GOOD's helper.
    // Parameterization prevents phase_meta_evaluation independently evaluating
    // helper: every observed division failure must come through probe's lookup.
    let foreign = Parser::new("meta fn helper(value: Int) -> Int { value / (value - value) }")
        .parse_module()
        .unwrap();
    for reversed_registration in [false, true] {
        let mut session = Session::new(Default::default());
        let mut pipeline = CompilationPipeline::new_core(&mut session, CoreConfig::new("."));
        let mut declarations = [(GOOD, &good), (BAD, &bad), (GROUP, &foreign)];
        if reversed_registration {
            declarations.reverse();
        }
        for (owner, module) in declarations {
            pipeline
                .register_meta_declarations(&Text::from(owner), module)
                .unwrap();
        }
        assert_eq!(pipeline.session.error_count(), 0);
        pipeline
            .run_user_validation_phases(GROUP, &[(GOOD, &good)])
            .unwrap();
        assert_eq!(
            pipeline.session.error_count(),
            0,
            "the correct file owner's successful helper must win over the archive group's failing helper"
        );

        let refusal = pipeline
            .run_user_validation_phases(GOOD, &[(BAD, &bad)])
            .unwrap_err();
        assert!(refusal.to_string().contains("meta failures"), "{refusal:#}");
        assert!(
            pipeline.session.error_count() > 0,
            "a real failure in the selected file must be reported even when the aggregate group names a successful module"
        );
    }
    println!("T2281_META_OWNERS_CHECKED");
}
