//! Actual emitted-call evidence for later reference representation analysis.
//!
//! These records confer no owning lifetime or declared reference capability.
//! A source body is usable only while its emitted implementation still exists
//! unchanged; runtime replacement and missing evidence remain opaque.
use verum_common::{List, Map, Text};
use verum_llvm::{
    module::Module,
    values::{
        AnyValue, AsValueRef, BasicMetadataValueEnum, BasicValueEnum, CallSiteValue, FunctionValue,
    },
};

/// Physical adaptation performed by call lowering, before the native call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArgumentView {
    Register(u16),
    /// Runtime sidecar selects the original cell when present, otherwise the
    /// register word. This is not unconditional evidence of an address.
    FieldCellOrRegister(u16),
    TemporaryCell(u16),
    Adjusted(u16),
}

impl ArgumentView {
    /// A conversion is not evidence that the original register's representation
    /// survived. Keep the emitted operand, but require an explicit future
    /// adapter rule before substituting its original register fact.
    pub(crate) fn after_coercion(
        self,
        before: BasicValueEnum<'_>,
        after: BasicMetadataValueEnum<'_>,
    ) -> Self {
        if before.as_value_ref() == after.as_value_ref() {
            return self;
        }
        self.opaque()
    }

    fn opaque(self) -> Self {
        let reg = match self {
            Self::Register(reg)
            | Self::FieldCellOrRegister(reg)
            | Self::TemporaryCell(reg)
            | Self::Adjusted(reg) => reg,
        };
        Self::Adjusted(reg)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct NativeArgument<'ctx> {
    pub view: ArgumentView,
    pub value: BasicMetadataValueEnum<'ctx>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResultView {
    Word,
    /// The existing caller normalization emitted a load through the raw result.
    LoadedSlot,
    /// A bridge changed the shape; no source-return substitution is allowed.
    Opaque,
}

#[derive(Debug, Clone)]
pub(crate) struct NativeCallReceipt<'ctx> {
    pub instruction: usize,
    pub destination: u16,
    pub call: CallSiteValue<'ctx>,
    pub arguments: List<NativeArgument<'ctx>>,
    pub result: Option<BasicValueEnum<'ctx>>,
    pub result_view: ResultView,
}

/// Actual non-call producer/adaptation or a deferred object-consuming view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReferenceSiteKind {
    ValueInput(u16),
    SlotOutput(u16),
    PassthroughOutput {
        register: u16,
        source: u16,
    },
    FieldOutput {
        register: u16,
        base: u16,
        slot: bool,
    },
    PayloadOutput {
        register: u16,
        raw_word: bool,
    },
}
#[derive(Debug, Clone)]
pub(crate) struct ReferenceSite<'ctx> {
    pub instruction: usize,
    pub kind: ReferenceSiteKind,
    pub anchor: Option<verum_llvm::values::InstructionValue<'ctx>>,
}
#[derive(Debug)]
struct StoredReferenceSite {
    instruction: usize,
    kind: ReferenceSiteKind,
    native_instruction: Option<usize>,
}

/// Native instruction positions survive the emission phase without retaining
/// handles that a runtime body replacement may delete. Resolve rehydrates only
/// from the live, sealed body (even an identical re-emission has fresh handles).
#[derive(Debug)]
struct StoredCall {
    instruction: usize,
    destination: u16,
    native_instruction: usize,
    arguments: List<ArgumentView>,
    result_instruction: Option<usize>,
    result_view: ResultView,
}

#[derive(Debug)]
struct SourceBody<'ctx> {
    id: u32,
    function: FunctionValue<'ctx>,
    seal: blake3::Hash,
    calls: List<StoredCall>,
    reference_sites: List<StoredReferenceSite>,
}

/// Bounded, compilation-local facts. No register states or printed IR survive
/// capture. The final body check runs before dereferencing any saved call handle.
#[derive(Debug, Default)]
pub(crate) struct NativeCallAuthority<'ctx> {
    bodies: Map<Text, SourceBody<'ctx>>,
    receipt_count: usize,
    argument_count: usize,
    reference_count: usize,
}

pub(crate) const MAX_RECEIPTS: usize = 131_072;
pub(crate) const MAX_ARGUMENTS: usize = 1024;
pub(crate) const MAX_ARGUMENT_WORDS: usize = 524_288;
const MAX_BODY_INSTRUCTIONS: usize = 262_144;

/// Linkage-only demotion is not a change to source-body semantics. Function
/// type/calling convention and the entire body remain part of the seal.
fn body_seal(function: FunctionValue<'_>) -> Option<blake3::Hash> {
    if function.count_basic_blocks() == 0
        || function
            .get_basic_blocks()
            .iter()
            .flat_map(|block| block.get_instructions())
            .take(MAX_BODY_INSTRUCTIONS + 1)
            .count()
            > MAX_BODY_INSTRUCTIONS
    {
        return None;
    }
    let printed = function.print_to_string();
    let text = printed.to_str().ok()?;
    let (_, body) = text.split_once("{\n")?;
    let mut digest = blake3::Hasher::new();
    digest.update(function.get_type().print_to_string().to_bytes());
    digest.update(&function.get_call_conventions().to_le_bytes());
    for location in std::iter::once(verum_llvm::attributes::AttributeLoc::Function)
        .chain(std::iter::once(
            verum_llvm::attributes::AttributeLoc::Return,
        ))
        .chain((0..function.count_params()).map(verum_llvm::attributes::AttributeLoc::Param))
    {
        let attributes = function.attributes(location);
        digest.update(&(attributes.len() as u64).to_le_bytes());
        for attribute in attributes {
            if attribute.is_string() {
                for bytes in [
                    attribute.get_string_kind_id().to_bytes(),
                    attribute.get_string_value().to_bytes(),
                ] {
                    digest.update(&(bytes.len() as u64).to_le_bytes());
                    digest.update(bytes);
                }
            } else if attribute.is_enum() {
                digest.update(&attribute.get_enum_kind_id().to_le_bytes());
                digest.update(&attribute.get_enum_value().to_le_bytes());
            } else if attribute.is_type() {
                digest.update(&attribute.get_enum_kind_id().to_le_bytes());
                digest.update(attribute.get_type_value().print_to_string().to_bytes());
            } else {
                return None;
            }
        }
    }
    digest.update(body.as_bytes());
    Some(digest.finalize())
}

impl<'ctx> NativeCallAuthority<'ctx> {
    pub(crate) fn clear(&mut self) {
        self.bodies.clear();
        self.receipt_count = 0;
        self.argument_count = 0;
        self.reference_count = 0;
    }

    pub(crate) fn capture(
        &mut self,
        id: u32,
        function: FunctionValue<'ctx>,
        calls: List<NativeCallReceipt<'ctx>>,
        reference_sites: List<ReferenceSite<'ctx>>,
    ) {
        let Some(seal) = body_seal(function) else {
            return;
        };
        let name: Text = function.get_name().to_string_lossy().as_ref().into();
        let instructions: List<_> = function
            .get_basic_blocks()
            .iter()
            .flat_map(|block| block.get_instructions())
            .collect();
        let positions: Map<usize, usize> = instructions
            .iter()
            .enumerate()
            .map(|(index, instruction)| (instruction.as_value_ref() as usize, index))
            .collect();
        let remaining = MAX_RECEIPTS.saturating_sub(self.receipt_count);
        let mut argument_budget = MAX_ARGUMENT_WORDS.saturating_sub(self.argument_count);
        let calls: List<_> = calls
            .into_iter()
            .filter_map(|receipt| {
                if receipt.arguments.len() > MAX_ARGUMENTS
                    || receipt.arguments.len() > argument_budget
                {
                    return None;
                }
                let native_instruction = *positions.get(&(receipt.call.as_value_ref() as usize))?;
                let instruction = instructions[native_instruction];
                let actual = CallSiteValue::try_from(instruction).ok()?;
                if actual.count_arguments() as usize != receipt.arguments.len() {
                    return None;
                }
                // Saved values are compared as opaque addresses only. Read current
                // operands, never a possibly deleted orphan instruction's contents.
                for (index, argument) in receipt.arguments.iter().enumerate() {
                    if instruction
                        .get_operand(index as u32)?
                        .value()?
                        .as_value_ref()
                        != argument.value.as_value_ref()
                    {
                        return None;
                    }
                }
                let result_instruction = receipt
                    .result
                    .and_then(|value| positions.get(&(value.as_value_ref() as usize)).copied());
                let result_view = if receipt.result.is_some() && result_instruction.is_none() {
                    ResultView::Opaque
                } else {
                    receipt.result_view
                };
                argument_budget -= receipt.arguments.len();
                Some(StoredCall {
                    instruction: receipt.instruction,
                    destination: receipt.destination,
                    native_instruction,
                    arguments: receipt.arguments.into_iter().map(|arg| arg.view).collect(),
                    result_instruction,
                    result_view,
                })
            })
            .take(remaining)
            .collect();
        let reference_sites: List<_> = reference_sites
            .into_iter()
            .filter_map(|site| {
                let native_instruction = match site.anchor {
                    Some(anchor) => Some(*positions.get(&(anchor.as_value_ref() as usize))?),
                    None => None,
                };
                Some(StoredReferenceSite {
                    instruction: site.instruction,
                    kind: site.kind,
                    native_instruction,
                })
            })
            .take(MAX_RECEIPTS.saturating_sub(self.reference_count))
            .collect();
        self.reference_count += reference_sites.len();
        self.receipt_count += calls.len();
        self.argument_count += calls.iter().map(|call| call.arguments.len()).sum::<usize>();
        self.bodies.insert(
            name,
            SourceBody {
                id,
                function,
                seal,
                calls,
                reference_sites,
            },
        );
    }

    /// Runtime emission may overwrite VBC bodies after the source pass. Drop
    /// stale candidate evidence before handing the module to later phases.
    pub(crate) fn discard_stale(
        &mut self,
        module: &Module<'ctx>,
    ) -> Map<u32, List<(u32, NativeCallReceipt<'ctx>)>> {
        let valid = self.resolve(module);
        self.bodies.retain(|_, body| {
            let Some(calls) = valid.get(&body.id) else {
                return false;
            };
            let sites: verum_common::Set<_> =
                calls.iter().map(|(_, call)| call.instruction).collect();
            body.calls.retain(|call| sites.contains(&call.instruction));
            true
        });
        self.receipt_count = self.bodies.values().map(|body| body.calls.len()).sum();
        self.argument_count = self
            .bodies
            .values()
            .flat_map(|body| body.calls.iter())
            .map(|call| call.arguments.len())
            .sum();
        self.reference_count = self
            .bodies
            .values()
            .map(|body| body.reference_sites.len())
            .sum();
        valid
    }

    pub(crate) fn resolve_reference_sites(
        &self,
        live: &Map<u32, List<(u32, NativeCallReceipt<'ctx>)>>,
    ) -> Map<u32, List<ReferenceSite<'ctx>>> {
        // `live` is the immediately preceding discard_stale snapshot. No edits
        // may occur between these operations; avoid hashing every body twice.
        self.bodies
            .values()
            .filter_map(|body| {
                if !live.contains_key(&body.id) {
                    return None;
                }
                let current = body.function;
                let instructions: List<_> = current
                    .get_basic_blocks()
                    .iter()
                    .flat_map(|block| block.get_instructions())
                    .collect();
                let sites = body
                    .reference_sites
                    .iter()
                    .filter_map(|site| {
                        Some(ReferenceSite {
                            instruction: site.instruction,
                            kind: site.kind,
                            anchor: match site.native_instruction {
                                Some(index) => Some(*instructions.get(index)?),
                                None => None,
                            },
                        })
                    })
                    .collect();
                Some((body.id, sites))
            })
            .collect()
    }

    /// Resolve once at the end of native emission, before any consumer edits
    /// bodies. Each body is sealed at most once in this pass, rather than once
    /// per incoming call. The returned evidence is valid for this frozen phase.
    pub(crate) fn resolve(
        &self,
        module: &Module<'ctx>,
    ) -> Map<u32, List<(u32, NativeCallReceipt<'ctx>)>> {
        let live: Map<Text, &SourceBody<'ctx>> = self
            .bodies
            .iter()
            .filter_map(|(name, body)| {
                let current = module.get_function(name)?;
                (current == body.function && body_seal(current) == Some(body.seal))
                    .then(|| (name.clone(), body))
            })
            .collect();
        live.values()
            .map(|body| {
                let instructions: List<_> = body
                    .function
                    .get_basic_blocks()
                    .iter()
                    .flat_map(|block| block.get_instructions())
                    .collect();
                let calls = body
                    .calls
                    .iter()
                    .filter_map(|stored| {
                        let instruction = *instructions.get(stored.native_instruction)?;
                        let call = CallSiteValue::try_from(instruction).ok()?;
                        let selected = call.get_called_fn_value()?;
                        let name = selected.get_name().to_str().ok()?;
                        let callee = live.get(&Text::from(name))?;
                        if selected != callee.function
                            || call.get_call_convention() != selected.get_call_conventions()
                            || stored.arguments.len() != selected.count_params() as usize
                            || stored.arguments.len() != call.count_arguments() as usize
                        {
                            return None;
                        }
                        let arguments: Option<List<_>> = stored
                            .arguments
                            .iter()
                            .enumerate()
                            .map(|(index, view)| {
                                Some(NativeArgument {
                                    view: if selected.count_attributes(
                                        verum_llvm::attributes::AttributeLoc::Param(index as u32),
                                    ) != 0
                                        || !call
                                            .attributes(
                                                verum_llvm::attributes::AttributeLoc::Param(
                                                    index as u32,
                                                ),
                                            )
                                            .is_empty()
                                    {
                                        view.opaque()
                                    } else {
                                        *view
                                    },
                                    value: instruction.get_operand(index as u32)?.value()?.into(),
                                })
                            })
                            .collect();
                        let result = if let Some(position) = stored.result_instruction {
                            let instruction = *instructions.get(position)?;
                            let ty = instruction.get_type();
                            if !ty.is_int_type() && !ty.is_pointer_type() && !ty.is_float_type() {
                                return None;
                            }
                            // SAFETY: the instruction is freshly retrieved from a live,
                            // sealed body, and its basic scalar type was checked above.
                            Some(unsafe { BasicValueEnum::new(instruction.as_value_ref()) })
                        } else {
                            None
                        };
                        Some((
                            callee.id,
                            NativeCallReceipt {
                                instruction: stored.instruction,
                                destination: stored.destination,
                                call,
                                arguments: arguments?,
                                result,
                                result_view: if selected
                                    .count_attributes(verum_llvm::attributes::AttributeLoc::Return)
                                    != 0
                                    || !call
                                        .attributes(verum_llvm::attributes::AttributeLoc::Return)
                                        .is_empty()
                                {
                                    ResultView::Opaque
                                } else {
                                    stored.result_view
                                },
                            },
                        ))
                    })
                    .collect();
                (body.id, calls)
            })
            .collect()
    }
}
