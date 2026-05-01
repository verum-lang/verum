//! Output rendering module for the Verum Playground.
//!

//! This module provides rich output formatting for various data types,
//! including tensors, structured data, collections, and more.

mod renderer;
mod structured;
mod tensor;

pub use renderer::{OutputFormat, OutputRenderer, RenderedOutput};
pub use structured::{render_collection, render_struct, render_variant};
pub use tensor::{TensorPreview, TensorStats, render_tensor};
