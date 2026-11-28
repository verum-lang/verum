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
fn raw_place_requires_the_selected_local_borrow_and_conversion() {
    use verum_vbc::instruction::CbgrSubOpcode;

    let ast = verum_fast_parser::Parser::new("fn check(value: Int) -> Int { value }")
        .parse_module()
        .unwrap();
    let mut module = verum_vbc::codegen::VbcCodegen::new().compile_module(&ast).unwrap();
    let id = module.find_function_by_name("check").unwrap();
    let mut calls = Map::new();
    calls.insert(id.0, List::new());

    for (producer, place) in [
        (I::RefLocal { dst: Reg(1), src: Reg(0) }, R::MutableLocalSlot),
        (I::Ref { dst: Reg(1), src: Reg(0) }, R::SharedLocalSlot),
        (I::MemExtended { sub_op: Mem::DerefValue as u8, operands: vec![1, 0] }, R::Unknown),
    ] {
        module.functions.iter_mut().find(|f| f.id == id).unwrap().instructions = Some(vec![
            producer,
            I::Mov { dst: Reg(2), src: Reg(1) },
            I::CbgrExtended { sub_op: CbgrSubOpcode::ToRawPtr.to_byte(), operands: vec![3, 2] },
            I::GetF { dst: Reg(4), obj: Reg(3), field_idx: 0 },
            I::SetF { obj: Reg(3), field_idx: 0, value: Reg(4) },
            I::Ret { value: Reg(3) },
        ]);
        for slot_register in [0, 1] {
            for cast_source in [1, 2] {
                let mut sites = Map::new();
                let mut receipts = List::new();
                receipts.push(ReferenceSite {
                    instruction: 0,
                    kind: ReferenceSiteKind::SlotOutput(slot_register),
                    anchor: None,
                });
                receipts.push(ReferenceSite {
                    instruction: 2,
                    kind: ReferenceSiteKind::PassthroughOutput { register: 3, source: cast_source },
                    anchor: None,
                });
                sites.insert(id.0, receipts);
                let facts = Analysis::new(&module, &calls, &sites).body(id);
                let expected = if slot_register == 1 && cast_source == 2 { &place } else { &R::Unknown };
                assert_eq!(&facts.result, expected);
                let field_view = |pc| facts.value_inputs.get(&pc).is_some_and(|regs| regs.contains(&3));
                assert_eq!(field_view(3), *expected != R::Unknown);
                assert_eq!(field_view(4), *expected == R::MutableLocalSlot);
            }
        }
    }
    assert_eq!(R::MutableLocalSlot.join(&R::Slot), R::Slot);
    assert_eq!(R::MutableLocalSlot.join(&R::SharedLocalSlot), R::Unknown);
    for local in [R::MutableLocalSlot, R::SharedLocalSlot] {
        assert_eq!(local.substitute(&[], 0), R::Unknown);
        assert_eq!(R::Parameter(0, None).substitute(std::slice::from_ref(&local), 0), local);
    }
}

#[test]
fn sparse_sidecars_are_bounded_even_after_register_facts_are_lost() {
    let mut cells = State::default();
    let mut origins = State::default();
    for reg in 0..256 {
        cells.cells.insert(reg, false);
        origins.parameter_origins.entry(reg).or_default().insert(0);
    }
    assert!(cells.bounded());
    assert!(origins.bounded());
    cells.cells.insert(256, false);
    origins.parameter_origins.entry(256).or_default().insert(0);
    assert!(!cells.bounded());
    assert!(!origins.bounded());
}
