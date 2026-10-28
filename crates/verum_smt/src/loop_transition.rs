//! Checked scalar loop lowering. Branches join with typed `ite` expressions;
//! assignments read the current environment, never a flattened pre-state.
use crate::contract::{ContractError, ContractResult};
use verum_ast::{BinOp, Block, Expr, ExprKind, LiteralKind, PatternKind, Type, TypeKind, UnOp};
use verum_common::{List, Map, Text};
use z3::ast::{Ast, Bool, Dynamic, Int};

fn unsupported(message: &str) -> ContractError {
    ContractError::InvalidContract(Text::from(message))
}

pub(crate) fn integer_bounds(value: &Int) -> Bool {
    Bool::and(&[value.ge(i64::MIN), value.le(i64::MAX)])
}

#[derive(Clone)]
pub(crate) struct ScalarState {
    names: Map<Text, usize>,
    pub values: List<Dynamic>,
    mutable: List<bool>,
}

impl ScalarState {
    pub fn new(variables: &[(Text, Type)]) -> ContractResult<Self> {
        let mut state = Self {
            names: Map::new(),
            values: List::new(),
            mutable: List::new(),
        };
        for (name, ty) in variables {
            if state.names.contains_key(name) {
                return Err(unsupported("duplicate loop state variable"));
            }
            let value = match ty.kind {
                TypeKind::Int => Int::fresh_const(name.as_str()).into(),
                TypeKind::Bool => Bool::fresh_const(name.as_str()).into(),
                _ => {
                    return Err(unsupported(
                        "loop lowering supports Int and Bool state only",
                    ));
                }
            };
            state.bind(name.clone(), value, true);
        }
        Ok(state)
    }

    fn bind(&mut self, name: Text, value: Dynamic, mutable: bool) {
        self.names.insert(name, self.values.len());
        self.values.push(value);
        self.mutable.push(mutable);
    }

    fn slot(&self, expr: &Expr) -> ContractResult<usize> {
        if let ExprKind::Path(path) = &expr.kind {
            if let Some(name) = path.as_ident() {
                if let Some(slot) = self.names.get(&Text::from(name.as_str())) {
                    return Ok(*slot);
                }
            }
        }
        Err(unsupported(
            "loop expression requires a declared scalar variable",
        ))
    }

    pub fn bounds(&self) -> Bool {
        Bool::and(
            &self
                .values
                .iter()
                .filter_map(|v| v.as_int().map(|i| integer_bounds(&i)))
                .collect::<List<_>>(),
        )
    }

    /// Specifications use mathematical integer terms over bounded source state.
    /// Executable arithmetic is separately checked at every intermediate step.
    pub fn term(&self, expr: &Expr) -> ContractResult<Dynamic> {
        Encoder::new(false).value(self, expr, &Bool::from_bool(true))
    }

    pub fn predicate(&self, expr: &Expr) -> ContractResult<Bool> {
        self.term(expr)?
            .as_bool()
            .ok_or_else(|| unsupported("loop predicate must be boolean"))
    }

    pub fn condition(&self, expr: &Expr) -> ContractResult<(Bool, Bool)> {
        let mut encoder = Encoder::new(true);
        let value = encoder
            .value(self, expr, &Bool::from_bool(true))?
            .as_bool()
            .ok_or_else(|| unsupported("loop exit condition must be boolean"))?;
        Ok((value, Bool::and(&encoder.obligations)))
    }

    pub fn execute(&self, body: &Expr) -> ContractResult<(Self, Bool)> {
        let mut after = self.clone();
        let mut encoder = Encoder::new(true);
        encoder.execute(&mut after, body, &Bool::from_bool(true))?;
        Ok((after, Bool::and(&encoder.obligations)))
    }
}

struct Encoder {
    checked: bool,
    obligations: List<Bool>,
}

impl Encoder {
    fn new(checked: bool) -> Self {
        Self {
            checked,
            obligations: List::new(),
        }
    }

    fn value(&mut self, state: &ScalarState, expr: &Expr, path: &Bool) -> ContractResult<Dynamic> {
        let value = match &expr.kind {
            ExprKind::Literal(lit) => match &lit.kind {
                LiteralKind::Bool(value) => Bool::from_bool(*value).into(),
                LiteralKind::Int(value) if value.suffix.is_none() => {
                    let value = i64::try_from(value.value)
                        .map_err(|_| unsupported("loop integer literal exceeds Int range"))?;
                    Int::from_i64(value).into()
                }
                _ => return Err(unsupported("unsupported loop literal")),
            },
            ExprKind::Path(_) => state.values[state.slot(expr)?].clone(),
            ExprKind::Paren(inner) => return self.value(state, inner, path),
            ExprKind::Unary { op, expr } => {
                let value = self.value(state, expr, path)?;
                match op {
                    UnOp::Neg => value
                        .as_int()
                        .ok_or_else(|| unsupported("negation requires Int"))?
                        .unary_minus()
                        .into(),
                    UnOp::Not => value
                        .as_bool()
                        .ok_or_else(|| unsupported("logical negation requires Bool"))?
                        .not()
                        .into(),
                    _ => {
                        return Err(unsupported(
                            "loop references and bitwise unary operations are unsupported",
                        ));
                    }
                }
            }
            ExprKind::Binary { op, left, right } if !op.is_assignment() => {
                let left = self.value(state, left, path)?;
                // Runtime Boolean operators short-circuit; inactive arithmetic
                // is still type checked but cannot create a safety obligation.
                let right_path = match op {
                    BinOp::And | BinOp::Or => {
                        let guard = left
                            .as_bool()
                            .ok_or_else(|| unsupported("logical operand requires Bool"))?;
                        Bool::and(&[
                            path.clone(),
                            if *op == BinOp::And {
                                guard
                            } else {
                                guard.not()
                            },
                        ])
                    }
                    _ => path.clone(),
                };
                let right = self.value(state, right, &right_path)?;
                self.binary(*op, left, right)?
            }
            _ => {
                return Err(unsupported(
                    "unsupported effect or expression in scalar loop",
                ));
            }
        };
        if self.checked {
            if let Some(integer) = value.as_int() {
                self.obligations
                    .push(path.implies(integer_bounds(&integer)));
            }
        }
        Ok(value)
    }

    fn binary(&self, op: BinOp, left: Dynamic, right: Dynamic) -> ContractResult<Dynamic> {
        if left.get_sort() != right.get_sort() {
            return Err(unsupported("loop operands have different scalar types"));
        }
        if matches!(op, BinOp::Eq | BinOp::Ne) {
            let eq = left.eq(&right);
            return Ok(if op == BinOp::Eq { eq } else { eq.not() }.into());
        }
        if let (Some(l), Some(r)) = (left.as_int(), right.as_int()) {
            return Ok(match op {
                BinOp::Add => (&l + &r).into(),
                BinOp::Sub => (&l - &r).into(),
                BinOp::Mul => (&l * &r).into(),
                BinOp::Lt => l.lt(&r).into(),
                BinOp::Le => l.le(&r).into(),
                BinOp::Gt => l.gt(&r).into(),
                BinOp::Ge => l.ge(&r).into(),
                _ => return Err(unsupported("unsupported loop integer operation")),
            });
        }
        if let (Some(l), Some(r)) = (left.as_bool(), right.as_bool()) {
            return Ok(match op {
                BinOp::And => Bool::and(&[l, r]).into(),
                BinOp::Or => Bool::or(&[l, r]).into(),
                BinOp::Imply if !self.checked => l.implies(r).into(),
                BinOp::Iff if !self.checked => l.iff(r).into(),
                _ => return Err(unsupported("unsupported loop Boolean operation")),
            });
        }
        Err(unsupported("unsupported loop operand types"))
    }

    fn execute(&mut self, state: &mut ScalarState, expr: &Expr, path: &Bool) -> ContractResult<()> {
        match &expr.kind {
            ExprKind::Binary { op, left, right } if op.is_assignment() => {
                let slot = state.slot(left)?;
                if !state.mutable[slot] {
                    return Err(unsupported("assignment to immutable loop local"));
                }
                let rhs = self.value(state, right, path)?;
                let value = match op {
                    BinOp::Assign => rhs,
                    BinOp::AddAssign => self.binary(BinOp::Add, state.values[slot].clone(), rhs)?,
                    BinOp::SubAssign => self.binary(BinOp::Sub, state.values[slot].clone(), rhs)?,
                    BinOp::MulAssign => self.binary(BinOp::Mul, state.values[slot].clone(), rhs)?,
                    _ => return Err(unsupported("unsupported compound loop assignment")),
                };
                if value.get_sort() != state.values[slot].get_sort() {
                    return Err(unsupported("loop assignment changes scalar type"));
                }
                if let Some(integer) = value.as_int() {
                    self.obligations
                        .push(path.implies(integer_bounds(&integer)));
                }
                state.values[slot] = value;
            }
            ExprKind::Block(block) => self.block(state, block, path)?,
            ExprKind::Paren(inner) => self.execute(state, inner, path)?,
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let mut guard = Bool::from_bool(true);
                for condition in &condition.conditions {
                    let verum_ast::expr::ConditionKind::Expr(condition) = condition else {
                        return Err(unsupported(
                            "pattern guards require a typed pattern transition",
                        ));
                    };
                    let current_path = Bool::and(&[path, &guard]);
                    let value = self
                        .value(state, condition, &current_path)?
                        .as_bool()
                        .ok_or_else(|| unsupported("loop branch condition must be boolean"))?;
                    guard = Bool::and(&[guard, value]);
                }
                if condition.conditions.is_empty() {
                    return Err(unsupported("missing loop branch condition"));
                }
                let mut then_state = state.clone();
                let mut else_state = state.clone();
                self.block(&mut then_state, then_branch, &Bool::and(&[path, &guard]))?;
                if let Some(other) = else_branch {
                    self.execute(&mut else_state, other, &Bool::and(&[path, &guard.not()]))?;
                }
                for (i, value) in state.values.iter_mut().enumerate() {
                    *value = guard.ite(&then_state.values[i], &else_state.values[i]);
                }
            }
            _ => {
                self.value(state, expr, path)?;
            }
        }
        Ok(())
    }

    fn block(&mut self, state: &mut ScalarState, block: &Block, path: &Bool) -> ContractResult<()> {
        let names = state.names.clone();
        let slots = state.values.len();
        for stmt in &block.stmts {
            if !stmt.attributes.is_empty() {
                return Err(unsupported(
                    "statement attributes require explicit loop semantics",
                ));
            }
            match &stmt.kind {
                verum_ast::StmtKind::Expr { expr, .. } => self.execute(state, expr, path)?,
                verum_ast::StmtKind::Let {
                    pattern,
                    ty,
                    value: Some(value),
                } => {
                    let value = self.value(state, value, path)?;
                    if let Some(ty) = ty {
                        if !matches!(
                            (&ty.kind, value.as_int(), value.as_bool()),
                            (TypeKind::Int, Some(_), _) | (TypeKind::Bool, _, Some(_))
                        ) {
                            return Err(unsupported(
                                "loop local annotation must match its scalar value",
                            ));
                        }
                    }
                    match &pattern.kind {
                        PatternKind::Ident {
                            name,
                            mutable,
                            by_ref: false,
                            subpattern: None,
                        } => state.bind(Text::from(name.as_str()), value, *mutable),
                        PatternKind::Wildcard => {}
                        _ => return Err(unsupported("loop locals require plain scalar bindings")),
                    }
                }
                verum_ast::StmtKind::Empty => {}
                _ => return Err(unsupported("unsupported loop statement")),
            }
        }
        if let Some(tail) = &block.expr {
            self.execute(state, tail, path)?;
        }
        state.names = names;
        state.values.truncate(slots);
        state.mutable.truncate(slots);
        Ok(())
    }
}
