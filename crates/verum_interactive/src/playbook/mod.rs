//! Verum Playbook - Jupyter-like TUI notebook interface
//!
//! The Playbook provides an interactive notebook experience in the terminal,
//! similar to Jupyter notebooks but optimized for Verum development.
//!
//! # Features
//!
//! - **Cell-based editing**: Code and Markdown cells
//! - **Incremental execution**: Smart re-run of dependent cells
//! - **Type inference display**: Shows inferred types for bindings
//! - **LSP integration**: Syntax highlighting, completions, hover info via verum_lsp
//! - **Vim keybindings**: Optional vim-like navigation
//! - **File format**: `.vrbook` JSON format
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │ Verum Playbook: project.vrbook                     [Run All] [Save] │
//! ├─────────────────────────────────────────────┬───────────────────────┤
//! │ [1]: let data = load_csv("data.csv")        │ Variables             │
//! │ → data: DataFrame<{name: Text, age: Int}>   │ ├─ data: DataFrame    │
//! ├─────────────────────────────────────────────┤ ├─ result: List<Int>  │
//! │ [2]: let result = data                      │ └─ stats: Stats       │
//! │      |> filter(row => row.age > 18)         │                       │
//! │      |> map(row => row.name)                │ Outline               │
//! │ → ["Alice", "Bob", "Charlie"]               │ ├─ Cell 1: load data  │
//! ├─────────────────────────────────────────────┤ ├─ Cell 2: filter     │
//! │ [3]: # Analysis Results (Markdown)          │ └─ Cell 3: markdown   │
//! │ The data contains **42** valid entries.     │                       │
//! ├─────────────────────────────────────────────┼───────────────────────┤
//! │ [>] _                                       │ Type: (waiting)       │
//! └─────────────────────────────────────────────┴───────────────────────┘
//!   j/k: navigate  Enter: edit  Shift+Enter: run  Ctrl+s: save  ?: help
//! ```

pub mod app;
pub mod session;
pub mod ui;
pub mod keybindings;
pub mod persistence;

pub use app::PlaybookApp;
pub use session::{Cell, CellId, CellKind, CellOutput, TensorStats, SessionState};
