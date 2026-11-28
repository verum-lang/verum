#[path = "../support/selected_stdlib.rs"]
mod selected_stdlib;

pub fn module() -> verum_vbc::VbcModule {
    use verum_ast::{FileId, cfg::TargetConfig};
    use verum_fast_parser::FastParser;
    use verum_vbc::codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen};

    let mut units = verum_common::List::new();
    for (index, line) in include_str!("joined_task.symbols").lines().enumerate() {
        let (path, symbols) = line.split_once(' ').unwrap_or((line, ""));
        let mut unit = selected_stdlib::source_unit(path, symbols, index);
        if path == "async/executor.vr" {
            let child = FastParser::new()
                .parse_module_str(include_str!("joined_task.vr"), FileId::new(100))
                .unwrap();
            unit.items.extend(child.items);
        }
        units.push(unit);
    }
    let inputs: verum_common::List<_> = units.iter().collect();
    let config = CodegenConfig::new("").with_target(TargetConfig::macos_aarch64());
    let mut compiler = VbcCodegen::with_config(config);
    compiler.initialize();
    compiler.collect_unit_declarations(&inputs).unwrap();
    compiler.resolve_pending_imports();
    compiler.compile_unit_items(&inputs, ItemFailurePolicy::Strict).unwrap();
    let mut module = compiler.finalize_module().unwrap();
    module.resolve_protocol_dispatch();
    module
}
