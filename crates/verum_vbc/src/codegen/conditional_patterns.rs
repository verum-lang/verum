//! Ownership of temporary sums in refutable conditions.
use super::{CodegenError, CodegenResult, VbcCodegen};
use crate::codegen::registers::{BindingObservation, PatternAlias};
use crate::instruction::{Instruction, Reg};
use crate::types::{TypeKind, TypeRef};
use verum_ast::{Expr, ExprKind, Pattern, PatternKind, VariantPatternData};
use verum_common::{List, ResourceDiscipline};

struct Field {
    name: verum_common::Text,
    register: Reg,
    field: u32,
    reads: BindingObservation,
}

pub(super) struct ConditionalSubject {
    register: Reg,
    declared: Option<TypeRef>,
    pub(super) failure: Option<String>,
    fields: List<Field>,
}

impl ConditionalSubject {
    pub(super) fn failure_label<'a>(&'a self, fallback: &'a str) -> &'a str {
        self.failure.as_deref().unwrap_or(fallback)
    }
}

impl VbcCodegen {
    pub(super) fn conditional_pattern(
        term: &verum_ast::ConditionKind,
    ) -> Option<(&Pattern, &Expr)> {
        match term {
            verum_ast::ConditionKind::Let { pattern, value } => Some((pattern, value)),
            verum_ast::ConditionKind::Expr(Expr {
                kind:
                    ExprKind::Is {
                        expr,
                        pattern,
                        negated: false,
                    },
                ..
            }) if Self::pattern_has_bindings(pattern) => Some((pattern, expr)),
            _ => None,
        }
    }

    /// The mismatch edge only owns subjects already evaluated in this chain.
    /// The lexical owner also survives through the body and its abrupt exits.
    pub(super) fn bind_conditional_subject(
        &mut self,
        pattern: &Pattern,
        expr: &Expr,
        failure: &str,
    ) -> CodegenResult<ConditionalSubject> {
        let start = self.ctx.instructions.len();
        let value = self
            .compile_expr(expr)?
            .ok_or_else(|| CodegenError::internal("conditional pattern has no scrutinee"))?;
        let ty =
            self.declared_expression_result(expr, Some(value), start, self.ctx.instructions.len());
        let owned = matches!(
            Self::normalized_place(expr).kind,
            ExprKind::Call { .. } | ExprKind::MethodCall { .. }
        ) && ty
            .as_ref()
            .and_then(TypeRef::base_type_id)
            .and_then(|id| self.type_by_id(id))
            .is_some_and(|decl| decl.kind == TypeKind::Sum)
            && !matches!(ty, Some(TypeRef::Reference { .. }));
        let mut subject = ConditionalSubject {
            register: value,
            declared: None,
            failure: None,
            fields: List::new(),
        };
        if let Some(ty) = ty.filter(|_| owned) {
            let name = self.ctx.new_label("$condition_subject");
            let reg = self.ctx.define_var(&name, false);
            self.ctx
                .lookup_var_mut(&name)
                .expect("new condition subject")
                .conditional_owner = true;
            let discipline = self.value_resource_discipline(&ty);
            self.ctx
                .registers
                .value_uses
                .set_type(reg, ty.clone(), discipline);
            self.ctx.emit(Instruction::Mov {
                dst: reg,
                src: value,
            });
            self.ctx.free_temp(value);
            subject.register = reg;
            subject.declared = Some(ty);
            subject.failure = Some(self.ctx.new_label("condition_unmatched"));
        }
        let previous = self.enter_scrutinee_type_for_bind(expr, std::iter::once(pattern));
        let test = self.ctx.alloc_temp();
        self.compile_pattern_test(pattern, subject.register, test)?;
        self.ctx
            .emit_forward_jump(subject.failure_label(failure), |offset| {
                Instruction::JmpNot { cond: test, offset }
            });
        self.ctx.free_temp(test);
        self.compile_pattern_bind_from_expr(pattern, subject.register, expr)?;
        self.exit_scrutinee_type_for_bind(previous);

        // Only a direct, exclusive or type-parameter field of a shell without user Drop can
        // change owners. References and nested patterns stay with the subject.
        if subject
            .declared
            .as_ref()
            .is_some_and(|ty| self.can_extract_sum_field(ty))
            && let PatternKind::Variant {
                data: Some(VariantPatternData::Tuple(parts)),
                ..
            } = &pattern.kind
        {
            for (field, part) in parts.iter().enumerate() {
                let PatternKind::Ident {
                    name,
                    by_ref: false,
                    subpattern: None,
                    ..
                } = &part.kind
                else {
                    continue;
                };
                let Some(binding) = self.ctx.lookup_var(name.as_str()) else {
                    continue;
                };
                let reg = binding.reg;
                if binding.pattern_alias == Some(PatternAlias::Scrutinee)
                    && self
                        .ctx
                        .registers
                        .value_uses
                        .active
                        .get(&reg)
                        .is_some_and(|fact| {
                            fact.id == binding.binding_id
                                && (matches!(
                                    fact.discipline,
                                    ResourceDiscipline::Affine | ResourceDiscipline::Linear
                                ) || fact
                                    .declaration_type
                                    .as_ref()
                                    .is_some_and(TypeRef::is_generic))
                        })
                    && let Some(reads) = self.ctx.registers.observe_binding_lookups(name.as_str())
                {
                    subject.fields.push(Field {
                        name: name.name.clone(),
                        register: reg,
                        field: field as u32,
                        reads,
                    });
                }
            }
        }
        Ok(subject)
    }

    /// Later conditions can observe a field or shadow its name. Only a binding
    /// still unique after all conditions have passed takes its original slot.
    pub(super) fn acquire_conditional_fields(&mut self, subject: &ConditionalSubject) {
        for field in &subject.fields {
            if field.reads.was_observed() {
                continue;
            }
            let Some(binding) = self.ctx.lookup_var_mut(&field.name) else {
                continue;
            };
            if binding.reg != field.register {
                continue;
            }
            binding.pattern_alias = None;
            binding.conditional_owner = true;
            let empty = self.ctx.alloc_temp();
            self.ctx.emit(Instruction::LoadUnit { dst: empty });
            self.ctx.emit(Instruction::SetVariantData {
                variant: subject.register,
                field: field.field,
                value: empty,
            });
            self.ctx.free_temp(empty);
        }
    }

    pub(super) fn retire_failed_condition(&mut self, subject: &ConditionalSubject) {
        if let (Some(label), Some(ty)) = (&subject.failure, &subject.declared) {
            self.ctx.define_label(label);
            self.emit_local_drop(subject.register, Some(ty));
        }
    }

    pub(super) fn release_condition_register(&mut self, subject: &ConditionalSubject) {
        if subject.declared.is_none() {
            self.ctx.free_temp(subject.register);
        }
    }

    pub(super) fn retire_conditional_owners(&mut self, outer_level: usize) {
        for binding in self.ctx.registers.conditional_owners_above(outer_level) {
            if let Some(ty) = self.ctx.registers.value_uses.captured_declaration(&binding) {
                self.emit_local_drop(binding.reg, Some(&ty));
            }
        }
    }
}
