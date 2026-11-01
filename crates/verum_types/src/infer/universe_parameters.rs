//! Source-level universe names are lexical binders, never numeric name hashes.

use super::TypeChecker;
use crate::{Result, TypeError, ty::UniverseLevel};
use verum_ast::visitor::{Visitor, walk_type};
use verum_ast::{
    Span,
    ty::{GenericParam, GenericParamKind, TypeBound, TypeBoundKind, TypeKind},
};
use verum_common::{List, Map, Text};

#[derive(Default)]
pub(super) struct UniverseParameterScopes {
    frames: List<UniverseParameterFrame>,
    validation_depth: usize,
    // Exact source identity reuses a binder during preregistration/body passes.
    // Hash collisions in the map cannot equate distinct keys. Module ownership
    // distinguishes independently parsed modules with the same source offsets.
    identities: Map<(Text, Span, Text), u32>,
}

struct UniverseParameterFrame {
    // None is an ordinary generic parameter that shadows an outer level name.
    bindings: Map<Text, Option<u32>>,
    inherit: bool,
}

impl UniverseParameterScopes {
    fn lookup(&self, name: &Text) -> Option<u32> {
        for frame in self.frames.iter().rev() {
            if let Some(binding) = frame.bindings.get(name) {
                return *binding;
            }
            if !frame.inherit {
                break;
            }
        }
        None
    }
}

fn error(message: impl Into<Text>) -> TypeError {
    error_at(message, Span::dummy())
}

fn error_at(message: impl Into<Text>, span: Span) -> TypeError {
    TypeError::UniverseBinding {
        message: message.into(),
        span,
    }
}

impl TypeChecker {
    pub(super) fn with_universe_parameter_scope<T>(
        &mut self,
        parameters: &List<GenericParam>,
        inherit: bool,
        operation: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        if self.universe_parameters.frames.len() >= 64 {
            return Err(error("universe parameter scopes exceed their depth budget"));
        }
        let mut bindings = Map::new();
        for parameter in parameters {
            let name = match &parameter.kind {
                GenericParamKind::Type { name, .. }
                | GenericParamKind::HigherKinded { name, .. }
                | GenericParamKind::Const { name, .. }
                | GenericParamKind::Meta { name, .. }
                | GenericParamKind::Context { name }
                | GenericParamKind::Level { name }
                | GenericParamKind::KindAnnotated { name, .. } => &name.name,
                GenericParamKind::Lifetime { .. } => continue,
            };
            if bindings.contains_key(name) {
                return Err(error_at(
                    format!("duplicate generic parameter `{name}`"),
                    parameter.span,
                ));
            }
            let binding = if matches!(parameter.kind, GenericParamKind::Level { .. }) {
                if !self.universe_poly_enabled {
                    return Err(error(
                        "universe level parameters require `[types].universe_polymorphism = true`",
                    ));
                }
                let key = (
                    self.current_module_path.clone(),
                    parameter.span,
                    name.clone(),
                );
                let id = if let Some(id) = self.universe_parameters.identities.get(&key) {
                    *id
                } else {
                    let UniverseLevel::Variable(id) =
                        self.ctx.fresh_universe_var().map_err(error)?
                    else {
                        return Err(error("universe allocator did not return a variable"));
                    };
                    self.universe_parameters.identities.insert(key, id);
                    id
                };
                self.ctx
                    .universe_ctx_mut()
                    .reserve_variable(id)
                    .map_err(error)?;
                Some(id)
            } else {
                None
            };
            bindings.insert(name.clone(), binding);
        }
        self.universe_parameters
            .frames
            .push(UniverseParameterFrame { bindings, inherit });
        let result = (|| {
            // Bounds may otherwise be deferred or ignored by ordinary type
            // inference. Name binding and feature gating must still be checked.
            for parameter in parameters {
                match &parameter.kind {
                    GenericParamKind::Type {
                        bounds, default, ..
                    } => {
                        self.validate_universe_bounds(bounds)?;
                        if let Some(default) = default {
                            self.validate_universe_annotations(default)?;
                        }
                    }
                    GenericParamKind::HigherKinded { bounds, .. }
                    | GenericParamKind::KindAnnotated { bounds, .. } => {
                        self.validate_universe_bounds(bounds)?
                    }
                    GenericParamKind::Const { ty, .. } | GenericParamKind::Meta { ty, .. } => {
                        self.validate_universe_annotations(ty)?
                    }
                    _ => {}
                }
            }
            operation(self)
        })();
        self.universe_parameters.frames.pop();
        result
    }

    fn validate_universe_bounds(&mut self, bounds: &List<TypeBound>) -> Result<()> {
        if self.universe_parameters.validation_depth >= 64 {
            return Err(error(
                "universe annotation nesting exceeds its depth budget",
            ));
        }
        self.universe_parameters.validation_depth += 1;
        let result = (|| {
            for bound in bounds {
                match &bound.kind {
                    TypeBoundKind::Equality(ty) | TypeBoundKind::GenericProtocol(ty) => {
                        self.validate_universe_annotations(ty)?
                    }
                    TypeBoundKind::AssociatedTypeEquality { eq_type, .. } => {
                        self.validate_universe_annotations(eq_type)?
                    }
                    TypeBoundKind::AssociatedTypeBound { bounds, .. } => {
                        self.validate_universe_bounds(bounds)?
                    }
                    _ => {}
                }
            }
            Ok(())
        })();
        self.universe_parameters.validation_depth -= 1;
        result
    }

    pub(super) fn reject_level_as_type(&self, path: &verum_ast::ty::Path) -> Result<()> {
        if let [verum_ast::ty::PathSegment::Name(name)] = path.segments.as_slice()
            && self.universe_parameters.lookup(&name.name).is_some()
        {
            return Err(error_at(
                format!(
                    "universe level parameter `{}` is not a type; use `Type({})`",
                    name.name, name.name
                ),
                path.span,
            ));
        }
        Ok(())
    }

    /// Lower only declared level names. The bounded explicit stack also prevents
    /// adversarial programmatically constructed ASTs from recursing unchecked.
    pub(super) fn ast_universe_level_to_internal(
        &self,
        expression: &verum_ast::UniverseLevelExpr,
    ) -> Result<UniverseLevel> {
        use verum_ast::UniverseLevelExpr as Ast;
        enum Work<'a> {
            Visit(&'a Ast),
            Max,
            Succ,
        }
        let mut work = List::new();
        work.push(Work::Visit(expression));
        let mut values = List::new();
        let mut count = 0;
        while let Some(step) = work.pop() {
            match step {
                Work::Visit(expr) => {
                    count += 1;
                    if count > crate::universe_level::MAX_LEVEL_NODES {
                        return Err(error("universe level expression exceeds its node budget"));
                    }
                    match expr {
                        Ast::Concrete(value) => values.push(UniverseLevel::Concrete(*value)),
                        Ast::Variable(name) => {
                            if !self.universe_poly_enabled {
                                return Err(error(
                                    "universe-polymorphic types require `[types].universe_polymorphism = true`",
                                ));
                            }
                            let id =
                                self.universe_parameters.lookup(&name.name).ok_or_else(|| {
                                    error_at(
                                        format!("unbound universe level parameter `{}`", name.name),
                                        name.span,
                                    )
                                })?;
                            values.push(UniverseLevel::Variable(id));
                        }
                        Ast::Max(a, b) => {
                            work.push(Work::Max);
                            work.push(Work::Visit(b));
                            work.push(Work::Visit(a));
                        }
                        Ast::Succ(inner) => {
                            work.push(Work::Succ);
                            work.push(Work::Visit(inner));
                        }
                    }
                }
                Work::Max => {
                    let right = values
                        .pop()
                        .ok_or_else(|| error("invalid universe expression"))?;
                    let left = values
                        .pop()
                        .ok_or_else(|| error("invalid universe expression"))?;
                    values.push(UniverseLevel::Max(Box::new(left), Box::new(right)));
                }
                Work::Succ => {
                    let inner = values
                        .pop()
                        .ok_or_else(|| error("invalid universe expression"))?;
                    values.push(UniverseLevel::Succ(Box::new(inner)));
                }
            }
        }
        values
            .pop()
            .ok_or_else(|| error("empty universe expression"))
    }

    pub(super) fn validate_universe_annotations(&mut self, ty: &verum_ast::ty::Type) -> Result<()> {
        let mut validator = UniverseAnnotationValidator {
            checker: self,
            result: Ok(()),
        };
        validator.visit_type(ty);
        validator.result
    }

    pub(super) fn validate_function_universes(
        &mut self,
        function: &verum_ast::FunctionDecl,
    ) -> Result<()> {
        let mut validator = UniverseAnnotationValidator {
            checker: self,
            result: Ok(()),
        };
        validator.visit_function(function);
        validator.result
    }
}

/// Validate universe binding before a signature's unrelated unresolved nominal
/// types take a lenient import path. This does not assert its universe bounds.
struct UniverseAnnotationValidator<'a> {
    checker: &'a mut TypeChecker,
    result: Result<()>,
}

impl Visitor for UniverseAnnotationValidator<'_> {
    fn visit_type(&mut self, ty: &verum_ast::ty::Type) {
        if self.result.is_err() {
            return;
        }
        if self.checker.universe_parameters.validation_depth >= 64 {
            self.result = Err(error(
                "universe annotation nesting exceeds its depth budget",
            ));
            return;
        }
        self.checker.universe_parameters.validation_depth += 1;
        self.result = match &ty.kind {
            TypeKind::Universe { level: Some(level) } => self
                .checker
                .ast_universe_level_to_internal(level)
                .map(|_| ()),
            TypeKind::Path(path) => self.checker.reject_level_as_type(path),
            TypeKind::Rank2Function {
                type_params,
                params,
                return_type,
                ..
            } => self
                .checker
                .with_universe_parameter_scope(type_params, true, |checker| {
                    for parameter in params {
                        checker.validate_universe_annotations(parameter)?;
                    }
                    checker.validate_universe_annotations(return_type)
                }),
            _ => {
                walk_type(self, ty);
                self.checker.universe_parameters.validation_depth -= 1;
                return;
            }
        };
        self.checker.universe_parameters.validation_depth -= 1;
    }

    fn visit_expr(&mut self, _: &verum_ast::Expr) {}
    fn visit_block(&mut self, _: &verum_ast::Block) {}
}
