//! Inline assembly code generation for LLVM.
//!
//! This module handles lowering of Verum `@asm` expressions to LLVM inline assembly.
//!
//! # Example Usage
//!
//! ```verum
//! let result: Int = @asm(
//!     "mov {0}, {1}",
//!     out("r") result,
//!     in("r") input,
//!     options(pure, nomem)
//! );
//! ```
//!
//! # LLVM Inline Assembly
//!
//! LLVM inline assembly uses a constraint-based system:
//! - `r` = general purpose register
//! - `m` = memory operand
//! - `i` = immediate integer
//! - `=r` = output to register
//! - `+r` = read-write register
//!
//! Verum inline assembly uses `@asm(template, operands..., options(...))` syntax
//! with full type safety. Operands use constraint-based system:
//! - `in("reg") expr` for input operands
//! - `out("reg") var` for output operands
//! - `inout("reg") var` for read-write operands
//! - `sym symbol` for symbolic references
//! Options include: nostack, nomem, preserves_flags, noreturn, att_syntax, raw.
//! LLVM constraint letters: r=register, m=memory, i=immediate, =r=output.

use verum_ast::expr::{AsmOperand, AsmOperandKind, AsmOptions};
use verum_common::{List, Text};
use verum_llvm::context::Context;
use verum_llvm::types::{BasicType, BasicTypeEnum, FunctionType};
use verum_llvm::values::{BasicValue, BasicValueEnum, PointerValue};
use verum_llvm::InlineAsmDialect;

use super::error::{LlvmLoweringError, Result};

/// Inline assembly code generator.
///
/// Translates Verum @asm expressions to LLVM inline assembly calls.
pub struct InlineAsmGenerator<'ctx> {
    context: &'ctx Context,
}

impl<'ctx> InlineAsmGenerator<'ctx> {
    /// Create a new inline assembly generator.
    pub fn new(context: &'ctx Context) -> Self {
        Self { context }
    }

    /// Generate LLVM inline assembly from AST operands.
    ///
    /// Returns the PointerValue representing the inline assembly function,
    /// along with the input values that should be passed to it.
    pub fn generate(
        &self,
        template: &Text,
        operands: &List<AsmOperand>,
        options: &AsmOptions,
        get_value: impl Fn(&verum_ast::Expr) -> Result<BasicValueEnum<'ctx>>,
        get_output_ptr: impl Fn(&verum_ast::Expr) -> Result<PointerValue<'ctx>>,
    ) -> Result<AsmCall<'ctx>> {
        // Collect inputs, outputs, and clobbers
        let mut inputs: Vec<(Text, BasicValueEnum<'ctx>)> = Vec::new();
        let mut outputs: Vec<(Text, PointerValue<'ctx>)> = Vec::new();
        let mut clobbers: Vec<Text> = Vec::new();

        for operand in operands.iter() {
            match &operand.kind {
                AsmOperandKind::In { constraint, expr } => {
                    let value = get_value(expr)?;
                    inputs.push((constraint.constraint.clone(), value));
                }
                AsmOperandKind::Out { constraint, place, late: _ } => {
                    let ptr = get_output_ptr(place)?;
                    outputs.push((format_output_constraint(&constraint.constraint), ptr));
                }
                AsmOperandKind::InOut { constraint, place } => {
                    // InOut uses '+' constraint modifier
                    let value = get_value(place)?;
                    let ptr = get_output_ptr(place)?;
                    let inout_constraint = format!("+{}", constraint.constraint.trim_start_matches('+'));
                    inputs.push((inout_constraint.into(), value));
                    outputs.push((format_output_constraint(&constraint.constraint), ptr));
                }
                AsmOperandKind::InLateOut { constraint, in_expr, out_place } => {
                    let value = get_value(in_expr)?;
                    let ptr = get_output_ptr(out_place)?;
                    inputs.push((constraint.constraint.clone(), value));
                    outputs.push((format_output_constraint(&constraint.constraint), ptr));
                }
                AsmOperandKind::Sym { path: _ } => {
                    // Symbol operands are handled as immediate addresses
                    // For now, we skip them (would need symbol resolution)
                }
                AsmOperandKind::Const { expr } => {
                    let value = get_value(expr)?;
                    inputs.push(("i".into(), value)); // 'i' for immediate
                }
                AsmOperandKind::Clobber { reg } => {
                    clobbers.push(reg.clone());
                }
            }
        }

        // Build constraint string
        // Format: "output1,output2,...:input1,input2,...:~clobber1,~clobber2,..."
        let mut constraints = Vec::new();

        // Outputs first (with '=' prefix for pure outputs)
        for (constraint, _) in &outputs {
            constraints.push(constraint.clone());
        }

        // Then inputs
        for (constraint, _) in &inputs {
            constraints.push(constraint.clone());
        }

        // Then clobbers (prefixed with '~')
        for clobber in &clobbers {
            constraints.push(format!("~{{{}}}", clobber).into());
        }

        // Add memory clobber if not pure
        if !options.pure_asm && !options.nomem {
            constraints.push("~{memory}".into());
        }

        let constraint_str: String = constraints
            .iter()
            .map(|c| c.as_str())
            .collect::<Vec<_>>()
            .join(",");

        // Build function type
        // We always return void and write outputs to pointers
        let input_types: Vec<BasicTypeEnum<'ctx>> = inputs
            .iter()
            .map(|(_, v)| v.get_type())
            .collect();

        let fn_type = self.context.void_type().fn_type(
            &input_types.iter().map(|t| (*t).into()).collect::<Vec<_>>(),
            false,
        );

        // Determine dialect
        let dialect = if options.intel_syntax {
            Some(InlineAsmDialect::Intel)
        } else {
            Some(InlineAsmDialect::ATT)
        };

        // Create inline assembly
        let sideeffects = options.volatile || !options.pure_asm;
        let alignstack = !options.nostack;
        let can_throw = options.may_unwind;

        let asm = self.context.create_inline_asm(
            fn_type,
            template.to_string(),
            constraint_str,
            sideeffects,
            alignstack,
            dialect,
            can_throw,
        );

        Ok(AsmCall {
            asm_fn: asm,
            fn_type,
            inputs: inputs.into_iter().map(|(_, v)| v).collect(),
            outputs,
        })
    }
}

/// Result of generating inline assembly.
pub struct AsmCall<'ctx> {
    /// The inline assembly function pointer.
    pub asm_fn: PointerValue<'ctx>,
    /// The function type.
    pub fn_type: FunctionType<'ctx>,
    /// Input values to pass to the assembly.
    pub inputs: Vec<BasicValueEnum<'ctx>>,
    /// Output pointers to write results to.
    pub outputs: Vec<(Text, PointerValue<'ctx>)>,
}

/// Format a constraint as an output constraint.
fn format_output_constraint(constraint: &Text) -> Text {
    let s = constraint.as_str();
    if s.starts_with('=') || s.starts_with('+') {
        constraint.clone()
    } else {
        format!("={}", s).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_output_constraint() {
        assert_eq!(format_output_constraint(&"r".into()).as_str(), "=r");
        assert_eq!(format_output_constraint(&"=r".into()).as_str(), "=r");
        assert_eq!(format_output_constraint(&"+r".into()).as_str(), "+r");
    }
}
