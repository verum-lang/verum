//! Exact elaboration of a closed, total finite fragment of ordinary functions.
//! Contracts become predicates of the translated body, never assumed axioms.
//! This is source semantics; it does not certify VBC/LLVM lowering or a host run.
use crate::{SourceBinding, SourceSpan, scoped::Error, sha256};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use verum_ast::decl::{
    FunctionBody, FunctionDecl, FunctionParamKind, Item, ItemKind, TypeDeclBody,
};
use verum_ast::expr::{Block, ConditionKind};
use verum_ast::literal::LiteralKind;
use verum_ast::pattern::{Pattern, PatternKind};
use verum_ast::stmt::StmtKind;
use verum_ast::ty::{Type, TypeKind};
use verum_ast::{BinOp, Expr, ExprKind, Module, Path, PathSegment, UnOp};
use verum_common::Maybe;
use verum_proof::proof_checker::{Certificate, DataConstructor, Level, Term};

pub mod proof;
pub const PROFILE: &str = "verum-source-finite/1";
type Result<T> = std::result::Result<T, Error>;
type Env = Vec<(String, Term)>;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImplementationBinding {
    pub semantics: String,
    pub function: Term,
    pub function_type: Term,
    pub dependencies: Vec<SourceBinding>,
    pub input_cases: usize,
    pub requires_count: usize,
    pub ensures_count: usize,
}
pub struct Artifact {
    pub certificate: Certificate,
    pub implementation: ImplementationBinding,
    pub source: SourceBinding,
}
#[derive(Clone)]
struct Value {
    term: Term,
    ty: Term,
}
#[derive(Clone)]
struct Function {
    term: Term,
    domains: Vec<Term>,
    result: Term,
}
struct Elab<'a> {
    source: &'a str,
    source_hash: String,
    items: BTreeMap<String, &'a Item>,
    types: BTreeMap<String, Term>,
    variants: BTreeMap<String, Vec<String>>,
    functions: BTreeMap<String, Function>,
    active: BTreeSet<String>,
    dependencies: BTreeSet<String>,
    remaining: usize,
    selected: &'a str,
}
fn unsupported(message: impl Into<String>) -> Error {
    Error::Unsupported(message.into())
}
fn rejected(message: impl Into<String>) -> Error {
    Error::Rejected(message.into())
}
fn data(name: String, count: usize) -> Term {
    Term::Data {
        name,
        level: Level::Concrete(0),
        indices: vec![],
        constructors: (0..count).map(|_| DataConstructor::from(vec![])).collect(),
    }
}
pub fn boolean() -> Term {
    data("verum-source-finite/1:Bool".into(), 2)
}
fn constructor(ty: &Term, index: usize) -> Term {
    Term::Construct {
        data: Box::new(ty.clone()),
        constructor: index,
        args: vec![],
    }
}
fn truth(value: bool) -> Term {
    constructor(&boolean(), usize::from(value))
}
fn select(scrutinee: Term, domain: Term, result: Term, cases: Vec<Term>) -> Term {
    // Every admitted carrier is closed, so motive extension needs no lifting.
    Term::Induct {
        scrutinee: Box::new(scrutinee),
        motive: Box::new(Term::lam(domain, result)),
        cases,
    }
}
fn choose(condition: Term, ty: Term, yes: Term, no: Term) -> Term {
    select(condition, boolean(), ty, vec![no, yes])
}
fn and(a: Term, b: Term) -> Term {
    choose(a, boolean(), b, truth(false))
}
fn name(path: &Path) -> Result<String> {
    path.segments
        .iter()
        .map(|part| match part {
            PathSegment::Name(n) => Ok(n.as_str()),
            _ => Err(unsupported(
                "only exact local declaration paths are supported",
            )),
        })
        .collect::<Result<Vec<_>>>()
        .map(|parts| parts.join("."))
}
fn binder(pattern: &Pattern) -> Result<String> {
    match &pattern.kind {
        PatternKind::Ident {
            by_ref: false,
            mutable: false,
            name,
            subpattern: Maybe::None,
        } if name.as_str() != "result" => Ok(name.as_str().into()),
        _ => Err(unsupported(
            "parameters and locals require immutable names; result is reserved for postconditions",
        )),
    }
}
fn require_type(actual: &Term, expected: &Term) -> Result<()> {
    if actual == expected {
        Ok(())
    } else {
        Err(rejected("finite source type mismatch"))
    }
}
impl Elab<'_> {
    fn binding(&self, name: &str) -> Result<SourceBinding> {
        let item = self
            .items
            .get(name)
            .ok_or_else(|| rejected("missing declaration"))?;
        let text = self
            .source
            .get(item.span.start as usize..item.span.end as usize)
            .filter(|text| !text.is_empty())
            .ok_or_else(|| rejected("invalid declaration span"))?;
        Ok(SourceBinding {
            sha256: self.source_hash.clone(),
            declaration: name.into(),
            span: Some(SourceSpan {
                start: item.span.start,
                end: item.span.end,
                sha256: sha256(text.as_bytes()),
            }),
        })
    }
    fn ty(&mut self, ty: &Type) -> Result<Term> {
        let n = match &ty.kind {
            TypeKind::Bool => return Ok(boolean()),
            TypeKind::Path(path) => name(path)?,
            _ => {
                return Err(unsupported(
                    "finite functions admit Bool and closed fieldless source variants",
                ));
            }
        };
        self.type_name(&n)
    }
    fn type_name(&mut self, n: &str) -> Result<Term> {
        if n == "Bool" {
            return Ok(boolean());
        }
        if let Some(ty) = self.types.get(n) {
            return Ok(ty.clone());
        }
        let item = *self
            .items
            .get(n)
            .ok_or_else(|| Error::Scope(format!("unknown local type {n}")))?;
        let ItemKind::Type(decl) = &item.kind else {
            return Err(rejected("value used as a type"));
        };
        if !item.attributes.is_empty()
            || !decl.attributes.is_empty()
            || !decl.generics.is_empty()
            || !decl.value_params.is_empty()
            || decl.resource_modifier.is_some()
            || decl.generic_where_clause.is_some()
            || decl.meta_where_clause.is_some()
        {
            return Err(unsupported(
                "type attributes, ownership modifiers and constraints need explicit semantics",
            ));
        }
        let TypeDeclBody::Variant(variants) = &decl.body else {
            return Err(unsupported(
                "only closed fieldless variant declarations are supported",
            ));
        };
        if variants.is_empty() {
            return Err(unsupported(
                "empty source variant declarations are unsupported",
            ));
        }
        let mut names = BTreeSet::new();
        let mut order = Vec::new();
        for variant in variants {
            if variant.data.is_some()
                || !variant.generic_params.is_empty()
                || variant.where_clause.is_some()
                || !variant.attributes.is_empty()
                || variant.path_endpoints.is_some()
            {
                return Err(unsupported(
                    "variant fields, refinements and path constructors need explicit semantics",
                ));
            }
            let variant_name = variant.name.as_str().to_owned();
            if !names.insert(variant_name.clone()) {
                return Err(rejected("duplicate variant"));
            }
            order.push(variant_name);
        }
        let ty = data(
            format!("source:{}:{}:{n}", self.source_hash, item.span.start),
            order.len(),
        );
        self.types.insert(n.into(), ty.clone());
        self.variants.insert(n.into(), order);
        self.dependencies.insert(n.into());
        Ok(ty)
    }
    fn variant(&mut self, qualified: &str) -> Result<Value> {
        let (owner, variant) = qualified.rsplit_once('.').ok_or_else(|| {
            unsupported("finite source constructors must be qualified by their declaration")
        })?;
        let ty = self.type_name(owner)?;
        let index = self
            .variants
            .get(owner)
            .and_then(|variants| variants.iter().position(|n| n == variant))
            .ok_or_else(|| Error::Scope(format!("unknown constructor {qualified}")))?;
        Ok(Value {
            term: constructor(&ty, index),
            ty,
        })
    }
    fn signature(&mut self, decl: &FunctionDecl) -> Result<(Env, Term)> {
        if decl.is_async
            || decl.is_meta
            || decl.stage_level != 0
            || decl.is_generator
            || decl.is_cofix
            || decl.is_unsafe
            || decl.is_transparent
            || decl.extern_abi.is_some()
            || decl.is_variadic
            || !decl.generics.is_empty()
            || decl.throws_clause.is_some()
            || decl.std_attr.is_some()
            || !decl.contexts.is_empty()
            || decl.generic_where_clause.is_some()
            || decl.meta_where_clause.is_some()
            || !decl.decreases.is_empty()
            || !decl.attributes.is_empty()
        {
            return Err(unsupported(
                "function modifiers, contexts, attributes and constraints need explicit semantics",
            ));
        }
        let mut env = Vec::new();
        let mut names = BTreeSet::new();
        for param in &decl.params {
            if !param.attributes.is_empty() {
                return Err(unsupported("parameter attributes"));
            }
            let FunctionParamKind::Regular {
                pattern,
                ty,
                default_value: Maybe::None,
            } = &param.kind
            else {
                return Err(unsupported(
                    "only explicit immutable parameters without defaults",
                ));
            };
            let n = binder(pattern)?;
            if !names.insert(n.clone()) {
                return Err(rejected("duplicate parameter"));
            }
            env.push((n, self.ty(ty)?));
        }
        let Maybe::Some(result) = &decl.return_type else {
            return Err(unsupported("explicit return type required"));
        };
        Ok((env, self.ty(result)?))
    }
    fn function(&mut self, n: &str) -> Result<Function> {
        if let Some(f) = self.functions.get(n) {
            return Ok(f.clone());
        }
        if !self.active.insert(n.into()) {
            return Err(unsupported("recursive calls require a termination proof"));
        }
        let item = *self
            .items
            .get(n)
            .ok_or_else(|| Error::Scope(format!("unknown local function {n}")))?;
        let ItemKind::Function(decl) = &item.kind else {
            return Err(rejected("non-function call target"));
        };
        if !item.attributes.is_empty() {
            return Err(unsupported("function item attributes"));
        }
        if n != self.selected && (!decl.requires.is_empty() || !decl.ensures.is_empty()) {
            return Err(unsupported(
                "calls to contracted helpers require separately discharged call obligations",
            ));
        }
        let (mut env, result) = self.signature(decl)?;
        let domains: Vec<_> = env.iter().map(|(_, ty)| ty.clone()).collect();
        let value = match &decl.body {
            Maybe::Some(FunctionBody::Block(block)) => self.block(block, &mut env)?,
            Maybe::Some(FunctionBody::Expr(expr)) => self.expr(expr, &mut env)?,
            _ => return Err(unsupported("function body missing")),
        };
        require_type(&value.ty, &result)?;
        let term = domains
            .iter()
            .rev()
            .fold(value.term, |body, ty| Term::lam(ty.clone(), body));
        let f = Function {
            term,
            domains,
            result,
        };
        self.functions.insert(n.into(), f.clone());
        self.dependencies.insert(n.into());
        self.active.remove(n);
        Ok(f)
    }
    fn block(&mut self, block: &Block, env: &mut Env) -> Result<Value> {
        let saved = env.len();
        let mut values = Vec::new();
        for stmt in &block.stmts {
            if !stmt.attributes.is_empty() {
                return Err(unsupported("statement attributes"));
            }
            let StmtKind::Let {
                pattern,
                ty,
                value: Maybe::Some(value),
            } = &stmt.kind
            else {
                return Err(unsupported(
                    "finite blocks require immutable initialized locals and a tail expression",
                ));
            };
            let local = binder(pattern)?;
            let value = self.expr(value, env)?;
            if let Maybe::Some(annotation) = ty {
                require_type(&value.ty, &self.ty(annotation)?)?;
            }
            env.push((local, value.ty.clone()));
            values.push(value.term);
        }
        let Maybe::Some(tail) = &block.expr else {
            return Err(unsupported("finite block requires a tail value"));
        };
        let mut value = self.expr(tail, env)?;
        env.truncate(saved);
        for local in values.into_iter().rev() {
            value.term = Term::Let {
                value: Box::new(local),
                body: Box::new(value.term),
            };
        }
        Ok(value)
    }
    fn expr(&mut self, expr: &Expr, env: &mut Env) -> Result<Value> {
        self.remaining = self
            .remaining
            .checked_sub(1)
            .ok_or_else(|| Error::Inconclusive {
                code: "source_node_limit",
                message: "finite elaboration node budget exhausted".into(),
            })?;
        match &expr.kind {
            ExprKind::Paren(inner) => self.expr(inner, env),
            ExprKind::Block(block) => self.block(block, env),
            ExprKind::Literal(lit) => match lit.kind {
                LiteralKind::Bool(value) => Ok(Value {
                    term: truth(value),
                    ty: boolean(),
                }),
                _ => Err(unsupported(
                    "only Boolean literals are in the finite source profile",
                )),
            },
            ExprKind::Path(path) => {
                let n = name(path)?;
                if let Some((index, (_, ty))) = env
                    .iter()
                    .rev()
                    .enumerate()
                    .find(|(_, (bound, _))| *bound == n)
                {
                    Ok(Value {
                        term: Term::var(index),
                        ty: ty.clone(),
                    })
                } else {
                    if n.split_once('.')
                        .is_some_and(|(owner, _)| env.iter().any(|(bound, _)| bound == owner))
                    {
                        return Err(unsupported("value shadows constructor owner"));
                    }
                    self.variant(&n)
                }
            }
            ExprKind::Field { expr: owner, field } => {
                let ExprKind::Path(path) = &owner.kind else {
                    return Err(unsupported("runtime field access"));
                };
                let owner = name(path)?;
                if env.iter().any(|(n, _)| n == &owner) {
                    return Err(unsupported("value shadows constructor owner"));
                }
                self.variant(&format!("{owner}.{}", field.as_str()))
            }
            ExprKind::Call {
                func,
                type_args,
                args,
            } => {
                let ExprKind::Path(path) = &func.kind else {
                    return Err(unsupported("only exact local function calls"));
                };
                if !type_args.is_empty() {
                    return Err(unsupported("generic call arguments"));
                }
                let n = name(path)?;
                if env.iter().any(|(bound, _)| bound == &n) {
                    return Err(unsupported("call target is shadowed by a value"));
                }
                if let Some(Item {
                    kind: ItemKind::Function(callee),
                    ..
                }) = self.items.get(&n).copied()
                {
                    if !callee.requires.is_empty() || !callee.ensures.is_empty() {
                        return Err(unsupported(
                            "calls to contracted functions require separately discharged call obligations",
                        ));
                    }
                }
                // Check every argument before building a reducible application.
                let f = self.function(&n)?;
                if args.len() != f.domains.len() {
                    return Err(rejected("call arity mismatch"));
                }
                let mut term = f.term;
                for (arg, domain) in args.iter().zip(&f.domains) {
                    let value = self.expr(arg, env)?;
                    require_type(&value.ty, domain)?;
                    term = Term::app(term, value.term);
                }
                Ok(Value { term, ty: f.result })
            }
            ExprKind::Unary {
                op: UnOp::Not,
                expr,
            } => {
                let value = self.expr(expr, env)?;
                require_type(&value.ty, &boolean())?;
                Ok(Value {
                    term: choose(value.term, boolean(), truth(false), truth(true)),
                    ty: boolean(),
                })
            }
            ExprKind::Binary { op, left, right } => {
                let left = self.expr(left, env)?;
                let right = self.expr(right, env)?;
                // User-defined Eq implementations are outside this closed profile.
                require_type(&left.ty, &boolean())?;
                require_type(&right.ty, &boolean())?;
                let a = left.term;
                let b = right.term;
                let term = match op {
                    BinOp::And => and(a, b),
                    BinOp::Or => choose(a, boolean(), truth(true), b),
                    BinOp::Eq => choose(
                        a,
                        boolean(),
                        b.clone(),
                        choose(b, boolean(), truth(false), truth(true)),
                    ),
                    BinOp::Ne => choose(
                        a,
                        boolean(),
                        choose(b.clone(), boolean(), truth(false), truth(true)),
                        b,
                    ),
                    _ => return Err(unsupported("operator outside finite Boolean semantics")),
                };
                Ok(Value {
                    term,
                    ty: boolean(),
                })
            }
            ExprKind::If {
                condition,
                then_branch,
                else_branch: Maybe::Some(other),
            } => {
                let [ConditionKind::Expr(condition)] = condition.conditions.as_slice() else {
                    return Err(unsupported(
                        "if-let and condition chains require explicit semantics",
                    ));
                };
                let condition = self.expr(condition, env)?;
                require_type(&condition.ty, &boolean())?;
                let yes = self.block(then_branch, env)?;
                let no = self.expr(other, env)?;
                require_type(&yes.ty, &no.ty)?;
                Ok(Value {
                    term: choose(condition.term, yes.ty.clone(), yes.term, no.term),
                    ty: yes.ty,
                })
            }
            ExprKind::Match { expr, arms } => {
                let value = self.expr(expr, env)?;
                let Term::Data { constructors, .. } = &value.ty else {
                    return Err(rejected("non-finite match"));
                };
                let mut cases = vec![None; constructors.len()];
                let mut result = None;
                for arm in arms {
                    if arm.guard.is_some()
                        || arm.with_clause.is_some()
                        || !arm.attributes.is_empty()
                    {
                        return Err(unsupported("guarded or attributed match arms"));
                    }
                    let indices = match &arm.pattern.kind {
                        PatternKind::Wildcard => cases
                            .iter()
                            .enumerate()
                            .filter_map(|(i, c)| c.is_none().then_some(i))
                            .collect(),
                        PatternKind::Literal(lit) if value.ty == boolean() => match lit.kind {
                            LiteralKind::Bool(b) => vec![usize::from(b)],
                            _ => return Err(unsupported("non-Boolean literal pattern")),
                        },
                        PatternKind::Variant {
                            path,
                            data: Maybe::None,
                        } => {
                            let qualified = name(path)?;
                            if qualified.split_once('.').is_some_and(|(owner, _)| {
                                env.iter().any(|(bound, _)| bound == owner)
                            }) {
                                return Err(unsupported("value shadows pattern constructor owner"));
                            }
                            let variant = self.variant(&qualified)?;
                            require_type(&variant.ty, &value.ty)?;
                            let Term::Construct { constructor, .. } = variant.term else {
                                unreachable!()
                            };
                            vec![constructor]
                        }
                        _ => {
                            return Err(unsupported(
                                "match requires qualified fieldless constructors or Boolean literals",
                            ));
                        }
                    };
                    // Validate even unreachable branches before any normalization.
                    let body = self.expr(&arm.body, env)?;
                    if let Some(ty) = &result {
                        require_type(&body.ty, ty)?;
                    }
                    result = Some(body.ty.clone());
                    if indices.is_empty() {
                        return Err(rejected("unreachable match arm"));
                    }
                    for index in indices {
                        if cases[index].replace(body.term.clone()).is_some() {
                            return Err(rejected("overlapping match arms"));
                        }
                    }
                }
                let cases = cases
                    .into_iter()
                    .collect::<Option<Vec<_>>>()
                    .ok_or_else(|| rejected("non-exhaustive match"))?;
                let ty = result.ok_or_else(|| rejected("empty match"))?;
                Ok(Value {
                    term: select(value.term, value.ty, ty.clone(), cases),
                    ty,
                })
            }
            _ => Err(unsupported(
                "expression outside total finite source semantics",
            )),
        }
    }
    fn predicates(&mut self, expressions: &[Expr], env: &mut Env) -> Result<Term> {
        let mut term = truth(true);
        for expression in expressions {
            let value = self.expr(expression, env)?;
            require_type(&value.ty, &boolean())?;
            term = and(term, value.term);
        }
        Ok(term)
    }
}

pub fn elaborate(source: &str, module: &Module, selected: &str) -> Result<Artifact> {
    if !module.attributes.is_empty() {
        return Err(unsupported("module attributes"));
    }
    let mut items = BTreeMap::new();
    for item in &module.items {
        let n = match &item.kind {
            ItemKind::Function(f) => f.name.as_str(),
            ItemKind::Type(t) => t.name.as_str(),
            _ => {
                return Err(unsupported(
                    "finite source modules admit local types and functions only; imports and overloads require resolved identities",
                ));
            }
        };
        if n == "Bool" || items.insert(n.into(), item).is_some() {
            return Err(rejected("duplicate or shadowed declaration"));
        }
    }
    let mut e = Elab {
        source,
        source_hash: sha256(source.as_bytes()),
        items,
        types: BTreeMap::new(),
        variants: BTreeMap::new(),
        functions: BTreeMap::new(),
        active: BTreeSet::new(),
        dependencies: BTreeSet::new(),
        remaining: 20_000,
        selected,
    };
    let f = e.function(selected)?;
    let item = *e
        .items
        .get(selected)
        .ok_or_else(|| rejected("declaration missing"))?;
    let ItemKind::Function(decl) = &item.kind else {
        return Err(rejected("function required"));
    };
    if decl.ensures.is_empty() {
        return Err(unsupported("at least one explicit postcondition required"));
    }
    let (mut env, _) = e.signature(decl)?;
    let pre = e.predicates(&decl.requires, &mut env)?;
    // A checked local definition binds the actual computed result exactly once.
    let actual = (0..env.len())
        .rev()
        .fold(f.term.clone(), |call, i| Term::app(call, Term::var(i)));
    env.push(("result".into(), f.result.clone()));
    let post = e.predicates(&decl.ensures, &mut env)?;
    let post = Term::Let {
        value: Box::new(actual),
        body: Box::new(post),
    };
    let body = choose(pre, boolean(), post, truth(true));
    let predicate = f
        .domains
        .iter()
        .rev()
        .fold(body, |term, ty| Term::lam(ty.clone(), term));
    let certificate = proof::certificate(&f.domains, &predicate, &boolean())?;
    let input_cases = f.domains.iter().try_fold(1usize, |n, ty| {
        let Term::Data { constructors, .. } = ty else {
            return Err(rejected("non-finite parameter"));
        };
        n.checked_mul(constructors.len())
            .ok_or_else(|| Error::Inconclusive {
                code: "finite_case_limit",
                message: "finite input cardinality overflow".into(),
            })
    })?;
    let function_type = f
        .domains
        .iter()
        .rev()
        .fold(f.result, |ty, domain| Term::pi(domain.clone(), ty));
    let dependencies = e
        .dependencies
        .iter()
        .map(|n| e.binding(n))
        .collect::<Result<_>>()?;
    Ok(Artifact {
        certificate,
        source: e.binding(selected)?,
        implementation: ImplementationBinding {
            semantics: PROFILE.into(),
            function: f.term,
            function_type,
            dependencies,
            input_cases,
            requires_count: decl.requires.len(),
            ensures_count: decl.ensures.len(),
        },
    })
}
