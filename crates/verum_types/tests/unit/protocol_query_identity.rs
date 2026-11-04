    use super::*;

    fn nan(payload: u64) -> Type {
        Type::meta_value(
            ConstValue::Float(f64::from_bits(0x7ff8_0000_0000_0000 | payload)),
            Type::Float,
        )
    }
    fn path() -> Path {
        Path::single(Ident::new("ExactEvidence", Span::default()))
    }
    fn register(checker: &mut ProtocolChecker, ty: Type) {
        checker.set_coherence_mode(CoherenceMode::Off);
        checker
            .register_impl(ProtocolImpl {
                protocol: path(),
                protocol_args: List::new(),
                for_type: ty,
                where_clauses: List::new(),
                methods: Map::new(),
                associated_types: Map::new(),
                associated_consts: Map::new(),
                specialization: Maybe::None,
                impl_crate: Maybe::None,
                span: Span::default(),
                type_param_fn_bounds: Map::new(),
            })
            .expect("actual implementation registration");
    }

    #[test]
    fn optimistic_cache_rechecks_exact_receiver_after_routing_collision() {
        let first = nan(1);
        let second = nan(2);
        let mut checker = ProtocolChecker::new();
        register(&mut checker, first.clone());
        assert_eq!(
            checker.make_type_key(&first),
            checker.make_type_key(&second)
        );
        assert!(checker.implements_optimistic(&first, &path()));
        assert!(!checker.implements_optimistic(&second, &path()));
        assert!(checker.implements_optimistic(&first, &path()));
    }

    #[test]
    fn obligation_recursion_compares_full_query_witnesses() {
        let first = nan(1);
        let second = nan(2);
        let checker = ProtocolChecker::new();
        let bound = ProtocolBound::simple(path());
        let active = (checker.resolution_revision, first.clone(), bound.clone());
        let _first = ImplQueryGuard::enter(active.clone()).expect("first obligation");
        assert!(
            ImplQueryGuard::enter(active).is_none(),
            "actual recursion is refused"
        );
        let _second = ImplQueryGuard::enter((checker.resolution_revision, second, bound.clone()))
            .expect("different IEEE value is not recursion");
        let with_arg = ProtocolBound::positive(path(), [first.clone()].into_iter().collect());
        let _argument =
            ImplQueryGuard::enter((checker.resolution_revision, first.clone(), with_arg))
                .expect("distinct protocol application is not recursion");
        let other_checker = ProtocolChecker::new();
        let _other = ImplQueryGuard::enter((other_checker.resolution_revision, first, bound))
            .expect("different registry is not this obligation");
    }

    #[test]
    fn associated_recursion_does_not_confuse_distinct_ieee_indices() {
        let first = nan(1);
        let second = nan(2);
        let mut checker = ProtocolChecker::new();
        checker.set_coherence_mode(CoherenceMode::Off);
        let mut associated = Map::new();
        associated.insert(
            "Output".into(),
            AssociatedTypeAssignment::ordinary(Type::Int),
        );
        checker
            .register_impl(ProtocolImpl {
                protocol: path(),
                protocol_args: List::new(),
                for_type: second.clone(),
                where_clauses: List::new(),
                methods: Map::new(),
                associated_types: associated,
                associated_consts: Map::new(),
                specialization: Maybe::None,
                impl_crate: Maybe::None,
                span: Span::default(),
                type_param_fn_bounds: Map::new(),
            })
            .expect("actual associated implementation");
        let outer = (checker.resolution_revision, first, Text::from("Output"));
        ASSOC_TYPE_RESOLUTION_STACK.with(|stack| stack.borrow_mut().push(outer.clone()));
        let result = checker.try_find_associated_type(&second, &"Output".into());
        ASSOC_TYPE_RESOLUTION_STACK.with(|stack| {
            let mut stack = stack.borrow_mut();
            let index = stack
                .iter()
                .rposition(|entry| entry == &outer)
                .expect("outer query retained");
            stack.remove(index);
        });
        assert_eq!(result, Some(Type::Int));
    }
