//! Evaluate closed generic value arguments in the constant's lexical scope.
use super::{CodegenError, CodegenErrorKind, CodegenResult, FunctionInfo, VbcCodegen};
use crate::{
    const_sequence::{ConstSequence, sequence_domain},
    module::FunctionId,
    types::{TypeId, TypeRef},
};
use verum_ast::{ArrayExpr, Expr, ExprKind, Span, checked_const::Scalar};
use verum_common::const_value::{MAX_SEQUENCE_DEPTH, MAX_SEQUENCE_ELEMENTS};
use verum_common::{Map, Set, Text};

#[derive(Clone, Copy)]
struct Scope<'a> {
    owner: Option<&'a str>,
    bindings: Option<&'a Map<Text, FunctionInfo>>,
    at_call: bool,
}

struct Constants<'a> {
    cg: &'a VbcCodegen,
    active: Set<FunctionId>,
    scalars: Map<FunctionId, Scalar>,
    remaining: usize,
}

fn invalid(span: Span, message: impl Into<String>) -> CodegenError {
    CodegenError::with_span(CodegenErrorKind::TypeInference(message.into()), span)
}

impl VbcCodegen {
    pub(super) fn closed_value_argument(
        &self,
        expression: &Expr,
        domain: &TypeRef,
    ) -> CodegenResult<TypeRef> {
        let scope = Scope {
            owner: self.current_count_module_scope(),
            bindings: None,
            at_call: true,
        };
        let mut constants = Constants {
            cg: self,
            active: Set::new(),
            scalars: Map::new(),
            remaining: MAX_SEQUENCE_ELEMENTS,
        };
        if *domain == TypeRef::Concrete(TypeId::INT) {
            constants
                .integer(expression, scope)
                .map(TypeRef::ConstValue)
        } else {
            constants
                .sequence(expression, domain, scope, 0)
                .map(TypeRef::ConstSequence)
        }
    }
}

impl Constants<'_> {
    fn declaration(
        &self,
        expr: &Expr,
        scope: Scope<'_>,
        expected: &TypeRef,
    ) -> Option<FunctionInfo> {
        let name = VbcCodegen::constant_integer_name(expr)?;
        let info =
            self.cg
                .scoped_constant_binding(&name, scope.owner, scope.bindings, scope.at_call)?;
        (self
            .cg
            .runtime_value_carrier(info.return_type.as_ref()?)
            .as_ref()
            == Some(expected))
        .then_some(info)
    }

    fn integer(&mut self, expr: &Expr, scope: Scope<'_>) -> CodegenResult<i64> {
        super::source_substitution::checked_scalar_expression(expr, &mut |leaf| {
            if self
                .declaration(leaf, scope, &TypeRef::Concrete(TypeId::INT))
                .is_none()
            {
                return Ok(None);
            }
            self.cg.const_eval_i64_in_scope(
                leaf,
                scope.owner,
                scope.bindings,
                scope.at_call,
                &mut self.active,
                &mut self.scalars,
                0,
            )
        })
    }

    fn charge(&mut self, count: usize, span: Span) -> CodegenResult<()> {
        self.remaining = self.remaining.checked_sub(count).ok_or_else(|| {
            invalid(
                span,
                format!("constant sequence exceeds {MAX_SEQUENCE_ELEMENTS} elements"),
            )
        })?;
        Ok(())
    }

    fn sequence(
        &mut self,
        expr: &Expr,
        domain: &TypeRef,
        scope: Scope<'_>,
        depth: usize,
    ) -> CodegenResult<ConstSequence> {
        if depth >= MAX_SEQUENCE_DEPTH {
            return Err(invalid(expr.span, "constant sequence nesting is too deep"));
        }
        let expr = VbcCodegen::normalized_place(expr);
        let ExprKind::Array(array) = &expr.kind else {
            let info = self.declaration(expr, scope, domain).ok_or_else(|| {
                invalid(
                    expr.span,
                    "expected a constant sequence of the declared type",
                )
            })?;
            let (init, owner, bindings) =
                self.cg.constant_initializers.get(&info.id).ok_or_else(|| {
                    invalid(expr.span, "sequence constant has no source initializer")
                })?;
            if !self.active.insert(info.id) {
                return Err(invalid(expr.span, "cyclic sequence constant"));
            }
            let result = self.sequence(
                init,
                domain,
                Scope {
                    owner: owner.as_deref(),
                    bindings: Some(bindings),
                    at_call: false,
                },
                depth + 1,
            );
            self.active.remove(&info.id);
            return result;
        };
        let (element, extent) = sequence_domain(domain)
            .ok_or_else(|| invalid(expr.span, "expected a sequence value parameter"))?;
        let count = match array {
            ArrayExpr::List(values) => values.len(),
            ArrayExpr::Repeat { count, .. } => usize::try_from(self.integer(count, scope)?)
                .map_err(|_| invalid(count.span, "sequence repeat count must be nonnegative"))?,
        };
        if extent.is_some_and(|n| n != count as u64) {
            return Err(invalid(
                expr.span,
                format!(
                    "constant sequence length {count} differs from the declared extent {}",
                    extent.unwrap()
                ),
            ));
        }
        self.charge(count, expr.span)?;
        if *element == TypeRef::Concrete(TypeId::INT) {
            return self
                .elements(array, count, |this, item| this.integer(item, scope))
                .map(|values| ConstSequence::Integers(values.into()));
        }
        self.elements(array, count, |this, item| {
            this.sequence(item, element, scope, depth + 1)
        })
        .map(|values| ConstSequence::Arrays(values.into()))
    }

    fn elements<T: Clone>(
        &mut self,
        array: &ArrayExpr,
        count: usize,
        mut evaluate: impl FnMut(&mut Self, &Expr) -> CodegenResult<T>,
    ) -> CodegenResult<Vec<T>> {
        match array {
            ArrayExpr::List(items) => items.iter().map(|item| evaluate(self, item)).collect(),
            ArrayExpr::Repeat { value, .. } => {
                let before = self.remaining;
                // Even a zero-length repeat has a typed element expression.
                let item = evaluate(self, value)?;
                let cost = before - self.remaining;
                self.remaining = before;
                let total = cost
                    .checked_mul(count)
                    .ok_or_else(|| invalid(value.span, "sequence extent overflow"))?;
                self.charge(total, value.span)?;
                Ok(vec![item; count])
            }
        }
    }
}
