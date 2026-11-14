//! Declaration-owned copy operations. Recursive types form a finite graph;
//! interpreting that graph must never traverse borrowed storage.
use crate::{
    module::{FunctionId, VbcModule},
    mono::TypeSubstitution,
    types::{FieldDescriptor, MAX_TYPE_REF_NODES, TypeId, TypeKind, TypeRef, VariantKind},
};
use verum_common::{List, Map, ResourceDiscipline, Text, well_known_types::WellKnownType};

/// Shared execution policy for one typed copy, in either tier. Exhaustion is
/// a runtime resource diagnostic, not a restriction on recursive language types.
/// Work bounds both visited edges and the explicit temporary workspace. There is
/// no independent type/value nesting limit and no use of the native call stack.
/// Allocation bytes count copied payload/backing capacity, excluding backend headers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CopyLimits {
    pub max_work: usize,
    pub max_allocation_bytes: usize,
}
pub const DEFAULT_COPY_LIMITS: CopyLimits = CopyLimits {
    max_work: 1 << 20,
    max_allocation_bytes: 64 << 20,
};

/// Internal operation failure shared by execution tiers. Zero denotes success
/// in the native checked helper; these codes are not serialized bytecode ABI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum CopyFailure {
    InvalidStorage = 1,
    OwnedCycle = 2,
    WorkLimit = 3,
    AllocationLimit = 4,
    AllocationFailure = 5,
    RetainRefused = 6,
    RollbackFailure = 7,
}
impl CopyFailure {
    pub const ALL: [Self; 7] = [
        Self::InvalidStorage,
        Self::OwnedCycle,
        Self::WorkLimit,
        Self::AllocationLimit,
        Self::AllocationFailure,
        Self::RetainRefused,
        Self::RollbackFailure,
    ];
    pub const fn message(self) -> &'static str {
        match self {
            Self::InvalidStorage => "typed copy requires valid declared owned storage",
            Self::OwnedCycle => "typed copy encountered a cycle in owned storage",
            Self::WorkLimit => "typed copy exhausted its work budget",
            Self::AllocationLimit => "typed copy exhausted its allocation budget",
            Self::AllocationFailure => "typed copy could not allocate operation storage",
            Self::RetainRefused => "retain_copy requires a nonzero, non-overflowing UInt32 counter",
            Self::RollbackFailure => "retained-copy rollback lost its live original owner",
        }
    }
}

/// Index in a checked copy plan. Node zero describes the requested value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeId(pub usize);

/// The storage operation established by one exact source declaration.
#[derive(Clone, Debug)]
pub enum CopyNode {
    /// Scalars, immutable text and explicit references retain their carrier.
    Identity,
    /// An explicit alias preserves the target's copy operation.
    Alias(NodeId),
    /// Record and tuple slots in declaration order.
    Record { owner: TypeId, fields: List<NodeId> },
    /// The runtime tag chooses one of these declared payloads.
    Sum {
        owner: TypeId,
        variants: List<(u32, List<NodeId>)>,
    },
    /// The source List declaration authorizes its owned element storage.
    List { owner: TypeId, element: NodeId },
    /// The Map owns both the key and value of every occupied bucket.
    Map {
        owner: TypeId,
        key: NodeId,
        value: NodeId,
    },
    /// A deque owns its initialized ring elements, never the spare capacity.
    Deque { owner: TypeId, element: NodeId },
    /// A fixed-size array owns exactly its declared number of elements.
    Array { element: NodeId, length: u64 },
    /// Shared ownership is retained by its exact source Clone implementation.
    Shared {
        owner: TypeId,
        function: FunctionId,
        type_args: List<TypeRef>,
        contract: crate::retain_forward::RetainForward,
    },
}

/// A bounded, fully resolved graph. Back-edges express recursive declarations.
#[derive(Clone, Debug)]
pub struct CopyPlan {
    pub nodes: List<CopyNode>,
}

struct Builder<'a> {
    module: &'a VbcModule,
    types: List<TypeRef>,
    indices: Map<TypeRef, NodeId>,
}

impl Builder<'_> {
    fn intern(&mut self, ty: TypeRef) -> Result<NodeId, Text> {
        if let Some(index) = self.indices.get(&ty) {
            return Ok(*index);
        }
        if self.types.len() >= MAX_TYPE_REF_NODES {
            return Err("typed copy exceeds the declaration graph limit".into());
        }
        let index = NodeId(self.types.len());
        self.indices.insert(ty.clone(), index);
        self.types.push(ty);
        Ok(index)
    }

    fn fields(
        &mut self,
        fields: &[FieldDescriptor],
        substitution: &TypeSubstitution,
    ) -> Result<List<NodeId>, Text> {
        fields
            .iter()
            .map(|field| {
                let ty = field
                    .declaration_type
                    .as_ref()
                    .ok_or_else(|| Text::from("typed copy field lacks its declaration type"))?;
                let ty = substitution
                    .apply(ty)
                    .map_err(|error| Text::from(error.to_string()))?;
                self.intern(ty)
            })
            .collect()
    }

    fn node(&mut self, ty: &TypeRef) -> Result<CopyNode, Text> {
        let (owner, args) = match ty {
            TypeRef::Reference { .. } => return Ok(CopyNode::Identity),
            TypeRef::Concrete(id)
                if id.is_builtin()
                    && *id != TypeId::PTR
                    && *id != TypeId::RESERVED
                    && id.well_known_name().is_some() =>
            {
                return Ok(CopyNode::Identity);
            }
            TypeRef::Concrete(id) => (*id, &[][..]),
            TypeRef::Instantiated { base, args } => (*base, args.as_slice()),
            TypeRef::Tuple(parts) => {
                return Ok(CopyNode::Record {
                    owner: TypeId::TUPLE,
                    fields: parts
                        .iter()
                        .map(|part| self.intern(part.clone()))
                        .collect::<Result<_, _>>()?,
                });
            }
            TypeRef::Array { element, length } => {
                return Ok(CopyNode::Array {
                    element: self.intern((**element).clone())?,
                    length: *length,
                });
            }
            _ => return Err("typed copy requires a resolved value declaration".into()),
        };
        let declaration = self
            .module
            .get_type(owner)
            .ok_or_else(|| Text::from("typed copy has no declaring owner"))?;
        if declaration.resource_discipline != ResourceDiscipline::Unrestricted {
            return Err("typed copy requires an unrestricted declaring owner".into());
        }
        if declaration.type_params.len() != args.len() {
            return Err("typed copy arguments do not match the declaring owner".into());
        }
        let substitution = TypeSubstitution::new(&declaration.type_params, args);
        let standard = self.module.declared_runtime_carrier(owner);
        if declaration.kind == TypeKind::Record {
            match standard {
                Some(WellKnownType::List) => {
                    let [element] = args else {
                        return Err("List copy requires its element declaration".into());
                    };
                    return Ok(CopyNode::List {
                        owner,
                        element: self.intern(element.clone())?,
                    });
                }
                Some(WellKnownType::Shared) => {
                    let reference = declaration.clone_fn.ok_or_else(|| {
                        Text::from("Shared copy requires its declared Clone body")
                    })?;
                    let function = self.module.resolved_function_id(reference);
                    let descriptor = self
                        .module
                        .get_function(function)
                        .ok_or_else(|| Text::from("Shared copy references a missing Clone body"))?;
                    let parameters = descriptor
                        .semantic_parameter_types()
                        .filter(|parameters| parameters.len() == 1)
                        .ok_or_else(|| Text::from("Shared Clone lacks its receiver declaration"))?;
                    let Some(TypeRef::Reference {
                        inner,
                        mutability: crate::types::Mutability::Immutable,
                        ..
                    }) = &parameters[0]
                    else {
                        return Err("Shared Clone requires a read-only receiver declaration".into());
                    };
                    let mut selected = TypeSubstitution::empty();
                    if !crate::mono::unify_declared_types(inner, ty, &mut selected) {
                        return Err("Shared Clone belongs to a different receiver".into());
                    }
                    let output = descriptor
                        .semantic_return
                        .as_ref()
                        .ok_or_else(|| Text::from("Shared Clone lacks its result declaration"))?;
                    if selected
                        .apply(output)
                        .map_err(|error| Text::from(error.to_string()))?
                        != *ty
                    {
                        return Err("Shared Clone returns a different declaration".into());
                    }
                    let type_args = descriptor
                        .type_params
                        .iter()
                        .map(|parameter| selected.get(parameter.id).cloned())
                        .collect::<Option<List<_>>>()
                        .ok_or_else(|| {
                            Text::from("Shared Clone has unresolved generic witnesses")
                        })?;
                    let contract = crate::retain_forward::RetainForward::verify(
                        self.module,
                        function,
                        &type_args,
                        ty,
                    )?;
                    return Ok(CopyNode::Shared {
                        owner,
                        function,
                        type_args,
                        contract,
                    });
                }
                Some(WellKnownType::Map) => {
                    let [key, value] = args else {
                        return Err("Map copy requires key and value declarations".into());
                    };
                    return Ok(CopyNode::Map {
                        owner,
                        key: self.intern(key.clone())?,
                        value: self.intern(value.clone())?,
                    });
                }
                Some(WellKnownType::Deque) => {
                    let [element] = args else {
                        return Err("Deque copy requires its element declaration".into());
                    };
                    return Ok(CopyNode::Deque {
                        owner,
                        element: self.intern(element.clone())?,
                    });
                }
                _ => {}
            }
        }
        match declaration.kind {
            TypeKind::Alias => {
                let target = declaration
                    .alias_target
                    .as_ref()
                    .ok_or_else(|| Text::from("typed copy alias has no declaration target"))?;
                let target = substitution
                    .apply(target)
                    .map_err(|error| Text::from(error.to_string()))?;
                Ok(CopyNode::Alias(self.intern(target)?))
            }
            TypeKind::Record | TypeKind::Newtype | TypeKind::Tuple => Ok(CopyNode::Record {
                owner,
                fields: self.fields(&declaration.fields, &substitution)?,
            }),
            TypeKind::Unit => Ok(CopyNode::Record {
                owner,
                fields: List::new(),
            }),
            TypeKind::Sum => {
                let mut variants = List::new();
                for variant in &declaration.variants {
                    let count = match variant.kind {
                        VariantKind::Unit => 0,
                        VariantKind::Tuple => usize::from(variant.arity),
                        VariantKind::Record => variant.fields.len(),
                    };
                    if variant.fields.len() != count {
                        return Err("typed copy variant payload declaration is incomplete".into());
                    }
                    variants.push((variant.tag, self.fields(&variant.fields, &substitution)?));
                }
                Ok(CopyNode::Sum { owner, variants })
            }
            _ => Err("typed copy has no owned declaration operation for this type".into()),
        }
    }
}

/// Build every owned component before touching runtime storage. Unresolved
/// descriptors, affine owners and nonregular expanding types are refused.
pub fn build(module: &VbcModule, ty: &TypeRef) -> Result<CopyPlan, Text> {
    if matches!(ty, TypeRef::Reference { .. }) {
        let mut nodes = List::new();
        nodes.push(CopyNode::Identity);
        return Ok(CopyPlan { nodes });
    }
    if module.resource_discipline(ty) != ResourceDiscipline::Unrestricted {
        return Err("typed copy requires a fully declared unrestricted value".into());
    }
    let mut builder = Builder {
        module,
        types: List::new(),
        indices: Map::new(),
    };
    builder.intern(ty.clone())?;
    let mut nodes = List::new();
    while nodes.len() < builder.types.len() {
        let ty = builder.types[nodes.len()].clone();
        nodes.push(builder.node(&ty)?);
    }
    for start in 0..nodes.len() {
        let mut current = NodeId(start);
        let mut remaining = nodes.len();
        while let CopyNode::Alias(next) = nodes[current.0] {
            if remaining == 0 {
                return Err("typed copy alias cycle has no value declaration".into());
            }
            remaining -= 1;
            current = next;
        }
    }
    Ok(CopyPlan { nodes })
}
