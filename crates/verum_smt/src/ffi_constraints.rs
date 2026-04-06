//! FFI Boundary Constraint Translation to SMT
//!
//! This module translates FFI boundary contracts into SMT constraints for verification.
//! FFI boundary contracts are compile-time specifications (not types) that describe
//! cross-language interfaces via C ABI. Verum supports only C ABI for FFI.
//!
//! - **Preconditions** → Requirements (MUST check before calling FFI)
//! - **Postconditions** → Assumptions (HOPE they hold, cannot verify)
//! - **Memory Effects** → Frame conditions (for optimization)
//! - **Ownership** → Allocation constraints (for memory safety)
//!
//! ## Architecture
//!
//! The translation process:
//! 1. Parse FFI function contracts from AST
//! 2. Encode preconditions as SMT assertions to verify
//! 3. Encode postconditions as SMT assumptions (unverified)
//! 4. Encode memory effects as frame conditions using array theory
//! 5. Encode ownership as allocation/deallocation constraints
//!
//! ## SMT Theory Usage
//!
//! - **Bitvectors (BV)**: For pointer representation (64-bit)
//! - **Arrays**: For memory modeling (address → value mapping)
//! - **Quantifiers**: For frame conditions (∀ addr not in range)
//! - **Uninterpreted Functions**: For opaque C functions

use crate::{Context, Error, Result, TranslationError};
use verum_ast::ffi::{FFIBoundary, FFIFunction, MemoryEffects, Ownership};
use verum_ast::{BinOp, Expr, ExprKind, Literal, LiteralKind, Type, UnOp};
use verum_common::{List, Maybe, Text};
use verum_common::ToText;

use z3::ast::{Array, BV, Bool, Dynamic, Int, Real};

/// A translated SMT constraint for FFI verification.
#[derive(Debug, Clone)]
pub struct SMTConstraint {
    /// The Z3 boolean expression representing the constraint
    pub expr: Bool,

    /// Human-readable description of what this constraint checks
    pub description: Text,

    /// Category of constraint (precondition, postcondition, frame, allocation)
    pub category: ConstraintCategory,
}

/// Category of FFI constraint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConstraintCategory {
    /// Precondition that must be verified before FFI call
    Precondition,

    /// Postcondition that is assumed (not verified)
    Postcondition,

    /// Frame condition (memory not modified outside specified range)
    FrameCondition,

    /// Allocation constraint (memory ownership transfer)
    AllocationConstraint,
}

/// Encoder for translating FFI boundary contracts to SMT constraints.
pub struct FFIConstraintEncoder<'ctx> {
    context: &'ctx Context,

    /// Variable bindings for current scope
    bindings: std::collections::HashMap<Text, Dynamic>,

    /// Memory array for modeling heap
    memory_pre: Maybe<Array>,
    memory_post: Maybe<Array>,
}

impl<'ctx> FFIConstraintEncoder<'ctx> {
    /// Create a new FFI constraint encoder.
    pub fn new(context: &'ctx Context) -> Self {
        Self {
            context,
            bindings: std::collections::HashMap::new(),
            memory_pre: Maybe::None,
            memory_post: Maybe::None,
        }
    }

    /// Encode all constraints for an FFI boundary.
    ///
    /// Returns a list of SMT constraints for all functions in the boundary.
    pub fn encode_boundary(&mut self, boundary: &FFIBoundary) -> Result<List<SMTConstraint>> {
        let mut constraints = List::new();

        for function in &boundary.functions {
            let mut func_constraints = self.encode_function(function)?;
            constraints.append(&mut func_constraints);
        }

        Ok(constraints)
    }

    /// Encode all constraints for a single FFI function.
    pub fn encode_function(&mut self, function: &FFIFunction) -> Result<List<SMTConstraint>> {
        let mut constraints = List::new();

        // Bind function parameters
        for (param_name, param_type) in &function.signature.params {
            let z3_var = self.create_variable(param_name.as_str(), param_type)?;
            self.bindings
                .insert(Text::from(param_name.as_str()), z3_var);
        }

        // 1. Encode preconditions as requirements
        for precond in &function.requires {
            let constraint = self.encode_precondition(precond, function)?;
            constraints.push(constraint);
        }

        // 2. Encode postconditions as assumptions
        for postcond in &function.ensures {
            let constraint = self.encode_postcondition(postcond, function)?;
            constraints.push(constraint);
        }

        // 3. Encode memory effects as frame conditions
        let memory_constraints = self.encode_memory_effects(&function.memory_effects, function)?;
        constraints.extend(memory_constraints);

        // 4. Encode ownership as allocation constraints
        let ownership_constraints = self.encode_ownership(&function.ownership, function)?;
        constraints.extend(ownership_constraints);

        // Clear bindings for next function
        self.bindings.clear();

        Ok(constraints)
    }

    /// Encode a precondition expression to SMT.
    ///
    /// Preconditions must be verified at the Verum side before calling FFI.
    pub fn encode_precondition(
        &self,
        precond: &Expr,
        function: &FFIFunction,
    ) -> Result<SMTConstraint> {
        // Create encoder with function parameter bindings
        let mut encoder_with_params = Self {
            context: self.context,
            bindings: self.bindings.clone(),
            memory_pre: self.memory_pre.clone(),
            memory_post: self.memory_post.clone(),
        };

        // Bind function parameters if not already bound
        for (param_name, param_type) in &function.signature.params {
            let param_name_text = Text::from(param_name.as_str());
            if !encoder_with_params.bindings.contains_key(&param_name_text) {
                let z3_var =
                    encoder_with_params.create_variable(param_name.as_str(), param_type)?;
                encoder_with_params.bindings.insert(param_name_text, z3_var);
            }
        }

        let z3_expr = encoder_with_params.translate_expr(precond)?;
        let bool_expr = z3_expr
            .as_bool()
            .ok_or_else(|| Error::Internal("Precondition must be boolean".to_string()))?;

        Ok(SMTConstraint {
            expr: bool_expr,
            description: Text::from(format!(
                "Precondition for {}: {:?}",
                function.name.as_str(),
                precond
            )),
            category: ConstraintCategory::Precondition,
        })
    }

    /// Encode a postcondition expression to SMT.
    ///
    /// Postconditions are assumptions - we hope they hold but cannot verify them
    /// since the FFI implementation is opaque.
    pub fn encode_postcondition(
        &self,
        postcond: &Expr,
        function: &FFIFunction,
    ) -> Result<SMTConstraint> {
        // Create encoder with parameter and result bindings
        let mut encoder_with_bindings = Self {
            context: self.context,
            bindings: self.bindings.clone(),
            memory_pre: self.memory_pre.clone(),
            memory_post: self.memory_post.clone(),
        };

        // Bind function parameters if not already bound
        for (param_name, param_type) in &function.signature.params {
            let param_name_text = Text::from(param_name.as_str());
            if !encoder_with_bindings
                .bindings
                .contains_key(&param_name_text)
            {
                let z3_var =
                    encoder_with_bindings.create_variable(param_name.as_str(), param_type)?;
                encoder_with_bindings
                    .bindings
                    .insert(param_name_text, z3_var);
            }
        }

        // Bind result variable
        let result_var =
            encoder_with_bindings.create_variable("result", &function.signature.return_type)?;
        encoder_with_bindings
            .bindings
            .insert(Text::from("result"), result_var);

        let z3_expr = encoder_with_bindings.translate_expr(postcond)?;
        let bool_expr = z3_expr
            .as_bool()
            .ok_or_else(|| Error::Internal("Postcondition must be boolean".to_string()))?;

        Ok(SMTConstraint {
            expr: bool_expr,
            description: Text::from(format!(
                "Postcondition for {}: {:?}",
                function.name.as_str(),
                postcond
            )),
            category: ConstraintCategory::Postcondition,
        })
    }

    /// Encode memory effects as frame conditions.
    ///
    /// Frame conditions specify what memory can change across the FFI call:
    /// - Pure: No memory changes
    /// - Reads: No memory changes
    /// - Writes: Only specified range changes
    pub fn encode_memory_effects(
        &mut self,
        effects: &MemoryEffects,
        function: &FFIFunction,
    ) -> Result<List<SMTConstraint>> {
        let mut constraints = List::new();

        // Initialize memory arrays if not already done
        if self.memory_pre.is_none() {
            let bv_sort = z3::Sort::bitvector(64);
            let byte_sort = z3::Sort::bitvector(8);
            self.memory_pre = Maybe::Some(Array::new_const("mem_pre", &bv_sort, &byte_sort));
            self.memory_post = Maybe::Some(Array::new_const("mem_post", &bv_sort, &byte_sort));
        }

        match effects {
            MemoryEffects::Pure => {
                // Pure functions don't modify any memory
                let constraint = self.encode_no_memory_change(function)?;
                constraints.push(constraint);
            }

            MemoryEffects::Reads(_) => {
                // Read-only functions also don't modify memory
                let constraint = self.encode_no_memory_change(function)?;
                constraints.push(constraint);
            }

            MemoryEffects::Writes(ranges) => {
                // Only specified ranges can be modified
                let constraint = self.encode_frame_condition(ranges, function)?;
                constraints.push(constraint);
            }

            MemoryEffects::Allocates => {
                // Allocation doesn't require frame condition
                // (handled by ownership constraints)
            }

            MemoryEffects::Deallocates(_) => {
                // Deallocation doesn't require frame condition
                // (handled by ownership constraints)
            }

            MemoryEffects::Combined(effects_list) => {
                // Recursively encode each effect
                for effect in effects_list {
                    let mut effect_constraints = self.encode_memory_effects(effect, function)?;
                    constraints.append(&mut effect_constraints);
                }
            }
        }

        Ok(constraints)
    }

    /// Encode "no memory change" constraint (for Pure and Reads).
    fn encode_no_memory_change(&self, function: &FFIFunction) -> Result<SMTConstraint> {
        let mem_pre = self
            .memory_pre
            .as_ref()
            .ok_or_else(|| Error::Internal("Memory arrays not initialized".to_string()))?;
        let mem_post = self
            .memory_post
            .as_ref()
            .ok_or_else(|| Error::Internal("Memory arrays not initialized".to_string()))?;

        // Create quantified formula: ∀ addr. mem_post[addr] == mem_pre[addr]
        let addr_var = BV::new_const("addr", 64);
        let pre_value = mem_pre.select(&addr_var);
        let post_value = mem_post.select(&addr_var);

        let bool_equality = pre_value.eq(&post_value);

        // Create forall quantifier
        let forall_expr = z3::ast::forall_const(&[&addr_var], &[], &bool_equality);

        Ok(SMTConstraint {
            expr: forall_expr,
            description: Text::from(format!(
                "Frame condition for {}: no memory modified",
                function.name.as_str()
            )),
            category: ConstraintCategory::FrameCondition,
        })
    }

    /// Encode frame condition for specific memory ranges.
    fn encode_frame_condition(
        &self,
        ranges: &verum_common::Maybe<verum_common::List<verum_common::Text>>,
        function: &FFIFunction,
    ) -> Result<SMTConstraint> {
        let mem_pre = self
            .memory_pre
            .as_ref()
            .ok_or_else(|| Error::Internal("Memory arrays not initialized".to_string()))?;
        let mem_post = self
            .memory_post
            .as_ref()
            .ok_or_else(|| Error::Internal("Memory arrays not initialized".to_string()))?;

        match ranges {
            Maybe::None => {
                // No specific ranges - anything can change
                // Create trivially true constraint
                let true_expr = Bool::from_bool(true);
                Ok(SMTConstraint {
                    expr: true_expr,
                    description: format!(
                        "Frame condition for {}: unrestricted writes",
                        function.name.as_str()
                    )
                    .to_text(),
                    category: ConstraintCategory::FrameCondition,
                })
            }

            Maybe::Some(range_names) => {
                // Only specified ranges can change
                // For simplicity, we encode: ∀ addr. (addr not in ranges) → (mem_post[addr] == mem_pre[addr])
                let addr_var = BV::new_const("addr", 64);
                let pre_value = mem_pre.select(&addr_var);
                let post_value = mem_post.select(&addr_var);

                // Build "not in ranges" predicate
                let mut not_in_ranges = Bool::from_bool(true);
                for range_name in range_names {
                    // Get range pointer from bindings
                    if let Some(range_ptr) = self.bindings.get(range_name.as_str())
                        && let Some(ptr_bv) = range_ptr.as_bv()
                    {
                        // addr != range_ptr
                        let not_equal = addr_var.eq(&ptr_bv).not();
                        not_in_ranges = Bool::and(&[&not_in_ranges, &not_equal]);
                    }
                }

                // (addr not in ranges) → (mem_post[addr] == mem_pre[addr])
                let bool_equality = pre_value.eq(&post_value);
                let implication = not_in_ranges.implies(&bool_equality);

                // Create forall quantifier
                let forall_expr = z3::ast::forall_const(&[&addr_var], &[], &implication);

                Ok(SMTConstraint {
                    expr: forall_expr,
                    description: format!(
                        "Frame condition for {}: writes only to {:?}",
                        function.name.as_str(),
                        range_names
                    )
                    .to_text(),
                    category: ConstraintCategory::FrameCondition,
                })
            }
        }
    }

    /// Encode ownership semantics as allocation constraints.
    pub fn encode_ownership(
        &self,
        ownership: &Ownership,
        function: &FFIFunction,
    ) -> Result<List<SMTConstraint>> {
        let mut constraints = List::new();

        match ownership {
            Ownership::Borrow => {
                // Borrowed references don't transfer ownership
                // No allocation constraints needed
            }

            Ownership::TransferTo(param_name) => {
                // We transfer ownership to C - parameter must be non-null
                let param_text = Text::from(param_name.as_str());
                if let Some(ptr_var) = self.bindings.get(&param_text)
                    && let Some(ptr_bv) = ptr_var.as_bv()
                {
                    let null_ptr = BV::from_u64(0, 64);
                    let not_null = ptr_bv.eq(&null_ptr).not();

                    constraints.push(SMTConstraint {
                        expr: not_null,
                        description: format!(
                            "Allocation constraint for {}: {} must be non-null for transfer",
                            function.name.as_str(),
                            param_name
                        )
                        .to_text(),
                        category: ConstraintCategory::AllocationConstraint,
                    });
                }
            }

            Ownership::TransferFrom(result_name) => {
                // C transfers ownership to us - result must be checked for null
                // This is typically checked in postconditions, so we create a reminder
                let true_expr = Bool::from_bool(true);
                constraints.push(SMTConstraint {
                    expr: true_expr,
                    description: format!(
                        "Allocation constraint for {}: caller must check {} for null",
                        function.name.as_str(),
                        result_name
                    )
                    .to_text(),
                    category: ConstraintCategory::AllocationConstraint,
                });
            }

            Ownership::Shared => {
                // Shared access - no ownership transfer
                // No allocation constraints needed
            }
        }

        Ok(constraints)
    }

    /// Translate a Verum expression to Z3.
    fn translate_expr(&self, expr: &Expr) -> Result<Dynamic> {
        match &expr.kind {
            ExprKind::Literal(lit) => self.translate_literal(lit),

            ExprKind::Path(path) => {
                if let Some(ident) = path.as_ident() {
                    let name = ident.as_str();

                    // Check bindings
                    let name_text = Text::from(name);
                    if let Some(var) = self.bindings.get(&name_text) {
                        return Ok(var.clone());
                    }

                    // Boolean constants
                    match name {
                        "true" => Ok(Dynamic::from_ast(&Bool::from_bool(true))),
                        "false" => Ok(Dynamic::from_ast(&Bool::from_bool(false))),
                        _ => {
                            // Create fresh variable (default to Int)
                            Ok(Dynamic::from_ast(&Int::new_const(name)))
                        }
                    }
                } else {
                    Err(Error::Translation(TranslationError::UnsupportedPath(
                        format!("{:?}", path).into(),
                    )))
                }
            }

            ExprKind::Binary { op, left, right } => self.translate_binary_op(*op, left, right),

            ExprKind::Unary { op, expr } => self.translate_unary_op(*op, expr),

            ExprKind::Call { func, args, .. } => self.translate_call(func, args),

            ExprKind::Paren(inner) => self.translate_expr(inner),

            ExprKind::Index { expr, index } => {
                // Array indexing: arr[i]
                let array_expr = self.translate_expr(expr)?;
                let index_expr = self.translate_expr(index)?;

                // For now, treat as select operation
                if let Some(array) = array_expr.as_array() {
                    Ok(array.select(&index_expr))
                } else {
                    Err(Error::Internal("Index operation on non-array".to_string()))
                }
            }

            ExprKind::Field { expr, field } => {
                let field_name = field.as_str();

                match field_name {
                    "length" | "len" | "size" => {
                        // Array/string length
                        let base_name = format!("length_of_{:?}", expr);
                        Ok(Dynamic::from_ast(&Int::new_const(base_name.as_str())))
                    }
                    _ => Err(Error::Translation(TranslationError::UnsupportedExpr(
                        format!("Field access: {}", field_name).into(),
                    ))),
                }
            }

            _ => Err(Error::Translation(TranslationError::UnsupportedExpr(
                format!("{:?}", expr.kind).into(),
            ))),
        }
    }

    /// Translate a literal to Z3.
    fn translate_literal(&self, lit: &Literal) -> Result<Dynamic> {
        match &lit.kind {
            LiteralKind::Int(int_lit) => {
                // Convert i128 to i64 (may overflow for very large values)
                let z3_int = Int::from_i64(int_lit.value as i64);
                Ok(Dynamic::from_ast(&z3_int))
            }

            LiteralKind::Float(float_lit) => {
                // Use Real sort for floats
                let val_str = format!("{}", float_lit.value);
                // Z3 Real from string (as fraction numerator/denominator)
                let z3_real = Real::from_rational_str(&val_str, "1").ok_or_else(|| {
                    Error::Internal(format!("Invalid float: {}", float_lit.value))
                })?;
                Ok(Dynamic::from_ast(&z3_real))
            }

            LiteralKind::Bool(value) => {
                let z3_bool = Bool::from_bool(*value);
                Ok(Dynamic::from_ast(&z3_bool))
            }

            LiteralKind::Text(_) | LiteralKind::Char(_) => {
                // Strings not fully supported in SMT
                Err(Error::Unsupported("String literals in SMT".to_string()))
            }

            _ => Err(Error::Translation(TranslationError::UnsupportedLiteral(
                format!("{:?}", lit.kind).into(),
            ))),
        }
    }

    /// Translate a binary operation.
    fn translate_binary_op(&self, op: BinOp, left: &Expr, right: &Expr) -> Result<Dynamic> {
        let left_z3 = self.translate_expr(left)?;
        let right_z3 = self.translate_expr(right)?;

        // Try different numeric types with type coercion support
        let result = if let (Some(l_int), Some(r_int)) = (left_z3.as_int(), right_z3.as_int()) {
            self.translate_binary_op_int(op, &l_int, &r_int)?
        } else if let (Some(l_real), Some(r_real)) = (left_z3.as_real(), right_z3.as_real()) {
            self.translate_binary_op_real(op, &l_real, &r_real)?
        } else if let (Some(l_bool), Some(r_bool)) = (left_z3.as_bool(), right_z3.as_bool()) {
            self.translate_binary_op_bool(op, &l_bool, &r_bool)?
        } else if let (Some(l_bv), Some(r_bv)) = (left_z3.as_bv(), right_z3.as_bv()) {
            self.translate_binary_op_bv(op, &l_bv, &r_bv)?
        // Handle BV vs Int comparison (pointer vs integer literal)
        // This handles cases like `ptr != 0` where ptr is a pointer (BV) and 0 is an int literal
        } else if let (Some(l_bv), Some(r_int)) = (left_z3.as_bv(), right_z3.as_int()) {
            // Try to get the concrete value from the Int constant
            // For pointer comparisons with literals (like null check with 0)
            let bv_size = l_bv.get_size();
            // Check if it's a numeric literal (constant)
            if let Some(r_val) = r_int.as_i64() {
                let r_bv = BV::from_i64(r_val, bv_size);
                self.translate_binary_op_bv(op, &l_bv, &r_bv)?
            } else {
                // For symbolic ints, create a fresh BV variable with same constraints
                // This is a simplification - full int2bv would require Z3 context
                let r_bv = BV::from_i64(0, bv_size); // Fallback for non-constant
                self.translate_binary_op_bv(op, &l_bv, &r_bv)?
            }
        } else if let (Some(l_int), Some(r_bv)) = (left_z3.as_int(), right_z3.as_bv()) {
            // Same handling for reversed operand order
            let bv_size = r_bv.get_size();
            if let Some(l_val) = l_int.as_i64() {
                let l_bv = BV::from_i64(l_val, bv_size);
                self.translate_binary_op_bv(op, &l_bv, &r_bv)?
            } else {
                let l_bv = BV::from_i64(0, bv_size);
                self.translate_binary_op_bv(op, &l_bv, &r_bv)?
            }
        // Handle Int vs Real comparison (promote Int to Real)
        } else if let (Some(l_int), Some(r_real)) = (left_z3.as_int(), right_z3.as_real()) {
            let l_real = l_int.to_real();
            self.translate_binary_op_real(op, &l_real, &r_real)?
        } else if let (Some(l_real), Some(r_int)) = (left_z3.as_real(), right_z3.as_int()) {
            let r_real = r_int.to_real();
            self.translate_binary_op_real(op, &l_real, &r_real)?
        } else {
            return Err(Error::Internal(format!(
                "Type mismatch in binary op: {:?}",
                op
            )));
        };

        Ok(result)
    }

    /// Translate binary op for integers.
    fn translate_binary_op_int(&self, op: BinOp, left: &Int, right: &Int) -> Result<Dynamic> {
        let result = match op {
            BinOp::Add => Dynamic::from_ast(&Int::add(&[left, right])),
            BinOp::Sub => Dynamic::from_ast(&Int::sub(&[left, right])),
            BinOp::Mul => Dynamic::from_ast(&Int::mul(&[left, right])),
            BinOp::Div => Dynamic::from_ast(&left.div(right)),
            BinOp::Rem => Dynamic::from_ast(&left.rem(right)),
            BinOp::Lt => Dynamic::from_ast(&left.lt(right)),
            BinOp::Le => Dynamic::from_ast(&left.le(right)),
            BinOp::Gt => Dynamic::from_ast(&left.gt(right)),
            BinOp::Ge => Dynamic::from_ast(&left.ge(right)),
            BinOp::Eq => Dynamic::from_ast(&left.eq(right)),
            BinOp::Ne => Dynamic::from_ast(&left.eq(right).not()),
            _ => {
                return Err(Error::Internal(format!("Unsupported int op: {:?}", op)));
            }
        };
        Ok(result)
    }

    /// Translate binary op for reals (floats).
    fn translate_binary_op_real(&self, op: BinOp, left: &Real, right: &Real) -> Result<Dynamic> {
        let result = match op {
            BinOp::Add => Dynamic::from_ast(&Real::add(&[left, right])),
            BinOp::Sub => Dynamic::from_ast(&Real::sub(&[left, right])),
            BinOp::Mul => Dynamic::from_ast(&Real::mul(&[left, right])),
            BinOp::Div => Dynamic::from_ast(&left.div(right)),
            BinOp::Lt => Dynamic::from_ast(&left.lt(right)),
            BinOp::Le => Dynamic::from_ast(&left.le(right)),
            BinOp::Gt => Dynamic::from_ast(&left.gt(right)),
            BinOp::Ge => Dynamic::from_ast(&left.ge(right)),
            BinOp::Eq => Dynamic::from_ast(&left.eq(right)),
            BinOp::Ne => Dynamic::from_ast(&left.eq(right).not()),
            _ => {
                return Err(Error::Internal(format!("Unsupported real op: {:?}", op)));
            }
        };
        Ok(result)
    }

    /// Translate binary op for booleans.
    fn translate_binary_op_bool(&self, op: BinOp, left: &Bool, right: &Bool) -> Result<Dynamic> {
        let result = match op {
            BinOp::And => Dynamic::from_ast(&Bool::and(&[left, right])),
            BinOp::Or => Dynamic::from_ast(&Bool::or(&[left, right])),
            BinOp::Eq => Dynamic::from_ast(&left.eq(right)),
            BinOp::Ne => Dynamic::from_ast(&left.eq(right).not()),
            BinOp::Imply => Dynamic::from_ast(&left.implies(right)),
            _ => {
                return Err(Error::Internal(format!("Unsupported bool op: {:?}", op)));
            }
        };
        Ok(result)
    }

    /// Translate binary op for bitvectors (pointers).
    fn translate_binary_op_bv(&self, op: BinOp, left: &BV, right: &BV) -> Result<Dynamic> {
        let result = match op {
            BinOp::Add => Dynamic::from_ast(&left.bvadd(right)),
            BinOp::Sub => Dynamic::from_ast(&left.bvsub(right)),
            BinOp::Mul => Dynamic::from_ast(&left.bvmul(right)),
            BinOp::Eq => Dynamic::from_ast(&left.eq(right)),
            BinOp::Ne => Dynamic::from_ast(&left.eq(right).not()),
            BinOp::BitAnd => Dynamic::from_ast(&left.bvand(right)),
            BinOp::BitOr => Dynamic::from_ast(&left.bvor(right)),
            BinOp::BitXor => Dynamic::from_ast(&left.bvxor(right)),
            BinOp::Shl => Dynamic::from_ast(&left.bvshl(right)),
            BinOp::Shr => Dynamic::from_ast(&left.bvlshr(right)),
            _ => {
                return Err(Error::Internal(format!("Unsupported bv op: {:?}", op)));
            }
        };
        Ok(result)
    }

    /// Translate a unary operation.
    fn translate_unary_op(&self, op: UnOp, expr: &Expr) -> Result<Dynamic> {
        let expr_z3 = self.translate_expr(expr)?;

        let result = match op {
            UnOp::Neg => {
                if let Some(int_val) = expr_z3.as_int() {
                    Dynamic::from_ast(&int_val.unary_minus())
                } else if let Some(real_val) = expr_z3.as_real() {
                    Dynamic::from_ast(&real_val.unary_minus())
                } else {
                    return Err(Error::Internal(
                        "Cannot negate non-numeric value".to_string(),
                    ));
                }
            }

            UnOp::Not => {
                if let Some(bool_val) = expr_z3.as_bool() {
                    Dynamic::from_ast(&bool_val.not())
                } else {
                    return Err(Error::Internal(
                        "Cannot negate non-boolean value".to_string(),
                    ));
                }
            }

            _ => {
                return Err(Error::Internal(format!("Unsupported unary op: {:?}", op)));
            }
        };

        Ok(result)
    }

    /// Translate a function call.
    fn translate_call(&self, func: &Expr, args: &List<Expr>) -> Result<Dynamic> {
        // Handle built-in functions
        if let ExprKind::Path(path) = &func.kind
            && let Some(ident) = path.as_ident()
        {
            let func_name = ident.as_str();

            return match func_name {
                "abs" => self.translate_abs(args),
                "min" => self.translate_min(args),
                "max" => self.translate_max(args),
                "old" => self.translate_old(args),
                _ => {
                    // Uninterpreted function
                    Err(Error::Internal(format!(
                        "Uninterpreted function: {}",
                        func_name
                    )))
                }
            };
        }

        Err(Error::Internal(
            "Complex function calls not supported".to_string(),
        ))
    }

    /// Translate abs() function.
    fn translate_abs(&self, args: &List<Expr>) -> Result<Dynamic> {
        if args.len() != 1 {
            return Err(Error::Internal(
                "abs() requires exactly 1 argument".to_string(),
            ));
        }

        let arg_z3 = self.translate_expr(&args[0])?;

        if let Some(int_val) = arg_z3.as_int() {
            let zero = Int::from_i64(0);
            let is_negative = int_val.lt(&zero);
            let negated = int_val.unary_minus();

            // ite(x < 0, -x, x)
            let result = is_negative.ite(&negated, &int_val);
            Ok(Dynamic::from_ast(&result))
        } else if let Some(real_val) = arg_z3.as_real() {
            let zero = Real::from_rational(0, 1);
            let is_negative = real_val.lt(&zero);
            let negated = real_val.unary_minus();

            let result = is_negative.ite(&negated, &real_val);
            Ok(Dynamic::from_ast(&result))
        } else {
            Err(Error::Internal(
                "abs() requires numeric argument".to_string(),
            ))
        }
    }

    /// Translate min() function.
    fn translate_min(&self, args: &List<Expr>) -> Result<Dynamic> {
        if args.len() != 2 {
            return Err(Error::Internal(
                "min() requires exactly 2 arguments".to_string(),
            ));
        }

        let arg1_z3 = self.translate_expr(&args[0])?;
        let arg2_z3 = self.translate_expr(&args[1])?;

        if let (Some(int1), Some(int2)) = (arg1_z3.as_int(), arg2_z3.as_int()) {
            let cond = int1.lt(&int2);
            let result = cond.ite(&int1, &int2);
            Ok(Dynamic::from_ast(&result))
        } else if let (Some(real1), Some(real2)) = (arg1_z3.as_real(), arg2_z3.as_real()) {
            let cond = real1.lt(&real2);
            let result = cond.ite(&real1, &real2);
            Ok(Dynamic::from_ast(&result))
        } else {
            Err(Error::Internal(
                "min() requires numeric arguments".to_string(),
            ))
        }
    }

    /// Translate max() function.
    fn translate_max(&self, args: &List<Expr>) -> Result<Dynamic> {
        if args.len() != 2 {
            return Err(Error::Internal(
                "max() requires exactly 2 arguments".to_string(),
            ));
        }

        let arg1_z3 = self.translate_expr(&args[0])?;
        let arg2_z3 = self.translate_expr(&args[1])?;

        if let (Some(int1), Some(int2)) = (arg1_z3.as_int(), arg2_z3.as_int()) {
            let cond = int1.gt(&int2);
            let result = cond.ite(&int1, &int2);
            Ok(Dynamic::from_ast(&result))
        } else if let (Some(real1), Some(real2)) = (arg1_z3.as_real(), arg2_z3.as_real()) {
            let cond = real1.gt(&real2);
            let result = cond.ite(&real1, &real2);
            Ok(Dynamic::from_ast(&result))
        } else {
            Err(Error::Internal(
                "max() requires numeric arguments".to_string(),
            ))
        }
    }

    /// Translate old() function for postconditions.
    fn translate_old(&self, args: &List<Expr>) -> Result<Dynamic> {
        if args.len() != 1 {
            return Err(Error::Internal(
                "old() requires exactly 1 argument".to_string(),
            ));
        }

        // Look for variable with _pre suffix
        if let ExprKind::Path(path) = &args[0].kind
            && let Some(ident) = path.as_ident()
        {
            let old_name = Text::from(format!("{}_pre", ident.as_str()));
            if let Some(var) = self.bindings.get(&old_name) {
                return Ok(var.clone());
            }
        }

        // If not found, create a fresh "old" variable
        let base_name = format!("old_{:?}", args[0]);
        Ok(Dynamic::from_ast(&Int::new_const(base_name.as_str())))
    }

    /// Create a Z3 variable for a parameter.
    fn create_variable(&self, name: &str, ty: &Type) -> Result<Dynamic> {
        use verum_ast::TypeKind;

        let var = match &ty.kind {
            TypeKind::Int => Dynamic::from_ast(&Int::new_const(name)),

            TypeKind::Float => Dynamic::from_ast(&Real::new_const(name)),

            TypeKind::Bool => Dynamic::from_ast(&Bool::new_const(name)),

            TypeKind::Reference { .. } => {
                // References as 64-bit bitvectors (pointer representation)
                Dynamic::from_ast(&BV::new_const(name, 64))
            }

            TypeKind::Pointer { .. } => {
                // Pointers as 64-bit bitvectors (pointer representation)
                Dynamic::from_ast(&BV::new_const(name, 64))
            }

            _ => {
                return Err(Error::Unsupported(format!("Type {:?} in FFI", ty)));
            }
        };

        Ok(var)
    }
}

/// Verification result for FFI calls.
#[derive(Debug, Clone)]
pub enum VerificationResult {
    /// All preconditions verified, safe to call
    Success,

    /// Precondition violated
    PreconditionViolation {
        constraint: SMTConstraint,
        counterexample: Maybe<Text>,
    },

    /// Unknown result (timeout, solver limitation)
    Unknown,
}

/// Verify an FFI call with given arguments.
///
/// This checks that all preconditions hold before making the FFI call.
pub fn verify_ffi_call<'ctx>(
    context: &Context,
    function: &FFIFunction,
    encoder: &mut FFIConstraintEncoder<'ctx>,
) -> Result<VerificationResult> {
    // Encode all function constraints
    let constraints = encoder.encode_function(function)?;

    // Create solver
    let solver = context.solver();

    // Check each precondition
    for constraint in constraints {
        if constraint.category == ConstraintCategory::Precondition {
            // Negate the precondition to find violations
            solver.assert(constraint.expr.not());

            match context.check(&solver) {
                z3::SatResult::Unsat => {
                    // Negation is unsat, so precondition always holds
                    solver.reset();
                    continue;
                }
                z3::SatResult::Sat => {
                    // Found counterexample
                    let model = context.get_model(&solver);
                    let counterexample = model.map(|m| Text::from(format!("{}", m)));

                    return Ok(VerificationResult::PreconditionViolation {
                        constraint,
                        counterexample,
                    });
                }
                z3::SatResult::Unknown => {
                    return Ok(VerificationResult::Unknown);
                }
            }
        }
    }

    Ok(VerificationResult::Success)
}

#[cfg(test)]
mod tests {
    // Tests moved to tests/ffi_constraints_tests.rs
}
