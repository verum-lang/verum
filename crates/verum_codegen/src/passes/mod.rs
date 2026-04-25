//! AOT-time analysis & transformation passes operating on `VbcModule`
//! BEFORE the LLVM / MLIR lowering. Each pass is a pure function from
//! `&VbcModule` (or a function within it) to a metadata-bearing
//! analysis result that the lowering can consume.
//!
//! ## Pass catalogue
//!
//! | Pass               | Input          | Output                  | Used by      |
//! |--------------------|----------------|-------------------------|--------------|
//! | `tensor_fusion`    | VbcFunction    | Vec<TensorChain>        | LLVM + MLIR  |
//!
//! Future passes (sub-tasks of #91 / #94):
//!   * effect_driven_dce — remove Pure calls whose return is unused
//!   * tensor_shape_propagation — feed shape refinements into the
//!     subsequent fusion / kernel-emission decisions
//!   * checkpoint_layout_diff — compute univalence-Transport schemas
//!     from `@version(N)` attributes (#93)
//!
//! All passes here run BEFORE register-allocation / SSA-construction
//! at the LLVM level so they observe the canonical VBC instruction
//! stream — easier to reason about than mid-LLVM-IR.

pub mod tensor_fusion;
