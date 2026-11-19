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

    /// Only an open nominal receiver and its actual empty allocation introduce
    /// a fresh element variable. An explicit type argument remains rigid.
    pub(super) fn unbound_list_allocation(
        &self,
        initializer: &Expr,
        output: Reg,
        start: usize,
    ) -> Option<crate::types::TypeId> {
        let ExprKind::MethodCall {
            receiver,
            type_args,
            ..
        } = &Self::normalized_place(initializer).kind
        else {
            return None;
        };
        let ExprKind::Path(path) = &receiver.kind else {
            return None;
        };
        let name = path.as_ident()?;
        if !type_args.is_empty() || self.is_value_receiver_binding(name.as_str()) {
            return None;
        }
        let crate::Instruction::NewList { dst, .. } = self.ctx.instructions.get(start..)?.last()?
        else {
            return None;
        };
        let owner = self.nominal_type_id(name.as_str())?;
        (*dst == output && self.declared_runtime_carrier(owner) == Some(WellKnownType::List))
            .then_some(owner)
    }

    /// Bind the local element from a selected collection parameter before the
    /// borrow is emitted. Subsequent slice calls can then select the List's own
    /// element declaration. A slice parameter cannot supply this evidence.
    pub(super) fn constrain_list_borrow(
        &mut self,
        argument: &Expr,
        expected: Option<&TypeRef>,
    ) -> CodegenResult<()> {
        let ExprKind::Unary {
            op: op @ (UnOp::Ref | UnOp::RefMut),
            expr,
        } = &Self::normalized_place(argument).kind
        else {
            return Ok(());
        };
        let Some(name) = Self::named_place_binding(expr) else {
            return Ok(());
        };
        let Some(TypeRef::Reference {
            inner,
            mutability,
            tier: CbgrTier::Tier0,
        }) = expected
            .cloned()
            .and_then(|ty| self.expand_declared_alias(ty))
        else {
            return Ok(());
        };
        let Some(list @ TypeRef::Instantiated { .. }) = self.expand_declared_alias(*inner) else {
            return Ok(());
        };
        let TypeRef::Instantiated { base, args } = &list else {
            unreachable!()
        };
        if args.len() != 1
            || self.declared_runtime_carrier(*base) != Some(WellKnownType::List)
            || !self.declared_type_in_scope(&list)
        {
            return Ok(());
        }
        let Some(binding) = self.ctx.lookup_var(name) else {
            return Ok(());
        };
        let invalid = |message: &str| {
            CodegenError::with_span(
                CodegenErrorKind::InvalidSourceSignature(message.into()),
                argument.span,
            )
        };
        if mutability == Mutability::Mutable && *op != UnOp::RefMut {
            return Err(invalid(
                "a mutable List parameter requires a mutable source borrow",
            ));
        }
        let reg = binding.reg;
        if let Some(owner) = binding.pending_list_element {
            if *op == UnOp::RefMut && !binding.is_mutable {
                return Err(invalid("a mutable List borrow requires a mutable local"));
            }
            if owner != *base {
                return Err(invalid(
                    "List borrow differs from its selected parameter declaration",
                ));
            }
            if !self.ctx.array_result_facts.is_fresh_list(reg) {
                // The source may have gained elements or escaped. This borrow
                // cannot independently establish their declaration.
                return Ok(());
            }
            self.ctx.lookup_var_mut(name).unwrap().pending_list_element = None;
            self.publish_resolved_binding_type(name, &list);
        } else if let Some(actual) = self.declared_named_binding_type(name, reg)
            && let Some(TypeRef::Instantiated {
                base: actual_owner, ..
            }) = self.expand_declared_alias(actual.clone())
            && self.declared_runtime_carrier(actual_owner) == Some(WellKnownType::List)
            && self.body_declaration_type(&actual).as_ref()
                != self.body_declaration_type(&list).as_ref()
        {
            return Err(invalid(
                "List borrow differs from its selected parameter declaration",
            ));
        }
        Ok(())
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
        if let ExprKind::Unary {
            op: UnOp::Ref | UnOp::RefMut,
            expr,
        } = &Self::normalized_place(argument).kind
            && let Some(name) = Self::named_place_binding(expr)
            && self
                .ctx
                .lookup_var(name)
                .is_some_and(|binding| binding.pending_list_element.is_some())
        {
            return Err(invalid(
                "slice argument requires its source element declaration",
            ));
        }
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
