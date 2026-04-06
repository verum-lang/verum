//! Demonstration of world-class context error messages.
//!
//! This example shows how the enhanced diagnostics system provides
//! clear, actionable error messages for context-related issues.

use verum_diagnostics::{
    CallChain, CallFrame, ContextGroupUndefinedError, ContextNotDeclaredError,
    ContextNotProvidedError, ContextTypeMismatchError, Renderer, Span,
};

fn main() {
    println!("=== Verum Diagnostics: Context Error Examples ===\n");

    // Example 1: Simple context not declared error
    demo_simple_context_error();

    println!("\n{}\n", "=".repeat(80));

    // Example 2: Context error with call chain
    demo_context_error_with_call_chain();

    println!("\n{}\n", "=".repeat(80));

    // Example 3: Context error with "did you mean" suggestions
    demo_context_error_with_suggestions();

    println!("\n{}\n", "=".repeat(80));

    // Example 4: Context not provided
    demo_context_not_provided();

    println!("\n{}\n", "=".repeat(80));

    // Example 5: Context type mismatch
    demo_context_type_mismatch();

    println!("\n{}\n", "=".repeat(80));

    // Example 6: Context group undefined
    demo_context_group_undefined();
}

fn demo_simple_context_error() {
    println!("Example 1: Simple Context Not Declared Error\n");

    let error =
        ContextNotDeclaredError::new("Database", Span::new("src/user_service.vr", 42, 15, 25))
            .build();

    let mut renderer = Renderer::default();
    renderer.add_test_content(
        "src/user_service.vr",
        r#"fn get_user(id: UserId) -> Maybe<User> {
    let user = Database.fetch_user(id).await?;
    Some(user)
}
"#,
    );

    let output = renderer.render(&error);
    println!("{}", output);
}

fn demo_context_error_with_call_chain() {
    println!("Example 2: Context Error with Call Chain Visualization\n");

    let chain = CallChain::new("Database")
        .add_frame(CallFrame::new("main", Span::new("src/main.vr", 10, 1, 5)))
        .add_frame(CallFrame::new(
            "handle_request",
            Span::new("src/handlers.vr", 20, 5, 19),
        ))
        .add_frame(
            CallFrame::new("process_user", Span::new("src/user_service.vr", 35, 8, 20))
                .with_contexts(vec!["Logger".into()].into()),
        )
        .add_frame(
            CallFrame::new("get_user", Span::new("src/user_service.vr", 42, 12, 20))
                .with_contexts(vec!["Database".into(), "Logger".into()].into())
                .origin(),
        );

    let error =
        ContextNotDeclaredError::new("Database", Span::new("src/user_service.vr", 42, 15, 25))
            .with_call_chain(chain)
            .build();

    let mut renderer = Renderer::default();
    renderer.add_test_content(
        "src/user_service.vr",
        r#"async fn process_user(id: UserId) -> Result<UserData, Error> {
    Logger.log(Level.Info, f"Processing user {id}");
    let user = get_user(id).await?;
    Ok(UserData { user })
}

async fn get_user(id: UserId) -> User {
    Database.fetch_user(id).await?
}
"#,
    );

    let output = renderer.render(&error);
    println!("{}", output);
}

fn demo_context_error_with_suggestions() {
    println!("Example 3: Context Error with 'Did You Mean' Suggestions\n");

    let similar = vec![
        "DataSource".into(),
        "DatabasePool".into(),
        "DatabaseConnection".into(),
    ]
    .into();

    let error = ContextNotDeclaredError::new(
        "Datbase", // Typo
        Span::new("src/user_service.vr", 42, 15, 23),
    )
    .with_similar_contexts(similar)
    .build();

    let mut renderer = Renderer::default();
    renderer.add_test_content(
        "src/user_service.vr",
        r#"async fn get_user(id: UserId) -> User
    using [Datbase]  // Typo here
{
    Datbase.fetch_user(id).await?
}
"#,
    );

    let output = renderer.render(&error);
    println!("{}", output);
}

fn demo_context_not_provided() {
    println!("Example 4: Context Declared But Not Provided\n");

    let error = ContextNotProvidedError::new(
        "Database",
        Span::new("src/user_service.vr", 10, 1, 60), // Function declaration
        Span::new("src/main.vr", 25, 8, 20),         // Call site
    )
    .build();

    let mut renderer = Renderer::default();
    renderer.add_test_content(
        "src/user_service.vr",
        "async fn get_user(id: UserId) -> User\n    using [Database]\n{\n    Database.fetch_user(id).await?\n}\n",
    );
    renderer.add_test_content(
        "src/main.vr",
        r#"async fn main() {
    // Missing: provide Database = ...
    let user = get_user(UserId(42)).await;
    println!("{:?}", user);
}
"#,
    );

    let output = renderer.render(&error);
    println!("{}", output);
}

fn demo_context_type_mismatch() {
    println!("Example 5: Context Type Mismatch\n");

    let error = ContextTypeMismatchError::new(
        "Database",
        "DatabaseInterface",
        "LoggerInterface",
        Span::new("src/main.vr", 15, 17, 30),
    )
    .build();

    let mut renderer = Renderer::default();
    renderer.add_test_content(
        "src/main.vr",
        r#"async fn main() {
    provide Database = ConsoleLogger::new();  // Wrong type!
    let user = get_user(UserId(42)).await;
}
"#,
    );

    let output = renderer.render(&error);
    println!("{}", output);
}

fn demo_context_group_undefined() {
    println!("Example 6: Context Group Undefined\n");

    let available = vec![
        "ApiContext".into(),
        "AdminContext".into(),
        "TestContext".into(),
    ]
    .into();

    let error =
        ContextGroupUndefinedError::new("WebContext", Span::new("src/handlers.vr", 10, 11, 21))
            .with_available_groups(available)
            .build();

    let mut renderer = Renderer::default();
    renderer.add_test_content(
        "src/handlers.vr",
        r#"async fn handle_request(req: Request) -> Response
    using WebContext  // Undefined
{
    // handler code
}
"#,
    );

    let output = renderer.render(&error);
    println!("{}", output);
}
