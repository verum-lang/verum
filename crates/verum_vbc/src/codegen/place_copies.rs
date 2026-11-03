//! Semantic authority for explicit dereference copies at existing value boundaries.
//! ABI layouts and missing declaration facts do not authorize duplication.
use super::VbcCodegen;
use crate::{Instruction, instruction::Reg, types::TypeRef};
use verum_ast::{Expr, ExprKind, UnOp};
use verum_common::Maybe;

/// A branch join retains a declaration only if every returning arm agrees.
/// Divergence contributes no value; an unknown arm never borrows another arm's
/// ownership authority.
pub(super) fn join_result_types(left: Maybe<TypeRef>, right: Maybe<TypeRef>) -> Maybe<TypeRef> {
    let (left, right) = (left?, right?);
    let never = TypeRef::Concrete(crate::types::TypeId::NEVER);
    if left == never { Some(right) }
    else if right == never || left == right { Some(left) }
    else { None }
}

impl VbcCodegen {
    /// A bound name's display and emission queries share its active declaration.
    /// A nested pattern need not publish a second, textual type hint.
    pub(super) fn declared_binding_type_name(&self, expression: &Expr) -> Maybe<verum_common::Text> {
        if !matches!(Self::normalized_place(expression).kind, ExprKind::Path(_)) {
            return None;
        }
        let ty = self.expand_declared_alias(self.declared_expression_result(expression, None, 0, 0)?)?;
        self.scoped_callable_type_name(&ty, Some(&self.ctx.current_generic_param_ids))
    }

    /// Resolve a carried declaration in the active callable's binder scope.
    /// Undeclared names never become positional generic placeholders.
    pub(super) fn semantic_type_in_current_scope(&self, name: &str) -> Maybe<TypeRef> {
        let ast = verum_fast_parser::Parser::new(name).parse_type().ok()?;
        self.source_type_in_current_scope(&ast)
    }

    pub(super) fn source_type_in_current_scope(&self, ast: &verum_ast::Type) -> Maybe<TypeRef> {
        let ids: Vec<_> = self.ctx.current_generic_param_ids.values()
            .copied().map(crate::types::TypeParamId).collect();
        self.semantic_formal_type(ast, &self.ctx.current_generic_param_ids, &ids)
    }

    pub(super) fn declared_expression_result(
        &self, expr: &Expr, reg: Maybe<Reg>, start: usize, end: usize,
    ) -> Maybe<TypeRef> {
        self.copy_source_type_inner(expr, reg.map(|reg| (reg, start, end)), 0)
    }

    pub(super) fn declared_block_result(
        &self, block: &verum_ast::Block, reg: Maybe<Reg>, start: usize, end: usize,
    ) -> Maybe<TypeRef> {
        if let Some(expr) = &block.expr {
            return self.declared_expression_result(expr, reg, start, end);
        }
        // A semicolon discards a value but cannot make a diverging edge return.
        let never = TypeRef::Concrete(crate::types::TypeId::NEVER);
        if let Some(verum_ast::Stmt { kind: verum_ast::StmtKind::Expr { expr, .. }, .. }) = block.stmts.last()
            && self.declared_expression_result(expr, reg, start, end) == Some(never.clone()) {
            return Some(never);
        }
        Some(TypeRef::Concrete(crate::types::TypeId::UNIT))
    }

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
        let expr = Self::normalized_place(expr);
        let nested = |value, emitted| self.copy_source_type_inner(value, emitted, depth + 1);
        match &expr.kind {
            ExprKind::Return(_) | ExprKind::Break { .. } | ExprKind::Continue { .. } =>
                Some(TypeRef::Concrete(crate::types::TypeId::NEVER)),
            ExprKind::Block(block) => self.ctx.declared_result_types.get(&block.span).cloned().flatten(),
            ExprKind::Match { .. } | ExprKind::Select { .. } | ExprKind::Await(_) | ExprKind::Try(_) => self.ctx.declared_result_types.get(&expr.span).cloned().flatten(),
            ExprKind::If { then_branch, else_branch, .. } => join_result_types(
                self.ctx.declared_result_types.get(&then_branch.span).cloned().flatten(),
                else_branch.as_ref().map_or(Some(TypeRef::Concrete(crate::types::TypeId::UNIT)), |branch| nested(branch, None)),
            ),
            ExprKind::Closure { .. } => self.compiled_callable_signature(expr).cloned(),
            ExprKind::Path(path) => {
                let name = match path.segments.as_slice() {
                    [verum_ast::PathSegment::Name(name)] => name.name.as_str(),
                    [verum_ast::PathSegment::SelfValue] => "self",
                    _ => return None,
                };
                let binding = self.ctx.lookup_var(name)?;
                if binding.is_cell { return None; }
                let fact = self.ctx.registers.value_uses.active.get(&binding.reg)?;
                let ty = fact.declaration_type.as_ref()?;
                // A borrowed pattern carries a reference declaration, never
                // ownership of the payload. Retain that borrow for projections.
                if binding.is_pattern_alias && !matches!(ty, TypeRef::Reference { .. }) { return None; }
                (fact.id == binding.binding_id).then(|| ty.clone())
            }
            ExprKind::Record { path, .. } => {
                if let Some(declared) = self.constructor_declaration(expr, None, None, depth + 1) {
                    return Some(declared);
                }
                let ast = verum_ast::Type::new(verum_ast::ty::TypeKind::Path(path.clone()), expr.span);
                self.semantic_formal_type(&ast, &self.ctx.current_generic_param_ids, &[])
            }
            ExprKind::Field { expr: receiver, field } => {
                // The receiver's semantic declaration owns this projection.
                // A layout offset or a same-named field on another type cannot
                // supply its ownership or replace missing archive evidence.
                let mut receiver = nested(receiver, None)?;
                while let TypeRef::Reference { inner, .. } = receiver { receiver = *inner; }
                let (id, arguments) = match receiver {
                    TypeRef::Concrete(id) => (id, Vec::new()),
                    TypeRef::Instantiated { base, args } => (base, args),
                    _ => return None,
                };
                let declaration = self.type_by_id(id)?;
                if declaration.kind != crate::types::TypeKind::Record
                    || declaration.type_params.len() != arguments.len() { return None; }
                let member = declaration.fields.iter().find(|member|
                    self.ctx.strings.get(member.name.0 as usize)
                        .is_some_and(|name| name == field.name.as_str()))?;
                let ty = member.declaration_type.as_ref()?;
                let ids: verum_common::List<_> = declaration.type_params.iter().map(|p| p.id).collect();
                if !super::formal_parameters::resolved(ty, &ids, 0) { return None; }
                crate::mono::TypeSubstitution::new(&declaration.type_params, &arguments).apply(ty).ok()
            }
            ExprKind::Index { expr: base, index } => self.indexed_element_declaration(base, index),
            ExprKind::Unary { op: UnOp::Deref, expr: inner } => {
                let input = emitted.and_then(|(reg, start, end)| {
                    let (index, op) = self.copy_result_producer(reg, start, end)?;
                    if let Instruction::Deref { ref_reg, .. } = op {
                        Some((*ref_reg, start, index))
                    } else { None }
                });
                match nested(inner, input)? {
                    TypeRef::Reference { inner, .. } => Some(*inner),
                    owner => self.declared_protocol_member(
                        &owner, verum_common::well_known_types::WellKnownProtocol::Deref, "Target",
                    ),
                }
            }
            ExprKind::Unary { op, expr: inner } if matches!(op, UnOp::Ref | UnOp::RefMut)
                && let ExprKind::Index { expr: base, index } = &inner.kind =>
                self.indexed_borrow_declaration(base, index, *op == UnOp::RefMut),
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
            ExprKind::MethodCall { receiver, method, type_args, args } => {
                let selected = emitted.and_then(|(reg, start, end)|
                    self.copy_selected_call_result(reg, start, end, type_args, args, depth));
                if selected.is_some() { return selected; }
                let owner = self.extract_expr_type_name(receiver)?;
                if let Some(result) = self.dynamic_call_result(&owner, method.name.as_str(), args.len(), type_args) {
                    return result;
                }
                self.declared_method_result_type(&owner, method.name.as_str(), type_args)
            }
            ExprKind::Call { type_args, args, .. } => {
                if let Some(result) = self.lexical_call_result(expr) {
                    return result;
                }
                let (reg, start, end) = emitted?;
                self.copy_selected_call_result(reg, start, end, type_args, args, depth)
            }
            // Raw pointers, dynamic calls and unresolved
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

    fn copy_selected_call_result(
        &self, reg: Reg, start: usize, end: usize, type_args: &[verum_ast::ty::GenericArg],
        source_args: &[Expr], depth: usize,
    ) -> Maybe<TypeRef> {
        let (index, op) = self.copy_result_producer(reg, start, end)?;
        let (id, args) = match op {
            Instruction::Call { func_id, args, .. } | Instruction::CallG { func_id, args, .. } => (*func_id, args),
            _ => return None,
        };
        let info = self.ctx.lookup_function_by_id(crate::module::FunctionId(id))?;
        let TypeRef::Function { params, return_type, .. } = info.callable_signature.as_ref()? else { return None; };
        if params.len() != usize::from(args.count) { return None; }
        let mut substitution = crate::mono::TypeSubstitution::empty();
        self.bind_source_type_arguments(&info.explicit_type_param_ids, type_args, &mut substitution)?;
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
            if !self.bind_source_declaration(formal, actual, &mut substitution) { return None; }
        }
        // Layout-only archive returns and unbound slots remain unknown. In
        // particular, CallG's ABI witnesses cannot reconstruct reference layers.
        if super::constructor_types::fully_bound(return_type, &substitution) {
            return substitution.apply(return_type).ok();
        }
        // Temporaries such as literal arguments have no lexical binding. For a
        // direct call, independently instantiate the selected source signature
        // from its operands and require agreement with the emitted call.
        let Instruction::CallG { type_args: witnesses, .. } = op else { return None; };
        if info.type_param_ids.len() != witnesses.len() || params.len() != source_args.len() {
            return None;
        }
        let result = self.source_call_result(info, type_args, source_args, None, depth)?;
        for (id, witness) in info.type_param_ids.iter().zip(witnesses) {
            if let Some(bound) = substitution.get(*id) {
                if bound != witness { return None; }
            } else {
                substitution.bind(*id, witness.clone());
            }
        }
        (super::constructor_types::fully_bound(return_type, &substitution)
            && result == substitution.apply(return_type).ok()?).then_some(result)
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
