//! Lower the conversion of a known Try residual through its declared FromResidual member.
use super::{CodegenError, CodegenErrorKind, CodegenResult, VbcCodegen};
use crate::{
    Instruction,
    instruction::{Reg, RegRange},
    module::FunctionId,
    mono::TypeSubstitution,
    types::TypeRef,
};
use verum_ast::Span;

#[derive(PartialEq)]
pub(super) struct ResidualConversion {
    method: FunctionId,
    arguments: Vec<TypeRef>,
}

impl VbcCodegen {
    pub(super) fn emit_residual_conversion(
        &mut self,
        conversion: ResidualConversion,
        source: Reg,
    ) -> Reg {
        let dst = self.ctx.alloc_temp();
        let args = RegRange::new(source, 1);
        let func_id = conversion.method.0;
        self.ctx.emit(if conversion.arguments.is_empty() {
            Instruction::Call { dst, func_id, args }
        } else {
            Instruction::CallG {
                dst,
                func_id,
                args,
                type_args: conversion.arguments,
            }
        });
        dst
    }

    /// The failure arm carries Try.Residual, even though its register is shared with
    /// the success arm. Bind the conversion against the residual and the function
    /// result together, in the declaration's generic parameter order.
    pub(super) fn declared_residual_conversion(
        &self,
        input: &TypeRef,
        output: &TypeRef,
        span: Span,
    ) -> CodegenResult<Option<ResidualConversion>> {
        let Some(try_id) = self.nominal_type_id("core.base.ops.Try") else {
            return Ok(None);
        };
        let Some(from_id) = self.nominal_type_id("core.base.ops.FromResidual") else {
            return Ok(None);
        };
        let TypeRef::Instantiated { base, args } = input else {
            return Ok(None);
        };
        let Some(carrier) = self.type_by_id(*base) else {
            return Ok(None);
        };
        let Some(outer) = output.base_type_id().and_then(|id| self.type_by_id(id)) else {
            return Ok(None);
        };
        let residual = self
            .associated_binding(
                carrier
                    .protocols
                    .iter()
                    .filter(|implementation| implementation.protocol.0 == try_id.0),
                "Residual",
            )
            .ok_or_else(|| invalid("Try has no unique declared Residual on this receiver", span))?;
        if carrier.type_params.len() != args.len() {
            return Err(invalid(
                "Try receiver has no complete declaration application",
                span,
            ));
        }
        let residual = TypeSubstitution::new(&carrier.type_params, args)
            .apply(residual)
            .map_err(|error| invalid(error.to_string(), span))?;
        let slot = self
            .type_by_id(from_id)
            .and_then(|protocol| {
                protocol.variants.iter().find(|member| {
                    self.ctx
                        .strings
                        .get(member.name.0 as usize)
                        .is_some_and(|name| name == "from_residual")
                })
            })
            .ok_or_else(|| invalid("FromResidual has no declared from_residual member", span))?
            .tag as usize;
        let methods = outer
            .protocols
            .iter()
            .filter(|implementation| implementation.protocol.0 == from_id.0)
            .filter_map(|implementation| implementation.methods.get(slot).copied().map(FunctionId));
        let selected = self.residual_method(methods, &residual, output, span)?;
        selected.map(Some).ok_or_else(|| {
            let source = self
                .declaration_type_name(&residual, 0)
                .unwrap_or_else(|| format!("{residual:?}"));
            let target = self
                .declaration_type_name(output, 0)
                .unwrap_or_else(|| format!("{output:?}"));
            invalid(
                format!("no declared FromResidual converts {source} into {target}"),
                span,
            )
        })
    }

    /// Stdlib-free source units may declare the concrete conversion directly.
    /// Match the source and target, including their applied arguments, exactly.
    pub(super) fn declared_error_conversion(
        &self,
        input: &TypeRef,
        output: &TypeRef,
        span: Span,
    ) -> CodegenResult<Option<ResidualConversion>> {
        let Some(owner) = output
            .base_type_id()
            .and_then(|id| self.type_by_id(id))
            .and_then(|ty| self.type_declaration_identity(ty))
        else {
            return Ok(None);
        };
        let family = format!("{owner}.from");
        let registered = self
            .impl_method_registrations
            .get(&family)
            .into_iter()
            .flatten()
            .map(|method| method.id);
        let primary = self.ctx.lookup_function(&family).map(|method| method.id);
        self.residual_method(registered.chain(primary), input, output, span)
    }

    fn residual_method(
        &self,
        methods: impl Iterator<Item = FunctionId>,
        input: &TypeRef,
        output: &TypeRef,
        span: Span,
    ) -> CodegenResult<Option<ResidualConversion>> {
        let mut selected = None;
        for id in methods {
            let Some(method) = self.ctx.lookup_function_by_id(id) else {
                continue;
            };
            let Some(TypeRef::Function {
                params,
                return_type,
                ..
            }) = method.callable_signature.as_ref()
            else {
                continue;
            };
            let [parameter] = params.as_slice() else {
                continue;
            };
            let mut bindings = TypeSubstitution::empty();
            if !self.bind_source_declaration(return_type, output, &mut bindings)
                || !self.bind_source_declaration(parameter, input, &mut bindings)
            {
                continue;
            }
            let arguments = method
                .type_param_ids
                .iter()
                .map(|id| bindings.get(*id).cloned())
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| {
                    invalid(
                        "residual conversion has an unresolved declaration witness",
                        span,
                    )
                })?;
            let candidate = ResidualConversion {
                method: id,
                arguments,
            };
            if selected
                .as_ref()
                .is_some_and(|previous| previous != &candidate)
            {
                return Err(invalid(
                    "multiple conversion declarations match the residual and result",
                    span,
                ));
            }
            selected = Some(candidate);
        }
        Ok(selected)
    }
}

fn invalid(message: impl Into<String>, span: Span) -> CodegenError {
    CodegenError::with_span(
        CodegenErrorKind::InvalidSourceSignature(message.into()),
        span,
    )
}
