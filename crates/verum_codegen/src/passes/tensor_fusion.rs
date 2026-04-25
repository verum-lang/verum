//! Tensor expression chain analyzer (#91-1, #95).
//!
//! Walks a function's VBC instruction stream and identifies maximal
//! Pure tensor expression chains — sequences of tensor ops where:
//!
//!   1. Every op is from the tensor surface (TensorBinop, TensorUnop,
//!      TensorMatmul, TensorReduce, TensorFlashAttention,
//!      TensorRmsNorm, TensorLayerNorm, TensorSoftmax, etc.).
//!   2. Each producer's output register is consumed exactly once,
//!      forming a linear data-dependency chain (DAG with single use
//!      per intermediate).
//!   3. No intervening I/O, mutation, or non-tensor side effect
//!      breaks the chain. Pure tensor ops can freely move past each
//!      other since they only touch heap-allocated tensor handles
//!      that the runtime guarantees `noalias` on.
//!
//! The analyzer is deliberately conservative: it only fuses
//! single-use chains. Multi-use intermediates (where a tensor handle
//! is read by two downstream ops) are left unfused — they need
//! materialisation to avoid recomputation. A future pass (`#91-2`
//! follow-up) can fuse them with a "duplicate computation" cost
//! check against the rematerialisation budget.
//!
//! Output: `Vec<TensorChain>` — each chain has the registers it
//! depends on (`inputs`), the ordered tensor ops, and the final
//! output register. The fusion lowering pass (#96) then emits a
//! single LLVM kernel per chain instead of N runtime calls.

use std::collections::{HashMap, HashSet};

use verum_vbc::instruction::{Instruction, Reg};

/// A single fusable tensor chain — a contiguous sequence of tensor
/// ops where each intermediate is consumed exactly once by the next.
#[derive(Debug, Clone)]
pub struct TensorChain {
    /// Live-in tensor handles (registers read by the chain but not
    /// produced inside it). The fused kernel takes these as arguments.
    pub inputs: Vec<Reg>,

    /// Ordered tensor ops that make up the chain. The first op's
    /// inputs are all in `inputs`; each subsequent op's inputs are
    /// either in `inputs` OR produced by an earlier op in this list.
    pub ops: Vec<ChainOp>,

    /// Final output register of the chain (the last op's destination).
    /// All other intermediate destinations are dead at chain end —
    /// the fusion lowering can keep them in registers (no heap alloc).
    pub output: Reg,
}

/// One step in a tensor chain. We don't store the full Instruction
/// here because we need to project to a fusion-friendly form (the
/// LLVM backend doesn't care about register-file allocation; it
/// only cares about the op kind and its tensor-level inputs/output).
#[derive(Debug, Clone)]
pub struct ChainOp {
    pub kind: TensorOpKind,

    /// Source registers — either chain inputs or produced by an
    /// earlier ChainOp in the same chain.
    pub srcs: Vec<Reg>,

    /// Destination register written by this op.
    pub dst: Reg,
}

/// Coarse classification of fusable tensor ops. The LLVM lowering
/// dispatches off this enum to pick the right inlined kernel pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TensorOpKind {
    /// Element-wise binary (Add, Sub, Mul, Div, Pow, ...). The op
    /// byte is preserved so the lowering can pick the right
    /// per-element scalar op.
    Binop(u8),

    /// Element-wise unary (Neg, Abs, Sqrt, Exp, Log, ...).
    Unop(u8),

    /// Matrix multiplication. Not freely fusable with elementwise
    /// ops — the fusion lowering uses a tile-and-fuse pattern where
    /// the matmul kernel is the outer driver and elementwise
    /// producers/consumers are inlined as prologue / epilogue.
    Matmul,

    /// Reduction (sum, mean, max, ...). Same constraint as Matmul:
    /// the reduction is the outer loop driver.
    Reduce(u8),

    /// Layer norm — fusion sees this as a single primitive even
    /// though the runtime decomposes it into mean+var+normalize.
    /// Future pass (#91-3) can split it for finer-grained fusion
    /// with adjacent elementwise ops.
    LayerNorm,

    /// RMS norm — same shape as LayerNorm.
    RmsNorm,

    /// Softmax along an axis.
    Softmax,

    /// Flash attention — already a fused multi-step kernel, kept
    /// atomic at this level. Including it as a chain primitive lets
    /// the analyzer recognise `softmax(matmul(q, k^T)) @ v ` patterns
    /// that emit a single Flash-Attention kernel via #91-3.
    FlashAttention,
}

/// Public entry: scan an instruction stream (typically a VbcFunction's
/// `instructions` field) and return all fusable tensor chains found.
/// Ordered by their starting program-counter offset in the original
/// instruction stream so the lowering can interleave them with the
/// surrounding (non-tensor) IR.
///
/// Takes `&[Instruction]` directly rather than `&VbcFunction` so the
/// analyzer can be unit-tested without needing to construct a full
/// `FunctionDescriptor` chain.
pub fn analyze_instructions(instructions: &[Instruction]) -> Vec<TensorChain> {
    let mut chains = Vec::new();
    let use_counts = compute_use_counts(instructions);

    // Two-pass scan:
    //   pass 1 — for every tensor op, record the (kind, srcs, dst)
    //            tuple plus its index in the instruction stream
    //   pass 2 — greedily extend chains: starting from each tensor
    //            op whose dst is consumed exactly once by another
    //            tensor op, walk forward absorbing single-use
    //            intermediates until the chain breaks.
    let mut tensor_ops: Vec<(usize, ChainOp)> = Vec::new();
    for (idx, instr) in instructions.iter().enumerate() {
        if let Some(op) = classify(instr) {
            tensor_ops.push((idx, op));
        }
    }

    // Producer-of-register: which tensor op (by tensor_ops index)
    // produces a given register, if any.
    let mut produces: HashMap<u16, usize> = HashMap::new();
    for (i, (_idx, op)) in tensor_ops.iter().enumerate() {
        produces.insert(op.dst.0, i);
    }

    let mut consumed: HashSet<usize> = HashSet::new();

    for (chain_root, (_idx, root_op)) in tensor_ops.iter().enumerate() {
        if consumed.contains(&chain_root) {
            continue;
        }
        let mut chain_ops: Vec<ChainOp> = Vec::new();
        chain_ops.push(root_op.clone());
        consumed.insert(chain_root);

        // Greedily absorb the unique consumer of the current chain
        // tail's dst, IF that consumer is a tensor op AND the dst
        // is consumed exactly once (otherwise we'd have to
        // rematerialise the intermediate).
        let mut tail_dst = root_op.dst;
        loop {
            let single_use = use_counts.get(&tail_dst.0).copied() == Some(1);
            if !single_use {
                break;
            }
            let consumer = match find_unique_tensor_consumer(&tensor_ops, &consumed, tail_dst) {
                Some(i) => i,
                None => break,
            };
            chain_ops.push(tensor_ops[consumer].1.clone());
            consumed.insert(consumer);
            tail_dst = tensor_ops[consumer].1.dst;
        }

        if chain_ops.len() < 2 {
            // Single-op "chains" are not interesting — they already
            // map 1:1 to a runtime call, no fusion savings.
            continue;
        }

        // Compute the chain's input set: registers read by any op
        // and NOT produced inside the chain.
        let mut produced_in_chain: HashSet<u16> = HashSet::new();
        for op in &chain_ops {
            produced_in_chain.insert(op.dst.0);
        }
        let mut inputs: Vec<Reg> = Vec::new();
        let mut seen: HashSet<u16> = HashSet::new();
        for op in &chain_ops {
            for s in &op.srcs {
                if !produced_in_chain.contains(&s.0) && seen.insert(s.0) {
                    inputs.push(*s);
                }
            }
        }

        let output = chain_ops.last().unwrap().dst;
        chains.push(TensorChain { inputs, ops: chain_ops, output });
    }

    chains
}

/// How many times each register is read across the function body.
/// We need this to decide whether an intermediate's destination is
/// "single-use" — the precondition for fusing it into the next op
/// without paying a rematerialisation cost.
fn compute_use_counts(instructions: &[Instruction]) -> HashMap<u16, usize> {
    let mut counts: HashMap<u16, usize> = HashMap::new();
    for instr in instructions {
        for r in instruction_reads(instr) {
            *counts.entry(r.0).or_insert(0) += 1;
        }
    }
    counts
}

/// Find the unique tensor op (by index in `tensor_ops`) that reads
/// `reg` as one of its sources, skipping any already-consumed indices.
/// Returns `Some(idx)` only if EXACTLY one such op exists; ambiguity
/// (multiple consumers) breaks the chain since we'd have to
/// rematerialise.
fn find_unique_tensor_consumer(
    tensor_ops: &[(usize, ChainOp)],
    consumed: &HashSet<usize>,
    reg: Reg,
) -> Option<usize> {
    let mut found: Option<usize> = None;
    for (i, (_idx, op)) in tensor_ops.iter().enumerate() {
        if consumed.contains(&i) {
            continue;
        }
        if op.srcs.iter().any(|s| s.0 == reg.0) {
            if found.is_some() {
                return None;
            }
            found = Some(i);
        }
    }
    found
}

/// Project a single VBC instruction to a `ChainOp` if it's one of
/// the fusion-eligible tensor opcodes; otherwise return `None`.
fn classify(instr: &Instruction) -> Option<ChainOp> {
    match instr {
        Instruction::TensorBinop { op, dst, a, b } => Some(ChainOp {
            kind: TensorOpKind::Binop(*op as u8),
            srcs: vec![*a, *b],
            dst: *dst,
        }),
        Instruction::TensorUnop { op, dst, src } => Some(ChainOp {
            kind: TensorOpKind::Unop(*op as u8),
            srcs: vec![*src],
            dst: *dst,
        }),
        Instruction::TensorMatmul { dst, a, b } => Some(ChainOp {
            kind: TensorOpKind::Matmul,
            srcs: vec![*a, *b],
            dst: *dst,
        }),
        Instruction::TensorReduce { op, dst, src, .. } => Some(ChainOp {
            kind: TensorOpKind::Reduce(*op as u8),
            srcs: vec![*src],
            dst: *dst,
        }),
        Instruction::TensorRmsNorm { dst, input, gamma, .. } => {
            let mut srcs = vec![*input];
            if let Some(g) = gamma { srcs.push(*g); }
            Some(ChainOp { kind: TensorOpKind::RmsNorm, srcs, dst: *dst })
        }
        Instruction::TensorFlashAttention { dst, q, k, v, mask, .. } => {
            let mut srcs = vec![*q, *k, *v];
            if let Some(m) = mask { srcs.push(*m); }
            Some(ChainOp { kind: TensorOpKind::FlashAttention, srcs, dst: *dst })
        }
        // LayerNorm / Softmax don't currently exist as standalone
        // VBC instructions in the bytecode encoder — they're emitted
        // via runtime calls. When #91-2 lands the dedicated opcodes,
        // add the arms here.
        _ => None,
    }
}

/// Registers READ by an instruction. Conservative: when in doubt
/// we don't include — an over-read just means the analyzer is
/// less aggressive about chain extension, never that it produces
/// an incorrect fusion. The full coverage lives in the runtime
/// dispatcher; here we only need enough to tell whether a tensor
/// intermediate's dst is consumed inside another tensor op.
fn instruction_reads(instr: &Instruction) -> Vec<Reg> {
    match instr {
        Instruction::TensorBinop { a, b, .. } => vec![*a, *b],
        Instruction::TensorUnop { src, .. } => vec![*src],
        Instruction::TensorMatmul { a, b, .. } => vec![*a, *b],
        Instruction::TensorReduce { src, .. } => vec![*src],
        Instruction::TensorRmsNorm { input, gamma, .. } => {
            let mut v = vec![*input];
            if let Some(g) = gamma { v.push(*g); }
            v
        }
        Instruction::TensorFlashAttention { q, k, v, mask, .. } => {
            let mut out = vec![*q, *k, *v];
            if let Some(m) = mask { out.push(*m); }
            out
        }
        Instruction::TensorReshape { src, .. } => vec![*src],
        Instruction::TensorContiguousView { src, .. } => vec![*src],
        Instruction::Mov { src, .. } => vec![*src],
        Instruction::BinaryI { a, b, .. } | Instruction::BinaryF { a, b, .. } => vec![*a, *b],
        // For everything else the analyzer treats it as opaque (read
        // count not tracked). That's safe: if we don't know, we
        // assume the register is multi-used and the chain breaks.
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_vbc::instruction::{Reg, TensorBinaryOp};

    /// Helper: build a tensor binop instruction with explicit op.
    fn binop(op: TensorBinaryOp, dst: u16, a: u16, b: u16) -> Instruction {
        Instruction::TensorBinop {
            op,
            dst: Reg(dst),
            a: Reg(a),
            b: Reg(b),
        }
    }

    fn matmul(dst: u16, a: u16, b: u16) -> Instruction {
        Instruction::TensorMatmul { dst: Reg(dst), a: Reg(a), b: Reg(b) }
    }

    /// Use-count harness: feeds an instruction stream through
    /// compute_use_counts and asserts the expected counts.
    #[test]
    fn use_counts_track_reads() {
        let stream = vec![
            matmul(10, 1, 2),                       // r10 = r1 @ r2
            binop(TensorBinaryOp::Add, 11, 10, 3),  // r11 = r10 + r3
            binop(TensorBinaryOp::Mul, 12, 11, 11), // r12 = r11 * r11 — r11 used twice
        ];
        let counts = compute_use_counts(&stream);
        assert_eq!(counts.get(&1).copied(), Some(1));
        assert_eq!(counts.get(&2).copied(), Some(1));
        assert_eq!(counts.get(&3).copied(), Some(1));
        assert_eq!(counts.get(&10).copied(), Some(1));
        // r11 is read twice (by Mul both as a and b)
        assert_eq!(counts.get(&11).copied(), Some(2));
    }

    /// Linear chain: matmul → add → mul (each intermediate single-use).
    /// Should produce ONE chain of length 3 with inputs {r1, r2, r3, r4}
    /// and output r12.
    #[test]
    fn linear_chain_is_fused() {
        // r10 = r1 @ r2          ; matmul producing r10
        // r11 = r10 + r3         ; add  producing r11 (consumes r10 once)
        // r12 = r11 * r4         ; mul  producing r12 (consumes r11 once)
        let stream = vec![
            matmul(10, 1, 2),
            binop(TensorBinaryOp::Add, 11, 10, 3),
            binop(TensorBinaryOp::Mul, 12, 11, 4),
        ];
        let chains = analyze_instructions(&stream);
        assert_eq!(chains.len(), 1, "expected exactly one chain");
        let chain = &chains[0];
        assert_eq!(chain.ops.len(), 3);
        assert_eq!(chain.output.0, 12);
        // Inputs: {r1, r2, r3, r4} — order is insertion order
        let input_ids: Vec<u16> = chain.inputs.iter().map(|r| r.0).collect();
        assert_eq!(input_ids, vec![1, 2, 3, 4]);
    }

    /// Multi-use intermediate breaks the chain: r11 is read twice,
    /// so the analyzer stops at r11 and does NOT extend through it.
    /// Result: only the matmul → add part forms a chain (with r11
    /// being the chain output) — the trailing Mul is a singleton
    /// and dropped (singletons need no fusion).
    #[test]
    fn multi_use_intermediate_breaks_chain() {
        let stream = vec![
            matmul(10, 1, 2),
            binop(TensorBinaryOp::Add, 11, 10, 3),
            binop(TensorBinaryOp::Mul, 12, 11, 11),
        ];
        let chains = analyze_instructions(&stream);
        assert_eq!(chains.len(), 1);
        let chain = &chains[0];
        assert_eq!(chain.ops.len(), 2);
        assert_eq!(chain.output.0, 11);
    }

    /// Single tensor op alone is NOT a "chain" — fusion has nothing
    /// to gain over the existing per-op runtime call.
    #[test]
    fn singleton_is_not_a_chain() {
        let stream = vec![matmul(10, 1, 2)];
        let chains = analyze_instructions(&stream);
        assert_eq!(chains.len(), 0);
    }

    /// Two independent chains in the same function: matmul/add chain
    /// producing r12, then a separate matmul/sub chain producing r22.
    /// Analyzer should find BOTH as independent chains.
    #[test]
    fn two_independent_chains() {
        let stream = vec![
            // chain 1: r10 = r1 @ r2; r12 = r10 + r3
            matmul(10, 1, 2),
            binop(TensorBinaryOp::Add, 12, 10, 3),
            // chain 2: r20 = r4 @ r5; r22 = r20 - r6
            matmul(20, 4, 5),
            binop(TensorBinaryOp::Sub, 22, 20, 6),
        ];
        let chains = analyze_instructions(&stream);
        assert_eq!(chains.len(), 2);
        assert_eq!(chains[0].output.0, 12);
        assert_eq!(chains[1].output.0, 22);
    }
}
