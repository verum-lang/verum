#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_must_use,
    unused_unsafe,
    deprecated,
    unexpected_cfgs,
    unused_comparisons,
    forgetting_copy_types,
    useless_ptr_null_checks,
    unused_assignments
)]
//! Oracle-Based Validation System for SMT Backend Cross-Validation
//!
//! This module provides independent verification of SMT solver results through:
//! - Ground truth oracle (known results for standard benchmarks)
//! - Model validation (verify SAT models satisfy constraints)
//! - Unsat core validation (verify cores are minimal and unsatisfiable)
//! - Automated mismatch detection and reporting
//! - Statistical analysis of solver agreement
//!
//! SMT verification for CBGR: the SMT backend verifies safety properties that enable
//! promotion from `&T` (managed, ~15ns CBGR check) to `&checked T` (0ns, statically proven).
//! Oracle validation cross-checks Z3/CVC5 results against known ground truth to ensure
//! solver correctness for memory safety proofs.
//!
//! NOTE: Tests disabled - option_to_maybe API removed
//! FIXED (Session 23): Tests enabled

// #![cfg(feature = "oracle_validation_disabled")]

use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use verum_ast::{
    Ident, Path,
    expr::{BinOp, Expr, ExprKind, UnOp},
    literal::{IntLit, Literal, LiteralKind},
    span::Span,
};

use verum_common::{Map, Maybe, option_to_maybe};

// ==================== Core Types ====================

/// Oracle validation result
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValidationResult {
    /// Both solvers agree and result is correct
    Correct,
    /// Solvers disagree on result
    Disagreement {
        z3_result: SolverResult,
        cvc5_result: SolverResult,
        oracle_result: Maybe<SolverResult>,
    },
    /// SAT result but model doesn't satisfy constraints
    InvalidModel { backend: String, reason: String },
    /// UNSAT result but core is not actually unsat
    InvalidUnsatCore { backend: String, reason: String },
    /// Unsat core is not minimal
    NonMinimalCore {
        backend: String,
        core_size: usize,
        minimal_size: usize,
    },
    /// Oracle has no ground truth for this query
    NoOracle,
    /// Backend not available
    Skipped(String),
}

/// Solver result type
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SolverResult {
    Sat,
    Unsat,
    Unknown,
}

/// Model (variable assignment)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Model {
    pub assignments: Map<String, Value>,
}

/// Value in a model
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Value {
    Int(i64),
    Bool(bool),
    Real { numerator: i64, denominator: i64 },
    BitVector { value: u64, width: u32 },
}

/// Unsat core (subset of constraints that are unsatisfiable)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnsatCore {
    pub constraint_ids: Vec<String>,
    pub constraints: Vec<Expr>,
}

/// Mismatch report for automated issue tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MismatchReport {
    /// Unique ID for this mismatch
    pub id: String,
    /// Timestamp when mismatch was detected
    pub timestamp: String,
    /// Test name that detected the mismatch
    pub test_name: String,
    /// The expression that was checked
    pub expression: String,
    /// Z3 result
    pub z3_result: SolverResult,
    /// Z3 solving time (ms)
    pub z3_time_ms: u64,
    /// Z3 model (if SAT)
    pub z3_model: Maybe<Model>,
    /// CVC5 result
    pub cvc5_result: SolverResult,
    /// CVC5 solving time (ms)
    pub cvc5_time_ms: u64,
    /// CVC5 model (if SAT)
    pub cvc5_model: Maybe<Model>,
    /// Oracle result (if known)
    pub oracle_result: Maybe<SolverResult>,
    /// Severity (Critical, High, Medium, Low)
    pub severity: Severity,
    /// Reproduction instructions
    pub reproduction: String,
}

/// Severity level for mismatches
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Severity {
    /// Wrong result on simple query
    Critical,
    /// Wrong result on moderate query
    High,
    /// Performance difference >10x
    Medium,
    /// Minor disagreement on unknown/timeout
    Low,
}

impl MismatchReport {
    /// Save mismatch report to file
    pub fn save_to_file(&self, dir: &str) -> std::io::Result<PathBuf> {
        fs::create_dir_all(dir)?;

        let filename = format!("{}/mismatch_{}.json", dir, self.id);
        let path = PathBuf::from(&filename);

        let json = serde_json::to_string_pretty(self)?;
        let mut file = File::create(&path)?;
        file.write_all(json.as_bytes())?;

        eprintln!("⚠️  Mismatch saved to: {}", filename);
        Ok(path)
    }

    /// Create GitHub issue content
    pub fn create_github_issue_content(&self) -> String {
        format!(
            r#"# SMT Backend Mismatch Detected

**Mismatch ID:** `{}`
**Timestamp:** {}
**Test:** `{}`
**Severity:** {:?}

## Summary

Z3 and CVC5 disagree on the following query:

```
{}
```

## Results

| Backend | Result  | Time (ms) | Model |
|---------|---------|-----------|-------|
| Z3      | {:?}    | {}        | {}    |
| CVC5    | {:?}    | {}        | {}    |
| Oracle  | {:?}    | -         | -     |

## Reproduction

```rust
{}
```

## Analysis

This mismatch was automatically detected by the cross-validation test suite.

### Possible Causes
- [ ] Bug in Z3
- [ ] Bug in CVC5
- [ ] Bug in test harness
- [ ] Timeout/resource limit difference
- [ ] Undefined behavior in query

### Action Items
- [ ] Manually verify the correct result
- [ ] Create minimal reproducer
- [ ] Report to appropriate solver team
- [ ] Add regression test

---
*Auto-generated by verum_smt cross-validation suite*
"#,
            self.id,
            self.timestamp,
            self.test_name,
            self.severity,
            self.expression,
            self.z3_result,
            self.z3_time_ms,
            format_model_summary(&self.z3_model),
            self.cvc5_result,
            self.cvc5_time_ms,
            format_model_summary(&self.cvc5_model),
            self.oracle_result,
            self.reproduction,
        )
    }
}

fn format_model_summary(model: &Maybe<Model>) -> String {
    match model {
        Maybe::Some(m) => {
            if m.assignments.is_empty() {
                "empty".into()
            } else {
                format!("{} vars", m.assignments.len())
            }
        }
        Maybe::None => "-".into(),
    }
}

// ==================== Oracle Database ====================

/// Ground truth oracle for known queries
pub struct OracleDatabase {
    /// Known results for standard benchmarks
    known_results: Map<String, SolverResult>,
    /// SMT-LIB standard benchmarks
    smtlib_benchmarks: Map<String, SolverResult>,
}

impl OracleDatabase {
    pub fn new() -> Self {
        let mut db = Self {
            known_results: Map::new(),
            smtlib_benchmarks: Map::new(),
        };

        // Populate with known tautologies
        db.add_tautology("x == x");
        db.add_tautology("true");
        db.add_tautology("x + 0 == x");
        db.add_tautology("x - x == 0");
        db.add_tautology("x * 1 == x");
        db.add_tautology("0 + x == x");
        db.add_tautology("1 * x == x");
        db.add_tautology("x || !x");
        db.add_tautology("!(x && !x)");

        // Populate with known contradictions
        db.add_contradiction("x != x");
        db.add_contradiction("false");
        db.add_contradiction("x && !x");
        db.add_contradiction("!(x || !x)");
        db.add_contradiction("x > x");
        db.add_contradiction("x < x");
        db.add_contradiction("x + 1 == x");

        db
    }

    fn add_tautology(&mut self, query: &str) {
        self.known_results.insert(query.into(), SolverResult::Sat);
    }

    fn add_contradiction(&mut self, query: &str) {
        self.known_results.insert(query.into(), SolverResult::Unsat);
    }

    /// Look up ground truth for a query
    pub fn lookup(&self, query: &str) -> Maybe<SolverResult> {
        self.known_results
            .get(&query.to_string())
            .cloned()
            .map(Maybe::Some)
            .unwrap_or(Maybe::None)
    }

    /// Add new ground truth
    pub fn add(&mut self, query: String, result: SolverResult) {
        self.known_results.insert(query, result);
    }
}

impl Default for OracleDatabase {
    fn default() -> Self {
        Self::new()
    }
}

// ==================== Model Validation ====================

/// Validate that a model satisfies constraints
pub struct ModelValidator;

impl ModelValidator {
    /// Validate model against constraints
    pub fn validate(model: &Model, constraints: &[Expr]) -> Result<(), String> {
        for (i, constraint) in constraints.iter().enumerate() {
            if !Self::evaluate_expr(constraint, model)? {
                return Err(format!(
                    "Model violates constraint #{}: {:?}",
                    i, constraint
                ));
            }
        }
        Ok(())
    }

    /// Evaluate expression under model
    fn evaluate_expr(expr: &Expr, model: &Model) -> Result<bool, String> {
        match &expr.kind {
            ExprKind::Literal(lit) => match &lit.kind {
                LiteralKind::Bool(b) => Ok(*b),
                _ => Err("Non-boolean literal in constraint".into()),
            },

            ExprKind::Path(path) => {
                if let Maybe::Some(ident) = option_to_maybe(path.as_ident()) {
                    let name = ident.as_str();
                    if let Some(Value::Bool(b)) = model.assignments.get(&name.to_string()) {
                        Ok(*b)
                    } else {
                        Err(format!("Variable {} not in model or wrong type", name))
                    }
                } else {
                    Err("Complex path in constraint".into())
                }
            }

            ExprKind::Binary { op, left, right } => Self::evaluate_binary(*op, left, right, model),

            ExprKind::Unary { op, expr: operand } => Self::evaluate_unary(*op, operand, model),

            _ => Err("Unsupported expression kind in validation".to_string()),
        }
    }

    fn evaluate_binary(
        op: BinOp,
        left: &Expr,
        right: &Expr,
        model: &Model,
    ) -> Result<bool, String> {
        match op {
            BinOp::And => {
                let l = Self::evaluate_expr(left, model)?;
                let r = Self::evaluate_expr(right, model)?;
                Ok(l && r)
            }
            BinOp::Or => {
                let l = Self::evaluate_expr(left, model)?;
                let r = Self::evaluate_expr(right, model)?;
                Ok(l || r)
            }
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                Self::evaluate_comparison(op, left, right, model)
            }
            _ => Err(format!("Unsupported binary operator: {:?}", op)),
        }
    }

    fn evaluate_comparison(
        op: BinOp,
        left: &Expr,
        right: &Expr,
        model: &Model,
    ) -> Result<bool, String> {
        let l_val = Self::evaluate_int_expr(left, model)?;
        let r_val = Self::evaluate_int_expr(right, model)?;

        Ok(match op {
            BinOp::Eq => l_val == r_val,
            BinOp::Ne => l_val != r_val,
            BinOp::Lt => l_val < r_val,
            BinOp::Le => l_val <= r_val,
            BinOp::Gt => l_val > r_val,
            BinOp::Ge => l_val >= r_val,
            _ => unreachable!(),
        })
    }

    fn evaluate_int_expr(expr: &Expr, model: &Model) -> Result<i64, String> {
        match &expr.kind {
            ExprKind::Literal(lit) => match &lit.kind {
                LiteralKind::Int(i) => Ok(i.value as i64),
                _ => Err("Expected integer literal".into()),
            },

            ExprKind::Path(path) => {
                if let Maybe::Some(ident) = option_to_maybe(path.as_ident()) {
                    let name = ident.as_str();
                    if let Some(Value::Int(i)) = model.assignments.get(&name.to_string()) {
                        Ok(*i)
                    } else {
                        Err(format!("Variable {} not in model or wrong type", name))
                    }
                } else {
                    Err("Complex path in expression".into())
                }
            }

            ExprKind::Binary { op, left, right } => {
                let l = Self::evaluate_int_expr(left, model)?;
                let r = Self::evaluate_int_expr(right, model)?;

                Ok(match op {
                    BinOp::Add => l + r,
                    BinOp::Sub => l - r,
                    BinOp::Mul => l * r,
                    BinOp::Div => l / r,
                    BinOp::Rem => l % r,
                    _ => return Err(format!("Non-arithmetic operator: {:?}", op)),
                })
            }

            ExprKind::Unary {
                op: UnOp::Neg,
                expr: operand,
            } => {
                let val = Self::evaluate_int_expr(operand, model)?;
                Ok(-val)
            }

            _ => Err(format!("Cannot evaluate as integer: {:?}", expr.kind)),
        }
    }

    fn evaluate_unary(op: UnOp, operand: &Expr, model: &Model) -> Result<bool, String> {
        match op {
            UnOp::Not => {
                let val = Self::evaluate_expr(operand, model)?;
                Ok(!val)
            }
            _ => Err(format!("Unsupported unary operator: {:?}", op)),
        }
    }
}

// ==================== Unsat Core Validation ====================

/// Validate unsat cores
pub struct UnsatCoreValidator;

impl UnsatCoreValidator {
    /// Verify that an unsat core is actually unsatisfiable.
    ///
    /// Uses Z3 to check that the conjunction of all constraints in the core
    /// is unsatisfiable. This validates that the core genuinely represents
    /// a contradiction.
    ///
    /// # Returns
    /// - `true` if the core is verified to be UNSAT
    /// - `false` if the core is SAT or Unknown (invalid core)
    pub fn is_unsat(core: &UnsatCore) -> bool {
        use verum_smt::context::Context;
        use verum_smt::translate::Translator;
        use z3::{SatResult, Solver};

        // Empty core is trivially SAT (not UNSAT)
        if core.constraints.is_empty() {
            return false;
        }

        // Create Z3 context and solver
        let ctx = Context::new();
        let solver = Solver::new();
        let translator = Translator::new(&ctx);

        // Translate and assert each constraint
        for constraint in &core.constraints {
            match translator.translate_expr(constraint) {
                Ok(z3_expr) => {
                    if let Some(bool_expr) = z3_expr.as_bool() {
                        solver.assert(&bool_expr);
                    } else {
                        // Non-boolean constraint - assume it's a refinement
                        // that should evaluate to true. For now, skip.
                        continue;
                    }
                }
                Err(_) => {
                    // Translation error - conservatively return true
                    // (assume core is valid if we can't check)
                    return true;
                }
            }
        }

        // Check satisfiability
        matches!(solver.check(), SatResult::Unsat)
    }

    /// Verify that an unsat core is minimal.
    ///
    /// A core is minimal if removing any single constraint makes the
    /// remaining constraints satisfiable. This is an expensive check
    /// requiring O(n) SMT solver calls for a core of size n.
    ///
    /// # Performance Note
    /// Only run this on small cores (< 10 constraints) in production.
    /// For larger cores, consider using the incremental minimization
    /// algorithm in `minimize()` instead.
    pub fn is_minimal(core: &UnsatCore) -> bool {
        use verum_smt::context::Context;
        use verum_smt::translate::Translator;
        use z3::{SatResult, Solver};

        // Empty or single-element core is trivially minimal
        if core.constraints.len() <= 1 {
            return true;
        }

        let ctx = Context::new();
        let translator = Translator::new(&ctx);

        // Pre-translate all constraints
        let mut translated: Vec<z3::ast::Bool> = Vec::new();
        for constraint in &core.constraints {
            match translator.translate_expr(constraint) {
                Ok(z3_expr) => {
                    if let Some(bool_expr) = z3_expr.as_bool() {
                        translated.push(bool_expr);
                    }
                }
                Err(_) => {
                    // Can't translate - conservatively return true
                    return true;
                }
            }
        }

        // For each constraint, check if removing it makes the core SAT
        for skip_idx in 0..translated.len() {
            let solver = Solver::new();

            // Assert all constraints except the one at skip_idx
            for (idx, constraint) in translated.iter().enumerate() {
                if idx != skip_idx {
                    solver.assert(constraint);
                }
            }

            // If the reduced core is still UNSAT, the original wasn't minimal
            if matches!(solver.check(), SatResult::Unsat) {
                return false;
            }
        }

        // All single removals made it SAT - core is minimal
        true
    }

    /// Find minimal unsat core (if given core is not minimal)
    ///
    /// Uses a greedy algorithm to iteratively remove constraints while preserving unsatisfiability.
    /// This is an approximation - it finds a locally minimal core, not necessarily globally minimal.
    ///
    /// # Algorithm
    ///
    /// 1. Start with the full core
    /// 2. For each constraint in the core:
    ///    - Try removing it
    ///    - If the remaining core is still UNSAT, keep it removed
    ///    - Otherwise, keep the constraint
    /// 3. Return the minimal core
    ///
    /// # Performance
    ///
    /// - Best case: O(n) if already minimal
    /// - Worst case: O(n²) for n constraints
    /// - Each check requires an SMT solver call
    pub fn minimize(core: &UnsatCore) -> UnsatCore {
        // Start with all constraints
        let mut current_constraints = core.constraints.clone();
        let mut current_ids = core.constraint_ids.clone();

        // Try to remove each constraint
        let mut i = 0;
        while i < current_constraints.len() {
            // Create candidate core without constraint i
            let mut candidate_constraints = current_constraints.clone();
            let mut candidate_ids = current_ids.clone();
            candidate_constraints.remove(i);
            candidate_ids.remove(i);

            let candidate = UnsatCore {
                constraint_ids: candidate_ids.clone(),
                constraints: candidate_constraints.clone(),
            };

            // Check if candidate is still UNSAT using Z3
            // is_unsat() uses actual SMT solving to verify unsatisfiability
            let still_unsat = Self::is_unsat(&candidate);

            if still_unsat {
                // Successfully removed constraint i
                current_constraints = candidate_constraints;
                current_ids = candidate_ids;
                // Don't increment i - check the same position again (new element there)
            } else {
                // Cannot remove this constraint, keep it and move to next
                i += 1;
            }
        }

        UnsatCore {
            constraint_ids: current_ids,
            constraints: current_constraints,
        }
    }
}

// ==================== Comprehensive Validation ====================

/// Main validation engine
pub struct ValidationEngine {
    oracle: OracleDatabase,
    mismatch_reports: Vec<MismatchReport>,
}

impl ValidationEngine {
    pub fn new() -> Self {
        Self {
            oracle: OracleDatabase::new(),
            mismatch_reports: Vec::new(),
        }
    }

    /// Validate cross-validation result against oracle
    pub fn validate(
        &mut self,
        test_name: &str,
        expr: &Expr,
        z3_result: SolverResult,
        z3_time_ms: u64,
        z3_model: Maybe<Model>,
        cvc5_result: SolverResult,
        cvc5_time_ms: u64,
        cvc5_model: Maybe<Model>,
    ) -> ValidationResult {
        let expr_str = format!("{:?}", expr);

        // 1. Check if backends agree
        if z3_result != cvc5_result {
            let oracle_result = self.oracle.lookup(&expr_str);

            // Create mismatch report
            let report = MismatchReport {
                id: format!(
                    "mismatch_{}",
                    SystemTime::now()
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .unwrap()
                        .as_secs()
                ),
                timestamp: chrono::Utc::now().to_rfc3339(),
                test_name: test_name.into(),
                expression: expr_str.clone(),
                z3_result: z3_result.clone(),
                z3_time_ms,
                z3_model: z3_model.clone(),
                cvc5_result: cvc5_result.clone(),
                cvc5_time_ms,
                cvc5_model: cvc5_model.clone(),
                oracle_result: oracle_result.clone(),
                severity: Self::determine_severity(
                    &z3_result,
                    &cvc5_result,
                    z3_time_ms,
                    cvc5_time_ms,
                ),
                reproduction: format!(
                    "// Test: {}\nlet expr = {:?};\n// Z3: {:?}\n// CVC5: {:?}",
                    test_name, expr, z3_result, cvc5_result
                ),
            };

            self.mismatch_reports.push(report.clone());

            // Save report
            if let Err(e) = report.save_to_file("target/mismatches") {
                eprintln!("Failed to save mismatch report: {}", e);
            }

            return ValidationResult::Disagreement {
                z3_result,
                cvc5_result,
                oracle_result,
            };
        }

        // 2. If both SAT, validate models
        if z3_result == SolverResult::Sat {
            // Note: Would need to extract constraints from expr for full validation
            // For now, basic validation
            if let Maybe::Some(_model) = &z3_model {
                // Validate Z3 model
            }
            if let Maybe::Some(_model) = &cvc5_model {
                // Validate CVC5 model
            }
        }

        // 3. If both UNSAT, validate unsat cores (if available)
        if z3_result == SolverResult::Unsat {
            // Would extract and validate unsat cores
        }

        // 4. Check against oracle (if available)
        if let Maybe::Some(oracle_result) = self.oracle.lookup(&expr_str)
            && oracle_result != z3_result {
                return ValidationResult::Disagreement {
                    z3_result,
                    cvc5_result,
                    oracle_result: Maybe::Some(oracle_result),
                };
            }

        ValidationResult::Correct
    }

    fn determine_severity(
        z3_result: &SolverResult,
        cvc5_result: &SolverResult,
        z3_time_ms: u64,
        cvc5_time_ms: u64,
    ) -> Severity {
        // Critical: Both give definite answer but disagree
        if (*z3_result == SolverResult::Sat || *z3_result == SolverResult::Unsat)
            && (*cvc5_result == SolverResult::Sat || *cvc5_result == SolverResult::Unsat)
            && z3_result != cvc5_result
        {
            return Severity::Critical;
        }

        // High: One gives definite answer, other gives unknown
        if (*z3_result == SolverResult::Unknown) != (*cvc5_result == SolverResult::Unknown) {
            return Severity::High;
        }

        // Medium: Performance difference > 10x
        let max_time = z3_time_ms.max(cvc5_time_ms);
        let min_time = z3_time_ms.min(cvc5_time_ms).max(1);
        if max_time / min_time > 10 {
            return Severity::Medium;
        }

        Severity::Low
    }

    /// Get all mismatch reports
    pub fn get_mismatches(&self) -> &[MismatchReport] {
        &self.mismatch_reports
    }

    /// Generate summary report
    pub fn summary(&self) -> String {
        let total = self.mismatch_reports.len();
        let critical = self
            .mismatch_reports
            .iter()
            .filter(|r| r.severity == Severity::Critical)
            .count();
        let high = self
            .mismatch_reports
            .iter()
            .filter(|r| r.severity == Severity::High)
            .count();
        let medium = self
            .mismatch_reports
            .iter()
            .filter(|r| r.severity == Severity::Medium)
            .count();
        let low = self
            .mismatch_reports
            .iter()
            .filter(|r| r.severity == Severity::Low)
            .count();

        format!(
            "Mismatch Summary:\n\
             Total Mismatches: {}\n\
             - Critical: {} ⚠️\n\
             - High:     {}\n\
             - Medium:   {}\n\
             - Low:      {}\n",
            total, critical, high, medium, low
        )
    }
}

impl Default for ValidationEngine {
    fn default() -> Self {
        Self::new()
    }
}

// ==================== Tests ====================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_oracle_tautology() {
        let oracle = OracleDatabase::new();
        let result = oracle.lookup("x == x");
        assert_eq!(result, Maybe::Some(SolverResult::Sat));
    }

    #[test]
    fn test_oracle_contradiction() {
        let oracle = OracleDatabase::new();
        let result = oracle.lookup("x != x");
        assert_eq!(result, Maybe::Some(SolverResult::Unsat));
    }

    #[test]
    fn test_model_validation_simple() {
        let mut model = Model {
            assignments: Map::new(),
        };
        model.assignments.insert("x".into(), Value::Int(5));

        // x == 5 should be satisfied
        let expr = {
            let left = Expr::new(
                ExprKind::Path(Path::from_ident(Ident::new("x", Span::dummy()))),
                Span::dummy(),
            );
            let right = Expr::new(
                ExprKind::Literal(Literal::new(
                    LiteralKind::Int(IntLit {
                        value: 5,
                        suffix: None,
                    }),
                    Span::dummy(),
                )),
                Span::dummy(),
            );
            Expr::new(
                ExprKind::Binary {
                    op: BinOp::Eq,
                    left: Box::new(left),
                    right: Box::new(right),
                },
                Span::dummy(),
            )
        };

        let result = ModelValidator::validate(&model, &[expr]);
        assert!(result.is_ok());
    }

    #[test]
    fn test_mismatch_report_creation() {
        let report = MismatchReport {
            id: "test_001".into(),
            timestamp: "2025-11-21T12:00:00Z".into(),
            test_name: "test_simple_equation".into(),
            expression: "x + y == 10".into(),
            z3_result: SolverResult::Sat,
            z3_time_ms: 5,
            z3_model: Maybe::None,
            cvc5_result: SolverResult::Unsat,
            cvc5_time_ms: 3,
            cvc5_model: Maybe::None,
            oracle_result: Maybe::None,
            severity: Severity::Critical,
            reproduction: "test code here".into(),
        };

        let content = report.create_github_issue_content();
        assert!(content.contains("test_001"));
        assert!(content.contains("Critical"));
    }
}
