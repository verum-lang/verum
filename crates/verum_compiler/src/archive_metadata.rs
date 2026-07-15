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
        // Re-export chains are captured at archive precompile time;
        // archive→metadata convert path leaves the map empty (the
        // typechecker's re-export resolver degrades to a no-op when
        // the entry is absent, falling back to AST walks).
        module_reexports: OrderedMap::new(),
    };

    // METADATA-DETERMINISM-1 (the live #17-ph2 root): the archive index
    // order follows the BAKE's (rayon-parallel) write order, and the
    // simple-name `meta.types.insert` below was last-wins — every
    // rebake ROLLED DICE on which same-name descriptor owned the
    // simple slot. Concretely: core.meta.token's `Span` ALIAS record
    // vs core.tracing's / sqlite-meta's `Span` records — when the
    // alias lost, the checker's lazy alias registration never ran and
    // 42 unmounted-alias tests failed E400 'expected Span found
    // MetaSpan' (bake-dependent, code-independent). Sort the walk for
    // determinism; the collision POLICY lives at the insert.
    let mut sorted_index: Vec<_> = archive.index.iter().collect();
    sorted_index.sort_by(|a, b| a.name.cmp(&b.name));
    for entry in sorted_index {
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

        // MOUNT-TYPE-AUTHORITY-1: a colliding simple name must NOT
        // skip descriptor construction.  The historic early-continue
        // here (`if meta.types.contains_key(&type_name) { … continue; }`)
        // made the collision policy at the insert below unreachable
        // for every cross-module same-name pair (only the first
        // sorted-walk registrant ever built a descriptor), and —
        // worse — dropped the collision loser's QUALIFIED
        // `<module>.<Name>` key entirely, so mount-scoped consumers
        // had NO collision-immune slot to resolve through.  Live
        // regression: `core.action.effects` `EffectKind` (variant,
        // sorts first) vs `core.meta.diakrisis_attrs` `EffectKind`
        // (alias, sorts second) — the alias vanished from metadata
        // and every file that explicitly mounted it type-checked
        // against the action enum.  Fall through instead: both the
        // qualified key and the simple slot apply the RANKED
        // collision policy documented at the insert below.
        let simple_slot_was_occupied = meta.types.contains_key(&type_name);

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
                        // #101 — span population deferred to a source-walk
                        // pass in `precompile.rs::write_core_metadata_alongside_archive`.
                        decl_span: verum_common::Maybe::None,
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
            // #101 — span population deferred to source-walk pass.
            decl_span: Maybe::None,
            // FUNDAMENTAL #3 — propagate the transparent-wrapper flag
            // from the VBC type descriptor.  Downstream typechecker
            // lazy loader keys `__newtype_inner_X` registration on this
            // bit; without the propagation, archive-loaded `type
            // FileDesc is (Int);` lost its newtype identity at the
            // typechecker boundary and every `FileDesc.STDIN.0` site
            // failed with `cannot index type 'FileDesc'`.
            is_transparent_wrapper: ty.is_transparent_wrapper,
        };
        // Collision policy (deterministic, documented):
        //
        // * SIMPLE slot — strict FIRST-WINS in sorted-walk order,
        //   byte-identical to the pre-MOUNT-TYPE-AUTHORITY-1
        //   behavior (the early-continue skipped every repeat, so
        //   the first registrant always owned the slot).  The simple
        //   slot is the global last-resort surface for UNMOUNTED
        //   files; scores of suites pin its historic owners, so its
        //   occupancy must not change.  Files that MOUNT a collision
        //   loser resolve through the qualified keys instead — that
        //   is the fix.
        //
        // * QUALIFIED `<module>.<Name>` key — RANKED, order-
        //   independent within the module walk:
        //     2  forward alias  (`type Span is MetaSpan;` — target's
        //        base name differs from the alias's own name; a
        //        deliberate name-forward)
        //     1  structural     (record / variant / protocol / …)
        //     0  self-forward alias (`Ordering` →
        //        `core.base.Ordering` — re-export plumbing emitted
        //        alongside the real declaration in the SAME archive
        //        module; its target renders as a DOTTED name the
        //        checker cannot chase, so it must never own the
        //        qualified slot when the module also carries the
        //        structural descriptor)
        //   A slot is replaced only by a STRICTLY higher-ranked
        //   incoming descriptor; ties keep the first occupant.
        let rank = |kind: &TypeDescriptorKind, name: &Text| -> u8 {
            match kind {
                TypeDescriptorKind::Alias { target } => {
                    let base = target
                        .as_str()
                        .split('<')
                        .next()
                        .unwrap_or("");
                    let last_seg =
                        base.rsplit('.').next().unwrap_or(base).trim();
                    if last_seg == name.as_str() { 0 } else { 2 }
                }
                _ => 1,
            }
        };
        let incoming_rank = rank(&descriptor.kind, &type_name);
        let qualified_key: Text =
            format!("{}.{}", module_name, type_name).into();
        let insert_qualified = match meta.types.get(&qualified_key) {
            None => true,
            Some(existing) => incoming_rank > rank(&existing.kind, &type_name),
        };
        if insert_qualified {
            meta.types.insert(qualified_key, descriptor.clone());
        }
        if !simple_slot_was_occupied {
            meta.types.insert(type_name.clone(), descriptor);
        }
        // Declaration-order list records each SIMPLE name once —
        // collision repeats (which now fall through to here for
        // their qualified insert) must not duplicate the entry the
        // first registrant already pushed.
        if !simple_slot_was_occupied {
            meta.type_declaration_order.push(type_name);
        }

        collect_type_impls(ty, module, &mut meta.implementations, &type_id_to_name);
    }

    // Pass 2: functions.
    //
    // Task #16 reland blocker #3: filter out stage-1/stage-2 stub
    // FunctionDescriptors so they don't propagate into
    // `metadata.functions`.  Stubs live in two sentinel FunctionId
    // ranges reserved by `stdlib_bootstrap::pre_register_canonical_*`:
    //
    //   * Stage 1 (canonical-type static-method stubs):
    //        [STAGE1_BASE - WIDTH, STAGE1_BASE] where
    //        STAGE1_BASE = u32::MAX - 0x40_0000 (0xFFBF_FFFF).
    //   * Stage 2 (stdlib variant-constructor stubs):
    //        [STAGE2_BASE - WIDTH, STAGE2_BASE] where
    //        STAGE2_BASE = u32::MAX - 0xC0_0000 (0xFF3F_FFFF).
    //
    // Stubs are codegen-context entries only — they should NEVER
    // appear in the per-module compiled bytecode set.  When they
    // leaked into `module.functions` during the first reland
    // attempt, the typechecker's lazy-load (`register_inherent_methods_from_metadata`)
    // saw them as real descriptors with `parent_type_name` that
    // disrupted the canonical lookup (`Int.checked_add` regressed
    // at typecheck despite Int not being in the stage-1
    // canonical-types set — stubs for OTHER canonical types' static
    // methods polluted the metadata function table and the
    // typechecker mis-attributed methods across types).
    //
    // The runtime sentinel handler at
    // `verum_vbc::interpreter::dispatch_table::handlers::calls::handle_call`
    // (commit `b5f5462d4`) catches stuck stubs at the dispatch
    // boundary; THIS filter catches them at the metadata-emission
    // boundary so they never reach the typechecker's lazy-load
    // path either.  Both gates together make stage-1+2 reland
    // safe.
    // Filter STUBS only — `bytecode_length == 0` AND sentinel-range
    // ID together identify unresolved stage-1/2 stubs.  Real bodies
    // assigned a sentinel ID by the stub-overwrite-gate overlay path
    // have `bytecode_length > 0` and pass through.  See the matching
    // empty-body gate in `verum_vbc::codegen::push_function_dedup`
    // (commit history under task #16) for the rationale that closed
    // the `f98f7ea49` revert's `Int.checked_add` regression.
    const STAGE1_STUB_BASE: u32 = u32::MAX - 0x40_0000;
    const STAGE2_STUB_BASE: u32 = u32::MAX - 0xC0_0000;
    const STUB_RANGE_WIDTH: u32 = 0x10_0000;
    let is_stub_id = |id: u32| -> bool {
        let in_stage1 = id <= STAGE1_STUB_BASE && id >= STAGE1_STUB_BASE - STUB_RANGE_WIDTH;
        let in_stage2 = id <= STAGE2_STUB_BASE && id >= STAGE2_STUB_BASE - STUB_RANGE_WIDTH;
        in_stage1 || in_stage2
    };
    for fn_desc in &module.functions {
        if is_stub_id(fn_desc.id.0) && fn_desc.bytecode_length == 0 {
            continue;
        }
        let simple_name = match module.strings.get(fn_desc.name) {
            Some(s) => Text::from(s),
            None => continue,
        };
        let simple_already_registered = meta.functions.contains_key(&simple_name);

        // Task #21 — name-prefix takes precedence over TypeId map.
        //
        // TypeId aliasing problem: `type Byte is UInt8;` (declared in
        // core/base/mod.vr) and `implement UInt8 { … }` plus
        // `implement Byte { … }` BOTH allocate `TypeId(6)`.  The VBC
        // codegen stores inherent methods with their qualified NAME
        // (`"UInt8.wrapping_add"` and `"Byte.wrapping_add"` are two
        // SEPARATE function descriptors), but the receiver / param /
        // return TypeRefs all resolve to the SAME `TypeId(6)`.
        // `type_id_to_name` maps each TypeId to the FIRST name it
        // saw — which is whichever was registered first in the
        // module's type table (typically `Byte`).
        //
        // Pre-fix every `UInt8.wrapping_add` descriptor got
        // `parent_type = "Byte"` (and params/return rendered as
        // "Byte"), so:
        //  * `Byte.wrapping_add` qualified_key got registered first
        //    via line ~612.
        //  * `UInt8.wrapping_add` qualified_key construction said
        //    `parent_name = "Byte"` → re-registered the same key
        //    (no-op via the `contains_key` guard).
        //  * `meta.types["UInt8"].methods` never got
        //    "wrapping_add" pushed (parent_name = "Byte" → pushed to
        //    `meta.types["Byte"]`).
        //  * Pass 2.5's `synthesized_primitives` saw the
        //    `UInt8.wrapping_add` simple-name slot from line ~637
        //    but the methods list was already populated with the
        //    canonical UInt8 set (BIT_WIDTH / from_bits / …) and the
        //    existing methods didn't include `wrapping_add`.
        //
        // The fix: the NAME prefix is the authoritative source of
        // truth for the user-visible parent type.  Use it whenever
        // it matches a canonical primitive, regardless of whether
        // `type_id_to_name` also offers a (potentially-aliased)
        // mapping.
        let prefix_recovered = recover_primitive_parent_from_name(
            module.strings.get(fn_desc.name).unwrap_or(""),
        );
        let parent_type = match (&prefix_recovered, fn_desc.parent_type) {
            (Maybe::Some(_), _) => prefix_recovered.clone(),
            (Maybe::None, Some(tid)) => match type_id_to_name.get(&tid.0) {
                Some(name) => Maybe::Some(Text::from(name.as_str())),
                None => Maybe::None,
            },
            (Maybe::None, None) => Maybe::None,
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
        // SLICE-METHOD-TYPECHECK-E400 (#51): VBC TypeRefs have NO
        // slice form — `slice() -> &[T]` degrades to bare "List"
        // through the render, and the typechecker's metadata-loaded
        // scheme then types chained-slice receivers as List<_>.
        // RETNAME-CARRY (v2.6) holds the source-verbatim return name;
        // prefer it EXACTLY for the lossy class (slice/array spellings
        // containing '[') — broader verbatim use would leak `Self` /
        // local aliases that the TypeRef render correctly normalises.
        let carried_return = fn_desc
            .return_type_name
            .and_then(|sid| module.strings.get(sid))
            .filter(|s| s.contains('['));
        let return_type = match carried_return {
            Some(verbatim) => Text::from(verbatim),
            None => Text::from(type_ref_to_text(
                &fn_desc.return_type,
                &type_id_to_name,
            )),
        };

        // Task #21 — disambiguate aliased-TypeId types in param /
        // return rendering using the (just-resolved) parent_type as
        // the user-visible context.  For `UInt8.wrapping_add`, the
        // `Byte` rendered text gets rewritten to `UInt8` so the
        // typechecker's `register_inherent_methods_from_metadata`
        // builds the function signature with the correct
        // user-visible types and the call-site `v: UInt8;
        // v.wrapping_add(b: UInt8)` unifies.
        //
        // The rewrite is bidirectional: a `Byte.<m>` method's
        // `UInt8` references get rewritten to `Byte`.  Other
        // aliased pairs (e.g. USize / ISize / Ptr share TypeId(14)
        // but live in distinct user-facing namespaces) are NOT
        // rewritten because the spec separates them — only Byte
        // and UInt8 are spec-aliased via `public type Byte is
        // UInt8;` in `core/base/mod.vr`.
        let rewrite_aliased_typeid = |s: &Text, target: &str, alias: &str| -> Text {
            if !s.as_str().contains(alias) {
                return s.clone();
            }
            // Word-bounded replace: only swap "Byte" tokens, not
            // "Bytes" (List<Byte>) or "ByteCount" or similar.
            let mut out = String::with_capacity(s.as_str().len());
            let bytes = s.as_str().as_bytes();
            let mut i = 0;
            while i < bytes.len() {
                if bytes[i..].starts_with(alias.as_bytes()) {
                    let end = i + alias.len();
                    let before_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric() && bytes[i - 1] != b'_';
                    let after_ok = end == bytes.len()
                        || !bytes[end].is_ascii_alphanumeric() && bytes[end] != b'_';
                    if before_ok && after_ok {
                        out.push_str(target);
                        i = end;
                        continue;
                    }
                }
                out.push(bytes[i] as char);
                i += 1;
            }
            Text::from(out.as_str())
        };
        let (params, return_type) = if let Maybe::Some(parent_name) = &parent_type {
            let (target, alias) = match parent_name.as_str() {
                "UInt8" => Some(("UInt8", "Byte")),
                "Byte" => Some(("Byte", "UInt8")),
                _ => None,
            }
            .unwrap_or(("", ""));
            if !target.is_empty() {
                let params: List<ParamDescriptor> = params
                    .into_iter()
                    .map(|p| ParamDescriptor {
                        name: p.name.clone(),
                        ty: rewrite_aliased_typeid(&p.ty, target, alias),
                    })
                    .collect();
                let return_type = rewrite_aliased_typeid(&return_type, target, alias);
                (params, return_type)
            } else {
                (params, return_type)
            }
        } else {
            (params, return_type)
        };

        // Pillar-3 increment 1 (ARRAY-ITER-CONCRETIZE-1) — CARRY the
        // impl-block generic names on method descriptors.  The parent
        // type's `type_params` ARE the impl-block generics: the
        // type-decl pass registers `implement<T> [T]`'s `<T>` into the
        // parent TypeDescriptor, and `compile_function`'s
        // `method_generic_param_map` numbers exactly those params
        // FIRST (ids `0..k`) when resolving method param/return
        // TypeRefs — so the serialised `__generic_i` placeholders
        // with `i < k` are impl-level BY CONSTRUCTION.  The
        // typechecker's metadata scheme-birth site uses this to
        // order impl-level TypeVars first and set `impl_var_count`
        // (the split is unrecoverable from the strings alone —
        // see the field's doc in `verum_types::core_metadata`).
        let impl_generic_names: List<Text> = fn_desc
            .parent_type
            .and_then(|tid| module.types.iter().find(|t| t.id.0 == tid.0))
            .map(|parent_ty| {
                parent_ty
                    .type_params
                    .iter()
                    .filter_map(|tp| module.strings.get(tp.name).map(Text::from))
                    .collect()
            })
            .unwrap_or_default();

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
            impl_generic_names,
            // #97 — round-trip the const-storage marker.
            is_const: fn_desc.is_const,
            // #101 — span population deferred to source-walk pass.
            decl_span: Maybe::None,
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

    // Pass 2.5: synthesize TypeDescriptors for built-in primitive types
    // referenced by `Parent.method`-shaped function names that are
    // absent from `module.types`.
    //
    // Background: primitives like `Int`, `Byte`, `USize`, `ISize`,
    // `UInt8..128`, `Int8..128`, `Float`, `Float32`, `Float64`,
    // `Bool`, `Char` are BUILT-IN — their `TypeId` is recognised by
    // the runtime / VBC model but the stdlib's `implement <Primitive>
    // { … }` block doesn't emit a `module.types` entry for them (the
    // type ALREADY exists at the VM layer).  As a side-effect the
    // builder's `type_id_to_name` map — assembled from `module.types`
    // — has NO entry for the primitive's TypeId, so
    // `fn_desc.parent_type` (`Option<TypeId>`) resolves to `None` in
    // Pass 2 above even though the impl method's VBC-stored function
    // name is `"Int.checked_add"`.  Aliased TypeIds compound this:
    // TypeId(14) maps to all of USize / ISize / Ptr; TypeId(6) to
    // Byte / U8 / UInt8; the lossy `TypeId → name` direction can't
    // recover the canonical name.
    //
    // The typechecker's lazy-load path
    // (`ensure_stdlib_type_loaded`) consults `metadata.types[Int]`
    // and bails when it isn't there, so the inherent_methods bucket
    // for Int stays empty — every `n.checked_add(m)` /
    // `n.wrapping_sub(k)` user-side call site fails with `no method
    // named X found for type Int` despite the method descriptor
    // sitting in `metadata.functions` keyed under the simple name.
    //
    // **Fix**: parse the function-name STRING for the canonical
    // `Type.method` shape and use that prefix as the parent name.
    // This preserves the stdlib's `implement <Name> { … }` choice of
    // user-visible name regardless of the lossy TypeId.  Then
    // synthesize an empty stub TypeDescriptor for each unique parent
    // name and push the simple method names into its `methods` list.
    //
    // The stub carries no fields / variants / generics — primitives
    // are opaque at the source layer, and the typechecker's
    // `register_builtins` (the canonical primitive registrar) has
    // already populated the actual `Type` entries; this stub serves
    // ONLY as a method-name catalogue the lazy loader can iterate.
    //
    // Closes task #15.  Unblocks every inherent-method call on a
    // built-in primitive whose `implement <Primitive> { … }` block
    // lives in `core/base/primitives.vr`.  Touches core-tests/base/
    // primitives/* (8 files) + every user-code site that calls
    // `checked_*`, `wrapping_*`, `saturating_*`, `rotate_*`,
    // `count_ones`, `leading_zeros`, `to_le_bytes`, etc. on any
    // primitive integer / float / Bool / Char type.
    //
    // The synthesized descriptor is intentionally minimal so the
    // typechecker's `ensure_stdlib_type_loaded` short-circuits the
    // "already in ctx" branch (because `register_builtins` registered
    // the primitive's Type entry), proceeds to
    // `register_inherent_methods_from_metadata`, and enumerates the
    // `methods` list we just populated.
    //
    // The list of canonical primitive type names is fixed by the
    // language model: integer / float / boolean / character / byte /
    // pointer-sized.  We accept ANY uppercase-leading dotted-prefix
    // as a candidate but only synthesize for the closed set —
    // user-defined types with the same shape are filtered by
    // `is_canonical_primitive_name` below to keep the metadata layer
    // strictly bounded to primitives we know to be runtime-built-in.
    fn is_canonical_primitive_name(name: &str) -> bool {
        matches!(
            name,
            "Int" | "Float" | "Bool" | "Char" | "Byte" | "USize" | "ISize"
                | "Int8" | "Int16" | "Int32" | "Int64" | "Int128"
                | "UInt8" | "UInt16" | "UInt32" | "UInt64" | "UInt128"
                | "Float32" | "Float64"
        )
    }
    // ARCH-P2 (metadata determinism): both walks below were HashMap
    // hash-order — the per-primitive method-list ORDER and the
    // insertion order of synthesized descriptors into the
    // insertion-ordered `meta.types` flipped per bake (byte-diff of two
    // runtime.core_metadata bakes: a 143-byte primitive-name run,
    // Int32/Float/… vs UInt32/ISize/… — the tail of the types table).
    // Deterministic: sorted function keys feed the buckets; sorted
    // parent names drive the merge/insert loop.
    let mut synthesized_primitives: HashMap<Text, List<Text>> = HashMap::new();
    let mut fn_keys_sorted: Vec<&Text> = meta.functions.keys().collect();
    fn_keys_sorted.sort();
    for key in fn_keys_sorted {
        let dot = match key.as_str().find('.') {
            Some(d) => d,
            None => continue,
        };
        let parent_str = &key.as_str()[..dot];
        if !is_canonical_primitive_name(parent_str) {
            continue;
        }
        let parent = Text::from(parent_str);
        if meta.types.contains_key(&parent) {
            continue;
        }
        let method_simple = Text::from(&key.as_str()[dot + 1..]);
        let bucket = synthesized_primitives
            .entry(parent)
            .or_default();
        if !bucket.iter().any(|m| m == &method_simple) {
            bucket.push(method_simple);
        }
    }
    let mut synthesized_sorted: Vec<(Text, List<Text>)> =
        synthesized_primitives.into_iter().collect();
    synthesized_sorted.sort_by(|a, b| a.0.cmp(&b.0));
    for (parent_name, methods) in synthesized_sorted {
        // **Merge-or-synthesize**: when the primitive is already in
        // `meta.types` (from some prior Pass 1/2 registration via
        // source-declared impl block), MERGE the discovered methods
        // into its existing methods list instead of skipping.  Pre-fix
        // the gate at this site short-circuited on the
        // `contains_key` check, so primitives with empty methods lists
        // (Pass 2 couldn't resolve fn_desc.parent_type → name because
        // the primitive's TypeId isn't in the producing module's
        // local type_id_to_name map) stayed empty forever and every
        // `let a: UInt8 = 200; a.wrapping_add(b)` call failed with
        // `no method named 'wrapping_add' found for type 'UInt8'`.
        if let Some(existing) = meta.types.get_mut(&parent_name) {
            for m in methods.iter() {
                if !existing.methods.iter().any(|em| em == m) {
                    existing.methods.push(m.clone());
                }
            }
            continue;
        }
        let descriptor = TypeDescriptor {
            name: parent_name.clone(),
            module_path: Text::from(""),
            generic_params: List::new(),
            // `Record` with no fields is the canonical "opaque
            // primitive" stub — the typechecker's
            // `type_descriptor_to_type` path treats this as a no-op
            // when the type is already registered as a primitive in
            // ctx, so the stub doesn't shadow the actual primitive's
            // Type entry.
            kind: TypeDescriptorKind::Record { fields: List::new() },
            size: Maybe::None,
            alignment: Maybe::None,
            methods,
            implements: List::new(),
            decl_span: Maybe::None,
            // Primitive method-table stubs are never transparent
            // wrappers — they're synthetic carriers for the inherent
            // method table only.
            is_transparent_wrapper: false,
        };
        meta.types.insert(parent_name.clone(), descriptor);
        meta.type_declaration_order.push(parent_name);
    }
}

/// Task #21 — primitive parent-type recovery from a VBC-stored
/// function name.
///
/// Returns `Maybe::Some(parent)` when `name` is of shape
/// `<CanonicalPrimitive>.<method>` (e.g. `"UInt8.wrapping_add"`)
/// — the prefix is the inherent method's parent type when the
/// VBC TypeId can't be resolved via the module's local
/// `type_id_to_name` map.
///
/// Lossy TypeId aliasing makes the codegen-time prefix the
/// authoritative source of truth for primitive method parents:
/// TypeId(6) is shared by Byte / U8 / UInt8; TypeId(14) by
/// USize / ISize / Ptr; the typechecker dispatches by NAME so
/// the qualified form must carry the user-visible parent.
fn recover_primitive_parent_from_name(name: &str) -> Maybe<Text> {
    let dot = match name.find('.') {
        Some(d) => d,
        None => return Maybe::None,
    };
    let prefix = &name[..dot];
    let tail = &name[dot + 1..];
    // Tail must be a single segment — a `<Module>.<fn>` free
    // function (e.g. `core.shell.exec.run`) has further dots
    // and isn't an inherent method.
    if tail.contains('.') {
        return Maybe::None;
    }
    // Closed set of compiler-built-in primitive types; mirrored
    // from `is_canonical_primitive_name` in Pass 2.5 so the two
    // gates stay in sync.
    let is_primitive = matches!(
        prefix,
        "Int" | "Float" | "Bool" | "Char" | "Byte" | "USize" | "ISize"
            | "Int8" | "Int16" | "Int32" | "Int64" | "Int128"
            | "UInt8" | "UInt16" | "UInt32" | "UInt64" | "UInt128"
            | "Float32" | "Float64"
    );
    if is_primitive {
        Maybe::Some(Text::from(prefix))
    } else {
        Maybe::None
    }
}

fn convert_generic_params(
    src: &[verum_vbc::types::TypeParamDescriptor],
    module: &VbcModule,
) -> List<GenericParam> {
    // **Type-id → name index** (used for function-type bound
    // serialisation).  Built once per metadata pass; the per-param
    // type_bounds list is sparse (only HOFs carry any), so the
    // overhead is paid only when needed.
    let type_id_to_name: HashMap<u32, String> = module
        .types
        .iter()
        .filter_map(|t| module.strings.get(t.name).map(|n| (t.id.0, n.to_string())))
        .collect();
    let param_id_to_name: HashMap<u16, String> = src
        .iter()
        .filter_map(|tp| module.strings.get(tp.name).map(|n| (tp.id.0, n.to_string())))
        .collect();
    src.iter()
        .map(|tp| GenericParam {
            name: module
                .strings
                .get(tp.name)
                .map(Text::from)
                .unwrap_or_default(),
            bounds: List::new(),
            default: Maybe::None,
            type_bounds: tp
                .type_bounds
                .iter()
                .map(|tr| {
                    Text::from(type_ref_to_text_with_params(
                        tr,
                        &type_id_to_name,
                        &param_id_to_name,
                    ))
                })
                .collect(),
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
            // Canonicalize the bake-internal shadow-stub spelling
            // (`shadowed$FromResidual$137` — a foreign protocol
            // referenced by an impl before/without its declaring
            // module in the compile unit) back to the protocol's
            // real name. The metadata is the PUBLIC contract: the
            // typechecker's FromResidual/Try/marker scans match on
            // canonical names, so a shadow-spelled row is invisible
            // (`Result?` inside a Maybe-returning fn kept failing
            // E0203 because Maybe's two FromResidual rows carried
            // the shadow name; #47 tail).
            Some(s) => match s
                .strip_prefix("shadowed$")
                .and_then(|rest| rest.rsplit_once('$'))
            {
                Some((canon, _id)) => Text::from(canon),
                None => Text::from(s.as_str()),
            },
            None => continue,
        };
        // Pillar-3 increment 1 (ARRAY-ITER-CONCRETIZE-1) — render the
        // impl block's carried associated-type bindings (`type Item =
        // &T;` → `"&__generic_0"`).  The `__generic_i` ids follow the
        // parent type's `type_params` order (codegen's binding-time
        // param map), so the typechecker's metadata reader can link
        // them to the receiver's type args positionally.
        let associated_types: OrderedMap<Text, Text> = {
            let mut m = OrderedMap::new();
            for (name_sid, tref) in proto_impl.associated_types.iter() {
                if let Some(n) = module.strings.get(*name_sid) {
                    m.insert(
                        Text::from(n),
                        Text::from(type_ref_to_text(tref, type_id_to_name)),
                    );
                }
            }
            m
        };
        let descriptor = ImplementationDescriptor {
            protocol: protocol_name,
            target_type: target_name.clone(),
            generic_params: List::new(),
            where_clause: List::new(),
            associated_types,
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
            // Per-impl carry (ProtocolImpl.protocol_args_text, VBC
            // v2.7): source-rendered protocol type-args straight from
            // the archive record. The post-archive source-walk
            // (`precompile::scan_implementation_protocol_args`)
            // remains a FALLBACK for pre-carry archives only — its
            // (target, protocol) first-wins key collapsed sibling
            // impls (three `FromResidual<…> for Result` rows all
            // claimed `Result<Never, E>`; `m?` in a Result fn then
            // failed E0203).
            protocol_args: proto_impl
                .protocol_args_text
                .iter()
                .filter_map(|sid| module.strings.get(*sid).map(Text::from))
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
        TypeRef::Reference {
            inner,
            mutability,
            tier,
        } => {
            // ARCHIVE-REF-TIER-DROP-1: render the CBGR tier and the
            // mutability faithfully (grammar: `&[checked|unsafe] [mut] T`).
            // The previous `{ inner, .. }` render collapsed every
            // reference to `&T`, so every baked stdlib signature lost
            // its tier — `List.as_mut_ptr() -> &unsafe T` came back as
            // `&T`, the method-result auto-deref then stripped it to
            // bare `T`, and `cell.as_mut_ptr() as *mut UInt32` failed
            // E401 the moment the (Named, Pointer) cast arm stopped
            // being a blanket accept (A13).
            let tier_kw = match tier {
                verum_vbc::types::CbgrTier::Tier0 => "",
                verum_vbc::types::CbgrTier::Tier1 => "checked ",
                verum_vbc::types::CbgrTier::Tier2 => "unsafe ",
            };
            let mut_kw = match mutability {
                verum_vbc::types::Mutability::Mutable => "mut ",
                verum_vbc::types::Mutability::Immutable => "",
            };
            format!(
                "&{}{}{}",
                tier_kw,
                mut_kw,
                type_ref_to_text_with_params(inner, type_id_to_name, param_id_to_name)
            )
        }
        // Associated-type projection `F.Output` → `::Output<F>` (parseable back
        // into `Type::Generic { name: "::Output", args: [F] }` by
        // parse_descriptor_type_string).
        TypeRef::AssociatedProjection { base, assoc } => {
            format!(
                "::{}<{}>",
                assoc,
                type_ref_to_text_with_params(base, type_id_to_name, param_id_to_name)
            )
        }
        // CONST-GENERIC-VALUE-CARRY-1: const-generic VALUE argument renders
        // as its literal so `StackAllocator<256>` round-trips through the
        // metadata text form instead of degrading to `__opaque_typeref`.
        TypeRef::ConstValue(v) => v.to_string(),
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

    /// MOUNT-TYPE-AUTHORITY-1 regression contract, name-agnostic:
    ///
    /// 1. **No descriptor is ever lost** — every named type in every
    ///    archive module's type table has its qualified
    ///    `<module>.<Name>` key in `meta.types`.  The historic
    ///    early-continue on a taken simple slot dropped the
    ///    collision loser's qualified key entirely, so a file that
    ///    explicitly mounted the losing type had no
    ///    collision-immune slot to resolve through.
    /// 2. **Qualified slot never trades structure for a
    ///    self-forward alias** — when an archive module's type table
    ///    carries BOTH a structural descriptor and a self-forward
    ///    re-export alias for the same name (declaration + re-export
    ///    plumbing in one directory-granular module), the qualified
    ///    slot must hold the structural one.  A self-forward alias's
    ///    target renders as a DOTTED name the checker cannot chase —
    ///    letting it own `core.base.Ordering` broke every mounted
    ///    `Ordering` annotation.
    ///
    /// The SIMPLE slot is deliberately NOT asserted here: it is
    /// strict first-wins (sorted-walk order), byte-identical to the
    /// pre-fix behavior, and pinned by the existing suite corpus.
    #[test]
    fn collision_keeps_qualified_keys_and_structural_qualified_slot() {
        let archive = match crate::embedded_stdlib_vbc::get_runtime_archive() {
            Some(a) => a,
            None => return,
        };
        let meta = archive_to_core_metadata(archive);

        let mut checked = 0usize;
        let mut has_structural: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        for entry in archive.index.iter() {
            let module = match archive.load_module(&entry.name) {
                Ok(m) => m,
                Err(_) => continue,
            };
            for ty in &module.types {
                let Some(type_name) = module.strings.get(ty.name) else {
                    continue;
                };
                let qualified = format!("{}.{}", entry.name, type_name);
                assert!(
                    meta.types.contains_key(&Text::from(qualified.as_str())),
                    "qualified key '{}' missing from metadata.types — a \
                     collision dropped a descriptor",
                    qualified
                );
                if !matches!(ty.kind, TypeKind::Alias) {
                    has_structural.insert(qualified.clone());
                }
                checked += 1;
            }
        }
        assert!(checked > 100, "expected >100 archive types, got {checked}");

        for (key, desc) in meta.types.iter() {
            let Some((_, simple)) = key.as_str().rsplit_once('.') else {
                continue;
            };
            if simple.is_empty() || !has_structural.contains(key.as_str()) {
                continue;
            }
            if let TypeDescriptorKind::Alias { target } = &desc.kind {
                let base = target.as_str().split('<').next().unwrap_or("");
                let last_seg = base.rsplit('.').next().unwrap_or(base).trim();
                assert!(
                    last_seg != simple,
                    "qualified slot '{}' holds a SELF-FORWARD alias \
                     ('{}' -> '{}') while the module also declares the \
                     structural type — the ranked qualified policy \
                     regressed",
                    key,
                    simple,
                    target,
                );
            }
        }
    }
}
