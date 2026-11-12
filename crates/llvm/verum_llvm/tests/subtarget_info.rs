use verum_llvm::{OptimizationLevel, context::Context, targets::{Target, TargetTriple, InitializationConfig, RelocMode, CodeModel}};

#[test]
fn capabilities_follow_cpu_features_and_function_attributes() {
    Target::initialize_all(&InitializationConfig::default());
    for (triple, cpu, feature, expected_without, expected_with) in [
        // LLVM accepts these operations in the single-threaded wasm model too;
        // the instruction-set feature is a separate fact, not a width proxy.
        ("wasm32-unknown-unknown", "generic", "atomics", 64, 64),
        ("riscv32-unknown-elf", "generic-rv32", "a", 0, 32),
    ] {
        let triple = TargetTriple::create(triple);
        let target = Target::from_triple(&triple).unwrap();
        let context = Context::create();
        let module = context.create_module("target_metadata");
        module.set_triple(&triple);
        let function = module.add_function("probe", context.void_type().fn_type(&[], false), None);
        for (flag, expected) in [(format!("-{feature}"), expected_without), (format!("+{feature}"), expected_with)] {
            let machine = target.create_target_machine(&triple, cpu, &flag, OptimizationLevel::None, RelocMode::Default, CodeModel::Default).unwrap();
            let facts = machine.subtarget_info(function).expect("target metadata");
            assert_eq!(facts.max_atomic_bits, expected, "{triple:?} {flag}");
            assert_eq!(facts.features.get(&feature.into()), Some(&flag.starts_with('+')));
            assert!(!facts.features.contains_key(&"not-a-cpu-feature".into()));
        }
        // A function override must be read from the selected subtarget, not the
        // module machine's feature string.
        let machine = target.create_target_machine(&triple, cpu, &format!("-{feature}"), OptimizationLevel::None, RelocMode::Default, CodeModel::Default).unwrap();
        function.add_attribute(verum_llvm::attributes::AttributeLoc::Function,
            context.create_string_attribute("target-features", &format!("+{feature}")));
        assert_eq!(machine.subtarget_info(function).unwrap().max_atomic_bits, expected_with);
    }
}
