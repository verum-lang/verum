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
        let parent = self.config.module_name.clone();
        let owned: Vec<_> = sources.iter().map(|module| (parent.as_str(), *module)).collect();
        self.collect_owned_declaration_interface(&owned)
    }

    /// Elaborate a whole declaration graph after claiming all source-owned
    /// nominal identities. A parent supplies context for short or absent module
    /// headers; it is never substituted for an explicit qualified source owner.
    pub fn collect_owned_declaration_interface(
        &mut self,
        sources: &[(&str, &Module)],
    ) -> CodegenResult<VbcModule> {
        self.collecting_declaration_interface = true;
        let collected = self.collect_owned_unit_declarations(sources);
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
        source_signature: Option<FunctionDecl>,
        info: &FunctionInfo,
        name: &str,
        parent: Option<&str>,
        dense: &std::collections::HashMap<String, u16>,
        shadow: &std::collections::HashMap<String, u16>,
    ) -> CodegenResult<()> {
        self.claim_callable_generic_roster(info.id, source, parent, dense, shadow)?;
        let mut scope = dense.clone();
        scope.extend(shadow.clone());
        let name_id = StringId(self.intern_string(name));
        let mut descriptor = self.new_fn_descriptor(name_id);
        descriptor.source_signature = self.retain_callable_owner_signature(
            source, source_signature, parent, dense,
        )?;
        descriptor.id = info.id;
        descriptor.declaration_identity = Some(crate::module::CallableDeclarationIdentity {
            source_name: StringId(self.intern_string(source.name.name.as_str())),
            lexical_parent: self.current_source_callable_parent,
        });
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
        descriptor.semantic_return = self.source_semantic_result(source, &scope, &info.type_param_ids);
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
            let resolved = match semantic[index].clone() {
                Some(ty) => ty,
                None => declared.map(|ty| self.resolve_field_type_ref(ty, &scope)).transpose()?
                    .unwrap_or(TypeRef::Concrete(TypeId::UNIT)),
            };
            // The declaration interface carries the same callable bound as
            // ordinary body compilation; the semantic slot still names O.
            let type_ref = match declared {
                Some(ty) => self.substitute_fn_bound_for_generic(resolved, ty, source, &scope)?,
                None => resolved,
            };
            let default = self.param_default_const_id(parameter, &type_ref)?;
            descriptor.params.push(ParamDescriptor {
                name: StringId(self.intern_string(&info.param_names[index])),
                type_ref,
                type_name: declared
                    .map(|ty| {
                        let name = Self::render_scheme_type_name(ty, false, &|path| self.declared_dyn_path(path));
                        StringId(self.intern_string(&name))
                    })
                    .or_else(|| {
                        parameter.kind.receiver_type_spelling().map(|ty| StringId(self.intern_string(ty)))
                    })
                    .unwrap_or(StringId::EMPTY),
                default,
                is_mut: self.extract_param_name_and_mutable(parameter).is_some_and(|(_, mutable)| mutable),
                ..Default::default()
            });
        }
        descriptor.type_params = self.declared_callable_type_parameters(source, dense, shadow)?;
        descriptor.is_generator = info.is_generator;
        if info.is_generator { descriptor.properties |= crate::types::PropertySet::GENERATOR; }
        if parent.is_none() {
            descriptor.visibility = if matches!(source.visibility, verum_ast::Visibility::Public) {
                crate::types::Visibility::Public
            } else { crate::types::Visibility::Private };
        }
        descriptor.optimization_hints = self.extract_optimization_hints(source);
        descriptor.is_test = self.propagate_test_attr && source.attributes.iter().any(|attribute| attribute.is_named("test"));
        descriptor.is_gpu_only = Self::function_is_gpu_only(&source.attributes);
        descriptor.yield_type = info.yield_type.clone();
        if info.is_async { descriptor.properties |= crate::types::PropertySet::ASYNC; }
        descriptor.intrinsic_name = info.intrinsic_name.as_ref().map(|name| StringId(self.intern_string(name)));
        for context in &info.contexts {
            descriptor.contexts.push(self.declared_context_ref(context)?);
        }
        self.declared_callable_defaults.insert(info.id, source.params.iter().map(|parameter| {
            match &parameter.kind {
                FunctionParamKind::Regular { default_value, .. } => default_value.clone(),
                _ => None,
            }
        }).collect());
        self.retain_declared_callable(descriptor.clone())?;
        if self.collecting_declaration_interface {
            self.push_function_dedup(VbcFunction::new(descriptor, List::new().into()));
        }
        Ok(())
    }

    /// The interface and executable descriptor share the same layered ABI slots.
    pub(super) fn declared_callable_type_parameters(
        &mut self, func: &FunctionDecl,
        method_generic_param_map: &std::collections::HashMap<String, u16>,
        method_shadow_band: &std::collections::HashMap<String, u16>,
    ) -> CodegenResult<smallvec::SmallVec<[TypeParamDescriptor; 2]>> {
        let mut parameters = smallvec::SmallVec::new();
        let mut render_generic_param_map = method_generic_param_map.clone();
        render_generic_param_map.extend(method_shadow_band.clone());
            // Include the SHADOWED impl slots (layered-map): the map
            // keeps only the winner per name, but the descriptor's
            // pid-vector must stay dense — a hole at the displaced
            // impl pid would desync mono's by-entry binding.
            let mut pid_ordered: Vec<(String, u16)> = method_generic_param_map
                .iter()
                .map(|(n, pid)| (n.clone(), *pid))
                // VARIANT C: band entries are published AFTER the dense
                // vector (sort puts 0x8000+ last) so the typechecker's
                // scheme birth sees the method generic's DECLARED name
                // and the `__generic_{0x8000+k}` placeholders in bound
                // strings resolve; dense consumers (mono by-entry
                // binding, positional carries) filter the band range.
                .chain(method_shadow_band.iter().map(|(n, pid)| (n.clone(), *pid)))
                .collect();
            pid_ordered.sort_unstable_by_key(|(_, pid)| *pid);
            // FN-BOUND-CARRY (T0701): serialise each generic's
            // Function-typed bound (`F: fn(I.Item) -> B`) into
            // `type_bounds`.  An impl-level existential (`B` in
            // `implement<I, B, F: fn(I.Item) -> B>` — in the impl list
            // but NOT in the for-type args) is determined ONLY through
            // a sibling's fn-bound; without this carry the reader's
            // scheme leaves it a forever-free var (`m.next()` judged
            // `Some(_)` E404 while the receiver was fully concrete).
            // Method-level (incl. band) bounds render in the METHOD
            // scope; impl-level bounds render in the DENSE scope —
            // same layering as the signature rendering above.
            let fn_bound_for = |gname: &str,
                                is_method_level: bool,
                                name_is_banded: bool|
             -> CodegenResult<Option<TypeRef>> {
                use verum_ast::ty::{GenericParamKind, TypeBoundKind, TypeKind};
                let probe = |gps: &verum_common::List<verum_ast::ty::GenericParam>|
                 -> Option<verum_ast::ty::Type> {
                    for gp in gps.iter() {
                        let (n, bounds) = match &gp.kind {
                            GenericParamKind::Type { name, bounds, .. } => {
                                (name.name.as_str(), bounds)
                            }
                            GenericParamKind::HigherKinded { name, bounds, .. } => {
                                (name.name.as_str(), bounds)
                            }
                            _ => continue,
                        };
                        if n != gname {
                            continue;
                        }
                        for b in bounds.iter() {
                            let bound_ty = match &b.kind {
                                TypeBoundKind::Equality(ty) => ty,
                                TypeBoundKind::GenericProtocol(ty) => ty,
                                _ => continue,
                            };
                            if matches!(&bound_ty.kind, TypeKind::Function { .. }) {
                                return Some(bound_ty.clone());
                            }
                        }
                    }
                    None
                };
                let ast_bound = if is_method_level {
                    probe(&func.generics)
                } else if name_is_banded {
                    // T0701 (inspect leg): the dense slot of a
                    // COLLIDING name must never read the method's
                    // bound — with no impl-scope generics in reach
                    // (materialised protocol default methods) it
                    // carries NO bound rather than the wrong one.
                    self.current_impl_ast_generics
                        .as_ref()
                        .and_then(|gps| probe(gps))
                } else {
                    self.current_impl_ast_generics
                        .as_ref()
                        .and_then(|gps| probe(gps))
                        .or_else(|| probe(&func.generics))
                };
                ast_bound.map(|ty| {
                    let map = if is_method_level {
                        &render_generic_param_map
                    } else {
                        &method_generic_param_map
                    };
                    self.resolve_signature_type_ref(&ty, map)
                }).transpose()
            };
            let fn_own_names: std::collections::HashSet<String> = func
                .generics
                .iter()
                .filter_map(|gp| match &gp.kind {
                    verum_ast::ty::GenericParamKind::Type { name, .. }
                    | verum_ast::ty::GenericParamKind::HigherKinded { name, .. } => {
                        Some(name.name.to_string())
                    }
                    _ => None,
                })
                .collect();
            // Two phases: bound rendering borrows `self` immutably
            // (resolve_field_type_ref), interning needs `&mut self`.
            // T0701 (inspect leg): a DENSE pid whose name has a shadow-band
            // twin is the IMPL-level param BY CONSTRUCTION — the band exists
            // exactly because the method redeclared the name.  The bare
            // `fn_own_names.contains` test marked impl-F#1 method-level too,
            // so BOTH F slots serialised the METHOD's `fn(..) -> B` bound;
            // the extraction rule then pulled B from the RECEIVER'S stored
            // impl-F value (`inspect`'s `fn(&Item) -> Unit` closure) and
            // every `.inspect(..).map(|x| ..)` failed "expected Unit".
            let banded_names: std::collections::HashSet<&str> = method_shadow_band
                .iter()
                .map(|(n, _)| n.as_str())
                .collect();
            let rendered: Vec<(String, u16, Option<TypeRef>)> = pid_ordered
                .into_iter()
                .map(|(gname, pid)| {
                    let name_is_banded = banded_names.contains(gname.as_str());
                    let is_method_level = pid >= 0x8000
                        || (fn_own_names.contains(&gname) && !name_is_banded);
                    let tb = fn_bound_for(&gname, is_method_level, name_is_banded)?;
                    Ok((gname, pid, tb))
                })
                .collect::<CodegenResult<_>>()?;
            for (gname, pid, tb) in rendered {
                let mut type_bounds: smallvec::SmallVec<[TypeRef; 1]> =
                    smallvec::SmallVec::new();
                if let Some(tb) = tb {
                    type_bounds.push(tb);
                }
                let name_id = StringId(self.intern_string(&gname));
                parameters.push(crate::types::TypeParamDescriptor {
                        name: name_id,
                        id: crate::types::TypeParamId(pid),
                        bounds: smallvec::SmallVec::new(),
                        default: None,
                        variance: crate::types::Variance::Invariant,
                        type_bounds,
                    });
            }
        Ok(parameters)
    }
}
