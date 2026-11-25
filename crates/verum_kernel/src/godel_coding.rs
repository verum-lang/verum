//! Numeric syntax for recursive functions and Cantor coding of finite trees.
//!
//! Cell encodings carry the symbol and arity. All pairing operations produce
//! the exact natural-number code when it fits in a `u64`. The implementation
//! returns `None` on encoding overflow. A code identifies syntax; proof
//! admission checks a separate judgment.

use serde::{Deserialize, Serialize};

// =============================================================================
// Primitive recursive functions
// =============================================================================

/// Primitive recursive function representation (Kleene's normal
/// form, restricted to primitive recursion — no `μ` operator).
///
/// **Closure operations**:
///  * [`PrimitiveRecursive::Zero`] — the constant 0 function.
///  * [`PrimitiveRecursive::Succ`] — the unary successor `n ↦ n+1`.
///  * [`PrimitiveRecursive::Proj { i, k }`] — the `i`-th projection
///  of arity `k` (1-based: `i ∈ [1, k]`).
///  * [`PrimitiveRecursive::Comp { g, hs }`] — composition
///  `g(h_1(x⃗), ..., h_m(x⃗))`.
///  * [`PrimitiveRecursive::PrimRec { g, h }`] — primitive recursion
///  defined by `f(0, x⃗) = g(x⃗)`,
///  `f(n+1, x⃗) = h(n, f(n, x⃗), x⃗)`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PrimitiveRecursive {
    /// `0` — the zero function (arity 0).
    Zero,
    /// `S(n) = n + 1` — successor (arity 1).
    Succ,
    /// `π_i^k(x_1, ..., x_k) = x_i` (1-based indexing).
    Proj {
        /// 1-based projection index.
        i: u32,
        /// Arity of the projection.
        k: u32,
    },
    /// Composition `g(h_1, ..., h_m)`. `g` has arity `m`; every
    /// `h_j` has the same arity `n` (the composition's arity).
    Comp {
        /// The outer function.
        g: Box<PrimitiveRecursive>,
        /// The inner functions.
        hs: Vec<PrimitiveRecursive>,
    },
    /// Primitive recursion: `f(0, x⃗) = g(x⃗)`,
    /// `f(n+1, x⃗) = h(n, f(n, x⃗), x⃗)`.
    PrimRec {
        /// The base case `g`.
        g: Box<PrimitiveRecursive>,
        /// The step case `h`.
        h: Box<PrimitiveRecursive>,
    },
}

impl PrimitiveRecursive {
    /// Infer the unique input arity from the formation rules. In an empty
    /// composition the outer function is nullary and the result is nullary.
    pub fn arity(&self) -> Option<usize> {
        enum Frame<'a> {
            Visit(&'a PrimitiveRecursive),
            Compose(usize),
            Recur,
        }

        let mut work = vec![Frame::Visit(self)];
        let mut arities = Vec::new();
        while let Some(frame) = work.pop() {
            match frame {
                Frame::Visit(Self::Zero) => arities.push(0),
                Frame::Visit(Self::Succ) => arities.push(1),
                Frame::Visit(Self::Proj { i, k }) if *i > 0 && i <= k => arities.push(*k as usize),
                Frame::Visit(Self::Proj { .. }) => return None,
                Frame::Visit(Self::Comp { g, hs }) => {
                    work.push(Frame::Compose(hs.len()));
                    work.extend(hs.iter().rev().map(Frame::Visit));
                    work.push(Frame::Visit(g));
                }
                Frame::Visit(Self::PrimRec { g, h }) => {
                    work.push(Frame::Recur);
                    work.push(Frame::Visit(h));
                    work.push(Frame::Visit(g));
                }
                Frame::Compose(count) => {
                    let start = arities.len() - count - 1;
                    let (outer, inner) = arities[start..].split_first()?;
                    let input = inner.first().copied().unwrap_or(0);
                    if *outer != count || inner.iter().any(|&arity| arity != input) {
                        return None;
                    }
                    arities.truncate(start);
                    arities.push(input);
                }
                Frame::Recur => {
                    let step = arities.pop()?;
                    let base = arities.pop()?;
                    if step != base.checked_add(2)? {
                        return None;
                    }
                    arities.push(base + 1);
                }
            }
        }
        arities.pop()
    }

    /// Evaluate a well-formed term at precisely its declared arity. Each
    /// primitive application and recursion step consumes the caller's budget.
    pub fn eval(&self, args: &[u64], fuel: &mut u64) -> Result<u64, RecursiveEvalError> {
        check_arguments(self.arity(), args.len())?;
        self.eval_formed(args, fuel)
    }

    fn eval_formed(&self, args: &[u64], fuel: &mut u64) -> Result<u64, RecursiveEvalError> {
        use std::rc::Rc;
        enum Frame<'a> {
            Apply(&'a PrimitiveRecursive, Rc<[u64]>),
            Compose(&'a PrimitiveRecursive, usize),
            Step {
                h: &'a PrimitiveRecursive,
                args: Rc<[u64]>,
                index: u64,
            },
        }

        let mut work = vec![Frame::Apply(self, Rc::from(args))];
        let mut values = Vec::new();
        while let Some(frame) = work.pop() {
            *fuel = fuel.checked_sub(1).ok_or(RecursiveEvalError::WorkLimit)?;
            match frame {
                Frame::Apply(Self::Zero, _) => values.push(0),
                Frame::Apply(Self::Succ, args) => {
                    values.push(
                        args[0]
                            .checked_add(1)
                            .ok_or(RecursiveEvalError::WordOverflow)?,
                    );
                }
                Frame::Apply(Self::Proj { i, .. }, args) => values.push(args[*i as usize - 1]),
                Frame::Apply(Self::Comp { g, hs }, args) => {
                    work.push(Frame::Compose(g, hs.len()));
                    work.extend(hs.iter().rev().map(|h| Frame::Apply(h, Rc::clone(&args))));
                }
                Frame::Compose(g, count) => {
                    let args = values.split_off(values.len() - count);
                    work.push(Frame::Apply(g, Rc::from(args)));
                }
                Frame::Apply(Self::PrimRec { g, h }, args) => {
                    let params = Rc::from(&args[1..]);
                    if args[0] != 0 {
                        work.push(Frame::Step { h, args, index: 0 });
                    }
                    work.push(Frame::Apply(g, params));
                }
                Frame::Step { h, args, index } => {
                    let mut step = Vec::with_capacity(args.len() + 1);
                    step.push(index);
                    step.push(values.pop().expect("base or previous step value"));
                    step.extend_from_slice(&args[1..]);
                    if index + 1 < args[0] {
                        work.push(Frame::Step {
                            h,
                            args,
                            index: index + 1,
                        });
                    }
                    work.push(Frame::Apply(h, Rc::from(step)));
                }
            }
        }
        Ok(values.pop().expect("a formed term produces one value"))
    }

    /// Decide syntactic formation. Arithmetic representability requires a
    /// formula and a separate proof in a specified presentation.
    pub fn is_well_formed(&self) -> bool {
        self.arity().is_some()
    }
}

/// Reasons a word-sized evaluator cannot return a mathematical value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecursiveEvalError {
    /// A constructor does not satisfy its formation rule.
    InvalidTerm,
    /// The input vector has the wrong dimension.
    WrongArity {
        /// Number of inputs inferred from the term.
        expected: usize,
        /// Number of inputs supplied by the caller.
        actual: usize,
    },
    /// A natural-number intermediate exceeds the word domain.
    WordOverflow,
    /// The caller's evaluation budget was exhausted.
    WorkLimit,
}

fn check_arguments(arity: Option<usize>, actual: usize) -> Result<(), RecursiveEvalError> {
    let expected = arity.ok_or(RecursiveEvalError::InvalidTerm)?;
    if expected != actual {
        return Err(RecursiveEvalError::WrongArity { expected, actual });
    }
    Ok(())
}

/// Decide membership in the primitive-recursive syntax with checked arities.
pub fn is_primitive_recursive(pr: &PrimitiveRecursive) -> bool {
    pr.is_well_formed()
}

/// A primitive-recursive function extended by zero or more bounded searches.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MuRecursive {
    /// Lift a primitive recursive function.
    Prim(PrimitiveRecursive),
    /// Search the inclusive interval from zero to `bound` for the first zero
    /// of `f(y, args...)`. Every earlier application must be defined.
    MuMin {
        /// The function being minimised.
        f: Box<MuRecursive>,
        /// Inclusive upper search bound.
        bound: u64,
    },
}

impl MuRecursive {
    fn decompose(&self) -> (&PrimitiveRecursive, Vec<u64>) {
        let mut current = self;
        let mut bounds = Vec::new();
        loop {
            match current {
                Self::MuMin { f, bound } => {
                    bounds.push(*bound);
                    current = f;
                }
                Self::Prim(pr) => {
                    bounds.reverse();
                    return (pr, bounds);
                }
            }
        }
    }

    /// Each minimisation binds one input of its body.
    pub fn arity(&self) -> Option<usize> {
        let (pr, bounds) = self.decompose();
        pr.arity()?.checked_sub(bounds.len())
    }

    /// Return the least witness or `Ok(None)` for an undefined bounded
    /// minimisation. Invalid input, overflow and budget exhaustion are errors.
    pub fn eval(&self, args: &[u64], fuel: &mut u64) -> Result<Option<u64>, RecursiveEvalError> {
        let (pr, bounds) = self.decompose();
        check_arguments(
            pr.arity().and_then(|n| n.checked_sub(bounds.len())),
            args.len(),
        )?;
        let mut probe = vec![0; bounds.len()];
        probe.extend_from_slice(args);
        'search: loop {
            let mut value = pr.eval_formed(&probe, fuel)?;
            for (candidate, &bound) in probe.iter_mut().zip(&bounds) {
                if value == 0 {
                    value = *candidate;
                    *candidate = 0;
                } else if *candidate < bound {
                    *candidate += 1;
                    continue 'search;
                } else {
                    return Ok(None);
                }
            }
            return Ok(Some(value));
        }
    }
}

/// Decide membership in the arity-checked bounded-minimisation syntax.
pub fn is_mu_recursive(mu: &MuRecursive) -> bool {
    mu.arity().is_some()
}

// =============================================================================
// Gödel encoding
// =============================================================================

/// Cantor pairing restricted to pairs whose natural-number code fits in a word.
pub fn cantor_pair(a: u64, b: u64) -> Option<u64> {
    let diagonal = u128::from(a.checked_add(b)?);
    u64::try_from(diagonal * (diagonal + 1) / 2 + u128::from(b)).ok()
}

/// Exact inverse on the entire code space, including `u64::MAX`.
pub fn cantor_unpair(code: u64) -> (u64, u64) {
    let code = u128::from(code);
    let diagonal = ((8 * code + 1).isqrt() - 1) / 2;
    let b = code - diagonal * (diagonal + 1) / 2;
    ((diagonal - b) as u64, b as u64)
}

/// Payload of an arity-tagged list. Its length is part of the enclosing code.
pub fn encode_list(symbols: &[u64]) -> Option<u64> {
    symbols
        .iter()
        .rev()
        .try_fold(0, |tail, &head| cantor_pair(head, tail))
}

/// Recover the payload at the given arity. A nonzero remainder belongs to a
/// different encoding. Allocation failure also refuses the decode.
pub fn decode_list(mut code: u64, len: usize) -> Option<Vec<u64>> {
    let mut symbols = Vec::new();
    symbols.try_reserve_exact(len).ok()?;
    for _ in 0..len {
        let (head, tail) = cantor_unpair(code);
        symbols.push(head);
        code = tail;
    }
    (code == 0).then_some(symbols)
}

/// A simple AST-cell encoding: every cell is `(symbol, arity, args)`
/// where `symbol` and `arity` are tagged into a head pair.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GodelEncoding {
    /// Head symbol identifier.
    pub symbol: u64,
    /// Argument-cell encodings (recursively encoded sub-terms).
    pub args: Vec<GodelEncoding>,
}

/// A finite input can encode many zero-valued children. Resource refusal is
/// distinct from an invalid arity tag or a noncanonical nonzero tail.
#[derive(Debug, Copy, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GodelDecodeError {
    #[error("nonzero payload remains after the declared children")]
    TrailingPayload,
    #[error("decoded tree would exceed the node limit")]
    NodeLimit,
    #[error("allocation failed while decoding a syntax tree")]
    Allocation,
}

impl GodelEncoding {
    /// Encode on an explicit work stack. Walking siblings backwards builds the
    /// arity-tagged argument payload directly, with one frame per ancestor.
    pub fn encode(&self) -> Option<u64> {
        fn frame(cell: &GodelEncoding) -> Option<(u64, std::slice::Iter<'_, GodelEncoding>, u64)> {
            let arity = u64::try_from(cell.args.len()).ok()?;
            Some((cantor_pair(cell.symbol, arity)?, cell.args.iter(), 0))
        }
        let mut stack = vec![frame(self)?];
        while let Some((head, children, tail)) = stack.last_mut() {
            if let Some(child) = children.next_back() {
                stack.push(frame(child)?);
            } else {
                let code = cantor_pair(*head, *tail)?;
                stack.pop();
                let Some((_, _, parent_tail)) = stack.last_mut() else {
                    return Some(code);
                };
                *parent_tail = cantor_pair(code, *parent_tail)?;
            }
        }
        None
    }

    /// Recover the unique syntax tree at this code, within a caller-supplied
    /// total node budget. Charge each declared child before allocating its slot.
    pub fn decode(code: u64, node_limit: usize) -> Result<Self, GodelDecodeError> {
        struct Frame {
            node: GodelEncoding,
            arity: usize,
            payload: u64,
        }
        fn open(code: u64, available: &mut usize) -> Result<Frame, GodelDecodeError> {
            let (tag, payload) = cantor_unpair(code);
            let (symbol, arity) = cantor_unpair(tag);
            let arity = usize::try_from(arity).map_err(|_| GodelDecodeError::NodeLimit)?;
            *available = available
                .checked_sub(arity)
                .ok_or(GodelDecodeError::NodeLimit)?;
            let mut node = GodelEncoding::atom(symbol);
            node.args
                .try_reserve_exact(arity)
                .map_err(|_| GodelDecodeError::Allocation)?;
            Ok(Frame {
                node,
                arity,
                payload,
            })
        }

        let mut available = node_limit
            .checked_sub(1)
            .ok_or(GodelDecodeError::NodeLimit)?;
        let mut current = open(code, &mut available)?;
        let mut parents = Vec::new();
        loop {
            if current.node.args.len() < current.arity {
                let (child, tail) = cantor_unpair(current.payload);
                current.payload = tail;
                parents
                    .try_reserve(1)
                    .map_err(|_| GodelDecodeError::Allocation)?;
                parents.push(current);
                current = open(child, &mut available)?;
            } else {
                if current.payload != 0 {
                    return Err(GodelDecodeError::TrailingPayload);
                }
                let Some(mut parent) = parents.pop() else {
                    return Ok(current.node);
                };
                parent.node.args.push(current.node);
                current = parent;
            }
        }
    }

    /// Encode an atom `(symbol)` with zero arguments.
    pub fn atom(symbol: u64) -> Self {
        Self {
            symbol,
            args: vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ----- PrimitiveRecursive -----

    #[test]
    fn zero_is_zero() {
        assert_eq!(PrimitiveRecursive::Zero.eval(&[], &mut 1024).unwrap(), 0);
        assert_eq!(
            PrimitiveRecursive::Zero.eval(&[5, 7], &mut 10),
            Err(RecursiveEvalError::WrongArity {
                expected: 0,
                actual: 2
            })
        );
    }

    #[test]
    fn successor_increments() {
        assert_eq!(PrimitiveRecursive::Succ.eval(&[3], &mut 1024).unwrap(), 4);
        assert_eq!(PrimitiveRecursive::Succ.eval(&[0], &mut 1024).unwrap(), 1);
    }

    #[test]
    fn projection_picks_correct_argument() {
        let p13 = PrimitiveRecursive::Proj { i: 1, k: 3 };
        let p23 = PrimitiveRecursive::Proj { i: 2, k: 3 };
        let p33 = PrimitiveRecursive::Proj { i: 3, k: 3 };
        assert_eq!(p13.eval(&[10, 20, 30], &mut 1024).unwrap(), 10);
        assert_eq!(p23.eval(&[10, 20, 30], &mut 1024).unwrap(), 20);
        assert_eq!(p33.eval(&[10, 20, 30], &mut 1024).unwrap(), 30);
    }

    #[test]
    fn projection_well_formed_check() {
        assert!(PrimitiveRecursive::Proj { i: 1, k: 1 }.is_well_formed());
        assert!(PrimitiveRecursive::Proj { i: 3, k: 3 }.is_well_formed());
        assert!(
            !PrimitiveRecursive::Proj { i: 0, k: 1 }.is_well_formed(),
            "0-based projection is invalid"
        );
        assert!(
            !PrimitiveRecursive::Proj { i: 4, k: 3 }.is_well_formed(),
            "Out-of-bounds projection is invalid"
        );
    }

    #[test]
    fn primrec_addition_via_succ_and_proj() {
        // add(0, y) = y = π_1^1(y)
        // add(n+1, y) = S(add(n, y)) = S(π_2^3(n, add(n,y), y))
        let g = PrimitiveRecursive::Proj { i: 1, k: 1 };
        let h = PrimitiveRecursive::Comp {
            g: Box::new(PrimitiveRecursive::Succ),
            hs: vec![PrimitiveRecursive::Proj { i: 2, k: 3 }],
        };
        let add = PrimitiveRecursive::PrimRec {
            g: Box::new(g),
            h: Box::new(h),
        };
        assert_eq!(add.eval(&[3, 5], &mut 1024).unwrap(), 8);
        assert_eq!(add.eval(&[0, 7], &mut 1024).unwrap(), 7);
        assert_eq!(add.eval(&[10, 0], &mut 1024).unwrap(), 10);
    }

    #[test]
    fn primrec_const_function() {
        // const_5 = Comp(Succ × 5, Zero) — fold succ five times
        let mut f: PrimitiveRecursive = PrimitiveRecursive::Zero;
        for _ in 0..5 {
            f = PrimitiveRecursive::Comp {
                g: Box::new(PrimitiveRecursive::Succ),
                hs: vec![f],
            };
        }
        assert_eq!(f.eval(&[], &mut 1024).unwrap(), 5);
    }

    // ----- MuRecursive -----

    #[test]
    fn mu_min_finds_zero_within_bound() {
        let always_zero = zero_at(1);
        let mu = MuRecursive::MuMin {
            f: Box::new(MuRecursive::Prim(always_zero)),
            bound: 100,
        };
        assert_eq!(mu.eval(&[], &mut 1024).unwrap(), Some(0));
    }

    #[test]
    fn mu_min_returns_none_when_no_zero_in_bound() {
        // f(y, x) = Succ(Proj 1 1) — always positive, so μ never finds 0.
        let always_succ = MuRecursive::Prim(PrimitiveRecursive::Comp {
            g: Box::new(PrimitiveRecursive::Succ),
            hs: vec![PrimitiveRecursive::Proj { i: 1, k: 1 }],
        });
        let mu = MuRecursive::MuMin {
            f: Box::new(always_succ),
            bound: 50,
        };
        assert_eq!(mu.eval(&[], &mut 1024).unwrap(), None);
    }

    // ----- Class membership -----

    #[test]
    fn primitive_recursive_class_membership() {
        let f = PrimitiveRecursive::Succ;
        assert!(is_primitive_recursive(&f));
        assert_eq!(f.arity(), Some(1));
    }

    #[test]
    fn mu_recursive_class_membership() {
        let f = MuRecursive::Prim(PrimitiveRecursive::Succ);
        assert!(is_mu_recursive(&f));
    }

    fn zero_at(arity: u32) -> PrimitiveRecursive {
        (0..arity).fold(PrimitiveRecursive::Zero, |base, params| {
            PrimitiveRecursive::PrimRec {
                g: Box::new(base),
                h: Box::new(PrimitiveRecursive::Proj {
                    i: 2,
                    k: params + 2,
                }),
            }
        })
    }

    fn is_zero() -> PrimitiveRecursive {
        PrimitiveRecursive::PrimRec {
            g: Box::new(PrimitiveRecursive::Comp {
                g: Box::new(PrimitiveRecursive::Succ),
                hs: vec![PrimitiveRecursive::Zero],
            }),
            h: Box::new(zero_at(2)),
        }
    }

    #[test]
    fn formation_checks_every_constructor_even_in_unused_positions() {
        use PrimitiveRecursive::*;
        for invalid in [
            Proj { i: 0, k: 1 },
            Comp {
                g: Box::new(Succ),
                hs: vec![],
            },
            Comp {
                g: Box::new(Proj { i: 1, k: 2 }),
                hs: vec![Zero, Succ],
            },
            PrimRec {
                g: Box::new(Zero),
                h: Box::new(Succ),
            },
        ] {
            assert!(!is_primitive_recursive(&invalid));
            assert_eq!(
                invalid.eval(&[], &mut 50),
                Err(RecursiveEvalError::InvalidTerm)
            );
        }
        let closed = MuRecursive::MuMin {
            f: Box::new(MuRecursive::Prim(Zero)),
            bound: 3,
        };
        assert!(!is_mu_recursive(&closed));
        assert_eq!(
            closed.eval(&[], &mut 50),
            Err(RecursiveEvalError::InvalidTerm)
        );
        assert_eq!(
            Succ.eval(&[], &mut 50),
            Err(RecursiveEvalError::WrongArity {
                expected: 1,
                actual: 0
            })
        );
        assert_eq!(
            Succ.eval(&[u64::MAX], &mut 1),
            Err(RecursiveEvalError::WordOverflow)
        );
        assert_eq!(
            zero_at(1).eval(&[u64::MAX], &mut 10),
            Err(RecursiveEvalError::WorkLimit)
        );
    }

    #[test]
    fn bounded_minimization_preserves_first_witness_and_prefix_definedness() {
        use PrimitiveRecursive::*;
        let search = |inner| MuRecursive::MuMin {
            f: Box::new(inner),
            bound: 3,
        };
        assert_eq!(
            search(MuRecursive::Prim(is_zero())).eval(&[], &mut 100),
            Ok(Some(1))
        );
        // f(z, y) = [y = 0]. For y=0 the inner search has no zero.
        // A zero at y=1 cannot repair that earlier undefined application.
        let inner = search(MuRecursive::Prim(Comp {
            g: Box::new(is_zero()),
            hs: vec![Proj { i: 2, k: 2 }],
        }));
        assert_eq!(inner.eval(&[0], &mut 1000), Ok(None));
        assert_eq!(inner.eval(&[1], &mut 1000), Ok(Some(0)));
        assert_eq!(search(inner).eval(&[], &mut 1000), Ok(None));
        // If the inner search always returns zero, the outer first witness is zero.
        let all = search(search(MuRecursive::Prim(zero_at(2))));
        assert_eq!(all.eval(&[], &mut 100), Ok(Some(0)));
    }

    // ----- Cantor pairing -----

    #[test]
    fn cantor_pair_basic_values() {
        assert_eq!(cantor_pair(0, 0), Some(0));
        assert_eq!(cantor_pair(1, 0), Some(1));
        assert_eq!(cantor_pair(0, 1), Some(2));
        assert_eq!(cantor_pair(2, 0), Some(3));
        assert_eq!(cantor_pair(1, 1), Some(4));
        assert_eq!(cantor_pair(0, 2), Some(5));
    }

    #[test]
    fn cantor_pair_unpair_roundtrip() {
        for a in 0..20 {
            for b in 0..20 {
                let z = cantor_pair(a, b).unwrap();
                let (a2, b2) = cantor_unpair(z);
                assert_eq!(
                    (a, b),
                    (a2, b2),
                    "Cantor pair round-trip failure: ({}, {}) → {} → ({}, {})",
                    a,
                    b,
                    z,
                    a2,
                    b2
                );
            }
        }
    }

    #[test]
    fn cantor_pair_is_injective_on_small_inputs() {
        let mut seen = std::collections::HashSet::new();
        for a in 0..30 {
            for b in 0..30 {
                let z = cantor_pair(a, b).unwrap();
                assert!(
                    seen.insert(z),
                    "Cantor pair collision: ({}, {}) → {} already seen",
                    a,
                    b,
                    z
                );
            }
        }
    }

    #[test]
    fn word_boundaries_keep_exact_codes_and_refuse_overflow() {
        let (a, b) = cantor_unpair(u64::MAX);
        assert_eq!(cantor_pair(a, b), Some(u64::MAX));
        assert_eq!(cantor_pair(a + 1, b), None);
        assert_eq!(cantor_pair(u64::MAX, u64::MAX), None);
        // The old saturating product mapped both distinct inputs to 2^63 - 1.
        assert_eq!(cantor_pair(1 << 32, 0), Some((1 << 63) + (1 << 31)));
        assert_eq!(
            cantor_pair((1 << 32) + 1, 0),
            Some((1 << 63) + 3 * (1 << 31) + 1)
        );
        assert_eq!(encode_list(&[0, u64::MAX]), None);
        assert_eq!(decode_list(2, 1), None);
        assert_eq!(decode_list(1, 0), None);
        let nested = GodelEncoding {
            symbol: 0,
            args: vec![GodelEncoding::atom(u64::MAX)],
        };
        assert_eq!(nested.encode(), None);
    }

    proptest::proptest! {
        #[test]
        fn every_word_decodes_on_its_exact_diagonal(code: u64) {
            let (a, b) = cantor_unpair(code);
            proptest::prop_assert_eq!(cantor_pair(a, b), Some(code));
        }
    }

    // ----- List encoding -----

    #[test]
    fn encode_decode_list_roundtrip() {
        for input in [vec![1u64], vec![1, 2, 3], vec![5, 0, 7, 0], vec![]] {
            let z = encode_list(&input).unwrap();
            let out = decode_list(z, input.len()).unwrap();
            assert_eq!(
                out, input,
                "List round-trip failure: {:?} → {} → {:?}",
                input, z, out
            );
        }
    }

    // ----- Godel encoding -----

    #[test]
    fn godel_encode_is_deterministic() {
        let cell = GodelEncoding {
            symbol: 7,
            args: vec![GodelEncoding::atom(3), GodelEncoding::atom(5)],
        };
        let z1 = cell.encode().unwrap();
        let z2 = cell.encode().unwrap();
        assert_eq!(z1, z2);
    }

    #[test]
    fn godel_encode_distinguishes_distinct_cells() {
        let a = GodelEncoding {
            symbol: 1,
            args: vec![GodelEncoding::atom(2)],
        };
        let b = GodelEncoding {
            symbol: 2,
            args: vec![GodelEncoding::atom(1)],
        };
        assert_ne!(a.encode().unwrap(), b.encode().unwrap());
    }

    #[test]
    fn godel_encode_atom_is_simple_pair() {
        let atom = GodelEncoding::atom(7);
        // atom(7) encodes as ⟨⟨7, 0⟩, encode_list([])⟩ = ⟨⟨7, 0⟩, 0⟩
        // = ⟨cantor_pair(7, 0), 0⟩
        let expected = cantor_pair(cantor_pair(7, 0).unwrap(), 0);
        assert_eq!(atom.encode(), expected);
    }

    // ----- Diagonal-paradox primitive integration -----

    #[test]
    fn diagonal_addition_reaches_target() {
        // Demonstrates: primitive recursive add can reach any target.
        let g = PrimitiveRecursive::Proj { i: 1, k: 1 };
        let h = PrimitiveRecursive::Comp {
            g: Box::new(PrimitiveRecursive::Succ),
            hs: vec![PrimitiveRecursive::Proj { i: 2, k: 3 }],
        };
        let add = PrimitiveRecursive::PrimRec {
            g: Box::new(g),
            h: Box::new(h),
        };
        // For every target n, the diagonal x + x = 2n eventually hits 2n.
        for n in 0..10 {
            let result = add.eval(&[n, n], &mut 1024).unwrap();
            assert_eq!(result, 2 * n);
        }
    }
    #[test]
    fn tree_codes_are_canonical_and_charge_all_nodes_before_expansion() {
        let tree = GodelEncoding {
            symbol: 0,
            args: vec![
                GodelEncoding {
                    symbol: 1,
                    args: vec![GodelEncoding::atom(0)],
                },
                GodelEncoding::atom(1),
            ],
        };
        let code = tree.encode().unwrap();
        assert_eq!(GodelEncoding::decode(code, 4), Ok(tree));
        assert_eq!(
            GodelEncoding::decode(code, 3),
            Err(GodelDecodeError::NodeLimit)
        );
        assert_eq!(
            GodelEncoding::decode(0, 0),
            Err(GodelDecodeError::NodeLimit)
        );
        let tag = cantor_pair(0, 100_000).unwrap();
        let wide = cantor_pair(tag, 0).unwrap();
        assert_eq!(
            GodelEncoding::decode(wide, 100_000),
            Err(GodelDecodeError::NodeLimit)
        );
        let expanded = GodelEncoding::decode(wide, 100_001).unwrap();
        assert_eq!(expanded.args.len(), 100_000);
        assert_eq!(expanded.encode(), Some(wide));
        let invalid_leaf = cantor_pair(cantor_pair(0, 0).unwrap(), 1).unwrap();
        let invalid_child = cantor_pair(
            cantor_pair(0, 1).unwrap(),
            cantor_pair(invalid_leaf, 0).unwrap(),
        )
        .unwrap();
        for malformed in [invalid_leaf, invalid_child] {
            assert_eq!(
                GodelEncoding::decode(malformed, 2),
                Err(GodelDecodeError::TrailingPayload)
            );
        }
        for wire in 0..4096 {
            if let Ok(decoded) = GodelEncoding::decode(wire, 256) {
                assert_eq!(decoded.encode(), Some(wire));
            }
        }
    }
}
