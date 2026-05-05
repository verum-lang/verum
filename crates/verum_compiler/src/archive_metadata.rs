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
    ImplementationDescriptor, ParamDescriptor, ProtocolDescriptor, TypeDescriptor,
    TypeDescriptorKind, VariantCase, VariantPayload,
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
            collect_type_impls(ty, module, &mut meta.implementations);
            continue;
        }

        let kind = match ty.kind {
            TypeKind::Record => {
                let fields: List<FieldDescriptor> = ty
                    .fields
                    .iter()
                    .map(|f| FieldDescriptor {
                        name: module
                            .strings
                            .get(f.name)
                            .map(Text::from)
                            .unwrap_or_default(),
                        ty: Text::from(type_ref_to_text(
                            &f.type_ref,
                            &type_id_to_name,
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
                let cases: List<VariantCase> = ty
                    .variants
                    .iter()
                    .map(|v| {
                        let payload = match v.kind {
                            VariantKind::Unit => Maybe::None,
                            VariantKind::Tuple => {
                                let mut tys: List<Text> = List::new();
                                if let Some(p) = &v.payload {
                                    tys.push(Text::from(type_ref_to_text(
                                        p,
                                        &type_id_to_name,
                                    )));
                                }
                                for f in v.fields.iter() {
                                    tys.push(Text::from(type_ref_to_text(
                                        &f.type_ref,
                                        &type_id_to_name,
                                    )));
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
                                        ty: Text::from(type_ref_to_text(
                                            &f.type_ref,
                                            &type_id_to_name,
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
                meta.protocols.entry(type_name.clone()).or_insert_with(|| {
                    ProtocolDescriptor {
                        name: type_name.clone(),
                        module_path: module_path.clone(),
                        generic_params: convert_generic_params(&ty.type_params, module),
                        super_protocols: List::new(),
                        associated_types: List::new(),
                        required_methods: List::new(),
                        default_methods: List::new(),
                    }
                });
                TypeDescriptorKind::Protocol {
                    super_protocols: List::new(),
                    associated_types: List::new(),
                    required_methods: List::new(),
                    default_methods: List::new(),
                }
            }
            TypeKind::Newtype | TypeKind::Tuple | TypeKind::Unit
            | TypeKind::Primitive | TypeKind::Array | TypeKind::Tensor => {
                TypeDescriptorKind::Opaque
            }
        };

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
            implements: List::new(),
        };
        meta.types.insert(type_name.clone(), descriptor);
        meta.type_declaration_order.push(type_name);

        collect_type_impls(ty, module, &mut meta.implementations);
    }

    // Pass 2: functions.
    for fn_desc in &module.functions {
        let simple_name = match module.strings.get(fn_desc.name) {
            Some(s) => Text::from(s),
            None => continue,
        };
        if meta.functions.contains_key(&simple_name) {
            continue;
        }

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
        }

        meta.functions.insert(simple_name, descriptor);
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
) {
    let target_name = match module.strings.get(ty.name) {
        Some(s) => Text::from(s),
        None => return,
    };
    for proto_impl in ty.protocols.iter() {
        let descriptor = ImplementationDescriptor {
            protocol: Text::default(),
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
        TypeRef::Generic(pid) => format!("__generic_{}", pid.0),
        TypeRef::Instantiated { base, args } => {
            let base_name = builtin_type_name(base)
                .map(|n| n.to_string())
                .or_else(|| type_id_to_name.get(&base.0).cloned())
                .unwrap_or_else(|| format!("__opaque_type_{}", base.0));
            let arg_strings: Vec<String> = args
                .iter()
                .map(|a| type_ref_to_text(a, type_id_to_name))
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
                .map(|t| type_ref_to_text(t, type_id_to_name))
                .collect();
            format!(
                "fn({}) -> {}",
                p.join(", "),
                type_ref_to_text(return_type, type_id_to_name)
            )
        }
        TypeRef::Reference { inner, .. } => {
            format!("&{}", type_ref_to_text(inner, type_id_to_name))
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
