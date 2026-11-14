//! Full canonical declarations select reference methods and their exact returns.
use super::*;
use verum_common::ToText;

#[test]
fn actual_configuration_and_cbgr_method_results_preserve_their_declarations() {
    let session = crate::Session::new(crate::options::CompilerOptions::default());
    let (registry, phase) = release_stdlib_graph(&session);
    for (body, accepted, signature) in [
        ("fn probe(value:&dyn ConfigFormat)->FormatCapabilities {value.capabilities()}", true, None),
        ("fn probe(value:&dyn ConfigFormat)->Int {value.capabilities()}", false, None),
        ("fn probe(value:&ThinRef<Int>)->UInt16 {value.capabilities()}", true, Some("UInt16")),
        ("fn probe(value:&ThinRef<Int>)->Bool {value.capabilities()}", false, None),
        ("fn probe(value:&ThinRef<Int>)->UInt32 {value.generation()}", true, Some("UInt32")),
        ("fn probe(value:&FatRef<Int>)->UInt16 {value.epoch()}", true, Some("UInt16")),
        ("fn probe(value:&Heap<Int>)->UInt16 {value.capabilities()}", true, Some("UInt16")),
        ("fn probe(value:&Heap<Int>)->UInt32 {value.generation()}", true, Some("UInt32")),
    ] {
        let source = format!("module reference_probe; mount core.configuration.format.{{ConfigFormat,FormatCapabilities}}; mount core.mem.thin_ref.ThinRef; mount core.mem.fat_ref.FatRef; mount core.base.memory.Heap; {body}");
        let module = Parser::new(&source).parse_module().unwrap();
        let mut checker = phase.configured_checker();
        let result = check_source_module(&mut checker, "reference_probe", &module, &registry);
        if accepted {
            result.unwrap_or_else(|errors| panic!("{body}: {errors:?}"));
            let Some(expected) = signature else { continue; };
            let call = module.items.iter().find_map(|item| match &item.kind {
                verum_ast::ItemKind::Function(function) => match function.body.as_ref()? {
                    verum_ast::FunctionBody::Block(block) => block.expr.as_deref(),
                    verum_ast::FunctionBody::Expr(expression) => Some(expression),
                },
                _ => None,
            }).expect("probe tail call");
            let types = checker.take_type_registry();
            let selected = types.lookup_call_target(call.span).unwrap_or_else(|| panic!("{body}: missing selected signature at {:?}", call.span));
            let verum_types::Type::Function { return_type, .. } = selected else { panic!("{selected:?}"); };
            assert_eq!(return_type.to_text().as_str(), expected, "{body}: {selected:?}");
        } else {
            let errors = result.expect_err(body);
            assert!(errors.iter().any(|e| e.code() == Some("E400")), "{body}: {errors:?}");
        }
    }
}
