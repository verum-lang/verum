//! Demonstration of multi-snippet error rendering
//!
//! Run with: `cargo run --package verum_diagnostics --example multi_snippet_demo`
//!
//! This example shows off the beautiful error messages that Verum produces,
//! including multi-file errors, primary/secondary labels, and visual correlation.

use verum_diagnostics::{DiagnosticBuilder, RichRenderer, Span};

fn create_span(file: &str, line: usize, column: usize, end_line: usize, end_column: usize) -> Span {
    Span {
        file: file.into(),
        line,
        column,
        end_column,
        end_line: Some(end_line),
    }
}

fn main() {
    let mut renderer = RichRenderer::default(); // With colors

    println!("═══════════════════════════════════════════════════════════════");
    println!("           Verum Multi-Snippet Error Rendering Demo           ");
    println!("═══════════════════════════════════════════════════════════════\n");

    // =========================================================================
    // Example 1: Type mismatch across multiple files
    // =========================================================================
    println!("Example 1: Type Mismatch Across Multiple Files\n");

    renderer.add_test_content(
        "main.vr",
        "fn main() {\n    let db = connect();\n    process(db);\n}",
    );

    renderer.add_test_content(
        "database.vr",
        "fn connect() -> Database {\n    Database.new()\n}",
    );

    renderer.add_test_content("process.vr", "fn process(config: Config) {\n    // ...\n}");

    let diagnostic1 = DiagnosticBuilder::error()
        .code("E0308")
        .message("mismatched types")
        .span_label(
            create_span("main.vr", 3, 13, 3, 15),
            "expected `Config`, found `Database`",
        )
        .secondary_span(
            create_span("database.vr", 1, 17, 1, 25),
            "this function returns `Database`",
        )
        .secondary_span(
            create_span("process.vr", 1, 16, 1, 22),
            "this function expects `Config`",
        )
        .add_note("consider converting `Database` to `Config` using `.into()` or `.as_config()`")
        .help("if `Database` should implement `From<Config>`, add the trait implementation")
        .build();

    println!("{}", renderer.render(&diagnostic1));
    println!("\n");

    // =========================================================================
    // Example 2: Refinement constraint violation with context
    // =========================================================================
    println!("Example 2: Refinement Constraint Violation\n");

    renderer.add_test_content(
        "math.vr",
        "type Positive = Int where i > 0;\n\nfn calculate(x: Positive) -> Int {\n    x * 2\n}\n\nfn main() {\n    let value = -5;\n    let result = calculate(value);\n}",
    );

    let diagnostic2 = DiagnosticBuilder::error()
        .code("E0312")
        .message("refinement constraint not satisfied")
        .span_label(
            create_span("math.vr", 9, 29, 9, 34),
            "value `-5` fails constraint `i > 0`",
        )
        .secondary_span(
            create_span("math.vr", 1, 16, 1, 28),
            "constraint `i > 0` defined here",
        )
        .secondary_span(
            create_span("math.vr", 3, 14, 3, 22),
            "parameter requires `Positive` type",
        )
        .add_note("the compiler cannot prove that `-5 > 0`")
        .help("use runtime check: `Positive::try_from(value)?`")
        .help("or use compile-time proof: `@verify value > 0`")
        .build();

    println!("{}", renderer.render(&diagnostic2));
    println!("\n");

    // =========================================================================
    // Example 3: Multiple errors in same function (merged view)
    // =========================================================================
    println!("Example 3: Multiple Errors in Same Function (Merged View)\n");

    renderer.add_test_content(
        "validation.vr",
        "fn validate_user(name: Text, age: Int, email: Text) -> Result<User, Error> {\n    if name.is_empty() {\n        return Err(Error.InvalidName);\n    }\n    if age < 0 || age > 150 {\n        return Err(Error.InvalidAge);\n    }\n    if !email.contains(\"@\") {\n        return Err(Error.InvalidEmail);\n    }\n    Ok(User { name, age, email })\n}",
    );

    let diagnostic3 = DiagnosticBuilder::warning()
        .code("W0104")
        .message("unnecessary refinement checks detected")
        .span_label(
            create_span("validation.vr", 2, 8, 2, 23),
            "this check is redundant if `name` has type `NonEmptyText`",
        )
        .span_label(
            create_span("validation.vr", 5, 8, 5, 20),
            "this check is redundant if `age` has type `ValidAge`",
        )
        .span_label(
            create_span("validation.vr", 8, 8, 8, 28),
            "this check is redundant if `email` has type `Email`",
        )
        .add_note("consider using refinement types to encode these constraints at the type level")
        .help("replace `Text` with `NonEmptyText`, `Int` with `ValidAge where 0 <= age <= 150`")
        .build();

    println!("{}", renderer.render(&diagnostic3));
    println!("\n");

    // =========================================================================
    // Example 4: Context system error (missing context)
    // =========================================================================
    println!("Example 4: Context System Error\n");

    renderer.add_test_content(
        "handlers.vr",
        "fn handle_request(request: Request) {\n    let user = get_current_user();\n    log_access(user);\n}\n\nfn get_current_user() -> User using [Database] {\n    // ...\n}\n\nfn log_access(user: User) using [Logger] {\n    // ...\n}",
    );

    let diagnostic4 = DiagnosticBuilder::error()
        .code("E0302")
        .message("context not provided in calling scope")
        .span_label(
            create_span("handlers.vr", 2, 16, 2, 34),
            "function requires `Database` context",
        )
        .span_label(
            create_span("handlers.vr", 3, 5, 3, 15),
            "function requires `Logger` context",
        )
        .secondary_span(
            create_span("handlers.vr", 1, 1, 1, 31),
            "but `handle_request` does not declare `using [Database, Logger]`",
        )
        .help("add `using [Database, Logger]` to the function signature")
        .help("or use `provide` to inject contexts: `provide [db, logger] { /* ... */ }`")
        .build();

    println!("{}", renderer.render(&diagnostic4));
    println!("\n");

    // =========================================================================
    // Example 5: Multi-line span error
    // =========================================================================
    println!("Example 5: Multi-Line Span Error\n");

    renderer.add_test_content(
        "conditions.vr",
        "fn calculate() -> Int {\n    let result = if some_condition {\n        compute_int_value()\n    } else {\n        compute_text_value()\n    };\n    result\n}",
    );

    let diagnostic5 = DiagnosticBuilder::error()
        .code("E0308")
        .message("match arms have incompatible types")
        .span_label(
            create_span("conditions.vr", 2, 18, 6, 6),
            "`if` and `else` have incompatible types",
        )
        .secondary_span(
            create_span("conditions.vr", 3, 9, 3, 28),
            "this returns `Int`",
        )
        .secondary_span(
            create_span("conditions.vr", 5, 9, 5, 29),
            "this returns `Text`",
        )
        .add_note("expected type `Int` because of the `if` branch")
        .help("change the type of one branch to match the other")
        .help("or use a union type: `Result<Int | Text>`")
        .build();

    println!("{}", renderer.render(&diagnostic5));
    println!("\n");

    // =========================================================================
    // Example 6: CBGR (memory safety) error
    // =========================================================================
    println!("Example 6: CBGR Memory Safety Error\n");

    renderer.add_test_content(
        "memory.vr",
        "fn process_items() {\n    let items = create_list();\n    let first = items[0];\n    drop(items);\n    println!(first);  // Error: use after free\n}",
    );

    let diagnostic6 = DiagnosticBuilder::error()
        .code("E0316")
        .message("use of value after it was freed")
        .span_label(
            create_span("memory.vr", 5, 14, 5, 19),
            "value used here after being freed",
        )
        .secondary_span(
            create_span("memory.vr", 4, 5, 4, 16),
            "`items` freed here",
        )
        .secondary_span(
            create_span("memory.vr", 3, 17, 3, 26),
            "`first` holds reference to `items`",
        )
        .add_note("CBGR (Checked Borrow Generation Reference) detected this potential use-after-free at compile time")
        .help("consider cloning the value: `let first = items[0].clone();`")
        .help("or restructure to avoid early drop")
        .build();

    println!("{}", renderer.render(&diagnostic6));
    println!("\n");

    println!("═══════════════════════════════════════════════════════════════");
    println!("              End of Multi-Snippet Demo                       ");
    println!("═══════════════════════════════════════════════════════════════");
}
