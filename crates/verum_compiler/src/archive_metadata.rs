//! T2-extended: VbcArchive → CoreMetadata converter (precompile-time).
//!
//! Pairs with [`archive_ctx_loader`](crate::archive_ctx_loader) to
//! complete the single-path archive-driven epic.  T1 +
//! `populate_types_from_archive` populate the VBC codegen ctx;
//! THIS module produces the [`verum_types::CoreMetadata`] that the
//! typecheck phase consumes via `TypeChecker::new_with_core`.
//!
//! # When this runs
//!
//! At PRECOMPILE time (`verum stdlib precompile`).  The output —
//! the [`CoreMetadata`] struct — is serialised to bincode and
//! written next to `runtime.vbca` so build.rs can embed both bytes
//! into the compiler binary.  At runtime the embedded bytes
//! deserialise once into `Arc<CoreMetadata>` and feed straight
//! into `self.stdlib_metadata` — no AST walking, no per-module
//! parse, no source-driven fallback.
//!
//! # Why CoreMetadata
//!
//! `pipeline/phases_orchestration.rs::phase_type_check` already
//! has two modes:
//!
//! ```text
//! match &self.stdlib_metadata {
//!     Some(metadata) => TypeChecker::new_with_core(metadata),
//!     None => TypeChecker::with_minimal_context(), // bootstrap only
//! }
//! ```
//!
//! When `stdlib_metadata` is `Some`, the AST-walking stdlib
//! registration block (lines 608-681 in phases_orchestration.rs)
//! is gated `is_none()` and entirely skipped.  Populating
//! `stdlib_metadata` from the archive therefore makes the
//! typecheck phase archive-driven by construction — no surgery
//! in `verum_types` required, just precompile-side data plumbing.

use std::collections::HashMap;

use verum_common::{List, Maybe, OrderedMap, Text};
use verum_types::core_metadata::{
    CoreMetadata, FieldDescriptor, FunctionDescriptor, GenericParam,
    ImplementationDescriptor, MethodSignature, ParamDescriptor, ProtocolDescriptor,
    ReceiverKind, TypeDescriptor, TypeDescriptorKind, VariantCase, VariantPayload,
};
use verum_vbc::archive::VbcArchive;
use verum_vbc::module::VbcModule;
use verum_vbc::types::{TypeKind, TypeRef, VariantKind};

/// Convert a precompiled stdlib `VbcArchive` into a
/// [`CoreMetadata`] suitable for `TypeChecker::new_with_core`.
///
/// Best-effort: per-module decode failures are skipped with a
/// `tracing::warn!`.  An empty archive returns an empty metadata.
pub fn archive_to_core_metadata(archive: &VbcArchive) -> CoreMetadata {
    let mut meta = CoreMetadata {
        types: OrderedMap::new(),
        type_declaration_order: List::new(),
        protocols: OrderedMap::new(),
        functions: OrderedMap::new(),
        implementations: List::new(),
        monomorphizations: OrderedMap::new(),
        version: 1,
        content_hash: [0u8; 32],
        context_declarations: List::new(),
        context_decl_nodes: OrderedMap::new(),
    };

    for entry in &archive.index {
        let module = match archive.load_module(&entry.name) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(
                    target: "archive_metadata",
                    "skip module {}: decode failed ({:?})",
                    entry.name, e
                );
                continue;
            }
        };
        register_module_metadata(&module, &entry.name, &mut meta);
    }

    meta
}

fn register_module_metadata(
    module: &VbcModule,
    module_name: &str,
    meta: &mut CoreMetadata,
) {
    let type_id_to_name: HashMap<u32, String> = module
        .types
        .iter()
        .filter_map(|t| {
            module
                .strings
                .get(t.name)
                .map(|n| (t.id.0, n.to_string()))
        })
        .collect();

    let module_path = Text::from(module_name);

    // Pass 1: types + protocol-types.
    for ty in &module.types {
        let type_name = match module.strings.get(ty.name) {
            Some(s) => Text::from(s),
            None => continue,
        };

        if meta.types.contains_key(&type_name) {
            collect_type_impls(ty, module, &mut meta.implementations, &type_id_to_name);
            continue;
        }

        let kind = match ty.kind {
            TypeKind::Record => {
                // Generic record types (`type Pair<A,B> is { a: A, b: B }`)
                // need the same TypeParamId → param-name map as Sum
                // types so generic-param fields don't render as
                // opaque `__generic_{idx}` placeholders.
                let mut record_param_id_to_name: HashMap<u16, String> = HashMap::new();
                for tp in ty.type_params.iter() {
                    if let Some(n) = module.strings.get(tp.name) {
                        record_param_id_to_name.insert(tp.id.0, n.to_string());
                    }
                }
                let fields: List<FieldDescriptor> = ty
                    .fields
                    .iter()
                    .map(|f| FieldDescriptor {
                        name: module
                            .strings
                            .get(f.name)
                            .map(Text::from)
                            .unwrap_or_default(),
                        ty: Text::from(type_ref_to_text_with_params(
                            &f.type_ref,
                            &type_id_to_name,
                            &record_param_id_to_name,
                        )),
                        is_public: matches!(
                            f.visibility,
                            verum_vbc::types::Visibility::Public
                        ),
                    })
                    .collect();
                TypeDescriptorKind::Record { fields }
            }
            TypeKind::Sum => {
                // Build the parent's TypeParamId → param-name map so
                // `TypeRef::Generic(idx)` slots in tuple/record
                // variants render as the source-level name (T, E, K,
                // V, …) instead of the opaque `__generic_{idx}`
                // placeholder.  Required so the typechecker's
                // structural parser can match the variant payload's
                // type name back to the parent's `generic_params`
                // entries at use sites.
                let mut sum_param_id_to_name: HashMap<u16, String> = HashMap::new();
                for tp in ty.type_params.iter() {
                    if let Some(n) = module.strings.get(tp.name) {
                        sum_param_id_to_name.insert(tp.id.0, n.to_string());
                    }
                }
                let cases: List<VariantCase> = ty
                    .variants
                    .iter()
                    .map(|v| {
                        let payload = match v.kind {
                            VariantKind::Unit => Maybe::None,
                            VariantKind::Tuple => {
                                let mut tys: List<Text> = List::new();
                                if let Some(p) = &v.payload {
                                    tys.push(Text::from(type_ref_to_text_with_params(
                                        p,
                                        &type_id_to_name,
                                        &sum_param_id_to_name,
                                    )));
                                }
                                for f in v.fields.iter() {
                                    tys.push(Text::from(type_ref_to_text_with_params(
                                        &f.type_ref,
                                        &type_id_to_name,
                                        &sum_param_id_to_name,
                                    )));
                                }
                                // Generic-payload fallback: stale
                                // archives compiled before tuple-
                                // variant fields/payload were
                                // populated may still have empty
                                // `fields`/`payload` despite arity>0.
                                // Pad with the parent type's
                                // positional generic-param names so
                                // the descriptor lands at the correct
                                // arity.  Note: this fallback only
                                // works correctly for variants whose
                                // payloads happen to match positional
                                // params (rare); the field-populated
                                // path above is the load-bearing one
                                // for fresh archives.
                                if tys.is_empty() && v.arity > 0 {
                                    let arity = v.arity as usize;
                                    let mut filled = 0;
                                    for gp in ty.type_params.iter() {
                                        if filled >= arity {
                                            break;
                                        }
                                        if let Some(name) = module.strings.get(gp.name) {
                                            tys.push(Text::from(name));
                                            filled += 1;
                                        }
                                    }
                                    while filled < arity {
                                        tys.push(Text::from("_"));
                                        filled += 1;
                                    }
                                }
                                Maybe::Some(VariantPayload::Tuple(tys))
                            }
                            VariantKind::Record => {
                                let fields: List<FieldDescriptor> = v
                                    .fields
                                    .iter()
                                    .map(|f| FieldDescriptor {
                                        name: module
                                            .strings
                                            .get(f.name)
                                            .map(Text::from)
                                            .unwrap_or_default(),
                                        ty: Text::from(type_ref_to_text_with_params(
                                            &f.type_ref,
                                            &type_id_to_name,
                                            &sum_param_id_to_name,
                                        )),
                                        is_public: matches!(
                                            f.visibility,
                                            verum_vbc::types::Visibility::Public
                                        ),
                                    })
                                    .collect();
                                Maybe::Some(VariantPayload::Record(fields))
                            }
                        };
                        VariantCase {
                            name: module
                                .strings
                                .get(v.name)
                                .map(Text::from)
                                .unwrap_or_default(),
                            payload,
                        }
                    })
                    .collect();
                TypeDescriptorKind::Variant { cases }
            }
            TypeKind::Protocol => {
                // #130 Layer D — extract method signatures from the
                // protocol type's `variants` field.  Codegen at
                // `verum_vbc/src/codegen/mod.rs:8326-8367` encodes
                // each protocol method as a `VariantDescriptor`
                // whose `name` is the method name and whose `payload`
                // is `Some(TypeRef::Function { params, return_type, contexts })`.
                // Pre-fix `meta.protocols[name].required_methods` +
                // `default_methods` were hardcoded `List::new()` so
                // the eager `load_stdlib_from_metadata` path
                // (infer.rs:2178+) registered every protocol with an
                // empty methods map — `xs.into_iter().map(f)` then
                // failed at typecheck because `map` couldn't be
                // resolved as an Iterator method.
                //
                // We can't distinguish required vs default methods
                // from the VBC archive (codegen drops the
                // distinction at line 8326 — both default-body and
                // required-no-body items become variants with the
                // same shape).  Routing every method through
                // `required_methods` is correct for typecheck: the
                // `is_default` flag in `ProtocolMethod` only affects
                // whether the method is callable without an impl
                // override at compile time, and the typechecker's
                // `methods.get(name).map(|m| m.ty.clone())` lookup
                // ignores it — see infer.rs:#129 fallback branch at
                // ~line 47680.  Stdlib-agnostic per
                // `crates/verum_types/src/CLAUDE.md`: every method
                // signature comes from the protocol's own variants,
                // not a hardcoded list.
                let proto_param_id_to_name: HashMap<u16, String> =
                    ty.type_params
                        .iter()
                        .filter_map(|tp| {
                            module.strings.get(tp.name).map(|n| (tp.id.0, n.to_string()))
                        })
                        .collect();
                let required_methods: List<MethodSignature> = ty
                    .variants
                    .iter()
                    .filter_map(|v| {
                        let method_name = module
                            .strings
                            .get(v.name)
                            .map(Text::from)?;
                        let payload = v.payload.as_ref()?;
                        let (param_refs, ret_ref, ctx_refs) = match payload {
                            TypeRef::Function {
                                params,
                                return_type,
                                contexts,
                            } => (params.as_slice(), return_type.as_ref(), contexts.as_slice()),
                            _ => return None,
                        };
                        let params: List<ParamDescriptor> = param_refs
                            .iter()
                            .enumerate()
                            .map(|(idx, p_ref)| ParamDescriptor {
                                // VBC drops protocol-method param
                                // names at codegen time; positional
                                // synthetic names round-trip enough
                                // info for the typechecker (which
                                // matches by ordinal position, not
                                // name).
                                name: Text::from(format!("p{}", idx)),
                                ty: Text::from(type_ref_to_text_with_params(
                                    p_ref,
                                    &type_id_to_name,
                                    &proto_param_id_to_name,
                                )),
                            })
                            .collect();
                        let return_type = Text::from(type_ref_to_text_with_params(
                            ret_ref,
                            &type_id_to_name,
                            &proto_param_id_to_name,
                        ));
                        let contexts: List<Text> = ctx_refs
                            .iter()
                            .filter_map(|cref| {
                                module
                                    .context_names
                                    .get(cref.0 as usize)
                                    .and_then(|sid| module.strings.get(*sid))
                                    .map(Text::from)
                            })
                            .collect();
                        Some(MethodSignature {
                            name: method_name,
                            // VBC erases self-receiver kind at
                            // codegen time (codegen/mod.rs:8343
                            // skips self params).  SelfRef is the
                            // most common shape for protocol-
                            // declared methods and round-trips for
                            // the typechecker, which dispatches
                            // receivers separately from method
                            // params anyway.
                            receiver: ReceiverKind::SelfRef,
                            params,
                            return_type,
                            contexts,
                            // is_async also lost at codegen time
                            // (TypeRef::Function carries no async
                            // bit).  Best-effort default; affects
                            // computational-property propagation
                            // only.  Safe default for non-async
                            // protocols (the majority).
                            is_async: false,
                        })
                    })
                    .collect();
                let default_method_names: List<Text> = ty
                    .variants
                    .iter()
                    .filter_map(|v| module.strings.get(v.name).map(Text::from))
                    .collect();

                // Resolve super-protocol names via type_id_to_name
                // (VBC encodes super-protocol references in
                // protocol-types' own `protocols` field per
                // codegen/mod.rs:8316-8322).  Stdlib-agnostic — names
                // come from the type table.
                let super_protocols: List<Text> = ty
                    .protocols
                    .iter()
                    .filter_map(|pi| {
                        type_id_to_name
                            .get(&pi.protocol.0)
                            .map(|s| Text::from(s.as_str()))
                    })
                    .collect();

                meta.protocols.entry(type_name.clone()).or_insert_with(|| {
                    ProtocolDescriptor {
                        name: type_name.clone(),
                        module_path: module_path.clone(),
                        generic_params: convert_generic_params(&ty.type_params, module),
                        super_protocols: super_protocols.clone(),
                        associated_types: List::new(),
                        required_methods: required_methods.clone(),
                        default_methods: List::new(),
                    }
                });
                TypeDescriptorKind::Protocol {
                    super_protocols,
                    associated_types: List::new(),
                    required_methods,
                    default_methods: default_method_names,
                }
            }
            TypeKind::Newtype | TypeKind::Tuple | TypeKind::Unit
            | TypeKind::Primitive | TypeKind::Array | TypeKind::Tensor => {
                TypeDescriptorKind::Opaque
            }
            TypeKind::Alias => {
                // Build a TypeParamId → param-name map from the
                // alias's own type_params so the rendered target
                // string preserves source-level param names
                // (`Result<T, StreamError>` rather than
                // `Result<__generic_0, StreamError>`).
                let mut param_id_to_name: HashMap<u16, String> = HashMap::new();
                for tp in ty.type_params.iter() {
                    if let Some(n) = module.strings.get(tp.name) {
                        param_id_to_name.insert(tp.id.0, n.to_string());
                    }
                }
                let target = ty
                    .alias_target
                    .as_ref()
                    .map(|t| Text::from(type_ref_to_text_with_params(
                        t,
                        &type_id_to_name,
                        &param_id_to_name,
                    )))
                    .unwrap_or_default();
                TypeDescriptorKind::Alias { target }
            }
        };

        // #130 — populate `implements` from the VBC type
        // descriptor's `protocols` table.  Each entry's
        // `ProtocolImpl.protocol: ProtocolId` is an index into the
        // same module's type table (protocols ARE types).  Resolve
        // each ProtocolId → protocol name and gather them so the
        // typechecker's protocol-impl registration path
        // (`metadata.implementations` consumer at infer.rs:2401) and
        // dispatcher (`get_implementations(receiver)`) can see the
        // impl.  Pre-fix this list was hardcoded empty so every
        // `xs.into_iter().map(f)` chain failed at type-check.
        let implements: List<Text> = ty
            .protocols
            .iter()
            .filter_map(|pi| {
                type_id_to_name
                    .get(&pi.protocol.0)
                    .map(|s| Text::from(s.as_str()))
            })
            .collect();

        let descriptor = TypeDescriptor {
            name: type_name.clone(),
            module_path: module_path.clone(),
            generic_params: convert_generic_params(&ty.type_params, module),
            kind,
            size: if ty.size > 0 {
                Maybe::Some(ty.size as usize)
            } else {
                Maybe::None
            },
            alignment: if ty.alignment > 0 {
                Maybe::Some(ty.alignment as usize)
            } else {
                Maybe::None
            },
            methods: List::new(),
            implements,
        };
        meta.types.insert(type_name.clone(), descriptor);
        meta.type_declaration_order.push(type_name);

        collect_type_impls(ty, module, &mut meta.implementations, &type_id_to_name);
    }

    // Pass 2: functions.
    for fn_desc in &module.functions {
        let simple_name = match module.strings.get(fn_desc.name) {
            Some(s) => Text::from(s),
            None => continue,
        };
        let simple_already_registered = meta.functions.contains_key(&simple_name);

        let parent_type = match fn_desc.parent_type {
            Some(tid) => match type_id_to_name.get(&tid.0) {
                Some(name) => Maybe::Some(Text::from(name.as_str())),
                None => Maybe::None,
            },
            None => Maybe::None,
        };

        let params: List<ParamDescriptor> = fn_desc
            .params
            .iter()
            .map(|p| ParamDescriptor {
                name: module
                    .strings
                    .get(p.name)
                    .map(Text::from)
                    .unwrap_or_default(),
                ty: Text::from(type_ref_to_text(&p.type_ref, &type_id_to_name)),
            })
            .collect();
        let return_type = Text::from(type_ref_to_text(
            &fn_desc.return_type,
            &type_id_to_name,
        ));

        let descriptor = FunctionDescriptor {
            name: simple_name.clone(),
            module_path: module_path.clone(),
            generic_params: convert_generic_params(&fn_desc.type_params, module),
            params,
            return_type,
            contexts: List::new(),
            is_async: fn_desc
                .properties
                .contains(verum_vbc::types::PropertySet::ASYNC),
            is_unsafe: false,
            intrinsic_id: Maybe::None,
            parent_type: parent_type.clone(),
            // #97 — round-trip the const-storage marker.
            is_const: fn_desc.is_const,
        };

        // Mirror the SIMPLE method name (no `Type.` prefix) into
        // the parent type's `methods` list, and ALSO register the
        // descriptor under the simple name as a fallback lookup
        // key for `register_inherent_methods_from_metadata`.
        // Pre-fix the methods list contained `"Text.with_capacity"`
        // (the qualified VBC function name as-stored) but the
        // typechecker dispatches methods by simple name
        // (`with_capacity`), so the inherent_methods bucket lookup
        // never matched — every `text.with_capacity(n)` call site
        // failed `no method named with_capacity found for type Text`.
        if let Maybe::Some(parent_name) = &parent_type {
            let simple_method_name = if let Some(idx) = simple_name.as_str().rfind('.') {
                Text::from(&simple_name.as_str()[idx + 1..])
            } else {
                simple_name.clone()
            };
            if let Some(td) = meta.types.get_mut(parent_name) {
                if !td.methods.iter().any(|m| m == &simple_method_name) {
                    td.methods.push(simple_method_name.clone());
                }
            }
            // Also alias the descriptor under the qualified key so
            // `register_inherent_methods_from_metadata`'s
            // `metadata.functions.get("Text.with_capacity")` finds it.
            // Keep the simple-name slot first-wins for free fns.
            let qualified_key: Text =
                format!("{}.{}", parent_name, simple_method_name).into();
            if !meta.functions.contains_key(&qualified_key) {
                meta.functions.insert(qualified_key, descriptor.clone());
            }
        } else if !module_path.is_empty() {
            // Free function — register under MODULE-qualified key
            // ALWAYS, even when the simple-name slot is taken by an
            // earlier first-wins registration.  Required so
            // `core.shell.exec.run` and `core.sys.process_ops.run`
            // both have unambiguous qualified slots — the typechecker's
            // mount-import shadowing fallback then disambiguates by
            // walking the user's mount tree's prefix to construct
            // the right qualified key.
            let qualified_key: Text =
                format!("{}.{}", module_path, simple_name).into();
            if !meta.functions.contains_key(&qualified_key) {
                meta.functions.insert(qualified_key, descriptor.clone());
            }
        }

        // Simple-name slot is first-wins (preserves the existing
        // discipline; collisions are resolved at use-sites via the
        // qualified key registered above).
        if !simple_already_registered {
            meta.functions.insert(simple_name, descriptor);
        }
    }
}

fn convert_generic_params(
    src: &[verum_vbc::types::TypeParamDescriptor],
    module: &VbcModule,
) -> List<GenericParam> {
    src.iter()
        .map(|tp| GenericParam {
            name: module
                .strings
                .get(tp.name)
                .map(Text::from)
                .unwrap_or_default(),
            bounds: List::new(),
            default: Maybe::None,
        })
        .collect()
}

fn collect_type_impls(
    ty: &verum_vbc::types::TypeDescriptor,
    module: &VbcModule,
    impls: &mut List<ImplementationDescriptor>,
    type_id_to_name: &HashMap<u32, String>,
) {
    let target_name = match module.strings.get(ty.name) {
        Some(s) => Text::from(s),
        None => return,
    };
    for proto_impl in ty.protocols.iter() {
        // #130 — resolve ProtocolId → protocol name via the
        // module's type table.  Pre-fix this was hardcoded empty
        // and the typechecker's `metadata.implementations` consumer
        // at infer.rs:2401 silently registered every impl under a
        // blank protocol name.
        let protocol_name = match type_id_to_name.get(&proto_impl.protocol.0) {
            Some(s) => Text::from(s.as_str()),
            None => continue,
        };
        let descriptor = ImplementationDescriptor {
            protocol: protocol_name,
            target_type: target_name.clone(),
            generic_params: List::new(),
            where_clause: List::new(),
            associated_types: OrderedMap::new(),
            methods: proto_impl
                .methods
                .iter()
                .filter_map(|fn_id| {
                    module
                        .functions
                        .iter()
                        .find(|f| f.id.0 == *fn_id)
                        .and_then(|f| module.strings.get(f.name).map(Text::from))
                })
                .collect(),
        };
        impls.push(descriptor);
    }
}

/// Walk a [`TypeRef`] down to a bare nominal name for
/// [`CoreMetadata`]'s `Text`-typed fields.
fn type_ref_to_text(
    ty: &TypeRef,
    type_id_to_name: &HashMap<u32, String>,
) -> String {
    type_ref_to_text_with_params(ty, type_id_to_name, &HashMap::new())
}

/// Like `type_ref_to_text` but maps `TypeRef::Generic(idx)` back to
/// a meaningful parameter name (`T`, `E`, `K`, `V`, …) instead of
/// the opaque `__generic_{idx}` placeholder.  Required for alias
/// targets so `type IoResult<T> is Result<T, StreamError>;` lands
/// in CoreMetadata as `"Result<T, StreamError>"` — parseable by the
/// typechecker's structural parser — instead of the un-parseable
/// `"Result<__generic_0, StreamError>"`.
fn type_ref_to_text_with_params(
    ty: &TypeRef,
    type_id_to_name: &HashMap<u32, String>,
    param_id_to_name: &HashMap<u16, String>,
) -> String {
    match ty {
        TypeRef::Concrete(tid) => {
            if let Some(name) = builtin_type_name(tid) {
                return name.to_string();
            }
            type_id_to_name
                .get(&tid.0)
                .cloned()
                .unwrap_or_else(|| format!("__opaque_type_{}", tid.0))
        }
        TypeRef::Generic(pid) => {
            // Look up the param's source-level name (T, E, …) via
            // the caller's param_id_to_name; fall back to the
            // opaque placeholder if no mapping is available.
            param_id_to_name
                .get(&pid.0)
                .cloned()
                .unwrap_or_else(|| format!("__generic_{}", pid.0))
        }
        TypeRef::Instantiated { base, args } => {
            let base_name = builtin_type_name(base)
                .map(|n| n.to_string())
                .or_else(|| type_id_to_name.get(&base.0).cloned())
                .unwrap_or_else(|| format!("__opaque_type_{}", base.0));
            let arg_strings: Vec<String> = args
                .iter()
                .map(|a| type_ref_to_text_with_params(a, type_id_to_name, param_id_to_name))
                .collect();
            if arg_strings.is_empty() {
                base_name
            } else {
                format!("{}<{}>", base_name, arg_strings.join(", "))
            }
        }
        TypeRef::Function {
            params,
            return_type,
            ..
        } => {
            let p: Vec<String> = params
                .iter()
                .map(|t| type_ref_to_text_with_params(t, type_id_to_name, param_id_to_name))
                .collect();
            format!(
                "fn({}) -> {}",
                p.join(", "),
                type_ref_to_text_with_params(return_type, type_id_to_name, param_id_to_name)
            )
        }
        TypeRef::Reference { inner, .. } => {
            format!("&{}", type_ref_to_text_with_params(inner, type_id_to_name, param_id_to_name))
        }
        _ => "__opaque_typeref".to_string(),
    }
}

fn builtin_type_name(tid: &verum_vbc::types::TypeId) -> Option<&'static str> {
    use verum_vbc::types::TypeId;
    match *tid {
        TypeId::UNIT => Some("Unit"),
        TypeId::BOOL => Some("Bool"),
        TypeId::INT => Some("Int"),
        TypeId::FLOAT => Some("Float"),
        TypeId::TEXT => Some("Text"),
        TypeId::NEVER => Some("Never"),
        TypeId::CHAR => Some("Char"),
        TypeId::LIST => Some("List"),
        TypeId::MAP => Some("Map"),
        TypeId::MAYBE => Some("Maybe"),
        TypeId::RESULT => Some("Result"),
        // PTR (TypeId::14) intentionally NOT named — VBC uses it
        // as a generic carrier for "unknown / opaque" type refs.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke test: archive → CoreMetadata produces non-trivial
    /// type and function tables.
    #[test]
    fn converts_embedded_archive_to_metadata() {
        let archive = match crate::embedded_stdlib_vbc::get_runtime_archive() {
            Some(a) => a,
            None => return,
        };
        let meta = archive_to_core_metadata(archive);
        assert!(
            meta.types.len() > 100,
            "expected >100 types, got {}",
            meta.types.len()
        );
        assert!(
            meta.functions.len() > 1000,
            "expected >1000 functions, got {}",
            meta.functions.len()
        );
    }

    /// Bincode round-trip: serialise CoreMetadata → bytes → back.
    /// Confirms the serde derives produce a valid round-trip
    /// format for build.rs / runtime embedding.  Uses bincode 1.3
    /// API (`serialize`/`deserialize`), the workspace-pinned version.
    #[test]
    fn metadata_bincode_round_trip() {
        let archive = match crate::embedded_stdlib_vbc::get_runtime_archive() {
            Some(a) => a,
            None => return,
        };
        let meta = archive_to_core_metadata(archive);
        let bytes = bincode::serialize(&meta).expect("encode");
        let decoded: CoreMetadata = bincode::deserialize(&bytes).expect("decode");
        assert_eq!(meta.types.len(), decoded.types.len());
        assert_eq!(meta.functions.len(), decoded.functions.len());
        assert_eq!(meta.protocols.len(), decoded.protocols.len());
    }
}
