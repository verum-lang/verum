//! Bit-Vector Decision Procedure — finite-domain reasoning.
//!
//! Many verification problems live in finite domains: cryptography
//! (AES/ChaCha bit operations), embedded systems (CAN frames,
//! register layouts), bounded-precision arithmetic, packet
//! filtering. For these, full integer reasoning is overkill — a
//! decision procedure tailored to *fixed-width bit-vectors* is
//! both faster and more directly useful.
//!
//! This module provides:
//!
//! * [`BitVec`] — fixed-width bit-vector value (up to 64 bits) with
//!   bitwise ops, modular arithmetic, and equality decision
//! * [`BvOp`] — abstract bit-vector operation tree for symbolic
//!   reasoning
//! * [`BvFormula`] — quantifier-free bit-vector formula
//!   (equalities and disequalities of expressions)
//! * [`decide`] — sound + complete decision procedure for QF_BV
//!   formulas over concrete bit-vectors
//!
//! ## Soundness
//!
//! Every operation is performed in `u64` with explicit width
//! masking, so width truncation matches SMT-LIB QF_BV semantics
//! exactly (modulo the 64-bit width cap). Decision results are
//! reproducible and deterministic.
//!
//! ## Status
//!
//! Standalone algebraic core. Integration with the SMT translator
//! (so user-supplied bit-vector formulas reach this decider as a
//! fast path before falling back to Z3) is a future step.

use verum_common::{List, Text};

/// A fixed-width bit-vector value. Width is in [1, 64].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BitVec {
    pub bits: u64,
    pub width: u8,
}

impl BitVec {
    /// Construct a bit-vector. Bits beyond `width` are masked off.
    pub fn new(bits: u64, width: u8) -> Self {
        let w = width.min(64).max(1);
        let mask = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
        Self {
            bits: bits & mask,
            width: w,
        }
    }

    pub fn zero(width: u8) -> Self {
        Self::new(0, width)
    }

    pub fn ones(width: u8) -> Self {
        Self::new(u64::MAX, width)
    }

    fn mask(&self) -> u64 {
        if self.width >= 64 {
            u64::MAX
        } else {
            (1u64 << self.width) - 1
        }
    }

    pub fn add(&self, other: &BitVec) -> Result<BitVec, BvError> {
        self.check_widths(other)?;
        Ok(BitVec::new(self.bits.wrapping_add(other.bits), self.width))
    }

    pub fn sub(&self, other: &BitVec) -> Result<BitVec, BvError> {
        self.check_widths(other)?;
        Ok(BitVec::new(self.bits.wrapping_sub(other.bits), self.width))
    }

    pub fn mul(&self, other: &BitVec) -> Result<BitVec, BvError> {
        self.check_widths(other)?;
        Ok(BitVec::new(self.bits.wrapping_mul(other.bits), self.width))
    }

    pub fn and(&self, other: &BitVec) -> Result<BitVec, BvError> {
        self.check_widths(other)?;
        Ok(BitVec::new(self.bits & other.bits, self.width))
    }

    pub fn or(&self, other: &BitVec) -> Result<BitVec, BvError> {
        self.check_widths(other)?;
        Ok(BitVec::new(self.bits | other.bits, self.width))
    }

    pub fn xor(&self, other: &BitVec) -> Result<BitVec, BvError> {
        self.check_widths(other)?;
        Ok(BitVec::new(self.bits ^ other.bits, self.width))
    }

    pub fn not(&self) -> BitVec {
        BitVec::new(!self.bits & self.mask(), self.width)
    }

    /// Logical (zero-fill) shift left.
    pub fn shl(&self, n: u8) -> BitVec {
        if n >= self.width {
            return BitVec::zero(self.width);
        }
        BitVec::new(self.bits << n, self.width)
    }

    /// Logical shift right (zero-fill).
    pub fn lshr(&self, n: u8) -> BitVec {
        if n >= self.width {
            return BitVec::zero(self.width);
        }
        BitVec::new(self.bits >> n, self.width)
    }

    fn check_widths(&self, other: &BitVec) -> Result<(), BvError> {
        if self.width != other.width {
            Err(BvError::WidthMismatch {
                left: self.width,
                right: other.width,
            })
        } else {
            Ok(())
        }
    }
}

impl std::fmt::Display for BitVec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "0x{:x}:bv{}", self.bits, self.width)
    }
}

/// Errors from bit-vector operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BvError {
    WidthMismatch { left: u8, right: u8 },
}

impl std::fmt::Display for BvError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WidthMismatch { left, right } => write!(
                f,
                "bit-vector width mismatch: {}-bit vs {}-bit",
                left, right
            ),
        }
    }
}

impl std::error::Error for BvError {}

/// A symbolic bit-vector expression tree.
#[derive(Debug, Clone, PartialEq)]
pub enum BvOp {
    Const(BitVec),
    Var(Text),
    Add(Box<BvOp>, Box<BvOp>),
    Sub(Box<BvOp>, Box<BvOp>),
    Mul(Box<BvOp>, Box<BvOp>),
    And(Box<BvOp>, Box<BvOp>),
    Or(Box<BvOp>, Box<BvOp>),
    Xor(Box<BvOp>, Box<BvOp>),
    Not(Box<BvOp>),
}

impl BvOp {
    /// Evaluate under a variable assignment. Unbound variables
    /// yield `None`; type mismatches yield `Err`.
    pub fn evaluate(
        &self,
        env: &impl BvEnv,
    ) -> Result<Option<BitVec>, BvError> {
        Ok(match self {
            BvOp::Const(bv) => Some(*bv),
            BvOp::Var(name) => env.lookup(name),
            BvOp::Add(a, b) => binop(a, b, env, |x, y| x.add(y))?,
            BvOp::Sub(a, b) => binop(a, b, env, |x, y| x.sub(y))?,
            BvOp::Mul(a, b) => binop(a, b, env, |x, y| x.mul(y))?,
            BvOp::And(a, b) => binop(a, b, env, |x, y| x.and(y))?,
            BvOp::Or(a, b) => binop(a, b, env, |x, y| x.or(y))?,
            BvOp::Xor(a, b) => binop(a, b, env, |x, y| x.xor(y))?,
            BvOp::Not(a) => match a.evaluate(env)? {
                Some(v) => Some(v.not()),
                None => None,
            },
        })
    }
}

fn binop<F>(
    a: &BvOp,
    b: &BvOp,
    env: &impl BvEnv,
    op: F,
) -> Result<Option<BitVec>, BvError>
where
    F: FnOnce(&BitVec, &BitVec) -> Result<BitVec, BvError>,
{
    let av = a.evaluate(env)?;
    let bv = b.evaluate(env)?;
    match (av, bv) {
        (Some(x), Some(y)) => Ok(Some(op(&x, &y)?)),
        _ => Ok(None),
    }
}

/// Variable assignment lookup.
pub trait BvEnv {
    fn lookup(&self, name: &Text) -> Option<BitVec>;
}

impl BvEnv for std::collections::HashMap<Text, BitVec> {
    fn lookup(&self, name: &Text) -> Option<BitVec> {
        self.get(name).copied()
    }
}

/// A bit-vector formula: equalities and disequalities of expressions.
#[derive(Debug, Clone, PartialEq)]
pub enum BvFormula {
    Eq(BvOp, BvOp),
    Ne(BvOp, BvOp),
    /// Conjunction of sub-formulas — all must hold.
    And(List<BvFormula>),
    /// Disjunction — at least one must hold.
    Or(List<BvFormula>),
}

/// Result of formula decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BvDecision {
    /// Formula is satisfied under the given assignment.
    Satisfied,
    /// Formula is violated.
    Violated,
    /// Formula references unbound variables — undecided.
    Undecided,
}

impl BvDecision {
    pub fn is_satisfied(&self) -> bool {
        matches!(self, BvDecision::Satisfied)
    }
}

/// Decide a bit-vector formula under a variable assignment.
pub fn decide(formula: &BvFormula, env: &impl BvEnv) -> Result<BvDecision, BvError> {
    match formula {
        BvFormula::Eq(a, b) => match (a.evaluate(env)?, b.evaluate(env)?) {
            (Some(x), Some(y)) => Ok(if x == y {
                BvDecision::Satisfied
            } else {
                BvDecision::Violated
            }),
            _ => Ok(BvDecision::Undecided),
        },
        BvFormula::Ne(a, b) => match (a.evaluate(env)?, b.evaluate(env)?) {
            (Some(x), Some(y)) => Ok(if x != y {
                BvDecision::Satisfied
            } else {
                BvDecision::Violated
            }),
            _ => Ok(BvDecision::Undecided),
        },
        BvFormula::And(parts) => {
            let mut any_undecided = false;
            for p in parts.iter() {
                match decide(p, env)? {
                    BvDecision::Violated => return Ok(BvDecision::Violated),
                    BvDecision::Undecided => any_undecided = true,
                    BvDecision::Satisfied => {}
                }
            }
            Ok(if any_undecided {
                BvDecision::Undecided
            } else {
                BvDecision::Satisfied
            })
        }
        BvFormula::Or(parts) => {
            let mut any_undecided = false;
            for p in parts.iter() {
                match decide(p, env)? {
                    BvDecision::Satisfied => return Ok(BvDecision::Satisfied),
                    BvDecision::Undecided => any_undecided = true,
                    BvDecision::Violated => {}
                }
            }
            Ok(if any_undecided {
                BvDecision::Undecided
            } else {
                BvDecision::Violated
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn bv8(n: u64) -> BitVec {
        BitVec::new(n, 8)
    }

    fn bv32(n: u64) -> BitVec {
        BitVec::new(n, 32)
    }

    #[test]
    fn construction_masks_overflow_bits() {
        // 0x1FF on 8 bits → 0xFF
        let v = BitVec::new(0x1FF, 8);
        assert_eq!(v.bits, 0xFF);
        assert_eq!(v.width, 8);
    }

    #[test]
    fn zero_and_ones_constructors() {
        assert_eq!(BitVec::zero(16).bits, 0);
        assert_eq!(BitVec::ones(8).bits, 0xFF);
        assert_eq!(BitVec::ones(16).bits, 0xFFFF);
    }

    #[test]
    fn add_wraps_in_modular_arithmetic() {
        let a = bv8(0xFF);
        let b = bv8(0x01);
        // 0xFF + 0x01 = 0x100 → wraps to 0x00 in 8-bit.
        assert_eq!(a.add(&b).unwrap().bits, 0);
    }

    #[test]
    fn sub_wraps_in_modular_arithmetic() {
        let a = bv8(0x00);
        let b = bv8(0x01);
        // 0 - 1 = -1 → 0xFF in 8-bit.
        assert_eq!(a.sub(&b).unwrap().bits, 0xFF);
    }

    #[test]
    fn bitwise_ops_yield_expected() {
        let a = bv8(0b1100_1010);
        let b = bv8(0b1010_0110);
        assert_eq!(a.and(&b).unwrap().bits, 0b1000_0010);
        assert_eq!(a.or(&b).unwrap().bits, 0b1110_1110);
        assert_eq!(a.xor(&b).unwrap().bits, 0b0110_1100);
    }

    #[test]
    fn not_inverts_within_width() {
        let a = bv8(0b0000_1111);
        assert_eq!(a.not().bits, 0b1111_0000);
    }

    #[test]
    fn shl_zeros_when_shift_exceeds_width() {
        let a = bv8(0xFF);
        assert_eq!(a.shl(8).bits, 0);
        assert_eq!(a.shl(100).bits, 0);
    }

    #[test]
    fn lshr_zero_fills() {
        let a = bv8(0xFF);
        assert_eq!(a.lshr(4).bits, 0x0F);
        assert_eq!(a.lshr(8).bits, 0);
    }

    #[test]
    fn width_mismatch_rejected() {
        let a = bv8(1);
        let b = bv32(1);
        let r = a.add(&b);
        assert!(matches!(r, Err(BvError::WidthMismatch { .. })));
    }

    #[test]
    fn evaluate_constant_succeeds() {
        let env: HashMap<Text, BitVec> = HashMap::new();
        let op = BvOp::Const(bv8(42));
        assert_eq!(op.evaluate(&env).unwrap(), Some(bv8(42)));
    }

    #[test]
    fn evaluate_unbound_var_yields_none() {
        let env: HashMap<Text, BitVec> = HashMap::new();
        let op = BvOp::Var(Text::from("x"));
        assert_eq!(op.evaluate(&env).unwrap(), None);
    }

    #[test]
    fn evaluate_binop_with_two_concrete_args() {
        let mut env: HashMap<Text, BitVec> = HashMap::new();
        env.insert(Text::from("x"), bv8(5));
        env.insert(Text::from("y"), bv8(7));
        let op = BvOp::Add(
            Box::new(BvOp::Var(Text::from("x"))),
            Box::new(BvOp::Var(Text::from("y"))),
        );
        assert_eq!(op.evaluate(&env).unwrap(), Some(bv8(12)));
    }

    #[test]
    fn decide_eq_satisfied() {
        let env: HashMap<Text, BitVec> = HashMap::new();
        let f = BvFormula::Eq(
            BvOp::Const(bv8(5)),
            BvOp::Const(bv8(5)),
        );
        assert_eq!(decide(&f, &env).unwrap(), BvDecision::Satisfied);
    }

    #[test]
    fn decide_eq_violated() {
        let env: HashMap<Text, BitVec> = HashMap::new();
        let f = BvFormula::Eq(
            BvOp::Const(bv8(5)),
            BvOp::Const(bv8(6)),
        );
        assert_eq!(decide(&f, &env).unwrap(), BvDecision::Violated);
    }

    #[test]
    fn decide_eq_undecided_with_unbound() {
        let env: HashMap<Text, BitVec> = HashMap::new();
        let f = BvFormula::Eq(
            BvOp::Var(Text::from("x")),
            BvOp::Const(bv8(5)),
        );
        assert_eq!(decide(&f, &env).unwrap(), BvDecision::Undecided);
    }

    #[test]
    fn decide_and_short_circuits_on_violated() {
        let env: HashMap<Text, BitVec> = HashMap::new();
        let f = BvFormula::And(
            [
                BvFormula::Eq(BvOp::Const(bv8(0)), BvOp::Const(bv8(1))),
                BvFormula::Eq(
                    BvOp::Var(Text::from("never_evaluated")),
                    BvOp::Const(bv8(5)),
                ),
            ]
            .into_iter()
            .collect(),
        );
        // First conjunct violated → whole formula violated regardless.
        assert_eq!(decide(&f, &env).unwrap(), BvDecision::Violated);
    }

    #[test]
    fn decide_or_short_circuits_on_satisfied() {
        let env: HashMap<Text, BitVec> = HashMap::new();
        let f = BvFormula::Or(
            [
                BvFormula::Eq(BvOp::Const(bv8(0)), BvOp::Const(bv8(0))),
                BvFormula::Eq(
                    BvOp::Var(Text::from("dontcare")),
                    BvOp::Const(bv8(5)),
                ),
            ]
            .into_iter()
            .collect(),
        );
        assert_eq!(decide(&f, &env).unwrap(), BvDecision::Satisfied);
    }

    #[test]
    fn decide_xor_self_is_zero() {
        // x XOR x = 0 — classic bit-vector identity.
        let mut env: HashMap<Text, BitVec> = HashMap::new();
        env.insert(Text::from("x"), bv8(0xCA));
        let f = BvFormula::Eq(
            BvOp::Xor(
                Box::new(BvOp::Var(Text::from("x"))),
                Box::new(BvOp::Var(Text::from("x"))),
            ),
            BvOp::Const(bv8(0)),
        );
        assert_eq!(decide(&f, &env).unwrap(), BvDecision::Satisfied);
    }
}
