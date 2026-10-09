//! T1677: parsed project and source-module paths share forward type declarations.
//! These are compiler/checker controls, not runtime, AOT or registry acceptance.

use super::CompilationPipeline;
use crate::{CompilerOptions, Session, VerifyMode};
use std::path::PathBuf;
use tempfile::TempDir;
use verum_common::Text;
use verum_fast_parser::Parser;

const PAYLOAD: &str = "public type Payload is { value: Int };";
const SUM: &str = "public type Envelope is Empty | Data(Payload);";

struct Fixture {
    directory: TempDir,
}

struct Verdict {
    errors: usize,
    diagnostics: Text,
}

impl Fixture {
    fn new(files: &[(&str, &str)]) -> Self {
        let directory = tempfile::tempdir().expect("isolated project");
        std::fs::write(
            directory.path().join("Verum.toml"),
            "[cog]\nname = \"fixture\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        std::fs::create_dir(directory.path().join("src")).unwrap();
        for (name, source) in files {
            std::fs::write(directory.path().join("src").join(name), source).unwrap();
        }
        Self { directory }
    }

    fn entry(&self) -> PathBuf {
        self.directory.path().join("src/main.vr")
    }

    fn session(&self) -> Session {
        Session::new(CompilerOptions {
            input: self.entry(),
            output: self.directory.path().join("output"),
            verify_mode: VerifyMode::Runtime,
            check_only: true,
            ..Default::default()
        })
    }

    fn check(&self) -> Verdict {
        let mut session = self.session();
        let result = CompilationPipeline::new_check(&mut session)
            .check_project()
            .expect("project checker completes and retains diagnostics");
        Verdict {
            errors: result.user_errors,
            diagnostics: session.format_diagnostics().into(),
        }
    }
}

fn project(model: &str, imports: &str, body: &str) -> Verdict {
    let model = format!("module fixture.model;\n{model}");
    let main = format!(
        "module fixture.main;\nmount fixture.model.{{{imports}}};\n{body}\nfn main() -> Int {{ 0 }}"
    );
    Fixture::new(&[("model.vr", &model), ("main.vr", &main)]).check()
}

fn assert_clean(verdict: Verdict) {
    assert_eq!(verdict.errors, 0, "{}", verdict.diagnostics);
}

fn assert_missing(verdict: Verdict, name: &str) {
    assert!(verdict.errors > 0, "missing type was accepted");
    assert!(
        verdict.diagnostics.contains("E101") && verdict.diagnostics.contains(name),
        "{}",
        verdict.diagnostics
    );
}

fn source_phase(source: &str, project_module: bool) -> Verdict {
    let fixture = Fixture::new(&[("main.vr", source)]);
    let mut session = fixture.session();
    let module = Parser::new(source).parse_module().expect("valid source");
    let mut pipeline = CompilationPipeline::new_check(&mut session);
    let result = if project_module {
        pipeline.analyze_module(&Text::from("fixture.main"), &module)
    } else {
        pipeline.phase_type_check(&module)
    };
    let mut diagnostics: Text = pipeline.session.format_diagnostics().into();
    let mut errors = pipeline.session.error_count();
    if let Err(error) = result {
        errors = errors.max(1);
        diagnostics.push_str(&format!("\n{error:#}"));
    }
    Verdict {
        errors,
        diagnostics,
    }
}

#[test]
fn project_forward_tuple_payload_accepts_sum_only_mount() {
    assert_clean(project(
        &format!("{SUM}\n{PAYLOAD}"),
        "Envelope",
        "fn consume(value: Envelope) -> Int { 1 }",
    ));
}

#[test]
fn project_forward_tuple_payload_accepts_sum_then_payload_mount() {
    assert_clean(project(
        &format!("{SUM}\n{PAYLOAD}"),
        "Envelope, Payload",
        "fn wrap(value: Payload) -> Envelope { Envelope.Data(value) }",
    ));
}

#[test]
fn project_forward_tuple_payload_accepts_payload_then_sum_mount() {
    assert_clean(project(
        &format!("{SUM}\n{PAYLOAD}"),
        "Payload, Envelope",
        "fn wrap(value: Payload) -> Envelope { Envelope.Data(value) }",
    ));
}

#[test]
fn project_payload_before_sum_is_the_control() {
    assert_clean(project(
        &format!("{PAYLOAD}\n{SUM}"),
        "Envelope",
        "fn consume(value: Envelope) -> Int { 1 }",
    ));
}

#[test]
fn one_file_project_accepts_forward_tuple_payload() {
    let source = format!("module fixture.main;\n{SUM}\n{PAYLOAD}\nfn main() -> Int {{ 0 }}");
    assert_clean(Fixture::new(&[("main.vr", &source)]).check());
}

#[test]
fn standalone_checker_accepts_the_same_forward_payload() {
    assert_clean(source_phase(&format!("{SUM}\n{PAYLOAD}"), false));
}

#[test]
fn source_module_analysis_accepts_forward_tuple_payload() {
    assert_clean(source_phase(&format!("{SUM}\n{PAYLOAD}"), true));
}

#[test]
fn project_generic_forward_payload_keeps_arguments() {
    assert_clean(project(
        "public type Envelope<T> is Empty | Data(Payload<T>); public type Payload<T> is { value: T };",
        "Envelope, Payload",
        "fn wrap(value: Payload<Int>) -> Envelope<Int> { Envelope.Data(value) }",
    ));
}

#[test]
fn project_generic_forward_payload_rejects_wrong_argument() {
    let verdict = project(
        "public type Envelope<T> is Empty | Data(Payload<T>); public type Payload<T> is { value: T };",
        "Envelope, Payload",
        "fn wrong(value: Payload<Bool>) -> Envelope<Int> { Envelope.Data(value) }",
    );
    assert!(verdict.errors > 0, "wrong generic payload accepted");
    assert!(
        verdict.diagnostics.contains("E400"),
        "{}",
        verdict.diagnostics
    );
}

#[test]
fn project_inline_record_variant_accepts_forward_payload() {
    assert_clean(project(
        &format!("public type Envelope is Empty | Data {{ payload: Payload }};\n{PAYLOAD}"),
        "Envelope",
        "fn consume(value: Envelope) -> Int { 1 }",
    ));
}

#[test]
fn project_missing_tuple_payload_still_reports_e101() {
    assert_missing(project(SUM, "Envelope", ""), "Payload");
}

#[test]
fn source_module_missing_tuple_payload_still_reports_e101() {
    assert_missing(source_phase(SUM, true), "Payload");
}

// The strict oracle is retained: both the original and fixed checker accept
// this absent owner. Run separately with --ignored; T1677 is not full type-path
// validation, and T0811 remains open for the qualified-path defect.
#[test]
#[ignore = "T0811: unresolved qualified type paths are not validated"]
fn project_absent_qualified_owner_does_not_borrow_local_payload() {
    assert_missing(
        project(
            &format!("{PAYLOAD}\npublic type Envelope is Empty | Data(absent.Payload);"),
            "Envelope",
            "",
        ),
        "Payload",
    );
}

#[test]
fn project_same_shaped_foreign_payload_remains_distinct() {
    let model = format!("module fixture.model;\n{SUM}\n{PAYLOAD}");
    let foreign = format!("module fixture.foreign;\n{PAYLOAD}");
    let main = "module fixture.main; mount fixture.model.{Envelope}; mount fixture.foreign.{Payload as ForeignPayload}; fn wrong(value: ForeignPayload) -> Envelope { Envelope.Data(value) } fn main() -> Int { 0 }";
    let verdict = Fixture::new(&[
        ("model.vr", &model),
        ("foreign.vr", &foreign),
        ("main.vr", main),
    ])
    .check();
    assert!(verdict.errors > 0, "foreign nominal payload accepted");
    assert!(
        verdict.diagnostics.contains("E400"),
        "{}",
        verdict.diagnostics
    );
}

#[test]
fn project_private_payload_cannot_be_explicitly_mounted() {
    let verdict = project(
        "public type Envelope is Empty | Data(Payload); type Payload is { value: Int };",
        "Envelope, Payload",
        "",
    );
    assert!(verdict.errors > 0, "private payload became public");
    assert!(
        verdict.diagnostics.contains("E401"),
        "{}",
        verdict.diagnostics
    );
}
