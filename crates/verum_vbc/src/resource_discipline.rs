//! Effective usage constraints follow exact descriptors and owned components.
//! This query supplies no duplication operation or cleanup obligation.

use crate::associated_types::{DeclarationLookup, instantiate_witness, normalize_projections};
use crate::module::VbcModule;
use crate::mono::TypeSubstitution;
use crate::types::{
    MAX_TYPE_REF_DEPTH, MAX_TYPE_REF_NODES, StringId, TypeId, TypeKind,
    TypeParamId, TypeRef,
};
use verum_common::{List, Map, ResourceDiscipline, Text, well_known_types::WellKnownType};

enum ResolvedDiscipline {
    Unrestricted,
    /// The first declaration imposing the strongest constraint in this graph.
    Restricted {
        kind: ResourceDiscipline,
        owner: TypeId,
    },
}

impl ResolvedDiscipline {
    fn kind(&self) -> ResourceDiscipline {
        match self {
            Self::Unrestricted => ResourceDiscipline::Unrestricted,
            Self::Restricted { kind, .. } => *kind,
        }
    }
}

fn ownership_type_name(module: &VbcModule, id: TypeId) -> Text {
    let name = module.display_type_id(id);
    let origin = module
        .get_type(id)
        .and_then(|ty| ty.origin_module)
        .and_then(|origin| module.get_string(origin));
    match origin {
        Some(origin) if !name.contains('.') => format!("{origin}.{name} (type #{})", id.0).into(),
        _ => format!("{name} (type #{})", id.0).into(),
    }
}

/// Evidence lost when an ownership query is reduced to its conservative
/// `Unknown` result. Keep IDs from the traversed declarations until rendering.
enum DisciplineError {
    MissingDeclaration(TypeId),
    UndeclaredDiscipline(TypeId),
    Arity(TypeId, usize, usize),
    UnsupportedOwner(TypeId),
    InvalidMember(TypeId, Option<StringId>),
    MissingStorageArgument(TypeId, usize),
    UnboundParameter(TypeParamId),
    UnresolvedType,
    InvalidTypeShape,
    WorkLimit,
    InlineCycle,
}

impl DisciplineError {
    fn describe(self, module: &VbcModule) -> Text {
        let owner = |id| ownership_type_name(module, id);
        match self {
            Self::MissingDeclaration(id) => format!("missing VBC declaration for {}", owner(id)),
            Self::UndeclaredDiscipline(id) => {
                format!("{} has no declared ownership discipline", owner(id))
            }
            Self::Arity(id, expected, supplied) => format!(
                "{} expects {expected} type arguments, received {supplied}",
                owner(id)
            ),
            Self::UnsupportedOwner(id) => {
                format!("{} does not declare owned value storage", owner(id))
            }
            Self::InvalidMember(id, member) => {
                let component = match member {
                    Some(name) => module
                        .get_string(name)
                        .map(str::to_owned)
                        .unwrap_or_else(|| format!("member #{}", name.0)),
                    None => "alias target".into(),
                };
                format!(
                    "{} has no resolved ownership type for {component}",
                    owner(id)
                )
            }
            Self::MissingStorageArgument(id, index) => {
                format!("{} lacks owned storage argument {index}", owner(id))
            }
            Self::UnboundParameter(id) => {
                format!("missing ownership witness for type parameter T{}", id.0)
            }
            Self::UnresolvedType => "unresolved ownership type".into(),
            Self::InvalidTypeShape => "invalid or oversized ownership type".into(),
            Self::WorkLimit => "ownership graph exceeds the traversal budget".into(),
            Self::InlineCycle => "ownership graph contains a cycle through inline storage".into(),
        }
        .into()
    }
}

impl VbcModule {
    /// Resolve usage discipline from exact declarations and substituted owned fields.
    /// Unknown components or missing/legacy descriptors never imply unrestricted use.
    pub fn resource_discipline(&self, ty: &TypeRef) -> ResourceDiscipline {
        self.resolve_resource_discipline(ty)
            .map_or(ResourceDiscipline::Unknown, |resolved| resolved.kind())
    }

    fn resolve_resource_discipline(
        &self,
        ty: &TypeRef,
    ) -> Result<ResolvedDiscipline, DisciplineError> {
        resolve_discipline(
            self,
            ty,
            |id| self.declared_runtime_carrier(id),
        )
    }

    pub(crate) fn require_unrestricted_ownership(&self, ty: &TypeRef) -> Result<(), Text> {
        let resolved = self
            .resolve_resource_discipline(ty)
            .map_err(|error| error.describe(self))?;
        match resolved {
            ResolvedDiscipline::Unrestricted => Ok(()),
            ResolvedDiscipline::Restricted { kind, owner } => Err(format!(
                "effective ownership is {kind:?} through {}",
                ownership_type_name(self, owner)
            )
            .into()),
        }
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
            if let Err(reason) = self.require_unrestricted_ownership(&ty) {
                return Err(format!(
                    "shared callable capture {index} requires unrestricted ownership: {reason}"
                ));
            }
        }
        Ok(())
    }
}

/// The same bounded query used by a complete module and by source production.
pub(crate) fn resource_discipline_with(
    declarations: &impl DeclarationLookup,
    ty: &TypeRef,
    carrier: impl Fn(TypeId) -> Option<WellKnownType>,
) -> ResourceDiscipline {
    resolve_discipline(declarations, ty, carrier)
        .map_or(ResourceDiscipline::Unknown, |resolved| resolved.kind())
}

fn resolve_discipline(
    declarations: &impl DeclarationLookup,
    ty: &TypeRef,
    carrier: impl Fn(TypeId) -> Option<WellKnownType>,
) -> Result<ResolvedDiscipline, DisciplineError> {
    if is_reference_or_callable(ty) {
        return Ok(ResolvedDiscipline::Unrestricted);
    }
    if !bounded_type_shape(ty) {
        return Err(DisciplineError::InvalidTypeShape);
    }

    let mut types = List::new();
    let mut root = ty.clone();
    normalize_projections(declarations, &mut root).map_err(|_| DisciplineError::InvalidTypeShape)?;
    let mut indices = Map::new();
    indices.insert(root.clone(), 0usize);
    types.push(root);
    let mut direct_edges: List<List<usize>> = List::new();
    let mut remaining = MAX_TYPE_REF_NODES;
    let mut result = ResolvedDiscipline::Unrestricted;
    let mut cursor = 0;
    while cursor < types.len() {
        let ty = types[cursor].clone();
        let mut children: List<(TypeRef, bool)> = List::new();
        let nominal = match &ty {
            TypeRef::Concrete(id) => Some((*id, &[][..])),
            TypeRef::Instantiated { base, args } => Some((*base, args.as_slice())),
            TypeRef::Tuple(parts) => {
                if parts.len() > remaining {
                    return Err(DisciplineError::WorkLimit);
                }
                children.extend(parts.iter().cloned().map(|part| (part, false)));
                None
            }
            TypeRef::Array { element, .. } => {
                children.push(((**element).clone(), false));
                None
            }
            _ if is_reference_or_callable(&ty) => None,
            TypeRef::Generic(id) => return Err(DisciplineError::UnboundParameter(*id)),
            _ => return Err(DisciplineError::UnresolvedType),
        };
        if let Some((id, args)) = nominal {
            if id.is_builtin() && id != TypeId::PTR && id != TypeId::RESERVED {
                if !args.is_empty() {
                    return Err(DisciplineError::Arity(id, 0, args.len()));
                }
            } else {
                let descriptor = declarations.descriptor(id).ok_or(DisciplineError::MissingDeclaration(id))?;
                let combined = result.kind().with_component(descriptor.resource_discipline);
                if combined == ResourceDiscipline::Unknown {
                    return Err(DisciplineError::UndeclaredDiscipline(id));
                }
                if combined != result.kind() {
                    result = ResolvedDiscipline::Restricted {
                        kind: combined,
                        owner: id,
                    };
                }
                if descriptor.type_params.len() != args.len() {
                    return Err(DisciplineError::Arity(
                        id,
                        descriptor.type_params.len(),
                        args.len(),
                    ));
                }
                if matches!(descriptor.kind, TypeKind::Protocol | TypeKind::Tensor) {
                    return Err(DisciplineError::UnsupportedOwner(id));
                }
                let substitution = TypeSubstitution::new(&descriptor.type_params, args);
                let mut add_declared = |declared: Option<&TypeRef>, member| {
                    let invalid = || DisciplineError::InvalidMember(id, member);
                    let declared = declared.ok_or_else(invalid)?;
                    if children.len() >= remaining {
                        return Err(DisciplineError::WorkLimit);
                    }
                    if !bounded_type_shape(declared) {
                        return Err(DisciplineError::InvalidTypeShape);
                    }
                    let value = instantiate_witness(declarations, &substitution, declared)
                        .map_err(|_| invalid())?;
                    if !bounded_type_shape(&value) {
                        return Err(DisciplineError::InvalidTypeShape);
                    }
                    // The borrowed boundary changes ownership, not the validity
                    // of its declaration-owned parameter substitution.
                    if !is_reference_or_callable(&value) {
                        children.push((value, false));
                    }
                    Ok(())
                };
                if descriptor.kind == TypeKind::Alias {
                    add_declared(descriptor.alias_target.as_ref(), None)?;
                } else {
                    for field in &descriptor.fields {
                        add_declared(field.declaration_type.as_ref(), Some(field.name))?;
                    }
                    for variant in &descriptor.variants {
                        if let Some(payload) = &variant.payload {
                            add_declared(Some(payload), Some(variant.name))?;
                        }
                        for field in &variant.fields {
                            add_declared(field.declaration_type.as_ref(), Some(field.name))?;
                        }
                    }
                }
                if let Some(carrier) = carrier(id) {
                    for &position in carrier.owned_storage_parameters() {
                        let argument = args
                            .get(position)
                            .ok_or(DisciplineError::MissingStorageArgument(id, position))?;
                        if children.len() >= remaining {
                            return Err(DisciplineError::WorkLimit);
                        }
                        children.push((argument.clone(), true));
                    }
                }
            }
        }

        let mut edges = List::new();
        for (child, storage_edge) in children {
            if remaining == 0 {
                return Err(DisciplineError::WorkLimit);
            }
            remaining -= 1;
            if is_reference_or_callable(&child) {
                continue;
            }
            let next = if let Some(&next) = indices.get(&child) {
                next
            } else {
                if types.len() >= MAX_TYPE_REF_NODES {
                    return Err(DisciplineError::WorkLimit);
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
        Ok(result)
    } else {
        Err(DisciplineError::InlineCycle)
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
