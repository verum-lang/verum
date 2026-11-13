use verum_ast::cfg::{CfgEvaluator, TargetConfig};
use verum_ast::{FileId, ItemKind, TypeDeclBody, decl::ProtocolItemKind};
use verum_fast_parser::FastParser;

#[test]
fn protocol_source_keeps_constraints_attributes_and_method_contracts() {
    let source = r#"
        @cfg(target_os = "linux")
        public context protocol Port<T: Bound, N: meta Int> extends Base<T>
            where type T: Clone, meta N > 0 {
            type Output;
            fn read(&self, value: T) -> Self.Output using [Clock];
        }
    "#;
    let module = FastParser::new()
        .parse_module_str(source, FileId::new(7))
        .unwrap();
    let item = &module.items[0];
    let ItemKind::Type(declaration) = &item.kind else {
        panic!("protocols must enter the canonical type declaration path");
    };
    assert!(matches!(
        declaration.visibility,
        verum_ast::Visibility::Public
    ));
    assert_eq!(declaration.name.as_str(), "Port");
    assert_eq!(declaration.generics.len(), 2);
    assert_eq!(declaration.attributes.len(), 1);
    assert_eq!(
        declaration
            .meta_where_clause
            .as_ref()
            .unwrap()
            .predicates
            .len(),
        1
    );
    assert_eq!(
        source.get(declaration.name.span.start as usize..declaration.name.span.end as usize),
        Some("Port")
    );
    let TypeDeclBody::Protocol(body) = &declaration.body else {
        panic!("canonical protocol body");
    };
    assert!(body.is_context);
    assert_eq!(body.extends.len(), 1);
    assert_eq!(
        body.generic_where_clause.as_ref().unwrap().predicates.len(),
        1
    );
    assert!(matches!(body.items[0].kind, ProtocolItemKind::Type { .. }));
    let ProtocolItemKind::Function { decl: method, .. } = &body.items[1].kind else {
        panic!("declared method");
    };
    assert_eq!(method.params.len(), 2);
    assert_eq!(method.contexts.len(), 1);
    assert!(method.return_type.is_some());
    for (triple, included) in [
        ("x86_64-unknown-linux-gnu", true),
        ("aarch64-apple-darwin", false),
    ] {
        let cfg = CfgEvaluator::with_config(TargetConfig::for_target(triple));
        assert_eq!(cfg.should_include_item(item), included);
    }
}
