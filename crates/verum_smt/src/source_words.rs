//! Refutation of scalar source theorems in the executable signed-word domain.
//! A model of the negated source judgment can disqualify a proposed proof.
//! This check never supplies a proof body or grants kernel authority.
use crate::Context;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use verum_ast::decl::{FunctionBody, FunctionParam, FunctionParamKind, TheoremDecl, TypeDeclBody};
use verum_ast::expr::{BinOpCategory, Block, ConditionKind};
use verum_ast::visitor::{Visitor, walk_expr};
use verum_ast::{
    BinOp, Expr, ExprKind, ItemKind, LiteralKind, Module, Pattern, PatternKind, StmtKind, Type,
    TypeKind, UnOp,
};
use z3::ast::{self, Ast, BV, Bool};

const MAX_DEPTH: usize = 48;
const MAX_NODES: usize = 8192;

type Bindings = BTreeMap<String, Value>;

#[derive(Clone)]
enum Value {
    Word(BV),
    Truth(Bool),
    Record(String, Rc<Bindings>),
}

impl Value {
    fn scalar_ast(&self) -> Option<&dyn Ast> {
        match self {
            Self::Word(value) => Some(value),
            Self::Truth(value) => Some(value),
            Self::Record(..) => None,
        }
    }

    fn truth(self) -> Option<Bool> {
        match self {
            Self::Truth(value) => Some(value),
            _ => None,
        }
    }

    fn has_type(&self, ty: &Type) -> bool {
        match (&ty.kind, self) {
            (TypeKind::Int, Self::Word(_)) | (TypeKind::Bool, Self::Truth(_)) => true,
            (TypeKind::Path(path), Self::Record(name, _)) => {
                path.as_ident().is_some_and(|id| id.as_str() == name)
            }
            _ => false,
        }
    }
}

pub(crate) fn word_arithmetic(op: BinOp, lhs: &BV, rhs: &BV) -> Option<BV> {
    Some(match op {
        BinOp::Add => lhs.bvadd(rhs),
        BinOp::Sub => lhs.bvsub(rhs),
        BinOp::Mul => lhs.bvmul(rhs),
        _ => return None,
    })
}

fn parameter(param: &FunctionParam) -> Option<(&str, &Type)> {
    let FunctionParamKind::Regular { pattern, ty, .. } = &param.kind else {
        return None;
    };
    Some((binding_name(pattern)?, ty))
}

fn binding_name(pattern: &Pattern) -> Option<&str> {
    match &pattern.kind {
        PatternKind::Ident {
            name,
            by_ref: false,
            subpattern: None,
            ..
        } => Some(name.as_str()),
        _ => None,
    }
}

// If exact translation stops, inspect the rest of the judgment and its
// reachable helper bodies. An unsupported operand must not conceal word
// arithmetic from verification. Exhausting the inspection budget is inconclusive.
#[derive(Default)]
struct WordUse {
    found: bool,
    depth: usize,
    visited: usize,
    calls: BTreeSet<String>,
}

impl Visitor for WordUse {
    fn visit_expr(&mut self, expr: &Expr) {
        if self.found {
            return;
        }
        self.visited += 1;
        if self.visited >= MAX_NODES || self.depth >= MAX_DEPTH {
            self.found = true;
            return;
        }
        match &expr.kind {
            ExprKind::Binary { op, .. }
                if matches!(
                    op.category(),
                    BinOpCategory::Arithmetic | BinOpCategory::Bitwise
                ) =>
            {
                self.found = true
            }
            ExprKind::Binary {
                op: BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge,
                ..
            }
            | ExprKind::Unary { op: UnOp::Neg, .. } => self.found = true,
            ExprKind::Forall { bindings, .. } | ExprKind::Exists { bindings, .. }
                if bindings.iter().any(|b| {
                    b.ty.as_ref()
                        .is_some_and(|t| matches!(t.kind, TypeKind::Int))
                }) =>
            {
                self.found = true
            }
            ExprKind::Call { func, .. } => {
                if let ExprKind::Path(path) = &func.kind {
                    if let Some(name) = path.as_ident() {
                        self.calls.insert(name.as_str().to_owned());
                    }
                }
            }
            _ => {}
        }
        self.depth += 1;
        walk_expr(self, expr);
        self.depth -= 1;
    }
}

fn needs_word_check(theorem: &TheoremDecl, conclusion: &Expr, module: Option<&Module>) -> bool {
    let mut uses = WordUse::default();
    uses.visit_expr(conclusion);
    for hyp in &theorem.requires {
        uses.visit_expr(hyp);
    }
    if let Some(module) = module {
        let mut visited = BTreeSet::new();
        while !uses.found {
            let Some(name) = uses.calls.pop_first() else {
                break;
            };
            if !visited.insert(name.clone()) {
                continue;
            }
            for item in &module.items {
                if let ItemKind::Function(f) = &item.kind {
                    if f.name.as_str() == name {
                        match f.body.as_ref() {
                            Some(FunctionBody::Expr(e)) => uses.visit_expr(e),
                            Some(FunctionBody::Block(b)) => uses.visit_block(b),
                            _ => {}
                        }
                    }
                }
            }
        }
    }
    uses.found
}

struct Encoding<'a> {
    module: Option<&'a Module>,
    inputs: Bindings,
    has_words: bool,
    remaining: usize,
}

impl Encoding<'_> {
    fn input(&mut self, name: &str, ty: &Type, depth: usize) -> Option<Value> {
        if depth >= MAX_DEPTH {
            return None;
        }
        self.remaining = self.remaining.checked_sub(1)?;
        let value = match &ty.kind {
            TypeKind::Int => {
                self.has_words = true;
                Value::Word(BV::fresh_const(name, i64::BITS))
            }
            TypeKind::Bool => Value::Truth(Bool::fresh_const(name)),
            TypeKind::Path(path) => {
                let head = path.as_ident()?.as_str();
                let mut found = self
                    .module?
                    .items
                    .iter()
                    .filter_map(|item| match &item.kind {
                        ItemKind::Type(t) if t.name.as_str() == head => Some(t),
                        _ => None,
                    });
                let decl = found.next()?;
                if found.next().is_some()
                    || !decl.generics.is_empty()
                    || !decl.attributes.is_empty()
                    || !decl.value_params.is_empty()
                    || decl.resource_modifier.is_some()
                    || decl.generic_where_clause.is_some()
                    || decl.meta_where_clause.is_some()
                {
                    return None;
                }
                let TypeDeclBody::Record(fields) = &decl.body else {
                    return None;
                };
                let mut values = Bindings::new();
                for field in fields {
                    if !field.attributes.is_empty() || field.bit_spec.is_some() {
                        return None;
                    }
                    let field_name = field.name.as_str();
                    let value =
                        self.input(&format!("{name}.{field_name}"), &field.ty, depth + 1)?;
                    if values.insert(field_name.to_owned(), value).is_some() {
                        return None;
                    }
                }
                return Some(Value::Record(head.to_owned(), Rc::new(values)));
            }
            _ => return None,
        };
        self.inputs.insert(name.to_owned(), value.clone());
        Some(value)
    }

    fn binary(&mut self, op: BinOp, lhs: Value, rhs: Value) -> Option<Value> {
        let result = match (lhs, rhs) {
            (Value::Word(a), Value::Word(b)) => {
                if let Some(word) = word_arithmetic(op, &a, &b) {
                    return Some(Value::Word(word));
                }
                match op {
                    BinOp::Eq => a.eq(&b),
                    BinOp::Ne => a.eq(&b).not(),
                    BinOp::Lt => a.bvslt(&b),
                    BinOp::Le => a.bvsle(&b),
                    BinOp::Gt => a.bvsgt(&b),
                    BinOp::Ge => a.bvsge(&b),
                    _ => return None,
                }
            }
            (Value::Truth(a), Value::Truth(b)) => match op {
                BinOp::And => Bool::and(&[&a, &b]),
                BinOp::Or => Bool::or(&[&a, &b]),
                BinOp::Imply => a.implies(&b),
                BinOp::Iff | BinOp::Eq => a.iff(&b),
                BinOp::Ne => a.iff(&b).not(),
                _ => return None,
            },
            _ => return None,
        };
        Some(Value::Truth(result))
    }

    fn block(&mut self, block: &Block, scope: &Bindings, depth: usize) -> Option<Value> {
        if block.stmts.is_empty() {
            return self.value(block.expr.as_deref()?, scope, depth);
        }
        let mut locals = scope.clone();
        for stmt in &block.stmts {
            let StmtKind::Let {
                pattern,
                ty,
                value: Some(rhs),
            } = &stmt.kind
            else {
                return None;
            };
            let name = binding_name(pattern)?;
            let value = self.value(rhs, &locals, depth)?;
            if ty.as_ref().is_some_and(|ty| !value.has_type(ty)) {
                return None;
            }
            locals.insert(name.to_owned(), value);
        }
        self.value(block.expr.as_deref()?, &locals, depth)
    }

    fn value(&mut self, expr: &Expr, scope: &Bindings, depth: usize) -> Option<Value> {
        if depth >= MAX_DEPTH {
            return None;
        }
        self.remaining = self.remaining.checked_sub(1)?;
        let next = depth + 1;
        match &expr.kind {
            ExprKind::Literal(lit) => match &lit.kind {
                LiteralKind::Bool(b) => Some(Value::Truth(Bool::from_bool(*b))),
                LiteralKind::Int(n) if n.suffix.is_none() => {
                    self.has_words = true;
                    Some(Value::Word(BV::from_i64(
                        i64::try_from(n.value).ok()?,
                        i64::BITS,
                    )))
                }
                _ => None,
            },
            ExprKind::Path(path) => scope.get(path.as_ident()?.as_str()).cloned(),
            ExprKind::Field { expr, field } => {
                let Value::Record(_, fields) = self.value(expr, scope, next)? else {
                    return None;
                };
                fields.get(field.as_str()).cloned()
            }
            ExprKind::Paren(x) => self.value(x, scope, next),
            ExprKind::Block(block) => self.block(block, scope, next),
            ExprKind::Binary { op, left, right } => {
                let lhs = self.value(left, scope, next)?;
                let rhs = self.value(right, scope, next)?;
                self.binary(*op, lhs, rhs)
            }
            ExprKind::Unary { op, expr } => match (op, self.value(expr, scope, next)?) {
                (UnOp::Neg, Value::Word(x)) => Some(Value::Word(x.bvneg())),
                (UnOp::Not, Value::Truth(x)) => Some(Value::Truth(x.not())),
                _ => None,
            },
            ExprKind::Forall { bindings, body } | ExprKind::Exists { bindings, body } => {
                let mut locals = scope.clone();
                let mut bounds = Vec::new();
                for binding in bindings {
                    if binding.domain.is_some() || binding.guard.is_some() {
                        return None;
                    }
                    let name = binding_name(&binding.pattern)?;
                    let variable = match &binding.ty.as_ref()?.kind {
                        TypeKind::Int => {
                            self.has_words = true;
                            Value::Word(BV::fresh_const(name, i64::BITS))
                        }
                        TypeKind::Bool => Value::Truth(Bool::fresh_const(name)),
                        _ => return None,
                    };
                    locals.insert(name.to_owned(), variable.clone());
                    bounds.push(variable);
                }
                let pred = self.value(body, &locals, next)?.truth()?;
                let refs = bounds
                    .iter()
                    .map(Value::scalar_ast)
                    .collect::<Option<Vec<_>>>()?;
                Some(Value::Truth(
                    if matches!(expr.kind, ExprKind::Forall { .. }) {
                        ast::forall_const(&refs, &[], &pred)
                    } else {
                        ast::exists_const(&refs, &[], &pred)
                    },
                ))
            }
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let [ConditionKind::Expr(test)] = condition.conditions.as_slice() else {
                    return None;
                };
                let guard = self.value(test, scope, next)?.truth()?;
                let yes = self.block(then_branch, scope, next)?;
                let no = self.value(else_branch.as_deref()?, scope, next)?;
                match (yes, no) {
                    (Value::Word(a), Value::Word(b)) => Some(Value::Word(guard.ite(&a, &b))),
                    (Value::Truth(a), Value::Truth(b)) => Some(Value::Truth(guard.ite(&a, &b))),
                    _ => None,
                }
            }
            ExprKind::Call {
                func,
                type_args,
                args,
            } if type_args.is_empty() => {
                let ExprKind::Path(path) = &func.kind else {
                    return None;
                };
                let name = path.as_ident()?.as_str();
                if scope.contains_key(name) {
                    return None;
                }
                let mut found = self
                    .module?
                    .items
                    .iter()
                    .filter_map(|item| match &item.kind {
                        ItemKind::Function(f) if f.name.as_str() == name => Some(f),
                        _ => None,
                    });
                let f = found.next()?;
                if found.next().is_some()
                    || !f.generics.is_empty()
                    || !f.attributes.is_empty()
                    || f.is_meta
                    || f.is_async
                    || f.is_generator
                    || f.is_unsafe
                    || f.is_variadic
                    || f.extern_abi.is_some()
                    || f.std_attr.is_some()
                    || !f.contexts.is_empty()
                    || f.throws_clause.is_some()
                    || !f.requires.is_empty()
                    || !f.ensures.is_empty()
                    || f.generic_where_clause.is_some()
                    || f.meta_where_clause.is_some()
                    || f.params.len() != args.len()
                {
                    return None;
                }
                let mut locals = Bindings::new();
                for (param, arg) in f.params.iter().zip(args) {
                    let (name, ty) = parameter(param)?;
                    let value = self.value(arg, scope, next)?;
                    if !value.has_type(ty) || locals.insert(name.to_owned(), value).is_some() {
                        return None;
                    }
                }
                let result = match f.body.as_ref()? {
                    FunctionBody::Expr(e) => self.value(e, &locals, next)?,
                    FunctionBody::Block(b) => self.block(b, &locals, next)?,
                    _ => return None,
                };
                result.has_type(f.return_type.as_ref()?).then_some(result)
            }
            _ => None,
        }
    }
}

/// Check the negated judgment only when all source inputs, hypotheses, calls
/// and the conclusion admit an exact total-word translation. No uninterpreted
/// fallback, tactic hint or reflected integer axiom participates in this query.
pub fn refute(
    ctx: &Context,
    theorem: &TheoremDecl,
    conclusion: &Expr,
    module: Option<&Module>,
) -> Option<String> {
    let incomplete = || {
        needs_word_check(theorem, conclusion, module).then(||
            "signed-word verification inconclusive: the source judgment exceeds the exact translation fragment or its resource budget".to_owned()
        )
    };
    if !theorem.generics.is_empty()
        || theorem.generic_where_clause.is_some()
        || theorem.meta_where_clause.is_some()
    {
        return incomplete();
    }
    let mut enc = Encoding {
        module,
        inputs: Bindings::new(),
        has_words: false,
        remaining: MAX_NODES,
    };
    let mut scope = Bindings::new();
    for param in &theorem.params {
        let Some((name, ty)) = parameter(param) else {
            return incomplete();
        };
        let Some(value) = enc.input(name, ty, 0) else {
            return incomplete();
        };
        if scope.insert(name.to_owned(), value).is_some() {
            return incomplete();
        }
    }
    let judgment = (|| {
        let hypotheses = theorem
            .requires
            .iter()
            .map(|req| enc.value(req, &scope, 0)?.truth())
            .collect::<Option<Vec<_>>>()?;
        Some((hypotheses, enc.value(conclusion, &scope, 0)?.truth()?))
    })();
    let Some((hypotheses, goal)) = judgment else {
        return incomplete();
    };
    if !enc.has_words {
        return None;
    }
    let solver = ctx.solver();
    for hyp in hypotheses {
        solver.assert(hyp);
    }
    solver.assert(goal.not());
    match solver.check() {
        z3::SatResult::Unsat => None,
        z3::SatResult::Unknown => Some(format!(
            "signed-word verification inconclusive: {}",
            solver.get_reason_unknown().unwrap_or_default()
        )),
        z3::SatResult::Sat => {
            let model = solver.get_model();
            let inputs = enc
                .inputs
                .iter()
                .filter_map(|(name, value)| {
                    let model = model.as_ref()?;
                    let text = match value {
                        Value::Word(w) => (model.eval(w, true)?.as_u64()? as i64).to_string(),
                        Value::Truth(b) => model.eval(b, true)?.as_bool()?.to_string(),
                        Value::Record(..) => return None,
                    };
                    Some(format!("{name} = {text}"))
                })
                .collect::<Vec<_>>()
                .join(", ");
            Some(format!(
                "counterexample in signed {}-bit source arithmetic ({inputs})",
                i64::BITS
            ))
        }
    }
}
