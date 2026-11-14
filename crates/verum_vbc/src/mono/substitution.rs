//! Type substitution for monomorphization.

use verum_common::{List, Map};

use crate::module::FunctionDescriptor;
use crate::types::{
    MAX_TYPE_REF_DEPTH, MAX_TYPE_REF_NODES, TypeParamDescriptor, TypeParamId, TypeRef,
};

/// Type substitution environment for monomorphization.
///
/// Maps type parameters to concrete types.
#[derive(Clone, PartialEq, Eq)]
pub struct TypeSubstitution {
    /// Type parameter bindings: TypeParamId -> concrete TypeRef.
    bindings: Map<TypeParamId, TypeRef>,
}

impl TypeSubstitution {
    /// Creates a new substitution from type parameters and arguments.
    pub fn new(params: &[TypeParamDescriptor], args: &[TypeRef]) -> Self {
        let mut bindings = Map::new();
        for (param, arg) in params.iter().zip(args.iter()) {
            bindings.insert(param.id, arg.clone());
        }
        Self { bindings }
    }

    /// Creates a substitution from a function descriptor and type arguments.
    ///
    /// The descriptor maps compact argument positions to declaration-owned
    /// IDs, including shadowed method parameters. Descriptors without generic
    /// metadata retain the legacy positional argument convention.
    pub fn from_function(func: &FunctionDescriptor, args: &[TypeRef]) -> Self {
        // T1526/T1528: compact witnesses follow the declaration's exact ID
        // roster, including shadow parameters. The descriptor owns legacy
        // indexed compatibility as well; consumers never infer slot identity.
        if !func.type_params.is_empty() && args.len() <= func.type_params.len() {
            let mut result = Self::empty();
            for param in &func.type_params {
                if let Some(arg) = func.generic_argument(args, param.id) {
                    result.bind(param.id, arg.clone());
                }
            }
            return result;
        }
        // Metadata-free and legacy surplus vectors are indexed by ID. The
        // surplus can contain real const arguments omitted from old rosters.
        let mut bindings = Map::new();
        for (i, arg) in args.iter().enumerate() {
            bindings.insert(crate::types::TypeParamId(i as u16), arg.clone());
        }
        Self { bindings }
    }

    /// Creates an empty substitution.
    pub fn empty() -> Self {
        Self {
            bindings: Map::new(),
        }
    }

    /// Adds a binding to the substitution.
    pub fn bind(&mut self, param: TypeParamId, ty: TypeRef) {
        self.bindings.insert(param, ty);
    }

    /// Gets the binding for a type parameter.
    pub fn get(&self, param: TypeParamId) -> Option<&TypeRef> {
        self.bindings.get(&param)
    }

    /// Applies the substitution to a type reference.
    ///
    /// Check the complete output shape before constructing any replacements.
    pub fn apply(&self, type_ref: &TypeRef) -> Result<TypeRef, SubstitutionError> {
        validate_mapped_shape(type_ref, ParameterMapping::Substitute(self))?;
        map_free_parameters(type_ref, 0, &mut |id, depth| match self.bindings.get(&id) {
            Some(replacement) => lift_free_parameters_checked(replacement, depth),
            None => lifted_parameter(id, depth),
        })
    }

    /// Returns the number of bindings.
    pub fn len(&self) -> usize {
        self.bindings.len()
    }

    /// Returns true if there are no bindings.
    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }
}

/// Substitution must not silently wrap a representable type parameter index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubstitutionError {
    GenericIndexOverflow {
        index: u16,
        shift: u32,
    },
    BinderDepthOverflow,
    /// The substituted result exceeds the shared structural-depth boundary.
    TypeDepthLimit,
    /// The substituted result exceeds the shared total-component boundary.
    TypeNodeLimit,
    /// A closed sequence violates its independent payload extent or depth limit.
    ConstSequenceLimit,
}

impl std::fmt::Display for SubstitutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::GenericIndexOverflow { index, shift } => write!(
                f,
                "type parameter index {index} cannot be lifted by {shift} in the VBC binder representation"
            ),
            Self::BinderDepthOverflow => write!(f, "VBC type binder depth is not representable"),
            Self::TypeDepthLimit => write!(
                f,
                "substituted VBC type exceeds depth limit {MAX_TYPE_REF_DEPTH}"
            ),
            Self::TypeNodeLimit => write!(
                f,
                "substituted VBC type exceeds component limit {MAX_TYPE_REF_NODES}"
            ),

            Self::ConstSequenceLimit => write!(
                f,
                "substituted VBC constant sequence exceeds its structural limit"
            ),
        }
    }
}
impl std::error::Error for SubstitutionError {}

fn lifted_parameter(id: TypeParamId, shift: u32) -> Result<TypeRef, SubstitutionError> {
    u32::from(id.0)
        .checked_add(shift)
        .and_then(|index| u16::try_from(index).ok())
        .map(|index| TypeRef::Generic(TypeParamId(index)))
        .ok_or(SubstitutionError::GenericIndexOverflow { index: id.0, shift })
}

/// Move free parameters under additional binders. Local variables in nested
/// rank-2 types remain local; replacement types cannot capture those variables.
pub fn lift_free_parameters(ty: &TypeRef, amount: u32) -> Result<TypeRef, SubstitutionError> {
    validate_mapped_shape(ty, ParameterMapping::Lift(amount))?;
    lift_free_parameters_checked(ty, amount)
}

fn lift_free_parameters_checked(ty: &TypeRef, amount: u32) -> Result<TypeRef, SubstitutionError> {
    map_free_parameters(ty, 0, &mut |id, depth| {
        let shift = depth
            .checked_add(amount)
            .ok_or(SubstitutionError::BinderDepthOverflow)?;
        lifted_parameter(id, shift)
    })
}

#[derive(Clone, Copy)]
enum ParameterMapping<'a> {
    Substitute(&'a TypeSubstitution),
    Lift(u32),
}

/// Walk the prospective result without cloning its inputs. A replacement is
/// simultaneous: its free parameters are lifted, never substituted again.
/// The final bounded mapper can then use small, structurally bounded calls.
fn validate_mapped_shape<'a>(
    ty: &'a TypeRef,
    mapping: ParameterMapping<'a>,
) -> Result<(), SubstitutionError> {
    let mut pending = List::new();
    pending.push((ty, 0usize, 0u32, mapping));
    let mut remaining = MAX_TYPE_REF_NODES;
    while let Some((ty, depth, scope, mapping)) = pending.pop() {
        if depth >= MAX_TYPE_REF_DEPTH {
            return Err(SubstitutionError::TypeDepthLimit);
        }
        if let TypeRef::ConstSequence(sequence) = ty {
            sequence
                .validate()
                .map_err(|_| SubstitutionError::ConstSequenceLimit)?;
        }
        if let TypeRef::Generic(id) = ty
            && u32::from(id.0) >= scope
        {
            let free = TypeParamId((u32::from(id.0) - scope) as u16);
            match mapping {
                ParameterMapping::Substitute(substitution) => {
                    if let Some(replacement) = substitution.get(free) {
                        pending.push((replacement, depth, 0, ParameterMapping::Lift(scope)));
                        continue;
                    }
                }
                ParameterMapping::Lift(amount) => {
                    let shift = scope
                        .checked_add(amount)
                        .ok_or(SubstitutionError::BinderDepthOverflow)?;
                    lifted_parameter(free, shift)?;
                }
            }
        }
        remaining = remaining
            .checked_sub(1)
            .ok_or(SubstitutionError::TypeNodeLimit)?;
        let introduced = match ty {
            TypeRef::Rank2Function {
                type_param_count, ..
            } => u32::from(*type_param_count),
            TypeRef::Dynamic(object) => {
                // Protocol applications and equalities carry metadata even when
                // their immediate type-argument lists are empty, as in the codec.
                remaining = remaining
                    .checked_sub(object.bounds.len())
                    .and_then(|count| count.checked_sub(object.bindings.len()))
                    .ok_or(SubstitutionError::TypeNodeLimit)?;
                1
            }
            _ => 0,
        };
        let scope = scope
            .checked_add(introduced)
            .ok_or(SubstitutionError::BinderDepthOverflow)?;
        let mut exceeded = false;
        ty.for_each_child(|child| {
            if pending.len() >= remaining {
                exceeded = true;
            } else {
                pending.push((child, depth + 1, scope, mapping));
            }
        });
        if exceeded {
            return Err(SubstitutionError::TypeNodeLimit);
        }
    }
    Ok(())
}

/// Map only free parameters, presenting their IDs in the outer scope and the
/// number of intervening binders. This is the common traversal for substitution
/// and lifting, including nested quantified functions and compound types.
fn map_free_parameters(
    ty: &TypeRef,
    depth: u32,
    map: &mut impl FnMut(TypeParamId, u32) -> Result<TypeRef, SubstitutionError>,
) -> Result<TypeRef, SubstitutionError> {
    Ok(match ty {
        TypeRef::Generic(id) if u32::from(id.0) >= depth => {
            map(TypeParamId((u32::from(id.0) - depth) as u16), depth)?
        }
        TypeRef::Concrete(_)
        | TypeRef::Generic(_)
        | TypeRef::ConstValue(_) | TypeRef::ConstSequence(_)
        | TypeRef::RecordFieldProjection { .. } => ty.clone(),
        TypeRef::Instantiated { base, args } => TypeRef::Instantiated {
            base: *base,
            args: args
                .iter()
                .map(|ty| map_free_parameters(ty, depth, map))
                .collect::<Result<_, _>>()?,
        },
        TypeRef::Function {
            is_async,
            params,
            return_type,
            contexts,
        } => TypeRef::Function {
            is_async: *is_async,
            params: params
                .iter()
                .map(|ty| map_free_parameters(ty, depth, map))
                .collect::<Result<_, _>>()?,
            return_type: Box::new(map_free_parameters(return_type, depth, map)?),
            contexts: contexts.clone(),
        },
        TypeRef::Rank2Function {
            is_async,
            type_param_count,
            params,
            return_type,
            contexts,
        } => {
            let depth = depth
                .checked_add(u32::from(*type_param_count))
                .ok_or(SubstitutionError::BinderDepthOverflow)?;
            TypeRef::Rank2Function {
                is_async: *is_async,
                type_param_count: *type_param_count,
                params: params
                    .iter()
                    .map(|ty| map_free_parameters(ty, depth, map))
                    .collect::<Result<_, _>>()?,
                return_type: Box::new(map_free_parameters(return_type, depth, map)?),
                contexts: contexts.clone(),
            }
        }
        TypeRef::Dynamic(object) => {
            let depth = depth.checked_add(1).ok_or(SubstitutionError::BinderDepthOverflow)?;
            TypeRef::Dynamic(object.try_map(|ty| map_free_parameters(ty, depth, map))?)
        }
        TypeRef::Reference {
            inner,
            mutability,
            tier,
        } => TypeRef::Reference {
            inner: Box::new(map_free_parameters(inner, depth, map)?),
            mutability: *mutability,
            tier: *tier,
        },
        TypeRef::Tuple(elements) => TypeRef::Tuple(
            elements
                .iter()
                .map(|ty| map_free_parameters(ty, depth, map))
                .collect::<Result<_, _>>()?,
        ),
        TypeRef::Array { element, length } => TypeRef::Array {
            element: Box::new(map_free_parameters(element, depth, map)?),
            length: *length,
        },
        TypeRef::Slice(element) => {
            TypeRef::Slice(Box::new(map_free_parameters(element, depth, map)?))
        }
        TypeRef::AssociatedProjection { base, qualifier, assoc, args } => TypeRef::AssociatedProjection {
            base: Box::new(map_free_parameters(base, depth, map)?),
            qualifier: qualifier.as_ref().map(|application| {
                Ok(crate::types::ProtocolApplication {
                    protocol: application.protocol,
                    args: application.args.iter().map(|ty| map_free_parameters(ty, depth, map)).collect::<Result<_, SubstitutionError>>()?,
                })
            }).transpose()?,
            assoc: assoc.clone(),
            args: args.iter().map(|ty| map_free_parameters(ty, depth, map)).collect::<Result<_, _>>()?,
        },
    })
}

#[cfg(test)]
mod record_projection_tests {
    use super::*;
    use crate::types::TypeId;

    #[test]
    fn record_projection_is_not_a_free_callable_parameter() {
        let projection = TypeRef::RecordFieldProjection { owner: TypeId(2048), ordinal: u32::MAX };
        let mut substitution = TypeSubstitution::empty();
        substitution.bind(TypeParamId(0), TypeRef::Concrete(TypeId::INT));
        assert_eq!(substitution.apply(&projection).unwrap(), projection);
        assert_eq!(lift_free_parameters(&projection, u32::MAX).unwrap(), projection);
        let nested = TypeRef::Rank2Function {
            is_async: false,
            type_param_count: 1,
            params: vec![TypeRef::Generic(TypeParamId(1))],
            return_type: Box::new(projection.clone()),
            contexts: Default::default(),
        };
        let expected = TypeRef::Rank2Function {
            is_async: false,
            type_param_count: 1,
            params: vec![TypeRef::Concrete(TypeId::INT)],
            return_type: Box::new(projection),
            contexts: Default::default(),
        };
        assert_eq!(substitution.apply(&nested).unwrap(), expected);
    }
}
