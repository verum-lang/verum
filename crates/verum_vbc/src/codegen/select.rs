//! Selection drives retained Future values; it never awaits an operand while
//! preparing the others. The same polling loop lowers to VBC and native code.

use super::*;
use verum_ast::{Span, expr::SelectArm};
use verum_common::{List, Maybe};

pub(super) struct PollCall {
    pub(super) function: u32,
    pub(super) ready_tag: u32,
    pub(super) pending_tag: u32,
    pub(super) type_args: List<crate::types::TypeRef>,
    pub(super) future_type: crate::types::TypeRef,
    pub(super) output_type: crate::types::TypeRef,
}

struct Operand {
    future: Reg,
    enabled: Reg,
    initialized: Reg,
    owned: bool,
    result: Reg,
    poll: Maybe<PollCall>,
}

impl VbcCodegen {
    pub(super) fn future_is_owned(inner: &Expr) -> bool {
        !matches!(
            Self::normalized_place(inner).kind,
            ExprKind::Path(_)
                | ExprKind::Field { .. }
                | ExprKind::TupleIndex { .. }
                | ExprKind::Index { .. }
                | ExprKind::Unary {
                    op: UnOp::Deref,
                    ..
                }
        )
    }

    pub(super) fn resolve_protocol_poll(
        &mut self,
        inner: &Expr,
        expected: Option<&crate::types::TypeRef>,
    ) -> CodegenResult<Maybe<PollCall>> {
        let declaration = self
            .constructor_declaration(inner, None, expected, 0)
            .or_else(|| self.source_expression_declaration(inner, None, 0));
        let Some(name) = self
            .extract_expr_type_name(inner)
            .or_else(|| self.infer_expr_type_name(inner))
            .or_else(|| {
                declaration
                    .as_ref()
                    .and_then(|ty| self.declaration_type_name(ty, 0))
            })
        else {
            return Ok(None);
        };
        let declared_owner = declaration.as_ref().and_then(|ty| ty.base_type_id());
        if declared_owner
            .and_then(|owner| self.future_implementation_for_type(owner))
            .is_none()
            && self.future_implementation(&name).is_none()
        {
            return Ok(None);
        }
        let future_type = declaration.ok_or_else(|| {
            CodegenError::internal(format!(
                "Future `{name}` has no complete ownership declaration"
            ))
        })?;
        let owner_id = future_type
            .base_type_id()
            .ok_or_else(|| CodegenError::internal("Future has no nominal owner"))?;
        let spelling = self.resolve_type_alias(Self::strip_generic_args(&name));
        let owner = if self.type_name_to_id.get(spelling.as_str()) == Some(&owner_id) {
            spelling
        } else {
            self.type_by_id(owner_id)
                .and_then(|ty| self.ctx.strings.get(ty.name.0 as usize))
                .cloned()
                .ok_or_else(|| CodegenError::internal("Future has no declared owner name"))?
        };
        let method = self
            .ctx
            .lookup_function_in_scope(&format!("{owner}.poll"))
            .filter(|method| method.param_count == 2)
            .cloned()
            .ok_or_else(|| {
                CodegenError::internal(format!("Future `{name}` has no callable poll declaration"))
            })?;
        let nominal = match method.return_type {
            Some(crate::types::TypeRef::Concrete(id))
            | Some(crate::types::TypeRef::Instantiated { base: id, .. }) => id,
            _ => {
                return Err(CodegenError::internal(
                    "Future poll has no nominal result declaration",
                ));
            }
        };
        let ready_tag = self.lookup_stdlib_variant_tag(
            "Ready",
            "Poll.Ready",
            "poll result",
            Some(nominal),
            "select",
        )?;
        let output_type = self
            .associated_future_output(&future_type, 0)
            .ok_or_else(|| CodegenError::internal("Future.Output has no ownership declaration"))?;
        if expected.is_some_and(|expected| expected != &output_type) {
            return Err(CodegenError::internal(
                "Future.Output conflicts with its contextual declaration",
            ));
        }
        let receiver = self
            .ctx
            .semantic_fn_params
            .get(&method.id)
            .and_then(|params| params.first())
            .and_then(|ty| ty.as_ref())
            .or_else(|| {
                self.functions
                    .iter()
                    .find(|function| function.descriptor.id == method.id)
                    .and_then(|function| function.descriptor.semantic_parameter_types())
                    .and_then(|params| params.first())
                    .and_then(|ty| ty.as_ref())
            })
            .ok_or_else(|| {
                CodegenError::internal("Future.poll receiver lacks a semantic declaration")
            })?;
        let crate::types::TypeRef::Reference {
            inner: receiver, ..
        } = receiver
        else {
            return Err(CodegenError::internal(
                "Future.poll receiver is not a borrowed owner",
            ));
        };
        let mut bindings = crate::mono::TypeSubstitution::empty();
        if !self.bind_source_declaration(receiver, &future_type, &mut bindings) {
            return Err(CodegenError::internal(
                "Future.poll receiver conflicts with the selected owner",
            ));
        }
        let type_args = method
            .type_param_ids
            .iter()
            .map(|id| bindings.get(*id).cloned())
            .collect::<Option<List<_>>>()
            .ok_or_else(|| {
                CodegenError::internal("Future.poll generic declaration is incomplete")
            })?;
        Ok(Some(PollCall {
            future_type,
            output_type,
            function: method.id.0,
            ready_tag,
            pending_tag: self.lookup_stdlib_variant_tag(
                "Pending",
                "Poll.Pending",
                "poll result",
                Some(nominal),
                "Future poll",
            )?,
            type_args: type_args.into(),
        }))
    }

    pub(super) fn compile_select(
        &mut self,
        biased: bool,
        arms: &List<SelectArm>,
        demand: ResultDemand,
        span: Span,
    ) -> CodegenResult<Option<Reg>> {
        if arms.is_empty() {
            return Err(CodegenError::internal("select requires at least one arm"));
        }
        let mut operands = List::new();
        let mut default = None;
        // Guards and future expressions are evaluated exactly once, in source
        // order. A disabled arm does not construct or poll its future.
        for (index, arm) in arms.iter().enumerate() {
            let Some(expression) = &arm.future else {
                if default.replace(index).is_some() {
                    return Err(CodegenError::internal("select has multiple else arms"));
                }
                operands.push(None);
                continue;
            };
            let ExprKind::Await(inner) = &Self::normalized_place(expression).kind else {
                return Err(CodegenError::internal("select operands require .await"));
            };
            let enabled = self.ctx.alloc_temp();
            let initialized = self.ctx.alloc_temp();
            let future = self.ctx.alloc_temp();
            self.ctx.emit(Instruction::LoadFalse { dst: initialized });
            self.ctx.emit(Instruction::LoadTrue { dst: enabled });
            let skip = self.ctx.new_label("select_disabled");
            if let Some(guard) = &arm.guard {
                let value = self
                    .compile_expr(guard)?
                    .or_internal("select guard has no value")?;
                self.ctx.emit(Instruction::Mov {
                    dst: enabled,
                    src: value,
                });
                self.ctx.free_temp(value);
                self.ctx
                    .emit_forward_jump(&skip, |offset| Instruction::JmpNot {
                        cond: enabled,
                        offset,
                    });
            }
            let owned = Self::future_is_owned(inner);
            let type_name = self
                .extract_expr_type_name(inner)
                .or_else(|| self.infer_expr_type_name(inner));
            let poll = if let Some(poll) = self.resolve_protocol_poll(inner, None)? {
                Some(poll)
            } else if matches!(inner.kind, ExprKind::Spawn { .. })
                || type_name.as_deref().is_some_and(|name| {
                    name == "$TaskHandle" || name.split('<').next() == Some("Future")
                })
            {
                None
            } else {
                return Err(CodegenError::internal(format!(
                    "select requires a Future implementation or spawned task, found {}; eager async calls are not selectable",
                    type_name.as_deref().unwrap_or("an unresolved operand")
                )));
            };
            let evaluated = if !owned && poll.is_some() {
                self.compile_unary(UnOp::RefMut, inner)?
            } else {
                self.compile_expr(inner)?
            }
            .or_internal("select future has no value")?;
            self.ctx.emit(Instruction::Mov {
                dst: future,
                src: evaluated,
            });
            self.ctx.free_temp(evaluated);
            self.ctx.emit(Instruction::LoadTrue { dst: initialized });
            self.ctx.define_label(&skip);
            operands.push(Some(Operand {
                future,
                enabled,
                initialized,
                owned,
                result: self.ctx.alloc_temp(),
                poll,
            }));
        }

        let resolve = |canonical: &str, short: &str| {
            self.ctx
                .lookup_function_in_scope(canonical)
                .or_else(|| self.ctx.lookup_function_in_scope(short))
                .map(|method| method.id.0)
                .ok_or_else(|| CodegenError::internal(format!("select requires {canonical}")))
        };
        let protocol = operands
            .iter()
            .flatten()
            .any(|operand| operand.poll.is_some());
        let support = if protocol {
            Some((
                resolve("core.async.waker.noop_waker", "noop_waker")?,
                resolve("core.async.waker.Context.from_waker", "Context.from_waker")?,
            ))
        } else {
            None
        };
        let dest = self.ctx.alloc_temp();
        let waker = self.ctx.alloc_temp();
        let context = self.ctx.alloc_temp();
        let polled = self.ctx.alloc_temp();
        let ready = self.ctx.alloc_temp();
        let cursor = self.ctx.alloc_temp();
        let count = self.ctx.alloc_temp();
        let visited = self.ctx.alloc_temp();
        let one = self.ctx.alloc_temp();
        let args = self.ctx.registers.reserve(2);
        let cx_arg = Reg(args.0 + 1);
        if let Some((noop, from_waker)) = support {
            self.ctx.emit(Instruction::Call {
                dst: waker,
                func_id: noop,
                args: RegRange::new(args, 0),
            });
            self.ctx.emit(Instruction::RefObj {
                dst: args,
                src: waker,
            });
            self.ctx.emit(Instruction::Call {
                dst: context,
                func_id: from_waker,
                args: RegRange::new(args, 1),
            });
            self.ctx.emit(Instruction::RefMut {
                dst: cx_arg,
                src: context,
            });
        }
        self.ctx.emit(Instruction::LoadI {
            dst: count,
            value: arms.len() as i64,
        });
        self.ctx.emit(Instruction::LoadI { dst: one, value: 1 });
        self.ctx.emit(Instruction::LoadI {
            dst: visited,
            value: 0,
        });
        if biased {
            self.ctx.emit(Instruction::LoadI {
                dst: cursor,
                value: 0,
            });
        } else {
            // Randomize the first arm, then visit every arm exactly once per
            // sweep. Selection probability is not a deterministic latency bound.
            let mut operands = Vec::new();
            crate::encoding::encode_reg(cursor, &mut operands);
            self.ctx.emit(Instruction::SysExtended {
                sub_op: crate::instruction::SysSubOpcode::RandomFloat.to_byte(),
                operands,
            });
            self.ctx.emit(Instruction::LoadF {
                dst: polled,
                value: arms.len() as f64,
            });
            self.ctx.emit(Instruction::BinaryF {
                op: BinaryFloatOp::Mul,
                dst: cursor,
                a: cursor,
                b: polled,
            });
            self.ctx.emit(Instruction::CvtToI {
                dst: cursor,
                src: cursor,
            });
        }
        let dispatch = self.ctx.new_label("select_poll");
        let advance = self.ctx.new_label("select_advance");
        let end = self.ctx.new_label("select_end");
        let bodies: List<_> = arms
            .iter()
            .map(|_| self.ctx.new_label("select_body"))
            .collect();
        self.ctx.define_label(&dispatch);
        for (index, operand) in operands.iter().enumerate() {
            let Some(operand) = operand else {
                continue;
            };
            let next = self.ctx.new_label("select_next");
            self.ctx.emit(Instruction::LoadI {
                dst: ready,
                value: index as i64,
            });
            self.ctx.emit(Instruction::CmpI {
                op: CompareOp::Eq,
                dst: ready,
                a: cursor,
                b: ready,
            });
            self.ctx
                .emit_forward_jump(&next, |offset| Instruction::JmpNot {
                    cond: ready,
                    offset,
                });
            self.ctx
                .emit_forward_jump(&advance, |offset| Instruction::JmpNot {
                    cond: operand.enabled,
                    offset,
                });
            if let Some(poll) = &operand.poll {
                if operand.owned {
                    self.ctx.emit(Instruction::RefMut {
                        dst: args,
                        src: operand.future,
                    });
                } else {
                    self.ctx.emit(Instruction::Mov {
                        dst: args,
                        src: operand.future,
                    });
                }
                let call_args = RegRange::new(args, 2);
                if poll.type_args.is_empty() {
                    self.ctx.emit(Instruction::Call {
                        dst: polled,
                        func_id: poll.function,
                        args: call_args,
                    });
                } else {
                    self.ctx.emit(Instruction::CallG {
                        dst: polled,
                        func_id: poll.function,
                        args: call_args,
                        type_args: poll.type_args.to_vec(),
                    });
                }
                self.ctx.emit(Instruction::IsVar {
                    dst: ready,
                    value: polled,
                    tag: poll.ready_tag,
                });
                let completed = self.ctx.new_label("select_ready");
                self.ctx
                    .emit_forward_jump(&completed, |offset| Instruction::JmpIf {
                        cond: ready,
                        offset,
                    });
                self.assert_poll_pending(polled, ready, poll.pending_tag);
                self.ctx.emit(Instruction::DropRef { src: polled });
                self.ctx
                    .emit_forward_jump(&advance, |offset| Instruction::Jmp { offset });
                self.ctx.define_label(&completed);
                self.take_poll_output(polled, operand.result, ready);
            } else {
                self.ctx.emit(Instruction::FutureReady {
                    dst: ready,
                    future: operand.future,
                });
                self.ctx
                    .emit_forward_jump(&advance, |offset| Instruction::JmpNot {
                        cond: ready,
                        offset,
                    });
                self.ctx.emit(Instruction::FutureGet {
                    dst: operand.result,
                    future: operand.future,
                });
            }
            // Completed but mismatching operands cannot be polled again.
            self.ctx.emit(Instruction::LoadFalse {
                dst: operand.enabled,
            });
            if let Some(pattern) = &arms[index].pattern
                && !matches!(
                    &pattern.kind,
                    verum_ast::PatternKind::Wildcard
                        | verum_ast::PatternKind::Ident {
                            subpattern: None,
                            ..
                        }
                )
            {
                let previous = self.enter_scrutinee_type_for_bind(
                    arms[index].future.as_ref().unwrap(),
                    [pattern].into_iter(),
                );
                self.compile_pattern_test(pattern, operand.result, ready)?;
                self.exit_scrutinee_type_for_bind(previous);
                self.ctx
                    .emit_forward_jump(&bodies[index], |offset| Instruction::JmpIf {
                        cond: ready,
                        offset,
                    });
                let output_type = operand
                    .poll
                    .as_ref()
                    .map(|poll| poll.output_type.clone())
                    .ok_or_else(|| {
                        CodegenError::internal(
                            "refutable task selection requires an Output declaration",
                        )
                    })?;
                self.ctx.emit(Instruction::DropTyped {
                    src: operand.result,
                    type_ref: output_type,
                });
                self.ctx
                    .emit_forward_jump(&advance, |offset| Instruction::Jmp { offset });
            } else {
                self.ctx
                    .emit_forward_jump(&bodies[index], |offset| Instruction::Jmp { offset });
            }
            self.ctx.define_label(&next);
        }
        self.ctx.define_label(&advance);
        self.ctx.emit(Instruction::BinaryI {
            op: BinaryIntOp::Add,
            dst: cursor,
            a: cursor,
            b: one,
        });
        self.ctx.emit(Instruction::BinaryI {
            op: BinaryIntOp::Mod,
            dst: cursor,
            a: cursor,
            b: count,
        });
        self.ctx.emit(Instruction::BinaryI {
            op: BinaryIntOp::Add,
            dst: visited,
            a: visited,
            b: one,
        });
        self.ctx.emit(Instruction::CmpI {
            op: CompareOp::Lt,
            dst: ready,
            a: visited,
            b: count,
        });
        self.ctx
            .emit_backward_jump(&dispatch, |offset| Instruction::JmpIf {
                cond: ready,
                offset,
            })?;
        if let Some(index) = default {
            self.ctx
                .emit_forward_jump(&bodies[index], |offset| Instruction::Jmp { offset });
        } else {
            self.ctx.emit(Instruction::LoadI {
                dst: visited,
                value: 0,
            });
            self.ctx.emit(Instruction::LoadFalse { dst: ready });
            let resume = self.ctx.new_label("select_resume");
            for operand in operands.iter().flatten() {
                self.ctx
                    .emit_forward_jump(&resume, |offset| Instruction::JmpIf {
                        cond: operand.enabled,
                        offset,
                    });
            }
            self.cleanup_select_operands(&operands);
            let message = self
                .ctx
                .intern_string_raw("select has no enabled matching arms");
            self.ctx.emit(Instruction::Assert {
                cond: ready,
                message_id: message,
            });
            self.ctx.define_label(&resume);
            self.ctx.emit(Instruction::AsyncYield);
            self.ctx
                .emit_backward_jump(&dispatch, |offset| Instruction::Jmp { offset })?;
        }
        let mut result_type = Some(crate::types::TypeRef::Concrete(crate::types::TypeId::NEVER));
        for (index, arm) in arms.iter().enumerate() {
            self.ctx.define_label(&bodies[index]);
            // Finish temporary future lifetimes before the arm body, including
            // bodies that return, break or throw. Named operands stay borrowed.
            self.cleanup_select_operands(&operands);
            self.ctx.enter_scope();
            if let Some(pattern) = &arm.pattern {
                self.compile_pattern_bind_from_expr(
                    pattern,
                    operands[index].as_ref().unwrap().result,
                    arm.future.as_ref().unwrap(),
                )?;
            }
            let start = self.ctx.instructions.len();
            let value = self.compile_expr_with_demand(&arm.body, demand)?;
            result_type = super::super::place_copies::join_result_types(
                result_type,
                self.declared_expression_result(
                    &arm.body,
                    value,
                    start,
                    self.ctx.instructions.len(),
                ),
            );
            if let Some(value) = value {
                self.ctx.emit(Instruction::Mov {
                    dst: dest,
                    src: value,
                });
                self.ctx.free_temp(value);
            } else {
                self.ctx.emit(Instruction::LoadUnit { dst: dest });
            }
            let (_, defers) = self.ctx.exit_scope(false);
            for instructions in defers {
                for instruction in instructions {
                    self.ctx.emit(instruction);
                }
            }
            self.ctx.clear_active_pattern_cache();
            self.ctx
                .emit_forward_jump(&end, |offset| Instruction::Jmp { offset });
        }
        self.ctx.define_label(&end);
        self.ctx.declared_result_types.insert(span, result_type);
        for operand in operands.into_iter().flatten() {
            for reg in [
                operand.future,
                operand.enabled,
                operand.initialized,
                operand.result,
            ] {
                self.ctx.free_temp(reg);
            }
        }
        for reg in [
            waker, context, polled, ready, cursor, count, visited, one, args, cx_arg,
        ] {
            self.ctx.free_temp(reg);
        }
        Ok(Some(dest))
    }

    pub(super) fn assert_poll_pending(&mut self, polled: Reg, test: Reg, tag: u32) {
        self.ctx.emit(Instruction::IsVar {
            dst: test,
            value: polled,
            tag,
        });
        let message_id = self
            .ctx
            .intern_string_raw("Future.poll returned neither Ready nor Pending");
        self.ctx.emit(Instruction::Assert {
            cond: test,
            message_id,
        });
    }

    pub(super) fn take_poll_output(&mut self, polled: Reg, result: Reg, scratch: Reg) {
        self.ctx.emit(Instruction::GetVariantData {
            dst: result,
            variant: polled,
            field: 0,
        });
        // Move the output out before destroying its Poll carrier.
        self.ctx.emit(Instruction::LoadUnit { dst: scratch });
        self.ctx.emit(Instruction::SetVariantData {
            variant: polled,
            field: 0,
            value: scratch,
        });
        self.ctx.emit(Instruction::DropRef { src: polled });
    }

    fn cleanup_select_operands(&mut self, operands: &List<Maybe<Operand>>) {
        for operand in operands.iter().flatten().filter(|operand| operand.owned) {
            let skip = self.ctx.new_label("select_drop_skip");
            self.ctx
                .emit_forward_jump(&skip, |offset| Instruction::JmpNot {
                    cond: operand.initialized,
                    offset,
                });
            if let Some(poll) = &operand.poll {
                self.ctx.emit(Instruction::DropTyped {
                    src: operand.future,
                    type_ref: poll.future_type.clone(),
                });
            } else {
                self.ctx.emit(Instruction::DropRef {
                    src: operand.future,
                });
            }
            self.ctx.emit(Instruction::LoadFalse {
                dst: operand.initialized,
            });
            self.ctx.define_label(&skip);
        }
    }
}
