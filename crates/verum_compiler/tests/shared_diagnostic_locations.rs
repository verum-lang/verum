use std::fs;
use verum_compiler::{CompilationPipeline, CompilerOptions, CoreConfig, Session};

#[test]
fn admission_groups_uses_of_the_same_declaration_and_keeps_each_error() {
    check_declarations("", "public type Cell is { value: Int };", 2);
}

#[test]
fn inline_declarations_keep_their_owning_files_and_original_source_ranges() {
    check_declarations(
        ".nested",
        "public module nested {\npublic type Cell is { value: Int };\n}",
        3,
    );
}

fn check_declarations(scope: &str, declaration: &str, line: usize) {
    let project = tempfile::tempdir().unwrap();
    let core = project.path().join("core");
    fs::create_dir(&core).unwrap();
    fs::write(core.join("mod.vr"), "module core; public module model; public module other; public module left; public module right; public module third;").unwrap();
    for name in ["model", "other"] {
        fs::write(
            core.join(format!("{name}.vr")),
            format!("module core.{name};\n{declaration}\n"),
        )
        .unwrap();
    }
    for (name, source) in [("left", "model"), ("right", "model"), ("third", "other")] {
        fs::write(
            core.join(format!("{name}.vr")),
            format!("module core.{name};\nmount core.{source}{scope}.Cell;\npublic fn read(cell: &Cell) -> Int {{ cell.value }}"),
        ).unwrap();
    }
    let output = project.path().join("runtime.vbca");
    let mut serial = None;
    for num_threads in [1, 4] {
        let mut session = Session::new(CompilerOptions {
            input: core.join("mod.vr"),
            num_threads,
            ..Default::default()
        });
        let error = CompilationPipeline::new_core(
            &mut session,
            CoreConfig::new(&core).with_output(&output),
        )
        .compile_core()
        .expect_err("private fields were admitted")
        .to_string();
        assert_eq!(session.error_count(), 3, "{error}");
        assert!(!output.exists());
        let groups: Vec<_> = error
            .lines()
            .filter(|line| line.contains("diagnostics in"))
            .collect();
        assert_eq!(groups.len(), 1, "{error}");
        assert!(
            groups[0].contains(&format!("model.vr:{line}:"))
                && groups[0].contains("E202")
                && groups[0].contains("2 diagnostics in 2 modules"),
            "{error}"
        );
        for file in ["left", "right", "third"] {
            assert!(error.contains(&format!("{file}.vr:3:")), "{error}");
        }
        if let Some(previous) = &serial {
            assert_eq!(previous, &error);
        } else {
            serial = Some(error);
        }
    }
}
