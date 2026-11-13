//! Execute the actual tensor shape-product body independently of device allocation.
use verum_ast::ItemKind;
use verum_compiler::precompile::{PrecompileConfig, precompile_stdlib};
use verum_fast_parser::Parser;
use verum_vbc::{archive::read_archive_from_file, interpreter::Interpreter};

#[test]
fn tensor_shape_element_counts_execute_from_the_source_archive() {
    let source = include_str!("../../../core/math/tensor.vr");
    let ast = Parser::new(source).parse_module().unwrap();
    let item = ast.items.iter().find(|item| {
        matches!(&item.kind, ItemKind::Function(function) if function.name.as_str() == "shape_element_count")
    }).expect("shape product in the tensor source");
    let function = &source[item.span.start as usize..item.span.end as usize];
    let directory = tempfile::tempdir().unwrap();
    let core = directory.path().join("core");
    std::fs::create_dir(&core).unwrap();
    let probes = r#"
        public fn scalar() -> USize { shape_element_count(&[]) }
        public fn empty_axis() -> USize { shape_element_count(&[3 as USize, 0 as USize]) }
        public fn matrix() -> USize { shape_element_count(&[2 as USize, 3 as USize, 4 as USize]) }
        public fn empty_after_large_axes() -> USize { shape_element_count(&[~(0 as USize), 2 as USize, 0 as USize]) }
        public fn maximum() -> Bool { shape_element_count(&[~(0 as USize), 1 as USize]) == ~(0 as USize) }
        public fn overflow() -> USize { shape_element_count(&[~(0 as USize), 2 as USize]) }
    "#;
    std::fs::write(
        core.join("mod.vr"),
        format!("module core; {function}\n{probes}"),
    )
    .unwrap();
    let config = PrecompileConfig {
        stdlib_path: core,
        output_path: directory.path().join("tensor-shape.vbca"),
        target_triple: None,
        threads: None,
        verbose: false,
    };
    precompile_stdlib(&config).unwrap();
    let archive = read_archive_from_file(&config.output_path).unwrap();
    let module = archive.load_module("core").unwrap();
    for (name, expected) in [
        ("core.scalar", 1),
        ("core.empty_axis", 0),
        ("core.matrix", 24),
        ("core.empty_after_large_axes", 0),
    ] {
        let entry = module.find_function_by_name(name).unwrap();
        let result = Interpreter::new(module.clone().into())
            .execute_function(entry)
            .unwrap();
        assert_eq!(result.try_as_i64(), Some(expected), "{name}");
    }
    let maximum = module.find_function_by_name("core.maximum").unwrap();
    let result = Interpreter::new(module.clone().into())
        .execute_function(maximum)
        .unwrap();
    assert_eq!(result.try_as_bool(), Some(true));
    let overflow = module.find_function_by_name("core.overflow").unwrap();
    let error = Interpreter::new(module.into())
        .execute_function(overflow)
        .unwrap_err();
    assert!(
        error.to_string().contains("tensor shape exceeds USize"),
        "{error}"
    );
}
