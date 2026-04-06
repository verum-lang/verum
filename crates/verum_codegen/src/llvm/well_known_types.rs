//! Well-known Verum stdlib type names for LLVM lowering.
//!
//! Re-exports `WellKnownType` from `verum_common` and extends it with
//! VBC TypeId integration and LLVM-specific register marking helpers.

pub use verum_common::well_known_types::WellKnownType;

use verum_vbc::types::TypeId;

/// Extension trait adding VBC TypeId integration to WellKnownType.
pub trait WellKnownTypeExt {
    /// Try to resolve a VBC TypeId to a well-known type.
    fn from_type_id(tid: TypeId) -> Option<WellKnownType>;

    /// Mark the appropriate register tracker for collection/text types.
    /// Returns true if a tracker was applied.
    fn mark_register(self, ctx: &mut super::context::FunctionContext<'_, '_>, reg: u16) -> bool;
}

impl WellKnownTypeExt for WellKnownType {
    fn from_type_id(tid: TypeId) -> Option<WellKnownType> {
        if tid == TypeId::INT { return Some(WellKnownType::Int); }
        if tid == TypeId::FLOAT { return Some(WellKnownType::Float); }
        if tid == TypeId::BOOL { return Some(WellKnownType::Bool); }
        if tid == TypeId::TEXT { return Some(WellKnownType::Text); }
        if tid == TypeId::LIST { return Some(WellKnownType::List); }
        if tid == TypeId::MAP { return Some(WellKnownType::Map); }
        if tid == TypeId::SET { return Some(WellKnownType::Set); }
        if tid == TypeId::DEQUE { return Some(WellKnownType::Deque); }
        if tid == TypeId::MAYBE { return Some(WellKnownType::Maybe); }
        if tid == TypeId::RESULT { return Some(WellKnownType::Result); }
        if tid == TypeId::HEAP { return Some(WellKnownType::Heap); }
        if tid == TypeId::SHARED { return Some(WellKnownType::Shared); }
        if tid == TypeId::CHANNEL { return Some(WellKnownType::Channel); }
        None
    }

    fn mark_register(self, ctx: &mut super::context::FunctionContext<'_, '_>, reg: u16) -> bool {
        match self {
            WellKnownType::List => { ctx.mark_list_register(reg); true }
            WellKnownType::Map => { ctx.mark_map_register(reg); true }
            WellKnownType::Set => { ctx.mark_set_register(reg); true }
            WellKnownType::Deque => { ctx.mark_deque_register(reg); true }
            WellKnownType::Text => { ctx.mark_text_register(reg); true }
            _ => false,
        }
    }
}

/// Resolve a type name from either a VBC TypeId or a module type name lookup.
pub fn resolve_well_known_from_type_ref(
    type_ref: &verum_vbc::types::TypeRef,
    vbc_mod: &verum_vbc::module::VbcModule,
) -> Option<WellKnownType> {
    match type_ref {
        verum_vbc::types::TypeRef::Concrete(tid) => {
            WellKnownType::from_type_id(*tid)
                .or_else(|| vbc_mod.get_type_name(*tid).and_then(|n| WellKnownType::from_name(&n)))
        }
        verum_vbc::types::TypeRef::Instantiated { base, .. } => {
            WellKnownType::from_type_id(*base)
                .or_else(|| vbc_mod.get_type_name(*base).and_then(|n| WellKnownType::from_name(&n)))
        }
        _ => None,
    }
}
