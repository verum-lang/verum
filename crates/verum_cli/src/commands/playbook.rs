//! Playbook command - Interactive notebook environment for Verum
//!
//! The Playbook provides a Jupyter-like TUI notebook experience optimized
//! for exploring Verum's capabilities:
//!
//! # Features
//!
//! - **Cell-based editing**: Code and Markdown cells with rich output
//! - **Cross-cell state**: Variables persist across cells with dependency tracking
//! - **Rich output**: Tensors, structured data, collections with smart formatting
//! - **Execution tiers**: Choose between interpreter (safe) and JIT/AOT (fast)
//! - **Vim keybindings**: Optional vim-like navigation
//! - **Discovery**: Explore core/ capabilities interactively
//! - **Tutorials**: Interactive language learning
//! - **File format**: `.vrbook` JSON format with export to `.vr`

use std::io;
use std::path::PathBuf;

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::prelude::*;

use crate::error::{CliError, Result};
use crate::ui;

use verum_interactive::PlaybookApp;

/// Options for the playbook command
pub struct PlaybookOptions<'a> {
    pub file: Option<&'a str>,
    pub tier: u8,
    pub vim_mode: bool,
    pub preload: Option<&'a str>,
    pub tutorial: bool,
    pub profile: bool,
    pub export: Option<&'a str>,
    pub no_color: bool,
}

/// Execute the playbook command with enhanced options
pub fn execute(options: PlaybookOptions) -> Result<()> {
    // Validate tier (0=interpreter, 1=aot; clamp higher values)
    let tier = options.tier.min(1);

    ui::step("Starting Verum Playbook");

    // Show tier info
    let tier_desc = match tier {
        0 => "Tier 0: Interpreter (full CBGR validation, ~15ns/check)",
        1 => "Tier 1: AOT Native (production, native-C parity bar)",
        _ => unreachable!(),
    };
    ui::info(tier_desc);

    // Create or load playbook app
    let mut app = match options.file {
        Some(path) => {
            let path = PathBuf::from(path);
            ui::info(&format!("Loading: {}", path.display()));
            PlaybookApp::from_file(path).map_err(|e| CliError::custom(e.to_string()))?
        }
        None => {
            // Empty launch opens the GALLERY — tours, recent books,
            // blank sheet — not a bare buffer (Playground Reborn §3).
            let mut app = PlaybookApp::new();
            app.open_gallery();
            app
        }
    };

    // Configure options
    if options.vim_mode {
        app.set_vim_mode(true);
        ui::info("Vim keybindings enabled");
    }

    if options.profile {
        app.set_profiling(true);
        ui::info("Performance profiling enabled");
    }

    // Preload file if specified
    if let Some(preload_path) = options.preload {
        ui::info(&format!("Preloading: {}", preload_path));
        app.preload_file(preload_path)
            .map_err(|e| CliError::custom(format!("Failed to preload: {}", e)))?;
    }

    if options.tutorial {
        ui::info("Starting interactive tutorial...");
        app.start_tutorial();
    }

    // Run the TUI
    let export_path = options.export.map(PathBuf::from);
    run_tui(app, export_path, options.no_color).map_err(|e| CliError::custom(e.to_string()))?;

    ui::success("Playbook closed");
    Ok(())
}

/// Run the TUI event loop
fn run_tui(mut app: PlaybookApp, export_path: Option<PathBuf>, _no_color: bool) -> io::Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Run the event loop
    let res = run_event_loop(&mut terminal, &mut app);

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    // Export if requested
    if let Some(export_path) = export_path {
        if let Err(e) = app.export_to_script(&export_path) {
            eprintln!("Warning: Failed to export: {}", e);
        } else {
            println!("Exported to: {}", export_path.display());
        }
    }

    res
}

/// Main event loop
fn run_event_loop<B: Backend>(terminal: &mut Terminal<B>, app: &mut PlaybookApp) -> io::Result<()>
where
    B::Error: Into<io::Error>,
{
    loop {
        terminal.draw(|f| app.render(f)).map_err(Into::into)?;

        // Poll background execution results (non-blocking)
        app.poll_execution();

        // Non-blocking event check: faster refresh during execution for spinner
        let poll_timeout = if app.is_executing() {
            std::time::Duration::from_millis(50)
        } else {
            std::time::Duration::from_millis(200)
        };

        if event::poll(poll_timeout)? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    app.handle_key(key);
                }
                Event::Mouse(mouse) => {
                    app.handle_mouse(mouse);
                }
                _ => {}
            }
        }

        if app.should_quit {
            return Ok(());
        }
    }
}

/// Export playbook to Verum script
pub fn export_to_script(input: &str, output: Option<&str>, include_outputs: bool) -> Result<()> {
    ui::step(&format!("Exporting playbook to script: {}", input));

    let input_path = PathBuf::from(input);
    let output_path = output
        .map(PathBuf::from)
        .unwrap_or_else(|| input_path.with_extension("vr"));

    // Check input file exists
    if !input_path.exists() {
        return Err(CliError::FileNotFound(input.to_string()));
    }

    // Load playbook
    let app = PlaybookApp::from_file(input_path).map_err(|e| CliError::custom(e.to_string()))?;

    // Export
    if include_outputs {
        app.export_to_script_with_outputs(&output_path)
    } else {
        app.export_to_script(&output_path)
    }
    .map_err(|e| CliError::custom(format!("Failed to export: {}", e)))?;

    ui::success(&format!("Exported to: {}", output_path.display()));
    Ok(())
}

/// Import Verum script into playbook format
pub fn import_from_script(input: &str, output: Option<&str>) -> Result<()> {
    ui::step(&format!("Importing script to playbook: {}", input));

    let input_path = PathBuf::from(input);
    let output_path = output
        .map(PathBuf::from)
        .unwrap_or_else(|| input_path.with_extension("vrbook"));

    // Check input file exists
    if !input_path.exists() {
        return Err(CliError::FileNotFound(input.to_string()));
    }

    // Read source
    let source = std::fs::read_to_string(&input_path)
        .map_err(|e| CliError::custom(format!("Failed to read: {}", e)))?;

    // Create playbook from source
    let app = PlaybookApp::from_source(&source);

    // Save
    app.save_to(&output_path)
        .map_err(|e| CliError::custom(format!("Failed to save: {}", e)))?;

    ui::success(&format!("Created: {}", output_path.display()));
    Ok(())
}

/// Headless replay of a .vrbook (T0858 slice 4): the verdict logic
/// lives in `persistence::replay_book` (one carrier); this command
/// renders it and picks the exit code (2: chain out of step, 3:
/// divergence or failed run). With `freeze_to`, additionally writes
/// the frozen snapshot report — what ACTUALLY happened on this run,
/// chain addresses included; the live book stays the truth.
pub fn replay(file: &str, freeze_to: Option<&str>) -> Result<()> {
    use verum_interactive::playbook::persistence::{
        ReplayVerdict, chain_of_cells, load_playbook_file, replay_book,
    };

    let path = PathBuf::from(file);
    let book = load_playbook_file(&path)
        .map_err(|e| CliError::custom(format!("reading {}: {e}", path.display())))?;

    let (verdict, session) = replay_book(&book);
    match verdict {
        ReplayVerdict::ChainOutOfStep {
            cell,
            recorded,
            recomputed,
        } => {
            ui::error(&format!(
                "chain out of step at cell {} — the book's record was                  edited out of step with its sources (recorded {},                  recomputed {})",
                cell + 1,
                &recorded[..12.min(recorded.len())],
                &recomputed[..12.min(recomputed.len())],
            ));
            std::process::exit(2);
        }
        ReplayVerdict::ExecutionFailed { error } => {
            ui::error(&format!("replay execution failed: {error}"));
            std::process::exit(3);
        }
        ReplayVerdict::Divergent {
            cell,
            address,
            recorded,
            replayed,
        } => {
            ui::error(&format!(
                "DIVERGENT at cell {} (address {})",
                cell + 1,
                &address[..12.min(address.len())],
            ));
            println!("  recorded: {recorded}");
            println!("  replayed: {replayed}");
            std::process::exit(3);
        }
        ReplayVerdict::Identical {
            cells,
            compared,
            unrecorded,
            head,
        } => {
            ui::success(&format!(
                "replay identical: {} cells ({} compared, {} unrecorded), chain head {}",
                cells,
                compared,
                unrecorded,
                &head[..12.min(head.len())],
            ));
        }
    }

    if let Some(report_path) = freeze_to {
        let session = session.expect("identical verdict carries the session");
        let chain = chain_of_cells(&book.cells);
        let report = render_frozen_report(&book, &session, &chain);
        std::fs::write(report_path, report)
            .map_err(|e| CliError::custom(format!("writing {report_path}: {e}")))?;
        ui::success(&format!("frozen report written: {report_path}"));
    }
    Ok(())
}

/// The frozen report: sources, chain addresses, and THIS run's
/// outputs. Markdown so it reads anywhere.
fn render_frozen_report(
    book: &verum_interactive::playbook::persistence::PlaybookFile,
    session: &verum_interactive::SessionState,
    chain: &[String],
) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let title = book
        .metadata
        .title
        .clone()
        .unwrap_or_else(|| "untitled".to_string());
    let _ = writeln!(out, "# {title} — frozen replay report
");
    let _ = writeln!(
        out,
        "- verum {} · chain head `{}`
",
        env!("CARGO_PKG_VERSION"),
        chain.last().map(String::as_str).unwrap_or("<empty>"),
    );
    for (k, cell) in session.cells.iter().enumerate() {
        if cell.is_code() {
            let _ = writeln!(out, "## cell {} · `{}`
", k + 1, &chain[k][..12]);
            let _ = writeln!(out, "```verum
{}
```
", cell.source.as_str().trim_end());
            if let Some(output) = &cell.output {
                let _ = writeln!(out, "```
{}
```
", frozen_output_text(output));
            }
        } else {
            let _ = writeln!(out, "{}
", cell.source.as_str().trim_end());
        }
    }
    out
}

fn frozen_output_text(output: &verum_interactive::CellOutput) -> String {
    use verum_interactive::CellOutput;
    match output {
        CellOutput::Value { repr, type_info, .. } => format!("{repr} : {type_info}"),
        CellOutput::Stream { stdout, stderr } => {
            let mut t = stdout.as_str().to_string();
            if !stderr.is_empty() {
                t.push_str("\n[stderr] ");
                t.push_str(stderr.as_str());
            }
            t
        }
        CellOutput::Error { message, .. } => format!("error: {message}"),
        CellOutput::Multi { outputs } => outputs
            .iter()
            .filter(|o| !matches!(o, CellOutput::Timing { .. }))
            .map(frozen_output_text)
            .collect::<Vec<_>>()
            .join("\n"),
        CellOutput::Empty => String::new(),
        other => serde_json::to_string(other).unwrap_or_else(|_| "<output>".to_string()),
    }
}
