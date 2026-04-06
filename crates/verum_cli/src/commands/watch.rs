// Watch command - rebuild on file changes

use crate::error::Result;
use crate::ui;
use crossbeam_channel::unbounded;
use notify::{Event, RecursiveMode, Watcher};

pub fn execute(command: &str, clear: bool) -> Result<()> {
    ui::step(&format!("Watching for changes (running '{}')", command));

    let (tx, rx) = unbounded();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
        if let Ok(event) = res {
            let _ = tx.send(event);
        }
    })?;

    let manifest_dir = crate::config::Manifest::find_manifest_dir()?;
    watcher.watch(&manifest_dir.join("src"), RecursiveMode::Recursive)?;

    ui::info("Watching for file changes... (Ctrl+C to stop)");

    for _event in rx {
        if clear {
            print!("\x1B[2J\x1B[1;1H"); // Clear screen
        }

        ui::step("Files changed, rebuilding...");

        let result = match command {
            "build" => crate::commands::build::execute(
                None,  // profile_name
                None,  // refs
                None,  // verify
                false, // release
                None,  // target
                None,  // jobs
                false, // keep_temps
                false, // all_features
                false, // no_default_features
                None,  // features
                false, // timings
                // Advanced linking options
                None,  // lto
                false, // static_link
                false, // strip
                false, // strip_debug
                false, // emit_asm
                false, // emit_llvm
                false, // emit_bc
                false, // emit_types
                false, // emit_vbc
                // Lint options (use defaults for watch command)
                false,     // deny_warnings
                false,     // strict_intrinsics
                Vec::new(), // deny_lint
                Vec::new(), // warn_lint
                Vec::new(), // allow_lint
                Vec::new(), // forbid_lint
            ),
            "test" => crate::commands::test::execute(
                None,  // filter
                false, // release
                false, // nocapture
                None,  // test_threads
                false, // coverage
                None,  // verify
            ),
            "run" => crate::commands::run::execute(
                None,                   // tier
                None,                   // profile
                false,                  // release
                None,                   // example
                None,                   // bin
                verum_common::List::new(), // args
            ),
            _ => {
                ui::warn(&format!("Unknown command: {}", command));
                continue;
            }
        };

        match result {
            Ok(_) => ui::success("Done"),
            Err(e) => ui::error(&format!("{}", e)),
        }

        println!();
        ui::info("Waiting for changes...");
    }

    Ok(())
}
