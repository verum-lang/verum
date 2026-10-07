//! Phase-local comparison: full owned export versus the borrowed final view.
//! The registry is source-derived; fixture construction and declaration parsing
//! are outside the timed loop. This does not measure a complete stdlib bake.
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, VbcCodegen},
    module::FunctionId,
};

fn registry(imported: usize) -> VbcCodegen {
    let source = "module current; public fn service<T,F:fn(T)->T>(value:T, other:T, callback:F)->T {callback(value)}";
    let ast = Parser::new(source)
        .parse_module()
        .expect("benchmark source");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("current"));
    codegen.compile_module(&ast).expect("source metadata");
    let seed = codegen
        .export_functions()
        .get("current.service")
        .expect("source owner")
        .clone();
    for index in 0..imported {
        let mut info = seed.clone();
        info.id = FunctionId(1000 + index as u32);
        codegen
            .ctx_mut()
            .register_function(format!("prior.module{index}.service"), info);
    }
    codegen
}

fn exports(c: &mut Criterion) {
    let mut group = c.benchmark_group("bootstrap_function_exports");
    for imported in [1024, 8192] {
        let codegen = registry(imported);
        let expected = codegen.export_functions().len();
        assert_eq!(codegen.export_function_view().len(), expected);
        group.throughput(Throughput::Elements(expected as u64));
        group.bench_with_input(
            BenchmarkId::new("owned", imported),
            &codegen,
            |b, codegen| {
                b.iter(|| black_box(codegen.export_functions()));
            },
        );
        group.bench_with_input(
            BenchmarkId::new("borrowed", imported),
            &codegen,
            |b, codegen| {
                b.iter(|| black_box(codegen.export_function_view()));
            },
        );
    }
    group.finish();
}
criterion_group!(benches, exports);
criterion_main!(benches);
