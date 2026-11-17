//! Selected slice formals and generic witnesses share one declaration contract.
//! Materialization reuses explicit range-borrow lowering and its storage facts.
use super::{CodegenError, CodegenErrorKind, CodegenResult, VbcCodegen};
use crate::{
    instruction::Reg,
    types::{CbgrTier, Mutability, TypeRef},
};
use verum_ast::{Expr, ExprKind, UnOp};
use verum_common::{Heap, well_known_types::WellKnownType};

impl VbcCodegen {
    pub(super) fn slice_parameter_element(&self, ty: &TypeRef) -> Option<TypeRef> {
        let TypeRef::Reference {
            inner,
            tier: CbgrTier::Tier0,
            ..
        } = self.expand_declared_alias(ty.clone())?
        else {
            return None;
        };
        let TypeRef::Slice(element) = self.expand_declared_alias(*inner)? else {
            return None;
        };
        Some(*element)
    }

    /// The source supplies the element witness. The formal selects the borrow
    /// operation, never an element type, storage permission or write capability.
    pub(super) fn declared_slice_argument_type(
        &self,
        formal: &TypeRef,
        actual: &TypeRef,
    ) -> Option<TypeRef> {
        let TypeRef::Reference {
            inner,
            mutability: wanted,
            tier: CbgrTier::Tier0,
        } = self.expand_declared_alias(formal.clone())?
        else {
            return None;
        };
        if !matches!(self.expand_declared_alias(*inner)?, TypeRef::Slice(_)) {
            return None;
        }
        let TypeRef::Reference {
            inner,
            mutability,
            tier: CbgrTier::Tier0,
        } = self.expand_declared_alias(actual.clone())?
        else {
            return None;
        };
        if wanted == Mutability::Mutable && mutability != Mutability::Mutable {
            return None;
        }
        let element = match self.expand_declared_alias(*inner)? {
            TypeRef::Array { element, .. } | TypeRef::Slice(element) => *element,
            TypeRef::Instantiated { base, args }
                if self.declared_runtime_carrier(base) == Some(WellKnownType::List) =>
            {
                let [element] = args.as_slice() else {
                    return None;
                };
                element.clone()
            }
            _ => return None,
        };
        if !self.declared_type_in_scope(&element) {
            return None;
        }
        Some(TypeRef::Reference {
            inner: Heap::new(TypeRef::Slice(Heap::new(element))),
            mutability,
            tier: CbgrTier::Tier0,
        })
    }

    pub(super) fn compile_borrowed_slice_argument(
        &mut self,
        argument: &Expr,
        expected: Option<&TypeRef>,
    ) -> CodegenResult<Option<Reg>> {
        let Some(expected) = expected else {
            return Ok(None);
        };
        let Some(element) = self.slice_parameter_element(expected) else {
            return Ok(None);
        };
        let invalid = |message: &str| {
            CodegenError::with_span(
                CodegenErrorKind::InvalidSourceSignature(message.into()),
                argument.span,
            )
        };
        let actual = self
            .source_expression_declaration(argument, None, 0)
            .and_then(|ty| self.expand_declared_alias(ty))
            .ok_or_else(|| invalid("slice argument requires its source element declaration"))?;
        let TypeRef::Reference {
            inner,
            mutability: supplied,
            ..
        } = &actual
        else {
            return Err(invalid("slice argument requires an explicit source borrow"));
        };
        let TypeRef::Reference {
            mutability: wanted, ..
        } = self.expand_declared_alias(expected.clone()).unwrap()
        else {
            unreachable!()
        };
        if wanted == Mutability::Mutable && *supplied != Mutability::Mutable {
            return Err(invalid(
                "a mutable slice parameter requires a mutable source borrow",
            ));
        }
        let adapted = self
            .declared_slice_argument_type(expected, &actual)
            .ok_or_else(|| invalid("slice argument has no supported collection borrow"))?;
        let source_element = self.slice_parameter_element(&adapted).unwrap();
        let element = self.expand_declared_alias(element);
        if element.is_none() || self.expand_declared_alias(source_element) != element {
            return Err(invalid(
                "slice argument element differs from its selected parameter declaration",
            ));
        }
        let source_type = self
            .expand_declared_alias((**inner).clone())
            .ok_or_else(|| invalid("slice argument requires its source declaration"))?;
        if matches!(source_type, TypeRef::Slice(_)) {
            return Ok(None);
        }

        // A direct borrow uses the original place. A saved List reference must
        // load its borrowed descriptor; a saved array borrow can already be a
        // slice descriptor and the range producer preserves that representation.
        let (source, end) = match &Self::normalized_place(argument).kind {
            ExprKind::Unary {
                op: UnOp::Ref | UnOp::RefMut,
                expr,
            } => ((**expr).clone(), None),
            _ => match source_type {
                TypeRef::Array { length, .. } => (
                    argument.clone(),
                    Some(Heap::new(Expr::new(
                        ExprKind::Literal(verum_ast::Literal::int(
                            i128::from(length),
                            argument.span,
                        )),
                        argument.span,
                    ))),
                ),
                _ => (
                    Expr::new(
                        ExprKind::Unary {
                            op: UnOp::Deref,
                            expr: Heap::new(argument.clone()),
                        },
                        argument.span,
                    ),
                    None,
                ),
            },
        };
        let range = Expr::new(
            ExprKind::Range {
                start: None,
                end,
                inclusive: false,
            },
            source.span,
        );
        let indexed = Expr::new(
            ExprKind::Index {
                expr: Heap::new(source),
                index: Heap::new(range),
            },
            argument.span,
        );
        let borrowed = Expr::new(
            ExprKind::Unary {
                op: if wanted == Mutability::Mutable {
                    UnOp::RefMut
                } else {
                    UnOp::Ref
                },
                expr: Heap::new(indexed),
            },
            argument.span,
        );
        self.compile_expr(&borrowed)
    }
}
