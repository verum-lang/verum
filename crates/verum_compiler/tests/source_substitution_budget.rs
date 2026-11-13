//! Source type limits are diagnostics and cannot publish a partial archive.
use std::fs;
use verum_common::Text;
use verum_compiler::{CompilationPipeline, CompilerOptions, CoreConfig, Session};

#[test]
fn a_let_type_exceeding_the_substitution_budget_cannot_publish_an_archive() {
    for depth in [32, 64] {
        let root = tempfile::tempdir().unwrap();
        let core = root.path().join("core");
        fs::create_dir(&core).unwrap();
        let mut source = Text::from(
            "module core; type Sample is {ptr:&Int,total:Int}; public fn forward(value:Sample)->Sample{",
        );
        let mut previous = Text::from("value");
        for index in 0..depth {
            let current: Text = format!("alias{index}").into();
            source.push_str(&format!("let {current}=&{previous};"));
            previous = current;
        }
        source.push_str(&format!(
            "Sample{{ptr:{previous}.ptr,total:{previous}.total}}}}"
        ));
        let input = core.join("mod.vr");
        fs::write(&input, source.as_bytes()).unwrap();
        let output = root.path().join("invalid.vbca");
        let mut session = Session::new(CompilerOptions {
            input,
            ..Default::default()
        });
        let result = CompilationPipeline::new_core(
            &mut session,
            CoreConfig::new(&core).with_output(&output),
        )
        .compile_core();
        let error = result.expect_err("the existing substitution limit must remain enforced");
        assert!(
            format!("{error:#}").contains("structural depth budget"),
            "depth {depth}: {error:#}"
        );
        assert!(
            !output.exists(),
            "refused source published an archive at depth {depth}"
        );
    }
}
