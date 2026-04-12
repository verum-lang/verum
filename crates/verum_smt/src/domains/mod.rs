//! Phase D.3: SMT Domain Extensions
//!
//! Domain-specific SMT encodings for stdlib verification goals that
//! don't fit neatly into Z3's built-in theories.
//!
//! ## Modules
//!
//! * [`sheaf`] — encoding of descent conditions for ∞-sheaves as
//!   SMT formulas. Used by `core/math/infinity_topos.vr` descent
//!   verification.
//! * [`epistemic`] — encoding of epistemic-state propagation
//!   (density matrices, projective measurement) as constraint
//!   satisfaction. Used by `core/math/linalg.vr` quantum extensions.

pub mod epistemic;
pub mod sheaf;
