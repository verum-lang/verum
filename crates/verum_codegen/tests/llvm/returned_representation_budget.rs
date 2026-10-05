use super::*;

#[test]
fn a_shared_aggregate_dag_does_not_expand_exponentially() {
    let mut state = State::default();
    state.objects.insert(0, R::Slot);
    for level in 1..=16 {
        let mut fields = List::new();
        fields.push(R::Allocation(level - 1));
        fields.push(R::Allocation(level - 1));
        let mut variants = List::new();
        variants.push((1, fields));
        state.objects.insert(level, R::Sum(TypeId(600), variants));
    }
    assert_eq!(state.materialize(&R::Allocation(16), 0), R::Unknown);
    assert!(
        matches!(state.materialize(&R::Allocation(3), 0), R::Sum(..)),
        "the bounded positive remains analyzable"
    );
}

#[test]
fn parameter_substitution_has_one_budget_across_all_fields() {
    let mut leaf_fields = List::new();
    for _ in 0..32 {
        leaf_fields.push(R::Slot);
    }
    let mut leaf_variants = List::new();
    leaf_variants.push((1, leaf_fields));
    let argument = R::Sum(TypeId(600), leaf_variants);
    let mut fields = List::new();
    for _ in 0..32 {
        fields.push(R::Parameter(0, Some(TypeId(600))));
    }
    let mut variants = List::new();
    variants.push((1, fields));
    let result = R::Sum(TypeId(601), variants).substitute(&[argument], 0);
    assert_eq!(result, R::Unknown);
}

#[test]
fn mutation_invalidates_future_reads_but_not_the_already_read_word() {
    let mut state = State::default();
    state.put(Reg(0), R::Parameter(0, Some(TypeId(600))));
    state.guards.push((state.raw(Reg(0)), 1));
    let extracted = state.projection(Reg(0), 0);
    state.put(Reg(1), extracted.clone());
    state.invalidate_storage();
    assert_eq!(state.projection(Reg(0), 0), R::Unknown);
    assert_eq!(state.value(Reg(1)), extracted);
    assert_eq!(
        R::Slot.join(&R::Unknown),
        R::Unknown,
        "reachable unknown is not an unreachable exit"
    );
}

#[test]
fn absent_or_adapted_payload_receipt_is_not_a_raw_word_proof() {
    let mut state = State::default();
    state.put(Reg(0), R::Parameter(0, Some(TypeId(600))));
    state.guards.push((state.raw(Reg(0)), 1));
    let mut sites: Map<usize, List<ReferenceSiteKind>> = Map::new();
    assert_eq!(
        payload_projection(&state, &sites, 3, Reg(1), Reg(0), 0),
        R::Unknown
    );
    for raw_word in [false, true] {
        let mut facts = List::new();
        facts.push(ReferenceSiteKind::PayloadOutput {
            register: 1,
            raw_word,
        });
        sites.insert(3, facts);
        let result = payload_projection(&state, &sites, 3, Reg(1), Reg(0), 0);
        assert_eq!(matches!(result, R::Projection(..)), raw_word);
    }
}

#[test]
fn payload_replacement_spends_one_budget_before_copying_multiple_variants() {
    let mut variants = List::new();
    for tag in 0..64 {
        variants.push((tag, List::from_elem(R::Unknown, 1)));
    }
    let object = R::Sum(TypeId(600), variants);
    let mut nested = List::new();
    nested.push((1, List::from_elem(R::Slot, 16)));
    assert_eq!(
        replace_payload(&object, 0, &R::Sum(TypeId(601), nested)),
        R::Unknown
    );
    assert!(matches!(replace_payload(&object, 0, &R::Slot), R::Sum(..)));
}

#[test]
fn reference_opcode_does_not_prove_a_native_passthrough() {
    let ast = verum_fast_parser::Parser::new("fn echo(x: Int) -> Int { x }")
        .parse_module()
        .unwrap();
    let mut module = verum_vbc::codegen::VbcCodegen::new()
        .compile_module(&ast)
        .unwrap();
    let id = module
        .functions
        .iter()
        .find(|f| module.get_string(f.name) == Some("echo"))
        .unwrap()
        .id;
    let descriptor = module.functions.iter_mut().find(|f| f.id == id).unwrap();
    let mut instructions = List::new();
    instructions.push(I::RefObj {
        dst: Reg(1),
        src: Reg(0),
    });
    instructions.push(I::Ret { value: Reg(1) });
    descriptor.instructions = Some(instructions.into_iter().collect());
    let mut calls = Map::new();
    calls.insert(id.0, List::new());
    let mut sites = Map::new();
    let mut missing = Analysis::new(&module, &calls, &sites);
    assert_eq!(missing.body(id).result, R::Unknown);
    let mut outputs = List::new();
    outputs.push(ReferenceSite {
        instruction: 0,
        kind: ReferenceSiteKind::PassthroughOutput {
            register: 1,
            source: 0,
        },
        anchor: None,
    });
    sites.insert(id.0, outputs);
    let mut proven = Analysis::new(&module, &calls, &sites);
    assert!(matches!(proven.body(id).result, R::Parameter(0, _)));
}

#[test]
fn sparse_sidecars_are_bounded_even_after_register_facts_are_lost() {
    let mut cells = State::default();
    let mut origins = State::default();
    for reg in 0..256 {
        cells.cells.insert(reg, false);
        origins.parameter_origins.insert(reg, 0);
    }
    assert!(cells.bounded());
    assert!(origins.bounded());
    cells.cells.insert(256, false);
    origins.parameter_origins.insert(256, 0);
    assert!(!cells.bounded());
    assert!(!origins.bounded());
}
