//! VBC-level CBGR escape analysis for reference tier promotion.
//!
//! This module implements a lightweight escape analysis pass that operates on
//! decoded VBC instructions (between VBC codegen and LLVM lowering). It determines
//! which `&T` references can be promoted from Tier 0 (full runtime CBGR checks,
//! ~15ns overhead) to Tier 1 (`&checked T`, compiler-proven safe, zero overhead).
//!
//! # Architecture
//!
//! ```text
//! VBC Codegen -> VbcEscapeAnalyzer -> Tier Decisions -> LLVM Lowering
//!                      |
//!                      v
//!              Map<(FunctionId, usize), CbgrTier>
//! ```
//!
//! # Promotion Rules (MVP)
//!
//! A `Ref`/`RefMut` instruction at offset `i` in function `f` is promoted to
//! Tier 1 when ALL of the following hold:
//!
//! 1. **Local source**: The source register was defined by a local instruction
//!    (LoadK, LoadI, LoadTrue/False, New with stack-local semantics, arithmetic,
//!    or Mov from another local). References to function parameters stay Tier 0.
//!
//! 2. **Non-escaping**: The destination register of the Ref/RefMut is never:
//!    - Passed as an argument to `Call`, `CallM`, `CallClosure`, `TailCall`
//!    - Stored into a heap object (`SetF`, `SetE` where the object is heap-allocated)
//!    - Captured by a closure (`NewClosure`)
//!    - Sent to another thread (`Spawn`, `NurserySpawn`, channel send)
//!    - Returned from the function (`Ret`)
//!
//! 3. **Bounded lifetime**: The reference does not outlive the function scope
//!    (implied by non-escaping for this MVP).
//!
//! Everything else defaults to Tier 0.
//!
//! # Example
//!
//! ```rust,ignore
//! use verum_vbc::cbgr_analysis::VbcEscapeAnalyzer;
//! use verum_vbc::module::VbcFunction;
//!
//! let functions: Vec<VbcFunction> = /* decoded functions */;
//! let analyzer = VbcEscapeAnalyzer::new();
//! let result = analyzer.analyze(&functions);
//!
//! for ((func_id, offset), tier) in &result.decisions {
//!     // Use tier decision during LLVM lowering
//! }
//! ```

use std::collections::{HashMap, HashSet};

use crate::instruction::Instruction;
use crate::module::{FunctionId, VbcFunction};
use crate::types::CbgrTier;

// ============================================================================
// Analysis Result
// ============================================================================

/// Result of VBC-level escape analysis.
///
/// Maps `(FunctionId, instruction_offset)` to the decided CBGR tier for each
/// `Ref` or `RefMut` instruction in the module.
#[derive(Debug, Clone)]
pub struct EscapeAnalysisResult {
    /// Tier decisions keyed by (function ID, instruction offset).
    pub decisions: HashMap<(FunctionId, usize), CbgrTier>,
    /// Statistics from the analysis pass.
    pub stats: EscapeAnalysisStats,
}

/// Statistics collected during escape analysis.
#[derive(Debug, Clone, Default)]
pub struct EscapeAnalysisStats {
    /// Total Ref/RefMut instructions analyzed.
    pub total_refs: usize,
    /// References promoted to Tier 1 (compiler-proven safe).
    pub promoted_to_tier1: usize,
    /// References kept at Tier 0 (need runtime checks).
    pub kept_at_tier0: usize,
    /// Functions analyzed.
    pub functions_analyzed: usize,
}

impl EscapeAnalysisStats {
    /// Promotion rate as a percentage.
    pub fn promotion_rate(&self) -> f64 {
        if self.total_refs == 0 {
            0.0
        } else {
            self.promoted_to_tier1 as f64 / self.total_refs as f64 * 100.0
        }
    }
}

// ============================================================================
// Analyzer
// ============================================================================

/// VBC-level escape analyzer for CBGR tier promotion.
///
/// Performs a conservative analysis over decoded VBC instructions to determine
/// which references can skip runtime generation checks.
pub struct VbcEscapeAnalyzer {
    _private: (),
}

impl VbcEscapeAnalyzer {
    /// Create a new escape analyzer.
    pub fn new() -> Self {
        Self { _private: () }
    }

    /// Analyze a set of decoded VBC functions and produce tier decisions.
    ///
    /// For each `Ref` or `RefMut` instruction, determines whether it can be
    /// promoted to Tier 1 (zero-cost) or must remain Tier 0 (runtime-checked).
    pub fn analyze(&self, functions: &[VbcFunction]) -> EscapeAnalysisResult {
        let mut decisions = HashMap::new();
        let mut stats = EscapeAnalysisStats::default();

        for func in functions {
            let func_id = func.descriptor.id;
            stats.functions_analyzed += 1;

            let func_decisions = self.analyze_function(func);

            for (offset, tier) in func_decisions {
                stats.total_refs += 1;
                match tier {
                    CbgrTier::Tier1 => stats.promoted_to_tier1 += 1,
                    CbgrTier::Tier0 => stats.kept_at_tier0 += 1,
                    CbgrTier::Tier2 => {} // not produced by this pass
                }
                decisions.insert((func_id, offset), tier);
            }
        }

        EscapeAnalysisResult { decisions, stats }
    }

    /// Analyze a single function. Returns (instruction_offset, tier) pairs
    /// for every Ref/RefMut instruction found.
    fn analyze_function(&self, func: &VbcFunction) -> Vec<(usize, CbgrTier)> {
        let instrs = &func.instructions;

        // Phase 1: Identify parameter registers.
        // Registers r0..r(n-1) are function parameters (per VBC calling convention).
        let param_count = func.descriptor.params.len() as u16;
        let param_regs: HashSet<u16> = (0..param_count).collect();

        // Phase 2: Build register provenance map.
        // For each register, track whether it originates from a local definition
        // or from a parameter / heap allocation.
        let provenance = self.build_provenance(instrs, &param_regs);

        // Phase 3: Build ref-register usage map.
        // For each Ref/RefMut dst register, check if it escapes.
        let escaping = self.find_escaping_registers(instrs);

        // Phase 4: Decide tiers.
        let mut results = Vec::new();
        for (offset, instr) in instrs.iter().enumerate() {
            match instr {
                Instruction::Ref { dst, src } => {
                    let tier = self.decide_tier(dst.0, src.0, &provenance, &escaping);
                    results.push((offset, tier));
                }
                Instruction::RefMut { dst, src } => {
                    let tier = self.decide_tier(dst.0, src.0, &provenance, &escaping);
                    results.push((offset, tier));
                }
                _ => {}
            }
        }

        results
    }

    /// Decide the CBGR tier for a single Ref/RefMut instruction.
    ///
    /// Returns Tier1 if both:
    /// - The source register has local provenance (not a param, not heap-derived)
    /// - The destination register (the reference) does not escape
    fn decide_tier(
        &self,
        dst_reg: u16,
        src_reg: u16,
        provenance: &HashMap<u16, RegisterProvenance>,
        escaping: &HashSet<u16>,
    ) -> CbgrTier {
        let source_is_local = provenance
            .get(&src_reg)
            .map_or(false, |p| *p == RegisterProvenance::Local);

        let ref_escapes = escaping.contains(&dst_reg);

        if source_is_local && !ref_escapes {
            CbgrTier::Tier1
        } else {
            CbgrTier::Tier0
        }
    }

    /// Build a provenance map for all registers in the function.
    ///
    /// Each register is classified as:
    /// - `Local`: defined by a local computation (let binding, literal, arithmetic)
    /// - `Param`: comes from a function parameter
    /// - `Heap`: comes from a heap allocation or heap dereference
    /// - `Unknown`: cannot be determined (conservative: treated as non-local)
    fn build_provenance(
        &self,
        instrs: &[Instruction],
        param_regs: &HashSet<u16>,
    ) -> HashMap<u16, RegisterProvenance> {
        let mut provenance: HashMap<u16, RegisterProvenance> = HashMap::new();

        // Mark parameter registers.
        for &reg in param_regs {
            provenance.insert(reg, RegisterProvenance::Param);
        }

        // Single forward pass over instructions.
        for instr in instrs {
            match instr {
                // Local value producers: these create stack-local values.
                Instruction::LoadK { dst, .. }
                | Instruction::LoadI { dst, .. }
                | Instruction::LoadF { dst, .. }
                | Instruction::LoadTrue { dst }
                | Instruction::LoadFalse { dst }
                | Instruction::LoadUnit { dst }
                | Instruction::LoadSmallI { dst, .. }
                | Instruction::LoadNil { dst } => {
                    provenance.insert(dst.0, RegisterProvenance::Local);
                }

                // Arithmetic / comparison / conversion results are local.
                Instruction::BinaryI { dst, .. }
                | Instruction::BinaryF { dst, .. }
                | Instruction::BinaryG { dst, .. }
                | Instruction::UnaryI { dst, .. }
                | Instruction::UnaryF { dst, .. }
                | Instruction::CmpI { dst, .. }
                | Instruction::CmpF { dst, .. }
                | Instruction::Not { dst, .. }
                | Instruction::Bitwise { dst, .. }
                | Instruction::CvtIF { dst, .. }
                | Instruction::CvtFI { dst, .. }
                | Instruction::CvtIC { dst, .. }
                | Instruction::CvtCI { dst, .. }
                | Instruction::CvtBI { dst, .. }
                | Instruction::Len { dst, .. }
                | Instruction::Concat { dst, .. } => {
                    provenance.insert(dst.0, RegisterProvenance::Local);
                }

                // Mov propagates provenance from source.
                Instruction::Mov { dst, src } => {
                    let src_prov = provenance
                        .get(&src.0)
                        .copied()
                        .unwrap_or(RegisterProvenance::Unknown);
                    provenance.insert(dst.0, src_prov);
                }

                // Heap allocations: New / NewG produce heap pointers.
                Instruction::New { dst, .. } | Instruction::NewG { dst, .. } => {
                    provenance.insert(dst.0, RegisterProvenance::Heap);
                }

                // Dereferencing a heap reference yields a heap-derived value.
                Instruction::Deref { dst, .. } => {
                    provenance.insert(dst.0, RegisterProvenance::Heap);
                }

                // Field access from an object: heap-derived.
                Instruction::GetF { dst, .. } | Instruction::GetE { dst, .. } => {
                    provenance.insert(dst.0, RegisterProvenance::Heap);
                }

                // Call results: unknown provenance (could be anything).
                Instruction::Call { dst, .. }
                | Instruction::CallM { dst, .. }
                | Instruction::CallG { dst, .. } => {
                    provenance.insert(dst.0, RegisterProvenance::Unknown);
                }

                // Ref/RefMut: the destination is a reference (not a value).
                // Mark as Unknown so references-to-references stay conservative.
                Instruction::Ref { dst, .. } | Instruction::RefMut { dst, .. } => {
                    provenance.insert(dst.0, RegisterProvenance::Unknown);
                }

                // Closure creation: captures may close over anything.
                Instruction::NewClosure { dst, .. } => {
                    provenance.insert(dst.0, RegisterProvenance::Heap);
                }

                _ => {
                    // For any other instruction with a destination register that
                    // we don't explicitly handle, leave provenance as-is.
                    // This is conservative: unknown registers won't be promoted.
                }
            }
        }

        provenance
    }

    /// Find all registers that "escape" the current function scope.
    ///
    /// A register escapes if it is:
    /// - Passed as a call argument (Call, CallM, CallClosure, TailCall, CallG)
    /// - Returned from the function (Ret)
    /// - Stored into a heap object (SetF, SetE)
    /// - Captured by a closure (NewClosure)
    /// - Sent to another thread (Spawn, NurserySpawn)
    fn find_escaping_registers(&self, instrs: &[Instruction]) -> HashSet<u16> {
        let mut escaping = HashSet::new();

        for instr in instrs {
            match instr {
                // Return escapes the function.
                Instruction::Ret { value } => {
                    escaping.insert(value.0);
                }

                // Call arguments escape to the callee.
                Instruction::Call { args, .. }
                | Instruction::TailCall { args, .. }
                | Instruction::CallG { args, .. }
                | Instruction::Spawn { args, .. } => {
                    for i in 0..args.count as u16 {
                        escaping.insert(args.start.0 + i);
                    }
                }

                // Method call: receiver + arguments escape.
                Instruction::CallM {
                    receiver, args, ..
                } => {
                    escaping.insert(receiver.0);
                    for i in 0..args.count as u16 {
                        escaping.insert(args.start.0 + i);
                    }
                }

                // Closure call: closure + arguments escape.
                Instruction::CallClosure {
                    closure, args, ..
                } => {
                    escaping.insert(closure.0);
                    for i in 0..args.count as u16 {
                        escaping.insert(args.start.0 + i);
                    }
                }

                // Closure captures escape (they're stored in the closure object).
                Instruction::NewClosure { captures, .. } => {
                    for cap in captures {
                        escaping.insert(cap.0);
                    }
                }

                // Storing into an object field: the value escapes to the heap.
                Instruction::SetF { value, .. } | Instruction::SetE { value, .. } => {
                    escaping.insert(value.0);
                }

                // Nursery spawn: the task closure escapes.
                Instruction::NurserySpawn { task, .. } => {
                    escaping.insert(task.0);
                }

                _ => {}
            }
        }

        escaping
    }
}

impl Default for VbcEscapeAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Register Provenance
// ============================================================================

/// Classification of where a register's value originates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RegisterProvenance {
    /// Defined by a local computation (literal, arithmetic, let binding).
    /// Safe to take a Tier 1 reference to.
    Local,
    /// Comes from a function parameter.
    /// Not safe for Tier 1: caller controls lifetime.
    Param,
    /// Comes from a heap allocation or heap dereference.
    /// Not safe for Tier 1: heap objects may be freed independently.
    Heap,
    /// Cannot determine provenance.
    /// Conservative: treated as non-promotable.
    Unknown,
}

// ============================================================================
// Convenience: lookup a tier decision
// ============================================================================

impl EscapeAnalysisResult {
    /// Look up the decided tier for a specific instruction.
    ///
    /// Returns `None` if the instruction at the given offset is not a Ref/RefMut,
    /// or was not analyzed.
    pub fn get_tier(&self, func_id: FunctionId, offset: usize) -> Option<CbgrTier> {
        self.decisions.get(&(func_id, offset)).copied()
    }

    /// Look up the tier, defaulting to Tier 0 if not found.
    ///
    /// This is the recommended API for LLVM lowering: if no analysis result
    /// exists for an instruction, fall back to the safe default (full checks).
    pub fn tier_or_default(&self, func_id: FunctionId, offset: usize) -> CbgrTier {
        self.decisions
            .get(&(func_id, offset))
            .copied()
            .unwrap_or(CbgrTier::Tier0)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instruction::{Reg, RegRange};
    use crate::module::{FunctionDescriptor, FunctionId, ParamDescriptor, VbcFunction};
    use crate::types::{StringId, TypeId, TypeRef};

    /// Helper to create a minimal FunctionDescriptor for testing.
    fn make_descriptor(id: u32, param_count: usize) -> FunctionDescriptor {
        let mut desc = FunctionDescriptor {
            id: FunctionId(id),
            register_count: 16,
            ..Default::default()
        };
        for _ in 0..param_count {
            desc.params.push(ParamDescriptor {
                name: StringId::EMPTY,
                type_ref: TypeRef::Concrete(TypeId::INT),
                is_mut: false,
                default: None,
            });
        }
        desc
    }

    #[test]
    fn test_local_ref_promoted_to_tier1() {
        // let x = 42;
        // let r = &x;    // Should be Tier 1: x is local, r doesn't escape.
        let instrs = vec![
            Instruction::LoadI {
                dst: Reg(0),
                value: 42,
            },
            Instruction::Ref {
                dst: Reg(1),
                src: Reg(0),
            },
            Instruction::RetV,
        ];
        let func = VbcFunction::new(make_descriptor(0, 0), instrs);

        let analyzer = VbcEscapeAnalyzer::new();
        let result = analyzer.analyze(&[func]);

        assert_eq!(
            result.get_tier(FunctionId(0), 1),
            Some(CbgrTier::Tier1),
            "Local non-escaping ref should be promoted to Tier 1"
        );
        assert_eq!(result.stats.promoted_to_tier1, 1);
        assert_eq!(result.stats.kept_at_tier0, 0);
    }

    #[test]
    fn test_param_ref_stays_tier0() {
        // fn foo(x: Int) {
        //     let r = &x;  // Should stay Tier 0: x is a parameter.
        // }
        let instrs = vec![
            // r0 is the parameter
            Instruction::Ref {
                dst: Reg(1),
                src: Reg(0),
            },
            Instruction::RetV,
        ];
        let func = VbcFunction::new(make_descriptor(0, 1), instrs);

        let analyzer = VbcEscapeAnalyzer::new();
        let result = analyzer.analyze(&[func]);

        assert_eq!(
            result.get_tier(FunctionId(0), 0),
            Some(CbgrTier::Tier0),
            "Ref to parameter should stay Tier 0"
        );
        assert_eq!(result.stats.kept_at_tier0, 1);
    }

    #[test]
    fn test_returned_ref_stays_tier0() {
        // let x = 42;
        // let r = &x;
        // return r;  // Escapes via return.
        let instrs = vec![
            Instruction::LoadI {
                dst: Reg(0),
                value: 42,
            },
            Instruction::Ref {
                dst: Reg(1),
                src: Reg(0),
            },
            Instruction::Ret { value: Reg(1) },
        ];
        let func = VbcFunction::new(make_descriptor(0, 0), instrs);

        let analyzer = VbcEscapeAnalyzer::new();
        let result = analyzer.analyze(&[func]);

        assert_eq!(
            result.get_tier(FunctionId(0), 1),
            Some(CbgrTier::Tier0),
            "Returned ref should stay Tier 0"
        );
    }

    #[test]
    fn test_ref_passed_to_call_stays_tier0() {
        // let x = 42;
        // let r = &x;
        // foo(r);  // Escapes via call argument.
        let instrs = vec![
            Instruction::LoadI {
                dst: Reg(0),
                value: 42,
            },
            Instruction::Ref {
                dst: Reg(1),
                src: Reg(0),
            },
            Instruction::Call {
                dst: Reg(2),
                func_id: 1,
                args: RegRange {
                    start: Reg(1),
                    count: 1,
                },
            },
            Instruction::RetV,
        ];
        let func = VbcFunction::new(make_descriptor(0, 0), instrs);

        let analyzer = VbcEscapeAnalyzer::new();
        let result = analyzer.analyze(&[func]);

        assert_eq!(
            result.get_tier(FunctionId(0), 1),
            Some(CbgrTier::Tier0),
            "Ref passed to call should stay Tier 0"
        );
    }

    #[test]
    fn test_heap_deref_ref_stays_tier0() {
        // let h = Heap(42);    // New -> heap
        // let v = *h;          // Deref -> heap-derived
        // let r = &v;          // Should stay Tier 0: v is heap-derived
        let instrs = vec![
            Instruction::New {
                dst: Reg(0),
                type_id: 0,
                field_count: 1,
            },
            Instruction::Deref {
                dst: Reg(1),
                ref_reg: Reg(0),
            },
            Instruction::Ref {
                dst: Reg(2),
                src: Reg(1),
            },
            Instruction::RetV,
        ];
        let func = VbcFunction::new(make_descriptor(0, 0), instrs);

        let analyzer = VbcEscapeAnalyzer::new();
        let result = analyzer.analyze(&[func]);

        assert_eq!(
            result.get_tier(FunctionId(0), 2),
            Some(CbgrTier::Tier0),
            "Ref to heap-derived value should stay Tier 0"
        );
    }

    #[test]
    fn test_closure_capture_escapes() {
        // let x = 42;
        // let r = &x;
        // let c = || { r };  // r captured -> escapes
        let instrs = vec![
            Instruction::LoadI {
                dst: Reg(0),
                value: 42,
            },
            Instruction::Ref {
                dst: Reg(1),
                src: Reg(0),
            },
            Instruction::NewClosure {
                dst: Reg(2),
                func_id: 1,
                captures: vec![Reg(1)],
            },
            Instruction::RetV,
        ];
        let func = VbcFunction::new(make_descriptor(0, 0), instrs);

        let analyzer = VbcEscapeAnalyzer::new();
        let result = analyzer.analyze(&[func]);

        assert_eq!(
            result.get_tier(FunctionId(0), 1),
            Some(CbgrTier::Tier0),
            "Ref captured by closure should stay Tier 0"
        );
    }

    #[test]
    fn test_ref_stored_in_field_escapes() {
        // let x = 42;
        // let r = &x;
        // obj.field = r;  // Escapes via SetF.
        let instrs = vec![
            Instruction::LoadI {
                dst: Reg(0),
                value: 42,
            },
            Instruction::Ref {
                dst: Reg(1),
                src: Reg(0),
            },
            Instruction::SetF {
                obj: Reg(3),
                field_idx: 0,
                value: Reg(1),
            },
            Instruction::RetV,
        ];
        let func = VbcFunction::new(make_descriptor(0, 0), instrs);

        let analyzer = VbcEscapeAnalyzer::new();
        let result = analyzer.analyze(&[func]);

        assert_eq!(
            result.get_tier(FunctionId(0), 1),
            Some(CbgrTier::Tier0),
            "Ref stored in field should stay Tier 0"
        );
    }

    #[test]
    fn test_multiple_refs_mixed_tiers() {
        // let x = 42;
        // let y = 99;
        // let rx = &x;     // Local, non-escaping -> Tier 1
        // let ry = &y;     // Local, but escaping via return -> Tier 0
        // return ry;
        let instrs = vec![
            Instruction::LoadI {
                dst: Reg(0),
                value: 42,
            },
            Instruction::LoadI {
                dst: Reg(1),
                value: 99,
            },
            Instruction::Ref {
                dst: Reg(2),
                src: Reg(0),
            },
            Instruction::Ref {
                dst: Reg(3),
                src: Reg(1),
            },
            Instruction::Ret { value: Reg(3) },
        ];
        let func = VbcFunction::new(make_descriptor(0, 0), instrs);

        let analyzer = VbcEscapeAnalyzer::new();
        let result = analyzer.analyze(&[func]);

        assert_eq!(
            result.get_tier(FunctionId(0), 2),
            Some(CbgrTier::Tier1),
            "rx: local non-escaping -> Tier 1"
        );
        assert_eq!(
            result.get_tier(FunctionId(0), 3),
            Some(CbgrTier::Tier0),
            "ry: local but returned -> Tier 0"
        );
        assert_eq!(result.stats.promoted_to_tier1, 1);
        assert_eq!(result.stats.kept_at_tier0, 1);
    }

    #[test]
    fn test_mov_propagates_provenance() {
        // let x = 42;
        // let y = x;      // Mov: propagates Local provenance
        // let r = &y;     // Should be Tier 1
        let instrs = vec![
            Instruction::LoadI {
                dst: Reg(0),
                value: 42,
            },
            Instruction::Mov {
                dst: Reg(1),
                src: Reg(0),
            },
            Instruction::Ref {
                dst: Reg(2),
                src: Reg(1),
            },
            Instruction::RetV,
        ];
        let func = VbcFunction::new(make_descriptor(0, 0), instrs);

        let analyzer = VbcEscapeAnalyzer::new();
        let result = analyzer.analyze(&[func]);

        assert_eq!(
            result.get_tier(FunctionId(0), 2),
            Some(CbgrTier::Tier1),
            "Ref to mov'd local should be Tier 1"
        );
    }

    #[test]
    fn test_refmut_same_rules() {
        // let mut x = 42;
        // let r = &mut x;  // Local, non-escaping -> Tier 1
        let instrs = vec![
            Instruction::LoadI {
                dst: Reg(0),
                value: 42,
            },
            Instruction::RefMut {
                dst: Reg(1),
                src: Reg(0),
            },
            Instruction::RetV,
        ];
        let func = VbcFunction::new(make_descriptor(0, 0), instrs);

        let analyzer = VbcEscapeAnalyzer::new();
        let result = analyzer.analyze(&[func]);

        assert_eq!(
            result.get_tier(FunctionId(0), 1),
            Some(CbgrTier::Tier1),
            "RefMut to local non-escaping should be Tier 1"
        );
    }

    #[test]
    fn test_empty_function() {
        let instrs = vec![Instruction::RetV];
        let func = VbcFunction::new(make_descriptor(0, 0), instrs);

        let analyzer = VbcEscapeAnalyzer::new();
        let result = analyzer.analyze(&[func]);

        assert_eq!(result.stats.total_refs, 0);
        assert_eq!(result.stats.functions_analyzed, 1);
    }

    #[test]
    fn test_tier_or_default() {
        let result = EscapeAnalysisResult {
            decisions: HashMap::new(),
            stats: EscapeAnalysisStats::default(),
        };

        // Missing entries default to Tier 0.
        assert_eq!(result.tier_or_default(FunctionId(0), 5), CbgrTier::Tier0);
    }
}
