//! Verum Runtime Stubs — ALL C CODE ELIMINATED
//!
//! The entire C runtime has been migrated to LLVM IR:
//!   - platform_ir.rs: 7,985 LOC (platform, TLS, threading, networking, etc.)
//!   - tensor_ir.rs: 2,425 LOC (tensor, GPU, ML, autodiff, regex)
//!   - metal_ir.rs: 1,757 LOC (Apple Metal GPU via objc_msgSend)
//!
//! This module provides an empty C stub for the build system.
//! The C compiler still needs a source file to produce an object file,
//! even though all functions come from the LLVM module.

/// Empty C stub — all functions in LLVM IR.
/// The build system compiles this to an empty .o file.
pub const RUNTIME_C: &str = "#include <stdint.h>\n// All runtime functions in LLVM IR\n";
