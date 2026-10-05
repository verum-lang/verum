//! Declaration facts and emission observations shared by both source producers.
//! Observations do not grant cleanup authority. A consuming source context may
//! use an exact active declaration fact to select its explicit value handoff.
use crate::{
    Instruction,
    instruction::Reg,
    types::TypeRef,
    value_use::{MAX_VALUE_USES, ValueUseReceipt},
};
use verum_common::value_use::{
    BindingId, DuplicationOperation, ValueUseEvent, ValueUseId, ValueUseOperation, ValueUseSite,
};
use verum_common::{List, Map, Maybe, ResourceDiscipline};

#[derive(Debug, Clone)]
pub(super) struct BindingFact {
    pub id: BindingId,
    pub declaration_type: Maybe<TypeRef>,
    pub discipline: ResourceDiscipline,
}
#[derive(Debug, Clone, Default)]
pub(super) struct ValueUseRecorder {
    next_binding: u32,
    pub active: Map<Reg, BindingFact>,
    // Only an uninterrupted sequence of emitted Mov operations forwards this
    // source identity. Any other opcode clears it; this is not CFG inference.
    forwarded: Map<Reg, BindingFact>,
    pub uses: List<ValueUseReceipt>,
    exhausted: bool,
    // Scoped guards observe actual source identities while their bindings are
    // live. They are independent of the optional receipt budget and never
    // grant ownership permission. Nested functions reset/restore this recorder.
    return_use_capture: List<bool>,
}
impl ValueUseRecorder {
    pub fn allocate(&mut self, reg: Reg) -> BindingId {
        let id = BindingId(self.next_binding);
        self.next_binding = self.next_binding.saturating_add(1);
        self.active.insert(
            reg,
            BindingFact {
                id,
                declaration_type: None,
                discipline: ResourceDiscipline::Unknown,
            },
        );
        id
    }
    pub fn forget(&mut self, reg: Reg) {
        self.active.remove(&reg);
        self.forwarded.remove(&reg);
    }
    pub fn set_type(&mut self, reg: Reg, ty: TypeRef, discipline: ResourceDiscipline) {
        if let Some(fact) = self.active.get_mut(&reg) {
            fact.declaration_type = Some(ty);
            fact.discipline = discipline;
        }
    }
    pub fn fact(&self, reg: Reg) -> Maybe<BindingFact> {
        self.active
            .get(&reg)
            .or_else(|| self.forwarded.get(&reg))
            .cloned()
    }
    pub fn forward_result(&mut self, reg: Reg, fact: BindingFact) {
        self.forwarded.insert(reg, fact);
    }
    pub fn begin_return_use_capture(&mut self) -> bool {
        if self.return_use_capture.len() >= 128 {
            return false;
        }
        self.return_use_capture.push(false);
        true
    }
    pub fn finish_return_use_capture(&mut self) -> bool {
        self.return_use_capture.pop().unwrap_or(true)
    }
    fn capture_affine_operand(&mut self, reg: Reg) {
        if !self.return_use_capture.is_empty()
            && self
                .active
                .get(&reg)
                .or_else(|| self.forwarded.get(&reg))
                .is_some_and(|fact| {
                    matches!(
                        fact.discipline,
                        ResourceDiscipline::Affine | ResourceDiscipline::Linear
                    )
                })
        {
            for captured in &mut self.return_use_capture {
                *captured = true;
            }
        }
    }
    fn capture_return_inputs(&mut self, instruction: &Instruction) {
        if self.return_use_capture.is_empty() {
            return;
        }
        match instruction {
            // A named destination is handled by the existing local producer.
            // A temporary can feed a selected result, aggregate or call instead.
            Instruction::Mov { dst, src } if !self.active.contains_key(dst) => {
                self.capture_affine_operand(*src);
            }
            Instruction::Clone { src, .. } => self.capture_affine_operand(*src),
            Instruction::SetF { value, .. } | Instruction::SetVariantData { value, .. } => {
                self.capture_affine_operand(*value);
            }
            Instruction::Call { args, .. }
            | Instruction::CallG { args, .. }
            | Instruction::CallClosure { args, .. } => {
                for i in 0..args.count {
                    self.capture_affine_operand(Reg(args.start.0 + u16::from(i)));
                }
            }
            Instruction::CallM { receiver, args, .. } => {
                self.capture_affine_operand(*receiver);
                for i in 0..args.count {
                    self.capture_affine_operand(Reg(args.start.0 + u16::from(i)));
                }
            }
            Instruction::Pack {
                src_start, count, ..
            } => {
                for i in 0..*count {
                    self.capture_affine_operand(Reg(src_start.0 + u16::from(i)));
                }
            }
            _ => {}
        }
    }
    fn record(
        &mut self,
        index: usize,
        operand: Reg,
        fact: BindingFact,
        destination: Maybe<BindingId>,
        site: ValueUseSite,
        clone: bool,
    ) {
        if self.exhausted {
            return;
        }
        if self.uses.len() == MAX_VALUE_USES {
            self.uses.clear();
            self.exhausted = true;
            return;
        }
        let operation = if fact.discipline != ResourceDiscipline::Unrestricted {
            ValueUseOperation::Unknown
        } else if matches!(fact.declaration_type, Some(TypeRef::Reference { .. })) {
            ValueUseOperation::Borrow
        } else if clone {
            ValueUseOperation::Copy(DuplicationOperation::ValueCopy)
        } else {
            // Mov, arguments and returns do not establish an ownership handoff.
            ValueUseOperation::Unknown
        };
        self.uses.push(ValueUseReceipt {
            event: ValueUseEvent {
                id: ValueUseId(self.uses.len() as u32),
                binding: fact.id,
                destination,
                site,
                operation,
            },
            instruction: index as u32,
            operand,
            declaration_type: fact.declaration_type,
        });
    }
    pub fn observe(&mut self, instruction: &Instruction, index: usize) {
        self.capture_return_inputs(instruction);
        match instruction {
            Instruction::Mov { dst, src } => {
                let fact = self.fact(*src);
                self.forwarded.remove(dst);
                if let Some(fact) = fact {
                    if let Some(destination) = self.active.get(dst).map(|f| f.id) {
                        self.record(
                            index,
                            *src,
                            fact.clone(),
                            Some(destination),
                            ValueUseSite::Local,
                            false,
                        );
                    }
                    self.forwarded.insert(*dst, fact);
                }
                return;
            }
            Instruction::Clone { dst, src } => {
                if let Some(fact) = self.fact(*src) {
                    let destination = self.active.get(dst).map(|f| f.id);
                    self.record(index, *src, fact, destination, ValueUseSite::Local, true);
                }
            }
            Instruction::Call { args, .. } | Instruction::CallG { args, .. } => {
                for i in 0..args.count {
                    let reg = Reg(args.start.0 + u16::from(i));
                    if let Some(fact) = self.fact(reg) {
                        self.record(
                            index,
                            reg,
                            fact,
                            None,
                            ValueUseSite::Argument(u16::from(i)),
                            false,
                        );
                    }
                }
            }
            Instruction::Ret { value } => {
                if let Some(fact) = self.fact(*value) {
                    self.record(index, *value, fact, None, ValueUseSite::Return, false);
                }
            }
            _ => {}
        }
        self.forwarded.clear();
    }
}

impl super::VbcCodegen {
    /// A consuming local use selects transfer from the exact source binding's
    /// resolved declaration. This reads declaration facts, not the optional
    /// observation plan; budget exhaustion may discard receipts but cannot
    /// change program semantics. Captured cells and unknown/generic carriers
    /// need their own producer contract and are deliberately excluded.
    pub(super) fn consumes_named_place(&self, expr: &verum_ast::Expr, reg: Reg) -> bool {
        let verum_ast::ExprKind::Path(path) = &expr.kind else {
            return false;
        };
        let [verum_ast::ty::PathSegment::Name(name)] = path.segments.as_slice() else {
            return false;
        };
        let Some(binding) = self.ctx.lookup_var(name.as_str()) else {
            return false;
        };
        if binding.reg != reg
            || binding.is_cell
            || binding.is_pattern_alias
            || !matches!(
                binding.kind,
                super::RegisterKind::Local | super::RegisterKind::Parameter
            )
            || self.ctx.reference_bindings.contains(name.as_str())
            || self.ctx.is_raw_pointer(reg)
        {
            return false;
        }
        let Some(fact) = self.ctx.registers.value_uses.active.get(&reg) else {
            return false;
        };
        fact.id == binding.binding_id
            && fact.declaration_type.is_some()
            && !matches!(fact.declaration_type, Some(TypeRef::Reference { .. }))
            && matches!(
                fact.discipline,
                ResourceDiscipline::Affine | ResourceDiscipline::Linear
            )
    }
    /// The first complete return handoff is a direct, exact affine local.
    /// An observed named affine use through a parameter, aggregate or selected
    /// result needs its own consuming producer and is refused. Fresh or unknown
    /// result origins retain their existing route without new permission.
    /// No native destructor is enabled here.
    pub(super) fn emit_direct_affine_return(
        &mut self,
        expr: &verum_ast::Expr,
        source: Reg,
        uses_named_affine: bool,
    ) -> super::CodegenResult<bool> {
        let constrained = |mode| {
            matches!(
                mode,
                ResourceDiscipline::Affine | ResourceDiscipline::Linear
            )
        };
        let declared = self
            .current_return_ast_type
            .as_ref()
            .map(|ast| self.resolve_signature_type_ref(ast, &self.ctx.current_generic_param_ids));
        let declared_affine = declared.as_ref().is_some_and(|ty| {
            constrained(crate::resource_discipline::resource_discipline_with(
                ty,
                |id| {
                    self.type_index_of(id)
                        .and_then(|index| self.types.get(index))
                },
            ))
        });
        let fact = self.ctx.registers.value_uses.active.get(&source);
        // A result declaration is not a consuming event. In particular, a
        // fresh sum/record result must not be rejected merely because one of
        // its possible components is affine. Only a use of an existing named
        // source obligation enters this bounded return handoff.
        if !fact.is_some_and(|fact| constrained(fact.discipline)) {
            if !declared_affine || !uses_named_affine {
                return Ok(false);
            }
        }
        let bindings = self.ctx.registers.function_exit_bindings();
        let exact_local = bindings.iter().any(|(_, binding)| {
            binding.reg == source && fact.is_some_and(|fact| fact.id == binding.binding_id)
        });
        if !exact_local
            || !self.consumes_named_place(expr, source)
            || self.ctx.has_pending_defers()
            || bindings.iter().any(|(name, binding)| {
                binding.is_cell
                    || binding.is_pattern_alias
                    || self.ctx.current_fn_escaping_vars.contains(name.as_str())
            })
        {
            return Err(super::CodegenError::with_span(super::CodegenErrorKind::UnsupportedExpr(
                "affine return requires a direct local value with no deferred, aliased or escaping cleanup; parameter, aggregate and selected-result handoffs are not yet implemented".into(),
            ), expr.span));
        }

        let returned_fact = fact.cloned();
        // Establish the receiving slot first, then consume the original on this
        // CFG edge only. The other edge still owns its untouched local slot.
        let returned = self.ctx.registers.alloc_fresh();
        self.ctx.emit(Instruction::Mov {
            dst: returned,
            src: source,
        });
        self.ctx.emit(Instruction::LoadUnit { dst: source });
        for (_, binding) in bindings {
            // Reuse the existing lexical cleanup policy, keyed by the actual
            // declaration register even when an inner declaration shadows it.
            // Parameters never enter this inventory and acquire no obligation.
            if self.ctx.is_raw_pointer(binding.reg) {
                continue;
            }
            let borrowed_or_raw = self
                .ctx
                .registers
                .value_uses
                .active
                .get(&binding.reg)
                .filter(|fact| fact.id == binding.binding_id)
                .and_then(|fact| fact.declaration_type.as_ref())
                .is_some_and(|ty| matches!(ty, TypeRef::Reference { .. }));
            if borrowed_or_raw {
                continue;
            }
            self.ctx.emit(Instruction::DropRef { src: binding.reg });
        }
        if let Some(fact) = returned_fact {
            self.ctx.registers.value_uses.forward_result(returned, fact);
        }
        self.ctx.emit(Instruction::Ret { value: returned });
        Ok(true)
    }

    /// Publish only a supported, resolved source declaration; never a VarTypeKind
    /// or rendered bare-leaf guess. Opaque carrier fallbacks remain unknown.
    pub(super) fn publish_binding_type(&mut self, name: &str, ast: &verum_ast::Type) {
        use verum_ast::ty::{GenericArg, TypeKind};
        fn supported(ty: &verum_ast::Type, depth: usize) -> bool {
            if depth > 64 {
                return false;
            }
            match &ty.kind {
                TypeKind::Int
                | TypeKind::Float
                | TypeKind::Bool
                | TypeKind::Text
                | TypeKind::Unit
                | TypeKind::Char
                | TypeKind::Path(_) => true,
                TypeKind::Reference { inner, .. }
                | TypeKind::CheckedReference { inner, .. }
                | TypeKind::UnsafeReference { inner, .. }
                | TypeKind::Slice(inner) => supported(inner, depth + 1),
                TypeKind::Tuple(parts) => parts.iter().all(|p| supported(p, depth + 1)),
                TypeKind::Generic { base, args } => {
                    supported(base, depth + 1)
                        && args
                            .iter()
                            .all(|a| matches!(a, GenericArg::Type(t) if supported(t, depth + 1)))
                }
                _ => false,
            }
        }
        if !supported(ast, 0) {
            return;
        }
        let ty = self.resolve_signature_type_ref(ast, &self.ctx.current_generic_param_ids);
        fn resolved(ty: &TypeRef, depth: usize) -> bool {
            if depth > 64 {
                return false;
            }
            match ty {
                TypeRef::Concrete(id) => {
                    *id != crate::types::TypeId::PTR && *id != crate::types::TypeId::RESERVED
                }
                TypeRef::Reference { inner, .. } | TypeRef::Slice(inner) => {
                    resolved(inner, depth + 1)
                }
                TypeRef::Tuple(parts) => parts.iter().all(|t| resolved(t, depth + 1)),
                TypeRef::Instantiated { base, args } => {
                    *base != crate::types::TypeId::PTR
                        && args.iter().all(|t| resolved(t, depth + 1))
                }
                _ => false,
            }
        }
        if !resolved(&ty, 0) {
            return;
        }
        let discipline = crate::resource_discipline::resource_discipline_with(&ty, |id| {
            self.type_index_of(id)
                .and_then(|index| self.types.get(index))
        });
        if let Some(reg) = self.ctx.lookup_var(name).map(|v| v.reg) {
            self.ctx.registers.value_uses.set_type(reg, ty, discipline);
        }
    }
}

#[cfg(test)]
#[path = "../../tests/codegen/value_use_recorder.rs"]
mod tests;
