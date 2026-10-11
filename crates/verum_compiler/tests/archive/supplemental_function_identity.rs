//! T1744: one archive-local function keeps one consumer ID per compilation.
use super::{ArchiveCtxCache, SymbolGraph};
use verum_common::{List, Shared, Text};
use verum_fast_parser::Parser;
use verum_vbc::archive::{ArchiveBuilder, VbcArchive, read_archive, write_archive};
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::instruction::Instruction;
use verum_vbc::module::{FunctionDescriptor, FunctionId, VbcModule};

fn source(owner: &str, result: i64) -> VbcModule {
    let ast = Parser::new(&format!(
        "module {owner}; public type Counter is {{ value: Int }}; \
         implement Counter {{ public fn value(&self) -> Int {{ self.value }} \
         public fn empty(&self) -> Unit {{}} }} \
         public fn original() -> Int {{ {result} }} \
         public fn later() -> Int {{ original() + 5 }} \
         public fn latest() -> Int {{ later() + 1 }}"
    ))
    .parse_module()
    .expect("parse archive producer");
    VbcCodegen::with_config(CodegenConfig::new(owner))
        .compile_module(&ast)
        .expect("compile archive producer")
}

fn archive(modules: &[VbcModule]) -> VbcArchive {
    let mut builder = ArchiveBuilder::new();
    for module in modules {
        builder.add_module(&module.name, module, &[]).unwrap();
    }
    let mut bytes = List::new();
    write_archive(&builder.finish(), &mut bytes).expect("encode archive");
    read_archive(std::io::Cursor::new(bytes.as_slice())).expect("decode archive")
}

fn cache(archive: &VbcArchive) -> ArchiveCtxCache {
    let cache = ArchiveCtxCache::new();
    // This synthetic archive owns its graph; the process's embedded stdlib
    // graph is unrelated and must not participate in these controls.
    assert!(cache.graph.set(SymbolGraph::build(archive)).is_ok());
    cache
}

fn primary(cache: &ArchiveCtxCache, archive: &VbcArchive, body: &str, offset: u32) -> VbcCodegen {
    let ast = Parser::new(body)
        .parse_module()
        .expect("parse archive consumer");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    *codegen.next_func_id_mut() += offset;
    let (modules, _) = cache.apply_lazy_with_types(archive, &mut codegen, &ast, &[]);
    assert!(modules > 0, "primary must import the real archive");
    codegen
}

fn id(codegen: &mut VbcCodegen, name: &str) -> FunctionId {
    codegen.ctx_mut().lookup_function(name).expect(name).id
}

fn supplement(cache: &ArchiveCtxCache, archive: &VbcArchive, codegen: &mut VbcCodegen, name: &str) {
    assert!(
        codegen.ctx_mut().lookup_function(name).is_none(),
        "new requested name {name}"
    );
    // A new name may refer to a body already copied by an earlier wave.
    // Registration, stable identity and execution are the success oracles.
    cache.apply_lazy_supplemental(archive, codegen, [name.to_owned()].into_iter().collect());
    assert!(
        codegen.ctx_mut().lookup_function(name).is_some(),
        "requested declaration {name}"
    );
}

fn finish(codegen: &mut VbcCodegen) -> VbcModule {
    let module = codegen
        .finalize_module_from_state()
        .expect("finalize imported bodies");
    verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).expect("encode merged consumer"),
    )
    .expect("decode merged consumer")
}

fn function(module: &VbcModule, id: FunctionId) -> &FunctionDescriptor {
    let matches: List<_> = module.functions.iter().filter(|f| f.id == id).collect();
    assert_eq!(
        matches.len(),
        1,
        "consumer ID must have exactly one body: {id:?}"
    );
    assert!(
        matches[0].has_source_body,
        "source body became a generated stub: {id:?}"
    );
    matches[0]
}

fn wire_id(module: &VbcModule, name: &str) -> FunctionId {
    let matches: List<_> = module
        .functions
        .iter()
        .filter(|function| module.get_string(function.name) == Some(name))
        .collect();
    assert_eq!(matches.len(), 1, "exact serialized declaration: {name}");
    matches[0].id
}

fn call_targets(module: &VbcModule, name: &str) -> List<u32> {
    let body = function(module, wire_id(module, name));
    let start = body.bytecode_offset as usize;
    verum_vbc::bytecode::decode_instructions(
        &module.bytecode[start..start + body.bytecode_length as usize],
    )
    .expect("decode merged body")
    .iter()
    .filter_map(|instruction| match instruction {
        Instruction::Call { func_id, .. } | Instruction::TailCall { func_id, .. } => Some(*func_id),
        _ => None,
    })
    .collect()
}

fn assert_result(module: &VbcModule, name: &str, expected: i64) {
    // Final serialization assigns contiguous IDs. Keep registration identity
    // checks before this boundary, then resolve the exact declaration anew.
    let id = wire_id(module, name);
    function(module, id);
    let result = verum_vbc::interpreter::Interpreter::new(Shared::new(module.clone()).into_arc())
        .execute_function(id)
        .expect("execute imported source body");
    assert_eq!(
        result.try_as_i64(),
        Some(expected),
        "wrong declaration result for {id:?}"
    );
}

#[test]
fn supplemental_registration_preserves_primary_qualified_and_bare_ids() {
    let archive = archive(&[source("alpha", 37)]);
    let cache = cache(&archive);
    let mut codegen = primary(&cache, &archive, "fn main() -> Int { original() }", 0);
    let original = id(&mut codegen, "alpha.original");
    assert_eq!(id(&mut codegen, "original"), original);
    supplement(&cache, &archive, &mut codegen, "alpha.later");
    assert_eq!(id(&mut codegen, "alpha.original"), original);
    assert_eq!(id(&mut codegen, "original"), original);
    assert_ne!(id(&mut codegen, "alpha.later"), original);
    let module = finish(&mut codegen);
    assert_result(&module, "alpha.original", 37);
    assert_result(&module, "alpha.later", 42);
}

#[test]
fn a_new_body_calls_the_original_consumer_id() {
    let archive = archive(&[source("alpha", 37)]);
    let cache = cache(&archive);
    let mut codegen = primary(&cache, &archive, "fn main() -> Int { original() }", 0);
    let original = id(&mut codegen, "alpha.original");
    supplement(&cache, &archive, &mut codegen, "alpha.later");
    assert_ne!(id(&mut codegen, "alpha.later"), original);
    let module = finish(&mut codegen);
    assert!(
        call_targets(&module, "alpha.later").contains(&wire_id(&module, "alpha.original").0),
        "local call lost its existing body identity"
    );
    assert_result(&module, "alpha.later", 42);
}

#[test]
fn declared_method_aliases_and_real_empty_bodies_survive_supplemental_load() {
    let archive = archive(&[source("alpha", 37)]);
    let cache = cache(&archive);
    let mut codegen = primary(
        &cache,
        &archive,
        "fn main(counter: &Counter) -> Int { counter.empty(); counter.value() + original() }",
        0,
    );
    let names = ["Counter.value", "Counter.empty"];
    let original: List<_> = names
        .iter()
        .map(|name| {
            let qualified: Text = format!("alpha.{name}").into();
            let qualified_id = id(&mut codegen, &qualified);
            assert_eq!(id(&mut codegen, name), qualified_id);
            qualified_id
        })
        .collect();
    supplement(&cache, &archive, &mut codegen, "alpha.later");
    for (name, original) in names.iter().zip(original.iter()) {
        assert_eq!(id(&mut codegen, &format!("alpha.{name}")), *original);
        assert_eq!(id(&mut codegen, name), *original);
    }
    let module = finish(&mut codegen);
    for name in names {
        function(&module, wire_id(&module, name));
    }
}

#[test]
fn same_leaf_foreign_functions_remain_distinct_in_both_archive_orders() {
    for reverse in [false, true] {
        let mut modules = List::from_iter([source("alpha", 37), source("beta", 99)]);
        if reverse {
            modules.reverse();
        }
        let archive = archive(&modules);
        let cache = cache(&archive);
        let mut codegen = primary(&cache, &archive, "fn main() -> Int { original() }", 0);
        let alpha = id(&mut codegen, "alpha.original");
        let beta = id(&mut codegen, "beta.original");
        assert_ne!(alpha, beta);
        supplement(&cache, &archive, &mut codegen, "alpha.later");
        assert_eq!(id(&mut codegen, "alpha.original"), alpha);
        assert_eq!(id(&mut codegen, "beta.original"), beta);
        let module = finish(&mut codegen);
        assert_result(&module, "alpha.original", 37);
        assert_result(&module, "beta.original", 99);
    }
}

#[test]
fn later_supplemental_waves_reuse_the_same_call_closure() {
    let archive = archive(&[source("alpha", 37)]);
    let cache = cache(&archive);
    let mut codegen = primary(&cache, &archive, "fn main() -> Int { original() }", 0);
    let original = id(&mut codegen, "alpha.original");
    supplement(&cache, &archive, &mut codegen, "alpha.later");
    let later = id(&mut codegen, "alpha.later");
    supplement(&cache, &archive, &mut codegen, "alpha.latest");
    assert_eq!(id(&mut codegen, "alpha.original"), original);
    assert_eq!(id(&mut codegen, "alpha.later"), later);
    assert_ne!(id(&mut codegen, "alpha.latest"), later);
    let next = *codegen.next_func_id_mut();
    assert_eq!(
        cache.apply_lazy_supplemental(
            &archive,
            &mut codegen,
            ["alpha.latest".to_owned()].into_iter().collect()
        ),
        0
    );
    assert_eq!(*codegen.next_func_id_mut(), next);
    let module = finish(&mut codegen);
    assert!(call_targets(&module, "alpha.latest").contains(&wire_id(&module, "alpha.later").0));
    assert_result(&module, "alpha.latest", 43);
}

#[test]
fn a_new_compilation_epoch_does_not_reuse_previous_consumer_ids() {
    let archive = archive(&[source("alpha", 37)]);
    let cache = cache(&archive);
    let mut originals = List::new();
    for offset in [0, 1000] {
        let mut codegen = primary(&cache, &archive, "fn main() -> Int { original() }", offset);
        let original = id(&mut codegen, "alpha.original");
        originals.push(original);
        supplement(&cache, &archive, &mut codegen, "alpha.later");
        assert_eq!(id(&mut codegen, "alpha.original"), original);
        assert_ne!(id(&mut codegen, "alpha.later"), original);
        let module = finish(&mut codegen);
        assert!(
            call_targets(&module, "alpha.later").contains(&wire_id(&module, "alpha.original").0)
        );
        assert_result(&module, "alpha.later", 42);
    }
    assert_ne!(
        originals[0], originals[1],
        "new consumer allocation space must be respected"
    );
}

#[test]
fn interleaved_compilations_keep_their_own_function_ids_and_bodies() {
    let archive = archive(&[source("alpha", 37)]);
    let cache = cache(&archive);
    let mut first = primary(&cache, &archive, "fn main() -> Int { original() }", 0);
    let first_original = id(&mut first, "alpha.original");
    let mut second = primary(&cache, &archive, "fn main() -> Int { original() }", 1000);
    let second_original = id(&mut second, "alpha.original");
    assert_ne!(first_original, second_original);

    supplement(&cache, &archive, &mut first, "alpha.later");
    supplement(&cache, &archive, &mut second, "alpha.later");
    assert_eq!(id(&mut first, "alpha.original"), first_original);
    assert_eq!(id(&mut second, "alpha.original"), second_original);
    assert_ne!(
        id(&mut first, "alpha.later"),
        id(&mut second, "alpha.later")
    );
    for codegen in [&mut first, &mut second] {
        let module = finish(codegen);
        assert!(
            call_targets(&module, "alpha.later").contains(&wire_id(&module, "alpha.original").0)
        );
        assert_result(&module, "alpha.original", 37);
        assert_result(&module, "alpha.later", 42);
    }
}

#[test]
fn a_later_name_registration_preserves_remapped_nominal_signatures() {
    use verum_vbc::types::TypeRef;

    let ast = Parser::new(
        "module alpha; public type Parcel is { value: Int }; \
         public fn original() -> Int { 37 } \
         public fn make() -> Parcel { Parcel { value: 37 } } \
         public fn echo(value: Parcel) -> Parcel { value }",
    )
    .parse_module()
    .expect("parse nominal archive producer");
    let producer = VbcCodegen::with_config(CodegenConfig::new("alpha"))
        .compile_module(&ast)
        .expect("compile nominal archive producer");
    let source_type = producer
        .types
        .iter()
        .find(|ty| producer.get_string(ty.name) == Some("Parcel"))
        .expect("declared producer Parcel")
        .id;
    let archive = archive(&[producer]);
    let cache = cache(&archive);
    let consumer = Parser::new("fn main() -> Int { original() }")
        .parse_module()
        .expect("parse nominal consumer");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    // Import an unrelated declared record first, so consumer IDs cannot
    // accidentally equal the archive-local Parcel ID.
    codegen.import_archive_module_types(&source("occupier", 99));
    let (modules, _) = cache.apply_lazy_with_types(&archive, &mut codegen, &consumer, &[]);
    assert!(modules > 0);
    supplement(&cache, &archive, &mut codegen, "alpha.make");
    let expected = codegen
        .ctx_mut()
        .lookup_function("alpha.make")
        .expect("first supplemental nominal return")
        .return_type
        .clone()
        .expect("declared return type");
    assert_ne!(
        expected,
        TypeRef::Concrete(source_type),
        "fixture must shift the nominal ID"
    );
    assert!(codegen.ctx_mut().lookup_function("alpha.echo").is_none());

    // The first supplemental wave merges the complete entry. This later
    // wave may add only names/signatures, but they still need consumer IDs.
    cache.apply_lazy_supplemental(
        &archive,
        &mut codegen,
        ["alpha.echo".to_owned()].into_iter().collect(),
    );
    for name in ["alpha.make", "make", "alpha.echo", "echo"] {
        let info = codegen.ctx_mut().lookup_function(name).expect(name);
        assert_eq!(
            info.return_type.as_ref(),
            Some(&expected),
            "return owner for {name}"
        );
    }
    let echo = id(&mut codegen, "alpha.echo");
    assert_eq!(
        codegen
            .ctx_mut()
            .archive_fn_param_types
            .get(&echo.0)
            .map(|params| params.as_slice()),
        Some(&[expected][..]),
        "parameter owner for alpha.echo"
    );
}

#[test]
fn resetting_the_same_codegen_starts_a_new_archive_identity_space() {
    let archive = archive(&[source("alpha", 37)]);
    let cache = cache(&archive);
    let mut codegen = primary(&cache, &archive, "fn main() -> Int { original() }", 0);
    let previous = id(&mut codegen, "alpha.original");
    supplement(&cache, &archive, &mut codegen, "alpha.later");
    assert_result(&finish(&mut codegen), "alpha.later", 42);

    let reset = Parser::new("fn reset_marker() -> Int { 0 }")
        .parse_module()
        .expect("parse reset boundary");
    codegen
        .compile_module(&reset)
        .expect("reset and compile a new module");
    assert!(
        codegen
            .ctx_mut()
            .lookup_function("alpha.original")
            .is_none()
    );
    *codegen.next_func_id_mut() += 1000;
    let consumer = Parser::new("fn main() -> Int { original() }")
        .parse_module()
        .expect("parse new consumer");
    let (modules, _) = cache.apply_lazy_with_types(&archive, &mut codegen, &consumer, &[]);
    assert!(modules > 0);
    let current = id(&mut codegen, "alpha.original");
    assert!(current.0 >= 1000, "reset must use the new allocator space");
    assert_ne!(current, previous);
    supplement(&cache, &archive, &mut codegen, "alpha.later");
    assert_eq!(id(&mut codegen, "alpha.original"), current);
    assert_result(&finish(&mut codegen), "alpha.later", 42);
}
