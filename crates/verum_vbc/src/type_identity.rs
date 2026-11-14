//! Checked, program-local identity of a complete declaration-owned type.
//! A compact fingerprint is never accepted as evidence of key equality.
use crate::{
    module::VbcModule,
    mono::TypeSubstitution,
    types::{TypeId, TypeKind, TypeRef},
};
use verum_common::{List, Map, Text};

use crate::types::{MAX_TYPE_REF_DEPTH as MAX_DEPTH, MAX_TYPE_REF_NODES as MAX_NODES};

/// Evidence belongs to one assembled program, not to reusable VM settings.
/// Arc<VbcModule> users share it; cloning a mutable module starts a fresh scope.
#[derive(Debug, Default)]
pub(crate) struct TypeIdentityScope(std::sync::Mutex<TypeIdentities>);

impl Clone for TypeIdentityScope {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl VbcModule {
    /// Query identity against collision evidence shared by every executor of
    /// this immutable, assembled program. Clones and archives start fresh scopes.
    pub fn type_identity(&self, ty: &TypeRef) -> Result<u64, Text> {
        self.type_identities
            .0
            .lock()
            .map_err(|_| Text::from("type identity registry poisoned"))?
            .query(self, ty)
    }
}

/// Collision guard shared by all queries in one execution/lowering context.
#[derive(Debug, Default)]
pub struct TypeIdentities {
    keys: Map<u64, List<u8>>,
}

impl TypeIdentities {
    /// Admit a compact identity only when every earlier observation agrees
    /// with its complete key. The failed insertion leaves the registry intact.
    pub fn observe(&mut self, id: u64, key: &[u8]) -> Result<u64, Text> {
        match self.keys.get(&id) {
            Some(previous) if previous.as_slice() != key => Err(
                "type identity collision: distinct declarations cannot share an identifier".into(),
            ),
            Some(_) => Ok(id),
            None => {
                self.keys.insert(id, key.into());
                Ok(id)
            }
        }
    }

    /// Resolve aliases and witnesses before fingerprinting. Identifiers are
    /// local to the linked program; they are not persistent schema identifiers.
    pub fn query(&mut self, module: &VbcModule, ty: &TypeRef) -> Result<u64, Text> {
        let ty = canonical_type(module, ty)?;
        let mut encoded = Default::default();
        crate::bytecode::encode_type_ref(&ty, &mut encoded);
        let key = List::from(encoded);
        let hash = blake3::hash(&key);
        let id = u64::from_le_bytes(hash.as_bytes()[..8].try_into().expect("fixed digest width"));
        self.observe(id, &key)
    }
}

/// Normalize only explicit type facts. No spelling or object-layout inference.
pub fn canonical_type(module: &VbcModule, ty: &TypeRef) -> Result<TypeRef, Text> {
    check_shape(ty)?;
    let mut remaining = MAX_NODES;
    normalize(module, ty, 0, 0, false, &mut remaining)
}

// Validate before cloning/substitution. The recursive construction below only
// handles this small bounded shape, including each introduced alias target.
fn check_shape(ty: &TypeRef) -> Result<(), Text> {
    let mut pending = List::from_iter([(ty, 0)]);
    let mut remaining = MAX_NODES;
    while let Some((ty, depth)) = pending.pop() {
        if depth >= MAX_DEPTH || remaining == 0 {
            return Err("type reflection resource limit exceeded".into());
        }
        remaining -= 1;
        match ty {
            TypeRef::Instantiated { args, .. } | TypeRef::Tuple(args) => {
                if args.len() > remaining {
                    return Err("type reflection resource limit exceeded".into());
                }
                pending.extend(args.iter().map(|arg| (arg, depth + 1)));
            }
            TypeRef::Reference { inner, .. } | TypeRef::Slice(inner) => {
                pending.push((inner, depth + 1))
            }
            TypeRef::Array { element, .. } => pending.push((element, depth + 1)),
            TypeRef::Dynamic(object) => {
                pending.extend(object.types().map(|ty| (ty, depth + 1)));
            }
            TypeRef::Function { .. } | TypeRef::Rank2Function { .. } => {
                return Err(
                    "callable type identity requires its full computational contract".into(),
                );
            }
            TypeRef::AssociatedProjection { .. } | TypeRef::RecordFieldProjection { .. } => {
                return Err("type reflection requires a resolved projection witness".into());
            }
            TypeRef::ConstSequence(sequence) => {
                let mut sequences = List::from_iter([(sequence, depth)]);
                while let Some((value, depth)) = sequences.pop() {
                    if depth >= MAX_DEPTH || value.len() > remaining {
                        return Err("type reflection resource limit exceeded".into());
                    }
                    remaining -= value.len();
                    if let crate::const_sequence::ConstSequence::Arrays(rows) = value {
                        sequences.extend(rows.iter().map(|row| (row, depth + 1)));
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn normalize(
    module: &VbcModule,
    ty: &TypeRef,
    depth: usize,
    binders: u32,
    value_argument: bool,
    remaining: &mut usize,
) -> Result<TypeRef, Text> {
    if depth >= MAX_DEPTH || *remaining == 0 {
        return Err("type reflection resource limit exceeded".into());
    }
    *remaining -= 1;
    let child =
        |ty, remaining: &mut usize| normalize(module, ty, depth + 1, binders, false, remaining);
    Ok(match ty {
        TypeRef::Concrete(id) | TypeRef::Instantiated { base: id, .. } => {
            let args = match ty {
                TypeRef::Instantiated { args, .. } => args.as_slice(),
                _ => &[],
            };
            let scalar = id.well_known_name().and_then(|name| {
                TypeId::from_well_known_scalar_name(
                    verum_common::well_known_types::type_names::canonical_identity_name(name),
                )
            });
            if *id == TypeId::PTR {
                return Err("type identity requires semantic pointer or integer witnesses, not an ABI carrier".into());
            }
            if let Some(scalar) = scalar {
                if !args.is_empty() {
                    return Err("builtin type application lacks declaration metadata".into());
                }
                TypeRef::Concrete(scalar)
            } else {
                let declaration = module
                    .get_type(*id)
                    .ok_or_else(|| Text::from("type reflection requires the exact declaration"))?;
                if args.len() != declaration.type_params.len() {
                    return Err("type reflection has incomplete declaration arguments".into());
                }
                if declaration.kind == TypeKind::Alias {
                    if declaration.source_declaration.as_ref().is_some_and(|source|
                        matches!(&source.body, verum_ast::TypeDeclBody::Alias(target)
                            if matches!(target.kind, verum_ast::ty::TypeKind::Refined { .. })))
                    {
                        return Err("refinement type identity requires its predicate, not only runtime layout".into());
                    }
                    let target = declaration
                        .alias_target
                        .as_ref()
                        .ok_or_else(|| Text::from("type alias has no declared target"))?;
                    check_shape(target)?;
                    let target = TypeSubstitution::new(&declaration.type_params, args)
                        .apply(target)
                        .map_err(|error| Text::from(error.to_string()))?;
                    check_shape(&target)?;
                    child(&target, remaining)?
                } else if args.is_empty() {
                    TypeRef::Concrete(*id)
                } else {
                    TypeRef::Instantiated {
                        base: *id,
                        args: args
                            .iter()
                            .map(|arg| normalize(module, arg, depth + 1, binders, true, remaining))
                            .collect::<Result<_, _>>()?,
                    }
                }
            }
        }
        TypeRef::Dynamic(object) => {
            crate::dynamic_protocol::validate(object, &|id| module.get_type(id))?;
            let binders = binders.checked_add(1).ok_or_else(|| Text::from("type binder scope overflow"))?;
            TypeRef::Dynamic(object.try_map(|ty| normalize(module, ty, depth + 1, binders, true, remaining))?)
        }
        TypeRef::Generic(id) if u32::from(id.0) < binders => ty.clone(),
        TypeRef::Reference {
            inner,
            mutability,
            tier,
        } => TypeRef::Reference {
            inner: verum_common::Heap::new(child(inner, remaining)?),
            mutability: *mutability,
            tier: *tier,
        },
        TypeRef::Array { element, length } => TypeRef::Array {
            element: verum_common::Heap::new(child(element, remaining)?),
            length: *length,
        },
        TypeRef::Slice(inner) => TypeRef::Slice(verum_common::Heap::new(child(inner, remaining)?)),
        TypeRef::Tuple(parts) => TypeRef::Tuple(
            parts
                .iter()
                .map(|part| child(part, remaining))
                .collect::<Result<_, _>>()?,
        ),
        TypeRef::ConstSequence(sequence) if value_argument => {
            TypeRef::ConstSequence(sequence.clone())
        }
        TypeRef::ConstValue(value) if value_argument => TypeRef::ConstValue(*value),
        _ => return Err("type reflection has unresolved or incomplete semantic witnesses".into()),
    })
}
