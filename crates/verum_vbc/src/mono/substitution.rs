//! Type substitution for monomorphization.

use std::collections::HashMap;

use crate::module::FunctionDescriptor;
use crate::types::{TypeParamDescriptor, TypeParamId, TypeRef};

/// Type substitution environment for monomorphization.
///
/// Maps type parameters to concrete types.
pub struct TypeSubstitution {
    /// Type parameter bindings: TypeParamId -> concrete TypeRef.
    bindings: HashMap<TypeParamId, TypeRef>,
}

impl TypeSubstitution {
    /// Creates a new substitution from type parameters and arguments.
    pub fn new(params: &[TypeParamDescriptor], args: &[TypeRef]) -> Self {
        let mut bindings = HashMap::new();
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
        let mut bindings = HashMap::new();
        for (i, arg) in args.iter().enumerate() {
            bindings.insert(crate::types::TypeParamId(i as u16), arg.clone());
        }
        Self { bindings }
    }

    /// Creates an empty substitution.
    pub fn empty() -> Self {
        Self {
            bindings: HashMap::new(),
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
    /// Recursively substitutes type parameters with their bindings.
    pub fn apply(&self, type_ref: &TypeRef) -> Result<TypeRef, SubstitutionError> {
        map_free_parameters(type_ref, 0, &mut |id, depth| match self.bindings.get(&id) {
            Some(replacement) => lift_free_parameters(replacement, depth),
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
    GenericIndexOverflow { index: u16, shift: u32 },
    BinderDepthOverflow,
}

impl std::fmt::Display for SubstitutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::GenericIndexOverflow { index, shift } => write!(
                f,
                "type parameter index {index} cannot be lifted by {shift} in the VBC binder representation"
            ),
            Self::BinderDepthOverflow => write!(f, "VBC type binder depth is not representable"),
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
    map_free_parameters(ty, 0, &mut |id, depth| {
        let shift = depth
            .checked_add(amount)
            .ok_or(SubstitutionError::BinderDepthOverflow)?;
        lifted_parameter(id, shift)
    })
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
        TypeRef::Concrete(_) | TypeRef::Generic(_) | TypeRef::ConstValue(_) => ty.clone(),
        TypeRef::Instantiated { base, args } => TypeRef::Instantiated {
            base: *base,
            args: args
                .iter()
                .map(|ty| map_free_parameters(ty, depth, map))
                .collect::<Result<_, _>>()?,
        },
        TypeRef::Function {
            params,
            return_type,
            contexts,
        } => TypeRef::Function {
            params: params
                .iter()
                .map(|ty| map_free_parameters(ty, depth, map))
                .collect::<Result<_, _>>()?,
            return_type: Box::new(map_free_parameters(return_type, depth, map)?),
            contexts: contexts.clone(),
        },
        TypeRef::Rank2Function {
            type_param_count,
            params,
            return_type,
            contexts,
        } => {
            let depth = depth
                .checked_add(u32::from(*type_param_count))
                .ok_or(SubstitutionError::BinderDepthOverflow)?;
            TypeRef::Rank2Function {
                type_param_count: *type_param_count,
                params: params
                    .iter()
                    .map(|ty| map_free_parameters(ty, depth, map))
                    .collect::<Result<_, _>>()?,
                return_type: Box::new(map_free_parameters(return_type, depth, map)?),
                contexts: contexts.clone(),
            }
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
        TypeRef::AssociatedProjection { base, assoc } => TypeRef::AssociatedProjection {
            base: Box::new(map_free_parameters(base, depth, map)?),
            assoc: assoc.clone(),
        },
    })
}
