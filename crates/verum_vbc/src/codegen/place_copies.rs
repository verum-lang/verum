//! Semantic authority for explicit dereference copies at existing value boundaries.
//! ABI layouts and missing declaration facts do not authorize duplication.
use super::VbcCodegen;
use crate::{Instruction, instruction::Reg, types::TypeRef};
use verum_ast::{Expr, ExprKind, UnOp};
use verum_common::Maybe;

impl VbcCodegen {
    /// Parentheses preserve a place for both copies and consuming handoffs.
    pub(super) fn normalized_place(mut expr: &Expr) -> &Expr {
        while let ExprKind::Paren(inner) = &expr.kind { expr = inner; }
        expr
    }

    /// `start..end` belongs to this expression evaluation, captured before any
    /// later operand or destination binding can replace its registers/names.
    pub(super) fn copy_source_type(
        &self, expr: &Expr, reg: Reg, start: usize, end: usize,
    ) -> Maybe<TypeRef> {
        self.copy_source_type_inner(expr, Some((reg, start, end)), 0)
    }

    fn copy_source_type_inner(
        &self, expr: &Expr, emitted: Maybe<(Reg, usize, usize)>, depth: usize,
    ) -> Maybe<TypeRef> {
        use crate::types::{CbgrTier, Mutability};
        if depth >= 64 { return None; }
        let nested = |value, emitted| self.copy_source_type_inner(value, emitted, depth + 1);
        match &Self::normalized_place(expr).kind {
            ExprKind::Path(path) => {
                let binding = self.ctx.lookup_var(path.as_ident()?.name.as_str())?;
                if binding.is_cell || binding.is_pattern_alias { return None; }
                let fact = self.ctx.registers.value_uses.active.get(&binding.reg)?;
                (fact.id == binding.binding_id).then(|| fact.declaration_type.clone()).flatten()
            }
            ExprKind::Record { path, .. } => {
                let ast = verum_ast::Type::new(verum_ast::ty::TypeKind::Path(path.clone()), expr.span);
                self.semantic_formal_type(&ast, &self.ctx.current_generic_param_ids, &[])
            }
            ExprKind::Unary { op: UnOp::Deref, expr: inner } => {
                let input = emitted.and_then(|(reg, start, end)| {
                    let (index, op) = self.copy_result_producer(reg, start, end)?;
                    if let Instruction::Deref { ref_reg, .. } = op {
                        Some((*ref_reg, start, index))
                    } else { None }
                });
                match nested(inner, input)? {
                    TypeRef::Reference { inner, .. } => Some(*inner),
                    _ => None,
                }
            }
            ExprKind::Unary { op, expr: inner } if matches!(op,
                UnOp::Ref | UnOp::RefMut | UnOp::RefChecked | UnOp::RefCheckedMut
                    | UnOp::RefUnsafe | UnOp::RefUnsafeMut) => Some(TypeRef::Reference {
                inner: Box::new(nested(inner, None)?),
                mutability: if matches!(op, UnOp::RefMut | UnOp::RefCheckedMut | UnOp::RefUnsafeMut) {
                    Mutability::Mutable
                } else { Mutability::Immutable },
                tier: match op {
                    UnOp::RefChecked | UnOp::RefCheckedMut => CbgrTier::Tier1,
                    UnOp::RefUnsafe | UnOp::RefUnsafeMut => CbgrTier::Tier2,
                    _ => CbgrTier::Tier0,
                },
            }),
            ExprKind::Call { .. } | ExprKind::MethodCall { .. } => {
                let (reg, start, end) = emitted?;
                self.copy_selected_call_result(reg, start, end)
            }
            // Projection layouts, raw pointers, dynamic calls and unresolved
            // expressions cannot establish the missing declaration authority.
            _ => None,
        }
    }

    /// Recognize only an uninterrupted result flow within the captured interval.
    /// Every other instruction (including branches and unknown writes) ends proof.
    fn copy_result_producer(&self, mut reg: Reg, start: usize, end: usize) -> Maybe<(usize, &Instruction)> {
        let instructions = self.ctx.instructions.get(start..end)?;
        if self.ctx.has_label_in_instruction_range(start, end) { return None; }
        for (offset, instruction) in instructions.iter().enumerate().rev().take(128) {
            match instruction {
                Instruction::Mov { dst, src } if *dst == reg => reg = *src,
                Instruction::ChkRef { ref_reg } if *ref_reg == reg => {},
                Instruction::Call { dst, .. } | Instruction::CallG { dst, .. }
                    | Instruction::Deref { dst, .. } if *dst == reg => return Some((start + offset, instruction)),
                _ => return None,
            }
        }
        None
    }

    /// Actual call operands may forward a declaration; they do not derive a
    /// semantic type from a Load/Ref opcode or a runtime carrier's shape.
    fn copy_argument_declaration(&self, mut reg: Reg, start: usize, end: usize) -> Maybe<TypeRef> {
        if let Some(ty) = self.copy_active_declaration(reg) { return Some(ty); }
        for instruction in self.ctx.instructions.get(start..end)?.iter().rev().take(128) {
            match instruction {
                Instruction::Mov { dst, src } => {
                    if *dst == reg {
                        reg = *src;
                        if let Some(ty) = self.copy_active_declaration(reg) { return Some(ty); }
                    }
                }
                Instruction::ChkRef { .. } => {},
                _ => return None,
            }
        }
        None
    }

    fn copy_active_declaration(&self, reg: Reg) -> Maybe<TypeRef> {
        let fact = self.ctx.registers.value_uses.active.get(&reg)?;
        self.ctx.registers.all_variable_names().iter().any(|name| {
            self.ctx.lookup_var(name).is_some_and(|binding| binding.reg == reg
                && binding.binding_id == fact.id && !binding.is_cell && !binding.is_pattern_alias)
        }).then(|| fact.declaration_type.clone()).flatten()
    }

    fn copy_selected_call_result(&self, reg: Reg, start: usize, end: usize) -> Maybe<TypeRef> {
        let (index, op) = self.copy_result_producer(reg, start, end)?;
        let (id, args) = match op {
            Instruction::Call { func_id, args, .. } | Instruction::CallG { func_id, args, .. } => (*func_id, args),
            _ => return None,
        };
        let info = self.ctx.lookup_function_by_id(crate::module::FunctionId(id))?;
        let TypeRef::Function { params, return_type, .. } = info.callable_signature.as_ref()? else { return None; };
        if params.len() != usize::from(args.count) { return None; }
        let mut substitution = crate::mono::TypeSubstitution::empty();
        for (offset, formal) in params.iter().enumerate() {
            let argument = Reg(args.start.0.checked_add(u16::try_from(offset).ok()?)?);
            let Some(actual) = self.copy_argument_declaration(argument, start, index) else { continue; };
            // The declaration's self parameter permits the same outer auto-borrow
            // as the selected method call. Nested generic arguments are not peeled.
            let (formal, actual) = if offset == 0
                && info.param_names.first().is_some_and(|name| name == "self") {
                let mut formal = formal;
                let mut actual = &actual;
                while let TypeRef::Reference { inner, .. } = formal { formal = inner; }
                while let TypeRef::Reference { inner, .. } = actual { actual = inner; }
                if !matches!(formal, TypeRef::Concrete(_) | TypeRef::Instantiated { .. }) { return None; }
                (formal, actual)
            } else { (formal, &actual) };
            if !bind_copy_parameter(formal, actual, &mut substitution, 0) { return None; }
        }
        // Layout-only archive returns and unbound slots remain unknown. In
        // particular, CallG's ABI witnesses cannot reconstruct reference layers.
        Some(substitution.apply(return_type))
    }

    pub(super) fn copy_type_is_value(&self, ty: &TypeRef) -> bool {
        use crate::types::{TypeId, TypeKind};
        match ty {
            TypeRef::Concrete(id) | TypeRef::Instantiated { base: id, .. }
                if *id != TypeId::PTR && *id != TypeId::RESERVED => {
                match self.type_by_id(*id) {
                    Some(declaration) => !declaration.is_transparent_wrapper
                        && matches!(declaration.kind, TypeKind::Record | TypeKind::Sum
                            | TypeKind::Tuple | TypeKind::Array | TypeKind::Primitive | TypeKind::Unit),
                    None => id.is_builtin(),
                }
            }
            TypeRef::Tuple(_) | TypeRef::Array { .. } => true,
            _ => false,
        }
    }
}

/// Match the exact declaration's parameter pattern, retaining reference layers
/// and repeated-variable agreement. No nominal-roster positional substitution.
fn bind_copy_parameter(
    formal: &TypeRef, actual: &TypeRef, substitution: &mut crate::mono::TypeSubstitution, depth: usize,
) -> bool {
    if depth >= 64 { return false; }
    match (formal, actual) {
        (TypeRef::Generic(id), actual) => match substitution.get(*id) {
            Some(bound) => bound == actual,
            None => { substitution.bind(*id, actual.clone()); true },
        },
        (TypeRef::Instantiated { base: left, args: a }, TypeRef::Instantiated { base: right, args: b }) =>
            left == right && a.len() == b.len() && a.iter().zip(b).all(|(a, b)| bind_copy_parameter(a, b, substitution, depth + 1)),
        (TypeRef::Reference { inner: a, mutability: am, tier: at }, TypeRef::Reference { inner: b, mutability: bm, tier: bt }) =>
            am == bm && at == bt && bind_copy_parameter(a, b, substitution, depth + 1),
        (TypeRef::Tuple(a), TypeRef::Tuple(b)) => a.len() == b.len()
            && a.iter().zip(b).all(|(a, b)| bind_copy_parameter(a, b, substitution, depth + 1)),
        (TypeRef::Array { element: a, length: al }, TypeRef::Array { element: b, length: bl }) =>
            al == bl && bind_copy_parameter(a, b, substitution, depth + 1),
        (TypeRef::Slice(a), TypeRef::Slice(b)) => bind_copy_parameter(a, b, substitution, depth + 1),
        (TypeRef::Concrete(a), TypeRef::Concrete(b)) => a == b,
        (TypeRef::ConstValue(a), TypeRef::ConstValue(b)) => a == b,
        _ => false,
    }
}
