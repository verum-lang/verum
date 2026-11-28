use std::sync::Arc;
use verum_ast::cfg::{CfgEvaluator, TargetConfig};
use verum_ast::decl::{ImplItemKind, ImplKind, ProtocolItemKind};
use verum_ast::{FunctionParamKind, GenericArg, ItemKind, TypeDeclBody, TypeKind};
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
};
const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

fn parse(path: &str) -> verum_ast::Module {
    Parser::new(&std::fs::read_to_string(format!("{ROOT}/{path}")).unwrap())
        .parse_module()
        .unwrap()
}

fn duration_timeout(function: &verum_ast::FunctionDecl) {
    let timeout = function.params.iter().find_map(|parameter| match &parameter.kind {
        FunctionParamKind::Regular { pattern, ty, .. } if matches!(&pattern.kind, verum_ast::PatternKind::Ident {name,..} if name.name.as_str()=="timeout") => Some(ty), _=>None,
    }).expect("actual timeout parameter");
    let TypeKind::Generic { base, args } = &timeout.kind else {
        panic!("timeout is a Maybe")
    };
    assert!(matches!(&base.kind, TypeKind::Path(path) if path.to_string()=="Maybe"));
    assert!(
        matches!(args.as_slice(), [GenericArg::Type(ty)] if matches!(&ty.kind, TypeKind::Path(path) if path.to_string()=="EngineDuration"))
    );
}

#[test]
fn all_platform_wait_methods_match_the_real_io_engine_timeout_type() {
    let engine = parse("core/sys/io_engine.vr");
    let protocol = engine
        .items
        .iter()
        .find_map(|item| match &item.kind {
            ItemKind::Type(decl) if decl.name.name.as_str() == "IOEngine" => match &decl.body {
                TypeDeclBody::Protocol(body) => Some(&body.items),
                _ => None,
            },
            ItemKind::Protocol(decl) if decl.name.name.as_str() == "IOEngine" => Some(&decl.items),
            _ => None,
        })
        .unwrap();
    let wait = protocol
        .iter()
        .find_map(|item| match &item.kind {
            ProtocolItemKind::Function { decl, .. } if decl.name.name.as_str() == "wait" => {
                Some(decl)
            }
            _ => None,
        })
        .unwrap();
    duration_timeout(wait);
    for (path, expected) in [("core/sys/linux/io.vr", 3), ("core/sys/windows/io.vr", 2)] {
        let module = parse(path);
        let mut count = 0;
        for item in &module.items {
            let ItemKind::Impl(declaration) = &item.kind else {
                continue;
            };
            let ImplKind::Protocol { protocol, .. } = &declaration.kind else {
                continue;
            };
            if protocol.to_string() != "IOEngine" {
                continue;
            }
            for item in &declaration.items {
                if let ImplItemKind::Function(function) = &item.kind {
                    if function.name.name.as_str() == "wait" {
                        duration_timeout(function);
                        count += 1;
                    }
                }
            }
        }
        assert_eq!(count, expected, "{path}");
    }
}

#[test]
fn actual_engine_duration_construction_and_conversion_execute() {
    let mut provider = parse("core/sys/io_engine.vr");
    // A source API unit: preserve the actual constructor and conversion bodies,
    // without compiling unrelated OS operations or replacing any dependency.
    provider.items.retain_mut(|item| match &mut item.kind {
        ItemKind::Module(_) => true,
        ItemKind::Type(decl) => decl.name.name.as_str()=="EngineDuration",
        ItemKind::Impl(decl) if matches!(&decl.kind,ImplKind::Inherent(ty) if matches!(&ty.kind, TypeKind::Path(path) if path.to_string()=="EngineDuration")) => {
            decl.items.retain(|item| matches!(&item.kind,ImplItemKind::Function(function) if ["from_nanos","from_millis","as_nanos","as_millis"].contains(&function.name.name.as_str())));
            assert_eq!(decl.items.len(),4); true
        }, _=>false,
    });
    let client=Parser::new("module duration_probe; mount core.sys.io_engine.EngineDuration; fn run() -> UInt64 { let a=EngineDuration.from_millis(7_u64); let b=EngineDuration.from_nanos(1000000_u64); a.as_nanos()+b.as_millis() }").parse_module().unwrap();
    let mut generator = VbcCodegen::with_config(CodegenConfig::new("main").with_validation());
    generator
        .collect_unit_declarations(&[&provider, &client])
        .unwrap();
    generator
        .compile_unit_items(&[&provider, &client], ItemFailurePolicy::Strict)
        .unwrap();
    let module = generator.finalize_module_from_state().unwrap();
    let wire = verum_vbc::serialize::serialize_module(&module).unwrap();
    for module in [
        module,
        verum_vbc::deserialize::deserialize_module(&wire).unwrap(),
    ] {
        let function = module.find_function_by_name("duration_probe.run").unwrap();
        assert_eq!(
            Interpreter::new(Arc::new(module))
                .execute_function(function)
                .unwrap()
                .try_as_i64(),
            Some(7000001)
        );
    }
}

#[test]
fn synthetic_exit_codes_round_trip_through_the_target_status_representation() {
    let mut process = parse("core/io/process.vr");
    process.items.retain_mut(|item| match &mut item.kind {
        ItemKind::Module(module) => module.items.is_none(),
        ItemKind::Type(ty) => ty.name.as_str() == "ExitStatus",
        ItemKind::Impl(implementation) => {
            let ImplKind::Inherent(ty) = &implementation.kind else { return false };
            if !matches!(&ty.kind, TypeKind::Path(path) if path.to_string() == "ExitStatus") {
                return false;
            }
            implementation.items.retain(|item| matches!(&item.kind,
                ImplItemKind::Function(f) if matches!(f.name.as_str(),
                    "from_exit_code" | "success" | "is_exited" | "is_signaled" | "code")
            ));
            true
        }
        _ => false,
    });
    process.items.extend(Parser::new(r#"
        fn exit_code_probe(value: Int) -> Int {
            let status = ExitStatus.from_exit_code(value);
            if !status.is_exited() || status.is_signaled() { return -1; }
            if status.success() != (status.code() == 0) { return -2; }
            status.code()
        }
    "#).parse_module().unwrap().items);
    for (target, expected_large) in [
        (TargetConfig::macos_aarch64(), 120),
        (TargetConfig::linux_aarch64(), 120),
        (TargetConfig::windows_x86_64(), 0x12345678),
    ] {
        let mut generator = VbcCodegen::with_config(CodegenConfig::new("").with_target(target));
        let module = generator.compile_module(&process).unwrap();
        let wire = verum_vbc::serialize::serialize_module(&module).unwrap();
        for module in [module, verum_vbc::deserialize::deserialize_module(&wire).unwrap()] {
            let probe = module.find_function_by_name("core.io.process.exit_code_probe").unwrap();
            let mut vm = Interpreter::new(Arc::new(module));
            for (input, expected) in [(0, 0), (37, 37), (0x12345678, expected_large)] {
                let result = vm.execute_function_with_args(probe, &[verum_vbc::Value::from_i64(input)]).unwrap();
                assert_eq!(result.try_as_i64(), Some(expected));
            }
        }
    }
}

#[test]
fn actual_watcher_source_selects_exactly_one_available_backend() {
    let source = std::fs::read_to_string(format!("{ROOT}/core/sys/fs_watch.vr")).unwrap();
    let module = Parser::new(&source).parse_module().unwrap();
    for (triple, expected) in [
        ("aarch64-apple-darwin", "KqueueBackend"),
        ("x86_64-unknown-linux-gnu", "InotifyBackend"),
        ("x86_64-pc-windows-msvc", "RdcwBackend"),
    ] {
        let cfg = CfgEvaluator::for_target(triple);
        let active: Vec<_> = module
            .items
            .iter()
            .filter(|item| cfg.should_include_item(item))
            .collect();
        let aliases: Vec<_> = active
            .iter()
            .filter_map(|item| match &item.kind {
                ItemKind::Type(decl) if decl.name.name.as_str() == "FsWatcherImpl" => Some(decl),
                _ => None,
            })
            .collect();
        assert_eq!(aliases.len(), 1, "{triple}");
        let TypeDeclBody::Alias(ty) = &aliases[0].body else {
            panic!("actual selected storage must alias its backend")
        };
        let TypeKind::Path(path) = &ty.kind else {
            panic!("exact nominal alias")
        };
        assert_eq!(path.segments.len(), 1);
        assert_eq!(path.to_string(), expected, "{triple}");
        let backends: Vec<_> = active
            .iter()
            .filter_map(|item| match &item.kind {
                ItemKind::Type(decl)
                    if ["KqueueBackend", "InotifyBackend", "RdcwBackend"]
                        .contains(&decl.name.name.as_str()) =>
                {
                    Some(decl.name.name.as_str())
                }
                _ => None,
            })
            .collect();
        assert_eq!(backends, vec![expected], "{triple}");
    }
}
