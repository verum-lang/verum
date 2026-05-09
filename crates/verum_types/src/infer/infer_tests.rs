#[cfg(test)]
mod qtt_v2_enforcement_tests {
    //! QTT V2 enforcement pass tests. Validates the
    //! integration: `@quantity(0|1|omega)` attribute on a parameter
    //! produces a `Quantity` declaration that drives `qtt_walker`-
    //! based usage counting + `qtt_usage::check_usage` validation.
    use super::extract_quantity_from_attrs;
    use verum_ast::Ident;
    use verum_ast::attr::{Attribute, Quantity as AstQty, QuantityAttr};
    use verum_ast::expr::{Expr, ExprKind};
    use verum_ast::span::Span;
    use verum_common::{List, Maybe, Text};

    fn span() -> Span {
        Span::default()
    }

    fn quantity_attr(q: AstQty) -> Attribute {
        let raw = QuantityAttr::new(q, span());
        // Surface form: @quantity(<glyph>) — encoded as Path arg.
        let mut segs: List<verum_ast::ty::PathSegment> = List::new();
        segs.push(verum_ast::ty::PathSegment::Name(Ident {
            name: Text::from(raw.quantity.surface_glyph()),
            span: span(),
        }));
        let path = verum_ast::ty::Path::new(segs, span());
        let mut args: List<Expr> = List::new();
        args.push(Expr::new(ExprKind::Path(path), span()));
        Attribute {
            name: Text::from("quantity"),
            args: Maybe::Some(args),
            span: span(),
        }
    }

    fn attr_list(qs: Vec<AstQty>) -> List<Attribute> {
        let mut l: List<Attribute> = List::new();
        for q in qs {
            l.push(quantity_attr(q));
        }
        l
    }

    #[test]
    fn empty_attrs_default_to_omega() {
        let attrs: List<Attribute> = List::new();
        assert_eq!(
            extract_quantity_from_attrs(&attrs),
            crate::ty::Quantity::Omega,
        );
    }

    #[test]
    fn quantity_zero_attr_extracts_zero() {
        let attrs = attr_list(vec![AstQty::Zero]);
        assert_eq!(
            extract_quantity_from_attrs(&attrs),
            crate::ty::Quantity::Zero,
        );
    }

    #[test]
    fn quantity_one_attr_extracts_linear() {
        let attrs = attr_list(vec![AstQty::One]);
        assert_eq!(
            extract_quantity_from_attrs(&attrs),
            crate::ty::Quantity::One,
        );
    }

    #[test]
    fn quantity_many_attr_extracts_omega() {
        let attrs = attr_list(vec![AstQty::Many]);
        assert_eq!(
            extract_quantity_from_attrs(&attrs),
            crate::ty::Quantity::Omega,
        );
    }

    #[test]
    fn first_quantity_attr_wins_over_extras() {
        // Multiple @quantity attributes on the same param: the
        // first one wins (deterministic ordering, no collision
        // diagnostic — the parser tolerates duplicates because
        // they're discoverable via the AST round-trip).
        let attrs = attr_list(vec![AstQty::One, AstQty::Zero]);
        assert_eq!(
            extract_quantity_from_attrs(&attrs),
            crate::ty::Quantity::One,
        );
    }

    #[test]
    fn unrelated_attr_does_not_affect_extraction() {
        let mut l: List<Attribute> = List::new();
        l.push(Attribute {
            name: Text::from("inline"),
            args: Maybe::None,
            span: span(),
        });
        l.push(quantity_attr(AstQty::One));
        assert_eq!(extract_quantity_from_attrs(&l), crate::ty::Quantity::One,);
    }
}

#[cfg(test)]
mod mount_cycle_tests {
    //! Regression: when a stdlib module's glob expansion re-enters itself via
    //! `public mount` re-exports the interpreter used to SIGBUS with ~900k
    //! `__mh_execute_header` frames. The compiler now guards every glob-
    //! expansion entry point with a `HashSet<Text>` visited-set and emits
    //! `TypeError::ImportCycle` (E0811) when the set re-enters.

    use super::TypeChecker;
    use crate::TypeError;
    use verum_ast::decl::{ModuleDecl, MountDecl, MountTree, MountTreeKind, Visibility};
    use verum_ast::span::Span;
    use verum_ast::ty::{Ident, Path, PathSegment};
    use verum_common::{List, Maybe, Text};

    fn mount_glob_decl(path_str: &str) -> MountDecl {
        let span = Span::dummy();
        let segments: List<PathSegment> = path_str
            .split('.')
            .map(|seg| PathSegment::Name(Ident::new(seg, span)))
            .collect();
        MountDecl {
            visibility: Visibility::Private,
            tree: MountTree {
                kind: MountTreeKind::Glob(Path::new(segments, span)),
                alias: Maybe::None,
                span,
            },
            alias: Maybe::None,
            span,
        }
    }

    fn make_module(name: &str) -> ModuleDecl {
        let span = Span::dummy();
        ModuleDecl {
            name: Ident::new(name, span),
            visibility: Visibility::Public,
            items: Maybe::Some(List::new()),
            profile: Maybe::None,
            features: Maybe::None,
            contexts: List::new(),
            span,
        }
    }

    /// Direct test: calling `import_all_from_inline_module` for a module
    /// whose path is already on the glob-in-progress stack must return
    /// `TypeError::ImportCycle`, not stack-overflow.
    #[test]
    fn inline_module_cycle_returns_import_cycle_error() {
        let mut checker = TypeChecker::new();
        let key: Text = "cog.loopy".into();

        // Register an empty inline module so the code path doesn't bail early
        // with "module not found".
        checker
            .inline_modules
            .insert(key.clone(), make_module("loopy"));

        // Seed the glob-in-progress set to simulate being mid-expansion of
        // this module (as would happen if the caller is one stack frame up).
        checker.glob_imports_in_progress.insert(key.clone());
        checker.glob_imports_stack.push(key.clone());

        // Recursively entering the same module must produce E0811, not SIGBUS.
        let err = checker
            .import_all_from_inline_module(key.as_str())
            .expect_err("expected ImportCycle error on re-entry");

        match err {
            TypeError::ImportCycle {
                cycle_path,
                modules_in_cycle,
                ..
            } => {
                assert!(
                    cycle_path.as_str().contains("loopy"),
                    "cycle_path should mention the looping module, got: {}",
                    cycle_path
                );
                assert!(
                    modules_in_cycle.iter().any(|m| m.as_str() == "cog.loopy"),
                    "modules_in_cycle should include cog.loopy, got: {:?}",
                    modules_in_cycle
                );
            }
            other => panic!("expected ImportCycle, got: {:?}", other),
        }
    }

    /// Direct test: `import_all_from_module` (registry-backed path) is
    /// symmetrically guarded.
    #[test]
    fn registry_module_cycle_returns_import_cycle_error() {
        let mut checker = TypeChecker::new();
        let key: Text = "core.loopy".into();

        // Simulate being mid-expansion.
        checker.glob_imports_in_progress.insert(key.clone());
        checker.glob_imports_stack.push(key.clone());

        let registry = verum_modules::ModuleRegistry::new();
        let err = checker
            .import_all_from_module(&key, &registry)
            .expect_err("expected ImportCycle error on re-entry");

        assert!(matches!(err, TypeError::ImportCycle { .. }));
    }

    /// Positive control: a fresh checker (no in-progress cycle) must NOT
    /// produce ImportCycle — the guard triggers only on actual re-entry.
    #[test]
    fn non_cyclic_inline_mount_does_not_trigger_guard() {
        let mut checker = TypeChecker::new();
        let key: Text = "cog.fine".into();
        checker
            .inline_modules
            .insert(key.clone(), make_module("fine"));

        // No seeding — this is a clean call.
        let result = checker.import_all_from_inline_module(key.as_str());
        assert!(
            result.is_ok(),
            "clean inline-module glob should not be flagged as a cycle, got {:?}",
            result
        );

        // After the call the guard must have cleaned up after itself.
        assert!(
            !checker.glob_imports_in_progress.contains(&key),
            "glob_imports_in_progress must drop key on exit"
        );
        assert!(
            checker.glob_imports_stack.is_empty(),
            "glob_imports_stack must be empty after clean exit"
        );
    }

    /// Compile-time regression: ensure the MountDecl helper builds a glob
    /// that actually lowers to MountTreeKind::Glob (guards against silent
    /// grammar drift inside the test harness).
    #[test]
    fn mount_glob_decl_helper_produces_glob_kind() {
        let decl = mount_glob_decl("core.action");
        assert!(matches!(decl.tree.kind, MountTreeKind::Glob(_)));
    }

    /// Regression: `find_type_declaration_with_source_module` used to recurse
    /// indefinitely when a module re-exported a sibling whose last segment
    /// matched the target type name, e.g.
    ///

    /// ```ignore
    /// // core/tmp_repro/mod.vr (module path "core.tmp_repro")
    /// public mount core.tmp_repro.sub;
    /// ```
    ///

    /// Looking up type `sub` in module `core.tmp_repro` would match the mount,
    /// strip the last segment back to `core.tmp_repro`, and re-enter the same
    /// AST — SIGBUSing after ~32k recursive frames in release builds.
    ///

    /// The fix threads a visited-set through
    /// `find_type_declaration_with_source_module_inner`; re-entry now returns
    /// `None` instead of blowing the stack.
    #[test]
    fn self_referential_mount_terminates_with_none() {
        use verum_ast::decl::{Item, ItemKind, MountDecl, MountTree, MountTreeKind, Visibility};
        use verum_common::FileId;

        let checker = TypeChecker::new();
        let span = Span::dummy();

        // Build MountDecl equivalent to `public mount core.tmp_repro.sub;`
        // (a Path mount, not a Glob, so it hits the
        // `find_type_declaration_with_source_module` re-export code path).
        let segments: List<PathSegment> = ["core", "tmp_repro", "sub"]
            .iter()
            .map(|seg| PathSegment::Name(Ident::new(*seg, span)))
            .collect();
        let mount_item = Item::new(
            ItemKind::Mount(MountDecl {
                visibility: Visibility::Public,
                tree: MountTree {
                    kind: MountTreeKind::Path(Path::new(segments, span)),
                    alias: Maybe::None,
                    span,
                },
                alias: Maybe::None,
                span,
            }),
            span,
        );

        let items: List<Item> = List::from(vec![mount_item]);
        let ast = verum_ast::Module::new(items, FileId::new(0), span);

        let registry = verum_modules::ModuleRegistry::new();
        // The key property: this call MUST return (rather than blow the
        // stack). The answer itself is `None` — `sub` is not actually
        // resolvable through the self-referential mount — and that is the
        // correct fallback signal for upstream callers.
        let result = checker.find_type_declaration_with_source_module(
            &ast,
            "sub",
            &Text::from("core.tmp_repro"),
            &registry,
        );
        assert!(
            result.is_none(),
            "self-referential mount should resolve to None (was: {:?})",
            result
        );
    }

    // ============================================================
    // [protocols].higher_kinded_protocols wire-up pins (task #264).
    // ============================================================

    #[test]
    fn hkt_protocols_default_is_disabled() {
        // Pin: documented Verum.toml default — HKT-bearing protocol
        // declarations are rejected unless the user explicitly opts
        // in via `[protocols].higher_kinded_protocols = true`.
        let checker = TypeChecker::new();
        assert!(
            !checker.higher_kinded_protocols_enabled(),
            "default must be false"
        );
    }

    #[test]
    fn hkt_protocols_setter_round_trips() {
        let mut checker = TypeChecker::new();
        checker.set_higher_kinded_protocols_enabled(true);
        assert!(checker.higher_kinded_protocols_enabled());
        checker.set_higher_kinded_protocols_enabled(false);
        assert!(!checker.higher_kinded_protocols_enabled());
        // Idempotent.
        checker.set_higher_kinded_protocols_enabled(false);
        assert!(!checker.higher_kinded_protocols_enabled());
    }

    #[test]
    fn hkt_protocols_disabled_rejects_higher_kinded_param() {
        // Pin: when [protocols].higher_kinded_protocols is false, a
        // protocol declaring an HKT generic parameter is rejected at
        // registration time with TypeError::Other citing the manifest.
        use verum_ast::decl::{ProtocolDecl, Visibility};
        use verum_ast::ty::{GenericParam, GenericParamKind, Ident};
        use verum_common::Maybe as VMaybe;

        let mut checker = TypeChecker::new();
        // Default false → reject.
        assert!(!checker.higher_kinded_protocols_enabled());

        let proto_decl = ProtocolDecl {
            visibility: Visibility::Internal,
            name: Ident::new("Functor", Span::default()),
            generics: verum_common::List::from(vec![GenericParam {
                kind: GenericParamKind::HigherKinded {
                    name: Ident::new("F", Span::default()),
                    arity: 1,
                    bounds: verum_common::List::new(),
                },
                is_implicit: false,
                span: Span::default(),
            }]),
            bounds: verum_common::List::new(),
            items: verum_common::List::new(),
            generic_where_clause: VMaybe::None,
            meta_where_clause: VMaybe::None,
            span: Span::default(),
            is_context: false,
        };

        let result = checker.register_protocol_decl_item(&proto_decl);
        match result {
            Err(TypeError::Other(msg)) => {
                assert!(
                    msg.as_str().contains("higher_kinded_protocols"),
                    "rejection must cite the manifest field; got: {}",
                    msg
                );
                assert!(
                    msg.as_str().contains("Functor"),
                    "rejection must name the protocol; got: {}",
                    msg
                );
                assert!(
                    msg.as_str().contains("F<"),
                    "rejection must show the HKT param syntax; got: {}",
                    msg
                );
            }
            other => panic!("expected TypeError::Other, got {:?}", other),
        }
    }

    #[test]
    fn hkt_protocols_enabled_accepts_higher_kinded_param() {
        // Pin: with [protocols].higher_kinded_protocols = true (and
        // [types].higher_kinded already implicit at the manifest
        // validation layer), HKT-bearing protocol declarations
        // register successfully.
        use verum_ast::decl::{ProtocolDecl, Visibility};
        use verum_ast::ty::{GenericParam, GenericParamKind, Ident};
        use verum_common::Maybe as VMaybe;

        let mut checker = TypeChecker::new();
        checker.set_higher_kinded_protocols_enabled(true);

        let proto_decl = ProtocolDecl {
            visibility: Visibility::Internal,
            name: Ident::new("Functor", Span::default()),
            generics: verum_common::List::from(vec![GenericParam {
                kind: GenericParamKind::HigherKinded {
                    name: Ident::new("F", Span::default()),
                    arity: 1,
                    bounds: verum_common::List::new(),
                },
                is_implicit: false,
                span: Span::default(),
            }]),
            bounds: verum_common::List::new(),
            items: verum_common::List::new(),
            generic_where_clause: VMaybe::None,
            meta_where_clause: VMaybe::None,
            span: Span::default(),
            is_context: false,
        };

        let result = checker.register_protocol_decl_item(&proto_decl);
        assert!(
            result.is_ok(),
            "with hkt protocols enabled, registration must succeed; got {:?}",
            result
        );
    }

    #[test]
    fn hkt_protocols_disabled_accepts_regular_protocol() {
        // Pin: the gate ONLY rejects HigherKinded params. Regular
        // type params (`protocol Eq<T>`) register fine even when
        // the HKT flag is false. No false positives.
        use verum_ast::decl::{ProtocolDecl, Visibility};
        use verum_ast::ty::{GenericParam, GenericParamKind, Ident};
        use verum_common::Maybe as VMaybe;

        let mut checker = TypeChecker::new();
        // Default false.
        assert!(!checker.higher_kinded_protocols_enabled());

        let proto_decl = ProtocolDecl {
            visibility: Visibility::Internal,
            name: Ident::new("Eq", Span::default()),
            generics: verum_common::List::from(vec![GenericParam {
                kind: GenericParamKind::Type {
                    name: Ident::new("T", Span::default()),
                    bounds: verum_common::List::new(),
                    default: VMaybe::None,
                },
                is_implicit: false,
                span: Span::default(),
            }]),
            bounds: verum_common::List::new(),
            items: verum_common::List::new(),
            generic_where_clause: VMaybe::None,
            meta_where_clause: VMaybe::None,
            span: Span::default(),
            is_context: false,
        };

        let result = checker.register_protocol_decl_item(&proto_decl);
        assert!(
            result.is_ok(),
            "regular type-param protocol must register even with hkt disabled; got {:?}",
            result
        );
    }

    // ============================================================
    // [protocols].generic_associated_types wire-up pins (task #265).
    // ============================================================

    #[test]
    fn gat_default_is_disabled() {
        let checker = TypeChecker::new();
        assert!(
            !checker.generic_associated_types_enabled(),
            "default must be false"
        );
    }

    #[test]
    fn gat_setter_round_trips() {
        let mut checker = TypeChecker::new();
        checker.set_generic_associated_types_enabled(true);
        assert!(checker.generic_associated_types_enabled());
        checker.set_generic_associated_types_enabled(false);
        assert!(!checker.generic_associated_types_enabled());
    }

    #[test]
    fn gat_disabled_rejects_generic_associated_type() {
        // Pin: when [protocols].generic_associated_types is false,
        // a protocol body containing a `type Item<T>` declaration
        // (non-empty type_params on the associated type) is rejected
        // at registration time with TypeError::Other citing the
        // manifest field.
        use verum_ast::decl::{ProtocolDecl, ProtocolItem, ProtocolItemKind, Visibility};
        use verum_ast::ty::{GenericParam, GenericParamKind, Ident};
        use verum_common::Maybe as VMaybe;

        let mut checker = TypeChecker::new();
        // Default false — gate active.
        assert!(!checker.generic_associated_types_enabled());

        let gat_item = ProtocolItem {
            kind: ProtocolItemKind::Type {
                name: Ident::new("Item", Span::default()),
                type_params: verum_common::List::from(vec![GenericParam {
                    kind: GenericParamKind::Type {
                        name: Ident::new("T", Span::default()),
                        bounds: verum_common::List::new(),
                        default: VMaybe::None,
                    },
                    is_implicit: false,
                    span: Span::default(),
                }]),
                bounds: verum_common::List::new(),
                where_clause: VMaybe::None,
                default_type: VMaybe::None,
            },
            span: Span::default(),
        };

        let proto_decl = ProtocolDecl {
            visibility: Visibility::Internal,
            name: Ident::new("Stream", Span::default()),
            generics: verum_common::List::new(),
            bounds: verum_common::List::new(),
            items: verum_common::List::from(vec![gat_item]),
            generic_where_clause: VMaybe::None,
            meta_where_clause: VMaybe::None,
            span: Span::default(),
            is_context: false,
        };

        let result = checker.register_protocol_decl_item(&proto_decl);
        match result {
            Err(TypeError::Other(msg)) => {
                assert!(
                    msg.as_str().contains("generic_associated_types"),
                    "rejection must cite the manifest field; got: {}",
                    msg
                );
                assert!(
                    msg.as_str().contains("Stream"),
                    "rejection must name the protocol; got: {}",
                    msg
                );
                assert!(
                    msg.as_str().contains("Item"),
                    "rejection must name the GAT; got: {}",
                    msg
                );
            }
            other => panic!("expected TypeError::Other, got {:?}", other),
        }
    }

    #[test]
    fn gat_enabled_accepts_generic_associated_type() {
        // Pin: with [protocols].generic_associated_types = true,
        // GAT-bearing protocol declarations register successfully.
        use verum_ast::decl::{ProtocolDecl, ProtocolItem, ProtocolItemKind, Visibility};
        use verum_ast::ty::{GenericParam, GenericParamKind, Ident};
        use verum_common::Maybe as VMaybe;

        let mut checker = TypeChecker::new();
        checker.set_generic_associated_types_enabled(true);

        let gat_item = ProtocolItem {
            kind: ProtocolItemKind::Type {
                name: Ident::new("Item", Span::default()),
                type_params: verum_common::List::from(vec![GenericParam {
                    kind: GenericParamKind::Type {
                        name: Ident::new("T", Span::default()),
                        bounds: verum_common::List::new(),
                        default: VMaybe::None,
                    },
                    is_implicit: false,
                    span: Span::default(),
                }]),
                bounds: verum_common::List::new(),
                where_clause: VMaybe::None,
                default_type: VMaybe::None,
            },
            span: Span::default(),
        };

        let proto_decl = ProtocolDecl {
            visibility: Visibility::Internal,
            name: Ident::new("Stream", Span::default()),
            generics: verum_common::List::new(),
            bounds: verum_common::List::new(),
            items: verum_common::List::from(vec![gat_item]),
            generic_where_clause: VMaybe::None,
            meta_where_clause: VMaybe::None,
            span: Span::default(),
            is_context: false,
        };

        let result = checker.register_protocol_decl_item(&proto_decl);
        assert!(
            result.is_ok(),
            "with GAT enabled, registration must succeed; got {:?}",
            result
        );
    }

    #[test]
    fn gat_disabled_accepts_regular_associated_type() {
        // Pin: the gate ONLY rejects associated types with non-empty
        // type_params. Regular `type Output;` (zero type_params)
        // registers fine even with the GAT flag off.
        use verum_ast::decl::{ProtocolDecl, ProtocolItem, ProtocolItemKind, Visibility};
        use verum_ast::ty::Ident;
        use verum_common::Maybe as VMaybe;

        let mut checker = TypeChecker::new();
        // Default false.
        assert!(!checker.generic_associated_types_enabled());

        let regular_item = ProtocolItem {
            kind: ProtocolItemKind::Type {
                name: Ident::new("Output", Span::default()),
                type_params: verum_common::List::new(),
                bounds: verum_common::List::new(),
                where_clause: VMaybe::None,
                default_type: VMaybe::None,
            },
            span: Span::default(),
        };

        let proto_decl = ProtocolDecl {
            visibility: Visibility::Internal,
            name: Ident::new("Iterator", Span::default()),
            generics: verum_common::List::new(),
            bounds: verum_common::List::new(),
            items: verum_common::List::from(vec![regular_item]),
            generic_where_clause: VMaybe::None,
            meta_where_clause: VMaybe::None,
            span: Span::default(),
            is_context: false,
        };

        let result = checker.register_protocol_decl_item(&proto_decl);
        assert!(
            result.is_ok(),
            "regular zero-param associated type must register even with GAT disabled; got {:?}",
            result
        );
    }

    // ============================================================
    // MLS classification sidecar pin tests (#289 Phase 2b-Foundation).
    // ============================================================

    #[test]
    fn classification_sidecar_default_is_public() {
        // Pin: looking up an unknown binding returns Public — the
        // safe default. Lattice's join() identity element so taint
        // propagation through unclassified contexts is a no-op.
        let checker = TypeChecker::new();
        let level = checker.binding_classification(&Text::from("x"));
        assert_eq!(level, verum_common::mls::MlsLevel::Public);
    }

    #[test]
    fn classification_sidecar_explicit_returns_none_for_unknown() {
        // Pin: distinguishes "not in map" from "explicitly Public"
        // for sink-detection use cases.
        let checker = TypeChecker::new();
        let level = checker.binding_classification_explicit(&Text::from("x"));
        assert!(level.is_none());
    }

    #[test]
    fn classification_sidecar_set_round_trips() {
        // Pin: setter stores the classification; getter retrieves
        // it. Foundation primitive — Phase 2b-Integration uses
        // this pair at parameter-introduction sites.
        let mut checker = TypeChecker::new();
        let var = Text::from("secret_data");
        checker.set_binding_classification(var.clone(), verum_common::mls::MlsLevel::Secret);
        assert_eq!(
            checker.binding_classification(&var),
            verum_common::mls::MlsLevel::Secret
        );
        assert_eq!(
            checker.binding_classification_explicit(&var),
            Some(verum_common::mls::MlsLevel::Secret)
        );
    }

    #[test]
    fn classification_sidecar_overwrite_uses_latest() {
        // Pin: re-setting overwrites — useful for shadowing scopes
        // where a binding is rebound at higher / lower
        // classification (Phase 2b-Full handles scoping; the
        // sidecar primitive is the underlying storage).
        let mut checker = TypeChecker::new();
        let var = Text::from("v");
        checker.set_binding_classification(var.clone(), verum_common::mls::MlsLevel::Public);
        checker.set_binding_classification(var.clone(), verum_common::mls::MlsLevel::TopSecret);
        assert_eq!(
            checker.binding_classification(&var),
            verum_common::mls::MlsLevel::TopSecret
        );
    }

    #[test]
    fn classification_sidecar_drain_clears_map() {
        // Pin: drain returns the full map and empties the
        // checker's storage. Used by audit reports + scope-exit
        // cleanup.
        let mut checker = TypeChecker::new();
        checker.set_binding_classification(Text::from("a"), verum_common::mls::MlsLevel::Secret);
        checker.set_binding_classification(Text::from("b"), verum_common::mls::MlsLevel::TopSecret);
        let drained = checker.drain_classification_map();
        assert_eq!(drained.len(), 2);
        // After drain, lookups return Public again.
        assert_eq!(
            checker.binding_classification(&Text::from("a")),
            verum_common::mls::MlsLevel::Public
        );
        assert!(
            checker
                .binding_classification_explicit(&Text::from("b"))
                .is_none()
        );
    }

    #[test]
    fn classification_sidecar_uses_lattice_join_when_combining() {
        // Pin: callers use the lattice's `join` to combine
        // classifications across multiple sources — this test
        // verifies the sidecar interoperates with the lattice
        // primitive from #282 Phase 2a.
        let mut checker = TypeChecker::new();
        checker
            .set_binding_classification(Text::from("source"), verum_common::mls::MlsLevel::Secret);
        let other = verum_common::mls::MlsLevel::TopSecret;
        let combined = checker
            .binding_classification(&Text::from("source"))
            .join(other);
        assert_eq!(combined, verum_common::mls::MlsLevel::TopSecret);
    }

    // ============================================================
    // MLS Phase 2b-Integration pin tests (#291) — sidecar seeding
    // from parameter @classification attributes at function-
    // signature registration time.
    // ============================================================

    /// Build a `@classification(<level>)` attribute for tests.
    fn mk_classification_attr_2b(level: &str) -> verum_ast::attr::Attribute {
        use verum_ast::expr::{Expr, ExprKind};
        let path = verum_ast::ty::Path::single(verum_ast::ty::Ident::new(level, Span::default()));
        let arg = Expr::new(ExprKind::Path(path), Span::default());
        let mut args = List::new();
        args.push(arg);
        verum_ast::attr::Attribute::new(
            Text::from("classification"),
            Maybe::Some(args),
            Span::default(),
        )
    }

    /// Build a Regular FunctionParam with a single Ident pattern
    /// and an optional `@classification` attribute.
    fn mk_param(name: &str, classification: Option<&str>) -> verum_ast::decl::FunctionParam {
        use verum_ast::decl::FunctionParamKind;
        use verum_ast::pattern::{Pattern, PatternKind};
        let mut attrs = List::new();
        if let Some(level) = classification {
            attrs.push(mk_classification_attr_2b(level));
        }
        verum_ast::decl::FunctionParam {
            kind: FunctionParamKind::Regular {
                pattern: Pattern {
                    kind: PatternKind::Ident {
                        by_ref: false,
                        mutable: false,
                        name: verum_ast::ty::Ident::new(name, Span::default()),
                        subpattern: Maybe::None,
                    },
                    span: Span::default(),
                },
                ty: verum_ast::ty::Type {
                    kind: verum_ast::ty::TypeKind::Path(verum_ast::ty::Path::single(
                        verum_ast::ty::Ident::new("Int", Span::default()),
                    )),
                    span: Span::default(),
                },
                default_value: Maybe::None,
            },
            attributes: attrs,
            span: Span::default(),
        }
    }

    /// Build a FunctionDecl with the given parameters for sidecar
    /// seeding tests.
    fn mk_function_decl_2b(
        params: List<verum_ast::decl::FunctionParam>,
    ) -> verum_ast::FunctionDecl {
        verum_ast::FunctionDecl {
            visibility: Default::default(),
            name: verum_ast::ty::Ident::new("test_fn", Span::default()),
            generics: List::new(),
            params,
            return_type: Maybe::None,
            throws_clause: Maybe::None,
            body: None,
            attributes: List::new(),
            is_async: false,
            is_meta: false,
            is_unsafe: false,
            span: Span::default(),
            generic_where_clause: Maybe::None,
            meta_where_clause: Maybe::None,
            requires: List::new(),
            ensures: List::new(),
            stage_level: 0,
            is_pure: false,
            is_generator: false,
            is_cofix: false,
            is_transparent: false,
            extern_abi: Maybe::None,
            is_variadic: false,
            std_attr: Maybe::None,
            contexts: List::new(),
        }
    }

    #[test]
    fn read_param_classification_returns_public_for_no_attr() {
        // Pin: helper returns Public when no @classification is
        // present — matches the safe-default semantic.
        let attrs: List<verum_ast::attr::Attribute> = List::new();
        let level = super::read_param_classification(&attrs);
        assert_eq!(level, verum_common::mls::MlsLevel::Public);
    }

    #[test]
    fn read_param_classification_extracts_secret() {
        let mut attrs = List::new();
        attrs.push(mk_classification_attr_2b("secret"));
        let level = super::read_param_classification(&attrs);
        assert_eq!(level, verum_common::mls::MlsLevel::Secret);
    }

    #[test]
    fn read_param_classification_takes_max_when_multiple() {
        // Pin: multiple @classification attributes take the highest
        // (lattice join). Pathological but legal AST.
        let mut attrs = List::new();
        attrs.push(mk_classification_attr_2b("secret"));
        attrs.push(mk_classification_attr_2b("top_secret"));
        let level = super::read_param_classification(&attrs);
        assert_eq!(level, verum_common::mls::MlsLevel::TopSecret);
    }

    #[test]
    fn register_function_signature_seeds_sidecar_for_classified_param() {
        // Pin: after register_function_signature, the sidecar
        // contains an entry for each Regular Ident parameter whose
        // attributes carry a non-Public classification.
        let mut params = List::new();
        params.push(mk_param("data", Some("secret")));
        let func = mk_function_decl_2b(params);

        let mut checker = TypeChecker::new();
        let _ = checker.register_function_signature(&func);

        assert_eq!(
            checker.binding_classification(&Text::from("data")),
            verum_common::mls::MlsLevel::Secret,
            "register_function_signature must seed sidecar for classified params"
        );
    }

    #[test]
    fn register_function_signature_does_not_seed_unclassified_params() {
        // Pin: parameters without @classification do NOT seed the
        // sidecar — keeps the map sparse (only classified
        // bindings are tracked).
        let mut params = List::new();
        params.push(mk_param("plain", None));
        let func = mk_function_decl_2b(params);

        let mut checker = TypeChecker::new();
        let _ = checker.register_function_signature(&func);

        // Unclassified binding returns Public via the default path
        // but should NOT have an explicit entry.
        assert!(
            checker
                .binding_classification_explicit(&Text::from("plain"))
                .is_none(),
            "unclassified params must not produce a sidecar entry"
        );
    }

    #[test]
    fn register_function_signature_seeds_multiple_classified_params() {
        // Pin: every classified parameter gets its own sidecar
        // entry. Multi-parameter functions track each binding
        // independently.
        let mut params = List::new();
        params.push(mk_param("low", None));
        params.push(mk_param("med", Some("secret")));
        params.push(mk_param("high", Some("top_secret")));
        let func = mk_function_decl_2b(params);

        let mut checker = TypeChecker::new();
        let _ = checker.register_function_signature(&func);

        assert!(
            checker
                .binding_classification_explicit(&Text::from("low"))
                .is_none()
        );
        assert_eq!(
            checker.binding_classification(&Text::from("med")),
            verum_common::mls::MlsLevel::Secret
        );
        assert_eq!(
            checker.binding_classification(&Text::from("high")),
            verum_common::mls::MlsLevel::TopSecret
        );
    }

    // ============================================================
    // MLS Phase 2b-Followup pin tests (#292) — expression
    // classification + let-binding propagation.
    // ============================================================

    fn mk_path_expr(name: &str) -> verum_ast::expr::Expr {
        use verum_ast::expr::{Expr, ExprKind};
        let path = verum_ast::ty::Path::single(verum_ast::ty::Ident::new(name, Span::default()));
        Expr::new(ExprKind::Path(path), Span::default())
    }

    fn mk_int_lit(n: i64) -> verum_ast::expr::Expr {
        use verum_ast::expr::{Expr, ExprKind};
        use verum_ast::literal::{IntLit, Literal, LiteralKind};
        Expr::new(
            ExprKind::Literal(Literal::new(
                LiteralKind::Int(IntLit {
                    value: n as i128,
                    suffix: Maybe::None,
                }),
                Span::default(),
            )),
            Span::default(),
        )
    }

    #[test]
    fn expr_classification_path_resolves_classified_binding() {
        // Pin: a Path expression referring to a classified binding
        // returns that binding's classification — the load-bearing
        // read site for let-binding propagation.
        let mut checker = TypeChecker::new();
        checker.set_binding_classification(
            Text::from("secret_data"),
            verum_common::mls::MlsLevel::Secret,
        );
        let expr = mk_path_expr("secret_data");
        assert_eq!(
            checker.expr_classification(&expr),
            verum_common::mls::MlsLevel::Secret
        );
    }

    #[test]
    fn expr_classification_path_unknown_returns_public() {
        // Pin: unknown Path expressions return Public (sparse-by-
        // design). No false positives from typos.
        let checker = TypeChecker::new();
        let expr = mk_path_expr("nonexistent");
        assert_eq!(
            checker.expr_classification(&expr),
            verum_common::mls::MlsLevel::Public
        );
    }

    #[test]
    fn expr_classification_literal_returns_public() {
        // Pin: literal expressions are unclassified. Constants are
        // not derived from any classified source.
        let checker = TypeChecker::new();
        let expr = mk_int_lit(42);
        assert_eq!(
            checker.expr_classification(&expr),
            verum_common::mls::MlsLevel::Public
        );
    }

    #[test]
    fn expr_classification_binary_joins_operand_classifications() {
        // Pin: `a + b` where a is Secret and b is Public produces
        // Secret. Lattice JOIN semantics — both operands taint the
        // result.
        use verum_ast::expr::{BinOp, Expr, ExprKind};
        let mut checker = TypeChecker::new();
        checker.set_binding_classification(Text::from("a"), verum_common::mls::MlsLevel::Secret);
        let left = mk_path_expr("a");
        let right = mk_int_lit(5);
        let binop = Expr::new(
            ExprKind::Binary {
                op: BinOp::Add,
                left: verum_common::Heap::new(left),
                right: verum_common::Heap::new(right),
            },
            Span::default(),
        );
        assert_eq!(
            checker.expr_classification(&binop),
            verum_common::mls::MlsLevel::Secret
        );
    }

    #[test]
    fn expr_classification_binary_max_when_both_classified() {
        // Pin: when both operands are classified at different
        // levels, the lattice JOIN produces the maximum.
        use verum_ast::expr::{BinOp, Expr, ExprKind};
        let mut checker = TypeChecker::new();
        checker.set_binding_classification(
            Text::from("secret_v"),
            verum_common::mls::MlsLevel::Secret,
        );
        checker
            .set_binding_classification(Text::from("ts_v"), verum_common::mls::MlsLevel::TopSecret);
        let left = mk_path_expr("secret_v");
        let right = mk_path_expr("ts_v");
        let binop = Expr::new(
            ExprKind::Binary {
                op: BinOp::Mul,
                left: verum_common::Heap::new(left),
                right: verum_common::Heap::new(right),
            },
            Span::default(),
        );
        assert_eq!(
            checker.expr_classification(&binop),
            verum_common::mls::MlsLevel::TopSecret
        );
    }

    // ============================================================
    // MLS Phase 2b-Final pin tests (#293) — call-site down-flow
    // helper + parameter classification metadata registration.
    // ============================================================

    #[test]
    fn register_function_signature_stores_param_classifications() {
        // Pin: parameter classification metadata is stored at
        // signature-registration time so call sites can look it up
        // by function name. Sparse map: every function gets an
        // entry (even if all-Public) so the lookup contract is
        // uniform.
        let mut params = List::new();
        params.push(mk_param("low", None));
        params.push(mk_param("med", Some("secret")));
        params.push(mk_param("high", Some("top_secret")));
        let func = mk_function_decl_2b(params);
        let mut checker = TypeChecker::new();
        let _ = checker.register_function_signature(&func);

        let levels = checker
            .function_param_classifications(&Text::from("test_fn"))
            .expect("registration must populate param classifications");
        assert_eq!(levels.len(), 3);
        assert_eq!(levels[0], verum_common::mls::MlsLevel::Public);
        assert_eq!(levels[1], verum_common::mls::MlsLevel::Secret);
        assert_eq!(levels[2], verum_common::mls::MlsLevel::TopSecret);
    }

    #[test]
    fn function_param_classifications_returns_none_for_unknown() {
        let checker = TypeChecker::new();
        assert!(
            checker
                .function_param_classifications(&Text::from("never_registered"))
                .is_none()
        );
    }

    #[test]
    fn check_classification_downflow_accepts_higher_param() {
        // Pin: lattice subsumption — Public arg flowing into
        // Secret param is ACCEPTED (param provides MORE protection
        // than the unclassified data requires).
        let checker = TypeChecker::new();
        let result = checker.check_classification_downflow(
            verum_common::mls::MlsLevel::Public,
            verum_common::mls::MlsLevel::Secret,
            "foo",
            0,
            "x",
        );
        assert!(
            result.is_ok(),
            "arg=Public into param=Secret must accept (over-protection)"
        );
    }

    #[test]
    fn check_classification_downflow_accepts_equal() {
        let checker = TypeChecker::new();
        for level in [
            verum_common::mls::MlsLevel::Public,
            verum_common::mls::MlsLevel::Secret,
            verum_common::mls::MlsLevel::TopSecret,
        ] {
            assert!(
                checker
                    .check_classification_downflow(level, level, "f", 0, "p")
                    .is_ok()
            );
        }
    }

    #[test]
    fn check_classification_downflow_rejects_secret_to_public() {
        // Pin: the load-bearing reject — Secret arg into Public
        // param is the leak we're catching.
        let checker = TypeChecker::new();
        let result = checker.check_classification_downflow(
            verum_common::mls::MlsLevel::Secret,
            verum_common::mls::MlsLevel::Public,
            "log_visible",
            0,
            "msg",
        );
        match result {
            Err(TypeError::Other(msg)) => {
                let s = msg.as_str();
                assert!(s.contains("MLS down-flow"), "got: {}", s);
                assert!(s.contains("secret"), "got: {}", s);
                assert!(s.contains("public"), "got: {}", s);
                assert!(s.contains("log_visible"), "got: {}", s);
                assert!(s.contains("@declassify"), "got: {}", s);
            }
            other => panic!("expected TypeError::Other, got {:?}", other),
        }
    }

    #[test]
    fn check_classification_downflow_rejects_top_secret_to_secret() {
        // Pin: TopSecret arg into Secret param is rejected — the
        // param provides only Secret-level protection, but the
        // argument requires TopSecret-level protection. Without
        // this rejection, downstream operations on the param
        // would handle TopSecret data under Secret-grade rules.
        let checker = TypeChecker::new();
        let result = checker.check_classification_downflow(
            verum_common::mls::MlsLevel::TopSecret,
            verum_common::mls::MlsLevel::Secret,
            "f",
            1,
            "data",
        );
        assert!(
            result.is_err(),
            "TopSecret arg into Secret param must reject (under-protection)"
        );
    }

    // ============================================================
    // MLS Phase 2b-Final-Integration pin tests (#294) — module
    // walker that calls check_classification_downflow at every
    // detected call site.
    // ============================================================

    /// Build a Module with a single function whose body is a
    /// statement-expression call.
    fn mk_module_with_call(
        callee_name: &str,
        callee_param: (&str, Option<&str>),
        caller_arg_path: &str,
        caller_classified_locals: Vec<(&str, &str)>,
    ) -> verum_ast::Module {
        use verum_ast::expr::{Expr, ExprKind};

        // The callee declaration:
        let mut callee_params = List::new();
        callee_params.push(mk_param(callee_param.0, callee_param.1));
        let callee = {
            let mut decl = mk_function_decl_2b(callee_params);
            decl.name = verum_ast::ty::Ident::new(callee_name, Span::default());
            decl
        };

        // The caller body: just one call expression `callee(arg)`.
        let func_path =
            verum_ast::ty::Path::single(verum_ast::ty::Ident::new(callee_name, Span::default()));
        let func_expr = Expr::new(ExprKind::Path(func_path), Span::default());
        let arg_path = verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
            caller_arg_path,
            Span::default(),
        ));
        let arg_expr = Expr::new(ExprKind::Path(arg_path), Span::default());
        let mut args = List::new();
        args.push(arg_expr);
        let call_expr = Expr::new(
            ExprKind::Call {
                func: verum_common::Heap::new(func_expr),
                args,
                type_args: List::new(),
            },
            Span::default(),
        );
        let call_stmt = verum_ast::stmt::Stmt {
            kind: verum_ast::stmt::StmtKind::Expr {
                expr: call_expr,
                has_semi: false,
            },
            attributes: Vec::new(),
            span: Span::default(),
        };
        let mut stmts = List::new();
        stmts.push(call_stmt);
        let body = verum_ast::expr::Block {
            stmts,
            expr: Maybe::None,
            span: Span::default(),
        };

        // The caller declaration with classified locals as
        // parameters.
        let mut caller_params = List::new();
        for (name, level) in caller_classified_locals {
            caller_params.push(mk_param(name, Some(level)));
        }
        let caller = {
            let mut decl = mk_function_decl_2b(caller_params);
            decl.name = verum_ast::ty::Ident::new("caller", Span::default());
            decl.body = Some(verum_ast::decl::FunctionBody::Block(body));
            decl
        };

        let mut items = List::new();
        items.push(verum_ast::decl::Item::new(
            verum_ast::ItemKind::Function(callee),
            Span::default(),
        ));
        items.push(verum_ast::decl::Item::new(
            verum_ast::ItemKind::Function(caller),
            Span::default(),
        ));
        verum_ast::Module {
            items,
            attributes: List::new(),
            file_id: verum_ast::FileId::new(0),
            span: Span::default(),
        }
    }

    #[test]
    fn module_walker_detects_secret_to_public_call_site_leak() {
        // Pin: caller passes a Secret-classified local to a
        // function whose parameter is unclassified (Public). The
        // walker emits one TypeError::Other diagnostic per leak.
        let module = mk_module_with_call(
            "log_visible", // callee
            ("msg", None), // callee param: unclassified
            "secret_data", // caller arg: a name in caller's params
            vec![("secret_data", "secret")],
        );

        let mut checker = TypeChecker::new();
        // Register both functions so their param classifications
        // are visible to the walker.
        for item in &module.items {
            if let verum_ast::ItemKind::Function(func) = &item.kind {
                let _ = checker.register_function_signature(func);
            }
        }

        let errors = checker.check_module_call_classifications(&module);
        assert_eq!(
            errors.len(),
            1,
            "secret arg → public param must produce one error"
        );
        match &errors[0] {
            TypeError::Other(msg) => {
                let s = msg.as_str();
                assert!(s.contains("MLS down-flow"), "got: {}", s);
                assert!(s.contains("log_visible"), "got: {}", s);
                assert!(s.contains("secret"), "got: {}", s);
            }
            other => panic!("expected TypeError::Other, got {:?}", other),
        }
    }

    #[test]
    fn module_walker_accepts_classified_param_chain() {
        // Pin: when caller's classified local flows into a
        // matching-classification parameter, no leak.
        let module = mk_module_with_call(
            "encrypt",
            ("data", Some("secret")), // callee param: secret
            "secret_data",
            vec![("secret_data", "secret")],
        );

        let mut checker = TypeChecker::new();
        for item in &module.items {
            if let verum_ast::ItemKind::Function(func) = &item.kind {
                let _ = checker.register_function_signature(func);
            }
        }

        let errors = checker.check_module_call_classifications(&module);
        assert!(
            errors.is_empty(),
            "secret arg → secret param must accept; got {} errors",
            errors.len()
        );
    }

    #[test]
    fn module_walker_accepts_unclassified_program() {
        // Pin: a program with no classifications anywhere
        // produces zero diagnostics. Phase 2b is dormant in
        // public-floor builds — zero overhead.
        let module = mk_module_with_call("plain_fn", ("arg", None), "x", vec![("x", "public")]);

        let mut checker = TypeChecker::new();
        for item in &module.items {
            if let verum_ast::ItemKind::Function(func) = &item.kind {
                let _ = checker.register_function_signature(func);
            }
        }

        let errors = checker.check_module_call_classifications(&module);
        assert!(
            errors.is_empty(),
            "fully-public program must produce no diagnostics"
        );
    }

    #[test]
    fn module_walker_accepts_over_protection() {
        // Pin: passing a public arg to a secret-classified param
        // is fine — parameter provides MORE protection than the
        // unclassified data requires.
        let module = mk_module_with_call(
            "encrypt",
            ("data", Some("secret")),
            "x",
            vec![("x", "public")],
        );

        let mut checker = TypeChecker::new();
        for item in &module.items {
            if let verum_ast::ItemKind::Function(func) = &item.kind {
                let _ = checker.register_function_signature(func);
            }
        }

        let errors = checker.check_module_call_classifications(&module);
        assert!(
            errors.is_empty(),
            "public arg → secret param (over-protection) must accept"
        );
    }

    // ============================================================
    // MLS Phase 2b @declassify escape hatch pin tests (#295).
    //

    // When a function carries `@declassify`, its body is the
    // boundary where classified data is explicitly allowed to
    // flow into lower-classification sinks. The walker skips
    // such functions entirely.
    // ============================================================

    /// Build a `@declassify` attribute (no args needed).
    fn mk_declassify_attr_simple() -> verum_ast::attr::Attribute {
        verum_ast::attr::Attribute::simple(verum_common::Text::from("declassify"), Span::default())
    }

    /// Build a Module with a `@declassify`-marked caller passing a
    /// classified arg into a public param.
    fn mk_module_with_declassify_caller(caller_has_declassify: bool) -> verum_ast::Module {
        let mut module = mk_module_with_call(
            "log_visible", // unclassified callee
            ("msg", None),
            "secret_data",
            vec![("secret_data", "secret")],
        );
        if caller_has_declassify {
            // The second item is the caller — promote its
            // attributes to include @declassify.
            if let verum_ast::ItemKind::Function(ref mut f) = module.items[1].kind {
                f.attributes.push(mk_declassify_attr_simple());
            }
        }
        module
    }

    #[test]
    fn declassify_caller_skips_walker() {
        // Pin: when the caller carries @declassify, the walker
        // skips its body entirely — no leak diagnostic even though
        // a Secret arg flows into a Public param.
        let module = mk_module_with_declassify_caller(true);
        let mut checker = TypeChecker::new();
        for item in &module.items {
            if let verum_ast::ItemKind::Function(func) = &item.kind {
                let _ = checker.register_function_signature(func);
            }
        }
        let errors = checker.check_module_call_classifications(&module);
        assert!(
            errors.is_empty(),
            "@declassify caller must skip down-flow walker; got {} errors",
            errors.len()
        );
    }

    #[test]
    fn no_declassify_still_fires_walker() {
        // Pin: same module WITHOUT @declassify still fires the
        // leak diagnostic — regression-control for the escape
        // hatch (it's not silently always-on).
        let module = mk_module_with_declassify_caller(false);
        let mut checker = TypeChecker::new();
        for item in &module.items {
            if let verum_ast::ItemKind::Function(func) = &item.kind {
                let _ = checker.register_function_signature(func);
            }
        }
        let errors = checker.check_module_call_classifications(&module);
        assert_eq!(errors.len(), 1, "without @declassify, the leak still fires");
    }

    #[test]
    fn has_declassify_attr_on_function_returns_true_with_attr() {
        let mut params = List::new();
        params.push(mk_param("x", None));
        let mut func = mk_function_decl_2b(params);
        func.attributes.push(mk_declassify_attr_simple());
        assert!(super::has_declassify_attr_on_function(&func));
    }

    #[test]
    fn has_declassify_attr_on_function_returns_false_without_attr() {
        let func = mk_function_decl_2b(List::new());
        assert!(!super::has_declassify_attr_on_function(&func));
    }

    #[test]
    fn has_declassify_ignores_other_attrs() {
        // Pin: only @declassify produces true. Sibling attributes
        // (@inline, @classification, etc.) don't accidentally
        // trip the escape hatch.
        let mut params = List::new();
        params.push(mk_param("x", None));
        let mut func = mk_function_decl_2b(params);
        func.attributes.push(mk_classification_attr_2b("secret"));
        // @classification but no @declassify → walker still fires.
        assert!(!super::has_declassify_attr_on_function(&func));
    }

    #[test]
    fn module_walker_detects_top_secret_to_secret_underflow() {
        // Pin: TopSecret arg into Secret param is rejected — the
        // parameter provides only Secret-grade protection.
        let module = mk_module_with_call(
            "secret_only_handler",
            ("data", Some("secret")),
            "ts_data",
            vec![("ts_data", "top_secret")],
        );

        let mut checker = TypeChecker::new();
        for item in &module.items {
            if let verum_ast::ItemKind::Function(func) = &item.kind {
                let _ = checker.register_function_signature(func);
            }
        }

        let errors = checker.check_module_call_classifications(&module);
        assert_eq!(
            errors.len(),
            1,
            "top_secret → secret must reject (under-protection)"
        );
    }

    #[test]
    fn check_classification_downflow_accepts_public_to_secret() {
        // Pin: Public arg flowing into Secret param is ACCEPTED.
        // The parameter provides MORE protection than the
        // unclassified argument requires — over-protection is
        // fine, only under-protection is a leak.
        let checker = TypeChecker::new();
        let result = checker.check_classification_downflow(
            verum_common::mls::MlsLevel::Public,
            verum_common::mls::MlsLevel::Secret,
            "f",
            0,
            "p",
        );
        assert!(
            result.is_ok(),
            "Public arg into Secret param must accept (over-protection)"
        );
    }

    #[test]
    fn expr_classification_call_propagates_through_args() {
        // Pin: function calls propagate classification from
        // arguments to result. `foo(secret_arg)` taints the result
        // at Secret. The function's own classification is the
        // join with arg classifications — Phase 2b-Final will
        // refine this with parameter-classification matching.
        use verum_ast::expr::{Expr, ExprKind};
        let mut checker = TypeChecker::new();
        checker.set_binding_classification(
            Text::from("secret_arg"),
            verum_common::mls::MlsLevel::Secret,
        );
        let func = mk_path_expr("foo");
        let mut args = List::new();
        args.push(mk_path_expr("secret_arg"));
        let call = Expr::new(
            ExprKind::Call {
                func: verum_common::Heap::new(func),
                args,
                type_args: List::new(),
            },
            Span::default(),
        );
        assert_eq!(
            checker.expr_classification(&call),
            verum_common::mls::MlsLevel::Secret
        );
    }
}
