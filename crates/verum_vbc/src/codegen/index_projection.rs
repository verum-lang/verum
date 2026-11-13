//! Resolve record and index projections against the declarations that own their storage.
use super::*;

pub(super) struct ProjectionMethod {
    name: String,
    id: crate::module::FunctionId,
    witnesses: Vec<crate::types::TypeRef>,
    result: crate::types::TypeRef,
}

enum ReceiverAdjustment {
    Native(usize),
    Method(ProjectionMethod),
}

struct FieldProjection {
    owner: crate::types::TypeRef,
    ordinal: u32,
    adjustments: Vec<ReceiverAdjustment>,
}

struct IndexProjection {
    adjustments: Vec<ReceiverAdjustment>,
    element: Option<crate::types::TypeRef>,
    method: Option<ProjectionMethod>,
}

impl VbcCodegen {
    /// Runtime carrier identity follows the defining descriptor, even when a
    /// source producer assigned a local ID. Foreign same-leaf declarations do
    /// not inherit a carrier merely by resembling its name or layout.
    pub(in crate::codegen) fn declared_runtime_carrier(
        &self,
        id: crate::types::TypeId,
    ) -> Option<WKT> {
        if let Some(carrier) = id.well_known_name().and_then(WKT::from_name) {
            return Some(carrier);
        }
        if let Some(declaration) = self.type_by_id(id) {
            if declaration.kind != crate::types::TypeKind::Record {
                return None;
            }
            let identity = self.type_declaration_identity(declaration)?;
            let (owner, name) = identity.rsplit_once('.')?;
            return WKT::from_declaration(name, owner);
        }
        None
    }

    pub(super) fn projection_operand_declaration(&self, base: &Expr) -> Option<crate::types::TypeRef> {
        let declared = self
            .declared_expression_result(base, None, 0, 0)
            .or_else(|| self.source_call_declaration(base, None, 0));
        if let Some(declared) = declared {
            return self.expand_declared_alias(declared);
        }
        // Parameters carry explicit source declarations. Their legacy display
        // hint can erase wrappers and is never a replacement for a missing one.
        if Self::expr_ident_name(base)
            .and_then(|name| self.ctx.lookup_var(&name))
            .is_some_and(|binding| binding.kind == crate::codegen::RegisterKind::Parameter)
        {
            return None;
        }
        self.extract_expr_type_name(base)
            .and_then(|name| self.semantic_type_in_current_scope(&name))
            .and_then(|ty| self.expand_declared_alias(ty))
    }

    /// A field/index does not discard the access capability of its owner.
    /// Mutable references authorize reborrows; owned bindings need mutation
    /// permission. Layout knowledge cannot turn a shared path into a mutable one.
    pub(super) fn mutable_projection_place(&self, base: &Expr, depth: usize) -> bool {
        use crate::types::{Mutability, TypeRef};
        if depth >= 64 {
            return false;
        }
        let base = Self::normalized_place(base);
        let Some(mut owner) = self.projection_operand_declaration(base) else {
            return false;
        };
        let mut borrowed = false;
        while let TypeRef::Reference {
            inner, mutability, ..
        } = owner
        {
            if mutability != Mutability::Mutable {
                return false;
            }
            let Some(next) = self.expand_declared_alias(*inner) else {
                return false;
            };
            owner = next;
            borrowed = true;
        }
        match &base.kind {
            ExprKind::Field { expr, field } => {
                self.mutable_projection_place(expr, depth + 1)
                    && self.projection_operand_declaration(expr)
                        .and_then(|owner| self.field_projection(owner, &field.name, true))
                        .is_some()
            }
            ExprKind::TupleIndex { expr, .. } => self.mutable_projection_place(expr, depth + 1),
            ExprKind::Index { expr, index } => self.index_projection(expr, index, true).is_some(),
            ExprKind::Unary {
                op: UnOp::Deref,
                expr,
            } => {
                matches!(
                    self.projection_operand_declaration(expr),
                    Some(TypeRef::Reference {
                        mutability: Mutability::Mutable,
                        ..
                    })
                ) && self.mutable_projection_place(expr, depth + 1)
            }
            ExprKind::Path(path) => {
                let name = match path.segments.as_slice() {
                    [verum_ast::PathSegment::Name(name)] => Some(name.name.as_str()),
                    [verum_ast::PathSegment::SelfValue] => Some("self"),
                    _ => None,
                };
                borrowed
                    || name
                        .and_then(|name| self.ctx.lookup_var(name))
                        .is_some_and(|binding| binding.is_mutable)
            }
            // An exact owned temporary has its own storage. A borrowed result
            // has already passed the reference check above.
            ExprKind::Call { .. }
            | ExprKind::MethodCall { .. }
            | ExprKind::Record { .. }
            | ExprKind::Array(_)
            | ExprKind::Block(_)
            | ExprKind::Cast { .. } => true,
            _ => borrowed,
        }
    }

    pub(super) fn require_mutable_index_projection(
        &self,
        base: &Expr,
        index: &Expr,
    ) -> CodegenResult<()> {
        self.index_projection(base, index, true)
            .map(|_| ())
            .ok_or_else(|| {
                CodegenError::with_span(
                    CodegenErrorKind::TypeInference(
                        "cannot index through this place mutably: no declared mutable projection"
                            .into(),
                    ),
                    base.span,
                )
            })
    }

    /// Select a projection on this exact owner, retaining the method's binder
    /// roster and reference result. This lookup never searches through Deref.
    fn projection_method(
        &self,
        owner: &crate::types::TypeRef,
        method: &str,
        index: Option<&Expr>,
    ) -> Option<ProjectionMethod> {
        use crate::types::TypeRef;
        let owner_name = self.declaration_type_name(owner, 0)?;
        let name = self.registered_receiver_method(&owner_name, method)?;
        let info = self.ctx.lookup_qualified_function(&name)?;
        let TypeRef::Function {
            params,
            return_type,
            ..
        } = info.callable_signature.as_ref()?
        else {
            return None;
        };
        if params.len() != 1 + usize::from(index.is_some()) {
            return None;
        }
        let TypeRef::Reference {
            inner: formal_owner,
            ..
        } = params.first()?
        else {
            return None;
        };
        let mut bindings = crate::mono::TypeSubstitution::empty();
        if !self.bind_source_declaration(formal_owner, owner, &mut bindings) {
            return None;
        }
        if let Some(index) = index {
            let actual = self.projection_operand_declaration(index).or_else(|| {
                Self::literal_default_type_name(index)
                    .and_then(|name| self.semantic_type_in_current_scope(&name))
            })?;
            if !self.bind_source_declaration(&params[1], &actual, &mut bindings) {
                return None;
            }
        }
        if !crate::codegen::constructor_types::fully_bound(return_type, &bindings) {
            return None;
        }
        Some(ProjectionMethod {
            name,
            id: info.id,
            witnesses: info
                .type_param_ids
                .iter()
                .map(|id| bindings.get(*id).cloned())
                .collect::<Option<_>>()?,
            result: bindings.apply(return_type).ok()?,
        })
    }

    /// A user projection used for mutation must return a mutable reference.
    /// The callable and its witnesses come from the same declaration as the
    /// selected target, including inherited owner parameters.
    pub(super) fn dereference_projection(
        &self,
        owner: &crate::types::TypeRef,
        mutable: bool,
    ) -> Option<(ProjectionMethod, crate::types::TypeRef)> {
        use crate::types::{Mutability, TypeRef};
        let method = self.projection_method(
            owner, if mutable { "deref_mut" } else { "deref" }, None,
        )?;
        let TypeRef::Reference { inner, mutability, .. } = &method.result else {
            return None;
        };
        use verum_common::well_known_types::WellKnownProtocol;
        if mutable && *mutability != Mutability::Mutable {
            return None;
        }
        let protocol = if mutable { WellKnownProtocol::DerefMut } else { WellKnownProtocol::Deref };
        let descriptor = owner.base_type_id().and_then(|id| self.type_by_id(id))?;
        let implements_projection = descriptor.protocols.iter().any(|row| {
            row.methods.contains(&method.id.0)
                && self.type_by_id(crate::types::TypeId(row.protocol.0))
                    .and_then(|ty| self.ctx.strings.get(ty.name.0 as usize))
                    .is_some_and(|name| protocol.matches(name.rsplit('.').next().unwrap_or(name)))
        });
        if !implements_projection {
            return None;
        }
        let target = (**inner).clone();
        Some((method, target))
    }

    /// A record's own field takes precedence over implicit dereference. The
    /// declaration supplies both the final field slot and every storage step.
    fn field_projection(
        &self,
        mut owner: crate::types::TypeRef,
        field: &str,
        mutable: bool,
    ) -> Option<FieldProjection> {
        use crate::types::{Mutability, TypeRef};
        let mut adjustments = Vec::new();
        let mut seen = Vec::new();
        for _ in 0..64 {
            owner = self.expand_declared_alias(owner)?;
            let mut references = 0;
            while let TypeRef::Reference { inner, mutability, .. } = owner {
                if mutable && mutability != Mutability::Mutable {
                    return None;
                }
                owner = self.expand_declared_alias(*inner)?;
                references += 1;
            }
            let id = owner.base_type_id()?;
            let descriptor = self.type_by_id(id)?;
            if let Some(ordinal) = descriptor.fields.iter().position(|member| {
                self.ctx.strings.get(member.name.0 as usize).is_some_and(|name| name == field)
            }) {
                return Some(FieldProjection {
                    owner,
                    ordinal: ordinal.try_into().ok()?,
                    adjustments,
                });
            }
            if seen.contains(&owner) {
                return None;
            }
            seen.push(owner.clone());
            if let Some(carrier) = self.declared_runtime_carrier(id).filter(|kind| kind.is_smart_pointer()) {
                if mutable && carrier != WKT::Heap {
                    return None;
                }
                let TypeRef::Instantiated { args, .. } = owner else { return None };
                let [target] = args.as_slice() else { return None };
                owner = target.clone();
                adjustments.push(ReceiverAdjustment::Native(references + 1));
            } else {
                let (method, target) = self.dereference_projection(&owner, mutable)?;
                owner = target;
                if self.declaration_type_name(&owner, 0)?.len() > 65536 {
                    return None;
                }
                adjustments.push(ReceiverAdjustment::Method(method));
            }
        }
        None
    }

    pub(in crate::codegen) fn projected_field_declaration(
        &self, owner: crate::types::TypeRef, field: &str,
    ) -> Option<crate::types::TypeRef> {
        let projection = self.field_projection(owner, field, false)?;
        let descriptor = self.type_by_id(projection.owner.base_type_id()?)?;
        let arguments = match &projection.owner {
            crate::types::TypeRef::Instantiated { args, .. } => args.as_slice(),
            _ => &[],
        };
        if descriptor.type_params.len() != arguments.len() {
            return None;
        }
        let member = descriptor.fields.get(projection.ordinal as usize)?.declaration_type.as_ref()?;
        let ids: verum_common::List<_> = descriptor.type_params.iter().map(|parameter| parameter.id).collect();
        if !crate::codegen::formal_parameters::resolved(member, &ids, 0) {
            return None;
        }
        crate::mono::TypeSubstitution::new(&descriptor.type_params, arguments).apply(member).ok()
    }

    pub(super) fn compile_field_storage(
        &mut self, base: &Expr, field: &str, mutable: bool,
    ) -> CodegenResult<(Reg, String, u32)> {
        let refusal = || CodegenError::with_span(CodegenErrorKind::TypeInference(format!(
            "cannot access field `{field}`: no declared {} projection",
            if mutable { "mutable" } else { "read" },
        )), base.span);
        if mutable && !self.mutable_projection_place(base, 0) { return Err(refusal()); }
        let owner = self.projection_operand_declaration(base).ok_or_else(refusal)?;
        let projection = self.field_projection(owner, field, mutable).ok_or_else(refusal)?;
        let name = self.declaration_type_name(&projection.owner, 0).ok_or_else(refusal)?;
        let value = self.compile_projection_base(base, mutable)?;
        let value = self.emit_receiver_adjustments(value, projection.adjustments, true);
        Ok((value, name, projection.ordinal))
    }

    fn compile_projection_base(&mut self, base: &Expr, mutable: bool) -> CodegenResult<Reg> {
        if mutable && let ExprKind::Field { expr, field } = &Self::normalized_place(base).kind {
            let (owner, _, ordinal) = self.compile_field_storage(expr, &field.name, true)?;
            let value = self.ctx.alloc_temp();
            self.ctx.emit(Instruction::GetF { dst: value, obj: owner, field_idx: ordinal });
            self.ctx.free_temp(owner);
            Ok(value)
        } else {
            self.compile_expr(base)?.or_internal("projection base has no value")
        }
    }

    /// Resolve the complete receiver before emitting any adjustment. Element
    /// identity and physical projection must come from this same finite plan.
    fn index_projection(
        &self,
        base: &Expr,
        index: &Expr,
        mutable: bool,
    ) -> Option<IndexProjection> {
        use crate::types::{Mutability, TypeId, TypeRef};
        if mutable && !self.mutable_projection_place(base, 0) {
            return None;
        }
        let mut owner = self.projection_operand_declaration(base)?;
        let mut adjustments = Vec::new();
        let mut seen = Vec::new();
        for _ in 0..64 {
            owner = self.expand_declared_alias(owner)?;
            let mut references = 0;
            while let TypeRef::Reference {
                inner, mutability, ..
            } = owner
            {
                if mutable && mutability != Mutability::Mutable {
                    return None;
                }
                owner = self.expand_declared_alias(*inner)?;
                references += 1;
            }
            if seen.contains(&owner) {
                return None;
            }
            seen.push(owner.clone());
            let nominal = owner
                .base_type_id()
                .and_then(|id| self.declared_runtime_carrier(id));
            let args = match &owner {
                TypeRef::Instantiated { args, .. } => args.as_slice(),
                _ => &[],
            };
            let native = match &owner {
                TypeRef::Array { element, .. } | TypeRef::Slice(element) => {
                    Some(Some((**element).clone()))
                }
                TypeRef::Instantiated { base, args } if *base == TypeId::ARRAY => {
                    Some(args.first().cloned())
                }
                TypeRef::Concrete(TypeId::ARRAY) => Some(None),
                TypeRef::Concrete(TypeId::BYTE_LIST) => Some(Some(TypeRef::Concrete(TypeId::U8))),
                _ => match nominal {
                    Some(WKT::List | WKT::Deque) => Some(args.first().cloned()),
                    Some(WKT::Map) => Some(args.get(1).cloned()),
                    Some(WKT::Text) => Some(Some(TypeRef::Concrete(TypeId::CHAR))),
                    _ => None,
                },
            };
            if let Some(element) = native {
                return Some(IndexProjection {
                    adjustments,
                    element,
                    method: None,
                });
            }
            if let Some(method) = self.projection_method(
                &owner,
                if mutable { "index_mut" } else { "index" },
                Some(index),
            ) && let TypeRef::Reference {
                inner, mutability, ..
            } = &method.result
                && (!mutable || *mutability == Mutability::Mutable)
            {
                return Some(IndexProjection {
                    adjustments,
                    element: Some((**inner).clone()),
                    method: Some(method),
                });
            }
            if nominal.is_some_and(|kind| kind.is_smart_pointer())
                && (!mutable || nominal == Some(WKT::Heap))
            {
                let [target] = args else {
                    return None;
                };
                let target = target.clone();
                adjustments.push(ReceiverAdjustment::Native(references + 1));
                owner = target;
                continue;
            }
            let (method, target) = self.dereference_projection(&owner, mutable)?;
            owner = target;
            // Expanding declarations must not allocate an exponential type tree.
            if self.declaration_type_name(&owner, 0)?.len() > 65536 {
                return None;
            }
            adjustments.push(ReceiverAdjustment::Method(method));
        }
        None
    }

    pub(in crate::codegen) fn indexed_element_declaration(
        &self,
        base: &Expr,
        index: &Expr,
    ) -> Option<crate::types::TypeRef> {
        use crate::types::{CbgrTier, Mutability, TypeRef};
        let projection = self.index_projection(base, index, false)?;
        let element = projection.element?;
        if projection.method.is_none() && matches!(index.kind, ExprKind::Range { .. }) {
            Some(TypeRef::Reference {
                inner: Box::new(TypeRef::Slice(Box::new(element))),
                mutability: Mutability::Immutable,
                tier: CbgrTier::Tier0,
            })
        } else {
            Some(element)
        }
    }

    pub(in crate::codegen) fn indexed_borrow_declaration(
        &self,
        base: &Expr,
        index: &Expr,
        mutable: bool,
    ) -> Option<crate::types::TypeRef> {
        use crate::types::{CbgrTier, Mutability, TypeRef};
        let projection = self.index_projection(base, index, mutable)?;
        let element = projection.element?;
        let inner = if projection.method.is_none() && matches!(index.kind, ExprKind::Range { .. }) {
            TypeRef::Slice(Box::new(element))
        } else {
            element
        };
        Some(TypeRef::Reference {
            inner: Box::new(inner),
            mutability: if mutable {
                Mutability::Mutable
            } else {
                Mutability::Immutable
            },
            tier: CbgrTier::Tier0,
        })
    }

    pub(super) fn emit_projection_method(
        &mut self,
        receiver: Reg,
        method: ProjectionMethod,
        index: Option<Reg>,
    ) -> Reg {
        let result = self.ctx.alloc_temp();
        let method_id = self.intern_string(&method.name);
        if !method.witnesses.is_empty() {
            self.ctx.emit(Instruction::SetCallWitness {
                type_args: method.witnesses,
            });
        }
        self.ctx.emit(Instruction::CallM {
            dst: result,
            receiver,
            method_id,
            args: RegRange::new(index.unwrap_or(receiver), u8::from(index.is_some())),
        });
        result
    }

    pub(super) fn compile_index_receiver(
        &mut self,
        base: &Expr,
        index: &Expr,
        mutable: bool,
    ) -> CodegenResult<(Reg, Option<ProjectionMethod>)> {
        let projection = self.index_projection(base, index, mutable).ok_or_else(|| {
            let owner = self
                .projection_operand_declaration(base)
                .and_then(|owner| self.declaration_type_name(&owner, 0))
                .unwrap_or_else(|| "receiver with unavailable declaration".into());
            CodegenError::with_span(CodegenErrorKind::TypeInference(format!(
                "cannot index {owner}: no declared {} projection",
                if mutable { "mutable" } else { "read" },
            )), base.span)
        })?;
        let value = self.compile_projection_base(base, mutable)?;
        Ok((self.emit_receiver_adjustments(value, projection.adjustments, false), projection.method))
    }

    fn emit_receiver_adjustments(
        &mut self, mut value: Reg, adjustments: Vec<ReceiverAdjustment>, keep_final_reference: bool,
    ) -> Reg {
        let count = adjustments.len();
        for (index, adjustment) in adjustments.into_iter().enumerate() {
            match adjustment {
                ReceiverAdjustment::Native(count) => {
                    for _ in 0..count {
                        let next = self.ctx.alloc_temp();
                        self.ctx.emit(Instruction::Deref {
                            dst: next,
                            ref_reg: value,
                        });
                        self.ctx.free_temp(value);
                        value = next;
                    }
                }
                ReceiverAdjustment::Method(method) => {
                    let reference = self.emit_projection_method(value, method, None);
                    self.ctx.free_temp(value);
                    if keep_final_reference && index + 1 == count {
                        value = reference;
                    } else {
                        value = self.ctx.alloc_temp();
                        self.ctx.emit(Instruction::Deref { dst: value, ref_reg: reference });
                        self.ctx.free_temp(reference);
                    }
                }
            }
        }
        value
    }
}
