//! Declaration facts and emission observations shared by both source producers.
//! Observations do not grant cleanup authority. A consuming source context may
//! use an exact active declaration fact to select its explicit value handoff.
use super::CodegenOptionExt;
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

#[cfg(test)]
#[path = "../../tests/codegen/parameter_aliases.rs"]
mod parameter_aliases;

/// Parameter obligations come from the selected declaration, never its ABI ID.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ParameterOwnership {
    Value,
    Borrowed,
    RawAddress,
}

#[derive(Debug, Clone)]
pub(super) struct BindingFact {
    pub id: BindingId,
    pub declaration_type: Maybe<TypeRef>,
    /// Physical access to a refinement's base does not establish its predicate.
    pub(super) refinement_storage: Maybe<verum_common::Shared<TypeRef>>,
    pub discipline: ResourceDiscipline,
}
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct ReturnUseCapture {
    pub unhandled: bool,
    pub transferred: bool,
    pub call_transferred: bool,
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
    return_use_capture: List<ReturnUseCapture>,
}
impl ValueUseRecorder {
    pub(super) fn captured_declaration(&self, binding: &super::RegisterInfo) -> Maybe<TypeRef> {
        self.active.get(&binding.reg)
            .filter(|fact| fact.id == binding.binding_id)
            .and_then(|fact| fact.declaration_type.clone())
    }

    pub fn allocate(&mut self, reg: Reg) -> BindingId {
        let id = BindingId(self.next_binding);
        self.next_binding = self.next_binding.saturating_add(1);
        self.active.insert(
            reg,
            BindingFact {
                id,
                declaration_type: None,
                refinement_storage: None,
                discipline: ResourceDiscipline::Unknown,
            },
        );
        id
    }
    pub(super) fn forget(&mut self, reg: Reg) -> Option<BindingFact> {
        self.forwarded.remove(&reg);
        self.active.remove(&reg)
    }
    pub fn set_type(&mut self, reg: Reg, ty: TypeRef, discipline: ResourceDiscipline) {
        if let Some(fact) = self.active.get_mut(&reg) {
            fact.declaration_type = Some(ty);
            fact.refinement_storage = None;
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
        self.return_use_capture.push(ReturnUseCapture::default());
        true
    }
    pub fn finish_return_use_capture(&mut self) -> ReturnUseCapture {
        self.return_use_capture.pop().unwrap_or(ReturnUseCapture {
            unhandled: true,
            transferred: false,
            call_transferred: false,
        })
    }
    fn capture_owned_call_argument(&mut self) {
        // The parameter declaration authorizes the argument handoff only.
        // It supplies no producer evidence for the call's result, even when
        // that result has the same type or is stored into a fresh aggregate.
        for captured in &mut self.return_use_capture {
            captured.call_transferred = true;
        }
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
                captured.unhandled = true;
            }
        }
    }
    fn capture_return_inputs(&mut self, instruction: &Instruction, transferred: Maybe<Reg>) {
        if self.return_use_capture.is_empty() {
            return;
        }
        match instruction {
            Instruction::Mov { src, .. } if transferred == Some(*src) => {
                for captured in &mut self.return_use_capture { captured.transferred = true; }
            }
            // A named destination is handled by the existing local producer.
            // A temporary can feed a selected result, aggregate or call instead.
            Instruction::Mov { dst, src } if !self.active.contains_key(dst) => {
                self.capture_affine_operand(*src);
            }
            Instruction::Clone { src, .. } | Instruction::CloneTyped { src, .. } => self.capture_affine_operand(*src),
            Instruction::SetF { value, .. } | Instruction::SetVariantData { value, .. } => {
                // The owning store emitter clears this exact source immediately
                // after installation. Its operand is no longer an outstanding
                // named return obligation; other operands remain guarded.
                if transferred == Some(*value) {
                    for captured in &mut self.return_use_capture {
                        captured.transferred = true;
                    }
                } else {
                    self.capture_affine_operand(*value);
                }
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
        self.observe_with_transfer(instruction, index, None);
    }
    pub(super) fn observe_with_transfer(
        &mut self, instruction: &Instruction, index: usize, transferred: Maybe<Reg>,
    ) {
        self.capture_return_inputs(instruction, transferred);
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
            Instruction::Clone { dst, src } | Instruction::CloneTyped { dst, src, .. } => {
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

impl crate::associated_types::DeclarationLookup for super::VbcCodegen {
    fn descriptor(&self, id: crate::types::TypeId) -> Option<&crate::types::TypeDescriptor> {
        self.type_by_id(id)
    }
    fn spelling(&self, id: crate::types::StringId) -> Option<&str> {
        self.ctx.strings.get(id.0 as usize).map(|name| name.as_str())
    }
}

impl super::VbcCodegen {
    pub(super) fn value_resource_discipline(&self, ty: &TypeRef) -> ResourceDiscipline {
        crate::resource_discipline::resource_discipline_with(
            self, ty, |id| self.declared_runtime_carrier(id),
        )
    }

    pub(super) fn bind_capture_declarations(
        &mut self,
        captures: &[(String, bool)],
        declarations: &[Maybe<TypeRef>],
    ) {
        for ((name, _), declaration) in captures.iter().zip(declarations) {
            if let (Some(binding), Some(ty)) = (self.ctx.lookup_var(name), declaration) {
                let reg = binding.reg;
                let discipline = self.value_resource_discipline(ty);
                self.ctx.registers.value_uses.set_type(reg, ty.clone(), discipline);
            }
        }
    }

    /// A consuming local use selects transfer from the exact source binding's
    /// resolved declaration. This reads declaration facts, not the optional
    /// observation plan; budget exhaustion may discard receipts but cannot
    /// change program semantics. Captured cells and unknown/generic carriers
    /// need their own producer contract and are deliberately excluded.
    pub(super) fn consumes_named_place(&self, expr: &verum_ast::Expr, reg: Reg) -> bool {
        Self::named_place_binding(expr).is_some_and(|name| self.consumes_named_binding(name, reg))
    }

    /// A binding owning an indirect value stores the handle itself. Borrow
    /// that slot so replacing the entire value preserves its allocator data.
    pub(super) fn borrows_handle_binding(&self, expr: &verum_ast::Expr, reg: Reg) -> bool {
        Self::named_place_binding(expr)
            .and_then(|name| self.named_ownership_carrier(name, reg))
            .and_then(|(ty, _)| ty.base_type_id())
            .is_some_and(|id| match id.builtin_value_storage() {
                Some(crate::types::BuiltinValueStorage::Text) => true,
                Some(crate::types::BuiltinValueStorage::Immediate) => false,
                None => self.type_by_id(id).is_some_and(|decl|
                    matches!(decl.kind, crate::types::TypeKind::Record | crate::types::TypeKind::Sum)),
            })
    }

    /// A generic source retains its exact declared parameter identity.
    /// The frame witness selects copy or transfer; missing witnesses grant neither.
    pub(super) fn generic_named_place(&self, expr: &verum_ast::Expr, reg: Reg) -> Maybe<TypeRef> {
        self.generic_named_binding(Self::named_place_binding(expr)?, reg)
    }
    pub(super) fn generic_named_binding(&self, name: &str, reg: Reg) -> Maybe<TypeRef> {
        let (ty, _) = self.named_ownership_carrier(name, reg)?;
        ty.is_generic().then(|| ty.clone())
    }

    pub(super) fn declared_named_binding_type(&self, name: &str, reg: Reg) -> Maybe<TypeRef> {
        let binding = self.ctx.lookup_var(name)?;
        if binding.reg != reg || binding.is_cell { return None; }
        let ty = self.ctx.registers.value_uses.captured_declaration(binding)?;
        // An observed slot carries its own declaration without transferring
        // the scrutinee. An explicit alias needs evidence of a reference.
        (binding.pattern_alias != Some(super::registers::PatternAlias::Explicit)
            || matches!(ty, TypeRef::Reference { .. })).then_some(ty)
    }

    /// An exact type witness, not an object header, decides the generic branch.
    /// Unknown is refused before either a copy or an ownership handoff occurs.
    fn resource_is_unrestricted(&mut self, ty: TypeRef) -> Reg {
        use crate::instruction::CompareOp;
        let mode = self.ctx.registers.alloc_fresh();
        self.ctx.emit(Instruction::TypeResource { dst: mode, type_ref: ty });
        let constant = self.ctx.registers.alloc_fresh();
        let predicate = self.ctx.registers.alloc_fresh();
        self.ctx.emit(Instruction::LoadI { dst: constant, value: ResourceDiscipline::Unknown as u8 as i64 });
        self.ctx.emit(Instruction::CmpI { dst: predicate, op: CompareOp::Ne, a: mode, b: constant });
        let message_id = self.intern_string("generic ownership requires a resolved declaration witness");
        self.ctx.emit(Instruction::Assert { cond: predicate, message_id });
        self.ctx.emit(Instruction::LoadI { dst: constant, value: ResourceDiscipline::Unrestricted as u8 as i64 });
        self.ctx.emit(Instruction::CmpI { dst: predicate, op: CompareOp::Eq, a: mode, b: constant });
        self.ctx.free_temp(mode);
        self.ctx.free_temp(constant);
        predicate
    }

    /// Establish a receiving value before clearing an affine source. The shared
    /// VBC producer emits the same branch for interpretation and specialization.
    pub(super) fn emit_generic_value_handoff(&mut self, source: Reg, ty: TypeRef, copy: bool) -> Reg {
        let ty = self.runtime_member_witness(ty);
        let result = self.ctx.registers.alloc_fresh();
        let unrestricted = self.resource_is_unrestricted(ty.clone());
        let copy_label = self.ctx.new_label("resource_copy");
        let done = self.ctx.new_label("resource_received");
        self.ctx.emit_forward_jump(&copy_label, |offset| Instruction::JmpIf { cond: unrestricted, offset });
        self.ctx.emit_transferring_value(result, source);
        self.ctx.emit_forward_jump(&done, |offset| Instruction::Jmp { offset });
        self.ctx.define_label(&copy_label);
        self.ctx.emit(if copy { Instruction::CloneTyped { dst: result, src: source, type_ref: ty } }
            else { Instruction::Mov { dst: result, src: source } });
        self.ctx.define_label(&done);
        self.ctx.free_temp(unrestricted);
        result
    }

    /// One declaration-owned classifier for callee inventory and outgoing
    /// handoffs. A complete semantic slot is authoritative. Only an explicitly
    /// written raw type can explain an absent full semantic pointer witness.
    /// Aliases follow exact declaration IDs, never names in the caller scope.
    pub(super) fn parameter_ownership(
        &self,
        semantic: Option<&TypeRef>,
        written: Option<&verum_ast::Type>,
    ) -> super::CodegenResult<ParameterOwnership> {
        use verum_ast::ty::TypeKind;
        let raw_mutability = |ty: &verum_ast::Type| match ty.kind {
            TypeKind::Pointer { mutable, .. } | TypeKind::VolatilePointer { mutable, .. } => Some(mutable),
            _ => None,
        };
        let Some(mut witness) = semantic.cloned() else {
            return if written.and_then(raw_mutability).is_some() { Ok(ParameterOwnership::RawAddress) }
                else { Err(super::CodegenError::unsupported_expr(
                    "owned parameter requires a complete declaration and binding inventory")) };
        };
        let mut seen = verum_common::Set::new();
        for _ in 0..crate::types::MAX_TYPE_REF_DEPTH {
            let (id, arguments) = match &witness {
                TypeRef::Reference { .. } => return Ok(ParameterOwnership::Borrowed),
                TypeRef::Concrete(crate::types::TypeId::RESERVED) => return Err(
                    super::CodegenError::unsupported_expr("parameter ownership requires a declared type")),
                TypeRef::Concrete(id) => (*id, &[][..]),
                TypeRef::Instantiated { base, args } => (*base, args.as_slice()),
                _ => return Ok(ParameterOwnership::Value),
            };
            let Some(declaration) = self.type_by_id(id)
                .filter(|declaration| declaration.kind == crate::types::TypeKind::Alias)
                else { return Ok(ParameterOwnership::Value) };
            if !seen.insert(id) || arguments.len() != declaration.type_params.len() {
                return Err(super::CodegenError::unsupported_expr(
                    "parameter ownership requires a complete acyclic alias declaration"));
            }
            let raw = match declaration.source_declaration.as_ref() {
                Some(source) => match &source.body {
                    verum_ast::TypeDeclBody::Alias(target) => raw_mutability(target),
                    _ => None,
                },
                None => declaration.alias_target_name
                    .and_then(|id| self.ctx.strings.get(id.0 as usize))
                    .and_then(|source| verum_fast_parser::Parser::new(source).parse_type().ok())
                    .and_then(|target| raw_mutability(&target)),
            };
            if let Some(mutable) = raw {
                // Raw pointers retain their pointee and permission in the
                // shared Tier2 reference representation. A present target is
                // required evidence, not a contradiction or an owning value.
                let permission = if mutable { crate::types::Mutability::Mutable }
                    else { crate::types::Mutability::Immutable };
                if !matches!(declaration.alias_target.as_ref(),
                    Some(TypeRef::Reference { tier: crate::types::CbgrTier::Tier2, mutability, .. })
                        if *mutability == permission)
                {
                    return Err(super::CodegenError::unsupported_expr(
                        "raw address alias contradicts its semantic target"));
                }
                return Ok(ParameterOwnership::RawAddress);
            }
            let target = declaration.alias_target.as_ref()
                .or_internal("parameter ownership requires its resolved alias declaration")?;
            witness = crate::mono::TypeSubstitution::new(&declaration.type_params, arguments)
                .apply(target).map_err(|error| super::CodegenError::unsupported_expr(error.to_string()))?;
        }
        Err(super::CodegenError::unsupported_expr("parameter ownership alias depth exceeds the type limit"))
    }

    /// The selected callable owns this semantic slot. No ABI carrier, method
    /// spelling or destructor lookup can authorize argument ownership.
    pub(super) fn emit_owned_call_argument(
        &mut self,
        callable: &super::FunctionInfo,
        index: usize,
        argument: &verum_ast::Expr,
        source: Reg,
        actual: Option<&TypeRef>,
    ) -> super::CodegenResult<Reg> {
        if callable.is_generator || callable.external_intrinsic().is_some()
            || callable.foreign_declaration.is_some() {
            return Ok(source);
        }
        let parameter = self.ctx.semantic_fn_params.get(&callable.id)
            .and_then(|params| params.get(index)).cloned().flatten()
            .or_else(|| self.refined_parameter_carrier(callable, index));
        // The name registry may contain a forward declaration or a flattened
        // overload key. Only the selected callable's retained source signature
        // can establish a raw parameter when no semantic TypeRef exists.
        let written = self.declared_callable_descriptors.get(&callable.id)
            .and_then(|declaration| declaration.source_signature.as_ref())
            .and_then(|signature| signature.params.get(index))
            .and_then(|parameter| match &parameter.kind {
                verum_ast::FunctionParamKind::Regular { ty, .. } => Some(ty),
                _ => None,
            });
        if !self.ctx.registers.emits_owned_parameter_calls {
            // Synthetic and generator bodies keep their existing handoff. A new
            // argument copy would silently change an unflagged caller's ABI.
            let owns = (callable.has_source_body || self.functions.iter().any(|function|
                function.descriptor.id == callable.id && function.descriptor.has_source_body))
                && self.parameter_ownership(parameter.as_ref(), written)
                    .is_ok_and(|ownership| ownership == ParameterOwnership::Value);
            if !owns { return Ok(source); }
            if let Some(ty) = self.generic_named_place(argument, source) {
                self.ctx.registers.value_uses.capture_owned_call_argument();
                return Ok(self.emit_generic_value_handoff(source, ty, false));
            }
            if self.consumes_named_place(argument, source) {
                self.ctx.registers.value_uses.capture_owned_call_argument();
                let received = self.ctx.registers.alloc_fresh();
                self.ctx.emit_transferring_value(received, source);
                return Ok(received);
            }
            return Ok(source);
        }
        if self.parameter_ownership(parameter.as_ref(), written)? != ParameterOwnership::Value {
            return Ok(source);
        }
        self.emit_declared_call_argument(parameter.as_ref(), argument, source, actual)
    }

    /// The one argument boundary used by direct, method and callable-value
    /// dispatch. A place keeps its owner only after a declared value copy;
    /// a temporary or an exclusive named value transfers exactly once.
    pub(super) fn emit_declared_call_argument(
        &mut self,
        parameter: Option<&TypeRef>,
        argument: &verum_ast::Expr,
        source: Reg,
        actual: Option<&TypeRef>,
    ) -> super::CodegenResult<Reg> {
        if !self.ctx.registers.emits_owned_parameter_calls { return Ok(source); }
        if self.parameter_ownership(parameter, None)? != ParameterOwnership::Value { return Ok(source); }
        // A refined named place can supply its storage for this copy while
        // its full semantic domain remains absent from the callable metadata.
        let storage = actual.is_none().then(|| {
            let name = Self::named_place_binding(argument)?;
            self.named_ownership_carrier(name, source).map(|(ty, _)| ty.clone())
        }).flatten();
        let actual = actual.or(storage.as_ref());
        if actual.is_some_and(|ty| self.parameter_ownership(Some(ty), None)
            .is_ok_and(|ownership| ownership != ParameterOwnership::Value)) || self.ctx.is_raw_pointer(source) {
            return Ok(source);
        }
        if let Some(ty) = self.generic_named_place(argument, source) {
            self.ctx.registers.value_uses.capture_owned_call_argument();
            return Ok(self.emit_generic_value_handoff(source, ty, true));
        }
        if self.consumes_named_place(argument, source) {
            self.ctx.registers.value_uses.capture_owned_call_argument();
            let received = self.ctx.registers.alloc_fresh();
            self.ctx.emit_transferring_value(received, source);
            return Ok(received);
        }
        let copy = self.binds_a_copy_of_a_place(argument, source, actual)?;
        if copy {
            let type_ref = actual.cloned().ok_or_else(|| super::CodegenError::with_span(
                super::CodegenErrorKind::Internal(format!(
                    "call argument `{}` requires its retained source type for value copying",
                    Self::named_place_binding(argument).unwrap_or("<projection>"),
                )), argument.span,
            ))?;
            let received = self.ctx.registers.alloc_fresh();
            self.ctx.emit(Instruction::CloneTyped { dst: received, src: source,
                type_ref });
            return Ok(received);
        }
        Ok(source)
    }

    /// Callee inventory is derived from exactly the semantic parameter which
    /// the argument boundary uses, independently of mutability and use count.
    pub(super) fn establish_parameter_owner(&mut self, reg: Reg, ty: &TypeRef) -> super::CodegenResult<()> {
        if self.parameter_ownership(Some(ty), None)? == ParameterOwnership::Value {
            let fact = self.ctx.registers.value_uses.active.get(&reg)
                .or_internal("parameter ownership requires its active binding")?;
            self.ctx.registers.parameter_owners.insert(reg, (fact.id, ty.clone()));
        }
        Ok(())
    }

    fn emit_parameter_cleanup(&mut self, returned: Option<BindingId>) {
        for (_, binding) in self.ctx.registers.function_exit_bindings() {
            let Some((identity, ty)) = self.ctx.registers.parameter_owners.get(&binding.reg).cloned()
                else { continue };
            if binding.kind != super::RegisterKind::Parameter || identity != binding.binding_id
                || returned == Some(identity) || binding.is_cell || binding.pattern_alias.is_some() {
                continue;
            }
            // Generic parameters carry the selected caller's exact witness.
            // A cleared slot is Unit and typed cleanup is idempotent for it.
            self.ctx.emit(Instruction::DropTyped { src: binding.reg, type_ref: ty });
        }
    }

    pub(super) fn emit_parameter_return(&mut self, value: Option<Reg>) {
        let returned = value.and_then(|reg| self.ctx.registers.value_uses.fact(reg)).map(|fact| fact.id);
        self.emit_parameter_cleanup(returned);
        self.ctx.emit(match value {
            Some(value) => Instruction::Ret { value },
            None => Instruction::RetV,
        });
    }

    /// Both generic and concrete transfers require the same active declaration.
    /// Borrowed aliases, captured cells and raw views never acquire its ownership.
    /// Instantiated destructor bodies receive the lexical type arguments used
    /// at construction. Native sum producers use the same declared tags and payloads.
    /// An erased heap header only identifies the declaration.
    fn local_drop_declaration(&self, declared: &TypeRef) -> TypeRef {
        self.expand_declared_alias_while(declared.clone(), |alias| {
            alias.drop_fn.is_none()
                && alias.resource_discipline == ResourceDiscipline::Unrestricted
        }).unwrap_or_else(|| declared.clone())
    }

    pub(super) fn emit_local_drop(&mut self, src: Reg, declared: Option<&TypeRef>) {
        let declared = declared.map(|ty| self.local_drop_declaration(ty));
        if let Some(ty) = declared.as_ref()
            && (matches!(ty, TypeRef::Reference { .. } | TypeRef::Tuple(_) | TypeRef::Generic(_))
                || matches!(ty, TypeRef::Concrete(base)
                    if crate::drop_plan::builtin_drop_plan(*base).is_some())
                || matches!(ty, TypeRef::Concrete(base) | TypeRef::Instantiated { base, .. }
                    if self.type_by_id(*base).is_some_and(|declaration|
                        (!base.is_builtin() && !base.is_semantic_type())
                        || (*base == crate::types::TypeId::DEQUE && declaration.drop_fn.is_some())
                        || declaration.kind == crate::types::TypeKind::Sum)))
        {
            // Structural owners need their declaration even without explicit
            // Drop glue: a sum's active payload and generic owned fields cannot
            // be recovered from the erased record layout. References retire
            // only their binding slot and leave the referent live.
            self.ctx.emit(Instruction::DropTyped { src, type_ref: ty.clone() });
        } else {
            self.ctx.emit(Instruction::DropRef { src });
        }
    }

    /// A read-only operation borrows places. A separately evaluated temporary
    /// retains cleanup from its captured source declaration after the read.
    /// Register recycling alone neither transfers nor retires that ownership.
    pub(super) fn retire_observed_operand(
        &mut self, expr: &verum_ast::Expr, reg: Reg, declared: Option<&TypeRef>,
    ) {
        use verum_ast::{ExprKind, UnOp};
        let named = Self::named_place_binding(expr)
            .is_some_and(|name| self.ctx.lookup_var(name).is_some());
        let projection = !self.is_declared_unit_constructor_value(Self::normalized_place(expr))
            && matches!(&Self::normalized_place(expr).kind,
            ExprKind::Field { .. } | ExprKind::TupleIndex { .. } | ExprKind::Index { .. }
                | ExprKind::Unary { op: UnOp::Deref, .. });
        if !named && !projection && let Some(declared) = declared {
            self.emit_local_drop(reg, Some(declared));
        }
        self.ctx.free_temp(reg);
    }

    /// Every aggregate takes a declared value: places copy, exclusive names
    /// transfer, and fresh values already own their storage. The concrete store
    /// only chooses a destination; it cannot change that ownership contract.
    pub(super) fn emit_declared_value_store(
        &mut self,
        expr: &verum_ast::Expr,
        value: Reg,
        declared: Option<&TypeRef>,
        store: impl FnOnce(Reg) -> Instruction,
    ) -> super::CodegenResult<()> {
        if let Some(ty) = self.generic_named_place(expr, value) {
            let received = self.emit_generic_value_handoff(value, ty, true);
            self.ctx.emit(store(received));
            self.ctx.free_temp(received);
            return Ok(());
        }
        if self.consumes_named_place(expr, value) {
            match store(value) {
                Instruction::Mov { dst, src } => self.ctx.emit_transferring_value(dst, src),
                instruction => self.ctx.emit_transferring_store(instruction, value),
            }
            return Ok(());
        }
        let received = self.copy_place_if_needed(expr, value, declared)?;
        self.ctx.emit(store(received));
        if received != value { self.ctx.free_temp(received); }
        Ok(())
    }

    /// Receive a value from a selected result edge before its source scope exits.
    pub(super) fn receive_owned_place(
        &mut self, expr: &verum_ast::Expr, value: Reg, declared: Option<&TypeRef>,
        demand: super::ResultDemand,
    ) -> super::CodegenResult<Reg> {
        // The callable exit checks ownership across every live local and defer.
        // Keep an exclusive source visible to that check. A transfer on one
        // branch does not certify the whole function's return.
        if self.consumes_named_place(expr, value) {
            if demand.is_callable_return() { return Ok(value); }
            let received = self.ctx.alloc_temp();
            self.ctx.emit_transferring_value(received, value);
            return Ok(received);
        }
        if demand.is_callable_return() && let Some(ty) = self.generic_named_place(expr, value) {
            // Only the concrete witness can license a copy. A conditional
            // return has no unconditional transfer receipt for an exclusive T.
            return Ok(self.copy_declared_value(value, ty));
        }
        self.copy_place_if_needed(expr, value, declared)
    }

    fn copy_place_if_needed(
        &mut self, expr: &verum_ast::Expr, value: Reg, declared: Option<&TypeRef>,
    ) -> super::CodegenResult<Reg> {
        Ok(if let Some(ty) = self.generic_named_place(expr, value) {
            self.emit_generic_value_handoff(value, ty, true)
        } else if self.binds_a_copy_of_a_place(expr, value, declared)? {
            self.copy_place_storage(value, declared, Self::named_place_binding(expr), expr.span)?
        } else { value })
    }

    /// A refinement keeps its predicate in the source signature. Only the
    /// physical copy may use the carrier of that exact, live binding.
    pub(super) fn copy_place_storage(
        &mut self, value: Reg, declared: Option<&TypeRef>, name: Option<&str>, span: verum_ast::Span,
    ) -> super::CodegenResult<Reg> {
        let ty = declared.or_else(|| {
            let (carrier, discipline) = self.named_ownership_carrier(name?, value)?;
            (discipline == ResourceDiscipline::Unrestricted).then_some(carrier)
        }).cloned().ok_or_else(|| super::CodegenError::with_span(
            super::CodegenErrorKind::InvalidSourceSignature(
                "value copy requires its retained source declaration".into(),
            ), span,
        ))?;
        Ok(self.copy_declared_value(value, ty))
    }

    fn copy_declared_value(&mut self, value: Reg, ty: TypeRef) -> Reg {
        let received = self.ctx.alloc_temp();
        self.ctx.emit(Instruction::CloneTyped { dst: received, src: value, type_ref: ty });
        received
    }

    /// Overwriting a live place retires its previous owner only after publishing
    /// the prepared replacement. The declaration selects cleanup; the destination
    /// operations supply storage, never an inferred allocator or runtime type.
    pub(super) fn emit_declared_replacement(
        &mut self,
        value: Reg,
        declared: Option<&TypeRef>,
        read: impl FnOnce(Reg) -> Instruction,
        write: impl FnOnce(Reg) -> Instruction,
    ) -> super::CodegenResult<()> {
        self.retire_displaced_value(declared, |codegen, previous| {
            if let Some(dst) = previous { codegen.ctx.emit(read(dst)); }
            codegen.ctx.emit(write(value));
        })
    }

    pub(super) fn replace_referent(
        &mut self, reference: Reg, value: Reg, declared: Option<&TypeRef>,
    ) -> super::CodegenResult<()> {
        self.retire_displaced_value(declared, |codegen, previous| {
            if let Some(dst) = previous {
                codegen.emit_value_exchange(dst, reference, value);
            } else {
                codegen.ctx.emit(Instruction::DerefMut { ref_reg: reference, value });
            }
        })
    }

    /// Both assignment and the source-level exchange intrinsic address one
    /// initialized slot. Retain its old owner before invoking any cleanup.
    pub(super) fn emit_value_exchange(&mut self, dst: Reg, place: Reg, value: Reg) {
        let mut operands = Default::default();
        for reg in [dst, place, value] { crate::encoding::encode_reg(reg, &mut operands); }
        self.ctx.emit(Instruction::MemExtended {
            sub_op: crate::instruction::MemSubOpcode::Replace as u8, operands,
        });
    }

    fn retire_displaced_value(
        &mut self, declared: Option<&TypeRef>, exchange: impl FnOnce(&mut Self, Option<Reg>),
    ) -> super::CodegenResult<()> {
        let declared = declared.map(|ty| self.local_drop_declaration(ty))
            .ok_or_else(|| super::CodegenError::new(
                super::CodegenErrorKind::InvalidSourceSignature(
                    "replacement requires its destination declaration".into(),
                ),
            ))?;
        let previous = (!crate::drop_plan::has_trivial_cleanup(&declared)).then(|| self.ctx.alloc_temp());
        exchange(self, previous);
        if let Some(previous) = previous {
            self.emit_local_drop(previous, Some(&declared));
            self.ctx.free_temp(previous);
        }
        Ok(())
    }

    fn named_ownership_carrier(&self, name: &str, reg: Reg) -> Maybe<(&TypeRef, ResourceDiscipline)> {
        let binding = self.ctx.lookup_var(name)?;
        if binding.reg != reg || binding.is_cell || binding.pattern_alias.is_some()
            || !matches!(binding.kind, super::RegisterKind::Local | super::RegisterKind::Parameter)
            || self.ctx.reference_bindings.contains(name) || self.ctx.is_raw_pointer(reg) {
            return None;
        }
        let fact = self.ctx.registers.value_uses.active.get(&reg)?;
        if fact.id != binding.binding_id { return None; }
        let carrier = fact.declaration_type.as_ref().or(fact.refinement_storage.as_deref())?;
        (!matches!(carrier, TypeRef::Reference { .. })).then_some((carrier, fact.discipline))
    }

    /// Shorthand fields use the same exact binding contract as expression fields.
    pub(super) fn consumes_named_binding(&self, name: &str, reg: Reg) -> bool {
        self.named_ownership_carrier(name, reg).is_some_and(|(_, discipline)|
            matches!(discipline, ResourceDiscipline::Affine | ResourceDiscipline::Linear))
    }

    /// A pattern alias has no owned cleanup only when the current binding's
    /// declaration is a reference. Names and recycled registers grant no such
    /// permission; an unresolved alias remains an outstanding obligation.
    fn binding_is_declared_reference(&self, binding: &super::RegisterInfo) -> bool {
        self.ctx.registers.value_uses.active.get(&binding.reg).is_some_and(|fact|
            fact.id == binding.binding_id
                && matches!(fact.declaration_type, Some(TypeRef::Reference { .. })))
    }

    /// Duplication discipline does not describe storage lifetime: an ordinary
    /// copyable value may own Text or invoke Drop. Only the shared primitive
    /// storage plan and an explicit reference establish trivial cleanup here.
    pub(super) fn declaration_requires_drop(&self, declaration: &TypeRef) -> bool {
        !crate::drop_plan::has_trivial_cleanup(&self.local_drop_declaration(declaration))
    }
    /// Emit the exit edge without popping compile-time scopes shared with
    /// the untaken path. A result must survive deferred code that changes a local.
    pub(super) fn emit_lexical_return(
        &mut self, value: Option<Reg>, preserved: Option<Reg>, residual: bool, independent: bool,
    ) {
        let returned = value.and_then(|reg| self.ctx.registers.value_uses.fact(reg)).map(|fact| fact.id);
        let has_defers = self.ctx.has_pending_defers();
        let value = if has_defers {
            value.map(|src| {
                let dst = self.ctx.registers.alloc_fresh();
                self.ctx.emit(Instruction::Mov { dst, src });
                dst
            })
        } else { value };
        let has_owners = independent && self.ctx.registers.value_uses.active.values().any(|fact| {
            fact.declaration_type.as_ref().is_some_and(|ty| self.declaration_requires_drop(ty))
        });
        if has_defers || has_owners {
            let mut bindings = self.ctx.registers.function_exit_bindings().into_iter().peekable();
            for level in (0..=self.ctx.registers.scope_level()).rev() {
                if has_defers {
                    for instruction in self.ctx.exit_edge_defers(level, residual) {
                        self.ctx.emit(instruction);
                    }
                }
                while let Some((name, binding)) = bindings.next_if(|(_, b)| b.scope_level == level) {
                    if binding.kind == super::RegisterKind::Parameter
                        || !has_owners || preserved == Some(binding.reg) || binding.is_cell
                        || binding.pattern_alias.is_some() || self.ctx.is_raw_pointer(binding.reg)
                        || self.ctx.current_fn_escaping_vars.contains(name.as_str())
                    {
                        continue;
                    }
                    if let Some(fact) = self.ctx.registers.value_uses.active.get(&binding.reg)
                        && fact.id == binding.binding_id
                        && let Some(ty) = fact.declaration_type.clone()
                        && self.declaration_requires_drop(&ty)
                    {
                        self.emit_local_drop(binding.reg, Some(&ty));
                    }
                }
            }
        }
        self.emit_parameter_cleanup(returned);
        self.ctx.emit(match value {
            Some(value) => Instruction::Ret { value },
            None => Instruction::RetV,
        });
    }

    /// Transfer a direct affine local or a fresh aggregate whose owning stores
    /// already consumed their inputs. Unproven selected results and calls remain
    /// refused. Both routes discharge the remaining lexical locals on this edge.
    /// No native destructor is enabled here.
    pub(super) fn emit_direct_affine_return(
        &mut self,
        expr: &verum_ast::Expr,
        source: Reg,
        capture: ReturnUseCapture,
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
            .map(|ast| self.resolve_signature_type_ref(ast, &self.ctx.current_generic_param_ids)).transpose()?;
        let declared_discipline = declared.as_ref().map(|ty| {
            self.value_resource_discipline(ty)
        });
        let declared_affine = declared_discipline.is_some_and(constrained);
        let fact = self.ctx.registers.value_uses.active.get(&source);
        // A result declaration is not a consuming event. In particular, a
        // fresh sum/record result must not be rejected merely because one of
        // its possible components is affine. Only a use of an existing named
        // source obligation enters this bounded return handoff.
        if !fact.is_some_and(|fact| constrained(fact.discipline)) && !capture.transferred {
            if !declared_affine || !capture.unhandled { return Ok(false); }
        }
        let bindings = self.ctx.registers.function_exit_bindings();
        let exact_local = bindings.iter().any(|(_, binding)| {
            binding.reg == source && fact.is_some_and(|fact| fact.id == binding.binding_id)
        });
        let direct_local = exact_local && self.consumes_named_place(expr, source);
        // A consumed argument cannot establish a resource-bearing result.
        // An unrestricted result needs no resource handoff of its own, but
        // still leaves the remaining lexical locals to clean up on this edge.
        let supported_temporary = capture.transferred && !capture.unhandled && fact.is_none()
            && (!capture.call_transferred || declared_discipline == Some(ResourceDiscipline::Unrestricted));
        let unsupported = if !direct_local && !supported_temporary {
            Some(verum_common::Text::from("the selected result has no proved ownership handoff"))
        } else if self.ctx.has_pending_defers() {
            Some(verum_common::Text::from("deferred cleanup has no result-preservation contract"))
        } else {
            bindings.iter().find_map(|(name, binding)| {
                if binding.is_cell {
                    Some(format!("captured cell `{name}` has no owned exit contract").into())
                } else if binding.pattern_alias == Some(super::registers::PatternAlias::Explicit)
                    && !self.binding_is_declared_reference(binding) {
                    Some(format!("pattern alias `{name}` has no current reference declaration").into())
                } else if self.ctx.current_fn_escaping_vars.contains(name.as_str()) {
                    Some(format!("escaping binding `{name}` has no proved exit contract").into())
                } else { None }
            })
        };
        if let Some(reason) = unsupported {
            return Err(super::CodegenError::with_span(super::CodegenErrorKind::UnsupportedExpr(
                format!("affine return requires a direct local value or a proved producer handoff: {reason}").into(),
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
            // Declared parameter owners share this exit inventory with locals.
            if self.ctx.is_raw_pointer(binding.reg) {
                continue;
            }
            if self.binding_is_declared_reference(&binding) {
                continue;
            }
            let declared = self.ctx.registers.value_uses.active.get(&binding.reg)
                .and_then(|fact| fact.declaration_type.clone());
            self.emit_local_drop(binding.reg, declared.as_ref());
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
        let Some(ty) = self.source_type_in_current_scope(ast) else { return };
        self.publish_resolved_binding_type(name, &ty);
    }

    /// Keep method selection and ownership tied to the same declared binding.
    /// A widened reference must replace the initializer's narrower owner.
    pub(super) fn publish_resolved_binding_type(&mut self, name: &str, ty: &TypeRef) {
        if let Some(declared) = self.declaration_type_name(ty, 0) {
            self.ctx.variable_type_names.insert(name.to_owned(), declared);
        } else {
            self.ctx.variable_type_names.remove(name);
        }
        let discipline = self.value_resource_discipline(ty);
        if let Some(reg) = self.ctx.lookup_var(name).map(|v| v.reg) {
            self.ctx.registers.value_uses.set_type(reg, ty.clone(), discipline);
        }
    }
}

#[cfg(test)]
#[path = "../../tests/codegen/value_use_recorder.rs"]
mod tests;
