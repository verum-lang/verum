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
        /// Concrete arguments of the declaring receiver, in declaration order.
        owner_args: List<TypeRef>,
        /// Declared Drop body.
        function: FunctionId,
        /// Witnesses in the body's declared parameter order.
        type_args: List<TypeRef>,
    },
    /// Contiguous owned record or tuple fields.
    Record {
        /// Exact nominal record, or the canonical structural tuple carrier.
        owner: TypeId,
        /// Concrete arguments of the declaration (empty for a structural tuple).
        owner_args: List<TypeRef>,
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
    /// Generic arguments of this exact, source-declared standard record.
    /// A runtime representation may interpret them only after checking its own storage.
    pub(crate) fn standard_record_arguments(
        &self,
        module: &VbcModule,
        expected: verum_common::well_known_types::WellKnownType,
    ) -> Option<&[TypeRef]> {
        let (owner, arguments) = match self {
            Self::Glue {
                owner, owner_args, ..
            }
            | Self::Record {
                owner, owner_args, ..
            } => (*owner, owner_args.as_slice()),
            _ => return None,
        };
        let declaration = module.get_type(owner)?;
        let origin = declaration
            .origin_module
            .and_then(|id| module.get_string(id))
            .unwrap_or(&module.name);
        let kind = verum_common::well_known_types::WellKnownType::from_declaration(
            module.get_string(declaration.name)?,
            origin,
        )?;
        (declaration.kind == TypeKind::Record && kind == expected).then_some(arguments)
    }

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
                owner_args: List::new(),
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
    if let Some(reference) = declaration.drop_fn {
        let function = module.resolved_function_id(reference);
        let descriptor = module
            .get_function(function)
            .ok_or_else(|| {
                let name = module.get_string(declaration.name).unwrap_or("<unnamed>");
                Text::from(format!("typed destruction of {name} refers to missing Drop function {reference}"))
            })?;
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
            owner_args: args.iter().cloned().collect(),
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
                let declared = substitution.apply(declared)
                    .map_err(|error| Text::from(error.to_string()))?;
                build_inner(module, &declared, active, budget)
            })
            .collect()
    };
    let result = match declaration.kind {
        TypeKind::Alias => {
            let target = declaration.alias_target.as_ref().ok_or_else(|| {
                Text::from("typed destruction alias has no resolved declaration target")
            })?;
            // Older producers used a pointer or Unit as the stand-in for an
            // unresolved alias. A written scalar target must attest these two
            // ambiguous encodings before either can authorize an empty drop.
            if matches!(target, TypeRef::Concrete(TypeId::UNIT | TypeId::PTR)) {
                let spelling = declaration.alias_target_name.and_then(|name| module.get_string(name));
                if spelling.and_then(TypeId::from_well_known_scalar_name) != target.base_type_id() {
                    return Err("typed destruction alias target lacks scalar declaration evidence".into());
                }
            }
            let target = substitution.apply(target)
                .map_err(|error| Text::from(error.to_string()))?;
            build_inner(module, &target, active, budget)
        }
        TypeKind::Record => fields(&declaration.fields).map(|fields| DropPlan::Record {
            owner,
            owner_args: args.iter().cloned().collect(),
            fields,
        }),
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
        _ => Err(format!(
            "typed destruction requires a record, sum, tuple or explicit Drop declaration; found {:?} {}",
            declaration.kind,
            module.get_string(declaration.name).unwrap_or("<unnamed>"),
        ).into()),
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
    module.specialized_function_id(function, args)
}
