//! Constructor declarations bind owner parameters from fields and contextual
//! results. The owner's binder roster, never a same-spelled caller parameter,
//! determines substitution order.
use super::*;
use crate::{
    mono::TypeSubstitution,
    types::{Mutability, TypeKind, TypeRef},
};
use std::borrow::Cow;
use verum_ast::{Expr, ExprKind};

impl VbcCodegen {
    pub(super) fn declaration_type_name(&self, ty: &TypeRef, depth: usize) -> Option<String> {
        if depth >= 64 {
            return None;
        }
        match ty {
            TypeRef::RecordFieldProjection { .. } => None,
            TypeRef::Generic(id) => self
                .ctx
                .current_generic_param_ids
                .iter()
                .find_map(|(name, actual)| (*actual == id.0).then(|| name.clone())),
            TypeRef::Instantiated { base, args } => {
                let owner = self.render_field_type_ref(&TypeRef::Concrete(*base), true)?;
                let args = args
                    .iter()
                    .map(|arg| self.declaration_type_name(arg, depth + 1))
                    .collect::<Option<Vec<_>>>()?;
                Some(format!("{owner}<{}>", args.join(", ")))
            }
            TypeRef::Reference { inner, mutability, tier } => {
                use crate::types::{CbgrTier, Mutability};
                let tier = match tier {
                    CbgrTier::Tier0 => "",
                    CbgrTier::Tier1 => "checked ",
                    CbgrTier::Tier2 => "unsafe ",
                };
                let access = if *mutability == Mutability::Mutable { "mut " } else { "" };
                Some(format!("&{tier}{access}{}", self.declaration_type_name(inner, depth + 1)?))
            }
            TypeRef::Tuple(parts) => Some(format!(
                "({})",
                parts
                    .iter()
                    .map(|part| self.declaration_type_name(part, depth + 1))
                    .collect::<Option<Vec<_>>>()?
                    .join(", ")
            )),
            _ => self.render_field_type_ref(ty, true),
        }
    }

    /// Type of a field read from a selected sum arm, instantiated in the
    /// receiver's own telescope. This describes the emitted projection.
    pub(super) fn projected_variant_field(
        &self,
        ty: &TypeRef,
        tag: u32,
        field: u32,
    ) -> Option<TypeRef> {
        let (id, args) = match ty {
            TypeRef::Concrete(id) => (*id, &[][..]),
            TypeRef::Instantiated { base, args } => (*base, args.as_slice()),
            _ => return None,
        };
        let owner = self.type_by_id(id)?;
        if owner.kind != crate::types::TypeKind::Sum || owner.type_params.len() != args.len() {
            return None;
        }
        let variant = owner.variants.iter().find(|v| v.tag == tag)?;
        let member = variant
            .fields
            .get(field as usize)?
            .declaration_type
            .as_ref()?;
        let ids: verum_common::List<_> = owner.type_params.iter().map(|param| param.id).collect();
        if !super::formal_parameters::resolved(member, &ids, 0) {
            return None;
        }
        let result = TypeSubstitution::new(&owner.type_params, args)
            .apply(member)
            .ok()?;
        self.declared_type_in_scope(&result).then_some(result)
    }

    pub(super) fn future_implementation_for_type(
        &self,
        owner: crate::types::TypeId,
    ) -> Option<&crate::types::ProtocolImpl> {
        self.type_by_id(owner)?
            .protocols
            .iter()
            .find(|implementation| {
                self.type_by_id(crate::types::TypeId(implementation.protocol.0))
                    .and_then(|protocol| self.ctx.strings.get(protocol.name.0 as usize))
                    .is_some_and(|name| {
                        verum_common::well_known_types::WellKnownProtocol::Future
                            .matches(name.rsplit('.').next().unwrap_or(name))
                    })
            })
    }
    /// Forward interfaces and compiled declarations can both describe one
    /// implementation. Only a declared, unambiguous member is evidence.
    pub(super) fn associated_binding<'a>(
        &'a self,
        implementations: impl Iterator<Item = &'a crate::types::ProtocolImpl>,
        member: &str,
    ) -> Option<&'a TypeRef> {
        let mut bindings = implementations.flat_map(|implementation| &implementation.associated_types)
            .filter(|(name, _)| self.ctx.strings.get(name.0 as usize)
                .is_some_and(|name| name == member))
            .map(|(_, ty)| ty);
        let first = bindings.next()?;
        bindings.all(|other| other == first).then_some(first)
    }

    /// Instantiate an associated type from the selected canonical protocol on
    /// this exact nominal owner, retaining any type arguments of the caller.
    pub(super) fn declared_protocol_member(
        &self, ty: &TypeRef, protocol: verum_common::well_known_types::WellKnownProtocol, member: &str,
    ) -> Option<TypeRef> {
        self.selected_implementation_member(ty, protocol, member)
    }

    fn future_output_declaration(&self, owner: crate::types::TypeId) -> Option<&TypeRef> {
        let protocol = self.future_implementation_for_type(owner)?.protocol;
        self.associated_binding(
            self.type_by_id(owner)?.protocols.iter().filter(|implementation| implementation.protocol == protocol),
            "Output",
        )
    }

    pub(super) fn declared_type_in_scope(&self, ty: &TypeRef) -> bool {
        let Ok(ids) = self.current_callable_param_ids() else { return false; };
        super::formal_parameters::resolved(ty, &ids, 0)
    }

    /// Match a source declaration after validating the actual caller's scope.
    pub(super) fn bind_source_declaration(
        &self,
        pattern: &TypeRef,
        actual: &TypeRef,
        bindings: &mut TypeSubstitution,
    ) -> bool {
        if !self.declared_type_in_scope(actual) { return false; }
        if fully_bound(pattern, bindings) {
            // Only a complete callee substitution moves this pattern into
            // the caller's scope. There its projections and body premises can
            // be normalized without interpreting a caller ID as a callee slot.
            let Some(expected) = bindings.apply(pattern).ok()
                .and_then(|ty| self.body_declaration_type(&ty)) else { return false; };
            let Some(actual) = self.body_declaration_type(actual) else { return false; };
            if !self.declared_type_in_scope(&expected) { return false; }
            let mut rigid = TypeSubstitution::empty();
            let Ok(ids) = self.current_callable_param_ids() else { return false; };
            for id in ids {
                rigid.bind(id, TypeRef::Generic(id));
            }
            // Reuse structural compatibility (including the two declared
            // array carriers), with every caller parameter fixed to itself.
            return crate::mono::match_declared_types(&expected, &actual, &mut rigid);
        }
        let mut selected = bindings.clone();
        if !crate::mono::match_declared_types(pattern, actual, &mut selected)
            || !fully_bound(pattern, &selected)
        { return false; }
        *bindings = selected;
        true
    }

    pub(super) fn constructor_declaration(
        &self,
        expr: &Expr,
        expected: Option<&TypeRef>,
        output: Option<&TypeRef>,
        depth: usize,
    ) -> Option<TypeRef> {
        if depth >= 64 {
            return None;
        }
        let expr = Self::normalized_place(expr);
        if let ExprKind::Field { expr: receiver, .. } | ExprKind::MethodCall { receiver, .. } = &expr.kind
            && Self::named_place_binding(receiver).is_some_and(|name| self.is_value_receiver_binding(name)) {
            return None;
        }
        let (owner, arguments) = match &expr.kind {
            ExprKind::Record {
                path,
                fields,
                base: None,
            } => {
                let (owner, declared_fields) = self.named_constructor_fields(path, expected)?;
                if declared_fields.len() != fields.len() {
                    return None;
                }
                let mut arguments = Vec::new();
                for field in declared_fields {
                    let name = self.ctx.strings.get(field.name.0 as usize)?;
                    let value = fields
                        .iter()
                        .find(|value| value.name.name.as_str() == name.as_str())?;
                    let value = value.value.as_ref().map(Cow::Borrowed).unwrap_or_else(|| {
                        Cow::Owned(Expr::path(verum_ast::Path::single(value.name.clone())))
                    });
                    arguments.push((field.declaration_type.as_ref()?, value));
                }
                (owner, arguments)
            }
            ExprKind::MethodCall {
                receiver,
                method,
                args,
                type_args,
            } if type_args.is_empty() => {
                let ExprKind::Path(path) = &receiver.kind else {
                    return None;
                };
                let name = path.to_string();
                let id = self.semantic_type_in_current_scope(&name)?.base_type_id()?;
                let declaration = self.type_by_id(id)?;
                if declaration.kind != TypeKind::Sum {
                    return None;
                }
                let variant = declaration.variants.iter().find(|variant| {
                    self.ctx
                        .strings
                        .get(variant.name.0 as usize)
                        .is_some_and(|name| name == method.name.as_str())
                })?;
                if variant.fields.len() != args.len() {
                    return None;
                }
                let arguments = variant
                    .fields
                    .iter()
                    .zip(args)
                    .map(|(field, argument)| {
                        Some((field.declaration_type.as_ref()?, Cow::Borrowed(argument)))
                    })
                    .collect::<Option<Vec<_>>>()?;
                (id, arguments)
            }
            ExprKind::Field {
                expr: receiver,
                field,
            } => {
                let ExprKind::Path(path) = &receiver.kind else {
                    return None;
                };
                let name = path.to_string();
                let id = self.semantic_type_in_current_scope(&name)?.base_type_id()?;
                let declaration = self.type_by_id(id)?;
                let variant = declaration.variants.iter().find(|variant| {
                    self.ctx
                        .strings
                        .get(variant.name.0 as usize)
                        .is_some_and(|name| name == field.name.as_str())
                })?;
                if !variant.fields.is_empty() || variant.arity != 0 {
                    return None;
                }
                (id, Vec::new())
            }
            _ => return None,
        };
        self.instantiate_constructor_fields(owner, arguments.into(), expected, output, depth)
    }

    /// Named records and named sum payloads use the same field telescope.
    /// The constructor's selected owner supplies its generic parameters; the
    /// payload is not an independent nominal type with an erased application.
    fn named_constructor_fields(
        &self, path: &verum_ast::Path, expected: Option<&TypeRef>,
    ) -> Option<(TypeId, &[crate::types::FieldDescriptor])> {
        let name = path.to_string();
        if let Some(record) = self.semantic_type_in_current_scope(&name)
            .and_then(|ty| ty.base_type_id()).and_then(|id| self.type_by_id(id))
            .filter(|declaration| declaration.kind == TypeKind::Record)
        {
            return Some((record.id, &record.fields));
        }
        let name = self.resolve_variant_path_alias(&name.replace("::", "."));
        let owner = name.rsplit_once('.')
            .and_then(|(qualifier, _)| self.declared_sum_descriptor(qualifier))
            .or_else(|| {
                let selected = if name.contains('.') {
                    self.ctx.lookup_qualified_function(&name)
                } else {
                    self.ctx.lookup_function_in_scope(&name)
                }?;
                selected.variant_tag?;
                self.type_by_id(selected.return_type.as_ref()?.base_type_id()?)
            })
            .or_else(|| expected.and_then(TypeRef::base_type_id).and_then(|id| self.type_by_id(id)))?;
        if owner.kind != TypeKind::Sum { return None; }
        let variant = self.scrutinee_variant(owner, &name)?;
        if variant.kind != crate::types::VariantKind::Record { return None; }
        Some((owner.id, &variant.fields))
    }

    /// A selected constructor supplies its exact nominal owner and payload
    /// declaration; its generic witnesses come from those declared fields.
    pub(super) fn selected_variant_declaration(
        &self, owner: TypeId, tag: u32, operands: &[Expr], expected: Option<&TypeRef>,
    ) -> Option<TypeRef> {
        let declaration = self.type_by_id(owner)?;
        if declaration.kind != TypeKind::Sum { return None; }
        let variant = declaration.variants.iter().find(|variant| variant.tag == tag)?;
        if variant.fields.len() != operands.len() { return None; }
        let arguments = variant.fields.iter().zip(operands)
            .map(|(field, operand)| Some((field.declaration_type.as_ref()?, Cow::Borrowed(operand))))
            .collect::<Option<verum_common::List<_>>>()?;
        self.instantiate_constructor_fields(owner, arguments, expected, None, 0)
    }

    fn instantiate_constructor_fields(
        &self, owner: TypeId, arguments: verum_common::List<(&TypeRef, Cow<'_, Expr>)>,
        expected: Option<&TypeRef>, output: Option<&TypeRef>, depth: usize,
    ) -> Option<TypeRef> {
        let declaration = self.type_by_id(owner)?;
        let template = if declaration.type_params.is_empty() {
            TypeRef::Concrete(owner)
        } else {
            TypeRef::Instantiated {
                base: owner,
                args: declaration
                    .type_params
                    .iter()
                    .map(|p| TypeRef::Generic(p.id))
                    .collect(),
            }
        };
        let mut bindings = TypeSubstitution::empty();
        if let Some(expected) = expected {
            if !self.bind_source_declaration(&template, expected, &mut bindings) {
                return None;
            }
        }
        if let Some(expected_output) = output {
            let associated = self.future_output_declaration(owner)?;
            if !self.bind_source_declaration(associated, expected_output, &mut bindings) {
                return None;
            }
        }
        let owner_ids: verum_common::List<_> =
            declaration.type_params.iter().map(|p| p.id).collect();
        for (field, value) in arguments {
            if !super::formal_parameters::resolved(field, &owner_ids, 0) {
                return None;
            }
            let contextual = bindings.apply(field).ok()?;
            // An unbound owner parameter is not a caller parameter, even when
            // its numeric ID coincides. Context only travels after all free
            // owner parameters of the field have acquired actual bindings.
            let known = fully_bound(field, &bindings);
            let actual = self.source_expression_declaration(
                &value,
                known.then_some(&contextual),
                depth + 1,
            );
            if std::env::var_os("VERUM_TRACE_MONO").is_some() {
                let bound: verum_common::List<_> = owner_ids.iter().map(|id| (id, bindings.get(*id))).collect();
                eprintln!("[constructor-field] owner={owner:?} formal={field:?} actual={actual:?} bindings={bound:?}");
            }
            let actual = actual?;
            // A declared payload can receive the value in a reference. With no
            // contextual binding a generic field may instead hold the reference itself.
            let actual = match &actual {
                TypeRef::Reference { inner, .. } if known && inner.as_ref() == &contextual => inner.as_ref(),
                _ => &actual,
            };
            if !self.bind_source_declaration(field, actual, &mut bindings) {
                return None;
            }
        }
        let args = declaration
            .type_params
            .iter()
            .map(|p| bindings.get(p.id).cloned())
            .collect::<Option<Vec<_>>>()?;
        let result = if args.is_empty() {
            TypeRef::Concrete(owner)
        } else {
            TypeRef::Instantiated { base: owner, args }
        };
        Some(result)
    }

    /// Numeric literals obtain their carrier from an already resolved source
    /// declaration. Its canonical scalar identity and layout own the range;
    /// a user record's name or size cannot authorize numeric construction.
    /// `Some(None)` is a refused numeric context, not permission to retry the
    /// literal's default type (which can have the very same primitive ID).
    fn contextual_numeric_literal(
        &self,
        expression: &Expr,
        expected: &TypeRef,
    ) -> Option<Option<TypeRef>> {
        use crate::instruction::LayoutProperty;
        use verum_ast::{UnOp, literal::LiteralKind};

        let primitive = self.expand_declared_alias(expected.clone())?;
        let TypeRef::Concrete(id) = primitive else { return None };
        let mut expression = Self::normalized_place(expression);
        let negated = if let ExprKind::Unary { op: UnOp::Neg, expr } = &expression.kind {
            expression = Self::normalized_place(expr);
            true
        } else {
            false
        };
        let ExprKind::Literal(literal) = &expression.kind else { return None };
        let accepts = match &literal.kind {
            LiteralKind::Int(integer) if integer.suffix.is_none() && id.is_integer() => {
                (|| {
                    let value = if negated { integer.value.checked_neg()? } else { integer.value };
                    let bits = crate::type_layout::query_with(
                        &primitive, LayoutProperty::Bits, |id| self.type_by_id(id),
                    )?;
                    let signed = crate::type_layout::query_with(
                        &primitive, LayoutProperty::IsSigned, |id| self.type_by_id(id),
                    )? != 0;
                    if !(1..=128).contains(&bits) { return None; }
                    Some(if signed {
                        bits == 128 || {
                            let half = 1_i128.checked_shl(u32::try_from(bits - 1).ok()?)?;
                            value >= -half && value < half
                        }
                    } else {
                        value >= 0 && (bits == 128 || (value as u128) < (1_u128 << bits))
                    })
                })().unwrap_or(false)
            }
            LiteralKind::Float(float) if float.suffix.is_none() && id.is_float() => true,
            _ => return None,
        };
        Some((accepts && self.declared_type_in_scope(expected)).then(|| expected.clone()))
    }

    pub(super) fn source_expression_declaration(
        &self,
        expr: &Expr,
        expected: Option<&TypeRef>,
        depth: usize,
    ) -> Option<TypeRef> {
        if depth >= 64 {
            return None;
        }
        let expr = Self::normalized_place(expr);
        if let Some(function) = self.selected_function_value(expr).ok()? {
            return self.function_value_signature(function, expected);
        }
        if let Some(expected) = expected
            && let Some(literal) = self.contextual_numeric_literal(expr, expected) {
            return literal;
        }
        if let ExprKind::Array(verum_ast::ArrayExpr::List(items)) = &expr.kind
            && let Some(owner) = expected
            && let Some(target) = self.expand_declared_alias(owner.clone())
            && matches!(target, TypeRef::Instantiated { .. } | TypeRef::Concrete(_))
        {
            let element = self.nominal_array_element(&target)?;
            return items.iter().all(|item|
                self.source_expression_declaration(item, Some(element), depth + 1).as_ref() == Some(element)
            ).then(|| owner.clone());
        }
        if let Some(result) = self.lexical_call_result(expr) {
            return result;
        }
        if let Some(ty) = self.constructor_declaration(expr, expected, None, depth + 1) {
            return Some(ty);
        }
        let ty = self
            .source_call_declaration(expr, expected, depth + 1)
            .or_else(|| {
                (!matches!(expr.kind, ExprKind::Record { .. }))
                    .then(|| self.declared_expression_result(expr, None, 0, 0))
                    .flatten()
            })
            .or_else(|| {
                self.extract_expr_type_name(expr)
                    .or_else(|| self.infer_expr_type_name(expr))
                    .or_else(|| Self::literal_default_type_name(expr))
                    .and_then(|name| self.semantic_type_in_current_scope(&name))
            })?;
        if !self.declared_type_in_scope(&ty) {
            return None;
        }
        if let Some(id) = ty.base_type_id() {
            if let Some(owner) = self.type_by_id(id) {
                let arity = match &ty {
                    TypeRef::Instantiated { args, .. } => args.len(),
                    _ => 0,
                };
                if owner.type_params.len() != arity {
                    return None;
                }
            }
        }
        if let Some(expected) = expected {
            if expected != &ty {
                return None;
            }
        }
        Some(ty)
    }

    pub(super) fn admits_constructor_context(expr: &Expr) -> bool {
        matches!(Self::normalized_place(expr).kind,
            ExprKind::Call { .. } | ExprKind::MethodCall { .. } | ExprKind::Record { .. }
                | ExprKind::Array(verum_ast::ArrayExpr::List(_))
                | ExprKind::Path(_) | ExprKind::Field { .. } | ExprKind::Block(_)
                | ExprKind::Unsafe(_) | ExprKind::If { .. } | ExprKind::Match { .. })
    }

    /// A call's expected result can bind its own parameters. Fully bound
    /// parameters supply both constructor context and argument adaptations.
    pub(super) fn contextual_call_parameters(
        &self, info: &FunctionInfo, explicit: &[verum_ast::ty::GenericArg], operands: &[Expr],
    ) -> CodegenResult<Option<Vec<Option<TypeRef>>>> {
        let Some(TypeRef::Function { params, return_type, .. }) = info.callable_signature.as_ref() else {
            return Ok(None);
        };
        let mut bindings = TypeSubstitution::empty();
        self.bind_source_type_arguments(info, explicit, &mut bindings)?;
        if let Some(expected) = self.ctx.expected_value_type.as_ref() {
            let mut result_bindings = bindings.clone();
            if self.bind_source_declaration(return_type, expected, &mut result_bindings) {
                bindings = result_bindings;
            }
        }
        let offset = usize::from(info.param_names.first().is_some_and(|name| name == "self")
            && params.len() == operands.len() + 1);
        for (index, (formal, argument)) in params.iter().skip(offset).zip(operands).enumerate() {
            if !formal.is_generic() || self.slice_parameter_element(formal).is_none() { continue; }
            let invalid = |message: &str| CodegenError::with_span(
                CodegenErrorKind::InvalidSourceSignature(message.into()), argument.span);
            let actual = self.source_expression_declaration(argument, None, 0)
                .ok_or_else(|| invalid("generic slice argument requires its source element declaration"))?;
            self.bind_declared_operand(info, index + offset, formal, &actual, &mut bindings)
                .ok_or_else(|| invalid("generic slice argument cannot satisfy its selected parameter declaration"))?;
        }
        Ok(params.iter().map(|formal| {
            if fully_bound(formal, &bindings) {
                Some(Some(bindings.apply(formal).ok()?))
            } else {
                Some(None)
            }
        }).collect())
    }

    /// A projected field belongs to its instantiated record, so its free
    /// arguments are caller witnesses even when none of them is concrete yet.
    pub(super) fn declared_argument_witnesses<'a>(
        &self, id: u32, count: usize, args: impl Iterator<Item = &'a Expr>,
        captured: Option<&[Option<TypeRef>]>,
    ) -> Option<Vec<TypeRef>> {
        let info = self.ctx.lookup_function_by_id(crate::module::FunctionId(id))?;
        self.selected_argument_witnesses(info, count, args, captured)
    }

    /// Synthetic constructors share no executable ID. Bind the callable that
    /// was actually selected, preserving its nominal target and parameters.
    pub(super) fn selected_argument_witnesses<'a>(
        &self, info: &FunctionInfo, count: usize, args: impl Iterator<Item = &'a Expr>,
        captured: Option<&[Option<TypeRef>]>,
    ) -> Option<Vec<TypeRef>> {
        self.selected_argument_witnesses_in(info, count, args, captured, TypeSubstitution::empty())
    }

    pub(super) fn selected_argument_witnesses_in<'a>(
        &self, info: &FunctionInfo, count: usize, args: impl Iterator<Item = &'a Expr>,
        captured: Option<&[Option<TypeRef>]>, bindings: TypeSubstitution,
    ) -> Option<Vec<TypeRef>> {
        let bindings = self.selected_argument_bindings_in(info, count, args, captured, bindings).ok()?;
        info.type_param_ids.iter().map(|id| bindings.get(*id).cloned()).collect()
    }

    /// An unescaped empty allocation owns no element yet. Use the nominal
    /// method's other arguments to instantiate its receiver before dispatch.
    pub(super) fn constrain_open_list_receiver(
        &mut self, receiver: &Expr, method: &str, args: &[Expr], explicit: &[verum_ast::ty::GenericArg],
    ) -> CodegenResult<()> {
        let Some(name) = Self::named_place_binding(receiver) else { return Ok(()); };
        let Some(binding) = self.ctx.lookup_var(name) else { return Ok(()); };
        let Some(owner) = binding.pending_list_element else { return Ok(()); };
        if !self.ctx.array_result_facts.is_fresh_list(binding.reg) { return Ok(()); }
        let Some(nominal) = self.declaration_type_name(&TypeRef::Concrete(owner), 0) else { return Ok(()); };
        let Some(key) = self.registered_receiver_method(&nominal, method) else { return Ok(()); };
        let Some(info) = self.ctx.lookup_qualified_function(&key) else { return Ok(()); };
        if !self.has_declared_method_receiver(info)? { return Ok(()); }
        let Some(TypeRef::Function { params, .. }) = &info.callable_signature else { return Ok(()); };
        let Some(first) = params.first() else { return Ok(()); };
        let carrier = crate::receiver_methods::pointee(first);
        if carrier.base_type_id() != Some(owner) { return Ok(()); }
        let mut bindings = TypeSubstitution::empty();
        self.bind_source_type_arguments(info, explicit, &mut bindings)?;
        let operands = (1..).zip(args);
        let Ok(bindings) = self.bind_indexed_operands(info, args.len() + 1, operands, None, bindings)
        else { return Ok(()); };
        if !fully_bound(carrier, &bindings) { return Ok(()); }
        let Ok(inferred) = bindings.apply(carrier) else { return Ok(()); };
        if !self.declared_type_in_scope(&inferred) { return Ok(()); }
        self.ctx.lookup_var_mut(name).unwrap().pending_list_element = None;
        self.publish_resolved_binding_type(name, &inferred);
        Ok(())
    }

    /// Inference may leave a declaration-owned slot open for explicit arguments.
    /// Failed evidence is distinct from a successful, incomplete substitution.
    pub(super) fn selected_argument_bindings_in<'a>(
        &self, info: &FunctionInfo, count: usize, args: impl Iterator<Item = &'a Expr>,
        captured: Option<&[Option<TypeRef>]>, bindings: TypeSubstitution,
    ) -> CodegenResult<TypeSubstitution> {
        self.bind_indexed_operands(info, count, args.enumerate(), captured, bindings)
    }

    fn bind_indexed_operands<'a>(
        &self, info: &FunctionInfo, count: usize, args: impl Iterator<Item = (usize, &'a Expr)>,
        captured: Option<&[Option<TypeRef>]>, mut bindings: TypeSubstitution,
    ) -> CodegenResult<TypeSubstitution> {
        let invalid = |message: String| CodegenError::new(CodegenErrorKind::TypeInference(message));
        let missing = |index: usize| invalid(format!(
            "missing source argument evidence for parameter {} (`{}`)", index + 1,
            info.param_names.get(index).map(String::as_str).unwrap_or("_")));
        if count > info.param_count {
            return Err(invalid(format!("call takes {} arguments but received {count}", info.param_count)));
        }
        for index in count..info.param_count {
            let constant = self.declared_callable_descriptors.get(&info.id)
                .and_then(|declaration| declaration.params.get(index))
                .is_some_and(|parameter| parameter.default.is_some());
            let expression = self.declared_callable_defaults.get(&info.id)
                .and_then(|defaults| defaults.get(index)).is_some_and(Option::is_some);
            if !constant && !expression { return Err(missing(index)); }
        }
        if info.type_param_ids.is_empty() { return Ok(bindings); }
        // Async, generator and contextual functions do not have an ordinary
        // synchronous function result. Their source parameter declarations
        // still bind generic witnesses, independently of the result carrier.
        let signature = match info.callable_signature.as_ref() {
            Some(TypeRef::Function { params, return_type, .. }) => Some((params, return_type)),
            _ => None,
        };
        let declared = self.ctx.semantic_fn_params.get(&info.id);
        let parameter = |index| signature.and_then(|(params, _)| params.get(index))
            .or_else(|| declared?.get(index).and_then(Option::as_ref));
        if let (Some((_, return_type)), Some(expected)) = (signature, self.ctx.expected_value_type.as_ref()) {
            // A surrounding call can have a different result than this operand.
            // A failed structural match contributes no partial substitutions.
            let mut result_bindings = bindings.clone();
            if self.bind_source_declaration(return_type, expected, &mut result_bindings) {
                bindings = result_bindings;
            }
        }
        for (index, arg) in args {
            let formal = parameter(index).ok_or_else(|| missing(index))?;
            if !formal.is_generic() { continue; }
            let expected = fully_bound(formal, &bindings).then(|| bindings.apply(formal).ok()).flatten();
            // An unsuffixed literal has the selected scalar carrier only when
            // its value fits that declaration. Other operands keep their
            // evaluated receipts even after a move consumes their source place.
            let contextual_literal = expected.as_ref()
                .and_then(|expected| self.contextual_numeric_literal(arg, expected));
            let actual = if let Some(literal) = contextual_literal {
                literal
            } else if let Some(actual) = captured.and_then(|arguments| arguments.get(index)).and_then(Option::as_ref) {
                Some(actual.clone())
            } else if matches!(
                Self::normalized_place(arg).kind, ExprKind::Call { .. } | ExprKind::MethodCall { .. }
                    | ExprKind::Array(verum_ast::ArrayExpr::List(_))
            ) {
                self.source_expression_declaration(arg, expected.as_ref(), 0)
            } else {
                self.compiled_callable_signature(arg).cloned().or_else(||
                    self.declared_expression_result(arg, None, 0, 0)).or_else(|| {
                    // A literal binding a still-free formal has the language's
                    // literal type. No register or rendered variable hint is evidence.
                    Self::literal_default_type_name(arg)
                        .and_then(|name| self.semantic_type_in_current_scope(&name))
                })
            };
            if std::env::var_os("VERUM_TRACE_MONO").is_some() {
                eprintln!("[declared-operand] function={:?} slot={index} formal={formal:?} actual={actual:?}", info.id);
            }
            let display = |ty: &TypeRef| self.declaration_type_name(ty, 0).unwrap_or_else(|| format!("{ty:?}"));
            let diagnosis = |reason: &str, actual: Option<&TypeRef>| {
                let name = info.param_names.get(index).map(String::as_str).unwrap_or("_");
                let wanted = expected.as_ref().unwrap_or(formal);
                let mut message = format!("{reason} source argument evidence for `{name}` (argument {}), expected {}", index + 1, display(wanted));
                if let Some(ty) = actual { message.push_str(&format!(", found {}", display(ty))); }
                invalid(message).at_source(arg.span)
            };
            let actual = actual.ok_or_else(|| diagnosis("missing", None))?;
            // Only an owned receiver needs an implicit borrow. An existing
            // reference goes through the ordinary argument adaptation, which
            // preserves permission and can borrow a fixed array as a slice.
            let formal = match formal {
                TypeRef::Reference { inner, .. } if index == 0
                    && !matches!(actual, TypeRef::Reference { .. })
                    && self.has_declared_method_receiver(info)? => inner.as_ref(),
                _ => formal,
            };
            let actual = &actual;
            let accepted = self.bind_declared_operand(info, index, formal, actual, &mut bindings);
            if std::env::var_os("VERUM_TRACE_MONO").is_some() {
                let bound: verum_common::List<_> = info.type_param_ids.iter().map(|id| (id, bindings.get(*id))).collect();
                eprintln!("[declared-call] function={:?} slot={index} formal={formal:?} actual={actual:?} accepted={} bindings={bound:?}", info.id, accepted.is_some());
            }
            accepted.ok_or_else(|| diagnosis("conflicting", Some(actual)))?;
            self.complete_declared_callable_arguments(info, &mut bindings)
                .ok_or_else(|| diagnosis("conflicting", Some(actual)))?;
        }
        // Omitted arguments are real producers emitted from this declaration's
        // defaults. Their receipts can bind an otherwise unconstrained slot;
        // neither the omitted syntax nor the formal type supplies that evidence.
        for index in count..info.param_count {
            let formal = parameter(index).ok_or_else(|| missing(index))?;
            if !formal.is_generic() { continue; }
            let expected = fully_bound(formal, &bindings).then(|| bindings.apply(formal).ok()).flatten();
            let contextual_literal = expected.as_ref().and_then(|expected|
                self.declared_callable_defaults.get(&info.id)?.get(index)?.as_ref()
                    .and_then(|expression| self.contextual_numeric_literal(expression, expected)));
            let actual = match contextual_literal {
                Some(literal) => literal.ok_or_else(|| missing(index))?,
                None => captured.and_then(|args| args.get(index)).and_then(Option::as_ref)
                    .cloned().ok_or_else(|| missing(index))?,
            };
            self.bind_declared_operand(info, index, formal, &actual, &mut bindings)
                .and_then(|()| self.complete_declared_callable_arguments(info, &mut bindings))
                .ok_or_else(|| missing(index))?;
        }
        self.complete_declared_callable_arguments(info, &mut bindings)
            .ok_or_else(|| invalid("conflicting source argument evidence for the selected callable's bounds".into()))?;
        Ok(bindings)
    }

    pub(super) fn bind_static_receiver_declaration(
        &self, function: &FunctionInfo, owner: &TypeRef, bindings: &mut TypeSubstitution,
    ) -> Option<()> {
        let descriptor = self.declared_callable_descriptors.get(&function.id)?;
        if descriptor.source_signature.as_ref()?.is_method() { return None; }
        let declared_owner = descriptor.source_receiver.as_ref()?;
        self.bind_source_declaration(declared_owner, crate::receiver_methods::pointee(owner), bindings)
            .then_some(())
    }

    /// The receiver of a factory inside an implementation is a type in the
    /// caller's scope. Its arguments may be reordered, specialized or absent
    /// from the enclosing method's own generic parameters.
    pub(super) fn lexical_factory_receiver(&self, receiver: &Expr) -> Option<&TypeRef> {
        let ExprKind::Path(path) = &receiver.kind else { return None; };
        let name = path.as_ident()?.as_str();
        let owner = self.current_impl_receiver.as_ref().filter(|owner| owner.semantic)?;
        if name == "Self" { return Some(&owner.ty); }
        if self.is_value_receiver_binding(name) { return None; }
        (self.nominal_type_id(name)? == owner.ty.base_type_id()?).then_some(&owner.ty)
    }

    /// A receiver can determine F without mentioning the result parameter in
    /// its own nominal arguments. Only F's declared callable bound can then
    /// determine that residual slot; caller and callee IDs remain separate.
    pub(super) fn complete_declared_callable_arguments(
        &self, function: &FunctionInfo, bindings: &mut TypeSubstitution,
    ) -> Option<()> {
        let declaration = self.declared_callable_descriptors.get(&function.id)
            .or_else(|| self.functions.iter().find(|body| body.descriptor.id == function.id)
                .map(|body| &body.descriptor));
        let Some(declaration) = declaration else { return Some(()); };
        loop {
            let before = bindings.len();
            self.complete_declared_associated_arguments(function, declaration, bindings)?;
            for parameter in &declaration.type_params {
                let actual = match bindings.get(parameter.id) {
                    Some(signature @ TypeRef::Function { .. }) => Some(signature),
                    Some(TypeRef::Generic(id)) => self.ctx.registers.callable_bound_signatures.get(id),
                    // A reference can be passed to a declared &F parameter, where
                    // F binds to its pointee. A reference bound to F itself cannot
                    // be dispatched as a closure, even if the result is known.
                    Some(TypeRef::Reference { .. }) if parameter.type_bounds.iter()
                        .any(|bound| matches!(bound, TypeRef::Function { .. })) => return None,
                    _ => None,
                };
                let Some(actual) = actual.cloned() else { continue; };
                for bound in &parameter.type_bounds {
                    if matches!(bound, TypeRef::Function { .. }) {
                        let bound = self.callable_declaration_type(function, bound)?;
                        self.bind_callable_operand(&bound, &actual, bindings).then_some(())?;
                    }
                }
            }
            if bindings.len() == before { return Some(()); }
        }
    }

    pub(super) fn bind_declared_operand(&self, function: &FunctionInfo, index: usize, formal: &TypeRef,
        actual: &TypeRef, bindings: &mut TypeSubstitution) -> Option<()> {
        let slice = self.declared_slice_argument_type(formal, actual);
        let slice_formal = slice.as_ref().and_then(|_| self.expand_declared_alias(formal.clone()));
        let formal = slice_formal.as_ref().unwrap_or(formal);
        let actual = slice.as_ref().unwrap_or(actual);
        // A selected owned alias can be passed to its exact unrefined base.
        // Keep a Generic formal's nominal witness and every nested argument
        // unchanged; writable reference carriers remain invariant below.
        let adapted = match (formal, actual.base_type_id()) {
            (TypeRef::Concrete(formal_id), Some(actual_id))
                if self.type_by_id(*formal_id).is_none_or(|declaration|
                    declaration.kind != TypeKind::Alias)
                    && self.type_by_id(actual_id).is_some_and(|declaration|
                        declaration.kind == TypeKind::Alias)
                    && !matches!(actual, TypeRef::Reference { .. }) =>
                self.expand_declared_alias(actual.clone()).filter(|base| base == formal),
            _ => None,
        };
        let actual = adapted.as_ref().unwrap_or(actual);
        // A checked call can adapt the outer reference carrier. Its witness
        // still describes the same pointee. Keep nested qualifiers exact and
        // never infer write capability from an immutable argument.
        let (formal, actual) = match (formal, actual) {
            (TypeRef::Reference { inner: expected, mutability: expected_access, .. },
             TypeRef::Reference { inner: supplied, mutability: supplied_access, .. }) => {
                if *expected_access == Mutability::Mutable && *supplied_access != Mutability::Mutable {
                    return None;
                }
                (expected.as_ref(), supplied.as_ref())
            }
            _ => (formal, actual),
        };
        self.bind_callable_operand(formal, actual, bindings).then_some(())?;
        if matches!(formal, TypeRef::Generic(_)) && matches!(actual, TypeRef::Function { .. })
            && let Some(bound) = self.callable_parameter_bound(function, index)
        {
            let bound = self.callable_declaration_type(function, bound)?;
            self.bind_callable_operand(&bound, actual, bindings).then_some(())?;
        }
        Some(())
    }

    /// A diverging callback supplies no result, so it satisfies a selected
    /// result type without changing that type's witness. This is covariance
    /// at the function boundary, not equality of Never with another type.
    fn bind_callable_operand(&self, formal: &TypeRef, actual: &TypeRef,
        bindings: &mut TypeSubstitution) -> bool {
        if let (TypeRef::Function { is_async: expected_async, params: expected_params,
                return_type: expected_result, contexts: expected_contexts },
            TypeRef::Function { is_async, params, return_type, contexts }) = (formal, actual)
        {
            // This boundary checks a callable allowance, not type equality.
            // A synchronous producer also satisfies an async-capable bound;
            // the opposite direction would admit an undeclared computation.
            if (*is_async && !*expected_async) || expected_contexts != contexts || expected_params.len() != params.len() {
                return false;
            }
            let mut selected = bindings.clone();
            if !expected_params.iter().zip(params).all(|(expected, supplied)|
                self.bind_source_declaration(expected, supplied, &mut selected))
                || (return_type.as_ref() != &TypeRef::Concrete(crate::types::TypeId::NEVER)
                    && !self.bind_source_declaration(expected_result, return_type, &mut selected))
            {
                return false;
            }
            *bindings = selected;
            return true;
        }
        self.bind_source_declaration(formal, actual, bindings)
    }

    /// Only the callable's explicit slots are named at the call site. An
    /// impl receiver can contribute additional parameters to its full roster.
    pub(super) fn bind_source_type_arguments(
        &self,
        function: &FunctionInfo,
        arguments: &[verum_ast::ty::GenericArg],
        bindings: &mut TypeSubstitution,
    ) -> CodegenResult<()> {
        let slots = &function.explicit_type_param_ids;
        let invalid = |message: &str| CodegenError::new(CodegenErrorKind::TypeInference(message.into()));
        if arguments.len() > slots.len() {
            return Err(invalid("explicit generic argument exceeds the selected declaration's roster"));
        }
        let mut expected_bindings = TypeSubstitution::empty();
        let result = match function.callable_signature.as_ref() {
            Some(TypeRef::Function { return_type, .. }) => Some(return_type.as_ref()),
            _ => function.return_type.as_ref(),
        };
        if let (Some(result), Some(expected)) = (result, self.ctx.expected_value_type.as_ref()) {
            if !self.bind_source_declaration(result, expected, &mut expected_bindings) {
                expected_bindings = TypeSubstitution::empty();
            }
        }
        for (index, (slot, argument)) in slots.iter().zip(arguments).enumerate() {
            let expected = slot.and_then(|id| expected_bindings.get(id).or_else(|| bindings.get(id)));
            // A source hole is an obligation for the completed call. Without
            // context it contributes no binding yet; final witness encoding
            // still resolves and checks the entire explicit argument.
            if expected.is_none() && argument.type_for_declared_slot()
                .is_some_and(|ty| Self::explicit_type_contains_hole(&ty)) {
                continue;
            }
            let Some(actual) = self.callable_argument_witness(function, index, argument, expected)? else { continue; };
            let id = slot.ok_or_else(|| invalid("generic argument has no declared slot"))?;
            if !self.bind_source_declaration(&TypeRef::Generic(id), &actual, bindings) {
                return Err(invalid("explicit generic argument conflicts with its declared witness"));
            }
        }
        Ok(())
    }

    /// An in-scope callee shadows every module declaration, even when its
    /// result cannot be recovered. The outer option records that distinction.
    pub(super) fn lexical_call_result(&self, expr: &Expr) -> Option<Option<TypeRef>> {
        let ExprKind::Call { func, .. } = &expr.kind else { return None };
        let ExprKind::Path(path) = &Self::normalized_place(func).kind else { return None };
        let name = path.as_ident()?;
        self.ctx.lookup_var(name.as_str())?;
        let result = self.declared_expression_result(func, None, 0, 0).and_then(|ty| match ty {
            TypeRef::Function { return_type, .. } => Some(*return_type),
            TypeRef::Generic(id) => self.ctx.registers.callable_bound_signatures.get(&id).and_then(|signature| {
                let TypeRef::Function { return_type, .. } = signature else { return None };
                Some((**return_type).clone())
            }),
            _ => None,
        });
        Some(result)
    }

    /// Resolve a source call in its retained lexical environment. A mount is
    /// an explicit binding even when its target cannot be linked; it must not
    /// fall through to an unrelated declaration or an ambient builtin.
    pub(super) fn lookup_source_function(&self, name: &str, arity: usize) -> Option<&FunctionInfo> {
        if !name.contains('.') && !name.contains("::")
            && let Some(selected) = self.source_mounted_function(name, arity)
        {
            return selected.ok();
        }
        self.ctx.lookup_function_with_arity_in_scope(name, arity)
    }

    pub(super) fn source_call_declaration(
        &self,
        expr: &Expr,
        expected: Option<&TypeRef>,
        depth: usize,
    ) -> Option<TypeRef> {
        if depth >= 64 {
            return None;
        }
        if let Some(result) = self.lexical_call_result(expr) {
            return result;
        }
        let (name, type_args, args) = match &expr.kind {
            ExprKind::Call {
                func,
                type_args,
                args,
            } => {
                if let Some(function) = self.selected_function_path(func).ok()? {
                    return self.source_call_result(function, None, type_args, args, expected, depth);
                }
                let ExprKind::Path(path) = &func.kind else {
                    return None;
                };
                (path.to_string(), type_args, args)
            }
            ExprKind::MethodCall {
                receiver,
                method,
                type_args,
                args,
            } => {
                if let Some(owner) = self.lexical_factory_receiver(receiver)
                    && let Some((callable, owner)) = self.declared_static_receiver_method(owner, method.as_str(), args.len()).ok()?
                {
                    // A bare nominal can be specialized by the surrounding
                    // call. Explicit Self always denotes this implementation.
                    if Self::named_place_binding(receiver) != Some("Self")
                        && let Some(result) = self.source_call_result(callable, None, type_args, args, expected, depth)
                    {
                        return Some(result);
                    }
                    return self.source_call_result(callable, Some(&owner), type_args, args, expected, depth);
                }
                if let ExprKind::Path(path) = &receiver.kind
                    && let Some(ident) = path.as_ident()
                    && !self.is_value_receiver_binding(ident.as_str())
                    && self.nominal_type_id(ident.as_str()).is_some()
                {
                    let key = self.registered_receiver_method(ident.as_str(), method.name.as_str())?;
                    let callable = self.ctx.lookup_qualified_function(&key)?;
                    if callable.param_names.first().is_some_and(|name| name == "self") {
                        return None;
                    }
                    return self.source_call_result(callable, None, type_args, args, expected, depth);
                }
                if let Some(receiver) = self.source_expression_declaration(receiver, None, depth + 1) {
                    if let Some((function, owner)) = self.declared_static_receiver_method(&receiver, method.as_str(), args.len()).ok()? {
                        return self.source_call_result(function, Some(&owner), type_args, args, expected, depth);
                    }
                    let selected = self.declared_receiver_method(
                        &receiver, method.name.as_str(), args.len() + 1,
                    ).ok()??;
                    return match selected {
                        super::expressions::index_projection::DeclaredReceiverMethod::Function(function, owner) =>
                            self.source_call_result(function, Some(&owner), type_args, args, expected, depth),
                        super::expressions::index_projection::DeclaredReceiverMethod::Protocol(member) =>
                            (type_args.is_empty() && !member.is_async).then_some(member.output),
                    };
                }
                // Dotted free functions and static methods use the same source
                // namespace grammar. Flatten only namespace paths; a value
                // receiver needs a separately established self declaration.
                let owner = self.try_flatten_module_path_resolved(receiver)?.join(".");
                (format!("{owner}.{}", method.name), type_args, args)
            }
            _ => return None,
        };
        let function = self.lookup_source_function(&name, args.len())?;
        self.source_call_result(function, None, type_args, args, expected, depth)
    }

    pub(super) fn source_call_result(
        &self,
        function: &FunctionInfo,
        receiver: Option<&TypeRef>,
        type_args: &[verum_ast::ty::GenericArg],
        args: &[Expr],
        expected: Option<&TypeRef>,
        depth: usize,
    ) -> Option<TypeRef> {
        let TypeRef::Function {
            params,
            return_type,
            ..
        } = function.callable_signature.as_ref()?
        else {
            return None;
        };
        let receiver_slots = usize::from(receiver.is_some() && self.has_declared_method_receiver(function).ok()?);
        if params.len() != args.len() + receiver_slots {
            return None;
        }
        let mut bindings = TypeSubstitution::empty();
        self.bind_source_type_arguments(function, type_args, &mut bindings).ok()?;
        if let Some(receiver) = receiver {
            if receiver_slots == 0 {
                self.bind_static_receiver_declaration(function, receiver, &mut bindings)?;
            } else {
                let formal = crate::receiver_methods::pointee(params.first()?);
                self.bind_declared_operand(function, 0, formal, receiver, &mut bindings)?;
            }
            self.complete_declared_callable_arguments(function, &mut bindings)?;
        }
        if let Some(expected) = expected {
            if !self.bind_source_declaration(return_type, expected, &mut bindings) {
                return None;
            }
        }
        for (index, (formal, argument)) in params.iter().skip(receiver_slots).zip(args).enumerate() {
            let contextual = bindings.apply(formal).ok()?;
            let expected = fully_bound(formal, &bindings).then_some(&contextual);
            let actual = self.compiled_callable_signature(argument).cloned().or_else(|| {
                // Reference adaptation belongs to the selected call. Recover
                // the operand's own declaration before contextual matching.
                let supplied = matches!(formal, TypeRef::Reference { .. })
                    .then(|| self.source_expression_declaration(argument, None, depth + 1))
                    .flatten();
                supplied.or_else(|| self.source_expression_declaration(argument, expected, depth + 1))
            })?;
            self.bind_declared_operand(function, index + receiver_slots, formal, &actual, &mut bindings)?;
        }
        self.complete_declared_callable_arguments(function, &mut bindings)?;
        fully_bound(return_type, &bindings).then(|| bindings.apply(return_type).ok()).flatten()
    }

    pub(super) fn associated_future_output(&self, ty: &TypeRef, depth: usize) -> Option<TypeRef> {
        if depth >= 64 {
            return None;
        }
        let (owner, args) = match ty {
            TypeRef::Concrete(id) => (*id, &[][..]),
            TypeRef::Instantiated { base, args } => (*base, args.as_slice()),
            _ => return None,
        };
        let declaration = self.type_by_id(owner)?;
        if declaration.type_params.len() != args.len() {
            return None;
        }
        let output = self.future_output_declaration(owner)?;
        let output = TypeSubstitution::new(&declaration.type_params, args).apply(output).ok()?;
        self.resolve_declared_projections(&output, depth + 1)
    }

    fn resolve_declared_projections(&self, ty: &TypeRef, depth: usize) -> Option<TypeRef> {
        if depth >= 64 {
            return None;
        }
        Some(match ty {
            // Associated-type resolution cannot choose a record receiver.
            TypeRef::RecordFieldProjection { owner, ordinal } => TypeRef::RecordFieldProjection {
                owner: *owner, ordinal: *ordinal,
            },
            TypeRef::AssociatedProjection { base, qualifier, assoc, args } => {
                if qualifier.is_some() || !args.is_empty() { return None; }
                let base = self.resolve_declared_projections(base, depth + 1)?;
                let (owner, args) = match &base {
                    TypeRef::Concrete(id) => (*id, &[][..]),
                    TypeRef::Instantiated { base, args } => (*base, args.as_slice()),
                    _ => return None,
                };
                let declaration = self.type_by_id(owner)?;
                if declaration.type_params.len() != args.len() {
                    return None;
                }
                let binding = self.associated_binding(declaration.protocols.iter(), assoc)?;
                let declared = TypeSubstitution::new(&declaration.type_params, args).apply(binding).ok()?;
                self.resolve_declared_projections(&declared, depth + 1)?
            }
            TypeRef::Instantiated { base, args } => TypeRef::Instantiated {
                base: *base,
                args: args
                    .iter()
                    .map(|arg| self.resolve_declared_projections(arg, depth + 1))
                    .collect::<Option<_>>()?,
            },
            TypeRef::Dynamic(object) => TypeRef::Dynamic(object.try_map(|ty|
                self.resolve_declared_projections(ty, depth + 1).ok_or(())).ok()?),
            TypeRef::Tuple(parts) => TypeRef::Tuple(
                parts
                    .iter()
                    .map(|part| self.resolve_declared_projections(part, depth + 1))
                    .collect::<Option<_>>()?,
            ),
            _ => ty.clone(),
        })
    }
}

pub(super) fn fully_bound(ty: &TypeRef, bindings: &TypeSubstitution) -> bool {
    fully_bound_at(ty, bindings, 0)
}

/// Locally quantified parameters are bound by the enclosing function type.
/// Only free parameters, lowered back into the caller's scope, need witnesses.
fn fully_bound_at(ty: &TypeRef, bindings: &TypeSubstitution, depth: u32) -> bool {
    match ty {
        TypeRef::RecordFieldProjection { .. } => false,
        TypeRef::Generic(id) => {
            let index = u32::from(id.0);
            index < depth || bindings.get(crate::types::TypeParamId((index - depth) as u16)).is_some()
        }
        TypeRef::Instantiated { args, .. } | TypeRef::Tuple(args) => {
            args.iter().all(|ty| fully_bound_at(ty, bindings, depth))
        }
        TypeRef::Reference { inner, .. } | TypeRef::Slice(inner) => fully_bound_at(inner, bindings, depth),
        TypeRef::Array { element, .. } => fully_bound_at(element, bindings, depth),
        TypeRef::Function {
            params,
            return_type,
            ..
        } => {
            params.iter().all(|ty| fully_bound_at(ty, bindings, depth))
                && fully_bound_at(return_type, bindings, depth)
        }
        TypeRef::Rank2Function {
            type_param_count,
            params,
            return_type,
            ..
        } => {
            let Some(depth) = depth.checked_add(u32::from(*type_param_count)) else { return false; };
            params.iter().all(|ty| fully_bound_at(ty, bindings, depth))
                && fully_bound_at(return_type, bindings, depth)
        }
        TypeRef::Dynamic(object) => depth.checked_add(1).is_some_and(|depth|
            object.types().all(|ty| fully_bound_at(ty, bindings, depth))),
        TypeRef::AssociatedProjection { base, qualifier, args, .. } => fully_bound_at(base, bindings, depth)
            && args.iter().all(|argument| fully_bound_at(argument, bindings, depth))
            && qualifier.as_ref().is_none_or(|application| application.args.iter().all(|argument| fully_bound_at(argument, bindings, depth))),
        _ => true,
    }
}

#[cfg(test)]
mod binder_tests {
    use super::*;
    use crate::types::TypeParamId;

    #[test]
    fn negation_does_not_supply_literal_evidence_for_other_domains() {
        for source in ["-true", "-\"text\"", "-2_u16", "-(3_km)", "-x"] {
            let expression = verum_fast_parser::Parser::new(source).parse_expr().unwrap();
            assert_eq!(VbcCodegen::literal_default_type_name(&expression), None, "{source}");
        }
    }

    #[test]
    fn record_projection_requires_a_receiver_not_a_generic_binding() {
        let projection = TypeRef::RecordFieldProjection { owner: TypeId(2048), ordinal: 0 };
        let mut bindings = TypeSubstitution::empty();
        bindings.bind(TypeParamId(0), TypeRef::Concrete(TypeId::INT));
        assert!(!fully_bound(&projection, &bindings));
        let cg = VbcCodegen::new();
        assert_eq!(cg.resolve_declared_projections(&projection, 0), Some(projection.clone()));
        assert!(cg.declaration_type_name(&projection, 0).is_none());
        assert!(!fully_bound(&TypeRef::Rank2Function {
            is_async: false,
            type_param_count: 1, params: vec![TypeRef::Generic(TypeParamId(0))],
            return_type: Box::new(projection), contexts: Default::default(),
        }, &bindings));
    }

    #[test]
    fn quantified_local_is_bound_but_free_slot_requires_its_outer_witness() {
        let quantified = |index| TypeRef::Rank2Function {
            is_async: false,
            type_param_count: 1,
            params: vec![TypeRef::Generic(TypeParamId(0))],
            return_type: Box::new(TypeRef::Generic(TypeParamId(index))),
            contexts: Default::default(),
        };
        let mut bindings = TypeSubstitution::empty();
        assert!(fully_bound(&quantified(0), &bindings));
        assert!(!fully_bound(&quantified(1), &bindings));
        bindings.bind(TypeParamId(1), TypeRef::Concrete(TypeId::BOOL));
        assert!(!fully_bound(&quantified(1), &bindings));
        bindings.bind(TypeParamId(0), TypeRef::Concrete(TypeId::INT));
        assert!(fully_bound(&quantified(1), &bindings));
    }
}

#[cfg(test)]
#[path = "../../tests/codegen/call_operand_bindings.rs"]
mod call_operand_bindings;
