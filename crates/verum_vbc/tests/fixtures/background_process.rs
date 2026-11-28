#[path = "../support/selected_stdlib.rs"]
mod selected_stdlib;

pub fn module() -> verum_vbc::VbcModule {
    use verum_ast::{FileId, ItemKind, cfg::TargetConfig};
    use verum_fast_parser::FastParser;
    use verum_vbc::codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen};
    let mut units = verum_common::List::new();
    for (index, line) in include_str!("background_process.symbols")
        .lines()
        .enumerate()
    {
        let (path, names) = line.split_once(' ').unwrap_or((line, ""));
        let mut unit = selected_stdlib::source_unit(path, names, index);
        if path == "shell/exec.vr" {
            let probes = FastParser::new()
                .parse_module_str(include_str!("background_process.vr"), FileId::new(10000))
                .unwrap();
            unit.items.extend(
                probes
                    .items
                    .into_iter()
                    .filter(|item| !matches!(item.kind, ItemKind::Module(_))),
            );
        }
        units.push(unit);
    }
    let config = CodegenConfig::new("").with_target(TargetConfig::macos_aarch64());
    let inputs: verum_common::List<_> = units.iter().collect();
    let mut compiler = VbcCodegen::with_config(config);
    compiler.initialize();
    compiler.collect_unit_declarations(&inputs).unwrap();
    compiler.resolve_pending_imports();
    compiler
        .compile_unit_items(&inputs, ItemFailurePolicy::Strict)
        .unwrap();
    let mut module = compiler.finalize_module().unwrap();
    module.resolve_protocol_dispatch();
    module
}
