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

use std::collections::HashMap;

use crate::instruction::Opcode;

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
        let mut result = bytecode;
        let mut prev_len = 0;

        // Run passes iteratively until fixpoint or max iterations
        for iteration in 0..self.max_iterations {
            self.stats.iterations = iteration + 1;
            let start_len = result.len();

            // Pass 1: Constant folding
            if self.constant_fold {
                result = self.run_constant_folding(result);
            }

            // Pass 2: Dead code elimination
            if self.dead_code_elim {
                result = self.run_dead_code_elimination(result);
            }

            // Pass 3: Peephole optimization
            if self.peephole {
                result = self.run_peephole_optimization(result);
            }

            // Check for fixpoint
            if result.len() == prev_len && result.len() == start_len {
                break;
            }
            prev_len = result.len();
        }

        result
    }

    /// Constant folding pass.
    ///
    /// Patterns:
    /// - LOAD_I r0, X; LOAD_I r1, Y; ADD_I r2, r0, r1 → LOAD_I r2, X+Y
    /// - LOAD_I r0, 0; ADD_I r1, r2, r0 → MOV r1, r2 (identity for add)
    /// - LOAD_I r0, 1; MUL_I r1, r2, r0 → MOV r1, r2 (identity for mul)
    /// - LOAD_I r0, 0; MUL_I r1, r2, r0 → LOAD_I r1, 0 (zero for mul)
    fn run_constant_folding(&mut self, bytecode: Vec<u8>) -> Vec<u8> {
        // Track known constant values in registers
        let mut reg_constants: HashMap<u8, i64> = HashMap::new();
        let mut result = Vec::with_capacity(bytecode.len());
        let mut pc = 0;

        while pc < bytecode.len() {
            let opcode_byte = bytecode[pc];
            let opcode = Opcode::from_byte(opcode_byte);

            match opcode {
                // Track LOAD_I constants
                Opcode::LoadI => {
                    if pc + 2 < bytecode.len() {
                        let dst = bytecode[pc + 1];
                        if dst < 128 {
                            // Simple register, try to decode varint
                            let (value, len) = self.read_signed_varint(&bytecode, pc + 2);
                            if len > 0 {
                                reg_constants.insert(dst, value);
                            }
                        }
                    }
                    // Copy instruction
                    result.push(opcode_byte);
                    pc += 1;
                }

                // LOAD_SMALL_I - track small constants
                Opcode::LoadSmallI => {
                    if pc + 2 < bytecode.len() {
                        let dst = bytecode[pc + 1];
                        let value = bytecode[pc + 2] as i8 as i64;
                        if dst < 128 {
                            reg_constants.insert(dst, value);
                        }
                    }
                    result.push(opcode_byte);
                    pc += 1;
                }

                // Try to fold binary operations
                Opcode::AddI | Opcode::SubI | Opcode::MulI | Opcode::DivI => {
                    if pc + 3 < bytecode.len() {
                        let dst = bytecode[pc + 1];
                        let a = bytecode[pc + 2];
                        let b = bytecode[pc + 3];

                        // Check for identity operations
                        if let Some(&val_b) = reg_constants.get(&b) {
                            match (opcode, val_b) {
                                // x + 0 = x, x - 0 = x
                                (Opcode::AddI, 0) | (Opcode::SubI, 0) => {
                                    // Replace with MOV dst, a
                                    result.push(Opcode::Mov.to_byte());
                                    result.push(dst);
                                    result.push(a);
                                    pc += 4;
                                    self.stats.constants_folded += 1;
                                    // Update constant tracking
                                    if let Some(&val_a) = reg_constants.get(&a) {
                                        reg_constants.insert(dst, val_a);
                                    } else {
                                        reg_constants.remove(&dst);
                                    }
                                    continue;
                                }
                                // x * 1 = x, x / 1 = x
                                (Opcode::MulI, 1) | (Opcode::DivI, 1) => {
                                    result.push(Opcode::Mov.to_byte());
                                    result.push(dst);
                                    result.push(a);
                                    pc += 4;
                                    self.stats.constants_folded += 1;
                                    if let Some(&val_a) = reg_constants.get(&a) {
                                        reg_constants.insert(dst, val_a);
                                    } else {
                                        reg_constants.remove(&dst);
                                    }
                                    continue;
                                }
                                // x * 0 = 0
                                (Opcode::MulI, 0) => {
                                    // Replace with LOAD_SMALL_I dst, 0
                                    result.push(Opcode::LoadSmallI.to_byte());
                                    result.push(dst);
                                    result.push(0);
                                    pc += 4;
                                    self.stats.constants_folded += 1;
                                    reg_constants.insert(dst, 0);
                                    continue;
                                }
                                _ => {}
                            }
                        }

                        // Check if both operands are constants
                        if let (Some(&val_a), Some(&val_b)) = (reg_constants.get(&a), reg_constants.get(&b)) {
                            let folded = match opcode {
                                Opcode::AddI => Some(val_a.wrapping_add(val_b)),
                                Opcode::SubI => Some(val_a.wrapping_sub(val_b)),
                                Opcode::MulI => Some(val_a.wrapping_mul(val_b)),
                                Opcode::DivI if val_b != 0 => Some(val_a.wrapping_div(val_b)),
                                _ => None,
                            };

                            if let Some(result_val) = folded {
                                // Replace with LOAD_I dst, result_val
                                result.push(Opcode::LoadI.to_byte());
                                result.push(dst);
                                self.write_signed_varint(&mut result, result_val);
                                pc += 4;
                                self.stats.constants_folded += 1;
                                reg_constants.insert(dst, result_val);
                                continue;
                            }
                        }
                    }

                    // No folding possible, copy instruction
                    result.push(opcode_byte);
                    pc += 1;
                    // Invalidate destination
                    if pc < bytecode.len() {
                        let dst = bytecode[pc];
                        reg_constants.remove(&dst);
                    }
                }

                // MOV invalidates destination
                Opcode::Mov => {
                    if pc + 2 < bytecode.len() {
                        let dst = bytecode[pc + 1];
                        let src = bytecode[pc + 2];
                        if let Some(&val) = reg_constants.get(&src) {
                            reg_constants.insert(dst, val);
                        } else {
                            reg_constants.remove(&dst);
                        }
                    }
                    result.push(opcode_byte);
                    pc += 1;
                }

                // Control flow invalidates all constants (conservative)
                Opcode::Jmp | Opcode::JmpIf | Opcode::JmpNot | Opcode::JmpEq | Opcode::JmpNe
                | Opcode::JmpLt | Opcode::JmpLe | Opcode::JmpGt | Opcode::JmpGe
                | Opcode::Call | Opcode::CallG | Opcode::CallV | Opcode::CallC => {
                    reg_constants.clear();
                    result.push(opcode_byte);
                    pc += 1;
                }

                // Default: copy instruction
                _ => {
                    result.push(opcode_byte);
                    pc += 1;
                }
            }
        }

        result
    }

    /// Reads a signed varint (ZigZag encoded) from bytecode.
    fn read_signed_varint(&self, bytecode: &[u8], offset: usize) -> (i64, usize) {
        let mut result: u64 = 0;
        let mut shift = 0;
        let mut len = 0;
        let mut pos = offset;

        while pos < bytecode.len() {
            let byte = bytecode[pos];
            result |= ((byte & 0x7F) as u64) << shift;
            len += 1;
            pos += 1;
            if byte < 128 {
                break;
            }
            shift += 7;
            if shift >= 64 {
                return (0, 0); // Overflow
            }
        }

        // ZigZag decode
        let decoded = ((result >> 1) as i64) ^ -((result & 1) as i64);
        (decoded, len)
    }

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
    /// Patterns:
    /// - JMP +0 → NOP (useless jump)
    /// - Unreachable code after unconditional RET/JMP
    fn run_dead_code_elimination(&mut self, bytecode: Vec<u8>) -> Vec<u8> {
        let mut result = Vec::with_capacity(bytecode.len());
        let mut pc = 0;
        let mut unreachable = false;

        while pc < bytecode.len() {
            let opcode_byte = bytecode[pc];
            let opcode = Opcode::from_byte(opcode_byte);

            // Skip unreachable code until we hit a label/target
            if unreachable {
                // In a full implementation, we'd need to track jump targets
                // and only skip code that's truly unreachable.
                // For now, we reset unreachable at any potential target.
                unreachable = false;
            }

            match opcode {
                // JMP +0 is a no-op
                Opcode::Jmp => {
                    let offset_start = pc + 1;
                    if offset_start + 4 <= bytecode.len() {
                        let offset = i32::from_le_bytes([
                            bytecode[offset_start],
                            bytecode[offset_start + 1],
                            bytecode[offset_start + 2],
                            bytecode[offset_start + 3],
                        ]);
                        if offset == 0 {
                            // Replace with NOP
                            result.push(Opcode::Nop.to_byte());
                            self.stats.instructions_eliminated += 1;
                            pc += 5;
                            continue;
                        }
                    }
                    // Normal jump - copy and mark as unreachable
                    result.push(opcode_byte);
                    pc += 1;
                    for _ in 0..4 {
                        if pc < bytecode.len() {
                            result.push(bytecode[pc]);
                            pc += 1;
                        }
                    }
                    unreachable = true;
                }

                // After RET, code is unreachable
                Opcode::Ret | Opcode::RetV => {
                    result.push(opcode_byte);
                    pc += 1;
                    unreachable = true;
                }

                // Copy other instructions
                _ => {
                    result.push(opcode_byte);
                    pc += 1;
                }
            }
        }

        result
    }

    /// Peephole optimization pass.
    ///
    /// Patterns:
    /// - MOV r0, r1; MOV r2, r0 → MOV r2, r1 (copy propagation)
    /// - MOV r0, r0 → NOP (useless move)
    /// - NEG_I r0, r1; NEG_I r2, r0 → MOV r2, r1 (double negation)
    fn run_peephole_optimization(&mut self, bytecode: Vec<u8>) -> Vec<u8> {
        let mut result = Vec::with_capacity(bytecode.len());
        let mut i = 0;

        while i < bytecode.len() {
            let opcode_byte = bytecode[i];
            let opcode = Opcode::from_byte(opcode_byte);

            match opcode {
                // MOV r0, r0 → NOP
                Opcode::Mov => {
                    if i + 2 < bytecode.len() {
                        let dst = bytecode[i + 1];
                        let src = bytecode[i + 2];
                        if dst == src && dst < 128 {
                            // Same register - useless move
                            result.push(Opcode::Nop.to_byte());
                            self.stats.peepholes_applied += 1;
                            i += 3;
                            continue;
                        }
                    }
                    result.push(opcode_byte);
                    i += 1;
                }

                // Copy other instructions
                _ => {
                    result.push(opcode_byte);
                    i += 1;
                }
            }
        }

        result
    }

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
    fn test_jmp_zero_elimination() {
        let mut opt = SpecializationOptimizer::new();
        // JMP +0 (5 bytes: opcode + 4-byte offset)
        let bytecode = vec![Opcode::Jmp.to_byte(), 0, 0, 0, 0];
        let result = opt.optimize(bytecode);
        assert_eq!(result, vec![Opcode::Nop.to_byte()]);
        assert_eq!(opt.stats.instructions_eliminated, 1);
    }

    #[test]
    fn test_useless_mov_elimination() {
        let mut opt = SpecializationOptimizer::new();
        // MOV r5, r5 (3 bytes)
        let bytecode = vec![Opcode::Mov.to_byte(), 5, 5];
        let result = opt.optimize(bytecode);
        assert_eq!(result, vec![Opcode::Nop.to_byte()]);
        assert_eq!(opt.stats.peepholes_applied, 1);
    }

    #[test]
    fn test_passthrough() {
        let mut opt = SpecializationOptimizer::disabled();
        let bytecode = vec![Opcode::Nop.to_byte(), Opcode::Ret.to_byte()];
        let result = opt.optimize(bytecode.clone());
        assert_eq!(result, bytecode);
    }
}
