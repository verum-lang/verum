use super::*;

#[test]
fn release_generic_dynamic_protocol_declarations() {
    let session = crate::Session::new(crate::CompilerOptions::default());
    let (registry, phase) = release_stdlib_graph(&session);
    let module = registry.read().get_by_path("core.net.proxy.rate_limit").unwrap().ast.clone();
    check_source_module(&mut phase.configured_checker(), "core.net.proxy.rate_limit", &module, &registry)
        .unwrap_or_else(|errors| panic!("rate limiter: {errors:?}"));

    let source = "module dynamic_client; mount core.math.graph.{Graph, GraphPresheaf, FreeGraphCategory};
        fn inspect<V,L,S>(p:&dyn GraphPresheaf<V,L,S>)->Heap<dyn Graph<V,L>> {p.graph()}
        fn underlying<V,L,P:FreeGraphCategory<V,L>>(p:&P)->Heap<dyn Graph<V,L>> {p.underlying()}";
    let module = Parser::new(source).parse_module().unwrap();
    check_source_module(&mut phase.configured_checker(), "dynamic_client", &module, &registry)
        .unwrap_or_else(|errors| panic!("{source}: {errors:?}"));

    // Other declarations in this complete provider have independent domain
    // errors. Inspect the actual interfaces the release checker published.
    let owner = "core.math.infinity_topos";
    let module = registry.read().get_by_path(owner).unwrap().ast.clone();
    let mut checker = phase.configured_checker();
    let admission = check_source_module(&mut checker, owner, &module, &registry);
    let protocols = checker.protocol_checker.read();
    let topology = protocols.get_protocol_definition("core.math.infinity_topos.GrothendieckTopology")
        .unwrap_or_else(|| panic!("topology declaration missing: {admission:?}"));
    let verum_types::protocol::ProtocolConstraints::Declared { parameters, .. } = &topology.constraints else {
        panic!("missing topology telescope");
    };
    let [verum_types::context::GenericBinder::Type(category)] = parameters.as_slice() else { panic!("category binder"); };
    let method = &topology.methods[&"is_covering".into()];
    let verum_types::Type::Function { params, .. } = &method.ty else { panic!("is_covering signature"); };
    let Some(verum_types::Type::Reference { inner, .. }) = params.last() else { panic!("borrowed sieve"); };
    let verum_types::Type::DynProtocol { bounds } = inner.as_ref() else { panic!("dynamic sieve"); };
    let [sieve] = bounds.as_slice() else { panic!("single sieve application"); };
    assert_eq!(sieve.protocol, verum_types::protocol::ProtocolBound::object("core.math.infinity_topos.Sieve").protocol);
    assert_eq!(sieve.args.as_slice(), &[verum_types::Type::Var(*category)]);

    let Some(verum_types::Type::Record(fields)) = checker.resolved_type("__struct_fields_core.math.infinity_topos.Site") else {
        panic!("registered Site fields missing: {admission:?}");
    };
    let verum_types::Type::DynProtocol { bounds } = &fields[&Text::from("topology")] else { panic!("dynamic topology"); };
    let [bound] = bounds.as_slice() else { panic!("single topology application"); };
    assert_eq!(bound.protocol, verum_types::protocol::ProtocolBound::object("core.math.infinity_topos.GrothendieckTopology").protocol);
    assert_eq!(bound.args.as_slice(), std::slice::from_ref(&fields[&Text::from("underlying_category")]));
}
