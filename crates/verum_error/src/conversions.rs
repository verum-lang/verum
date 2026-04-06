//! Conversion implementations from module-specific error types to unified VerumError
//!
//! This module provides documentation and templates for implementing `From` trait
//! conversions from module-specific error types to the unified VerumError type.
//!
//! # Architecture
//!
//! To avoid circular dependencies, each module implements its own `From<ModuleError> for VerumError`
//! conversion in its own codebase. This module provides templates and guidelines for these
//! implementations.
//!
//! # Usage Pattern
//!
//! These conversions enable seamless error propagation via the `?` operator:
//!
//! ```rust,ignore
//! use verum_error::unified::{VerumError, Result};
//!
//! fn example() -> Result<()> {
//!     perform_cbgr_check()?;  // Automatically converts from verum_cbgr::Error
//!     verify_types()?;         // Automatically converts from TypeError
//!     Ok(())
//! }
//! ```
//!
//! # Implementation Locations
//!
//! - **verum_cbgr**: Implements `From<verum_cbgr::Error> for VerumError`
//! - **verum_types**: Implements `From<verum_types::TypeError> for VerumError`
//! - **verum_smt**: Implements `From<verum_smt::Error> for VerumError`

use crate::unified::VerumError;
#[allow(unused_imports)]
use verum_common::{List, Text};

// ============================================================================
// Documentation for implementing conversions
// ============================================================================

// Guidelines for implementing error conversions
//
// When implementing `From<SpecificError> for VerumError`, follow these patterns:
//
// 1. **Direct mapping**: When the unified error has a variant that directly corresponds
//    to the specific error, map to that variant:
//    ```rust,ignore
//    impl From<CbgrError> for VerumError {
//        fn from(err: CbgrError) -> Self {
//            match err {
//                CbgrError::UseAfterFree { expected, actual } => {
//                    VerumError::UseAfterFree { expected, actual }
//                }
//                // ...
//            }
//        }
//    }
//    ```
//
// 2. **Lossy conversion**: When the specific error has more detail than the unified
//    error, convert to the closest match:
//    ```rust,ignore
//    TypeError::BranchMismatch { then_ty, else_ty, .. } => {
//        VerumError::TypeMismatch {
//            expected: then_ty,
//            actual: else_ty,
//        }
//    }
//    ```
//
// 3. **Catch-all**: When no direct mapping exists, use `VerumError::Other`:
//    ```rust,ignore
//    _ => VerumError::Other {
//        message: err.to_string().into(),
//    }
//    ```
//
// 4. **Batch errors**: For collections of errors (like parse errors), use the
//    appropriate collection variant:
//    ```rust,ignore
//    impl From<List<ParseError>> for VerumError {
//        fn from(errors: List<ParseError>) -> Self {
//            let messages: List<Text> = errors.iter()
//                .map(|e| e.to_string().into())
//                .collect();
//            VerumError::ParseErrors(messages)
//        }
//    }
//    ```
