use verum_ast::FileId;
use verum_common::{List, Text};
use verum_fast_parser::FastParser;
use verum_types::TypeChecker;

const SHAPES: &str = "
    type Slot<T> is Empty | Ready(T) | Pair(Int, T);
    type Checkout<T> is { slot: Slot<T>, owner: &Int, count: Int };
";

fn diagnostics(declarations: &str, caller: &str) -> List<Text> {
    let source = format!("{SHAPES} {declarations} {caller}");
    check_module(&source, None)
}

fn check_module(source: &str, prelude: Option<&str>) -> List<Text> {
    let module = FastParser::new()
        .parse_module_str(source, FileId::new(1))
        .expect("valid test source");
    let mut checker = TypeChecker::with_minimal_context();
    checker.register_primitives();
    checker.set_current_module_path("probe");
    if let Some(source) = prelude {
        use verum_modules::{ModuleInfo, ModulePath, ModuleRegistry, extract_exports_from_module};
        let file = FileId::new(2);
        let ast = FastParser::new().parse_module_str(source, file).unwrap();
        let mut registry = ModuleRegistry::new();
        let id = registry.allocate_id();
        let path = ModulePath::from_str("core.prelude");
        let exports = extract_exports_from_module(&ast, id, &path).unwrap();
        let mut info = ModuleInfo::new(id, path, ast, file, source.into());
        info.exports = exports;
        registry.register(info);
        checker.set_module_registry_direct(registry);
    }
    let mut errors = checker.prepare_module_declarations(&module);
    checker.with_module_declarations(&module, |checker| {
        for item in &module.items {
            if let Err(error) = checker.check_item(item) {
                errors.push(error);
            }
        }
    });
    errors.extend(checker.take_deferred_errors());
    errors
        .iter()
        .chain(checker.diagnostic_sources())
        .map(|error| error.to_string().into())
        .collect()
}

fn method(empty: &str) -> Text {
    format!(
        "implement<T> Checkout<T> {{
        fn project(&mut self, candidate: &Int) -> &mut T {{
            match &mut self.slot {{
                Slot.Ready(item) => item,
                Slot.Pair(_, second) => second,
                Slot.Empty => {empty},
            }}
        }}
    }}"
    )
    .into()
}

const READ_AFTER: &str = "
    fn keep_checkout(owner: &Int) -> Checkout<Int> {
        let mut checkout: Checkout<Int> = Checkout { slot: Slot.Ready(5), owner: owner, count: 0 };
        { let short = 2; let value = *checkout.project(&short); }
        checkout
    }
";

fn assert_unknown_write(declarations: &str) {
    let errors = diagnostics(declarations, "");
    assert!(errors.is_empty(), "invalid declaration: {errors:?}");
    let errors = diagnostics(declarations, READ_AFTER);
    assert!(
        errors
            .iter()
            .any(|error| error.contains("cannot establish the lifetime")
                || error.contains("does not live long enough")),
        "expected a conservative mutable call in {declarations}: {errors:?}"
    );
}

#[test]
fn returning_each_borrowed_binder_preserves_the_checkout() {
    let errors = diagnostics(&method("panic(\"empty\")"), READ_AFTER);
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn exhaustive_returns_from_different_payload_positions_work_with_a_source_prelude() {
    let shapes = SHAPES.replace("Empty | ", "");
    let declaration = method("panic(\"empty\")").replace("Slot.Empty => panic(\"empty\"),", "");
    let errors = check_module(
        &format!("{shapes} {declaration} {READ_AFTER}"),
        Some("public fn panic(message: Text) -> ! { unreachable() }"),
    );
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn a_raw_declaration_cannot_borrow_the_callers_builtin_over_its_own_prelude() {
    let source = format!("{SHAPES} {} {READ_AFTER}", method("panic(\"empty\")"));
    let errors = check_module(
        &source,
        Some("public fn panic(message: Text) -> ! { unreachable() }"),
    );
    assert!(
        errors
            .iter()
            .any(|error| error.contains("cannot establish the lifetime")),
        "{errors:?}"
    );
}

#[test]
fn returning_the_borrow_still_requires_the_checkout_to_live() {
    let errors = diagnostics(
        &method("panic(\"empty\")"),
        "
        fn escape(owner: &Int) -> &mut Int {
            let mut checkout = Checkout { slot: Slot.Ready(1), owner: owner, count: 0 };
            checkout.project(owner)
        }
    ",
    );
    assert!(
        errors
            .iter()
            .any(|error| error.contains("checkout") && error.contains("does not live long enough")),
        "{errors:?}"
    );
}

#[test]
fn terminal_spelling_does_not_authorize_user_code_or_argument_evaluation() {
    assert_unknown_write(&format!(
        "fn panic(message: Text) -> ! {{ unreachable() }} {}",
        method("panic(\"empty\")")
    ));
    assert_unknown_write(&format!(
        "fn message() -> Text {{ \"empty\" }} {}",
        method("panic(message())")
    ));
    assert_unknown_write(&method("panic({ self.count = 1; \"empty\" })"));
    assert_unknown_write(&method("{ self.count = 1; panic(\"empty\") }"));
}

#[test]
fn a_guard_and_an_active_destructor_keep_the_mutable_call_conservative() {
    let guarded = method("panic(\"empty\")").replace(
        "Slot.Empty =>",
        "Slot.Empty if { self.count = 1; true } => panic(\"guarded\"), Slot.Empty =>",
    );
    assert_unknown_write(&guarded);

    let cleanup = "static mut OBSERVED: Int = 0;
        type Drop is protocol { fn drop(&mut self); };
        type affine Pending is { count: Int };
        implement Drop for Pending { fn drop(&mut self) { OBSERVED = self.count; } }";
    let body = method("{ let pending = Pending { count: 1 }; panic(\"empty\") }");
    assert_unknown_write(&format!("{cleanup} {body}"));
}
