//! Carry the first successful alternative from a pattern test to its bindings.
use super::{CodegenError, CodegenErrorKind, CodegenResult, PatternBindingMode, VbcCodegen};
use crate::instruction::{CompareOp, Instruction, Reg};
use crate::types::TypeRef;
use verum_ast::Pattern;
use verum_common::List;

fn binding_error(pattern: &Pattern, message: impl Into<String>) -> CodegenError {
    CodegenError::with_span(
        CodegenErrorKind::InvalidSourceSignature(message.into()),
        pattern.span,
    )
}

impl VbcCodegen {
    fn or_binding_names(&self, pattern: &Pattern) -> List<String> {
        let mut names = List::new();
        self.collect_pattern_variables(pattern, &mut names);
        names.sort_unstable();
        names
    }

    pub(super) fn test_or_pattern(
        &mut self,
        pattern: &Pattern,
        alternatives: &[Pattern],
        scrutinee: Reg,
        matched: Reg,
    ) -> CodegenResult<()> {
        let first = alternatives
            .first()
            .ok_or_else(|| binding_error(pattern, "empty or-pattern"))?;
        let names = self.or_binding_names(first);
        for alternative in &alternatives[1..] {
            if self.or_binding_names(alternative) != names {
                return Err(binding_error(
                    alternative,
                    "or-pattern alternatives bind different names",
                ));
            }
        }
        let key = (pattern.span, std::ptr::from_ref(pattern).addr());
        let choice = if names.is_empty() {
            None
        } else if let Some(register) = self.ctx.registers.or_pattern_choices.get(&key) {
            Some(*register)
        } else {
            let register = self.ctx.alloc_temp();
            self.ctx.registers.or_pattern_choices.insert(key, register);
            Some(register)
        };
        let end = self.ctx.new_label("or_test_end");
        for (index, alternative) in alternatives.iter().enumerate() {
            let next = self.ctx.new_label("or_test_next");
            self.compile_pattern_test(alternative, scrutinee, matched)?;
            self.ctx
                .emit_forward_jump(&next, |offset| Instruction::JmpNot {
                    cond: matched,
                    offset,
                });
            if let Some(dst) = choice {
                self.ctx.emit(Instruction::LoadI {
                    dst,
                    value: index as i64,
                });
            }
            self.ctx
                .emit_forward_jump(&end, |offset| Instruction::Jmp { offset });
            self.ctx.define_label(&next);
        }
        self.ctx.define_label(&end);
        Ok(())
    }

    pub(super) fn bind_or_pattern(
        &mut self,
        pattern: &Pattern,
        alternatives: &[Pattern],
        scrutinee: Reg,
        mode: &PatternBindingMode,
        declaration: Option<&TypeRef>,
    ) -> CodegenResult<()> {
        let first = alternatives
            .first()
            .ok_or_else(|| binding_error(pattern, "empty or-pattern"))?;
        let names = self.or_binding_names(first);
        if names.is_empty()
            && alternatives
                .iter()
                .all(|alternative| self.or_binding_names(alternative).is_empty())
        {
            return Ok(());
        }
        let key = (pattern.span, std::ptr::from_ref(pattern).addr());
        // Irrefutable parameter and local destructuring can reach the binder
        // directly. Refutable sites already ran the test, including any guard.
        if !self.ctx.registers.or_pattern_choices.contains_key(&key) {
            let matched = self.ctx.alloc_temp();
            self.test_or_pattern(pattern, alternatives, scrutinee, matched)?;
            let valid = self.ctx.new_label("or_binding_matched");
            self.ctx
                .emit_forward_jump(&valid, |offset| Instruction::JmpIf {
                    cond: matched,
                    offset,
                });
            let message_id = self.intern_string("or-pattern did not match");
            self.ctx.emit(Instruction::Panic { message_id });
            self.ctx.define_label(&valid);
            self.ctx.free_temp(matched);
        }
        let selected = self.ctx.registers.or_pattern_choices[&key];
        let expected = self.ctx.alloc_temp();
        let matches = self.ctx.alloc_temp();
        let end = self.ctx.new_label("or_binding_end");
        let mut destinations = List::new();
        for (index, alternative) in alternatives.iter().enumerate() {
            let next = self.ctx.new_label("or_binding_next");
            if index + 1 < alternatives.len() {
                self.ctx.emit(Instruction::LoadI {
                    dst: expected,
                    value: index as i64,
                });
                self.ctx.emit(Instruction::CmpI {
                    op: CompareOp::Eq,
                    dst: matches,
                    a: selected,
                    b: expected,
                });
                self.ctx
                    .emit_forward_jump(&next, |offset| Instruction::JmpNot {
                        cond: matches,
                        offset,
                    });
            }
            if index != 0 {
                self.ctx.enter_scope();
            }
            let bound = (|| {
                self.compile_pattern_bind_mode(alternative, scrutinee, mode, declaration)?;
                for (position, name) in names.iter().enumerate() {
                    let info = self.ctx.lookup_var(name).ok_or_else(|| {
                        binding_error(alternative, format!("missing or-pattern binding '{name}'"))
                    })?;
                    let reg = info.reg;
                    let ty = self
                        .ctx
                        .registers
                        .value_uses
                        .active
                        .get(&reg)
                        .and_then(|fact| fact.declaration_type.clone());
                    let contract = (ty, info.is_mutable, info.pattern_alias);
                    if index == 0 {
                        destinations.push((reg, contract));
                    } else {
                        let (destination, established) = &destinations[position];
                        if *established != contract {
                            return Err(binding_error(
                                alternative,
                                format!("incompatible or-pattern binding '{name}'"),
                            ));
                        }
                        self.ctx.emit(Instruction::Mov {
                            dst: *destination,
                            src: reg,
                        });
                    }
                }
                Ok(())
            })();
            if index != 0 {
                self.ctx.exit_scope(false);
            }
            bound?;
            if index + 1 < alternatives.len() {
                self.ctx
                    .emit_forward_jump(&end, |offset| Instruction::Jmp { offset });
                self.ctx.define_label(&next);
            }
        }
        self.ctx.define_label(&end);
        self.ctx.free_temp(expected);
        self.ctx.free_temp(matches);
        Ok(())
    }
}
