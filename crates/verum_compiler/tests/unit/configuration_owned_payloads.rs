//! Owned allocation storage preserves its payload loans, not the address of its backing memory.
use super::*;

#[test]
fn actual_owned_payload_storage_separates_allocation_from_borrowed_values() {
    let session = crate::Session::new(crate::options::CompilerOptions::default());
    let (registry, phase) = release_stdlib_graph(&session);
    let mut failures = List::new();
    for (label, body, accepted) in [
        ("owned_scalar", "fn probe()->Heap<Int> {Heap.new(73)}", true),
        ("owned_list", "fn probe()->List<Heap<Int>> {let mut values:List<Heap<Int>> = List.new();values.push(Heap.new(73));values}", true),
        ("owned_shared", "fn probe()->List<Shared<Int>> {let mut values:List<Shared<Int>> = List.new();values.push(Shared.new(73));values}", true),
        ("shared_affine_payload", "fn probe()->Shared<Heap<Int>> {let shared = Shared.new(Heap.new(73));let copied = shared;let preserved = shared;copied}", true),
        ("borrowed_input", "fn probe(value:&Int)->Heap<&Int> {Heap.new(value)}", true),
        ("local_reference", "fn probe()->Heap<&Int> {let local=73;Heap.new(&local)}", false),
        ("local_mutable_reference", "fn probe()->Heap<&mut Int> {let mut local=73;Heap.new(&mut local)}", false),
        ("shared_local_reference", "fn probe()->Shared<&Int> {let local=73;Shared.new(&local)}", false),
        ("foreign_shared", "type Shared<T> is {ptr:&T}; fn probe()->Shared<Int> {let local=73;Shared{ptr:&local}}", false),
        ("foreign_heap", "type Heap<T> is {ptr:&T}; fn probe()->Heap<Int> {let local=73;Heap{ptr:&local}}", false),
    ] {
        let imports = if label.starts_with("foreign_") { "" } else { "mount core.base.memory.{Heap,Shared}; mount core.collections.list.List;" };
        let source = format!("module owned_payload_probe; {imports} {body}");
        let module = Parser::new(&source).parse_module().unwrap();
        let result = check_source_module(&mut phase.configured_checker(), "owned_payload_probe", &module, &registry);
        eprintln!("ACTUAL_OWNED_PAYLOAD {label}: {result:?}");
        let agrees = if accepted { result.is_ok() } else {
            matches!(&result, Err(errors) if errors.iter().any(|error| matches!(error.code(), Some("E312" | "E313"))))
        };
        if !agrees { failures.push((label, result)); }
    }
    assert!(failures.is_empty(), "{failures:?}");
}
