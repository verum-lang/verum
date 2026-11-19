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

    /// Receiver syntax names the same active binding as a regular local. Its
    /// distinct AST segment must not turn an owned place into a temporary.
    pub(super) fn named_place_binding(expr: &Expr) -> Maybe<&str> {
        let ExprKind::Path(path) = &Self::normalized_place(expr).kind else { return None; };
        match path.segments.as_slice() {
            [verum_ast::PathSegment::Name(name)] => Some(name.name.as_str()),
            [verum_ast::PathSegment::SelfValue] => Some("self"),
            _ => None,
        }
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
        let declared = self.copy_source_type_unrefined(expr, emitted, depth)?;
        self.body_declaration_type(&declared)
    }

    fn copy_source_type_unrefined(
        &self, expr: &Expr, emitted: Maybe<(Reg, usize, usize)>, depth: usize,
    ) -> Maybe<TypeRef> {
        use crate::types::{CbgrTier, Mutability};
        if depth >= 64 { return None; }
        let expr = Self::normalized_place(expr);
        if let Some(result) = self.type_property_result(expr) { return Some(result); }
        if let Some(name) = Self::literal_default_type_name(expr)
            && let Some(id) = crate::types::TypeId::from_well_known_scalar_name(&name) {
            return Some(TypeRef::Concrete(id.declared_scalar_identity(&name)));
        }
        // A selected intrinsic has the same semantic result at every syntax
        // boundary. Inspect its actual producer, never the method's spelling or
        // a register's machine representation. This also covers formatted Text.
        if let Some((reg, start, end)) = emitted
            && let Some((_, producer)) = self.copy_result_producer(reg, start, end)
        {
            if let Some((_, result)) = producer.intrinsic_result_type() {
                return Some(result);
            }
            if let Instruction::MakeVariantTyped { type_id, tag, field_count: 0, .. } = producer {
                let expected = self.ctx.declared_result_types.get(&expr.span)
                    .and_then(Option::as_ref);
                return self.selected_variant_declaration(
                    crate::types::TypeId(*type_id), *tag, &[], expected);
            }
        }
        // Both call syntaxes retain the selected producer's declaration.
        // An intrinsic's actual result above is stronger than an unresolved
        // source-method entry; physical register width never supplies a type.
        if matches!(expr.kind, ExprKind::Call { .. } | ExprKind::MethodCall { .. })
            && let Some(result) = self.ctx.declared_result_types.get(&expr.span) {
            return result.clone();
        }
        let nested = |value, emitted| self.copy_source_type_inner(value, emitted, depth + 1);
        match &expr.kind {
            ExprKind::InterpolatedString { .. }
            | ExprKind::Literal(verum_ast::Literal {
                kind: verum_ast::LiteralKind::InterpolatedString(_), ..
            }) => Some(TypeRef::Concrete(crate::types::TypeId::TEXT)),
            ExprKind::Literal(verum_ast::literal::Literal {
                kind: verum_ast::literal::LiteralKind::ByteString(_),
                ..
            }) => Some(TypeRef::reference(
                TypeRef::Slice(verum_common::Heap::new(TypeRef::Concrete(crate::types::TypeId::U8))),
                Mutability::Immutable,
                CbgrTier::Tier0,
            )),
            ExprKind::Array(verum_ast::ArrayExpr::List(elements)) => {
                let element = nested(elements.first()?, None)?;
                if !elements.iter().skip(1).all(|value| nested(value, None).as_ref() == Some(&element)) {
                    return None;
                }
                Some(TypeRef::Array { element: Box::new(element), length: elements.len() as u64 })
            }
            ExprKind::Array(verum_ast::ArrayExpr::Repeat { value, count }) => {
                let length = u64::try_from(self.const_eval_i64(count).ok()??).ok()?;
                Some(TypeRef::Array { element: Box::new(nested(value, None)?), length })
            }
            ExprKind::Tuple(elements) => {
                if let Some(declared) = self.ctx.declared_result_types.get(&expr.span) {
                    return declared.clone();
                }
                if elements.is_empty() {
                    return Some(TypeRef::Concrete(crate::types::TypeId::UNIT));
                }
                Some(TypeRef::Tuple(elements.iter()
                    .map(|element| nested(element, None)).collect::<Option<_>>()?))
            }
            ExprKind::Return(_) | ExprKind::Break { .. } | ExprKind::Continue { .. } =>
                Some(TypeRef::Concrete(crate::types::TypeId::NEVER)),
            ExprKind::Block(block) | ExprKind::Unsafe(block) =>
                self.ctx.declared_result_types.get(&block.span).cloned().flatten(),
            ExprKind::Cast { ty, .. } => self.source_type_in_current_scope(ty),
            ExprKind::Match { .. } | ExprKind::Select { .. } | ExprKind::Await(_) | ExprKind::Try(_) => self.ctx.declared_result_types.get(&expr.span).cloned().flatten(),
            ExprKind::If { then_branch, else_branch, .. } => {
                let left = self.ctx.declared_result_types.get(&then_branch.span).cloned().flatten();
                let right = else_branch.as_ref().map_or(
                    Some(TypeRef::Concrete(crate::types::TypeId::UNIT)),
                    |branch| nested(branch, None),
                );
                match (left, right) {
                    (Some(known), None) => self.bind_nullary_branch(else_branch.as_deref()?, known),
                    (None, Some(known)) => self.bind_nullary_branch(then_branch.expr.as_deref()?, known),
                    (left, right) => join_result_types(left, right),
                }
            }
            ExprKind::Closure { .. } => self.compiled_callable_signature(expr).cloned(),
            ExprKind::Path(_) => {
                if let Some(function) = self.selected_function_value(expr).ok()? {
                    return self.function_value_signature(function, self.ctx.expected_value_type.as_ref());
                }
                let name = Self::named_place_binding(expr)?;
                let Some(binding) = self.ctx.lookup_var(name) else {
                    // Reading a const/meta parameter produces a value of its
                    // declared carrier. This also establishes index projections.
                    return self.current_source_substitution.get_value(name)
                        .and_then(|(witness, domain)| witness.semantic.then(|| domain.clone()));
                };
                self.declared_named_binding_type(name, binding.reg)
            }
            ExprKind::Is { .. } => Some(TypeRef::Concrete(crate::types::TypeId::BOOL)),
            ExprKind::Binary { op, left, right } => {
                use verum_ast::expr::BinOpCategory;
                let boolean = TypeRef::Concrete(crate::types::TypeId::BOOL);
                if op.is_comparison() { return Some(boolean); }
                let left = nested(left, None)?;
                let right = nested(right, None)?;
                let primitive_left = self.expand_declared_alias(left.clone())?;
                let primitive_right = self.expand_declared_alias(right.clone())?;
                if primitive_left == primitive_right {
                    if op.category() == BinOpCategory::Logical && primitive_left == boolean {
                        return Some(boolean);
                    }
                    if let TypeRef::Concrete(id) = primitive_left
                        && ((op.category() == BinOpCategory::Arithmetic && id.is_numeric())
                            || (op.category() == BinOpCategory::Bitwise && id.is_integer())) {
                        return Some(TypeRef::Concrete(id));
                    }
                }
                let (protocol, member) = op.standard_protocol_member()?;
                self.bounded_binary_output(&left, protocol, member, &right)
            }
            ExprKind::Unary { op: UnOp::Not | UnOp::Neg | UnOp::BitNot, expr: operand } => {
                let operand = self.expand_declared_alias(nested(operand, None)?)?;
                let TypeRef::Concrete(id) = operand else { return None; };
                let allowed = match &expr.kind {
                    ExprKind::Unary { op: UnOp::Not, .. } => id == crate::types::TypeId::BOOL,
                    ExprKind::Unary { op: UnOp::Neg, .. } => id.is_numeric(),
                    ExprKind::Unary { op: UnOp::BitNot, .. } => id.is_integer(),
                    _ => false,
                };
                allowed.then_some(TypeRef::Concrete(id))
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
            ExprKind::TupleIndex { expr: receiver, index } => {
                let mut owner = nested(receiver, None)?;
                for _ in depth..64 {
                    owner = self.expand_declared_alias(owner)?;
                    match owner {
                        TypeRef::Reference { inner, .. } => owner = *inner,
                        TypeRef::Tuple(parts) => return parts.get(*index as usize).cloned(),
                        _ => return None,
                    }
                }
                None
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
                    if type_args.is_empty()
                        && let Some(result) = self.bounded_static_output(&witness, method.name.as_str(), args.len()) {
                        return Some(result);
                    }
                    if matches!(witness, TypeRef::Generic(_)) { return None; }
                    let owner = self.resolve_declared_associated_types(&witness)?;
                    let name = self.declaration_type_name(&owner, depth + 1)?;
                    return self.declared_method_result_type(&name, method.name.as_str(), type_args);
                }
                let selected = emitted.and_then(|(reg, start, end)|
                    self.copy_selected_call_result(reg, start, end, type_args, args, depth));
                if selected.is_some() { return selected; }
                if let Some(result) = self.source_call_declaration(expr, None, depth + 1) {
                    return Some(result);
                }
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
    /// Retiring a different operand preserves this register. Branches and
    /// unknown writes still end the proof instead of guessing a prior producer.
    pub(super) fn copy_result_producer(&self, mut reg: Reg, start: usize, end: usize) -> Maybe<(usize, &Instruction)> {
        let instructions = self.ctx.instructions.get(start..end)?;
        // Entry at the first evaluated instruction still executes the producer.
        // A jump into the expression or past its last instruction can bypass it.
        if start == end || self.ctx.has_label_in_instruction_range(start + 1, end) { return None; }
        for (offset, instruction) in instructions.iter().enumerate().rev().take(128) {
            match instruction {
                Instruction::Mov { dst, src } if *dst == reg => reg = *src,
                Instruction::ChkRef { ref_reg } if *ref_reg == reg => {},
                Instruction::DropTyped { src, .. } | Instruction::DropRef { src }
                    if *src != reg => {},
                _ if instruction.intrinsic_result_type().is_some_and(|(dst, _)| dst == reg) =>
                    return Some((start + offset, instruction)),
                Instruction::Call { dst, .. } | Instruction::CallG { dst, .. }
                    | Instruction::Deref { dst, .. }
                    | Instruction::MakeVariantTyped { dst, .. }
                    if *dst == reg => return Some((start + offset, instruction)),
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

    /// Fill otherwise unconstrained parameters of a selected empty sum arm
    /// from the other result. Every returning edge must agree, including
    /// nested conditionals. An arbitrary unknown value supplies no evidence.
    fn bind_nullary_branch(&self, branch: &Expr, expected: TypeRef) -> Maybe<TypeRef> {
        let never = TypeRef::Concrete(crate::types::TypeId::NEVER);
        if expected == never { return None; }
        let mut pending = verum_common::List::from_iter([branch]);
        let mut remaining = 4096_usize;
        while let Some(value) = pending.pop() {
            remaining = remaining.checked_sub(1)?;
            let value = Self::normalized_place(value);
            if let Some(Some(ty)) = self.ctx.declared_result_types.get(&value.span) {
                if ty != &expected && ty != &never { return None; }
                continue;
            }
            if let Some(&(owner, tag)) = self.ctx.nullary_result_variants.get(&value.span) {
                if self.selected_variant_declaration(owner, tag, &[], Some(&expected))? != expected {
                    return None;
                }
                continue;
            }
            match &value.kind {
                ExprKind::Block(block) | ExprKind::Unsafe(block) => pending.push(block.expr.as_deref()?),
                ExprKind::If { then_branch, else_branch, .. } => {
                    pending.push(then_branch.expr.as_deref()?);
                    pending.push(else_branch.as_deref()?);
                }
                _ => return None,
            }
        }
        Some(expected)
    }

    fn copy_active_declaration(&self, reg: Reg) -> Maybe<TypeRef> {
        let fact = self.ctx.registers.value_uses.active.get(&reg)?;
        self.ctx.registers.all_variable_names().iter().any(|name| {
            self.ctx.lookup_var(name).is_some_and(|binding| binding.reg == reg
                && binding.binding_id == fact.id && !binding.is_cell
                && binding.pattern_alias != Some(super::registers::PatternAlias::Explicit))
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
        self.bind_source_type_arguments(info, type_args, &mut substitution).ok()?;
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
        let result = self.source_call_result(info, None, type_args, source_args, None, depth)?;
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
                    Some(declaration) => matches!(declaration.kind, TypeKind::Record | TypeKind::Sum
                            | TypeKind::Tuple | TypeKind::Array | TypeKind::Primitive | TypeKind::Unit),
                    None => id.is_builtin(),
                }
            }
            // A callable read from a field or element is still a place. Its
            // declared signature selects the copy operation; the actual
            // environment producer supplies the lifecycle permission.
            TypeRef::Tuple(_) | TypeRef::Array { .. } | TypeRef::Function { .. } => true,
            _ => false,
        }
    }
}
