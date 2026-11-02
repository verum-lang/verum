//! Introduction and elimination of nominal records with type-valued fields.
//! Slot numbers belong to the declaration. Projection substitutes the identity
//! of the receiver, while construction substitutes the supplied type value.
use super::TypeChecker;
use crate::ty::{RecordFieldSlot, RecordReceiver, Type};
use crate::{Result, TypeError};
use indexmap::IndexMap;
use verum_ast::{Expr, ExprKind, FieldInit, Span};
use verum_common::{Text, record_telescope::RecordTelescope};

type Fields = IndexMap<Text, Type>;

fn refusal(message: impl Into<Text>, span: Span) -> TypeError {
    TypeError::UniverseBinding {
        message: message.into(),
        span,
    }
}

struct Layout {
    owner: Text,
    telescope: RecordTelescope,
}

impl Layout {
    fn from_fields(fields: &Fields, span: Span) -> Result<Option<Self>> {
        let mut owner = None;
        let mut rows = Vec::with_capacity(fields.len());
        for ty in fields.values() {
            let mut row = Vec::new();
            for slot in ty.record_field_slots() {
                if owner.get_or_insert_with(|| slot.owner.clone()) != &slot.owner {
                    return Err(refusal(
                        "dependent field refers to another record declaration",
                        span,
                    ));
                }
                if !matches!(
                    fields.get_index(slot.ordinal),
                    Some((_, Type::Universe { .. }))
                ) {
                    return Err(refusal(
                        "dependent field requires a type-valued earlier slot",
                        span,
                    ));
                }
                row.push(slot.ordinal);
            }
            rows.push(row);
        }
        let Some(owner) = owner else { return Ok(None) };
        let telescope =
            RecordTelescope::from_dependencies(rows).map_err(|e| refusal(e.to_string(), span))?;
        Ok(Some(Self { owner, telescope }))
    }

    fn projection(
        &self,
        field: usize,
        ty: &Type,
        receiver: RecordReceiver,
        span: Span,
    ) -> Result<Type> {
        let substitutions = self
            .telescope
            .projection_substitution(field, &receiver)
            .map_err(|e| refusal(e.to_string(), span))?;
        ty.try_instantiate_record_fields(|slot, level| {
            let i = substitutions
                .binary_search_by_key(&slot.ordinal, |(index, _)| *index)
                .map_err(|_| Text::from("missing record projection"))?;
            let (_, projection) = &substitutions[i];
            Ok(Type::RecordField {
                slot: Box::new(slot.clone()),
                receiver: Some(Box::new(projection.receiver.clone())),
                level: level.clone(),
            })
        })
        .map_err(|e| refusal(e, span))
    }
}

pub(super) fn dependent_record_owner(fields: &Fields, span: Span) -> Result<Option<Text>> {
    Ok(Layout::from_fields(fields, span)?.map(|layout| layout.owner))
}

impl TypeChecker {
    /// Common ordinary-record fast path. The marker and field types are both
    /// written in the declaration registration transaction.
    pub(super) fn is_dependent_record(&self, ty: &Type) -> bool {
        if self
            .ctx
            .lookup_type("__has_dependent_record_layouts")
            .is_none()
        {
            return false;
        }
        let head = self.record_conversion_head(ty);
        match &head {
            Type::Record(fields) => fields
                .values()
                .any(|ty| !ty.record_field_slots().is_empty()),
            Type::Named { path, .. } => self
                .ctx
                .lookup_type(&format!("__dependent_record_{}", self.path_to_string(path)))
                .is_some(),
            Type::Generic { name, .. } => self
                .ctx
                .lookup_type(&format!("__dependent_record_{name}"))
                .is_some(),
            _ => false,
        }
    }

    pub(super) fn require_closed_field_projection(&self, ty: &Type, span: Span) -> Result<()> {
        if !ty.record_field_slots().is_empty() {
            return Err(refusal(
                "bind the record to an immutable local before projecting a dependent field",
                span,
            ));
        }
        Ok(())
    }

    fn dependent_fields(&self, ty: &Type, span: Span) -> Result<Option<(Fields, Layout)>> {
        if !self.is_dependent_record(ty) {
            return Ok(None);
        }
        let head = self.record_conversion_head(ty);
        let fields = self
            .resolve_to_record_type(&head)
            .ok_or_else(|| refusal("missing declared dependent record fields", span))?;
        let layout = Layout::from_fields(&fields, span)?
            .ok_or_else(|| refusal("missing dependent record telescope", span))?;
        Ok(Some((fields, layout)))
    }

    fn stable_receiver_type(&self, ty: &Type, span: Span) -> Result<bool> {
        let mut head = self.place_reference_head(ty, span)?;
        for _ in 0..64 {
            match head {
                Type::Reference {
                    mutable: false,
                    inner,
                } => head = self.place_reference_head(&inner, span)?,
                Type::Named { .. } | Type::Generic { .. } | Type::Record(_) => return Ok(true),
                _ => return Ok(false),
            }
        }
        Ok(false)
    }

    /// A projection can escape into the type of another value. Consequently its
    /// root must remain stable even after ordinary evaluation has finished.
    fn record_receiver(&mut self, expr: &Expr, origin: Span) -> Result<RecordReceiver> {
        let unstable = || {
            refusal(
                "dependent projection requires an immutable local receiver or shared reference",
                expr.span,
            )
        };
        match &expr.kind {
            ExprKind::Paren(inner) => self.record_receiver(inner, origin),
            ExprKind::Path(path) if path.segments.len() == 1 => {
                let name = path.last_segment_name();
                let scheme = self.ctx.env.lookup(name).ok_or_else(unstable)?;
                if !self.stable_receiver_type(&scheme.ty, expr.span)? {
                    return Err(unstable());
                }
                let binding = self
                    .ctx
                    .env
                    .receiver_id(name, origin)
                    .ok_or_else(unstable)?;
                Ok(RecordReceiver {
                    binding,
                    fields: Default::default(),
                })
            }
            ExprKind::Field {
                expr: parent,
                field,
            } => {
                let parent_ty = self.synth_expr_for_field_access(parent)?.ty;
                let fields = self
                    .resolve_to_record_type(&self.record_conversion_head(&parent_ty))
                    .ok_or_else(unstable)?;
                let field_ty = fields.get(&field.name).ok_or_else(unstable)?;
                if !self.stable_receiver_type(field_ty, expr.span)? {
                    return Err(unstable());
                }
                let mut receiver = self.record_receiver(parent, origin)?;
                receiver.fields.push(field.name.clone());
                Ok(receiver)
            }
            _ => Err(unstable()),
        }
    }

    /// Project a field, or check the same field as a writable place. An
    /// independent field of the record needs no receiver in its type.
    pub(super) fn dependent_field_type(
        &mut self,
        receiver: &Expr,
        ty: &Type,
        name: &str,
        writable: bool,
        span: Span,
    ) -> Result<Option<Type>> {
        let Some((fields, layout)) = self.dependent_fields(ty, span)? else {
            return Ok(None);
        };
        let (field, _, field_ty) = fields
            .get_full(name)
            .ok_or_else(|| refusal(format!("unknown field {name} in {}", layout.owner), span))?;
        if writable
            && !layout
                .telescope
                .may_mutate(field)
                .map_err(|e| refusal(e.to_string(), span))?
        {
            return Err(refusal(
                format!(
                    "changing {name} would invalidate a dependent field; replace the complete record"
                ),
                span,
            ));
        }
        if layout
            .telescope
            .dependencies(field)
            .map_err(|e| refusal(e.to_string(), span))?
            .is_empty()
        {
            return Ok(Some(field_ty.clone()));
        }
        let place = self.record_receiver(receiver, span)?;
        layout.projection(field, field_ty, place, span).map(Some)
    }

    fn carrier_value(&mut self, value: &Expr, expected: &Type) -> Result<Type> {
        let Type::Universe {
            level: expected_level,
        } = expected
        else {
            return Err(refusal(
                "record carrier must have a declared universe",
                value.span,
            ));
        };
        if let ExprKind::Paren(inner) = &value.kind {
            return self.carrier_value(inner, expected);
        }
        if let Some(syntax) = self.declaration_property_operand(value) {
            let ty = self.ast_to_type(&syntax)?;
            let level = match &ty {
                Type::Unit
                | Type::Bool
                | Type::Int
                | Type::Float
                | Type::Char
                | Type::Text
                | Type::Never => crate::universe_level::UniverseLevel::TYPE,
                Type::Universe { level } => level.succ(),
                Type::Prop => crate::universe_level::UniverseLevel::TYPE1,
                _ => {
                    return Err(refusal(
                        "record carrier requires an established universe for this type",
                        value.span,
                    ));
                }
            };
            if !level.is_less_or_equal(expected_level) {
                return Err(refusal(
                    "record carrier exceeds its declared universe",
                    value.span,
                ));
            }
            return Ok(ty);
        }
        if let ExprKind::Field {
            expr: receiver,
            field,
        } = &value.kind
        {
            let receiver_ty = self.synth_expr_for_field_access(receiver)?.ty;
            self.check_record_field_visibility(&receiver_ty, &field.name, value.span)?;
            if let Some((fields, layout)) = self.dependent_fields(&receiver_ty, value.span)?
                && let Some((ordinal, _, Type::Universe { level })) = fields.get_full(&field.name)
                && level.is_less_or_equal(expected_level)
            {
                return Ok(Type::RecordField {
                    slot: Box::new(RecordFieldSlot {
                        owner: layout.owner,
                        ordinal,
                    }),
                    receiver: Some(Box::new(self.record_receiver(receiver, value.span)?)),
                    level: level.clone(),
                });
            }
        }
        Err(refusal(
            "dependent construction requires a checked type or a stable type-valued field",
            value.span,
        ))
    }

    /// Type-check all supplied values against the ordered family. This walk is
    /// about static substitution only; it does not reorder runtime evaluation.
    pub(super) fn check_dependent_record_fields(
        &mut self,
        record: &Type,
        fields: &Fields,
        supplied: &[FieldInit],
        base: Option<&Expr>,
        span: Span,
    ) -> Result<bool> {
        if !self.is_dependent_record(record) {
            return Ok(false);
        }
        let layout = Layout::from_fields(fields, span)?
            .ok_or_else(|| refusal("missing dependent fields during construction", span))?;
        let mut arguments = vec![None; fields.len()];
        for arg in supplied {
            let index = fields.get_index_of(&arg.name.name).ok_or_else(|| {
                refusal(format!("unknown record field {}", arg.name.name), arg.span)
            })?;
            if arguments[index].replace(arg).is_some() {
                return Err(refusal(
                    format!("duplicate record field {}", arg.name.name),
                    arg.span,
                ));
            }
        }
        if let Some(base) = base {
            self.check_expr(base, record)?;
        }
        let mut prefix: Vec<Option<Type>> = Vec::with_capacity(fields.len());
        for (index, (name, field_ty)) in fields.iter().enumerate() {
            let substitutions = layout
                .telescope
                .construction_substitution(index, &prefix)
                .map_err(|e| refusal(e.to_string(), span))?;
            let expected = field_ty
                .try_instantiate_record_fields(|slot, _| {
                    substitutions
                        .binary_search_by_key(&slot.ordinal, |(i, _)| *i)
                        .map(|i| substitutions[i].1.clone())
                        .map_err(|_| Text::from("missing record construction substitution"))
                })
                .map_err(|e| refusal(e, span))?;
            let inherited;
            let shorthand;
            let value = if let Some(arg) = arguments[index] {
                if let Some(value) = &arg.value {
                    value
                } else {
                    shorthand = Expr::ident(arg.name.clone());
                    &shorthand
                }
            } else if let Some(base) = base {
                inherited = Expr::new(
                    ExprKind::Field {
                        expr: Box::new(base.clone()),
                        field: verum_ast::Ident::new(name.clone(), span),
                    },
                    span,
                );
                &inherited
            } else {
                return Err(refusal(
                    format!("missing dependent record field {name}"),
                    span,
                ));
            };
            if layout
                .telescope
                .may_mutate(index)
                .map_err(|e| refusal(e.to_string(), span))?
            {
                if substitutions.is_empty() {
                    self.check_expr(value, &expected)?;
                } else {
                    // Infer the value independently. Contextual checking can return the
                    // expected type and would lose the evidence being compared here.
                    let actual = self.synth_expr(value)?.ty;
                    self.unifier.unify(&actual, &expected, value.span)?;
                    self.check_record_family_agreement(&actual, &expected, value.span)?;
                }
                prefix.push(None);
            } else {
                prefix.push(Some(self.carrier_value(value, &expected)?));
            }
        }
        Ok(true)
    }
}
