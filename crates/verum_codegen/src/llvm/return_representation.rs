//! Finite native representation analysis, using actual selected-call receipts.
//!
//! A projection is symbolic until an exact caller supplies the aggregate fact.
//! Unknown effects and joins never prove a slot address. This is deliberately
//! finite and does not infer reference representation from type/method names.
use super::native_call::{
    ArgumentView, NativeCallReceipt, ReferenceSite, ReferenceSiteKind, ResultView,
};
use verum_common::{Heap, List, Map, Set};
use verum_vbc::{
    Instruction as I, Reg,
    module::{FunctionDescriptor, FunctionId, VbcModule},
    types::{TypeId, TypeRef},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Representation {
    Unknown,
    Value,
    Slot,
    Parameter(u16, Option<TypeId>),
    Projection(Heap<Representation>, TypeId, u32, u32),
    Sum(TypeId, List<(u32, List<Representation>)>),
    Allocation(usize),
    Test(Heap<Representation>, u32),
}
use Representation as R;
const MAX_FACT_NODES: usize = 512;
const MAX_FACT_DEPTH: usize = 16;

struct Nodes(usize);
impl Nodes {
    fn take(&mut self, depth: usize) -> Option<()> {
        if depth > MAX_FACT_DEPTH || self.0 == 0 {
            return None;
        }
        self.0 -= 1;
        Some(())
    }
}
impl R {
    fn copy_bounded(&self, budget: &mut Nodes, depth: usize) -> Option<Self> {
        budget.take(depth)?;
        Some(match self {
            Self::Projection(value, owner, tag, field) => Self::Projection(
                Heap::new(value.copy_bounded(budget, depth + 1)?),
                *owner,
                *tag,
                *field,
            ),
            Self::Test(value, tag) => {
                Self::Test(Heap::new(value.copy_bounded(budget, depth + 1)?), *tag)
            }
            Self::Sum(owner, variants) => {
                if variants.len() > 64 {
                    return None;
                }
                let mut result = List::new();
                for (tag, fields) in variants {
                    if fields.len() > 64 {
                        return None;
                    }
                    let mut copied = List::new();
                    for field in fields {
                        copied.push(field.copy_bounded(budget, depth + 1)?);
                    }
                    result.push((*tag, copied));
                }
                Self::Sum(*owner, result)
            }
            leaf => leaf.clone(),
        })
    }
    fn count_bounded(&self, budget: &mut Nodes, depth: usize) -> Option<()> {
        budget.take(depth)?;
        match self {
            R::Projection(inner, ..) | R::Test(inner, _) => inner.count_bounded(budget, depth + 1),
            R::Sum(_, variants) => {
                if variants.len() > 64 {
                    return None;
                }
                for (_, fields) in variants {
                    if fields.len() > 64 {
                        return None;
                    }
                    for field in fields {
                        field.count_bounded(budget, depth + 1)?;
                    }
                }
                Some(())
            }
            _ => Some(()),
        }
    }
    fn bounded(&self) -> bool {
        self.count_bounded(&mut Nodes(MAX_FACT_NODES), 0).is_some()
    }
    fn owner(&self) -> Option<TypeId> {
        match self {
            Self::Parameter(_, owner) => *owner,
            Self::Sum(owner, _) => Some(*owner),
            _ => None,
        }
    }
    fn project(&self, owner: TypeId, tag: u32, field: u32) -> Self {
        self.project_with_budget(owner, tag, field, &mut Nodes(MAX_FACT_NODES), 0)
            .unwrap_or(Self::Unknown)
    }
    fn project_with_budget(
        &self,
        owner: TypeId,
        tag: u32,
        field: u32,
        budget: &mut Nodes,
        depth: usize,
    ) -> Option<Self> {
        budget.take(depth)?;
        Some(match self {
            Self::Sum(actual, variants) if *actual == owner => match variants
                .iter()
                .find(|(t, _)| *t == tag)
                .and_then(|(_, fields)| fields.get(field as usize))
            {
                Some(value) => value.copy_bounded(budget, depth + 1)?,
                None => Self::Unknown,
            },
            Self::Parameter(..) | Self::Projection(..) => Self::Projection(
                Heap::new(self.copy_bounded(budget, depth + 1)?),
                owner,
                tag,
                field,
            ),
            _ => Self::Unknown,
        })
    }
    fn substitute(&self, args: &[Self], _depth: usize) -> Self {
        fn expand(value: &R, args: &[R], budget: &mut Nodes, depth: usize) -> Option<R> {
            budget.take(depth)?;
            Some(match value {
                R::Parameter(index, _) => {
                    args.get(*index as usize)?.copy_bounded(budget, depth + 1)?
                }
                R::Projection(value, owner, tag, field) => {
                    let base = expand(value, args, budget, depth + 1)?;
                    base.project_with_budget(*owner, *tag, *field, budget, depth + 1)?
                }
                R::Sum(owner, variants) => {
                    let mut result = List::new();
                    for (tag, fields) in variants {
                        let mut expanded = List::new();
                        for field in fields {
                            expanded.push(expand(field, args, budget, depth + 1)?);
                        }
                        result.push((*tag, expanded));
                    }
                    R::Sum(*owner, result)
                }
                R::Allocation(_) | R::Test(..) => R::Unknown,
                leaf => leaf.clone(),
            })
        }
        expand(self, args, &mut Nodes(MAX_FACT_NODES), 0).unwrap_or(Self::Unknown)
    }
    fn join(&self, other: &Self) -> Self {
        fn meet(a: &R, b: &R, budget: &mut Nodes, depth: usize) -> Option<R> {
            budget.take(depth)?;
            if a == b {
                return a.copy_bounded(budget, depth + 1);
            }
            if let (R::Sum(owner, xs), R::Sum(other, ys)) = (a, b) {
                if owner != other {
                    return Some(R::Unknown);
                }
                let mut tags: Set<u32> = xs.iter().chain(ys).map(|(tag, _)| *tag).collect();
                if tags.len() > 64 {
                    return None;
                }
                let mut tags: List<_> = tags.drain().collect();
                tags.sort_unstable();
                let mut result = List::new();
                for tag in tags {
                    let x = xs.iter().find(|(t, _)| *t == tag).map(|(_, fields)| fields);
                    let y = ys.iter().find(|(t, _)| *t == tag).map(|(_, fields)| fields);
                    let mut fields = List::new();
                    match (x, y) {
                        (Some(x), Some(y)) if x.len() == y.len() => {
                            for (a, b) in x.iter().zip(y) {
                                fields.push(meet(a, b, budget, depth + 1)?);
                            }
                        }
                        (Some(x), None) | (None, Some(x)) => {
                            for field in x {
                                fields.push(field.copy_bounded(budget, depth + 1)?);
                            }
                        }
                        _ => return Some(R::Unknown),
                    }
                    result.push((tag, fields));
                }
                return Some(R::Sum(*owner, result));
            }
            Some(R::Unknown)
        }
        meet(self, other, &mut Nodes(MAX_FACT_NODES), 0).unwrap_or(Self::Unknown)
    }
}

#[derive(Clone, Debug)]
pub(super) struct BodyFacts {
    pub result: Representation,
    pub value_inputs: Map<usize, Set<u16>>,
    pub parameter_uses: List<u8>,
    pub call_inputs: Map<usize, Set<usize>>,
    pub may_write: bool,
}
impl BodyFacts {
    fn unknown(_count: usize) -> Self {
        Self {
            result: R::Unknown,
            value_inputs: Map::new(),
            parameter_uses: List::new(),
            call_inputs: Map::new(),
            may_write: true,
        }
    }
}
#[derive(Clone, Default, PartialEq, Eq)]
struct State {
    registers: Map<u16, R>,
    objects: Map<usize, R>,
    guards: List<(R, u32)>,
    cells: Map<u16, bool>,
    // Word identity outlives knowledge of the memory it addresses. Preserve it
    // across effect barriers so a later return/store is still a non-value use.
    parameter_origins: Map<u16, u16>,
}
impl State {
    fn bounded(&self) -> bool {
        if self.cells.len() > 256 || self.parameter_origins.len() > 256 {
            return false;
        }
        let mut budget = Nodes(4096);
        for _ in self.cells.keys().chain(self.parameter_origins.keys()) {
            if budget.take(0).is_none() {
                return false;
            }
        }
        self.registers
            .values()
            .chain(self.objects.values())
            .chain(self.guards.iter().map(|(value, _)| value))
            .all(|value| value.count_bounded(&mut budget, 0).is_some())
    }
    fn raw(&self, reg: Reg) -> R {
        self.registers.get(&reg.0).cloned().unwrap_or(R::Unknown)
    }
    fn value(&self, reg: Reg) -> R {
        self.materialize(&self.raw(reg), 0)
    }
    fn materialize(&self, value: &R, _depth: usize) -> R {
        fn expand(state: &State, value: &R, budget: &mut Nodes, depth: usize) -> Option<R> {
            budget.take(depth)?;
            match value {
                R::Allocation(id) => expand(state, state.objects.get(id)?, budget, depth + 1),
                R::Sum(owner, variants) => {
                    let mut result = List::new();
                    for (tag, fields) in variants {
                        let mut expanded = List::new();
                        for field in fields {
                            expanded.push(expand(state, field, budget, depth + 1)?);
                        }
                        result.push((*tag, expanded));
                    }
                    Some(R::Sum(*owner, result))
                }
                _ => value.copy_bounded(budget, depth + 1),
            }
        }
        expand(self, value, &mut Nodes(MAX_FACT_NODES), 0).unwrap_or(R::Unknown)
    }
    fn put(&mut self, reg: Reg, value: R) {
        self.parameter_origins.remove(&reg.0);
        if let R::Parameter(index, _) = value {
            self.parameter_origins.insert(reg.0, index);
        }
        self.registers.insert(reg.0, value);
        self.cells.insert(reg.0, false);
    }
    fn tag(&self, reg: Reg) -> Option<u32> {
        let raw = self.raw(reg);
        self.guards
            .iter()
            .find(|(value, _)| *value == raw)
            .map(|(_, tag)| *tag)
            .or_else(|| match self.value(reg) {
                R::Sum(_, variants) if variants.len() == 1 => Some(variants[0].0),
                _ => None,
            })
    }
    fn projection(&self, reg: Reg, field: u32) -> R {
        let value = self.value(reg);
        match (value.owner(), self.tag(reg)) {
            (Some(owner), Some(tag)) => value.project(owner, tag, field),
            _ => R::Unknown,
        }
    }
    // Invalidate future reads of possibly changed storage, not a word already
    // extracted from that storage. This distinction matters for call effects.
    fn invalidate_storage(&mut self) {
        for value in self.registers.values_mut() {
            if matches!(value, R::Parameter(..) | R::Allocation(_) | R::Sum(..)) {
                *value = R::Unknown;
            }
        }
        self.objects.clear();
        self.guards.clear();
    }
    fn join(&self, other: &Self) -> Self {
        let mut result = Self::default();
        for (&reg, value) in &self.registers {
            let joined = self.materialize(value, 0).join(&other.value(Reg(reg)));
            if joined != R::Unknown {
                result.registers.insert(reg, joined);
            }
        }
        // Guards are path-specific. Equal facts survive a join; a predicate
        // from just one incoming edge never becomes a field-layout proof.
        for guard in &self.guards {
            if other.guards.contains(guard) {
                result.guards.push(guard.clone());
            }
        }
        for (&reg, &origin) in &self.parameter_origins {
            if other.parameter_origins.get(&reg) == Some(&origin) {
                result.parameter_origins.insert(reg, origin);
            }
        }
        for (&reg, &cell) in &self.cells {
            if other.cells.get(&reg) == Some(&cell) {
                result.cells.insert(reg, cell);
            }
        }
        result
    }
}
// An omitted receipt is not evidence that payload extraction left the raw
// word intact: site-budget exhaustion may have omitted an eager adapter too.
fn payload_projection(
    state: &State,
    sites: &Map<usize, List<ReferenceSiteKind>>,
    pc: usize,
    dst: Reg,
    variant: Reg,
    field: u32,
) -> R {
    if sites.get(&pc).is_some_and(|sites| {
        sites.contains(&ReferenceSiteKind::PayloadOutput {
            register: dst.0,
            raw_word: true,
        })
    }) {
        state.projection(variant, field)
    } else {
        R::Unknown
    }
}

fn replace_payload(object: &R, field: usize, payload: &R) -> R {
    let R::Sum(owner, variants) = object else {
        return R::Unknown;
    };
    let mut budget = Nodes(MAX_FACT_NODES - 1);
    let mut result = List::new();
    for (tag, fields) in variants {
        let mut copied = List::new();
        for (index, old) in fields.iter().enumerate() {
            let value = if index == field { payload } else { old };
            let Some(value) = value.copy_bounded(&mut budget, 1) else {
                return R::Unknown;
            };
            copied.push(value);
        }
        result.push((*tag, copied));
    }
    R::Sum(*owner, result)
}

fn nominal(ty: &TypeRef) -> Option<TypeId> {
    match ty {
        TypeRef::Concrete(id) | TypeRef::Instantiated { base: id, .. } => Some(*id),
        TypeRef::Reference { inner, .. } => nominal(inner),
        _ => None,
    }
}

pub(super) struct Analysis<'a, 'ctx> {
    module: &'a VbcModule,
    calls: &'a Map<u32, List<(u32, NativeCallReceipt<'ctx>)>>,
    sites: &'a Map<u32, List<ReferenceSite<'ctx>>>,
    cache: Map<FunctionId, BodyFacts>,
    active: Set<FunctionId>,
    work: usize,
}
impl<'a, 'ctx> Analysis<'a, 'ctx> {
    pub fn new(
        module: &'a VbcModule,
        calls: &'a Map<u32, List<(u32, NativeCallReceipt<'ctx>)>>,
        sites: &'a Map<u32, List<ReferenceSite<'ctx>>>,
    ) -> Self {
        Self {
            module,
            calls,
            sites,
            cache: Map::new(),
            active: Set::new(),
            work: 0,
        }
    }
    pub fn body(&mut self, id: FunctionId) -> BodyFacts {
        // Presence proves a surviving sealed native source body, including a
        // body with no calls. A readable VBC body alone confers no authority.
        if !self.calls.contains_key(&id.0) {
            return BodyFacts::unknown(0);
        }
        if let Some(cached) = self.cache.get(&id) {
            return cached.clone();
        }
        let Some(fd) = self.module.get_function(id) else {
            return BodyFacts::unknown(0);
        };
        let Some(instructions) = fd.instructions.as_deref() else {
            return BodyFacts::unknown(0);
        };
        if fd.params.len() > 1024
            || instructions.is_empty()
            || instructions.len() > 16_384
            || self.active.len() >= 32
            || self.work >= 2_000_000
            || !self.active.insert(id)
        {
            return BodyFacts::unknown(instructions.len());
        }
        let result = self.analyze(fd, instructions);
        self.active.remove(&id);
        self.cache.insert(id, result.clone());
        result
    }
    fn analyze(&mut self, fd: &FunctionDescriptor, instructions: &[I]) -> BodyFacts {
        let mut initial = State::default();
        for (i, param) in fd.params.iter().enumerate() {
            initial.put(
                Reg(i as u16),
                R::Parameter(i as u16, nominal(&param.type_ref)),
            );
        }
        let mut states: Map<usize, State> = Map::new();
        states.insert(0, initial);
        let mut returns: Option<R> = None;
        let mut value_inputs: Map<usize, Set<u16>> = Map::new();
        let mut parameter_uses = List::from_elem(0_u8, fd.params.len());
        let mut call_inputs: Map<usize, Set<usize>> = Map::new();
        let mut may_write = false;
        let calls: Map<_, _> = self
            .calls
            .get(&fd.id.0)
            .into_iter()
            .flatten()
            .map(|(id, call)| (call.instruction, (*id, call.clone())))
            .collect();
        let sites: Map<usize, List<ReferenceSiteKind>> = self
            .sites
            .get(&fd.id.0)
            .into_iter()
            .flatten()
            .fold(Map::new(), |mut map, site| {
                map.entry(site.instruction).or_default().push(site.kind);
                map
            });
        // Only forward CFG edges are admitted, so instruction order is a
        // topological order. Consume each incoming state once; never cache a
        // register map per instruction after it has been processed.
        for pc in 0..instructions.len() {
            let Some(mut state) = states.remove(&pc) else {
                continue;
            };
            self.work += 1;
            if !state.bounded()
                || self.work > 2_000_000
                || states.len() > 64
                || state.registers.len() > 256
                || state.objects.len() > 64
            {
                return BodyFacts::unknown(instructions.len());
            }
            let instruction = &instructions[pc];
            value_inputs.remove(&pc);
            let mut edges: List<(usize, State)> = List::new();
            let mut next = true;
            let mut write: Option<(Reg, R)> = None;
            let mut written_cell = Some(false);
            let mut written_origin = None;
            let mut use_input = |reg: Reg, demand: u8, state: &State| {
                if let Some(&origin) = state.parameter_origins.get(&reg.0) {
                    if let Some(uses) = parameter_uses.get_mut(origin as usize) {
                        *uses |= demand;
                    }
                } else if state.raw(reg) == R::Slot && demand == 1 {
                    value_inputs.entry(pc).or_default().insert(reg.0);
                }
            };
            match instruction {
                I::Mov { dst, src } => {
                    write = Some((*dst, state.raw(*src)));
                    written_cell = state.cells.get(&src.0).copied();
                    written_origin = state.parameter_origins.get(&src.0).copied();
                }
                I::LoadI { dst, .. }
                | I::LoadK { dst, .. }
                | I::LoadF { dst, .. }
                | I::LoadUnit { dst }
                | I::LoadTrue { dst }
                | I::LoadFalse { dst }
                | I::LoadSmallI { dst, .. }
                | I::New { dst, .. }
                | I::NewG { dst, .. } => write = Some((*dst, R::Value)),
                I::GetVariantDataRef { dst, variant, .. } => {
                    use_input(*variant, 1, &state);
                    write = Some((
                        *dst,
                        if sites.get(&pc).is_some_and(|sites| {
                            sites.contains(&ReferenceSiteKind::SlotOutput(dst.0))
                        }) {
                            R::Slot
                        } else {
                            R::Unknown
                        },
                    ));
                }
                I::GetVariantData {
                    dst,
                    variant,
                    field,
                } => {
                    use_input(*variant, 1, &state);
                    write = Some((
                        *dst,
                        payload_projection(&state, &sites, pc, *dst, *variant, *field),
                    ));
                }
                I::AsVar { dst, value, .. } => {
                    use_input(*value, 1, &state);
                    write = Some((
                        *dst,
                        payload_projection(&state, &sites, pc, *dst, *value, 0),
                    ));
                }
                I::MakeVariantTyped {
                    dst,
                    type_id,
                    tag,
                    field_count,
                } if *field_count <= 64 => {
                    let mut variants = List::new();
                    variants.push((*tag, List::from_elem(R::Unknown, *field_count as usize)));
                    state.objects.insert(pc, R::Sum(TypeId(*type_id), variants));
                    write = Some((*dst, R::Allocation(pc)));
                }
                I::SetVariantData {
                    variant,
                    field,
                    value,
                } => {
                    use_input(*value, 4, &state);
                    use_input(*variant, 1, &state);
                    let payload = state.value(*value);
                    if let R::Allocation(site) = state.raw(*variant) {
                        let updated = state
                            .objects
                            .get(&site)
                            .map(|object| replace_payload(object, *field as usize, &payload))
                            .unwrap_or(R::Unknown);
                        state.objects.insert(site, updated);
                    } else {
                        may_write = true;
                        state.invalidate_storage();
                    }
                }
                I::IsVar { dst, value, tag } => {
                    use_input(*value, 1, &state);
                    write = Some((*dst, R::Test(Heap::new(state.raw(*value)), *tag)));
                }
                I::Jmp { offset } => {
                    let target = pc as i64 + i64::from(*offset);
                    if target <= pc as i64 || target as usize >= instructions.len() {
                        return BodyFacts::unknown(instructions.len());
                    }
                    edges.push((target as usize, state.clone()));
                    next = false;
                }
                I::JmpIf { cond, offset } | I::JmpNot { cond, offset } => {
                    let target = pc as i64 + i64::from(*offset);
                    if target <= pc as i64 || target as usize >= instructions.len() {
                        return BodyFacts::unknown(instructions.len());
                    }
                    use_input(*cond, 8, &state);
                    let mut yes = state.clone();
                    if let R::Test(value, tag) = state.raw(*cond) {
                        // Equal representation does not imply the same object.
                        // Only symbolic parameters and fresh allocation sites
                        // carry identity through a tag guard; joined sums do not.
                        if matches!(*value, R::Parameter(..) | R::Allocation(_)) {
                            yes.guards.push((*value, tag));
                        }
                    }
                    if matches!(instruction, I::JmpIf { .. }) {
                        edges.push((target as usize, yes));
                    } else {
                        edges.push((target as usize, state.clone()));
                        state = yes;
                    }
                }
                I::Ret { value } => {
                    use_input(*value, 4, &state);
                    let result = state.value(*value);
                    returns = Some(
                        returns
                            .as_ref()
                            .map(|old| old.join(&result))
                            .unwrap_or(result),
                    );
                    next = false;
                }
                I::RetV => {
                    returns = Some(
                        returns
                            .as_ref()
                            .map(|old| old.join(&R::Value))
                            .unwrap_or(R::Value),
                    );
                    next = false;
                }
                I::Panic { .. } | I::Unreachable => next = false,
                I::TryBegin { .. }
                | I::Throw { .. }
                | I::TailCall { .. }
                | I::JmpCmp { .. }
                | I::Switch { .. }
                | I::CtxProvide { .. } => return BodyFacts::unknown(instructions.len()),
                I::Call { dst, .. } | I::CallG { dst, .. } | I::CallM { dst, .. } => {
                    let result = if let Some((id, receipt)) = calls.get(&pc) {
                        let callee = self.body(FunctionId(*id));
                        let inputs: List<_> = receipt
                            .arguments
                            .iter()
                            .enumerate()
                            .map(|(index, argument)| {
                                let (reg, mut value) = match argument.view {
                                    ArgumentView::Register(reg) => (reg, state.value(Reg(reg))),
                                    ArgumentView::TemporaryCell(reg) => (reg, R::Slot),
                                    ArgumentView::FieldCellOrRegister(reg) => (
                                        reg,
                                        match state.cells.get(&reg) {
                                            Some(true) => R::Slot,
                                            Some(false) => state.value(Reg(reg)),
                                            None => R::Unknown,
                                        },
                                    ),
                                    ArgumentView::Adjusted(reg) => (reg, R::Unknown),
                                };
                                let demand = callee.parameter_uses.get(index).copied().unwrap_or(8);
                                // An adapted cell is not the caller register's carrier.
                                let identity = matches!(argument.view, ArgumentView::Register(_))
                                    || (matches!(
                                        argument.view,
                                        ArgumentView::FieldCellOrRegister(_)
                                    ) && state.cells.get(&reg) == Some(&false));
                                use_input(Reg(reg), if identity { demand } else { 8 }, &state);
                                if demand == 1 && value == R::Slot {
                                    call_inputs.entry(pc).or_default().insert(index);
                                    value = R::Unknown; // contents are not another proved cell
                                }
                                value
                            })
                            .collect();
                        let result = callee.result.substitute(&inputs, 0);
                        if callee.may_write {
                            may_write = true;
                            state.invalidate_storage();
                        }
                        match receipt.result_view {
                            ResultView::Word => result,
                            ResultView::LoadedSlot | ResultView::Opaque => R::Unknown,
                        }
                    } else {
                        for uses in &mut parameter_uses {
                            *uses |= 8;
                        }
                        may_write = true;
                        state.invalidate_storage();
                        R::Unknown
                    };
                    write = Some((*dst, result));
                }
                I::GetF { dst, obj, .. } => {
                    use_input(*obj, 1, &state);
                    write = Some((*dst, R::Unknown));
                }
                I::Deref { dst, ref_reg } => {
                    let identity = sites.get(&pc).is_some_and(|sites| {
                        sites.contains(&ReferenceSiteKind::ValueInput(ref_reg.0))
                    });
                    use_input(*ref_reg, if identity { 1 } else { 2 }, &state);
                    write = Some((*dst, R::Unknown));
                }
                I::RefObj { dst, src } => {
                    let value = if sites.get(&pc).is_some_and(|sites| {
                        sites.contains(&ReferenceSiteKind::PassthroughOutput {
                            register: dst.0,
                            source: src.0,
                        })
                    }) {
                        written_origin = state.parameter_origins.get(&src.0).copied();
                        state.raw(*src)
                    } else {
                        use_input(*src, 8, &state);
                        R::Unknown
                    };
                    write = Some((*dst, value));
                }
                I::RefLocal { dst, src } => {
                    use_input(*src, 8, &state);
                    write = Some((*dst, R::Unknown));
                }
                I::SetF { obj, value, .. } => {
                    use_input(*obj, 1, &state);
                    use_input(*value, 4, &state);
                    may_write = true;
                    state.invalidate_storage();
                }
                I::DerefMut { ref_reg, value } => {
                    use_input(*ref_reg, 2, &state);
                    use_input(*value, 4, &state);
                    may_write = true;
                    state.invalidate_storage();
                }
                I::DropRef { src } => {
                    use_input(*src, 8, &state);
                    may_write = true;
                    state.invalidate_storage();
                }
                I::ChkRef { ref_reg } => use_input(*ref_reg, 8, &state),
                I::Nop | I::SetCallWitness { .. } => {}
                // Unhandled writes/effects are a barrier, never a stale fact.
                I::CbgrExtended { .. } | I::FfiExtended { .. }
                    if sites.get(&pc).is_some_and(|sites| {
                        sites
                            .iter()
                            .any(|site| matches!(site, ReferenceSiteKind::FieldOutput { .. }))
                    }) =>
                {
                    for site in &sites[&pc] {
                        if let ReferenceSiteKind::FieldOutput {
                            register,
                            base,
                            slot,
                        } = site
                        {
                            use_input(Reg(*base), 1, &state);
                            write =
                                Some((Reg(*register), if *slot { R::Slot } else { R::Unknown }));
                            written_cell = Some(true);
                        }
                    }
                }
                _ => {
                    for uses in &mut parameter_uses {
                        *uses |= 8;
                    }
                    may_write = true;
                    state = State::default();
                }
            }
            if let Some((dst, mut value)) = write {
                if !value.bounded() {
                    value = R::Unknown;
                }
                state.put(dst, value);
                if let Some(origin) = written_origin {
                    state.parameter_origins.insert(dst.0, origin);
                }
                if let Some(cell) = written_cell {
                    state.cells.insert(dst.0, cell);
                } else {
                    state.cells.remove(&dst.0);
                }
            }
            if next && pc + 1 < instructions.len() {
                edges.push((pc + 1, state));
            }
            for (target, incoming) in edges {
                let joined = states
                    .get(&target)
                    .map(|old| old.join(&incoming))
                    .unwrap_or(incoming);
                if !joined.bounded() {
                    return BodyFacts::unknown(instructions.len());
                }
                states.insert(target, joined);
            }
        }
        let result = returns.unwrap_or(R::Unknown);
        let result = if result.bounded() { result } else { R::Unknown };
        BodyFacts {
            result,
            value_inputs,
            parameter_uses,
            call_inputs,
            may_write,
        }
    }
}

/// Compute the complete edit plan before changing any sealed body. Each load
/// is a use-local view; original registers and cell addresses remain untouched.
pub(super) fn apply<'ctx>(
    context: &'ctx verum_llvm::context::Context,
    module: &verum_llvm::module::Module<'ctx>,
    vbc: &VbcModule,
    authority: &super::native_call::NativeCallAuthority<'ctx>,
    calls: Map<u32, List<(u32, NativeCallReceipt<'ctx>)>>,
) -> super::error::Result<()> {
    use super::error::{BuildExt, LlvmLoweringError};
    use verum_llvm::values::{AsValueRef, BasicValue, BasicValueEnum, InstructionValue};
    // This value-slot ABI is currently 64 bits. A different target needs its
    // own exact storage authority, never an implicit i64 pointer assumption.
    if !super::target_triple::target_is_x86_64(module)
        && !super::target_triple::target_is_aarch64(module)
    {
        return Ok(());
    }
    let sites = authority.resolve_reference_sites(&calls);
    let mut analysis = Analysis::new(vbc, &calls, &sites);
    let mut plans: List<(InstructionValue<'ctx>, Option<u32>, BasicValueEnum<'ctx>)> = List::new();
    let mut ids: List<_> = sites.keys().copied().collect();
    ids.sort_unstable();
    for id in ids {
        let body_sites = &sites[&id];
        let facts = analysis.body(FunctionId(id));
        for site in body_sites {
            let ReferenceSiteKind::ValueInput(register) = site.kind else {
                continue;
            };
            if !facts
                .value_inputs
                .get(&site.instruction)
                .is_some_and(|regs| regs.contains(&register))
            {
                continue;
            }
            let Some(anchor) = site.anchor else {
                continue;
            };
            let Some(value) = anchor.get_operand(0).and_then(|operand| operand.value()) else {
                continue;
            };
            plans.push((anchor, None, value));
        }
        for (_, call) in calls.get(&id).into_iter().flatten() {
            let Some(arguments) = facts.call_inputs.get(&call.instruction) else {
                continue;
            };
            // This is the actual emitted argument list, after static receiver
            // omission and ABI adaptation, not a reconstructed VBC arg range.
            let instruction = unsafe { InstructionValue::new(call.call.as_value_ref()) };
            for &index in arguments {
                let Some(argument) = call.arguments.get(index) else {
                    continue;
                };
                let value = instruction
                    .get_operand(index as u32)
                    .and_then(|operand| operand.value());
                if let Some(value) = value {
                    if value.as_value_ref() == argument.value.as_value_ref() {
                        plans.push((instruction, Some(index as u32), value));
                    }
                }
            }
        }
    }
    let builder = context.create_builder();
    for (instruction, operand, value) in plans {
        let pointer = match value {
            BasicValueEnum::IntValue(bits) if bits.get_type().get_bit_width() == 64 => {
                builder.position_before(&instruction);
                builder
                    .build_int_to_ptr(
                        bits,
                        context.ptr_type(verum_llvm::AddressSpace::default()),
                        "reference_cell",
                    )
                    .or_llvm_err()?
            }
            BasicValueEnum::PointerValue(pointer) => {
                builder.position_before(&instruction);
                pointer
            }
            _ => continue,
        };
        let loaded = builder
            .build_load(context.i64_type(), pointer, "reference_value_load")
            .or_llvm_err()?;
        if let Some(index) = operand {
            let adapted = if value.is_pointer_value() {
                builder
                    .build_int_to_ptr(
                        loaded.into_int_value(),
                        value.into_pointer_value().get_type(),
                        "reference_argument",
                    )
                    .map(Into::into)
                    .or_llvm_err()?
            } else {
                loaded
            };
            if !instruction.set_operand(index, adapted) {
                return Err(LlvmLoweringError::internal(
                    "validated reference operand disappeared",
                ));
            }
        } else {
            let replacement = loaded.as_instruction_value().ok_or_else(|| {
                LlvmLoweringError::internal("reference view load is not an instruction")
            })?;
            instruction.replace_all_uses_with(&replacement);
            instruction.erase_from_basic_block();
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../tests/llvm/returned_representation_budget.rs"]
mod budget_tests;
