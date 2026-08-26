//! Which modules does `check_project` actually type-check?
//!
//! `verum check` on a PROJECT reports zero errors for a file that
//! `verum check <that file>` refuses with `E400`. One source, two
//! commands, two verdicts — and the project form is the one every real
//! program uses. This prints what the project path sees, so the loss can
//! be read off rather than inferred.
//!
//! Usage: `dump_check_project_reach <project-root>`

use std::path::PathBuf;
use verum_compiler::{CompilationPipeline, CompilerOptions, Session};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: dump_check_project_reach <project-root>")?;

    let options = CompilerOptions {
        input: root.join("src"),
        output: std::env::temp_dir().join("verum-check-reach-out"),
        ..Default::default()
    };
    let mut session = Session::new(options);
    let mut pipeline = CompilationPipeline::new_check(&mut session);

    let result = pipeline.check_project()?;

    // What the project path actually holds, per module: the item kinds
    // at top level, and how many items a `module X;` wrapper absorbed.
    for (path, module) in pipeline.loaded_modules_for_testing() {
        let kinds: Vec<String> = module
            .items
            .iter()
            .map(|i| match &i.kind {
                verum_ast::ItemKind::Function(f) => format!("fn {}", f.name.name),
                verum_ast::ItemKind::Module(m) => format!(
                    "module {} [{} items]",
                    m.name.name,
                    m.items.as_ref().map_or(0, |v| v.len())
                ),
                verum_ast::ItemKind::Type(t) => format!("type {}", t.name.name),
                verum_ast::ItemKind::Mount(_) => "mount".to_string(),
                other => format!("{other:?}").chars().take(18).collect(),
            })
            .collect();
        println!("module {path}: {}", kinds.join(", "));
    }

    println!("files_checked   = {}", result.files_checked);
    println!("user_errors     = {}", result.user_errors);
    println!("warnings        = {}", result.warnings);
    println!("types_inferred  = {}", result.types_inferred);
    println!("session errors  = {}", session.error_count());
    let _ = session.display_diagnostics();
    Ok(())
}
