//! Source declaration interfaces for bootstrap dependencies whose bodies are not
//! compiled yet. Each interface owns its type IDs; the normal dependency importer
//! remaps them. These modules contain no executable function implementations.
use super::{CodegenResult, FunctionInfo, VbcCodegen};
use crate::module::{ParamDescriptor, VbcFunction, VbcModule};
use crate::types::{StringId, TypeId, TypeParamDescriptor, TypeRef};
use verum_ast::{FunctionDecl, FunctionParamKind, Module};
use verum_common::List;

impl VbcCodegen {
    /// Collect source signatures and nominal layouts without lowering bodies or
    /// global initializers. Import this metadata through the same scoped-ID path
    /// as already compiled dependencies; never merge it as executable bytecode.
    pub fn collect_declaration_interface(
        &mut self,
        sources: &[&Module],
    ) -> CodegenResult<VbcModule> {
        self.collecting_declaration_interface = true;
        let collected = self.collect_unit_declarations(sources);
        self.collecting_declaration_interface = false;
        collected?;
        let mut module = self.build_module()?;
        module.bytecode.clear();
        module.global_ctors.clear();
        for function in &mut module.functions {
            function.bytecode_offset = 0;
            function.bytecode_length = 0;
            function.value_uses = None;
        }
        Ok(module)
    }

    pub(super) fn capture_declaration_interface(
        &mut self,
        source: &FunctionDecl,
        info: &FunctionInfo,
        name: &str,
        parent: Option<&str>,
        dense: &std::collections::HashMap<String, u16>,
        shadow: &std::collections::HashMap<String, u16>,
    ) {
        if !self.collecting_declaration_interface {
            return;
        }
        let mut scope = dense.clone();
        scope.extend(shadow.clone());
        let name_id = StringId(self.intern_string(name));
        let mut descriptor = self.new_fn_descriptor(name_id);
        descriptor.id = info.id;
        descriptor.has_source_body = info.has_source_body;
        descriptor.parent_type = parent.and_then(|name| {
            let owner = self
                .ctx
                .current_source_module
                .as_deref()
                .unwrap_or(&self.config.module_name);
            self.nominal_type_id(&format!("{owner}.{name}"))
                .or_else(|| self.nominal_type_id(name))
        });
        descriptor.return_type = source
            .return_type
            .as_ref()
            .and_then(|ty| self.semantic_formal_type(ty, &scope, &info.type_param_ids))
            .or_else(|| info.return_type.clone())
            .unwrap_or(TypeRef::Concrete(TypeId::UNIT));
        descriptor.return_type_name = info
            .return_type_name
            .as_ref()
            .map(|name| StringId(self.intern_string(name)));
        descriptor.explicit_type_param_ids = info.explicit_type_param_ids.clone();
        let semantic = self
            .ctx
            .semantic_fn_params
            .get(&info.id)
            .cloned()
            .unwrap_or_else(|| (0..source.params.len()).map(|_| None).collect());
        descriptor.semantic_params = Some(semantic.clone());
        for (index, parameter) in source.params.iter().enumerate() {
            let declared = match &parameter.kind {
                FunctionParamKind::Regular { ty, .. } => Some(ty),
                _ => None,
            };
            descriptor.params.push(ParamDescriptor {
                name: StringId(self.intern_string(&info.param_names[index])),
                type_ref: semantic[index]
                    .clone()
                    .or_else(|| declared.map(|ty| self.resolve_field_type_ref(ty, &scope)))
                    .unwrap_or(TypeRef::Concrete(TypeId::UNIT)),
                type_name: declared
                    .map(|ty| StringId(self.intern_string(&Self::extract_type_name_from_ast(ty))))
                    .unwrap_or(StringId::EMPTY),
                ..Default::default()
            });
        }
        for id in &info.type_param_ids {
            let name = dense
                .iter()
                .chain(shadow.iter())
                .find(|(_, slot)| **slot == id.0)
                .map(|(name, _)| name.as_str());
            descriptor.type_params.push(TypeParamDescriptor {
                id: *id,
                name: name
                    .map(|name| StringId(self.intern_string(name)))
                    .unwrap_or(StringId::EMPTY),
                ..Default::default()
            });
        }
        self.push_function_dedup(VbcFunction::new(descriptor, List::new().into()));
    }
}
