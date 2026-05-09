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
    is_stdlib_toplevel_path, span_to_line_col, expr_kind_description,
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

impl TypeChecker {
    /// Register a type declaration (type alias, ADT, etc.)
    /// This should be called before other passes to make types available.
    ///

    /// Relies on RUST_MIN_STACK=16MB for stack safety on deeply nested types.
    pub fn register_type_declaration(&mut self, type_decl: &verum_ast::TypeDecl) -> Result<()> {
        // #124 — primitive type names are reserved.
        //
        // A user (or stdlib) `type Unit is | UDays | UHours | …;` must not
        // shadow the primitive `Unit` (= `()`). Pre-fix this declaration
        // overwrote the canonical `define_type("Unit", Type::Unit)` from
        // bootstrap, so any subsequent `-> Unit` annotation in user code
        // resolved to the variant union and the type checker emitted
        // nonsense diagnostics like
        //   `expected 'UDays(Unit) | UHours(Unit) | …', found 'Unit'`
        // for the primitive return position.
        //
        // The list mirrors the existing `wkt_names` + the historical
        // grep-around for primitive names in this file (see line ~23668
        // and ~38602). Adding a new primitive to the language is a
        // language-level decision; any new entry here travels with the
        // matching `Type::Foo` constructor in `verum_types::ty::Type`.
        const PRIMITIVE_NAMES: &[&str] = &[
            "Int", "Float", "Bool", "Char", "Text", "Unit", "Never", "Byte",
        ];
        let raw_name = type_decl.name.name.as_str();
        if PRIMITIVE_NAMES.contains(&raw_name) {
            tracing::warn!(
                "type `{}` collides with primitive name and was skipped \
                 — rename to e.g. `{}_` to keep the declaration",
                raw_name,
                raw_name
            );
            return Ok(());
        }
        self.register_type_declaration_inner(type_decl)
    }

    fn register_type_declaration_inner(&mut self, type_decl: &verum_ast::TypeDecl) -> Result<()> {
        use crate::context::TypeScheme;
        use indexmap::IndexMap;
        use verum_ast::decl::{TypeDeclBody, VariantData};
        use verum_ast::ty::Path;
        use verum_common::Text;

        let type_name: Text = type_decl.name.name.as_str().into();
        // Save a copy for cleanup at end of function
        let type_name_for_cleanup = type_name.clone();

        // Track number of generic parameters for this type.
        // This enables proper type inference when the type is used without explicit type args.
        let generics_count = type_decl.generics.len();
        if generics_count > 0 {
            self.type_generics_count
                .insert(type_name.clone(), generics_count);
        }

        // CRITICAL: Prevent infinite recursion when registering mutually recursive types.
        // If we're already in the process of registering this type, return early.
        // The placeholder was already added, so lookups will find a Type::Var.
        if self.types_being_registered.contains(&type_name) {
            // Return early - a placeholder has already been registered
            return Ok(());
        }
        // Mark this type as being registered
        self.types_being_registered.insert(type_name.clone());

        // CRITICAL FIX: Execute the registration body and ensure cleanup happens on BOTH
        // success AND failure paths. Previously, errors via `?` would return early without
        // removing the type from `types_being_registered`, causing subsequent retry attempts
        // to short-circuit at the recursion guard above and return Ok(()) without actually
        // registering the type (including __type_var_order_ storage).
        //

        // We use a closure + match pattern to ensure cleanup happens regardless of result.
        let result = self.register_type_declaration_body(type_decl, type_name.clone());

        // Always cleanup on both success and failure (but NOT on recursion guard early return above)
        self.types_being_registered.remove(&type_name_for_cleanup);

        result
    }

    /// Inner body of register_type_declaration that may return early with errors.
    /// This is separated from register_type_declaration_inner to ensure proper cleanup
    /// of types_being_registered on both success and failure paths.
    fn register_type_declaration_body(
        &mut self,
        type_decl: &verum_ast::TypeDecl,
        type_name: verum_common::Text,
    ) -> Result<()> {
        use crate::context::TypeScheme;
        use indexmap::IndexMap;
        use verum_ast::decl::{TypeDeclBody, VariantData};
        use verum_ast::ty::Path;
        use verum_common::Text;

        // Register affine types for move semantics enforcement
        // Spec: L0-critical/reference_system/value_transfer - Affine type safety
        if let Some(verum_ast::decl::ResourceModifier::Affine) = &type_decl.resource_modifier {
            self.affine_tracker.register_affine_type(type_name.clone());
        }
        // Linear modifier — must consume exactly once.
        if let Some(verum_ast::decl::ResourceModifier::Linear) = &type_decl.resource_modifier {
            self.affine_tracker.register_linear_type(type_name.clone());
        }
        // `@must_consume` attribute — alias for `type linear`. Lets API
        // authors mark must-consume types with attribute syntax (which
        // survives derive expansion / macro re-export) instead of the
        // prefix `linear` keyword. Same compile-time effect: drop without
        // explicit consumption is a hard error.
        let must_consume = type_decl
            .attributes
            .iter()
            .any(|a| a.name.as_str() == "must_consume");
        if must_consume {
            self.affine_tracker.register_linear_type(type_name.clone());
        }

        // Save type parameter names so we can clean them up later
        // This prevents type parameter pollution across different type declarations
        let mut type_param_names = List::new();

        // Track type parameter names to TypeVars for proper polymorphic quantification
        // This mapping is used when registering variant constructors
        let mut type_param_vars: indexmap::IndexMap<Text, TypeVar> = indexmap::IndexMap::new();

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
        let is_variant_type = matches!(&type_decl.body, verum_ast::decl::TypeDeclBody::Variant(_));
        let use_type_vars = is_variant_type;

        for param in &type_decl.generics {
            use verum_ast::ty::GenericParamKind;

            let param_name: Text = match &param.kind {
                GenericParamKind::Type { name, .. } => name.name.as_str().into(),
                GenericParamKind::HigherKinded { name, .. } => name.name.as_str().into(),
                GenericParamKind::KindAnnotated { name, .. } => name.name.as_str().into(),
                GenericParamKind::Const { name, .. } => name.name.as_str().into(),
                GenericParamKind::Meta { name, .. } => name.name.as_str().into(),
                GenericParamKind::Lifetime { name } => name.name.as_str().into(),
                GenericParamKind::Context { name } => name.name.as_str().into(),
                GenericParamKind::Level { name } => name.name.as_str().into(),
            };

            if use_type_vars {
                // For variant types AND aliases: use Type::Var for proper polymorphic type inference
                // This ensures that variant constructors like Some(v) can unify T with Int
                // and that type aliases like IoResult<File> can substitute T with File
                let type_var = TypeVar::fresh();
                let param_type = Type::Var(type_var);
                self.ctx.define_type(param_name.clone(), param_type);
                type_param_vars.insert(param_name.clone(), type_var);
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
                self.ctx.define_type(param_name.clone(), param_type);
            }
            type_param_names.push(param_name);
        }

        // For simple type aliases, register the type in the environment
        // More complex types (records, variants) are handled during type checking
        match &type_decl.body {
            TypeDeclBody::Alias(aliased_type) => {
                // Register the type alias in the type context
                let resolved_type = self.ast_to_type(aliased_type)?;

                // CRITICAL: Store the RESOLVED type in the alias table, but store a NAMED type
                // reference in type_defs. This ensures that when `Reducer<Int, Int>` is looked up:
                // 1. resolve_type_name("Reducer") returns Type::Named { path: "Reducer", args: [] }
                // 2. ast_to_type adds the args to get Type::Named { path: "Reducer", args: [Int, Int] }
                // 3. normalize_type looks up the alias, builds substitution, and applies it
                self.ctx
                    .define_alias(type_name.clone(), resolved_type.clone());
                // Also register in the unifier for transparent alias unification
                self.unifier
                    .register_type_alias(type_name.clone(), resolved_type.clone());

                // Register type parameter names in the unifier for generic alias expansion.
                // This enables IoResult<Text> -> Result<Text, StreamError> during unification.
                if !type_decl.generics.is_empty() {
                    let alias_param_names: List<Text> = type_decl
                        .generics
                        .iter()
                        .filter_map(|param| {
                            use verum_ast::ty::GenericParamKind;
                            match &param.kind {
                                GenericParamKind::Type { name, .. } => Some(name.name.clone()),
                                _ => None,
                            }
                        })
                        .collect();
                    self.unifier
                        .register_type_alias_params(type_name.clone(), alias_param_names);
                }

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
                if !type_decl.generics.is_empty() {
                    let mut param_record: indexmap::IndexMap<verum_common::Text, Type> =
                        indexmap::IndexMap::new();
                    for param in type_decl.generics.iter() {
                        use verum_ast::ty::GenericParamKind;
                        // Record every positional parameter that carries a user-visible
                        // name — Type, HigherKinded, Const, Meta, Context, and Level.
                        // The arity counter (below, and the user-facing
                        // "type T expects N arguments" check in compile_type_path)
                        // reads `param_record.len()`; skipping Meta/Const/HKT/etc.
                        // made `type Matrix<T, Rows: meta Int, Cols: meta Int>`
                        // look like it only takes 1 argument, so `Matrix<Float, 2, 3>`
                        // raised "expects 1 type argument(s), but 3 were provided".
                        let name_opt = match &param.kind {
                            GenericParamKind::Type { name, .. } => Some(name.name.clone()),
                            GenericParamKind::HigherKinded { name, .. } => Some(name.name.clone()),
                            GenericParamKind::Const { name, .. } => Some(name.name.clone()),
                            GenericParamKind::Meta { name, .. } => Some(name.name.clone()),
                            GenericParamKind::Context { name } => Some(name.name.clone()),
                            GenericParamKind::Lifetime { .. } => None,
                            _ => None,
                        };
                        if let Some(n) = name_opt {
                            param_record.insert(n, Type::Int);
                        }
                    }
                    let type_params_key: Text = format!("__type_params_{}", type_name).into();
                    self.ctx
                        .define_type(type_params_key.clone(), Type::Record(param_record.clone()));
                    // Register kind for this type constructor (stdlib-agnostic)
                    let arity = param_record.len();
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
            TypeDeclBody::Variant(_) => self.register_variant_type_body(type_decl, &type_name, &type_param_vars, &type_param_names)?,
            TypeDeclBody::Record(_) => self.register_record_type_body(type_decl, &type_name)?,
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
                    self.ctx.define_type(type_name.clone(), newtype_ty.clone());

                    // Store inner type as Unit for coercion checks
                    let inner_key = format!("__newtype_inner_{}", type_name);
                    self.ctx.define_type(inner_key, Type::Unit);

                    // Register constructor that takes no arguments: Signal: fn() -> Signal
                    // This allows both `Signal()` and `()` coercion via newtype rules
                    let constructor_ty = Type::function(List::new(), newtype_ty);
                    self.ctx.env.insert_mono(type_name.as_str(), constructor_ty);
                } else if resolved_types.len() == 1 {
                    // Single-element "tuple" is a newtype with constructor
                    // type UserId is (Int); should allow UserId(42)
                    let inner_type = resolved_types.first().cloned().unwrap_or(Type::Unit);

                    // Create a Named type for the newtype (not an alias)
                    let newtype_ty = Type::Named {
                        path: Path::single(type_decl.name.clone()),
                        args: List::new(),
                    };
                    self.ctx.define_type(type_name.clone(), newtype_ty.clone());

                    // Store inner type for field access (.0)
                    let inner_key = format!("__newtype_inner_{}", type_name);
                    self.ctx.define_type(inner_key, inner_type.clone());

                    // Register constructor function: UserId: fn(Int) -> UserId
                    let constructor_ty =
                        Type::function(vec![inner_type].into_iter().collect(), newtype_ty);
                    self.ctx.env.insert_mono(type_name.as_str(), constructor_ty);
                } else {
                    // Multi-element tuple: create Named type with constructor
                    let named_tuple_ty = Type::Named {
                        path: Path::single(type_decl.name.clone()),
                        args: List::new(),
                    };
                    self.ctx
                        .define_type(type_name.clone(), named_tuple_ty.clone());

                    // Store tuple fields for field access (.0, .1, .2)
                    let tuple_fields_key = format!("__tuple_fields_{}", type_name);
                    self.ctx
                        .define_type(tuple_fields_key, Type::Tuple(resolved_types.clone()));

                    // Register constructor function: Color: fn(Int, Int, Int) -> Color
                    let constructor_ty = Type::function(resolved_types, named_tuple_ty);
                    self.ctx.env.insert_mono(type_name.as_str(), constructor_ty);
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
                self.ctx.define_type(type_name.clone(), newtype_ty.clone());

                // Store the inner type for field access (.0)
                let inner_key = format!("__newtype_inner_{}", type_name);
                self.ctx.define_type(inner_key, inner_resolved.clone());

                // Register a constructor function: UserId: fn(Int) -> UserId
                // This allows UserId(42) syntax
                let constructor_ty =
                    Type::function(vec![inner_resolved].into_iter().collect(), newtype_ty);
                self.ctx.env.insert_mono(type_name.as_str(), constructor_ty);
            }
            TypeDeclBody::Protocol(_) => self.register_protocol_type_body(type_decl, &type_name)?,
            TypeDeclBody::Unit => {
                // Unit type (e.g., `type Empty;` or `type Signal is ();`)
                // Register as Named type wrapping Unit
                let ty = Type::Named {
                    path: Path::single(type_decl.name.clone()),
                    args: List::new(),
                };
                self.ctx.define_type(type_name.clone(), ty.clone());

                // Store inner type as Unit for newtype coercion checks
                // This allows `let s: Signal = ();` without explicit wrapping
                let inner_key = format!("__newtype_inner_{}", type_name);
                self.ctx.define_type(inner_key, Type::Unit);
            }
            TypeDeclBody::Inductive(_) => {
                // Dependent type features (v2.0+) - register as named type for now
                let ty = Type::Named {
                    path: Path::single(type_decl.name.clone()),
                    args: List::new(),
                };
                self.ctx.define_type(type_name.clone(), ty);
            }
            TypeDeclBody::Coinductive(protocol_body) => {
                // Coinductive type — register named type and validate that the
                // protocol body declares at least one destructor (observation method).
                let ty = Type::Named {
                    path: Path::single(type_decl.name.clone()),
                    args: List::new(),
                };
                self.ctx.define_type(type_name.clone(), ty);

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
                self.ctx.define_type(type_name.clone(), ty.clone());

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
                    let struct_key = format!("__struct_fields_{}", type_name);
                    self.ctx.define_type(struct_key, Type::Record(fields));
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
                self.ctx
                    .define_alias(type_name.clone(), base_resolved.clone());
                self.unifier
                    .register_type_alias(type_name.clone(), base_resolved.clone());
                let ty = Type::Named {
                    path: Path::single(type_decl.name.clone()),
                    args: List::new(),
                };
                self.ctx.define_type(type_name.clone(), ty);
            }
        }

        // CRITICAL: Clean up type parameters from the type context
        // This prevents generic type parameters from one type (e.g., T from Maybe<T>)
        // from polluting the environment and interfering with other types
        for param_name in type_param_names {
            self.ctx.remove_type(&param_name);
        }

        // NOTE: Cleanup of types_being_registered is now done in register_type_declaration_inner
        // to ensure it happens on BOTH success and error paths. This enables retry after failed
        // registration attempts (e.g., when a dependency like List wasn't available initially).

        Ok(())
    }

    /// Register a sum (variant/enum) type declaration body.
    /// Handles import-provenance guards, placeholder registration for
    /// recursive types, variant constructor synthesis (with TypeVar substitution),
    /// and `__type_var_order_*` / `__type_params_*` metadata.
    fn register_variant_type_body(
        &mut self,
        type_decl: &verum_ast::TypeDecl,
        type_name: &verum_common::Text,
        type_param_vars: &indexmap::IndexMap<verum_common::Text, TypeVar>,
        type_param_names: &List<verum_common::Text>,
    ) -> Result<()> {
        use crate::context::TypeScheme;
        use indexmap::IndexMap;
        use verum_ast::decl::{TypeDeclBody, VariantData};
        use verum_ast::ty::Path;
        use verum_common::Text;
        let TypeDeclBody::Variant(variants) = &type_decl.body
            else { unreachable!() };
        let type_name = type_name.clone();
        let type_param_names = type_param_names.clone();
        let type_param_vars = type_param_vars.clone();
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
                            // Different variant structure — protect the explicitly imported type.
                            for param_name in &type_param_names {
                                self.ctx.remove_type(param_name);
                            }
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
                                    Err(_) => self.ast_to_type_lenient(&field.ty),
                                };
                                record_map.insert(field_name, field_type);
                            }
                            Type::Record(record_map)
                        }
                    };

                    // Register record variant fields for pattern matching + construction:
                    if let Type::Record(ref fields) = payload_type {
                        let struct_key = format!("__struct_fields_{}", variant_name);
                        self.ctx
                            .define_type(struct_key, Type::Record(fields.clone()));
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

                // CRITICAL: Store type parameter information for generic variant type substitution.
                // This enables proper substitution when using generic variant types like Validated<E, A>.
                // We store:
                // 1. __type_params_{name}: Parameter names in declaration order (for name-based substitution)
                // 2. __type_var_order_{name}: TypeVars in declaration order (for TypeVar-based substitution)
                if !type_param_vars.is_empty() {
                    // Store parameter names in order (like we do for record types)
                    let mut param_record: indexmap::IndexMap<verum_common::Text, Type> =
                        indexmap::IndexMap::new();
                    for (name, _) in &type_param_vars {
                        param_record.insert(name.clone(), Type::Int);
                    }
                    let type_params_key: Text = format!("__type_params_{}", type_name).into();
                    self.ctx
                        .define_type(type_params_key, Type::Record(param_record));

                    // CRITICAL: Store TypeVars in declaration order for proper substitution.
                    // When a Variant base type is instantiated with type args (e.g., Validated<E, A>),
                    // we need to map the TypeVars to their corresponding type args in the correct order.
                    // The type_param_vars map preserves declaration order (IndexMap), so we extract
                    // TypeVars in that order and store them as a Tuple type.
                    let type_vars_in_order: List<Type> =
                        type_param_vars.values().map(|tv| Type::Var(*tv)).collect();
                    let type_var_order_key: Text = format!("__type_var_order_{}", type_name).into();
                    self.ctx
                        .define_type(type_var_order_key, Type::Tuple(type_vars_in_order));
                }

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
                    let type_vars_in_order: List<TypeVar> =
                        type_param_vars.values().copied().collect();
                    self.unifier
                        .register_original_variant_type(type_name.clone(), variant_type.clone());
                    self.unifier
                        .register_type_var_order(type_name.clone(), type_vars_in_order);
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
                    let return_type = if type_param_vars.is_empty() {
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
                            args: type_param_vars.values().map(|tv| Type::Var(*tv)).collect(),
                        }
                    };

                    let constructor = InductiveConstructor {
                        name: ctor_name,
                        type_params: type_param_vars
                            .iter()
                            .map(|(n, tv)| (n.clone(), Box::new(Type::Var(*tv))))
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

                self.ctx
                    .register_inductive_type(type_name.clone(), inductive_constructors);

                // Register variant constructor parent mappings for scope-aware resolution.
                // This enables resolving ambiguous constructors like Ok(x) to the correct
                // parent type (Result vs CheckedResult) based on expected type context.
                for variant in variants.iter() {
                    let variant_name: Text = variant.name.name.as_str().into();
                    let parents = self
                        .variant_constructor_parents
                        .entry(variant_name)
                        .or_default();
                    // Avoid duplicate parent entries from re-registration
                    if !parents.iter().any(|p| p == &type_name) {
                        parents.push(type_name.clone());
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
                        // Unit variant: register as a value of the variant type
                        // e.g., Red : Color
                        // For generic types, we need to quantify over type variables
                        // e.g., None : ∀T. Maybe<T>
                        let free_vars = variant_type.free_vars();
                        if free_vars.is_empty() {
                            // Only register short name if it doesn't already exist
                            if !short_name_exists {
                                self.ctx
                                    .env
                                    .insert_mono(variant_name.clone(), variant_type.clone());
                            }

                            // Always register qualified name for Type.Variant syntax
                            let qualified_name: Text =
                                format!("{}.{}", type_name, variant_name).into();
                            self.ctx
                                .env
                                .insert_mono(qualified_name, variant_type.clone());
                        } else {
                            let vars_list: List<TypeVar> = free_vars.into_iter().collect();

                            // Only register short name if it doesn't already exist
                            if !short_name_exists {
                                self.ctx.env.insert(
                                    variant_name.clone(),
                                    TypeScheme::poly(vars_list.clone(), variant_type.clone()),
                                );
                            }
                            // Always register qualified name for Type.Variant syntax
                            let qualified_name: Text =
                                format!("{}.{}", type_name, variant_name).into();
                            self.ctx.env.insert(
                                qualified_name,
                                TypeScheme::poly(vars_list, variant_type.clone()),
                            );
                        }

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
                            format!("{}.{}", type_name, variant_name).into();
                        let qualified_variant_type = Type::Named {
                            path: Path::single(verum_ast::ty::Ident::new(
                                variant_name.as_str(),
                                Span::default(),
                            )),
                            args: List::new(),
                        };
                        self.ctx
                            .define_type(qualified_type_name, qualified_variant_type);
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
                        let free_vars = constructor_ty.free_vars();
                        if free_vars.is_empty() {
                            // Only register short name if it doesn't already exist
                            if !short_name_exists {
                                self.ctx
                                    .env
                                    .insert_mono(variant_name.clone(), constructor_ty.clone());
                            }
                            // Always register qualified name for Type.Variant syntax
                            let qualified_name: Text =
                                format!("{}.{}", type_name, variant_name).into();
                            self.ctx.env.insert_mono(qualified_name, constructor_ty);
                        } else {
                            let vars_list: List<TypeVar> = free_vars.into_iter().collect();

                            // Only register short name if it doesn't already exist
                            if !short_name_exists {
                                self.ctx.env.insert(
                                    variant_name.clone(),
                                    TypeScheme::poly(vars_list.clone(), constructor_ty.clone()),
                                );
                            }
                            // Always register qualified name for Type.Variant syntax
                            let qualified_name: Text =
                                format!("{}.{}", type_name, variant_name).into();
                            self.ctx.env.insert(
                                qualified_name,
                                TypeScheme::poly(vars_list, constructor_ty),
                            );
                        }

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
                            let qualified_type_name: Text =
                                format!("{}.{}", type_name, variant_name).into();
                            self.ctx
                                .define_type(qualified_type_name, payload_type.clone());
                        }
                    }
                }
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
        let type_name = type_name.clone();
                // CRITICAL: Register a placeholder type FIRST to handle recursive type definitions
                // and prevent infinite recursion when processing field types.
                // Example: type Node is { next: Maybe<Node> }
                // Without this placeholder, resolving Maybe<Node> would try to resolve Node again.
                let placeholder_var = TypeVar::fresh();
                self.ctx
                    .define_type(type_name.clone(), Type::Var(placeholder_var));

                // Convert record declarations to Type::Record for field type info
                // CRITICAL FIX: Use ast_to_type with fallback to ast_to_type_lenient.
                // When a type is imported from another module (e.g., DefectInfo from core.result),
                // its field types (e.g., Maybe<Text>) might not be available yet if their
                // defining modules haven't been fully processed. Using lenient resolution
                // ensures that the struct fields are always registered, even if some field
                // types are temporarily unresolved (they become type variables that get
                // resolved during unification).
                //

                // Example: `type DefectInfo is { location: Maybe<Text> }`
                // If Maybe isn't registered yet, ast_to_type would fail, preventing
                // __struct_fields_DefectInfo from being created. With lenient fallback,
                // Maybe<Text> becomes Named{Maybe, [Text]} or a type variable, and field
                // access will still work once Maybe is properly resolved.
                let mut record_map: IndexMap<Text, Type> = IndexMap::new();
                for field in fields {
                    let field_name: Text = field.name.name.as_str().into();
                    // Try strict resolution first, fall back to lenient
                    let field_type = match self.ast_to_type(&field.ty) {
                        Ok(ty) => ty,
                        Err(_) => {
                            // Lenient resolution: unresolved types become fresh type variables
                            // or Named types that will be resolved later
                            self.ast_to_type_lenient(&field.ty)
                        }
                    };
                    record_map.insert(field_name, field_type);
                }

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
                    let named_type = Type::Named {
                        path: Path::single(type_decl.name.clone()),
                        args: type_decl
                            .generics
                            .iter()
                            .map(|param| {
                                use verum_ast::ty::GenericParamKind;
                                match &param.kind {
                                    GenericParamKind::Type { name, .. } => Type::Named {
                                        path: Path::single(name.clone()),
                                        args: List::new(),
                                    },
                                    _ => Type::Unit,
                                }
                            })
                            .collect(),
                    };
                    // Store ONLY the named type, no __struct_fields_ entry
                    self.ctx.define_type(type_name.clone(), named_type);

                    // Skip field structure registration for empty records
                    // This prevents `{ }` from being used as the type in error messages
                } else {
                    // Non-empty record: Register as Named type with field structure

                    // IMPORTANT: Register as Named type to preserve nominal identity
                    // This allows protocol implementations to be looked up by name (e.g., "Point")
                    // rather than by structural type (e.g., "{ x: Int, y: Int }")
                    //

                    // The record structure is stored separately for field access validation
                    let named_type = Type::Named {
                        path: Path::single(type_decl.name.clone()),
                        args: type_decl
                            .generics
                            .iter()
                            .map(|param| {
                                use verum_ast::ty::GenericParamKind;
                                match &param.kind {
                                    GenericParamKind::Type { name, .. } => Type::Named {
                                        path: Path::single(name.clone()),
                                        args: List::new(),
                                    },
                                    _ => Type::Unit,
                                }
                            })
                            .collect(),
                    };

                    // Store the named type for protocol method dispatch
                    self.ctx.define_type(type_name.clone(), named_type.clone());

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
                                    params,
                                    return_type,
                                    contexts,
                                    type_params,
                                    properties,
                                } => Type::Function {
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
                    let struct_key: Text = format!("__struct_fields_{}", type_name).into();
                    self.ctx.define_type(struct_key, Type::Record(record_map));

                    // Store type parameter names for bidirectional inference.
                    // This allows us to substitute concrete types for generic parameters
                    // when checking record literals like `Box { value: 42 }` against `Box<Int>`,
                    // and the arity checker in `compile_type_path` uses `param_record.len()`
                    // to validate `expected_count == provided_count`.
                    //

                    // Every positional parameter must be included — not just `Type` kinds —
                    // otherwise a declaration like
                    //  type Matrix<T, Rows: meta Int, Cols: meta Int> is { ... };
                    // registers as arity=1 and every `Matrix<Float, 2, 3>` usage errors
                    // out with "expects 1 type argument(s), but 3 were provided".
                    if !type_decl.generics.is_empty() {
                        let mut param_record: indexmap::IndexMap<verum_common::Text, Type> =
                            indexmap::IndexMap::new();
                        for (idx, param) in type_decl.generics.iter().enumerate() {
                            let _ = idx;
                            use verum_ast::ty::GenericParamKind;
                            let name_opt = match &param.kind {
                                GenericParamKind::Type { name, .. } => Some(name.name.clone()),
                                GenericParamKind::HigherKinded { name, .. } => {
                                    Some(name.name.clone())
                                }
                                GenericParamKind::Const { name, .. } => Some(name.name.clone()),
                                GenericParamKind::Meta { name, .. } => Some(name.name.clone()),
                                GenericParamKind::Context { name } => Some(name.name.clone()),
                                GenericParamKind::Lifetime { .. } => None,
                                _ => None,
                            };
                            if let Some(n) = name_opt {
                                param_record.insert(n, Type::Int);
                            }
                        }
                        let type_params_key: Text = format!("__type_params_{}", type_name).into();
                        self.ctx
                            .define_type(type_params_key, Type::Record(param_record));
                    }
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
        let type_name = type_name.clone();
                // Protocol types (e.g., `type Database is protocol { ... }`)
                // Register as Named type for type checking
                let ty = Type::Named {
                    path: Path::single(type_decl.name.clone()),
                    args: List::new(),
                };
                self.ctx.define_type(type_name.clone(), ty);

                // CRITICAL FIX: Register protocol with its methods in protocol_checker
                // This enables method lookup for bounded generic types like `fn foo<T: Display>(item: T)`
                // Without this, calling protocol methods on bounded type parameters fails.
                use verum_ast::decl::ProtocolItemKind;
                let mut protocol_methods: Map<Text, crate::protocol::ProtocolMethod> = Map::new();
                let mut protocol_assoc_types: Map<Text, crate::protocol::AssociatedType> =
                    Map::new();

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

                for item in &protocol_body.items {
                    // Extract associated types (e.g., `type Item;` or `type Iterator: Iterator;`)
                    if let ProtocolItemKind::Type { name, bounds, .. } = &item.kind {
                        let assoc_name: Text = name.name.clone();

                        // Convert bounds to ProtocolBounds
                        let bound_list: List<crate::protocol::ProtocolBound> = bounds
                            .iter()
                            .filter_map(|bound_path| {
                                bound_path
                                    .as_ident()
                                    .map(|id| crate::protocol::ProtocolBound {
                                        protocol: Path::single(id.clone()),
                                        args: List::new(),
                                        is_negative: false,
                                    })
                            })
                            .collect();

                        let assoc_type =
                            crate::protocol::AssociatedType::simple(assoc_name.clone(), bound_list);
                        protocol_assoc_types.insert(assoc_name, assoc_type);
                    }

                    if let ProtocolItemKind::Function {
                        decl: method,
                        default_impl,
                    } = &item.kind
                    {
                        // CRITICAL FIX: Register method's generic type params BEFORE processing param types
                        // This allows A, B etc. to be resolved in `fn map<A, B>(self: Self.F<A>, ...) -> Self.F<B>`
                        // Higher-kinded types (HKTs): type constructors as first-class entities, kind inference (Type -> Type), HKT instantiation — Higher-kinded types in protocols
                        let mut method_type_param_names = List::new();
                        for generic_param in &method.generics {
                            use verum_ast::ty::GenericParamKind;
                            match &generic_param.kind {
                                GenericParamKind::Type { name, .. } => {
                                    let type_var = Type::Var(TypeVar::fresh());
                                    let name_text: Text = name.name.clone();
                                    self.ctx.define_type(name_text.clone(), type_var);
                                    method_type_param_names.push(name_text);
                                }
                                GenericParamKind::HigherKinded { name, arity, .. } => {
                                    use crate::advanced_protocols::Kind;
                                    let name_text: Text = name.name.clone();
                                    let kind = match *arity {
                                        0 => Kind::type_kind(),
                                        1 => Kind::unary_constructor(),
                                        2 => Kind::binary_constructor(),
                                        n => {
                                            let mut k = Kind::Type;
                                            for _ in 0..n {
                                                k = Kind::Arrow(Box::new(Kind::Type), Box::new(k));
                                            }
                                            k
                                        }
                                    };
                                    let type_constructor =
                                        Type::type_constructor(name_text.clone(), *arity, kind);
                                    self.ctx.define_type(name_text.clone(), type_constructor);
                                    method_type_param_names.push(name_text);
                                }
                                _ => {}
                            }
                        }

                        // Build method type from signature, EXCLUDING self parameter
                        // (it's implicit in method calls). Track receiver kind for
                        // object safety analysis.
                        let mut resolved_receiver_kind = verum_common::Maybe::None;
                        let param_types: Result<List<Type>> = method
                            .params
                            .iter()
                            .filter_map(|p| {
                                use verum_ast::decl::FunctionParamKind;
                                match &p.kind {
                                    FunctionParamKind::SelfRef
                                    | FunctionParamKind::SelfRefChecked
                                    | FunctionParamKind::SelfRefUnsafe => {
                                        resolved_receiver_kind = verum_common::Maybe::Some(
                                            crate::protocol::ReceiverKind::Ref,
                                        );
                                        None // Skip self - implicit
                                    }
                                    FunctionParamKind::SelfRefMut
                                    | FunctionParamKind::SelfRefCheckedMut
                                    | FunctionParamKind::SelfRefUnsafeMut => {
                                        resolved_receiver_kind = verum_common::Maybe::Some(
                                            crate::protocol::ReceiverKind::RefMut,
                                        );
                                        None // Skip self - implicit
                                    }
                                    FunctionParamKind::SelfValue
                                    | FunctionParamKind::SelfValueMut
                                    | FunctionParamKind::SelfOwn
                                    | FunctionParamKind::SelfOwnMut => {
                                        resolved_receiver_kind = verum_common::Maybe::Some(
                                            crate::protocol::ReceiverKind::Value,
                                        );
                                        None // Skip self - implicit
                                    }
                                    FunctionParamKind::Regular { ty, .. } => {
                                        Some(self.ast_to_type(ty))
                                    }
                                }
                            })
                            .collect();

                        if let Ok(params) = param_types {
                            let return_type = if let Some(ref ret_ty) = method.return_type {
                                self.ast_to_type(ret_ty).unwrap_or(Type::unit())
                            } else {
                                Type::unit()
                            };

                            let method_ty = Type::function(params, return_type);
                            let method_name: Text = method.name.name.clone();

                            // CRITICAL FIX: Check BOTH method.body AND default_impl for default implementation
                            // In protocol items, the default body may be in either location:
                            // - method.body: for regular function declarations
                            // - default_impl: for protocol method default implementations
                            let has_default = method.body.is_some() || default_impl.is_some();

                            let mut protocol_method = crate::protocol::ProtocolMethod::simple(
                                method_name.clone(),
                                method_ty,
                                has_default,
                            );
                            protocol_method.receiver_kind = resolved_receiver_kind;
                            // Record method-level type-param names so
                            // downstream substitution (e.g.
                            // lookup_all_protocol_methods) can exclude
                            // them from impl-level substitution and
                            // avoid shadow-replacing a method-scoped
                            // `F` with an impl-scoped `F` of the
                            // same spelling.
                            protocol_method.type_param_names = method_type_param_names.clone();
                            protocol_methods.insert(method_name, protocol_method);
                        }

                        // Clean up method-level type-param bindings so
                        // they don't leak into sibling methods or into
                        // the enclosing protocol / module scope.
                        for name in &method_type_param_names {
                            self.ctx.remove_type(name);
                        }
                    }
                }

                // Convert extends clause to super_protocols
                // This enables protocol inheritance: type Monad is protocol extends Applicative { ... }
                let super_protocols: List<crate::protocol::ProtocolBound> = protocol_body
                    .extends
                    .iter()
                    .filter_map(|extend_ty| {
                        // Extract protocol name from the type
                        match &extend_ty.kind {
                            verum_ast::ty::TypeKind::Path(path) => {
                                Some(crate::protocol::ProtocolBound {
                                    protocol: path.clone(),
                                    args: List::new(),
                                    is_negative: false,
                                })
                            }
                            verum_ast::ty::TypeKind::Generic { base, args } => {
                                // Handle generic extends like: extends Converter<A, B>
                                if let verum_ast::ty::TypeKind::Path(path) = &base.kind {
                                    let type_args: List<Type> = args
                                        .iter()
                                        .filter_map(|arg| {
                                            if let verum_ast::ty::GenericArg::Type(ty) = arg {
                                                self.ast_to_type(ty).ok()
                                            } else {
                                                None
                                            }
                                        })
                                        .collect();
                                    Some(crate::protocol::ProtocolBound {
                                        protocol: path.clone(),
                                        args: type_args,
                                        is_negative: false,
                                    })
                                } else {
                                    None
                                }
                            }
                            _ => None,
                        }
                    })
                    .collect();

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
                    name: type_name.clone(),
                    kind,
                    type_params: self.convert_generic_params_to_type_params(&type_decl.generics),
                    methods: protocol_methods.clone(),
                    associated_types: protocol_assoc_types,
                    associated_consts: Map::new(),
                    super_protocols,
                    specialization_info: Maybe::None,
                    defining_crate: Maybe::None,
                    span: type_decl.span,
                };
                let _ = self.protocol_checker.write().register_protocol(protocol);

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
                if protocol_body.is_context {
                    self.context_resolver
                        .register_protocol_as_context(type_name);
                } else {
                    // Register as constraint protocol for better error messages
                    self.context_resolver
                        .register_constraint_protocol(type_name);
                }

                // Restore the previous current_self_type
                self.set_current_self_type(old_self_type);
        Ok(())
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

    /// Pass 1: Register type name only (creates placeholder).
    ///

    /// This is the first pass of two-pass type resolution. It registers
    /// the type name as a placeholder, allowing forward references to work.
    ///

    /// # Example
    /// ```verum
    /// type SearchRequest is { sort_by: SortOrder };
    /// type SortOrder is Relevance | Downloads;
    /// ```
    ///

    /// After calling `register_type_name_only` for both types, `SearchRequest`
    /// can reference `SortOrder` even though it's defined later.
    pub fn register_type_name_only(&mut self, type_decl: &verum_ast::TypeDecl) {
        use verum_common::Text;

        let type_name: Text = type_decl.name.name.as_str().into();
        let span = type_decl.span;

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
        if self.user_code_phase && self.ctx.env.lookup(type_name.as_str()).is_some() {
            let should_evict = {
                use crate::ty::Type as T;
                matches!(
                    self.ctx.env.lookup(type_name.as_str()).map(|s| &s.ty),
                    Some(T::Function { .. }) | Some(T::Variant(_))
                )
            };
            if should_evict {
                let _ = self.ctx.env.remove(type_name.as_str());
            }
        }

        // Register as placeholder - will be resolved in pass 2
        let placeholder = Type::Placeholder {
            name: type_name.clone(),
            span,
        };
        self.ctx.define_type(type_name.clone(), placeholder);

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

        let type_name: Text = type_decl.name.name.as_str().into();
        let span = type_decl.span;

        // Register affine types for move semantics enforcement
        // Spec: L0-critical/reference_system/value_transfer - Affine type safety
        if let Some(verum_ast::decl::ResourceModifier::Affine) = &type_decl.resource_modifier {
            self.affine_tracker.register_affine_type(type_name.clone());
        }
        if let Some(verum_ast::decl::ResourceModifier::Linear) = &type_decl.resource_modifier {
            self.affine_tracker.register_linear_type(type_name.clone());
        }
        // `@must_consume` attribute — synonym for `type linear`, must be
        // registered in BOTH the primary and the resolution-loop pass to
        // avoid drift when types are visited recursively. See companion
        // primary-pass registration in register_type_declaration_body.
        let must_consume_2 = type_decl
            .attributes
            .iter()
            .any(|a| a.name.as_str() == "must_consume");
        if must_consume_2 {
            self.affine_tracker.register_linear_type(type_name.clone());
        }

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

        // Save type parameter names so we can clean them up later
        let mut type_param_names = List::new();

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
                GenericParamKind::Lifetime { name } => name.name.as_str().into(),
                GenericParamKind::Context { name } => name.name.as_str().into(),
                GenericParamKind::Level { name } => name.name.as_str().into(),
            };

            if is_variant_type {
                // For variant types: use Type::Var for proper polymorphic type inference
                // This ensures that variant constructors like Some(v) can unify T with Int
                let type_var = TypeVar::fresh();
                let param_type = Type::Var(type_var);
                self.ctx.define_type(param_name.clone(), param_type);
            } else {
                // For record types: use Named type for bidirectional type inference
                let param_type = Type::Named {
                    path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                        param_name.as_str(),
                        Span::default(),
                    )),
                    args: List::new(),
                };
                self.ctx.define_type(param_name.clone(), param_type);
            }
            type_param_names.push(param_name);
        }

        // CRITICAL FIX: Update __type_var_order_ with pass-2 TypeVars.
        // Pass 1 (register_type_declaration_body) stores __type_var_order_ with its own TypeVars.
        // Pass 2 (this function) creates NEW fresh TypeVars for the variant body.
        // If we don't update __type_var_order_, the stored TypeVars won't match the
        // variant body TypeVars, causing substitution failures in ast_to_type.
        // E.g., Validation<E, A> would get E and A swapped because pass-1 TypeVars
        // can't be found in the pass-2 variant body during apply_subst.
        if is_variant_type && !type_param_names.is_empty() {
            let mut new_type_vars: List<Type> = List::new();
            for param_name in &type_param_names {
                if let verum_common::Maybe::Some(Type::Var(tv)) = self.ctx.lookup_type(param_name) {
                    new_type_vars.push(Type::Var(*tv));
                }
            }
            if !new_type_vars.is_empty() {
                let type_var_order_key: Text = format!("__type_var_order_{}", type_name).into();
                self.ctx
                    .define_type(type_var_order_key, Type::Tuple(new_type_vars));
            }
        }

        // Now resolve the actual type definition (similar to register_type_declaration)
        let result = self.resolve_type_body(type_decl, &type_name);

        // Clean up type parameters
        for param_name in type_param_names {
            self.ctx.remove_type(&param_name);
        }

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

        match &type_decl.body {
            TypeDeclBody::Alias(aliased_type) => {
                let resolved_type = self.ast_to_type(aliased_type)?;
                // Register as alias for proper resolution (stores resolved type)
                self.ctx
                    .define_alias(type_name.clone(), resolved_type.clone());
                // Also register in the unifier for transparent alias unification
                self.unifier
                    .register_type_alias(type_name.clone(), resolved_type.clone());
                // Register type parameter names for generic alias expansion in unifier
                if !type_decl.generics.is_empty() {
                    let alias_param_names: List<Text> = type_decl
                        .generics
                        .iter()
                        .filter_map(|param| {
                            use verum_ast::ty::GenericParamKind;
                            match &param.kind {
                                GenericParamKind::Type { name, .. } => Some(name.name.clone()),
                                _ => None,
                            }
                        })
                        .collect();
                    self.unifier
                        .register_type_alias_params(type_name.clone(), alias_param_names);
                }
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

                // CRITICAL FIX: Register type parameters for generic type aliases.
                // Include EVERY positional parameter (Type, HigherKinded, Const,
                // Meta, Context, Level) — not just `Type` — because the arity
                // check in `compile_type_path` reads `param_record.len()` to
                // validate `expected_count == provided_count`. A declaration
                // like `type SquareMatrix<T, N: meta Int> is Matrix<T, N, N>`
                // otherwise registers arity=1 and `SquareMatrix<Float, 3>`
                // fails "expects 1 type argument(s), but 2 were provided".
                // Keep in sync with the matching loop in
                // `register_type_declaration_inner` (line ~46540).
                if !type_decl.generics.is_empty() {
                    use verum_ast::ty::GenericParamKind;
                    let mut param_record: indexmap::IndexMap<verum_common::Text, Type> =
                        indexmap::IndexMap::new();
                    for param in type_decl.generics.iter() {
                        let name_opt = match &param.kind {
                            GenericParamKind::Type { name, .. } => Some(name.name.clone()),
                            GenericParamKind::HigherKinded { name, .. } => Some(name.name.clone()),
                            GenericParamKind::KindAnnotated { name, .. } => Some(name.name.clone()),
                            GenericParamKind::Const { name, .. } => Some(name.name.clone()),
                            GenericParamKind::Meta { name, .. } => Some(name.name.clone()),
                            GenericParamKind::Context { name } => Some(name.name.clone()),
                            GenericParamKind::Lifetime { .. } => None,
                            _ => None,
                        };
                        if let Some(n) = name_opt {
                            param_record.insert(n, Type::Int);
                        }
                    }
                    let type_params_key: Text = format!("__type_params_{}", type_name).into();
                    self.ctx
                        .define_type(type_params_key, Type::Record(param_record));
                }
            }
            TypeDeclBody::Variant(_) => self.resolve_variant_type_body(type_decl, type_name)?,
            TypeDeclBody::Record(_) => self.resolve_record_type_body(type_decl, type_name)?,
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
                    self.ctx.define_type(type_name.clone(), newtype_ty.clone());

                    // Store inner type for field access (.0)
                    let inner_key = format!("__newtype_inner_{}", type_name);
                    self.ctx.define_type(inner_key, inner_type.clone());

                    // Register constructor function: UserId: fn(Int) -> UserId
                    let constructor_ty =
                        Type::function(vec![inner_type].into_iter().collect(), newtype_ty);
                    self.ctx.env.insert_mono(type_name.as_str(), constructor_ty);
                } else {
                    // Multi-element tuple: create Named type with constructor
                    let named_tuple_ty = Type::Named {
                        path: Path::single(type_decl.name.clone()),
                        args: List::new(),
                    };
                    self.ctx
                        .define_type(type_name.clone(), named_tuple_ty.clone());

                    // Store tuple fields for field access (.0, .1, .2)
                    let tuple_fields_key = format!("__tuple_fields_{}", type_name);
                    self.ctx
                        .define_type(tuple_fields_key, Type::Tuple(resolved_types.clone()));

                    // Register constructor function: Color: fn(Int, Int, Int) -> Color
                    let constructor_ty = Type::function(resolved_types, named_tuple_ty);
                    self.ctx.env.insert_mono(type_name.as_str(), constructor_ty);
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
                self.ctx.define_type(type_name.clone(), newtype_ty.clone());

                // Store the inner type for field access (.0)
                let inner_key = format!("__newtype_inner_{}", type_name);
                self.ctx.define_type(inner_key, inner_resolved.clone());

                // Register a constructor function: UserId: fn(Int) -> UserId
                let constructor_ty =
                    Type::function(vec![inner_resolved].into_iter().collect(), newtype_ty);
                self.ctx.env.insert_mono(type_name.as_str(), constructor_ty);
            }
            TypeDeclBody::Protocol(_) => self.resolve_protocol_type_body(type_decl, type_name)?,
            TypeDeclBody::Unit => {
                // Unit type (e.g., `type Empty;` or `type Signal is ();`)
                let ty = Type::Named {
                    path: Path::single(type_decl.name.clone()),
                    args: List::new(),
                };
                self.ctx.define_type(type_name.clone(), ty);

                // Store inner type as Unit for newtype coercion checks
                let inner_key = format!("__newtype_inner_{}", type_name);
                self.ctx.define_type(inner_key, Type::Unit);
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
                self.ctx.define_type(type_name.clone(), ty);

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
                    let struct_key = format!("__struct_fields_{}", type_name);
                    self.ctx.define_type(struct_key, Type::Record(fields));
                }
            }
            TypeDeclBody::Inductive(_) => {
                // Dependent type features (v2.0+) - register as named type for now
                let ty = Type::Named {
                    path: Path::single(type_decl.name.clone()),
                    args: List::new(),
                };
                self.ctx.define_type(type_name.clone(), ty);
            }
            TypeDeclBody::Coinductive(protocol_body) => {
                // Coinductive type (Pass 2) — resolve destructor return types and
                // verify each declared destructor has a valid return type.
                let ty = Type::Named {
                    path: Path::single(type_decl.name.clone()),
                    args: List::new(),
                };
                self.ctx.define_type(type_name.clone(), ty);

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
                self.ctx
                    .define_alias(type_name.clone(), base_resolved.clone());
                self.unifier
                    .register_type_alias(type_name.clone(), base_resolved.clone());
                // Replace the Pass-1 placeholder with a concrete Named
                // type so verify_no_placeholders sees a resolved entry.
                let named_type = Type::Named {
                    path: Path::single(type_decl.name.clone()),
                    args: List::new(),
                };
                self.ctx.define_type(type_name.clone(), named_type.clone());

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
                                let field_type = match self.ast_to_type(&field.ty) {
                                    Ok(ty) => ty,
                                    Err(_) => self.ast_to_type_lenient(&field.ty),
                                };
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
                        self.affine_tracker.register_affine_type(type_name.clone());
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
                        self.variant_record_fields
                            .insert(vname.clone(), fields.clone());
                    }
                }

                for (vname, vty) in &variant_map {
                    if let Type::Record(fields) = vty {
                        let ctor_type = Type::Function {
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

                    // CRITICAL FIX: Register original variant type and type var order with unifier
                    // For imports, try to look up existing type var order, or extract from variant type
                    let type_var_order_key: Text = format!("__type_var_order_{}", type_name).into();
                    if let verum_common::Maybe::Some(Type::Tuple(type_vars)) =
                        self.ctx.lookup_type(&type_var_order_key)
                    {
                        // Use existing type var order
                        let type_vars_in_order: List<TypeVar> = type_vars
                            .iter()
                            .filter_map(|t| {
                                if let Type::Var(tv) = t {
                                    Some(*tv)
                                } else {
                                    None
                                }
                            })
                            .collect();
                        self.unifier.register_original_variant_type(
                            type_name.clone(),
                            variant_type.clone(),
                        );
                        self.unifier
                            .register_type_var_order(type_name.clone(), type_vars_in_order);
                    } else {
                        // Fallback: Extract free vars from variant type (order may not be correct)
                        let free_vars = variant_type.free_vars();
                        if !free_vars.is_empty() {
                            let type_vars_in_order: List<TypeVar> = free_vars.into_iter().collect();
                            self.unifier.register_original_variant_type(
                                type_name.clone(),
                                variant_type.clone(),
                            );
                            self.unifier
                                .register_type_var_order(type_name.clone(), type_vars_in_order);
                        }
                    }
                }

                // Register variant constructor parent mappings for scope-aware resolution.
                for variant in variants.iter() {
                    let vn: Text = variant.name.name.as_str().into();
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
                        let free_vars = variant_type.free_vars();
                        if free_vars.is_empty() {
                            if !short_name_exists {
                                self.ctx
                                    .env
                                    .insert_mono(variant_name.clone(), variant_type.clone());
                            }
                            let qualified_name: Text =
                                format!("{}.{}", type_name, variant_name).into();
                            self.ctx
                                .env
                                .insert_mono(qualified_name, variant_type.clone());
                        } else {
                            let vars_list: List<TypeVar> = free_vars.into_iter().collect();
                            if !short_name_exists {
                                self.ctx.env.insert(
                                    variant_name.clone(),
                                    TypeScheme::poly(vars_list.clone(), variant_type.clone()),
                                );
                            }
                            let qualified_name: Text =
                                format!("{}.{}", type_name, variant_name).into();
                            self.ctx.env.insert(
                                qualified_name,
                                TypeScheme::poly(vars_list, variant_type.clone()),
                            );
                        }
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

                        let free_vars = constructor_ty.free_vars();
                        if free_vars.is_empty() {
                            if !short_name_exists {
                                self.ctx
                                    .env
                                    .insert_mono(variant_name.clone(), constructor_ty.clone());
                            }
                            let qualified_name: Text =
                                format!("{}.{}", type_name, variant_name).into();
                            self.ctx.env.insert_mono(qualified_name, constructor_ty);
                        } else {
                            let vars_list: List<TypeVar> = free_vars.into_iter().collect();
                            if !short_name_exists {
                                self.ctx.env.insert(
                                    variant_name.clone(),
                                    TypeScheme::poly(vars_list.clone(), constructor_ty.clone()),
                                );
                            }
                            let qualified_name: Text =
                                format!("{}.{}", type_name, variant_name).into();
                            self.ctx.env.insert(
                                qualified_name,
                                TypeScheme::poly(vars_list, constructor_ty),
                            );
                        }

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
        let type_name = type_name.clone();
                // CRITICAL: Register a placeholder type FIRST to handle recursive type definitions
                // and prevent infinite recursion when processing field types.
                let placeholder_var = TypeVar::fresh();
                // Module-aware: publish the placeholder under the qualified key
                // so same-named record types in different modules don't clobber
                // each other during recursive resolution.
                self.define_type_in_current_module(type_name.clone(), Type::Var(placeholder_var));

                // Use lenient resolution fallback for cross-module imports
                let mut record_map: IndexMap<Text, Type> = IndexMap::new();
                for field in fields {
                    let field_name: Text = field.name.name.as_str().into();
                    let field_type = match self.ast_to_type(&field.ty) {
                        Ok(ty) => ty,
                        Err(_) => self.ast_to_type_lenient(&field.ty),
                    };
                    record_map.insert(field_name, field_type);
                }

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
                    let named_type = Type::Named {
                        path: Path::single(type_decl.name.clone()),
                        args: type_decl
                            .generics
                            .iter()
                            .map(|param| {
                                use verum_ast::ty::GenericParamKind;
                                match &param.kind {
                                    GenericParamKind::Type { name, .. } => Type::Named {
                                        path: Path::single(name.clone()),
                                        args: List::new(),
                                    },
                                    _ => Type::Unit,
                                }
                            })
                            .collect(),
                    };
                    // Module-aware: empty record registration.
                    self.define_type_in_current_module(type_name.clone(), named_type);

                    // CRITICAL: Also register empty struct fields for pattern matching
                    // Empty record allows `match e { Empty {} => ... }` patterns
                    let struct_key: Text = format!("__struct_fields_{}", type_name).into();
                    self.ctx.define_type(struct_key, Type::Record(record_map));
                } else {
                    // Non-empty record: Register as Named type with field structure

                    // Register as Named type for nominal identity
                    let named_type = Type::Named {
                        path: Path::single(type_decl.name.clone()),
                        args: type_decl
                            .generics
                            .iter()
                            .map(|param| {
                                use verum_ast::ty::GenericParamKind;
                                match &param.kind {
                                    GenericParamKind::Type { name, .. } => Type::Named {
                                        path: Path::single(name.clone()),
                                        args: List::new(),
                                    },
                                    _ => Type::Unit,
                                }
                            })
                            .collect(),
                    };

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
                                    params,
                                    return_type,
                                    contexts,
                                    type_params,
                                    properties,
                                } => Type::Function {
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
                    let struct_key: Text = format!("__struct_fields_{}", type_name).into();
                    self.ctx.define_type(struct_key, Type::Record(record_map));

                    // Store type parameters for bidirectional inference.
                    // Count every positional kind (Type, HigherKinded, Const, Meta,
                    // Context), not just `Type`. The arity check in compile_type_path
                    // reads `param_record.len()`; filtering down to `Type` broke
                    // declarations like
                    //  type Matrix<T, Rows: meta Int, Cols: meta Int> is { … };
                    // where the Meta slots would otherwise vanish and
                    // `Matrix<Float, 2, 3>` reported "expects 1 type argument, got 3".
                    if !type_decl.generics.is_empty() {
                        let mut param_record: indexmap::IndexMap<verum_common::Text, Type> =
                            indexmap::IndexMap::new();
                        for param in type_decl.generics.iter() {
                            use verum_ast::ty::GenericParamKind;
                            let name_opt = match &param.kind {
                                GenericParamKind::Type { name, .. } => Some(name.name.clone()),
                                GenericParamKind::HigherKinded { name, .. } => {
                                    Some(name.name.clone())
                                }
                                GenericParamKind::Const { name, .. } => Some(name.name.clone()),
                                GenericParamKind::Meta { name, .. } => Some(name.name.clone()),
                                GenericParamKind::Context { name } => Some(name.name.clone()),
                                GenericParamKind::Lifetime { .. } => None,
                                _ => None,
                            };
                            if let Some(n) = name_opt {
                                param_record.insert(n, Type::Int);
                            }
                        }
                        let type_params_key: Text = format!("__type_params_{}", type_name).into();
                        self.ctx
                            .define_type(type_params_key, Type::Record(param_record));
                    }
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
        let type_name = type_name.clone();
                // Protocol types are registered as Named types
                let ty = Type::Named {
                    path: Path::single(type_decl.name.clone()),
                    args: List::new(),
                };
                self.ctx.define_type(type_name.clone(), ty);

                // CRITICAL FIX: Register protocol with its methods in protocol_checker
                // This enables method lookup for bounded generic types like `fn foo<T: Display>(item: T)`
                // Without this, calling protocol methods on bounded type parameters fails.
                use verum_ast::decl::ProtocolItemKind;
                let mut protocol_methods: Map<Text, crate::protocol::ProtocolMethod> = Map::new();
                let mut protocol_assoc_types: Map<Text, crate::protocol::AssociatedType> =
                    Map::new();

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

                for item in &protocol_body.items {
                    // Extract associated types (e.g., `type Item;` or `type Iterator: Iterator;`)
                    if let ProtocolItemKind::Type { name, bounds, .. } = &item.kind {
                        let assoc_name: Text = name.name.clone();

                        // Convert bounds to ProtocolBounds
                        let bound_list: List<crate::protocol::ProtocolBound> = bounds
                            .iter()
                            .filter_map(|bound_path| {
                                bound_path
                                    .as_ident()
                                    .map(|id| crate::protocol::ProtocolBound {
                                        protocol: Path::single(id.clone()),
                                        args: List::new(),
                                        is_negative: false,
                                    })
                            })
                            .collect();

                        let assoc_type =
                            crate::protocol::AssociatedType::simple(assoc_name.clone(), bound_list);
                        protocol_assoc_types.insert(assoc_name, assoc_type);
                    }

                    if let ProtocolItemKind::Function {
                        decl: method,
                        default_impl,
                    } = &item.kind
                    {
                        // CRITICAL FIX: Register method's generic type params BEFORE processing param types
                        // This allows A, B etc. to be resolved in `fn map<A, B>(self: Self.F<A>, ...) -> Self.F<B>`
                        // Higher-kinded types (HKTs): type constructors as first-class entities, kind inference (Type -> Type), HKT instantiation — Higher-kinded types in protocols
                        let mut method_type_param_names = List::new();
                        for generic_param in &method.generics {
                            use verum_ast::ty::GenericParamKind;
                            match &generic_param.kind {
                                GenericParamKind::Type { name, .. } => {
                                    let type_var = Type::Var(TypeVar::fresh());
                                    let name_text: Text = name.name.clone();
                                    self.ctx.define_type(name_text.clone(), type_var);
                                    method_type_param_names.push(name_text);
                                }
                                GenericParamKind::HigherKinded { name, arity, .. } => {
                                    use crate::advanced_protocols::Kind;
                                    let name_text: Text = name.name.clone();
                                    let kind = match *arity {
                                        0 => Kind::type_kind(),
                                        1 => Kind::unary_constructor(),
                                        2 => Kind::binary_constructor(),
                                        n => {
                                            let mut k = Kind::Type;
                                            for _ in 0..n {
                                                k = Kind::Arrow(Box::new(Kind::Type), Box::new(k));
                                            }
                                            k
                                        }
                                    };
                                    let type_constructor =
                                        Type::type_constructor(name_text.clone(), *arity, kind);
                                    self.ctx.define_type(name_text.clone(), type_constructor);
                                    method_type_param_names.push(name_text);
                                }
                                _ => {}
                            }
                        }

                        // Build method type from signature, EXCLUDING self parameter
                        // (it's implicit in method calls). Track receiver kind for
                        // object safety analysis.
                        let mut resolved_receiver_kind = verum_common::Maybe::None;
                        let param_types: Result<List<Type>> = method
                            .params
                            .iter()
                            .filter_map(|p| {
                                use verum_ast::decl::FunctionParamKind;
                                match &p.kind {
                                    FunctionParamKind::SelfRef
                                    | FunctionParamKind::SelfRefChecked
                                    | FunctionParamKind::SelfRefUnsafe => {
                                        resolved_receiver_kind = verum_common::Maybe::Some(
                                            crate::protocol::ReceiverKind::Ref,
                                        );
                                        None // Skip self - implicit
                                    }
                                    FunctionParamKind::SelfRefMut
                                    | FunctionParamKind::SelfRefCheckedMut
                                    | FunctionParamKind::SelfRefUnsafeMut => {
                                        resolved_receiver_kind = verum_common::Maybe::Some(
                                            crate::protocol::ReceiverKind::RefMut,
                                        );
                                        None // Skip self - implicit
                                    }
                                    FunctionParamKind::SelfValue
                                    | FunctionParamKind::SelfValueMut
                                    | FunctionParamKind::SelfOwn
                                    | FunctionParamKind::SelfOwnMut => {
                                        resolved_receiver_kind = verum_common::Maybe::Some(
                                            crate::protocol::ReceiverKind::Value,
                                        );
                                        None // Skip self - implicit
                                    }
                                    FunctionParamKind::Regular { ty, .. } => {
                                        Some(self.ast_to_type(ty))
                                    }
                                }
                            })
                            .collect();

                        if let Ok(params) = param_types {
                            let return_type = if let Some(ref ret_ty) = method.return_type {
                                self.ast_to_type(ret_ty).unwrap_or(Type::unit())
                            } else {
                                Type::unit()
                            };

                            let method_ty = Type::function(params, return_type);
                            let method_name: Text = method.name.name.clone();

                            // CRITICAL FIX: Check BOTH method.body AND default_impl for default implementation
                            // In protocol items, the default body may be in either location:
                            // - method.body: for regular function declarations
                            // - default_impl: for protocol method default implementations
                            let has_default = method.body.is_some() || default_impl.is_some();

                            let mut protocol_method = crate::protocol::ProtocolMethod::simple(
                                method_name.clone(),
                                method_ty,
                                has_default,
                            );
                            protocol_method.receiver_kind = resolved_receiver_kind;
                            // Record method-level type-param names so
                            // downstream substitution (e.g.
                            // lookup_all_protocol_methods) can exclude
                            // them from impl-level substitution and
                            // avoid shadow-replacing a method-scoped
                            // `F` with an impl-scoped `F` of the
                            // same spelling.
                            protocol_method.type_param_names = method_type_param_names.clone();
                            protocol_methods.insert(method_name, protocol_method);
                        }

                        // Clean up method-level type-param bindings so
                        // they don't leak into sibling methods or into
                        // the enclosing protocol / module scope.
                        for name in &method_type_param_names {
                            self.ctx.remove_type(name);
                        }
                    }
                }

                // Convert extends clause to super_protocols (second pass)
                let super_protocols: List<crate::protocol::ProtocolBound> = protocol_body
                    .extends
                    .iter()
                    .filter_map(|extend_ty| match &extend_ty.kind {
                        verum_ast::ty::TypeKind::Path(path) => {
                            Some(crate::protocol::ProtocolBound {
                                protocol: path.clone(),
                                args: List::new(),
                                is_negative: false,
                            })
                        }
                        verum_ast::ty::TypeKind::Generic { base, args } => {
                            if let verum_ast::ty::TypeKind::Path(path) = &base.kind {
                                let type_args: List<Type> = args
                                    .iter()
                                    .filter_map(|arg| {
                                        if let verum_ast::ty::GenericArg::Type(ty) = arg {
                                            self.ast_to_type(ty).ok()
                                        } else {
                                            None
                                        }
                                    })
                                    .collect();
                                Some(crate::protocol::ProtocolBound {
                                    protocol: path.clone(),
                                    args: type_args,
                                    is_negative: false,
                                })
                            } else {
                                None
                            }
                        }
                        _ => None,
                    })
                    .collect();

                // Create and register Protocol object
                // Convert AST is_context bool to ProtocolKind
                let kind = if protocol_body.is_context {
                    crate::protocol::ProtocolKind::ConstraintAndInjectable
                } else {
                    crate::protocol::ProtocolKind::Constraint
                };

                let protocol = crate::protocol::Protocol {
                    name: type_name.clone(),
                    kind,
                    type_params: self.convert_generic_params_to_type_params(&type_decl.generics),
                    methods: protocol_methods.clone(),
                    associated_types: protocol_assoc_types,
                    associated_consts: Map::new(),
                    super_protocols,
                    specialization_info: Maybe::None,
                    defining_crate: Maybe::None,
                    span: type_decl.span,
                };
                let _ = self.protocol_checker.write().register_protocol(protocol);

                // Register as context or constraint protocol based on is_context flag
                if protocol_body.is_context {
                    self.context_resolver
                        .register_protocol_as_context(type_name.clone());
                } else {
                    // Register as constraint protocol for better error messages
                    self.context_resolver
                        .register_constraint_protocol(type_name.clone());
                }

                // Restore the previous current_self_type
                self.set_current_self_type(old_self_type);
        Ok(())
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
            // Placeholder type - this is an error!
            Placeholder { name, span } => {
                // Avoid duplicate errors for the same placeholder
                if !visited.contains(name) {
                    visited.insert(name.clone());
                    errors.push(crate::TypeError::UnresolvedPlaceholder {
                        name: name.clone(),
                        span: *span,
                    });
                }
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

    /// Register a protocol declaration from ItemKind::Protocol (context protocol syntax).
    /// Converts the ProtocolDecl into the same type system entities as TypeDeclBody::Protocol.
    pub(super) fn register_protocol_decl_item(&mut self, proto_decl: &verum_ast::ProtocolDecl) -> Result<()> {
        let type_name: verum_common::Text = proto_decl.name.name.as_str().into();

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
            for generic in proto_decl.generics.iter() {
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
            for item in proto_decl.items.iter() {
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

        // Register type name
        self.ctx.define_type(
            type_name.clone(),
            Type::Named {
                path: verum_ast::ty::Path::single(proto_decl.name.clone()),
                args: verum_common::List::new(),
            },
        );

        // Convert protocol methods to type system methods
        let mut protocol_methods: verum_common::Map<
            verum_common::Text,
            crate::protocol::ProtocolMethod,
        > = verum_common::Map::new();

        for item in proto_decl.items.iter() {
            if let verum_ast::decl::ProtocolItemKind::Function {
                decl: func,
                default_impl,
            } = &item.kind
            {
                let method_name: verum_common::Text = func.name.name.as_str().into();
                let self_type = Type::Named {
                    path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                        "Self",
                        Span::default(),
                    )),
                    args: verum_common::List::new(),
                };
                // Track receiver kind for object safety checks
                let mut method_receiver_kind = verum_common::Maybe::None;

                // Register method-level generic params as fresh TypeVars
                // BEFORE lowering the signature. Without this step,
                // `fn map<B, F: fn(Self.Item) -> B>` lowers to a
                // signature with Type::Named("B") / Type::Named("F")
                // — and later `substitute_type_params` during
                // `lookup_all_protocol_methods` cannot tell these
                // names apart from *impl-level* generics that happen
                // to share spelling. For instance, a receiver of
                // `MappedIter<I, F>` binds impl's `F` → closure_ty
                // in the subst map, and then shadow-substitutes
                // the method's unrelated `F` parameter to that
                // same closure_ty. The observable failure was
                // `once_with(|| 5).map(|x| x*10).next()` typing as
                // `fn(Int) -> Int` instead of `Int`.
                //

                // Using a fresh TypeVar gives each method-level
                // generic an ID-based identity; substitute_type_params
                // matches Named-by-name and Var-by-id, so method vars
                // are structurally disjoint from impl vars even when
                // the written names coincide.
                self.ctx.enter_scope();
                let mut method_param_names: verum_common::List<verum_common::Text> =
                    verum_common::List::new();
                let mut method_param_bounds: verum_common::Map<verum_common::Text, Type> =
                    verum_common::Map::new();
                for generic_param in &func.generics {
                    use verum_ast::ty::GenericParamKind;
                    if let GenericParamKind::Type { name, bounds, .. } = &generic_param.kind {
                        let tv = crate::ty::TypeVar::fresh();
                        let name_text: verum_common::Text = name.name.clone();
                        self.ctx.define_type(name_text.clone(), Type::Var(tv));
                        method_param_names.push(name_text.clone());
                        // Capture function-type bounds (F: fn(X) -> Y)
                        // so that closure type inference can consult them
                        // during method calls.
                        for bound in bounds {
                            if let verum_ast::ty::TypeBoundKind::Equality(bt)
                            | verum_ast::ty::TypeBoundKind::GenericProtocol(bt) = &bound.kind
                            {
                                if let verum_ast::ty::TypeKind::Function { .. } = &bt.kind {
                                    if let Ok(bound_ty) = self.ast_to_type(bt) {
                                        method_param_bounds.insert(name_text.clone(), bound_ty);
                                    }
                                }
                            }
                        }
                    }
                }

                // Collect only non-self parameters (self is implicit in method calls)
                let params: verum_common::List<Type> = func
                    .params
                    .iter()
                    .filter_map(|p| {
                        match &p.kind {
                            verum_ast::decl::FunctionParamKind::SelfRef => {
                                method_receiver_kind =
                                    verum_common::Maybe::Some(crate::protocol::ReceiverKind::Ref);
                                None // Skip self param - it's implicit
                            }
                            verum_ast::decl::FunctionParamKind::SelfRefMut => {
                                method_receiver_kind = verum_common::Maybe::Some(
                                    crate::protocol::ReceiverKind::RefMut,
                                );
                                None // Skip self param - it's implicit
                            }
                            verum_ast::decl::FunctionParamKind::SelfValue
                            | verum_ast::decl::FunctionParamKind::SelfValueMut => {
                                method_receiver_kind =
                                    verum_common::Maybe::Some(crate::protocol::ReceiverKind::Value);
                                None // Skip self param - it's implicit
                            }
                            verum_ast::decl::FunctionParamKind::Regular { ty, .. } => {
                                self.ast_to_type(ty).ok()
                            }
                            _ => Some(Type::Var(crate::ty::TypeVar::fresh())),
                        }
                    })
                    .collect();
                let return_type = if let verum_common::Maybe::Some(ref ret) = func.return_type {
                    self.ast_to_type(ret).unwrap_or(Type::Unit)
                } else {
                    Type::Unit
                };
                let method_ty = Type::Function {
                    params,
                    return_type: Box::new(return_type),
                    properties: None,
                    contexts: None,
                    type_params: verum_common::List::new(),
                };
                // Exit method scope — impl callers won't accidentally see
                // these TypeVars in lookup_type, so they can't be shadowed
                // or reused.
                self.ctx.exit_scope();

                let mut pm = crate::protocol::ProtocolMethod::simple(
                    method_name.clone(),
                    method_ty,
                    default_impl.is_some(),
                );
                pm.receiver_kind = method_receiver_kind;
                pm.type_param_names = method_param_names;
                pm.type_param_bounds = method_param_bounds;
                protocol_methods.insert(method_name, pm);
            }
        }

        // Determine protocol kind from is_context flag
        let kind = if proto_decl.is_context {
            crate::protocol::ProtocolKind::ConstraintAndInjectable
        } else {
            crate::protocol::ProtocolKind::Constraint
        };

        // Register protocol
        let protocol = crate::protocol::Protocol {
            name: type_name.clone(),
            kind,
            type_params: self.convert_generic_params_to_type_params(&proto_decl.generics),
            methods: protocol_methods,
            associated_types: verum_common::Map::new(),
            associated_consts: verum_common::Map::new(),
            super_protocols: proto_decl
                .bounds
                .iter()
                .filter_map(|b| {
                    if let verum_ast::ty::TypeKind::Path(path) = &b.kind {
                        Some(crate::protocol::ProtocolBound {
                            protocol: path.clone(),
                            args: verum_common::List::new(),
                            is_negative: false,
                        })
                    } else {
                        None
                    }
                })
                .collect(),
            specialization_info: verum_common::Maybe::None,
            defining_crate: verum_common::Maybe::None,
            span: proto_decl.span,
        };
        let _ = self.protocol_checker.write().register_protocol(protocol);

        // Object safety enforcement for context protocols.
        // Context protocols must be object-safe because they're used via dynamic dispatch
        // in the DI system (vtable-based runtime resolution).
        // Check: methods must not return Self, must not have generic type params.
        if proto_decl.is_context {
            for item in proto_decl.items.iter() {
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

        // Auto-register context protocols as injectable
        if proto_decl.is_context {
            self.context_resolver
                .register_protocol_as_context(type_name.clone());

            // CRITICAL: Also register with context_checker by creating a synthetic ContextDecl.
            // This ensures `using [Serializable]` can find the context when it checks
            // context_checker.declarations.
            let synthetic_methods: List<verum_ast::decl::FunctionDecl> = proto_decl
                .items
                .iter()
                .filter_map(|item| {
                    if let verum_ast::decl::ProtocolItemKind::Function { decl, .. } = &item.kind {
                        Some(decl.clone())
                    } else {
                        None
                    }
                })
                .collect();
            let synthetic_context = verum_ast::decl::ContextDecl {
                visibility: proto_decl.visibility.clone(),
                is_async: false,
                name: proto_decl.name.clone(),
                generics: proto_decl.generics.clone(),
                methods: synthetic_methods,
                associated_types: List::new(),
                associated_consts: List::new(),
                sub_contexts: List::new(),
                span: proto_decl.span,
            };
            self.context_declarations
                .insert(type_name.clone(), synthetic_context.clone());
            self.context_checker
                .register_context(type_name.clone(), synthetic_context);
        } else {
            self.context_resolver
                .register_constraint_protocol(type_name.clone());
        }

        // Register Kind
        {
            use crate::kind_inference::KindInference;
            let protocol_kind = if proto_decl.is_context {
                crate::kind_inference::Kind::ConstraintAndInjectable
            } else {
                crate::kind_inference::Kind::Constraint
            };
            self.kind_inferer()
                .register_type_constructor(type_name.as_str(), protocol_kind);
        }

        Ok(())
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

    /// Relies on RUST_MIN_STACK=16MB for stack safety on deep recursion.
    pub fn register_impl_block(&mut self, impl_decl: &verum_ast::decl::ImplDecl) -> Result<()> {
        self.register_impl_block_inner(impl_decl)
    }

    /// Inner implementation of register_impl_block
    fn register_impl_block_inner(&mut self, impl_decl: &verum_ast::decl::ImplDecl) -> Result<()> {
        use verum_ast::decl::{FunctionParamKind, ImplItemKind, ImplKind};

        if let ImplKind::Protocol {
            protocol, for_type, ..
        } = &impl_decl.kind
        {
            let proto_name = protocol
                .as_ident()
                .map(|i| i.as_str().to_string())
                .unwrap_or_default();
            if proto_name == "Into" || proto_name.contains("Into") {
                // #[cfg(debug_assertions)]
                // eprintln!("[DEBUG impl_block_inner] Processing Into impl, for_type AST: {:?}", for_type);
                // #[cfg(debug_assertions)]
                // eprintln!("[DEBUG impl_block_inner] Generics: {:?}", impl_decl.generics.iter().map(|g| {
                // if let verum_ast::ty::GenericParamKind::Type { name, .. } = &g.kind {
                // name.name.as_str().to_string()
                // } else {
                // "?".to_string()
                // }
                // }).collect::<Vec<_>>());
            }
        }

        // Register generic type parameters from the impl block
        let mut type_param_names = List::new();
        for generic_param in &impl_decl.generics {
            use verum_ast::ty::GenericParamKind;
            match &generic_param.kind {
                GenericParamKind::Type { name, .. } => {
                    let type_var = Type::Var(TypeVar::fresh());
                    let name_text: Text = name.name.clone();
                    self.ctx.define_type(name_text.clone(), type_var);
                    type_param_names.push(name_text);
                }
                GenericParamKind::Const { name, ty } => {
                    // For const generics like `const SIZE: Int`, register them as their declared type.
                    // This prevents "type not found" errors when used in type arguments like StackAllocator<SIZE>.
                    let name_text: Text = name.name.clone();
                    let const_type = self.ast_to_type(ty).unwrap_or(Type::Int);
                    self.ctx.define_type(name_text.clone(), const_type);
                    type_param_names.push(name_text);
                }
                GenericParamKind::Meta { name, ty, .. } => {
                    // For meta parameters like `N: meta usize`, similar handling as const generics
                    let name_text: Text = name.name.clone();
                    let meta_type = self.ast_to_type(ty).unwrap_or(Type::Int);
                    self.ctx.define_type(name_text.clone(), meta_type);
                    type_param_names.push(name_text);
                }
                _ => {}
            }
        }

        // CRITICAL: If no explicit type parameters on implement, extract them from the target type
        // This handles: implement Either<L, R> { ... } where L, R are inferred from the type
        if impl_decl.generics.is_empty() {
            let target_type = match &impl_decl.kind {
                ImplKind::Inherent(for_type) => Some(for_type),
                ImplKind::Protocol { for_type, .. } => Some(for_type),
            };

            if let Some(for_type) = target_type {
                // Extract type parameters from generic type like Either<L, R>
                if let verum_ast::ty::TypeKind::Generic { args, .. } = &for_type.kind {
                    for arg in args {
                        // Check if the argument is a simple type parameter (uppercase identifier)
                        if let verum_ast::ty::GenericArg::Type(ty) = arg {
                            if let verum_ast::ty::TypeKind::Path(path) = &ty.kind {
                                if let Some(ident) = path.as_ident() {
                                    let name = ident.name.as_str();
                                    // Type parameters are conventionally uppercase single letters or short names
                                    if name
                                        .chars()
                                        .next()
                                        .map(|c| c.is_uppercase())
                                        .unwrap_or(false)
                                        && self.ctx.lookup_type(name).is_none()
                                    {
                                        let type_var = Type::Var(TypeVar::fresh());
                                        let name_text: Text = name.into();
                                        self.ctx.define_type(name_text.clone(), type_var);
                                        type_param_names.push(name_text);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Extract implicit type parameters from where clause bounds.
        // For `implement Iterator for MapIterator<I, F> where F: Fn(I.Item) -> U`,
        // `U` is not in the generic params but needs a fresh TypeVar.
        if let Some(ref where_clause) = impl_decl.generic_where_clause {
            for predicate in &where_clause.predicates {
                use verum_ast::ty::WherePredicateKind;
                if let WherePredicateKind::Type { bounds, .. } = &predicate.kind {
                    for bound in bounds {
                        // Extract free type names from bounds that aren't already registered
                        let bound_ty_ast = match &bound.kind {
                            verum_ast::ty::TypeBoundKind::Equality(ty) => Some(ty),
                            verum_ast::ty::TypeBoundKind::GenericProtocol(ty) => Some(ty),
                            _ => None,
                        };
                        if let Some(bound_ty) = bound_ty_ast {
                            self.extract_implicit_type_params_from_type(
                                bound_ty,
                                &mut type_param_names,
                            );
                        }
                    }
                }
            }
        }

        match &impl_decl.kind {
            ImplKind::Inherent(_) => self.register_inherent_impl_methods(impl_decl, &type_param_names)?,
            ImplKind::Protocol { .. } => self.register_protocol_impl_methods(impl_decl, &type_param_names)?,
        }

        // Clean up type parameters
        for param_name in type_param_names {
            self.ctx.remove_type(&param_name);
        }

        Ok(())
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
    /// Register method signatures from an `implement Type { ... }` (inherent) block.
    /// Processes both static and instance methods, resolves Self type, and wires
    /// per-instantiation method-gating patterns.
    fn register_inherent_impl_methods(
        &mut self,
        impl_decl: &verum_ast::decl::ImplDecl,
        type_param_names: &List<verum_common::Text>,
    ) -> Result<()> {
        use verum_ast::decl::{FunctionParamKind, ImplItemKind, ImplKind};
        use verum_common::Text;
        let ImplKind::Inherent(for_type) = &impl_decl.kind
            else { unreachable!() };
        let type_param_names = type_param_names.clone();
                // CRITICAL FIX: Set current_self_type FIRST so Self types in method signatures resolve correctly
                // This enables patterns like `implement TypeName { fn new() -> Self { ... } }`
                let self_type = self.ast_to_type(for_type)?;
                let previous_self_type = self.current_self_type.clone();
                // Capture the impl's `for_type` argument list for
                // per-instantiation method gating. For
                // `implement<T: Copy> Register<T, ReadOnly>` this is
                // `[Var(T), Named(ReadOnly)]` — Var slots match any
                // concrete arg at lookup time; Named slots pin the
                // receiver's arg. Same encoding as the cross-file
                // `import_impl_blocks_for_type` path so both stdlib
                // and user impls populate the same registry.
                let impl_self_type_args: List<Type> = match &self_type {
                    Type::Named { args, .. } | Type::Generic { args, .. } => args.clone(),
                    _ => List::new(),
                };
                self.set_current_self_type(Maybe::Some(self_type.clone()));

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
                    // CRITICAL: Handle primitive types for inherent impls
                    // e.g., `implement Int { fn max_value() -> Int { ... } }`
                    verum_ast::ty::TypeKind::Int => Some(WKT::Int.as_str().to_string()),
                    verum_ast::ty::TypeKind::Float => Some(WKT::Float.as_str().to_string()),
                    verum_ast::ty::TypeKind::Bool => Some(WKT::Bool.as_str().to_string()),
                    verum_ast::ty::TypeKind::Char => Some(WKT::Char.as_str().to_string()),
                    verum_ast::ty::TypeKind::Text => Some(WKT::Text.as_str().to_string()),
                    verum_ast::ty::TypeKind::Unit => Some("Unit".to_string()),
                    // CRITICAL: Handle slice and array types for inherent impls
                    // e.g., `implement<T> [T] { fn len(&self) -> Int { ... } }`
                    verum_ast::ty::TypeKind::Slice(_) => Some("Slice".to_string()),
                    verum_ast::ty::TypeKind::Array { .. } => Some("Array".to_string()),
                    _ => None,
                };

                // Register all method signatures
                if let Some(type_name_str) = type_name {
                    for item in &impl_decl.items {
                        if let ImplItemKind::Function(func) = &item.kind {
                            // Wrap method registration in error recovery so that a single
                            // method failing to resolve types doesn't prevent remaining methods
                            // in the impl block from being registered.
                            let method_result: Result<()> = {
                                // Check if this is a static method (no self parameter)
                                let is_static = func
                                    .params
                                    .first()
                                    .map(|p| {
                                        !matches!(
                                            p.kind,
                                            FunctionParamKind::SelfValue
                                                | FunctionParamKind::SelfValueMut
                                                | FunctionParamKind::SelfRef
                                                | FunctionParamKind::SelfRefMut
                                                | FunctionParamKind::SelfOwn
                                                | FunctionParamKind::SelfOwnMut
                                        )
                                    })
                                    .unwrap_or(true);

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
                                        self.self_by_value_methods.insert((
                                            verum_common::Text::from(type_name_str.as_str()),
                                            verum_common::Text::from(func.name.name.as_str()),
                                        ));
                                    }
                                }

                                if is_static {
                                    // Register method-level generic type parameters FIRST
                                    // This fixes: implement<T> Wrapper<T> { fn map<U>(...) -> U }
                                    // where U needs to be in scope during signature resolution
                                    let mut method_type_param_names = List::new();
                                    // CRITICAL: Collect type bounds for function type parameters
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

                                            // Extract type bounds (e.g., F: fn(T) -> U)
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

                                    // Register static method
                                    // Use lenient fallback for cross-module types not yet loaded
                                    let param_types: List<Type> = func
                                        .params
                                        .iter()
                                        .filter_map(|p| match &p.kind {
                                            FunctionParamKind::Regular { ty, .. } => Some(ty),
                                            _ => None,
                                        })
                                        .map(|ty| {
                                            self.ast_to_type(ty)
                                                .unwrap_or_else(|_| self.ast_to_type_lenient(ty))
                                        })
                                        .collect();

                                    let return_type = func
                                        .return_type
                                        .as_ref()
                                        .map(|t| {
                                            self.ast_to_type(t)
                                                .unwrap_or_else(|_| self.ast_to_type_lenient(t))
                                        })
                                        .unwrap_or(Type::Unit);

                                    // CRITICAL FIX: Wrap return type in Future<T> for async static methods
                                    // (and Generator<Y, Unit> for `fn*` static methods, plus
                                    // Future<Generator<Y, Unit>> for `async fn*`).
                                    // This enables proper await/for-await type checking for
                                    // Connection.connect().await AND Connection.events_stream().
                                    let final_return_type = self.wrap_return_type_for_sig_full(
                                        return_type,
                                        &func.throws_clause,
                                        func.is_async,
                                        func.is_generator,
                                    );

                                    let func_ty = Type::function(param_types, final_return_type);
                                    let qualified_name =
                                        format!("{}.{}", type_name_str, func.name.name);

                                    // CRITICAL FIX: Use generalize_ordered to preserve type parameter order
                                    // Same fix as for instance methods - ensures correct type variable binding
                                    let mut ordered_params: List<verum_common::Text> =
                                        type_param_names.clone();
                                    for param in &method_type_param_names {
                                        ordered_params.push(param.clone());
                                    }
                                    let mut func_scheme =
                                        self.ctx.generalize_ordered(func_ty, &ordered_params);
                                    // Record how many ordered vars are impl-level so method-level params
                                    // (e.g., method's own <U>) aren't bound from receiver type args at call time.
                                    func_scheme.impl_var_count = type_param_names.len();

                                    // CRITICAL: Add type bounds to the TypeScheme for closure type inference
                                    if !method_type_var_bounds.is_empty() {
                                        func_scheme =
                                            func_scheme.with_type_bounds(method_type_var_bounds);
                                    }

                                    // Clean up method-level generic type parameters AFTER generalize_ordered
                                    for param_name in method_type_param_names {
                                        self.ctx.remove_type(&param_name);
                                    }

                                    self.ctx
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
                                    {
                                        let mut methods = self.inherent_methods.write();
                                        let type_methods =
                                            methods.entry(type_name_text).or_default();
                                        type_methods.insert(static_key, func_scheme);
                                    }

                                    tracing::debug!(
                                        "Registered static method signature: {}",
                                        qualified_name
                                    );
                                } else {
                                    // Register method-level generic type parameters FIRST
                                    // This fixes: implement<T> Wrapper<T> { fn map<U>(...) -> U }
                                    // where U needs to be in scope during signature resolution
                                    let mut method_type_param_names = List::new();
                                    // CRITICAL: Collect type bounds for function type parameters
                                    // This enables closure type inference for methods like:
                                    // fn map<U, F: fn(T) -> U>(self, f: F) -> Maybe<U>
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

                                            // Extract type bounds (e.g., F: fn(T) -> U)
                                            if !bounds.is_empty() {
                                                // #[cfg(debug_assertions)]
                                                // eprintln!("[DEBUG method_reg] Type param '{}' has {} bounds", name.name.as_str(), bounds.len());
                                                let extracted_bounds =
                                                    self.extract_type_bounds_from_ast(bounds);
                                                // #[cfg(debug_assertions)]
                                                // eprintln!("[DEBUG method_reg] Extracted {} type bounds", extracted_bounds.len());
                                                if !extracted_bounds.is_empty() {
                                                    method_type_var_bounds
                                                        .insert(fresh_var, extracted_bounds);
                                                }
                                            }
                                        }
                                    }

                                    // #[cfg(debug_assertions)]
                                    // eprintln!("[DEBUG method_reg] Method '{}' has {} method_type_var_bounds entries", func.name.name.as_str(), method_type_var_bounds.len());

                                    // Register instance method
                                    // Use lenient fallback for cross-module types not yet loaded
                                    let param_types: List<Type> = func
                                        .params
                                        .iter()
                                        .filter(|p| !p.is_self())
                                        .filter_map(|p| match &p.kind {
                                            FunctionParamKind::Regular { ty, .. } => Some(ty),
                                            _ => None,
                                        })
                                        .map(|ty| {
                                            self.ast_to_type(ty)
                                                .unwrap_or_else(|_| self.ast_to_type_lenient(ty))
                                        })
                                        .collect();

                                    let return_type = func
                                        .return_type
                                        .as_ref()
                                        .map(|t| {
                                            self.ast_to_type(t)
                                                .unwrap_or_else(|_| self.ast_to_type_lenient(t))
                                        })
                                        .unwrap_or(Type::Unit);

                                    // CRITICAL FIX: Wrap return type in Future<T> for async methods
                                    // (and Generator<Y, Unit> for `fn*` methods, plus
                                    // Future<Generator<Y, Unit>> for `async fn*`).
                                    // This enables proper await/for-await type checking for
                                    // inherent impl methods.
                                    let final_return_type = self.wrap_return_type_for_sig_full(
                                        return_type,
                                        &func.throws_clause,
                                        func.is_async,
                                        func.is_generator,
                                    );

                                    let method_ty = Type::function(param_types, final_return_type);
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
                                    // Method-level vars (e.g. `map<U, F: …>`): already
                                    // registered in method_type_param_names, resolve
                                    // to their fresh TypeVars here.
                                    for name in method_type_param_names.iter() {
                                        if let Option::Some(Type::Var(v)) =
                                            self.ctx.lookup_type(name.as_str())
                                        {
                                            ordered_vars.push(*v);
                                        }
                                    }

                                    let mut method_scheme =
                                        self.ctx.generalize_with_vars(method_ty, &ordered_vars);
                                    // impl_var_count is the number of impl-level
                                    // vars IN for_type, i.e. the first block.
                                    // This is exactly `receiver.args.len()` at call
                                    // time, so bind_limit = impl_var_count binds
                                    // receiver args to their correct positional
                                    // TypeVars and leaves impl_vars_outside free
                                    // (to be inferred from bounds / unification).
                                    method_scheme.impl_var_count = impl_vars_in_for_type.len();

                                    // CRITICAL: Add type bounds to the TypeScheme for closure type inference
                                    // This enables: fn map<U, F: fn(T) -> U>(self, f: F) -> Maybe<U>
                                    // When F is instantiated to a fresh var, the bound fn(T) -> U is preserved
                                    if !method_type_var_bounds.is_empty() {
                                        method_scheme =
                                            method_scheme.with_type_bounds(method_type_var_bounds);
                                    }

                                    // Clean up method-level generic type parameters AFTER generalize_ordered
                                    for param_name in method_type_param_names {
                                        self.ctx.remove_type(&param_name);
                                    }

                                    // Register in inherent_methods map (using shared RwLock)
                                    {
                                        let mut methods_guard = self.inherent_methods.write();
                                        let methods = methods_guard
                                            .entry(type_name_text.clone())
                                            .or_default();
                                        methods.insert(method_name_text.clone(), method_scheme);
                                    }

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
                                        let mut patterns_guard = self.method_impl_patterns.write();
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
                            }; // end of method registration closure
                            if let Err(_e) = method_result {
                                // Method type resolution failed — skip this method and continue
                                // with remaining methods in the impl block
                            }
                        }

                        // Handle associated constants
                        // All constants are registered within the same file regardless of visibility.
                        // Visibility (public/private) controls cross-file access, not same-file access.
                        // This matches how methods work: you can always access your own impl items.
                        if let ImplItemKind::Const { name, ty, .. } = &item.kind {
                            // Build the constant type
                            let const_type = self.ast_to_type(ty).unwrap_or(Type::Int);
                            let qualified_name = format!("{}.{}", type_name_str, name.name);

                            // Generalize with impl type parameters if any
                            let const_scheme = if type_param_names.is_empty() {
                                TypeScheme::mono(const_type)
                            } else {
                                self.ctx.generalize_ordered(const_type, &type_param_names)
                            };

                            // Register in environment with qualified name
                            self.ctx.env.insert(qualified_name.as_str(), const_scheme);

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
    ) -> Result<()> {
        use verum_ast::decl::{FunctionParamKind, ImplItemKind, ImplKind};
        use verum_common::Text;
        let ImplKind::Protocol { protocol, protocol_args, for_type } = &impl_decl.kind
            else { unreachable!() };
        let type_param_names = type_param_names.clone();
                // CRITICAL FIX: Register protocol implementation methods BEFORE type-checking
                // This enables method calls like `x.cmp(y)` where cmp is defined in
                // `implement Ord for Int { fn cmp(&self, other: &Int) -> Ordering { ... } }`

                // Extract type name - handles both path types and primitive types
                let type_name_opt = match &for_type.kind {
                    verum_ast::ty::TypeKind::Path(path) => {
                        path.as_ident().map(|id| id.name.as_str().to_string())
                    }
                    verum_ast::ty::TypeKind::Generic { base, .. } => match &base.kind {
                        verum_ast::ty::TypeKind::Path(path) => {
                            path.as_ident().map(|id| id.name.as_str().to_string())
                        }
                        _ => None,
                    },
                    // CRITICAL: Handle primitive types for protocol implementations
                    // e.g., `implement Hash for Int { ... }`
                    verum_ast::ty::TypeKind::Int => Some(WKT::Int.as_str().to_string()),
                    verum_ast::ty::TypeKind::Float => Some(WKT::Float.as_str().to_string()),
                    verum_ast::ty::TypeKind::Bool => Some(WKT::Bool.as_str().to_string()),
                    verum_ast::ty::TypeKind::Char => Some(WKT::Char.as_str().to_string()),
                    verum_ast::ty::TypeKind::Text => Some(WKT::Text.as_str().to_string()),
                    verum_ast::ty::TypeKind::Unit => Some("Unit".to_string()),
                    _ => None,
                };

                // Extract protocol name for debug logging
                let protocol_name = protocol
                    .as_ident()
                    .map(|id| id.name.as_str())
                    .unwrap_or("?");

                if let Some(type_name_str) = type_name_opt {
                    // Set current_self_type for Self resolution in method signatures
                    let self_type = self.ast_to_type(for_type)?;
                    let previous_self_type = self.current_self_type.clone();
                    self.set_current_self_type(Maybe::Some(self_type));

                    for item in &impl_decl.items {
                        if let ImplItemKind::Function(func) = &item.kind {
                            // Check if this is a static method (no self parameter)
                            let is_static = func
                                .params
                                .first()
                                .map(|p| {
                                    !matches!(
                                        p.kind,
                                        FunctionParamKind::SelfValue
                                            | FunctionParamKind::SelfValueMut
                                            | FunctionParamKind::SelfRef
                                            | FunctionParamKind::SelfRefMut
                                            | FunctionParamKind::SelfOwn
                                            | FunctionParamKind::SelfOwnMut
                                    )
                                })
                                .unwrap_or(true);

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

                                let method_ty = Type::function(param_types, final_return_type);
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
                                method_scheme.impl_var_count = impl_vars_in_for_type.len();

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
                                {
                                    let mut methods_guard = self.inherent_methods.write();
                                    let type_methods =
                                        methods_guard.entry(type_name_text.clone()).or_default();
                                    type_methods.insert(method_name_text.clone(), method_scheme);
                                }

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

                                let method_ty = Type::function(param_types, final_return_type);
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
                                method_scheme.impl_var_count = impl_vars_in_for_type.len();

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
                    let protocol_text = Text::from(protocol_name);
                    // Get the implementing type for Self substitution BEFORE acquiring the read lock
                    // to avoid borrow checker conflicts
                    let impl_self_type = self
                        .current_self_type
                        .clone()
                        .unwrap_or_else(|| self.ast_to_type(for_type).unwrap_or(Type::Unit));
                    if let Maybe::Some(proto) =
                        self.protocol_checker.read().get_protocol(&protocol_text)
                    {
                        let type_name_text = verum_common::Text::from(type_name_str.as_str());

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

                                // CRITICAL: Enter a scope to register method-level type params
                                // Methods like `fn map<B, F: fn(Self.Item) -> B>` have their own type params
                                self.ctx.enter_scope();

                                // Register fresh TypeVars for method type param names and build name->TypeVar map
                                let mut method_param_type_vars: Map<Text, TypeVar> = Map::new();
                                let mut method_type_vars: List<TypeVar> = List::new();
                                for param_name in &proto_method.type_param_names {
                                    let type_var = TypeVar::fresh();
                                    self.ctx
                                        .define_type(param_name.clone(), Type::Var(type_var));
                                    method_param_type_vars.insert(param_name.clone(), type_var);
                                    method_type_vars.push(type_var);
                                }

                                // The method type from the protocol needs Self substituted with the implementing type
                                let method_ty =
                                    self.substitute_self_type(&proto_method.ty, &impl_self_type);

                                // Build ordered TypeVar list: impl params first,
                                // then method params. Pass the TypeVars directly
                                // (not names) so that shared spellings between
                                // impl and method cannot collide through
                                // name-based lookup in `generalize_ordered`.
                                // Example that would previously break:
                                //  impl<T, F: fn()->T> Iterator for OnceWith<T,F>
                                //  fn map<B, F: fn(Self.Item)->B>(...)
                                // Here both impl and method declare `F`, and the
                                // name-lookup resolver would pin both positions
                                // to method_F, losing impl_F entirely.
                                let mut ordered_vars: List<TypeVar> = impl_type_vars.clone();
                                for v in method_type_vars.iter() {
                                    ordered_vars.push(*v);
                                }

                                // Create a TypeScheme with all type parameters
                                let mut method_scheme = self
                                    .ctx
                                    .generalize_with_vars(method_ty.clone(), &ordered_vars);
                                // Record impl-level vs method-level split. For `impl Iterator for Range<Int>`
                                // type_param_names is empty, so impl_var_count = 0 — method-level params
                                // like U in chain<U: Iterator<Item = Self.Item>> must NOT be bound to
                                // receiver type args (Range's Int). Without this, bind_limit falls back
                                // to fresh_vars.len() and pins U := Int (see #57).
                                method_scheme.impl_var_count = impl_type_vars.len();

                                // CRITICAL: Transfer type param bounds from the protocol method
                                // The bounds are keyed by param NAME, so we can look up the corresponding TypeVar
                                let method_scheme = if !proto_method.type_param_bounds.is_empty() {
                                    let mut substituted_bounds: Map<TypeVar, List<Type>> =
                                        Map::new();
                                    for (param_name, bound_ty) in &proto_method.type_param_bounds {
                                        // Substitute Self in the bound type
                                        let bound_with_self =
                                            self.substitute_self_type(bound_ty, &impl_self_type);

                                        // Look up the TypeVar for this param name
                                        if let Some(&type_var) =
                                            method_param_type_vars.get(param_name)
                                        {
                                            // Verify the TypeVar is in the scheme
                                            if method_scheme.vars.contains(&type_var) {
                                                let existing =
                                                    substituted_bounds.entry(type_var).or_default();
                                                existing.push(bound_with_self.clone());
                                                // if method_name.as_str() == "map" {
                                                //  eprintln!("[DEBUG default_method_registration] map: param_name={}, type_var={:?}, bound={:?}",
                                                //  param_name, type_var, bound_with_self);
                                                // }
                                            }
                                        }
                                    }
                                    method_scheme.with_type_bounds(substituted_bounds)
                                } else {
                                    method_scheme
                                };

                                // Exit method type parameter scope
                                self.ctx.exit_scope();

                                // Register in inherent_methods
                                {
                                    let mut methods_guard = self.inherent_methods.write();
                                    let type_methods =
                                        methods_guard.entry(type_name_text.clone()).or_default();

                                    // Only add if not already registered (don't override explicit definitions)
                                    if !type_methods.contains_key(method_name) {
                                        type_methods.insert(method_name.clone(), method_scheme);

                                        tracing::debug!(
                                            "Registered protocol DEFAULT method: {}.{} (from {})",
                                            type_name_str,
                                            method_name,
                                            protocol_name
                                        );
                                    }
                                }
                            }
                        }
                    }

                    // ═══════════════════════════════════════════════════════════════
                    // CRITICAL FIX: Register ProtocolImpl EARLY in register phase
                    // This enables blanket impls like `implement<S: Stream> StreamExt for S {}`
                    // to be available when type-checking method calls that need to find them.
                    // ═══════════════════════════════════════════════════════════════

                    // Collect where clauses from impl generic parameters
                    // CRITICAL FIX: Use the SAME Type::Var that was defined for this type parameter.
                    // When try_match_type matches for_type, it builds substitution keys like "T329"
                    // for Type::Var(TypeVar{id:329}). The where clause must use the same Type::Var
                    // so apply_substitution can look up the correct key.
                    let mut impl_where_clauses: List<crate::protocol::WhereClause> = List::new();
                    // CRITICAL: Collect function type bounds for type parameters.
                    // For `implement<F: fn(Fut.Output) -> T, T> ...`, captures F's TypeVar → fn type.
                    let mut type_param_fn_bounds: Map<TypeVar, Type> = Map::new();
                    for generic_param in &impl_decl.generics {
                        use verum_ast::ty::GenericParamKind;
                        if let GenericParamKind::Type { name, bounds, .. } = &generic_param.kind {
                            if !bounds.is_empty() {
                                // Look up the Type::Var that was defined at line 45851
                                // Clone immediately to release the immutable borrow before calling convert_type_bounds
                                let name_text: Text = name.name.clone();
                                let type_var_opt =
                                    self.ctx.lookup_type(name_text.as_str()).cloned();
                                if let Some(type_var) = type_var_opt {
                                    // Convert bounds to protocol bounds
                                    if let Ok(protocol_bounds) =
                                        self.convert_type_bounds_to_protocol_bounds(bounds)
                                    {
                                        impl_where_clauses.push(crate::protocol::WhereClause {
                                            ty: type_var.clone(),
                                            bounds: protocol_bounds,
                                        });
                                    }
                                    // Also capture function type bounds (e.g., F: fn(Fut.Output) -> T)
                                    for bound in bounds {
                                        match &bound.kind {
                                            verum_ast::ty::TypeBoundKind::Equality(bound_ty) => {
                                                if let verum_ast::ty::TypeKind::Function {
                                                    ..
                                                } = &bound_ty.kind
                                                {
                                                    if let Type::Var(tv) = &type_var {
                                                        if let Ok(resolved_fn) =
                                                            self.ast_to_type(bound_ty)
                                                        {
                                                            type_param_fn_bounds
                                                                .insert(*tv, resolved_fn);
                                                        }
                                                    }
                                                }
                                            }
                                            // Capture Fn/FnOnce/FnMut protocol bounds as function type bounds.
                                            // E.g., F: Fn(I.Item) -> U becomes fn(I.Item) -> U for
                                            // associated type resolution in substitute_impl_type_params.
                                            verum_ast::ty::TypeBoundKind::GenericProtocol(
                                                bound_ty,
                                            ) => {
                                                if let verum_ast::ty::TypeKind::Function {
                                                    ..
                                                } = &bound_ty.kind
                                                {
                                                    if let Type::Var(tv) = &type_var {
                                                        if let Ok(resolved_fn) =
                                                            self.ast_to_type(bound_ty)
                                                        {
                                                            type_param_fn_bounds
                                                                .insert(*tv, resolved_fn);
                                                        }
                                                    }
                                                }
                                            }
                                            verum_ast::ty::TypeBoundKind::Protocol(path) => {
                                                // Check if the protocol is Fn/FnOnce/FnMut with generic args
                                                // These are parsed as Protocol(path) when the path has generic args
                                                if let Some(ident) = path.as_ident() {
                                                    let proto_name = ident.name.as_str();
                                                    if matches!(
                                                        proto_name,
                                                        "Fn" | "FnOnce" | "FnMut"
                                                    ) {
                                                        // Try to extract function signature from the path's generic args
                                                        // This handles "Fn(A) -> B" parsed as path with generics
                                                    }
                                                }
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                            }
                        }
                    }

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

                                let method_ty = Type::function(params, return_type);
                                methods.insert(method_name, method_ty);
                            }
                        }
                    }

                    // Add default methods from the protocol
                    // CRITICAL FIX: Substitute Self with the implementing type when inserting default methods.
                    // Protocol default methods like `fn max(self, other: Self) -> Self` need Self replaced
                    // with the actual type (e.g., Int64) so that method lookup returns the correct type.
                    // Without this, calling Int64.max(other) returns Self instead of Int64.
                    let protocol_text = Text::from(protocol_name);
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
                                    self.substitute_self_type(&proto_method.ty, &impl_self_type);
                                methods.insert(method_name.clone(), substituted_method_ty);
                            }
                        }
                    }

                    // CRITICAL FIX: Collect associated types from impl items
                    // This enables proper resolution of associated type projections like T.Item
                    let mut associated_types: Map<Text, Type> = Map::new();
                    for item in &impl_decl.items {
                        if let ImplItemKind::Type {
                            name, ty: assoc_ty, ..
                        } = &item.kind
                        {
                            if let Ok(resolved_ty) = self.ast_to_type(assoc_ty) {
                                let assoc_name: Text = name.name.clone();
                                associated_types.insert(assoc_name, resolved_ty);
                            }
                        }
                    }

                    // Convert for_type to Type
                    // CRITICAL FIX: Use ast_to_type_lenient to avoid expanding type aliases.
                    // This ensures that `implement<T> FromResidual<Maybe<Never>> for Result<T, E>`
                    // keeps for_type as Named{Result, [T, E]} instead of expanding to the Variant form.
                    // This is necessary for get_implementations() to match lookups correctly.
                    let for_type_resolved = self.ast_to_type_for_protocol_impl(for_type)?;

                    // CRITICAL FIX: Resolve protocol type arguments (e.g., Result<Never, E> in FromResidual<Result<Never, E>>)
                    // This is essential for protocol matching in ? operator desugaring
                    let resolved_protocol_args: List<Type> = protocol_args
                        .iter()
                        .filter_map(|arg| {
                            use verum_ast::ty::GenericArg;
                            match arg {
                                GenericArg::Type(ty) => Some(
                                    self.ast_to_type(ty)
                                        .unwrap_or_else(|_| self.ast_to_type_lenient(ty)),
                                ),
                                GenericArg::Const(_)
                                | GenericArg::Lifetime(_)
                                | GenericArg::Binding(_) => None,
                            }
                        })
                        .collect();

                    // Process where clause for additional function type bounds.
                    // This captures bounds like `F: Fn(I.Item) -> U` from where clauses
                    // that aren't in the generic parameter declarations.
                    // (debug removed)
                    if let Some(ref where_clause) = impl_decl.generic_where_clause {
                        for predicate in &where_clause.predicates {
                            use verum_ast::ty::WherePredicateKind;
                            if let WherePredicateKind::Type { ty, bounds } = &predicate.kind {
                                if let verum_ast::ty::TypeKind::Path(path) = &ty.kind {
                                    if let Some(ident) = path.as_ident() {
                                        let param_name: Text = ident.name.clone();
                                        // Look up the type variable for this parameter
                                        if let Some(type_var) =
                                            self.ctx.lookup_type(param_name.as_str()).cloned()
                                        {
                                            // Add where clause to impl_where_clauses
                                            if let Ok(protocol_bounds) =
                                                self.convert_type_bounds_to_protocol_bounds(bounds)
                                            {
                                                impl_where_clauses.push(
                                                    crate::protocol::WhereClause {
                                                        ty: type_var.clone(),
                                                        bounds: protocol_bounds,
                                                    },
                                                );
                                            }
                                            // Extract function type bounds from where clause
                                            for bound in bounds {
                                                match &bound.kind {
                                                    verum_ast::ty::TypeBoundKind::Equality(bound_ty)
                                                    | verum_ast::ty::TypeBoundKind::GenericProtocol(bound_ty) => {
                                                        if let verum_ast::ty::TypeKind::Function { .. } = &bound_ty.kind {
                                                            if let Type::Var(tv) = &type_var {
                                                                if let Ok(resolved_fn) = self.ast_to_type(bound_ty) {
                                                                    type_param_fn_bounds.insert(*tv, resolved_fn);
                                                                }
                                                            }
                                                        }
                                                    }
                                                    _ => {}
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // Create and register ProtocolImpl
                    let protocol_impl = crate::protocol::ProtocolImpl {
                        protocol: protocol.clone(),
                        protocol_args: resolved_protocol_args,
                        for_type: for_type_resolved,
                        where_clauses: impl_where_clauses,
                        methods: methods.clone(),
                        associated_types,
                        associated_consts: Map::new(),
                        specialization: Maybe::None,
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

                    // Register with protocol checker (ignore coherence errors)
                    if let Err(e) = self.protocol_checker.write().register_impl(protocol_impl) {
                        tracing::debug!("Protocol impl registration warning: {}", e);
                    }

                    // Restore previous self type
                    self.set_current_self_type(previous_self_type);
                }
        Ok(())
    }

    pub fn pre_register_const(&mut self, const_decl: &verum_ast::decl::ConstDecl) {
        if let Ok(const_ty) = self.ast_to_type(&const_decl.ty) {
            self.ctx
                .env
                .insert(const_decl.name.name.as_str(), TypeScheme::mono(const_ty));
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
        self.ctx.env.insert(name.as_str(), TypeScheme::mono(ty));
    }

    pub fn register_function_signature(&mut self, func: &verum_ast::FunctionDecl) -> Result<()> {
        use verum_ast::decl::FunctionParamKind;
        use verum_common::Set;

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
        self.function_param_names
            .insert(func.name.name.clone(), collected_param_names);

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
            .insert(func.name.name.clone(), param_classifications);

        // Register generic type parameters FIRST
        // Track both names AND TypeVars for explicit TypeScheme construction
        // This is critical for phantom type parameters (e.g., `fn foo<T>()` where T isn't used)
        // Implicit arguments: compiler-inferred function arguments resolved by unification or type class search
        let mut type_param_names = List::new();
        let mut type_param_vars: List<TypeVar> = List::new();
        let mut implicit_type_vars: Set<TypeVar> = Set::new();
        let mut param_protocol_bounds: Map<TypeVar, List<crate::protocol::ProtocolBound>> =
            Map::new();

        // Save any existing types that will be shadowed by generic parameters,
        // so we can restore them after processing the function signature.
        // This prevents generic params like `Output` or `Args` from permanently
        // deleting legitimate type definitions from the context.
        let mut saved_types: List<(Text, Option<Type>)> = List::new();

        for generic_param in &func.generics {
            use verum_ast::ty::GenericParamKind;
            match &generic_param.kind {
                GenericParamKind::Type { name, bounds, .. } => {
                    let fresh_var = TypeVar::fresh();
                    let type_var = Type::Var(fresh_var);
                    let name_text: Text = name.name.clone();
                    saved_types
                        .push((name_text.clone(), self.ctx.lookup_type(&name_text).cloned()));
                    self.ctx.define_type(name_text.clone(), type_var);
                    type_param_names.push(name_text);
                    type_param_vars.push(fresh_var);

                    // Track if this is an implicit parameter
                    if generic_param.is_implicit {
                        implicit_type_vars.insert(fresh_var);
                    }

                    // Extract protocol bounds for this type parameter
                    if !bounds.is_empty() {
                        if let Ok(protocol_bounds) =
                            self.convert_type_bounds_to_protocol_bounds(bounds)
                        {
                            if !protocol_bounds.is_empty() {
                                param_protocol_bounds.insert(fresh_var, protocol_bounds);
                            }
                        }
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
                    self.ctx.define_type(name_text.clone(), type_var);
                    type_param_names.push(name_text);
                    type_param_vars.push(fresh_var);

                    // Track if this is an implicit parameter
                    if generic_param.is_implicit {
                        implicit_type_vars.insert(fresh_var);
                    }

                    // Extract protocol bounds for HKT parameters
                    if !bounds.is_empty() {
                        if let Ok(protocol_bounds) =
                            self.convert_type_bounds_to_protocol_bounds(bounds)
                        {
                            if !protocol_bounds.is_empty() {
                                param_protocol_bounds.insert(fresh_var, protocol_bounds);
                            }
                        }
                    }
                }
                GenericParamKind::Meta { name, ty, .. } => {
                    // Meta (const generic) parameters are compile-time values.
                    // Create a fresh TypeVar so they are included in the scheme's vars list,
                    // allowing explicit type arguments `foo<N>(args)` to bind meta values.
                    let fresh_var = TypeVar::fresh();
                    let meta_type = self.ast_to_type(ty)?;
                    let name_text: Text = name.name.clone();
                    saved_types
                        .push((name_text.clone(), self.ctx.lookup_type(&name_text).cloned()));
                    // Define the meta param as its underlying type (e.g., Int)
                    // so that references to N in the function body resolve correctly
                    self.ctx.define_type(name_text.clone(), meta_type);
                    type_param_names.push(name_text);
                    type_param_vars.push(fresh_var);

                    // Track if this is an implicit parameter
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
                FunctionParamKind::Regular { ty, .. } => self.ast_to_type(ty),
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
            self.ast_to_type(ret_ty)?
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
        // Protect builtin generic/meta functions from being DOWNGRADED by stdlib.
        // When a generic builtin (e.g., fn<T>(T) -> T for abs) already exists,
        // don't let a concrete stdlib version (e.g., fn(Float) -> Float) override it.
        // But DO allow equally-or-more-generic re-registrations (e.g., mount core.intrinsics).
        let should_skip_registration = {
            // Meta reflection builtins: ALWAYS protect. The compiler registers
            // fn(Type) -> Bool (1 param) for compile-time type inspection, but
            // stdlib declares fn<T>() -> Bool (0 params). The builtin version
            // is always correct.
            let is_meta_builtin = matches!(
                func.name.name.as_str(),
                "is_copy"
                    | "is_send"
                    | "is_sync"
                    | "is_sized"
                    | "needs_drop"
                    | "is_struct"
                    | "is_enum"
                    | "is_tuple"
                    | "implements"
                    | "type_name"
                    | "simple_name_of"
                    | "kind_of"
                    | "fields_of"
                    | "type_fields"
                    | "variants_of"
                    | "type_id"
                    | "size_of"
                    | "align_of"
            );
            if is_meta_builtin {
                self.ctx.env.lookup(func.name.name.as_str()).is_some()
            } else {
                // I/O and arithmetic builtins: protect when existing is MORE generic
                let is_potentially_protected = matches!(
                    func.name.name.as_str(),
                    "print"
                        | "println"
                        | "eprint"
                        | "eprintln"
                        | "add"
                        | "sub"
                        | "mul"
                        | "div"
                        | "rem"
                        | "neg"
                        | "abs"
                        | "min"
                        | "max"
                        | "clamp"
                        | "pow"
                        | "sqrt"
                        | "floor"
                        | "ceil"
                        | "round"
                        | "sin"
                        | "cos"
                        | "tan"
                );
                if is_potentially_protected {
                    if let Some(existing) = self.ctx.env.lookup(func.name.name.as_str()) {
                        !existing.vars.is_empty() && scheme.vars.len() < existing.vars.len()
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
        };
        if !should_skip_registration {
            self.ctx.env.insert(func.name.name.as_str(), scheme);
        }

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
                .insert(func.name.name.clone(), required_params);
        }

        // Restore previous types instead of removing — this prevents generic params
        // (e.g., `Output`, `Args`) from permanently deleting real type definitions.
        for (name, saved) in saved_types {
            match saved {
                Some(ty) => self.ctx.define_type(name.clone(), ty),
                None => self.ctx.remove_type(&name),
            }
        }

        tracing::debug!(
            "Registered function signature: {} (required_params: {})",
            func.name.name,
            required_params
        );
        Ok(())
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
                GenericParamKind::Meta { name, ty, .. } => {
                    let fresh_var = TypeVar::fresh();
                    let meta_type = self.ast_to_type(ty)?;
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
                FunctionParamKind::Regular { ty, .. } => self.ast_to_type(ty),
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
            self.ast_to_type(ret_ty)?
        } else {
            // No explicit return type - assume Unit
            Type::Unit
        };

        // Wrap return type in Future<T> for async functions
        let return_for_sig = if func.is_async {
            Type::Future {
                output: Box::new(return_type),
            }
        } else {
            return_type
        };

        // Build function type (intrinsics don't use context requirements)
        let func_type = Type::function(param_types, return_for_sig);

        // Create TypeScheme with tracked type parameters
        let scheme = if type_param_vars.is_empty() {
            TypeScheme::mono(func_type)
        } else if implicit_type_vars.is_empty() {
            TypeScheme::poly(type_param_vars.clone(), func_type)
        } else {
            TypeScheme::poly_with_implicit(type_param_vars.clone(), func_type, implicit_type_vars)
        };

        // Register using the INTRINSIC NAME, not the function name
        self.ctx.env.insert(intrinsic_name, scheme);

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
            .insert_mono(const_decl.name.name.as_str(), const_type);

        tracing::debug!("Registered const: {}", const_decl.name.name);
        Ok(())
    }

    /// Check an implementation block and register methods
    /// Protocol system: method resolution, associated types, default implementations, protocol objects (&dyn Protocol) — .6 - Protocol implementations
    ///

    /// Relies on RUST_MIN_STACK=16MB for stack safety on deeply nested impl blocks.
    pub(super) fn check_impl_block(&mut self, impl_decl: &verum_ast::decl::ImplDecl) -> Result<()> {
        self.check_impl_block_inner(impl_decl)
    }

    /// Inner implementation of check_impl_block
    fn check_impl_block_inner(&mut self, impl_decl: &verum_ast::decl::ImplDecl) -> Result<()> {
        use verum_ast::decl::{ImplItemKind, ImplKind};

        // CRITICAL FIX: Register generic type parameters from the impl block FIRST
        // This allows T to be resolved in: implement<T> List<T> { fn push(&self, value: T) { ... } }
        let mut type_param_names = List::new();

        // Collect where clauses for ProtocolImpl registration
        // This enables blanket implementations like `implement<S: Stream> StreamExt for S {}`
        // to be properly constrained and matched during method lookup.
        let mut impl_where_clauses: List<crate::protocol::WhereClause> = List::new();
        // CRITICAL: Collect function type bounds for type parameters.
        // For `implement<F: fn(Fut.Output) -> T, T> ...`, captures F's TypeVar → fn type.
        let mut type_param_fn_bounds: Map<TypeVar, Type> = Map::new();

        for generic_param in &impl_decl.generics {
            use verum_ast::ty::GenericParamKind;
            match &generic_param.kind {
                GenericParamKind::Type { name, bounds, .. } => {
                    let type_var = Type::Var(TypeVar::fresh());
                    let name_text: Text = name.name.clone();
                    self.ctx.define_type(name_text.clone(), type_var.clone());

                    // Register bounds from generic parameter declaration
                    // Example: implement<T: Clone> List<T> { ... }
                    if !bounds.is_empty() {
                        let protocol_bounds =
                            self.convert_type_bounds_to_protocol_bounds(bounds)?;

                        // Add to local type context for type checking
                        let type_param =
                            crate::context::TypeParam::new(name_text.clone(), name.span)
                                .with_bounds(protocol_bounds.clone());
                        self.ctx.env.add_type_param(type_param);

                        // CRITICAL: Also register bounds in type_var_bounds for method resolution.
                        // Without this, calling methods on bounded type params (e.g., E: Module<Text, Out>)
                        // fails because get_type_var_bounds returns empty, and the wrong method type is used.
                        if let Type::Var(tvar) = &type_var {
                            self.register_type_var_bounds(*tvar, protocol_bounds.clone());
                        }

                        // CRITICAL FIX: Also add to impl_where_clauses for blanket impl resolution
                        // For `implement<S: Stream> StreamExt for S {}`, the bound `S: Stream`
                        // must be stored so that method lookup can verify concrete types satisfy it.
                        //

                        // Use the SAME Type::Var that's used in for_type, so that the substitution
                        // keys match when apply_substitution is called.
                        impl_where_clauses.push(crate::protocol::WhereClause {
                            ty: type_var.clone(),
                            bounds: protocol_bounds,
                        });

                        // Also capture function type bounds (e.g., F: fn(Fut.Output) -> T)
                        for bound in bounds {
                            match &bound.kind {
                                verum_ast::ty::TypeBoundKind::Equality(bound_ty) => {
                                    if let verum_ast::ty::TypeKind::Function { .. } = &bound_ty.kind
                                    {
                                        if let Type::Var(tv) = &type_var {
                                            if let Ok(resolved_fn) = self.ast_to_type(bound_ty) {
                                                type_param_fn_bounds.insert(*tv, resolved_fn);
                                            }
                                        }
                                    }
                                }
                                verum_ast::ty::TypeBoundKind::GenericProtocol(bound_ty) => {
                                    // Fn/FnOnce/FnMut protocol bounds as function type bounds
                                    if let verum_ast::ty::TypeKind::Function { .. } = &bound_ty.kind
                                    {
                                        if let Type::Var(tv) = &type_var {
                                            if let Ok(resolved_fn) = self.ast_to_type(bound_ty) {
                                                type_param_fn_bounds.insert(*tv, resolved_fn);
                                            }
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }

                    type_param_names.push(name_text);
                }
                GenericParamKind::Meta { name, .. } => {
                    // Meta parameters (compile-time values) need type variables too.
                    // E.g., implement<D: meta USize{D > 0}, N: meta USize{N > 0}> StateSpaceKernel<D, N>
                    let type_var = Type::Var(TypeVar::fresh());
                    let name_text: Text = name.name.clone();
                    self.ctx.define_type(name_text.clone(), type_var);
                    type_param_names.push(name_text);
                }
                GenericParamKind::Const { name, ty } => {
                    // Const generic parameters need type registration too.
                    let name_text: Text = name.name.clone();
                    let const_type = self.ast_to_type(ty).unwrap_or(Type::Int);
                    self.ctx.define_type(name_text.clone(), const_type);
                    type_param_names.push(name_text);
                }
                _ => {} // Lifetime, Context, Level params handled elsewhere
            }
        }

        // CRITICAL: If no explicit type parameters on implement, extract them from the target type
        // This handles: implement Either<L, R> { ... } where L, R are inferred from the type
        // Also: shadow stdlib type bindings (e.g. `I` -> `Interval`) by saving the prior binding
        // and restoring it after the impl block. Without this, an impl like
        // `implement Iterator for MapIterator<I, F>` would resolve `I` to whatever stdlib
        // type happens to be aliased to that name, defeating the whole impl.
        let mut shadowed_type_bindings: Vec<(Text, Option<Type>)> = Vec::new();
        if impl_decl.generics.is_empty() {
            let target_type = match &impl_decl.kind {
                ImplKind::Inherent(for_type) => Some(for_type),
                ImplKind::Protocol { for_type, .. } => Some(for_type),
            };

            if let Some(for_type) = target_type {
                // Extract type parameters from generic type like Either<L, R>
                if let verum_ast::ty::TypeKind::Generic { args, .. } = &for_type.kind {
                    for arg in args {
                        // Check if the argument is a simple type parameter (uppercase identifier)
                        if let verum_ast::ty::GenericArg::Type(ty) = arg {
                            if let verum_ast::ty::TypeKind::Path(path) = &ty.kind {
                                if let Some(ident) = path.as_ident() {
                                    let name = ident.name.as_str();
                                    // Type parameters are conventionally uppercase single letters or short names
                                    if name
                                        .chars()
                                        .next()
                                        .map(|c| c.is_uppercase())
                                        .unwrap_or(false)
                                    {
                                        let name_text: Text = name.into();
                                        let prior = self.ctx.lookup_type(name).cloned();
                                        // If the prior binding is already a Type::Var
                                        // (i.e. another impl-level type-param in scope),
                                        // skip — that one is still valid.
                                        if matches!(prior, Some(Type::Var(_))) {
                                            continue;
                                        }
                                        let type_var = Type::Var(TypeVar::fresh());
                                        self.ctx.define_type(name_text.clone(), type_var);
                                        type_param_names.push(name_text.clone());
                                        shadowed_type_bindings.push((name_text, prior));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Process where clause constraints: where type T: Clone, type E: Clone
        // Generic bounds tracking: type parameters carry protocol constraints (e.g., T: Ord) that are checked at instantiation sites
        if let Some(ref where_clause) = impl_decl.generic_where_clause {
            for predicate in &where_clause.predicates {
                use verum_ast::ty::WherePredicateKind;
                match &predicate.kind {
                    WherePredicateKind::Type { ty, bounds } => {
                        // Extract type parameter name from the type
                        if let verum_ast::ty::TypeKind::Path(path) = &ty.kind
                            && let Some(ident) = path.as_ident()
                        {
                            let param_name: Text = ident.name.clone();

                            // Convert AST bounds to protocol bounds
                            let protocol_bounds =
                                self.convert_type_bounds_to_protocol_bounds(bounds)?;

                            // Add or update type parameter with bounds (for local type checking)
                            let type_param =
                                crate::context::TypeParam::new(param_name.clone(), predicate.span)
                                    .with_bounds(protocol_bounds.clone());
                            self.ctx.env.add_type_param(type_param);

                            // CRITICAL: Also store for ProtocolImpl.where_clauses
                            // Convert the constrained type to Type
                            let constrained_ty = self.ast_to_type(ty)?;

                            // CRITICAL: Register the protocol bounds on the TypeVar so
                            // method dispatch inside method bodies of this impl can find
                            // protocol methods through `I: Iterator` style where-clauses.
                            // Without this, `self.iter.next()` fails with "no method named
                            // `next` found for type `I`" because the bound-first dispatch
                            // in `infer_method_call_inner_impl` calls `get_type_var_bounds`
                            // which returns empty.
                            if let Type::Var(tv) = &constrained_ty {
                                self.register_type_var_bounds(*tv, protocol_bounds.clone());
                            }

                            impl_where_clauses.push(crate::protocol::WhereClause {
                                ty: constrained_ty.clone(),
                                bounds: protocol_bounds,
                            });

                            // Extract function type bounds from where clauses for
                            // associated type resolution (e.g., F: Fn(I.Item) -> U).
                            for bound in bounds {
                                match &bound.kind {
                                    verum_ast::ty::TypeBoundKind::Equality(bound_ty)
                                    | verum_ast::ty::TypeBoundKind::GenericProtocol(bound_ty) => {
                                        if let verum_ast::ty::TypeKind::Function { .. } =
                                            &bound_ty.kind
                                        {
                                            if let Type::Var(tv) = &constrained_ty {
                                                if let Ok(resolved_fn) = self.ast_to_type(bound_ty)
                                                {
                                                    type_param_fn_bounds.insert(*tv, resolved_fn);
                                                }
                                            }
                                        }
                                    }
                                    verum_ast::ty::TypeBoundKind::Protocol(proto_path) => {
                                        // Fn/FnOnce/FnMut protocol bounds may be parsed as
                                        // Protocol with a path that has function-like structure
                                        if let Some(proto_ident) = proto_path.as_ident() {
                                            let pn = proto_ident.name.as_str();
                                            if matches!(pn, "Fn" | "FnOnce" | "FnMut") {
                                                // Protocol path for Fn - check for generic args
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    _ => {
                        // Other where predicate kinds (Meta, Value, Ensures) handled elsewhere
                    }
                }
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
                for item in &impl_decl.items {
                    // Skip impl items gated by @cfg that don't match the current platform
                    if !self.cfg_evaluator.should_include(&item.attributes) {
                        continue;
                    }
                    if let ImplItemKind::Function(func) = &item.kind {
                        self.check_function(func)?;
                        tracing::debug!("Type-checked method {}.{}", ty, func.name.name);
                    }
                }
                self.in_impl_block = prev_in_impl;

                // Restore previous self type
                self.set_current_self_type(previous_self_type);
            }
            ImplKind::Protocol {
                protocol,
                protocol_args: ast_protocol_args,
                for_type,
            } => {
                // Protocol impl: register in protocol checker
                let ty = self.ast_to_type(for_type)?;
                let protocol_name = protocol
                    .as_ident()
                    .map(|i| i.name.clone())
                    .unwrap_or_else(|| "unknown".into());

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

                // Collect method types from impl block
                let mut methods: Map<Text, Type> = Map::new();
                for item in &impl_decl.items {
                    if let ImplItemKind::Function(func) = &item.kind {
                        // Build function type from signature
                        let method_name: Text = func.name.name.as_str().into();

                        // CRITICAL FIX: Register method's generic type params BEFORE processing param types
                        // This is needed for: fn bimap<E1, T1, E2, T2>(self: Result<T1, E1>, ...) -> Result<T2, E2>
                        let mut method_type_param_names: List<verum_common::Text> = List::new();
                        for generic_param in &func.generics {
                            use verum_ast::ty::GenericParamKind;
                            match &generic_param.kind {
                                GenericParamKind::Type { name, .. } => {
                                    let type_var = Type::Var(TypeVar::fresh());
                                    let name_text: Text = name.name.clone();
                                    self.ctx.define_type(name_text.clone(), type_var);
                                    method_type_param_names.push(name_text);
                                }
                                GenericParamKind::HigherKinded { name, arity, .. } => {
                                    use crate::advanced_protocols::Kind;
                                    let name_text: Text = name.name.clone();
                                    let kind = match *arity {
                                        0 => Kind::type_kind(),
                                        1 => Kind::unary_constructor(),
                                        2 => Kind::binary_constructor(),
                                        n => {
                                            let mut k = Kind::type_kind();
                                            for _ in 0..n {
                                                k = Kind::Arrow(
                                                    Box::new(Kind::type_kind()),
                                                    Box::new(k),
                                                );
                                            }
                                            k
                                        }
                                    };
                                    let type_constructor =
                                        Type::type_constructor(name_text.clone(), *arity, kind);
                                    self.ctx.define_type(name_text.clone(), type_constructor);
                                    method_type_param_names.push(name_text);
                                }
                                _ => {}
                            }
                        }

                        // Build method type EXCLUDING self parameter
                        // This is consistent with Protocol.methods and inherent_methods conventions.
                        // The method call handler checks args directly against params (no skip).
                        let param_types: Result<List<Type>> = func
                            .params
                            .iter()
                            .filter(|p| !p.is_self()) // Exclude self parameter
                            .filter_map(|p| {
                                use verum_ast::decl::FunctionParamKind;
                                match &p.kind {
                                    FunctionParamKind::Regular { ty, .. } => {
                                        Some(self.ast_to_type(ty))
                                    }
                                    // Self params are filtered out above, these cases shouldn't be reached
                                    _ => None,
                                }
                            })
                            .collect();
                        let param_types = param_types?;

                        let return_type = func
                            .return_type
                            .as_ref()
                            .map(|t| self.ast_to_type(t))
                            .unwrap_or(Ok(Type::Unit))?;

                        // CRITICAL FIX: Wrap return type in Future<T> for async methods
                        // This enables proper await type checking: async_method().await yields T, not Future<T>
                        let final_return_type = if func.is_async {
                            Type::Future {
                                output: Box::new(return_type),
                            }
                        } else {
                            return_type
                        };

                        let method_ty = Type::function(param_types, final_return_type);
                        methods.insert(method_name.clone(), method_ty);
                    }
                }

                // Collect associated type definitions from impl block
                // This handles: implement Functor for ListFunctor { type F<T> is List<T>; ... }
                let mut associated_types: Map<Text, Type> = Map::new();
                for item in &impl_decl.items {
                    if let ImplItemKind::Type {
                        name,
                        type_params,
                        ty: assoc_ty,
                    } = &item.kind
                    {
                        // For GATs like `type F<T> is List<T>`, register type params temporarily
                        let mut temp_type_params: List<verum_common::Text> = List::new();
                        for param in type_params {
                            use verum_ast::ty::GenericParamKind;
                            match &param.kind {
                                GenericParamKind::Type {
                                    name: param_name, ..
                                } => {
                                    let type_var = Type::Var(TypeVar::fresh());
                                    let param_name_text: Text = param_name.name.clone();
                                    self.ctx.define_type(param_name_text.clone(), type_var);
                                    temp_type_params.push(param_name_text);
                                }
                                _ => {}
                            }
                        }

                        // Convert the associated type's RHS
                        if let Ok(resolved_ty) = self.ast_to_type(assoc_ty) {
                            let assoc_name: Text = name.name.clone();
                            associated_types.insert(assoc_name, resolved_ty);
                        }

                        // Clean up temporary type params
                        for param_name in temp_type_params {
                            self.ctx.remove_type(&param_name);
                        }
                    }
                }

                // CRITICAL FIX: Copy default methods from protocol definition
                // When `implement Read for File` only provides `read`, we need to also
                // include default methods like `read_to_string` from the Read protocol.
                // This enables File.read_to_string() to work via protocol method lookup.
                // Note: We substitute Self with the implementing type `ty` for correct method signatures.
                if let Maybe::Some(protocol_def) =
                    self.protocol_checker.read().get_protocol(&protocol_name)
                {
                    for (method_name, protocol_method) in protocol_def.methods.iter() {
                        // Only add if not already provided by the impl
                        if !methods.contains_key(method_name) && protocol_method.has_default {
                            // Copy the default method's type signature with Self substituted
                            let substituted_method_ty =
                                self.substitute_self_type(&protocol_method.ty, &ty);
                            methods.insert(method_name.clone(), substituted_method_ty);
                            tracing::debug!(
                                "Added default method {} from protocol {} to impl for {}",
                                method_name,
                                protocol_name,
                                ty
                            );
                        }
                    }
                }

                // CRITICAL FIX: Resolve protocol type arguments (e.g., Result<Never, E> in FromResidual<Result<Never, E>>)
                let resolved_protocol_args: List<Type> = ast_protocol_args
                    .iter()
                    .filter_map(|arg| {
                        use verum_ast::ty::GenericArg;
                        match arg {
                            GenericArg::Type(ty) => Some(
                                self.ast_to_type(ty)
                                    .unwrap_or_else(|_| self.ast_to_type_lenient(ty)),
                            ),
                            GenericArg::Const(_)
                            | GenericArg::Lifetime(_)
                            | GenericArg::Binding(_) => None,
                        }
                    })
                    .collect();

                // Create ProtocolImpl with collected methods and associated types
                // CRITICAL: Include where_clauses for blanket impl constraint checking
                let protocol_impl = crate::protocol::ProtocolImpl {
                    protocol: protocol.clone(),
                    protocol_args: resolved_protocol_args,
                    for_type: ty.clone(),
                    where_clauses: impl_where_clauses.clone(),
                    methods: methods.clone(),
                    associated_types,
                    associated_consts: Map::new(),
                    specialization: Maybe::None,
                    impl_crate: Maybe::Some(self.current_module_path.clone()),
                    span: impl_decl.span,
                    type_param_fn_bounds: type_param_fn_bounds.clone(),
                };

                // CRITICAL FIX: Register protocol impl with protocol checker
                // This enables blanket implementations like `implement<T, U: From<T>> Into<U> for T`
                // to be found during method lookup. The previous comment was incorrect -
                // register_impl_block_inner is only called for LOCAL impl blocks (inside function bodies),
                // not for TOP-LEVEL impl blocks which go through check_impl_block_inner.
                if let Err(e) = self
                    .protocol_checker
                    .write()
                    .register_impl(protocol_impl.clone())
                {
                    tracing::debug!("Protocol impl registration warning: {}", e);
                }

                // DEBUG: Log Into impl registration
                let proto_name_str = protocol_impl
                    .protocol
                    .as_ident()
                    .map(|i| i.as_str().to_string())
                    .unwrap_or_default();
                if proto_name_str == "Into" {
                    // #[cfg(debug_assertions)]
                    // eprintln!("[DEBUG check_impl_block] Registered Into impl, for_type: {:?}, where_clauses: {:?}",
                    // protocol_impl.for_type, protocol_impl.where_clauses);
                }

                // NOTE: Protocol methods are already registered in inherent_methods by
                // register_impl_block_inner (which uses generalize_ordered for correct
                // type param handling). Do NOT overwrite here — generalize() does not
                // include method-level type params that are defined in the environment,
                // resulting in monomorphic schemes that fail for generic methods like
                // extend<I: Iterator<Item>>(&mut self, iter: I).

                // Type-check method bodies
                let prev_in_impl = self.in_impl_block;
                self.in_impl_block = true;
                for item in &impl_decl.items {
                    // Skip impl items gated by @cfg that don't match the current platform
                    if !self.cfg_evaluator.should_include(&item.attributes) {
                        continue;
                    }
                    if let ImplItemKind::Function(func) = &item.kind {
                        self.check_function(func)?;
                        tracing::debug!(
                            "Type-checked protocol method {}.{} for {}",
                            protocol_name,
                            func.name.name,
                            ty
                        );
                    }
                }
                self.in_impl_block = prev_in_impl;

                // Restore previous self type
                self.set_current_self_type(previous_self_type);
            }
        }

        // Clean up type parameters from the type context
        for param_name in type_param_names {
            self.ctx.remove_type(&param_name);
        }

        // Restore any stdlib type bindings we shadowed when extracting implicit
        // type-params from the for_type (e.g. `I` was a fresh type-var here but
        // points at `Interval` in the outer scope). Without this restore the
        // outer scope would lose its binding once we exited.
        for (name, prior) in shadowed_type_bindings {
            if let Some(prior_ty) = prior {
                self.ctx.define_type(name, prior_ty);
            }
        }

        Ok(())
    }

    /// Check if a type represents the Never (bottom) type.
    ///

    /// The Never type can be represented in two ways:
    /// 1. Type::Never - the primitive never type
    /// 2. Type::Named { path: "Never", ... } - a named reference to Never
    ///

    /// This function handles both cases for proper subtyping behavior.
    /// Type lattice: Never is bottom (subtype of all), Unknown is top (supertype of all)
    pub(super) fn is_never_type(&self, ty: &Type) -> bool {
        match ty {
            Type::Never => true,
            Type::Named { path, args } if args.is_empty() => {
                // Check if the name is "Never"
                if let Some(segment) = path.segments.last() {
                    if let verum_ast::ty::PathSegment::Name(ident) = segment {
                        return ident.name.as_str() == "Never";
                    }
                }
                false
            }
            _ => false,
        }
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
                    if let Some(borrow) = self.borrow_tracker.get_held_borrow(&var_name) {
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
        !self.current_function_params.contains(var_name)
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
                        if let Some(borrow) = self.borrow_tracker.get_held_borrow(&var_name) {
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
            let var_name = match &arg.kind {
                ExprKind::Path(path) => {
                    if path.segments.len() == 1 {
                        if let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0] {
                            Some(ident.name.clone())
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
                _ => None,
            };

            if let Some(name) = var_name {
                // Check if this variable is affine and consume it
                if self.affine_tracker.is_affine_binding(name.as_str()) {
                    self.affine_tracker.use_value(name.as_str(), arg.span)?;
                }
            }
        }
        Ok(())
    }

    // ============================================================
    // Function Parameter Aliasing Detection
    // Reference safety invariants: managed refs validated at dereference, checked refs proven safe at compile time, unsafe refs unchecked — Aliasing Rules
    // ============================================================

    /// Check for aliasing conflicts between function arguments.
    /// Detects when multiple arguments reference the same data with conflicting access modes.
    ///

    /// Examples of conflicts:
    /// - `foo(&mut x, &mut x)` - two mutable references to same variable
    /// - `bar(&mut x, &x)` - mutable and immutable reference to same variable
    /// - `baz(&mut x.field, &mut x)` - field mutable borrow conflicts with whole struct
    pub(super) fn check_function_arg_aliasing(
        &self,
        args: &[Expr],
        param_types: &List<Type>,
        call_span: Span,
    ) -> Result<()> {
        use verum_ast::expr::ExprKind;

        /// Information about a reference argument
        #[derive(Debug)]
        struct RefArg {
            target: Text,
            field_path: Option<Text>,
            is_mutable: bool,
            arg_index: usize,
            span: Span,
        }

        // Collect all reference arguments
        let mut ref_args: List<RefArg> = List::new();

        for (i, (arg, param_ty)) in args.iter().zip(param_types.iter()).enumerate() {
            let resolved_param = self.unifier.apply(param_ty);

            // Check if parameter expects a reference
            let is_mut_ref = matches!(
                &resolved_param,
                Type::Reference { mutable: true, .. }
                    | Type::CheckedReference { mutable: true, .. }
                    | Type::UnsafeReference { mutable: true, .. }
            );
            let is_immut_ref = matches!(
                &resolved_param,
                Type::Reference { mutable: false, .. }
                    | Type::CheckedReference { mutable: false, .. }
                    | Type::UnsafeReference { mutable: false, .. }
            );

            if !is_mut_ref && !is_immut_ref {
                continue;
            }

            // Extract the target variable and field path
            let (target, field_path) = self.extract_ref_target(arg);

            if let Some(target) = target {
                ref_args.push(RefArg {
                    target,
                    field_path,
                    is_mutable: is_mut_ref,
                    arg_index: i,
                    span: arg.span,
                });
            }
        }

        // Check for conflicts between reference arguments
        for i in 0..ref_args.len() {
            for j in (i + 1)..ref_args.len() {
                let ref_a = &ref_args[i];
                let ref_b = &ref_args[j];

                // Skip if different variables
                if ref_a.target != ref_b.target {
                    continue;
                }

                // Check for overlap (same field path or one is prefix of other)
                let overlaps = match (&ref_a.field_path, &ref_b.field_path) {
                    (None, _) | (_, None) => true, // Whole variable overlaps with anything
                    (Some(pa), Some(pb)) => {
                        pa.as_str() == pb.as_str()
                            || pa.as_str().starts_with(&format!("{}.", pb))
                            || pb.as_str().starts_with(&format!("{}.", pa))
                    }
                };

                if !overlaps {
                    continue; // Different fields, no conflict
                }

                // Check for aliasing conflict
                // Rule: at most one mutable reference, or multiple immutable references
                if ref_a.is_mutable || ref_b.is_mutable {
                    let conflict_desc = if ref_a.is_mutable && ref_b.is_mutable {
                        "two mutable references"
                    } else {
                        "mutable and immutable reference"
                    };

                    let target_desc = match (&ref_a.field_path, &ref_b.field_path) {
                        (Some(f), _) => format!("{}.{}", ref_a.target, f),
                        (_, Some(f)) => format!("{}.{}", ref_b.target, f),
                        _ => ref_a.target.to_string(),
                    };

                    return Err(TypeError::BorrowConflict {
                        var: verum_common::Text::from(target_desc),
                        existing_borrow_span: ref_a.span,
                        existing_is_mut: ref_a.is_mutable,
                        new_borrow_span: ref_b.span,
                        new_is_mut: ref_b.is_mutable,
                    });
                }
            }
        }

        Ok(())
    }

    /// Extract the target variable and field path from a reference expression.
    /// For `&x.field.subfield`, returns (Some("x"), Some("field.subfield"))
    /// For `&x`, returns (Some("x"), None)
    /// For `&mut x`, returns (Some("x"), None)
    fn extract_ref_target(&self, expr: &Expr) -> (Option<Text>, Option<Text>) {
        use verum_ast::expr::ExprKind;

        match &expr.kind {
            // Direct reference: &x or &mut x
            ExprKind::Unary { op, expr: inner } => {
                use verum_ast::expr::UnOp;
                if matches!(op, UnOp::Ref | UnOp::RefMut) {
                    return self.extract_capture_target(inner);
                }
                (None, None)
            }
            // Variable might be passed directly if param expects reference (auto-ref)
            ExprKind::Path(path) => {
                if let Some(verum_ast::ty::PathSegment::Name(ident)) = path.segments.first() {
                    (Some(verum_common::Text::from(ident.name.as_str())), None)
                } else {
                    (None, None)
                }
            }
            // Field access might be passed directly
            ExprKind::Field { .. } => self.extract_capture_target(expr),
            _ => (None, None),
        }
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
