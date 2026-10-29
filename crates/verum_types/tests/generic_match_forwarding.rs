//! Generic payloads stay tied to their declaration when forwarded to a method.
use verum_ast::ItemKind;
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_types::infer::TypeChecker;

fn errors(source: &str) -> List<Text> {
    let ast = Parser::new(source).parse_module().expect("source grammar");
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    for item in &ast.items {
        if let ItemKind::Type(decl) = &item.kind {
            checker
                .register_type_declaration(decl)
                .expect("type declaration");
        }
    }
    for item in &ast.items {
        if let ItemKind::Impl(decl) = &item.kind {
            checker.register_impl_block(decl).expect("impl declaration");
        }
        if let ItemKind::Function(function) = &item.kind {
            checker
                .register_function_signature(function)
                .expect("signature");
        }
    }
    let mut errors: List<_> = ast
        .items
        .iter()
        .filter_map(|item| {
            checker
                .check_item(item)
                .err()
                .map(|e| Text::from(format!("{e:?}")))
        })
        .collect();
    errors.extend(
        checker
            .take_deferred_errors()
            .into_iter()
            .map(|e| Text::from(format!("{e:?}"))),
    );
    errors.extend(
        checker
            .diagnostics()
            .iter()
            .filter(|d| d.is_error())
            .map(|e| Text::from(format!("{e:?}"))),
    );
    errors
}

const DECLARATIONS: &str = r#"
type Count is { value: Int };
type affine Ticket is { count: &mut Count };
implement Drop for Ticket { fn drop(&mut self) { self.count.value = self.count.value + 1; } }
type Choice<T> is Held(T) | Empty;
implement<T> Drop for Choice<T> {
    fn drop(&mut self) {
        match &mut self { Choice.Held(value) => value.drop(), Choice.Empty => {} }
    }
}
type Sink<T> is { slot: &mut Choice<T> };
implement<T> Sink<T> {
    fn store(&self, value: T) {
        let owned: T = value;
        *self.slot = Choice.Held(owned);
    }
}
type Lease<T> is { value: Choice<T> };
"#;

#[test]
fn implementation_parameter_remains_determined_after_match_and_forwarding() {
    for annotation in ["", ": T"] {
        for qualifier in ["", "Choice."] {
            let source = format!(
                r#"{DECLARATIONS}
implement<T> Lease<T> {{
    fn release(mut self, sink: &Sink<T>) {{
        let value{annotation} = match self.value {{
            {qualifier}Held(value) => value, {qualifier}Empty => return
        }};
        self.value = Choice.Empty;
        sink.store(value);
    }}
}}
fn probe() {{
    let mut count = Count {{ value: 0 }};
    let mut slot: Choice<Ticket> = Choice.Empty;
    let sink: Sink<Ticket> = Sink {{ slot: &mut slot }};
    let lease: Lease<Ticket> = Lease {{ value: Choice.Held(Ticket {{ count: &mut count }}) }};
    lease.release(&sink);
}}
"#
            );
            let diagnostics = errors(&source);
            assert!(diagnostics.is_empty(), "{source}\n{diagnostics:?}");
        }
    }
}

#[test]
fn concrete_payload_mismatch_is_rejected() {
    let source = format!(
        r#"{DECLARATIONS}
fn wrong(value: Choice<Int>, sink: &Sink<Bool>) {{
    let payload = match value {{ Choice.Held(payload) => payload, Choice.Empty => return }};
    sink.store(payload);
}}
"#
    );
    assert!(!errors(&source).is_empty(), "incompatible payload accepted");
}

#[test]
fn an_uninstantiated_result_is_still_ambiguous() {
    let source = format!(
        r#"{DECLARATIONS}
fn empty<T>() -> Choice<T> {{ Choice.Empty }}
fn unresolved() {{ let value = empty(); }}
"#
    );
    let diagnostics = errors(&source);
    assert!(
        diagnostics.iter().any(|e| e.contains("E404")),
        "{diagnostics:?}"
    );
}
