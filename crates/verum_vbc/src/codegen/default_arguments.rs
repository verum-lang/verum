//! A serialized default is a value plus its selected parameter's carrier.
//! The constant pool alone cannot distinguish Bool from Int or narrow integers
//! from the declaration's wide representation.
use super::context::ConstantEntry;
use super::{CodegenError, CodegenErrorKind, CodegenResult, VbcCodegen};
use crate::module::{ConstId, ParamDescriptor};
use crate::types::{TypeId, TypeRef};
use crate::{Instruction, Reg};
use verum_ast::literal::IntSuffix;
use verum_ast::{ExprKind, FunctionParam, FunctionParamKind, LiteralKind, UnOp};
use verum_common::well_known_types::type_names;

#[derive(Clone, Copy)]
enum IntegerValue {
    Signed(i128),
    Unsigned(u128),
}

fn invalid(ty: &TypeRef) -> CodegenError {
    CodegenError::new(CodegenErrorKind::InvalidSourceSignature(format!(
        "constant default does not inhabit its declared carrier {ty:?}"
    )))
}

impl VbcCodegen {
    pub(super) fn param_default_const_id(
        &mut self,
        parameter: &FunctionParam,
        ty: &TypeRef,
    ) -> CodegenResult<Option<ConstId>> {
        let FunctionParamKind::Regular {
            default_value: Some(expression),
            ..
        } = &parameter.kind
        else {
            return Ok(None);
        };
        // Open/generic carriers retain their source expression. This stage has
        // no instantiated type authority with which to serialize a constant.
        let Some(TypeRef::Concrete(carrier)) = self.runtime_value_carrier(ty) else {
            return Ok(None);
        };
        let mut expression = expression;
        let mut negations = 0usize;
        let literal = loop {
            if negations > 128 {
                return Err(invalid(ty));
            }
            match &expression.kind {
                ExprKind::Literal(literal) => break literal,
                ExprKind::Paren(inner) => expression = inner,
                ExprKind::Unary {
                    op: UnOp::Neg,
                    expr,
                } => {
                    negations += 1;
                    expression = expr;
                }
                _ => return Ok(None),
            }
        };
        let negate = negations % 2 == 1;
        let value = match &literal.kind {
            LiteralKind::Int(integer)
                if !integer.suffix.as_ref().is_some_and(IntSuffix::is_custom) =>
            {
                // Parser integer nodes retain the nonnegative token magnitude
                // as a u128 bit pattern; unary minus is a separate AST node.
                // Materialized negative values use the same canonical shape.
                let mut value = IntegerValue::Unsigned(integer.value as u128);
                for _ in 0..negations {
                    value = match value {
                        IntegerValue::Signed(value) => {
                            IntegerValue::Signed(value.checked_neg().ok_or_else(|| invalid(ty))?)
                        }
                        IntegerValue::Unsigned(value) if value == 1u128 << 127 => {
                            IntegerValue::Signed(i128::MIN)
                        }
                        IntegerValue::Unsigned(value) if value <= i128::MAX as u128 => {
                            IntegerValue::Signed(-(value as i128))
                        }
                        _ => return Err(invalid(ty)),
                    };
                }
                Self::integer_default(value, carrier).ok_or_else(|| invalid(ty))?
            }
            LiteralKind::Bool(value) if negations == 0 && carrier == TypeId::BOOL => {
                ConstantEntry::Int(i64::from(*value))
            }
            LiteralKind::Float(value)
                if value
                    .suffix
                    .as_ref()
                    .is_some_and(|suffix| suffix.is_custom()) =>
            {
                return Ok(None);
            }
            LiteralKind::Float(value) if carrier.is_float() => {
                ConstantEntry::Float(if negate { -value.value } else { value.value })
            }
            LiteralKind::Text(value) if negations == 0 && carrier == TypeId::TEXT => {
                return Ok(Some(self.ctx.add_const_string(value.as_str())));
            }
            LiteralKind::Bool(_) | LiteralKind::Float(_) | LiteralKind::Text(_) => {
                return Err(invalid(ty));
            }
            // Aggregates, suffix-defined values and syntax-valued defaults use
            // their existing source lowering, never an invented constant kind.
            _ => return Ok(None),
        };
        Ok(Some(self.intern_default(value)))
    }

    fn intern_default(&mut self, value: ConstantEntry) -> ConstId {
        match value {
            ConstantEntry::Int(value) => self.ctx.add_const_int(value),
            ConstantEntry::Int128 { raw, signed } => self.ctx.add_const_i128(raw, signed),
            ConstantEntry::Float(value) => self.ctx.add_const_float(value),
            _ => unreachable!("native scalar default"),
        }
    }

    fn integer_default(value: IntegerValue, carrier: TypeId) -> Option<ConstantEntry> {
        if carrier == TypeId::I128 {
            let raw = match value {
                IntegerValue::Signed(value) => value as u128,
                IntegerValue::Unsigned(value) if value <= i128::MAX as u128 => value,
                _ => return None,
            };
            return Some(ConstantEntry::Int128 { raw, signed: true });
        }
        if carrier == TypeId::U128 {
            let raw = match value {
                IntegerValue::Signed(value) if value >= 0 => value as u128,
                IntegerValue::Unsigned(value) => value,
                _ => return None,
            };
            return Some(ConstantEntry::Int128 { raw, signed: false });
        }
        // PTR also represents raw pointers and unresolved types. Its ABI
        // number cannot establish an integer domain or signedness; a textual
        // type hint cannot restore that lost authority.
        let name = carrier.well_known_name()?;
        if !carrier.is_integer() {
            return None;
        }
        let minimum = type_names::integer_limit(name, "MIN")?;
        let maximum = type_names::integer_limit(name, "MAX")?;
        let value = match value {
            IntegerValue::Signed(value) => value,
            IntegerValue::Unsigned(value) if value <= i128::MAX as u128 => value as i128,
            _ => return None,
        };
        (minimum <= value && value <= maximum).then_some(ConstantEntry::Int(value as i64))
    }

    pub(super) fn load_parameter_default(
        &mut self,
        dst: Reg,
        parameter: &ParamDescriptor,
    ) -> CodegenResult<Instruction> {
        let id = parameter
            .default
            .ok_or_else(|| invalid(&parameter.type_ref))?;
        let value = self
            .ctx
            .constants
            .get(id.0 as usize)
            .cloned()
            .ok_or_else(|| invalid(&parameter.type_ref))?;
        let Some(TypeRef::Concrete(carrier)) = self.runtime_value_carrier(&parameter.type_ref)
        else {
            return Err(invalid(&parameter.type_ref));
        };
        if carrier == TypeId::BOOL {
            let ConstantEntry::Int(value @ (0 | 1)) = value else {
                return Err(invalid(&parameter.type_ref));
            };
            return Ok(
                self.load_scalar_constant(dst, verum_ast::checked_const::Scalar::Bool(value == 1))
            );
        }
        let normalized = match value {
            ConstantEntry::Int(value) => {
                // UInt64's established wire representation stores its exact
                // bit pattern in the signed 64-bit constant-pool slot.
                let value = if carrier == TypeId::U64 {
                    IntegerValue::Unsigned(value as u64 as u128)
                } else {
                    IntegerValue::Signed(value as i128)
                };
                Self::integer_default(value, carrier).ok_or_else(|| invalid(&parameter.type_ref))?
            }
            ConstantEntry::Int128 { raw, signed } => Self::integer_default(
                if signed {
                    IntegerValue::Signed(raw as i128)
                } else {
                    IntegerValue::Unsigned(raw)
                },
                carrier,
            )
            .ok_or_else(|| invalid(&parameter.type_ref))?,
            ConstantEntry::Float(_) if carrier.is_float() => {
                return Ok(Instruction::LoadK {
                    dst,
                    const_id: id.0,
                });
            }
            ConstantEntry::String(_) if carrier == TypeId::TEXT => {
                return Ok(Instruction::LoadK {
                    dst,
                    const_id: id.0,
                });
            }
            _ => return Err(invalid(&parameter.type_ref)),
        };
        Ok(Instruction::LoadK {
            dst,
            const_id: self.intern_default(normalized).0,
        })
    }
}
