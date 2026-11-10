//! Computational bounds come from the callable actually selected by inference.
use super::*;
use crate::computational_properties::{ComputationalProperty, PropertyRequirement, PropertySet};
use verum_ast::visitor::{Visitor, walk_expr};

struct Calls<'a> {
    checker: &'a TypeChecker,
    bound: PropertyRequirement,
}
impl Visitor for Calls<'_> {
    fn visit_expr(&mut self, expression: &Expr) {
        match &expression.kind {
            ExprKind::Closure { .. } => return,
            ExprKind::Call { .. } | ExprKind::MethodCall { .. } => {
                let target = self
                    .checker
                    .type_registry
                    .lookup_call_target(expression.span)
                    .map(|ty| self.checker.unifier.apply(ty));
                let requirement = match target.as_ref() {
                    Some(Type::Function {
                        properties: Some(known),
                        ..
                    }) => PropertyRequirement::AtMost(known.clone()),
                    Some(Type::Function {
                        property_requirement,
                        ..
                    }) => property_requirement.clone(),
                    _ => PropertyRequirement::Unrestricted,
                };
                self.bound = self.bound.join(&requirement);
            }
            _ => (),
        }
        walk_expr(self, expression);
    }
    fn visit_item(&mut self, _: &verum_ast::Item) {}
}

impl TypeChecker {
    pub(super) fn callable_body_bound(&mut self, body: &Expr) -> PropertyRequirement {
        let local = self.property_inferrer.infer_local_operations(body);
        let mut calls = Calls {
            checker: self,
            bound: PropertyRequirement::AtMost(local),
        };
        calls.visit_expr(body);
        calls.bound
    }

    pub(super) fn callable_function_body_bound(
        &mut self,
        body: &verum_ast::FunctionBody,
    ) -> PropertyRequirement {
        let local = self.property_inferrer.infer_function_body(body);
        let mut calls = Calls {
            checker: self,
            bound: PropertyRequirement::AtMost(local),
        };
        match body {
            verum_ast::FunctionBody::Block(block) => calls.visit_block(block),
            verum_ast::FunctionBody::Expr(expr) => calls.visit_expr(expr),
        }
        calls.bound
    }

    /// A concrete source method can establish synchrony before its signature
    /// is used as a callback. Unknown callees keep the body's bound open.
    pub(super) fn concrete_method_requirement(
        &mut self,
        method: &verum_ast::FunctionDecl,
    ) -> PropertyRequirement {
        let declared = PropertyRequirement::declared_function(method.is_pure, method.is_meta);
        if !method.is_async && declared.may_be_async()
            && let Some(body) = &method.body
            && !self.callable_function_body_bound(body).may_be_async()
        {
            return PropertyRequirement::source_callable(false);
        }
        declared
    }

    pub(super) fn callable_bound_with_marker(
        bound: PropertyRequirement,
        is_async: bool,
    ) -> PropertyRequirement {
        if is_async {
            bound.join(&PropertyRequirement::AtMost(PropertySet::single(
                ComputationalProperty::Async,
            )))
        } else {
            bound
        }
    }
}
