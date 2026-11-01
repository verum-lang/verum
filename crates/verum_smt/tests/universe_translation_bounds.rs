use verum_smt::type_translator::{TypeTranslationError, TypeTranslator};
use verum_types::ty::{Type, UniverseLevel as Level};

#[test]
fn unresolved_levels_cannot_become_concrete_sorts_through_their_ids() {
    let mut translator = TypeTranslator::new();
    for id in [0, 1, 1000, u32::MAX] {
        translator
            .translate_type_to_sort(&Type::universe(Level::Concrete(id)))
            .unwrap();
        for level in [Level::Variable(id), Level::Variable(id).succ()] {
            assert!(matches!(
                translator.translate_type_to_sort(&Type::universe(level)),
                Err(TypeTranslationError::InvalidUniverse(_))
            ));
        }
    }
}

#[test]
fn concrete_structural_levels_keep_their_exact_values() {
    let mut translator = TypeTranslator::new();
    let boundary = Level::Concrete(u32::MAX);
    let original = translator
        .translate_type_to_sort(&Type::universe(boundary.clone()))
        .unwrap();
    let successor = translator
        .translate_type_to_sort(&Type::universe(boundary.succ()))
        .unwrap();
    assert_ne!(original, successor);
    assert_eq!(successor.to_string(), "Universe_4294967296");
    let equivalent = Level::Max(Box::new(boundary.succ()), Box::new(Level::TYPE1));
    assert_eq!(
        translator
            .translate_type_to_sort(&Type::universe(equivalent))
            .unwrap(),
        successor
    );
}
