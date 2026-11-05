//! Referenced forward callables retain their declaration's contract. A name or
//! argument count is never a substitute for parameter and result types.
use super::{CodegenError, CodegenErrorKind, CodegenResult, VbcCodegen, remap_type_ref_archive};
use crate::module::{ConstId, Constant, FunctionDescriptor, FunctionId, VbcModule};
use crate::types::{ContextRef, ProtocolId, StringId, TypeId};
use verum_common::Text;

impl VbcCodegen {
    fn declared_callable_identity(&self, descriptor: &FunctionDescriptor) -> Option<Text> {
        let name = self.ctx.strings.get(descriptor.name.0 as usize)?;
        let owner = descriptor.origin_module
            .and_then(|id| self.ctx.strings.get(id.0 as usize))
            .map_or(self.config.module_name.as_str(), |owner| owner.as_str());
        Some(crate::module::qualify_module_name(owner, name).into())
    }

    /// Linkage follows the selected declaration, never a consumer's mount alias.
    pub(super) fn declared_callable_name(&self, id: FunctionId) -> Option<Text> {
        self.declared_callable_descriptors.get(&id)
            .and_then(|descriptor| self.declared_callable_identity(descriptor))
    }

    pub(super) fn retain_declared_callable(&mut self, descriptor: FunctionDescriptor) -> CodegenResult<()> {
        if let Some(previous) = self.declared_callable_descriptors.get(&descriptor.id) {
            let previous_owner = self.declared_callable_identity(previous);
            let owner = self.declared_callable_identity(&descriptor);
            if previous_owner != owner || !self.same_declared_callable_contract(previous, &descriptor)?
            {
                return Err(CodegenError::new(CodegenErrorKind::InvalidSourceSignature(format!(
                    "conflicting callable declarations for {:?}: {:?} and {:?}", descriptor.id, previous_owner, owner,
                ))));
            }
        }
        self.declared_callable_descriptors.insert(descriptor.id, descriptor);
        Ok(())
    }

    fn same_declared_callable_contract(
        &self, left: &FunctionDescriptor, right: &FunctionDescriptor,
    ) -> CodegenResult<bool> {
        if CallableContract::from(left) != CallableContract::from(right)
            || left.params.len() != right.params.len()
            || !left.params.iter().zip(&right.params).all(|(left, right)| {
                left.name == right.name && left.type_ref == right.type_ref
                    && left.type_name == right.type_name && left.is_mut == right.is_mut
                    && self.same_declared_default(left.default, right.default)
            })
        { return Ok(false); }
        match (&left.source_signature, &right.source_signature) {
            (Some(left), Some(right)) => verum_ast::source_signature::semantically_equal_signatures(left, right)
                .map_err(|message| CodegenError::new(CodegenErrorKind::InvalidSourceSignature(message.into_string()))),
            // Synthetic descriptors and legacy metadata can carry no source
            // signature; equality never upgrades that absence to source proof.
            (None, None) => Ok(true),
            _ => Ok(false),
        }
    }

    // Defaults are declarations, but pool positions are not identities. Type
    // constants in particular are relocated into fresh slots on each import.
    fn same_declared_default(&self, left: Option<ConstId>, right: Option<ConstId>) -> bool {
        use super::context::ConstantEntry;
        let (left, right) = match (left, right) {
            (None, None) => return true,
            (Some(left), Some(right)) => (left, right),
            _ => return false,
        };
        match (self.ctx.constants.get(left.0 as usize), self.ctx.constants.get(right.0 as usize)) {
            (Some(ConstantEntry::Int(a)), Some(ConstantEntry::Int(b))) => a == b,
            (Some(ConstantEntry::Int128 { raw: a, signed: sa }), Some(ConstantEntry::Int128 { raw: b, signed: sb })) => a == b && sa == sb,
            (Some(ConstantEntry::Float(a)), Some(ConstantEntry::Float(b))) => a.to_bits() == b.to_bits(),
            (Some(ConstantEntry::String(a)), Some(ConstantEntry::String(b))) => {
                matches!((self.ctx.strings.get(*a as usize), self.ctx.strings.get(*b as usize)), (Some(a), Some(b)) if a == b)
            }
            (Some(ConstantEntry::Bytes(a)), Some(ConstantEntry::Bytes(b))) => {
                matches!((self.ctx.bytes.get(*a as usize), self.ctx.bytes.get(*b as usize)), (Some(a), Some(b)) if a == b)
            }
            (Some(ConstantEntry::Type(a)), Some(ConstantEntry::Type(b))) => a == b,
            _ => false,
        }
    }

    pub(super) fn referenced_forward_descriptor(&self, id: FunctionId, name: &str) -> CodegenResult<FunctionDescriptor> {
        let mut descriptor = self.declared_callable_descriptors.get(&id).cloned()
            .ok_or_else(|| CodegenError::new(CodegenErrorKind::InvalidSourceSignature(format!(
                "referenced callable `{name}` has no declaration-owned forward contract",
            ))))?;
        // Source bodies must be loaded and compiled through their ordinary
        // dependency path. Only an actual bodyless intrinsic declaration owns
        // a runtime implementation boundary; empty generated returns do not.
        if descriptor.has_source_body || descriptor.intrinsic_name.is_none() {
            return Err(CodegenError::new(CodegenErrorKind::InvalidSourceSignature(format!(
                "referenced callable `{name}` has no loaded implementation",
            ))));
        }
        descriptor.register_count = u16::try_from(descriptor.params.len().max(1))
            .map_err(|_| CodegenError::internal("forward callable exceeds register space"))?;
        descriptor.locals_count = descriptor.register_count;
        descriptor.bytecode_offset = 0;
        descriptor.bytecode_length = 0;
        descriptor.instructions = None;
        descriptor.value_uses = None;
        Ok(descriptor)
    }

    /// Import only declaration metadata, with every retained ID translated into
    /// this pool. Body receipts, offsets and register hints are not declarations.
    pub(super) fn import_forward_declaration(
        &mut self,
        module: &VbcModule,
        source: &FunctionDescriptor,
        id: FunctionId,
        types: &std::collections::HashMap<u32, u32>,
        lexical_parent: Option<FunctionId>,
    ) -> CodegenResult<FunctionDescriptor> {
        let mut descriptor = source.clone();
        descriptor.id = id;
        descriptor.declaration_identity = source.declaration_identity.as_ref().map(|identity| {
            if identity.lexical_parent.is_some() != lexical_parent.is_some() {
                return Err(CodegenError::internal("imported callable lexical parent has no relocation"));
            }
            Ok(crate::module::CallableDeclarationIdentity {
                source_name: self.import_forward_string(module, identity.source_name)?,
                lexical_parent,
            })
        }).transpose()?;
        descriptor.name = self.import_forward_string(module, source.name)?;
        descriptor.origin_module = Some(match source.origin_module {
            Some(owner) => self.import_forward_string(module, owner)?,
            None => StringId(self.ctx.intern_string_raw(&module.name)),
        });
        descriptor.return_type_name = source.return_type_name
            .map(|name| self.import_forward_string(module, name)).transpose()?;
        descriptor.intrinsic_name = source.intrinsic_name
            .map(|name| self.import_forward_string(module, name)).transpose()?;
        descriptor.parent_type = source.parent_type.map(|parent| TypeId(*types.get(&parent.0).unwrap_or(&parent.0)));
        for parameter in &mut descriptor.params {
            parameter.name = self.import_forward_string(module, parameter.name)?;
            if parameter.type_name != StringId::EMPTY {
                parameter.type_name = self.import_forward_string(module, parameter.type_name)?;
            }
            parameter.type_ref = remap_type_ref_archive(&parameter.type_ref, types);
            parameter.default = parameter.default
                .map(|value| self.import_forward_default(module, value, types)).transpose()?;
        }
        for parameter in &mut descriptor.type_params {
            if parameter.name != StringId::EMPTY {
                parameter.name = self.import_forward_string(module, parameter.name)?;
            }
            for bound in &mut parameter.bounds {
                *bound = ProtocolId(*types.get(&bound.0).unwrap_or(&bound.0));
            }
            parameter.default = parameter.default.as_ref().map(|ty| remap_type_ref_archive(ty, types));
            for bound in &mut parameter.type_bounds {
                *bound = remap_type_ref_archive(bound, types);
            }
        }
        descriptor.semantic_params = super::formal_parameters::remapped_parameters(source, types);
        descriptor.semantic_return = super::formal_parameters::remapped_result(source, types);
        descriptor.return_type = remap_type_ref_archive(&source.return_type, types);
        descriptor.yield_type = source.yield_type.as_ref().map(|ty| remap_type_ref_archive(ty, types));
        for context in &mut descriptor.contexts {
            let name = module.context_names.get(context.0 as usize)
                .and_then(|id| module.strings.get(*id))
                .ok_or_else(|| CodegenError::internal("missing forward callable context declaration"))?;
            *context = ContextRef(self.intern_context_name(name));
        }
        descriptor.bytecode_offset = 0;
        descriptor.bytecode_length = 0;
        descriptor.instructions = None;
        descriptor.value_uses = None;
        descriptor.debug_variables.clear();
        descriptor.register_type_hints.clear();
        descriptor.func_id_base = 0;
        Ok(descriptor)
    }

    fn import_forward_string(&mut self, module: &VbcModule, name: StringId) -> CodegenResult<StringId> {
        let name = module.strings.get(name)
            .ok_or_else(|| CodegenError::internal("missing forward callable declaration string"))?;
        Ok(StringId(self.ctx.intern_string_raw(name)))
    }

    fn import_forward_default(
        &mut self,
        module: &VbcModule,
        id: crate::module::ConstId,
        types: &std::collections::HashMap<u32, u32>,
    ) -> CodegenResult<crate::module::ConstId> {
        let value = module.constants.get(id.0 as usize)
            .ok_or_else(|| CodegenError::internal("missing forward callable default constant"))?;
        Ok(match value {
            Constant::Int(value) => self.ctx.add_const_int(*value),
            Constant::Int128 { raw, signed } => self.ctx.add_const_i128(*raw, *signed),
            Constant::Float(value) => self.ctx.add_const_float(*value),
            Constant::String(name) => {
                let value = module.strings.get(*name)
                    .ok_or_else(|| CodegenError::internal("missing forward callable default text"))?;
                self.ctx.add_const_string(value)
            }
            Constant::Bytes(value) => self.ctx.add_const_bytes(value.clone()),
            Constant::Type(value) => {
                let id = crate::module::ConstId(u32::try_from(self.ctx.constants.len())
                    .map_err(|_| CodegenError::internal("forward default exceeds constant space"))?);
                self.ctx.constants.push(super::context::ConstantEntry::Type(remap_type_ref_archive(value, types)));
                id
            },
            Constant::Function(_) | Constant::Protocol(_) | Constant::Array(_) => {
                return Err(CodegenError::new(CodegenErrorKind::InvalidSourceSignature(
                    "forward callable default requires an unsupported constant relocation".into(),
                )));
            }
        })
    }
}

/// All remapped declaration/ABI facts, independent of the source carrier.
/// Exhaustive destructuring makes new descriptor fields require classification.
#[derive(PartialEq)]
struct CallableContract<'a> {
    declaration_identity: &'a Option<crate::module::CallableDeclarationIdentity>,
    parent_type: Option<TypeId>,
    type_params: &'a [crate::types::TypeParamDescriptor],
    return_type: &'a crate::types::TypeRef,
    contexts: &'a [ContextRef],
    properties: crate::types::PropertySet,
    is_generic: bool,
    visibility: crate::types::Visibility,
    is_generator: bool,
    yield_type: &'a Option<crate::types::TypeRef>,
    calling_convention: crate::module::CallingConvention,
    optimization_hints: &'a crate::module::OptimizationHints,
    is_test: bool,
    is_gpu_only: bool,
    intrinsic_name: Option<StringId>,
    is_const: bool,
    return_type_name: Option<StringId>,
    explicit_type_param_ids: &'a [Option<crate::types::TypeParamId>],
    semantic_params: &'a Option<verum_common::List<Option<crate::types::TypeRef>>>,
    semantic_return: &'a Option<crate::types::TypeRef>,
    has_source_body: bool,
}
impl<'a> From<&'a FunctionDescriptor> for CallableContract<'a> {
    fn from(descriptor: &'a FunctionDescriptor) -> Self {
        let FunctionDescriptor {
            // These identities and semantic values have their own comparison.
            id: _, name: _, origin_module: _, params: _, source_signature: _,
            // Body layout, diagnostics and optimization receipts are not declarations.
            value_uses: _, bytecode_offset: _, bytecode_length: _, locals_count: _,
            register_count: _, max_stack: _, is_inline_candidate: _, suspend_point_count: _,
            instructions: _, func_id_base: _, debug_variables: _, register_type_hints: _,
            parent_type, type_params, return_type, contexts, properties, is_generic,
            visibility, is_generator, yield_type, calling_convention, optimization_hints,
            is_test, is_gpu_only, intrinsic_name, is_const, return_type_name,
            explicit_type_param_ids, semantic_params, semantic_return, has_source_body, declaration_identity,
        } = descriptor;
        Self {
            parent_type: *parent_type, type_params, return_type, contexts, properties: *properties,
            is_generic: *is_generic, visibility: *visibility, is_generator: *is_generator,
            yield_type, calling_convention: *calling_convention, optimization_hints,
            is_test: *is_test, is_gpu_only: *is_gpu_only, intrinsic_name: *intrinsic_name,
            is_const: *is_const, return_type_name: *return_type_name, explicit_type_param_ids,
            semantic_params, semantic_return, has_source_body: *has_source_body, declaration_identity,
        }
    }
}
