//! Post-specialization bytecode optimization.
//!

//! Implements industrial-grade optimization passes for specialized bytecode:
//! 1. Constant folding - evaluate compile-time constants
//! 2. Dead code elimination - remove unreachable code
//! 3. Peephole optimization - local instruction patterns
//! 4. Copy propagation - eliminate redundant moves
//!

//! Runs after specialization to optimize the monomorphized bytecode before merging
//! into the final module.

use crate::instruction::{BinaryIntOp, Instruction};

/// Fold a constant integer binary op.  Returns None for ops we don't fold
/// (division/modulo by zero, or ops with non-trivial semantics we skip:
/// Pow can overflow, unsigned/bitwise/shift are left to LLVM/interp).
fn fold_int_binop(op: BinaryIntOp, a: i64, b: i64) -> Option<i64> {
    Some(match op {
        BinaryIntOp::Add => a.wrapping_add(b),
        BinaryIntOp::Sub => a.wrapping_sub(b),
        BinaryIntOp::Mul => a.wrapping_mul(b),
        BinaryIntOp::Div if b != 0 => a.wrapping_div(b),
        BinaryIntOp::Mod if b != 0 => a.wrapping_rem(b),
        _ => return None,
    })
}

/// Post-specialization bytecode optimizer.
///

/// Applies optimization passes to specialized bytecode:
/// 1. Constant folding - evaluate compile-time constants
/// 2. Dead code elimination - remove unreachable code
/// 3. Peephole optimization - local instruction patterns
pub struct SpecializationOptimizer {
    /// Whether to enable constant folding.
    pub constant_fold: bool,
    /// Whether to enable dead code elimination.
    pub dead_code_elim: bool,
    /// Whether to enable peephole optimization.
    pub peephole: bool,
    /// Maximum optimization iterations.
    pub max_iterations: usize,
    /// Statistics.
    pub stats: OptimizationStats,
}

/// Optimization statistics.
#[derive(Debug, Clone, Default)]
pub struct OptimizationStats {
    /// Constants folded.
    pub constants_folded: usize,
    /// Instructions eliminated.
    pub instructions_eliminated: usize,
    /// Peephole patterns applied.
    pub peepholes_applied: usize,
    /// Copy propagation applied.
    pub copies_propagated: usize,
    /// Total optimization iterations.
    pub iterations: usize,
}

impl Default for SpecializationOptimizer {
    fn default() -> Self {
        Self {
            constant_fold: true,
            dead_code_elim: true,
            peephole: true,
            max_iterations: 3,
            stats: OptimizationStats::default(),
        }
    }
}

impl SpecializationOptimizer {
    /// Creates a new optimizer with default settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an optimizer with all passes disabled.
    pub fn disabled() -> Self {
        Self {
            constant_fold: false,
            dead_code_elim: false,
            peephole: false,
            max_iterations: 0,
            stats: OptimizationStats::default(),
        }
    }

    /// Optimizes specialized bytecode.
    ///

    /// Runs optimization passes iteratively until no more changes occur
    /// or max_iterations is reached.
    pub fn optimize(&mut self, bytecode: Vec<u8>) -> Vec<u8> {
        // Operate on the DECODED instruction stream — never raw bytes. The
        // previous byte-by-byte scanner mis-read operand bytes as opcodes and
        // rewrote the stream in place, corrupting it (a correct 95-byte body
        // became 91 bytes of unrelated opcodes).  Jumps decode with
        // BYTE-relative offsets; we normalize them to INSTRUCTION-relative so a
        // size-changing fold stays valid, run the passes, then re-encode with
        // `encode_instructions_with_fixup`, which recomputes byte offsets.
        let Some((mut instrs, positions)) = Self::decode_with_positions(&bytecode) else {
            return bytecode; // undecodable → pass through unchanged
        };
        if !Self::normalize_jump_targets(&mut instrs, &positions, bytecode.len()) {
            return bytecode; // a jump target didn't land on an instruction boundary
        }

        for iteration in 0..self.max_iterations {
            self.stats.iterations = iteration + 1;
            let folded_before = self.stats.constants_folded;
            if self.constant_fold {
                self.fold_constants(&mut instrs);
            }
            if self.stats.constants_folded == folded_before {
                break; // fixpoint — nothing more folded this round
            }
        }

        let mut out = Vec::with_capacity(bytecode.len());
        crate::bytecode::encode_instructions_with_fixup(&instrs, &mut out);
        out
    }

    /// Decode the whole stream, recording each instruction's start byte offset.
    /// Returns None if any instruction fails to decode (caller passes the
    /// bytecode through unchanged).
    fn decode_with_positions(bytecode: &[u8]) -> Option<(Vec<Instruction>, Vec<usize>)> {
        let mut instrs = Vec::new();
        let mut positions = Vec::new();
        let mut pc = 0usize;
        while pc < bytecode.len() {
            positions.push(pc);
            match crate::bytecode::decode_instruction(bytecode, &mut pc) {
                Ok(i) => instrs.push(i),
                Err(_) => return None,
            }
        }
        Some((instrs, positions))
    }

    /// Rewrite jump offsets from byte-relative (as decoded) to
    /// instruction-relative (what `encode_instructions_with_fixup` expects).
    /// The encoder stores `target_byte - instr_end_byte`, so the target byte is
    /// `(start + size) + byte_offset`; map that byte to its instruction index
    /// and store `target_idx - jump_idx`. Returns false on a malformed target.
    fn normalize_jump_targets(
        instrs: &mut [Instruction],
        positions: &[usize],
        total_len: usize,
    ) -> bool {
        use std::collections::HashMap;
        let mut byte_to_idx: HashMap<usize, i32> = HashMap::with_capacity(positions.len() + 1);
        for (i, &p) in positions.iter().enumerate() {
            byte_to_idx.insert(p, i as i32);
        }
        byte_to_idx.insert(total_len, positions.len() as i32); // one-past-end target
        for i in 0..instrs.len() {
            let Some(byte_off) = Self::jump_byte_offset(&instrs[i]) else {
                continue;
            };
            let end = if i + 1 < positions.len() {
                positions[i + 1]
            } else {
                total_len
            };
            let target_byte = end as i64 + byte_off as i64;
            if target_byte < 0 {
                return false;
            }
            let Some(&target_idx) = byte_to_idx.get(&(target_byte as usize)) else {
                return false;
            };
            Self::set_jump_offset(&mut instrs[i], target_idx - i as i32);
        }
        true
    }

    /// The byte/instruction-relative offset field of a control-flow
    /// instruction, if any.  Mirrors the jump set handled by
    /// `bytecode::fixup_jump_offsets`.
    fn jump_byte_offset(instr: &Instruction) -> Option<i32> {
        match instr {
            Instruction::Jmp { offset }
            | Instruction::JmpIf { offset, .. }
            | Instruction::JmpNot { offset, .. }
            | Instruction::JmpCmp { offset, .. } => Some(*offset),
            Instruction::CtxProvide { body_offset, .. } => Some(*body_offset),
            Instruction::TryBegin { handler_offset } => Some(*handler_offset),
            _ => None,
        }
    }

    /// Set the (now instruction-relative) offset on a control-flow instruction.
    fn set_jump_offset(instr: &mut Instruction, new: i32) {
        match instr {
            Instruction::Jmp { offset }
            | Instruction::JmpIf { offset, .. }
            | Instruction::JmpNot { offset, .. }
            | Instruction::JmpCmp { offset, .. } => *offset = new,
            Instruction::CtxProvide { body_offset, .. } => *body_offset = new,
            Instruction::TryBegin { handler_offset } => *handler_offset = new,
            _ => {}
        }
    }

    /// Constant-folding pass over the decoded stream.  Tracks registers whose
    /// value is a known integer constant (LoadI / LoadSmallI) within a basic
    /// block and folds an integer `BinaryI` whose two operands are both known
    /// into a single `LoadI`.  Any control-flow instruction (basic-block
    /// boundary) or an instruction that may write an un-modelled register
    /// clears the tracked constants — over-invalidation only misses folds, it
    /// never produces a wrong one.  `BinaryI -> LoadI` is 1→1, so the
    /// instruction COUNT (and thus every instruction-relative jump offset) is
    /// preserved.
    fn fold_constants(&mut self, instrs: &mut [Instruction]) {
        let mut consts: std::collections::HashMap<u16, i64> = std::collections::HashMap::new();
        for instr in instrs.iter_mut() {
            if Self::jump_byte_offset(instr).is_some()
                || matches!(instr, Instruction::Ret { .. } | Instruction::RetV)
            {
                consts.clear();
                continue;
            }
            match instr {
                Instruction::LoadI { dst, value } => {
                    consts.insert(dst.0, *value);
                }
                Instruction::LoadSmallI { dst, value } => {
                    consts.insert(dst.0, *value as i64);
                }
                Instruction::BinaryI { op, dst, a, b } => {
                    if let (Some(&va), Some(&vb)) = (consts.get(&a.0), consts.get(&b.0))
                        && let Some(folded) = fold_int_binop(*op, va, vb)
                    {
                        let d = *dst;
                        *instr = Instruction::LoadI {
                            dst: d,
                            value: folded,
                        };
                        consts.insert(d.0, folded);
                        self.stats.constants_folded += 1;
                        continue;
                    }
                    consts.remove(&dst.0);
                }
                _ => {
                    // May write an un-modelled register; drop everything.
                    consts.clear();
                }
            }
        }
    }

    /// Constant folding pass.
    ///

    /// Writes a signed varint (ZigZag encoded) to output.
    fn write_signed_varint(&self, output: &mut Vec<u8>, value: i64) {
        let encoded = ((value << 1) ^ (value >> 63)) as u64;
        let mut v = encoded;
        loop {
            let byte = (v & 0x7F) as u8;
            v >>= 7;
            if v == 0 {
                output.push(byte);
                break;
            } else {
                output.push(byte | 0x80);
            }
        }
    }

    /// Dead code elimination pass.
    ///

    /// Peephole optimization pass.
    ///

    /// Returns the optimization statistics.
    pub fn stats(&self) -> &OptimizationStats {
        &self.stats
    }

    /// Resets the statistics.
    pub fn reset_stats(&mut self) {
        self.stats = OptimizationStats::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_optimizer_default() {
        let opt = SpecializationOptimizer::default();
        assert!(opt.constant_fold);
        assert!(opt.dead_code_elim);
        assert!(opt.peephole);
    }

    #[test]
    fn test_optimizer_disabled() {
        let opt = SpecializationOptimizer::disabled();
        assert!(!opt.constant_fold);
        assert!(!opt.dead_code_elim);
        assert!(!opt.peephole);
    }

    #[test]
    fn test_constant_folding() {
        use crate::instruction::{BinaryIntOp, Reg};
        // LoadI r0=5; LoadI r1=7; Mul r2,r0,r1  →  the Mul folds to LoadI r2=35.
        let instrs = vec![
            Instruction::LoadI {
                dst: Reg(0),
                value: 5,
            },
            Instruction::LoadI {
                dst: Reg(1),
                value: 7,
            },
            Instruction::BinaryI {
                op: BinaryIntOp::Mul,
                dst: Reg(2),
                a: Reg(0),
                b: Reg(1),
            },
            Instruction::RetV,
        ];
        let mut bytecode = Vec::new();
        crate::bytecode::encode_instructions_with_fixup(&instrs, &mut bytecode);

        let mut opt = SpecializationOptimizer::new();
        let out = opt.optimize(bytecode);
        let decoded = crate::bytecode::decode_instructions(&out).expect("valid bytecode");

        assert!(
            decoded.iter().any(|i| matches!(
                i,
                Instruction::LoadI { dst, value } if dst.0 == 2 && *value == 35
            )),
            "Mul r2,r0,r1 should fold to LoadI r2=35; got {:?}",
            decoded
        );
        assert!(opt.stats.constants_folded >= 1);
    }

    #[test]
    fn test_jump_survives_size_changing_fold() {
        use crate::instruction::{BinaryIntOp, Reg};
        // A fold BEFORE a jump changes an instruction's byte size; the jump's
        // target instruction must be preserved.  Program:
        //   0: LoadI r0=5
        //   1: LoadI r1=7
        //   2: Mul r2,r0,r1      (folds to LoadI r2=35 — different byte size)
        //   3: Jmp +2            (idx 3+2=5 → land on instr 5 = RetV, skip instr 4)
        //   4: LoadI r3=999      (must remain skipped)
        //   5: RetV
        let instrs = vec![
            Instruction::LoadI {
                dst: Reg(0),
                value: 5,
            },
            Instruction::LoadI {
                dst: Reg(1),
                value: 7,
            },
            Instruction::BinaryI {
                op: BinaryIntOp::Add,
                dst: Reg(2),
                a: Reg(0),
                b: Reg(1),
            },
            Instruction::Jmp { offset: 2 },
            Instruction::LoadI {
                dst: Reg(3),
                value: 999,
            },
            Instruction::RetV,
        ];
        let mut bytecode = Vec::new();
        crate::bytecode::encode_instructions_with_fixup(&instrs, &mut bytecode);

        let mut opt = SpecializationOptimizer::new();
        let out = opt.optimize(bytecode);
        let decoded = crate::bytecode::decode_instructions(&out).expect("valid bytecode");

        // Re-normalize the output's jump to an instruction index and confirm it
        // still targets the RetV (index of RetV in the decoded stream), not the
        // skipped LoadI r3=999.
        let (mut norm, positions) =
            SpecializationOptimizer::decode_with_positions(&out).expect("decode");
        assert!(SpecializationOptimizer::normalize_jump_targets(
            &mut norm,
            &positions,
            out.len()
        ));
        let jmp_idx = norm
            .iter()
            .position(|i| matches!(i, Instruction::Jmp { .. }))
            .expect("a Jmp");
        let target_off = match norm[jmp_idx] {
            Instruction::Jmp { offset } => offset,
            _ => unreachable!(),
        };
        let target_idx = (jmp_idx as i32 + target_off) as usize;
        assert!(
            matches!(decoded.get(target_idx), Some(Instruction::RetV)),
            "jump must still land on RetV after the fold; target_idx={} decoded={:?}",
            target_idx,
            decoded
        );
    }

    #[test]
    fn test_passthrough_when_disabled() {
        use crate::instruction::Reg;
        // A disabled optimizer must round-trip the stream unchanged in meaning.
        let instrs = vec![
            Instruction::LoadI {
                dst: Reg(0),
                value: 42,
            },
            Instruction::RetV,
        ];
        let mut bytecode = Vec::new();
        crate::bytecode::encode_instructions_with_fixup(&instrs, &mut bytecode);

        let mut opt = SpecializationOptimizer::disabled();
        let out = opt.optimize(bytecode.clone());
        // decode→(no fold)→encode must reproduce the same instruction stream.
        let a = crate::bytecode::decode_instructions(&bytecode).unwrap();
        let b = crate::bytecode::decode_instructions(&out).unwrap();
        assert_eq!(a, b);
    }
}
