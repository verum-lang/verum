//! Path resolution methods extracted from `infer.rs`.
//!
//! Contains the 10 `TypeChecker` methods and 1 standalone function responsible
//! for resolving type names, qualified paths, multi-segment paths, crate-rooted
//! paths, super paths, self paths, variable field access, and inline module paths.

use crate::context::ModuleId;
use crate::infer::{InferResult, TypeChecker};
use crate::ty::Type;
use crate::{Result, TypeError};
use smallvec::SmallVec;
use verum_ast::expr::{Expr, ExprKind};
use verum_ast::span::Span;
use verum_ast::ty::Path;
use verum_common::{List, Map, Maybe, Text, ToText};
use verum_diagnostics::Diagnostic;
use verum_modules::resolver::NameKind;

impl TypeChecker {
    /// Resolve a type name using module-aware resolution
    /// Name resolution across modules: qualified paths, import disambiguation, re-exports, path resolution in imports — Cross-module type resolution
    ///
    /// Resolution order:
    /// 1. Check current module's type definitions
    /// 2. Use NameResolver to find the type across modules
    /// 3. Load type from module registry
    /// 4. Verify visibility (types must be accessible)
    /// Reports a name that resolved WITHOUT anyone asking for it
    /// (T0780).
    ///
    /// Every public stdlib symbol is visible bare, without a `mount`.
    /// This prints one line per use where the resolving file neither
    /// DECLARED the name nor MOUNTED it — the exact dependency that
    /// has to shrink before visibility can be made to follow `mount`.
    ///
    /// `VERUM_REPORT_AMBIENT=1` reports everything; setting it to a
    /// module-path substring reports only that module, the way
    /// `VERUM_DUMP_VBC` filters by function name. Aggregate the output
    /// with `sort | uniq -c`.
    ///
    /// Names the file declares are excluded because a declaration owns
    /// its name; mounted names are excluded because a `mount` IS the
    /// request. What remains is ambient.
    fn report_ambient_use(&self, name: &str, resolved: &Type) {
        let Some(filter) = std::env::var_os("VERUM_REPORT_AMBIENT") else {
            return;
        };
        // A TYPE PARAMETER is not an ambient symbol. Declarations
        // register their own parameters (`K`, `V`, `T`, `Rhs`) into the
        // same table while their body is being checked, and clean them
        // up afterwards; a parameter resolves to a fresh type VARIABLE,
        // which is what tells it apart from a nominal type. Counting
        // them made `K`/`V`/`T` the top three "ambient" names — 1892 of
        // 5352 in the first measurement — and buried the real ones.
        if matches!(resolved, Type::Var(_)) {
            return;
        }
        if self.current_module_declared_types.contains(name)
            || self
                .imported_names
                .contains_key(&verum_common::Text::from(name))
        {
            return;
        }
        let module = if self.current_module_path.as_str().is_empty() {
            "<user>"
        } else {
            self.current_module_path.as_str()
        };
        let filter = filter.to_string_lossy();
        if filter != "1" && !module.contains(filter.as_ref()) {
            return;
        }
        // Name the OWNER too. Migration is `mount <owner>.{<name>};`,
        // so a report that only names the symbol leaves the mechanical
        // half of the work undone.
        //
        // The owner comes from the stdlib metadata, which records each
        // type's declaring module — NOT from scanning `type_defs` for a
        // `<module>.<name>` key. Archive-loaded types are registered
        // flat only, so that scan answered `<unknown>` for 1573 of 1990
        // uses and matched service keys (`__struct_fields_*`) for some
        // of the rest.
        let owner = match &self.core_metadata {
            Maybe::Some(m) => match m.types.get(&verum_common::Text::from(name)) {
                Some(td) => {
                    let declared = match &td.origin_module_path {
                        Maybe::Some(p) if !p.as_str().is_empty() => p.as_str(),
                        _ => td.module_path.as_str(),
                    };
                    if declared.is_empty() {
                        "<unknown>".to_string()
                    } else {
                        declared.to_string()
                    }
                }
                None => "<not-in-metadata>".to_string(),
            },
            Maybe::None => "<no-metadata>".to_string(),
        };
        eprintln!("[ambient] {}\t{}\t{}", module, name, owner);
    }

    /// Is ambient visibility an ERROR for the file being checked?
    ///
    /// `VERUM_STRICT_VISIBILITY` turns the report above into a hard
    /// diagnostic, so a module can be brought onto explicit `mount`s
    /// one at a time and STAY there. Set it to `1` for everything, or
    /// to a module-path substring to hold just that module to the rule
    /// — the same filter shape as the report.
    ///
    /// This is how the acceptance gets reached incrementally: ~1400
    /// uses across five core/ files reach names their file never
    /// requested, which is far too many to fix in one change, and
    /// exactly the shape a ratchet handles.
    fn strict_visibility_applies(&self, module: &str) -> bool {
        match std::env::var("VERUM_STRICT_VISIBILITY") {
            Ok(v) if v == "1" => true,
            Ok(v) if !v.is_empty() => module.contains(v.as_str()),
            _ => false,
        }
    }

    pub(crate) fn resolve_type_name(&mut self, name: &str, span: Span) -> Result<Type> {
        self.resolve_type_name_with_parameter_key(name, span)
            .map(|(ty, _)| ty)
    }

    fn resolve_type_name_with_parameter_key(
        &mut self,
        name: &str,
        span: Span,
    ) -> Result<(Type, Option<Text>)> {
        // CRITICAL FIX: Resolve "Self" to the concrete type from current_self_type.
        // When inside an implement block, Self should resolve to the implementing type
        // (e.g., RetryPolicy, AtomicU8) rather than remaining as Type::Named("Self").
        // This prevents type mismatch errors like "expected 'RetryPolicy', found 'Self'".
        if name == "Self" {
            if let Maybe::Some(ref self_ty) = self.current_self_type {
                return Ok((self_ty.clone(), None));
            }
        }

        // Step 0.25: the registering declaration's OWN generic
        // parameters (innermost frame only — lexical scope). This is
        // the frame mechanism that replaced flat-table parameter
        // registration: parameters resolve here ONLY while their
        // declaration's body registers, so a lazily-entered
        // registration can never leak `T` into the resolution that
        // triggered it (T0683). Checked before import-ambiguity —
        // a parameter is more local than any import.
        if let Some(ty) = self.lookup_decl_param(name) {
            return Ok((ty.clone(), None));
        }

        if self.resolving_source_signature {
            return self.resolve_declaring_source_type_with_parameter_key(name, span);
        }

        // Check for import ambiguity first
        // Name resolution across modules: qualified paths, import disambiguation, re-exports, path resolution in imports — Import Ambiguity
        if let Some(sources) = self.imported_names.get(&verum_common::Text::from(name)) {
            if sources.len() > 1 {
                let sources_str = sources
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(TypeError::AmbiguousName {
                    name: verum_common::Text::from(name),
                    sources: verum_common::Text::from(sources_str),
                    span,
                });
            }
        }

        // Step 0: Architectural module-aware lookup. When two stdlib
        // modules legitimately declare a type with the same simple name
        // (e.g. `RecvError` in both `core.async.broadcast` and
        // `core.net.quic.stream_sm.recv`), the flat `ctx.type_defs` map's
        // "last-registered wins" semantics used to silently reroute
        // references in the *declaring* module to the other module's type
        // — producing confusing cross-module type mismatches
        // (`broadcast_stream.vr`). We now publish every type under the
        // fully-qualified key `{module_path}.{name}` in addition to the
        // flat name (see `define_type_in_current_module`). Here we prefer
        // that qualified lookup first, so the declaring module always
        // resolves its own types regardless of load order.
        let qualified = self.declared_type_key(name);
        if qualified.as_str() != name
            && let Some(ty) = self.ctx.lookup_type(qualified.as_str())
        {
            return Ok((ty.clone(), Some(qualified)));
        }

        // Step 0.5: MOUNT-TYPE-AUTHORITY-1 — a file's explicitly
        // MOUNTED type names are authoritative for that file's
        // bare-name resolution (mirror of the VBC codegen context's
        // `explicit_mount_names` discipline on the function side, at
        // the type layer).  When the file wrote
        // `mount <module>.{Name}`, `Name` in ITS signatures and
        // annotations means `<module>.Name` — never whichever
        // same-named type from an unrelated module happens to own
        // the flat `type_defs` slot (Step 1) or the last-wins simple
        // slot in `core_metadata.types` (Step 1.5's lazy load).
        // Unmounted names skip this in one map probe; declaring
        // modules still win via Step 0 above.
        // A source rename is a mount binding, not a new type alias with its
        // own generic parameters. Resolve its declaration before the flat
        // redirect loses that distinction. Lexical binders still win.
        let lexical_binder = self.ctx.type_defs.get(&Text::from(name)).is_some_and(|ty| {
            matches!(ty, Type::Var(_) | Type::TypeConstructor { .. })
        });
        if !lexical_binder
            && self.imports_in_progress.is_empty()
            && self.glob_imports_in_progress.is_empty()
            && self.imported_names.contains_key(&Text::from(name))
            && let Some((declaration, owner)) = self.declaring_source_type_declaration(name)
        {
            return self
                .load_source_type_declaration_with_parameter_key(&declaration, &owner, span)
                .map(|(ty, key)| (ty, Some(key)));
        }
        if let Some((ty, key)) = self.resolve_type_name_mount_scoped_with_parameter_key(name) {
            if std::env::var("VERUM_TRACE_MOUNT_AUTH")
                .is_ok_and(|v| v == "1" || v == name)
            {
                eprintln!(
                    "[mount-auth] resolve_type_name('{}') step 0.5 -> {:?}",
                    name, ty
                );
            }
            return Ok((ty, Some(key)));
        }

        // Step 1: Try current module first (fast path)
        if let Maybe::Some(ty) = self.ctx.lookup_type(name) {
            self.report_ambient_use(name, &ty);
            if std::env::var("VERUM_TRACE_MOUNT_AUTH")
                .is_ok_and(|v| v == "1" || v == name)
            {
                eprintln!(
                    "[mount-auth] resolve_type_name('{}') step 1 flat -> {:?}",
                    name, ty
                );
            }
            let key = (!matches!(ty, Type::Var(_) | Type::TypeConstructor { .. }))
                .then(|| Text::from(name));
            return Ok((ty.clone(), key));
        }

        // Step 1.5: VBCA lookup-on-miss for stdlib types (#128).
        //
        // The lazy pre-pass (`register_stdlib_types_for_module` →
        // `collect_named_types_from_function_body`) intentionally
        // skips function bodies, so stdlib types referenced ONLY
        // from inline body expressions (e.g. `let v: List<Int> =
        // List<Int>.new()`) never get pre-loaded.  Without this
        // miss-trigger, body-position references to unmounted
        // stdlib types throw `type not found: List` / `Map` etc.
        //
        // Stdlib-agnostic per `crates/verum_types/src/AGENTS.md`:
        // the existence test is `core_metadata.types` membership,
        // not a hardcoded list of stdlib type names.
        // `ensure_stdlib_type_loaded` is idempotent (gates on
        // `lookup_type(name).is_none()` internally) so the retry
        // is safe in any context.
        let name_text = verum_common::Text::from(name);
        let mut pending: Vec<verum_common::Text> = Vec::new();
        self.ensure_stdlib_type_loaded(&name_text, &mut pending);
        let mut bound = 256usize;
        while let Some(next) = pending.pop() {
            bound = bound.saturating_sub(1);
            if bound == 0 {
                break;
            }
            self.ensure_stdlib_type_loaded(&next, &mut pending);
        }
        if let Maybe::Some(ty) = self.ctx.lookup_type(name) {
            self.report_ambient_use(name, &ty);
            let key = (!matches!(ty, Type::Var(_) | Type::TypeConstructor { .. }))
                .then(|| Text::from(name));
            return Ok((ty.clone(), key));
        }

        // Step 2: Use module resolver to find the type
        if let Maybe::Some(current_module) = self.current_module() {
            match self.module_resolver.resolve_name(name, current_module) {
                Ok(resolved) => {
                    // Check if it's a type
                    if resolved.kind != NameKind::Type {
                        return Err(TypeError::NotAType {
                            name: name.to_text(),
                            actual_kind: format!("{}", resolved.kind).into(),
                            span,
                        });
                    }

                    // Step 3: Load type from module registry
                    let module_id = resolved.module_id;
                    if let Maybe::Some(ty) = self.ctx.lookup_module_type(module_id, name) {
                        // This legacy cache records no parameter-registration key.
                        return Ok((ty.clone(), None));
                    }

                    // Try loading from module registry directly.
                    // Snapshot items outside the read-guard: later
                    // calls like `register_type_declaration` take
                    // &mut self and would alias the registry borrow.
                    let items_snapshot: Option<Vec<_>> = {
                        let reg = self.module_registry.read();
                        reg.get(module_id)
                            .map(|m| m.ast.items.iter().cloned().collect())
                    };
                    if let Some(items) = items_snapshot {
                        // Search for type definition in module AST
                        for item in &items {
                            if let verum_ast::ItemKind::Type(type_decl) = &item.kind
                                && type_decl.name.name.as_str() == name
                            {
                                // Found the type definition
                                // CRITICAL: Check if type is already registered to prevent infinite recursion.
                                // This can happen when processing mutually recursive types.
                                if let Maybe::Some(existing_ty) = self.ctx.lookup_type(name) {
                                    let ty_cloned = existing_ty.clone();
                                    self.ctx
                                        .define_module_type(module_id, name, ty_cloned.clone());
                                    return Ok((ty_cloned, Some(name.into())));
                                }

                                // Register the type declaration to resolve type aliases.
                                // This ensures that type aliases like `type X is { ... }` are resolved
                                // to their underlying Record types, not just stored as Named references.
                                // ALIAS-VS-MARKER scope (#41): lazy registration
                                // must carry the declaring module's local
                                // visibility, same as the bulk passes.
                                let prev_alias_scope = self.alias_scope.take();
                                self.alias_scope =
                                    Some(Self::compute_alias_scope(&items));
                                let reg_result = self.register_type_declaration(type_decl);
                                self.alias_scope = prev_alias_scope;
                                if let Err(e) = reg_result {
                                    tracing::debug!(
                                        "Failed to register type '{}' from module {}: {}",
                                        name,
                                        module_id,
                                        e
                                    );
                                    // Registration failed - create forward reference.
                                    // This handles cases like recursive types or types with
                                    // dependencies that haven't been resolved yet. The type
                                    // will be resolved in a later pass during normalization.
                                    let ty = Type::Named {
                                        path: verum_ast::ty::Path::single(
                                            verum_ast::ty::Ident::new(name, span),
                                        ),
                                        args: List::new(),
                                    };
                                    self.ctx.define_module_type(module_id, name, ty.clone());
                                    return Ok((ty, None));
                                }

                                // Type was registered - look it up again
                                if let Maybe::Some(ty) = self.ctx.lookup_type(name) {
                                    let ty_cloned = ty.clone();
                                    // Also store in module_type_defs for future qualified lookups
                                    self.ctx
                                        .define_module_type(module_id, name, ty_cloned.clone());
                                    return Ok((ty_cloned, Some(name.into())));
                                }

                                // Type was registered but not found in lookup.
                                // This can occur with complex type hierarchies or during
                                // incremental compilation. Return forward reference that
                                // will be resolved during type normalization.
                                let ty = Type::Named {
                                    path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
                                        name, span,
                                    )),
                                    args: List::new(),
                                };
                                return Ok((ty, None));
                            }
                        }
                    }

                    // Type not found in module
                    Err(TypeError::TypeNotFound {
                        name: name.to_text(),
                        span,
                    })
                }
                Err(_) => Err(TypeError::TypeNotFound {
                    name: name.to_text(),
                    span,
                }),
            }
        } else {
            Err(TypeError::TypeNotFound {
                name: name.to_text(),
                span,
            })
        }
    }

    /// Resolve a qualified type path (e.g., "std.collections.List")
    /// Name resolution across modules: qualified paths, import disambiguation, re-exports, path resolution in imports — .2 - Qualified path resolution
    pub(crate) fn resolve_qualified_type(&mut self, path: &Path, span: Span) -> Result<Type> {
        self.resolve_qualified_type_with_parameter_key(path, span)
            .map(|(ty, _)| ty)
    }

    /// Resolve the base and the declaration registration whose parameter
    /// slots describe that exact template. Forward references, projections
    /// and lexical type constructors have no selected declaration template.
    pub(super) fn resolve_qualified_type_with_parameter_key(
        &mut self,
        path: &Path,
        span: Span,
    ) -> Result<(Type, Option<Text>)> {
        use verum_ast::ty::PathSegment;

        if path.segments.is_empty() {
            return Err(TypeError::Other(
                "internal error: empty type path. This is a compiler bug — please report it."
                    .into(),
            ));
        }

        // Single segment - handle based on segment type
        if path.segments.len() == 1 {
            match &path.segments[0] {
                PathSegment::Name(ident) => {
                    // Simple name resolution for regular identifiers
                    return self.resolve_type_name_with_parameter_key(ident.name.as_str(), span);
                }
                PathSegment::SelfValue => {
                    // `self` as a type refers to the current self type in an impl block
                    if let Maybe::Some(self_ty) = &self.current_self_type {
                        return Ok((self_ty.clone(), None));
                    }
                    return Err(TypeError::Other(
                        "Cannot use `self` as a type outside of an implement block".into(),
                    ));
                }
                PathSegment::Super => {
                    // `super` alone cannot be a type - it's a module reference
                    return Err(TypeError::Other(
                        "`super` is not a type; use `super.TypeName` to reference a type from the parent module".into(),
                    ));
                }
                PathSegment::Cog => {
                    // `crate` alone cannot be a type - it's a module reference
                    return Err(TypeError::Other(
                        "`crate` is not a type; use `crate.module.TypeName` to reference a type from the crate root".into(),
                    ));
                }
                PathSegment::Relative => {
                    // `.` alone cannot be a type - it's a relative import marker
                    return Err(TypeError::Other(
                        "Relative import marker `.` is not a type; use `.TypeName` or `.module.TypeName`".into(),
                    ));
                }
            }
        }

        // CRITICAL: Handle associated type projections like Self.Item
        // These are paths starting with Self followed by an associated type name
        // They should be represented as projection types, NOT resolved via module system
        if let Some(PathSegment::SelfValue) = path.segments.first() {
            // #[cfg(debug_assertions)]
            // eprintln!("[DEBUG resolve_qualified_type] Path starts with Self, segments={}", path.segments.len());
            if path.segments.len() >= 2 {
                // This is an associated type projection: Self.Item, Self.Output, etc.
                // Get the associated type name(s)
                let assoc_parts: List<&str> = path
                    .segments
                    .iter()
                    .skip(1) // Skip Self
                    .filter_map(|seg| match seg {
                        PathSegment::Name(ident) => Some(ident.name.as_str()),
                        _ => None,
                    })
                    .collect();

                if !assoc_parts.is_empty() {
                    // Create a projection type using the convention "Self.AssocType"
                    // This will be resolved later when the concrete implementor is known
                    let projection_name: Text = format!("Self.{}", assoc_parts.join(".")).into();

                    // The base type is Self - get it if available, otherwise use a placeholder
                    let self_type = if let Maybe::Some(ref self_ty) = self.current_self_type {
                        self_ty.clone()
                    } else {
                        // Inside protocol definitions, Self is abstract
                        // Create a named type for Self that will be substituted later
                        Type::Named {
                            path: verum_ast::ty::Path::new(
                                List::from(vec![PathSegment::Name(verum_ast::ty::Ident::new(
                                    Text::from("Self"),
                                    span,
                                ))]),
                                span,
                            ),
                            args: List::new(),
                        }
                    };

                    // Return a Generic type representing the projection
                    // Format: Type::Generic { name: "::Item", args: [Self] }
                    // The "::" prefix indicates this is an associated type projection
                    let assoc_name: Text = Text::from(format!("::{}", assoc_parts.join(".")));
                    return Ok((
                        Type::Generic {
                            name: assoc_name,
                            args: List::from(vec![self_type]),
                        },
                        None,
                    ));
                }
            }
        }

        // Resolve relative syntax once, before selecting declaration authority.
        // Type annotations and record constructors use the same module-path
        // rules as mounts, including repeated parent segments.
        let absolute_path = matches!(path.segments.first(),
            Some(PathSegment::Super | PathSegment::Relative)).then(|| {
            let owner = verum_modules::ModulePath::from_str(self.current_module_path.as_str());
            let resolved = verum_modules::imports::path_to_module_path_resolved(path, &owner);
            Path::new(resolved.into_segments().into_iter().map(|name|
                PathSegment::Name(verum_ast::Ident::new(name, span))).collect(), span)
        });
        let path = absolute_path.as_ref().unwrap_or(path);
        // A namespace alias changes a path's spelling, never its nominal owner.
        // Resolve consumer aliases before retaining a type in a signature. A
        // declaring-source scope has its own lexical mounts and resolves those
        // below; it must not inherit the consumer's namespace bindings.
        let namespace_path = if !self.resolving_source_signature {
            path.segments.first().and_then(|head| {
                let PathSegment::Name(head) = head else { return None; };
                let owner = self.module_aliases.get(&head.name)?;
                let segments = owner.as_str().split('.')
                    .map(|name| PathSegment::Name(verum_ast::Ident::new(name, span)))
                    .chain(path.segments.iter().skip(1).cloned()).collect();
                Some(Path::new(segments, span))
            })
        } else { None };
        let path = namespace_path.as_ref().unwrap_or(path);

        // Resolve source authority before cached templates so an earlier
        // owner-local load cannot make a private declaration visible elsewhere.
        let qualified = self.path_to_string(path);
        if self.resolving_source_signature {
            // A qualified head in an archived declaration uses that owner's
            // retained namespace bindings, just like an unqualified type name.
            // It must not become an unresolved forward type in the consumer.
            return self.resolve_declaring_source_type_with_parameter_key(&qualified, span);
        }
        if let Some((ty, key)) =
            self.resolve_qualified_source_type_with_parameter_key(&qualified, span)?
        {
            return Ok((ty, Some(key)));
        }
        // A metadata-only provider may have no module registry entry. Its
        // semantic declaration must still replay before an uppercase forward
        // reference or a cached ABI template can answer for the same name.
        if let Some((owner, name)) = qualified.as_str().rsplit_once('.')
            && let Some(descriptor) = self.core_metadata.as_ref().and_then(|metadata|
                Self::metadata_type_with_declaring_key(metadata, &qualified))
            && descriptor.source_declaration.is_some()
        {
            if !descriptor.is_public && owner != self.current_module_path.as_str() {
                return Err(TypeError::VisibilityError {
                    name: name.into(), visibility: "Private".into(), module_path: owner.into(), span,
                });
            }
            return self.ensure_mounted_type_loaded_with_parameter_key(name, owner)
                .map(|(ty, key)| (ty, Some(key)))
                .ok_or_else(|| TypeError::TypeNotFound { name: qualified, span });
        }
        // Published exact archive identity wins over legacy leaf fallbacks.
        if let Some(ty) = self.ctx.lookup_type(qualified.as_str()) {
            return Ok((ty.clone(), Some(qualified)));
        }

        // Try resolving via inline modules first for qualified type paths
        // This handles types referenced as module.Type (e.g., math.Vector, data.models.User)
        // Also handles super.Type and crate.module.Type paths by resolving the prefix first
        {
            let has_crate_prefix = matches!(path.segments.first(), Some(PathSegment::Cog));
            let path_str: Text = if has_crate_prefix {
                // For crate.X.Y.Z, resolve to cog.X.Y.Z (skip the Cog segment, use cog prefix)
                let remaining: Vec<String> = path
                    .segments
                    .iter()
                    .skip(1)
                    .filter_map(|seg| match seg {
                        PathSegment::Name(ident) => Some(ident.name.as_str().to_string()),
                        _ => None,
                    })
                    .collect();
                // Return with cog prefix since inline modules are registered as "cog.X.Y"
                Text::from(format!("cog.{}", remaining.join(".")))
            } else {
                path.segments
                    .iter()
                    .filter_map(|seg| match seg {
                        PathSegment::Name(ident) => Some(ident.name.as_str().to_string()),
                        PathSegment::Super => Some("super".to_string()),
                        PathSegment::Cog => None,
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join(".").into()
            };

            if let Some((module_key, type_name)) =
                self.find_inline_module_for_import(&path_str, has_crate_prefix)
            {
                if let Some(module) = self
                    .inline_modules
                    .get(&verum_common::Text::from(module_key.as_str()))
                    .cloned()
                {
                    if let Some(items) = &module.items {
                        for item in items.iter() {
                            if let verum_ast::ItemKind::Type(type_decl) = &item.kind {
                                if type_decl.name.name.as_str() == type_name
                                    && self.cfg_evaluator.should_include_item(item)
                                {
                                    return self.load_source_type_declaration_with_parameter_key(
                                        type_decl, &module_key.into(), span,
                                    ).map(|(ty, key)| (ty, Some(key)));
                                }
                            }
                        }
                    }
                }
            }

            // For super/crate paths, also try resolving functions in inline modules
            // This handles cases like `crate.database.connection.Pool.new(...)` where Pool is found
            // above but the path includes a function call
            if has_crate_prefix {
                // Preserve unresolved type paths for the later loading pass;
                // an unrelated leaf entry is not evidence of this owner.
                if let Some(last_seg) = path.segments.last() {
                    if let PathSegment::Name(ident) = last_seg {
                        let last_name = ident.name.as_str();
                        if last_name.chars().next().is_some_and(|c| c.is_uppercase()) {
                            // Create a forward reference for the type
                            return Ok((
                                Type::Named {
                                    path: path.clone(),
                                    args: List::new(),
                                },
                                None,
                            ));
                        }
                    }
                }
            }
        }

        // Multi-segment path - use path resolution
        if let Maybe::Some(current_module) = self.current_module() {
            match self.module_resolver.resolve_path(path, current_module) {
                Ok(resolved) => {
                    // Verify it's a type
                    if resolved.kind != NameKind::Type {
                        return Err(TypeError::NotAType {
                            name: resolved.local_name.to_text(),
                            actual_kind: format!("{}", resolved.kind).into(),
                            span,
                        });
                    }

                    // Load type from the resolved module
                    let type_name = resolved.local_name.as_str();
                    if let Maybe::Some(ty) =
                        self.ctx.lookup_module_type(resolved.module_id, type_name)
                    {
                        // This legacy cache records no parameter-registration key.
                        return Ok((ty.clone(), None));
                    }

                    // CRITICAL FIX: Fall back to unqualified lookup for imported types
                    // When types are imported from other modules, they may be registered
                    // with define_type (unqualified) rather than define_module_type (qualified).
                    // This ensures we can find them even if they weren't stored in module_type_defs.
                    if let Maybe::Some(ty) = self.ctx.lookup_type(type_name) {
                        return Ok((ty.clone(), Some(type_name.into())));
                    }

                    // Type exists but not yet loaded - return named type reference
                    Ok((
                        Type::Named {
                            path: path.clone(),
                            args: List::new(),
                        },
                        None,
                    ))
                }
                Err(_module_err) => {
                    // Path not resolved - try fallback strategies before error

                    // Preserve the entire qualified forward reference. Uppercase
                    // leaves remain eligible for the existing deferred loading path,
                    // but do not resolve through an unrelated same-leaf declaration.
                    if let Some(last_seg) = path.segments.last() {
                        if let PathSegment::Name(ident) = last_seg {
                            let last_name = ident.name.as_str();
                            // Only do this if the last segment looks like a type name (starts uppercase)
                            if last_name.chars().next().is_some_and(|c| c.is_uppercase()) {
                                // Create a forward reference with its original owner.
                                return Ok((
                                    Type::Named {
                                        path: path.clone(),
                                        args: List::new(),
                                    },
                                    None,
                                ));
                            }
                        }
                    }
                    Err(TypeError::TypeNotFound {
                        name: self.path_to_string(path),
                        span,
                    })
                }
            }
        } else {
            // No module context
            let path_str = self.path_to_string(path);
            // Retain the owner on unresolved forward references.
            if let Some(last_seg) = path.segments.last() {
                if let PathSegment::Name(ident) = last_seg {
                    let last_name = ident.name.as_str();
                    if last_name.chars().next().is_some_and(|c| c.is_uppercase()) {
                        return Ok((
                            Type::Named {
                                path: path.clone(),
                                args: List::new(),
                            },
                            None,
                        ));
                    }
                }
            }
            Err(TypeError::TypeNotFound {
                name: path_str,
                span,
            })
        }
    }
    /// Extract module path segments from a nested Field expression.
    ///
    /// When we have `outer.inner.func()`, the receiver is:
    ///  Field { expr: Path("outer"), field: "inner" }
    ///
    /// This function recursively extracts the path segments: ["outer", "inner"]
    /// Returns None if the expression doesn't represent a module path.
    ///
    /// Module declaration: inline "module name { ... }" or file-based (foo.vr defines module foo) — Nested Inline Modules
    pub(crate) fn extract_module_path_from_field<'a>(
        &self,
        expr: &'a Expr,
        final_field: &'a verum_ast::Ident,
    ) -> Option<Vec<&'a str>> {
        match &expr.kind {
            ExprKind::Path(path) if path.segments.len() == 1 => {
                if let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0] {
                    Some(vec![ident.name.as_str(), final_field.name.as_str()])
                } else {
                    None
                }
            }
            ExprKind::Field {
                expr: inner_expr,
                field: inner_field,
            } => {
                // Recursively extract from nested Field
                let mut segments = self.extract_module_path_from_field(inner_expr, inner_field)?;
                segments.push(final_field.name.as_str());
                Some(segments)
            }
            _ => None,
        }
    }

    /// Field syntax retains namespace keywords until the complete qualified
    /// value can be resolved. A standalone namespace never has a value type.
    pub(super) fn rooted_field_path(&self, expression: &Expr) -> Option<Path> {
        use verum_ast::ty::PathSegment;
        fn collect(expression: &Expr, segments: &mut List<PathSegment>) -> Option<()> {
            match &expression.kind {
                ExprKind::Path(path) => segments.extend(path.segments.iter().cloned()),
                ExprKind::Field { expr, field } => {
                    collect(expr, segments)?;
                    segments.push(PathSegment::Name(field.clone()));
                }
                _ => return None,
            }
            Some(())
        }
        let mut segments = List::new();
        collect(expression, &mut segments)?;
        if !matches!(segments.first(), Some(PathSegment::Cog | PathSegment::Super)) {
            return None;
        }
        Some(Path::new(segments, expression.span))
    }

    /// Compute the parent module path from the current module path and a path with `super` segments.
    ///
    /// # Arguments
    /// * `current_path` - The current module path (e.g., "cog.outer.inner")
    /// * `segments` - The path segments starting with `Super` (e.g., [Super] or [Super, Name("sibling")])
    ///
    /// # Returns
    /// * `Some(parent_path)` - The resolved parent module path
    /// * `None` - If there are too many `super` segments
    ///
    /// # Example
    /// From "cog.outer.inner" with [Super], returns Some("cog.outer")
    /// From "cog.outer.inner" with [Super, Super], returns Some("cog")
    ///
    /// Circular import handling: detection and error reporting for cyclic module dependencies — Relative module paths
    pub(crate) fn compute_parent_module_path(
        &self,
        current_path: &Text,
        segments: &[verum_ast::ty::PathSegment],
    ) -> Option<String> {
        use verum_ast::ty::PathSegment;

        // Split current path into parts
        let mut parts: Vec<&str> = current_path.as_str().split('.').collect();

        // Process segments
        for segment in segments.iter() {
            match segment {
                PathSegment::Super => {
                    // Go up one level
                    if parts.is_empty() {
                        return None; // Can't go above root
                    }
                    parts.pop();
                }
                PathSegment::Name(ident) => {
                    // This would be a sibling navigation (super.sibling)
                    // For function calls, we don't add the name - it's the method
                    // So we just stop processing segments
                    break;
                }
                _ => {
                    // Other segment types not supported in super paths
                    break;
                }
            }
        }

        if parts.is_empty() {
            // Resolved to crate root level
            Some(String::new())
        } else {
            Some(parts.join("."))
        }
    }

    /// Resolve a multi-segment path expression
    ///
    /// Import and re-export system: "mount module.{item1, item2}" for imports, pub use for re-exports, glob imports — Cross-module resolution
    ///
    /// Handles paths like:
    /// - `module::function` - Function from another module
    /// - `module::Type::associated_fn` - Associated function on a type
    /// - `cog.module.item` - Cog-rooted path
    /// Resolve a path where the first segment is a variable.
    /// Converts path `p.x.y` where `p` is a variable into a chain of field accesses.
    /// This handles cases like `match p.x { ... }` where the parser creates a Path
    /// but semantically it should be field access.
    pub(crate) fn resolve_variable_field_access(
        &mut self,
        path: &Path,
        span: Span,
    ) -> Result<InferResult> {
        use verum_ast::ty::PathSegment;

        let Some(PathSegment::Name(root)) = path.segments.first() else {
            return Err(TypeError::Other("expected a local record receiver".into()));
        };
        self.resolve_local_field_chain(root.clone(), path, span)
    }

    /// Both parser representations of a local field access use ordinary
    /// projection, including visibility, partial moves and dependent indices.
    fn resolve_local_field_chain(
        &mut self, root: verum_ast::Ident, path: &Path, span: Span,
    ) -> Result<InferResult> {
        let mut receiver = Expr::ident(root);
        for segment in path.segments.iter().skip(1) {
            let verum_ast::ty::PathSegment::Name(field) = segment else {
                return Err(TypeError::Other("expected a field name in local projection".into()));
            };
            receiver = Expr::new(ExprKind::Field {
                expr: Box::new(receiver),
                field: field.clone(),
            }, span);
        }
        self.synth_expr(&receiver)
    }

    /// Identify the first actual inline declaration in a lexical or absolute
    /// path. Canonical module keys may occupy several syntax segments.
    fn inline_namespace_prefix(&self, path: &Path) -> Option<(Text, usize)> {
        use verum_ast::ty::PathSegment;
        let PathSegment::Name(first) = path.segments.first()? else { return None; };
        if self.ctx.env.is_locally_bound(first.name.as_str()) { return None; }
        if let Some(owner) = self.inline_namespace_owner(first.name.as_str()) {
            return Some((owner, 1));
        }
        let mut owner = Text::new();
        for (index, segment) in path.segments.iter().enumerate().take(path.segments.len().saturating_sub(1)) {
            let PathSegment::Name(name) = segment else { return None; };
            if index != 0 { owner.push('.'); }
            owner.push_str(name.name.as_str());
            if self.inline_modules.get(&owner).is_some_and(|module| module.items.is_some()) {
                return Some((owner, index + 1));
            }
        }
        None
    }

    /// Resolve a path that starts with an inline module.
    /// Handles paths like `api.v2.func()` where `api` is an inline module.
    ///
    /// Navigation:
    /// 1. Start with the first segment (inline module)
    /// 2. For each subsequent segment, look inside the current module's items
    /// 3. Final segment should be a function or type
    ///
    /// Module declaration: inline "module name { ... }" or file-based (foo.vr defines module foo) — Inline Modules
    pub(crate) fn resolve_inline_module_path(
        &mut self,
        path: &Path,
        span: Span,
    ) -> Result<InferResult> {
        self.resolve_inline_module_path_with_arguments(path, span, None)
    }

    pub(super) fn resolve_inline_module_path_with_arguments(
        &mut self,
        path: &Path,
        span: Span,
        arguments: Option<&List<verum_ast::ty::GenericArg>>,
    ) -> Result<InferResult> {
        use verum_ast::ItemKind;
        use verum_ast::ty::PathSegment;

        // Extract path segments as strings
        let segments: Vec<&str> = path
            .segments
            .iter()
            .filter_map(|seg| {
                if let PathSegment::Name(ident) = seg {
                    Some(ident.name.as_str())
                } else {
                    None
                }
            })
            .collect();

        if segments.is_empty() {
            return Err(TypeError::Other("Empty path".into()));
        }

        let (first_module_name, consumed) = self.inline_namespace_prefix(path)
            .ok_or_else(|| TypeError::UnboundVariable {
                name: self.path_to_string(path), span,
            })?;
        let mut current_module = self
            .inline_modules
            .get(&first_module_name)
            .cloned()
            .ok_or_else(|| TypeError::UnboundVariable {
                name: first_module_name.clone(),
                span,
            })?;

        // Navigate through intermediate segments (modules)
        // For path `api.v2.func`, navigate: api -> v2 -> func
        for (i, &segment_name) in segments.iter().enumerate().skip(consumed) {
            let segment_text = verum_common::Text::from(segment_name);

            // Check if this is the last segment (item to resolve)
            let is_last = i == segments.len() - 1;

            // Look for the segment in current module's items
            if let Some(items) = &current_module.items {
                let mut found = false;

                for item in items.iter() {
                    match &item.kind {
                        // Nested module - navigate into it
                        ItemKind::Module(nested_module)
                            if nested_module.name.name.as_str() == segment_name =>
                        {
                            if is_last {
                                // Path ends at a module - this might be an error
                                // or the user wants the module as a value (not supported yet)
                                return Err(TypeError::Other(verum_common::Text::from(format!(
                                    "Path '{}' resolves to a module, not a value",
                                    self.path_to_string(path)
                                ))));
                            }
                            current_module = nested_module.clone();
                            found = true;
                            break;
                        }

                        // Function - return its type (only for last segment)
                        ItemKind::Function(func) if func.name.name.as_str() == segment_name => {
                            if is_last {
                                // Check visibility - function must be public to access from outside
                                if !matches!(func.visibility, verum_ast::decl::Visibility::Public) {
                                    return Err(TypeError::VisibilityError {
                                        name: verum_common::Text::from(segment_name),
                                        visibility: verum_common::Text::from(format!(
                                            "{:?}",
                                            func.visibility
                                        )),
                                        module_path: verum_common::Text::from(
                                            current_module.name.name.as_str(),
                                        ),
                                        span,
                                    });
                                }
                                let func_ty = self.infer_function_type_with_arguments(func, arguments)?;
                                return Ok(InferResult::new(func_ty));
                            } else {
                                return Err(TypeError::Other(verum_common::Text::from(format!(
                                    "'{}' is a function, not a module",
                                    segment_name
                                ))));
                            }
                        }

                        // Type - return it (only for last segment)
                        ItemKind::Type(type_decl)
                            if type_decl.name.name.as_str() == segment_name =>
                        {
                            if is_last {
                                // Check visibility - type must be public to access from outside
                                if !matches!(
                                    type_decl.visibility,
                                    verum_ast::decl::Visibility::Public
                                ) {
                                    return Err(TypeError::VisibilityError {
                                        name: verum_common::Text::from(segment_name),
                                        visibility: verum_common::Text::from(format!(
                                            "{:?}",
                                            type_decl.visibility
                                        )),
                                        module_path: verum_common::Text::from(
                                            current_module.name.name.as_str(),
                                        ),
                                        span,
                                    });
                                }
                                // Return the type as a Named type
                                let type_path = verum_ast::ty::Path {
                                    segments: vec![PathSegment::Name(type_decl.name.clone())]
                                        .into(),
                                    span,
                                };
                                return Ok(InferResult::new(Type::Named {
                                    path: type_path,
                                    args: List::new(),
                                }));
                            } else {
                                return Err(TypeError::Other(verum_common::Text::from(format!(
                                    "'{}' is a type, not a module",
                                    segment_name
                                ))));
                            }
                        }

                        // Constant
                        ItemKind::Const(const_decl)
                            if const_decl.name.name.as_str() == segment_name =>
                        {
                            if is_last {
                                // Check visibility - constant must be public to access from outside
                                if !matches!(
                                    const_decl.visibility,
                                    verum_ast::decl::Visibility::Public
                                ) {
                                    return Err(TypeError::VisibilityError {
                                        name: verum_common::Text::from(segment_name),
                                        visibility: verum_common::Text::from(format!(
                                            "{:?}",
                                            const_decl.visibility
                                        )),
                                        module_path: verum_common::Text::from(
                                            current_module.name.name.as_str(),
                                        ),
                                        span,
                                    });
                                }
                                let const_ty = self.ast_to_type(&const_decl.ty)?;
                                return Ok(InferResult::new(const_ty));
                            } else {
                                return Err(TypeError::Other(verum_common::Text::from(format!(
                                    "'{}' is a constant, not a module",
                                    segment_name
                                ))));
                            }
                        }

                        _ => {}
                    }
                }

                if !found && !is_last {
                    return Err(TypeError::Other(verum_common::Text::from(format!(
                        "Module '{}' not found in '{}'",
                        segment_name,
                        current_module.name.name.as_str()
                    ))));
                }

                if !found && is_last {
                    // STATIC-OVER-MODULE-ITEM (#40): a selectively-imported
                    // TYPE can be registered as an inline module too
                    // (`mount core.base.result.{Validated}`), so
                    // `Validated.valid(42)` routes HERE — and the item scan
                    // above only sees the module stub's AST items, never the
                    // impl-block statics that import_impl_blocks_for_type
                    // registered in env under the QUALIFIED key
                    // ("Validated.valid"). Pre-fix this arm raised
                    // `unbound variable: valid` and every static on such a
                    // type was unreachable (base/result: 166 failures).
                    // Before erroring, consult the qualified env scheme —
                    // exactly Strategy 3's discipline.
                    let qualified = segments.join(".");
                    if let Some(scheme) = self.ctx.env.lookup(qualified.as_str()).cloned() {
                        let instance = self.instantiate_value_instance(&scheme, span)?;
                        if let Some(arguments) = arguments.filter(|arguments| !arguments.is_empty()) {
                            let slots = &instance.explicit_method_vars;
                            self.bind_explicit_method_arguments(slots.as_ref(), arguments,
                                &verum_ast::Ident::new(segment_text.clone(), span), span)?;
                        }
                        return Ok(InferResult::new(instance.ty));
                    }
                    return Err(TypeError::UnboundVariable {
                        name: verum_common::Text::from(segment_name),
                        span,
                    });
                }
            } else {
                // A namespace declaration is not a value declaration. Missing
                // source items must not manufacture an inhabitant of bottom.
                return Err(TypeError::UnboundVariable {
                    name: self.path_to_string(path), span,
                });
            }
        }

        // Should not reach here - the loop should handle all cases
        Err(TypeError::Other(verum_common::Text::from(format!(
            "Failed to resolve path '{}'",
            self.path_to_string(path)
        ))))
    }

    /// - `super::module::item` - Parent module path
    /// - `Type::associated_const` - Associated constant
    ///
    /// Resolution strategy:
    /// 1. Check for special keywords (cog, super, self)
    /// 2. Try to resolve as module path via NameResolver
    /// 3. Try to resolve as type with associated item
    /// 4. Report detailed error with suggestions
    pub(crate) fn resolve_multi_segment_path(
        &mut self,
        path: &Path,
        span: Span,
    ) -> Result<InferResult> {
        use verum_ast::ty::PathSegment;

        // Handle special first segment keywords
        if let Some(first) = path.segments.first() {
            match first {
                PathSegment::Cog => {
                    return self.resolve_crate_rooted_path(path, span);
                }
                PathSegment::Super => {
                    return self.resolve_super_path(path, span);
                }
                PathSegment::SelfValue => {
                    return self.resolve_self_path(path, span);
                }
                PathSegment::Relative => {
                    return self.resolve_super_path(path, span); // Relative imports work like super
                }
                PathSegment::Name(ident) => {
                    // CRITICAL FIX: Check if first segment is a local variable
                    // If so, treat multi-segment path as field access chain (p.x.y -> p.x.y)
                    // This handles cases like `match p.x { ... }` where `p` is a variable
                    // and `x` is a field, not a module path.
                    let name = ident.name.as_str();
                    if self.ctx.env.lookup(name).is_some() {
                        // STATIC-OVER-FIELD (#40): the first segment being an
                        // env binding does NOT always mean variable-field
                        // access. A selectively-imported TYPE name lands in
                        // env too (`mount core.base.result.{Validated}` binds
                        // "Validated"), and its impl-block statics are
                        // registered under the QUALIFIED key
                        // ("Validated.valid"). Pre-fix, `Validated.valid(42)`
                        // took the field-access branch and died with
                        // `unbound variable: valid` before Strategy 3's
                        // qualified env lookup could fire — every static on a
                        // selectively-imported sum type was unreachable
                        // (base/result: 166 failures). When the full
                        // qualified path IS a registered env scheme, prefer
                        // it; a genuine variable (`p.x.y`) never has a
                        // qualified "p.x.y" env key, so field access is
                        // unaffected.
                        let qualified = path
                            .segments
                            .iter()
                            .filter_map(|s| {
                                if let PathSegment::Name(id) = s {
                                    Some(id.name.as_str())
                                } else {
                                    None
                                }
                            })
                            .collect::<Vec<_>>()
                            .join(".");
                        if self.ctx.env.lookup(qualified.as_str()).is_none() {
                            // First segment is a variable - convert path to field access chain
                            return self.resolve_variable_field_access(path, span);
                        }
                        // fall through: Strategy 3 resolves the qualified static
                    }

                    // Check if first segment is an inline module
                    // This handles paths like `api.v2.func()` where `api` is an inline module
                    // Module declaration: inline "module name { ... }" or file-based (foo.vr defines module foo) — Inline Modules
                    if self.inline_namespace_prefix(path).is_some() {
                        return self.resolve_inline_module_path(path, span);
                    }

                    // Continue with normal resolution
                }
            }
        }

        // Extract path segments as strings
        let temp_segments: Vec<Text> = path
            .segments
            .iter()
            .filter_map(|seg| {
                if let PathSegment::Name(ident) = seg {
                    Some(ident.name.clone())
                } else {
                    None
                }
            })
            .collect();
        let segments: List<verum_common::Text> = temp_segments.into();

        if segments.is_empty() {
            return Err(TypeError::Other("Empty path".into()));
        }

        // Strategy 1: Try as module path (module::item)
        if let Maybe::Some(current_module) = self.current_module() {
            match self.module_resolver.resolve_path(path, current_module) {
                Ok(resolved) => {
                    // Successfully resolved via module system
                    let local_name = resolved.local_name.as_str();

                    // Check what kind of item it resolved to
                    match resolved.kind {
                        NameKind::Function => {
                            // Look up function type from resolved module
                            if let Maybe::Some(ty) =
                                self.ctx.lookup_module_type(resolved.module_id, local_name)
                            {
                                return Ok(InferResult::new(ty.clone()));
                            }
                            // Function not yet loaded - try module registry.
                            // Snapshot the AST outside the read-guard so that
                            // infer_function_type below (which takes &mut self)
                            // doesn't alias the registry borrow.
                            let items_snapshot: Option<Vec<_>> = {
                                let reg = self.module_registry.read();
                                reg.get(resolved.module_id)
                                    .map(|m| m.ast.items.iter().cloned().collect())
                            };
                            if let Some(items) = items_snapshot {
                                for item in &items {
                                    if let verum_ast::ItemKind::Function(func_decl) = &item.kind
                                        && func_decl.name.name.as_str() == local_name
                                    {
                                        // Found the function - infer its type
                                        let func_ty = self.infer_function_type(func_decl)?;
                                        self.ctx.define_module_type(
                                            resolved.module_id,
                                            local_name,
                                            func_ty.clone(),
                                        );
                                        return Ok(InferResult::new(func_ty));
                                    }
                                }
                                // **Impl-method fallback**: when local_name
                                // has shape `Type.method` (the module
                                // resolver hands back the qualified Type
                                // .method form for paths like
                                // `Map.new` after multi-mount-driven
                                // disambiguation), the top-level
                                // ItemKind::Function walk above misses
                                // the entry — `new` is declared inside
                                // an `implement Map { ... }` block, not
                                // as a top-level fn.  Search ImplDecl
                                // blocks here for the matching method
                                // so the fallback covers both top-level
                                // fns AND impl-method dispatch.
                                //
                                // Pre-fix: 10 tests failed with
                                // `unbound variable: core.collections.map.Map.new`
                                // when `mount core.collections.map.Map`
                                // appeared alongside any sibling
                                // collection mount (List/Set) that
                                // triggered the resolver to return the
                                // dotted `local_name` form instead of
                                // the bare method name.
                                if let Some(dot_idx) = local_name.find('.') {
                                    let type_part = &local_name[..dot_idx];
                                    let method_part = &local_name[dot_idx + 1..];
                                    for item in &items {
                                        let verum_ast::ItemKind::Impl(impl_decl) = &item.kind
                                        else {
                                            continue;
                                        };
                                        // Match the impl-block target type's last segment.
                                        let target_name = match &impl_decl.kind {
                                            verum_ast::decl::ImplKind::Inherent(target) => target,
                                            verum_ast::decl::ImplKind::Protocol {
                                                for_type, ..
                                            } => for_type,
                                        };
                                        // Walk a Type AST node down to its head Path's last
                                        // segment.  Handles both `Map` (TypeKind::Path) and
                                        // `Map<K, V>` (TypeKind::Generic wrapping the Path).
                                        fn head_segment(
                                            ty: &verum_ast::ty::Type,
                                        ) -> Option<&str> {
                                            match &ty.kind {
                                                verum_ast::ty::TypeKind::Path(p) => p
                                                    .segments
                                                    .last()
                                                    .and_then(|s| match s {
                                                        verum_ast::ty::PathSegment::Name(id) => {
                                                            Some(id.name.as_str())
                                                        }
                                                        _ => None,
                                                    }),
                                                verum_ast::ty::TypeKind::Generic {
                                                    base, ..
                                                } => head_segment(base),
                                                _ => None,
                                            }
                                        }
                                        let target_last = head_segment(target_name);
                                        if target_last != Some(type_part) {
                                            continue;
                                        }
                                        for impl_item in impl_decl.items.iter() {
                                            let verum_ast::decl::ImplItemKind::Function(func_decl) =
                                                &impl_item.kind
                                            else {
                                                continue;
                                            };
                                            if func_decl.name.name.as_str() != method_part {
                                                continue;
                                            }
                                            let func_ty = self.infer_function_type(func_decl)?;
                                            self.ctx.define_module_type(
                                                resolved.module_id,
                                                local_name,
                                                func_ty.clone(),
                                            );
                                            return Ok(InferResult::new(func_ty));
                                        }
                                    }
                                }
                            }
                            return Err(TypeError::UnboundVariable {
                                name: self.path_to_string(path),
                                span,
                            });
                        }
                        NameKind::Type => {
                            // Type - could be followed by associated item
                            // For now, return error - types are not expressions
                            return Err(TypeError::Other(verum_common::Text::from(format!(
                                "`{}` is a type, not a value. Did you mean to use an associated function or constant?",
                                self.path_to_string(path)
                            ))));
                        }
                        NameKind::Const | NameKind::Static => {
                            // Look up constant type
                            if let Maybe::Some(ty) =
                                self.ctx.lookup_module_type(resolved.module_id, local_name)
                            {
                                return Ok(InferResult::new(ty.clone()));
                            }
                            return Err(TypeError::UnboundVariable {
                                name: self.path_to_string(path),
                                span,
                            });
                        }
                        _ => {
                            // Other kinds - try generic lookup
                            if let Maybe::Some(ty) =
                                self.ctx.lookup_module_type(resolved.module_id, local_name)
                            {
                                return Ok(InferResult::new(ty.clone()));
                            }
                        }
                    }
                }
                Err(_) => {
                    // Module resolution failed - try other strategies
                }
            }
        }

        // Strategy 2: Try as Type::associated_item or Type::Variant
        // First segment might be a type name in current scope
        if segments.len() >= 2 {
            let type_name = segments[0].as_str();

            // Check if first segment is a known type
            if let Maybe::Some(ty) = self.ctx.lookup_type(type_name) {
                // Found type - now look for associated item or variant constructor
                let item_name = segments[1].as_str();

                // Strategy 2a: Check if type is a Variant type and item is a variant constructor
                // Variant type constructors: sum type variants act as constructor functions (Some(x), None, etc.)
                if let Type::Variant(variants) = &ty
                    && let Some(payload_ty) = variants.get(item_name)
                {
                    // Found variant constructor - return function type
                    // If payload is Unit, it's a nullary constructor (value)
                    // Otherwise it's a constructor function
                    if matches!(payload_ty, Type::Unit) {
                        // Nullary variant - return the variant type itself
                        return Ok(InferResult::new(ty.clone()));
                    } else {
                        // Constructor function: fn(payload_ty) -> VariantType
                        // For tuple variants with multiple fields, unpack into multiple parameters
                        let params = match payload_ty {
                            Type::Tuple(tuple_types) => tuple_types.clone(),
                            _ => {
                                let mut p = List::new();
                                p.push(payload_ty.clone());
                                p
                            }
                        };
                        let constructor_ty = Type::function(params, ty.clone());
                        return Ok(InferResult::new(constructor_ty));
                    }
                }

                // Check for associated functions/constants on the type
                if let Type::Named {
                    path: type_path, ..
                } = &ty
                {
                    // Look up protocol implementations for this type
                    if let Some(assoc_ty) = self.lookup_associated_item(type_path, item_name) {
                        return Ok(InferResult::new(assoc_ty));
                    }
                }

                // Try looking in impl blocks
                if let Some(assoc_ty) = self.lookup_impl_item(ty.clone(), item_name, span)? {
                    return Ok(InferResult::new(assoc_ty));
                }
            }
        }

        // Strategy 3: Check local environment with qualified name
        let qualified_name = segments.join(".");
        if let Some(scheme) = self.ctx.env.lookup(qualified_name.as_str()).cloned() {
            let ty = self.instantiate_value_scheme(&scheme, span)?;
            return Ok(InferResult::new(ty));
        }

        // Provide helpful error message
        let path_str = self.path_to_string(path);
        let suggestions = self.generate_path_suggestions(&segments);

        Err(TypeError::UnboundVariable {
            name: if suggestions.is_empty() {
                path_str
            } else {
                verum_common::Text::from(format!(
                    "{}. Did you mean one of: {}?",
                    path_str,
                    suggestions.join(", ")
                ))
            },
            span,
        })
    }

    /// Resolve a constructor through the selected type declaration, retaining
    /// its exact module key, visibility and parameter telescope. A missing
    /// owner never asks the ambient constructor table for a same-named case.
    pub(super) fn declared_constructor_path(
        &mut self, path: &Path, span: Span,
    ) -> Result<Option<InferResult>> {
        use verum_ast::ty::PathSegment;
        use verum_modules::{ModulePath, visibility::VisibilityChecker};
        let Some((PathSegment::Name(case), prefix)) = path.segments.split_last() else {
            return Ok(None);
        };
        if prefix.len() < 2 { return Ok(None); }
        let prefix = Path::new(prefix.iter().cloned().collect(), span);
        let owner_scope = ModulePath::from_str(self.current_module_path.as_str());
        let qualified = verum_modules::imports::path_to_module_path_resolved(&prefix, &owner_scope).to_string();
        let Some((module_name, type_name)) = qualified.rsplit_once('.') else { return Ok(None); };
        // Registry aliases are declarations. Legacy namespace-prefix guesses
        // in other lookup paths cannot authorize this qualified request.
        let source_exists = self.module_registry.read().get_by_path_aliased(module_name).is_some();
        let inline = self.inline_modules.get(&Text::from(module_name));
        if !source_exists && inline.is_none() { return Ok(None); }
        if let Some(declaration) = inline.and_then(|module| module.items.as_ref())
            .and_then(|items| items.iter().find_map(|item| match &item.kind {
                verum_ast::ItemKind::Type(decl) if decl.name.name.as_str() == type_name => Some(decl),
                _ => None,
            }))
        {
            let visibility = declaration.visibility.resolve_declared_scope(module_name)
                .map_err(|message| TypeError::VisibilityError { name: type_name.into(), visibility: message,
                    module_path: module_name.into(), span })?;
            if !VisibilityChecker::new().is_visible(visibility.clone(), &ModulePath::from_str(module_name), &owner_scope) {
                return Err(TypeError::VisibilityError { name: type_name.into(), visibility: format!("{visibility:?}").into(),
                    module_path: module_name.into(), span });
            }
        }
        let selected = match self.resolve_qualified_type_with_parameter_key(&prefix, span) {
            Ok((_, Some(owner))) => self.canonical_alias_target(&owner),
            Ok((_, None)) => return Ok(None),
            Err(TypeError::TypeNotFound { name, .. }) if name.as_str() == qualified => return Ok(None),
            Err(error) => return Err(error),
        };
        let declared_case = self.unifier.declared_variant_template(&selected)
            .is_some_and(|(_, body)| matches!(body, Type::Variant(cases) if cases.contains_key(&case.name)));
        if !declared_case { return Ok(None); }
        self.selected_constructor_value(selected.as_str(), case.name.as_str(), span)
            .map(|ty| Some(InferResult::new(ty)))
    }

    /// Resolve cog-rooted path (cog.module.item)
    pub(crate) fn resolve_crate_rooted_path(
        &mut self,
        path: &Path,
        span: Span,
    ) -> Result<InferResult> {
        use verum_ast::ty::PathSegment;

        if let Some(value) = self.declared_constructor_path(path, span)? { return Ok(value); }

        // The module resolver handles crate:: resolution via resolve_path
        if let Maybe::Some(current_module) = self.current_module() {
            // resolve_path handles crate:: prefix properly
            if let Ok(resolved) = self.module_resolver.resolve_path(path, current_module)
                && let Maybe::Some(ty) = self
                    .ctx
                    .lookup_module_type(resolved.module_id, resolved.local_name.as_str())
            {
                return Ok(InferResult::new(ty.clone()));
            }
        }

        // Try inline module resolution: crate.X.Y.Z -> resolve via inline modules
        // Build the path without the crate prefix, using cog. prefix for inline module lookup
        {
            let remaining: Vec<String> = path
                .segments
                .iter()
                .skip(1)
                .filter_map(|seg| match seg {
                    PathSegment::Name(ident) => Some(ident.name.as_str().to_string()),
                    _ => None,
                })
                .collect();
            if !remaining.is_empty() {
                let path_str = format!("cog.{}", remaining.join("."));
                // Try as module.item
                if let Some((module_key, item_name)) =
                    self.find_inline_module_for_import(&path_str, true)
                {
                    // Build a synthetic path with only name segments for inline module resolution
                    let synthetic_segments: List<PathSegment> = remaining
                        .iter()
                        .map(|s| PathSegment::Name(verum_ast::ty::Ident::new(s.as_str(), span)))
                        .collect();
                    let synthetic_path = verum_ast::ty::Path::new(synthetic_segments, span);

                    // First try looking up the first segment as inline module
                    if self
                        .inline_modules
                        .contains_key(&verum_common::Text::from(remaining[0].as_str()))
                    {
                        if let Ok(result) = self.resolve_inline_module_path(&synthetic_path, span) {
                            return Ok(result);
                        }
                    }
                    // Try with cog-prefixed module key
                    let first_cog = format!("cog.{}", remaining[0]);
                    if self
                        .inline_modules
                        .contains_key(&verum_common::Text::from(first_cog.as_str()))
                    {
                        // Navigate inline modules starting from the cog-prefixed module
                        if let Some(module) = self
                            .inline_modules
                            .get(&verum_common::Text::from(module_key.as_str()))
                            .cloned()
                        {
                            if let Some(items) = &module.items {
                                for item in items.iter() {
                                    match &item.kind {
                                        verum_ast::ItemKind::Type(type_decl)
                                            if type_decl.name.name.as_str() == item_name =>
                                        {
                                            // ALIAS-VS-MARKER scope (#41)
                                            let prev_alias_scope = self.alias_scope.take();
                                            self.alias_scope =
                                                Some(Self::compute_alias_scope(items));
                                            let reg_result =
                                                self.register_type_declaration(type_decl);
                                            self.alias_scope = prev_alias_scope;
                                            if let Err(e) = reg_result {
                                                tracing::debug!(
                                                    "Failed to register type from crate path: {}",
                                                    e
                                                );
                                            }
                                            if let Maybe::Some(ty) =
                                                self.ctx.lookup_type(&item_name)
                                            {
                                                return Ok(InferResult::new(ty.clone()));
                                            }
                                            return Ok(InferResult::new(Type::Named {
                                                path: verum_ast::ty::Path::single(
                                                    verum_ast::ty::Ident::new(
                                                        item_name.as_str(),
                                                        span,
                                                    ),
                                                ),
                                                args: List::new(),
                                            }));
                                        }
                                        verum_ast::ItemKind::Function(func)
                                            if func.name.name.as_str() == item_name =>
                                        {
                                            let func_ty = self.infer_function_type(func)?;
                                            return Ok(InferResult::new(func_ty));
                                        }
                                        verum_ast::ItemKind::Const(c)
                                            if c.name.name.as_str() == item_name =>
                                        {
                                            let const_ty = self.ast_to_type(&c.ty)?;
                                            return Ok(InferResult::new(const_ty));
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

        Err(TypeError::UnboundVariable {
            name: self.path_to_string(path), span,
        })
    }

    /// Resolve super path (super::module::item)
    pub(crate) fn resolve_super_path(&mut self, path: &Path, span: Span) -> Result<InferResult> {
        use verum_ast::ty::PathSegment;

        // File-based modules share the same export graph as mounted calls.
        // Resolve every leading parent step before selecting the callable.
        let current = verum_modules::ModulePath::from_str(&self.current_module_path);
        let target = verum_modules::imports::path_to_module_path_resolved(path, &current);
        if let (Some(owner), Some(name)) = (target.parent(), target.segments().last()) {
            let source = self.module_registry.read().get_by_path(&owner.to_string());
            if let Some(module) = source
                && let Some((scheme, _)) = self.source_qualified_callable(module, name.clone())
            {
                return Ok(InferResult::new(self.instantiate_value_scheme(&scheme, span)?));
            }
        }

        // The module resolver handles super:: resolution via resolve_path
        if let Maybe::Some(current_module) = self.current_module() {
            // resolve_path handles super:: prefix by looking at module_parents
            if let Ok(resolved) = self.module_resolver.resolve_path(path, current_module)
                && let Maybe::Some(ty) = self
                    .ctx
                    .lookup_module_type(resolved.module_id, resolved.local_name.as_str())
            {
                return Ok(InferResult::new(ty.clone()));
            }
        }

        // Try inline module resolution: super.X -> resolve parent module + X
        {
            let current = self.current_module_path.as_str().to_string();
            let parent = if let Some(dot_pos) = current.rfind('.') {
                current[..dot_pos].to_string()
            } else {
                "cog".to_string()
            };
            // Build remaining segments after super
            let remaining: Vec<String> = path
                .segments
                .iter()
                .skip(1)
                .filter_map(|seg| match seg {
                    PathSegment::Name(ident) => Some(ident.name.as_str().to_string()),
                    _ => None,
                })
                .collect();
            if !remaining.is_empty() {
                let resolved_path = format!("{}.{}", parent, remaining.join("."));
                if let Some((module_key, item_name)) =
                    self.find_inline_module_for_import(&resolved_path, false)
                {
                    if let Some(module) = self
                        .inline_modules
                        .get(&verum_common::Text::from(module_key.as_str()))
                        .cloned()
                    {
                        if let Some(items) = &module.items {
                            for item in items.iter() {
                                match &item.kind {
                                    verum_ast::ItemKind::Type(type_decl)
                                        if type_decl.name.name.as_str() == item_name =>
                                    {
                                        // ALIAS-VS-MARKER scope (#41)
                                        let prev_alias_scope = self.alias_scope.take();
                                        self.alias_scope =
                                            Some(Self::compute_alias_scope(items));
                                        let reg_result =
                                            self.register_type_declaration(type_decl);
                                        self.alias_scope = prev_alias_scope;
                                        if let Err(e) = reg_result {
                                            tracing::debug!(
                                                "Failed to register type from super path: {}",
                                                e
                                            );
                                        }
                                        if let Maybe::Some(ty) = self.ctx.lookup_type(&item_name) {
                                            return Ok(InferResult::new(ty.clone()));
                                        }
                                        return Ok(InferResult::new(Type::Named {
                                            path: verum_ast::ty::Path::single(
                                                verum_ast::ty::Ident::new(item_name.as_str(), span),
                                            ),
                                            args: List::new(),
                                        }));
                                    }
                                    verum_ast::ItemKind::Function(func)
                                        if func.name.name.as_str() == item_name =>
                                    {
                                        let func_ty = self.infer_function_type(func)?;
                                        return Ok(InferResult::new(func_ty));
                                    }
                                    verum_ast::ItemKind::Const(c)
                                        if c.name.name.as_str() == item_name =>
                                    {
                                        let const_ty = self.ast_to_type(&c.ty)?;
                                        return Ok(InferResult::new(const_ty));
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
                // Also try navigating into the parent module's inline module tree
                let parent_key = verum_common::Text::from(parent.as_str());
                if self.inline_modules.contains_key(&parent_key) {
                    let synthetic_segments: List<PathSegment> = {
                        // Split parent into segments and add remaining
                        let parent_parts: Vec<&str> = parent.split('.').collect();
                        // Use only the short name (not cog-prefixed) for inline module lookup
                        let short_parent =
                            if parent_parts.first() == Some(&"cog") && parent_parts.len() > 1 {
                                parent_parts[1..].to_vec()
                            } else {
                                parent_parts
                            };
                        let mut segs: Vec<PathSegment> = short_parent
                            .iter()
                            .map(|s| PathSegment::Name(verum_ast::ty::Ident::new(*s, span)))
                            .collect();
                        for r in &remaining {
                            segs.push(PathSegment::Name(verum_ast::ty::Ident::new(
                                r.as_str(),
                                span,
                            )));
                        }
                        segs.into()
                    };
                    let synthetic_path = verum_ast::ty::Path::new(synthetic_segments, span);
                    if let Ok(result) = self.resolve_inline_module_path(&synthetic_path, span) {
                        return Ok(result);
                    }
                }
            }
        }

        Err(TypeError::UnboundVariable {
            name: self.path_to_string(path), span,
        })
    }

    /// Resolve self path (self::item or Self::associated)
    /// FIXED: Also handles self.field.method in implement blocks where self is a variable
    pub(crate) fn resolve_self_path(&mut self, path: &Path, span: Span) -> Result<InferResult> {
        use verum_ast::ty::PathSegment;

        if self.current_self_type.is_some() && self.ctx.env.lookup("self").is_some() {
            return self.resolve_local_field_chain(verum_ast::Ident::new("self", span), path, span);
        }

        // Fallback: treat as module path (self::module::item)
        // The module resolver handles self:: resolution via resolve_path
        if let Maybe::Some(current_module) = self.current_module() {
            if path.segments.len() <= 1 {
                return Err(TypeError::Other(
                    "self requires additional path segments".into(),
                ));
            }

            // resolve_path handles self:: prefix properly
            if let Ok(resolved) = self.module_resolver.resolve_path(path, current_module)
                && let Maybe::Some(ty) = self
                    .ctx
                    .lookup_module_type(resolved.module_id, resolved.local_name.as_str())
            {
                return Ok(InferResult::new(ty.clone()));
            }
        }

        Err(TypeError::UnboundVariable {
            name: self.path_to_string(path),
            span,
        })
    }
}
