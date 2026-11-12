//! Execution-target facts. Supplied by the backend, never guessed from the host
//! or serialized as host-dependent answers inside portable generic bytecode.
use crate::{
    instruction::LayoutProperty,
    module::VbcModule,
    types::{TypeId, TypeKind, TypeRef},
};
use verum_common::{OrderedMap, Text};

/// Immutable facts from the configured backend target machine.
#[derive(Debug, Clone)]
pub struct TargetCapabilities {
    /// Atomic expansion limit in the target's execution model (bits).
    pub max_atomic_bits: u32,
    /// Native pointer width, from the backend data layout.
    pub pointer_bits: u32,
    /// Actual CPU feature catalog, with enabled and disabled entries.
    pub features: OrderedMap<Text, bool>,
}

impl TargetCapabilities {
    /// Whether the selected target enables this CPU instruction-set feature.
    pub fn has_feature(&self, name: &str) -> bool {
        self.features.get(&name.into()).copied().unwrap_or(false)
    }

    /// Atomic storage for a declared scalar. Unknown type identities are errors;
    /// non-scalar carriers are unsupported, irrespective of a coincidental size.
    pub fn has_atomic(&self, module: &VbcModule, ty: &TypeRef) -> Result<bool, Text> {
        let mut ty = ty.clone();
        for _ in 0..64 {
            match &ty {
                TypeRef::Concrete(id) | TypeRef::Instantiated { base: id, .. } => {
                    if *id == TypeId::PTR {
                        return Ok(self.supports_atomic_width(self.pointer_bits));
                    }
                    if id.is_builtin() {
                        if !id.is_integer() && *id != TypeId::BOOL {
                            return Ok(false);
                        }
                        let bytes = crate::type_layout::query(module, &ty, LayoutProperty::Size)
                            .ok_or_else(|| Text::from("missing scalar storage width"))?;
                        return Ok(u32::try_from(bytes.saturating_mul(8))
                            .is_ok_and(|bits| self.supports_atomic_width(bits)));
                    }
                    let declaration = module
                        .get_type(*id)
                        .ok_or_else(|| Text::from("missing atomic type declaration"))?;
                    if declaration.kind != TypeKind::Alias {
                        return Ok(false);
                    }
                    let arguments = match &ty {
                        TypeRef::Instantiated { args, .. } => args.as_slice(),
                        _ => &[],
                    };
                    if arguments.len() != declaration.type_params.len() {
                        return Err("incomplete atomic alias arguments".into());
                    }
                    let target = declaration
                        .alias_target
                        .as_ref()
                        .ok_or_else(|| Text::from("missing atomic alias target"))?;
                    ty = crate::mono::TypeSubstitution::new(&declaration.type_params, arguments)
                        .apply(target)
                        .map_err(|error| Text::from(error.to_string()))?;
                }
                TypeRef::Generic(_)
                | TypeRef::AssociatedProjection { .. }
                | TypeRef::RecordFieldProjection { .. }
                | TypeRef::ConstValue(_)
                | TypeRef::ConstSequence(_) => return Err("unresolved atomic type witness".into()),
                _ => return Ok(false),
            }
        }
        Err("atomic alias traversal limit exceeded".into())
    }

    fn supports_atomic_width(&self, bits: u32) -> bool {
        // The operation registry owns VBC's implemented widths. A target fact
        // cannot manufacture a wider interpreter/native storage operation.
        bits > 0
            && bits <= self.max_atomic_bits
            && crate::intrinsics::registry::supports_atomic_storage(bits)
    }
}
