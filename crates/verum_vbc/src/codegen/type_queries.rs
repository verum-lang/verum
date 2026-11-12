//! Type operands are declarations, never runtime values or guessed names.
use super::{CodegenError, CodegenResult, VbcCodegen};
use crate::{Instruction, Reg, intrinsics::registry::CodegenStrategy};
use verum_ast::Expr;

impl VbcCodegen {
    /// Shared by both intrinsic spellings, before ordinary operand evaluation.
    pub(super) fn emit_type_operand_intrinsic(
        &mut self,
        strategy: &CodegenStrategy,
        args: &[Expr],
        dst: Reg,
    ) -> CodegenResult<bool> {
        if !matches!(
            strategy,
            CodegenStrategy::TypeLayout(_) | CodegenStrategy::TargetHasAtomic
        ) {
            return Ok(false);
        }
        let [_, operand] = args else {
            return Err(CodegenError::internal(
                "declaration query requires one type operand",
            ));
        };
        let ty = self
            .declaration_property_operand_in_scope(
                operand,
                self.ctx.current_source_module.as_deref(),
                true,
            )
            .ok_or_else(|| {
                CodegenError::internal("declaration query requires a type, not a runtime value")
            })?;
        let mut count_error = None;
        let type_ref = self.resolve_type_ref_scoped_with_count(
            &ty,
            &self.ctx.current_generic_param_ids,
            true,
            self.ctx.current_source_module.as_deref(),
            &mut |count| match self.const_eval_i64(count) {
                Ok(value) => value.and_then(|value| u64::try_from(value).ok()),
                Err(error) => {
                    count_error = Some(error);
                    None
                }
            },
        );
        if let Some(error) = count_error {
            return Err(error);
        }
        let type_ref = type_ref?;
        let type_ref = self
            .resolve_declared_associated_types(&type_ref)
            .unwrap_or(type_ref);
        let instruction = match strategy {
            CodegenStrategy::TypeLayout(property) => Instruction::TypeLayout {
                dst,
                type_ref,
                property: *property,
            },
            CodegenStrategy::TargetHasAtomic => Instruction::TargetHasAtomic { dst, type_ref },
            _ => unreachable!("selected above"),
        };
        self.ctx.emit(instruction);
        Ok(true)
    }
}
