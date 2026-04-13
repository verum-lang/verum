//! Synthetic Differential Geometry (SDG) — nilpotent infinitesimals.
//!
//! Classical differential geometry defines derivatives through
//! limits and ε-δ arguments. **Synthetic** differential geometry,
//! pioneered by Lawvere and Kock in the 1970s, axiomatises the
//! existence of *nilpotent infinitesimals* — non-zero quantities
//! `d` such that `d² = 0` — and *defines* the derivative directly:
//!
//! ```text
//!     f'(x) = the unique a such that  f(x + d) = f(x) + a·d  for all d ∈ D
//! ```
//!
//! where `D = { d : d² = 0 }` is the object of first-order
//! infinitesimals. The **Kock–Lawvere axiom** asserts this `a`
//! exists and is unique.
//!
//! ## Why this matters for Verum
//!
//! SDG gives derivatives a *type-theoretic* foundation rather than
//! relying on metalevel limit machinery. This integrates cleanly
//! with the dependent-type infrastructure already in place and
//! makes automatic differentiation a **language feature** rather
//! than a separate compiler pass:
//!
//! * Forward-mode AD becomes evaluation `f(x + d)` with `d ∈ D`.
//! * Higher-order derivatives use higher-order infinitesimals
//!   `D_k = { d : d^(k+1) = 0 }`.
//! * The chain rule is a theorem about composition of polynomial
//!   maps in `D`.
//!
//! ## API
//!
//! * [`Infinitesimal`] — a value paired with its nilpotency order
//! * [`InfinitesimalRing`] — symbolic computation in `R[d]/(d^(k+1))`
//! * [`derivative`] — extract f'(x) by polynomial expansion
//! * [`PolynomialMap`] — finite-degree polynomial f: R → R
//!
//! This is the **standalone algebraic core**. Integration into the
//! type system (typing `D` as a refinement of `Float`, allowing
//! `f(x + d)` syntax) is a future step.

use verum_common::List;

/// An infinitesimal of a given nilpotency order. `order = k` means
/// `d^(k+1) = 0`. The standard "first-order" infinitesimal has
/// `order = 1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Infinitesimal {
    pub order: u32,
}

impl Infinitesimal {
    /// First-order: `d² = 0`. The canonical case.
    pub fn first_order() -> Self {
        Self { order: 1 }
    }

    pub fn of_order(k: u32) -> Self {
        Self { order: k }
    }
}

/// A polynomial in one variable with f64 coefficients,
/// stored low-to-high (index = degree).
///
/// `[3.0, 2.0, 5.0]` represents `3 + 2x + 5x²`.
#[derive(Debug, Clone, PartialEq)]
pub struct PolynomialMap {
    pub coefficients: List<f64>,
}

impl PolynomialMap {
    pub fn new<I: IntoIterator<Item = f64>>(coeffs: I) -> Self {
        Self {
            coefficients: coeffs.into_iter().collect(),
        }
    }

    pub fn constant(c: f64) -> Self {
        Self::new([c])
    }

    pub fn identity() -> Self {
        Self::new([0.0, 1.0])
    }

    pub fn degree(&self) -> usize {
        self.coefficients.len().saturating_sub(1)
    }

    /// Standard polynomial evaluation at `x`.
    pub fn evaluate(&self, x: f64) -> f64 {
        let mut acc = 0.0;
        let mut x_pow = 1.0;
        for c in self.coefficients.iter() {
            acc += c * x_pow;
            x_pow *= x;
        }
        acc
    }

    /// Symbolic derivative as a new polynomial. The coefficient of
    /// `x^i` in `f'` is `(i+1) * coefficients[i+1]`.
    pub fn derivative_polynomial(&self) -> PolynomialMap {
        if self.coefficients.len() <= 1 {
            return PolynomialMap::constant(0.0);
        }
        let new: Vec<f64> = self
            .coefficients
            .iter()
            .enumerate()
            .skip(1)
            .map(|(i, c)| (i as f64) * c)
            .collect();
        PolynomialMap::new(new)
    }

    /// Evaluate `f(x + d)` truncated at order `inf.order` (so any
    /// term with `d^k` for `k > inf.order` is dropped — that is
    /// the Kock–Lawvere quotient).
    ///
    /// Returns the resulting polynomial in `d`, with coefficients
    /// `[f(x), f'(x), f''(x)/2!, ..., f^(k)(x)/k!]`.
    pub fn evaluate_at_perturbation(
        &self,
        x: f64,
        inf: Infinitesimal,
    ) -> List<f64> {
        let mut result = vec![0.0; (inf.order as usize) + 1];

        // For each coefficient c_n of f, contribute c_n * (x + d)^n
        // expanded via the binomial theorem and truncated.
        for (n, c) in self.coefficients.iter().enumerate() {
            // (x + d)^n = sum_{k=0}^{n} C(n, k) * x^(n-k) * d^k
            // Truncate: k ≤ inf.order
            let max_k = (n).min(inf.order as usize);
            for k in 0..=max_k {
                let coeff = binomial(n, k) as f64 * x.powi((n - k) as i32);
                result[k] += c * coeff;
            }
        }

        result.into_iter().collect()
    }

    /// Extract the derivative `f'(x)` as the coefficient of `d^1`
    /// in the truncated `f(x + d)` expansion. This is the
    /// **Kock–Lawvere** definition: differentiation as polynomial
    /// extraction, not as a limit.
    pub fn derivative_at(&self, x: f64) -> f64 {
        let expansion = self.evaluate_at_perturbation(x, Infinitesimal::first_order());
        if expansion.len() >= 2 {
            expansion[1]
        } else {
            0.0
        }
    }

    /// Higher-order derivative `f^(k)(x) / k!`, as the coefficient
    /// of `d^k` in the truncated `(x + d)` expansion.
    pub fn nth_derivative_coeff_at(&self, x: f64, k: u32) -> f64 {
        let expansion = self.evaluate_at_perturbation(x, Infinitesimal::of_order(k));
        let idx = k as usize;
        if idx < expansion.len() {
            expansion[idx]
        } else {
            0.0
        }
    }
}

/// Binomial coefficient C(n, k). Computed iteratively to avoid
/// factorial overflow for moderate n.
fn binomial(n: usize, k: usize) -> u64 {
    if k > n {
        return 0;
    }
    let k = k.min(n - k);
    let mut result: u64 = 1;
    for i in 0..k {
        result = result * (n - i) as u64 / (i + 1) as u64;
    }
    result
}

/// Verifies the Kock–Lawvere axiom: for the given polynomial f,
/// the unique `a` such that `f(x + d) = f(x) + a·d (mod d²)` is
/// `f'(x)` as computed both by the polynomial-derivative rule and
/// by the SDG perturbation extraction. Returns true iff the two
/// definitions agree to within `tolerance`.
pub fn kock_lawvere_holds(
    f: &PolynomialMap,
    x: f64,
    tolerance: f64,
) -> bool {
    let classical = f.derivative_polynomial().evaluate(x);
    let synthetic = f.derivative_at(x);
    (classical - synthetic).abs() < tolerance
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx_eq(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn binomial_small_values() {
        assert_eq!(binomial(0, 0), 1);
        assert_eq!(binomial(5, 0), 1);
        assert_eq!(binomial(5, 5), 1);
        assert_eq!(binomial(5, 2), 10);
        assert_eq!(binomial(6, 3), 20);
    }

    #[test]
    fn first_order_infinitesimal_is_one() {
        let d = Infinitesimal::first_order();
        assert_eq!(d.order, 1);
    }

    #[test]
    fn polynomial_constant_evaluates_to_constant() {
        let f = PolynomialMap::constant(7.0);
        assert!(approx_eq(f.evaluate(0.0), 7.0));
        assert!(approx_eq(f.evaluate(100.0), 7.0));
    }

    #[test]
    fn polynomial_identity_evaluates_to_x() {
        let f = PolynomialMap::identity();
        assert!(approx_eq(f.evaluate(3.0), 3.0));
        assert!(approx_eq(f.evaluate(-1.5), -1.5));
    }

    #[test]
    fn polynomial_quadratic_evaluation() {
        // f(x) = 3 + 2x + 5x²  →  f(2) = 3 + 4 + 20 = 27
        let f = PolynomialMap::new([3.0, 2.0, 5.0]);
        assert!(approx_eq(f.evaluate(2.0), 27.0));
    }

    #[test]
    fn derivative_of_constant_is_zero() {
        let f = PolynomialMap::constant(7.0);
        assert!(approx_eq(f.derivative_at(5.0), 0.0));
    }

    #[test]
    fn derivative_of_identity_is_one() {
        let f = PolynomialMap::identity();
        assert!(approx_eq(f.derivative_at(0.0), 1.0));
        assert!(approx_eq(f.derivative_at(99.0), 1.0));
    }

    #[test]
    fn derivative_of_quadratic_is_linear() {
        // f(x) = x²  →  f'(x) = 2x
        let f = PolynomialMap::new([0.0, 0.0, 1.0]);
        assert!(approx_eq(f.derivative_at(0.0), 0.0));
        assert!(approx_eq(f.derivative_at(3.0), 6.0));
        assert!(approx_eq(f.derivative_at(-2.0), -4.0));
    }

    #[test]
    fn derivative_of_cubic_at_x() {
        // f(x) = x³  →  f'(x) = 3x²
        let f = PolynomialMap::new([0.0, 0.0, 0.0, 1.0]);
        assert!(approx_eq(f.derivative_at(2.0), 12.0));
        assert!(approx_eq(f.derivative_at(-1.0), 3.0));
    }

    #[test]
    fn derivative_polynomial_matches_synthetic() {
        // f(x) = 3 + 2x + 5x²  →  f'(x) = 2 + 10x
        let f = PolynomialMap::new([3.0, 2.0, 5.0]);
        let fprime = f.derivative_polynomial();
        assert!(approx_eq(fprime.evaluate(0.0), 2.0));
        assert!(approx_eq(fprime.evaluate(1.0), 12.0));

        // synthetic agrees
        for &x in &[0.0, 1.0, -2.5, 7.0] {
            let classical = fprime.evaluate(x);
            let synthetic = f.derivative_at(x);
            assert!(approx_eq(classical, synthetic),
                "disagreement at x={}: classical={}, synthetic={}",
                x, classical, synthetic);
        }
    }

    #[test]
    fn kock_lawvere_holds_for_polynomial() {
        let f = PolynomialMap::new([1.0, -3.0, 2.0, 0.5]);
        for &x in &[-2.0, -0.5, 0.0, 1.0, 4.0] {
            assert!(kock_lawvere_holds(&f, x, 1e-9),
                "Kock-Lawvere violated at x={}", x);
        }
    }

    #[test]
    fn second_order_derivative_extracts_2nd_taylor_coeff() {
        // f(x) = x² → f''(x)/2! = 1 (the coefficient of x² in f)
        let f = PolynomialMap::new([0.0, 0.0, 1.0]);
        // Coefficient of d² in (x+d)² is 1 (binomial(2, 2)).
        assert!(approx_eq(f.nth_derivative_coeff_at(0.0, 2), 1.0));
        assert!(approx_eq(f.nth_derivative_coeff_at(7.0, 2), 1.0));
    }

    #[test]
    fn perturbation_expansion_first_order() {
        // f(x) = x²  at x=3 with d² = 0
        // (x + d)² = x² + 2xd + d² → x² + 2xd
        // Expected: [9, 6]   (constant 9 = 3², d-coefficient 2*3 = 6)
        let f = PolynomialMap::new([0.0, 0.0, 1.0]);
        let exp = f.evaluate_at_perturbation(3.0, Infinitesimal::first_order());
        assert_eq!(exp.len(), 2);
        assert!(approx_eq(exp[0], 9.0));
        assert!(approx_eq(exp[1], 6.0));
    }

    #[test]
    fn perturbation_expansion_second_order() {
        // f(x) = x³ at x=2 with d³ = 0
        // (x + d)³ = x³ + 3x²d + 3xd² + d³ → 8 + 12d + 6d² (mod d³)
        let f = PolynomialMap::new([0.0, 0.0, 0.0, 1.0]);
        let exp = f.evaluate_at_perturbation(2.0, Infinitesimal::of_order(2));
        assert_eq!(exp.len(), 3);
        assert!(approx_eq(exp[0], 8.0));
        assert!(approx_eq(exp[1], 12.0));
        assert!(approx_eq(exp[2], 6.0));
    }

    #[test]
    fn polynomial_degree_reports_correctly() {
        assert_eq!(PolynomialMap::constant(5.0).degree(), 0);
        assert_eq!(PolynomialMap::identity().degree(), 1);
        assert_eq!(PolynomialMap::new([1.0, 2.0, 3.0, 4.0]).degree(), 3);
    }

    #[test]
    fn empty_polynomial_evaluates_to_zero() {
        let f = PolynomialMap::new(Vec::<f64>::new());
        assert!(approx_eq(f.evaluate(42.0), 0.0));
    }
}
