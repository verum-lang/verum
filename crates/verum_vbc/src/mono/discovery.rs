//! Exact call-site facts shared by initial and nested mono discovery (T1526).

use super::{
    InstantiationGraph, MonoPhaseConfig, MonoPhaseError, SourceLocation, TypeSubstitution,
};
use crate::instruction::Instruction;
use crate::module::{FunctionDescriptor, FunctionId, VbcModule};
use crate::types::TypeRef;

// An associated projection is still a deferred type, even when its base is
// concrete. Rank-2 binders need their own substitution scope (T1521).
pub(super) fn concrete(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::Generic(_)
        | TypeRef::AssociatedProjection { .. }
        | TypeRef::Rank2Function { .. } => false,
        TypeRef::Instantiated { args, .. } | TypeRef::Tuple(args) => args.iter().all(concrete),
        TypeRef::Function {
            params,
            return_type,
            ..
        } => params.iter().all(concrete) && concrete(return_type),
        TypeRef::Reference { inner, .. } | TypeRef::Slice(inner) => concrete(inner),
        TypeRef::Array { element, .. } => concrete(element),
        _ => true,
    }
}

/// Compact only the declared IDs: sparse shadow-witness padding is not an
/// unresolved parameter and must not become a 32769-entry mono key.
pub fn canonical_type_args(func: &FunctionDescriptor, args: &[TypeRef]) -> Option<Vec<TypeRef>> {
    if args.is_empty() {
        return None;
    }
    if !func.type_params.is_empty() && args.len() > func.type_params.len() {
        // Old metadata can omit actual const slots. Preserve a concrete
        // indexed vector in full; compacting it would erase specialization
        // identity and leave the body's LoadT for that slot unsubstituted.
        if args.iter().all(concrete) {
            return Some(args.to_vec());
        }
        // Only identity padding of an old sparse witness can be discarded.
        // A real undeclared fact mixed with unknown slots is not a complete
        // instantiation and must not be silently truncated.
        if args.iter().enumerate().any(|(index, arg)| {
            !func.type_params.iter().any(|p| p.id.0 as usize == index)
                && !matches!(arg,TypeRef::Generic(id) if id.0 as usize == index)
        }) {
            return None;
        }
    }
    let actual = if func.type_params.is_empty() {
        args.to_vec()
    } else {
        let subst = TypeSubstitution::from_function(func, args);
        func.type_params
            .iter()
            .map(|p| subst.get(p.id).cloned())
            .collect::<Option<Vec<_>>>()?
    };
    actual.iter().all(concrete).then_some(actual)
}

pub(super) fn call_target(module: &VbcModule, raw: u32, base: u32) -> Option<FunctionId> {
    let id = module.resolve_band_id(raw).or_else(|| {
        if crate::stub_ranges::is_stub_id(raw) {
            return None;
        }
        raw.checked_add(base).map(FunctionId)
    })?;
    module.get_function(id).map(|_| id)
}

/// Discover only calls carrying an exact body ID and complete concrete args.
/// Bare CallM names are deliberately not candidates for this graph.
pub fn discover_call_instantiations(
    module: &VbcModule,
    instructions: &[Instruction],
    function_base: u32,
    graph: &mut InstantiationGraph,
) -> Result<(), MonoPhaseError> {
    for instr in instructions {
        if let Some((callee, args)) = concrete_call(module, instr, function_base) {
            record_concrete_instantiation(
                graph,
                callee,
                args,
                MonoPhaseConfig::default().max_instantiations,
            )?;
        }
    }
    Ok(())
}

/// Initial sites and nested sites have the same fail-closed resource policy.
pub fn record_concrete_instantiation(
    graph: &mut InstantiationGraph,
    callee: FunctionId,
    args: Vec<TypeRef>,
    limit: usize,
) -> Result<(), MonoPhaseError> {
    if args
        .iter()
        .any(|ty| super::graph::type_ref_depth(ty) > super::graph::MAX_TYPE_REF_DEPTH)
    {
        return Err(MonoPhaseError::ResourceLimit {
            resource: "type depth",
            limit: super::graph::MAX_TYPE_REF_DEPTH,
        });
    }
    if !graph.contains(callee, &args) && graph.len() >= limit {
        return Err(MonoPhaseError::ResourceLimit {
            resource: "instantiation count",
            limit,
        });
    }
    graph.record_instantiation(callee, args, SourceLocation::default());
    Ok(())
}

pub(super) fn concrete_call(
    module: &VbcModule,
    instr: &Instruction,
    base: u32,
) -> Option<(FunctionId, Vec<TypeRef>)> {
    let Instruction::CallG {
        func_id, type_args, ..
    } = instr
    else {
        return None;
    };
    let callee = call_target(module, *func_id, base)?;
    let func = module.get_function(callee)?;
    if func.bytecode_length == 0
        && func
            .instructions
            .as_ref()
            .is_none_or(|body| body.is_empty())
    {
        return None;
    }
    if !func.is_generic
        && func.type_params.is_empty()
        && !func.params.iter().any(|p| p.type_ref.is_generic())
        && !func.return_type.is_generic()
        && !type_args
            .iter()
            .any(|t| matches!(t, TypeRef::ConstValue(_)))
    {
        return None;
    }
    canonical_type_args(func, type_args).map(|args| (callee, args))
}
