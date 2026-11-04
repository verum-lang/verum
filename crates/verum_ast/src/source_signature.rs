//! Declaration-owned signatures that retain source universe binders.
//!
//! These checks establish a bounded, closed annotation carrier, not that its
//! constraints hold. Consumers must elaborate it in the declaring module and
//! instantiate its quantified levels independently at each use.

use crate::decl::{FunctionDecl, FunctionParamKind};
use crate::ty::{
    GenericParam, GenericParamKind, TypeBound, TypeBoundKind, TypeKind, UniverseLevelExpr,
    WherePredicateKind,
};
use crate::visitor::{Visitor, walk_expr, walk_type};
use serde::Serialize;
use verum_common::{List, Map, Text};

type Result<T> = std::result::Result<T, Text>;

/// Determine whether a source signature needs the universe-preserving carrier.
/// The executable function body is neither inspected nor cloned.
pub fn signature_uses_universes(function: &FunctionDecl) -> Result<bool> {
    check_size(function)?;
    let mut scan = SignatureScan::new(false);
    scan.visit_function(function);
    scan.finish().map(|_| scan.uses_universes)
}

/// Detect universe annotations in an independently declared type, such as a
/// static or constant annotation, without interpreting rendered names.
pub fn type_uses_universes(ty: &crate::Type) -> Result<bool> {
    Budget::new().visit(ty, 0).map_err(|error| error.0)?;
    let mut scan = SignatureScan::new(false);
    scan.visit_type(ty);
    scan.finish().map(|_| scan.uses_universes)
}

/// Detect universe semantics in a legacy cached type declaration. A cache may
/// refuse this declaration when it lacks a structured carrier; guessing from
/// rendered names or silently treating levels as ordinary type slots is unsound.
pub fn type_declaration_uses_universes(declaration: &crate::TypeDecl) -> Result<bool> {
    check_type_declaration_size(declaration)?;
    let mut scan = SignatureScan::new(false);
    scan.visit_type_decl(declaration);
    scan.finish().map(|_| scan.uses_universes)
}

/// Defaults belong to their source telescope, including its lexical scope and
/// prior binders. Value kinds, sorts and refinements likewise cannot be inferred
/// from runtime type slots. Keep every nominal value telescope for exact
/// application elaboration.
pub fn type_declaration_needs_semantic_carrier(declaration: &crate::TypeDecl) -> Result<bool> {
    let universes = type_declaration_uses_universes(declaration)?;
    let mut scan = SignatureScan::new(false);
    scan.visit_type_decl(declaration);
    scan.finish()?;
    Ok(universes || scan.uses_applied_projection || scan.uses_rank_two || scan.uses_callable_contexts
        || matches!(&declaration.body, crate::TypeDeclBody::Protocol(body) | crate::TypeDeclBody::Coinductive(body)
            if body.items.iter().any(|item| matches!(&item.kind, crate::decl::ProtocolItemKind::Type { .. })))
        || declaration.generics.iter().any(|param| {
            matches!(
                param.kind,
                GenericParamKind::Type {
                    default: Some(_),
                    ..
                } | GenericParamKind::Meta { .. } | GenericParamKind::Const { .. }
            )
        }))
}

/// Preserve declaration semantics that the runtime descriptor erases. Protocol
/// methods retain their signatures; executable bodies never enter the carrier.
pub fn source_type_declaration(declaration: &crate::TypeDecl) -> Result<Option<crate::TypeDecl>> {
    if !type_declaration_needs_semantic_carrier(declaration)
        .map_err(|message| type_declaration_error(declaration, message))?
    {
        return Ok(None);
    }
    validate_type_declaration(declaration, true)
        .map_err(|message| type_declaration_error(declaration, message))?;
    Ok(Some(copy_type_declaration(declaration)))
}

/// Validate a complete decoded declaration before replay. This establishes
/// bounded syntax and lexical level closure, not its typing or proof obligations.
/// Executable method bodies and nominal universe parameters are refused;
/// consumers must not fall back to erased metadata on failure.
pub fn validate_source_type_declaration(declaration: &crate::TypeDecl) -> Result<()> {
    validate_type_declaration(declaration, false)
        .map_err(|message| type_declaration_error(declaration, message))
}

fn validate_type_declaration(
    declaration: &crate::TypeDecl,
    allow_source_bodies: bool,
) -> Result<()> {
    use crate::decl::{ProtocolItemKind, TypeDeclBody};
    check_type_declaration_size(declaration)?;
    let name = declaration.name.name.as_str();
    if name.is_empty() || name.contains('.') || name.contains("::") {
        return Err("nominal carrier name must be a nonempty unqualified source identifier".into());
    }
    if declaration
        .generics
        .iter()
        .any(|parameter| matches!(parameter.kind, GenericParamKind::Level { .. }))
    {
        return Err("nominal universe parameters require unsupported level instantiation".into());
    }
    if !allow_source_bodies {
        if let TypeDeclBody::Protocol(body) | TypeDeclBody::Coinductive(body) = &declaration.body {
            for item in &body.items {
                if let ProtocolItemKind::Function { decl, default_impl } = &item.kind {
                    if decl.body.is_some() || default_impl.is_some() {
                        return Err(format!(
                            "method `{}` must not contain an executable body in metadata",
                            decl.name.name
                        )
                        .into());
                    }
                }
            }
        }
    }
    let mut scan = SignatureScan::new(true);
    scan.visit_type_decl(declaration);
    scan.finish()?;
    supported(&scan, false)
}

fn type_declaration_error(declaration: &crate::TypeDecl, message: Text) -> Text {
    format!("type declaration `{}`: {message}", declaration.name.name).into()
}

fn copy_type_declaration(declaration: &crate::TypeDecl) -> crate::TypeDecl {
    use crate::decl::{ProtocolBody, ProtocolItem, ProtocolItemKind, TypeDeclBody};
    let body = match &declaration.body {
        TypeDeclBody::Protocol(body) | TypeDeclBody::Coinductive(body) => {
            let copied = ProtocolBody {
                is_context: body.is_context,
                extends: body.extends.clone(),
                generic_where_clause: body.generic_where_clause.clone(),
                items: body
                    .items
                    .iter()
                    .map(|item| match &item.kind {
                        ProtocolItemKind::Function { decl, .. } => ProtocolItem {
                            kind: ProtocolItemKind::Function {
                                decl: copy_function_signature(decl),
                                default_impl: None,
                            },
                            span: item.span,
                        },
                        _ => item.clone(),
                    })
                    .collect(),
            };
            if matches!(declaration.body, TypeDeclBody::Protocol(_)) {
                TypeDeclBody::Protocol(copied)
            } else {
                TypeDeclBody::Coinductive(copied)
            }
        }
        body => body.clone(),
    };
    crate::TypeDecl {
        visibility: declaration.visibility.clone(),
        name: declaration.name.clone(),
        generics: declaration.generics.clone(),
        attributes: declaration.attributes.clone(),
        body,
        value_params: declaration.value_params.clone(),
        resource_modifier: declaration.resource_modifier,
        generic_where_clause: declaration.generic_where_clause.clone(),
        meta_where_clause: declaration.meta_where_clause.clone(),
        span: declaration.span,
    }
}

/// Capture the entire declaration signature, with only its executable body
/// removed. Ordinary signatures retain their established metadata path.
pub fn source_signature(function: &FunctionDecl, is_method: bool) -> Result<Option<FunctionDecl>> {
    check_size(function)?;
    let mut needs = SignatureScan::new(false);
    needs.visit_function(function);
    needs.finish()?;
    if !needs.uses_universes && !needs.uses_applied_projection && !needs.uses_rank_two && !needs.uses_callable_contexts {
        return Ok(None);
    }
    let mut scan = SignatureScan::new(true);
    scan.visit_function(function);
    scan.finish()?;
    supported(&scan, is_method || function.is_method())?;
    // Do not clone the body just to discard it: it is outside the carrier's
    // size budget and can contain arbitrarily larger executable syntax.
    Ok(Some(copy_function_signature(function)))
}

fn copy_function_signature(function: &FunctionDecl) -> FunctionDecl {
    FunctionDecl {
        visibility: function.visibility.clone(),
        is_async: function.is_async,
        is_meta: function.is_meta,
        stage_level: function.stage_level,
        is_pure: function.is_pure,
        is_generator: function.is_generator,
        is_cofix: function.is_cofix,
        is_unsafe: function.is_unsafe,
        is_transparent: function.is_transparent,
        extern_abi: function.extern_abi.clone(),
        is_variadic: function.is_variadic,
        name: function.name.clone(),
        generics: function.generics.clone(),
        params: function.params.clone(),
        return_type: function.return_type.clone(),
        throws_clause: function.throws_clause.clone(),
        std_attr: function.std_attr.clone(),
        contexts: function.contexts.clone(),
        generic_where_clause: function.generic_where_clause.clone(),
        meta_where_clause: function.meta_where_clause.clone(),
        decreases: function.decreases.clone(),
        requires: function.requires.clone(),
        ensures: function.ensures.clone(),
        attributes: function.attributes.clone(),
        body: None,
        span: function.span,
    }
}

/// Validate a decoded universe signature before using any of its annotations.
/// A body, free/wrong-kind level reference, unsupported binding scope or universe
/// kind bound, or budget failure is an error; it must not activate a lossy
/// metadata fallback.
pub fn validate_source_signature(function: &FunctionDecl, is_method: bool) -> Result<()> {
    if function.body.is_some() {
        return Err("source signature must not contain a function body".into());
    }
    check_size(function)?;
    let mut scan = SignatureScan::new(true);
    scan.visit_function(function);
    scan.finish()?;
    supported(&scan, is_method || function.is_method())
}

fn supported(scan: &SignatureScan, is_method: bool) -> Result<()> {
    if scan.uses_universes && is_method {
        return Err("universe-bearing method signatures are not supported in metadata".into());
    }
    if scan.universe_in_rank_two {
        return Err("universe-bearing rank-2 signatures are not supported in metadata".into());
    }
    Ok(())
}

struct SignatureScan {
    validate: bool,
    uses_universes: bool,
    uses_applied_projection: bool,
    uses_rank_two: bool,
    uses_callable_contexts: bool,
    universe_in_rank_two: bool,
    rank_two_depth: usize,
    type_bound_depth: usize,
    nominal_depth: usize,
    direct_nominal_method: bool,
    scopes: List<Map<Text, bool>>,
    error: Option<Text>,
}

impl SignatureScan {
    fn new(validate: bool) -> Self {
        Self {
            validate,
            uses_universes: false,
            uses_applied_projection: false,
            uses_rank_two: false,
            uses_callable_contexts: false,
            universe_in_rank_two: false,
            rank_two_depth: 0,
            type_bound_depth: 0,
            nominal_depth: 0,
            direct_nominal_method: false,
            scopes: List::new(),
            error: None,
        }
    }
    fn finish(&self) -> Result<()> {
        self.error.clone().map_or(Ok(()), Err)
    }
    fn fail(&mut self, message: impl Into<Text>) {
        if self.error.is_none() {
            self.error = Some(message.into());
        }
    }
    fn universe(&mut self) {
        self.uses_universes = true;
        self.universe_in_rank_two |= self.rank_two_depth != 0;
        if self.validate && self.type_bound_depth != 0 {
            // Lexical closure alone does not establish the formation judgment
            // promised by a kind bound, even for a concrete universe level.
            self.fail("universe-containing type bounds require unsupported formation checking");
        }
    }
    fn bound(&self, name: &Text) -> Option<bool> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).copied())
    }
    fn push_generics(&mut self, parameters: &List<GenericParam>) {
        let mut names = Map::new();
        for parameter in parameters {
            let name = match &parameter.kind {
                GenericParamKind::Type { name, .. }
                | GenericParamKind::HigherKinded { name, .. }
                | GenericParamKind::KindAnnotated { name, .. }
                | GenericParamKind::Const { name, .. }
                | GenericParamKind::Meta { name, .. }
                | GenericParamKind::Context { name }
                | GenericParamKind::Level { name } => &name.name,
                GenericParamKind::Lifetime { .. } => continue,
            };
            let is_level = matches!(parameter.kind, GenericParamKind::Level { .. });
            if is_level {
                self.universe();
                if self.validate && self.nominal_depth != 0 {
                    self.fail("universe binders inside nominal declarations require unsupported level instantiation");
                }
            }
            if names.insert(name.clone(), is_level).is_some() && self.validate {
                self.fail(format!(
                    "duplicate generic binder `{name}` in source signature"
                ));
            }
        }
        self.scopes.push(names);
        for parameter in parameters {
            match &parameter.kind {
                GenericParamKind::Type {
                    bounds, default, ..
                } => {
                    self.bounds(bounds);
                    if let Some(ty) = default {
                        self.visit_type(ty);
                    }
                }
                GenericParamKind::HigherKinded { bounds, .. }
                | GenericParamKind::KindAnnotated { bounds, .. } => self.bounds(bounds),
                GenericParamKind::Const { ty, .. } => self.visit_type(ty),
                GenericParamKind::Meta { ty, refinement, .. } => {
                    self.visit_type(ty);
                    if let Some(expr) = refinement {
                        self.visit_expr(expr);
                    }
                }
                _ => {}
            }
        }
    }
    fn bounds(&mut self, bounds: &List<TypeBound>) {
        self.type_bound_depth += 1;
        for bound in bounds {
            match &bound.kind {
                TypeBoundKind::Equality(ty) | TypeBoundKind::GenericProtocol(ty) => {
                    // F: fn(A) -> Type(u) constrains a callable's result;
                    // A: Type(u) instead asserts a formation judgment.
                    // Callable equality bounds are checked by the existing
                    // source checker and retain their levels in the scheme.
                    if matches!(ty.kind, TypeKind::Function { .. }) {
                        let saved = std::mem::take(&mut self.type_bound_depth);
                        self.visit_type(ty);
                        self.type_bound_depth = saved;
                    } else {
                        self.visit_type(ty);
                    }
                }
                TypeBoundKind::AssociatedTypeEquality { eq_type, .. } => self.visit_type(eq_type),
                TypeBoundKind::AssociatedTypeBound { bounds, .. } => self.bounds(bounds),
                _ => {}
            }
        }
        self.type_bound_depth -= 1;
    }
    fn context(&mut self, context: &crate::context::ContextRequirement) {
        for ty in &context.args {
            self.visit_type(ty);
        }
        if let Some(condition) = &context.condition {
            self.visit_expr(condition);
        }
        for transform in &context.transforms {
            for argument in &transform.args {
                self.visit_expr(argument);
            }
        }
    }
    fn attributes(&mut self, attributes: &List<crate::attr::Attribute>) {
        for attribute in attributes {
            if let Some(arguments) = &attribute.args {
                for argument in arguments {
                    self.visit_expr(argument);
                }
            }
        }
    }
    fn record_fields(&mut self, fields: &List<crate::decl::RecordField>) {
        for field in fields {
            self.visit_type(&field.ty);
            self.attributes(&field.attributes);
            if let Some(default) = &field.default_value {
                self.visit_expr(default);
            }
        }
    }
    fn variants(&mut self, variants: &List<crate::decl::Variant>) {
        for variant in variants {
            if self.validate
                && variant
                    .generic_params
                    .iter()
                    .any(|parameter| matches!(parameter.kind, GenericParamKind::Level { .. }))
            {
                self.fail(
                    "constructor universe parameters require unsupported level instantiation",
                );
            }
            self.push_generics(&variant.generic_params);
            self.attributes(&variant.attributes);
            if let Some(clause) = &variant.where_clause {
                self.visit_where_clause(clause);
            }
            match &variant.data {
                Some(crate::decl::VariantData::Tuple(types)) => {
                    for ty in types {
                        self.visit_type(ty);
                    }
                }
                Some(crate::decl::VariantData::Record(fields)) => self.record_fields(fields),
                None => {}
            }
            if let Some((from, to)) = &variant.path_endpoints {
                self.visit_expr(from);
                self.visit_expr(to);
            }
            self.scopes.pop();
        }
    }
    fn level(&mut self, level: &UniverseLevelExpr) {
        match level {
            UniverseLevelExpr::Variable(name)
                if self.validate && self.bound(&name.name) != Some(true) =>
            {
                self.fail(format!(
                    "unbound or wrong-kind universe binder `{}` in source signature",
                    name.name
                ));
            }
            UniverseLevelExpr::Max(a, b) => {
                self.level(a);
                self.level(b);
            }
            UniverseLevelExpr::Succ(level) => self.level(level),
            _ => {}
        }
    }
}

impl Visitor for SignatureScan {
    fn visit_axiom(&mut self, axiom: &crate::decl::AxiomDecl) {
        self.push_generics(&axiom.generics);
        self.attributes(&axiom.attributes);
        for parameter in &axiom.params {
            self.attributes(&parameter.attributes);
            self.visit_function_param(parameter);
        }
        if let Some(ty) = &axiom.return_type {
            self.visit_type(ty);
        }
        self.visit_expr(&axiom.proposition);
        for clause in [&axiom.generic_where_clause, &axiom.meta_where_clause]
            .into_iter()
            .flatten()
        {
            self.visit_where_clause(clause);
        }
        self.scopes.pop();
    }
    fn visit_where_predicate(&mut self, predicate: &crate::ty::WherePredicate) {
        match &predicate.kind {
            WherePredicateKind::Type { ty, bounds } => {
                self.type_bound_depth += 1;
                self.visit_type(ty);
                self.bounds(bounds);
                self.type_bound_depth -= 1;
            }
            WherePredicateKind::Meta { constraint } => self.visit_expr(constraint),
            WherePredicateKind::Value { predicate } => self.visit_expr(predicate),
            WherePredicateKind::Ensures { postcondition } => self.visit_expr(postcondition),
        }
    }
    fn visit_type_decl(&mut self, declaration: &crate::TypeDecl) {
        use crate::decl::{ProtocolItemKind, TypeDeclBody};
        if self.validate && self.nominal_depth != 0 {
            self.fail(
                "nested type declarations in nominal signature expressions are not supported",
            );
        }
        self.nominal_depth += 1;
        self.push_generics(&declaration.generics);
        self.attributes(&declaration.attributes);
        for parameter in &declaration.value_params {
            self.attributes(&parameter.attributes);
            self.visit_function_param(parameter);
        }
        for clause in [
            &declaration.generic_where_clause,
            &declaration.meta_where_clause,
        ]
        .into_iter()
        .flatten()
        {
            self.visit_where_clause(clause);
        }
        match &declaration.body {
            TypeDeclBody::Protocol(body) | TypeDeclBody::Coinductive(body) => {
                if let Some(clause) = &body.generic_where_clause {
                    self.visit_where_clause(clause);
                }
                for ty in &body.extends {
                    self.visit_type(ty);
                }
                for item in &body.items {
                    match &item.kind {
                        ProtocolItemKind::Function { decl, .. } => {
                            self.direct_nominal_method = true;
                            self.visit_function(decl);
                        }
                        ProtocolItemKind::Type {
                            type_params,
                            bounds,
                            where_clause,
                            default_type,
                            ..
                        } => {
                            self.push_generics(type_params);
                            self.bounds(bounds);
                            if let Some(clause) = where_clause {
                                self.visit_where_clause(clause);
                            }
                            if let Some(ty) = default_type {
                                self.visit_type(ty);
                            }
                            self.scopes.pop();
                        }
                        ProtocolItemKind::Const { ty, .. } => self.visit_type(ty),
                        ProtocolItemKind::Axiom(axiom) => self.visit_axiom(axiom),
                    }
                }
            }
            TypeDeclBody::Alias(ty) | TypeDeclBody::Newtype(ty) => self.visit_type(ty),
            TypeDeclBody::Record(fields) => self.record_fields(fields),
            TypeDeclBody::Variant(variants) | TypeDeclBody::Inductive(variants) => {
                self.variants(variants)
            }
            TypeDeclBody::Tuple(types) | TypeDeclBody::SigmaTuple(types) => {
                for ty in types {
                    self.visit_type(ty);
                }
            }
            TypeDeclBody::Quotient { base, relation } => {
                self.visit_type(base);
                self.visit_expr(relation);
            }
            TypeDeclBody::Unit => {}
        }
        self.scopes.pop();
        self.nominal_depth -= 1;
    }
    fn visit_function(&mut self, function: &FunctionDecl) {
        let direct_nominal_method = std::mem::take(&mut self.direct_nominal_method);
        if self.validate
            && self.nominal_depth != 0
            && !direct_nominal_method
            && function.body.is_some()
        {
            self.fail("nested function bodies in nominal signature expressions are not supported");
        }
        self.push_generics(&function.generics);
        for parameter in &function.params {
            self.attributes(&parameter.attributes);
            if let FunctionParamKind::Regular {
                pattern,
                ty,
                default_value,
            } = &parameter.kind
            {
                self.visit_pattern(pattern);
                self.visit_type(ty);
                if let Some(value) = default_value {
                    self.visit_expr(value);
                }
            }
        }
        if let Some(ty) = &function.return_type {
            self.visit_type(ty);
        }
        if let Some(throws) = &function.throws_clause {
            for ty in &throws.error_types {
                self.visit_type(ty);
            }
        }
        for context in &function.contexts {
            self.context(context);
        }
        for clause in [&function.generic_where_clause, &function.meta_where_clause]
            .into_iter()
            .flatten()
        {
            self.visit_where_clause(clause);
        }
        for expression in function
            .decreases
            .iter()
            .chain(&function.requires)
            .chain(&function.ensures)
        {
            self.visit_expr(expression);
        }
        self.attributes(&function.attributes);
        self.scopes.pop();
    }
    fn visit_type(&mut self, ty: &crate::Type) {
        if self.validate && matches!(&ty.kind, TypeKind::Qualified { trait_ref, .. }
            if !trait_ref.segments.is_empty()) {
            self.fail("qualified associated projection requires a typed protocol application carrier");
        }
        if matches!(&ty.kind, TypeKind::Generic { base, .. }
            if matches!(base.kind, TypeKind::AssociatedType { .. } | TypeKind::Qualified { .. })) {
            self.uses_applied_projection = true;
        }
        if self.error.is_some() {
            return;
        }
        match &ty.kind {
            TypeKind::Universe { level } => {
                self.universe();
                if let Some(level) = level {
                    self.level(level);
                }
            }
            TypeKind::Path(path) if self.validate => {
                if let [crate::ty::PathSegment::Name(name)] = path.segments.as_slice()
                    && self.bound(&name.name) == Some(true)
                {
                    self.fail(format!(
                        "universe binder `{}` used as an ordinary type",
                        name.name
                    ));
                }
            }
            TypeKind::Rank2Function {
                type_params,
                params,
                return_type,
                contexts,
                where_clause,
                ..
            } => {
                // The VBC local count cannot retain parameter kinds, bounds
                // or the local where clause. Keep the actual source telescope.
                self.uses_rank_two = true;
                self.rank_two_depth += 1;
                self.push_generics(type_params);
                for parameter in params {
                    self.visit_type(parameter);
                }
                self.visit_type(return_type);
                if let Some(clause) = where_clause {
                    self.visit_where_clause(clause);
                }
                for context in &contexts.requirements {
                    self.context(context);
                }
                self.scopes.pop();
                self.rank_two_depth -= 1;
            }
            TypeKind::Function { contexts, .. } => {
                self.uses_callable_contexts |= !contexts.is_empty();
                walk_type(self, ty);
                for context in &contexts.requirements {
                    self.context(context);
                }
            }
            TypeKind::Existential { bounds, .. } => self.bounds(bounds),
            _ => walk_type(self, ty),
        }
    }
    fn visit_expr(&mut self, expression: &crate::Expr) {
        if self.error.is_none() {
            walk_expr(self, expression);
        }
    }
}

// A serialization walk bounds *every* retained AST field, including token trees,
// attributes and expression forms that the semantic visitor does not interpret.
// No bytes are produced. Root body is skipped before it can consume any budget.
const MAX_DEPTH: usize = 128;
const MAX_NODES: usize = 65_536;
const MAX_TEXT_BYTES: usize = 1_048_576;

#[derive(Debug)]
struct BudgetError(Text);
impl std::fmt::Display for BudgetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for BudgetError {}
impl serde::ser::Error for BudgetError {
    fn custom<T: std::fmt::Display>(message: T) -> Self {
        Self(format!("{message}").into())
    }
}
struct Budget {
    nodes: usize,
    bytes: usize,
    skip_root_type_body: bool,
}
impl Budget {
    fn new() -> Self {
        Self {
            nodes: 0,
            bytes: 0,
            skip_root_type_body: false,
        }
    }
    fn visit<T: ?Sized + Serialize>(
        &mut self,
        value: &T,
        depth: usize,
    ) -> std::result::Result<(), BudgetError> {
        if depth > MAX_DEPTH || self.nodes >= MAX_NODES {
            return Err(BudgetError(
                "source signature exceeds its structural budget".into(),
            ));
        }
        self.nodes += 1;
        value.serialize(Frame {
            budget: self,
            depth,
        })
    }
    fn text(&mut self, bytes: usize) -> std::result::Result<(), BudgetError> {
        self.bytes = self
            .bytes
            .checked_add(bytes)
            .ok_or_else(|| BudgetError("source signature text budget overflow".into()))?;
        if self.bytes > MAX_TEXT_BYTES {
            return Err(BudgetError(
                "source signature exceeds its text budget".into(),
            ));
        }
        Ok(())
    }
}
fn check_size(function: &FunctionDecl) -> Result<()> {
    Budget::new().visit(function, 0).map_err(|error| error.0)
}

fn check_type_declaration_size(declaration: &crate::TypeDecl) -> Result<()> {
    use crate::decl::{ProtocolBody, ProtocolItemKind, TypeDeclBody};
    let mut budget = Budget::new();
    let check = |budget: &mut Budget| -> std::result::Result<(), BudgetError> {
        match &declaration.body {
            TypeDeclBody::Protocol(body) | TypeDeclBody::Coinductive(body) => {
                // Bound every outer field without traversing the excluded
                // executable bodies. The direct copier uses the same boundary.
                budget.skip_root_type_body = true;
                budget.visit(declaration, 0)?;
                budget.skip_root_type_body = false;
                let ProtocolBody {
                    is_context,
                    extends,
                    generic_where_clause,
                    items,
                } = body;
                budget.visit(is_context, 1)?;
                budget.visit(extends, 1)?;
                budget.visit(generic_where_clause, 1)?;
                for item in items {
                    match &item.kind {
                        ProtocolItemKind::Function { decl, .. } => {
                            budget.visit(&item.span, 1)?;
                            // At this root, the serializer omits only decl.body.
                            budget.visit(decl, 0)?;
                        }
                        _ => budget.visit(item, 1)?,
                    }
                }
            }
            _ => budget.visit(declaration, 0)?,
        }
        Ok(())
    };
    check(&mut budget).map_err(|error| error.0)
}
struct Frame<'a> {
    budget: &'a mut Budget,
    depth: usize,
}
struct Compound<'a> {
    frame: Frame<'a>,
    skip_body: bool,
}
macro_rules! primitive {
    ($($name:ident($ty:ty)),* $(,)?) => { $(fn $name(self, _: $ty) -> std::result::Result<(), BudgetError> { Ok(()) })* };
}
impl<'a> serde::Serializer for Frame<'a> {
    type Ok = ();
    type Error = BudgetError;
    type SerializeSeq = Compound<'a>;
    type SerializeTuple = Compound<'a>;
    type SerializeTupleStruct = Compound<'a>;
    type SerializeTupleVariant = Compound<'a>;
    type SerializeMap = Compound<'a>;
    type SerializeStruct = Compound<'a>;
    type SerializeStructVariant = Compound<'a>;
    primitive!(
        serialize_bool(bool),
        serialize_i8(i8),
        serialize_i16(i16),
        serialize_i32(i32),
        serialize_i64(i64),
        serialize_i128(i128),
        serialize_u8(u8),
        serialize_u16(u16),
        serialize_u32(u32),
        serialize_u64(u64),
        serialize_u128(u128),
        serialize_f32(f32),
        serialize_f64(f64),
        serialize_char(char)
    );
    fn serialize_str(self, value: &str) -> std::result::Result<(), BudgetError> {
        self.budget.text(value.len())
    }
    fn serialize_bytes(self, value: &[u8]) -> std::result::Result<(), BudgetError> {
        self.budget.text(value.len())
    }
    fn serialize_none(self) -> std::result::Result<(), BudgetError> {
        Ok(())
    }
    fn serialize_some<T: ?Sized + Serialize>(
        self,
        value: &T,
    ) -> std::result::Result<(), BudgetError> {
        self.budget.visit(value, self.depth + 1)
    }
    fn serialize_unit(self) -> std::result::Result<(), BudgetError> {
        Ok(())
    }
    fn serialize_unit_struct(self, _: &'static str) -> std::result::Result<(), BudgetError> {
        Ok(())
    }
    fn serialize_unit_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
    ) -> std::result::Result<(), BudgetError> {
        Ok(())
    }
    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _: &'static str,
        value: &T,
    ) -> std::result::Result<(), BudgetError> {
        self.budget.visit(value, self.depth + 1)
    }
    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        value: &T,
    ) -> std::result::Result<(), BudgetError> {
        self.budget.visit(value, self.depth + 1)
    }
    fn serialize_seq(self, _: Option<usize>) -> std::result::Result<Compound<'a>, BudgetError> {
        Ok(Compound {
            frame: self,
            skip_body: false,
        })
    }
    fn serialize_tuple(self, _: usize) -> std::result::Result<Compound<'a>, BudgetError> {
        self.serialize_seq(None)
    }
    fn serialize_tuple_struct(
        self,
        _: &'static str,
        _: usize,
    ) -> std::result::Result<Compound<'a>, BudgetError> {
        self.serialize_seq(None)
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> std::result::Result<Compound<'a>, BudgetError> {
        self.serialize_seq(None)
    }
    fn serialize_map(self, _: Option<usize>) -> std::result::Result<Compound<'a>, BudgetError> {
        self.serialize_seq(None)
    }
    fn serialize_struct(
        self,
        name: &'static str,
        _: usize,
    ) -> std::result::Result<Compound<'a>, BudgetError> {
        let skip_body = self.depth == 0
            && (name == "FunctionDecl" || (name == "TypeDecl" && self.budget.skip_root_type_body));
        Ok(Compound {
            frame: self,
            skip_body,
        })
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> std::result::Result<Compound<'a>, BudgetError> {
        self.serialize_seq(None)
    }
}
macro_rules! compound {
    ($trait:ident, $method:ident) => {
        impl serde::ser::$trait for Compound<'_> {
            type Ok = ();
            type Error = BudgetError;
            fn $method<T: ?Sized + Serialize>(
                &mut self,
                value: &T,
            ) -> std::result::Result<(), BudgetError> {
                self.frame.budget.visit(value, self.frame.depth + 1)
            }
            fn end(self) -> std::result::Result<(), BudgetError> {
                Ok(())
            }
        }
    };
}
compound!(SerializeSeq, serialize_element);
compound!(SerializeTuple, serialize_element);
compound!(SerializeTupleStruct, serialize_field);
compound!(SerializeTupleVariant, serialize_field);
impl serde::ser::SerializeMap for Compound<'_> {
    type Ok = ();
    type Error = BudgetError;
    fn serialize_key<T: ?Sized + Serialize>(
        &mut self,
        value: &T,
    ) -> std::result::Result<(), BudgetError> {
        self.frame.budget.visit(value, self.frame.depth + 1)
    }
    fn serialize_value<T: ?Sized + Serialize>(
        &mut self,
        value: &T,
    ) -> std::result::Result<(), BudgetError> {
        self.frame.budget.visit(value, self.frame.depth + 1)
    }
    fn end(self) -> std::result::Result<(), BudgetError> {
        Ok(())
    }
}
impl serde::ser::SerializeStruct for Compound<'_> {
    type Ok = ();
    type Error = BudgetError;
    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        name: &'static str,
        value: &T,
    ) -> std::result::Result<(), BudgetError> {
        if self.skip_body && name == "body" {
            return Ok(());
        }
        self.frame.budget.visit(value, self.frame.depth + 1)
    }
    fn end(self) -> std::result::Result<(), BudgetError> {
        Ok(())
    }
}
impl serde::ser::SerializeStructVariant for Compound<'_> {
    type Ok = ();
    type Error = BudgetError;
    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        _: &'static str,
        value: &T,
    ) -> std::result::Result<(), BudgetError> {
        self.frame.budget.visit(value, self.frame.depth + 1)
    }
    fn end(self) -> std::result::Result<(), BudgetError> {
        Ok(())
    }
}

/// Retain the implementation header and associated assignments in their actual
/// source scope. Method/proof bodies have separate executable authorities.
/// Validate the retained fields before cloning, including nested AST data.
pub fn source_associated_implementation(
    declaration: &crate::decl::ImplDecl,
) -> Result<crate::decl::ImplDecl> {
    source_associated_implementation_items(declaration, &declaration.items.iter().collect::<Vec<_>>())
}

/// Capture exactly the items admitted for the compilation target.
pub fn source_associated_implementation_items(
    declaration: &crate::decl::ImplDecl,
    selected_items: &[&crate::decl::ImplItem],
) -> Result<crate::decl::ImplDecl> {
    use crate::decl::{ImplDecl, ImplItemKind};
    let ImplDecl {
        is_unsafe,
        generics,
        kind,
        generic_where_clause,
        meta_where_clause,
        specialize_attr,
        items: _,
        span,
    } = declaration;
    let items = selected_items;
    let mut budget = Budget::new();
    budget
        .visit(
            &(
                is_unsafe,
                generics,
                kind,
                generic_where_clause,
                meta_where_clause,
                specialize_attr,
                span,
            ),
            0,
        )
        .map_err(|error| error.0)?;
    for item in items {
        match &item.kind {
            ImplItemKind::Type { .. } => budget.visit(item, 1).map_err(|error| error.0)?,
            ImplItemKind::Function(function) => {
                check_size(function)?;
                budget.visit(&(&item.attributes, &item.visibility, &item.span), 1).map_err(|error| error.0)?;
            }
            _ => return Err("associated implementation carrier cannot represent constants or proof clauses".into()),
        }
    }
    let retained = ImplDecl {
        is_unsafe: *is_unsafe,
        generics: generics.clone(),
        kind: kind.clone(),
        generic_where_clause: generic_where_clause.clone(),
        meta_where_clause: meta_where_clause.clone(),
        specialize_attr: specialize_attr.clone(),
        span: *span,
        items: items
            .iter()
            .filter_map(|item| match &item.kind {
                ImplItemKind::Type { .. } => Some((*item).clone()),
                ImplItemKind::Function(function) => Some(crate::decl::ImplItem {
                    attributes: item.attributes.clone(), visibility: item.visibility.clone(),
                    span: item.span, kind: ImplItemKind::Function(copy_function_signature(function)),
                }),
                _ => None,
            })
            .collect(),
    };
    validate_source_associated_implementation(&retained)?;
    Ok(retained)
}

/// Bounded syntax and binder closure only. This does not prove conformance,
/// method availability, associated equalities or specialization coverage.
pub fn validate_source_associated_implementation(
    declaration: &crate::decl::ImplDecl,
) -> Result<()> {
    use crate::decl::{ImplItemKind, ImplKind};
    use crate::ty::GenericArg;
    Budget::new()
        .visit(declaration, 0)
        .map_err(|error| error.0)?;
    let ImplKind::Protocol {
        protocol,
        protocol_args,
        for_type,
    } = &declaration.kind
    else {
        return Err("associated implementation carrier requires a protocol declaration".into());
    };
    if protocol.segments.is_empty() {
        return Err("associated implementation carrier has no protocol identity".into());
    }
    let mut scan = SignatureScan::new(true);
    scan.nominal_depth = 1;
    scan.push_generics(&declaration.generics);
    scan.visit_type(for_type);
    for argument in protocol_args {
        match argument {
            GenericArg::Type(ty) => scan.visit_type(ty),
            GenericArg::Const(expression) => scan.visit_expr(expression),
            GenericArg::Binding(binding) => scan.visit_type(&binding.ty),
            GenericArg::Lifetime(_) => {}
        }
    }
    for clause in [
        &declaration.generic_where_clause,
        &declaration.meta_where_clause,
    ]
    .into_iter()
    .flatten()
    {
        scan.visit_where_clause(clause);
    }
    if let Some(clause) = declaration
        .specialize_attr
        .as_ref()
        .and_then(|attribute| attribute.when_clause.as_ref())
    {
        scan.visit_where_clause(clause);
    }
    let mut names = verum_common::Set::new();
    for item in &declaration.items {
        if let ImplItemKind::Function(function) = &item.kind {
            if function.body.is_some() {
                return Err("associated implementation carrier contains a method body".into());
            }
            scan.attributes(&item.attributes);
            scan.visit_function(function);
            continue;
        }
        let ImplItemKind::Type {
            name,
            type_params,
            ty,
        } = &item.kind
        else {
            return Err("associated implementation carrier contains an executable item".into());
        };
        if name.name.is_empty() || !names.insert(name.name.clone()) {
            return Err(
                "associated implementation carrier has an empty or repeated assignment".into(),
            );
        }
        scan.push_generics(type_params);
        scan.attributes(&item.attributes);
        scan.visit_type(ty);
        scan.scopes.pop();
    }
    scan.scopes.pop();
    scan.finish()
}
