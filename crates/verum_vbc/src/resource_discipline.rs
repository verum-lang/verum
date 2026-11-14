//! Effective usage constraints follow exact descriptors and owned components.
//! This query supplies no duplication operation or cleanup obligation.

use crate::module::VbcModule;
use crate::mono::TypeSubstitution;
use crate::types::{
    MAX_TYPE_REF_DEPTH, MAX_TYPE_REF_NODES, TypeDescriptor, TypeId, TypeKind, TypeRef,
};
use verum_common::{List, Map, ResourceDiscipline, well_known_types::WellKnownType};

impl VbcModule {
    /// Resolve usage discipline from exact declarations and substituted owned fields.
    /// Unknown components or missing/legacy descriptors never imply unrestricted use.
    pub fn resource_discipline(&self, ty: &TypeRef) -> ResourceDiscipline {
        resource_discipline_with(
            ty,
            |id| self.get_type(id),
            |id| self.declared_runtime_carrier(id),
        )
    }

    /// The canonical runtime carrier of an exact source-owned record.
    pub fn declared_runtime_carrier(&self, id: TypeId) -> Option<WellKnownType> {
        let declaration = self.get_type(id)?;
        if declaration.kind != TypeKind::Record {
            return None;
        }
        let owner = match declaration.origin_module {
            Some(owner) => self.get_string(owner)?,
            None => self.name.as_str(),
        };
        WellKnownType::from_declaration(self.get_string(declaration.name)?, owner)
    }

    /// Ordinary function values share their environment. Admission checks the
    /// original captured declarations, including mutable cell contents, before
    /// allocation. ABI slots and runtime object tags supply no such authority.
    /// Capture type parameters belong to the creating function's binder.
    pub fn validate_closure_captures(
        &self,
        types: Option<&[Option<TypeRef>]>,
        count: usize,
        resolve: impl Fn(&TypeRef) -> TypeRef,
    ) -> Result<(), String> {
        let Some(types) = types else {
            return if count == 0 {
                Ok(())
            } else {
                Err("shared callable requires declared capture types; legacy environment has no ownership evidence".into())
            };
        };
        if types.len() != count {
            return Err(format!(
                "shared callable capture arity mismatch: declared {}, supplied {count}",
                types.len()
            ));
        }
        for (index, ty) in types.iter().enumerate() {
            let Some(ty) = ty else {
                return Err(format!(
                    "shared callable capture {index} has no declaration type"
                ));
            };
            let ty = resolve(ty);
            let discipline = self.resource_discipline(&ty);
            if discipline != ResourceDiscipline::Unrestricted {
                return Err(format!(
                    "shared callable capture {index} requires unrestricted ownership; found {discipline:?} for {ty:?}"
                ));
            }
        }
        Ok(())
    }
}

/// The same bounded query used by a complete module and by source production.
/// Both callbacks resolve the same exact declaration ID.
pub fn resource_discipline_with<'a>(
    ty: &TypeRef,
    resolve: impl Fn(TypeId) -> Option<&'a TypeDescriptor>,
    carrier: impl Fn(TypeId) -> Option<WellKnownType>,
) -> ResourceDiscipline {
    use ResourceDiscipline::{Unknown, Unrestricted};
    if is_reference_or_callable(ty) {
        return Unrestricted;
    }
    if !bounded_type_shape(ty) {
        return Unknown;
    }

    let mut types = List::new();
    types.push(ty.clone());
    let mut indices = Map::new();
    indices.insert(ty.clone(), 0usize);
    let mut direct_edges: List<List<usize>> = List::new();
    let mut remaining = MAX_TYPE_REF_NODES;
    let mut mode = Unrestricted;
    let mut cursor = 0;
    while cursor < types.len() {
        let ty = types[cursor].clone();
        let mut children: List<(TypeRef, bool)> = List::new();
        let nominal = match &ty {
            TypeRef::Concrete(id) => Some((*id, &[][..])),
            TypeRef::Instantiated { base, args } => Some((*base, args.as_slice())),
            TypeRef::Tuple(parts) => {
                if parts.len() > remaining {
                    return Unknown;
                }
                children.extend(parts.iter().cloned().map(|part| (part, false)));
                None
            }
            TypeRef::Array { element, .. } => {
                children.push(((**element).clone(), false));
                None
            }
            _ if is_reference_or_callable(&ty) => None,
            _ => return Unknown,
        };
        if let Some((id, args)) = nominal {
            if id.is_builtin() && id != TypeId::PTR && id != TypeId::RESERVED {
                if !args.is_empty() {
                    return Unknown;
                }
            } else {
                let Some(descriptor) = resolve(id) else {
                    return Unknown;
                };
                mode = mode.with_component(descriptor.resource_discipline);
                if mode == Unknown
                    || descriptor.type_params.len() != args.len()
                    || matches!(descriptor.kind, TypeKind::Protocol | TypeKind::Tensor)
                {
                    return Unknown;
                }
                let substitution = TypeSubstitution::new(&descriptor.type_params, args);
                let mut add_declared = |declared: Option<&TypeRef>| -> Option<()> {
                    let declared = declared?;
                    if children.len() >= remaining || !bounded_type_shape(declared) {
                        return None;
                    }
                    let value = substitution.apply(declared).ok()?;
                    if !bounded_type_shape(&value) {
                        return None;
                    }
                    // The borrowed boundary changes ownership, not the validity
                    // of its declaration-owned parameter substitution.
                    if is_reference_or_callable(&value) {
                        return Some(());
                    }
                    children.push((value, false));
                    Some(())
                };
                if descriptor.kind == TypeKind::Alias {
                    if add_declared(descriptor.alias_target.as_ref()).is_none() {
                        return Unknown;
                    }
                } else {
                    for field in &descriptor.fields {
                        if add_declared(field.declaration_type.as_ref()).is_none() {
                            return Unknown;
                        }
                    }
                    for variant in &descriptor.variants {
                        if let Some(payload) = &variant.payload
                            && add_declared(Some(payload)).is_none()
                        {
                            return Unknown;
                        }
                        for field in &variant.fields {
                            if add_declared(field.declaration_type.as_ref()).is_none() {
                                return Unknown;
                            }
                        }
                    }
                }
                if let Some(carrier) = carrier(id) {
                    for &position in carrier.owned_storage_parameters() {
                        let Some(argument) = args.get(position) else {
                            return Unknown;
                        };
                        if children.len() >= remaining {
                            return Unknown;
                        }
                        children.push((argument.clone(), true));
                    }
                }
            }
        }

        let mut edges = List::new();
        for (child, storage_edge) in children {
            if remaining == 0 {
                return Unknown;
            }
            remaining -= 1;
            if is_reference_or_callable(&child) {
                continue;
            }
            let next = if let Some(&next) = indices.get(&child) {
                next
            } else {
                if types.len() >= MAX_TYPE_REF_NODES {
                    return Unknown;
                }
                let next = types.len();
                indices.insert(child.clone(), next);
                types.push(child);
                next
            };
            if !storage_edge {
                edges.push(next);
            }
        }
        direct_edges.push(edges);
        cursor += 1;
    }

    // An owning container supplies finite storage for a recursive value.
    // Every remaining by-value edge must be acyclic. Check the entire graph,
    // including paths shared with an already visited container branch.
    let mut incoming = List::new();
    incoming.resize(types.len(), 0usize);
    for edges in &direct_edges {
        for &child in edges {
            incoming[child] += 1;
        }
    }
    let mut ready: List<usize> = incoming
        .iter()
        .enumerate()
        .filter_map(|(index, &count)| (count == 0).then_some(index))
        .collect();
    let mut visited = 0;
    while let Some(index) = ready.pop() {
        visited += 1;
        for &child in &direct_edges[index] {
            incoming[child] -= 1;
            if incoming[child] == 0 {
                ready.push(child);
            }
        }
    }
    if visited == types.len() {
        mode
    } else {
        Unknown
    }
}

fn is_reference_or_callable(ty: &TypeRef) -> bool {
    // Callable construction validates its captures separately. A function's
    // parameter and result declarations are not its environment storage.
    matches!(
        ty,
        TypeRef::Reference { .. }
            | TypeRef::Slice(_)
            | TypeRef::Function { .. }
            | TypeRef::Rank2Function { .. }
    )
}

fn bounded_type_shape(ty: &TypeRef) -> bool {
    let mut pending = List::new();
    pending.push((ty, 0usize));
    let mut remaining = MAX_TYPE_REF_NODES;
    while let Some((ty, depth)) = pending.pop() {
        if remaining == 0 || depth >= MAX_TYPE_REF_DEPTH {
            return false;
        }
        remaining -= 1;
        if let TypeRef::ConstSequence(sequence) = ty
            && sequence.validate().is_err()
        {
            return false;
        }
        let mut exceeded = false;
        ty.for_each_child(|child| {
            if pending.len() >= remaining {
                exceeded = true;
            } else {
                pending.push((child, depth + 1));
            }
        });
        if exceeded {
            return false;
        }
    }
    true
}
