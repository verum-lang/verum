//! Named record operations elaborated to the kernel's ordinary Sigma rules.
//! Declaration positions give dependencies their identity. The compilation
//! frontend uses the same ordered substitution and partial-write plans.

use std::collections::BTreeMap;
use thiserror::Error;
use verum_common::{
    Heap, Text,
    record_telescope::{RecordTelescope, TelescopeError},
};

use crate::{
    AxiomRegistry, Context, CoreTerm, InductiveRegistry, KernelError, free_vars,
    infer_with_inductives, substitute_all, verify_with_inductives,
};

/// Nested Sigma checking is recursive in the kernel. Bound the depth added by
/// the record lowering independently of the flat frontend telescope budget.
pub const MAX_SIGMA_FIELDS: usize = 64;

/// A rejected telescope operation or its underlying kernel typing judgment.
#[derive(Debug, Error)]
pub enum RecordError {
    /// No right-associated representation is available for this field count.
    #[error("Sigma records require between one and {MAX_SIGMA_FIELDS} fields")]
    FieldCount,
    /// Empty or repeated field name.
    #[error("record field names must be nonempty and unique: {0}")]
    FieldName(Text),
    /// The declared schema was a value rather than a type.
    #[error("record field type does not inhabit a universe")]
    NotAType,
    /// The record value supplied a different number of components.
    #[error("expected {expected} record components, got {actual}")]
    WrongArity {
        /// Number of declared fields.
        expected: usize,
        /// Number of supplied components.
        actual: usize,
    },
    /// A partial write could invalidate the type of a later component.
    #[error("another record field depends on slot {0}; reconstruct the whole record")]
    DependedOn(usize),
    /// The shared ordered-dependency checker rejected the operation.
    #[error(transparent)]
    Telescope(#[from] TelescopeError),
    /// The kernel rejected a schema, value, or projection.
    #[error(transparent)]
    Kernel(#[from] KernelError),
}

/// The terms actually substituted into a field, and the type checked afterward.
#[derive(Clone, Debug)]
pub struct SpecializedField {
    /// Each named dependency slot and its simultaneous replacement.
    pub substitutions: Vec<(usize, CoreTerm)>,
    /// The field type after applying those replacements.
    pub ty: CoreTerm,
}

/// A kernel-checked introduction and its component typing observations.
#[derive(Clone, Debug)]
pub struct RecordIntroduction {
    /// Checked right-associated pair.
    pub term: CoreTerm,
    /// Specialized components in declaration order.
    pub fields: Vec<SpecializedField>,
}

/// A kernel-checked elimination and its result type.
#[derive(Clone, Debug)]
pub struct RecordProjection {
    /// Checked projection of the supplied receiver.
    pub term: CoreTerm,
    /// Simultaneous substitutions and the resulting projection type.
    pub field: SpecializedField,
}

/// A named record schema formed through nested kernel Sigma judgments.
#[derive(Debug)]
pub struct SigmaRecord {
    fields: Vec<(Text, CoreTerm)>,
    plan: RecordTelescope,
    schema: CoreTerm,
}

impl SigmaRecord {
    /// Form a nested Sigma in the supplied kernel environment. Free field
    /// references are obtained from the kernel's lexical free-variable walk.
    pub fn form(
        ctx: &Context,
        fields: Vec<(Text, CoreTerm)>,
        axioms: &AxiomRegistry,
        inductives: &InductiveRegistry,
    ) -> Result<Self, RecordError> {
        if fields.is_empty() || fields.len() > MAX_SIGMA_FIELDS {
            return Err(RecordError::FieldCount);
        }
        let mut slots = BTreeMap::new();
        for (index, (name, _)) in fields.iter().enumerate() {
            if name.is_empty() || slots.insert(name.clone(), index).is_some() {
                return Err(RecordError::FieldName(name.clone()));
            }
        }
        let dependencies = fields.iter().map(|(_, ty)| {
            free_vars(ty)
                .into_iter()
                .filter_map(|name| slots.get(&name).copied())
        });
        let plan = RecordTelescope::from_dependencies(dependencies)?;
        let ((_, last), preceding) = fields.split_last().ok_or(RecordError::FieldCount)?;
        let schema = preceding
            .iter()
            .rev()
            .fold(last.clone(), |suffix, (name, ty)| CoreTerm::Sigma {
                binder: name.clone(),
                fst_ty: Heap::new(ty.clone()),
                snd_ty: Heap::new(suffix),
            });
        let kind = infer_with_inductives(ctx, &schema, axioms, inductives)?;
        if !matches!(
            crate::support::normalize_full(&kind, axioms, inductives),
            CoreTerm::Universe(_)
        ) {
            return Err(RecordError::NotAType);
        }
        Ok(Self {
            fields,
            plan,
            schema,
        })
    }

    /// The formed type available for binding record receivers in a context.
    pub fn schema(&self) -> &CoreTerm {
        &self.schema
    }
    /// The canonical slot dependencies computed from the field types.
    pub fn plan(&self) -> &RecordTelescope {
        &self.plan
    }

    fn specialize(&self, field: usize, substitutions: Vec<(usize, CoreTerm)>) -> SpecializedField {
        let replacements: Vec<_> = substitutions
            .iter()
            .map(|(slot, value)| (self.fields[*slot].0.clone(), value.clone()))
            .collect();
        let ty = substitute_all(&self.fields[field].1, &replacements);
        SpecializedField { substitutions, ty }
    }

    /// Check each value at its specialized field type, then check the packed
    /// pair at the complete Sigma type. The kernel handles binder capture.
    pub fn construct(
        &self,
        ctx: &Context,
        values: &[CoreTerm],
        axioms: &AxiomRegistry,
        inductives: &InductiveRegistry,
    ) -> Result<RecordIntroduction, RecordError> {
        if values.len() != self.fields.len() {
            return Err(RecordError::WrongArity {
                expected: self.fields.len(),
                actual: values.len(),
            });
        }
        let mut prefix = Vec::with_capacity(values.len());
        let mut fields = Vec::with_capacity(values.len());
        for (slot, value) in values.iter().enumerate() {
            let substitutions = self.plan.construction_substitution(slot, &prefix)?;
            let field = self.specialize(slot, substitutions);
            verify_with_inductives(ctx, value, &field.ty, axioms, inductives)?;
            fields.push(field);
            prefix.push(Some(value.clone()));
        }
        let (last, preceding) = values.split_last().ok_or(RecordError::FieldCount)?;
        let term = preceding.iter().rev().fold(last.clone(), |suffix, value| {
            CoreTerm::Pair(Heap::new(value.clone()), Heap::new(suffix))
        });
        verify_with_inductives(ctx, &term, self.schema(), axioms, inductives)?;
        Ok(RecordIntroduction { term, fields })
    }

    fn select(&self, receiver: &Text, slot: usize) -> Result<CoreTerm, TelescopeError> {
        let selector = self.plan.selector(slot)?;
        let mut term = CoreTerm::Var(receiver.clone());
        for _ in 0..selector.snd_steps {
            term = CoreTerm::Snd(Heap::new(term));
        }
        if selector.take_fst {
            term = CoreTerm::Fst(Heap::new(term));
        }
        Ok(term)
    }

    /// Specialize at projections of this receiver and independently check the
    /// resulting Fst/Snd judgment. A named receiver is stable in this context.
    pub fn project(
        &self,
        ctx: &Context,
        receiver: &Text,
        slot: usize,
        axioms: &AxiomRegistry,
        inductives: &InductiveRegistry,
    ) -> Result<RecordProjection, RecordError> {
        let projected = self.plan.projection_substitution(slot, receiver)?;
        verify_with_inductives(
            ctx,
            &CoreTerm::Var(receiver.clone()),
            self.schema(),
            axioms,
            inductives,
        )?;
        let substitutions: Result<Vec<_>, TelescopeError> = projected
            .into_iter()
            .map(|(index, projection)| {
                Ok((index, self.select(&projection.receiver, projection.field)?))
            })
            .collect();
        let field = self.specialize(slot, substitutions?);
        let term = self.select(receiver, slot)?;
        verify_with_inductives(ctx, &term, &field.ty, axioms, inductives)?;
        Ok(RecordProjection { term, field })
    }

    /// Check the typing obligation for updating one component. Changing an
    /// index with dependent fields requires a fresh whole-record introduction.
    pub fn check_mutation(
        &self,
        ctx: &Context,
        receiver: &Text,
        slot: usize,
        replacement: &CoreTerm,
        axioms: &AxiomRegistry,
        inductives: &InductiveRegistry,
    ) -> Result<RecordProjection, RecordError> {
        if !self.plan.may_mutate(slot)? {
            return Err(RecordError::DependedOn(slot));
        }
        let projection = self.project(ctx, receiver, slot, axioms, inductives)?;
        verify_with_inductives(ctx, replacement, &projection.field.ty, axioms, inductives)?;
        Ok(projection)
    }
}
