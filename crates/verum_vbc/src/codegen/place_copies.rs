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
        if let Some(result) = self.type_property_result(expr) { return Some(result); }
        let nested = |value, emitted| self.copy_source_type_inner(value, emitted, depth + 1);
        match &expr.kind {
            ExprKind::Literal(verum_ast::literal::Literal {
                kind: verum_ast::literal::LiteralKind::ByteString(_),
                ..
            }) => Some(TypeRef::reference(
                TypeRef::Slice(verum_common::Heap::new(TypeRef::Concrete(crate::types::TypeId::U8))),
                Mutability::Immutable,
                CbgrTier::Tier0,
            )),
            ExprKind::Return(_) | ExprKind::Break { .. } | ExprKind::Continue { .. } =>
                Some(TypeRef::Concrete(crate::types::TypeId::NEVER)),
            ExprKind::Block(block) | ExprKind::Unsafe(block) =>
                self.ctx.declared_result_types.get(&block.span).cloned().flatten(),
            ExprKind::Cast { ty, .. } => self.source_type_in_current_scope(ty),
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
                let Some(binding) = self.ctx.lookup_var(name) else {
                    // Reading a const/meta parameter produces a value of its
                    // declared carrier. This also establishes index projections.
                    return self.current_source_substitution.get_value(name)
                        .and_then(|(witness, domain)| witness.semantic.then(|| domain.clone()));
                };
                if binding.is_cell { return None; }
                let fact = self.ctx.registers.value_uses.active.get(&binding.reg)?;
                let ty = fact.declaration_type.as_ref()?;
                // A borrowed pattern carries a reference declaration, never
                // ownership of the payload. Retain that borrow for projections.
                if binding.is_pattern_alias && !matches!(ty, TypeRef::Reference { .. }) { return None; }
                (fact.id == binding.binding_id).then(|| ty.clone())
            }
            ExprKind::Binary { op, left, right } => {
                let (protocol, member) = op.standard_protocol_member()?;
                let left = nested(left, None)?;
                let right = nested(right, None)?;
                self.bounded_binary_output(&left, protocol, member, &right)
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
                self.projected_field_declaration(nested(receiver, None)?, &field.name)
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
                if args.len() == 1 && type_args.is_empty()
                    && matches!(method.name.as_str(), "offset" | "add" | "sub")
                    && let Some(pointer @ TypeRef::Reference { tier: CbgrTier::Tier2, .. }) = nested(receiver, None)
                {
                    return Some(pointer);
                }
                if let Some(witness) = self.type_parameter_static_receiver(receiver).ok()? {
                    if let TypeRef::Generic(id) = witness {
                        return type_args.is_empty().then(|| self.bounded_static_output(id, method.name.as_str(), args.len())).flatten();
                    }
                    let owner = self.resolve_declared_associated_types(&witness)?;
                    let name = self.declaration_type_name(&owner, depth + 1)?;
                    return self.declared_method_result_type(&name, method.name.as_str(), type_args);
                }
                let selected = emitted.and_then(|(reg, start, end)|
                    self.copy_selected_call_result(reg, start, end, type_args, args, depth));
                if selected.is_some() { return selected; }
                if emitted.is_some() && let Some(result) = self.source_call_declaration(expr, None, depth + 1) {
                    return Some(result);
                }
                let owner = self.extract_expr_type_name(receiver)?;
                if let Some(result) = self.dynamic_call_result(&owner, method.name.as_str(), args.len(), type_args) {
                    return result;
                }
                self.declared_method_result_type(&owner, method.name.as_str(), type_args)
            }
            ExprKind::Call { type_args, args, .. } => {
                if let Some(result) = self.ctx.declared_result_types.get(&expr.span) {
                    return result.clone();
                }
                if let Some(result) = self.lexical_call_result(expr) {
                    return result;
                }
                if emitted.is_none() {
                    return self.source_call_declaration(expr, None, depth + 1);
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
        self.bind_source_type_arguments(info, type_args, &mut substitution)?;
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
            self.bind_declared_operand(info, offset, formal, actual, &mut substitution)?;
        }
        if let Instruction::CallG { type_args: witnesses, .. } = op
            && let Some(roster) = self.callable_generic_roster(info)
            && roster.iter().any(|parameter| parameter.value_parameter().is_some())
        {
            if info.type_param_ids.len() != witnesses.len() { return None; }
            let arguments = info.type_param_ids.iter().zip(witnesses)
                .map(|(id, witness)| (id.0, witness.clone())).collect();
            self.validate_callable_value_bindings(info, &arguments).ok()?;
            // A value witness is the exact scalar consumed by this selected
            // call. Type-role ABI witnesses still cannot recover references or
            // supply missing source declarations.
            for parameter in roster.iter().filter(|parameter| parameter.value_parameter().is_some()) {
                let id = parameter.witness_slot?;
                if !self.bind_source_declaration(
                    &TypeRef::Generic(id), arguments.get(&id.0)?, &mut substitution,
                ) { return None; }
            }
        }
        // A representation-preserving cast can reuse an operand register.
        // Its old binding cannot contradict the actual selected call's
        // witness, even when it appears to bind every result parameter.
        let witnesses = match op {
            Instruction::CallG { type_args, .. } => {
                if info.type_param_ids.len() != type_args.len() { return None; }
                for (id, witness) in info.type_param_ids.iter().zip(type_args) {
                    if substitution.get(*id).is_some_and(|bound| bound != witness) { return None; }
                }
                Some(type_args)
            }
            _ => None,
        };
        // Layout-only archive returns and unbound slots remain unknown. In
        // particular, CallG's ABI witnesses cannot reconstruct reference layers.
        if super::constructor_types::fully_bound(return_type, &substitution) {
            return substitution.apply(return_type).ok();
        }
        // Temporaries such as literal arguments have no lexical binding. For a
        // direct call, independently instantiate the selected source signature
        // from its operands and require agreement with the emitted call.
        let witnesses = witnesses?;
        if params.len() != source_args.len() { return None; }
        let result = self.source_call_result(info, type_args, source_args, None, depth)?;
        for (id, witness) in info.type_param_ids.iter().zip(witnesses) {
            substitution.bind(*id, witness.clone());
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
