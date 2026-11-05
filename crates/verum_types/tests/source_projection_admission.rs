//! Source projections require their exact declaring protocol and receiver bound.
use verum_ast::{ItemKind, TypeDecl};
use verum_fast_parser::Parser;
use verum_types::TypeChecker;

const ADD: &str = "type Add<Rhs = Self> is protocol { type Output; fn add(self, rhs: Rhs) -> Self.Output; };";

fn module(source: &str) -> verum_ast::Module {
    Parser::new(source).parse_module().unwrap()
}
fn register(checker: &mut TypeChecker, source: &str) -> Result<(), String> {
    for item in &module(source).items {
        match &item.kind {
            ItemKind::Type(declaration) => checker.register_type_declaration(declaration),
            _ => checker.check_item(item),
        }.map_err(|error| error.to_string())?;
    }
    let errors = checker.take_deferred_errors();
    if errors.is_empty() { Ok(()) } else { Err(format!("{errors:?}")) }
}
fn checker() -> TypeChecker {
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_current_module_path("owner");
    register(&mut checker, ADD).unwrap();
    checker
}
fn accepts(source: &str) {
    register(&mut checker(), source).unwrap();
}
fn refuses(source: &str) {
    let result = register(&mut checker(), source);
    assert!(result.is_err(), "unproved projection accepted: {source}");
}

#[test]
fn source_self_projection_is_a_valid_default_protocol_argument() {
    accepts("type Sequence is protocol { type Item; fn sum(self) -> Self.Item where Self.Item: Add<Output = Self.Item>; };");
}
#[test]
fn associated_member_order_does_not_change_source_authority() {
    accepts("type Sequence is protocol { fn sum(self) -> Self.Item where Self.Item: Add<Output = Self.Item>; type Item; };");
}
#[test]
fn two_pass_source_registration_uses_the_same_header() {
    let mut checker = checker();
    let ast = module("type Sequence is protocol { fn sum(self) -> Self.Item where Self.Item: Add; type Item; };");
    checker.register_all_type_names(&ast.items);
    for result in checker.resolve_all_type_definitions(&ast.items) {
        result.unwrap();
    }
    assert!(checker.take_deferred_errors().is_empty());
}
#[test]
fn unrelated_protocol_member_does_not_authorize_self_projection() {
    refuses("type Foreign is protocol { type Item; }; type Sequence is protocol { fn sum(self) -> Self.Item where Self.Item: Add; };");
}
#[test]
fn missing_associated_member_refuses_before_protocol_publication() {
    let mut checker = checker();
    assert!(register(&mut checker,
        "type Broken is protocol { type Item; fn sum(self) where Self.Missing: Add; };").is_err());
    assert!(checker.protocol_checker.read().get_protocol_definition("owner.Broken").is_none());
    assert!(register(&mut checker,
        "type Next is protocol { fn sum(self) where Self.Item: Add; };").is_err(),
        "failed header leaked into following declaration");
    register(&mut checker,
        "type Good is protocol { fn sum(self) where Self.Item: Add; type Item; };").unwrap();
}
#[test]
fn method_receiver_bound_authorizes_its_own_projection() {
    accepts("type Channel is protocol { type Item; }; type Sequence is protocol { fn sum<T: Channel>(&self) where T.Item: Add; };");
}
#[test]
fn competing_method_receiver_bounds_refuse() {
    refuses("type Left is protocol { type Item; }; type Right is protocol { type Item; }; type Sequence is protocol { fn sum<T: Left + Right>(&self) where T.Item: Add; };");
}
#[test]
fn unbounded_method_receiver_refuses() {
    refuses("type Foreign is protocol { type Item; }; type Sequence is protocol { fn sum<T>(&self) where T.Item: Add; };");
}
#[test]
fn nominal_receiver_without_member_refuses() {
    refuses("type Plain is {}; type Foreign is protocol { type Item; }; type Sequence is protocol { fn sum(&self) where Plain.Item: Add; };");
}
#[test]
fn explicit_projection_argument_uses_the_same_checked_path() {
    accepts("type Use<T> is protocol {}; type Sequence is protocol { type Item; fn take(&self) where Self.Item: Use<Self.Item>; };");
}

#[test]
fn context_protocol_source_route_uses_complete_header() {
    accepts("context protocol Sequence { fn sum(&self) where Self.Item: Add; type Item; }");
}

#[test]
fn inherited_associated_member_keeps_its_declaring_owner() {
    accepts("type Channel is protocol { type Item; }; type Sequence is protocol extends Channel { fn sum(self) -> Self.Item where Self.Item: Add; };");
}

fn qualified(receiver: &str, owner: &str) -> verum_ast::Type {
    let span = verum_ast::Span::dummy();
    let ident = |name| verum_ast::Ident::new(name, span);
    verum_ast::Type::new(verum_ast::TypeKind::Qualified {
        self_ty: Box::new(verum_ast::Type::new(verum_ast::TypeKind::Path(
            verum_ast::Path::single(ident(receiver))), span)),
        trait_ref: verum_ast::Path::new(owner.split('.').map(|part| verum_ast::PathSegment::Name(ident(part))).collect(), span),
        assoc_name: ident("Item"),
    }, span)
}

#[test]
fn symbolic_qualified_ast_cannot_erase_its_selected_owner() {
    let mut checker = checker();
    register(&mut checker, "type Left is protocol { type Item; }; type Right is protocol { type Item; };").unwrap();
    let mut source = declaration("type Sequence is protocol { type Item; fn test(&self, value: Self.Item); };");
    let verum_ast::TypeDeclBody::Protocol(body) = &mut source.body else { panic!("protocol") };
    let verum_ast::decl::ProtocolItemKind::Function { decl, .. } = &mut body.items[1].kind else { panic!("method") };
    let verum_ast::decl::FunctionParamKind::Regular { ty, .. } = &mut decl.params[1].kind else { panic!("regular parameter") };
    *ty = qualified("Self", "Right");
    let result = checker.register_type_declaration(&source);
    assert!(result.is_err(), "symbolic qualified owner was erased: {result:?}");
}

#[test]
fn concrete_qualified_ast_selects_its_real_implementation() {
    let mut checker = checker();
    register(&mut checker, "type Left is protocol { type Item; }; type Right is protocol { type Item; }; type Value is {};").unwrap();
    for item in &module("implement Left for Value { type Item = Int; } implement Right for Value { type Item = Text; }").items {
        let ItemKind::Impl(declaration) = &item.kind else { panic!("implementation") };
        checker.register_impl_block(declaration).unwrap();
    }
    assert_eq!(checker.ast_to_type(&qualified("Value", "owner.Left")).unwrap(), verum_types::Type::Int);
    assert_eq!(checker.ast_to_type(&qualified("Value", "owner.Right")).unwrap(), verum_types::Type::Text);
}

fn declaration(source: &str) -> TypeDecl {
    let ast = module(source);
    let ItemKind::Type(declaration) = &ast.items[0].kind else { panic!("type declaration") };
    declaration.clone()
}

#[test]
fn retained_source_sidecar_replays_complete_projection_header() {
    use std::sync::Arc;
    use verum_common::List;
    use verum_types::core_metadata::*;
    let mut metadata = CoreMetadata { version: CORE_METADATA_VERSION, ..Default::default() };
    for (name, source) in [
        ("Add", ADD),
        ("Sequence", "type Sequence is protocol { fn sum(self) -> Self.Item where Self.Item: Add<Output = Self.Item>; type Item; };"),
    ] {
        let source = declaration(source);
        let descriptor = TypeDescriptor {
            source_declaration: verum_ast::source_signature::source_type_declaration(&source).unwrap(),
            name: name.into(), module_path: "owner".into(), origin_module_path: None,
            generic_params: List::new(),
            kind: TypeDescriptorKind::Protocol { super_protocols: List::new(), associated_types: List::new(), required_methods: List::new(), default_methods: List::new() },
            size: None, alignment: None, methods: List::new(), implements: List::new(), decl_span: None,
            is_public: true, is_transparent_wrapper: false,
            resource_discipline: verum_common::ResourceDiscipline::Unrestricted,
        };
        let key: verum_common::Text = format!("owner.{name}").into();
        metadata.type_declaration_order.push(key.clone());
        metadata.types.insert(key, descriptor);
    }
    let restored = decode_metadata(&encode_metadata(&metadata).unwrap()).unwrap();
    let mut checker = TypeChecker::new_with_core_eager(Arc::new(restored));
    assert!(checker.take_deferred_errors().is_empty());
    assert!(checker.diagnostic_sources().is_empty());
    assert!(checker.protocol_checker.read().get_protocol_definition("owner.Sequence").is_some());
}

#[test]
fn recursive_protocol_bound_uses_its_exact_pending_parameter_header() {
    accepts("type Sequence is protocol { type Item; fn chain<U: Sequence<Item = Self.Item>>(&self, other: U); };");
}

#[test]
fn projection_from_recursive_receiver_bound_uses_pending_header() {
    accepts("type Sequence is protocol { type Item; fn combine<U: Sequence>(&self) where U.Item: Add; };");
}

#[test]
fn pending_and_published_bounds_cannot_choose_different_same_leaf_members() {
    refuses("type Foreign is protocol { type Item; }; type Sequence is protocol { type Item; fn combine<U: Sequence + Foreign>(&self) where U.Item: Add; };");
}

#[test]
fn nested_source_protocol_cannot_borrow_the_outer_self_bound() {
    use verum_modules::{ModuleId, ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module};
    let provider = format!("{ADD} public type Inner is protocol {{ fn invalid(&self) where Self.Item: Add; }};");
    let ast = module(&provider);
    let id = ModuleId::new(1);
    let path = ModulePath::from_str("foreign");
    let mut info = ModuleInfo::new(id, path.clone(), ast.clone(), ast.file_id, provider.into());
    info.exports = extract_exports_from_module(&ast, id, &path).unwrap();
    let mut registry = ModuleRegistry::new();
    registry.register(info);
    let mut checker = checker();
    checker.set_module_registry_direct(registry);
    register(&mut checker, "type HasItem is protocol { type Item; };").unwrap();
    let result = register(&mut checker, "type Outer is protocol { fn visit(&self) -> foreign.Inner where Self: HasItem; };");
    assert!(result.is_err(), "inner Self.Item borrowed the outer Self:HasItem: {result:?}");
    assert!(checker.protocol_checker.read().get_protocol_definition("foreign.Inner").is_none());
}
