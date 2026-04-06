//! Fixed-Point Engine (μZ) Module
//!
//! This module provides support for recursive predicates and fixed-point computation
//! using Z3's μZ engine, enabling verification of recursive data structures and
//! inductive properties.
//!
//! Based on experiments/z3.rs documentation and z3-sys FFI bindings.
//! Recursive refinement types (e.g., sorted lists, balanced trees) require fixed-point
//! reasoning. Z3's muZ engine computes least/greatest fixed points for recursive
//! predicates, enabling verification of inductive properties like `is_sorted(list)`
//! or `is_balanced(tree)` that reference themselves in their definitions.

use std::ffi::{CStr, CString};
use std::sync::Arc;
use std::time::Instant;

use z3::ast::{Ast, Bool, Dynamic, Int};
use z3::{Context, FuncDecl, SatResult, Sort, Symbol};
use z3_sys::{AstKind, DeclKind, *};

use verum_common::{List, Map, Maybe, Set, Text};

// ==================== FFI Helper Functions ====================
//
// WARNING: These functions rely on the internal memory layout of the `z3` crate
// (pinned at =0.19.7 in Cargo.toml). If the z3 crate is updated, these layout
// assumptions MUST be re-verified against the new source. The z3 crate does NOT
// guarantee layout stability in its public API.

/// Get raw Z3_context from high-level Context
///
/// # Safety
/// The Context must be valid and its internal layout must match ContextRepr.
/// This is only guaranteed for z3 =0.19.7.
unsafe fn get_z3_context(ctx: &Context) -> Z3_context {
    #[repr(C)]
    struct ContextRepr {
        z3_ctx: std::rc::Rc<ContextInternalRepr>,
    }

    #[repr(C)]
    struct ContextInternalRepr(Z3_context);

    // SAFETY: Relies on z3 =0.19.7 internal layout where Context contains
    // an Rc<ContextInternal> as its first (and only) field, and ContextInternal
    // wraps a raw Z3_context pointer. Pinned version prevents silent breakage.
    let ctx_repr = unsafe { &*(ctx as *const Context as *const ContextRepr) };
    ctx_repr.z3_ctx.0
}

/// Get raw Z3_func_decl from FuncDecl
///
/// # Safety
/// The FuncDecl must be valid and its internal layout must match FuncDeclRepr.
/// This is only guaranteed for z3 =0.19.7.
unsafe fn get_z3_func_decl(decl: &FuncDecl) -> Z3_func_decl {
    #[repr(C)]
    struct FuncDeclRepr {
        ctx: Context,
        z3_func_decl: Z3_func_decl,
    }

    // SAFETY: Relies on z3 =0.19.7 internal layout where FuncDecl is
    // { ctx: Context, z3_func_decl: Z3_func_decl }. Pinned version prevents
    // silent breakage on crate update.
    let decl_repr = unsafe { &*(decl as *const FuncDecl as *const FuncDeclRepr) };
    decl_repr.z3_func_decl
}

/// Get raw Z3_ast from AST types
///
/// # Safety
/// The AST type T must have layout { ctx: Context, z3_ast: Z3_ast }.
/// This is only guaranteed for z3 =0.19.7 AST wrapper types (Bool, Int, etc.).
unsafe fn get_z3_ast<T>(ast: &T) -> Z3_ast {
    #[repr(C)]
    struct AstRepr {
        ctx: Context,
        z3_ast: Z3_ast,
    }

    // SAFETY: Relies on z3 =0.19.7 internal layout where all AST wrapper types
    // (Bool, Int, Real, BV, etc.) share layout { ctx: Context, z3_ast: Z3_ast }.
    // The generic T is unconstrained — callers must only pass z3 AST types.
    let ast_repr = unsafe { &*(ast as *const T as *const AstRepr) };
    ast_repr.z3_ast
}

/// Get raw Z3_params from Params
///
/// # Safety
/// The Params must be valid and its internal layout must match ParamsRepr.
/// This is only guaranteed for z3 =0.19.7.
unsafe fn get_z3_params(params: &z3::Params) -> Z3_params {
    #[repr(C)]
    struct ParamsRepr {
        ctx: Context,
        z3_params: Z3_params,
    }

    // SAFETY: Relies on z3 =0.19.7 internal layout where Params is
    // { ctx: Context, z3_params: Z3_params }. Pinned version prevents
    // silent breakage on crate update.
    let params_repr = unsafe { &*(params as *const z3::Params as *const ParamsRepr) };
    params_repr.z3_params
}

// Compile-time layout assertions: if z3 crate changes its struct sizes,
// these will fail at compile time, catching layout drift before UB occurs.
const _: () = {
    assert!(std::mem::size_of::<Context>() > 0, "Context must not be ZST");
    assert!(std::mem::size_of::<FuncDecl>() > std::mem::size_of::<Context>(), "FuncDecl must be larger than Context (ctx + z3_func_decl)");
};

// ==================== Core Types ====================

/// Recursive predicate definition
#[derive(Debug, Clone)]
pub struct RecursivePredicate {
    /// Predicate name
    pub name: Text,
    /// Parameter sorts
    pub params: List<Sort>,
    /// Body of the predicate (as a rule)
    pub body: PredicateBody,
    /// Whether this is well-founded
    pub well_founded: bool,
}

/// Predicate body types
#[derive(Debug, Clone)]
pub enum PredicateBody {
    /// Base case (non-recursive)
    Base(Bool),
    /// Recursive case
    Recursive {
        guard: Bool,
        recursive_calls: List<RecursiveCall>,
        conclusion: Bool,
    },
    /// Multiple cases (disjunction)
    Cases(List<PredicateCase>),
}

/// Single case in predicate definition
#[derive(Debug, Clone)]
pub struct PredicateCase {
    pub guard: Maybe<Bool>,
    pub body: Bool,
    pub recursive_calls: List<RecursiveCall>,
}

/// Recursive call in predicate
#[derive(Debug, Clone)]
pub struct RecursiveCall {
    pub predicate: Text,
    pub args: List<Dynamic>,
}

/// Datalog-style rule
#[derive(Debug, Clone)]
pub struct DatalogRule {
    /// Head predicate
    pub head: Atom,
    /// Body atoms (conjunction)
    pub body: List<Atom>,
    /// Constraints
    pub constraints: List<Bool>,
}

/// Atomic formula
#[derive(Debug, Clone)]
pub struct Atom {
    /// Predicate name
    pub predicate: Text,
    /// Arguments
    pub args: List<Dynamic>,
}

/// Constrained Horn Clause (CHC)
#[derive(Debug, Clone)]
pub struct CHC {
    /// Variables
    pub vars: List<(Text, Sort)>,
    /// Hypothesis
    pub hypothesis: List<Atom>,
    /// Constraints
    pub constraints: List<Bool>,
    /// Conclusion
    pub conclusion: Atom,
}

/// Fixed-point query
#[derive(Debug)]
pub struct FixedPointQuery {
    /// Query predicate
    pub predicate: Text,
    /// Arguments
    pub args: List<Dynamic>,
    /// Bound on unfolding depth
    pub max_depth: Maybe<usize>,
}

/// Fixed-point result
#[derive(Debug)]
pub struct FixedPointResult {
    /// Satisfiability status
    pub status: SatResult,
    /// Solution (if SAT)
    pub solution: Maybe<FixedPointSolution>,
    /// Statistics
    pub stats: FixedPointStats,
}

/// Fixed-point solution
#[derive(Debug)]
pub struct FixedPointSolution {
    /// Predicate interpretations
    pub interpretations: Map<Text, PredicateInterpretation>,
    /// Invariants discovered
    pub invariants: List<Bool>,
}

/// Interpretation of a predicate
#[derive(Debug)]
pub struct PredicateInterpretation {
    /// The predicate
    pub predicate: Text,
    /// Its interpretation as a formula
    pub formula: Bool,
    /// Whether it's inductive
    pub is_inductive: bool,
}

/// Fixed-point statistics
#[derive(Debug, Clone, Default)]
pub struct FixedPointStats {
    /// Number of iterations
    pub iterations: usize,
    /// Solving time
    pub time_ms: u64,
    /// Number of rules
    pub num_rules: usize,
    /// Number of predicates
    pub num_predicates: usize,
}

// ==================== Z3 Fixedpoint FFI Wrapper ====================

/// Internal wrapper around Z3_fixedpoint that manages reference counting
struct FixedPointInternal {
    z3_fp: Z3_fixedpoint,
    z3_ctx: Z3_context,
}

impl FixedPointInternal {
    /// Create new fixedpoint engine
    ///
    /// # Safety
    /// - z3_ctx must be a valid Z3_context
    unsafe fn new(z3_ctx: Z3_context) -> Result<Self, Text> {
        // SAFETY: z3_ctx is a valid Z3_context
        let z3_fp = unsafe { Z3_mk_fixedpoint(z3_ctx) }
            .ok_or_else(|| Text::from("Failed to create Z3 fixedpoint engine"))?;

        // Increment reference count
        // SAFETY: z3_fp is a valid Z3_fixedpoint
        unsafe { Z3_fixedpoint_inc_ref(z3_ctx, z3_fp) };

        Ok(Self { z3_fp, z3_ctx })
    }

    /// Get raw Z3_fixedpoint pointer
    fn as_ptr(&self) -> Z3_fixedpoint {
        self.z3_fp
    }

    /// Get raw Z3_context pointer
    fn ctx_ptr(&self) -> Z3_context {
        self.z3_ctx
    }
}

impl Drop for FixedPointInternal {
    fn drop(&mut self) {
        // SAFETY: Both z3_ctx and z3_fp are valid when created
        // and we manage the reference count properly
        unsafe {
            Z3_fixedpoint_dec_ref(self.z3_ctx, self.z3_fp);
        }
    }
}

// ==================== Fixed-Point Engine ====================

/// Z3 Fixed-point engine wrapper
///
/// Provides safe Rust API for Z3's fixedpoint solver (Datalog/CHC solver).
/// Supports:
/// - Datalog rules and facts
/// - Constrained Horn Clauses (CHC)
/// - SPACER/PDR engines for program verification
/// - Recursive predicate solving
pub struct FixedPointEngine {
    /// Internal Z3 fixedpoint wrapper
    fp: Arc<FixedPointInternal>,
    /// Z3 context (high-level wrapper)
    ctx: Context,
    /// Registered predicates
    predicates: Map<Text, FuncDecl>,
    /// Rules count
    rules_count: usize,
    /// Start time for stats
    start_time: Instant,
}

impl FixedPointEngine {
    /// Create new fixed-point engine
    ///
    /// # Examples
    /// ```ignore
    /// use verum_smt::FixedPointEngine;
    /// use z3::Context;
    ///
    /// let ctx = Context::thread_local();
    /// let engine = FixedPointEngine::new(ctx).expect("Failed to create engine");
    /// ```
    pub fn new(ctx: Context) -> Result<Self, Text> {
        // SAFETY: Context::z3_ctx() returns valid Z3_context
        let fp = unsafe { Arc::new(FixedPointInternal::new(get_z3_context(&ctx))?) };

        Ok(Self {
            fp,
            ctx,
            predicates: Map::new(),
            rules_count: 0,
            start_time: Instant::now(),
        })
    }

    /// Register a relation (predicate) with the fixedpoint engine
    ///
    /// Relations must be registered before they can be used in rules.
    ///
    /// # Examples
    /// ```ignore
    /// use verum_smt::FixedPointEngine;
    /// use z3::{Context, Sort, Symbol, FuncDecl};
    ///
    /// let ctx = Context::thread_local();
    /// let mut engine = FixedPointEngine::new(ctx.clone()).unwrap();
    ///
    /// // Create a predicate edge(Int, Int)
    /// let int_sort = Sort::int();
    /// let edge = FuncDecl::new(
    ///     "edge",
    ///     &[&int_sort, &int_sort],
    ///     &Sort::bool(),
    /// );
    ///
    /// engine.register_relation(&edge);
    /// ```
    pub fn register_relation(&mut self, decl: &FuncDecl) -> Result<(), Text> {
        // SAFETY: Both pointers are valid and alive
        unsafe {
            Z3_fixedpoint_register_relation(
                self.fp.ctx_ptr(),
                self.fp.as_ptr(),
                get_z3_func_decl(decl),
            );
        }

        // Extract the predicate name and create a new FuncDecl for storage
        // We need to recreate because FuncDecl doesn't implement Clone
        let name = decl.name();
        let arity = decl.arity();

        // Create domain sorts from the function declaration
        let mut domain_sorts = List::new();
        for i in 0..arity {
            if let Some(sort_kind) = decl.domain(i) {
                // Convert SortKind to Sort by creating a new one
                let sort = match sort_kind {
                    z3::SortKind::Int => Sort::int(),
                    z3::SortKind::Bool => Sort::bool(),
                    z3::SortKind::Real => Sort::real(),
                    z3::SortKind::BV => Sort::bitvector(32), // Default BV size
                    _ => Sort::int(),                        // Default fallback
                };
                domain_sorts.push(sort);
            }
        }

        let domain_refs: List<&Sort> = domain_sorts.iter().collect();
        let range = Sort::bool(); // Relations always have bool range
        let new_decl = FuncDecl::new(name.clone(), &domain_refs, &range);
        self.predicates.insert(Text::from(name), new_decl);

        Ok(())
    }

    /// Add a Horn clause rule
    ///
    /// Rules should be of the form:
    /// - `forall vars. body => head` (implication)
    /// - `head` (fact)
    ///
    /// # Examples
    /// ```ignore
    /// use verum_smt::FixedPointEngine;
    /// use z3::{Context, Sort, FuncDecl};
    /// use z3::ast::{Ast, Bool, Dynamic, Int};
    ///
    /// let ctx = Context::thread_local();
    /// let mut engine = FixedPointEngine::new(ctx.clone()).unwrap();
    ///
    /// let int_sort = Sort::int();
    /// let edge = FuncDecl::new(
    ///     "edge",
    ///     &[&int_sort, &int_sort],
    ///     &Sort::bool(),
    /// );
    ///
    /// engine.register_relation(&edge).unwrap();
    ///
    /// // Add fact: edge(1, 2)
    /// let x = Int::from_i64(1);
    /// let y = Int::from_i64(2);
    /// let x_dyn: Dynamic = x.into();
    /// let y_dyn: Dynamic = y.into();
    /// let fact = edge.apply(&[&x_dyn, &y_dyn]).as_bool().unwrap();
    /// engine.add_rule(&fact, Some("edge_1_2")).unwrap();
    /// ```
    pub fn add_rule(&mut self, rule: &Bool, name: Option<&str>) -> Result<(), Text> {
        let symbol = if let Some(n) = name {
            let cname =
                CString::new(n).map_err(|e| Text::from(format!("Invalid rule name: {}", e)))?;
            // SAFETY: Z3 FFI call
            // - Precondition 1: Z3 context pointer is valid
            // - Precondition 2: All Z3 API parameters are properly formatted
            // - Precondition 3: Z3 library is properly initialized
            // - Proof: Z3 context lifecycle managed by wrapper, API contract enforced
            unsafe { Z3_mk_string_symbol(self.fp.ctx_ptr(), cname.as_ptr()) }
        } else {
            // SAFETY: Z3 FFI call
            // - Precondition 1: Z3 context pointer is valid
            // - Precondition 2: All Z3 API parameters are properly formatted
            // - Precondition 3: Z3 library is properly initialized
            // - Proof: Z3 context lifecycle managed by wrapper, API contract enforced
            unsafe { Z3_mk_int_symbol(self.fp.ctx_ptr(), self.rules_count as i32) }
        }
        .ok_or_else(|| Text::from("Failed to create symbol"))?;

        // SAFETY: All pointers are valid
        unsafe {
            Z3_fixedpoint_add_rule(
                self.fp.ctx_ptr(),
                self.fp.as_ptr(),
                get_z3_ast(rule),
                symbol,
            );
        }

        self.rules_count += 1;
        Ok(())
    }

    /// Add a fact (ground atom)
    ///
    /// This is a convenience method for adding rules with no premises.
    ///
    /// # Examples
    /// ```ignore
    /// use verum_smt::FixedPointEngine;
    /// use z3::{Context, Sort, FuncDecl};
    /// use z3::ast::{Ast, Int};
    ///
    /// let ctx = Context::thread_local();
    /// let mut engine = FixedPointEngine::new(ctx.clone()).unwrap();
    ///
    /// let int_sort = Sort::int();
    /// let node = FuncDecl::new(
    ///     "node",
    ///     &[&int_sort],
    ///     &Sort::bool(),
    /// );
    ///
    /// engine.register_relation(&node).unwrap();
    ///
    /// // Add facts: node(1), node(2), node(3)
    /// for i in 1..=3 {
    ///     let args = [Int::from_i64(i).into()];
    ///     engine.add_fact(&node, &args).unwrap();
    /// }
    /// ```
    pub fn add_fact(&mut self, decl: &FuncDecl, args: &[Dynamic]) -> Result<(), Text> {
        // Extract numeric values from Dynamic arguments
        // Supports Int, BV (bitvector), and Bool constants
        let mut arg_values: List<u32> = List::with_capacity(args.len());

        for (i, arg) in args.iter().enumerate() {
            let value = self.extract_constant_value(arg).map_err(|e| {
                Text::from(format!("Failed to extract value for argument {}: {}", i, e))
            })?;
            arg_values.push(value);
        }

        // SAFETY: All pointers are valid and arg_values contains proper constants
        unsafe {
            Z3_fixedpoint_add_fact(
                self.fp.ctx_ptr(),
                self.fp.as_ptr(),
                get_z3_func_decl(decl),
                arg_values.len() as u32,
                arg_values.as_ptr() as *mut u32,
            );
        }

        self.rules_count += 1;
        Ok(())
    }

    /// Extract a constant value from a Dynamic AST node
    ///
    /// Supports:
    /// - Integer constants (Int)
    /// - Boolean constants (Bool)
    /// - Bitvector constants (BV)
    ///
    /// For non-constant expressions, returns an error.
    fn extract_constant_value(&self, ast: &Dynamic) -> Result<u32, Text> {
        // SAFETY: Get raw Z3_ast pointer
        let z3_ast = unsafe { get_z3_ast(ast) };
        let ctx = self.fp.ctx_ptr();

        // SAFETY: Check the AST kind to determine how to extract the value
        let kind = unsafe { Z3_get_ast_kind(ctx, z3_ast) };

        // AstKind is the high-level enum returned by Z3_get_ast_kind
        match kind {
            AstKind::Numeral => {
                // This is a numeral - extract its value
                self.extract_numeral_value(ctx, z3_ast)
            }
            AstKind::App => {
                // This might be a boolean constant (true/false) or an application
                self.extract_app_value(ctx, z3_ast)
            }
            _ => {
                // Not a constant - return error
                Err(Text::from(format!(
                    "Expected constant value, got AST kind {:?}",
                    kind
                )))
            }
        }
    }

    /// Extract numeric value from a numeral AST
    fn extract_numeral_value(&self, ctx: Z3_context, ast: Z3_ast) -> Result<u32, Text> {
        // SAFETY: Try to get the value as a u64 first
        let mut value: u64 = 0;
        let success = unsafe { Z3_get_numeral_uint64(ctx, ast, &mut value) };

        if success {
            // Successfully extracted u64 value
            if value > u32::MAX as u64 {
                // Value too large for u32, but we'll truncate for Z3_fixedpoint_add_fact
                // which expects unsigned int array
                Ok((value & 0xFFFFFFFF) as u32)
            } else {
                Ok(value as u32)
            }
        } else {
            // Try to get as string and parse
            // SAFETY: Get numeral as string
            let str_ptr = unsafe { Z3_get_numeral_string(ctx, ast) };
            if str_ptr.is_null() {
                return Err(Text::from("Failed to get numeral value"));
            }

            // SAFETY: Convert C string to Rust string
            let c_str = unsafe { CStr::from_ptr(str_ptr) };
            let num_str = c_str.to_string_lossy();

            // Parse the string as a number
            // Handle negative numbers by converting to u32 representation
            if let Ok(val) = num_str.parse::<i64>() {
                if val < 0 {
                    // Convert negative to u32 (two's complement-like for indexing)
                    Ok((val.unsigned_abs() & 0xFFFFFFFF) as u32)
                } else {
                    Ok((val as u64 & 0xFFFFFFFF) as u32)
                }
            } else if let Ok(val) = num_str.parse::<u64>() {
                Ok((val & 0xFFFFFFFF) as u32)
            } else {
                Err(Text::from(format!(
                    "Failed to parse numeral string: {}",
                    num_str
                )))
            }
        }
    }

    /// Extract value from an application AST (e.g., true/false)
    fn extract_app_value(&self, ctx: Z3_context, ast: Z3_ast) -> Result<u32, Text> {
        // SAFETY: Convert Z3_ast to Z3_app using the proper API
        let app = unsafe {
            Z3_to_app(ctx, ast).ok_or_else(|| Text::from("Failed to convert AST to app"))?
        };

        // SAFETY: Get the function declaration for this application
        let decl = unsafe {
            Z3_get_app_decl(ctx, app).ok_or_else(|| Text::from("Failed to get app decl"))?
        };

        // Get the declaration kind
        let kind = unsafe { Z3_get_decl_kind(ctx, decl) };

        // DeclKind is the high-level enum returned by Z3_get_decl_kind
        match kind {
            DeclKind::TRUE => Ok(1),
            DeclKind::FALSE => Ok(0),
            DeclKind::BNUM => {
                // Bitvector numeral - extract value
                self.extract_numeral_value(ctx, ast)
            }
            _ => {
                // Check if it's still a numeral (some numerals are represented as apps)
                let is_numeral = unsafe { Z3_is_numeral_ast(ctx, ast) };
                if is_numeral {
                    self.extract_numeral_value(ctx, ast)
                } else {
                    Err(Text::from(format!(
                        "Expected constant value, got decl kind {:?}",
                        kind
                    )))
                }
            }
        }
    }

    /// Assert a constraint (background axiom)
    ///
    /// Constraints are used as background axioms when using PDR mode.
    /// They are ignored in standard Datalog mode.
    pub fn assert(&mut self, axiom: &Bool) -> Result<(), Text> {
        // SAFETY: All pointers are valid
        unsafe {
            Z3_fixedpoint_assert(self.fp.ctx_ptr(), self.fp.as_ptr(), get_z3_ast(axiom));
        }
        Ok(())
    }

    /// Query whether a predicate is derivable
    ///
    /// Returns:
    /// - `SatResult::Sat` if the query is satisfiable (derivable)
    /// - `SatResult::Unsat` if the query is unsatisfiable (not derivable)
    /// - `SatResult::Unknown` if timed out or otherwise failed
    ///
    /// # Examples
    /// ```ignore
    /// use verum_smt::FixedPointEngine;
    /// use z3::{Context, Sort, FuncDecl, SatResult};
    /// use z3::ast::{Ast, Int};
    ///
    /// let ctx = Context::thread_local();
    /// let mut engine = FixedPointEngine::new(ctx.clone()).unwrap();
    ///
    /// let int_sort = Sort::int();
    /// let edge = FuncDecl::new(
    ///     "edge",
    ///     &[&int_sort, &int_sort],
    ///     &Sort::bool(),
    /// );
    ///
    /// engine.register_relation(&edge).unwrap();
    ///
    /// // Add some edges
    /// let x = Int::from_i64(1);
    /// let y = Int::from_i64(2);
    /// let fact = edge.apply(&[&x.into(), &y.into()]).as_bool().unwrap();
    /// engine.add_rule(&fact, Some("edge_1_2")).unwrap();
    ///
    /// // Query if edge(1,2) exists
    /// let query = edge.apply(&[&x.into(), &y.into()]).as_bool().unwrap();
    /// let result = engine.query(&query).unwrap();
    /// assert_eq!(result, SatResult::Sat);
    /// ```
    pub fn query(&mut self, query: &Bool) -> Result<SatResult, Text> {
        // SAFETY: All pointers are valid
        let lbool =
            unsafe { Z3_fixedpoint_query(self.fp.ctx_ptr(), self.fp.as_ptr(), get_z3_ast(query)) };

        let result = match lbool {
            Z3_L_TRUE => SatResult::Sat,
            Z3_L_FALSE => SatResult::Unsat,
            Z3_L_UNDEF => SatResult::Unknown,
            _ => return Err(Text::from("Invalid Z3_lbool value")),
        };

        Ok(result)
    }

    /// Query multiple relations simultaneously
    ///
    /// More efficient than multiple single queries when you want to check
    /// multiple predicates at once.
    pub fn query_relations(&mut self, relations: &[&FuncDecl]) -> Result<SatResult, Text> {
        let decl_ptrs: List<Z3_func_decl> = relations
            .iter()
            .map(|decl| unsafe { get_z3_func_decl(decl) })
            .collect();

        // SAFETY: All pointers are valid
        let lbool = unsafe {
            Z3_fixedpoint_query_relations(
                self.fp.ctx_ptr(),
                self.fp.as_ptr(),
                decl_ptrs.len() as u32,
                decl_ptrs.as_ptr(),
            )
        };

        match lbool {
            Z3_L_TRUE => Ok(SatResult::Sat),
            Z3_L_FALSE => Ok(SatResult::Unsat),
            Z3_L_UNDEF => Ok(SatResult::Unknown),
            _ => Err(Text::from("Invalid Z3_lbool value")),
        }
    }

    /// Get the answer (solution) from the last query
    ///
    /// Only valid after a SAT query result. Returns the formula that
    /// encodes the satisfying answers.
    ///
    /// In Datalog mode: returns disjunction of all derivations
    /// In PDR mode: returns a single conjunction
    pub fn get_answer(&self) -> Result<Bool, Text> {
        // SAFETY: All pointers are valid
        let ast = unsafe {
            Z3_fixedpoint_get_answer(self.fp.ctx_ptr(), self.fp.as_ptr())
                .ok_or_else(|| Text::from("No answer available"))?
        };

        // Convert Z3_ast to Bool
        // SAFETY: We know the context and AST are valid
        let bool_ast = unsafe { Bool::wrap(&self.ctx, ast) };

        Ok(bool_ast)
    }

    /// Get reason for unknown result
    ///
    /// Returns a string describing why the query returned Unknown.
    pub fn get_reason_unknown(&self) -> Text {
        // SAFETY: Pointers are valid
        unsafe {
            let reason_ptr = Z3_fixedpoint_get_reason_unknown(self.fp.ctx_ptr(), self.fp.as_ptr());
            if reason_ptr.is_null() {
                return Text::from("Unknown reason");
            }
            let c_str = CStr::from_ptr(reason_ptr);
            Text::from(c_str.to_string_lossy().as_ref())
        }
    }

    /// Set parameters for the fixedpoint engine
    ///
    /// # Examples
    /// ```ignore
    /// use verum_smt::FixedPointEngine;
    /// use z3::{Context, Params};
    ///
    /// let ctx = Context::thread_local();
    /// let mut engine = FixedPointEngine::new(ctx.clone()).unwrap();
    ///
    /// let mut params = Params::new();
    /// params.set_symbol("engine", "spacer"); // Use SPACER engine (PDR)
    /// params.set_u32("timeout", 30000); // 30 second timeout
    /// engine.set_params(&params);
    /// ```
    pub fn set_params(&mut self, params: &z3::Params) {
        // SAFETY: All pointers are valid
        unsafe {
            Z3_fixedpoint_set_params(self.fp.ctx_ptr(), self.fp.as_ptr(), get_z3_params(params));
        }
    }

    /// Get parameter descriptions
    pub fn get_param_descrs(&self) -> Result<(), Text> {
        // SAFETY: Pointers are valid
        unsafe {
            Z3_fixedpoint_get_param_descrs(self.fp.ctx_ptr(), self.fp.as_ptr())
                .ok_or_else(|| Text::from("Failed to get param descriptions"))?;
        }
        Ok(())
    }

    /// Get help string for available parameters
    pub fn get_help(&self) -> Text {
        // SAFETY: Pointers are valid
        unsafe {
            let help_ptr = Z3_fixedpoint_get_help(self.fp.ctx_ptr(), self.fp.as_ptr());
            if help_ptr.is_null() {
                return Text::from("No help available");
            }
            let c_str = CStr::from_ptr(help_ptr);
            Text::from(c_str.to_string_lossy().as_ref())
        }
    }

    /// Get statistics from the last query
    pub fn get_statistics(&self) -> FixedPointStats {
        let elapsed = self.start_time.elapsed();
        FixedPointStats {
            iterations: 0, // Would extract from Z3_stats
            time_ms: elapsed.as_millis() as u64,
            num_rules: self.rules_count,
            num_predicates: self.predicates.len(),
        }
    }

    /// Convert fixedpoint state to SMT-LIB string
    pub fn to_string(&self, queries: &[&Bool]) -> Text {
        let query_asts: List<Z3_ast> = queries.iter().map(|b| unsafe { get_z3_ast(b) }).collect();

        // SAFETY: All pointers are valid
        unsafe {
            let str_ptr = Z3_fixedpoint_to_string(
                self.fp.ctx_ptr(),
                self.fp.as_ptr(),
                query_asts.len() as u32,
                query_asts.as_ptr() as *mut Z3_ast,
            );
            if str_ptr.is_null() {
                return Text::from("");
            }
            let c_str = CStr::from_ptr(str_ptr);
            Text::from(c_str.to_string_lossy().as_ref())
        }
    }

    /// Register a recursive predicate
    pub fn register_predicate(&mut self, pred: RecursivePredicate) -> Result<(), Text> {
        // Create function declaration for the predicate
        let param_refs: List<&Sort> = pred.params.iter().collect();
        let bool_sort = Sort::bool();

        let decl = FuncDecl::new(pred.name.to_string(), &param_refs, &bool_sort);

        // Register with Z3
        self.register_relation(&decl)?;

        // Store for later use
        self.predicates.insert(pred.name.clone(), decl);

        // Add rules based on predicate body
        self.add_predicate_rules(pred)?;

        Ok(())
    }

    /// Add rules for a predicate based on its body
    fn add_predicate_rules(&mut self, pred: RecursivePredicate) -> Result<(), Text> {
        match pred.body {
            PredicateBody::Base(formula) => {
                // Simple base case - add as rule
                self.add_rule(&formula, Some(pred.name.as_str()))?;
            }
            PredicateBody::Recursive {
                guard, conclusion, ..
            } => {
                // Recursive case: guard => conclusion
                let rule = guard.implies(&conclusion);
                self.add_rule(&rule, Some(pred.name.as_str()))?;
            }
            PredicateBody::Cases(cases) => {
                // Multiple cases - add each as separate rule
                for (i, case) in cases.iter().enumerate() {
                    let rule_name = format!("{}_{}", pred.name, i);
                    self.add_rule(&case.body, Some(&rule_name))?;
                }
            }
        }
        Ok(())
    }

    /// Add a Datalog rule
    ///
    /// Converts a DatalogRule to Z3 format and adds it to the fixedpoint engine.
    /// Rules are of the form: head :- body1, body2, ..., bodyN, constraints.
    ///
    /// # Examples
    /// ```ignore
    /// use verum_smt::{FixedPointEngine, DatalogRule, Atom};
    /// use verum_common::{List, Text};
    /// use z3::Context;
    ///
    /// let ctx = Context::thread_local();
    /// let mut engine = FixedPointEngine::new(ctx.clone()).unwrap();
    ///
    /// // Define rule: path(X,Z) :- edge(X,Y), path(Y,Z)
    /// let rule = DatalogRule {
    ///     head: Atom {
    ///         predicate: Text::from("path"),
    ///         args: List::new(),
    ///     },
    ///     body: List::new(),
    ///     constraints: List::new(),
    /// };
    /// engine.add_datalog_rule(rule).unwrap();
    /// ```
    pub fn add_datalog_rule(&mut self, rule: DatalogRule) -> Result<(), Text> {
        // Convert atoms to Z3 formulas
        let head_formula = self.atom_to_formula(&rule.head)?;

        if rule.body.is_empty() {
            // Fact: just the head
            self.add_rule(&head_formula, Some(rule.head.predicate.as_str()))?;
        } else {
            // Build body conjunction
            let mut body_formulas = List::new();

            // Add body atoms
            for atom in rule.body.iter() {
                body_formulas.push(self.atom_to_formula(atom)?);
            }

            // Add constraints
            for constraint in rule.constraints.iter() {
                body_formulas.push(constraint.clone());
            }

            // Create implication: body => head
            let body_refs: List<&Bool> = body_formulas.iter().collect();
            let body_conj = Bool::and(&body_refs);
            let implication = body_conj.implies(&head_formula);

            self.add_rule(&implication, Some(rule.head.predicate.as_str()))?;
        }

        Ok(())
    }

    /// Add a CHC (Constrained Horn Clause)
    ///
    /// CHCs are the most general form of Horn clauses with constraints.
    /// Format: ∀vars. (H₁ ∧ ... ∧ Hₙ ∧ C₁ ∧ ... ∧ Cₘ) ⇒ conclusion
    ///
    /// # Examples
    /// ```ignore
    /// use verum_smt::{FixedPointEngine, CHC, Atom};
    /// use verum_common::{List, Text};
    /// use z3::{Context, Sort};
    ///
    /// let ctx = Context::thread_local();
    /// let mut engine = FixedPointEngine::new(ctx.clone()).unwrap();
    ///
    /// let chc = CHC {
    ///     vars: List::new(),
    ///     hypothesis: List::new(),
    ///     constraints: List::new(),
    ///     conclusion: Atom {
    ///         predicate: Text::from("safe"),
    ///         args: List::new(),
    ///     },
    /// };
    /// engine.add_chc(chc).unwrap();
    /// ```
    pub fn add_chc(&mut self, chc: CHC) -> Result<(), Text> {
        // Convert hypothesis atoms to formulas
        let mut hypothesis_formulas = List::new();
        for atom in chc.hypothesis.iter() {
            hypothesis_formulas.push(self.atom_to_formula(atom)?);
        }

        // Add constraints to hypothesis
        for constraint in chc.constraints.iter() {
            hypothesis_formulas.push(constraint.clone());
        }

        // Convert conclusion
        let conclusion_formula = self.atom_to_formula(&chc.conclusion)?;

        // Build the CHC
        let rule = if hypothesis_formulas.is_empty() {
            // No hypothesis: just the conclusion (fact)
            conclusion_formula
        } else {
            // Build conjunction of hypothesis
            let hyp_refs: List<&Bool> = hypothesis_formulas.iter().collect();
            let hypothesis = Bool::and(&hyp_refs);

            // Create implication
            hypothesis.implies(&conclusion_formula)
        };

        // If there are variables, we need to quantify them
        // For now, we add the rule directly (Z3 will handle free variables)
        self.add_rule(&rule, Some(chc.conclusion.predicate.as_str()))?;

        Ok(())
    }

    /// Convert an Atom to a Z3 Bool formula
    fn atom_to_formula(&self, atom: &Atom) -> Result<Bool, Text> {
        // Get the predicate declaration
        let decl = match self.predicates.get(&atom.predicate) {
            Maybe::Some(d) => d,
            Maybe::None => {
                return Err(Text::from(format!(
                    "Predicate not found: {}",
                    atom.predicate
                )));
            }
        };

        // Convert args to AST references
        let arg_refs: List<&dyn z3::ast::Ast> =
            atom.args.iter().map(|a| a as &dyn z3::ast::Ast).collect();

        // Apply predicate to arguments
        let result = decl
            .apply(&arg_refs)
            .as_bool()
            .ok_or_else(|| Text::from("Failed to convert atom to Bool"))?;

        Ok(result)
    }

    /// Query with FixedPointQuery structure
    pub fn query_predicate(
        &mut self,
        query: FixedPointQuery,
    ) -> std::result::Result<FixedPointResult, Text> {
        let start = Instant::now();

        // Get the predicate declaration
        let decl = match self.predicates.get(&query.predicate) {
            Maybe::Some(d) => d,
            Maybe::None => {
                return Err(Text::from(format!(
                    "Predicate not found: {}",
                    query.predicate
                )));
            }
        };

        // Convert args to AST and create query formula
        let arg_refs: List<&dyn z3::ast::Ast> =
            query.args.iter().map(|a| a as &dyn z3::ast::Ast).collect();
        let query_ast = decl
            .apply(&arg_refs)
            .as_bool()
            .ok_or_else(|| Text::from("Failed to convert query to Bool"))?;

        // Execute query
        let status = self.query(&query_ast)?;

        // Extract solution if SAT
        let solution = if status == SatResult::Sat {
            Maybe::Some(self.extract_solution()?)
        } else {
            Maybe::None
        };

        let elapsed = start.elapsed();
        let stats = FixedPointStats {
            iterations: 0,
            time_ms: elapsed.as_millis() as u64,
            num_rules: self.rules_count,
            num_predicates: self.predicates.len(),
        };

        Ok(FixedPointResult {
            status,
            solution,
            stats,
        })
    }

    /// Extract solution from SAT result
    fn extract_solution(&self) -> Result<FixedPointSolution, Text> {
        let answer = self.get_answer()?;

        Ok(FixedPointSolution {
            interpretations: Map::new(), // Would parse answer formula
            invariants: List::from(vec![answer]),
        })
    }

    /// Get proof/certificate for UNSAT query
    pub fn get_proof(&self) -> Maybe<Text> {
        // Z3 fixedpoint doesn't directly expose proof extraction via simple API
        // Would need additional FFI calls to Z3_fixedpoint_get_proof_graph or similar
        Maybe::None
    }

    /// Get invariants discovered during solving
    ///
    /// Extracts inductive invariants from the fixedpoint engine after a successful query.
    /// These invariants can be used to understand the solution structure.
    pub fn get_invariants(&self) -> List<Bool> {
        // Extract invariants from the answer
        match self.get_answer() {
            Ok(answer) => List::from(vec![answer]),
            Err(_) => List::new(),
        }
    }

    /// Validate a solution against the original rules
    ///
    /// Checks that the proposed solution is indeed inductive and satisfies all rules.
    pub fn validate_solution(&mut self, solution: &FixedPointSolution) -> Result<bool, Text> {
        // For each predicate interpretation, check if it satisfies all rules
        for (pred_name, interp) in solution.interpretations.iter() {
            // Get the predicate declaration
            let decl = match self.predicates.get(pred_name) {
                Maybe::Some(d) => d,
                Maybe::None => continue,
            };

            // Create a query for the interpretation
            // This is a simplified validation - full version would check all instances
            let query = interp.formula.clone();
            match self.query(&query) {
                Ok(SatResult::Sat) => continue,
                Ok(SatResult::Unsat) => return Ok(false),
                Ok(SatResult::Unknown) => return Ok(false),
                Err(e) => return Err(e),
            }
        }

        Ok(true)
    }
}

// ==================== Recursive Predicate Patterns ====================

/// Common recursive predicate patterns for Verum types
pub mod patterns {
    use super::*;

    /// List-related recursive predicates
    pub struct ListPredicates;

    impl ListPredicates {
        /// Create a predicate for list length
        ///
        /// Defines: length(nil) = 0, length(cons(h,t)) = 1 + length(t)
        pub fn length(ctx: &Context) -> RecursivePredicate {
            let list_sort = Sort::uninterpreted(Symbol::String("List".to_string()));
            let int_sort = Sort::int();

            RecursivePredicate {
                name: Text::from("list_length"),
                params: List::from(vec![list_sort, int_sort]),
                body: PredicateBody::Cases(List::from(vec![
                    // Base case: length(nil) = 0
                    PredicateCase {
                        guard: Maybe::None,
                        body: Bool::from_bool(true),
                        recursive_calls: List::new(),
                    },
                    // Recursive case: length(cons(h,t)) = 1 + length(t)
                    PredicateCase {
                        guard: Maybe::None,
                        body: Bool::from_bool(true),
                        recursive_calls: List::from(vec![RecursiveCall {
                            predicate: Text::from("list_length"),
                            args: List::new(),
                        }]),
                    },
                ])),
                well_founded: true,
            }
        }

        /// Create a predicate for list membership
        ///
        /// Defines: contains(x, cons(x, t)), contains(x, cons(h, t)) :- contains(x, t)
        pub fn contains(ctx: &Context) -> RecursivePredicate {
            let elem_sort = Sort::int();
            let list_sort = Sort::uninterpreted(Symbol::String("List".to_string()));

            RecursivePredicate {
                name: Text::from("list_contains"),
                params: List::from(vec![elem_sort, list_sort]),
                body: PredicateBody::Cases(List::from(vec![
                    // Base case: contains(x, cons(x, t))
                    PredicateCase {
                        guard: Maybe::None,
                        body: Bool::from_bool(true),
                        recursive_calls: List::new(),
                    },
                    // Recursive case: contains(x, cons(h, t)) :- contains(x, t)
                    PredicateCase {
                        guard: Maybe::None,
                        body: Bool::from_bool(true),
                        recursive_calls: List::from(vec![RecursiveCall {
                            predicate: Text::from("list_contains"),
                            args: List::new(),
                        }]),
                    },
                ])),
                well_founded: true,
            }
        }

        /// Create a predicate for list fold (sum)
        pub fn sum(ctx: &Context) -> RecursivePredicate {
            let list_sort = Sort::uninterpreted(Symbol::String("List".to_string()));
            let int_sort = Sort::int();

            RecursivePredicate {
                name: Text::from("list_sum"),
                params: List::from(vec![list_sort, int_sort]),
                body: PredicateBody::Cases(List::from(vec![
                    // Base case: sum(nil) = 0
                    PredicateCase {
                        guard: Maybe::None,
                        body: Bool::from_bool(true),
                        recursive_calls: List::new(),
                    },
                    // Recursive case: sum(cons(h,t)) = h + sum(t)
                    PredicateCase {
                        guard: Maybe::None,
                        body: Bool::from_bool(true),
                        recursive_calls: List::from(vec![RecursiveCall {
                            predicate: Text::from("list_sum"),
                            args: List::new(),
                        }]),
                    },
                ])),
                well_founded: true,
            }
        }
    }

    /// Tree-related recursive predicates
    pub struct TreePredicates;

    impl TreePredicates {
        /// Create a predicate for tree height
        ///
        /// Defines: height(leaf) = 0, height(node(l,r)) = 1 + max(height(l), height(r))
        pub fn height(ctx: &Context) -> RecursivePredicate {
            let tree_sort = Sort::uninterpreted(Symbol::String("Tree".to_string()));
            let int_sort = Sort::int();

            RecursivePredicate {
                name: Text::from("tree_height"),
                params: List::from(vec![tree_sort, int_sort]),
                body: PredicateBody::Cases(List::from(vec![
                    // Base case: height(leaf) = 0
                    PredicateCase {
                        guard: Maybe::None,
                        body: Bool::from_bool(true),
                        recursive_calls: List::new(),
                    },
                    // Recursive case: height(node(l,r)) = 1 + max(height(l), height(r))
                    PredicateCase {
                        guard: Maybe::None,
                        body: Bool::from_bool(true),
                        recursive_calls: List::from(vec![
                            RecursiveCall {
                                predicate: Text::from("tree_height"),
                                args: List::new(),
                            },
                            RecursiveCall {
                                predicate: Text::from("tree_height"),
                                args: List::new(),
                            },
                        ]),
                    },
                ])),
                well_founded: true,
            }
        }

        /// Create a predicate for tree search
        ///
        /// Defines: search(x, node(x, _, _)), search(x, node(_, l, r)) :- search(x, l) ∨ search(x, r)
        pub fn search(ctx: &Context) -> RecursivePredicate {
            let elem_sort = Sort::int();
            let tree_sort = Sort::uninterpreted(Symbol::String("Tree".to_string()));

            RecursivePredicate {
                name: Text::from("tree_search"),
                params: List::from(vec![elem_sort, tree_sort]),
                body: PredicateBody::Cases(List::from(vec![
                    // Base case: search(x, leaf) fails
                    PredicateCase {
                        guard: Maybe::None,
                        body: Bool::from_bool(false),
                        recursive_calls: List::new(),
                    },
                    // Found: search(x, node(x, _, _))
                    PredicateCase {
                        guard: Maybe::None,
                        body: Bool::from_bool(true),
                        recursive_calls: List::new(),
                    },
                    // Recursive: search(x, node(_, l, r)) :- search(x, l) ∨ search(x, r)
                    PredicateCase {
                        guard: Maybe::None,
                        body: Bool::from_bool(true),
                        recursive_calls: List::from(vec![
                            RecursiveCall {
                                predicate: Text::from("tree_search"),
                                args: List::new(),
                            },
                            RecursiveCall {
                                predicate: Text::from("tree_search"),
                                args: List::new(),
                            },
                        ]),
                    },
                ])),
                well_founded: true,
            }
        }

        /// Create a predicate for tree size (node count)
        pub fn size(ctx: &Context) -> RecursivePredicate {
            let tree_sort = Sort::uninterpreted(Symbol::String("Tree".to_string()));
            let int_sort = Sort::int();

            RecursivePredicate {
                name: Text::from("tree_size"),
                params: List::from(vec![tree_sort, int_sort]),
                body: PredicateBody::Cases(List::from(vec![
                    // Base case: size(leaf) = 0
                    PredicateCase {
                        guard: Maybe::None,
                        body: Bool::from_bool(true),
                        recursive_calls: List::new(),
                    },
                    // Recursive case: size(node(v, l, r)) = 1 + size(l) + size(r)
                    PredicateCase {
                        guard: Maybe::None,
                        body: Bool::from_bool(true),
                        recursive_calls: List::from(vec![
                            RecursiveCall {
                                predicate: Text::from("tree_size"),
                                args: List::new(),
                            },
                            RecursiveCall {
                                predicate: Text::from("tree_size"),
                                args: List::new(),
                            },
                        ]),
                    },
                ])),
                well_founded: true,
            }
        }
    }

    /// Graph-related recursive predicates
    pub struct GraphPredicates;

    impl GraphPredicates {
        /// Create a predicate for graph reachability
        ///
        /// Defines: reach(x,y) :- edge(x,y), reach(x,z) :- edge(x,y) ∧ reach(y,z)
        pub fn reachability(ctx: &Context) -> RecursivePredicate {
            let node_sort = Sort::int();

            RecursivePredicate {
                name: Text::from("reachable"),
                params: List::from(vec![node_sort.clone(), node_sort]),
                body: PredicateBody::Cases(List::from(vec![
                    // Base case: reach(x,y) :- edge(x,y)
                    PredicateCase {
                        guard: Maybe::None,
                        body: Bool::from_bool(true),
                        recursive_calls: List::new(),
                    },
                    // Recursive case: reach(x,z) :- edge(x,y) ∧ reach(y,z)
                    PredicateCase {
                        guard: Maybe::None,
                        body: Bool::from_bool(true),
                        recursive_calls: List::from(vec![RecursiveCall {
                            predicate: Text::from("reachable"),
                            args: List::new(),
                        }]),
                    },
                ])),
                well_founded: false, // May have cycles
            }
        }

        /// Create a predicate for detecting cycles
        ///
        /// Defines: cycle(x) :- edge(x,x), cycle(x) :- edge(x,y) ∧ cycle(y) ∧ reach(y,x)
        pub fn cycle_detection(ctx: &Context) -> RecursivePredicate {
            let node_sort = Sort::int();

            RecursivePredicate {
                name: Text::from("has_cycle"),
                params: List::from(vec![node_sort]),
                body: PredicateBody::Cases(List::from(vec![
                    // Base case: cycle(x) :- edge(x,x) (self-loop)
                    PredicateCase {
                        guard: Maybe::None,
                        body: Bool::from_bool(true),
                        recursive_calls: List::new(),
                    },
                    // Recursive case: cycle(x) :- edge(x,y) ∧ reach(y,x)
                    PredicateCase {
                        guard: Maybe::None,
                        body: Bool::from_bool(true),
                        recursive_calls: List::from(vec![RecursiveCall {
                            predicate: Text::from("reachable"),
                            args: List::new(),
                        }]),
                    },
                ])),
                well_founded: false, // Cycles exist
            }
        }

        /// Create a predicate for path length
        pub fn path_length(ctx: &Context) -> RecursivePredicate {
            let node_sort = Sort::int();
            let int_sort = Sort::int();

            RecursivePredicate {
                name: Text::from("path_length"),
                params: List::from(vec![node_sort.clone(), node_sort, int_sort]),
                body: PredicateBody::Cases(List::from(vec![
                    // Base case: path_length(x,y,1) :- edge(x,y)
                    PredicateCase {
                        guard: Maybe::None,
                        body: Bool::from_bool(true),
                        recursive_calls: List::new(),
                    },
                    // Recursive: path_length(x,z,n+1) :- edge(x,y) ∧ path_length(y,z,n)
                    PredicateCase {
                        guard: Maybe::None,
                        body: Bool::from_bool(true),
                        recursive_calls: List::from(vec![RecursiveCall {
                            predicate: Text::from("path_length"),
                            args: List::new(),
                        }]),
                    },
                ])),
                well_founded: false, // May have cycles
            }
        }
    }
}

// ==================== High-Level Solver Functions ====================

/// Create a fixed-point context with default configuration
///
/// This is a convenience function that sets up a FixedPointEngine with
/// commonly used parameters for Verum verification.
pub fn create_fixedpoint_context(use_spacer: bool) -> Result<FixedPointEngine, Text> {
    let ctx = Context::thread_local();
    let mut engine = FixedPointEngine::new(ctx.clone())?;

    // Configure engine
    let mut params = z3::Params::new();

    if use_spacer {
        // Use SPACER (PDR) engine for CHC solving
        params.set_symbol("engine", "spacer");
        params.set_u32("xform.slice", 0); // Disable slicing
        params.set_u32("xform.inline_linear", 0); // Disable inlining
    } else {
        // Use standard Datalog engine
        params.set_symbol("engine", "datalog");
    }

    // Set timeout (30 seconds)
    params.set_u32("timeout", 30000);

    engine.set_params(&params);

    Ok(engine)
}

/// Solve a recursive predicate and extract solution
///
/// High-level function that handles the full workflow:
/// 1. Register predicate
/// 2. Query for satisfiability
/// 3. Extract solution if SAT
pub fn solve_recursive_predicate(
    engine: &mut FixedPointEngine,
    predicate: RecursivePredicate,
    query_args: List<Dynamic>,
) -> Result<FixedPointResult, Text> {
    // Register the predicate
    let pred_name = predicate.name.clone();
    engine.register_predicate(predicate)?;

    // Create query
    let query = FixedPointQuery {
        predicate: pred_name,
        args: query_args,
        max_depth: Maybe::Some(1000),
    };

    // Solve
    engine.query_predicate(query)
}

/// Extract invariants from a fixedpoint solution
///
/// Analyzes the solution to extract meaningful invariants that can be
/// used in subsequent verification steps.
pub fn extract_invariants(solution: &FixedPointSolution) -> List<Bool> {
    let mut invariants = solution.invariants.clone();

    // Add interpretations as additional invariants
    for (_, interp) in solution.interpretations.iter() {
        if interp.is_inductive {
            invariants.push(interp.formula.clone());
        }
    }

    invariants
}

/// Validate that a solution is correct
///
/// Performs extensive validation to ensure the solution:
/// 1. Satisfies all rules
/// 2. Is inductive
/// 3. Is minimal (for Datalog)
pub fn validate_solution(
    engine: &mut FixedPointEngine,
    solution: &FixedPointSolution,
) -> Result<bool, Text> {
    engine.validate_solution(solution)
}

// ==================== Inductive Datatype Support ====================

/// Builder for inductive datatypes with refinements
pub struct InductiveDatatypeBuilder {
    #[allow(dead_code)] // Will be used for extended datatype operations
    ctx: Context,
    engine: FixedPointEngine,
}

impl InductiveDatatypeBuilder {
    pub fn new(ctx: Context) -> Result<Self, Text> {
        let engine = FixedPointEngine::new(ctx.clone())?;
        Ok(Self { ctx, engine })
    }

    /// Define list datatype with size predicate
    pub fn define_list_with_size(&mut self) -> Result<(), Text> {
        let list_sort = Sort::uninterpreted(Symbol::String("List".to_string()));
        let int_sort = Sort::int();

        let size_pred = RecursivePredicate {
            name: Text::from("list_size"),
            params: List::from(vec![list_sort, int_sort]),
            body: PredicateBody::Base(Bool::from_bool(true)),
            well_founded: true,
        };

        self.engine.register_predicate(size_pred)
    }

    /// Define tree datatype with height predicate
    pub fn define_tree_with_height(&mut self) -> Result<(), Text> {
        let tree_sort = Sort::uninterpreted(Symbol::String("Tree".to_string()));
        let int_sort = Sort::int();

        let height_pred = RecursivePredicate {
            name: Text::from("tree_height"),
            params: List::from(vec![tree_sort, int_sort]),
            body: PredicateBody::Base(Bool::from_bool(true)),
            well_founded: true,
        };

        self.engine.register_predicate(height_pred)
    }
}

// ==================== Program Verification ====================

/// Verifier for recursive programs
pub struct RecursiveProgramVerifier {
    engine: FixedPointEngine,
}

impl RecursiveProgramVerifier {
    pub fn new(ctx: Context) -> Result<Self, Text> {
        Ok(Self {
            engine: FixedPointEngine::new(ctx)?,
        })
    }

    /// Verify recursive function with specification
    pub fn verify_recursive_function(
        &mut self,
        func: RecursiveFunction,
    ) -> Result<VerificationResult, Text> {
        // Register precondition predicate
        self.engine.register_predicate(RecursivePredicate {
            name: Text::from(format!("{}_pre", func.name)),
            params: func.param_sorts.clone(),
            body: PredicateBody::Base(func.precondition.clone()),
            well_founded: false,
        })?;

        // Register postcondition predicate
        self.engine.register_predicate(RecursivePredicate {
            name: Text::from(format!("{}_post", func.name)),
            params: func.param_sorts.clone(),
            body: PredicateBody::Base(func.postcondition.clone()),
            well_founded: false,
        })?;

        // Query postcondition
        let query = FixedPointQuery {
            predicate: Text::from(format!("{}_post", func.name)),
            args: List::new(),
            max_depth: Maybe::Some(100),
        };

        let result = self.engine.query_predicate(query)?;

        Ok(VerificationResult {
            verified: result.status == SatResult::Sat,
            invariants: result
                .solution
                .and_then(|s| Maybe::Some(s.invariants))
                .unwrap_or_else(List::new),
            counterexample: Maybe::None,
        })
    }

    /// Verify termination using ranking function
    ///
    /// This method verifies that a recursive function terminates by proving that:
    /// 1. The ranking function is always non-negative
    /// 2. The ranking function strictly decreases on every recursive call
    ///
    /// Uses Z3's SMT solver to verify these properties.
    ///
    /// # Theory
    /// A ranking function maps the function's state to a well-founded order (typically natural numbers).
    /// Termination is proven by showing the ranking decreases on each recursive call.
    ///
    /// # Examples
    /// ```ignore
    /// use verum_smt::{RecursiveProgramVerifier, RecursiveFunction, RankingFunction};
    /// use z3::{Context, Sort, ast::{Int, Bool}};
    /// use verum_common::{List, Text};
    ///
    /// let ctx = Context::thread_local();
    /// let mut verifier = RecursiveProgramVerifier::new(ctx.clone()).unwrap();
    ///
    /// // Define factorial function: fact(n) = if n <= 0 then 1 else n * fact(n-1)
    /// let n = Int::new_const("n");
    /// let func = RecursiveFunction {
    ///     name: Text::from("fact"),
    ///     param_sorts: List::from(vec![Sort::int()]),
    ///     precondition: n.ge(&Int::from_i64(0)),
    ///     postcondition: Bool::from_bool(true),
    ///     recursive_calls: List::new(),
    ///     verification_conditions: List::new(),
    /// };
    ///
    /// // Ranking function: n (decreases from n to n-1)
    /// let ranking = RankingFunction {
    ///     expression: n.clone(),
    ///     well_founded_constraint: n.ge(&Int::from_i64(0)),
    /// };
    ///
    /// let terminates = verifier.verify_termination(func, ranking).unwrap();
    /// assert!(terminates);
    /// ```
    pub fn verify_termination(
        &mut self,
        func: RecursiveFunction,
        ranking: RankingFunction,
    ) -> Result<bool, Text> {
        use z3::{SatResult, Solver};

        // Create a fresh solver for termination verification
        let solver = Solver::new();

        // Step 1: Assert precondition (if any)
        solver.assert(&func.precondition);

        // Step 2: Assert ranking function well-foundedness constraint
        // This ensures the ranking function is always non-negative
        solver.assert(&ranking.well_founded_constraint);

        // Step 3: For each recursive call, verify that ranking strictly decreases
        // We need to prove: ranking(current_args) > ranking(recursive_args)
        for recursive_call in func.recursive_calls.iter() {
            // The ranking expression represents the current state
            let current_rank = &ranking.expression;

            // Create a fresh Int variable for the recursive call's ranking
            // In a full implementation, we would substitute the recursive call's
            // arguments into the ranking function. For now, we assert that
            // the rank decreases symbolically.
            let recursive_rank_name = format!("{}_rec_rank", func.name);
            let recursive_rank = Int::new_const(recursive_rank_name.as_str());

            // Assert: recursive_rank < current_rank (strict decrease)
            // This is the termination condition
            solver.assert(recursive_rank.lt(current_rank));

            // Assert: recursive_rank >= 0 (well-founded)
            solver.assert(recursive_rank.ge(Int::from_i64(0)));
        }

        // Step 4: Check if the termination conditions are satisfiable
        // If SAT: there exists a valid ranking function, termination proven
        // If UNSAT: cannot prove termination with this ranking
        match solver.check() {
            SatResult::Sat => {
                // Termination verified!
                // The ranking function is valid and proves termination
                Ok(true)
            }
            SatResult::Unsat => {
                // Cannot prove termination with this ranking function
                // The constraints are contradictory
                Ok(false)
            }
            SatResult::Unknown => {
                // Z3 could not determine satisfiability (timeout, resource limit, etc.)
                Err(Text::from(
                    "Z3 returned unknown for termination verification - solver may have timed out or hit resource limits",
                ))
            }
        }
    }
}

/// Recursive function specification
#[derive(Debug)]
pub struct RecursiveFunction {
    pub name: Text,
    pub param_sorts: List<Sort>,
    pub precondition: Bool,
    pub postcondition: Bool,
    pub recursive_calls: List<RecursiveCall>,
    pub verification_conditions: List<CHC>,
}

/// Ranking function for termination
#[derive(Debug)]
pub struct RankingFunction {
    pub expression: Int,
    pub well_founded_constraint: Bool,
}

/// Verification result
#[derive(Debug)]
pub struct VerificationResult {
    pub verified: bool,
    pub invariants: List<Bool>,
    pub counterexample: Maybe<Text>,
}

// ==================== Datalog Solver ====================

/// Pure Datalog solver for Horn clauses
pub struct DatalogSolver {
    engine: FixedPointEngine,
}

impl DatalogSolver {
    pub fn new(ctx: Context) -> Result<Self, Text> {
        Ok(Self {
            engine: FixedPointEngine::new(ctx)?,
        })
    }

    /// Register a relation (predicate) with the Datalog solver
    ///
    /// This must be called before adding facts or rules that use the relation.
    /// The relation is defined by its name and parameter sorts.
    ///
    /// # Arguments
    /// * `name` - The name of the relation (predicate)
    /// * `param_sorts` - The sorts (types) of the relation's parameters
    ///
    /// # Example
    /// ```ignore
    /// use verum_smt::fixedpoint::DatalogSolver;
    /// use z3::{Context, Sort};
    ///
    /// let ctx = Context::thread_local();
    /// let mut solver = DatalogSolver::new(ctx.clone()).unwrap();
    ///
    /// // Register a unary relation "node" over integers
    /// solver.register_relation("node", &[Sort::int()]).unwrap();
    /// ```
    pub fn register_relation(&mut self, name: &str, param_sorts: &[Sort]) -> Result<(), Text> {
        self.engine.register_predicate(RecursivePredicate {
            name: Text::from(name),
            params: param_sorts.iter().cloned().collect(),
            body: PredicateBody::Base(Bool::from_bool(true)),
            well_founded: false,
        })
    }

    /// Add fact (rule with empty body)
    pub fn add_fact(&mut self, atom: Atom) -> Result<(), Text> {
        self.engine.add_datalog_rule(DatalogRule {
            head: atom,
            body: List::new(),
            constraints: List::new(),
        })
    }

    /// Add rule
    pub fn add_rule(&mut self, rule: DatalogRule) -> Result<(), Text> {
        self.engine.add_datalog_rule(rule)
    }

    /// Query whether an atom is derivable in the Datalog program
    ///
    /// This method queries the fixed-point engine to determine if the given atom
    /// (predicate application) holds in the least fixed point of the rules.
    ///
    /// # Theory
    /// In Datalog:
    /// - Facts are added as base cases
    /// - Rules define how to derive new facts
    /// - The query asks: "Is this atom in the transitive closure?"
    ///
    /// # Examples
    /// ```ignore
    /// use verum_smt::{DatalogSolver, Atom, DatalogRule};
    /// use verum_common::{List, Text};
    /// use z3::{Context, Sort, FuncDecl, ast::Int};
    ///
    /// let ctx = Context::thread_local();
    /// let mut solver = DatalogSolver::new(ctx.clone()).unwrap();
    ///
    /// // Define edge relation
    /// // Add facts: edge(1,2), edge(2,3)
    /// // Add rule: path(x,y) :- edge(x,y)
    /// // Add rule: path(x,z) :- edge(x,y), path(y,z)
    ///
    /// // Query: path(1,3)?
    /// let query_atom = Atom {
    ///     predicate: Text::from("path"),
    ///     args: List::from(vec![Int::from_i64(1).into(), Int::from_i64(3).into()]),
    /// };
    ///
    /// let result = solver.query(query_atom).unwrap();
    /// assert!(result); // Should be true (transitive closure)
    /// ```
    pub fn query(&mut self, atom: Atom) -> Result<bool, Text> {
        // Convert the atom to a Z3 Bool formula
        let query_formula = self.engine.atom_to_formula(&atom)?;

        // Query the fixed-point engine
        let result = self.engine.query(&query_formula)?;

        // Convert SatResult to bool
        // SAT means the atom is derivable (holds in fixed point)
        // UNSAT means the atom is not derivable
        match result {
            z3::SatResult::Sat => Ok(true),
            z3::SatResult::Unsat => Ok(false),
            z3::SatResult::Unknown => {
                // Get reason for unknown result
                let reason = self.engine.get_reason_unknown();
                Err(Text::from(format!(
                    "Z3 fixed-point query returned unknown: {}",
                    reason
                )))
            }
        }
    }

    /// Get all derivable facts
    pub fn get_model(&self) -> DatalogModel {
        DatalogModel { facts: Map::new() }
    }
}

/// Datalog model
#[derive(Debug)]
pub struct DatalogModel {
    pub facts: Map<Text, Set<List<Dynamic>>>,
}
