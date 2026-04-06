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
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
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
        1 => "Tier 1: AOT Native (production, 85-95% native speed)",
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
            ui::info("Creating new playbook");
            PlaybookApp::new()
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
    let app =
        PlaybookApp::from_file(input_path).map_err(|e| CliError::custom(e.to_string()))?;

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

