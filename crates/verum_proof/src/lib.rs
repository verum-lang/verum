//! Native proof terms are checked independently of source elaboration and SMT traces.
//! A successful replay establishes exactly the supplied kernel judgment.
#![forbid(unsafe_code)]
pub mod admission;
#[cfg(feature = "process")]
pub mod process;
pub mod proof_checker;
pub mod proof_checker_nbe;
