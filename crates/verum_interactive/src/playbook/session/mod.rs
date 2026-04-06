//! Playbook session state and cell management
//!
//! This module provides the core data structures for managing a playbook:
//! - `Cell`: Individual code or markdown cells
//! - `CellOutput`: Rich output types including values, tensors, errors
//! - `SessionState`: Complete session state with VBC execution

mod cell;
mod state;

pub use cell::{Cell, CellId, CellOutput, CellKind, TensorStats};
pub use state::SessionState;
