//! Type declaration registration methods for the type-checker.
//!
//! Contains ~78 `TypeChecker` methods covering:
//! - Type declaration pre-registration and body registration
//! - Record, sum, protocol, and context type registration
//! - Associated type registration and validation

#[allow(unused_imports)]
use crate::const_eval::ConstEvaluator;
#[allow(unused_imports)]
use crate::context::{TypeContext, TypeScheme};
#[allow(unused_imports)]
use crate::context_check::{ContextChecker, ContextRequirement, ContextSet};
#[allow(unused_imports)]
use crate::integer_hierarchy::IntegerHierarchy;
#[allow(unused_imports)]
use crate::operator_protocols::{OperatorProtocols, OutputStrategy};
#[allow(unused_imports)]
use crate::protocol::ProtocolChecker;
#[allow(unused_imports)]
use crate::refinement::RefinementChecker;
#[allow(unused_imports)]
use crate::subtype::Subtyping;
#[allow(unused_imports)]
use crate::ty::{Type, TypeVar};
#[allow(unused_imports)]
use crate::unify::Unifier;
#[allow(unused_imports)]
use crate::{Result, TypeCheckMetrics, TypeError};
#[allow(unused_imports)]
use super::{
    DeferredConstraint, FunctionContract, GeneratorContext,
    GlobalDepthGuard, InferMode, InferResult, TypeChecker,
    WKT_HEAP, WKT_RESULT, WKT_SHARED,
    DEREF_COERCION_DEPTH, GLOBAL_CALL_DEPTH,
    span_to_line_col, expr_kind_description,
    read_param_classification,
};
#[allow(unused_imports)]
use std::time::Instant;
#[allow(unused_imports)]
use verum_ast::{BinOp, Block, Expr, ExprKind, LiteralKind, Stmt, StmtKind, TokenTree, UnOp, Item};
#[allow(unused_imports)]
use verum_ast::decl::{
    FunctionBody, FunctionDecl, FunctionParamKind, ImplDecl, RecordField, TypeDecl, TypeDeclBody,
    ContextDecl, ProtocolDecl, Visibility, MountDecl, MountTree, MountTreeKind, ResourceModifier,
};
#[allow(unused_imports)]
use verum_ast::pattern::Pattern;
#[allow(unused_imports)]
use verum_ast::span::{Span, Spanned};
#[allow(unused_imports)]
use verum_ast::ty::{Ident, Path};
#[allow(unused_imports)]
use verum_diagnostics::{Diagnostic, DiagnosticBuilder};
#[allow(unused_imports)]
use verum_common::well_known_types::WellKnownType as WKT;
#[allow(unused_imports)]
use verum_common::well_known_types::type_names as wkt_names;
#[allow(unused_imports)]
use verum_common::{Heap, List, Map, Maybe, Set, Shared, Text, ToText};
#[allow(unused_imports)]
use verum_modules::{ModulePath, ModuleRegistry, NameResolver, resolve_import, resolver::NameKind};

#[derive(Clone, Copy)]
enum ExpectedImplementation<'a> {
    Protocol(&'a Text, &'a Type),
    Inherent(&'a Type),
}

impl TypeChecker {
    /// Record the arity a type declaration establishes for its own name.
    ///
    /// Called for EVERY type declaration — generic or not. A declaration
    /// with no parameters writes an EMPTY record rather than nothing,
    /// because `__type_params_<Name>` is a flat namespace shared by every
    /// module: staying silent leaves whatever a SAME-NAMED type wrote
    /// there earlier, and the next reader takes that stale arity for this
    /// type's.
    ///
    /// A project record named `Slice` inherited the standard library's
    /// `Slice<T>` arity exactly that way. Its fields resolved to the
    /// project's record while its arity came from the stdlib namesake, so
    /// every literal typed as `Slice<_>` and every annotation stayed
    /// `Slice` — one name answered by two types, half a question each.
    ///
    /// Writing the empty record also CLAIMS the slot: the lazy stdlib
    /// loaders that populate this key guard on `is_none()`, so a
    /// declaration that says "no parameters" is not overwritten later by
    /// a namesake being loaded on demand.
    ///
    /// One carrier. Four registrars — record, alias, variant, protocol —
    /// each held a copy of this loop, and the copies had already drifted:
    /// only one of them counted a `KindAnnotated` parameter.
    /// The arity currently recorded for `type_name`, or `None` when the
    /// key is absent.
    ///
    /// Absence is a distinct answer from zero: it means no declaration has
    /// claimed the name, which is the state a namesake's stale entry used
    /// to be read out of. Exposed so a test can assert the recorded FACT
    /// rather than a symptom several inference steps downstream of it.
    pub fn recorded_type_arity(&self, type_name: &str) -> Option<usize> {
        let key = format!("__type_params_{}", type_name);
        match self.ctx.lookup_type(key.as_str()) {
            verum_common::Maybe::Some(Type::Record(params)) => Some(params.len()),
            _ => None,
        }
    }

    /// Returns the arity it recorded, for callers that also register a
    /// kind for the type constructor.
    pub(crate) fn register_declared_type_arity(
        &mut self,
        type_name: &verum_common::Text,
        generics: &List<verum_ast::ty::GenericParam>,
    ) -> Result<usize> {
        use verum_ast::ty::GenericParamKind;

        // Every positional parameter that carries a user-visible name —
        // Type, HigherKinded, KindAnnotated, Const, Meta, Context. The
        // arity counter and the user-facing "type T expects N arguments"
        // check in `compile_type_path` read this map's length, so
        // skipping a kind makes a declaration look narrower than it is:
        // omitting Meta/Const made `type Matrix<T, Rows: meta Int, Cols:
        // meta Int>` count as arity 1, and `Matrix<Float, 2, 3>` was
        // refused as "expects 1 type argument(s), but 3 were provided".
        //
        // Lifetime annotations are erased in both declarations and
        // applications; they never occupy a retained substitution slot.
        let mut param_record: indexmap::IndexMap<verum_common::Text, Type> =
            indexmap::IndexMap::new();
        for param in generics.iter() {
            let name_opt = match &param.kind {
                GenericParamKind::Type { name, .. }
                | GenericParamKind::HigherKinded { name, .. }
                | GenericParamKind::KindAnnotated { name, .. }
                | GenericParamKind::Const { name, .. }
                | GenericParamKind::Meta { name, .. }
                | GenericParamKind::Context { name } => Some(name.name.clone()),
                GenericParamKind::Lifetime { .. } => None,
                _ => None,
            };
            if let Some(n) = name_opt {
                let parameter = self.lookup_decl_param(n.as_str()).cloned()
                    .ok_or_else(|| TypeError::OtherWithCodeSpanned { code: "E407".into(),
                        msg: format!("parameter '{n}' has no declaration-owned lexical binder").into(),
                        span: param.span })?;
                param_record.insert(n, parameter);
            }
        }

        self.record_declared_meta_refinements(type_name, generics);

        let arity = param_record.len();
        self.define_declared_type_metadata(
            "__type_params_",
            type_name.as_str(),
            Type::Record(param_record),
        );
        Ok(arity)
    }

    /// Nominal arguments use the same exact declaration roster as applications.
    /// Erased annotations and invented Unit slots never enter nominal identity.
    fn declared_nominal_arguments(&self, name: &Text, span: Span) -> Result<List<Type>> {
        Ok(self.declared_parameter_roster(name, span)?.values().cloned().collect())
    }

    /// A literal carrier keeps its language identity in the record that supplies
    /// its standard representation. Other records remain nominal, including a
    /// record with the same name and layout in a different module.
    fn declared_record_type(&self, declaration: &TypeDecl) -> Result<Type> {
        let name = declaration.name.as_str();
        let args = self.declared_nominal_arguments(&declaration.name.name, declaration.span)?;
        if args.is_empty()
            && WKT::declaring_source_module(name) == Some(self.current_module_path.as_str())
            && let Some(builtin) = self.ctx.builtin_type_definition(name)
            && builtin.primitive_name() == Some(name)
        {
            return Ok(builtin.clone());
        }
        Ok(Type::Named { path: Path::single(declaration.name.clone()), args })
    }

    fn declared_parameter_roster(&self, name: &Text, span: Span)
        -> Result<&indexmap::IndexMap<Text, Type>>
    {
        let owner = self.declared_type_key(name.as_str());
        match self.ctx.lookup_type(&format!("__type_params_{owner}")) {
            Some(Type::Record(parameters)) => Ok(parameters),
            _ => Err(TypeError::OtherWithCodeSpanned { code: "E407".into(),
                msg: "nominal declaration has no owned parameter roster".into(), span }),
        }
    }

    /// Both sum registration passes publish the prepared lexical telescope.
    /// Value parameters retain their sort, predicate and rigid identity; they
    /// must never be replaced by the temporary variables used to enter scope.
    fn register_sum_parameters(&mut self, declaration: &TypeDecl) -> Result<()> {
        let name = &declaration.name.name;
        self.register_declared_type_arity(name, &declaration.generics)?;
        let parameters = self.declared_nominal_arguments(name, declaration.span)?;
        self.define_declared_type_metadata("__type_var_order_", name, Type::Tuple(parameters));
        Ok(())
    }

    fn register_sum_constructor(
        &mut self, declaration: &TypeDecl, name: &Text, ty: Type, publish_short: bool,
    ) -> Result<()> {
        let owner = self.declared_type_key(&declaration.name.name);
        let parameters = self.declared_nominal_arguments(&declaration.name.name, declaration.span)?;
        let params = match ty { Type::Function { params, .. } => Some(params), _ => None };
        let scheme = self.nominal_constructor_scheme(&owner, &parameters, params, declaration.span)?;
        if publish_short { self.ctx.env.insert(name.clone(), scheme.clone()); }
        // Lazy source loading uses a temporary lexical scope. Keep the exact
        // constructor with the same transaction as its owning declaration;
        // publication must survive that scope only after the whole load succeeds.
        self.source_type_registration.constructor_values
            .insert(format!("{owner}.{name}").into(), scheme.clone());
        self.ctx.env.insert(format!("{}.{}", declaration.name.name, name), scheme);
        Ok(())
    }

    /// Preserve the value sort of meta/const parameters in the declaration
    /// frame. A runtime type PID exists only for ordinary type parameters.
    fn prepare_type_declaration_parameters(&mut self, declaration: &TypeDecl) -> Result<()> {
        use verum_ast::ty::GenericParamKind;
        // Value roles are source semantics even when the declaration needs no
        // additional archive carrier. Aliases must not publish an erased body.
        for parameter in &declaration.generics {
            if let GenericParamKind::Meta { name, ty, .. } | GenericParamKind::Const { name, ty } = &parameter.kind {
                let value = self.source_value_parameter(parameter)?;
                self.decl_param_frames.last_mut().expect("declaration parameter frame")
                    .insert(name.name.clone(), value);
            }
        }
        if matches!(&declaration.body, TypeDeclBody::Variant(_)) {
            self.register_sum_parameters(declaration)?;
        }
        // Tuple-like nominal declarations share this frame in both passes.
        // An explicit empty telescope also distinguishes their heads from scalars.
        if matches!(&declaration.body, TypeDeclBody::Tuple(_) | TypeDeclBody::Newtype(_)
            | TypeDeclBody::SigmaTuple(_) | TypeDeclBody::Unit)
        {
            self.register_declared_type_arity(&declaration.name.name, &declaration.generics)?;
        }
        if !matches!(&declaration.body, TypeDeclBody::Protocol(_) | TypeDeclBody::Coinductive(_)) {
            self.record_type_param_defaults(declaration.name.name.as_str(), &declaration.generics)?;
        }
        Ok(())
    }

    /// Alias application substitutes every retained declaration slot in source
    /// order. Lifetime annotations erase; value parameters retain their identity.
    /// A forward-declared target has a nominal identity before its fields
    /// and inherent methods are complete. An alias must retain that
    /// identity so later users look up the completed declaration.
    fn declared_alias_target(&mut self, source: &verum_ast::Type) -> Result<Type> {
        match self.declared_type(source)? {
            Type::Placeholder { name, .. } => Ok(Type::Named {
                path: Self::text_to_path(&name),
                args: List::new(),
            }),
            target => Ok(target),
        }
    }

    fn register_source_alias_parameters(
        &mut self,
        name: &Text,
        parameters: &List<verum_ast::ty::GenericParam>,
    ) {
        use verum_ast::ty::GenericParamKind;
        let names: List<Text> = parameters.iter().filter(|parameter| !matches!(parameter.kind, GenericParamKind::Lifetime { .. })).map(|parameter| {
            match &parameter.kind {
                GenericParamKind::Type { name, .. }
                | GenericParamKind::HigherKinded { name, .. }
                | GenericParamKind::KindAnnotated { name, .. }
                | GenericParamKind::Const { name, .. }
                | GenericParamKind::Meta { name, .. }
                | GenericParamKind::Context { name }
                | GenericParamKind::Level { name } => name.name.clone(),
                GenericParamKind::Lifetime { name } => name.name.clone(),
            }
        }).collect();
        self.unifier.register_type_alias_params(self.declared_type_key(name.as_str()), names.clone());
        self.unifier.register_type_alias_params(name.clone(), names);
    }

    /// Record constraints independently of the representation used for
    /// parameter slots: sums store actual variables rather than arity markers.
    fn record_declared_meta_refinements(
        &mut self,
        type_name: &Text,
        generics: &List<verum_ast::ty::GenericParam>,
    ) {
        use verum_ast::ty::GenericParamKind;
        // A refinement written on a META parameter is recorded by
        // POSITION, so the argument supplied at a use site can be
        // checked against it. The AST has carried this predicate all
        // along and nothing read it — every consumer of
        // `GenericParamKind::Meta` takes `{ name, .. }` — so
        // `type Digest<N: meta USize {it == 4 || it == 8}>` accepted
        // `Digest<5>` in silence (T0909).
        let mut refinements: Vec<(usize, verum_ast::expr::Expr)> = Vec::new();
        let mut index = 0usize;
        for param in generics.iter() {
            match &param.kind {
                GenericParamKind::Lifetime { .. } => continue,
                GenericParamKind::Meta { ty, refinement, .. } => {
                    // TWO PLACES carry the predicate, and only one of
                    // them is the field named for it. `N: meta USize
                    // {it == 4}` is parsed by `parse_type` reaching the
                    // brace first, so the predicate lands on the TYPE as
                    // a `Refined` wrapper and the `refinement` field
                    // stays empty — which is why looking only at the
                    // field found nothing (measured: `recorded=None`
                    // for a declaration that plainly had one).
                    if let verum_common::Maybe::Some(pred) = refinement {
                        refinements.push((index, (**pred).clone()));
                    } else if let verum_ast::ty::TypeKind::Refined { predicate, .. } = &ty.kind {
                        refinements.push((index, predicate.expr.clone()));
                    }
                }
                _ => {}
            }
            index += 1;
        }
        for key in [type_name.clone(), self.declared_type_key(type_name.as_str())] {
            if refinements.is_empty() {
                self.meta_param_refinements.remove(key.as_str());
            } else {
                self.meta_param_refinements
                    .insert(key.to_string(), refinements.clone());
            }
        }

    }

    /// Register a type declaration (type alias, ADT, etc.)
    /// This should be called before other passes to make types available.
    /// Source payload dependencies are scheduled in declaration order on an explicit work list.
    pub fn register_type_declaration(&mut self, type_decl: &verum_ast::TypeDecl) -> Result<()> {
        self.register_source_declaration(type_decl, true)
    }

    pub(super) fn register_source_declaration(
        &mut self,
        declaration: &verum_ast::TypeDecl,
        publish_cases: bool,
    ) -> Result<()> {
        let key = self.declared_type_key(declaration.name.name.as_str());
        self.with_source_type_registration(declaration, key, |checker| {
            checker.with_declared_universe_scope("type", &declaration.name.name, &declaration.generics, |checker| {
                checker.register_type_declaration_in_universe_scope(declaration, publish_cases)
            })
        })
    }

    fn register_type_declaration_in_universe_scope(
        &mut self, type_decl: &verum_ast::TypeDecl, publish_cases: bool,
    ) -> Result<()> {
        // Same scoping as `resolve_type_definition`: the refinement
        // name check (T1112) speaks only about a TYPE DECLARATION's own
        // predicate. Both entries set it because a module-level
        // `type P is Int { … }` takes this one, and the spec that
        // asserts the check went silent when only the other was wired
        // (T1115).
        let prev_in_type_decl = self.in_type_declaration;
        self.in_type_declaration = true;
        let out = self.register_type_declaration_inner(type_decl, publish_cases);
        self.in_type_declaration = prev_in_type_decl;
        out
    }

    /// Push a declaration's generic-parameter scope frame. Innermost
    /// frame wins in [`Self::lookup_decl_param`]; pop MUST pair every
    /// push (RAII-by-discipline — the registration bodies below run
    /// their fallible section inside an immediately-invoked closure
    /// precisely so the pop is unconditional).
    pub(crate) fn push_decl_param_frame(
        &mut self,
        frame: indexmap::IndexMap<verum_common::Text, Type>,
    ) {
        self.decl_param_frames.push(frame);
    }

    /// Pop the innermost declaration parameter frame.
    pub(crate) fn pop_decl_param_frame(&mut self) {
        self.decl_param_frames.pop();
    }

    /// Resolve a name against the INNERMOST declaration parameter
    /// frame only — a declaration's parameters are lexically scoped
    /// to that declaration; an inner (lazily-entered) registration
    /// must not see an outer declaration's parameters and vice
    /// versa.
    pub(crate) fn lookup_decl_param(&self, name: &str) -> Option<&Type> {
        self.decl_param_frames.last().and_then(|f| f.get(name))
    }

    /// Record the DEFAULT of each type parameter, in declaration order,
    /// under `__type_param_defaults_<Name>`.
    ///
    /// Stored as a tuple so position is the key — a default belongs to a
    /// parameter's SLOT, and filling `Pair<Int>` means supplying the
    /// second slot. A parameter with no default holds `Type::Unknown`,
    /// which is the hole: an application may omit a trailing run of
    /// parameters only while every one of them has a default.
    ///
    /// Nothing is recorded for a type whose parameters all lack defaults,
    /// so the common case adds no entry and no lookup cost.
    fn record_type_param_defaults(
        &mut self,
        type_name: &str,
        generics: &[verum_ast::ty::GenericParam],
    ) -> Result<()> {
        use verum_ast::ty::GenericParamKind;
        if generics.is_empty() {
            return Ok(());
        }
        let mut defaults: List<Type> = List::new();
        let mut any = false;
        for gp in generics.iter() {
            match &gp.kind {
                GenericParamKind::Lifetime { .. } => continue,
                GenericParamKind::Type {
                    default: Maybe::Some(ast_ty),
                    ..
                } => {
                    // Lenient: a default naming a type that is not
                    // registered yet must not fail the declaration it
                    // decorates — declarations are registered in file
                    // order and a default may point forward.
                    defaults.push(if self.resolving_source_signature {
                        self.ast_to_type(ast_ty)?
                    } else {
                        self.ast_to_type_lenient(ast_ty)
                    });
                    any = true;
                }
                _ => defaults.push(Type::Unknown),
            }
        }
        if !any {
            return Ok(());
        }
        self.define_declared_type_metadata(
            "__type_param_defaults_", type_name, Type::Tuple(defaults),
        );
        Ok(())
    }

    fn register_type_declaration_inner(
        &mut self, type_decl: &verum_ast::TypeDecl, publish_cases: bool,
    ) -> Result<()> {
        use crate::context::TypeScheme;
        use indexmap::IndexMap;
        use verum_ast::decl::{TypeDeclBody, VariantData};
        use verum_ast::ty::Path;
        use verum_common::Text;

        let type_name: Text = type_decl.name.name.as_str().into();
        // Track number of generic parameters for this type.
        // This enables proper type inference when the type is used without explicit type args.
        //
        // Recorded UNCONDITIONALLY, zero included, for the reason set out
        // on `register_declared_type_arity`: this map is keyed by simple
        // name across every module, so a declaration that stays silent
        // leaves a same-named type's count to answer for it.
        let generics_count = type_decl.generics.iter()
            .filter(|parameter| !matches!(parameter.kind, verum_ast::ty::GenericParamKind::Lifetime { .. }))
            .count();
        self.type_generics_count
            .insert(type_name.clone(), generics_count);

        // Only a recursive visit to the same declaration can reuse its placeholder.
        // A nested private mount may resolve this spelling in a different module.
        let key = self.declared_type_key(type_name.as_str());
        if !self.types_being_registered.insert(key.clone()) {
            return Ok(());
        }
        let result = self.register_type_declaration_body(type_decl, type_name, publish_cases);
        self.types_being_registered.remove(&key);

        result
    }

    /// Tuple/newtype constructors return the same nominal declaration that
    /// owns their field metadata, and are reachable through its exact path.
    fn register_declared_nominal_constructor(
        &mut self,
        name: &verum_common::Text,
        params: Option<List<Type>>,
        span: Span,
    ) -> Result<()> {
        let owner = self.declared_type_key(name.as_str());
        let parameters: List<Type> = self.declared_parameter_roster(name, span)?.iter()
            .map(|(name, parameter)| match parameter {
                Type::Named { path, args } if args.is_empty()
                    && path.as_ident().is_some_and(|ident| ident.name == *name) => {
                    Type::Var(TypeVar::fresh())
                }
                parameter => parameter.clone(),
            }).collect();
        let params = params.map(|params| -> Result<List<Type>> {
            match self.instantiate_declared_type(&owner, &Type::Tuple(params), &parameters)? {
                Type::Tuple(params) => Ok(params),
                _ => unreachable!("tuple constructor parameter list"),
            }
        }).transpose()?;
        let scheme = self.nominal_constructor_scheme(&owner, &parameters, params, span)?;
        self.ctx.env.insert(name.clone(), scheme.clone());
        // Publish its canonical value only when the enclosing load commits.
        self.source_type_registration.constructor_values.insert(owner, scheme);
        Ok(())
    }

    /// Inner body of register_type_declaration that may return early with errors.
    /// This is separated from register_type_declaration_inner to ensure proper cleanup
    /// of types_being_registered on both success and failure paths.
    fn register_type_declaration_body(
        &mut self,
        type_decl: &verum_ast::TypeDecl,
        type_name: verum_common::Text,
        publish_cases: bool,
    ) -> Result<()> {
        use crate::context::TypeScheme;
        use indexmap::IndexMap;
        use verum_ast::decl::{TypeDeclBody, VariantData};
        use verum_ast::ty::Path;
        use verum_common::Text;

        self.affine_tracker.register_resource_discipline(
            self.declared_type_key(type_name.as_str()), type_decl.resource_discipline(),
        );
        self.record_dependent_family(type_decl);

        // Save type parameter names so we can clean them up later
        // This prevents type parameter pollution across different type declarations


        // Track type parameter names to TypeVars for proper polymorphic quantification
        // This mapping is used when registering variant constructors


        // This declaration's parameter scope, pushed as ONE frame
        // below (see the comment at the push site).
        let mut param_frame: indexmap::IndexMap<Text, Type> = indexmap::IndexMap::new();

        // Register generic type parameters first
        // This allows us to resolve types like T in Maybe<T>
        //

        // CRITICAL: For variant types, we MUST use Type::Var for type parameters.
        // Using Type::Named { path: "T" } would cause unification failures because
        // Named types don't participate in type variable substitution.
        //

        // For type aliases, we need Type::Named for proper substitution:
        // When `Reducer<N, R>` is instantiated, we substitute named params like "A" and "R"
        // in the alias body. If we used Type::Var, substitution wouldn't find them.
        //

        // For record types, Named types are still needed for bidirectional inference.
        // We determine the approach based on whether this is a variant type.
        //
        // ALIAS-VS-MARKER (see `variant_decl_alias_target`): a bare
        // `type X is <ExistingType>;` parses as a one-variant sum but is
        // really a transparent alias. Reroute it to an alias body so the
        // rest of registration — and every downstream use — sees the aliased
        // type instead of a degenerate `X(Unit)` sum.
        let alias_override_body: Option<verum_ast::decl::TypeDeclBody> = self
            .variant_decl_alias_target(type_decl)
            .map(verum_ast::decl::TypeDeclBody::Alias);
        let effective_body: &verum_ast::decl::TypeDeclBody =
            alias_override_body.as_ref().unwrap_or(&type_decl.body);
        let is_variant_type = matches!(effective_body, verum_ast::decl::TypeDeclBody::Variant(_));
        let use_type_vars = is_variant_type;

        for param in &type_decl.generics {
            use verum_ast::ty::GenericParamKind;

            let param_name: Text = match &param.kind {
                GenericParamKind::Type { name, .. } => name.name.as_str().into(),
                GenericParamKind::HigherKinded { name, .. } => name.name.as_str().into(),
                GenericParamKind::KindAnnotated { name, .. } => name.name.as_str().into(),
                GenericParamKind::Const { name, .. } => name.name.as_str().into(),
                GenericParamKind::Meta { name, .. } => name.name.as_str().into(),
                GenericParamKind::Lifetime { .. } => continue,
                GenericParamKind::Context { name } => name.name.as_str().into(),
                GenericParamKind::Level { name } => name.name.as_str().into(),
            };

            if use_type_vars {
                // For variant types AND aliases: use Type::Var for proper polymorphic type inference
                // This ensures that variant constructors like Some(v) can unify T with Int
                // and that type aliases like IoResult<File> can substitute T with File
                let type_var = TypeVar::fresh();
                let param_type = Type::Var(type_var);
                param_frame.insert(param_name.clone(), param_type);

            } else {
                // For record types: use Named type for bidirectional type inference
                // This allows substitute_type_params to find and replace T with concrete types
                let param_type = Type::Named {
                    path: Path::single(verum_ast::ty::Ident::new(
                        param_name.as_str(),
                        Span::default(),
                    )),
                    args: List::new(),
                };
                param_frame.insert(param_name.clone(), param_type);
            }

        }
        // The frame — not the global flat table — carries this
        // declaration's parameters for the duration of its body.
        // Registration can be ENTERED lazily from the middle of an
        // unrelated annotation's resolution (mount + first use of a
        // generic stdlib alias); flat-table parameters left a window
        // where that outer resolution's retry lookup found a rigid
        // `Named{T}` and rigidified every later bare `T` in the
        // importing file (T0683). The frame pops before control
        // returns to the trigger, and — unlike the old flat-table
        // cleanup — popping cannot delete a legitimate same-named
        // GLOBAL type either.
        self.push_decl_param_frame(param_frame);

        // For simple type aliases, register the type in the environment
        // More complex types (records, variants) are handled during type checking.
        // NB: `effective_body` — not `type_decl.body` — so a bare
        // `type X is <ExistingType>;` rerouted above is registered as the
        // alias it actually is.
        //
        // GENERIC-PARAM-LEAK (T0148): several arms below `?`-propagate
        // (alias/newtype/tuple target resolution, variant/record/protocol
        // sub-registration).  Any such early exit used to skip the
        // type-parameter cleanup that follows the match, leaving this
        // declaration's generic param NAMES (`T`, `E`, …) bound in the
        // GLOBAL type table as `Named{param}` / `Var(_)`.  Import callers
        // deliberately swallow registration errors (infer/modules.rs:
        // "Log the error but don't fail … fall through to fallback
        // registration"), so the leaked binding silently poisoned every
        // later bare-`T` annotation/scheme resolution in the importing
        // file.  Canonical repro: `mount core.base.panic.{CatchResult}`
        // (a generic ALIAS whose target resolution fails on the lazy
        // import path) flipped the unrelated `let maybe =
        // Maybe.Some(42); assert_some(maybe)` to E400 "expected 'T',
        // found 'Int'" — base/panic unit_test failed typecheck FILE-WIDE.
        // Same run-body-then-cleanup discipline as
        // `types_being_registered` in `register_type_declaration_inner`:
        // the immediately-invoked closure captures every `?`, making the
        // parameter cleanup below unconditional.
        let (previous_premises, premises_result) = self.begin_nominal_declaration_premises(type_decl);
        let body_result: Result<()> = (|| {
        premises_result?;
        // Defaults share the declaration's lexical parameter frame and owner.
        // Imported signatures cannot postpone a missing name to the consumer.
        self.prepare_type_declaration_parameters(type_decl)?;
        match effective_body {
            TypeDeclBody::Alias(aliased_type) => {
                // Register the type alias in the type context
                let resolved_type = self.declared_alias_target(aliased_type)?;

                // CRITICAL: Store the RESOLVED type in the alias table, but store a NAMED type
                // reference in type_defs. This ensures that when `Reducer<Int, Int>` is looked up:
                // 1. resolve_type_name("Reducer") returns Type::Named { path: "Reducer", args: [] }
                // 2. ast_to_type adds the args to get Type::Named { path: "Reducer", args: [Int, Int] }
                // 3. normalize_type looks up the alias, builds substitution, and applies it
                self.declare_type_alias_target(type_name.clone(), resolved_type.clone());
                // Also register in the unifier for transparent alias unification
                self.unifier
                    .register_type_alias(type_name.clone(), resolved_type.clone());

                // Register type parameter names in the unifier for generic alias expansion.
                // This enables IoResult<Text> -> Result<Text, StreamError> during unification.
                self.register_source_alias_parameters(&type_name, &type_decl.generics);

                // For type_defs, store a Named type reference (not the resolved type)
                // This preserves the indirection needed for generic alias substitution
                let named_type = Type::Named {
                    path: Path::single(type_decl.name.clone()),
                    args: List::new(),
                };
                // Module-aware: register both unqualified (back-compat) AND
                // fully-qualified so signatures inside this module always
                // resolve their own types first, regardless of load order.
                self.define_type_in_current_module(type_name.clone(), named_type);

                // CRITICAL FIX: Register type parameters for generic type aliases
                // This enables proper substitution when the alias is instantiated with concrete types
                // E.g., for `type Reducer<A, R> is fn(R, A) -> R;`, we need __type_params_Reducer
                // so that `Reducer<Int, Bool>` correctly substitutes A=Int, R=Bool
                {
                    let arity = self
                        .register_declared_type_arity(&type_name, &type_decl.generics)?;
                    // Register kind for this type constructor (stdlib-agnostic)
                    if arity > 0 {
                        use crate::kind_inference::KindInference;
                        let kind = if arity == 1 {
                            crate::advanced_protocols::Kind::unary_constructor()
                        } else if arity == 2 {
                            crate::advanced_protocols::Kind::binary_constructor()
                        } else {
                            // N-ary: chain N arrows
                            let mut k = crate::advanced_protocols::Kind::Type;
                            for _ in 0..arity {
                                k = crate::advanced_protocols::Kind::arrow(
                                    crate::advanced_protocols::Kind::Type,
                                    k,
                                );
                            }
                            k
                        };
                        self.kind_inferer()
                            .register_type_constructor(type_name.as_str(), kind);
                    }
                }
            }
            TypeDeclBody::Variant(_) => self.register_variant_type_body(type_decl, &type_name, publish_cases)?,
            TypeDeclBody::Record(_) => self.register_record_type_body(type_decl, &type_name)?,
            // ── DECLARED-NAME REGISTRATION GOES THROUGH THE MODULE PATH ──
            // Every arm that registers the declared type under its own
            // name does so via `define_type_in_current_module`, never
            // `ctx.define_type` directly — the `Variant` and `Record`
            // arms above through the helpers they delegate to, the
            // tuple / newtype / alias arms below in place. That helper is what evicts the
            // same-named AMBIENT variant-constructor shadow (T0591) and
            // what records the name in `current_module_declared_types` for
            // the E430 horizon; a branch that writes the flat map directly
            // leaves a foreign stdlib variant free to answer for the name.
            //
            // Measured: only the 0- and 1-element tuple branches routed
            // correctly, so a local `type Pair is (Int, Int)` lost `Pair(3, 4)`
            // to `CoreTerm.Pair { fst: Heap<CoreTerm>, snd: Heap<CoreTerm> }`
            // from core/verify/kernel_soundness/ — `expected 'Heap<CoreTerm>',
            // found 'Int'` on a file that declares the type three lines above
            // the call. The collision needed the arities to MATCH, which is
            // why it looked like a spec-ordering problem rather than a
            // resolution one: `type Fst is (Int, Int)` was fine, because the
            // stdlib's `Fst` carries one field.
            TypeDeclBody::Tuple(types) => {
                // Tuple type declaration: type Point is (Float, Float);
                // Single-element tuples are newtypes: type UserId is (Int);
                // Zero-element tuples are unit wrappers: type Signal is ();
                let type_list: Result<List<Type>> =
                    types.iter().map(|t| self.ast_to_type(t)).collect();
                let resolved_types = type_list?;

                if resolved_types.is_empty() {
                    // Zero-element "tuple" is a unit wrapper: type Signal is ();
                    // This creates a distinct named type that wraps Unit
                    let newtype_ty = Type::Named {
                        path: Path::single(type_decl.name.clone()),
                        args: List::new(),
                    };
                    // T0591: route through the module-aware path so the
                    // same-name ambient variant-ctor shadow is evicted (a
                    // stdlib variant like `CacheTtl.Seconds` otherwise
                    // captured `Seconds(1.5)` and forced its Int payload).
                    self.define_type_in_current_module(type_name.clone(), newtype_ty.clone());

                    // Store inner type as Unit for coercion checks
                    self.define_declared_type_metadata("__newtype_inner_", type_name.as_str(), Type::Unit);

                    // Register constructor that takes no arguments: Signal: fn() -> Signal
                    // This allows both `Signal()` and `()` coercion via newtype rules
                    self.register_declared_nominal_constructor(&type_name, Some(List::new()), type_decl.span)?;
                } else if resolved_types.len() == 1 {
                    // Single-element "tuple" is a newtype with constructor
                    // type UserId is (Int); should allow UserId(42)
                    let inner_type = resolved_types.first().cloned().unwrap_or(Type::Unit);

                    // Create a Named type for the newtype (not an alias)
                    let newtype_ty = Type::Named {
                        path: Path::single(type_decl.name.clone()),
                        args: List::new(),
                    };
                    // T0591: route through the module-aware path so the
                    // same-name ambient variant-ctor shadow is evicted (a
                    // stdlib variant like `CacheTtl.Seconds` otherwise
                    // captured `Seconds(1.5)` and forced its Int payload).
                    self.define_type_in_current_module(type_name.clone(), newtype_ty.clone());

                    // Store inner type for field access (.0)
                    self.define_declared_type_metadata("__newtype_inner_", type_name.as_str(), inner_type.clone());

                    // Register constructor function: UserId: fn(Int) -> UserId
                    self.register_declared_nominal_constructor(&type_name, Some(vec![inner_type].into_iter().collect()), type_decl.span)?;
                } else {
                    // Multi-element tuple: create Named type with constructor
                    let named_tuple_ty = Type::Named {
                        path: Path::single(type_decl.name.clone()),
                        args: List::new(),
                    };
                    self.define_type_in_current_module(type_name.clone(), named_tuple_ty.clone());

                    // Store tuple fields for field access (.0, .1, .2)
                    self.define_declared_type_metadata("__tuple_fields_", type_name.as_str(), Type::Tuple(resolved_types.clone()));

                    // Register constructor function: Color: fn(Int, Int, Int) -> Color
                    self.register_declared_nominal_constructor(&type_name, Some(resolved_types), type_decl.span)?;
                }
            }
            TypeDeclBody::Newtype(inner_type) => {
                // Newtype: type UserId is Int;
                // Register as a distinct named type (not just an alias)
                // This gives the newtype its own type identity
                let inner_resolved = self.ast_to_type(inner_type)?;

                // Create the newtype as a Named type
                let newtype_ty = Type::Named {
                    path: Path::single(type_decl.name.clone()),
                    args: List::new(),
                };
                self.define_type_in_current_module(type_name.clone(), newtype_ty.clone());

                // Store the inner type for field access (.0)
                self.define_declared_type_metadata("__newtype_inner_", type_name.as_str(), inner_resolved.clone());

                // Register a constructor function: UserId: fn(Int) -> UserId
                // This allows UserId(42) syntax
                self.register_declared_nominal_constructor(&type_name, Some(vec![inner_resolved].into_iter().collect()), type_decl.span)?;
            }
            TypeDeclBody::Protocol(_) => self.register_protocol_type_body(type_decl, &type_name)?,
            TypeDeclBody::Unit => {
                // Unit type (e.g., `type Empty;` or `type Signal is ();`)
                // Register as Named type wrapping Unit
                let ty = Type::Named {
                    path: Path::single(type_decl.name.clone()),
                    args: List::new(),
                };
                self.define_type_in_current_module(type_name.clone(), ty.clone());

                // Store inner type as Unit for newtype coercion checks
                // This allows `let s: Signal = ();` without explicit wrapping
                self.define_declared_type_metadata("__newtype_inner_", type_name.as_str(), Type::Unit);

                self.register_declared_nominal_constructor(&type_name, None, type_decl.span)?;
            }

            TypeDeclBody::Inductive(_) => {
                // Dependent type features (v2.0+) - register as named type for now
                let ty = Type::Named {
                    path: Path::single(type_decl.name.clone()),
                    args: List::new(),
                };
                self.define_type_in_current_module(type_name.clone(), ty);
            }
            TypeDeclBody::Coinductive(protocol_body) => {
                // Coinductive type — register named type and validate that the
                // protocol body declares at least one destructor (observation method).
                let ty = Type::Named {
                    path: Path::single(type_decl.name.clone()),
                    args: List::new(),
                };
                self.define_type_in_current_module(type_name.clone(), ty);

                // Verify that every protocol item that is a function has an explicit
                // return type declared (destructors must be typed observations).
                for item in &protocol_body.items {
                    use verum_ast::decl::ProtocolItemKind;
                    if let ProtocolItemKind::Function { decl, .. } = &item.kind {
                        if decl.return_type.is_none() {
                            tracing::warn!(
                                "coinductive type `{}`: destructor `{}` has no declared return type; \
                                 observations should have explicit return types",
                                type_name,
                                decl.name.name
                            );
                        }
                    }
                }
            }
            TypeDeclBody::SigmaTuple(types) => {
                // Dependent pair / sigma type (e.g., `type Interval is lo: Int, hi: Int where hi >= lo;`)
                // The each sigma-binding element parses as
                // `TypeKind::Refined { base, predicate }` with
                // `predicate.binding = Some(field_name)`.
                let element_types: List<Type> = types
                    .iter()
                    .map(|t| self.ast_to_type(t))
                    .collect::<Result<_>>()?;

                let ty = Type::Named {
                    path: Path::single(type_decl.name.clone()),
                    args: List::new(),
                };
                self.define_type_in_current_module(type_name.clone(), ty.clone());

                // Store element types for tuple-like access
                let inner_key = format!("__tuple_elements_{}", type_name);
                self.ctx
                    .define_type(inner_key.clone(), Type::Tuple(element_types));

                // Also register named fields as struct fields for field access (e.g., iv.lo, iv.hi)
                // Extract field names from the refined nodes' predicate binder.
                let mut fields = indexmap::IndexMap::new();
                for sigma_ty in types {
                    if let verum_ast::ty::TypeKind::Refined {
                        ref base,
                        ref predicate,
                    } = sigma_ty.kind
                    {
                        if let verum_common::Maybe::Some(ref name) = predicate.binding {
                            let field_type = self.ast_to_type(base)?;
                            fields.insert(name.name.clone(), field_type);
                        }
                    }
                }
                if !fields.is_empty() {
                    self.define_declared_type_metadata(
                        "__struct_fields_",
                        type_name.as_str(),
                        Type::Record(fields),
                    );
                }
            }
            TypeDeclBody::Quotient { base, .. } => {
                // Honour `[types].quotient = false` from Verum.toml:
                // reject the quotient declaration with a hard error
                // citing the manifest. The pre-fix behaviour was
                // unconditional alias registration regardless of the
                // flag — meaning a project that disabled quotient
                // types in its manifest still elaborated them
                // silently. Since the equivalence relation is not
                // yet enforced at runtime (T1-T phase 2 is partial),
                // permitting quotient declarations under `quotient =
                // false` would also let users believe the relation
                // is checked when it isn't — a soundness gap.
                if !self.quotient_enabled {
                    return Err(TypeError::Other(verum_common::Text::from(format!(
                        "type `{}` declares a quotient body but \
                             `[types].quotient` is disabled in Verum.toml — \
                             enable it to use HIT-based modular equivalence \
                             types, or rewrite as a plain type alias to \
                             the carrier",
                        type_name.as_str()
                    ))));
                }
                // T1-T phase 2: register the quotient type as an alias
                // over its underlying carrier. Values of the quotient
                // type are represented at runtime by values of the
                // carrier; the equivalence relation is a compile-time
                // constraint that the elaborator lowers to HIT path
                // constructors when full dependent-type codegen lands.
                //

                // Registering as both (a) the named carrier in the
                // type context AND (b) an alias to the base type in
                // the unifier makes the quotient type resolvable from
                // both name lookups and nominal-unification paths.
                let base_resolved = self.ast_to_type(base)?;
                self.declare_type_alias_target(type_name.clone(), base_resolved.clone());
                self.unifier
                    .register_type_alias(type_name.clone(), base_resolved.clone());
                let ty = Type::Named {
                    path: Path::single(type_decl.name.clone()),
                    args: List::new(),
                };
                self.define_type_in_current_module(type_name.clone(), ty);
            }
        }

        Ok(())
        })();

        self.finish_nominal_declaration_premises(previous_premises);

        // CRITICAL: Pop this declaration's parameter frame —
        // unconditionally, on BOTH success and failure of the body above
        // (GENERIC-PARAM-LEAK, T0148; frame mechanism, T0683).
        self.pop_decl_param_frame();

        body_result?;

        // NOTE: Cleanup of types_being_registered is now done in register_type_declaration_inner
        // to ensure it happens on BOTH success and error paths. This enables retry after failed
        // registration attempts (e.g., when a dependency like List wasn't available initially).

        Ok(())
    }

    /// ALIAS-VS-MARKER disambiguation.
    ///
    /// `type X is Y;` parses as a one-variant sum `{ Y }` because the
    /// recursive-descent parser has no symbol table — `looks_like_variant`
    /// (verum_fast_parser) cannot tell a fresh marker name (`type
    /// SemaphoreError is Closed;`) from a reference to an existing type
    /// (`type Port is UInt16;`, `type ValidationError is MemValidationError;`).
    /// The parser deliberately defaults the bare form to a sum (task #13,
    /// marker-error idiom).
    ///
    /// At this (semantic) layer the symbol table DOES exist, so we can
    /// recover the author's intent: when the sole nullary variant's name
    /// resolves to an already-registered type, the declaration is a
    /// transparent type ALIAS, not a degenerate sum. Pre-fix the sum
    /// interpretation surfaced under strict checking (AOT) as
    /// `expected 'UInt16(Unit)'` / `expected 'MemValidationError(Unit)'`
    /// type-mismatches at every use of the alias, while interpreter
    /// leniency masked it.
    ///
    /// Discovery is purely registration-driven (NO hardcoded stdlib/primitive
    /// name lists — see crate AGENTS.md): a name is an existing type iff
    /// `lookup_type` returns a rich (`Variant`/`Record`), `Named`, or scalar
    /// entry. Fresh marker names return `None` and keep sum semantics.
    ///
    /// Returns the synthetic AST alias target (`Path(Y)`) on a hit.
    pub(super) fn variant_decl_alias_target(
        &self,
        type_decl: &verum_ast::TypeDecl,
    ) -> Option<verum_ast::ty::Type> {
        use verum_ast::decl::TypeDeclBody;
        let TypeDeclBody::Variant(variants) = &type_decl.body else {
            return None;
        };
        // Exactly one variant, and it must be a bare nullary marker shape:
        // no payload, no GADT generics/where, no HIT path-endpoints, no
        // attributes. Anything richer is an intentional sum.
        if variants.len() != 1 {
            return None;
        }
        let v = &variants[0];
        if v.data.is_some()
            || !v.generic_params.is_empty()
            || v.where_clause.is_some()
            || v.path_endpoints.is_some()
            || !v.attributes.is_empty()
        {
            return None;
        }
        let target = v.name.name.as_str();
        // A self-named single variant (`type Closed is Closed;`) is a genuine
        // (if degenerate) marker — never collapse it to a self-alias.
        if target == type_decl.name.name.as_str() {
            return None;
        }
        // SCOPE GATE (#41 semaphore class): the existence check below reads
        // `type_defs`, a process-global flat namespace. Without a visibility
        // gate, an unrelated module's type (core.net.tls13's `Closed`)
        // flips this module's `type SemaphoreError is Closed;` from
        // marker-sum to a cross-module alias — and which way it goes
        // depends on registration order. When the registration flow knows
        // the declaring module's items (`alias_scope`), only targets the
        // module can see LOCALLY (own decls, braced/leaf mounts) or
        // language builtins are alias-eligible. Verified: every legitimate
        // single-name alias in core/ (56 of 57) has its target same-file,
        // braced-mounted, or builtin; the 57th IS the poison case.
        if let Some(scope) = &self.alias_scope {
            let is_builtin_name = matches!(
                target,
                "Int" | "Float" | "Bool" | "Char" | "Text" | "Unit" | "Byte"
                    | "Int8" | "Int16" | "Int32" | "Int64"
                    | "UInt8" | "UInt16" | "UInt32" | "UInt64"
                    | "ISize" | "USize" | "Float32" | "Float64"
                    | "I8" | "I16" | "I32" | "I64"
                    | "U8" | "U16" | "U32" | "U64"
            );
            if !is_builtin_name && !scope.contains(target) {
                return None;
            }
        }
        let is_existing_type = matches!(
            self.ctx.lookup_type(target),
            Option::Some(
                Type::Variant(_)
                    | Type::Record(_)
                    | Type::Named { .. }
                    | Type::Int
                    | Type::Float
                    | Type::Bool
                    | Type::Char
                    | Type::Text
            )
        );
        if !is_existing_type {
            return None;
        }
        Some(verum_ast::ty::Type {
            kind: verum_ast::ty::TypeKind::Path(verum_ast::ty::Path::single(v.name.clone())),
            span: v.span,
        })
    }

    /// Compute the ALIAS-VS-MARKER visibility set for a module's items
    /// (see `alias_scope` on the struct): the module's own type-decl
    /// names plus every leaf name its mounts bring in directly (braced
    /// `mount a.{B, C as D}` members and bare-path leaves `mount a.B`).
    /// Glob mounts contribute nothing — their contents cannot be decided
    /// locally, and every legitimate stdlib alias target is same-file,
    /// braced-mounted, or a language builtin.
    pub fn compute_alias_scope(
        items: &[verum_ast::Item],
    ) -> std::collections::HashSet<String> {
        verum_ast::decl::locally_visible_type_names(items)
    }

    /// Set the ALIAS-VS-MARKER scope from a module's items for the
    /// duration of a registration pass (public wrapper for pipeline
    /// callers outside this crate). Pair with [`Self::clear_alias_scope`].
    pub fn set_alias_scope_from_items(&mut self, items: &[verum_ast::Item]) {
        self.alias_scope = Some(Self::compute_alias_scope(items));
    }

    /// Clear the ALIAS-VS-MARKER scope after a registration pass.
    pub fn clear_alias_scope(&mut self) {
        self.alias_scope = None;
    }

    /// Register a sum (variant/enum) type declaration body.
    /// Handles import-provenance guards, placeholder registration for
    /// recursive types, variant constructor synthesis (with TypeVar substitution),
    /// and `__type_var_order_*` / `__type_params_*` metadata.
    fn register_variant_type_body(
        &mut self,
        type_decl: &verum_ast::TypeDecl,
        type_name: &verum_common::Text,
        publish_cases: bool,
    ) -> Result<()> {
        use crate::context::TypeScheme;
        use indexmap::IndexMap;
        use verum_ast::decl::{TypeDeclBody, VariantData};
        use verum_ast::ty::Path;
        use verum_common::Text;
        let TypeDeclBody::Variant(variants) = &type_decl.body
            else { unreachable!() };
        let type_name = type_name.clone();
                // IMPORT PROVENANCE: If this type name was explicitly imported and the existing
                // type in ctx has DIFFERENT variant names, skip the entire registration.
                // This prevents e.g., atomic Ordering (Relaxed|Acquire|...) from overwriting
                // comparison Ordering (Less|Equal|Greater) through any code path.
                // Must check BEFORE the placeholder registration below, which would overwrite
                // the existing Variant type with a Named placeholder.
                //

                // CRITICAL: Only block when NOT currently processing the explicit import itself.
                // The `in_explicit_import_registration` flag distinguishes:
                // - true: we are inside register_type_declaration called BY the explicit import → ALLOW
                // - false: some other path is trying to register a conflicting type → BLOCK
                if !self.in_explicit_import_registration
                    && self.explicit_imports.contains(type_name.as_str())
                {
                    if let Maybe::Some(Type::Variant(existing_variants)) =
                        self.ctx.lookup_type(type_name.as_str())
                    {
                        let existing_names: std::collections::BTreeSet<&str> =
                            existing_variants.keys().map(|k| k.as_str()).collect();
                        let new_names: std::collections::BTreeSet<&str> =
                            variants.iter().map(|v| v.name.name.as_str()).collect();
                        if existing_names != new_names {
                            // Different variant structure — protect the
                            // explicitly imported type. The parameter
                            // frame pops unconditionally at the outer
                            // cleanup, so this early return needs no
                            // local parameter cleanup of its own.
                            return Ok(());
                        }
                    }
                }

                // Register a placeholder type name FIRST to handle recursive type definitions
                // like: type List<T> is Nil | Cons(T, List<T>)
                // This enables recursive references to resolve before the type body is fully processed.
                //

                // CRITICAL FIX: Use Type::Named (not Type::Var) as placeholder so that
                // `List<T>` references in the variant body can construct Generic types.
                // Type::Var won't work because Generic { base: Type::Var, args } is not how
                // type resolution expects to find named types.
                let placeholder_type = Type::Named {
                    path: Path::single(verum_ast::ty::Ident::new(
                        type_name.as_str(),
                        Span::default(),
                    )),
                    args: List::new(), // Empty args - specific instances will fill them
                };
                // Module-aware: also publish under `{module_path}.{type_name}`
                // so cross-module name collisions (e.g. multiple `RecvError`
                // in stdlib) resolve to the declaring module's definition.
                self.define_type_in_current_module(type_name.clone(), placeholder_type);

                // Convert variant declarations to Type::Variant.
                // HIT path-constructors (variants with `path_endpoints`)
                // are recorded in the side-channel `hit_path_constructors`
                // map for use by HIT-aware tactics, while still emitting
                // a regular `Type::Variant` entry so that downstream
                // pattern-matching, exhaustiveness, and codegen continue
                // to function unmodified.
                let mut variant_map: IndexMap<Text, Type> = IndexMap::new();
                let mut hit_constructors: List<crate::ty::PathConstructor> = List::new();

                for variant in variants {
                    let variant_name: Text = variant.name.name.as_str().into();

                    // If this variant carries explicit path endpoints,
                    // record it as a HIT path-constructor. Endpoints are
                    // lowered through the structured `expr_to_eqterm`
                    // translator so the cubical normalizer sees real
                    // term structure (Var/Const/App/Lambda) rather than
                    // opaque debug strings.
                    if let verum_common::Maybe::Some((from_expr, to_expr)) = &variant.path_endpoints
                    {
                        let lhs = crate::expr_to_eqterm::expr_to_eq_term(from_expr);
                        let rhs = crate::expr_to_eqterm::expr_to_eq_term(to_expr);
                        let pc = crate::ty::PathConstructor {
                            name: variant_name.clone(),
                            type_params: List::new(),
                            args: List::new(),
                            path_type: crate::ty::PathEndpoints {
                                ty: Box::new(Type::Unknown),
                                lhs: Box::new(lhs),
                                rhs: Box::new(rhs),
                            },
                        };
                        hit_constructors.push(pc);
                    }

                    // Convert variant data to type
                    let payload_type = match &variant.data {
                        None => Type::Unit, // Unit variant (no payload)
                        Some(VariantData::Tuple(types)) => {
                            if types.len() == 1 {
                                // Single payload: Some(T) -> T
                                self.ast_to_type(&types[0])?
                            } else {
                                // Multiple payloads: Point(Float, Float) -> (Float, Float)
                                let converted: Result<List<Type>> =
                                    types.iter().map(|t| self.ast_to_type(t)).collect();
                                Type::Tuple(converted?)
                            }
                        }
                        Some(VariantData::Record(fields)) => {
                            // Record variant: Error { code: Int, message: Text }
                            // Use lenient resolution fallback for cross-module imports
                            let mut record_map: IndexMap<Text, Type> = IndexMap::new();
                            for field in fields {
                                let field_name: Text = field.name.name.as_str().into();
                                let field_type = match self.ast_to_type(&field.ty) {
                                    Ok(ty) => ty,
                                    Err(error) if self.resolving_source_signature => return Err(error),
                                    Err(_) => self.ast_to_type_lenient(&field.ty),
                                };
                                record_map.insert(field_name, field_type);
                            }
                            Type::Record(record_map)
                        }
                    };

                    // Register record variant fields for pattern matching + construction:
                    if let Type::Record(ref fields) = payload_type {
                        let owner = self.declared_type_key(&type_name);
                        self.ctx.define_type(
                            format!("__struct_fields_{owner}.{variant_name}"),
                            Type::Record(fields.clone()),
                        );
                        if publish_cases {
                            self.ctx.define_type(
                                format!("__struct_fields_{variant_name}"),
                                Type::Record(fields.clone()),
                            );
                        }
                    }

                    variant_map.insert(variant_name, payload_type);
                }

                // C2-WIRE V1+#154 unified call site —
                // strict-positivity check on the variant body via
                // the canonical `check_variant_body_positivity`
                // helper. Mirrors the kernel's CoreTerm-level
                // walker so any path that bypasses one still hits
                // the other. Five sites previously inlined this
                // pattern; #154 collapsed them into the helper.
                if let Err(violation) = crate::positivity::check_variant_body_positivity(
                    type_name.as_str(),
                    &variant_map,
                ) {
                    return Err(TypeError::PositivityViolation {
                        type_name: verum_common::Text::from(violation.type_name.as_str()),
                        constructor: verum_common::Text::from(violation.constructor.as_str()),
                        position: verum_common::Text::from(violation.position.as_str()),
                        span: type_decl.span,
                    });
                }

                let variant_type = Type::Variant(variant_map.clone());
                // Module-aware registration: the same name (e.g. `RecvError`)
                // can legitimately appear in multiple stdlib modules. Publish
                // under `{current_module}.{type_name}` alongside the flat key
                // so signatures inside this module resolve their own variant
                // regardless of load order.
                self.define_type_in_current_module(type_name.clone(), variant_type.clone());

                // Register HIT path-constructor metadata (if any) for
                // consumption by HIT-aware tactics like `cubical` and
                // `descent`. The Type::Variant lowering above remains
                // the primary representation for ordinary type checking.
                if !hit_constructors.is_empty() {
                    self.journal_variant_paths(&type_name);
                    self.hit_path_constructors
                        .insert(type_name.clone(), hit_constructors);
                }

                // Register variant constructors in the env for synth-mode resolution.
                // This enables `Rect { w: 4, h: 6 }` to be resolved as Shape.Rect
                // when the variant name is used without qualification.
                // Register record variant constructors in env for synth-mode:
                // `Rect { w: 4, h: 6 }` resolves as Shape.Rect via env lookup
                for (vname, payload_ty) in &variant_map {
                    if let Type::Record(fields) = payload_ty {
                        let ctor_type = Type::Function {
                            property_requirement: crate::computational_properties::PropertyRequirement::Unrestricted,
                            params: verum_common::List::from_iter([Type::Record(fields.clone())]),
                            return_type: Box::new(variant_type.clone()),
                            properties: None,
                            contexts: None,
                            type_params: verum_common::List::new(),
                        };
                        self.ctx
                            .env
                            .insert(vname.clone(), crate::context::TypeScheme::mono(ctor_type));
                    }
                }

                let parameters = self.declared_nominal_arguments(&type_name, type_decl.span)?;

                // CRITICAL: Register variant type to name mapping for instance method lookup
                // This allows err.error_code() to find methods defined on RegistryError
                if let Some(sig) = Self::variant_type_signature(&variant_type) {
                    self.register_variant_type_name_first_wins(sig.clone(), type_name.clone());
                    if let Some(relaxed_sig) = Self::variant_type_signature_relaxed(&variant_type) {
                        if relaxed_sig != sig {
                            self.register_variant_type_name_first_wins(
                                relaxed_sig,
                                type_name.clone(),
                            );
                        }
                    }

                    // CRITICAL FIX: Register original variant type and type var order with unifier
                    // This enables proper Generic<->Variant unification by allowing extraction
                    // of type args in the correct declaration order.
                    for key in [type_name.clone(), self.declared_type_key(&type_name)] {
                        self.unifier.register_original_variant_type(key.clone(), variant_type.clone());
                        self.unifier.register_variant_parameters(key, parameters.clone());
                    }
                }

                // CRITICAL: Register inductive constructors for pattern matching.
                // This enables pattern matching on variant types like Maybe<T>, Result<T,E>.
                // Without this, expand_generic_to_variant can't find the constructors.
                use crate::ty::InductiveConstructor;
                let mut inductive_constructors = List::new();
                for variant in variants {
                    let ctor_name: Text = variant.name.name.as_str().into();
                    let ctor_args: List<Box<Type>> = match &variant.data {
                        None => List::new(),
                        Some(VariantData::Tuple(types)) => types
                            .iter()
                            .filter_map(|t| self.ast_to_type(t).ok())
                            .map(Box::new)
                            .collect(),
                        Some(VariantData::Record(fields)) => {
                            // For record variants, collect field types as args
                            fields
                                .iter()
                                .filter_map(|f| self.ast_to_type(&f.ty).ok())
                                .map(Box::new)
                                .collect()
                        }
                    };

                    // Build return type: TypeName or TypeName<TypeVars...>
                    let return_type = if parameters.is_empty() {
                        Type::Named {
                            path: Path::single(verum_ast::ty::Ident::new(
                                type_name.as_str(),
                                Span::default(),
                            )),
                            args: List::new(),
                        }
                    } else {
                        Type::Generic {
                            name: type_name.clone(),
                            args: parameters.clone(),
                        }
                    };

                    let constructor = InductiveConstructor {
                        name: ctor_name,
                        type_params: self.declared_parameter_roster(&type_name, type_decl.span)?
                            .iter()
                            .map(|(name, parameter)| (name.clone(), Box::new(parameter.clone())))
                            .collect(),
                        args: ctor_args,
                        return_type: Box::new(return_type),
                    };
                    inductive_constructors.push(constructor);
                }

                // C2-WIRE V1 + #154 unified — second
                // strict-positivity gate, this time on the
                // well-typed inductive_constructors (with full
                // type_params and return_type bookkeeping).
                // Defence-in-depth alongside the variant_map check
                // above: both must agree, by construction, on what
                // counts as a positive position. Now uses the
                // structured PositivityViolation error so the
                // diagnostic carries span/code instead of an opaque
                // text payload.
                if let Err(violation) = crate::positivity::check_user_inductive(
                    type_name.as_str(),
                    &inductive_constructors,
                ) {
                    return Err(TypeError::PositivityViolation {
                        type_name: verum_common::Text::from(violation.type_name.as_str()),
                        constructor: verum_common::Text::from(violation.constructor.as_str()),
                        position: verum_common::Text::from(violation.position.as_str()),
                        span: type_decl.span,
                    });
                }

                let inductive_key = if publish_cases { type_name.clone() } else { self.declared_type_key(&type_name) };
                self.ctx.register_inductive_type(inductive_key, inductive_constructors);

                // Hidden representation metadata never grants a bare case binding.
                if publish_cases {
                    for variant in variants {
                        let name = &variant.name.name;
                        self.journal_variant_parent(name);
                        let parents = self.variant_constructor_parents.entry(name.clone()).or_default();
                        if !parents.iter().any(|parent| parent == &type_name) {
                            parents.push(type_name.clone());
                        }
                    }
                }

                // Also register variant constructors as values/functions
                // This allows usage like: let x = Red; or let y = Some(42);
                for variant in variants {
                    let variant_name: Text = variant.name.name.as_str().into();
                    let payload_type = variant_map.get(&variant_name).unwrap_or(&Type::Unit);

                    // Variant short names use "last declaration wins" semantics.
                    // Each type declaration registers its variant constructors as short names,
                    // overriding any previous registrations. This allows user-defined types
                    // to shadow stdlib types (e.g., Stream<T> Nil/Cons overrides List<T> Nil/Cons).
                    // Qualified names (Type.Variant) are always available for disambiguation.
                    // Protect variant short names from overriding important bindings:
                    // 1. Primitive type names (Int, Float, Bool, etc.) — always protected
                    // 2. Polymorphic/function constructors from being downgraded by
                    //  monomorphic unit variants (prevents Keyword.Some overriding Maybe.Some)
                    // 3. Core constructors (Some, None, Ok, Err) — first-registered-wins
                    // Qualified names (Type.Variant) are always registered regardless.
                    let is_primitive_type_name = {
                        let vn = variant_name.as_str();
                        verum_common::well_known_types::type_names::is_primitive_value_type(vn)
                            || matches!(vn, "Bytes" | "UInt")
                    };
                    let short_name_exists = if !publish_cases || is_primitive_type_name {
                        true
                    } else if let Some(existing) = self.ctx.env.lookup(&variant_name) {
                        if self.user_code_phase {
                            // In user code phase: user-defined variants always shadow
                            // stdlib variants. Users must be able to define any variant
                            // name (e.g., Status.Success shadows StealResult.Success).
                            false
                        } else {
                            // During stdlib loading: protect rich/generic bindings
                            // (e.g., Heap<T>, Maybe<T>.None) from being overridden by
                            // variant constructors from other types.
                            !existing.vars.is_empty()
                                || matches!(existing.ty, Type::Function { .. })
                                || matches!(existing.ty, Type::Generic { .. })
                        }
                    } else {
                        false
                    };

                    if *payload_type == Type::Unit {
                        // Unit variant: register as a value of the variant type
                        // e.g., Red : Color
                        // For generic types, we need to quantify over type variables
                        // e.g., None : ∀T. Maybe<T>
                        self.register_sum_constructor(type_decl, &variant_name, variant_type.clone(), !short_name_exists)?;
                        // CRITICAL: Also register unit variants as TYPES (not just values).
                        // This enables variant names to be used as type arguments for meta
                        // type parameters. For example, in:
                        //  type Register<T, MODE: meta AccessMode> is { ... };
                        //  let reg: Register<UInt32, ReadOnly> = ...;
                        // The type checker resolves `ReadOnly` via lookup_type(), which
                        // searches type_defs, not the value environment.
                        // Each variant gets its own distinct Named type so that
                        // Register<T, ReadOnly> != Register<T, WriteOnly>.
                        if !short_name_exists {
                            let variant_named_type = Type::Named {
                                path: Path::single(verum_ast::ty::Ident::new(
                                    variant_name.as_str(),
                                    Span::default(),
                                )),
                                args: List::new(),
                            };
                            self.ctx
                                .define_type(variant_name.clone(), variant_named_type);
                        }
                        // Always register qualified type name
                        let qualified_type_name: Text =
                            format!("{}.{}", self.declared_type_key(&type_name), variant_name).into();
                        let qualified_variant_type = Type::Named {
                            path: Path::single(verum_ast::ty::Ident::new(
                                variant_name.as_str(),
                                Span::default(),
                            )),
                            args: List::new(),
                        };
                        if publish_cases {
                            self.ctx.define_type(format!("{type_name}.{variant_name}"), qualified_variant_type.clone());
                        }
                        self.ctx.define_type(qualified_type_name, qualified_variant_type);
                    } else {
                        // Variant with payload: register as a constructor function
                        // e.g., Some : ∀T. T -> Maybe<T>
                        // For multi-field variants, unpack tuple into multiple parameters
                        // e.g., Rectangle : (Float, Float) -> Shape
                        let params = match payload_type {
                            Type::Tuple(tuple_types) => tuple_types.clone(),
                            _ => {
                                let mut p = List::new();
                                p.push(payload_type.clone());
                                p
                            }
                        };
                        let constructor_ty = Type::function(params, variant_type.clone());

                        // CRITICAL FIX: For generic variant types like Maybe<T>, the constructor
                        // function contains free type variables that must be quantified.
                        // Without this, nested constructors like Some(Some(42)) cause infinite
                        // type errors because the same type variable gets unified with itself.
                        //

                        // Example: Some : ∀T. T -> Maybe<T>
                        // When called as Some(Some(42)):
                        //  - Inner Some(42): instantiates fresh T1, giving Maybe<Int>
                        //  - Outer Some(...): instantiates fresh T2, giving Maybe<Maybe<Int>>
                        self.register_sum_constructor(type_decl, &variant_name, constructor_ty, !short_name_exists)?;

                        // CRITICAL: Also register non-unit record variants as TYPES.
                        // This enables:
                        //  1. `implement Protocol for VariantName { ... }` syntax
                        //  2. `x is VariantName` type test patterns in match arms
                        // Each record variant gets its payload type as the type definition.
                        // For example: `type Widget is | Button { label: Text } | ...`
                        // registers `Button` as type `{ label: Text }`.
                        if let Type::Record(_) = payload_type {
                            if !short_name_exists {
                                self.ctx
                                    .define_type(variant_name.clone(), payload_type.clone());
                            }
                            // Always register qualified type name
                            let owner = self.declared_type_key(&type_name);
                            self.ctx.define_type(format!("{owner}.{variant_name}"), payload_type.clone());
                            if publish_cases {
                                self.ctx.define_type(format!("{type_name}.{variant_name}"), payload_type.clone());
                            }
                        }
                    }
                }
        Ok(())
    }

    /// Form a record in declaration order. Only a preceding field whose
    /// annotation is a universe introduces a type binder. The declaration
    /// frame also owns its generic parameters and is popped on every exit.
    fn elaborate_record_fields(
        &mut self,
        owner: &Text,
        fields: &[RecordField],
        allow_forward_declarations: bool,
    ) -> Result<indexmap::IndexMap<Text, Type>> {
        let mut entries = indexmap::IndexMap::with_capacity(fields.len());
        for (ordinal, field) in fields.iter().enumerate() {
            let name = &field.name.name;
            if entries.contains_key(name) {
                return Err(TypeError::Other(format!("duplicate field `{name}` in `{owner}`").into()));
            }
            let ty = match self.declared_type(&field.ty) {
                Ok(ty) => ty,
                Err(TypeError::TypeNotFound { name: missing, span })
                    if allow_forward_declarations && !self.resolving_source_signature =>
                {
                    // Only a forward *declaration* may be deferred. A missing
                    // name that designates this very record's field is already
                    // known to be unavailable as a type in this prefix.
                    if fields.iter().any(|field| field.name.name == missing) {
                        return Err(TypeError::TypeNotFound { name: missing, span });
                    }
                    self.ast_to_type_lenient(&field.ty)
                }
                Err(error) => return Err(error),
            };
            if let Type::Universe { level } = &ty {
                let slot = crate::ty::RecordFieldSlot { owner: owner.clone(), ordinal };
                self.decl_param_frames.last_mut()
                    .expect("record fields are checked inside their declaration frame")
                    .insert(name.clone(), Type::RecordField {
                        slot: Box::new(slot), receiver: None, level: level.clone(),
                    });
            }
            entries.insert(name.clone(), ty);
        }
        Ok(entries)
    }

    fn publish_record_fields(
        &mut self,
        name: &Text,
        fields: indexmap::IndexMap<Text, Type>,
        span: Span,
    ) -> Result<()> {
        // An ordinary record cannot bind a type-valued slot. Declarations
        // which do bind one share the projection/constructor layout validator.
        if fields.values().any(|ty| matches!(ty, Type::Universe { .. }))
            && let Some(owner) = super::dependent_records::dependent_record_owner(&fields, span)?
        {
            if owner != self.declared_type_key(name) {
                return Err(TypeError::UniverseBinding {
                    message: format!("dependent fields in `{name}` belong to `{owner}`").into(), span,
                });
            }
            self.define_declared_type_metadata("__dependent_record_", name, Type::Unit);
            self.ctx.define_type("__has_dependent_record_layouts", Type::Unit);
        }
        self.define_declared_type_metadata("__struct_fields_", name, Type::Record(fields));
        Ok(())
    }

    /// Register a record (struct-like) type declaration body.
    /// Resolves field types, registers `__type_params_*` for generic arity,
    /// emits the Record type and method-dispatch stubs.
    fn register_record_type_body(
        &mut self,
        type_decl: &verum_ast::TypeDecl,
        type_name: &verum_common::Text,
    ) -> Result<()> {
        use crate::context::TypeScheme;
        use indexmap::IndexMap;
        use verum_ast::decl::{TypeDeclBody, VariantData};
        use verum_ast::ty::Path;
        use verum_common::Text;
        let TypeDeclBody::Record(fields) = &type_decl.body
            else { unreachable!() };
        self.register_record_field_visibility(type_name.as_str(), fields);
        // Field binders may shadow declaration parameters in the field telescope.
        self.register_declared_type_arity(type_name, &type_decl.generics)?;
        let type_name = type_name.clone();
                // CRITICAL: Register a placeholder type FIRST to handle recursive type definitions
                // and prevent infinite recursion when processing field types.
                // Example: type Node is { next: Maybe<Node> }
                // Without this placeholder, resolving Maybe<Node> would try to resolve Node again.
                let placeholder_var = TypeVar::fresh();
                self.define_type_in_current_module(type_name.clone(), Type::Var(placeholder_var));

                let owner = self.declared_type_key(type_name.as_str());
                let record_map = self.elaborate_record_fields(&owner, fields, true)?;

                // C2-WIRE V3 + #154 unified call site —
                // strict-positivity on record-shaped types via the
                // canonical helper. The placeholder TypeVar is
                // passed through so post-elaboration `Type::Var`
                // occurrences (left over from the recursive
                // pre-pass placeholder) are recognised as the
                // recursive type.
                if let Err(violation) = crate::positivity::check_record_body_positivity(
                    type_name.as_str(),
                    &record_map,
                    Some(placeholder_var),
                ) {
                    return Err(TypeError::PositivityViolation {
                        type_name: verum_common::Text::from(violation.type_name.as_str()),
                        constructor: verum_common::Text::from(violation.constructor.as_str()),
                        position: verum_common::Text::from(violation.position.as_str()),
                        span: type_decl.span,
                    });
                }

                // CRITICAL: Empty records (e.g., `type AtomicUsize is { };`) are opaque builtin types.
                // These are used for types that have runtime representations but no accessible fields.
                // They must be treated as opaque Named types, NOT as structural empty records.
                //

                // Examples:
                // - `type AtomicUsize is { };` - Opaque atomic type for thread-safe counters
                // - `type Mutex<T> is { };` - Opaque synchronization primitive
                //

                // Empty records are registered ONLY as Named types without field information.
                // This prevents type checker from treating them as structural `{ }` types.
                if fields.is_empty() {
                    // Register as opaque Named type (no field structure)
                    let named_type = self.declared_record_type(type_decl)?;
                    // Store ONLY the named type, no __struct_fields_ entry
                    self.define_type_in_current_module(type_name.clone(), named_type);

                    // Skip field structure registration for empty records
                    // This prevents `{ }` from being used as the type in error messages
                } else {
                    // Non-empty record: Register as Named type with field structure

                    // IMPORTANT: Register as Named type to preserve nominal identity
                    // This allows protocol implementations to be looked up by name (e.g., "Point")
                    // rather than by structural type (e.g., "{ x: Int, y: Int }")
                    //

                    // The record structure is stored separately for field access validation
                    let named_type = self.declared_record_type(type_decl)?;

                    // Store the named type for protocol method dispatch
                    self.define_type_in_current_module(type_name.clone(), named_type.clone());

                    // CRITICAL FIX: Resolve placeholder TypeApp references in record field types.
                    // During recursive type registration, self-references like Node<T> in
                    // `type Node<T> is { next: Maybe<Heap<Node<T>>> }` become
                    // TypeApp { constructor: Var(placeholder_var), args: [Named("T")] }
                    // because Node was only registered as Var(placeholder) at resolution time.
                    // Now that we have the real Named type, replace these TypeApp references
                    // so that subsequent type checking can unify them correctly.
                    let record_map = {
                        fn resolve_placeholder(
                            ty: &Type,
                            placeholder: TypeVar,
                            type_path: &verum_ast::ty::Path,
                        ) -> Type {
                            match ty {
                                // Handle bare self-reference: Var(placeholder) -> Named { path, args: [] }
                                Type::Var(tv) if *tv == placeholder => Type::Named {
                                    path: type_path.clone(),
                                    args: List::new(),
                                },
                                Type::TypeApp { constructor, args } => {
                                    if let Type::Var(tv) = constructor.as_ref() {
                                        if *tv == placeholder {
                                            let resolved_args: List<Type> = args
                                                .iter()
                                                .map(|a| {
                                                    resolve_placeholder(a, placeholder, type_path)
                                                })
                                                .collect();
                                            return Type::Named {
                                                path: type_path.clone(),
                                                args: resolved_args,
                                            };
                                        }
                                    }
                                    let resolved_ctor =
                                        resolve_placeholder(constructor, placeholder, type_path);
                                    let resolved_args: List<Type> = args
                                        .iter()
                                        .map(|a| resolve_placeholder(a, placeholder, type_path))
                                        .collect();
                                    Type::TypeApp {
                                        constructor: Box::new(resolved_ctor),
                                        args: resolved_args,
                                    }
                                }
                                Type::Generic { name, args } => {
                                    let resolved_args: List<Type> = args
                                        .iter()
                                        .map(|a| resolve_placeholder(a, placeholder, type_path))
                                        .collect();
                                    Type::Generic {
                                        name: name.clone(),
                                        args: resolved_args,
                                    }
                                }
                                Type::Named { path, args } => {
                                    let resolved_args: List<Type> = args
                                        .iter()
                                        .map(|a| resolve_placeholder(a, placeholder, type_path))
                                        .collect();
                                    Type::Named {
                                        path: path.clone(),
                                        args: resolved_args,
                                    }
                                }
                                Type::Record(fields) => {
                                    let resolved: indexmap::IndexMap<Text, Type> = fields
                                        .iter()
                                        .map(|(k, v)| {
                                            (
                                                k.clone(),
                                                resolve_placeholder(v, placeholder, type_path),
                                            )
                                        })
                                        .collect();
                                    Type::Record(resolved)
                                }
                                Type::Variant(variants) => {
                                    let resolved: indexmap::IndexMap<Text, Type> = variants
                                        .iter()
                                        .map(|(k, v)| {
                                            (
                                                k.clone(),
                                                resolve_placeholder(v, placeholder, type_path),
                                            )
                                        })
                                        .collect();
                                    Type::Variant(resolved)
                                }
                                Type::Tuple(elems) => Type::Tuple(
                                    elems
                                        .iter()
                                        .map(|e| resolve_placeholder(e, placeholder, type_path))
                                        .collect(),
                                ),
                                Type::Reference { inner, mutable } => Type::Reference {
                                    inner: Box::new(resolve_placeholder(
                                        inner,
                                        placeholder,
                                        type_path,
                                    )),
                                    mutable: *mutable,
                                },
                                Type::Function {
                                    property_requirement,
                                    params,
                                    return_type,
                                    contexts,
                                    type_params,
                                    properties,
                                } => Type::Function {
                                    property_requirement: property_requirement.clone(),
                                    params: params
                                        .iter()
                                        .map(|p| resolve_placeholder(p, placeholder, type_path))
                                        .collect(),
                                    return_type: Box::new(resolve_placeholder(
                                        return_type,
                                        placeholder,
                                        type_path,
                                    )),
                                    contexts: contexts.clone(),
                                    type_params: type_params.clone(),
                                    properties: properties.clone(),
                                },
                                _ => ty.clone(),
                            }
                        }
                        let type_path = verum_ast::ty::Path::single(type_decl.name.clone());
                        let mut resolved_map = indexmap::IndexMap::new();
                        for (fname, fty) in record_map.iter() {
                            resolved_map.insert(
                                fname.clone(),
                                resolve_placeholder(fty, placeholder_var, &type_path),
                            );
                        }
                        resolved_map
                    };

                    // Also store the record structure under a different key for field access
                    // This allows us to validate field accesses while keeping the nominal type
                    self.publish_record_fields(&type_name, record_map, type_decl.span)?;

                }
        Ok(())
    }

    /// Register a protocol (trait-like) type declaration body.
    /// Sets up Self type, processes method signatures + associated types,
    /// registers the protocol in protocol_checker, and emits the Named type.
    fn register_protocol_type_body(
        &mut self,
        type_decl: &verum_ast::TypeDecl,
        type_name: &verum_common::Text,
    ) -> Result<()> {
        use crate::context::TypeScheme;
        use indexmap::IndexMap;
        use verum_ast::decl::{TypeDeclBody, VariantData};
        use verum_ast::ty::Path;
        use verum_common::Text;
        let TypeDeclBody::Protocol(protocol_body) = &type_decl.body
            else { unreachable!() };
        self.validate_protocol_type_declaration(type_decl, protocol_body)?;
        self.register_declared_type_arity(type_name, &type_decl.generics)?;
        let type_name = type_name.clone();
                // Protocol types (e.g., `type Database is protocol { ... }`)
                // Register as Named type for type checking
                let ty = Type::Named {
                    path: Path::single(type_decl.name.clone()),
                    args: List::new(),
                };
                self.define_type_in_current_module(type_name.clone(), ty);

                // CRITICAL FIX: Register protocol with its methods in protocol_checker
                // This enables method lookup for bounded generic types like `fn foo<T: Display>(item: T)`
                // Without this, calling protocol methods on bounded type parameters fails.
                use verum_ast::decl::ProtocolItemKind;

                // Self type is represented as Named type with path "Self"
                let self_type = Type::Named {
                    path: Path::single(Ident::new("Self", Span::default())),
                    args: List::new(),
                };

                // CRITICAL: Register "Self" in type environment so that Self.Item can be resolved
                // when parsing method signatures. This allows ast_to_type to recognize "Self"
                // as a valid type name during protocol registration.
                self.ctx
                    .define_type(verum_common::Text::from("Self"), self_type.clone());

                // CRITICAL: Also set current_self_type so that PathSegment::SelfValue
                // (which the parser produces for 'Self') can be resolved.
                // Save the old value to restore later.
                let old_self_type = self.current_self_type.clone();
                self.set_current_self_type(Maybe::Some(self_type.clone()));

                let registration = (|| -> Result<()> {

                // CRITICAL FIX: Register protocol's type parameters (like T in Into<T>) BEFORE
                // processing method signatures. This allows the return type `T` to be resolved
                // as Type::Var instead of Type::Named, enabling proper type inference with
                // blanket implementations.
                //

                // Track which type params we register so we can clean them up after
                let mut protocol_type_param_names: List<Text> = List::new();
                for generic_param in &type_decl.generics {
                    use verum_ast::ty::GenericParamKind;
                    if let GenericParamKind::Type { name, .. } = &generic_param.kind {
                        let type_var = TypeVar::fresh();
                        let param_name: Text = name.name.clone();
                        self.ctx
                            .define_type(param_name.clone(), Type::Var(type_var));
                        protocol_type_param_names.push(param_name);
                    }
                }

                let (constraint_scope, declared_supers) = self.protocol_constraint_scope(
                    &type_decl.generics, &protocol_body.extends,
                    &[type_decl.generic_where_clause.as_ref(), protocol_body.generic_where_clause.as_ref()],
                );

                let super_protocols = match declared_supers {
                    Some(supers) => supers,
                    None => self.convert_super_protocols(&protocol_body.extends)?,
                };
                let (protocol_methods, protocol_assoc_types, dyn_required) = self.elaborate_protocol_members(
                    self.declared_type_key(type_name.as_str()), &type_decl.generics,
                    &constraint_scope, &super_protocols, &protocol_body.items, &protocol_body.declared_default_methods,
                )?;

                // Create and register Protocol object
                // Convert AST is_context bool to ProtocolKind:
                // - true -> ConstraintAndInjectable (context protocol)
                // - false -> Constraint (regular protocol)
                let kind = if protocol_body.is_context {
                    crate::protocol::ProtocolKind::ConstraintAndInjectable
                } else {
                    crate::protocol::ProtocolKind::Constraint
                };

                let protocol = crate::protocol::Protocol {
                    source_parameters: Some(std::sync::Arc::new(type_decl.generics.clone())),
                    constraints: constraint_scope,
                    name: self.declared_type_key(type_name.as_str()),
                    kind,
                    type_params: self.convert_generic_params_to_type_params(&type_decl.generics)?,
                    methods: protocol_methods.clone(),
                    associated_types: protocol_assoc_types,
                    associated_consts: Map::new(),
                    super_protocols,
                    specialization_info: Maybe::None,
                    defining_crate: Maybe::None,
                    span: type_decl.span,
                };
                self.check_pending_protocol_object_safety(&protocol, dyn_required)?;
                self.check_protocol_family_applications(&protocol)?;
                self.unifier.register_protocol_name(protocol.name.as_str());
                // This run parsed the declaration, so its default-body flags are
                // trustworthy — see `protocols_with_known_defaults`.
                self.protocols_with_known_defaults.insert(
                    protocol.name.clone(),
                    (
                        protocol
                            .methods
                            .iter()
                            .filter(|(_, m)| !m.has_default)
                            .map(|(n, _)| n.clone())
                            .collect(),
                        protocol.methods.keys().cloned().collect(),
                    ),
                );
                // Capture the predecessor and installed identity atomically with
                // this write. A failed recursive load must not undo another
                // checker's later publication of the same protocol name.
                let mut registry = self.protocol_checker.write();
                if let Some(binding) = self.source_type_registration.protocols.get_mut(&protocol.name) {
                    let publication = registry.register_protocol_with_receipt(protocol)
                        .map_err(|error| TypeError::UniverseBinding {
                            message: error.to_string().into(),
                            span: type_decl.span,
                        })?;
                    binding.publications.push(publication);
                } else {
                    let _ = registry.register_protocol(protocol);
                }
                drop(registry);

                // IMPORTANT: Register protocols correctly based on their context status.
                //

                // Verum distinguishes between:
                // - **Constraint protocols**: `type Comparable is protocol { ... }` - for `where T: Comparable`
                // - **Context protocols**: `context type Database is protocol { ... }` - for `using [Database]`
                //

                // Only context protocols can be used in `using [...]` dependency injection clauses.
                // Constraint protocols are registered separately to provide better error messages.
                //

                // Context type system integration: context requirements tracked in function types, checked at call sites — Type System Integration
                // Injectable bindings publish with the completed source
                // transaction, never while dependency registration is partial.
                if !protocol_body.is_context {
                    // Register as constraint protocol for better error messages
                    self.context_resolver
                        .register_constraint_protocol(type_name);
                }

                Ok(())
                })();
                // Restore the receiver scope even when a method signature fails.
                self.set_current_self_type(old_self_type);
        registration
    }

    // ==================== Two-Pass Type Resolution ====================
    //

    // Order-independent type resolution uses two passes:
    //

    // Pass 1 (register_type_name_only): Register type names as placeholders.
    //  This allows forward references between types defined in any order.
    //

    // Pass 2 (resolve_type_definition): Resolve the full type definition.
    //  All type names are now available, so references can be resolved.
    //

    // Example:
    // ```verum
    // type SearchRequest is {
    //  sort_by: SortOrder, // Forward reference - SortOrder not yet defined
    // };
    // type SortOrder is Relevance | Downloads;
    // ```
    //

    // After Pass 1: SearchRequest -> Placeholder, SortOrder -> Placeholder
    // After Pass 2: Both fully resolved

    /// T0266 — ONE authority for recording a dependent family's declared
    /// value-parameter arity. Both registration entry points feed it:
    /// the two-pass flow (`register_type_name_only`, used by the check /
    /// compile pipeline) and the single-shot flow
    /// (`register_type_declaration`, used by module registration). The
    /// `DependentApp` arm of `ast_to_type` consults the map to reject
    /// `T<A>(x, y, z)` against a two-param declaration. Simple-name key —
    /// unqualified heads only; qualified/aliased carriers skip the check.
    fn record_dependent_family(&mut self, type_decl: &verum_ast::TypeDecl) {
        if type_decl.value_params.is_empty() {
            return;
        }
        let type_name: verum_common::Text = type_decl.name.name.as_str().into();
        if std::env::var_os("VERUM_TRACE_DEPAPP").is_some() {
            eprintln!(
                "[depapp] record family {} -> {}",
                type_name,
                type_decl.value_params.len()
            );
        }
        self.ctx
            .dependent_value_params
            .insert(type_name, type_decl.value_params.len());
    }

    pub fn register_type_name_only(&mut self, type_decl: &verum_ast::TypeDecl) {
        use verum_common::Text;

        let type_name: Text = type_decl.name.name.as_str().into();
        let span = type_decl.span;
        self.record_dependent_family(type_decl);

        // Check if this type is already defined (e.g., builtin types like List, Map, etc.)
        // During stdlib loading: don't override existing definitions with placeholders.
        // During user code phase: user types ALWAYS override stdlib types — the user's
        // type declaration takes precedence over any stdlib type with the same name.
        if let Some(existing_ty) = self.ctx.lookup_type(type_name.as_str()) {
            if !matches!(existing_ty, Type::Placeholder { .. }) && !self.user_code_phase {
                tracing::debug!(
                    "Pass 1: Skipping placeholder for {} - already defined as {:?}",
                    type_name,
                    existing_ty
                );
                return;
            }
            if self.user_code_phase && !matches!(existing_ty, Type::Placeholder { .. }) {
                // Clean up stale stdlib alias for this type name.
                // Without this, the old alias (e.g., Mask4 -> Mask<4>) would persist
                // and override the user's type definition during type resolution.
                self.ctx.remove_alias(type_name.as_str());
                self.unifier.remove_type_alias(type_name.as_str());
            }
        }

        // Architectural: during user-code phase, evict any stdlib variant-
        // constructor binding sitting on this simple name. Variant
        // constructors landed in `env` via `env.insert_mono(variant_name,
        // fn(...) -> ParentType)` (see `resolve_type_body::Variant`).
        // If a stdlib type like `HandshakeRole is Client(ClientSm) | ...`
        // pre-registered `Client` as a function, and the user then
        // declares `type Client is { ... }`, every `Client.new(...)` call
        // resolves through the stdlib function — user's static method
        // lookup never runs. The user's declaration is always authoritative
        // in their own module, so wipe the bare-name binding here.
        //

        // Qualified bindings like `HandshakeRole.Client` stay intact, so
        // stdlib callers that use the qualified form keep working.
        // A UNIT type's constructor is bound as `Named`, not as a
        // Function and not as a Variant — so the two-case list above
        // walked past it, and a user type whose name matches a stdlib
        // one kept the stdlib binding in value position while the
        // ANNOTATION resolved locally. Measured, three lines, standalone
        // file (T0993):
        //
        //     public type TimeoutError is ();
        //     fn make() -> TimeoutError { TimeoutError }
        //     error<E400>: expected 'TimeoutError', found 'SelectTimeoutError'
        //
        // One identifier, two meanings, in one file. The same shape with
        // a unique name passes, and a RECORD body passes — a record's
        // value is a literal, so it never consults this binding.
        //
        // The surface is the whole library: 5677 public type names in
        // `core/`, all bare-visible, and 104 of them declared more than
        // once inside `core/` itself.
        if self.user_code_phase && self.ctx.env.lookup(type_name.as_str()).is_some() {
            let should_evict = {
                use crate::ty::Type as T;
                matches!(
                    self.ctx.env.lookup(type_name.as_str()).map(|s| &s.ty),
                    Some(T::Function { .. }) | Some(T::Variant(_)) | Some(T::Named { .. })
                )
            };
            if should_evict {
                let _ = self.ctx.env.remove(type_name.as_str());
            }
        }

        // Register as placeholder - will be resolved in pass 2
        let placeholder = Type::Placeholder {
            name: self.declared_type_key(type_name.as_str()),
            span,
        };
        self.define_type_in_current_module(type_name.clone(), placeholder);

        // Also register generic type parameters as placeholders if present
        // This allows generic types like Maybe<T> to be referenced
        if !type_decl.generics.is_empty() {
            // Store the number of type parameters for later validation
            let type_params_key: Text = format!("__type_params_count_{}", type_name).into();
            // Use a simple marker to remember this is a generic type
            self.ctx.define_type(type_params_key, Type::Int);
        } else if self.user_code_phase {
            // User type has no generics — clean up stale generic markers from stdlib.
            // Without this, the type checker would still think this is a generic type
            // and instantiate it as `TypeName<_>` instead of plain `TypeName`.
            let type_params_key: Text = format!("__type_params_count_{}", type_name).into();
            self.ctx.remove_type(&type_params_key);
            let type_params_detail_key: Text = format!("__type_params_{}", type_name).into();
            self.ctx.remove_type(&type_params_detail_key);
            let type_var_order_key: Text = format!("__type_var_order_{}", type_name).into();
            self.ctx.remove_type(&type_var_order_key);
        }

        tracing::debug!("Pass 1: Registered type placeholder for: {}", type_name);
    }

    /// Process import aliases to register type aliases.
    ///
    /// This handles imports like `import module.{IoError as EngineIoError}` by
    /// registering `EngineIoError` as a type alias that resolves to `IoError`.
    ///
    /// This is called in Pass 0 of stdlib compilation to ensure import aliases
    /// are available before type registration passes reference them.
    ///
    /// Constant initialization ordering: topological sort of dependencies, cycle detection for const declarations — Import Aliases
    pub fn process_import_aliases(&mut self, import: &verum_ast::MountDecl) {
        use verum_ast::MountTreeKind;
        use verum_common::Text;

        /// Extract the last segment of a path as the item name
        fn extract_item_name(path: &verum_ast::ty::Path) -> Option<String> {
            path.segments.last().and_then(|seg| {
                if let verum_ast::ty::PathSegment::Name(ident) = seg {
                    Some(ident.name.as_str().to_string())
                } else {
                    None
                }
            })
        }

        /// Extract the full module path from an AST Path, joined by dots.
        /// e.g. `core.net.h3.qpack.static_table` → "core.net.h3.qpack.static_table".
        fn extract_full_path(path: &verum_ast::ty::Path) -> String {
            path.segments
                .iter()
                .filter_map(|seg| {
                    if let verum_ast::ty::PathSegment::Name(ident) = seg {
                        Some(ident.name.as_str().to_string())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join(".")
        }

        /// Process a single import tree for aliases
        fn process_tree(
            tree: &verum_ast::MountTree,
            ctx: &mut crate::context::TypeContext,
            module_aliases: &mut Map<Text, Text>,
            inline_modules: &Map<Text, verum_ast::decl::ModuleDecl>,
        ) {
            if let MountTreeKind::Path(path) = &tree.kind {
                // Check if this import has an alias
                if let Some(alias) = &tree.alias {
                    if let Some(original_name) = extract_item_name(path) {
                        let alias_name: Text = alias.name.as_str().into();
                        let original_text: Text = original_name.into();

                        // Register the alias as pointing to the original type
                        // We use a Named type reference so that when the alias is looked up,
                        // it resolves to the original type
                        let type_ref = Type::Named {
                            path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                                original_text.as_str(),
                                verum_ast::Span::dummy(),
                            )),
                            args: verum_common::List::new(),
                        };
                        ctx.define_type(alias_name.clone(), type_ref);

                        // If the aliased path resolves to a known module,
                        // register a module alias too so that later method-
                        // call dispatch (`alias.method(...)`) routes through
                        // module-path lookup rather than competing with a
                        // same-named stdlib value symbol. Example:
                        //  mount core.net.h3.qpack.static_table as stat;
                        // `stat` would otherwise be shadowed by
                        // core.sys.linux.syscall.stat (the POSIX stat fn).
                        let full_path: Text = extract_full_path(path).into();
                        if inline_modules.contains_key(&full_path) {
                            module_aliases.insert(alias_name.clone(), full_path.clone());
                            tracing::debug!(
                                "Pass 0: Registered module alias: {} -> {}",
                                alias_name,
                                full_path,
                            );
                        }

                        tracing::debug!(
                            "Pass 0: Registered import alias: {} -> {}",
                            alias_name,
                            original_text
                        );
                    }
                }
            }
        }

        // Process the import tree for aliases
        match &import.tree.kind {
            MountTreeKind::Nested { trees, .. } => {
                // Process each nested import for potential aliases
                for tree in trees {
                    process_tree(
                        tree,
                        &mut self.ctx,
                        &mut self.module_aliases,
                        &self.inline_modules,
                    );
                }
            }
            MountTreeKind::Path(_) => {
                // Single import might have alias at top level
                process_tree(
                    &import.tree,
                    &mut self.ctx,
                    &mut self.module_aliases,
                    &self.inline_modules,
                );
            }
            MountTreeKind::Glob(_) => {
                // Glob imports don't have aliases
            }
            // #5 / P1.5 — file-relative mount aliases are
            // captured by the session loader when it
            // registers the resolved file as a module; the
            // type-inference alias pipeline doesn't add
            // anything more.
            MountTreeKind::File { .. } => {}
        }
    }

    /// Pass 2: Resolve full type definition.
    ///
    /// This is the second pass of two-pass type resolution. All type names
    /// are now registered as placeholders, so we can resolve the full
    /// type definitions including forward references.
    ///
    /// # Cycle Detection
    ///
    /// Detects and reports cyclic type definitions that would cause infinite size:
    /// ```verum
    /// type A is { b: B }; // ERROR: A -> B -> A cycle without indirection
    /// type B is { a: A };
    /// ```
    ///
    /// Allowed with indirection:
    /// ```verum
    /// type A is { b: Box<B> }; // OK: Box provides indirection
    /// type B is { a: Box<A> };
    /// ```
    pub fn resolve_type_definition(
        &mut self,
        type_decl: &verum_ast::TypeDecl,
        resolution_stack: &mut List<verum_common::Text>,
    ) -> Result<()> {
        use verum_ast::decl::TypeDeclBody;
        use verum_common::Text;
        // Scoped so an early return cannot leave the flag set; a
        // refinement's name check applies here and nowhere else.
        let prev_in_type_decl = self.in_type_declaration;
        self.in_type_declaration = true;
        let key = self.declared_type_key(type_decl.name.name.as_str());
        let out = self.with_source_type_registration(type_decl, key, |checker| {
            checker.with_declared_universe_scope("type", &type_decl.name.name, &type_decl.generics, |checker| {
                checker.resolve_type_definition_inner(type_decl, resolution_stack)
            })
        });
        self.in_type_declaration = prev_in_type_decl;
        out
    }

    fn resolve_type_definition_inner(
        &mut self,
        type_decl: &verum_ast::TypeDecl,
        resolution_stack: &mut List<verum_common::Text>,
    ) -> Result<()> {
        use verum_ast::decl::TypeDeclBody;
        use verum_common::Text;

        let type_name: Text = type_decl.name.name.as_str().into();
        let span = type_decl.span;

        self.affine_tracker.register_resource_discipline(
            self.declared_type_key(type_name.as_str()), type_decl.resource_discipline(),
        );

        // Check for cycles: are we already resolving this type?
        if resolution_stack.iter().any(|t| t == &type_name) {
            // Found a cycle! Build the cycle path for error reporting
            let mut cycle_types: List<verum_common::Text> = List::new();
            let mut in_cycle = false;
            for t in resolution_stack.iter() {
                if t == &type_name {
                    in_cycle = true;
                }
                if in_cycle {
                    cycle_types.push(t.clone());
                }
            }
            cycle_types.push(type_name.clone());

            let cycle_path: Text = cycle_types
                .iter()
                .map(|t| t.as_str())
                .collect::<Vec<_>>()
                .join(" -> ")
                .into();

            return Err(crate::TypeError::TypeCycle {
                cycle_path,
                types_in_cycle: cycle_types,
                span,
            });
        }

        // Push this type onto the resolution stack
        resolution_stack.push(type_name.clone());

        // This declaration's parameter scope frame (T0683).
        let mut param_frame: indexmap::IndexMap<Text, Type> = indexmap::IndexMap::new();

        // Determine if this is a variant type for choosing parameter representation
        let is_variant_type = matches!(&type_decl.body, verum_ast::decl::TypeDeclBody::Variant(_));

        // Register generic type parameters first (same as register_type_declaration)
        // CRITICAL: For variant types, we MUST use Type::Var to enable proper
        // polymorphic type inference. Using Type::Named would cause unification
        // failures because Named types don't participate in type variable substitution.
        for param in &type_decl.generics {
            use verum_ast::ty::GenericParamKind;

            let param_name: Text = match &param.kind {
                GenericParamKind::Type { name, .. } => name.name.as_str().into(),
                GenericParamKind::HigherKinded { name, .. } => name.name.as_str().into(),
                GenericParamKind::KindAnnotated { name, .. } => name.name.as_str().into(),
                GenericParamKind::Const { name, .. } => name.name.as_str().into(),
                GenericParamKind::Meta { name, .. } => name.name.as_str().into(),
                GenericParamKind::Lifetime { .. } => continue,
                GenericParamKind::Context { name } => name.name.as_str().into(),
                GenericParamKind::Level { name } => name.name.as_str().into(),
            };

            if is_variant_type {
                // For variant types: use Type::Var for proper polymorphic type inference
                // This ensures that variant constructors like Some(v) can unify T with Int
                let type_var = TypeVar::fresh();
                let param_type = Type::Var(type_var);
                param_frame.insert(param_name.clone(), param_type);
            } else {
                // For record types: use Named type for bidirectional type inference
                let param_type = Type::Named {
                    path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                        param_name.as_str(),
                        Span::default(),
                    )),
                    args: List::new(),
                };
                param_frame.insert(param_name.clone(), param_type);
            }
        }

        // Same frame discipline as `register_type_declaration_body`
        // (T0683): parameters scope to THIS declaration's resolution
        // only.
        self.push_decl_param_frame(param_frame);

        // Now resolve the actual type definition (similar to register_type_declaration)
        let (previous_premises, premises_result) = self.begin_nominal_declaration_premises(type_decl);
        let result = premises_result
            .and_then(|_| self.prepare_type_declaration_parameters(type_decl))
            .and_then(|_| self.resolve_type_body(type_decl, &type_name));
        self.finish_nominal_declaration_premises(previous_premises);

        self.pop_decl_param_frame();

        // Pop from resolution stack
        resolution_stack.pop();

        result
    }

    /// Helper: Resolve the body of a type declaration.
    ///
    /// Called by `resolve_type_definition` after cycle detection and
    /// type parameter setup.
    fn resolve_type_body(
        &mut self,
        type_decl: &verum_ast::TypeDecl,
        type_name: &Text,
    ) -> Result<()> {
        use crate::context::TypeScheme;
        use indexmap::IndexMap;
        use verum_ast::decl::{TypeDeclBody, VariantData};
        use verum_ast::ty::Path;
        use verum_common::Text;

        // ALIAS-VS-MARKER (see `variant_decl_alias_target`): keep the
        // second (resolution) pass in lock-step with the first so a bare
        // `type X is <ExistingType>;` resolves as the alias it is, not as a
        // degenerate `X(Unit)` sum.
        let alias_override_body: Option<TypeDeclBody> = self
            .variant_decl_alias_target(type_decl)
            .map(TypeDeclBody::Alias);
        let effective_body: &TypeDeclBody =
            alias_override_body.as_ref().unwrap_or(&type_decl.body);

        match effective_body {
            TypeDeclBody::Alias(aliased_type) => {
                let resolved_type = self.declared_alias_target(aliased_type)?;
                // Register as alias for proper resolution (stores resolved type)
                self.declare_type_alias_target(type_name.clone(), resolved_type.clone());
                // Also register in the unifier for transparent alias unification
                self.unifier
                    .register_type_alias(type_name.clone(), resolved_type.clone());
                // Register type parameter names for generic alias expansion in unifier
                self.register_source_alias_parameters(&type_name, &type_decl.generics);
                // For type_defs, store a Named type reference (not the resolved type)
                // This preserves the indirection needed for generic alias substitution
                let named_type = Type::Named {
                    path: Path::single(type_decl.name.clone()),
                    args: List::new(),
                };
                // Module-aware: register both unqualified (back-compat) AND
                // fully-qualified so signatures inside this module always
                // resolve their own types first, regardless of load order.
                self.define_type_in_current_module(type_name.clone(), named_type);

                // Register the arity this alias declares for its own name.
                self.register_declared_type_arity(&type_name, &type_decl.generics)?;
            }
            TypeDeclBody::Variant(_) => self.resolve_variant_type_body(type_decl, type_name)?,
            TypeDeclBody::Record(_) => self.resolve_record_type_body(type_decl, type_name)?,
            // Same invariant as `register_type_declaration_body`: the
            // declared name is registered through
            // `define_type_in_current_module`, so the ambient
            // variant-constructor shadow is evicted on this pass too.
            TypeDeclBody::Tuple(types) => {
                // Tuple type declaration: type Point is (Float, Float);
                // Single-element tuples are newtypes: type UserId is (Int);
                let type_list: Result<List<Type>> =
                    types.iter().map(|t| self.ast_to_type(t)).collect();
                let resolved_types = type_list?;

                if resolved_types.len() == 1 {
                    // Single-element "tuple" is a newtype with constructor
                    let inner_type = resolved_types.first().cloned().unwrap_or(Type::Unit);

                    // Create a Named type for the newtype (not an alias)
                    let newtype_ty = Type::Named {
                        path: Path::single(type_decl.name.clone()),
                        args: List::new(),
                    };
                    // T0591: route through the module-aware path so the
                    // same-name ambient variant-ctor shadow is evicted (a
                    // stdlib variant like `CacheTtl.Seconds` otherwise
                    // captured `Seconds(1.5)` and forced its Int payload).
                    self.define_type_in_current_module(type_name.clone(), newtype_ty.clone());

                    // Store inner type for field access (.0)
                    self.define_declared_type_metadata("__newtype_inner_", type_name.as_str(), inner_type.clone());

                    // Register constructor function: UserId: fn(Int) -> UserId
                    self.register_declared_nominal_constructor(&type_name, Some(vec![inner_type].into_iter().collect()), type_decl.span)?;
                } else {
                    // Multi-element tuple: create Named type with constructor
                    let named_tuple_ty = Type::Named {
                        path: Path::single(type_decl.name.clone()),
                        args: List::new(),
                    };
                    self.define_type_in_current_module(type_name.clone(), named_tuple_ty.clone());

                    // Store tuple fields for field access (.0, .1, .2)
                    self.define_declared_type_metadata("__tuple_fields_", type_name.as_str(), Type::Tuple(resolved_types.clone()));

                    // Register constructor function: Color: fn(Int, Int, Int) -> Color
                    self.register_declared_nominal_constructor(&type_name, Some(resolved_types), type_decl.span)?;
                }
            }
            TypeDeclBody::Newtype(inner_type) => {
                // Newtype: type UserId is Int; (without parens)
                // Register as a distinct named type (not just an alias)
                let inner_resolved = self.ast_to_type(inner_type)?;

                // Create the newtype as a Named type
                let newtype_ty = Type::Named {
                    path: Path::single(type_decl.name.clone()),
                    args: List::new(),
                };
                self.define_type_in_current_module(type_name.clone(), newtype_ty.clone());

                // Store the inner type for field access (.0)
                self.define_declared_type_metadata("__newtype_inner_", type_name.as_str(), inner_resolved.clone());

                // Register a constructor function: UserId: fn(Int) -> UserId
                self.register_declared_nominal_constructor(&type_name, Some(vec![inner_resolved].into_iter().collect()), type_decl.span)?;
            }
            TypeDeclBody::Protocol(_) => self.resolve_protocol_type_body(type_decl, type_name)?,
            TypeDeclBody::Unit => {
                // Unit type (e.g., `type Empty;` or `type Signal is ();`)
                let ty = Type::Named {
                    path: Path::single(type_decl.name.clone()),
                    args: List::new(),
                };
                self.define_type_in_current_module(type_name.clone(), ty);

                // Store inner type as Unit for newtype coercion checks
                self.define_declared_type_metadata("__newtype_inner_", type_name.as_str(), Type::Unit);
                self.register_declared_nominal_constructor(&type_name, None, type_decl.span)?;
            }
            TypeDeclBody::SigmaTuple(types) => {
                // Dependent pair / sigma type - similar to Tuple but with named components
                let element_types: List<Type> = types
                    .iter()
                    .map(|t| self.ast_to_type(t))
                    .collect::<Result<_>>()?;

                let ty = Type::Named {
                    path: Path::single(type_decl.name.clone()),
                    args: List::new(),
                };
                self.define_type_in_current_module(type_name.clone(), ty);

                let inner_key = format!("__tuple_elements_{}", type_name);
                self.ctx.define_type(inner_key, Type::Tuple(element_types));

                // Register named fields as struct fields for field access.
                // The sigma-binding elements parse as
                // `TypeKind::Refined` with `predicate.binding = Some(field)`.
                let mut fields = indexmap::IndexMap::new();
                for sigma_ty in types {
                    if let verum_ast::ty::TypeKind::Refined {
                        ref base,
                        ref predicate,
                    } = sigma_ty.kind
                    {
                        if let verum_common::Maybe::Some(ref name) = predicate.binding {
                            let field_type = self.ast_to_type(base)?;
                            fields.insert(name.name.clone(), field_type);
                        }
                    }
                }
                if !fields.is_empty() {
                    self.define_declared_type_metadata(
                        "__struct_fields_",
                        type_name.as_str(),
                        Type::Record(fields),
                    );
                }
            }
            TypeDeclBody::Inductive(_) => {
                // Dependent type features (v2.0+) - register as named type for now
                let ty = Type::Named {
                    path: Path::single(type_decl.name.clone()),
                    args: List::new(),
                };
                self.define_type_in_current_module(type_name.clone(), ty);
            }
            TypeDeclBody::Coinductive(protocol_body) => {
                // Coinductive type (Pass 2) — resolve destructor return types and
                // verify each declared destructor has a valid return type.
                let ty = Type::Named {
                    path: Path::single(type_decl.name.clone()),
                    args: List::new(),
                };
                self.define_type_in_current_module(type_name.clone(), ty);

                // Verify that all destructors have explicit, resolvable return types.
                for item in &protocol_body.items {
                    use verum_ast::decl::ProtocolItemKind;
                    if let ProtocolItemKind::Function { decl, .. } = &item.kind {
                        if let Some(ret_ty_ast) = &decl.return_type {
                            // Attempt to resolve the return type — surface errors
                            // that would prevent observation methods from type-checking.
                            let _ = self.ast_to_type(ret_ty_ast)?;
                        }
                        // Missing return type was warned at Pass 1; nothing more to do.
                    }
                }
            }
            TypeDeclBody::Quotient { base, .. } => {
                // T1-T Pass 2: resolve the carrier type and register
                // the quotient as a nominal type sharing the carrier's
                // runtime representation. The equivalence relation is
                // a compile-time obligation discharged by the model-
                // verification pipeline; at Tier-0 a value of Q is
                // bit-identical to a value of the carrier.
                //

                // Two projection methods are registered here:
                //  Q.of(rep: T) -> Q (static constructor)
                //  q.rep(&self) -> T (instance accessor)
                //

                // Both are identity at runtime; typecheck uses them to
                // guard the boundary between Q and its carrier so
                // user code has to name the quotient explicitly when
                // crossing it in either direction.
                let base_resolved = self.ast_to_type(base)?;
                self.declare_type_alias_target(type_name.clone(), base_resolved.clone());
                self.unifier
                    .register_type_alias(type_name.clone(), base_resolved.clone());
                // Replace the Pass-1 placeholder with a concrete Named
                // type so verify_no_placeholders sees a resolved entry.
                let named_type = Type::Named {
                    path: Path::single(type_decl.name.clone()),
                    args: List::new(),
                };
                self.define_type_in_current_module(type_name.clone(), named_type.clone());

                // Register the `of` static constructor and `rep`
                // instance accessor in the protocol checker's method
                // registry so `Q.of(x)` and `q.rep()` resolve against
                // the quotient's nominal name rather than falling
                // through to the carrier's method set.
                use crate::core_integration::ProtocolCheckerExt;
                use crate::protocol::MethodSignature;
                {
                    let mut checker = self.protocol_checker.write();
                    let mut of_params = List::new();
                    of_params.push(base_resolved.clone());
                    checker.register_method_public(
                        type_name.as_str(),
                        MethodSignature::static_method(
                            Text::from("of"),
                            of_params,
                            named_type.clone(),
                        ),
                    );
                    checker.register_method_public(
                        type_name.as_str(),
                        MethodSignature::immutable(Text::from("rep"), List::new(), base_resolved),
                    );
                }
            }
        }

        tracing::debug!("Pass 2: Resolved type definition for: {}", type_name);
        Ok(())
    }

    /// Batch register all type names (Pass 1).
    ///
    /// Convenience method that calls `register_type_name_only` for all type
    /// declarations in a list. Should be called before `resolve_all_type_definitions`.
    /// Resolve a sum (variant/enum) type declaration body during two-pass resolution.
    /// Re-resolves constructor payloads after all dependencies are available.
    fn resolve_variant_type_body(
        &mut self,
        type_decl: &verum_ast::TypeDecl,
        type_name: &verum_common::Text,
    ) -> Result<()> {
        use crate::context::TypeScheme;
        use indexmap::IndexMap;
        use verum_ast::decl::{TypeDeclBody, VariantData};
        use verum_ast::ty::Path;
        use verum_common::Text;
        let verum_ast::decl::TypeDeclBody::Variant(variants) = &type_decl.body
            else { unreachable!() };
        let type_name = type_name.clone();
                // IMPORT PROVENANCE: Protect explicitly imported variant types from being
                // overwritten during stdlib type resolution (PRE-PASS S0b). When the user
                // writes `mount core.base.ordering.{Ordering}`, we must not let a later
                // stdlib module (e.g., core.sync.atomic.Ordering) overwrite it via
                // resolve_type_body. Only block during stdlib loading (user_code_phase=false)
                // and when NOT processing the explicit import itself.
                if !self.user_code_phase
                    && !self.in_explicit_import_registration
                    && self.explicit_imports.contains(type_name.as_str())
                {
                    if let verum_common::Maybe::Some(Type::Variant(existing_variants)) =
                        self.ctx.lookup_type(type_name.as_str())
                    {
                        let existing_names: std::collections::BTreeSet<&str> =
                            existing_variants.keys().map(|k| k.as_str()).collect();
                        let new_names: std::collections::BTreeSet<&str> =
                            variants.iter().map(|v| v.name.name.as_str()).collect();
                        if existing_names != new_names {
                            if type_name.as_str() == "Shape" || type_name.as_str() == "Expr" {
                                eprintln!(
                                    "[CONFLICT] Type '{}' already registered with different variants. existing={:?} new={:?}",
                                    type_name, existing_names, new_names
                                );
                            }
                            return Ok(());
                        }
                    }
                }

                // CRITICAL: Register a placeholder type FIRST to handle recursive type definitions
                // like: type List<T> is Nil | Cons(T, List<T>)
                // This enables recursive references to resolve before the type body is fully processed.
                // Use Type::Named (not Type::Var) so that List<T> references construct proper Generic types.
                let placeholder_type = Type::Named {
                    path: Path::single(verum_ast::ty::Ident::new(
                        type_name.as_str(),
                        Span::default(),
                    )),
                    args: List::new(),
                };
                // Module-aware: register under both unqualified and qualified
                // keys so recursive references inside this type's module
                // resolve to THIS placeholder (and later to the real variant),
                // not to a same-named type from a different module.
                self.define_type_in_current_module(type_name.clone(), placeholder_type);

                // Convert variant declarations to Type::Variant
                let mut variant_map: IndexMap<Text, Type> = IndexMap::new();

                for variant in variants {
                    let variant_name: Text = variant.name.name.as_str().into();

                    let payload_type = match &variant.data {
                        None => Type::Unit,
                        Some(VariantData::Tuple(types)) => {
                            if types.len() == 1 {
                                self.ast_to_type(&types[0])?
                            } else {
                                let converted: Result<List<Type>> =
                                    types.iter().map(|t| self.ast_to_type(t)).collect();
                                Type::Tuple(converted?)
                            }
                        }
                        Some(VariantData::Record(fields)) => {
                            let mut record_map: IndexMap<Text, Type> = IndexMap::new();
                            for field in fields {
                                let field_name: Text = field.name.name.as_str().into();
                                // Final resolution has the same obligation for
                                // named payload fields as tuple payloads.
                                let field_type = self.ast_to_type(&field.ty)?;
                                record_map.insert(field_name, field_type);
                            }
                            Type::Record(record_map)
                        }
                    };

                    // Note: __struct_fields_<VariantName> is registered AFTER the full
                    // variant_map is built (below) to avoid recursion in resolve_type_definition.

                    variant_map.insert(variant_name, payload_type);
                }

                // Check if any variant payload contains an affine type ("affine contagion")
                // If so, the containing type is also affine
                // Memory model: three-tier references (&T managed, &checked T verified, &unsafe T raw) with CBGR runtime checking — #affine-types
                for (_variant_name, payload_type) in &variant_map {
                    if self.type_contains_affine(payload_type) {
                        self.affine_tracker
                            .register_affine_type(self.declared_type_key(type_name.as_str()));
                        break;
                    }
                }

                // C2-WIRE V2 + #154 unified call site —
                // strict-positivity on the resolved variant body for
                // the `verum build` two-pass type-resolution loop
                // (Pass 1b — see phase_type_check). Routes through
                // the same canonical helper as the
                // register_type_declaration_inner site so both
                // paths agree on what counts as positive.
                if let Err(violation) = crate::positivity::check_variant_body_positivity(
                    type_name.as_str(),
                    &variant_map,
                ) {
                    return Err(crate::TypeError::PositivityViolation {
                        type_name: verum_common::Text::from(violation.type_name.as_str()),
                        constructor: verum_common::Text::from(violation.constructor.as_str()),
                        position: verum_common::Text::from(violation.position.as_str()),
                        span: type_decl.span,
                    });
                }

                let variant_type = Type::Variant(variant_map.clone());

                // Register record variant constructors in env BEFORE define_type.
                // define_type may trigger deferred expression checking that needs
                // the variant constructors to be available in the env.
                // This enables `Rect { w: 4, h: 6 }` to resolve via env lookup.
                // Uses env.insert (not define_type) to avoid triggering re-resolution.
                // Store variant record fields in a side map (NOT define_type which triggers recursion).
                // This map is read by check_expr Record handler for variant constructors.
                for (vname, vty) in &variant_map {
                    if let Type::Record(fields) = vty {
                        self.journal_variant_record(vname);
                        self.variant_record_fields
                            .insert(vname.clone(), fields.clone());
                    }
                }

                for (vname, vty) in &variant_map {
                    if let Type::Record(fields) = vty {
                        let ctor_type = Type::Function {
                            property_requirement: crate::computational_properties::PropertyRequirement::Unrestricted,
                            params: verum_common::List::from_iter([Type::Record(fields.clone())]),
                            return_type: Box::new(variant_type.clone()),
                            properties: None,
                            contexts: None,
                            type_params: verum_common::List::new(),
                        };
                        self.ctx
                            .env
                            .insert(vname.clone(), crate::context::TypeScheme::mono(ctor_type));
                    }
                }

                // NOW store the variant type definition (after constructors are in env)
                // Module-aware: publish under {module}.{name} too, so signatures
                // inside this module always find THIS variant (not a same-named
                // one last-registered by another module).
                self.define_type_in_current_module(type_name.clone(), variant_type.clone());

                // CRITICAL: Register variant type to name mapping for instance method lookup
                if let Some(sig) = Self::variant_type_signature(&variant_type) {
                    self.register_variant_type_name_first_wins(sig.clone(), type_name.clone());
                    if let Some(relaxed_sig) = Self::variant_type_signature_relaxed(&variant_type) {
                        if relaxed_sig != sig {
                            self.register_variant_type_name_first_wins(
                                relaxed_sig,
                                type_name.clone(),
                            );
                        }
                    }

                    let parameters = self.declared_nominal_arguments(&type_name, type_decl.span)?;
                    for key in [type_name.clone(), self.declared_type_key(&type_name)] {
                        self.unifier.register_original_variant_type(key.clone(), variant_type.clone());
                        self.unifier.register_variant_parameters(key, parameters.clone());
                    }
                }

                // Register variant constructor parent mappings for scope-aware resolution.
                for variant in variants.iter() {
                    let vn: Text = variant.name.name.as_str().into();
                    self.journal_variant_parent(&vn);
                    let parents = self.variant_constructor_parents.entry(vn).or_default();
                    if !parents.iter().any(|p| p.as_str() == type_name.as_str()) {
                        parents.push(type_name.clone());
                    }
                }

                // Register variant constructors (same as register_type_declaration)
                for variant in variants {
                    let variant_name: Text = variant.name.name.as_str().into();
                    let payload_type = variant_map.get(&variant_name).unwrap_or(&Type::Unit);

                    // Variant short names use "last declaration wins" semantics.
                    // See comment in register_type_declaration for rationale.
                    // Protect variant short names from overriding important bindings:
                    // 1. Primitive type names (Int, Float, Bool, etc.) — always protected
                    // 2. Polymorphic/function constructors from being downgraded by
                    //  monomorphic unit variants (prevents Keyword.Some overriding Maybe.Some)
                    // 3. Core constructors (Some, None, Ok, Err) — first-registered-wins
                    // Qualified names (Type.Variant) are always registered regardless.
                    let is_primitive_type_name = {
                        let vn = variant_name.as_str();
                        verum_common::well_known_types::type_names::is_primitive_value_type(vn)
                            || matches!(vn, "Bytes" | "UInt")
                    };
                    let short_name_exists = if is_primitive_type_name {
                        true // Never override primitive type names
                    } else if let Some(existing) = self.ctx.env.lookup(&variant_name) {
                        if self.user_code_phase {
                            // In user code phase: user-defined variants always shadow
                            // stdlib variants. Users must be able to define any variant
                            // name (e.g., Status.Success shadows StealResult.Success).
                            false
                        } else {
                            // During stdlib loading: protect rich/generic bindings
                            // (e.g., Heap<T>, Maybe<T>.None) from being overridden by
                            // variant constructors from other types.
                            !existing.vars.is_empty()
                                || matches!(existing.ty, Type::Function { .. })
                                || matches!(existing.ty, Type::Generic { .. })
                        }
                    } else {
                        false
                    };

                    if *payload_type == Type::Unit {
                        self.register_sum_constructor(type_decl, &variant_name, variant_type.clone(), !short_name_exists)?;
                    } else {
                        let params = match payload_type {
                            Type::Tuple(tuple_types) => tuple_types.clone(),
                            _ => {
                                let mut p = List::new();
                                p.push(payload_type.clone());
                                p
                            }
                        };
                        let constructor_ty = Type::function(params, variant_type.clone());

                        self.register_sum_constructor(type_decl, &variant_name, constructor_ty, !short_name_exists)?;

                        // Also register non-unit record variants as TYPES (same as first pass).
                        if let Type::Record(_) = payload_type {
                            if !short_name_exists {
                                self.ctx
                                    .define_type(variant_name.clone(), payload_type.clone());
                            }
                            let qualified_type_name: Text =
                                format!("{}.{}", type_name, variant_name).into();
                            self.ctx
                                .define_type(qualified_type_name, payload_type.clone());
                        }
                    }
                }
        Ok(())
    }

    /// Resolve a record (struct-like) type declaration body during two-pass resolution.
    /// Resolves field types and registers the Record type with protocol constraints.
    fn resolve_record_type_body(
        &mut self,
        type_decl: &verum_ast::TypeDecl,
        type_name: &verum_common::Text,
    ) -> Result<()> {
        use crate::context::TypeScheme;
        use indexmap::IndexMap;
        use verum_ast::decl::{TypeDeclBody, VariantData};
        use verum_ast::ty::Path;
        use verum_common::Text;
        let verum_ast::decl::TypeDeclBody::Record(fields) = &type_decl.body
            else { unreachable!() };
        self.register_record_field_visibility(type_name.as_str(), fields);
        // Field binders may shadow declaration parameters in the field telescope.
        self.register_declared_type_arity(type_name, &type_decl.generics)?;
        let type_name = type_name.clone();
                // CRITICAL: Register a placeholder type FIRST to handle recursive type definitions
                // and prevent infinite recursion when processing field types.
                let placeholder_var = TypeVar::fresh();
                // Module-aware: publish the placeholder under the qualified key
                // so same-named record types in different modules don't clobber
                // each other during recursive resolution.
                self.define_type_in_current_module(type_name.clone(), Type::Var(placeholder_var));

                let owner = self.declared_type_key(type_name.as_str());
                let record_map = self.elaborate_record_fields(&owner, fields, false)?;

                // C2-WIRE V3 + #154 unified call site —
                // strict-positivity for record-form types in the
                // `verum build` path. Routes through the same
                // canonical helper as the
                // register_type_declaration_inner record-arm site.
                if let Err(violation) = crate::positivity::check_record_body_positivity(
                    type_name.as_str(),
                    &record_map,
                    Some(placeholder_var),
                ) {
                    return Err(crate::TypeError::PositivityViolation {
                        type_name: verum_common::Text::from(violation.type_name.as_str()),
                        constructor: verum_common::Text::from(violation.constructor.as_str()),
                        position: verum_common::Text::from(violation.position.as_str()),
                        span: type_decl.span,
                    });
                }

                // Empty records (e.g., `type Empty is { };`) need field structure for pattern matching.
                if fields.is_empty() {
                    // Register as Named type
                    let named_type = self.declared_record_type(type_decl)?;
                    // Module-aware: empty record registration.
                    self.define_type_in_current_module(type_name.clone(), named_type);

                    // CRITICAL: Also register empty struct fields for pattern matching
                    // Empty record allows `match e { Empty {} => ... }` patterns
                    self.publish_record_fields(&type_name, record_map, type_decl.span)?;
                } else {
                    // Non-empty record: Register as Named type with field structure

                    // Register as Named type for nominal identity
                    let named_type = self.declared_record_type(type_decl)?;

                    // Module-aware: non-empty record registration.
                    self.define_type_in_current_module(type_name.clone(), named_type.clone());

                    // CRITICAL FIX: Resolve placeholder TypeApp references in record field types.
                    // Same fix as in register_type_declaration_body — see comment there.
                    let record_map = {
                        fn resolve_placeholder_v2(
                            ty: &Type,
                            placeholder: TypeVar,
                            type_path: &verum_ast::ty::Path,
                        ) -> Type {
                            match ty {
                                // Handle bare self-reference: Var(placeholder) -> Named { path, args: [] }
                                // This handles non-generic recursive types like:
                                //  type Node is { children: List<Heap<Node>> }
                                // where Node inside Heap<Node> is stored as Var(placeholder)
                                Type::Var(tv) if *tv == placeholder => Type::Named {
                                    path: type_path.clone(),
                                    args: List::new(),
                                },
                                Type::TypeApp { constructor, args } => {
                                    if let Type::Var(tv) = constructor.as_ref() {
                                        if *tv == placeholder {
                                            let resolved_args: List<Type> = args
                                                .iter()
                                                .map(|a| {
                                                    resolve_placeholder_v2(
                                                        a,
                                                        placeholder,
                                                        type_path,
                                                    )
                                                })
                                                .collect();
                                            return Type::Named {
                                                path: type_path.clone(),
                                                args: resolved_args,
                                            };
                                        }
                                    }
                                    let resolved_ctor =
                                        resolve_placeholder_v2(constructor, placeholder, type_path);
                                    let resolved_args: List<Type> = args
                                        .iter()
                                        .map(|a| resolve_placeholder_v2(a, placeholder, type_path))
                                        .collect();
                                    Type::TypeApp {
                                        constructor: Box::new(resolved_ctor),
                                        args: resolved_args,
                                    }
                                }
                                Type::Generic { name, args } => {
                                    let resolved_args: List<Type> = args
                                        .iter()
                                        .map(|a| resolve_placeholder_v2(a, placeholder, type_path))
                                        .collect();
                                    Type::Generic {
                                        name: name.clone(),
                                        args: resolved_args,
                                    }
                                }
                                Type::Named { path, args } => {
                                    let resolved_args: List<Type> = args
                                        .iter()
                                        .map(|a| resolve_placeholder_v2(a, placeholder, type_path))
                                        .collect();
                                    Type::Named {
                                        path: path.clone(),
                                        args: resolved_args,
                                    }
                                }
                                Type::Record(fields) => {
                                    let resolved: indexmap::IndexMap<Text, Type> = fields
                                        .iter()
                                        .map(|(k, v)| {
                                            (
                                                k.clone(),
                                                resolve_placeholder_v2(v, placeholder, type_path),
                                            )
                                        })
                                        .collect();
                                    Type::Record(resolved)
                                }
                                Type::Variant(variants) => {
                                    let resolved: indexmap::IndexMap<Text, Type> = variants
                                        .iter()
                                        .map(|(k, v)| {
                                            (
                                                k.clone(),
                                                resolve_placeholder_v2(v, placeholder, type_path),
                                            )
                                        })
                                        .collect();
                                    Type::Variant(resolved)
                                }
                                Type::Tuple(elems) => Type::Tuple(
                                    elems
                                        .iter()
                                        .map(|e| resolve_placeholder_v2(e, placeholder, type_path))
                                        .collect(),
                                ),
                                Type::Reference { inner, mutable } => Type::Reference {
                                    inner: Box::new(resolve_placeholder_v2(
                                        inner,
                                        placeholder,
                                        type_path,
                                    )),
                                    mutable: *mutable,
                                },
                                Type::Function {
                                    property_requirement,
                                    params,
                                    return_type,
                                    contexts,
                                    type_params,
                                    properties,
                                } => Type::Function {
                                    property_requirement: property_requirement.clone(),
                                    params: params
                                        .iter()
                                        .map(|p| resolve_placeholder_v2(p, placeholder, type_path))
                                        .collect(),
                                    return_type: Box::new(resolve_placeholder_v2(
                                        return_type,
                                        placeholder,
                                        type_path,
                                    )),
                                    contexts: contexts.clone(),
                                    type_params: type_params.clone(),
                                    properties: properties.clone(),
                                },
                                _ => ty.clone(),
                            }
                        }
                        let type_path = verum_ast::ty::Path::single(type_decl.name.clone());
                        let mut resolved_map = indexmap::IndexMap::new();
                        for (fname, fty) in record_map.iter() {
                            resolved_map.insert(
                                fname.clone(),
                                resolve_placeholder_v2(fty, placeholder_var, &type_path),
                            );
                        }
                        resolved_map
                    };

                    // Store record structure for field access
                    self.publish_record_fields(&type_name, record_map, type_decl.span)?;

                }
        Ok(())
    }

    /// Resolve a protocol (trait-like) type declaration body during two-pass resolution.
    /// Processes method signatures + associated types and updates the protocol registry.
    fn resolve_protocol_type_body(
        &mut self,
        type_decl: &verum_ast::TypeDecl,
        type_name: &verum_common::Text,
    ) -> Result<()> {
        use crate::context::TypeScheme;
        use indexmap::IndexMap;
        use verum_ast::decl::{TypeDeclBody, VariantData};
        use verum_ast::ty::Path;
        use verum_common::Text;
        let verum_ast::decl::TypeDeclBody::Protocol(protocol_body) = &type_decl.body
            else { unreachable!() };
        self.validate_protocol_type_declaration(type_decl, protocol_body)?;
        self.register_declared_type_arity(type_name, &type_decl.generics)?;
        let type_name = type_name.clone();
                // Protocol types are registered as Named types
                let ty = Type::Named {
                    path: Path::single(type_decl.name.clone()),
                    args: List::new(),
                };
                self.define_type_in_current_module(type_name.clone(), ty);

                // CRITICAL FIX: Register protocol with its methods in protocol_checker
                // This enables method lookup for bounded generic types like `fn foo<T: Display>(item: T)`
                // Without this, calling protocol methods on bounded type parameters fails.
                use verum_ast::decl::ProtocolItemKind;

                // Self type is represented as Named type with path "Self"
                let self_type = Type::Named {
                    path: Path::single(Ident::new("Self", Span::default())),
                    args: List::new(),
                };

                // CRITICAL: Register "Self" in type environment so that Self.Item can be resolved
                // when parsing method signatures. This allows ast_to_type to recognize "Self"
                // as a valid type name during protocol registration.
                self.ctx
                    .define_type(verum_common::Text::from("Self"), self_type.clone());

                // CRITICAL: Also set current_self_type so that PathSegment::SelfValue
                // (which the parser produces for 'Self') can be resolved.
                // Save the old value to restore later.
                let old_self_type = self.current_self_type.clone();
                self.set_current_self_type(Maybe::Some(self_type.clone()));
                let registration = (|| -> Result<()> {

                // CRITICAL FIX: Register protocol's type parameters (like T in Into<T>) BEFORE
                // processing method signatures. This allows the return type `T` to be resolved
                // as Type::Var instead of Type::Named, enabling proper type inference with
                // blanket implementations.
                //

                // Track which type params we register so we can clean them up after
                let mut protocol_type_param_names_2: List<Text> = List::new();
                for generic_param in &type_decl.generics {
                    use verum_ast::ty::GenericParamKind;
                    if let GenericParamKind::Type { name, .. } = &generic_param.kind {
                        let type_var = TypeVar::fresh();
                        let param_name: Text = name.name.clone();
                        self.ctx
                            .define_type(param_name.clone(), Type::Var(type_var));
                        protocol_type_param_names_2.push(param_name);
                    }
                }

                let (constraint_scope, declared_supers) = self.protocol_constraint_scope(
                    &type_decl.generics, &protocol_body.extends,
                    &[type_decl.generic_where_clause.as_ref(), protocol_body.generic_where_clause.as_ref()],
                );

                let super_protocols = match declared_supers {
                    Some(supers) => supers,
                    None => self.convert_super_protocols(&protocol_body.extends)?,
                };
                let (protocol_methods, protocol_assoc_types, dyn_required) = self.elaborate_protocol_members(
                    self.declared_type_key(type_name.as_str()), &type_decl.generics,
                    &constraint_scope, &super_protocols, &protocol_body.items, &protocol_body.declared_default_methods,
                )?;

                // Create and register Protocol object
                // Convert AST is_context bool to ProtocolKind
                let kind = if protocol_body.is_context {
                    crate::protocol::ProtocolKind::ConstraintAndInjectable
                } else {
                    crate::protocol::ProtocolKind::Constraint
                };

                let protocol = crate::protocol::Protocol {
                    source_parameters: Some(std::sync::Arc::new(type_decl.generics.clone())),
                    constraints: constraint_scope,
                    name: self.declared_type_key(type_name.as_str()),
                    kind,
                    type_params: self.convert_generic_params_to_type_params(&type_decl.generics)?,
                    methods: protocol_methods.clone(),
                    associated_types: protocol_assoc_types,
                    associated_consts: Map::new(),
                    super_protocols,
                    specialization_info: Maybe::None,
                    defining_crate: Maybe::None,
                    span: type_decl.span,
                };
                self.check_pending_protocol_object_safety(&protocol, dyn_required)?;
                self.check_protocol_family_applications(&protocol)?;
                self.unifier.register_protocol_name(protocol.name.as_str());
                // This run parsed the declaration, so its default-body flags are
                // trustworthy — see `protocols_with_known_defaults`.
                self.protocols_with_known_defaults.insert(
                    protocol.name.clone(),
                    (
                        protocol
                            .methods
                            .iter()
                            .filter(|(_, m)| !m.has_default)
                            .map(|(n, _)| n.clone())
                            .collect(),
                        protocol.methods.keys().cloned().collect(),
                    ),
                );
                let _ = self.protocol_checker.write().register_protocol(protocol);

                // Register as context or constraint protocol based on is_context flag
                if !protocol_body.is_context {
                    // Register as constraint protocol for better error messages
                    self.context_resolver
                        .register_constraint_protocol(type_name.clone());
                }

                Ok(())
                })();
                self.set_current_self_type(old_self_type);
        registration
    }

    pub fn register_all_type_names(&mut self, items: &[verum_ast::Item]) {
        for item in items {
            if let verum_ast::ItemKind::Type(type_decl) = &item.kind {
                self.register_type_name_only(type_decl);
            }
        }
    }

    /// Batch resolve all type definitions (Pass 2).
    ///
    /// Convenience method that calls `resolve_type_definition` for all type
    /// declarations in a list. Should be called after `register_all_type_names`.
    ///
    /// Returns errors for any type that cannot be resolved (cycles, undefined types, etc.)
    pub fn resolve_all_type_definitions(&mut self, items: &[verum_ast::Item]) -> List<Result<()>> {
        let mut results = List::new();
        let mut resolution_stack: List<verum_common::Text> = List::new();
        for item in items {
            if let verum_ast::ItemKind::Type(type_decl) = &item.kind {
                results.push(self.resolve_type_definition(type_decl, &mut resolution_stack));
            }
        }
        results
    }

    /// Verify that no placeholder types remain after two-pass resolution.
    ///
    /// Call this after `resolve_all_type_definitions` to ensure all forward
    /// references were successfully resolved.
    ///
    /// This performs a deep traversal of all type structures to find any remaining
    /// placeholders, including those nested in records, variants, functions, etc.
    pub fn verify_no_placeholders(&self) -> List<crate::TypeError> {
        let mut errors = List::new();
        let mut visited = std::collections::HashSet::new();

        // Check all registered types for remaining placeholders (deep traversal)
        for (name, ty) in self.ctx.all_types() {
            // Skip internal metadata keys
            if name.as_str().starts_with("__") {
                continue;
            }
            self.collect_placeholder_errors(ty, &mut errors, &mut visited);
        }

        errors
    }

    /// Recursively collect placeholder errors from a type and all its nested types.
    ///
    /// This performs a deep traversal to find placeholders nested in:
    /// - Record fields
    /// - Variant payloads
    /// - Function parameters and return types
    /// - Generic type arguments
    /// - Reference inner types
    /// - Array/slice element types
    /// - etc.
    fn collect_placeholder_errors(
        &self,
        ty: &Type,
        errors: &mut List<crate::TypeError>,
        visited: &mut std::collections::HashSet<Text>,
    ) {
        use Type::*;

        match ty {
            // Placeholder type - an error ONLY if genuinely undefined.
            Placeholder { name, span } => {
                // Avoid duplicate errors for the same placeholder
                if visited.contains(name) {
                    return;
                }
                // A forward reference resolves when the same name is defined
                // later in the same compilation: `type A is X { f: B }; type B
                // is ...`. Two-pass resolution stores the Pass-1 placeholder in
                // a sum-type variant body (records keep field structure under a
                // `__struct_fields_` key this walk skips, so they never tripped
                // this); use sites already resolve it lazily via
                // substitute_placeholders (infer/expr.rs). If `name` now names a
                // concrete (non-placeholder) type, it is resolved — not an
                // error; recurse into the real type to catch nested placeholders.
                // Only a name with no definition anywhere is a true unresolved
                // type. (T0453 / VARIANT-FIELD-FWDREF-1.)
                if let Some(resolved_ty) = self.ctx.lookup_type(name.as_str()) {
                    if !matches!(resolved_ty, Placeholder { .. }) {
                        visited.insert(name.clone());
                        self.collect_placeholder_errors(resolved_ty, errors, visited);
                        return;
                    }
                }
                visited.insert(name.clone());
                errors.push(crate::TypeError::UnresolvedPlaceholder {
                    name: name.clone(),
                    span: *span,
                });
            }

            // Compound types - recurse into nested types
            Named { args, .. } | Generic { args, .. } => {
                for arg in args {
                    self.collect_placeholder_errors(arg, errors, visited);
                }
            }

            Function {
                params,
                return_type,
                ..
            } => {
                for param in params {
                    self.collect_placeholder_errors(param, errors, visited);
                }
                self.collect_placeholder_errors(return_type, errors, visited);
            }

            Tuple(elements) => {
                for elem in elements {
                    self.collect_placeholder_errors(elem, errors, visited);
                }
            }

            Record(fields) => {
                for ty in fields.values() {
                    self.collect_placeholder_errors(ty, errors, visited);
                }
            }

            Variant(variants) => {
                for ty in variants.values() {
                    self.collect_placeholder_errors(ty, errors, visited);
                }
            }

            Reference { inner, .. }
            | CheckedReference { inner, .. }
            | UnsafeReference { inner, .. }
            | Ownership { inner, .. }
            | Pointer { inner, .. }
            | GenRef { inner } => {
                self.collect_placeholder_errors(inner, errors, visited);
            }

            Array { element, .. } | Slice { element } => {
                self.collect_placeholder_errors(element, errors, visited);
            }

            Refined { base, .. } | Quantified { inner: base, .. } => {
                self.collect_placeholder_errors(base, errors, visited);
            }

            Pi {
                param_type,
                return_type,
                ..
            } => {
                self.collect_placeholder_errors(param_type, errors, visited);
                self.collect_placeholder_errors(return_type, errors, visited);
            }

            Sigma {
                fst_type, snd_type, ..
            } => {
                self.collect_placeholder_errors(fst_type, errors, visited);
                self.collect_placeholder_errors(snd_type, errors, visited);
            }

            Eq { ty, .. } => {
                self.collect_placeholder_errors(ty, errors, visited);
            }

            // Primitive types and other types - no placeholders possible
            Unit
            | Bool
            | Int
            | Float
            | Char
            | Text
            | Never
            | Var(_)
            | Universe { .. }
            | Lifetime { .. }
            | Prop => {}

            // Handle remaining types (Exists, Forall, Meta, Future, Generator, etc.)
            // These types may contain nested types that need checking
            Exists { body, .. } | Forall { body, .. } => {
                self.collect_placeholder_errors(body, errors, visited);
            }

            Meta { .. } => {}

            Future { output } => {
                self.collect_placeholder_errors(output, errors, visited);
            }

            Generator {
                yield_ty,
                return_ty,
            } => {
                self.collect_placeholder_errors(yield_ty, errors, visited);
                self.collect_placeholder_errors(return_ty, errors, visited);
            }

            // Catch-all for any new variants - safe to skip for placeholder checking
            _ => {}
        }
    }

    /// Resolve all placeholders in registered types iteratively until a fixed point is reached.
    ///
    /// This is called after `resolve_all_type_definitions` to ensure that any remaining
    /// placeholders in nested structures are resolved. The algorithm:
    ///
    /// 1. Iterate over all registered types
    /// 2. For each type containing placeholders, substitute them with resolved types
    /// 3. Repeat until no changes are made (fixed point)
    ///
    /// Returns the number of iterations performed.
    ///
    /// # Fixed Point Algorithm
    ///
    /// The algorithm terminates when one of these conditions is met:
    /// - No placeholders were resolved in an iteration (fixed point)
    /// - Maximum iteration count is reached (prevents infinite loops)
    /// - All types are fully resolved
    pub fn resolve_forward_references(&mut self) -> usize {
        const MAX_ITERATIONS: usize = 100;
        let mut iteration = 0;

        loop {
            iteration += 1;
            if iteration > MAX_ITERATIONS {
                tracing::warn!(
                    "resolve_forward_references reached max iterations ({}), stopping",
                    MAX_ITERATIONS
                );
                break;
            }

            let mut changed = false;

            // Collect types that need resolution (to avoid borrowing issues)
            let types_to_resolve: List<(Text, Type)> = self
                .ctx
                .all_types()
                .filter(|(name, _)| !name.as_str().starts_with("__"))
                .map(|(name, ty)| (name.clone(), ty.clone()))
                .collect();

            for (name, ty) in types_to_resolve {
                if self.contains_placeholder(&ty) {
                    let resolved = self.substitute_placeholders(&ty);
                    if resolved != ty {
                        self.ctx.define_type(name.clone(), resolved);
                        changed = true;
                    }
                }
            }

            if !changed {
                tracing::debug!(
                    "resolve_forward_references reached fixed point after {} iterations",
                    iteration
                );
                break;
            }
        }

        iteration
    }

    /// Check if a type contains any placeholder types.
    ///
    /// This performs a deep traversal to detect placeholders anywhere in the type structure.
    pub fn contains_placeholder(&self, ty: &Type) -> bool {
        self.contains_placeholder_impl(ty, 0)
    }

    /// Internal implementation with depth tracking to prevent stack overflow.
    fn contains_placeholder_impl(&self, ty: &Type, depth: usize) -> bool {
        const MAX_DEPTH: usize = 100;
        if depth > MAX_DEPTH {
            return false;
        }

        use Type::*;

        match ty {
            Placeholder { .. } => true,

            Named { args, .. } | Generic { args, .. } => args
                .iter()
                .any(|arg| self.contains_placeholder_impl(arg, depth + 1)),

            Function {
                params,
                return_type,
                ..
            } => {
                params
                    .iter()
                    .any(|p| self.contains_placeholder_impl(p, depth + 1))
                    || self.contains_placeholder_impl(return_type, depth + 1)
            }

            Tuple(elements) => elements
                .iter()
                .any(|e| self.contains_placeholder_impl(e, depth + 1)),

            Record(fields) => fields
                .values()
                .any(|t| self.contains_placeholder_impl(t, depth + 1)),

            Variant(variants) => variants
                .values()
                .any(|t| self.contains_placeholder_impl(t, depth + 1)),

            Reference { inner, .. }
            | CheckedReference { inner, .. }
            | UnsafeReference { inner, .. }
            | Ownership { inner, .. }
            | Pointer { inner, .. }
            | GenRef { inner } => self.contains_placeholder_impl(inner, depth + 1),

            Array { element, .. } | Slice { element } => {
                self.contains_placeholder_impl(element, depth + 1)
            }

            Refined { base, .. } | Quantified { inner: base, .. } => {
                self.contains_placeholder_impl(base, depth + 1)
            }

            Pi {
                param_type,
                return_type,
                ..
            } => {
                self.contains_placeholder_impl(param_type, depth + 1)
                    || self.contains_placeholder_impl(return_type, depth + 1)
            }

            Sigma {
                fst_type, snd_type, ..
            } => {
                self.contains_placeholder_impl(fst_type, depth + 1)
                    || self.contains_placeholder_impl(snd_type, depth + 1)
            }

            Eq { ty: eq_ty, .. } => self.contains_placeholder_impl(eq_ty, depth + 1),

            Unit
            | Bool
            | Int
            | Float
            | Char
            | Text
            | Never
            | Var(_)
            | Universe { .. }
            | Lifetime { .. }
            | Prop => false,

            // Handle remaining types (Exists, Forall, Meta, Future, Generator, etc.)
            Exists { body, .. } | Forall { body, .. } => {
                self.contains_placeholder_impl(body, depth + 1)
            }

            Meta { .. } => false,

            Future { output } => self.contains_placeholder_impl(output, depth + 1),

            Generator {
                yield_ty,
                return_ty,
            } => {
                self.contains_placeholder_impl(yield_ty, depth + 1)
                    || self.contains_placeholder_impl(return_ty, depth + 1)
            }

            // Catch-all for any new variants
            _ => false,
        }
    }

    /// Substitute all placeholder types with their resolved types.
    ///
    /// This performs a deep traversal and replaces any Placeholder types
    /// with their corresponding resolved types from the type context.
    ///
    /// If a placeholder cannot be resolved (type not found), it is left as-is.
    pub fn substitute_placeholders(&self, ty: &Type) -> Type {
        self.substitute_placeholders_impl(ty, 0, &mut std::collections::HashSet::new())
    }

    /// Internal implementation with depth tracking and cycle detection.
    fn substitute_placeholders_impl(
        &self,
        ty: &Type,
        depth: usize,
        resolving: &mut std::collections::HashSet<Text>,
    ) -> Type {
        const MAX_DEPTH: usize = 100;
        if depth > MAX_DEPTH {
            return ty.clone();
        }

        use Type::*;
        let next_depth = depth + 1;

        match ty {
            // Placeholder - try to resolve it
            Placeholder { name, span } => {
                // Check for circular reference (type resolves to itself)
                if resolving.contains(name) {
                    // Circular reference detected - return as-is to avoid infinite loop
                    return ty.clone();
                }

                resolving.insert(name.clone());

                let result = if let Maybe::Some(resolved_ty) = self.ctx.lookup_type(name.as_str()) {
                    // Found the resolved type
                    if let Placeholder {
                        name: resolved_name,
                        ..
                    } = resolved_ty
                    {
                        // Still a placeholder - might need another pass
                        if resolved_name == name {
                            // Self-referential placeholder - return as-is
                            ty.clone()
                        } else {
                            // Different placeholder - recursively resolve
                            self.substitute_placeholders_impl(resolved_ty, next_depth, resolving)
                        }
                    } else {
                        // Not a placeholder - recursively resolve any nested placeholders
                        self.substitute_placeholders_impl(resolved_ty, next_depth, resolving)
                    }
                } else {
                    // Not found - return as-is (will be caught by verify_no_placeholders)
                    ty.clone()
                };

                resolving.remove(name);
                result
            }

            // Named and Generic types - resolve type arguments
            Named { path, args } => {
                let new_args: List<Type> = args
                    .iter()
                    .map(|arg| self.substitute_placeholders_impl(arg, next_depth, resolving))
                    .collect();
                Named {
                    path: path.clone(),
                    args: new_args,
                }
            }

            Generic { name, args } => {
                let new_args: List<Type> = args
                    .iter()
                    .map(|arg| self.substitute_placeholders_impl(arg, next_depth, resolving))
                    .collect();
                Generic {
                    name: name.clone(),
                    args: new_args,
                }
            }

            // Function types
            Function {
                property_requirement,
                params,
                return_type,
                contexts,
                type_params,
                properties,
            } => {
                let new_params: List<Type> = params
                    .iter()
                    .map(|p| self.substitute_placeholders_impl(p, next_depth, resolving))
                    .collect();
                let new_return =
                    self.substitute_placeholders_impl(return_type, next_depth, resolving);
                Function {
                    property_requirement: property_requirement.clone(),
                    params: new_params,
                    return_type: Box::new(new_return),
                    contexts: contexts.clone(),
                    type_params: type_params.clone(),
                    properties: properties.clone(),
                }
            }

            // Tuple types
            Tuple(elements) => {
                let new_elements: List<Type> = elements
                    .iter()
                    .map(|e| self.substitute_placeholders_impl(e, next_depth, resolving))
                    .collect();
                Tuple(new_elements)
            }

            // Record types
            Record(fields) => {
                let new_fields: indexmap::IndexMap<verum_common::Text, Type> = fields
                    .iter()
                    .map(|(name, ty)| {
                        (
                            name.clone(),
                            self.substitute_placeholders_impl(ty, next_depth, resolving),
                        )
                    })
                    .collect();
                Record(new_fields)
            }

            // Variant types
            Variant(variants) => {
                let new_variants: indexmap::IndexMap<verum_common::Text, Type> = variants
                    .iter()
                    .map(|(name, ty)| {
                        (
                            name.clone(),
                            self.substitute_placeholders_impl(ty, next_depth, resolving),
                        )
                    })
                    .collect();
                Variant(new_variants)
            }

            // Reference types
            Reference { inner, mutable } => Reference {
                inner: Box::new(self.substitute_placeholders_impl(inner, next_depth, resolving)),
                mutable: *mutable,
            },

            CheckedReference { inner, mutable } => CheckedReference {
                inner: Box::new(self.substitute_placeholders_impl(inner, next_depth, resolving)),
                mutable: *mutable,
            },

            UnsafeReference { inner, mutable } => UnsafeReference {
                inner: Box::new(self.substitute_placeholders_impl(inner, next_depth, resolving)),
                mutable: *mutable,
            },

            Ownership { inner, mutable } => Ownership {
                inner: Box::new(self.substitute_placeholders_impl(inner, next_depth, resolving)),
                mutable: *mutable,
            },

            Pointer { inner, mutable } => Pointer {
                inner: Box::new(self.substitute_placeholders_impl(inner, next_depth, resolving)),
                mutable: *mutable,
            },

            GenRef { inner } => GenRef {
                inner: Box::new(self.substitute_placeholders_impl(inner, next_depth, resolving)),
            },

            // Array and slice types
            Array { element, size } => Array {
                element: Box::new(
                    self.substitute_placeholders_impl(element, next_depth, resolving),
                ),
                size: *size,
            },

            Slice { element } => Slice {
                element: Box::new(
                    self.substitute_placeholders_impl(element, next_depth, resolving),
                ),
            },

            // Refined types
            Refined { base, predicate } => Refined {
                base: Box::new(self.substitute_placeholders_impl(base, next_depth, resolving)),
                predicate: predicate.clone(),
            },

            // Quantified types
            Quantified { inner, quantity } => Quantified {
                inner: Box::new(self.substitute_placeholders_impl(inner, next_depth, resolving)),
                quantity: *quantity,
            },

            // Dependent types
            Pi {
                param_name,
                param_type,
                return_type,
            } => Pi {
                param_name: param_name.clone(),
                param_type: Box::new(
                    self.substitute_placeholders_impl(param_type, next_depth, resolving),
                ),
                return_type: Box::new(self.substitute_placeholders_impl(
                    return_type,
                    next_depth,
                    resolving,
                )),
            },

            Sigma {
                fst_name,
                fst_type,
                snd_type,
            } => Sigma {
                fst_name: fst_name.clone(),
                fst_type: Box::new(
                    self.substitute_placeholders_impl(fst_type, next_depth, resolving),
                ),
                snd_type: Box::new(
                    self.substitute_placeholders_impl(snd_type, next_depth, resolving),
                ),
            },

            Eq {
                ty: eq_ty,
                lhs,
                rhs,
            } => Eq {
                ty: Box::new(self.substitute_placeholders_impl(eq_ty, next_depth, resolving)),
                lhs: lhs.clone(),
                rhs: rhs.clone(),
            },

            // Primitive types - no placeholders possible
            Unit
            | Bool
            | Int
            | Float
            | Char
            | Text
            | Never
            | Var(_)
            | Universe { .. }
            | Lifetime { .. }
            | Prop => ty.clone(),

            // Handle remaining types (Exists, Forall, Meta, Future, Generator, etc.)
            Exists { var, body } => Exists {
                var: *var,
                body: Box::new(self.substitute_placeholders_impl(body, next_depth, resolving)),
            },

            Forall { vars, body } => Forall {
                vars: vars.clone(),
                body: Box::new(self.substitute_placeholders_impl(body, next_depth, resolving)),
            },

            Meta {
                name,
                ty,
                refinement,
                value,
            } => Meta {
                name: name.clone(),
                ty: Box::new(self.substitute_placeholders_impl(ty, next_depth, resolving)),
                refinement: refinement.clone(),
                value: value.clone(),
            },

            Future { output } => Future {
                output: Box::new(self.substitute_placeholders_impl(output, next_depth, resolving)),
            },

            Generator {
                yield_ty,
                return_ty,
            } => Generator {
                yield_ty: Box::new(
                    self.substitute_placeholders_impl(yield_ty, next_depth, resolving),
                ),
                return_ty: Box::new(
                    self.substitute_placeholders_impl(return_ty, next_depth, resolving),
                ),
            },

            // TypeApp: resolve constructor placeholder vars and recurse into args
            // CRITICAL: During recursive record type registration, self-references like Node<T>
            // become TypeApp { constructor: Var(placeholder), args: [T] }. After registration,
            // the placeholder is no longer bound, so we need to resolve it here by looking up
            // the type name associated with the placeholder var.
            TypeApp { constructor, args } => {
                let resolved_constructor =
                    self.substitute_placeholders_impl(constructor, next_depth, resolving);
                let new_args: List<Type> = args
                    .iter()
                    .map(|arg| self.substitute_placeholders_impl(arg, next_depth, resolving))
                    .collect();
                // If constructor resolved to a Named type, convert TypeApp to Named with args
                match &resolved_constructor {
                    Named {
                        path,
                        args: existing_args,
                    } if existing_args.is_empty() => Named {
                        path: path.clone(),
                        args: new_args,
                    },
                    _ => TypeApp {
                        constructor: Box::new(resolved_constructor),
                        args: new_args,
                    },
                }
            }

            // Catch-all for any new variants - return as-is
            _ => ty.clone(),
        }
    }

    /// Detect circular type references that would cause infinite type size.
    ///
    /// This detects direct cycles like:
    /// ```verum
    /// type A is { b: B };
    /// type B is { a: A }; // ERROR: A -> B -> A cycle without indirection
    /// ```
    ///
    /// Returns a list of detected cycles, where each cycle is represented
    /// as a list of type names in the cycle (e.g., ["A", "B", "A"]).
    pub fn detect_circular_types(&self) -> List<List<verum_common::Text>> {
        let mut cycles = List::new();
        let mut visited = std::collections::HashSet::new();

        for (name, ty) in self.ctx.all_types() {
            if name.as_str().starts_with("__") {
                continue;
            }
            if !visited.contains(name) {
                let mut path = List::new();
                let mut path_set = std::collections::HashSet::new();
                self.detect_cycles_dfs(
                    name,
                    ty,
                    &mut path,
                    &mut path_set,
                    &mut visited,
                    &mut cycles,
                );
            }
        }

        cycles
    }

    /// DFS helper for cycle detection.
    fn detect_cycles_dfs(
        &self,
        type_name: &Text,
        ty: &Type,
        path: &mut List<verum_common::Text>,
        path_set: &mut std::collections::HashSet<Text>,
        visited: &mut std::collections::HashSet<Text>,
        cycles: &mut List<List<verum_common::Text>>,
    ) {
        // Skip if already visited in a complete traversal
        if visited.contains(type_name) {
            return;
        }

        // Check for cycle
        if path_set.contains(type_name) {
            // Found a cycle! Build the cycle path
            let mut cycle = List::new();
            let mut in_cycle = false;
            for name in path.iter() {
                if name == type_name {
                    in_cycle = true;
                }
                if in_cycle {
                    cycle.push(name.clone());
                }
            }
            cycle.push(type_name.clone());
            cycles.push(cycle);
            return;
        }

        // Add to current path
        path.push(type_name.clone());
        path_set.insert(type_name.clone());

        // Get referenced type names from this type
        let referenced_types = self.get_direct_type_references(ty);
        for ref_name in referenced_types {
            if let Maybe::Some(ref_ty) = self.ctx.lookup_type(ref_name.as_str()) {
                self.detect_cycles_dfs(&ref_name, ref_ty, path, path_set, visited, cycles);
            }
        }

        // Remove from current path
        path.pop();
        path_set.remove(type_name);

        // Mark as completely visited
        visited.insert(type_name.clone());
    }

    /// Get direct type references from a type (non-recursive).
    ///
    /// This returns the type names that are directly referenced by this type,
    /// excluding indirect references through box/reference types (which provide indirection).
    fn get_direct_type_references(&self, ty: &Type) -> List<verum_common::Text> {
        let mut refs = List::new();

        match ty {
            // Named types reference their base type
            Type::Named { path, .. } => {
                if let Some(ident) = path.as_ident() {
                    refs.push(ident.name.clone());
                }
            }

            // Record fields are direct references
            Type::Record(fields) => {
                for field_ty in fields.values() {
                    // Skip reference types - they provide indirection
                    if !self.is_indirect_type(field_ty) {
                        refs.extend(self.get_direct_type_references(field_ty));
                    }
                }
            }

            // Variant payloads are direct references
            Type::Variant(variants) => {
                for payload_ty in variants.values() {
                    if !self.is_indirect_type(payload_ty) {
                        refs.extend(self.get_direct_type_references(payload_ty));
                    }
                }
            }

            // Placeholders reference their target type
            Type::Placeholder { name, .. } => {
                refs.push(name.clone());
            }

            // Other types don't create direct cycles
            _ => {}
        }

        refs
    }

    /// Check if a type provides indirection (breaking potential cycles).
    ///
    /// These types store their inner type behind a pointer, allowing
    /// recursive type definitions:
    /// - Box<T>
    /// - &T, &checked T, &unsafe T
    /// - Pointer<T>
    fn is_indirect_type(&self, ty: &Type) -> bool {
        match ty {
            Type::Reference { .. }
            | Type::CheckedReference { .. }
            | Type::UnsafeReference { .. }
            | Type::Pointer { .. }
            | Type::VolatilePointer { .. }
            | Type::GenRef { .. } => true,

            // Check for heap allocation types (Heap<T>, Shared<T>) that provide indirection
            // STDLIB-AGNOSTIC: These are the standard Verum indirection types.
            // Note: Maybe and Result are sum types, NOT indirection types.
            Type::Named { path, .. } => {
                if let Some(ident) = path.as_ident() {
                    matches!(ident.name.as_str(), WKT_SHARED | WKT_HEAP)
                } else {
                    false
                }
            }

            _ => false,
        }
    }

    /// Register a protocol declaration
    /// This should be called before type checking
    pub fn register_protocol(&mut self, _protocol_decl: &verum_ast::ProtocolDecl) -> Result<()> {
        // Protocol registration is handled by register_protocol_decl_item
        Ok(())
    }

    /// Check if an AST type contains `Self` — used for object safety enforcement.
    fn type_contains_self(&self, ty: &verum_ast::Type) -> bool {
        match &ty.kind {
            verum_ast::ty::TypeKind::Path(path) => {
                // Self is represented as a single-segment path with name "Self"
                path.segments.iter().any(|s| match s {
                    verum_ast::ty::PathSegment::Name(id) => id.name.as_str() == "Self",
                    verum_ast::ty::PathSegment::SelfValue => true,
                    _ => false,
                })
            }
            verum_ast::ty::TypeKind::Tuple(types) => {
                types.iter().any(|t| self.type_contains_self(t))
            }
            verum_ast::ty::TypeKind::Reference { inner, .. } => self.type_contains_self(inner),
            verum_ast::ty::TypeKind::Array { element, .. } => self.type_contains_self(element),
            _ => false,
        }
    }

    /// Canonical protocol admission is independent of the source spelling.
    fn validate_protocol_type_declaration(&self, type_decl: &verum_ast::TypeDecl,
        body: &verum_ast::decl::ProtocolBody) -> Result<()> {
        let type_name = &type_decl.name.name;
        // Manifest-driven gate for HKT-bearing protocol declarations.
        // When `[protocols].higher_kinded_protocols = false` (the
        // default) a protocol that declares any `GenericParamKind::
        // HigherKinded` parameter (e.g. `protocol Functor<F<_>>`)
        // is rejected with a manifest-citing diagnostic. Closes the
        // inert-defense pattern at session.rs:590 — pre-fix the
        // resolver always accepted HKT protocols regardless of
        // manifest.
        //

        // Manifest validation also enforces that this flag can only
        // be true when `[types].higher_kinded = true`, so a
        // user-set HKT protocol path always reaches a fully-enabled
        // HKT pipeline.
        if !self.higher_kinded_protocols_enabled {
            for generic in type_decl.generics.iter() {
                if let verum_ast::ty::GenericParamKind::HigherKinded { name, arity, .. } =
                    &generic.kind
                {
                    let placeholders = (0..*arity).map(|_| "_").collect::<Vec<_>>().join(",");
                    return Err(TypeError::Other(verum_common::Text::from(format!(
                        "[protocols].higher_kinded_protocols = false rejects \
                         higher-kinded type parameter `{}<{}>` on protocol \
                         `{}`. Set [protocols].higher_kinded_protocols = true \
                         in Verum.toml to enable HKT protocol declarations \
                         (also requires [types].higher_kinded = true).",
                        name.name.as_str(),
                        placeholders,
                        type_name.as_str(),
                    ))));
                }
            }
        }

        // Manifest-driven gate for Generic Associated Types (GATs).
        // When `[protocols].generic_associated_types = false` (the
        // default) a protocol body that contains an associated-type
        // declaration with non-empty `type_params` (e.g.
        // `type Item<T>` inside `protocol Stream { ... }`) is
        // rejected with a manifest-citing diagnostic. Closes #265.
        //

        // Manifest validation also enforces that this flag can only
        // be true when `[protocols].associated_types = true`.
        // Regular zero-parameter associated types
        // (`type Output;`) are unaffected by this gate.
        if !self.generic_associated_types_enabled {
            for item in body.items.iter() {
                if let verum_ast::decl::ProtocolItemKind::Type {
                    name, type_params, ..
                } = &item.kind
                {
                    if !type_params.is_empty() {
                        return Err(TypeError::Other(verum_common::Text::from(format!(
                            "[protocols].generic_associated_types = false rejects \
                             generic associated type `{}<...>` ({} type parameter{}) \
                             on protocol `{}`. Set [protocols].generic_associated_types \
                             = true in Verum.toml to enable GAT declarations (also \
                             requires [protocols].associated_types = true).",
                            name.name.as_str(),
                            type_params.len(),
                            if type_params.len() == 1 { "" } else { "s" },
                            type_name.as_str(),
                        ))));
                    }
                }
            }
        }

        // Object safety enforcement for context protocols.
        // Context protocols must be object-safe because they're used via dynamic dispatch
        // in the DI system (vtable-based runtime resolution).
        // Check: methods must not return Self, must not have generic type params.
        if body.is_context {
            for item in body.items.iter() {
                if let verum_ast::decl::ProtocolItemKind::Function { decl: func, .. } = &item.kind {
                    // Check for Self in return type (not object-safe for context protocols)
                    if let verum_common::Maybe::Some(ref ret_ty) = func.return_type {
                        if self.type_contains_self(ret_ty) {
                            return Err(TypeError::Other(verum_common::Text::from(format!(
                                "context protocol '{}' is not object-safe: method '{}' returns Self.\n  \
                                 note: context protocols require object safety for dynamic dispatch.\n  \
                                 help: return a concrete type or Heap<dyn {}> instead",
                                type_name, func.name.name, type_name
                            ))));
                        }
                    }
                    // Check for generic type params on methods (not object-safe)
                    if !func.generics.is_empty() {
                        return Err(TypeError::Other(verum_common::Text::from(format!(
                            "context protocol '{}' is not object-safe: method '{}' has generic type parameters.\n  \
                             note: context protocols require object safety for dynamic dispatch.\n  \
                             help: remove generic parameters or use concrete types",
                            type_name, func.name.name
                        ))));
                    }
                }
            }
        }

        Ok(())
    }

    /// Older AST producers use ProtocolDecl; admission still has one authority.
    pub(super) fn register_protocol_decl_item(&mut self, declaration: &verum_ast::ProtocolDecl) -> Result<()> {
        self.register_type_declaration(&verum_ast::TypeDecl {
            visibility: declaration.visibility.clone(),
            name: declaration.name.clone(),
            generics: declaration.generics.clone(),
            attributes: List::new(),
            body: verum_ast::TypeDeclBody::Protocol(verum_ast::decl::ProtocolBody {
                is_context: declaration.is_context,
                extends: declaration.bounds.clone(),
                generic_where_clause: declaration.generic_where_clause.clone(),
                items: declaration.items.clone(),
                declared_default_methods: List::new(),
            }),
            value_params: List::new(),
            resource_modifier: None,
            generic_where_clause: None,
            meta_where_clause: declaration.meta_where_clause.clone(),
            span: declaration.span,
        })
    }

    /// Register method signatures from an implement block (Pass 1)
    ///
    /// This registers all method signatures (both static and instance) WITHOUT type-checking
    /// their bodies. This allows methods in different implement blocks for the same type
    /// to call each other, and enables forward references within the same block.
    ///
    /// This should be called before `check_impl_block` (which type-checks method bodies).
    /// Register method signatures from an implementation block.
    ///
    pub fn register_impl_block(&mut self, impl_decl: &verum_ast::decl::ImplDecl) -> Result<()> {
        self.register_impl_block_expected(impl_decl, None)
    }

    pub(super) fn register_owned_impl_block(
        &mut self,
        impl_decl: &verum_ast::decl::ImplDecl,
        expected_protocol: &Text,
        expected_target: &Type,
    ) -> Result<()> {
        self.register_impl_block_expected(impl_decl, Some(ExpectedImplementation::Protocol(expected_protocol, expected_target)))
    }

    pub(super) fn register_owned_inherent_impl_block(
        &mut self,
        declaration: &verum_ast::decl::ImplDecl,
        expected_target: &Type,
    ) -> Result<()> {
        self.register_impl_block_expected(declaration, Some(ExpectedImplementation::Inherent(expected_target)))
    }

    /// Nominal attachment never changes the full receiver used by selection.
    /// Source methods and VBC ownership use the same reference/pointer anchor.
    fn implementation_attachment_type(mut receiver: &Type) -> &Type {
        while let Type::Reference { inner, .. }
            | Type::CheckedReference { inner, .. }
            | Type::UnsafeReference { inner, .. }
            | Type::Pointer { inner, .. } = receiver
        { receiver = inner; }
        receiver
    }

    fn validate_impl_owner(
        &self,
        implementation: &crate::protocol::ProtocolImpl,
        expected: (&Text, &Type),
    ) -> Result<()> {
        let protocol = self.path_to_string(&implementation.protocol);
        if &protocol != expected.0 {
            return Err(TypeError::OtherWithCodeSpanned { code: "E405".into(),
                msg: "source implementation differs from its metadata protocol owner".into(), span: implementation.span });
        }
        self.validate_impl_target(&implementation.for_type, expected.1, implementation.span)
    }

    fn validate_impl_target(&self, actual: &Type, expected: &Type, span: Span) -> Result<()> {
        let attachment = Self::implementation_attachment_type(actual);
        if matches!((attachment, expected),
            (Type::Slice { .. }, Type::Slice { .. }) | (Type::Array { .. }, Type::Array { .. }))
        {
            return Ok(());
        }
        let target = match attachment {
            Type::Named { path, .. } => Type::Named { path: path.clone(), args: List::new() },
            Type::Generic { name, .. } if !name.starts_with("::") => Type::Generic { name: name.clone(), args: List::new() },
            ty if !matches!(ty, Type::Unknown) && ty.primitive_name().is_some() => ty.clone(),
            _ => return Err(TypeError::OtherWithCodeSpanned { code: "E405".into(),
                msg: "source implementation metadata requires a declared target identity".into(), span }),
        };
        if !self.protocol_checker.read().types_structurally_equal(&target, expected) {
            return Err(TypeError::OtherWithCodeSpanned { code: "E405".into(),
                msg: format!(
                    "source implementation differs from its metadata protocol or target owner: source `{}`, metadata `{}`",
                    target.to_text(), expected.to_text()
                ).into(), span });
        }
        Ok(())
    }

    fn register_impl_block_expected(
        &mut self,
        impl_decl: &verum_ast::decl::ImplDecl,
        expected: Option<ExpectedImplementation<'_>>,
    ) -> Result<()> {
        let filtered = self.cfg_filtered_impl(impl_decl);
        let declaration = filtered.as_ref().unwrap_or(impl_decl);
        if matches!(expected, Some(ExpectedImplementation::Inherent(_)))
            && !matches!(declaration.kind, verum_ast::decl::ImplKind::Inherent(_)) {
            return Err(TypeError::Other("inherent metadata contains a protocol implementation".into()));
        }
        if matches!(declaration.kind, verum_ast::decl::ImplKind::Protocol { .. }) {
            let registry = self.protocol_checker.read();
            if let Some(implementation) = registry.source_impl(&self.current_module_path, declaration)? {
                if let Some(ExpectedImplementation::Protocol(protocol, target)) = expected { self.validate_impl_owner(implementation, (protocol, target))?; }
                return Ok(());
            }
        } else if matches!(expected, Some(ExpectedImplementation::Protocol(_, _))) {
            return Err(TypeError::OtherWithCodeSpanned {
                code: "E405".into(), msg: "associated-family metadata requires a protocol implementation".into(), span: declaration.span,
            });
        }
        self.with_impl_parameter_scope(declaration, |checker, owner|
            checker.register_impl_block_inner(declaration, expected, owner)
        )?;
        if matches!(expected, Some(ExpectedImplementation::Protocol(_, _)))
            && self.protocol_checker.read().source_impl(&self.current_module_path, declaration)?.is_none()
        {
            return Err(TypeError::OtherWithCodeSpanned {
                code: "E405".into(),
                msg: "source implementation did not produce a declaration-owned registration".into(),
                span: declaration.span,
            });
        }
        Ok(())
    }

    pub(super) fn source_impl_specialization(
        declaration: &verum_ast::decl::ImplDecl,
    ) -> Result<Maybe<crate::advanced_protocols::SpecializationInfo>> {
        let Some(attribute) = &declaration.specialize_attr else { return Ok(None); };
        if attribute.negative || attribute.when_clause.is_some() {
            return Err(TypeError::OtherWithCodeSpanned {
                code: "E405".into(),
                msg: "conditional and negative specialization require an implementation constraint carrier".into(),
                span: attribute.span,
            });
        }
        let mut specialization = crate::advanced_protocols::SpecializationInfo::none();
        specialization.is_specialized = true;
        specialization.span = attribute.span;
        if let Some(rank) = attribute.rank {
            specialization.specificity_rank = usize::try_from(rank).map_err(|_| TypeError::OtherWithCodeSpanned {
                code: "E405".into(),
                msg: "specialization rank must be nonnegative".into(),
                span: attribute.span,
            })?;
        }
        Ok(Some(specialization))
    }

    /// Drop the impl items `@cfg(...)` excludes for this target, or `None`
    /// when the block has none — the overwhelmingly common case, which
    /// then costs one `is_empty()` per item and no clone.
    ///
    /// T0784: item-level `@cfg` was honoured for top-level items and not
    /// for items INSIDE an `implement` block, so a method written once per
    /// platform was reported as a duplicate AND its body was type-checked
    /// for the wrong target.  `core/net/unix.vr` spells `peer_cred` three
    /// times — Linux `ucred`, macOS `xucred`, Windows unsupported — and
    /// the Linux body calls a direct syscall that exists only there.
    ///
    /// Filtered ONCE here rather than at the eight `impl_decl.items` loops
    /// downstream, so a single place decides what belongs to this target,
    /// and it asks the same `CfgEvaluator` codegen and the statement
    /// filter ask.
    pub(super) fn cfg_filtered_impl(
        &self,
        impl_decl: &verum_ast::decl::ImplDecl,
    ) -> Option<verum_ast::decl::ImplDecl> {
        let include = |item: &verum_ast::decl::ImplItem| {
            item.attributes.is_empty() || self.cfg_evaluator.should_include(&item.attributes)
        };
        // Most attributed methods are still included. Find an actual exclusion
        // before copying any body; attributes alone must not clone the AST.
        let first_excluded = impl_decl.items.iter().position(|item| !include(item))?;
        let items = impl_decl.items.as_slice()[..first_excluded].iter()
            .chain(impl_decl.items.as_slice()[first_excluded + 1..].iter().filter(|item| include(item)))
            .cloned()
            .collect();
        Some(verum_ast::decl::ImplDecl {
            is_unsafe: impl_decl.is_unsafe,
            generics: impl_decl.generics.clone(),
            kind: impl_decl.kind.clone(),
            generic_where_clause: impl_decl.generic_where_clause.clone(),
            meta_where_clause: impl_decl.meta_where_clause.clone(),
            specialize_attr: impl_decl.specialize_attr.clone(),
            items,
            span: impl_decl.span,
        })
    }

    /// A receiver can reuse its nominal declaration's positional type
    /// parameters without repeating an impl telescope. Other names denote
    /// ordinary lexical types and must resolve as such.
    pub(super) fn implicit_impl_receiver_parameters(
        &mut self,
        implementation: &verum_ast::ImplDecl,
    ) -> Result<List<Text>> {
        use verum_ast::ty::{GenericArg, GenericParamKind, TypeKind};

        enum ReceiverSlot {
            Type(Text),
            Value(Text),
            Other,
        }

        let mut binders = List::new();
        if !implementation.generics.is_empty() {
            return Ok(binders);
        }
        let receiver = match &implementation.kind {
            verum_ast::decl::ImplKind::Inherent(ty) => ty,
            verum_ast::decl::ImplKind::Protocol { for_type, .. } => for_type,
        };
        let TypeKind::Generic { base, args } = &receiver.kind else {
            return Ok(binders);
        };
        let TypeKind::Path(receiver_path) = &base.kind else {
            return Ok(binders);
        };
        let (_, key) = self.resolve_qualified_type_with_parameter_key(receiver_path, base.span)?;
        let source_slots = |declaration: &verum_ast::TypeDecl| -> List<ReceiverSlot> {
            declaration.generics
                .iter()
                .filter(|p| {
                    !p.is_implicit
                        && !matches!(p.kind, GenericParamKind::Lifetime { .. })
                })
                .map(|p| match &p.kind {
                    GenericParamKind::Type { name, .. } => ReceiverSlot::Type(name.name.clone()),
                    GenericParamKind::Meta { name, .. } | GenericParamKind::Const { name, .. } => {
                        ReceiverSlot::Value(name.name.clone())
                    }
                    _ => ReceiverSlot::Other,
                })
                .collect()
        };
        let selected = key.as_ref().and_then(|owner| self.source_type_registration.declaration(owner))
            .map(source_slots).or_else(|| {
                receiver_path.as_ident().and_then(|name| self.declaring_source_type_declaration(name.as_str()))
                    .map(|(declaration, _)| source_slots(&declaration))
            });
        let receiver_slots = if let Some(slots) = selected {
            slots
        } else if let Some(Type::Record(slots)) = key
            .as_ref()
            .and_then(|key| self.ctx.lookup_type(&format!("__type_params_{key}")))
        {
            slots
                .iter()
                .map(|(name, ty)| match ty {
                    Type::Var(_) => ReceiverSlot::Type(name.clone()),
                    Type::Named { path, args }
                        if args.is_empty()
                            && path.as_ident().is_some_and(|id| &id.name == name) =>
                    {
                        ReceiverSlot::Type(name.clone())
                    }
                    Type::Meta { .. } => ReceiverSlot::Value(name.clone()),
                    _ => ReceiverSlot::Other,
                })
                .collect()
        } else {
            return Ok(binders);
        };
        let mut unresolved = List::new();
        for (position, argument) in args
            .iter()
            .filter(|arg| matches!(arg, GenericArg::Type(_) | GenericArg::Const(_)))
            .enumerate()
        {
            let GenericArg::Type(argument) = argument else {
                continue;
            };
            let TypeKind::Path(path) = &argument.kind else {
                continue;
            };
            let Some(ident) = path.as_ident() else {
                continue;
            };
            let name = &ident.name;
            let occupied = self.declaring_source_type_declaration(name).is_some()
                || self.ctx.lookup_type(name).is_some_and(|ty| match ty {
                    Type::Var(_) => false,
                    Type::Named { path, args }
                        if args.is_empty()
                            && path.as_ident().is_some_and(|id| &id.name == name) =>
                    {
                        verum_common::well_known_types::type_names::is_primitive_value_type(name)
                    }
                    _ => true,
                });
            if occupied {
                continue;
            }
            match receiver_slots.get(position) {
                Some(ReceiverSlot::Type(parameter)) if parameter == name => binders.push(name.clone()),
                Some(ReceiverSlot::Value(parameter)) if parameter == name => {
                    let owner = key.clone().unwrap_or_else(|| self.path_to_string(receiver_path));
                    return Err(TypeError::OtherWithCodeSpanned {
                        code: "E407".into(),
                        msg: format!("argument for `{owner}.{parameter}` has the wrong kind: an implicit type parameter cannot supply a compile-time value").into(),
                        span: argument.span,
                    });
                }
                _ => unresolved.push((name, argument.span)),
            }
        }
        // A repeated use may precede the positional declaration of a binder.
        for (name, span) in unresolved {
            if !binders.contains(name) {
                let (resolved, key) =
                    self.resolve_declaring_source_type_with_parameter_key(name, span)?;
                if matches!(resolved, Type::Var(_)) && key.is_none() {
                    return Err(TypeError::TypeNotFound {
                        name: name.clone(),
                        span,
                    });
                }
            }
        }
        Ok(binders)
    }

    fn with_impl_parameter_scope<T>(
        &mut self,
        declaration: &verum_ast::ImplDecl,
        operation: impl FnOnce(&mut Self, &indexmap::IndexMap<Text, Type>) -> Result<T>,
    ) -> Result<T> {
        use verum_ast::ty::GenericParamKind;
        let implicit = self.implicit_impl_receiver_parameters(declaration)?;
        let saved: Vec<_> = declaration
            .generics
            .iter()
            .filter_map(|parameter| match &parameter.kind {
                GenericParamKind::Type { name, .. }
                | GenericParamKind::Meta { name, .. }
                | GenericParamKind::Const { name, .. } => Some(&name.name),
                _ => None,
            })
            .chain(implicit.iter())
            .map(|name| (name.clone(), self.ctx.type_defs.get(name).cloned()))
            .collect();
        let previous_self = self.current_self_type.clone();
        let previous_mode = (self.in_impl_block, self.in_inherent_impl);
        // Meta parameter sorts see preceding binders; inline bounds and where
        // clauses are elaborated after the entire declaration frame is present.
        self.push_decl_param_frame(indexmap::IndexMap::new());
        let result = self.in_impl_proof_scope(|checker| {
            for parameter in &declaration.generics {
                let (name, ty) = match &parameter.kind {
                    GenericParamKind::Type { name, .. } => (name, Type::Var(TypeVar::fresh())),
                    GenericParamKind::Meta { name, .. } | GenericParamKind::Const { name, .. } =>
                        (name, checker.source_value_parameter(parameter)?),
                    _ => continue,
                };
                checker.ctx.define_type(name.name.clone(), ty.clone());
                checker.decl_param_frames.last_mut().expect("implementation frame")
                    .insert(name.name.clone(), ty);
            }
            for name in implicit {
                let ty = Type::Var(TypeVar::fresh());
                checker.ctx.define_type(name.clone(), ty.clone());
                checker.decl_param_frames.last_mut().expect("implementation frame").insert(name, ty);
            }
            let owner = checker.decl_param_frames.last().expect("implementation frame").clone();
            operation(checker, &owner)
        });
        self.pop_decl_param_frame();
        for (name, prior) in saved {
            if let Some(ty) = prior {
                self.ctx.type_defs.insert(name, ty);
            } else {
                self.ctx.type_defs.remove(&name);
            }
        }
        self.set_current_self_type(previous_self);
        (self.in_impl_block, self.in_inherent_impl) = previous_mode;
        result
    }

    fn register_impl_block_inner(
        &mut self,
        impl_decl: &verum_ast::decl::ImplDecl,
        expected: Option<ExpectedImplementation<'_>>,
        owner: &indexmap::IndexMap<Text, Type>,
    ) -> Result<()> {
        use verum_ast::decl::ImplKind;
        let names = owner.keys().cloned().collect();
        let (clauses, callables) = self.enter_source_impl_conditions(impl_decl, owner)?;
        match &impl_decl.kind {
            ImplKind::Inherent(target) => {
                if let Some(ExpectedImplementation::Inherent(expected)) = expected {
                    let actual = self.ast_to_type(target)?;
                    self.validate_impl_target(&actual, expected, impl_decl.span)?;
                }
                self.register_inherent_impl_methods(impl_decl, &names, &clauses, &callables)
            }
            ImplKind::Protocol { .. } => self.register_protocol_impl_methods(
                impl_decl, &names, owner, clauses, callables,
                match expected {
                    Some(ExpectedImplementation::Protocol(protocol, target)) => Some((protocol, target)),
                    _ => None,
                },
            ),
        }
    }

    /// Register a function signature without type-checking the body
    ///
    /// This enables forward references by registering all function signatures
    /// before any function bodies are checked. For example:
    ///
    /// ```verum
    /// fn main() -> Int {
    ///  fib(10) // fib is defined below, but this works due to forward ref support
    /// }
    ///
    /// fn fib(n: Int) -> Int {
    ///  if n <= 1 { n } else { fib(n - 1) + fib(n - 2) }
    /// }
    /// ```
    ///
    /// This should be called in a pass before `check_item` to ensure all
    /// functions are available in the environment.
    /// Pre-register a constant declaration's type for forward reference support.
    ///
    /// This is called before Phase 2 (type checking) so that constants defined
    /// after functions in source order are still visible within function bodies.
    /// Reject duplicate item names within a single `impl` / `implement` block.
    ///
    /// Without this guard the HashMap-backed registries (`inherent_methods`,
    /// protocol-method tables, associated-type tables) silently let the second
    /// definition overwrite the first, so the second body wins — a defect
    /// witnessed by `Heap.is_freed` appearing twice in `core/base/memory.vr`
    /// before commit X (both bodies identical, but the second masking the
    /// first is structurally indistinguishable from a typo or a botched
    /// merge). Method/const/type/proof names share a single per-impl
    /// namespace, so we reject collisions across kinds too.
    fn check_no_duplicate_impl_items(
        &self,
        impl_decl: &verum_ast::decl::ImplDecl,
    ) -> Result<()> {
        use std::collections::HashSet;
        use verum_ast::decl::ImplItemKind;
        let mut seen: HashSet<String> = HashSet::new();
        for item in impl_decl.items.iter() {
            // A method excluded by `@cfg` is not part of THIS target's
            // program, so it cannot collide with anything (T0778 family).
            // Three platform bodies under one name is the correct way to
            // write a platform-dependent method — `core/net/unix.vr`
            // spells `peer_cred` once per OS — and reporting them as a
            // duplicate is worse than a false alarm: the advice below
            // says to rename or merge them, which is precisely what must
            // NOT be done.  Asks the same `CfgEvaluator` the statement
            // filter and codegen ask.
            if !item.attributes.is_empty()
                && !self.cfg_evaluator.should_include(&item.attributes)
            {
                continue;
            }
            let name = match &item.kind {
                ImplItemKind::Function(f) => f.name.name.as_str().to_string(),
                ImplItemKind::Type { name, .. } => name.name.as_str().to_string(),
                ImplItemKind::Const { name, .. } => name.name.as_str().to_string(),
                ImplItemKind::Proof { axiom_name, .. } => axiom_name.name.as_str().to_string(),
            };
            if !seen.insert(name.clone()) {
                let kind = match &item.kind {
                    ImplItemKind::Function(_) => "method",
                    ImplItemKind::Type { .. } => "associated type",
                    ImplItemKind::Const { .. } => "associated const",
                    ImplItemKind::Proof { .. } => "proof clause",
                };
                return Err(TypeError::Other(verum_common::Text::from(format!(
                    "duplicate {} '{}' in impl block.\n  \
                     note: the same name cannot be defined more than once \
                     within a single `implement` block — the second \
                     definition would silently shadow the first.\n  \
                     help: rename one of the definitions, or merge their \
                     bodies if both were meant to express the same operation",
                    kind, name
                ))));
            }
        }
        Ok(())
    }

    /// Register method signatures from an `implement Type { ... }` (inherent) block.
    /// Processes both static and instance methods, resolves Self type, and wires
    /// per-instantiation method-gating patterns.
    fn register_inherent_impl_methods(
        &mut self,
        impl_decl: &verum_ast::decl::ImplDecl,
        type_param_names: &List<verum_common::Text>,
        conditions: &[crate::protocol::WhereClause],
        callables: &Map<TypeVar, Type>,
    ) -> Result<()> {
        use verum_ast::decl::{FunctionParamKind, ImplItemKind, ImplKind};
        use verum_common::Text;
        let inherit_domain = |scheme: &mut crate::context::TypeScheme| {
            scheme.where_clauses.extend(conditions.iter().cloned());
            for (variable, bound) in callables {
                scheme.var_type_bounds.entry(*variable).or_default().push(bound.clone());
            }
        };
        self.check_no_duplicate_impl_items(impl_decl)?;
        let ImplKind::Inherent(for_type) = &impl_decl.kind
            else { unreachable!() };
        let type_param_names = type_param_names.clone();
        // Preserve the impl's identities before a method shadows their names.
        let impl_bindings: List<(Text, Type)> = type_param_names
            .iter()
            .filter_map(|name| {
                self.ctx.lookup_type(name.as_str()).cloned()
                    .map(|ty| (name.clone(), ty))
            })
            .collect();

                // CRITICAL FIX: Set current_self_type FIRST so Self types in method signatures resolve correctly
                // This enables patterns like `implement TypeName { fn new() -> Self { ... } }`
                // Inherent methods attach to the same declaration identity as
                // protocol implementations, including sums and transparent aliases.
                let self_type = self.ast_to_type_for_protocol_impl(for_type)?;
                let previous_self_type = self.current_self_type.clone();
                // Capture the impl's `for_type` argument list for
                // per-instantiation method gating. For
                // `implement<T: Copy> Register<T, ReadOnly>` this is
                // `[Var(T), Named(ReadOnly)]` — Var slots match any
                // concrete arg at lookup time; Named slots pin the
                // receiver's arg. Same encoding as the cross-file
                // `import_impl_blocks_for_type` path so both stdlib
                // and user impls populate the same registry.
                let attachment = Self::implementation_attachment_type(&self_type);
                let impl_self_type_args: List<Type> = match attachment {
                    Type::Named { args, .. } | Type::Generic { args, .. } => args.clone(),
                    Type::Array { element, .. } | Type::Slice { element } => {
                        std::iter::once((**element).clone()).collect()
                    }
                    _ => List::new(),
                };
                self.set_current_self_type(Maybe::Some(self_type.clone()));

                // Get the type name for registering qualified method names
                let type_name = match attachment {
                    Type::Named { path, .. } => Some(self.path_to_string(path)),
                    Type::Generic { name, .. } => Some(name.clone()),
                    _ => None,
                }
                .or_else(|| match &for_type.kind {
                    verum_ast::ty::TypeKind::Path(path) => {
                        path.as_ident().map(|id| id.name.clone())
                    }
                    verum_ast::ty::TypeKind::Generic { base, .. } => {
                        // For generic types like List<T>, extract the base type name
                        match &base.kind {
                            verum_ast::ty::TypeKind::Path(path) => {
                                path.as_ident().map(|id| id.name.clone())
                            }
                            _ => None,
                        }
                    }
                    // CRITICAL: Handle primitive types for inherent impls
                    // e.g., `implement Int { fn max_value() -> Int { ... } }`
                    verum_ast::ty::TypeKind::Int => Some(WKT::Int.as_str().into()),
                    verum_ast::ty::TypeKind::Float => Some(WKT::Float.as_str().into()),
                    verum_ast::ty::TypeKind::Bool => Some(WKT::Bool.as_str().into()),
                    verum_ast::ty::TypeKind::Char => Some(WKT::Char.as_str().into()),
                    verum_ast::ty::TypeKind::Text => Some(WKT::Text.as_str().into()),
                    verum_ast::ty::TypeKind::Unit => Some("Unit".into()),
                    // CRITICAL: Handle slice and array types for inherent impls
                    // e.g., `implement<T> [T] { fn len(&self) -> Int { ... } }`
                    verum_ast::ty::TypeKind::Slice(_) => Some("Slice".into()),
                    verum_ast::ty::TypeKind::Array { .. } => Some("Array".into()),
                    _ => None,
                });

                // Register all method signatures
                if let Some(type_name_str) = type_name {
                    for item in &impl_decl.items {
                        if let ImplItemKind::Function(func) = &item.kind {
                            // Wrap method registration in error recovery so that a single
                            // method failing to resolve types doesn't prevent remaining methods
                            // in the impl block from being registered.
                            let previous_method_types: Vec<_> = func.generics.iter().filter_map(|parameter| {
                                let verum_ast::ty::GenericParamKind::Type { name, .. } = &parameter.kind else {
                                    return None;
                                };
                                Some((name.name.clone(), self.ctx.type_defs.get(&name.name).cloned()))
                            }).collect();
                            let assumption_depth = self.source_type_registration.projection_assumptions.len();
                            let callable_depth = self.source_type_registration.callable_assumptions.len();
                            let method_result: Result<()> = self.with_declared_callable_scope(func, |checker| {
                                // Check if this is a static method (no self parameter)
                                // `is_self` is the authority on what counts as a
                                // receiver. Re-listing the variants here left out
                                // the reference tiers (`&checked self`,
                                // `&unsafe self`), so a method declared with one
                                // was registered as an ASSOCIATED FUNCTION: it
                                // answered to `T.m(&v)` and was invisible to
                                // `v.m()`, which is how every call site spells it.
                                let is_static = !func.params.first().is_some_and(|p| p.is_self());

                                // Track self-by-value methods for affine type consumption
                                if !is_static {
                                    let takes_by_value = func
                                        .params
                                        .first()
                                        .map(|p| {
                                            matches!(
                                                p.kind,
                                                FunctionParamKind::SelfValue
                                                    | FunctionParamKind::SelfValueMut
                                                    | FunctionParamKind::SelfOwn
                                                    | FunctionParamKind::SelfOwnMut
                                            )
                                        })
                                        .unwrap_or(false);
                                    if takes_by_value {
                                        checker.self_by_value_methods.insert((
                                            verum_common::Text::from(type_name_str.as_str()),
                                            verum_common::Text::from(func.name.name.as_str()),
                                        ));
                                    }
                                }

                                // Allocate every method binder before resolving bounds which
                                // may refer to a later parameter or shadow an impl binder.
                                let mut method_type_vars = List::new();
                                for parameter in &func.generics {
                                    if let verum_ast::ty::GenericParamKind::Type { name, .. } = &parameter.kind {
                                        let variable = TypeVar::fresh();
                                        checker.ctx.define_type(name.name.clone(), Type::Var(variable));
                                        checker.decl_param_frames.last_mut().expect("method declaration frame")
                                            .insert(name.name.clone(), Type::Var(variable));
                                        method_type_vars.push(variable);
                                    }
                                }

                                let mut protocol_bounds = Map::new();
                                let mut type_bounds = Map::new();
                                for parameter in &func.generics {
                                    let verum_ast::ty::GenericParamKind::Type { name, bounds, .. } = &parameter.kind else {
                                        continue;
                                    };
                                    let Some(Type::Var(variable)) = checker.lookup_decl_param(name.name.as_str()).cloned() else {
                                        continue;
                                    };
                                    let protocols = checker.convert_type_bounds_for_receiver(bounds, Some(&Type::Var(variable)))?;
                                    if !protocols.is_empty() {
                                        protocol_bounds.insert(variable, protocols);
                                    }
                                    let types = checker.extract_type_bounds_from_ast(bounds);
                                    if !types.is_empty() {
                                        type_bounds.insert(variable, types);
                                    }
                                }
                                let method_clauses = checker.collect_where_clause_bounds(func, &mut protocol_bounds, &mut type_bounds)?;
                                checker.extend_source_parameter_premises(&protocol_bounds, &type_bounds);
                                checker.source_type_registration.projection_assumptions.extend(method_clauses.iter().cloned());
                                let publish_domain = |scheme: &mut crate::context::TypeScheme| {
                                    scheme.var_protocol_bounds = protocol_bounds;
                                    scheme.var_type_bounds = type_bounds;
                                    scheme.where_clauses.extend(method_clauses);
                                    inherit_domain(scheme);
                                };

                                if is_static {
                                    // Register static method
                                    // Signature admission preserves every declared value-domain obligation.
                                    let param_types: List<Type> = func
                                        .params
                                        .iter()
                                        .filter_map(|p| match &p.kind {
                                            FunctionParamKind::Regular { ty, .. } => Some(ty),
                                            _ => None,
                                        })
                                        .map(|ty| checker.declared_type(ty))
                                        .collect::<Result<List<Type>>>()?;

                                    let return_type = func
                                        .return_type
                                        .as_ref()
                                        .map(|t| checker.declared_type(t))
                                        .transpose()?
                                        .unwrap_or(Type::Unit);

                                    // CRITICAL FIX: Wrap return type in Future<T> for async static methods
                                    // (and Generator<Y, Unit> for `fn*` static methods, plus
                                    // Future<Generator<Y, Unit>> for `async fn*`).
                                    // This enables proper await/for-await type checking for
                                    // Connection.connect().await AND Connection.events_stream().
                                    let final_return_type = checker.wrap_return_type_for_sig_full(
                                        return_type,
                                        &func.throws_clause,
                                        func.is_async,
                                        func.is_generator,
                                    );

                                    let func_ty = Type::function_with_requirement(param_types, final_return_type, checker.source_callable_requirement(func));
                                    let qualified_name =
                                        format!("{}.{}", type_name_str, func.name.name);

                                    // Preserve declaration identities before method parameters
                                    // can shadow the impl's spelling. Receiver substitution
                                    // follows the declared pattern, not positional return slots.
                                    let mut ordered_vars: List<TypeVar> = impl_bindings
                                        .iter()
                                        .filter_map(|(_, ty)| match ty {
                                            Type::Var(var) => Some(*var),
                                            _ => None,
                                        })
                                        .collect();
                                    ordered_vars.extend(method_type_vars);
                                    let mut func_scheme =
                                        checker.ctx.generalize_with_vars(func_ty, &ordered_vars)
                                            .with_universe_parameters(checker.current_universe_parameters())
                                            .map_err(|message| TypeError::UniverseBinding { message, span: func.span })?;
                                    func_scheme.impl_self_type = Some(self_type.clone());
                                    checker.record_callable_declaration(&mut func_scheme, func);
                                    // Record how many ordered vars are impl-level so method-level params
                                    // (e.g., method's own <U>) aren't bound from receiver type args at call time.
                                    func_scheme.impl_var_count = type_param_names.len();
                                    if std::env::var("VERUM_TRACE_METHOD_LOOKUP").is_ok() { eprintln!("[implvc-set] site=decls.rs:5179 count={}", type_param_names.len()); }

                                    publish_domain(&mut func_scheme);


                                    checker.ctx
                                        .env
                                        .insert(qualified_name.as_str(), func_scheme.clone());

                                    // Also register in inherent_methods for cross-pass visibility
                                    // Use "$static$" prefix to distinguish from instance methods
                                    let static_key = verum_common::Text::from(format!(
                                        "$static${}",
                                        func.name.name.as_str()
                                    ));
                                    let type_name_text =
                                        verum_common::Text::from(type_name_str.as_str());
                                    checker.record_method_publication(&type_name_text, &static_key, &func_scheme);
                                    {
                                        let mut methods = checker.inherent_methods.write();
                                        let type_methods =
                                            methods.entry(type_name_text).or_default();
                                        type_methods.insert(static_key, func_scheme);
                                    }

                                    tracing::debug!(
                                        "Registered static method signature: {}",
                                        qualified_name
                                    );
                                } else {
                                    // Register instance method
                                    // Signature admission preserves every declared value-domain obligation.
                                    let param_types: List<Type> = func
                                        .params
                                        .iter()
                                        .filter(|p| !p.is_self())
                                        .filter_map(|p| match &p.kind {
                                            FunctionParamKind::Regular { ty, .. } => Some(ty),
                                            _ => None,
                                        })
                                        .map(|ty| checker.declared_type(ty))
                                        .collect::<Result<List<Type>>>()?;

                                    let return_type = func
                                        .return_type
                                        .as_ref()
                                        .map(|t| checker.declared_type(t))
                                        .transpose()?
                                        .unwrap_or(Type::Unit);

                                    // CRITICAL FIX: Wrap return type in Future<T> for async methods
                                    // (and Generator<Y, Unit> for `fn*` methods, plus
                                    // Future<Generator<Y, Unit>> for `async fn*`).
                                    // This enables proper await/for-await type checking for
                                    // inherent impl methods.
                                    let final_return_type = checker.wrap_return_type_for_sig_full(
                                        return_type,
                                        &func.throws_clause,
                                        func.is_async,
                                        func.is_generator,
                                    );

                                    let method_ty = Type::function_with_requirement(param_types, final_return_type, checker.source_callable_requirement(func));
                                    let type_name_text =
                                        verum_common::Text::from(type_name_str.as_str());
                                    let method_name_text =
                                        verum_common::Text::from(func.name.name.as_str());

                                    // Build ordered TypeVar list for generalisation.
                                    //

                                    // CRITICAL correctness rule: scheme.vars must be
                                    // ordered so that the FIRST N entries correspond
                                    // positionally to the receiver type's type
                                    // arguments, where N = number of impl-level
                                    // TypeVars that appear in `for_type`.
                                    //

                                    // Concretely: for
                                    //  implement<I: Iterator, B, F: fn(I.Item) -> B>
                                    //  Iterator for MappedIter<I, F>
                                    // declaration order is [I, B, F] but `for_type`
                                    // is `MappedIter<I, F>` — so the positional
                                    // arguments at call time are [I_arg, F_arg],
                                    // length 2. If we keep declaration order in
                                    // scheme.vars, a bind_limit of 2 binds I and B
                                    // — with B=F_arg (the closure type), which then
                                    // taints the return type `Maybe<B>` into
                                    // `Maybe<fn(Int)->Int>` at `.next()` call sites.
                                    //

                                    // Reorder: impl TypeVars present in `for_type`
                                    // go first (in declaration order), then
                                    // impl TypeVars NOT in `for_type` (e.g. B),
                                    // then method-level TypeVars. Everyone stays in
                                    // a predictable slot, and bind_limit stays
                                    // aligned with `receiver.args.len()`.
                                    let for_type_free: std::collections::HashSet<TypeVar> =
                                        self_type.free_vars().iter().copied().collect();

                                    let mut ordered_vars: List<TypeVar> = List::new();
                                    let mut impl_vars_in_for_type: Vec<TypeVar> = Vec::new();
                                    let mut impl_vars_outside: Vec<TypeVar> = Vec::new();
                                    for name in type_param_names.iter() {
                                        if let Option::Some(Type::Var(v)) =
                                            impl_bindings.iter().find(|(n, _)| n == name).map(|(_, ty)| ty)
                                        {
                                            if for_type_free.contains(v) {
                                                impl_vars_in_for_type.push(*v);
                                            } else {
                                                impl_vars_outside.push(*v);
                                            }
                                        }
                                    }
                                    for v in &impl_vars_in_for_type {
                                        ordered_vars.push(*v);
                                    }
                                    for v in &impl_vars_outside {
                                        ordered_vars.push(*v);
                                    }
                                    ordered_vars.extend(method_type_vars);

                                    let mut method_scheme =
                                        checker.ctx.generalize_with_vars(method_ty, &ordered_vars)
                                            .with_universe_parameters(checker.current_universe_parameters())
                                            .map_err(|message| TypeError::UniverseBinding { message, span: func.span })?;
                                    // impl_var_count is the number of impl-level
                                    // vars IN for_type, i.e. the first block.
                                    // This is exactly `receiver.args.len()` at call
                                    // time, so bind_limit = impl_var_count binds
                                    // receiver args to their correct positional
                                    // TypeVars and leaves impl_vars_outside free
                                    // (to be inferred from bounds / unification).
                                    method_scheme.receiver_kind =
                                        Some(crate::core_metadata::ReceiverKind::from_declaration(func));
                                    method_scheme.impl_var_count = impl_vars_in_for_type.len();
                                    method_scheme.impl_self_type = Some(self_type.clone());
                                    checker.record_callable_declaration(&mut method_scheme, func);
                                    if std::env::var("VERUM_TRACE_METHOD_LOOKUP").is_ok() { eprintln!("[implvc-set] site=decls.rs:5370 count={}", impl_vars_in_for_type.len()); }

                                    // Register in inherent_methods map — guarded ONE
                                    // write-path (T0585 ERROR-14); inherent
                                    // registrations keep last-write-wins and may
                                    // reclaim a protocol-poisoned slot.
                                    publish_domain(&mut method_scheme);
                                    checker.register_inherent_method_guarded(
                                        type_name_text.clone(),
                                        method_name_text.clone(),
                                        method_scheme,
                                        false,
                                    );

                                    // Per-instantiation gate: record the impl
                                    // block's `for_type` arg pattern so the
                                    // method-call lookup paths that *do* check
                                    // patterns can reject calls whose receiver
                                    // pinned a generic arg to a different
                                    // concrete type. Mirrors the same record
                                    // already done in `import_impl_blocks_for_type`
                                    // (cross-file path) — without this the
                                    // in-module path leaves patterns empty for
                                    // stdlib types like `Register<T, MODE>`.
                                    //

                                    // KNOWN GAP: not every method-call
                                    // resolution path consults
                                    // `method_impl_patterns`; e.g.,
                                    // `lookup_protocol_method_for_type` and the
                                    // base-name-fallback path both bypass it.
                                    // The patterns are populated correctly here;
                                    // closing the remaining gates requires
                                    // refactoring `inherent_methods` to a
                                    // multimap keyed on (type_name, method_name)
                                    // with the constraint stored alongside each
                                    // candidate signature. Tracked as task #35.
                                    if !impl_self_type_args.is_empty() {
                                        let mut patterns_guard = checker.method_impl_patterns.write();
                                        let type_patterns = patterns_guard
                                            .entry(type_name_text.clone())
                                            .or_default();
                                        let method_patterns = type_patterns
                                            .entry(method_name_text.clone())
                                            .or_default();
                                        method_patterns.push(impl_self_type_args.clone());
                                    }

                                    tracing::debug!(
                                        "Registered instance method signature: {}.{}",
                                        type_name_str,
                                        func.name.name
                                    );
                                }
                                Ok(())
                            });
                            self.source_type_registration.projection_assumptions.truncate(assumption_depth);
                            self.source_type_registration.callable_assumptions.truncate(callable_depth);
                            for (name, previous) in previous_method_types {
                                if let Some(ty) = previous {
                                    self.ctx.define_type(name, ty);
                                } else {
                                    self.ctx.remove_type(&name);
                                }
                            }
                            for (name, ty) in &impl_bindings {
                                self.ctx.define_type(name.clone(), ty.clone());
                            }

                            if let Err(error) = method_result {
                                self.deferred_soundness_errors.push(error);
                            }
                        }

                        // Handle associated constants
                        // All constants are registered within the same file regardless of visibility.
                        // Visibility (public/private) controls cross-file access, not same-file access.
                        // This matches how methods work: you can always access your own impl items.
                        if let ImplItemKind::Const { name, ty, .. } = &item.kind {
                            // Build the constant type
                            let const_type = match self.ast_to_type(ty) {
                                Ok(ty) => ty,
                                Err(error) => {
                                    self.deferred_soundness_errors.push(error);
                                    continue;
                                }
                            };
                            let qualified_name = format!("{}.{}", type_name_str, name.name);

                            // Generalize with impl type parameters if any
                            let mut const_scheme = if type_param_names.is_empty() {
                                TypeScheme::mono(const_type)
                            } else {
                                self.ctx.generalize_ordered(const_type, &type_param_names)
                            };
                            inherit_domain(&mut const_scheme);
                            const_scheme.value_declaration = Some(verum_common::Shared::new(crate::context::ValueDeclaration {
                                name: name.name.clone(), owner: self.current_module_path.clone(),
                                visibility: item.visibility.clone(),
                            }));

                            // Register in environment with qualified name
                            self.ctx.env.insert_value(qualified_name.as_str(), const_scheme);

                            tracing::debug!("Registered associated constant: {}", qualified_name);
                        }
                    }
                }

                // Restore previous self type
                self.set_current_self_type(previous_self_type);
        Ok(())
    }

    /// Register method signatures from a `implement Protocol for Type { ... }` block.
    /// Processes both instance and static methods, registers blanket impls,
    /// inherits default methods from the protocol, and commits the ProtocolImpl.
    fn register_protocol_impl_methods(
        &mut self,
        impl_decl: &verum_ast::decl::ImplDecl,
        type_param_names: &List<verum_common::Text>,
        owner_frame: &indexmap::IndexMap<Text, Type>,
        impl_where_clauses: List<crate::protocol::WhereClause>,
        type_param_fn_bounds: Map<TypeVar, Type>,
        expected: Option<(&Text, &Type)>,
    ) -> Result<()> {
        use verum_ast::decl::{FunctionParamKind, ImplItemKind, ImplKind};
        use verum_common::Text;
        self.check_no_duplicate_impl_items(impl_decl)?;
        let ImplKind::Protocol { protocol, protocol_args, for_type } = &impl_decl.kind
            else { unreachable!() };
        let protocol_identity = self.resolve_protocol_conformance_identity(protocol)?;
        let specialization = Self::source_impl_specialization(impl_decl)?;
        let type_param_names = type_param_names.clone();
                // CRITICAL FIX: Register protocol implementation methods BEFORE type-checking
                // This enables method calls like `x.cmp(y)` where cmp is defined in
                // `implement Ord for Int { fn cmp(&self, other: &Int) -> Ordering { ... } }`

                // Extract type name - handles both path types and primitive types
                let resolved_self_type = self.ast_to_type(for_type)?;
                let resolved_protocol_args = self.protocol_application_arguments(
                    &verum_ast::Path { span: protocol.span, ..Self::text_to_path(&protocol_identity) },
                    protocol_args, Some(&resolved_self_type),
                )?;
                let type_name_opt = match Self::implementation_attachment_type(&resolved_self_type) {
                    Type::Named { path, .. } => Some(self.path_to_string(path)),
                    Type::Generic { name, .. } => Some(name.clone()),
                    ty => ty.primitive_name()
                        .filter(|_| !matches!(ty, Type::Unknown)).map(Text::from),
                }
                .or_else(|| match &for_type.kind {
                    verum_ast::ty::TypeKind::Path(path) => {
                        path.as_ident().map(|id| id.name.clone())
                    }
                    verum_ast::ty::TypeKind::Generic { base, .. } => match &base.kind {
                        verum_ast::ty::TypeKind::Path(path) => {
                            path.as_ident().map(|id| id.name.clone())
                        }
                        _ => None,
                    },
                    // CRITICAL: Handle primitive types for protocol implementations
                    // e.g., `implement Hash for Int { ... }`
                    verum_ast::ty::TypeKind::Int => Some(WKT::Int.as_str().into()),
                    verum_ast::ty::TypeKind::Float => Some(WKT::Float.as_str().into()),
                    verum_ast::ty::TypeKind::Bool => Some(WKT::Bool.as_str().into()),
                    verum_ast::ty::TypeKind::Char => Some(WKT::Char.as_str().into()),
                    verum_ast::ty::TypeKind::Text => Some(WKT::Text.as_str().into()),
                    verum_ast::ty::TypeKind::Unit => Some("Unit".into()),
                    _ => None,
                });

                // Extract protocol name for debug logging
                let protocol_name = protocol
                    .as_ident()
                    .map(|id| id.name.as_str())
                    .unwrap_or("?");

                if let Some(type_name_str) = type_name_opt {
                    // COERCION MARKERS FROM USER CODE (T0725).
                    //
                    // `core/base/coercion.vr` declares these as PUBLIC
                    // protocols, which invites a user to implement them —
                    // and until now the implementation compiled and did
                    // nothing: the unifier learned markers only from
                    // `register_coercion_markers_from_metadata`, which walks
                    // the STANDARD LIBRARY's metadata. A user's own
                    // `implement IntCoercible for Fd {}` never reached it, so
                    // `takes_int(Fd(7))` still failed E400 while the same
                    // call with a stdlib `Duration` compiled.
                    //
                    // Registered here, where the impl's protocol name and
                    // target type are both already in hand, with the SAME
                    // dispatch the stdlib path uses — one behaviour, two
                    // entry points, rather than a second notion of what a
                    // marker means.
                    {
                        let target: Text = type_name_str.as_str().into();
                        match protocol_name {
                            "IntCoercible" => {
                                self.unifier.register_int_coercible_type(target)
                            }
                            "TensorLike" => {
                                self.unifier.register_tensor_family_type(target)
                            }
                            "RangeLike" => self.unifier.register_range_like_type(target),
                            "BytewiseFfi" => {
                                self.unifier.register_bytewise_ffi_type(target)
                            }
                            "SizedNumeric" => {
                                self.unifier.register_sized_numeric_type(target)
                            }
                            "ArrayCoercible" => {
                                self.unifier.register_array_coercible_type(target.clone());
                                crate::subtype::register_global_array_coercible(target);
                            }
                            _ => {}
                        }
                    }
                    // Set current_self_type for Self resolution in method signatures
                    let self_type = resolved_self_type;
                    let previous_self_type = self.current_self_type.clone();
                    self.set_current_self_type(Maybe::Some(self_type));
                    let preparation = (|| -> Result<()> {


                    for item in &impl_decl.items {
                        if let ImplItemKind::Function(func) = &item.kind {
                            // Check if this is a static method (no self parameter)
                            // Same authority as the sibling site above — see the
                            // note there on why re-listing the variants dropped
                            // the tiered receivers.
                            let is_static = !func.params.first().is_some_and(|p| p.is_self());

                            if !is_static {
                                // Track self-by-value methods for affine type consumption
                                let takes_by_value = func
                                    .params
                                    .first()
                                    .map(|p| {
                                        matches!(
                                            p.kind,
                                            FunctionParamKind::SelfValue
                                                | FunctionParamKind::SelfValueMut
                                                | FunctionParamKind::SelfOwn
                                                | FunctionParamKind::SelfOwnMut
                                        )
                                    })
                                    .unwrap_or(false);
                                if takes_by_value {
                                    self.self_by_value_methods.insert((
                                        verum_common::Text::from(type_name_str.as_str()),
                                        verum_common::Text::from(func.name.name.as_str()),
                                    ));
                                }

                                // Register method-level generic type parameters
                                let mut method_type_param_names = List::new();
                                let mut method_type_var_bounds: Map<TypeVar, List<Type>> =
                                    Map::new();
                                for generic_param in &func.generics {
                                    use verum_ast::ty::GenericParamKind;
                                    if let GenericParamKind::Type { name, bounds, .. } =
                                        &generic_param.kind
                                    {
                                        let fresh_var = TypeVar::fresh();
                                        let type_var = Type::Var(fresh_var);
                                        let name_text: Text = name.name.clone();
                                        self.ctx.define_type(name_text.clone(), type_var);
                                        method_type_param_names.push(name_text);

                                        // Extract type bounds
                                        if !bounds.is_empty() {
                                            let extracted_bounds =
                                                self.extract_type_bounds_from_ast(bounds);
                                            if !extracted_bounds.is_empty() {
                                                method_type_var_bounds
                                                    .insert(fresh_var, extracted_bounds);
                                            }
                                        }
                                    }
                                }

                                // Build method parameter types (excluding self)
                                let param_types: Result<List<Type>> = func
                                    .params
                                    .iter()
                                    .filter(|p| !p.is_self())
                                    .filter_map(|p| match &p.kind {
                                        FunctionParamKind::Regular { ty, .. } => Some(ty),
                                        _ => None,
                                    })
                                    .map(|ty| self.ast_to_type(ty))
                                    .collect();
                                let param_types = param_types?;

                                let return_type = func
                                    .return_type
                                    .as_ref()
                                    .map(|t| self.ast_to_type(t))
                                    .unwrap_or(Ok(Type::Unit))?;

                                // SHELL-5a — protocol-impl instance
                                // methods need the SAME throws →
                                // generator → async wrap order as
                                // every other function-decl path so
                                // an `async fn* m() -> Y` lands as
                                // `Future<Generator<Y, Unit>>` (not
                                // `Future<Y>` or raw `Y`).
                                let final_return_type = self.wrap_return_type_for_sig_full(
                                    return_type,
                                    &func.throws_clause,
                                    func.is_async,
                                    func.is_generator,
                                );

                                let method_ty = Type::function_with_requirement(param_types, final_return_type, self.source_callable_requirement(func));
                                let type_name_text =
                                    verum_common::Text::from(type_name_str.as_str());
                                let method_name_text =
                                    verum_common::Text::from(func.name.name.as_str());

                                // Resolve impl TypeVars against `for_type`'s
                                // free vars so the first block of scheme.vars
                                // positionally corresponds to the receiver's
                                // type arguments (see extended rationale in
                                // the Inherent branch — same rule for
                                // protocol-impl methods).
                                let self_type_for_reorder =
                                    self.ast_to_type(for_type).unwrap_or(Type::Unit);
                                let for_type_free: std::collections::HashSet<TypeVar> =
                                    self_type_for_reorder.free_vars().iter().copied().collect();

                                let mut ordered_vars: List<TypeVar> = List::new();
                                let mut impl_vars_in_for_type: Vec<TypeVar> = Vec::new();
                                let mut impl_vars_outside: Vec<TypeVar> = Vec::new();
                                for name in type_param_names.iter() {
                                    if let Option::Some(Type::Var(v)) =
                                        self.ctx.lookup_type(name.as_str())
                                    {
                                        if for_type_free.contains(v) {
                                            impl_vars_in_for_type.push(*v);
                                        } else {
                                            impl_vars_outside.push(*v);
                                        }
                                    }
                                }
                                for v in &impl_vars_in_for_type {
                                    ordered_vars.push(*v);
                                }
                                for v in &impl_vars_outside {
                                    ordered_vars.push(*v);
                                }
                                for name in method_type_param_names.iter() {
                                    if let Option::Some(Type::Var(v)) =
                                        self.ctx.lookup_type(name.as_str())
                                    {
                                        ordered_vars.push(*v);
                                    }
                                }

                                let mut method_scheme =
                                    self.ctx.generalize_with_vars(method_ty, &ordered_vars);
                                method_scheme.receiver_kind =
                                    Some(crate::core_metadata::ReceiverKind::from_declaration(func));
                                method_scheme.impl_var_count = impl_vars_in_for_type.len();
                                method_scheme.impl_self_type = self.current_self_type.clone();
                                self.record_callable_declaration(&mut method_scheme, func);
                                if let Some(declaration) = self.protocol_checker.read().get_protocol(&protocol_identity).and_then(|protocol| protocol.methods.get(&func.name.name)) {
                                    self.attach_protocol_method_requirements(&mut method_scheme, declaration, &resolved_protocol_args, self.current_self_type.as_ref().ok_or_else(|| TypeError::Other("protocol implementation receiver scope is missing".into()))?, func.span)?;
                                }
                                if std::env::var("VERUM_TRACE_METHOD_LOOKUP").is_ok() { eprintln!("[implvc-set] site=decls.rs:5671 count={}", impl_vars_in_for_type.len()); }

                                // Add type bounds for closure type inference
                                if !method_type_var_bounds.is_empty() {
                                    method_scheme =
                                        method_scheme.with_type_bounds(method_type_var_bounds);
                                }

                                // Clean up method-level generic type parameters
                                for param_name in method_type_param_names {
                                    self.ctx.remove_type(&param_name);
                                }

                                // Register in inherent_methods for cross-pass visibility.
                                // Guarded write (T0585 ERROR-14): a second protocol
                                // instantiation's same-name method poisons the
                                // single slot instead of last-write-winning.
                                self.register_inherent_method_guarded(
                                    type_name_text.clone(),
                                    method_name_text.clone(),
                                    method_scheme,
                                    true,
                                );

                                tracing::debug!(
                                    "Registered protocol instance method: {}.{} (from {})",
                                    type_name_str,
                                    func.name.name,
                                    protocol_name
                                );
                            } else {
                                // CRITICAL: Also register STATIC protocol methods like From<T>.from(value)
                                // Register method-level generic type parameters
                                let mut method_type_param_names = List::new();
                                let mut method_type_var_bounds: Map<TypeVar, List<Type>> =
                                    Map::new();
                                for generic_param in &func.generics {
                                    use verum_ast::ty::GenericParamKind;
                                    if let GenericParamKind::Type { name, bounds, .. } =
                                        &generic_param.kind
                                    {
                                        let fresh_var = TypeVar::fresh();
                                        let type_var = Type::Var(fresh_var);
                                        let name_text: Text = name.name.clone();
                                        self.ctx.define_type(name_text.clone(), type_var);
                                        method_type_param_names.push(name_text);

                                        // Extract type bounds
                                        if !bounds.is_empty() {
                                            let extracted_bounds =
                                                self.extract_type_bounds_from_ast(bounds);
                                            if !extracted_bounds.is_empty() {
                                                method_type_var_bounds
                                                    .insert(fresh_var, extracted_bounds);
                                            }
                                        }
                                    }
                                }

                                // Build method parameter types (all params - no self for static)
                                let param_types: Result<List<Type>> = func
                                    .params
                                    .iter()
                                    .filter_map(|p| match &p.kind {
                                        FunctionParamKind::Regular { ty, .. } => Some(ty),
                                        _ => None,
                                    })
                                    .map(|ty| self.ast_to_type(ty))
                                    .collect();
                                let param_types = param_types?;

                                let return_type = func
                                    .return_type
                                    .as_ref()
                                    .map(|t| self.ast_to_type(t))
                                    .unwrap_or(Ok(Type::Unit))?;

                                // SHELL-5a — protocol-impl static
                                // methods get the same throws →
                                // generator → async wrap order.
                                let final_return_type = self.wrap_return_type_for_sig_full(
                                    return_type,
                                    &func.throws_clause,
                                    func.is_async,
                                    func.is_generator,
                                );

                                let method_ty = Type::function_with_requirement(param_types, final_return_type, self.source_callable_requirement(func));
                                let type_name_text =
                                    verum_common::Text::from(type_name_str.as_str());
                                let method_name_text =
                                    verum_common::Text::from(func.name.name.as_str());

                                // Resolve impl TypeVars against `for_type`'s
                                // free vars so the first block of scheme.vars
                                // positionally corresponds to the receiver's
                                // type arguments (see extended rationale in
                                // the Inherent branch — same rule for
                                // protocol-impl methods).
                                let self_type_for_reorder =
                                    self.ast_to_type(for_type).unwrap_or(Type::Unit);
                                let for_type_free: std::collections::HashSet<TypeVar> =
                                    self_type_for_reorder.free_vars().iter().copied().collect();

                                let mut ordered_vars: List<TypeVar> = List::new();
                                let mut impl_vars_in_for_type: Vec<TypeVar> = Vec::new();
                                let mut impl_vars_outside: Vec<TypeVar> = Vec::new();
                                for name in type_param_names.iter() {
                                    if let Option::Some(Type::Var(v)) =
                                        self.ctx.lookup_type(name.as_str())
                                    {
                                        if for_type_free.contains(v) {
                                            impl_vars_in_for_type.push(*v);
                                        } else {
                                            impl_vars_outside.push(*v);
                                        }
                                    }
                                }
                                for v in &impl_vars_in_for_type {
                                    ordered_vars.push(*v);
                                }
                                for v in &impl_vars_outside {
                                    ordered_vars.push(*v);
                                }
                                for name in method_type_param_names.iter() {
                                    if let Option::Some(Type::Var(v)) =
                                        self.ctx.lookup_type(name.as_str())
                                    {
                                        ordered_vars.push(*v);
                                    }
                                }

                                let mut method_scheme =
                                    self.ctx.generalize_with_vars(method_ty, &ordered_vars);
                                method_scheme.receiver_kind =
                                    Some(crate::core_metadata::ReceiverKind::from_declaration(func));
                                method_scheme.impl_var_count = impl_vars_in_for_type.len();
                                method_scheme.impl_self_type = self.current_self_type.clone();
                                self.record_callable_declaration(&mut method_scheme, func);
                                if let Some(declaration) = self.protocol_checker.read().get_protocol(&protocol_identity).and_then(|protocol| protocol.methods.get(&func.name.name)) {
                                    self.attach_protocol_method_requirements(&mut method_scheme, declaration, &resolved_protocol_args, self.current_self_type.as_ref().ok_or_else(|| TypeError::Other("protocol implementation receiver scope is missing".into()))?, func.span)?;
                                }
                                if std::env::var("VERUM_TRACE_METHOD_LOOKUP").is_ok() { eprintln!("[implvc-set] site=decls.rs:5802 count={}", impl_vars_in_for_type.len()); }

                                // Add type bounds for closure type inference
                                if !method_type_var_bounds.is_empty() {
                                    method_scheme =
                                        method_scheme.with_type_bounds(method_type_var_bounds);
                                }

                                // Clean up method-level generic type parameters
                                for param_name in method_type_param_names {
                                    self.ctx.remove_type(&param_name);
                                }

                                // Register in inherent_methods for cross-pass visibility
                                // CRITICAL FIX: Use $static$ prefix to distinguish static methods
                                // from instance methods, matching the lookup code in method resolution.
                                let static_key = verum_common::Text::from(format!(
                                    "$static${}",
                                    func.name.name.as_str()
                                ));
                                self.record_method_publication(&type_name_text, &static_key, &method_scheme);
                                {
                                    let mut methods_guard = self.inherent_methods.write();
                                    let type_methods =
                                        methods_guard.entry(type_name_text.clone()).or_default();
                                    type_methods.insert(static_key, method_scheme);
                                }

                                tracing::debug!(
                                    "Registered protocol STATIC method: {}.{} (from {})",
                                    type_name_str,
                                    func.name.name,
                                    protocol_name
                                );
                            }
                        }
                    }

                    // ═══════════════════════════════════════════════════════════════
                    // CRITICAL FIX: Register protocol DEFAULT methods
                    // When `implement Read for File { fn read(...) }`, the File type
                    // should also get default methods like read_to_string, read_to_end.
                    // ═══════════════════════════════════════════════════════════════

                    // Collect names of methods explicitly defined in this impl
                    let mut defined_methods: Set<Text> = Set::new();
                    for item in &impl_decl.items {
                        if let ImplItemKind::Function(func) = &item.kind {
                            defined_methods.insert(func.name.name.clone());
                        }
                    }

                    // Look up the protocol to get its default methods
                    let protocol_text = protocol_identity.clone();
                    // Get the implementing type for Self substitution BEFORE acquiring the read lock
                    // to avoid borrow checker conflicts
                    let impl_self_type = self
                        .current_self_type
                        .clone()
                        .unwrap_or_else(|| self.ast_to_type(for_type).unwrap_or(Type::Unit));
                    let mut defaults = Vec::new();
                    if let Some(proto) = self.protocol_checker.read().get_protocol(&protocol_text)
                    {
                        for (method_name, proto_method) in &proto.methods {
                            // Only register methods with default implementation that
                            // weren't overridden in the impl
                            if proto_method.has_default && !defined_methods.contains(method_name) {
                                // Capture impl-level TypeVars BEFORE entering the
                                // method scope. Once we register method params
                                // (next few lines) any name that matches an
                                // impl-level param will shadow it in type_defs,
                                // and a later name-based lookup would pick up the
                                // method-level var. We therefore resolve impl
                                // param names to their TypeVars here, while the
                                // scope still contains only impl-level bindings.
                                let impl_type_vars: List<TypeVar> = type_param_names
                                    .iter()
                                    .filter_map(|name| match self.ctx.lookup_type(name.as_str()) {
                                        Option::Some(Type::Var(v)) => Some(*v),
                                        _ => None,
                                    })
                                    .collect();

                                let method_scheme = self.inherited_protocol_method_scheme(
                                    proto_method, &resolved_protocol_args, &impl_self_type, &impl_type_vars,
                                )?;

                                defaults.push((method_name.clone(), method_scheme));
                            }
                        }
                    }

                    for (name, scheme) in defaults {
                        self.register_inherited_method(&type_name_str.as_str().into(), &name, scheme);
                    }

                    // ═══════════════════════════════════════════════════════════════
                    // CRITICAL FIX: Register ProtocolImpl EARLY in register phase
                    // This enables blanket impls like `implement<S: Stream> StreamExt for S {}`
                    // to be available when type-checking method calls that need to find them.
                    // ═══════════════════════════════════════════════════════════════

                    // Collect methods from impl items
                    let mut methods: Map<Text, Type> = Map::new();
                    for item in &impl_decl.items {
                        if let ImplItemKind::Function(func) = &item.kind {
                            let method_name: Text = func.name.name.clone();
                            // Build method type (simplified - excludes self)
                            let param_types: Result<List<Type>> = func
                                .params
                                .iter()
                                .filter(|p| !p.is_self())
                                .filter_map(|p| match &p.kind {
                                    FunctionParamKind::Regular { ty, .. } => Some(ty),
                                    _ => None,
                                })
                                .map(|ty| self.ast_to_type(ty))
                                .collect();

                            if let Ok(params) = param_types {
                                let return_type = func
                                    .return_type
                                    .as_ref()
                                    .map(|t| self.ast_to_type(t))
                                    .unwrap_or(Ok(Type::Unit))
                                    .unwrap_or(Type::Unit);

                                let method_ty = Type::function_with_requirement(params, return_type, self.source_callable_requirement(func));
                                methods.insert(method_name, method_ty);
                            }
                        }
                    }

                    // Add default methods from the protocol
                    // CRITICAL FIX: Substitute Self with the implementing type when inserting default methods.
                    // Protocol default methods like `fn max(self, other: Self) -> Self` need Self replaced
                    // with the actual type (e.g., Int64) so that method lookup returns the correct type.
                    // Without this, calling Int64.max(other) returns Self instead of Int64.
                    let protocol_text = protocol_identity.clone();
                    // Get the implementing type for Self substitution - use current_self_type which was set at line 36161
                    let impl_self_type = self.current_self_type.clone().unwrap_or_else(|| {
                        // Fallback: resolve for_type again (shouldn't normally reach here)
                        self.ast_to_type(for_type).unwrap_or(Type::Unit)
                    });
                    if let Maybe::Some(proto) =
                        self.protocol_checker.read().get_protocol(&protocol_text)
                    {
                        for (method_name, proto_method) in &proto.methods {
                            if proto_method.has_default && !methods.contains_key(method_name) {
                                // Substitute Self with the implementing type
                                let substituted_method_ty =
                                    self.applied_protocol_method(proto_method, &resolved_protocol_args, &impl_self_type)?.ty;
                                methods.insert(method_name.clone(), substituted_method_ty);
                            }
                        }
                    }

                    let associated_types = self.elaborate_impl_associated_assignments(
                        &protocol_identity, impl_decl, owner_frame,
                    )?;

                    // Convert for_type to Type
                    // CRITICAL FIX: Use ast_to_type_lenient to avoid expanding type aliases.
                    // This ensures that `implement<T> FromResidual<Maybe<Never>> for Result<T, E>`
                    // keeps for_type as Named{Result, [T, E]} instead of expanding to the Variant form.
                    // This is necessary for get_implementations() to match lookups correctly.
                    let for_type_resolved = self.checked_impl_receiver(for_type)?;

                    // Create and register ProtocolImpl
                    let protocol_impl = crate::protocol::ProtocolImpl {
                        parameters: owner_frame.values().map(crate::context::GenericBinder::declared)
                            .collect::<std::result::Result<List<_>, _>>()
                            .map_err(|message| TypeError::UniverseBinding { message, span: impl_decl.span })?,
                        protocol: Self::text_to_path(&protocol_identity),
                        protocol_args: resolved_protocol_args,
                        for_type: for_type_resolved,
                        where_clauses: impl_where_clauses,
                        methods: methods.clone(),
                        associated_types,
                        associated_consts: Map::new(),
                        specialization,
                        impl_crate: Maybe::Some(self.current_module_path.clone()),
                        span: impl_decl.span,
                        type_param_fn_bounds: type_param_fn_bounds.clone(),
                    };

                    // DEBUG: Log FromResidual impl registration
                    // let proto_name = protocol_impl.protocol.as_ident().map(|i| i.as_str().to_string()).unwrap_or_default();
                    // if proto_name == "FromResidual" {
                    //  eprintln!("[DEBUG register_impl_block_inner] Registering FromResidual impl for {}, methods: {:?}",
                    //  protocol_impl.for_type, methods.keys().collect::<Vec<_>>());
                    // }
                    // if proto_name == "Into" {
                    //  eprintln!("[DEBUG register_impl] Registering Into impl, for_type: {:?}, where_clauses: {:?}",
                    //  protocol_impl.for_type, protocol_impl.where_clauses);
                    // }

                    if let Some(expected) = expected { self.validate_impl_owner(&protocol_impl, expected)?; }
                    self.check_source_impl_signatures(impl_decl, &protocol_impl, owner_frame)?;
                    let publication = self.protocol_checker.write().register_source_impl(
                        &self.current_module_path, impl_decl, protocol_impl,
                    );
                    publication
                    })();
                    self.set_current_self_type(previous_self_type);
                    preparation?;
                }
        Ok(())
    }

    pub fn pre_register_const(&mut self, const_decl: &verum_ast::decl::ConstDecl) {
        if let Ok(const_ty) = self.ast_to_type(&const_decl.ty) {
            self.ctx
                .env
                .insert_constant(const_decl.name.name.as_str(), TypeScheme::mono(const_ty));
        }
    }

    /// Pre-register a static variable's type for forward reference resolution.
    pub fn ctx_env_insert(&mut self, name: &str, scheme: TypeScheme) {
        self.ctx.env.insert(verum_common::Text::from(name), scheme);
    }
    pub fn ctx_env_lookup(&self, name: &str) -> Option<&TypeScheme> {
        self.ctx.env.lookup(name)
    }
    pub fn ctx_define_type(&mut self, name: &str, ty: Type) {
        self.ctx.define_type(verum_common::Text::from(name), ty);
    }
    pub fn pre_register_static(&mut self, name: &verum_common::Text, ty: Type) {
        self.ctx.env.insert_value(name.as_str(), TypeScheme::mono(ty));
    }

    /// Fold a function's `where` clause into the bound maps a scheme is
    /// built from.
    ///
    /// `where T: P` and `<T: P>` are two spellings of one constraint, and a
    /// call site can only check the one that reached the SCHEME. Until this
    /// existed only the inline spelling did, so `fn f<T: P>(x: &T)` rejected
    /// a non-conforming argument with E405 while `fn f<T>(x: &T) where T: P`
    /// accepted it — and since a protocol with exactly one implementation
    /// dispatches to that implementation, the call returned the OTHER type's
    /// answer rather than failing. A silent wrong value, in the spelling the
    /// language documentation recommends for anything but simple bounds.
    ///
    /// Called from BOTH passes that publish a scheme. Fixing only the
    /// signature pass is inert: the body check republishes the scheme, so a
    /// bound added in one place and not the other is overwritten before any
    /// call site sees it.
    pub(super) fn collect_where_clause_bounds(
        &mut self,
        func: &verum_ast::FunctionDecl,
        protocol_bounds_out: &mut Map<TypeVar, List<crate::protocol::ProtocolBound>>,
        type_bounds_out: &mut Map<TypeVar, List<Type>>,
    ) -> Result<List<crate::protocol::WhereClause>> {
        let mut structural = List::new();
        let Some(ref where_clause) = func.generic_where_clause else {
            return Ok(structural);
        };
        for predicate in &where_clause.predicates {
            use verum_ast::ty::WherePredicateKind;
            let WherePredicateKind::Type { ty, bounds } = &predicate.kind else {
                continue;
            };
            let binder = match &ty.kind {
                verum_ast::ty::TypeKind::Path(path) => match path.segments.as_slice() {
                    [verum_ast::ty::PathSegment::SelfValue] => self.current_self_type.as_ref(),
                    [verum_ast::ty::PathSegment::Name(ident)] => self.lookup_decl_param(ident.name.as_str())
                        .or_else(|| self.ctx.lookup_type(ident.name.as_str())),
                    _ => None,
                },
                _ => None,
            };
            let Some(Type::Var(tvar)) = binder.cloned() else {
                // A condition on a projection or composite has a subject of its
                // own. Retain it in the scheme and prove it after actual arguments bind.
                let subject = self.ast_to_type(ty)?;
                let protocols = self.convert_type_bounds_for_receiver(bounds, Some(&subject))?;
                if !protocols.is_empty() {
                    structural.push(crate::protocol::WhereClause { ty: subject, bounds: protocols });
                }
                continue;
            };
            let protocol_bounds = self.convert_type_bounds_for_receiver(bounds, Some(&Type::Var(tvar)))?;
            if !protocol_bounds.is_empty()
            {
                // MERGE, never replace: `fn f<T: Clone>(…) where T: Ord` is
                // one parameter carrying both, and dropping either half
                // would trade this defect for its mirror image.
                protocol_bounds_out
                    .entry(tvar)
                    .or_default()
                    .extend(protocol_bounds);
            }
            let type_bounds = self.extract_type_bounds_from_ast(bounds);
            if !type_bounds.is_empty() {
                type_bounds_out.entry(tvar).or_default().extend(type_bounds);
            }
        }
        Ok(structural)
    }

    pub fn register_function_signature(&mut self, func: &verum_ast::FunctionDecl) -> Result<()> {
        self.register_function_signature_as(func, &func.name.name)
    }

    /// Register a declaration under a chosen lexical binding without changing
    /// its source owner or universe/generic parameter identities.
    pub(super) fn register_function_signature_as(
        &mut self, func: &verum_ast::FunctionDecl, binding: &Text,
    ) -> Result<()> {
        use verum_ast::decl::FunctionParamKind;
        let scheme = self.source_function_scheme(func)?;
        // ================================================================
        // Record parameter names for dependent refinement enforcement.
        //

        // This populates `function_param_names` with the ordered list of
        // parameter names for this function. The call-site loop in the
        // `Type::Function` arm of the Call handler (around line 10580)
        // consults this map to substitute earlier concrete arguments into
        // subsequent parameters' refinement predicates — enabling truly
        // dependent refinement checking for signatures like
        // `fn safe_get(len: Int, i: Int{>= 0, < len}) -> Int`.
        //

        // Names that can't be extracted (e.g. because the parameter uses
        // a destructuring pattern rather than a simple identifier) are
        // stored as empty `Text` sentinels so the positional layout is
        // preserved; the call-site loop skips substitution for those
        // positions. This preserves backward compatibility: functions
        // whose signatures don't use refinements behave identically.
        // ================================================================
        let mut collected_param_names: List<Text> = List::new();
        for p in func.params.iter() {
            let name_opt: Option<Text> = match &p.kind {
                FunctionParamKind::Regular { pattern, .. } => {
                    if let verum_ast::pattern::PatternKind::Ident { name, .. } = &pattern.kind {
                        Some(name.name.clone())
                    } else {
                        None
                    }
                }
                _ => None,
            };
            collected_param_names.push(name_opt.unwrap_or_else(|| Text::from("")));
        }
        // Preserve the source declaration's scope as well as the legacy
        // bare refinement lookup. Constructor precedence must not borrow a
        // sibling module's same-named declaration as root authority.
        let qualified_function: Text =
            format!("{}.{}", self.current_module_path, func.name.name).into();
        self.function_param_names
            .insert(qualified_function, collected_param_names.clone());
        self.function_param_names
            .insert(binding.clone(), collected_param_names);

        // Phase 2b-Integration (#291) — seed the MLS classification
        // sidecar from per-parameter `@classification(<level>)`
        // attributes. This is the load-bearing wire that lets the
        // unifier consult parameter classifications during
        // synth/check arms. Pre-fix the sidecar was storage-only
        // (#289 Phase 2b-Foundation); now it's seeded from the AST
        // at signature-registration time so downstream let-bindings,
        // call sites, and the existing safety_gate Phase 3a sink
        // detector all see consistent classification state.
        //

        // Architectural note: only `Regular` parameters with `Ident`
        // patterns get sidecar entries — destructuring patterns
        // (Tuple, Record) carry classification at a different
        // granularity that Phase 2b-Integration-Patterns covers
        // separately. Self parameters use the function's own
        // `@classification` (already handled at the safety_gate
        // surface).
        //

        // Phase 2b-Final (#293) also collects a parallel List<MlsLevel>
        // for the function's parameter signature so call sites can
        // enforce the down-flow contract: argument classification
        // must subsume the parameter's required level.
        let mut param_classifications: List<verum_common::mls::MlsLevel> = List::new();
        for p in func.params.iter() {
            let level = match &p.kind {
                FunctionParamKind::Regular { .. } | _ => read_param_classification(&p.attributes),
            };
            param_classifications.push(level);
            if let FunctionParamKind::Regular { pattern, .. } = &p.kind {
                if let verum_ast::pattern::PatternKind::Ident { name, .. } = &pattern.kind {
                    if level != verum_common::mls::MlsLevel::Public {
                        self.classification_map.insert(name.name.clone(), level);
                    }
                }
            }
        }
        // Always populate (empty list when there are no params, or
        // all-Public list when none classified) so call-site
        // lookups by function name produce a consistent shape.
        self.function_param_classifications
            .insert(binding.clone(), param_classifications);

        // Protect builtin generic/meta functions from being DOWNGRADED by stdlib.
        // When a generic builtin (e.g., fn<T>(T) -> T for abs) already exists,
        // don't let a concrete stdlib version (e.g., fn(Float) -> Float) override it.
        // But DO allow equally-or-more-generic re-registrations (e.g., mount core.intrinsics).
        //
        // Both name lists are sourced from canonical constants in `infer::env`
        // so the registration site (`register_meta_builtins` /
        // `register_intrinsics`) and the protection site stay in sync —
        // drift is pinned by the `meta_reflection_builtins_all_registered`
        // and `io_and_numeric_builtins_pinned` tests.
        let func_name = binding.as_str();
        // T0231: the predicate lives in ONE place now —
        // `TypeChecker::insert_fn_scheme_guarded` (env.rs, next to the
        // canonical name lists) — because this registration path was
        // only ONE of the channels that bind fn schemes by name; the
        // prelude import replay (`import_item_from_module_body`)
        // bypassed the inline copy of the rule and clobbered the
        // polymorphic print/println intrinsics.
        // T1120: DECLARATION channel — the unit's own `fn` outranks an
        // ambient builtin (see `insert_fn_scheme_declared`).
        let should_skip_registration = !self.insert_fn_scheme_declared(func_name, scheme);

        // Calculate required params (those without default values)
        // Default params must come after required params, so find first default
        let mut required_params = 0;
        for param in &func.params {
            match &param.kind {
                FunctionParamKind::Regular { default_value, .. } => {
                    if default_value.is_none() {
                        required_params += 1;
                    } else {
                        // Found first default - all remaining are optional
                        break;
                    }
                }
                // Self parameters are always required
                _ => required_params += 1,
            }
        }
        if !should_skip_registration {
            self.function_required_params
                .insert(binding.clone(), required_params);
        }

        tracing::debug!(
            "Registered function signature: {} (required_params: {})",
            func.name.name,
            required_params
        );
        Ok(())
    }

    pub(super) fn source_function_scheme(&mut self, func: &verum_ast::FunctionDecl) -> Result<TypeScheme> {
        self.source_function_scheme_named(func, &func.name.name)
    }

    pub(super) fn source_function_scheme_named(
        &mut self, func: &verum_ast::FunctionDecl, owner: &Text,
    ) -> Result<TypeScheme> {
        // Carry the declaration-owned identities through source-authority lookup.
        // Imported signatures start with an empty frame; ordinary methods may
        // inherit their exact implementation frame. Flat ambient names are not binders.
        let frame = self.decl_param_frames.last().cloned().unwrap_or_default();
        self.push_decl_param_frame(frame);
        let assumption_depth = self.source_type_registration.projection_assumptions.len();
        let callable_depth = self.source_type_registration.callable_assumptions.len();
        let result = self.with_declared_universe_scope("function", owner, &func.generics, |checker| {
            checker.validate_function_universes(func)?;
            checker.function_scheme_in_universe_scope(func)
        });
        self.source_type_registration.projection_assumptions.truncate(assumption_depth);
        self.source_type_registration.callable_assumptions.truncate(callable_depth);
        self.pop_decl_param_frame();
        result
    }

    fn function_scheme_in_universe_scope(&mut self, func: &verum_ast::FunctionDecl) -> Result<TypeScheme> {
        use verum_ast::decl::FunctionParamKind;
        use verum_common::Set;
        // Register generic type parameters FIRST
        // Track both names AND TypeVars for explicit TypeScheme construction
        // This is critical for phantom type parameters (e.g., `fn foo<T>()` where T isn't used)
        // Implicit arguments: compiler-inferred function arguments resolved by unification or type class search
        let mut type_param_names = List::new();
        let mut type_param_vars: List<TypeVar> = List::new();
        let mut implicit_type_vars: Set<TypeVar> = Set::new();
        let mut param_protocol_bounds: Map<TypeVar, List<crate::protocol::ProtocolBound>> =
            Map::new();
        // **Function-type bounds** (`F: fn(A) -> B`-style closure-shape
        // constraints) — collected alongside protocol bounds so the call-site
        // scheme instantiation in `infer_expr_call` can recover the closure
        // shape via `get_function_type_bound`.  Pre-fix these bounds were
        // extracted at the type-parameter scope level but never wired into
        // the scheme; call sites then instantiated through the
        // bound-less `scheme.instantiate()` path, dropping the closure
        // shape — every HOF closure body inferred its `expected_return`
        // from synth_path arity-blind first-wins (the canonical
        // `Backend.None` / `Maybe.None` collision).
        let mut param_type_bounds: Map<TypeVar, List<Type>> = Map::new();

        // Save any existing types that will be shadowed by generic parameters,
        // so we can restore them after processing the function signature.
        // This prevents generic params like `Output` or `Args` from permanently
        // deleting legitimate type definitions from the context.
        let mut saved_types: List<(Text, Option<Type>)> = List::new();

        let result = (|| {
        for generic_param in &func.generics {
            use verum_ast::ty::GenericParamKind;
            match &generic_param.kind {
                GenericParamKind::Type { name, bounds, .. } => {
                    let fresh_var = TypeVar::fresh();
                    let type_var = Type::Var(fresh_var);
                    let name_text: Text = name.name.clone();
                    saved_types
                        .push((name_text.clone(), self.ctx.lookup_type(&name_text).cloned()));
                    self.ctx.define_type(name_text.clone(), type_var.clone());
                    self.decl_param_frames.last_mut().expect("function declaration frame")
                        .insert(name_text.clone(), type_var);
                    type_param_names.push(name_text);
                    type_param_vars.push(fresh_var);

                    // Track if this is an implicit parameter
                    if generic_param.is_implicit {
                        implicit_type_vars.insert(fresh_var);
                    }


                }
                // CRITICAL FIX: Handle HKT type parameters
                // Use a TypeVar for the HKT parameter so it gets instantiated properly
                // When resolving F<A>, we create TypeApp { constructor: Var(τF), args: [...] }
                // This allows unification to bind τF to a concrete constructor like List
                // Higher-kinded types (HKTs): type constructors as first-class entities, kind inference (Type -> Type), HKT instantiation — Higher-kinded types
                GenericParamKind::HigherKinded {
                    name,
                    arity: _,
                    bounds,
                    ..
                } => {
                    let name_text: Text = name.name.clone();

                    // Use a TypeVar so it gets collected by generalize() and replaced during instantiate()
                    let fresh_var = TypeVar::fresh();
                    let type_var = Type::Var(fresh_var);
                    saved_types
                        .push((name_text.clone(), self.ctx.lookup_type(&name_text).cloned()));
                    self.ctx.define_type(name_text.clone(), type_var.clone());
                    self.decl_param_frames.last_mut().expect("function declaration frame")
                        .insert(name_text.clone(), type_var);
                    type_param_names.push(name_text);
                    type_param_vars.push(fresh_var);

                    // Track if this is an implicit parameter
                    if generic_param.is_implicit {
                        implicit_type_vars.insert(fresh_var);
                    }


                }
                GenericParamKind::Meta { name, .. } | GenericParamKind::Const { name, .. } => {
                    let meta_type = self.source_value_parameter(generic_param)?;
                    let name_text = name.name.clone();
                    saved_types.push((name_text.clone(), self.ctx.lookup_type(&name_text).cloned()));
                    self.ctx.define_type(name_text.clone(), meta_type.clone());
                    self.decl_param_frames.last_mut().expect("function declaration frame")
                        .insert(name_text.clone(), meta_type);
                    type_param_names.push(name_text);
                }
                _ => {} // Other generic param kinds (Lifetime, Context) handled elsewhere
            }
        }

        // Allocate every binder before resolving bounds: `<F: P<Item=T>, T>`
        // refers to this declaration's T even when an ambient T exists.
        for parameter in &func.generics {
            use verum_ast::ty::GenericParamKind;
            let (name, bounds) = match &parameter.kind {
                GenericParamKind::Type { name, bounds, .. }
                | GenericParamKind::HigherKinded { name, bounds, .. } => (name, bounds),
                _ => continue,
            };
            let Some(Type::Var(var)) = self.ctx.lookup_type(name.name.as_str()).cloned() else {
                continue;
            };
            let protocol_bounds = self.convert_type_bounds_for_receiver(bounds, Some(&Type::Var(var)))?;
            {
                if !protocol_bounds.is_empty() {
                    param_protocol_bounds.insert(var, protocol_bounds);
                }
            }
            let type_bounds = self.extract_type_bounds_from_ast(bounds);
            if !type_bounds.is_empty() {
                param_type_bounds.insert(var, type_bounds);
            }
        }

        // `where T: P` carries the SAME bound as `<T: P>` and must reach the
        // same place: the scheme this pass publishes, which is what a call
        // site checks its argument against. One collector, called from both
        // passes that build a scheme — see `collect_where_clause_bounds`.
        let structural_conditions = self.collect_where_clause_bounds(
            func,
            &mut param_protocol_bounds,
            &mut param_type_bounds,
        )?;

        self.extend_source_parameter_premises(&param_protocol_bounds, &param_type_bounds);
        self.source_type_registration.projection_assumptions.extend(structural_conditions.iter().cloned());

        // Build parameter types
        let param_types: Result<List<_>> = func
            .params
            .iter()
            .map(|p| match &p.kind {
                FunctionParamKind::Regular { ty, .. } => self.declared_type(ty),
                FunctionParamKind::SelfValue
                | FunctionParamKind::SelfValueMut
                | FunctionParamKind::SelfRef
                | FunctionParamKind::SelfRefMut
                | FunctionParamKind::SelfRefChecked
                | FunctionParamKind::SelfRefCheckedMut
                | FunctionParamKind::SelfRefUnsafe
                | FunctionParamKind::SelfRefUnsafeMut
                | FunctionParamKind::SelfOwn
                | FunctionParamKind::SelfOwnMut => Ok(Type::unit()),
            })
            .collect();
        let param_types = param_types?;

        // Build return type
        let return_type = if let Some(ref ret_ty) = func.return_type {
            self.declared_type(ret_ty)?
        } else {
            // No explicit return type - use a fresh type variable
            // This will be unified with the body type during check_function
            Type::Var(TypeVar::fresh())
        };

        // Unified throws + generator + async wrap via the full helper —
        // ensures `async fn*` decls registered through this path get
        // the same `Future<Generator<Y, Unit>>` shape as decls
        // processed through `infer_function`. Without `is_generator`
        // here, mounted async-generator functions silently lost the
        // Generator wrapper across module boundaries — manifested as
        // "for await requires AsyncIterator … got Future<Y>" at
        // every call site (SHELL-5a regression). Closes that path.
        let return_for_sig = self.wrap_return_type_for_sig_full(
            return_type,
            &func.throws_clause,
            func.is_async,
            func.is_generator,
        );

        // Build context requirement if present
        let func_type = if !func.contexts.is_empty() {
            let contexts_list: List<_> = func.contexts.iter().cloned().collect();
            // T0660: value-typed contexts — learn plain types first.
            self.ensure_using_types_registered(&contexts_list);
            if let Ok(requirement) = self
                .context_resolver
                .resolve_requirement(&contexts_list, func.span)
            {
                Type::function_with_contexts(param_types, return_for_sig, requirement)
            } else {
                Type::function(param_types, return_for_sig)
            }
        } else {
            Type::function(param_types, return_for_sig)
        };

        let mut func_type = func_type;
        if let Type::Function { properties, property_requirement, .. } = &mut func_type {
            *properties = None;
            *property_requirement = self.source_callable_requirement(func);
        }

        // CRITICAL: Create TypeScheme explicitly with tracked type parameters.
        // We cannot rely on `generalize()` which uses `free_vars()` because:
        // - Phantom type parameters (e.g., `fn foo<T: Atomic>()`) don't appear in the function type
        // - Such parameters would be excluded by `free_vars()` but are still valid type parameters
        // Implicit arguments: compiler-inferred function arguments resolved by unification or type class search
        let scheme = if type_param_vars.is_empty() {
            // No type parameters - monomorphic function
            TypeScheme::mono(func_type)
        } else if implicit_type_vars.is_empty() {
            // All type parameters are explicit
            TypeScheme::poly(type_param_vars.clone(), func_type)
        } else {
            // Some parameters are implicit
            TypeScheme::poly_with_implicit(type_param_vars.clone(), func_type, implicit_type_vars)
        };
        // Attach protocol bounds to the scheme so they can be checked at call sites
        let scheme = if param_protocol_bounds.is_empty() {
            scheme
        } else {
            scheme.with_protocol_bounds(param_protocol_bounds)
        };
        // Attach function-type bounds to the scheme so call-site
        // instantiation can register them on the fresh TypeVars (see
        // `infer_expr_call`'s default path).  Mirrors the
        // `with_protocol_bounds` discipline above.
        let scheme = if param_type_bounds.is_empty() {
            scheme
        } else {
            scheme.with_type_bounds(param_type_bounds)
        };
        let mut scheme = scheme.with_universe_parameters(self.current_universe_parameters())
            .map_err(|message| TypeError::UniverseBinding { message, span: func.span })?;
        scheme.where_clauses = structural_conditions;
        scheme.returned_parameter = super::reference_origin::forwarded_parameter(func);
        self.record_callable_declaration(&mut scheme, func);
        Ok(scheme)
        })();

        // Restore previous types instead of removing — this prevents generic params
        // (e.g., `Output`, `Args`) from permanently deleting real type definitions.
        for (name, saved) in saved_types.into_iter().rev() {
            match saved {
                Some(ty) => self.ctx.define_type(name.clone(), ty),
                None => self.ctx.remove_type(&name),
            }
        }

        result
    }

    /// Register an intrinsic function using the intrinsic name (from @intrinsic("name") attribute)
    ///
    /// This is similar to `register_function_signature` but uses the intrinsic name
    /// for registration instead of the function name. This enables the type checker
    /// to automatically extract intrinsic signatures from stdlib rather than requiring
    /// hardcoded registrations in `register_builtins`.
    ///
    /// # Arguments
    /// * `func` - The function declaration with @intrinsic attribute
    /// * `intrinsic_name` - The intrinsic name extracted from @intrinsic("name")
    ///
    /// # Example
    ///
    /// For the stdlib declaration:
    /// ```verum
    /// @intrinsic("memcpy")
    /// public unsafe fn memcpy(dst: *mut Byte, src: *const Byte, len: Int);
    /// ```
    ///
    /// This method will register a TypeScheme for "memcpy" with the proper signature.
    pub fn register_intrinsic_function(
        &mut self,
        func: &verum_ast::FunctionDecl,
        intrinsic_name: &str,
    ) -> Result<()> {
        use verum_ast::decl::FunctionParamKind;
        use verum_common::Set;

        // Register generic type parameters FIRST
        let mut type_param_names = List::new();
        let mut type_param_vars: List<TypeVar> = List::new();
        let mut implicit_type_vars: Set<TypeVar> = Set::new();
        let mut saved_types: List<(Text, Option<Type>)> = List::new();

        for generic_param in &func.generics {
            use verum_ast::ty::GenericParamKind;
            match &generic_param.kind {
                GenericParamKind::Type { name, .. } => {
                    let fresh_var = TypeVar::fresh();
                    let type_var = Type::Var(fresh_var);
                    let name_text: Text = name.name.clone();
                    saved_types
                        .push((name_text.clone(), self.ctx.lookup_type(&name_text).cloned()));
                    self.ctx.define_type(name_text.clone(), type_var);
                    type_param_names.push(name_text);
                    type_param_vars.push(fresh_var);

                    if generic_param.is_implicit {
                        implicit_type_vars.insert(fresh_var);
                    }
                }
                GenericParamKind::HigherKinded { name, .. } => {
                    let name_text: Text = name.name.clone();
                    let fresh_var = TypeVar::fresh();
                    let type_var = Type::Var(fresh_var);
                    saved_types
                        .push((name_text.clone(), self.ctx.lookup_type(&name_text).cloned()));
                    self.ctx.define_type(name_text.clone(), type_var);
                    type_param_names.push(name_text);
                    type_param_vars.push(fresh_var);

                    if generic_param.is_implicit {
                        implicit_type_vars.insert(fresh_var);
                    }
                }
                GenericParamKind::Meta { name, ty, .. } | GenericParamKind::Const { name, ty } => {
                    let fresh_var = TypeVar::fresh();
                    let meta_type = self.source_value_parameter(generic_param)?;
                    let name_text: Text = name.name.clone();
                    saved_types
                        .push((name_text.clone(), self.ctx.lookup_type(&name_text).cloned()));
                    self.ctx.define_type(name_text.clone(), meta_type);
                    type_param_names.push(name_text);
                    type_param_vars.push(fresh_var);

                    if generic_param.is_implicit {
                        implicit_type_vars.insert(fresh_var);
                    }
                }
                _ => {} // Other generic param kinds (Lifetime, Context) handled elsewhere
            }
        }

        // Build parameter types
        let param_types: Result<List<_>> = func
            .params
            .iter()
            .map(|p| match &p.kind {
                FunctionParamKind::Regular { ty, .. } => self.declared_type(ty),
                FunctionParamKind::SelfValue
                | FunctionParamKind::SelfValueMut
                | FunctionParamKind::SelfRef
                | FunctionParamKind::SelfRefMut
                | FunctionParamKind::SelfRefChecked
                | FunctionParamKind::SelfRefCheckedMut
                | FunctionParamKind::SelfRefUnsafe
                | FunctionParamKind::SelfRefUnsafeMut
                | FunctionParamKind::SelfOwn
                | FunctionParamKind::SelfOwnMut => Ok(Type::unit()),
            })
            .collect();
        let param_types = param_types?;

        // Build return type
        let return_type = if let Some(ref ret_ty) = func.return_type {
            self.declared_type(ret_ty)?
        } else {
            // No explicit return type - assume Unit
            Type::Unit
        };

        // One carrier for signature wrapping (throws/generator/async —
        // the async law lives THERE, not in per-site copies).
        let return_for_sig = self.wrap_return_type_for_sig_full(
            return_type,
            &func.throws_clause,
            func.is_async,
            func.is_generator,
        );

        // Build function type (intrinsics don't use context requirements)
        let func_type = Type::function_with_requirement(param_types, return_for_sig, self.source_callable_requirement(func));

        // Create TypeScheme with tracked type parameters
        let scheme = if type_param_vars.is_empty() {
            TypeScheme::mono(func_type)
        } else if implicit_type_vars.is_empty() {
            TypeScheme::poly(type_param_vars.clone(), func_type)
        } else {
            TypeScheme::poly_with_implicit(type_param_vars.clone(), func_type, implicit_type_vars)
        };

        // Register using the INTRINSIC NAME, not the function name
        self.ctx.env.insert_value(intrinsic_name, scheme);

        // Restore previous types instead of removing
        for (name, saved) in saved_types {
            match saved {
                Some(ty) => self.ctx.define_type(name.clone(), ty),
                None => self.ctx.remove_type(&name),
            }
        }

        tracing::debug!(
            "Registered intrinsic: {} (from function {})",
            intrinsic_name,
            func.name.name
        );
        Ok(())
    }

    /// Register a const declaration as a value in the environment
    /// This enables constants like DEFAULT_BUF_CAPACITY to be used in expressions
    pub fn register_const_declaration(
        &mut self,
        const_decl: &verum_ast::decl::ConstDecl,
    ) -> Result<()> {
        // Convert the declared type
        let const_type = self.ast_to_type(&const_decl.ty)?;

        // Register the constant as a monomorphic value
        self.ctx
            .env
            .insert_value(const_decl.name.name.as_str(), TypeScheme::mono(const_type));

        tracing::debug!("Registered const: {}", const_decl.name.name);
        Ok(())
    }

    /// Register a const declaration sourced from `CoreMetadata` rather
    /// than an AST `ConstDecl`.  Mirror of [`register_const_declaration`]
    /// for the archive-driven typechecking path: the precompiled
    /// stdlib carries `public const X: T = N;` declarations as
    /// zero-arg [`FunctionDescriptor`]s with `is_const = true` and
    /// `return_type` as Text; this method parses the type text and
    /// inserts the const into the environment as a monomorphic value
    /// so user code can write `let x = SOME_CONST;` without parens.
    ///
    /// `name` is the bare const name (e.g. `"SSO_CAPACITY"`).
    /// `type_text` is the descriptor text (e.g. `"Int"` or `"Maybe<Int>"`).
    pub fn register_const_from_metadata(&mut self, name: &str, type_text: &str) -> Result<()> {
        let const_type =
            crate::infer::parse_descriptor_type_string(type_text).map_err(TypeError::Other)?;
        self.ctx.env.insert_value(name, TypeScheme::mono(const_type));
        tracing::debug!("Registered const from metadata: {}", name);
        Ok(())
    }

    /// Check an implementation block and register methods
    /// Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — .6 - Protocol implementations
    ///
    pub(super) fn check_impl_block(&mut self, impl_decl: &verum_ast::decl::ImplDecl) -> Result<()> {
        if matches!(impl_decl.kind, verum_ast::decl::ImplKind::Protocol { .. }) {
            self.register_impl_block(impl_decl)?;
        }
        let filtered = self.cfg_filtered_impl(impl_decl);
        let declaration = filtered.as_ref().unwrap_or(impl_decl);
        let depth = self.ctx.env.scope_depth();
        self.ctx.env.push_scope();
        let result = self.with_impl_parameter_scope(declaration, |checker, owner|
            checker.check_impl_block_inner(declaration, owner)
        );
        self.ctx.env.restore_scope_depth(depth);
        result
    }

    /// Inner implementation of check_impl_block
    /// T0742 PROBE (`VERUM_TRACE_IMPL_PARAM=1`): after each method body, print
    /// what every type parameter of the enclosing `implement` block currently
    /// resolves to.
    ///
    /// One block mints ONE var per parameter and shares it across every method
    /// (see the loop below), and the accumulated substitution is never reset
    /// between bodies — so the first body that pins the parameter decides it for
    /// all the rest. This names that body: the line whose `->` stops being the
    /// bare parameter var is the method that pinned it.
    ///
    /// The probe prints on EVERY method, pinned or not, so silence means the
    /// loop was never reached — it cannot be read as "nothing was pinned".
    fn trace_impl_param_pinning(&mut self, owner: &indexmap::IndexMap<Text, Type>, method: &str) {
        if std::env::var_os("VERUM_TRACE_IMPL_PARAM").is_none() {
            return;
        }
        for name in owner.keys() {
            let bound = self.ctx.lookup_type(name.as_str()).cloned();
            let resolved = bound.as_ref().map(|b| self.unifier.apply(b));
            eprintln!(
                "[impl-param] after {method}: {} = {} -> {}",
                name.as_str(),
                bound.map(|b| format!("{b:?}")).unwrap_or_else(|| "<unbound>".into()),
                resolved.map(|r| format!("{r:?}")).unwrap_or_else(|| "<unbound>".into()),
            );
        }
    }

    fn check_impl_block_inner(
        &mut self,
        impl_decl: &verum_ast::decl::ImplDecl,
        owner: &indexmap::IndexMap<Text, Type>,
    ) -> Result<()> {
        use verum_ast::decl::{ImplItemKind, ImplKind};

        // Inline bounds and where clauses can constrain the same parameter.
        // Install all of its proved premises together for body method lookup.
        let (clauses, _) = self.enter_source_impl_conditions(impl_decl, owner)?;
        let mut bounds_by_variable: Map<TypeVar, List<crate::protocol::ProtocolBound>> = Map::new();
        for clause in clauses {
            if let Type::Var(variable) = clause.ty {
                bounds_by_variable.entry(variable).or_default().extend(clause.bounds);
            }
        }
        for (name, ty) in owner {
            if let Type::Var(variable) = ty {
                let bounds = bounds_by_variable.remove(variable).unwrap_or_default();
                let parameter = crate::context::TypeParam::new(name.clone(), impl_decl.span)
                    .with_bounds(bounds.clone());
                self.ctx.env.add_type_param(parameter);
                self.register_type_var_bounds(*variable, bounds);
            }
        }

        match &impl_decl.kind {
            ImplKind::Inherent(for_type) => {
                // Inherent impl: register methods directly on the type
                let ty = self.ast_to_type(for_type)?;

                // Get the type name for registering qualified method names
                let type_name = match &for_type.kind {
                    verum_ast::ty::TypeKind::Path(path) => {
                        path.as_ident().map(|id| id.name.as_str().to_string())
                    }
                    verum_ast::ty::TypeKind::Generic { base, .. } => {
                        // For generic types like List<T>, extract the base type name
                        match &base.kind {
                            verum_ast::ty::TypeKind::Path(path) => {
                                path.as_ident().map(|id| id.name.as_str().to_string())
                            }
                            _ => None,
                        }
                    }
                    _ => None,
                };

                // Set self type for method context
                let previous_self_type = self.current_self_type.clone();
                self.set_current_self_type(Maybe::Some(ty.clone()));

                // Type-check all method bodies
                // Method signatures were already registered in register_impl_block (Pass 3 of pipeline)
                let prev_in_impl = self.in_impl_block;
                self.in_impl_block = true;
                let prev_inherent = self.in_inherent_impl;
                self.in_inherent_impl = true;
                for item in &impl_decl.items {
                    // Skip impl items gated by @cfg that don't match the current platform
                    if !self.cfg_evaluator.should_include(&item.attributes) {
                        continue;
                    }
                    if let ImplItemKind::Function(func) = &item.kind {
                        if let Some(checked) = self.check_function(func)? {
                            self.publish_checked_method_contract(impl_decl, func, &checked)?;
                        }
                        self.trace_impl_param_pinning(owner, &func.name.name);
                        tracing::debug!("Type-checked method {}.{}", ty, func.name.name);
                    }
                }
                self.in_impl_block = prev_in_impl;
                self.in_inherent_impl = prev_inherent;

                // Restore previous self type
                self.set_current_self_type(previous_self_type);
            }
            ImplKind::Protocol {
                protocol,
                for_type,
                ..
            } => {
                // Protocol impl: register in protocol checker
                let ty = self.ast_to_type(for_type)?;
                let protocol_name = self.resolve_protocol_conformance_identity(protocol)?;

                // CRITICAL FIX: Resolve Named types to ensure field access works in method bodies.
                // When `ty` is a Named type like `Location`, we need to ensure the type's fields
                // are accessible. If the type is registered as a Record, use that for self type.
                // This fixes: "Cannot access field 'file' on non-record type: _"
                let resolved_ty = match &ty {
                    Type::Named { path, args } => {
                        // Try to look up the type's record definition
                        if let Some(type_name) = path.as_ident().map(|i| i.name.as_str()) {
                            // First check if there's a struct fields registration
                            let struct_key = format!("__struct_fields_{}", type_name);
                            if self.ctx.lookup_type(&struct_key).is_some() {
                                // Type has fields registered - keep as Named (field access will work)
                                ty.clone()
                            } else if let Maybe::Some(registered_ty) =
                                self.ctx.lookup_type(type_name)
                            {
                                // Use the registered type if it's more concrete
                                match registered_ty {
                                    Type::Record(_) => registered_ty.clone(),
                                    Type::Var(_) => ty.clone(), // Keep Named if registered as TypeVar
                                    _ => ty.clone(),
                                }
                            } else {
                                ty.clone()
                            }
                        } else {
                            ty.clone()
                        }
                    }
                    _ => ty.clone(),
                };

                // Set self type for method context FIRST
                let previous_self_type = self.current_self_type.clone();
                self.set_current_self_type(Maybe::Some(resolved_ty.clone()));

                // The selected source declaration is already published by
                // register_impl_block. Body checking never creates a second instance.

                // Type-check method bodies
                let prev_inherent = self.in_inherent_impl;
                self.in_inherent_impl = false;
                let prev_in_impl = self.in_impl_block;
                self.in_impl_block = true;
                for item in &impl_decl.items {
                    // Skip impl items gated by @cfg that don't match the current platform
                    if !self.cfg_evaluator.should_include(&item.attributes) {
                        continue;
                    }
                    if let ImplItemKind::Function(func) = &item.kind {
                        if let Some(checked) = self.check_function(func)? {
                            self.publish_checked_method_contract(impl_decl, func, &checked)?;
                        }
                        tracing::debug!(
                            "Type-checked protocol method {}.{} for {}",
                            protocol_name,
                            func.name.name,
                            ty
                        );
                    }
                }
                self.in_impl_block = prev_in_impl;
                self.in_inherent_impl = prev_inherent;

                // Restore previous self type
                self.set_current_self_type(previous_self_type);
            }
        }

        // CONFORMANCE: an `implement P for T` block must provide every
        // method P requires.
        //
        // The check used to be that an implementation EXISTS, never that it
        // is COMPLETE, so `implement P for R { }` against a protocol
        // requiring `fn m` type-checked and then panicked at the call with
        // "method 'm' not found". Worse than a missing implementation,
        // which E405 catches: writing an EMPTY one silenced that too, so
        // the bound `T: P` promised a method the type did not have.
        self.check_protocol_impl_is_complete(impl_decl);

        Ok(())
    }

    /// Resolve a protocol reference in its declaring lexical scope, before the
    /// resulting bound or implementation leaves that scope. Forward nominal
    /// declarations retain their existing owned placeholder key.
    pub(super) fn resolve_protocol_reference(
        &mut self,
        protocol: &verum_ast::Path,
    ) -> Result<verum_ast::Path> {
        let (resolved, key) =
            self.resolve_qualified_type_with_parameter_key(protocol, protocol.span)?;
        let resolved = self.unifier.normalize_aliases(&resolved);
        let identity = match &resolved {
            Type::Named { path, .. } => self.path_to_string(path),
            Type::Generic { name, .. } => name.clone(),
            Type::Placeholder { .. } if key.is_some() => key.unwrap(),
            _ => {
                return Err(TypeError::OtherWithCodeSpanned {
                    code: "E405".into(),
                    msg: format!("implementation requires a protocol declaration, found `{resolved}`").into(),
                    span: protocol.span,
                });
            }
        };
        Ok(verum_ast::Path::new(
            identity.as_str().split('.').map(|segment| verum_ast::ty::PathSegment::Name(
                verum_ast::Ident::new(segment, protocol.span),
            )).collect(),
            protocol.span,
        ))
    }

    /// Conformance requires the actual declaration behind the resolved path.
    pub(super) fn resolve_protocol_conformance_identity(
        &mut self,
        protocol: &verum_ast::Path,
    ) -> Result<Text> {
        let resolved = self.resolve_protocol_reference(protocol)?;
        let identity = self.path_to_string(&resolved);
        if !self.protocol_checker.read().has_protocol(&identity) {
            // Replay an unloaded owned source template through the existing
            // loader, retaining defaults and binder identities.
            self.resolve_qualified_source_type_with_parameter_key(&identity, protocol.span)?;
            if !self.protocol_checker.read().has_protocol(&identity) {
                let written = self.path_to_string(protocol);
                if let Some((declaration, owner)) = self.declaring_source_type_declaration(&written) {
                    self.load_source_type_declaration_with_parameter_key(
                        &declaration, &owner, protocol.span,
                    )?;
                }
            }
        }
        if self.protocol_checker.read().has_protocol(&identity) {
            Ok(identity)
        } else {
            Err(TypeError::OtherWithCodeSpanned {
                code: "E405".into(),
                msg: format!("`{identity}` has no resolved protocol declaration for conformance").into(),
                span: protocol.span,
            })
        }
    }

    /// Report every method an `implement P for T` block owes P and does not
    /// provide.
    ///
    /// Only methods WITHOUT a default body are owed — a protocol that
    /// supplies one has already answered for the case. There are 124 such
    /// methods in `core/`, so treating defaults as required would reject
    /// most of the standard library.
    fn check_protocol_impl_is_complete(&mut self, impl_decl: &verum_ast::decl::ImplDecl) {
        use verum_ast::decl::{ImplItemKind, ImplKind};
        let ImplKind::Protocol { protocol, for_type, .. } = &impl_decl.kind else {
            return;
        };
        let proto_ident = match self.resolve_protocol_conformance_identity(protocol) {
            Ok(identity) => identity,
            Err(error) => {
                self.push_diagnostic_for(error);
                return;
            }
        };

        if let Err(error) = self.check_associated_type_obligations(impl_decl, &proto_ident) {
            self.push_diagnostic_for(error);
        }

        // Signatures are checked in the declaration-owned implementation
        // frame before this implementation is published as protocol evidence.
        // MEASUREMENT, not a diagnostic (VERUM_TRACE_PROTO_MISSING).
        //
        // A protocol method that the implementation never provides and
        // that carries no default is accepted today with silence: the
        // walk above is driven by the IMPL's items, so a method absent
        // from them is never visited. `check_method_conformance` in
        // `protocol.rs` has the arm for it — `ConformanceError::
        // MissingMethod` — reached only from tests.
        //
        // Counted before it is judged, the way T1029 counted the lenient
        // signature comparison: the readings that could make it noise
        // (an implementation split across blocks, a superprotocol
        // default, a blanket impl, a derived method) are cheaper to
        // measure over core/ than to argue about.
        if std::env::var("VERUM_TRACE_PROTO_MISSING").is_ok() {
            let guard = self.protocol_checker.read();
            if let verum_common::Maybe::Some(protocol) = guard.get_protocol(&proto_ident) {
                let provided: Vec<Text> = impl_decl
                    .items
                    .iter()
                    .filter_map(|item| match &item.kind {
                        verum_ast::decl::ImplItemKind::Function(f) => Some(f.name.name.clone()),
                        _ => None,
                    })
                    .collect();
                for m in protocol.methods.values() {
                    if !m.has_default && !provided.iter().any(|n| n == &m.name) {
                        eprintln!(
                            "[proto-missing] {}::{} not provided by the impl (module {})",
                            proto_ident, m.name, self.current_module_path
                        );
                    }
                }
            }
        }
        let Some((required_set, _declared)) =
            self.protocols_with_known_defaults.get(&proto_ident).cloned()
        else {
            return;
        };
        let required: List<Text> = required_set.into_iter().collect();
        if required.is_empty() {
            return;
        }

        let provided: Set<Text> = impl_decl
            .items
            .iter()
            .filter_map(|item| match &item.kind {
                ImplItemKind::Function(f) => Some(f.name.name.clone()),
                _ => None,
            })
            .collect();

        // Additional implementation methods cannot discharge or suppress the
        // obligations of the exact selected protocol declaration.
        // MEASUREMENT ONLY, off by default (T1029). Conformance compares
        // the method NAME and never its SIGNATURE, so an implementation
        // may promise one thing and be another — measured: a generic
        // function bounded by the protocol passes an `Int` into a `Text`
        // parameter and returns the `Bool` it gets back where it
        // promised an `Int`, and nothing complains.
        //
        // The repair needs a number before it needs a diagnostic: the
        // consumer's own comment above records 124 protocol methods in
        // core/ with default bodies, and turning comparison on blind
        // would repeat T1025's mistake where the blast radius is the
        // standard library. So this prints, and does not judge.
        //
        // Both sides are already available and neither is rebuilt here:
        // the protocol registry holds the full declaration
        // (protocol.rs:5015 `register_protocol` keeps it, 7458
        // `get_protocol` returns it) and `function_decl_to_type` turns
        // the implementation's declaration into the same shape.
        if std::env::var("VERUM_TRACE_PROTO_SIG").is_ok() {
            self.report_protocol_signature_drift(impl_decl, &proto_ident);
        }


        let mut missing: List<Text> = required
            .into_iter()
            .filter(|name| !provided.contains(name))
            .collect();
        // Sorted, because the set iterates in an unspecified order and the
        // message is what a spec pins. An unordered list makes a
        // deterministic compiler look flaky.
        missing.sort();
        if missing.is_empty() {
            return;
        }

        // The AST type has no Display; render through the checker's own
        // conversion so the message names the type the way every other
        // diagnostic does.
        let for_type_name = self
            .ast_to_type(for_type)
            .map(|t| t.to_text())
            .unwrap_or_else(|_| Text::from("?"));
        let listed: Vec<&str> = missing.iter().map(|m| m.as_str()).collect();
        let msg = format!(
            "`implement {} for {}` does not provide {}: `{}`\n  \
             help: a protocol method without a default body must be provided by every implementation\n  \
             note: without it the bound `{}` promises a method the type does not have, and the call panics at run time",
            proto_ident,
            for_type_name,
            if listed.len() == 1 { "the required method" } else { "the required methods" },
            listed.join("`, `"),
            proto_ident,
        );
        self.push_diagnostic_for(TypeError::OtherWithCodeSpanned {
            code: Text::from("E405"),
            msg: Text::from(msg),
            span: impl_decl.span,
        });
    }


    /// Print protocol/implementation signature disagreements (T1029).
    ///
    /// MEASUREMENT ONLY — behind `VERUM_TRACE_PROTO_SIG`, prints and
    /// never judges. Conformance today compares the method NAME and not
    /// its signature, and the number this produces over `core/` is what
    /// decides whether the real check can reject or must warn.
    ///
    /// Deliberately LENIENT: a pair is reported only when both sides are
    /// function types of the same shape and neither mentions `Self`.
    /// Substituting `Self` correctly is the real fix's job; over-reporting
    /// here would hand the next step a number made mostly of noise, which
    /// is worse than no number.
    fn report_protocol_signature_drift(
        &self,
        impl_decl: &verum_ast::decl::ImplDecl,
        proto_ident: &Text,
    ) {
        let guard = self.protocol_checker.read();
        let protocol = match guard.get_protocol(proto_ident) {
            verum_common::Maybe::Some(p) => p,
            verum_common::Maybe::None => return,
        };
        // The protocol's OWN type parameters, instantiated by every
        // implementation — `Module::forward(In)` implemented for
        // `AttentionInput` is what implementing a generic protocol MEANS,
        // not a disagreement. The registry holds the list, so this is a
        // question to the authority rather than the name-shape heuristic
        // it replaces (that one declined only 1-2 character names and let
        // `In` vs `AttentionInput` through).
        let proto_params: std::collections::HashSet<String> = protocol
            .type_params
            .iter()
            .map(|tp| tp.name.as_str().to_string())
            .collect();

        for item in impl_decl.items.iter() {
            let verum_ast::decl::ImplItemKind::Function(func) = &item.kind else {
                continue;
            };
            let name = func.name.name.clone();
            let proto_method = match protocol.methods.get(&name) {
                verum_common::Maybe::Some(m) => m,
                verum_common::Maybe::None => continue,
            };
            let impl_ty = crate::method_resolution::function_decl_to_type(func);
            // Both sides go through the protocol matcher's own
            // normalisation before comparison. Its doc-comment names
            // exactly the noise the first measurement produced —
            // "implementations are typically registered with Generic
            // types (e.g. Maybe<T>) while concrete types may be in
            // Variant form (e.g. None | Some(Int))" — and a sibling in
            // the same file states the principle: ONE canonical spelling
            // at the ONE authority every protocol-matching site
            // consults. Conformance is such a site and was not asking.
            // T1033: seven readings eliminated seven candidates for where
            // the `&mut` is lost, so the question moves to the artefact:
            // print WHAT the comparison reads and WHERE the protocol came
            // from. Identity by module path, not by name — a same-named
            // stranger is a standing hazard in this codebase (T0782 counts
            // 136 colliding type names in core/).
            if std::env::var("VERUM_TRACE_PROTO_ORIGIN").is_ok() {
                eprintln!(
                    "[proto-origin] {}::{} raw=`{}` protocol_span={:?}",
                    proto_ident.as_str(),
                    name.as_str(),
                    proto_method.ty.to_text().as_str(),
                    protocol.span
                );
            }
            let proto_ty = Self::normalise_for_comparison(&guard, &proto_method.ty);
            let impl_ty = Self::normalise_for_comparison(&guard, &impl_ty);
            if let Some(reason) =
                Self::signature_disagreement_with_params(&proto_ty, &impl_ty, &proto_params)
            {
                eprintln!(
                    "[proto-sig] {}::{} — {}",
                    proto_ident.as_str(),
                    name.as_str(),
                    reason
                );
            }
        }
    }

    /// Name a type's structural variant, for the kind trace only.
    fn type_kind_name(t: &Type) -> String {
        // One level deep for tuples: the outer kind alone said `Tuple` on
        // both sides of all 49 disagreements, which is true and useless —
        // the difference is an element down. Naming the elements is the
        // question the previous iteration could not answer.
        if let Type::Tuple(elems) = t {
            return format!(
                "Tuple({})",
                elems
                    .iter()
                    .map(Self::type_kind_name)
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        Self::type_kind_leaf(t).to_string()
    }

    fn type_kind_leaf(t: &Type) -> &'static str {
        match t {
            Type::Generic { .. } => "Generic",
            Type::Record(_) => "Record",
            Type::Var(_) => "Var",
            Type::Variant(_) => "Variant",
            Type::Tuple(_) => "Tuple",
            Type::Named { .. } => "Named",
            Type::Reference { .. } => "Reference",
            Type::Slice { .. } => "Slice",
            Type::Function { .. } => "Function",
            Type::Unit => "Unit",
            Type::Never => "Never",
            Type::Unknown => "Unknown",
            Type::Bool => "Bool",
            _ => "other",
        }
    }

    /// Put a type into the spelling the protocol matcher uses, so an
    /// alias and its expansion do not read as a disagreement.
    ///
    /// RECURSIVE, and that is the whole point. The authority itself
    /// matches only `Type::Variant` at the top level and lets everything
    /// else through untouched (protocol.rs:9078, `_ => ty.clone()`), so a
    /// variant NESTED inside a tuple or a generic argument survives it.
    /// Measured: normalising only the top level took core/base from 61
    /// disagreements to 54, and 49 of the survivors were one shape —
    /// `Iterator::size_hint` returning `(Int, None(Unit) | Some(Int))`
    /// against `(Int, Maybe<Int>)`, a variant one level down inside a
    /// tuple. The authority is not widened here: protocol matching relies
    /// on its current behaviour, and this walk is the caller's business.
    fn normalise_for_comparison(
        checker: &crate::protocol::ProtocolChecker,
        ty: &Type,
    ) -> Type {
        match ty {
            Type::Tuple(elems) => Type::Tuple(
                elems
                    .iter()
                    .map(|e| Self::normalise_for_comparison(checker, e))
                    .collect(),
            ),
            Type::Named { path, args } if !args.is_empty() => {
                let normalised = Type::Named {
                    path: path.clone(),
                    args: args
                        .iter()
                        .map(|a| Self::normalise_for_comparison(checker, a))
                        .collect(),
                };
                checker.normalize_variant_to_generic(&normalised)
            }
            Type::Reference { mutable, inner } => Type::Reference {
                mutable: *mutable,
                inner: Box::new(Self::normalise_for_comparison(checker, inner)),
            },
            Type::Slice { element } => Type::Slice {
                element: Box::new(Self::normalise_for_comparison(checker, element)),
            },
            Type::Function {
                property_requirement,
                params,
                return_type,
                contexts,
                type_params,
                properties,
            } => Type::Function {
                property_requirement: property_requirement.clone(),
                // RECURSE, do not call the authority directly. Written the
                // other way this arm made the whole recursive walk dead for
                // function types — which is every type reaching here — and
                // the walk read as landed while changing nothing: 54
                // disagreements before it and 54 after. The kind trace is
                // what exposed it: `protocol=Tuple(other, Variant)` after
                // normalisation means the Variant was never visited.
                params: params
                    .iter()
                    .map(|p| Self::normalise_for_comparison(checker, p))
                    .collect(),
                return_type: Box::new(Self::normalise_for_comparison(checker, return_type)),
                contexts: contexts.clone(),
                type_params: type_params.clone(),
                properties: properties.clone(),
            },
            other => {
                let normalised = checker.normalize_variant_to_generic(other);
                // A MISS is silent by construction: the authority returns
                // its input unchanged when the variant signature is not in
                // `variant_type_names` (protocol.rs:9086, `else ty.clone()`),
                // so "did not know" and "nothing to do" are the same value.
                // That is this session's own class inside the mechanism, and
                // the only way to see it is to notice the identity.
                if std::env::var("VERUM_TRACE_PROTO_SIG_MISS").is_ok()
                    && matches!(other, Type::Variant(_))
                    && normalised.to_text() == other.to_text()
                {
                    eprintln!(
                        "[proto-sig-miss] variant not normalised: {}",
                        other.to_text().as_str()
                    );
                }
                normalised
            }
        }
    }



    fn is_decidable_scalar(t: &Type) -> bool {
        matches!(
            t,
            Type::Unit
                | Type::Never
                | Type::Bool
                | Type::Int
                | Type::Float
                | Type::Char
                | Type::Text
        )
    }

    /// One side a built-in scalar, the other a NAMED type (T1078).
    ///
    /// Deliberately excludes named-vs-named (alias pairs, re-exports,
    /// two spellings of one type — T1029 measured 267 such messages and
    /// every class examined was an artefact), generics, references and
    /// `Unknown`, which is the ABSENCE of an answer rather than one.
    fn scalar_vs_named_disagreement(a: &Type, b: &Type, bound_names: &[Text]) -> bool {
        // `Self` is the one named type that MUST differ from the
        // implementation's: `fn clone(&self) -> Self` implemented for
        // `Int` returns `Int`, and that is the correct implementation,
        // not a disagreement. Measured: without this exclusion the rule
        // reported 10 such pairs in core/base/primitives.vr alone —
        // `clone` returning Bool/Char/Float/Int/Unit and `from`
        // returning Float — every one of them a correct impl.
        //
        // A PROTOCOL PARAMETER is the same case, and it cost a corpus
        // regression to learn (2026-09-03, found by verum-2b's sweep,
        // not by this session's probes):
        //
        //     type Normed<R> is protocol { fn abs(self) -> R; };
        //     implement Normed<Int> for Int { fn abs(self) -> Int {…} }
        //
        // `R` is bound by the implement's argument — `Normed<Int>` means
        // `R = Int` — so `-> Int` is exactly right, and the comparison
        // saw a scalar against the UNSUBSTITUTED `R`. Four files, 19
        // firings, all of them correct code refused.
        //
        // The doc comment on `unambiguous_signature_disagreement` had
        // already listed "a protocol parameter instantiated by the
        // implementation" as one of the benign readings that make
        // `signature_disagreement` unusable as a judge. The exclusion
        // for `Self` was written; the one for the parameters was not —
        // the same rule, one of its two cases spelled.
        let named_concrete = |t: &Type| match t {
            Type::Named { path, .. } => match path.segments.last() {
                Some(verum_ast::ty::PathSegment::Name(ident)) => {
                    let n = ident.name.as_str();
                    n != "Self" && !bound_names.iter().any(|p| p.as_str() == n)
                }
                _ => false,
            },
            _ => false,
        };
        (Self::is_decidable_scalar(a) && named_concrete(b))
            || (named_concrete(a) && Self::is_decidable_scalar(b))
    }

    /// A disagreement between a protocol method and its implementation
    /// that admits NO benign reading.
    ///
    /// `signature_disagreement` below is a MEASUREMENT instrument, not a
    /// judge: a 2560-file sweep produced 267 messages and every class
    /// examined turned out to be a comparison artefact — an alias pair, a
    /// variant expanded on one side only, a protocol parameter
    /// instantiated by the implementation, a reference lost in a
    /// round-trip. Turning that on as a diagnostic would report the
    /// checker's own gaps as the author's mistakes.
    ///
    /// So this speaks only where both sides of a differing position are
    /// built-in scalars, where none of those readings is possible:
    /// `Int` against `Text` is a disagreement in any normalisation.
    ///
    /// ARITY IS DELIBERATELY NOT CHECKED here. The two sides may or may
    /// not count the receiver the same way, and that is not established —
    /// reporting on it would be a guess wearing a diagnostic's clothes.
    ///
    /// `resolve` normalises a named type to the built-in scalar it IS
    /// under another name, so that `NonNeg` and `Int` compare equal and
    /// `Text` against `NonNeg` still compares different.  The MESSAGE
    /// keeps the author's own spelling — they wrote `NonNeg`, and being
    /// told about `Int` would send them looking for a type they never
    /// named.
    pub fn unambiguous_signature_disagreement(
        proto: &Type,
        imp: &Type,
        bound_names: &[Text],
        resolve: &dyn Fn(&Type) -> Option<Type>,
    ) -> Option<String> {
        let (
            Type::Function {
                params: pp,
                return_type: pr,
                ..
            },
            Type::Function {
                params: ip,
                return_type: ir,
                ..
            },
        ) = (proto, imp)
        else {
            return None;
        };
        if pp.len() != ip.len() {
            return None;
        }
        for (i, (a, b)) in pp.iter().zip(ip.iter()).enumerate() {
            // T1078 admitted scalar-vs-named for the RETURN and left the
            // parameters on the older scalar-vs-scalar rule — the same
            // defect in the sibling position, and the worse one of the
            // two: a caller behind the bound passes what the PROTOCOL
            // declares, so an implementation that reads the parameter as
            // another type reads a value nobody ever wrote there.
            // Measured before this repair, with zero diagnostics:
            //
            //     type NonNeg is Int { it >= 0 };
            //     type Sink is protocol { fn take(n: NonNeg) -> Int; };
            //     implement Sink for Box { fn take(n: Text) -> Int {…} }
            //
            // The predicate below is the same one the return uses, its
            // exclusions included: `Self`, the protocol's own parameters
            // and the method's generics are bound elsewhere than in this
            // comparison, so none of them can disagree here.
            let ra = resolve(a).unwrap_or_else(|| a.clone());
            let rb = resolve(b).unwrap_or_else(|| b.clone());
            let scalars_differ =
                Self::is_decidable_scalar(&ra) && Self::is_decidable_scalar(&rb) && ra != rb;
            if scalars_differ || Self::scalar_vs_named_disagreement(&ra, &rb, bound_names) {
                return Some(format!(
                    "parameter {} is `{}`, but the protocol declares `{}`",
                    i + 1,
                    b.to_text(),
                    a.to_text()
                ));
            }
        }
        // T1078: a scalar on ONE side and a NAMED type on the other is
        // just as undeniable as two different scalars, and it is the
        // case that costs a user silence: `implement Display for Shown`
        // with `fn fmt(&self, f: &mut Formatter) -> Text` — the
        // protocol declares `-> Result<(), FormatError>` — compiled
        // with zero diagnostics, and `to_string()` then printed an
        // empty string because the blanket writes through a formatter
        // the impl never touches.
        //
        // Only NAMED vs SCALAR is admitted, not named vs named: two
        // named types can be an alias pair, a re-export, or the same
        // type spelled differently, and T1029's sweep found every such
        // class to be a comparison artefact. A scalar is never an alias
        // for a named type, so this pair admits no benign reading.
        // `return_type` is a `Box<Type>`; the resolver speaks `&Type`.
        let rpr = resolve(&**pr).unwrap_or_else(|| (**pr).clone());
        let rir = resolve(&**ir).unwrap_or_else(|| (**ir).clone());
        if Self::scalar_vs_named_disagreement(&rpr, &rir, bound_names) {
            return Some(format!(
                "returns `{}`, but the protocol declares `{}`",
                ir.to_text(),
                pr.to_text()
            ));
        }
        if Self::is_decidable_scalar(&rpr) && Self::is_decidable_scalar(&rir) && rpr != rir {
            // `Unit` ON THE PROTOCOL SIDE is not an answer, it is the
            // absence of one — the same status the doc comment above
            // already grants `Unknown`. A declared type the checker
            // cannot resolve degrades to `Unit`, and then the
            // diagnostic tells the author the protocol declares
            // something it does not.
            //
            // Measured 2026-09-03, core/math/examples.vr:
            //
            //     protocol: fn omega(&self) -> X.cells(0)
            //     reported: "returns `Int`, but the protocol declares `Unit`"
            //
            // The implementation IS wrong there — but the message
            // names a declaration that does not exist, and the real
            // defect is upstream: `implement InfSubobjectClassifier
            // for T` supplies no argument for the protocol's
            // `<X: InfinityTopos>` parameter, so the projection has
            // nothing to resolve against. Reporting the checker's own
            // unresolved projection as the author's mistake is exactly
            // what `unambiguous_signature_disagreement` exists to
            // avoid.
            //
            // The cost is a genuine `-> Unit` vs `-> Int` mismatch going
            // unreported. That is the direction this file fails in
            // deliberately.
            if matches!(**pr, Type::Unit) {
                return None;
            }
            return Some(format!(
                "returns `{}`, but the protocol declares `{}`",
                ir.to_text(),
                pr.to_text()
            ));
        }
        None
    }

    pub fn signature_disagreement(proto: &Type, imp: &Type) -> Option<String> {
        Self::signature_disagreement_with_params(proto, imp, &std::collections::HashSet::new())
    }

    /// As above, plus the protocol's own type-parameter names: a protocol
    /// side that IS one of them has been instantiated, not contradicted.
    pub fn signature_disagreement_with_params(
        proto: &Type,
        imp: &Type,
        proto_params: &std::collections::HashSet<String>,
    ) -> Option<String> {
        let (Type::Function { params: pp, return_type: pr, .. }, Type::Function { params: ip, return_type: ir, .. }) =
            (proto, imp)
        else {
            return None;
        };
        if pp.len() != ip.len() {
            return Some(format!(
                "arity: protocol {} parameter(s), implementation {}",
                pp.len(),
                ip.len()
            ));
        }
        for (i, (a, b)) in pp.iter().zip(ip.iter()).enumerate() {
            if let Some(d) = Self::type_disagreement_with_params(a, b, proto_params) {
                return Some(format!("parameter {}: {}", i + 1, d));
            }
        }
        Self::type_disagreement_with_params(pr, ir, proto_params)
            .map(|d| format!("return type: {}", d))
    }

    /// Compare two types by their rendered form, declining to judge any
    /// pair that mentions `Self` — see the note on the caller.
    pub fn type_disagreement(a: &Type, b: &Type) -> Option<String> {
        Self::type_disagreement_with_params(a, b, &std::collections::HashSet::new())
    }

    pub fn type_disagreement_with_params(
        a: &Type,
        b: &Type,
        proto_params: &std::collections::HashSet<String>,
    ) -> Option<String> {
        let (ta, tb) = (a.to_text(), b.to_text());
        // `Self`, `Unknown` and `_` are all ABSENCES rather than types:
        // Self is what the real repair must substitute, Unknown is a
        // side that did not resolve, `_` is an inference hole. Counting
        // an absence as a difference is this task's own defect pointed
        // the other way, so all three decline.
        let absent = |t: &verum_common::Text| {
            let s = t.as_str();
            s.contains("Self") || s.contains("Unknown") || s == "_" || s.contains("`_`")
        };
        if ta == tb || absent(&ta) || absent(&tb) {
            return None;
        }

        // A qualified path and a bare one name the SAME type:
        // `&mut core.base.protocols.Hasher` against `&mut Hasher` was
        // six of the eleven distinct messages in the first measurement.
        // Compare on the last component of every dotted name. The real
        // repair should resolve paths to identity; for an instrument
        // whose job is to produce a countable number, dropping the
        // module prefix is the same answer for less machinery.
        let strip_paths = |t: &verum_common::Text| -> String {
            t.as_str()
                .split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.'))
                .map(|w| w.rsplit('.').next().unwrap_or(w))
                .collect::<Vec<_>>()
                .join(" ")
        };
        if strip_paths(&ta) == strip_paths(&tb) {
            return None;
        }

        // Generic parameters that differ only by NAME are the same
        // signature: `Into::into` reported protocol `T` against
        // implementation `U`. Alpha-equivalence proper needs the
        // declaration's parameter list; the instrument approximates it
        // by the convention every one of them follows — a single
        // segment, one or two characters, starting uppercase. Labelled
        // as the approximation it is, because a heuristic presented as
        // a rule is how a measurement turns into a false finding.
        let looks_like_a_type_parameter = |t: &verum_common::Text| {
            let s = t.as_str();
            (1..=2).contains(&s.len())
                && s.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        };
        if looks_like_a_type_parameter(&ta) && looks_like_a_type_parameter(&tb) {
            return None;
        }

        // The protocol side is one of the protocol's declared parameters:
        // the implementation instantiated it, which is the point.
        if proto_params.contains(ta.as_str()) {
            return None;
        }
        if std::env::var("VERUM_TRACE_PROTO_SIG_KIND").is_ok() {
            // The miss-trace fired ZERO times while 49 disagreements were
            // reported, so the thing rendering as `None(Unit) | Some(Int)`
            // is not a `Type::Variant` and never reaches the arm that would
            // notice a normalisation miss. Rather than guess again at what
            // it IS, print the structural kind of both sides.
            eprintln!(
                "[proto-sig-kind] protocol={} impl={}",
                Self::type_kind_name(a),
                Self::type_kind_name(b)
            );
        }
        Some(format!("protocol `{}`, implementation `{}`", ta.as_str(), tb.as_str()))
    }

    /// Resolve declaration-owned aliases before recognizing bottom.
    pub(super) fn is_never_type(&self, ty: &Type) -> bool {
        self.normalize_type(&self.unifier.apply(ty)).is_never()
    }

    /// Check if a type is a dependent type (has type indices).
    ///
    /// Dependent types include:
    /// - Inductive types with indices (e.g., Vec n T)
    /// - Pi types (dependent functions)
    /// - Sigma types (dependent pairs)
    ///
    /// Dependent pattern matching: patterns that refine types in branches, with coverage checking and type narrowing
    pub(super) fn is_dependent_type(&self, ty: &Type) -> bool {
        match ty {
            // Named types with TYPE arguments (like Maybe<T>, List<T>) are NOT dependent types.
            // They are parameterized generics. True dependent types have VALUE-level indices
            // (like Vec(n: Int) where the type depends on a runtime value).
            //

            // Currently we don't have a way to distinguish these in the type representation,
            // so we conservatively return false for all Named types. This means we use
            // regular pattern matching for generic types like Maybe<T>.
            //

            // A proper implementation would check if the type's definition includes
            // value-level indices (dependent indices) vs type-level parameters.
            Type::Named { .. } => false,
            // Function types with dependent return types (Pi types)
            // These would have return types that reference parameter names
            Type::Function { return_type, .. } => {
                // Check if return type depends on parameters
                self.is_dependent_type(return_type)
            }
            // Other types are not dependent
            _ => false,
        }
    }

    /// Type check a match expression with dependent pattern matching.
    ///
    /// This implements the dependent pattern matching algorithm from
    /// Dependent pattern matching: patterns that refine types in branches, with coverage checking and type narrowing
    ///
    /// Key steps:
    /// 1. Infer the motive (how result type depends on scrutinee)
    /// 2. For each branch, refine types based on constructor
    /// 3. Check branch body with refined types
    /// 4. Handle absurd patterns (impossible cases)
    pub(super) fn check_dependent_match(
        &mut self,
        scrutinee_ty: &Type,
        arms: &[verum_ast::pattern::MatchArm],
        _span: Span,
    ) -> Result<InferResult> {
        use crate::dependent_match::DependentPatternChecker;

        // CRITICAL: Never type is the bottom type - it's a subtype of all types.
        // If the first arm returns Never (e.g., panic()), we need to find a non-Never
        // arm to establish the result type. If ALL arms return Never, the result is Never.
        // Type lattice: Never is bottom (subtype of all), Unknown is top (supertype of all)
        let mut result_ty = Type::Never;

        // First pass: check all arms and find the result type (first non-Never type)
        for arm in arms.iter() {
            self.ctx.enter_scope();
            self.refinement_evidence.push_scope();

            // Refine on pattern (scope dep_checker to this block)
            let refinement_opt = {
                let mut dep_checker = DependentPatternChecker::new(
                    &mut self.ctx.env,
                    &mut self.unifier,
                    &self.ctx.inductive_constructors,
                );
                dep_checker.refine_on_pattern(&arm.pattern, scrutinee_ty, arm.span)?
            };

            // Check if this is an absurd pattern (impossible case)
            if let Some(ref refinement) = refinement_opt {
                if refinement.is_absurd() {
                    // Absurd pattern - branch is unreachable
                    self.ctx.exit_scope();
                    continue;
                }

                // Bind pattern variables with refined types
                let mut dep_checker = DependentPatternChecker::new(
                    &mut self.ctx.env,
                    &mut self.unifier,
                    &self.ctx.inductive_constructors,
                );
                dep_checker.bind_pattern_refined(&arm.pattern, scrutinee_ty, Some(refinement))?;
            } else {
                self.bind_pattern(&arm.pattern, scrutinee_ty)?;
            }

            // Type check the arm body
            let arm_result = self.synth_expr(&arm.body)?;
            let arm_ty = arm_result.ty;
            let is_arm_never = self.is_never_type(&arm_ty);

            // If we haven't found a non-Never type yet, use this one
            if self.is_never_type(&result_ty) && !is_arm_never {
                result_ty = arm_ty.clone();
            }

            // Skip unification for Never types (bottom type is subtype of all types)
            // Only unify if both types are non-Never
            if !is_arm_never && !self.is_never_type(&result_ty) {
                // Refine expected type based on constructor
                let expected_ty = if let Some(ref refinement) = refinement_opt {
                    refinement.refine_type(&result_ty)
                } else {
                    result_ty.clone()
                };

                self.unifier.unify(&arm_ty, &expected_ty, arm.body.span)?;
            }

            self.refinement_evidence.pop_scope();
            self.ctx.exit_scope();
        }

        // Infer motive from scrutinee and result types (using non-Never result)
        let _motive = {
            let mut dep_checker = DependentPatternChecker::new(
                &mut self.ctx.env,
                &mut self.unifier,
                &self.ctx.inductive_constructors,
            );
            dep_checker.infer_motive(scrutinee_ty, &result_ty)?
        };

        // Check exhaustiveness
        let patterns: Vec<Pattern> = arms.iter().map(|arm| arm.pattern.clone()).collect();
        {
            let mut dep_checker = DependentPatternChecker::new(
                &mut self.ctx.env,
                &mut self.unifier,
                &self.ctx.inductive_constructors,
            );
            dep_checker.check_exhaustiveness(scrutinee_ty, &patterns)?;
        }

        Ok(InferResult::new(result_ty))
    }

    // ==================== Dependent Type Helper Methods ====================
    // Dependent types (future v2.0+): Pi types, Sigma types, equality types, universe hierarchy, dependent pattern matching, termination checking — Helper methods for dependent type inference

    /// Check if a return type depends on a parameter.
    ///
    /// This is used to detect when we need to create a Pi type instead of a
    /// regular function type.
    ///
    /// A return type depends on a parameter if it contains a reference to the
    /// parameter's name in type indices or refinement predicates.
    pub(super) fn return_type_depends_on_param(
        &self,
        return_ty: &Type,
        param_names: &List<Option<Text>>,
    ) -> bool {
        if param_names.is_empty() || param_names[0].is_none() {
            return false;
        }

        let Some(param_name) = param_names[0].as_ref() else {
            return false;
        };
        self.type_contains_reference(return_ty, param_name)
    }

    /// Check if a type contains a reference to a given variable name.
    ///
    /// This recursively searches the type structure for any Named or Generic
    /// types whose arguments might reference the variable.
    fn type_contains_reference(&self, ty: &Type, var_name: &Text) -> bool {
        use Type::*;

        match ty {
            // Named types may have type arguments that reference the variable
            Named { args, .. } | Generic { args, .. } => args
                .iter()
                .any(|arg| self.type_contains_reference(arg, var_name)),

            // Pi type: check both parameter and return types
            Pi { return_type, .. } => self.type_contains_reference(return_type, var_name),

            // Sigma type: check both components
            Sigma { snd_type, .. } => self.type_contains_reference(snd_type, var_name),

            // Refined types: the predicate might reference the variable
            Refined { base, predicate } => {
                self.type_contains_reference(base, var_name)
                    || self.predicate_references_var(&predicate.predicate, var_name)
            }

            // Function types: check parameters and return type
            Function {
                params,
                return_type,
                ..
            } => {
                params
                    .iter()
                    .any(|p| self.type_contains_reference(p, var_name))
                    || self.type_contains_reference(return_type, var_name)
            }

            // Container types
            Tuple(tys) => tys
                .iter()
                .any(|t| self.type_contains_reference(t, var_name)),
            Array { element, .. } => self.type_contains_reference(element, var_name),
            Slice { element } => self.type_contains_reference(element, var_name),

            // Reference types
            Reference { inner, .. }
            | CheckedReference { inner, .. }
            | UnsafeReference { inner, .. }
            | Ownership { inner, .. }
            | Pointer { inner, .. }
            | GenRef { inner } => self.type_contains_reference(inner, var_name),

            // Base types don't contain references
            Unit | Bool | Int | Float | Char | Text | Never | Var(_) | Prop => false,

            // Other complex types (conservative: assume they might contain references)
            _ => false,
        }
    }

    /// Check if a refinement predicate references a variable.
    /// Dependent types (future v2.0+): Pi types, Sigma types, equality types, universe hierarchy, dependent pattern matching, termination checking — Section 2.5 - Refinement Type Integration
    ///
    /// This traverses the expression AST to find occurrences of the given variable.
    /// It's essential for determining if a refinement type is dependent on a value.
    fn predicate_references_var(&self, predicate: &Expr, var_name: &Text) -> bool {
        use verum_ast::expr::ExprKind;

        match &predicate.kind {
            // Path - check if it references the variable
            ExprKind::Path(path) => {
                // Check if any segment matches the variable name
                for segment in &path.segments {
                    if let verum_ast::ty::PathSegment::Name(ident) = segment
                        && ident.name.as_str() == var_name.as_str()
                    {
                        return true;
                    }
                }
                false
            }

            // Literal - doesn't reference any variable
            ExprKind::Literal(_) => false,

            // Binary operation - check both sides
            ExprKind::Binary { left, right, .. } => {
                self.predicate_references_var(left, var_name)
                    || self.predicate_references_var(right, var_name)
            }

            // Unary operation - check operand
            ExprKind::Unary { expr, .. } => self.predicate_references_var(expr, var_name),

            // Function call - check function and all arguments
            ExprKind::Call { func, args, .. } => {
                if self.predicate_references_var(func, var_name) {
                    return true;
                }
                for arg in args {
                    if self.predicate_references_var(arg, var_name) {
                        return true;
                    }
                }
                false
            }

            // Method call - check receiver and arguments
            ExprKind::MethodCall { receiver, args, .. } => {
                if self.predicate_references_var(receiver, var_name) {
                    return true;
                }
                for arg in args {
                    if self.predicate_references_var(arg, var_name) {
                        return true;
                    }
                }
                false
            }

            // Field access - check the expression
            ExprKind::Field { expr, .. } => self.predicate_references_var(expr, var_name),

            // Optional chaining - check the expression
            ExprKind::OptionalChain { expr, .. } => self.predicate_references_var(expr, var_name),

            // Tuple index - check the expression
            ExprKind::TupleIndex { expr, .. } => self.predicate_references_var(expr, var_name),

            // Index operation - check both expression and index
            ExprKind::Index { expr, index } => {
                self.predicate_references_var(expr, var_name)
                    || self.predicate_references_var(index, var_name)
            }

            // Pipeline - check both sides
            ExprKind::Pipeline { left, right } => {
                self.predicate_references_var(left, var_name)
                    || self.predicate_references_var(right, var_name)
            }

            // Null coalescing - check both sides
            ExprKind::NullCoalesce { left, right } => {
                self.predicate_references_var(left, var_name)
                    || self.predicate_references_var(right, var_name)
            }

            // Cast - check the expression
            ExprKind::Cast { expr, .. } => self.predicate_references_var(expr, var_name),

            // Try operator - check the expression
            ExprKind::Try(expr) => self.predicate_references_var(expr, var_name),
            ExprKind::TryBlock(block) => self.predicate_references_var(block, var_name),

            // Tuple - check all elements
            ExprKind::Tuple(elements) => {
                for elem in elements {
                    if self.predicate_references_var(elem, var_name) {
                        return true;
                    }
                }
                false
            }

            // Array - check elements based on ArrayExpr variant
            ExprKind::Array(array_expr) => self.array_expr_references_var(array_expr, var_name),

            // If expression - check condition and branches
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                // Check the condition (IfCondition can be an expression or let pattern)
                if self.if_condition_references_var(condition, var_name) {
                    return true;
                }
                // Check then_branch (Block)
                if self.block_references_var(then_branch, var_name) {
                    return true;
                }
                // Check else_branch
                if let verum_common::Maybe::Some(else_expr) = else_branch
                    && self.predicate_references_var(else_expr, var_name)
                {
                    return true;
                }
                false
            }

            // Match expression - check scrutinee and arms
            ExprKind::Match { expr, arms } => {
                if self.predicate_references_var(expr, var_name) {
                    return true;
                }
                for arm in arms {
                    // Check guard if present
                    if let verum_common::Maybe::Some(guard) = &arm.guard
                        && self.predicate_references_var(guard, var_name)
                    {
                        return true;
                    }
                    // Check body
                    if self.predicate_references_var(&arm.body, var_name) {
                        return true;
                    }
                }
                false
            }

            // Closure - check if variable is not shadowed by parameter
            ExprKind::Closure { params, body, .. } => {
                // Check if the variable is shadowed by a parameter pattern
                for param in params {
                    if self.pattern_binds_var(&param.pattern, var_name) {
                        return false; // Variable is shadowed
                    }
                }
                self.predicate_references_var(body, var_name)
            }

            // Block - check all statements and final expression
            ExprKind::Block(block) => self.block_references_var(block, var_name),

            // Range expressions - check bounds
            ExprKind::Range { start, end, .. } => {
                if let verum_common::Maybe::Some(s) = start
                    && self.predicate_references_var(s, var_name)
                {
                    return true;
                }
                if let verum_common::Maybe::Some(e) = end
                    && self.predicate_references_var(e, var_name)
                {
                    return true;
                }
                false
            }

            // Await - check inner expression
            ExprKind::Await(expr) => self.predicate_references_var(expr, var_name),

            // Record literal - check field values
            ExprKind::Record { fields, base, .. } => {
                for field in fields {
                    if let verum_common::Maybe::Some(ref value) = field.value
                        && self.predicate_references_var(value, var_name)
                    {
                        return true;
                    }
                }
                if let verum_common::Maybe::Some(base_expr) = base
                    && self.predicate_references_var(base_expr, var_name)
                {
                    return true;
                }
                false
            }

            // Quantifiers - check body (variable might be bound by pattern)
            ExprKind::Forall { bindings, body } | ExprKind::Exists { bindings, body } => {
                // Check if the variable is bound by any quantifier binding's pattern
                for binding in bindings {
                    if self.pattern_binds_var(&binding.pattern, var_name) {
                        return false; // Variable is bound
                    }
                }
                self.predicate_references_var(body, var_name)
            }

            // Return - check inner expression if present
            ExprKind::Return(expr) => {
                if let verum_common::Maybe::Some(e) = expr {
                    self.predicate_references_var(e, var_name)
                } else {
                    false
                }
            }

            // Break - check value if present
            ExprKind::Break { value, .. } => {
                if let verum_common::Maybe::Some(e) = value {
                    self.predicate_references_var(e, var_name)
                } else {
                    false
                }
            }

            // Continue - no expression
            ExprKind::Continue { .. } => false,

            // Yield - check inner expression
            ExprKind::Yield(expr) => self.predicate_references_var(expr, var_name),

            // Paren - check inner expression
            ExprKind::Paren(expr) => self.predicate_references_var(expr, var_name),

            // Comprehension - check expression and clauses
            ExprKind::Comprehension { expr, clauses } => {
                if self.predicate_references_var(expr, var_name) {
                    return true;
                }
                for clause in clauses {
                    if self.comprehension_clause_references_var(clause, var_name) {
                        return true;
                    }
                }
                false
            }

            // Loop expressions - check body
            ExprKind::Loop {
                body, invariants, ..
            } => {
                if self.block_references_var(body, var_name) {
                    return true;
                }
                for inv in invariants {
                    if self.predicate_references_var(inv, var_name) {
                        return true;
                    }
                }
                false
            }

            ExprKind::While {
                condition,
                body,
                invariants,
                decreases,
                ..
            } => {
                if self.predicate_references_var(condition, var_name) {
                    return true;
                }
                if self.block_references_var(body, var_name) {
                    return true;
                }
                for inv in invariants {
                    if self.predicate_references_var(inv, var_name) {
                        return true;
                    }
                }
                for dec in decreases {
                    if self.predicate_references_var(dec, var_name) {
                        return true;
                    }
                }
                false
            }

            ExprKind::For {
                pattern,
                iter,
                body,
                invariants,
                decreases,
                ..
            } => {
                // Check if variable is bound by pattern
                if self.pattern_binds_var(pattern, var_name) {
                    // Variable is bound, don't check body for this var
                    // But still check iter
                    return self.predicate_references_var(iter, var_name);
                }
                if self.predicate_references_var(iter, var_name) {
                    return true;
                }
                if self.block_references_var(body, var_name) {
                    return true;
                }
                for inv in invariants {
                    if self.predicate_references_var(inv, var_name) {
                        return true;
                    }
                }
                for dec in decreases {
                    if self.predicate_references_var(dec, var_name) {
                        return true;
                    }
                }
                false
            }

            // Other expressions - conservatively return false for unhandled cases
            _ => false,
        }
    }

    /// Check if an ArrayExpr references a variable
    fn array_expr_references_var(
        &self,
        array_expr: &verum_ast::expr::ArrayExpr,
        var_name: &Text,
    ) -> bool {
        use verum_ast::expr::ArrayExpr;
        match array_expr {
            ArrayExpr::List(elements) => {
                for elem in elements {
                    if self.predicate_references_var(elem, var_name) {
                        return true;
                    }
                }
                false
            }
            ArrayExpr::Repeat { value, count } => {
                self.predicate_references_var(value, var_name)
                    || self.predicate_references_var(count, var_name)
            }
        }
    }

    /// Check if an IfCondition references a variable
    fn if_condition_references_var(
        &self,
        condition: &verum_ast::expr::IfCondition,
        var_name: &Text,
    ) -> bool {
        use verum_ast::expr::ConditionKind;
        for cond in &condition.conditions {
            match cond {
                ConditionKind::Expr(expr) => {
                    if self.predicate_references_var(expr, var_name) {
                        return true;
                    }
                }
                ConditionKind::Let { value, .. } => {
                    if self.predicate_references_var(value, var_name) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Check if a Block references a variable
    fn block_references_var(&self, block: &verum_ast::expr::Block, var_name: &Text) -> bool {
        for stmt in &block.stmts {
            if self.stmt_references_var(stmt, var_name) {
                return true;
            }
        }
        if let verum_common::Maybe::Some(final_expr) = &block.expr
            && self.predicate_references_var(final_expr, var_name)
        {
            return true;
        }
        false
    }

    /// Check if a statement references a variable
    fn stmt_references_var(&self, stmt: &verum_ast::stmt::Stmt, var_name: &Text) -> bool {
        use verum_ast::stmt::StmtKind;

        match &stmt.kind {
            StmtKind::Let { value, .. } => {
                if let verum_common::Maybe::Some(expr) = value {
                    self.predicate_references_var(expr, var_name)
                } else {
                    false
                }
            }
            StmtKind::LetElse {
                value, else_block, ..
            } => {
                self.predicate_references_var(value, var_name)
                    || self.block_references_var(else_block, var_name)
            }
            StmtKind::Expr { expr, .. } => self.predicate_references_var(expr, var_name),
            StmtKind::Item(_) => false, // Items don't reference variables in the same way
            StmtKind::Defer(expr) => self.predicate_references_var(expr, var_name),
            StmtKind::Errdefer(expr) => self.predicate_references_var(expr, var_name),
            StmtKind::Provide { value, .. } => self.predicate_references_var(value, var_name),
            StmtKind::ProvideScope { value, block, .. } => {
                self.predicate_references_var(value, var_name)
                    || self.predicate_references_var(block, var_name)
            }
            StmtKind::Empty => false,
        }
    }

    /// Check if a comprehension clause references a variable
    fn comprehension_clause_references_var(
        &self,
        clause: &verum_ast::expr::ComprehensionClause,
        var_name: &Text,
    ) -> bool {
        use verum_ast::expr::ComprehensionClauseKind;
        match &clause.kind {
            ComprehensionClauseKind::For { iter, .. } => {
                self.predicate_references_var(iter, var_name)
            }
            ComprehensionClauseKind::If(condition) => {
                self.predicate_references_var(condition, var_name)
            }
            ComprehensionClauseKind::Let { value, .. } => {
                self.predicate_references_var(value, var_name)
            }
        }
    }

    // ============================================================
    // Return Value Lifetime Validation
    // Return reference validation: ensuring returned references do not outlive their referents via escape analysis — Dangling references
    // ============================================================

    /// Check if a type is a reference type.
    pub(super) fn is_reference_type(&self, ty: &Type) -> bool {
        matches!(
            ty,
            Type::Reference { .. } | Type::CheckedReference { .. } | Type::UnsafeReference { .. }
        )
    }

    /// Check that a returned reference doesn't refer to a local variable.
    /// Prevents dangling references when the function returns.
    pub(super) fn check_return_lifetime(&self, expr: &Expr, return_span: Span) -> Result<()> {
        use verum_ast::expr::ExprKind;

        match &expr.kind {
            // Direct reference to a variable
            // Spec: L0-critical/reference_system/reference_tiers/checked_ref_escape
            ExprKind::Unary { op, expr: inner } => {
                use verum_ast::expr::UnOp;
                // Check all reference types: managed (&T), checked (&checked T), and unsafe (&unsafe T)
                // All tiers must prevent returning references to local variables
                if matches!(
                    op,
                    UnOp::Ref
                        | UnOp::RefMut
                        | UnOp::RefChecked
                        | UnOp::RefCheckedMut
                        | UnOp::RefUnsafe
                        | UnOp::RefUnsafeMut
                ) {
                    // Get the variable being referenced
                    let (var_opt, _) = self.extract_capture_target(inner);
                    if let Some(var_name) = var_opt {
                        // Check if this is a local variable (not a parameter or captured)
                        // Local variables are created in the current scope
                        if self.is_local_variable(&var_name) {
                            return Err(TypeError::DanglingReference {
                                var: var_name,
                                ref_span: return_span,
                                drop_span: return_span, // Variable dropped at function end
                            });
                        }
                    }
                }
            }
            // Variable that holds a reference
            ExprKind::Path(path) => {
                if let Some(verum_ast::ty::PathSegment::Name(ident)) = path.segments.first() {
                    let var_name = verum_common::Text::from(ident.name.as_str());
                    // Check if returning a reference that was created from a local
                    for borrow in self.borrow_tracker.get_held_borrows(&var_name) {
                        if self.is_local_variable(&borrow.target) {
                            return Err(TypeError::DanglingReference {
                                var: borrow.target.clone(),
                                ref_span: return_span,
                                drop_span: return_span,
                            });
                        }
                    }
                }
            }
            // Field access - check the root variable
            ExprKind::Field { .. } => {
                let (var_opt, _) = self.extract_capture_target(expr);
                if let Some(var_name) = var_opt {
                    if self.is_local_variable(&var_name) {
                        // Returning reference to field of local variable
                        return Err(TypeError::DanglingReference {
                            var: var_name,
                            ref_span: return_span,
                            drop_span: return_span,
                        });
                    }
                }
            }
            // Method call that returns a reference
            // e.g., `container.get_ref()` where `container` is local
            // The returned reference is tied to the receiver's lifetime
            // Spec: L0-critical/reference_system/access_rules/ref_return_from_struct_fail
            ExprKind::MethodCall { receiver, .. } => {
                // Get the base variable being called on
                let (var_opt, _) = self.extract_capture_target(receiver);
                if let Some(var_name) = var_opt {
                    if self.is_local_variable(&var_name) {
                        // Returning a reference from a method on a local variable
                        // The returned reference is tied to the local's lifetime
                        return Err(TypeError::DanglingReference {
                            var: var_name,
                            ref_span: return_span,
                            drop_span: return_span,
                        });
                    }
                }
            }
            _ => {}
        }

        Ok(())
    }

    /// Check if a variable is a local variable (not a parameter).
    /// Local variables are dropped when the function returns, so returning
    /// references to them creates dangling references.
    fn is_local_variable(&self, var_name: &Text) -> bool {
        // A variable is local if:
        // 1. It's defined in the current scope (not a parameter)
        // 2. It's not captured from an outer scope
        //

        // For now, we use a heuristic: check if it's in the current function's
        // parameter list. If not found there, it's local.
        //

        // Parameters are tracked separately and are valid to return references to
        // (the caller owns the referent).
        //
        // Module-level items (const, function, type, static, imported via
        // `mount`) live in the root scope of `self.ctx.env` and carry
        // process lifetime.  Pre-fix the predicate gated only on the
        // function-parameter check, so every `&CONST_NAME` returned from
        // a function triggered the E312 false-positive
        // `\`CONST_NAME\` does not live long enough - reference outlives referent`.
        // The grammar (`grammar/verum.ebnf`) admits `fn f() -> &T { &CONST }`
        // as well-formed; the language semantics treat module-level items
        // as static-lifetime; the pre-fix checker contradicted both.
        // `lookup_in_root_only` walks the env's parent chain to the root
        // and probes that scope only — function-body `let` bindings live
        // in nested scopes pushed by the body-block checker, so they
        // remain correctly classified as local.
        if self.current_function_params.contains(var_name) {
            return false;
        }
        if self
            .ctx
            .env
            .lookup_in_root_only(var_name.as_str())
            .is_some()
        {
            return false;
        }
        true
    }

    /// Check if an expression is a &checked reference to a local variable.
    /// Returns the variable name if so, None otherwise.
    /// Spec: L0-critical/reference_system/reference_tiers/checked_promotion_fail
    pub(super) fn get_checked_ref_to_local(&self, expr: &Expr) -> Option<Text> {
        use verum_ast::expr::ExprKind;

        // Check if expression is a Path that holds a &checked reference to a local
        if let ExprKind::Path(path) = &expr.kind {
            if let Some(verum_ast::ty::PathSegment::Name(ident)) = path.segments.first() {
                let var_name = verum_common::Text::from(ident.name.as_str());

                // Check the type of this variable
                if let Some(scheme) = self.ctx.env.lookup(&var_name).cloned() {
                    let var_ty = scheme.instantiate();

                    // Is it a checked reference type?
                    if matches!(var_ty, Type::CheckedReference { .. }) {
                        // Check if the borrow target is a local variable
                        for borrow in self.borrow_tracker.get_held_borrows(&var_name) {
                            if self.is_local_variable(&borrow.target) {
                                return Some(borrow.target.clone());
                            }
                        }
                    }
                }
            }
        }

        None
    }

    // ============================================================
    // Affine Value Consumption in Function Calls
    // Spec: L0-critical/reference_system/value_transfer - Affine type safety
    // ============================================================

    /// Consume affine values that were passed by value (not by reference) to a function call.
    ///
    /// During argument checking, `in_call_arg_context=true` causes all identifier lookups
    /// to use `borrow_value()` instead of `use_value()`. This is correct for reference
    /// parameters, but for by-value parameters of affine types, we need to explicitly
    /// consume the value after checking completes.
    pub(super) fn consume_affine_call_args(&mut self, args: &[Expr], param_types: &[Type]) -> Result<()> {
        for (arg, param_ty) in args.iter().zip(param_types.iter()) {
            let resolved_param = self.unifier.apply(param_ty);

            // Skip reference parameters - those are borrows, not moves
            if self.is_reference_type(&resolved_param) {
                continue;
            }

            // Extract the variable name from the argument expression
            if let ExprKind::Path(path) = &arg.kind && let Some(ident) = path.as_ident() {
                let name = &ident.name;
                self.check_borrowed_move(name, &resolved_param, arg.span)?;
                // Check if this variable is affine and consume it
                if self.affine_tracker.is_affine_binding(name.as_str()) {
                    self.affine_tracker.use_value(name.as_str(), arg.span)?;
                }
            }
        }
        Ok(())
    }

    // ============================================================
    // Thread Safety Detection
    // CBGR checking: generation counter validation at each dereference, epoch-based tracking prevents wraparound — .2 - Thread safety
    // ============================================================

    /// Check if a type is known to be !Send (not thread-safe to transfer).
    /// These types contain internal mutable state that is not thread-safe.
    pub(super) fn is_non_send_type(&self, ty: &Type) -> bool {
        match ty {
            // Raw pointers are not Send
            Type::Named { path, .. } if Self::path_ends_with(path, "RawPtr") => true,
            Type::Named { path, .. } if Self::path_ends_with(path, "UnsafeCell") => true,
            Type::Named { path, .. } if Self::path_ends_with(path, "Cell") => true,
            Type::Named { path, .. } if Self::path_ends_with(path, "RefCell") => true,
            Type::Named { path, .. } if Self::path_ends_with(path, "Rc") => true,
            // References - check if inner type is Send
            Type::Reference { inner, .. } => self.is_non_send_type(inner),
            Type::CheckedReference { inner, .. } => self.is_non_send_type(inner),
            Type::UnsafeReference { inner, .. } => self.is_non_send_type(inner),
            // Generic types - check if any type argument is !Send
            Type::Generic { args, .. } => args.iter().any(|arg| self.is_non_send_type(arg)),
            Type::Named { args, .. } => args.iter().any(|arg| self.is_non_send_type(arg)),
            // Tuples - all elements must be Send
            Type::Tuple(elems) => elems.iter().any(|elem| self.is_non_send_type(elem)),
            // Arrays - element type must be Send
            Type::Array { element, .. } => self.is_non_send_type(element),
            // Function types are Send
            Type::Function { .. } => false,
            // Primitive types are Send
            Type::Int | Type::Float | Type::Bool | Type::Char | Type::Text | Type::Unit => false,
            // Default: assume Send (conservative for user-defined types)
            _ => false,
        }
    }

    /// Check if a type is known to be !Sync (not thread-safe to share).
    pub(super) fn is_non_sync_type(&self, ty: &Type) -> bool {
        match ty {
            // Types with interior mutability are not Sync
            Type::Named { path, .. } if Self::path_ends_with(path, "Cell") => true,
            Type::Named { path, .. } if Self::path_ends_with(path, "RefCell") => true,
            Type::Named { path, .. } if Self::path_ends_with(path, "UnsafeCell") => true,
            // Rc is not Sync (reference count is not atomic)
            Type::Named { path, .. } if Self::path_ends_with(path, "Rc") => true,
            // References inherit Sync from referent
            Type::Reference { inner, mutable, .. } => {
                if *mutable {
                    true // &mut T is not Sync
                } else {
                    self.is_non_sync_type(inner)
                }
            }
            Type::CheckedReference { inner, mutable, .. } => {
                if *mutable {
                    true
                } else {
                    self.is_non_sync_type(inner)
                }
            }
            Type::UnsafeReference { inner, mutable, .. } => {
                if *mutable {
                    true
                } else {
                    self.is_non_sync_type(inner)
                }
            }
            // Generic types - check type arguments
            Type::Generic { args, .. } => args.iter().any(|arg| self.is_non_sync_type(arg)),
            Type::Named { args, .. } => args.iter().any(|arg| self.is_non_sync_type(arg)),
            // Default: assume Sync
            _ => false,
        }
    }

    /// Helper to check if a path ends with a specific type name
    fn path_ends_with(path: &verum_ast::ty::Path, name: &str) -> bool {
        path.segments.last().is_some_and(|seg| {
            matches!(seg, verum_ast::ty::PathSegment::Name(ident) if ident.name.as_str() == name)
        })
    }

    // ============================================================
    // Iterator Invalidation Detection
    // Memory layout and reference representation: ThinRef (16 bytes) for sized types, FatRef (24 bytes) for unsized types — .4 - Iterator invalidation
    // ============================================================

    /// Check if a method is a mutating collection method that could invalidate iterators.
    /// These methods modify the collection's internal state and can invalidate any
    /// active iterators over the collection.
    pub(super) fn is_mutating_collection_method(&self, method_name: &str) -> bool {
        // List of methods that mutate collection contents and can invalidate iterators
        const MUTATING_METHODS: &[&str] = &[
            // List/Vec methods
            "push",
            "pop",
            "insert",
            "remove",
            "clear",
            "truncate",
            "resize",
            "extend",
            "append",
            "drain",
            "retain",
            "dedup",
            "swap_remove",
            "reserve",
            "shrink_to_fit",
            // Set methods
            "add",
            // Map/Dict methods
            "put",
            // Deque methods
            "push_front",
            "push_back",
            "pop_front",
            "pop_back",
            // Generic mutation
            "sort",
            "sort_by",
            "reverse",
            "shuffle",
        ];

        MUTATING_METHODS.contains(&method_name)
    }

    // ============================================================
    // Closure Capture Analysis
    // Memory layout and reference representation: ThinRef (16 bytes) for sized types, FatRef (24 bytes) for unsized types — .5 - Closure captures
    // ============================================================

    /// Analyze a closure body to find all captured variables and their capture modes.
    /// Returns a list of (variable_name, field_path, capture_mode, span) tuples.
    /// field_path is Some("field.subfield") for field access captures like `x.field.subfield`.
    ///
    /// This is essential for:
    /// 1. Detecting aliasing conflicts between captures and existing borrows
    /// 2. Determining whether the closure implements Fn, FnMut, or FnOnce
    /// 3. Checking Send/Sync bounds on captured variables
    /// 4. Field-level borrow splitting for more precise analysis
    pub(super) fn analyze_closure_captures(
        &self,
        body: &Expr,
        params: &[verum_ast::expr::ClosureParam],
        is_move: bool,
    ) -> List<(Text, Option<Text>, crate::aliasing::CaptureMode, Span)> {
        use crate::aliasing::CaptureMode;
        use std::collections::{HashMap, HashSet};

        // Key: (variable_name, field_path_option)
        // Value: (capture_mode, span)
        type CaptureKey = (Text, Option<Text>);

        // Track local bindings (parameters + let bindings in the closure)
        let mut local_bindings: HashSet<Text> = HashSet::new();

        // Add parameters to local bindings
        for param in params {
            self.collect_capture_pattern_bindings(&param.pattern, &mut local_bindings);
        }

        // Find all variable references in the body
        // Key: (var_name, field_path), Value: (mode, span)
        let mut var_uses: HashMap<CaptureKey, (CaptureMode, Span)> = HashMap::new();

        // Analyze the body recursively
        self.find_captures_in_expr_with_fields(body, &local_bindings, &mut var_uses, is_move);

        // Convert to result list
        var_uses
            .into_iter()
            .map(|((var, field_path), (mode, span))| (var, field_path, mode, span))
            .collect()
    }

    /// Extract variable name and field path from an expression.
    /// For `x.field.subfield`, returns (Some("x"), Some("field.subfield"))
    /// For `x`, returns (Some("x"), None)
    /// For other expressions, returns (None, None)
    fn extract_capture_target(&self, expr: &Expr) -> (Option<Text>, Option<Text>) {
        use verum_ast::expr::ExprKind;

        match &expr.kind {
            ExprKind::Path(path) => {
                if let Some(verum_ast::ty::PathSegment::Name(ident)) = path.segments.first() {
                    (Some(verum_common::Text::from(ident.name.as_str())), None)
                } else {
                    (None, None)
                }
            }
            ExprKind::Field { expr: inner, field } => {
                let (base_var, base_path) = self.extract_capture_target(inner);
                if let Some(var) = base_var {
                    let field_name = field.name.as_str();
                    let new_path = match base_path {
                        Some(path) => {
                            Some(verum_common::Text::from(format!("{}.{}", path, field_name)))
                        }
                        None => Some(verum_common::Text::from(field_name)),
                    };
                    (Some(var), new_path)
                } else {
                    (None, None)
                }
            }
            _ => (None, None),
        }
    }

    /// Recursively find captures in an expression with field-level tracking.
    /// This version tracks field paths like `x.field.subfield` separately from `x`.
    fn find_captures_in_expr_with_fields(
        &self,
        expr: &Expr,
        local_bindings: &std::collections::HashSet<Text>,
        var_uses: &mut std::collections::HashMap<
            (Text, Option<Text>),
            (crate::aliasing::CaptureMode, Span),
        >,
        is_move: bool,
    ) {
        use crate::aliasing::CaptureMode;
        use verum_ast::expr::ExprKind;

        match &expr.kind {
            // Variable reference - this is a potential capture
            ExprKind::Path(path) => {
                if let Some(verum_ast::ty::PathSegment::Name(ident)) = path.segments.first() {
                    let var_name = verum_common::Text::from(ident.name.as_str());

                    // Skip if it's a local binding
                    if local_bindings.contains(&var_name) {
                        return;
                    }

                    // Skip if it looks like a type/module name (starts with uppercase)
                    if ident
                        .name
                        .chars()
                        .next()
                        .map(|c| c.is_uppercase())
                        .unwrap_or(false)
                    {
                        return;
                    }

                    // Determine capture mode
                    let mode = if is_move {
                        CaptureMode::Move
                    } else {
                        CaptureMode::Borrow
                    };

                    // Update or insert - keep the more restrictive mode
                    let key = (var_name, None);
                    var_uses
                        .entry(key)
                        .and_modify(|(existing_mode, _)| {
                            if *existing_mode == CaptureMode::Borrow && mode == CaptureMode::Move {
                                *existing_mode = CaptureMode::Move;
                            }
                        })
                        .or_insert((mode, expr.span));
                }
            }

            // Field access - track as separate capture with field path
            ExprKind::Field { .. } => {
                let (base_var, field_path) = self.extract_capture_target(expr);
                if let Some(var_name) = base_var {
                    // Skip if base variable is local
                    if local_bindings.contains(&var_name) {
                        return;
                    }

                    // Skip if it looks like a type/module name
                    if var_name
                        .chars()
                        .next()
                        .map(|c| c.is_uppercase())
                        .unwrap_or(false)
                    {
                        return;
                    }

                    let mode = if is_move {
                        CaptureMode::Move
                    } else {
                        CaptureMode::Borrow
                    };

                    // Track field-level capture
                    let key = (var_name, field_path);
                    var_uses
                        .entry(key)
                        .and_modify(|(existing_mode, _)| {
                            if *existing_mode == CaptureMode::Borrow && mode == CaptureMode::Move {
                                *existing_mode = CaptureMode::Move;
                            }
                        })
                        .or_insert((mode, expr.span));
                }
            }

            // Mutable reference - capture as MutBorrow
            ExprKind::Unary { op, expr: inner } => {
                use verum_ast::expr::UnOp;
                if matches!(op, UnOp::RefMut) {
                    // Extract target with potential field path
                    let (base_var, field_path) = self.extract_capture_target(inner);
                    if let Some(var_name) = base_var {
                        if !local_bindings.contains(&var_name) {
                            let key = (var_name, field_path);
                            var_uses
                                .entry(key)
                                .and_modify(|(mode, _)| {
                                    if *mode == CaptureMode::Borrow {
                                        *mode = CaptureMode::MutBorrow;
                                    }
                                })
                                .or_insert((CaptureMode::MutBorrow, expr.span));
                        }
                    }
                }
                self.find_captures_in_expr_with_fields(inner, local_bindings, var_uses, is_move);
            }

            // Binary expressions (including assignment)
            ExprKind::Binary { op, left, right } => {
                use verum_ast::expr::BinOp;

                // Check if this is an assignment - the target needs MutBorrow
                let is_assignment = matches!(
                    op,
                    BinOp::Assign
                        | BinOp::AddAssign
                        | BinOp::SubAssign
                        | BinOp::MulAssign
                        | BinOp::DivAssign
                        | BinOp::RemAssign
                        | BinOp::BitAndAssign
                        | BinOp::BitOrAssign
                        | BinOp::BitXorAssign
                        | BinOp::ShlAssign
                        | BinOp::ShrAssign
                );

                if is_assignment {
                    // Extract target with potential field path
                    let (base_var, field_path) = self.extract_capture_target(left);
                    if let Some(var_name) = base_var {
                        if !local_bindings.contains(&var_name) {
                            let key = (var_name, field_path);
                            var_uses
                                .entry(key)
                                .and_modify(|(mode, _)| {
                                    if *mode == CaptureMode::Borrow {
                                        *mode = CaptureMode::MutBorrow;
                                    }
                                })
                                .or_insert((CaptureMode::MutBorrow, expr.span));
                        }
                    }
                }

                self.find_captures_in_expr_with_fields(left, local_bindings, var_uses, is_move);
                self.find_captures_in_expr_with_fields(right, local_bindings, var_uses, is_move);
            }

            // Function call - check func and args
            ExprKind::Call { func, args, .. } => {
                self.find_captures_in_expr_with_fields(func, local_bindings, var_uses, is_move);
                for arg in args {
                    self.find_captures_in_expr_with_fields(arg, local_bindings, var_uses, is_move);
                }
            }

            // Method call - check receiver and args
            ExprKind::MethodCall { receiver, args, .. } => {
                self.find_captures_in_expr_with_fields(receiver, local_bindings, var_uses, is_move);
                for arg in args {
                    self.find_captures_in_expr_with_fields(arg, local_bindings, var_uses, is_move);
                }
            }

            // Index access
            ExprKind::Index { expr: inner, index } => {
                self.find_captures_in_expr_with_fields(inner, local_bindings, var_uses, is_move);
                self.find_captures_in_expr_with_fields(index, local_bindings, var_uses, is_move);
            }

            // If expression
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                // Check condition
                for cond in &condition.conditions {
                    match cond {
                        verum_ast::expr::ConditionKind::Expr(e) => {
                            self.find_captures_in_expr_with_fields(
                                e,
                                local_bindings,
                                var_uses,
                                is_move,
                            );
                        }
                        verum_ast::expr::ConditionKind::Let { value, pattern, .. } => {
                            self.find_captures_in_expr_with_fields(
                                value,
                                local_bindings,
                                var_uses,
                                is_move,
                            );
                            // Note: pattern introduces new bindings, but only in then_branch
                        }
                    }
                }
                self.find_captures_in_block_with_fields(
                    then_branch,
                    local_bindings,
                    var_uses,
                    is_move,
                );
                if let verum_common::Maybe::Some(else_expr) = else_branch {
                    self.find_captures_in_expr_with_fields(
                        else_expr,
                        local_bindings,
                        var_uses,
                        is_move,
                    );
                }
            }

            // Match expression
            ExprKind::Match {
                expr: scrutinee,
                arms,
            } => {
                self.find_captures_in_expr_with_fields(
                    scrutinee,
                    local_bindings,
                    var_uses,
                    is_move,
                );
                for arm in arms {
                    // Pattern introduces new bindings
                    let mut arm_bindings = local_bindings.clone();
                    self.collect_capture_pattern_bindings(&arm.pattern, &mut arm_bindings);

                    if let verum_common::Maybe::Some(guard) = &arm.guard {
                        self.find_captures_in_expr_with_fields(
                            guard,
                            &arm_bindings,
                            var_uses,
                            is_move,
                        );
                    }
                    self.find_captures_in_expr_with_fields(
                        &arm.body,
                        &arm_bindings,
                        var_uses,
                        is_move,
                    );
                }
            }

            // Block expression
            ExprKind::Block(block) => {
                self.find_captures_in_block_with_fields(block, local_bindings, var_uses, is_move);
            }

            // Nested closure - don't recurse into it (it has its own capture set)
            ExprKind::Closure { .. } => {
                // Nested closures have their own captures; they capture from this closure's scope
                // For now, we just skip analyzing nested closures
            }

            // For loop
            ExprKind::For {
                pattern,
                iter,
                body,
                ..
            } => {
                self.find_captures_in_expr_with_fields(iter, local_bindings, var_uses, is_move);
                let mut loop_bindings = local_bindings.clone();
                self.collect_capture_pattern_bindings(pattern, &mut loop_bindings);
                self.find_captures_in_block_with_fields(body, &loop_bindings, var_uses, is_move);
            }

            // While loop
            ExprKind::While {
                condition, body, ..
            } => {
                self.find_captures_in_expr_with_fields(
                    condition,
                    local_bindings,
                    var_uses,
                    is_move,
                );
                self.find_captures_in_block_with_fields(body, local_bindings, var_uses, is_move);
            }

            // Loop
            ExprKind::Loop { body, .. } => {
                self.find_captures_in_block_with_fields(body, local_bindings, var_uses, is_move);
            }

            // Return
            ExprKind::Return(maybe_expr) => {
                if let verum_common::Maybe::Some(e) = maybe_expr {
                    self.find_captures_in_expr_with_fields(e, local_bindings, var_uses, is_move);
                }
            }

            // Tuple
            ExprKind::Tuple(elements) => {
                for elem in elements {
                    self.find_captures_in_expr_with_fields(elem, local_bindings, var_uses, is_move);
                }
            }

            // Array
            ExprKind::Array(array_expr) => {
                use verum_ast::expr::ArrayExpr;
                match array_expr {
                    ArrayExpr::List(elements) => {
                        for elem in elements {
                            self.find_captures_in_expr_with_fields(
                                elem,
                                local_bindings,
                                var_uses,
                                is_move,
                            );
                        }
                    }
                    ArrayExpr::Repeat { value, count } => {
                        self.find_captures_in_expr_with_fields(
                            value,
                            local_bindings,
                            var_uses,
                            is_move,
                        );
                        self.find_captures_in_expr_with_fields(
                            count,
                            local_bindings,
                            var_uses,
                            is_move,
                        );
                    }
                }
            }

            // Await
            ExprKind::Await(inner) => {
                self.find_captures_in_expr_with_fields(inner, local_bindings, var_uses, is_move);
            }

            // Record literal
            ExprKind::Record { fields, base, .. } => {
                for field in fields {
                    if let verum_common::Maybe::Some(ref value) = field.value {
                        self.find_captures_in_expr_with_fields(
                            value,
                            local_bindings,
                            var_uses,
                            is_move,
                        );
                    }
                }
                if let verum_common::Maybe::Some(base_expr) = base {
                    self.find_captures_in_expr_with_fields(
                        base_expr,
                        local_bindings,
                        var_uses,
                        is_move,
                    );
                }
            }

            // Range
            ExprKind::Range { start, end, .. } => {
                if let verum_common::Maybe::Some(s) = start {
                    self.find_captures_in_expr_with_fields(s, local_bindings, var_uses, is_move);
                }
                if let verum_common::Maybe::Some(e) = end {
                    self.find_captures_in_expr_with_fields(e, local_bindings, var_uses, is_move);
                }
            }

            // Cast
            ExprKind::Cast { expr: inner, .. } => {
                self.find_captures_in_expr_with_fields(inner, local_bindings, var_uses, is_move);
            }

            // Try
            ExprKind::Try(inner) => {
                self.find_captures_in_expr_with_fields(inner, local_bindings, var_uses, is_move);
            }

            // Other expressions that don't introduce captures or are handled elsewhere
            ExprKind::Literal(_)
            | ExprKind::Paren(_)
            | ExprKind::Break { .. }
            | ExprKind::Continue { .. } => {}

            // For any other expression kinds, recursively check sub-expressions
            _ => {}
        }
    }

    /// Find captures in a block (with field-level tracking)
    fn find_captures_in_block_with_fields(
        &self,
        block: &verum_ast::expr::Block,
        local_bindings: &std::collections::HashSet<Text>,
        var_uses: &mut std::collections::HashMap<
            (Text, Option<Text>),
            (crate::aliasing::CaptureMode, Span),
        >,
        is_move: bool,
    ) {
        let mut block_bindings = local_bindings.clone();

        for stmt in &block.stmts {
            self.find_captures_in_stmt_with_fields(stmt, &mut block_bindings, var_uses, is_move);
        }

        if let verum_common::Maybe::Some(final_expr) = &block.expr {
            self.find_captures_in_expr_with_fields(final_expr, &block_bindings, var_uses, is_move);
        }
    }

    /// Find captures in a statement (with field-level tracking)
    fn find_captures_in_stmt_with_fields(
        &self,
        stmt: &verum_ast::stmt::Stmt,
        local_bindings: &mut std::collections::HashSet<Text>,
        var_uses: &mut std::collections::HashMap<
            (Text, Option<Text>),
            (crate::aliasing::CaptureMode, Span),
        >,
        is_move: bool,
    ) {
        use verum_ast::stmt::StmtKind;

        match &stmt.kind {
            StmtKind::Let { pattern, value, .. } => {
                // First check the value (before adding bindings)
                if let verum_common::Maybe::Some(val) = value {
                    self.find_captures_in_expr_with_fields(val, local_bindings, var_uses, is_move);
                }
                // Then add the pattern bindings
                self.collect_capture_pattern_bindings(pattern, local_bindings);
            }
            StmtKind::LetElse {
                pattern,
                value,
                else_block,
                ..
            } => {
                self.find_captures_in_expr_with_fields(value, local_bindings, var_uses, is_move);
                self.find_captures_in_block_with_fields(
                    else_block,
                    local_bindings,
                    var_uses,
                    is_move,
                );
                self.collect_capture_pattern_bindings(pattern, local_bindings);
            }
            StmtKind::Expr { expr, .. } => {
                self.find_captures_in_expr_with_fields(expr, local_bindings, var_uses, is_move);
            }
            StmtKind::Defer(expr) => {
                self.find_captures_in_expr_with_fields(expr, local_bindings, var_uses, is_move);
            }
            StmtKind::Errdefer(expr) => {
                self.find_captures_in_expr_with_fields(expr, local_bindings, var_uses, is_move);
            }
            StmtKind::Provide { value, .. } => {
                self.find_captures_in_expr_with_fields(value, local_bindings, var_uses, is_move);
            }
            StmtKind::ProvideScope { value, block, .. } => {
                self.find_captures_in_expr_with_fields(value, local_bindings, var_uses, is_move);
                self.find_captures_in_expr_with_fields(block, local_bindings, var_uses, is_move);
            }
            StmtKind::Item(_) | StmtKind::Empty => {}
        }
    }

    /// Convert an expression to an EqTerm for type-level substitution.
    ///
    /// Equality types: propositional equality Eq<A, x, y> with reflexivity, symmetry, transitivity, substitution — Equality types and terms
    ///
    /// This is used for beta reduction in Pi types, where we need to substitute
    /// an argument value into the return type.
    pub(super) fn expr_to_eq_term(&self, expr: &Expr) -> Result<crate::ty::EqTerm> {
        use crate::ty::{EqConst, EqTerm};
        use verum_ast::expr::ExprKind;

        match &expr.kind {
            // Variable reference
            ExprKind::Path(path) => {
                if let Some(verum_ast::ty::PathSegment::Name(ident)) = path.segments.last() {
                    Ok(EqTerm::Var(ident.name.clone()))
                } else {
                    Err(TypeError::Other(verum_common::Text::from(
                        "Invalid path in dependent type",
                    )))
                }
            }

            // Literals
            ExprKind::Literal(lit) => {
                use verum_ast::literal::LiteralKind;
                match &lit.kind {
                    LiteralKind::Int(int_lit) => {
                        Ok(EqTerm::Const(EqConst::Int(int_lit.value as i64)))
                    }
                    LiteralKind::Bool(b) => Ok(EqTerm::Const(EqConst::Bool(*b))),
                    // Unit is represented as empty tuple in expressions, not a literal
                    _ => Err(TypeError::Other(verum_common::Text::from(
                        "Unsupported literal in dependent type",
                    ))),
                }
            }

            // Function application
            ExprKind::Call { func, args, .. } => {
                let func_term = self.expr_to_eq_term(func)?;
                let arg_terms: Result<List<EqTerm>> =
                    args.iter().map(|arg| self.expr_to_eq_term(arg)).collect();
                Ok(EqTerm::App {
                    func: Box::new(func_term),
                    args: arg_terms?,
                })
            }

            // Lambda
            ExprKind::Closure { params, body, .. } => {
                if params.len() != 1 {
                    return Err(TypeError::Other(verum_common::Text::from(
                        "Multi-parameter lambdas not yet supported in dependent types",
                    )));
                }

                let param_name = match &params[0].pattern.kind {
                    verum_ast::pattern::PatternKind::Ident { name, .. } => name.name.clone(),
                    _ => {
                        return Err(TypeError::Other(verum_common::Text::from(
                            "Complex patterns not supported in dependent type lambdas",
                        )));
                    }
                };

                let body_term = self.expr_to_eq_term(body)?;
                Ok(EqTerm::Lambda {
                    param: param_name,
                    body: Box::new(body_term),
                })
            }

            // Tuple/pair projection
            ExprKind::Field { expr: pair, field } => {
                let pair_term = self.expr_to_eq_term(pair)?;
                use crate::ty::ProjComponent;

                // Check if field is "fst", "first", "0" or "snd", "second", "1"
                let component = match field.name.as_str() {
                    "fst" | "first" | "0" => ProjComponent::Fst,
                    "snd" | "second" | "1" => ProjComponent::Snd,
                    _ => {
                        return Err(TypeError::Other(verum_common::Text::from(format!(
                            "Invalid projection field: {}",
                            field.name
                        ))));
                    }
                };

                Ok(EqTerm::Proj {
                    pair: Box::new(pair_term),
                    component,
                })
            }

            _ => Err(TypeError::Other(verum_common::Text::from(format!(
                "Expression not yet supported in dependent types: {}",
                expr_kind_description(&expr.kind)
            )))),
        }
    }

    /// Synthesize type for a dependent pair (Sigma type constructor).
    ///
    /// Sigma types (dependent pairs): (x: A, B(x)) where second component type depends on first value, refinement types desugar to Sigma — Sigma types
    ///
    /// Given a tuple (a, b) where the type of b depends on the value of a,
    /// we create a Sigma type: (x: A, B(x))
    fn synth_dependent_pair(
        &mut self,
        fst_expr: &Expr,
        snd_expr: &Expr,
        span: Span,
    ) -> Result<InferResult> {
        // Synthesize type of first component
        let fst_result = self.synth_expr(fst_expr)?;
        let fst_ty = fst_result.ty;

        // Try to extract a name for the first component (for dependent binding)
        let fst_name = match &fst_expr.kind {
            ExprKind::Path(path) => {
                if let Some(verum_ast::ty::PathSegment::Name(ident)) = path.segments.last() {
                    Some(ident.name.clone())
                } else {
                    None
                }
            }
            _ => None,
        };

        // Synthesize type of second component
        let snd_result = self.synth_expr(snd_expr)?;
        let snd_ty = snd_result.ty;

        // Check if second type depends on first value
        if let Some(ref name) = fst_name
            && self.type_contains_reference(&snd_ty, name)
        {
            // Dependent pair: (x: A, B(x))
            // Sigma types (dependent pairs): (x: A, B(x)) where second component type depends on first value, refinement types desugar to Sigma — Sigma type introduction
            let sigma_ty = Type::Sigma {
                fst_name: name.clone(),
                fst_type: Box::new(fst_ty),
                snd_type: Box::new(snd_ty),
            };
            return Ok(InferResult::new(sigma_ty));
        }

        // Regular tuple: (A, B)
        Ok(InferResult::new(Type::Tuple(vec![fst_ty, snd_ty].into())))
    }

    /// Project a component from a Sigma type.
    ///
    /// Sigma types (dependent pairs): (x: A, B(x)) where second component type depends on first value, refinement types desugar to Sigma — Sigma type elimination
    ///
    /// Given a value of type (x: A, B(x)):
    /// - fst : A
    /// - snd : B(fst)
    fn infer_sigma_projection(
        &mut self,
        pair_ty: &Type,
        component: crate::ty::ProjComponent,
        pair_expr: &Expr,
    ) -> Result<Type> {
        use crate::ty::ProjComponent;

        match pair_ty {
            Type::Sigma {
                fst_type,
                snd_type,
                fst_name: _,
            } => match component {
                ProjComponent::Fst => {
                    // First projection returns fst_type
                    Ok((**fst_type).clone())
                }
                ProjComponent::Snd => {
                    // Second projection returns snd_type with fst substituted
                    // We need to substitute the actual first component value
                    // For now, return snd_type as-is (conservative)
                    // A full implementation would extract and substitute the fst value
                    Ok((**snd_type).clone())
                }
            },

            Type::Tuple(tys) => {
                // Regular tuple projection
                let idx = match component {
                    ProjComponent::Fst => 0,
                    ProjComponent::Snd => 1,
                };
                if idx < tys.len() {
                    Ok(tys[idx].clone())
                } else {
                    Err(TypeError::Other(verum_common::Text::from(
                        "Tuple index out of bounds",
                    )))
                }
            }

            _ => Err(TypeError::Other(verum_common::Text::from(
                "Projection requires a tuple or Sigma type",
            ))),
        }
    }
}
