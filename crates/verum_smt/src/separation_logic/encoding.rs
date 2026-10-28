//! Exact spatial encoding. An unsupported expression is an error, never `true`
//! or a fresh value unrelated to the source expression.
use super::*;
use z3::ast::{Ast, BV};

impl SepLogicEncoder {
    /// Encode an assertion over 64-bit addresses and values. Pure propositions
    /// do not constrain the domain; points-to owns an exact singleton domain.
    /// Named predicates remain opaque relations of their full heap and arguments.
    pub fn encode_assertion(
        &self,
        assertion: &SepAssertion,
        heap: &Array,
        allocated: &Array,
    ) -> Result<Bool, SepLogicError> {
        if heap.get_sort() != Sort::array(&self.model.addr_sort, &self.model.value_sort)
            || allocated.get_sort() != Sort::array(&self.model.addr_sort, &Sort::bool())
        {
            return Err(SepLogicError::EncodingError(Text::from(
                "separation heaps require word64 addresses and values with a Boolean allocation map",
            )));
        }
        Ok(match assertion {
            SepAssertion::Emp => allocated.eq(&Array::const_array(
                &self.model.addr_sort,
                &Bool::from_bool(false),
            )),
            SepAssertion::Pure(expr) => self.encode_pure_expr(expr)?,
            SepAssertion::PointsTo { location, value } => {
                let address = self.encode_expr_as_addr(location)?;
                let value = self.encode_expr_as_value(value)?;
                let singleton = Array::const_array(&self.model.addr_sort, &Bool::from_bool(false))
                    .store(&address, &Bool::from_bool(true));
                Bool::and(&[&allocated.eq(&singleton), &heap.select(&address).eq(&value)])
            }
            SepAssertion::Sep { left, right } => {
                // Subheaps share values and split only the owned domain.
                let a = self.model.fresh_allocated("left_domain");
                let b = self.model.fresh_allocated("right_domain");
                let left = self.encode_assertion(left, heap, &a)?;
                let right = self.encode_assertion(right, heap, &b)?;
                let address = self.model.fresh_addr("partition");
                let av = a
                    .select(&address)
                    .as_bool()
                    .expect("Boolean allocation map");
                let bv = b
                    .select(&address)
                    .as_bool()
                    .expect("Boolean allocation map");
                let partition = Bool::and(&[
                    &allocated.select(&address).eq(&Bool::or(&[&av, &bv])),
                    &Bool::and(&[&av, &bv]).not(),
                ]);
                let partition = z3::ast::forall_const(&[&address], &[], &partition);
                let body = Bool::and(&[&left, &right, &partition]);
                z3::ast::exists_const(&[&a, &b], &[], &body)
            }
            SepAssertion::And { left, right } | SepAssertion::Or { left, right } => {
                let left = self.encode_assertion(left, heap, allocated)?;
                let right = self.encode_assertion(right, heap, allocated)?;
                if matches!(assertion, SepAssertion::And { .. }) {
                    Bool::and(&[&left, &right])
                } else {
                    Bool::or(&[&left, &right])
                }
            }
            SepAssertion::Exists { var, body } | SepAssertion::Forall { var, body } => {
                let variable = self.model.fresh_value(var.as_str());
                let previous = self
                    .bindings
                    .borrow_mut()
                    .insert(var.clone(), variable.clone());
                let body = self.encode_assertion(body, heap, allocated);
                // Restore scope even when the body cannot be encoded.
                self.restore_binding(var, previous);
                let body = body?;
                if matches!(assertion, SepAssertion::Exists { .. }) {
                    z3::ast::exists_const(&[&variable], &[], &body)
                } else {
                    z3::ast::forall_const(&[&variable], &[], &body)
                }
            }
            SepAssertion::Wand { left, right } => {
                let frame_heap = self.model.fresh_heap("wand_frame");
                let frame_domain = self.model.fresh_allocated("wand_frame_domain");
                let combined_heap = self.model.fresh_heap("wand_union");
                let combined_domain = self.model.fresh_allocated("wand_union_domain");
                let left = self.encode_assertion(left, &frame_heap, &frame_domain)?;
                let right = self.encode_assertion(right, &combined_heap, &combined_domain)?;
                let disjoint = self.encode_disjointness(allocated, &frame_domain);
                let union = self.encode_heap_composition(
                    &combined_heap,
                    &combined_domain,
                    heap,
                    allocated,
                    &frame_heap,
                    &frame_domain,
                );
                let body = Bool::and(&[&left, &disjoint, &union]).implies(&right);
                z3::ast::forall_const(
                    &[&frame_heap, &frame_domain, &combined_heap, &combined_domain],
                    &[],
                    &body,
                )
            }
            SepAssertion::Named { name, args } => {
                // A predicate of a partial heap cannot observe values outside
                // its owned domain. Canonicalize those values before passing
                // the heap to the opaque relation.
                let restricted = self.canonical_heap(heap, allocated);
                let mut values = [Dynamic::from_ast(&restricted), Dynamic::from_ast(allocated)]
                    .into_iter()
                    .collect::<List<_>>();
                for arg in args {
                    values.push(self.encode_expr_as_value(arg)?);
                }
                let sorts: List<Sort> = values.iter().map(Ast::get_sort).collect();
                let sort_refs: List<&Sort> = sorts.iter().collect();
                let arguments: List<&dyn Ast> = values.iter().map(|a| a as &dyn Ast).collect();
                let symbol = format!("spatial:{}:{}:{}", name.len(), name, args.len());
                z3::FuncDecl::new(symbol, &sort_refs, &Sort::bool())
                    .apply(&arguments)
                    .as_bool()
                    .expect("Boolean predicate")
            }
            SepAssertion::ListSegment { from, to, elements } if elements.is_empty() => {
                let equal = self
                    .encode_expr_as_addr(from)?
                    .eq(self.encode_expr_as_addr(to)?);
                Bool::and(&[
                    &equal,
                    &self.encode_assertion(&SepAssertion::Emp, heap, allocated)?,
                ])
            }
            SepAssertion::ListSegment { .. }
            | SepAssertion::Tree { .. }
            | SepAssertion::Block { .. }
            | SepAssertion::ArraySegment { .. } => {
                return Err(SepLogicError::EncodingError(Text::from(
                    "spatial shape requires an exact layout/domain definition; bounded approximations cannot establish entailment",
                )));
            }
        })
    }

    /// Restrict a total value array to its owned domain. Z3's array-map
    /// operation preserves extensionality without introducing lambda arguments
    /// or quantified witnesses into opaque predicate applications.
    fn canonical_heap(&self, heap: &Array, allocated: &Array) -> Array {
        let zero = self.model.value_const(0);
        let zeros = Array::const_array(&self.model.addr_sort, &zero);
        let condition = Bool::fresh_const("mask_condition");
        let value = self.model.fresh_value("mask_value");
        let choice = condition.ite(&value, &zero);
        let context = heap.get_ctx();
        let arrays = [
            allocated.get_z3_ast(),
            heap.get_z3_ast(),
            zeros.get_z3_ast(),
        ];
        // SAFETY: all nodes are live in the same Z3 context. The public entry
        // validates word64 -> Bool/word64 array sorts; the extracted built-in
        // ite declaration has signature (Bool, word64, word64) -> word64.
        // Mapping it over these arrays yields another word64 -> word64 array.
        // No z3 crate layout assumptions or transmutations are used.
        unsafe {
            let ctx = context.get_z3_context();
            let app = z3_sys::Z3_to_app(ctx, choice.get_z3_ast()).expect("ite application");
            let declaration = z3_sys::Z3_get_app_decl(ctx, app).expect("ite declaration");
            let mapped = z3_sys::Z3_mk_map(ctx, declaration, 3, arrays.as_ptr())
                .expect("well-sorted heap restriction");
            Array::wrap(context, mapped)
        }
    }

    /// A work bound never authorizes replacing an unfinished predicate by true.
    pub fn encode_with_unfolding(
        &self,
        assertion: &SepAssertion,
        heap: &Array,
        allocated: &Array,
        _depth: usize,
    ) -> Result<Bool, SepLogicError> {
        self.encode_assertion(assertion, heap, allocated)
    }

    fn scalar_binding(&self, name: &Text, boolean: bool) -> Result<Dynamic, SepLogicError> {
        if let Maybe::Some(value) = self.bindings.borrow().get(name) {
            let compatible = if boolean {
                value.as_bool().is_some()
            } else {
                value.as_bv().is_some_and(|v| v.get_size() == 64)
            };
            return if compatible {
                Ok(value.clone())
            } else {
                Err(SepLogicError::EncodingError(Text::from(format!(
                    "inconsistent scalar sort for separation variable {name}"
                ))))
            };
        }
        let value = if boolean {
            Dynamic::from_ast(&Bool::fresh_const(name.as_str()))
        } else {
            self.model.fresh_value(name.as_str())
        };
        self.bindings
            .borrow_mut()
            .insert(name.clone(), value.clone());
        Ok(value)
    }

    fn boolean_expression(&self, expression: &Expr) -> bool {
        match &expression.kind {
            ExprKind::Literal(lit) => matches!(lit.kind, LiteralKind::Bool(_)),
            ExprKind::Unary {
                op: verum_ast::UnOp::Not,
                ..
            } => true,
            ExprKind::Binary { op, .. } => matches!(
                op,
                BinOp::And
                    | BinOp::Or
                    | BinOp::Imply
                    | BinOp::Eq
                    | BinOp::Ne
                    | BinOp::Lt
                    | BinOp::Le
                    | BinOp::Gt
                    | BinOp::Ge
            ),
            ExprKind::Path(path) => path.as_ident().is_some_and(|id| {
                self.bindings
                    .borrow()
                    .get(&id.name)
                    .is_some_and(|v| v.as_bool().is_some())
            }),
            _ => false,
        }
    }

    fn encode_pure_expr(&self, expression: &Expr) -> Result<Bool, SepLogicError> {
        Ok(match &expression.kind {
            ExprKind::Literal(lit) if matches!(lit.kind, LiteralKind::Bool(_)) => {
                let LiteralKind::Bool(value) = lit.kind else {
                    unreachable!()
                };
                Bool::from_bool(value)
            }
            ExprKind::Path(path) if path.as_ident().is_some() => self
                .scalar_binding(&path.as_ident().unwrap().name, true)?
                .as_bool()
                .expect("checked Boolean binding"),
            ExprKind::Unary {
                op: verum_ast::UnOp::Not,
                expr,
            } => self.encode_pure_expr(expr)?.not(),
            ExprKind::Binary { op, left, right } => match op {
                BinOp::And | BinOp::Or | BinOp::Imply => {
                    let left = self.encode_pure_expr(left)?;
                    let right = self.encode_pure_expr(right)?;
                    match op {
                        BinOp::And => Bool::and(&[&left, &right]),
                        BinOp::Or => Bool::or(&[&left, &right]),
                        _ => left.implies(&right),
                    }
                }
                BinOp::Eq | BinOp::Ne => {
                    let equality =
                        if self.boolean_expression(left) || self.boolean_expression(right) {
                            self.encode_pure_expr(left)?
                                .eq(self.encode_pure_expr(right)?)
                        } else {
                            self.encode_expr_as_value(left)?
                                .eq(self.encode_expr_as_value(right)?)
                        };
                    if *op == BinOp::Eq {
                        equality
                    } else {
                        equality.not()
                    }
                }
                BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                    let left = self.encode_word(left)?;
                    let right = self.encode_word(right)?;
                    match op {
                        BinOp::Lt => left.bvslt(&right),
                        BinOp::Le => left.bvsle(&right),
                        BinOp::Gt => left.bvsgt(&right),
                        _ => left.bvsge(&right),
                    }
                }
                _ => return Err(Self::unsupported_scalar()),
            },
            _ => return Err(Self::unsupported_scalar()),
        })
    }

    fn unsupported_scalar() -> SepLogicError {
        SepLogicError::EncodingError(Text::from(
            "unsupported separation expression: expected Boolean logic or signed 64-bit literals, scoped variables and word arithmetic",
        ))
    }

    fn encode_word(&self, expression: &Expr) -> Result<BV, SepLogicError> {
        Ok(match &expression.kind {
            ExprKind::Literal(lit) => match &lit.kind {
                LiteralKind::Int(integer)
                    if matches!(
                        integer.suffix,
                        None | Some(verum_ast::literal::IntSuffix::I64)
                    ) =>
                {
                    BV::from_i64(
                        i64::try_from(integer.value).map_err(|_| Self::unsupported_scalar())?,
                        64,
                    )
                }
                _ => return Err(Self::unsupported_scalar()),
            },
            ExprKind::Path(path) if path.as_ident().is_some() => self
                .scalar_binding(&path.as_ident().unwrap().name, false)?
                .as_bv()
                .expect("checked word binding"),
            ExprKind::Unary {
                op: verum_ast::UnOp::Neg,
                expr,
            } => self.encode_word(expr)?.bvneg(),
            ExprKind::Binary { op, left, right } => {
                let left = self.encode_word(left)?;
                let right = self.encode_word(right)?;
                match op {
                    BinOp::Add => left.bvadd(&right),
                    BinOp::Sub => left.bvsub(&right),
                    BinOp::Mul => left.bvmul(&right),
                    _ => return Err(Self::unsupported_scalar()),
                }
            }
            _ => return Err(Self::unsupported_scalar()),
        })
    }

    pub(super) fn encode_expr_as_addr(&self, expression: &Expr) -> Result<Dynamic, SepLogicError> {
        self.encode_word(expression)
            .map(|value| Dynamic::from_ast(&value))
    }

    fn encode_expr_as_value(&self, expression: &Expr) -> Result<Dynamic, SepLogicError> {
        self.encode_expr_as_addr(expression)
    }
}
