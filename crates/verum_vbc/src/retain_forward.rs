//! Bounded proof that the selected callable forwards exactly one RetainCopy.
//!
//! This is not a Clone evaluator. Unknown calls, effects, control flow and
//! provenance refuse the contract; no name or instruction-presence heuristic
//! grants retained ownership. Receipts are local to a checked copy operation.
use crate::{
    FunctionId, Instruction, Reg, TypeRef, VbcModule,
    instruction::CbgrSubOpcode,
    mono::TypeSubstitution,
    types::{CbgrTier, MAX_TYPE_REF_NODES, Mutability, TypeId, TypeKind},
    value_use::ValueUsePlan,
};
use verum_cbgr::analysis::BlockId;
use verum_common::{List, Set, Text};

/// One declaration-owned field selected from the receiver's value provenance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RetainField {
    /// Exact record application, after substituting the callable witnesses.
    pub record: TypeRef,
    /// Field index in that record declaration.
    pub index: u16,
    /// Exact field declaration, including reference permissions.
    pub field: TypeRef,
    declaration_hash: [u8; 32],
}

/// Proof of a complete intrinsic-forward body. Fields are deliberately private:
/// consumers may inspect the selected path but cannot invent authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RetainForward {
    function: FunctionId,
    arguments: List<TypeRef>,
    owner: TypeRef,
    counter: List<RetainField>,
    body_hash: [u8; 32],
    signature_hash: [u8; 32],
    identity_hash: [u8; 32],
}

#[derive(Clone)]
enum Origin {
    Receiver,
    Field { parent: usize, field: RetainField },
    Counter { parent: usize, field: RetainField },
    Retained,
}
#[derive(Clone)]
struct Fact {
    ty: TypeRef,
    origin: Origin,
}

fn invalid(message: &str) -> Text {
    format!("retained-copy contract: {message}").into()
}

fn field(module: &VbcModule, ty: &TypeRef, index: u16, borrow: bool) -> Result<RetainField, Text> {
    let mut record = ty;
    // Field selection does not strengthen safe-reference permissions. An
    // explicitly unsafe reference delegates the proof to its source contract.
    if let TypeRef::Reference {
        inner,
        tier,
        mutability,
    } = record
    {
        if borrow && *tier != CbgrTier::Tier2 && *mutability != Mutability::Mutable {
            return Err(invalid(
                "counter projection strengthens a read-only receiver",
            ));
        }
        record = inner;
    } else if borrow {
        return Err(invalid(
            "counter projection has no declared mutable or unsafe place",
        ));
    }
    let (owner, args) = match record {
        TypeRef::Concrete(owner) => (*owner, &[][..]),
        TypeRef::Instantiated { base, args } => (*base, args.as_slice()),
        _ => {
            return Err(invalid(
                "field projection lacks its exact record declaration",
            ));
        }
    };
    let descriptor = module
        .get_type(owner)
        .filter(|d| d.kind == TypeKind::Record && d.type_params.len() == args.len())
        .ok_or_else(|| invalid("field projection has a different record owner or roster"))?;
    let selected_field = descriptor
        .fields
        .get(usize::from(index))
        .ok_or_else(|| invalid("field projection has no declared field"))?;
    let declared = selected_field
        .declaration_type
        .as_ref()
        .ok_or_else(|| invalid("field projection has no declared field type"))?;
    let field = TypeSubstitution::new(&descriptor.type_params, args)
        .apply(declared)
        .map_err(|e| invalid(&e.to_string()))?;
    if borrow && field != TypeRef::Concrete(TypeId::U32) {
        return Err(invalid(
            "counter projection is not the declared UInt32 slot",
        ));
    }
    // Seal the selected declaration, not just its ordinal or rendered type.
    // String contents participate because StringId is local to one module.
    let identity = bincode::serialize(&(
        descriptor.id,
        descriptor.kind,
        &descriptor.type_params,
        descriptor.fields.len(),
        descriptor.resource_discipline,
        descriptor.size,
        descriptor.alignment,
        selected_field,
    ))
    .map_err(|_| invalid("cannot seal selected field declaration"))?;
    let mut hash = blake3::Hasher::new();
    hash.update(&identity);
    for name in [
        Some(descriptor.name),
        descriptor.origin_module,
        Some(selected_field.name),
    ] {
        let name = name.and_then(|id| module.get_string(id)).unwrap_or("");
        hash.update(&(name.len() as u64).to_le_bytes());
        hash.update(name.as_bytes());
    }
    Ok(RetainField {
        record: record.clone(),
        index,
        field,
        declaration_hash: *hash.finalize().as_bytes(),
    })
}

impl RetainForward {
    /// Verify the exact selected source body and all its reachable exits. This
    /// bounded proof currently accepts one acyclic ordinary path; a branch is
    /// Unknown, not permission to ignore the path it could select.
    pub fn verify(
        module: &VbcModule,
        function: FunctionId,
        arguments: &[TypeRef],
        owner: &TypeRef,
    ) -> Result<Self, Text> {
        let descriptor = module
            .get_function(function)
            .filter(|d| {
                d.id == function
                    && d.has_source_body
                    && d.declaration_identity.is_some()
                    && d.source_signature.is_some()
                    && !d.is_generator
                    && d.yield_type.is_none()
                    && d.contexts.is_empty()
                    && d.intrinsic_name.is_none()
            })
            .ok_or_else(|| invalid("selected callable lacks a complete ordinary source body"))?;
        if descriptor.type_params.len() != arguments.len()
            || arguments.iter().any(TypeRef::is_generic)
        {
            return Err(invalid(
                "selected callable has unresolved or incorrect witnesses",
            ));
        }
        crate::retain_copy::layout(module, owner)?;
        let substitution = TypeSubstitution::new(&descriptor.type_params, arguments);
        let params = descriptor
            .semantic_parameter_types()
            .filter(|p| p.len() == 1)
            .ok_or_else(|| {
                invalid("selected callable must declare its single borrowed receiver")
            })?;
        let receiver = substitution
            .apply(
                params[0]
                    .as_ref()
                    .ok_or_else(|| invalid("missing receiver type"))?,
            )
            .map_err(|e| invalid(&e.to_string()))?;
        if !matches!(&receiver, TypeRef::Reference { inner, mutability: Mutability::Immutable, .. } if inner.as_ref() == owner)
        {
            return Err(invalid(
                "selected receiver does not borrow the copied owner",
            ));
        }
        let result = descriptor
            .semantic_return
            .as_ref()
            .ok_or_else(|| invalid("missing result type"))?;
        if substitution
            .apply(result)
            .map_err(|e| invalid(&e.to_string()))?
            != *owner
        {
            return Err(invalid("selected result differs from the copied owner"));
        }
        let (body, cfg) = module
            .ordinary_control_flow(function)
            .ok_or_else(|| invalid("body cannot be decoded as bounded ordinary control flow"))?;
        if body.is_empty()
            || body.len() > MAX_TYPE_REF_NODES
            || usize::from(descriptor.register_count) > MAX_TYPE_REF_NODES
        {
            return Err(invalid("body exceeds the proof work budget"));
        }
        let mut registers = List::new();
        registers.resize(usize::from(descriptor.register_count), None);
        if registers.is_empty() {
            return Err(invalid("receiver register is absent"));
        }
        let mut facts = List::new();
        facts.push(Fact {
            ty: receiver,
            origin: Origin::Receiver,
        });
        registers[0] = Some(0usize);
        let mut used_fields = Set::new();
        let mut visited = Set::new();
        let mut at = 0usize;
        let mut retained = false;
        let mut counter = None;
        let get = |reg: Reg, registers: &List<Option<usize>>| -> Result<usize, Text> {
            registers
                .get(usize::from(reg.0))
                .copied()
                .flatten()
                .ok_or_else(|| invalid("operand lacks declared receiver provenance"))
        };
        loop {
            if at >= body.len() || !visited.insert(at) {
                return Err(invalid("body falls through or contains a reachable cycle"));
            }
            let mut publication = None;
            match &body[at] {
                Instruction::Nop | Instruction::Jmp { .. } => (),
                Instruction::Mov { dst, src } => publication = Some((*dst, get(*src, &registers)?)),
                Instruction::GetF {
                    dst,
                    obj,
                    field_idx,
                } if !retained => {
                    let parent = get(*obj, &registers)?;
                    if !matches!(
                        facts[parent].origin,
                        Origin::Receiver | Origin::Field { .. }
                    ) {
                        return Err(invalid("field source is not receiver-derived"));
                    }
                    let index = u16::try_from(*field_idx)
                        .map_err(|_| invalid("field index exceeds its declaration"))?;
                    let field = field(module, &facts[parent].ty, index, false)?;
                    facts.push(Fact {
                        ty: field.field.clone(),
                        origin: Origin::Field { parent, field },
                    });
                    publication = Some((*dst, facts.len() - 1));
                }
                Instruction::CbgrExtended { sub_op, operands }
                    if !retained && *sub_op == CbgrSubOpcode::RefField as u8 =>
                {
                    let mut offset = 0;
                    let dst = crate::encoding::decode_reg(operands, &mut offset)
                        .map_err(|_| invalid("malformed field projection"))?;
                    let base = crate::encoding::decode_reg(operands, &mut offset)
                        .map_err(|_| invalid("malformed field projection"))?;
                    let index = crate::encoding::decode_varint(operands, &mut offset)
                        .map_err(|_| invalid("malformed field projection"))?;
                    let index = u16::try_from(index)
                        .map_err(|_| invalid("field index exceeds its declaration"))?;
                    if offset != operands.len() {
                        return Err(invalid("trailing field projection operands"));
                    }
                    let parent = get(base, &registers)?;
                    if !matches!(
                        facts[parent].origin,
                        Origin::Receiver | Origin::Field { .. }
                    ) {
                        return Err(invalid("counter is not receiver-derived"));
                    }
                    let field = field(module, &facts[parent].ty, index, true)?;
                    facts.push(Fact {
                        ty: TypeRef::Reference {
                            inner: Box::new(field.field.clone()),
                            tier: CbgrTier::Tier0,
                            mutability: Mutability::Mutable,
                        },
                        origin: Origin::Counter { parent, field },
                    });
                    publication = Some((dst, facts.len() - 1));
                }
                Instruction::RetainCopy {
                    dst,
                    src,
                    counter: count,
                    type_ref,
                } if !retained => {
                    let source = get(*src, &registers)?;
                    if !matches!(facts[source].origin, Origin::Receiver)
                        || substitution
                            .apply(type_ref)
                            .map_err(|e| invalid(&e.to_string()))?
                            != *owner
                    {
                        return Err(invalid(
                            "primitive source or owner differs from the selected receiver",
                        ));
                    }
                    let mut node = get(*count, &registers)?;
                    if !matches!(facts[node].origin, Origin::Counter { .. }) {
                        return Err(invalid(
                            "primitive counter is not the declared receiver-derived slot",
                        ));
                    }
                    let mut path = List::new();
                    loop {
                        match &facts[node].origin {
                            Origin::Counter { parent, field } | Origin::Field { parent, field } => {
                                used_fields.insert(node);
                                path.push(field.clone());
                                node = *parent;
                            }
                            Origin::Receiver => break,
                            _ => {
                                return Err(invalid(
                                    "counter provenance leaves the selected receiver",
                                ));
                            }
                        }
                    }
                    path.reverse();
                    counter = Some(path);
                    retained = true;
                    facts.push(Fact {
                        ty: owner.clone(),
                        origin: Origin::Retained,
                    });
                    publication = Some((*dst, facts.len() - 1));
                }
                Instruction::Ret { value } => {
                    if !retained
                        || !matches!(facts[get(*value, &registers)?].origin, Origin::Retained)
                    {
                        return Err(invalid(
                            "reachable return does not publish exactly the retained result",
                        ));
                    }
                    break;
                }
                _ => {
                    return Err(invalid(
                        "reachable operation is outside the primitive-forward contract",
                    ));
                }
            }
            if let Some((dst, fact)) = publication {
                *registers
                    .get_mut(usize::from(dst.0))
                    .ok_or_else(|| invalid("destination register is out of bounds"))? = Some(fact);
            }
            let successors = &cfg
                .blocks
                .get(&BlockId(at as u64))
                .ok_or_else(|| invalid("missing control-flow block"))?
                .successors;
            if successors.len() != 1 {
                return Err(invalid("reachable branch is not proved primitive-forward"));
            }
            at = usize::try_from(successors.iter().next().unwrap().0)
                .map_err(|_| invalid("control-flow index overflow"))?;
        }
        if facts.iter().enumerate().any(|(i, fact)| {
            matches!(fact.origin, Origin::Field { .. } | Origin::Counter { .. })
                && !used_fields.contains(&i)
        }) {
            return Err(invalid(
                "additional field read is outside the counter provenance",
            ));
        }
        let start = descriptor.bytecode_offset as usize;
        let end = start
            .checked_add(descriptor.bytecode_length as usize)
            .ok_or_else(|| invalid("body extent overflow"))?;
        let bytes = module
            .bytecode
            .get(start..end)
            .ok_or_else(|| invalid("body extent is absent"))?;
        let identity = bincode::serialize(&(
            descriptor.id,
            &descriptor.declaration_identity,
            descriptor.parent_type,
            descriptor.has_source_body,
            &descriptor.contexts,
            descriptor.properties,
            &descriptor.intrinsic_name,
        ))
        .map_err(|_| invalid("cannot seal selected identity"))?;
        Ok(Self {
            function,
            arguments: arguments.iter().cloned().collect(),
            owner: owner.clone(),
            counter: counter.ok_or_else(|| invalid("missing retained counter"))?,
            body_hash: *blake3::hash(bytes).as_bytes(),
            signature_hash: ValueUsePlan::signature_hash(descriptor, &[])
                .ok_or_else(|| invalid("cannot seal selected signature"))?,
            identity_hash: *blake3::hash(&identity).as_bytes(),
        })
    }

    /// Revalidate against mutations/remapping before consuming a local receipt.
    pub fn validate(&self, module: &VbcModule) -> Result<(), Text> {
        let current = Self::verify(module, self.function, &self.arguments, &self.owner)?;
        if current != *self {
            return Err(invalid("selected body, signature or declaration changed"));
        }
        Ok(())
    }
    /// Exact source-declared counter path. The final field is borrowed, the
    /// preceding fields are read using their declared reference carriers.
    pub fn counter_path(&self) -> &[RetainField] {
        &self.counter
    }
    /// Exact shallow retained wrapper type.
    pub fn owner(&self) -> &TypeRef {
        &self.owner
    }
}
