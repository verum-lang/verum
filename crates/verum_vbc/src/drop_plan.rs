//! Declaration-owned destruction shared by interpreted and native execution.
//! Runtime tags select a declared sum branch; they never establish ownership.
use crate::{
    module::{FunctionId, VbcModule},
    mono::TypeSubstitution,
    types::{TypeId, TypeKind, TypeRef},
};
use verum_common::{List, Text};

/// A complete cleanup operation. Explicit Drop implementations own their body;
/// structural cleanup visits only components with carried declaration types.
#[derive(Clone, Debug)]
pub enum DropPlan {
    /// Scalars and borrowed/raw carriers have no owned destructor.
    Ignore,
    /// A declared Drop method and its exact generic arguments.
    Glue {
        /// Exact nominal receiver.
        owner: TypeId,
        /// Declared Drop body.
        function: FunctionId,
        /// Witnesses in the body's declared parameter order.
        type_args: List<TypeRef>,
    },
    /// Contiguous owned record or tuple fields.
    Record {
        /// Exact nominal record, or the canonical structural tuple carrier.
        owner: TypeId,
        /// Payload slots in declaration order.
        fields: List<DropPlan>,
    },
    /// Nominal alternatives, each with its declared payload list.
    Sum {
        /// Exact nominal sum declaration.
        owner: TypeId,
        /// Declared tags and their payload slots.
        variants: List<(u32, List<DropPlan>)>,
    },
}

impl DropPlan {
    /// Generic destructor bodies required by this concrete plan.
    pub fn generic_calls(&self) -> List<(FunctionId, List<TypeRef>)> {
        let mut calls = List::new();
        self.collect_calls(&mut calls);
        calls
    }
    fn collect_calls(&self, calls: &mut List<(FunctionId, List<TypeRef>)>) {
        match self {
            Self::Glue {
                function,
                type_args,
                ..
            } if !type_args.is_empty() => calls.push((*function, type_args.clone())),
            Self::Record { fields, .. } => {
                for field in fields {
                    field.collect_calls(calls);
                }
            }
            Self::Sum { variants, .. } => {
                for (_, fields) in variants {
                    for field in fields {
                        field.collect_calls(calls);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Refuse unresolved/legacy ownership instead of interpreting layout carriers.
pub fn build(module: &VbcModule, ty: &TypeRef) -> Result<DropPlan, Text> {
    if !crate::mono::is_concrete_type(ty) {
        return Err("typed destruction has unresolved declaration witnesses".into());
    }
    build_inner(module, ty, &mut List::new(), &mut 4096)
}

fn build_inner(
    module: &VbcModule,
    ty: &TypeRef,
    active: &mut List<TypeRef>,
    budget: &mut usize,
) -> Result<DropPlan, Text> {
    if *budget == 0 || active.len() >= 64 || active.contains(ty) {
        return Err("typed destruction exceeds the finite declaration plan boundary".into());
    }
    *budget -= 1;
    let (owner, args) = match ty {
        TypeRef::Reference { .. } => return Ok(DropPlan::Ignore),
        TypeRef::Concrete(id)
            if id.is_builtin() && *id != TypeId::RESERVED && id.well_known_name().is_some() =>
        {
            return Ok(DropPlan::Ignore);
        }
        TypeRef::Concrete(id) => (*id, &[][..]),
        TypeRef::Instantiated { base, args } => (*base, args.as_slice()),
        TypeRef::Tuple(parts) => {
            active.push(ty.clone());
            let fields = parts
                .iter()
                .map(|part| build_inner(module, part, active, budget))
                .collect::<Result<List<_>, Text>>();
            active.pop();
            return fields.map(|fields| DropPlan::Record {
                owner: TypeId::TUPLE,
                fields,
            });
        }
        _ => {
            return Err(
                format!("typed destruction lacks a concrete owned declaration: {ty:?}").into(),
            );
        }
    };
    let declaration = module
        .get_type(owner)
        .ok_or_else(|| Text::from(format!("typed destruction has no descriptor for {owner:?}")))?;
    if declaration.type_params.len() != args.len() {
        return Err("typed destruction generic arity does not match its declaring owner".into());
    }
    if let Some(function) = declaration.drop_fn {
        let function = FunctionId(function);
        let descriptor = module
            .get_function(function)
            .ok_or_else(|| Text::from("typed destruction refers to a missing Drop body"))?;
        let receiver = descriptor
            .semantic_parameter_types()
            .and_then(|params| (params.len() == 1).then(|| params[0].as_ref()).flatten())
            .ok_or_else(|| {
                Text::from("typed destruction Drop receiver has no declaration evidence")
            })?;
        let TypeRef::Reference {
            inner: receiver, ..
        } = receiver
        else {
            return Err("typed destruction Drop receiver is not a borrowed owner".into());
        };
        let mut substitution = TypeSubstitution::empty();
        if !crate::mono::unify_declared_types(receiver, ty, &mut substitution) {
            return Err("typed destruction Drop receiver belongs to another owner".into());
        }
        let type_args = descriptor
            .type_params
            .iter()
            .map(|parameter| substitution.get(parameter.id).cloned())
            .collect::<Option<List<_>>>()
            .ok_or_else(|| Text::from("typed destruction Drop generic evidence is incomplete"))?;
        if descriptor.bytecode_length == 0
            && descriptor
                .instructions
                .as_ref()
                .is_none_or(|body| body.is_empty())
        {
            return Err("typed destruction Drop body is unavailable".into());
        }
        return Ok(DropPlan::Glue {
            owner,
            function,
            type_args,
        });
    }
    let substitution = TypeSubstitution::new(&declaration.type_params, args);
    active.push(ty.clone());
    let mut fields = |fields: &[crate::types::FieldDescriptor]| -> Result<List<DropPlan>, Text> {
        fields
            .iter()
            .map(|field| {
                let declared = field.declaration_type.as_ref().ok_or_else(|| {
                    Text::from("typed destruction field lacks declaration evidence")
                })?;
                build_inner(module, &substitution.apply(declared), active, budget)
            })
            .collect()
    };
    let result = match declaration.kind {
        TypeKind::Record => {
            fields(&declaration.fields).map(|fields| DropPlan::Record { owner, fields })
        }
        TypeKind::Sum => declaration
            .variants
            .iter()
            .map(|variant| {
                let expected = match variant.kind {
                    crate::types::VariantKind::Unit => 0,
                    crate::types::VariantKind::Tuple => usize::from(variant.arity),
                    crate::types::VariantKind::Record => variant.fields.len(),
                };
                if variant.fields.len() != expected {
                    return Err(
                        "typed destruction variant payload declaration is incomplete".into(),
                    );
                }
                fields(&variant.fields).map(|fields| (variant.tag, fields))
            })
            .collect::<Result<List<_>, Text>>()
            .map(|variants| DropPlan::Sum { owner, variants }),
        TypeKind::Unit => Ok(DropPlan::Ignore),
        TypeKind::Primitive => {
            Err("typed destruction nominal primitive lacks an owned declaration".into())
        }
        _ => Err(
            "typed destruction requires a record, sum, tuple or explicit Drop declaration".into(),
        ),
    };
    active.pop();
    result
}

/// Find a native destructor using the exact specialization receipt, never its
/// generated display name (which is not an injective type identity).
pub fn native_glue(
    module: &VbcModule,
    function: FunctionId,
    args: &[TypeRef],
) -> Option<FunctionId> {
    if args.is_empty() {
        return Some(function);
    }
    let receipt = module
        .specializations
        .iter()
        .find(|entry| entry.generic_fn == function && entry.type_args == args)?;
    let mut bodies = module.functions.iter().filter(|body| {
        body.type_params.is_empty()
            && body.bytecode_offset == receipt.bytecode_offset
            && body.bytecode_length == receipt.bytecode_length
    });
    let target = bodies.next()?.id;
    bodies.next().is_none().then_some(target)
}
