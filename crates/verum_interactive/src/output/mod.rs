//! Output rendering module for the Verum Playground.
//!
//! This module provides rich output formatting for various data types,
//! including tensors, structured data, collections, and more.

mod renderer;
mod tensor;
mod structured;

pub use renderer::{OutputRenderer, OutputFormat, RenderedOutput};
pub use tensor::{TensorStats, TensorPreview, render_tensor};
pub use structured::{render_struct, render_variant, render_collection};
