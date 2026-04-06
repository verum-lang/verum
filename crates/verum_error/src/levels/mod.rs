//! 5-Level Error Defense Architecture
//!
//! Implements Verum's defense-in-depth error architecture with five complementary
//! layers, each providing progressively stronger safety guarantees:
//!
//! - **Level 0: Type Prevention** - Compile-time safety via refinement types
//!   (e.g., `Int{> 0}`), affine/move semantics, and context tracking. Errors at
//!   this level are prevented from being written in the first place.
//! - **Level 1: Static Verification** - Proof-based safety via SMT integration
//!   (`@verify` annotations). The solver proves properties impossible to violate.
//! - **Level 2: Explicit Handling** - Runtime recovery via `Result<T, E>`, the `?`
//!   operator, error context chains, and combinators. Zero-cost on success path.
//! - **Level 3: Fault Tolerance** - Resilience patterns: supervision trees (with
//!   OneForOne/OneForAll/RestForOne restart strategies), circuit breakers (3-state:
//!   Closed/Open/HalfOpen), retry with backoff, and health monitoring.
//! - **Level 4: Security Containment** - Isolation boundaries via capability-based
//!   security, sandboxing, and fine-grained permission control.

pub mod level0;
pub mod level1;
pub mod level2;
pub mod level3;
pub mod level4;
