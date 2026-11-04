use verum_ast::{
    ItemKind,
    cfg::CfgEvaluator,
    decl::{ProtocolItemKind, TypeDeclBody},
};
use verum_fast_parser::Parser;

#[test]
fn protocol_methods_retain_attributes_for_all_body_forms() {
    for ending in [";", "{ 42 }", "= 42;"] {
        let source = format!(
            "type Rules is protocol {{ @deprecated(\"old method\") @cfg(target_os = \"windows\") fn read() -> Int {ending} }};"
        );
        let module = Parser::new(&source).parse_module().unwrap();
        let ItemKind::Type(declaration) = &module.items[0].kind else {
            panic!("type")
        };
        let TypeDeclBody::Protocol(body) = &declaration.body else {
            panic!("protocol")
        };
        let ProtocolItemKind::Function { decl, default_impl } = &body.items[0].kind else {
            panic!("function")
        };
        let names: Vec<_> = decl
            .attributes
            .iter()
            .map(|attr| attr.name.as_str())
            .collect();
        assert_eq!(names, ["deprecated", "cfg"]);
        assert_eq!(default_impl.is_some(), ending != ";");
        assert!(
            CfgEvaluator::for_target("x86_64-pc-windows-msvc").should_include(&decl.attributes)
        );
        assert!(!CfgEvaluator::for_target("aarch64-apple-darwin").should_include(&decl.attributes));
    }
}

#[test]
fn malformed_condition_survives_parse_after_inactive_condition() {
    let source =
        "type Rules is protocol { @cfg(target_os = \"windows\") @cfg(not()) fn read() -> Int; };";
    let module = Parser::new(source).parse_module().unwrap();
    let ItemKind::Type(declaration) = &module.items[0].kind else {
        panic!("type")
    };
    let TypeDeclBody::Protocol(body) = &declaration.body else {
        panic!("protocol")
    };
    let ProtocolItemKind::Function { decl, .. } = &body.items[0].kind else {
        panic!("method")
    };
    assert_eq!(decl.attributes.len(), 2);
    let evaluator = CfgEvaluator::for_target("aarch64-apple-darwin");
    let (_, failures) =
        evaluator.should_include_with_failures(std::slice::from_ref(&decl.attributes[1]));
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].span, decl.attributes[1].span);
}

#[test]
fn unattributed_protocol_method_stays_unattributed() {
    let module = Parser::new("type Rules is protocol { fn read() -> Int; };")
        .parse_module()
        .unwrap();
    let ItemKind::Type(declaration) = &module.items[0].kind else {
        panic!("type")
    };
    let TypeDeclBody::Protocol(body) = &declaration.body else {
        panic!("protocol")
    };
    let ProtocolItemKind::Function { decl, .. } = &body.items[0].kind else {
        panic!("function")
    };
    assert!(decl.attributes.is_empty());
}
