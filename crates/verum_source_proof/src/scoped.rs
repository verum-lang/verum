//! Isolated proof-only profile. Uses the production parser and kernels; no new proof checker.
use std::collections::{BTreeMap, BTreeSet};
use verum_ast::decl::{FunctionParamKind, ProofBody, TacticExpr, TheoremDecl};
use verum_ast::pattern::{Pattern, PatternKind};
use verum_ast::ty::{GenericParamKind, Type, TypeKind, UniverseLevelExpr};
use verum_ast::{BinOp, Expr, ExprKind, Path, PathSegment};
use verum_common::{Maybe, span::Span};
use verum_proof::proof_checker::{self as k, Certificate, Context, Level, Term};

mod tactics;

pub const PROFILE: &str = "verum-source-pi-id/1";
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Unsupported(String),
    Scope(String),
    Rejected(String),
}
type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct BinderId(pub u64);
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ir {
    Var(BinderId),
    Universe(Level),
    Pi {
        id: BinderId,
        domain: Box<Ir>,
        body: Box<Ir>,
    },
    Lam {
        id: BinderId,
        domain: Box<Ir>,
        body: Box<Ir>,
    },
    App(Box<Ir>, Box<Ir>),
    Id {
        carrier: Box<Ir>,
        left: Box<Ir>,
        right: Box<Ir>,
    },
    Refl {
        value: Box<Ir>,
    },
    /// Equality transport through an endpoint-dependent type family.
    J {
        motive: Box<Ir>,
        base: Box<Ir>,
        scrutinee: Box<Ir>,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Type,
    Value,
    Premise,
}
#[derive(Debug, Clone)]
pub struct Binding {
    pub id: BinderId,
    pub name: Option<String>,
    pub domain: Ir,
    pub role: Role,
}
#[derive(Debug)]
pub struct Artifact {
    pub certificate: Certificate,
    pub telescope: Vec<Binding>,
    pub level_bindings: Vec<(String, String)>,
    pub scoped_type: Ir,
    pub scoped_proof: Ir,
    pub conclusion: Term,
}
#[derive(Default)]
struct Elab {
    next: u64,
    stack: Vec<Binding>,
    levels: BTreeMap<String, String>,
    error_span: Option<Span>,
}
fn unsupported(s: impl Into<String>) -> Error {
    Error::Unsupported(s.into())
}
fn path_name(p: &Path) -> Result<&str> {
    match p.segments.as_slice() {
        [PathSegment::Name(n)] => Ok(n.as_str()),
        _ => Err(unsupported(
            "only unqualified bound identifiers are in this profile",
        )),
    }
}
fn pattern_name(p: &Pattern) -> Result<&str> {
    match &p.kind {
        PatternKind::Ident {
            by_ref: false,
            mutable: false,
            name,
            subpattern: Maybe::None,
        } => Ok(name.as_str()),
        _ => Err(unsupported(
            "only immutable plain binder patterns are in this profile",
        )),
    }
}
impl Elab {
    fn fresh(&mut self) -> BinderId {
        let id = BinderId(self.next);
        self.next += 1;
        id
    }
    fn frame(&self) -> Vec<BinderId> {
        self.stack.iter().map(|b| b.id).collect()
    }
    fn context(&self) -> Result<Context> {
        let mut frame = Vec::new();
        let mut ctx = Context::new();
        for b in &self.stack {
            let ty = lower(&b.domain, &mut frame)?;
            ctx = ctx.extend(ty);
            frame.push(b.id);
        }
        Ok(ctx)
    }
    fn infer(&self, ir: &Ir) -> Result<Term> {
        k::infer(&self.context()?, &lower(ir, &mut self.frame())?)
            .map_err(|e| Error::Rejected(format!("{e:?}")))
    }
    fn require_type(&self, ir: &Ir) -> Result<()> {
        match self.infer(ir)? {
            Term::Universe(_) => Ok(()),
            x => Err(Error::Rejected(format!(
                "expected a type, found kind {x:?}"
            ))),
        }
    }
    fn bind(&mut self, name: Option<String>, domain: Ir, premise: bool) -> Result<Binding> {
        self.require_type(&domain)?;
        let role = if premise {
            Role::Premise
        } else if matches!(domain, Ir::Universe(_)) {
            Role::Type
        } else {
            Role::Value
        };
        let b = Binding {
            id: self.fresh(),
            name,
            domain,
            role,
        };
        self.stack.push(b.clone());
        Ok(b)
    }
    fn lookup(&self, name: &str) -> Result<Ir> {
        self.stack
            .iter()
            .rev()
            .find(|b| b.name.as_deref() == Some(name))
            .map(|b| Ir::Var(b.id))
            .ok_or_else(|| Error::Scope(format!("unbound identifier {name}")))
    }
    fn level(&self, l: &UniverseLevelExpr) -> Result<Level> {
        Ok(match l {
            UniverseLevelExpr::Concrete(n) => Level::Concrete(*n),
            UniverseLevelExpr::Variable(n) => Level::Var(
                self.levels
                    .get(n.as_str())
                    .ok_or_else(|| Error::Scope(format!("unbound universe {}", n.as_str())))?
                    .clone(),
            ),
            UniverseLevelExpr::Max(a, b) => {
                Level::Max(Box::new(self.level(a)?), Box::new(self.level(b)?))
            }
            UniverseLevelExpr::Succ(a) => Level::Succ(Box::new(self.level(a)?)),
        })
    }
    fn locate(&mut self, error: Error, span: Span) -> Error {
        self.error_span.get_or_insert(span);
        error
    }
    fn ty(&mut self, t: &Type) -> Result<Ir> {
        self.ty_inner(t).map_err(|error| self.locate(error, t.span))
    }
    fn ty_inner(&mut self, t: &Type) -> Result<Ir> {
        let ir = match &t.kind {
            TypeKind::Universe { level } => Ir::Universe(match level {
                Maybe::None => Level::Concrete(0),
                Maybe::Some(l) => self.level(l)?,
            }),
            TypeKind::Path(p) => self.lookup(path_name(p)?)?,
            TypeKind::DependentApp {
                carrier,
                value_args,
            } => {
                // The family head has a Pi type, not a universe: do not call
                // `ty(carrier)` which would wrongly require it to be a type.
                let mut family = match &carrier.kind {
                    TypeKind::Path(p) => self.lookup(path_name(p)?)?,
                    _ => {
                        return Err(unsupported(
                            "dependent family head must be explicitly bound; implicit generic arguments are unsupported",
                        ));
                    }
                };
                if value_args.is_empty() {
                    return Err(unsupported("empty dependent application"));
                }
                for argument in value_args {
                    family = Ir::App(Box::new(family), Box::new(self.expr(argument)?));
                }
                family
            }
            TypeKind::Function {
                params,
                return_type,
                calling_convention,
                contexts,
            } => {
                if calling_convention.is_some() || !contexts.is_empty() {
                    return Err(unsupported(
                        "external/contextual functions are not proof transformers",
                    ));
                }
                if params.is_empty() {
                    return Err(unsupported(
                        "zero-argument runtime calls need explicit Unit semantics",
                    ));
                }
                let mut args = Vec::new();
                for p in params {
                    args.push((self.fresh(), self.ty(p)?));
                }
                let mut body = self.ty(return_type)?;
                for (id, domain) in args.into_iter().rev() {
                    body = Ir::Pi {
                        id,
                        domain: Box::new(domain),
                        body: Box::new(body),
                    };
                }
                body
            }
            TypeKind::Rank2Function {
                type_params,
                params,
                return_type,
                calling_convention,
                contexts,
                where_clause,
            } => {
                if calling_convention.is_some()
                    || !contexts.is_empty()
                    || where_clause.is_some()
                    || params.is_empty()
                {
                    return Err(unsupported(
                        "rank-2 proof functions need explicit domains, no effects/constraints and a nonempty argument list",
                    ));
                }
                let saved = self.stack.len();
                let result = (|| {
                    let mut bound = Vec::new();
                    for p in type_params {
                        if p.is_implicit {
                            return Err(unsupported(
                                "implicit rank-2 parameters require an elaboration contract",
                            ));
                        }
                        let (name, ty) = match &p.kind {
                            GenericParamKind::Meta {
                                name,
                                ty,
                                refinement: Maybe::None,
                            } => (name, ty),
                            _ => {
                                return Err(unsupported(
                                    "rank-2 binders require explicit `name: meta Type` or `name: meta A` domains",
                                ));
                            }
                        };
                        let domain = self.ty(ty)?;
                        bound.push(self.bind(Some(name.as_str().into()), domain, false)?);
                    }
                    let mut ordinary = Vec::new();
                    for p in params {
                        ordinary.push((self.fresh(), self.ty(p)?));
                    }
                    let mut body = self.ty(return_type)?;
                    for (id, domain) in ordinary.into_iter().rev() {
                        body = Ir::Pi {
                            id,
                            domain: Box::new(domain),
                            body: Box::new(body),
                        };
                    }
                    for b in bound.into_iter().rev() {
                        body = Ir::Pi {
                            id: b.id,
                            domain: Box::new(b.domain),
                            body: Box::new(body),
                        };
                    }
                    Ok(body)
                })();
                self.stack.truncate(saved);
                result?
            }
            TypeKind::PathType { carrier, lhs, rhs } => Ir::Id {
                carrier: Box::new(self.ty(carrier)?),
                left: Box::new(self.expr(lhs)?),
                right: Box::new(self.expr(rhs)?),
            },
            _ => {
                return Err(unsupported(format!(
                    "type form outside pure Pi/Id profile: {:?}",
                    t.kind
                )));
            }
        };
        self.require_type(&ir)?;
        Ok(ir)
    }
    fn expr(&mut self, e: &Expr) -> Result<Ir> {
        self.expr_inner(e)
            .map_err(|error| self.locate(error, e.span))
    }
    fn expr_inner(&mut self, e: &Expr) -> Result<Ir> {
        match &e.kind {
            // Grouping does not add a logical rule: elaborate the enclosed term
            // and retain its source range if it fails.
            ExprKind::Paren(inner) => self.expr(inner),
            ExprKind::Path(p) => self.lookup(path_name(p)?),
            ExprKind::TypeExpr(t) => self.ty(t),
            ExprKind::Call {
                func,
                type_args,
                args,
            } => {
                if !type_args.is_empty() {
                    return Err(unsupported(
                        "implicit type argument insertion is not implemented",
                    ));
                }
                if args.is_empty() {
                    return Err(unsupported("empty calls need explicit Unit semantics"));
                }
                let mut x = self.expr(func)?;
                for a in args {
                    x = Ir::App(Box::new(x), Box::new(self.expr(a)?));
                }
                self.infer(&x)?;
                Ok(x)
            }
            ExprKind::Binary {
                op: BinOp::Eq,
                left,
                right,
            } => {
                let left = self.expr(left)?;
                let right = self.expr(right)?;
                let ty = self.infer(&left)?;
                let mut frame = self.frame();
                let carrier = self.raise(&ty, &mut frame)?;
                let id = Ir::Id {
                    carrier: Box::new(carrier),
                    left: Box::new(left),
                    right: Box::new(right),
                };
                self.require_type(&id)?;
                Ok(id)
            }
            ExprKind::Closure {
                async_: false,
                move_: false,
                params,
                contexts,
                return_type: Maybe::None,
                body,
            } if contexts.is_empty() => {
                let saved = self.stack.len();
                let mut bound = Vec::new();
                let result = (|| {
                    for p in params {
                        let name = pattern_name(&p.pattern)?.to_owned();
                        let ty = match &p.ty {
                            Maybe::Some(t) => self.ty(t)?,
                            _ => return Err(unsupported("closure parameter needs explicit type")),
                        };
                        bound.push(self.bind(Some(name), ty, false)?);
                    }
                    let mut b = self.expr(body)?;
                    for x in bound.into_iter().rev() {
                        b = Ir::Lam {
                            id: x.id,
                            domain: Box::new(x.domain),
                            body: Box::new(b),
                        };
                    }
                    Ok(b)
                })();
                self.stack.truncate(saved);
                result
            }
            ExprKind::Forall { bindings, body } => {
                let saved = self.stack.len();
                let mut bound = Vec::new();
                let result = (|| {
                    for b in bindings {
                        if b.domain.is_some() || b.guard.is_some() {
                            return Err(unsupported(
                                "bounded/guarded quantifiers require separate semantics",
                            ));
                        }
                        let ty = match &b.ty {
                            Maybe::Some(t) => self.ty(t)?,
                            _ => return Err(unsupported("quantifier needs explicit type")),
                        };
                        bound.push(self.bind(
                            Some(pattern_name(&b.pattern)?.to_owned()),
                            ty,
                            false,
                        )?);
                    }
                    let mut b = self.expr(body)?;
                    self.require_type(&b)?;
                    for x in bound.into_iter().rev() {
                        b = Ir::Pi {
                            id: x.id,
                            domain: Box::new(x.domain),
                            body: Box::new(b),
                        };
                    }
                    Ok(b)
                })();
                self.stack.truncate(saved);
                result
            }
            _ => Err(unsupported(format!(
                "expression form outside pure Pi/Id profile: {:?}",
                e.kind
            ))),
        }
    }
    fn raise(&mut self, t: &Term, frame: &mut Vec<BinderId>) -> Result<Ir> {
        Ok(match t {
            Term::Universe(l) => Ir::Universe(l.clone()),
            Term::Var(i) => Ir::Var(
                *frame
                    .iter()
                    .rev()
                    .nth(*i)
                    .ok_or_else(|| Error::Scope(format!("kernel result escaped index {i}")))?,
            ),
            Term::Pi(a, b) | Term::Lam(a, b) => {
                let dom = self.raise(a, frame)?;
                let id = self.fresh();
                frame.push(id);
                let body = self.raise(b, frame)?;
                frame.pop();
                if matches!(t, Term::Pi(..)) {
                    Ir::Pi {
                        id,
                        domain: Box::new(dom),
                        body: Box::new(body),
                    }
                } else {
                    Ir::Lam {
                        id,
                        domain: Box::new(dom),
                        body: Box::new(body),
                    }
                }
            }
            Term::App(a, b) => Ir::App(
                Box::new(self.raise(a, frame)?),
                Box::new(self.raise(b, frame)?),
            ),
            Term::Id { ty, lhs, rhs } => Ir::Id {
                carrier: Box::new(self.raise(ty, frame)?),
                left: Box::new(self.raise(lhs, frame)?),
                right: Box::new(self.raise(rhs, frame)?),
            },
            Term::Refl(val) => Ir::Refl {
                value: Box::new(self.raise(val, frame)?),
            },
            Term::J {
                motive,
                base,
                scrutinee,
            } => Ir::J {
                motive: Box::new(self.raise(motive, frame)?),
                base: Box::new(self.raise(base, frame)?),
                scrutinee: Box::new(self.raise(scrutinee, frame)?),
            },
            _ => return Err(unsupported("kernel type outside selected fragment")),
        })
    }
    fn proof(&mut self, p: &ProofBody, goal: &Ir) -> Result<Ir> {
        match p {
            ProofBody::Term(e) => self.expr(e),
            ProofBody::Tactic(tactic) => self.tactics(tactic, goal),
            _ => Err(unsupported(
                "structured proof steps require explicit elaboration; use a term or supported tactics",
            )),
        }
    }
}
/// The only binder-coordinate conversion. An IR binder has one immutable identity.
pub fn lower(t: &Ir, frame: &mut Vec<BinderId>) -> Result<Term> {
    Ok(match t {
        Ir::Var(id) => Term::var(
            frame
                .iter()
                .rev()
                .position(|x| x == id)
                .ok_or_else(|| Error::Scope(format!("escaped binder {id:?}")))?,
        ),
        Ir::Universe(l) => Term::Universe(l.clone()),
        Ir::Pi { id, domain, body } | Ir::Lam { id, domain, body } => {
            if frame.contains(id) {
                return Err(Error::Scope("binder identity reused".into()));
            }
            let d = lower(domain, frame)?;
            frame.push(*id);
            let b = lower(body, frame);
            frame.pop();
            let b = b?;
            if matches!(t, Ir::Pi { .. }) {
                Term::pi(d, b)
            } else {
                Term::lam(d, b)
            }
        }
        Ir::App(a, b) => Term::app(lower(a, frame)?, lower(b, frame)?),
        Ir::Id {
            carrier,
            left,
            right,
        } => Term::Id {
            ty: Box::new(lower(carrier, frame)?),
            lhs: Box::new(lower(left, frame)?),
            rhs: Box::new(lower(right, frame)?),
        },
        Ir::Refl { value } => Term::Refl(Box::new(lower(value, frame)?)),
        Ir::J {
            motive,
            base,
            scrutinee,
        } => Term::j(
            lower(motive, frame)?,
            lower(base, frame)?,
            lower(scrutinee, frame)?,
        ),
    })
}
/// Caller must explicitly select this proof-only profile; it is not the global runtime fn semantics.
/// Diagnostics preserve the innermost source range without changing logical errors.
#[derive(Debug)]
pub struct LocatedError {
    pub error: Error,
    pub span: Option<Span>,
}
pub fn elaborate(theorem: &TheoremDecl, profile: &str) -> Result<Artifact> {
    elaborate_diagnostic(theorem, profile).map_err(|located| located.error)
}
pub fn elaborate_diagnostic(
    theorem: &TheoremDecl,
    profile: &str,
) -> std::result::Result<Artifact, LocatedError> {
    let mut e = Elab::default();
    elaborate_in(theorem, profile, &mut e).map_err(|error| LocatedError {
        error,
        span: e.error_span,
    })
}
fn elaborate_in(theorem: &TheoremDecl, profile: &str, e: &mut Elab) -> Result<Artifact> {
    if profile != PROFILE {
        return Err(unsupported(
            "explicit proof-only Pi/Id profile selection required",
        ));
    }
    if theorem.return_type.is_some()
        || theorem.generic_where_clause.is_some()
        || theorem.meta_where_clause.is_some()
        || !theorem.attributes.is_empty()
    {
        return Err(unsupported(
            "return annotations, constraints and attributes need explicit semantics",
        ));
    }
    if theorem.ensures.len() > 1
        || theorem
            .ensures
            .first()
            .is_some_and(|goal| goal.span != theorem.proposition.span)
    {
        return Err(unsupported(
            "multiple or separate ensures and explicit propositions require conjunction elaboration; none may be discarded",
        ));
    }
    let mut names = BTreeSet::new();
    let mut levels = Vec::new();
    for g in &theorem.generics {
        match &g.kind {
            GenericParamKind::Level { name } if !g.is_implicit => {
                if !names.insert(name.as_str().to_owned()) {
                    return Err(Error::Scope("duplicate declaration binder".into()));
                }
                let internal = format!("level_{}", levels.len());
                e.levels.insert(name.as_str().into(), internal.clone());
                levels.push((name.as_str().into(), internal));
            }
            _ => {
                return Err(unsupported(
                    "generic kinds/defaults/constraints require exact elaboration; use explicit universe binders and A:Type(u)",
                ));
            }
        }
    }
    for p in &theorem.params {
        if !p.attributes.is_empty() {
            return Err(unsupported("parameter attributes"));
        }
        match &p.kind {
            FunctionParamKind::Regular {
                pattern,
                ty,
                default_value: Maybe::None,
            } => {
                let name = pattern_name(pattern)?;
                if !names.insert(name.to_owned()) {
                    return Err(Error::Scope("duplicate declaration binder".into()));
                }
                let ty = e.ty(ty)?;
                e.bind(Some(name.to_owned()), ty, false)?;
            }
            _ => {
                return Err(unsupported(
                    "only explicit immutable parameters without defaults",
                ));
            }
        }
    }
    for pre in &theorem.requires {
        let p = e.expr(pre)?;
        e.bind(None, p, true)?;
    }
    let goal = e.expr(&theorem.proposition)?;
    e.require_type(&goal)
        .map_err(|error| e.locate(error, theorem.proposition.span))?;
    let proof = match &theorem.proof {
        Maybe::Some(p) => e.proof(p, &goal)?,
        _ => return Err(unsupported("proof body missing")),
    };
    let telescope = e.stack.clone();
    let conclusion = lower(&goal, &mut e.frame())?;
    let mut scoped_type = goal;
    let mut scoped_proof = proof;
    for b in telescope.iter().rev() {
        scoped_type = Ir::Pi {
            id: b.id,
            domain: Box::new(b.domain.clone()),
            body: Box::new(scoped_type),
        };
        scoped_proof = Ir::Lam {
            id: b.id,
            domain: Box::new(b.domain.clone()),
            body: Box::new(scoped_proof),
        };
    }
    let mut metadata = BTreeMap::new();
    metadata.insert("profile".into(), PROFILE.into());
    metadata.insert("declaration".into(), theorem.name.as_str().into());
    let proof_span = match &theorem.proof {
        Maybe::Some(ProofBody::Term(term) | ProofBody::Tactic(TacticExpr::Exact(term))) => {
            Some(term.span)
        }
        _ => None,
    };
    let certificate = Certificate {
        term: lower(&scoped_proof, &mut Vec::new())?,
        claimed_type: lower(&scoped_type, &mut Vec::new())?,
        metadata,
    };
    certificate.verify().map_err(|x| {
        e.error_span = e.error_span.or(proof_span);
        Error::Rejected(format!("substitution kernel: {x:?}"))
    })?;
    verum_proof::proof_checker_nbe::verify_certificate(&certificate).map_err(|x| {
        e.error_span = e.error_span.or(proof_span);
        Error::Rejected(format!("NbE kernel: {x:?}"))
    })?;
    Ok(Artifact {
        certificate,
        telescope,
        level_bindings: levels,
        scoped_type,
        scoped_proof,
        conclusion,
    })
}
