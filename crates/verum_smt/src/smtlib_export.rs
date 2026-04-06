//! SMT-LIB2 Export for External Verification
//!
//! This module provides facilities to export verification problems
//! to SMT-LIB2 format for use with external solvers and tools.
//!
//! ## Use Cases
//! - Share verification problems with other SMT solvers (CVC5, Yices, etc.)
//! - Debug verification failures in external tools
//! - Archive verification problems for regression testing
//! - Generate benchmarks for SMT-COMP
//!
//! ## SMT-LIB2 Standard
//! Complies with SMT-LIB 2.6 standard: http://smtlib.cs.uiowa.edu/

use std::fmt::Write as FmtWrite;
use verum_ast::expr::ExprKind;
use verum_ast::literal::LiteralKind;
use verum_ast::ty::TypeKind;
use verum_ast::{BinOp, Expr, Type, UnOp};
use verum_common::{List, Map, Maybe, Text};
use verum_common::ToText;

// ==================== SMT-LIB2 Exporter ====================

/// Exports verification problems to SMT-LIB2 format
pub struct SmtLibExporter {
    /// Variable declarations (public for testing)
    pub var_decls: Map<Text, Text>,
    /// Function declarations (public for testing)
    pub func_decls: Map<Text, (List<Text>, Text)>,
    /// Assertions (public for testing)
    pub assertions: List<Text>,
    /// Check-sat mode (public for testing)
    pub check_mode: CheckMode,
    /// Logic declaration (e.g., QF_LIA, QF_NRA) (public for testing)
    pub logic: Maybe<Text>,
}

impl SmtLibExporter {
    /// Create new SMT-LIB2 exporter
    pub fn new() -> Self {
        Self {
            var_decls: Map::new(),
            func_decls: Map::new(),
            assertions: List::new(),
            check_mode: CheckMode::CheckSat,
            logic: Maybe::None,
        }
    }

    /// Set logic declaration
    pub fn with_logic(mut self, logic: &str) -> Self {
        self.logic = Maybe::Some(logic.to_text());
        self
    }

    /// Set check mode
    pub fn with_check_mode(mut self, mode: CheckMode) -> Self {
        self.check_mode = mode;
        self
    }

    /// Declare a variable
    pub fn declare_var(&mut self, name: &str, ty: &Type) {
        let smt_type = self.type_to_smtlib(ty);
        self.var_decls.insert(name.to_text(), smt_type);
    }

    /// Declare a function
    pub fn declare_function(&mut self, name: &str, arg_types: &[Type], ret_type: &Type) {
        let smt_args: List<Text> = arg_types.iter().map(|t| self.type_to_smtlib(t)).collect();
        let smt_ret = self.type_to_smtlib(ret_type);
        self.func_decls.insert(name.to_text(), (smt_args, smt_ret));
    }

    /// Add an assertion
    pub fn assert(&mut self, expr: &Expr) {
        let smt_expr = self.expr_to_smtlib(expr);
        self.assertions.push(smt_expr);
    }

    /// Add a named assertion (for unsat cores)
    pub fn assert_named(&mut self, expr: &Expr, name: &str) {
        let smt_expr = self.expr_to_smtlib(expr);
        let named = format!("(! {} :named {})", smt_expr, name).into();
        self.assertions.push(named);
    }

    /// Export to SMT-LIB2 format
    pub fn export(&self) -> Text {
        let mut output = Text::new();

        // Logic declaration
        if let Maybe::Some(ref logic) = self.logic {
            let _ = writeln!(&mut output, "(set-logic {})", logic);
        }

        // Variable declarations
        for (name, ty) in &self.var_decls {
            let _ = writeln!(&mut output, "(declare-const {} {})", name, ty);
        }

        // Function declarations
        for (name, (args, ret)) in &self.func_decls {
            if args.is_empty() {
                let _ = writeln!(&mut output, "(declare-const {} {})", name, ret);
            } else {
                let args_str = args
                    .iter()
                    .map(|a| a.as_str())
                    .collect::<List<_>>()
                    .join(" ");
                let _ = writeln!(&mut output, "(declare-fun {} ({}) {})", name, args_str, ret);
            }
        }

        // Assertions
        for assertion in &self.assertions {
            let _ = writeln!(&mut output, "(assert {})", assertion);
        }

        // Check command
        match self.check_mode {
            CheckMode::CheckSat => { let _ = writeln!(&mut output, "(check-sat)"); }
            CheckMode::GetModel => {
                let _ = writeln!(&mut output, "(check-sat)");
                let _ = writeln!(&mut output, "(get-model)");
            }
            CheckMode::GetUnsatCore => {
                let _ = writeln!(&mut output, "(set-option :produce-unsat-cores true)");
                let _ = writeln!(&mut output, "(check-sat)");
                let _ = writeln!(&mut output, "(get-unsat-core)");
            }
            CheckMode::GetProof => {
                let _ = writeln!(&mut output, "(set-option :produce-proofs true)");
                let _ = writeln!(&mut output, "(check-sat)");
                let _ = writeln!(&mut output, "(get-proof)");
            }
        }

        let _ = writeln!(&mut output, "(exit)");

        output
    }

    /// Export to file
    pub fn export_to_file(&self, path: &str) -> Result<(), std::io::Error> {
        let content = self.export();
        std::fs::write(path, content.as_str())
    }

    // ==================== Translation Functions ====================

    /// Translate a Verum type to SMT-LIB format (public for testing)
    pub fn type_to_smtlib(&self, ty: &Type) -> Text {
        match &ty.kind {
            TypeKind::Path(path) => {
                let name = if let Some(ident) = path.as_ident() {
                    ident.as_str()
                } else {
                    "Unknown"
                };

                match name {
                    "Int" | "int" | "i32" | "i64" | "isize" => "Int".to_text(),
                    "Bool" | "bool" => "Bool".to_text(),
                    "Real" | "f32" | "f64" => "Real".to_text(),
                    _ => "Int".to_text(), // Default fallback
                }
            }
            TypeKind::Refined { base, .. } => self.type_to_smtlib(base),
            _ => "Int".to_text(), // Fallback
        }
    }

    /// Translate a Verum expression to SMT-LIB format (public for testing)
    pub fn expr_to_smtlib(&self, expr: &Expr) -> Text {
        match &expr.kind {
            ExprKind::Literal(lit) => self.literal_to_smtlib(lit),
            ExprKind::Path(path) => {
                if let Some(ident) = path.as_ident() {
                    ident.as_str().to_text()
                } else {
                    format!("{:?}", path).into()
                }
            }
            ExprKind::Binary { op, left, right } => {
                let left_smt = self.expr_to_smtlib(left);
                let right_smt = self.expr_to_smtlib(right);
                let op_smt = self.binary_op_to_smtlib(*op);
                format!("({} {} {})", op_smt, left_smt, right_smt).into()
            }
            ExprKind::Unary { op, expr: inner } => {
                let inner_smt = self.expr_to_smtlib(inner);
                let op_smt = self.unary_op_to_smtlib(*op);
                format!("({} {})", op_smt, inner_smt).into()
            }
            ExprKind::Call { func, args, .. } => {
                let func_smt = self.expr_to_smtlib(func);
                let args_smt: List<Text> = args
                    .iter()
                    .map(|a| self.expr_to_smtlib(a).to_text())
                    .collect();
                if args_smt.is_empty() {
                    func_smt
                } else {
                    format!("({} {})", func_smt, args_smt.join(" ")).into()
                }
            }
            ExprKind::Paren(inner) => self.expr_to_smtlib(inner),
            _ => format!("(unknown {:?})", expr.kind).into(),
        }
    }

    /// Translate a literal to SMT-LIB format (public for testing)
    pub fn literal_to_smtlib(&self, lit: &verum_ast::literal::Literal) -> Text {
        match &lit.kind {
            LiteralKind::Bool(b) => (*b).to_text(),
            LiteralKind::Int(i) => i.value.to_text(),
            LiteralKind::Float(f) => f.value.to_text(),
            _ => "0".to_text(),
        }
    }

    fn binary_op_to_smtlib(&self, op: BinOp) -> &'static str {
        match op {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "div",
            BinOp::Rem => "mod",
            BinOp::And => "and",
            BinOp::Or => "or",
            BinOp::Eq => "=",
            BinOp::Ne => "distinct",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::BitAnd => "bvand",
            BinOp::BitOr => "bvor",
            BinOp::BitXor => "bvxor",
            BinOp::Shl => "bvshl",
            BinOp::Shr => "bvlshr",
            _ => "unknown-op",
        }
    }

    fn unary_op_to_smtlib(&self, op: UnOp) -> &'static str {
        match op {
            UnOp::Not => "not",
            UnOp::Neg => "-",
            UnOp::BitNot => "bvnot",
            _ => "unknown-unary",
        }
    }
}

impl Default for SmtLibExporter {
    fn default() -> Self {
        Self::new()
    }
}

// ==================== Check Mode ====================

/// SMT-LIB2 check mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckMode {
    /// Just check satisfiability
    CheckSat,
    /// Check satisfiability and get model
    GetModel,
    /// Check satisfiability and get unsat core
    GetUnsatCore,
    /// Check satisfiability and get proof
    GetProof,
}

// ==================== Convenience Functions ====================

/// Export a verification problem to SMT-LIB2
pub fn export_verification_problem(predicate: &Expr, var_name: &str, var_type: &Type) -> Text {
    let mut exporter = SmtLibExporter::new()
        .with_logic("QF_LIA")
        .with_check_mode(CheckMode::CheckSat);

    exporter.declare_var(var_name, var_type);
    exporter.assert(predicate);

    exporter.export()
}

/// Export refinement type check to SMT-LIB2
///
/// Checks if a value satisfies a refinement predicate.
/// Encodes as: ∃v. ¬predicate(v) (to check for counterexamples)
pub fn export_refinement_check(predicate: &Expr, var_name: &str, var_type: &Type) -> Text {
    let mut exporter = SmtLibExporter::new()
        .with_logic("QF_LIA")
        .with_check_mode(CheckMode::GetModel);

    exporter.declare_var(var_name, var_type);

    // Negate predicate to find counterexample
    let negated = Expr {
        kind: ExprKind::Unary {
            op: UnOp::Not,
            expr: Box::new(predicate.clone()),
        },
        span: predicate.span,
        ref_kind: None,
        check_eliminated: false,
    };

    exporter.assert(&negated);

    exporter.export()
}

/// Export multiple assertions with named tracking (for unsat cores)
pub fn export_with_unsat_core(assertions: &[(Text, Expr)], var_decls: &[(Text, Type)]) -> Text {
    let mut exporter = SmtLibExporter::new()
        .with_logic("QF_LIA")
        .with_check_mode(CheckMode::GetUnsatCore);

    // Declare variables
    for (name, ty) in var_decls {
        exporter.declare_var(name.as_str(), ty);
    }

    // Add named assertions
    for (name, expr) in assertions {
        exporter.assert_named(expr, name.as_str());
    }

    exporter.export()
}

// ==================== Benchmark Generation ====================

/// Generate SMT-COMP benchmark from verification problem
pub struct BenchmarkGenerator {
    category: Text,
    difficulty: Difficulty,
    description: Text,
}

impl BenchmarkGenerator {
    pub fn new(category: &str) -> Self {
        Self {
            category: category.to_text(),
            difficulty: Difficulty::Medium,
            description: Text::new(),
        }
    }

    pub fn with_difficulty(mut self, difficulty: Difficulty) -> Self {
        self.difficulty = difficulty;
        self
    }

    pub fn with_description(mut self, desc: &str) -> Self {
        self.description = desc.to_text();
        self
    }

    /// Generate benchmark file with metadata
    pub fn generate(&self, problem: &SmtLibExporter) -> Text {
        let mut output = Text::new();

        // Metadata comments
        let _ = writeln!(&mut output, "; Category: {}", self.category);
        let _ = writeln!(&mut output, "; Difficulty: {:?}", self.difficulty);
        if !self.description.is_empty() {
            let _ = writeln!(&mut output, "; Description: {}", self.description);
        }
        let _ = writeln!(&mut output, "; Generated by Verum SMT backend");
        let _ = writeln!(&mut output);

        // Problem content
        let _ = write!(&mut output, "{}", problem.export());

        output
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Difficulty {
    Easy,
    Medium,
    Hard,
    Challenge,
}

// ==================== Tests ====================
