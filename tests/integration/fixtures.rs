//! Test Fixtures for Integration Tests
//!
//! Provides sample Verum programs and test data for integration testing.

use verum_std::core::{List, Text, Map};

// ============================================================================
// Simple Programs
// ============================================================================

pub const SIMPLE_ARITHMETIC: &str = "2 + 3 * 4";

pub const SIMPLE_FUNCTION: &str = r#"
fn add(x: Int, y: Int) -> Int {
    x + y
}
"#;

pub const SIMPLE_VARIABLE: &str = r#"
let x = 42;
let y = x + 10;
y
"#;

// ============================================================================
// Recursive Functions
// ============================================================================

pub const FACTORIAL: &str = r#"
fn factorial(n: Int) -> Int {
    match n {
        0 => 1,
        n => n * factorial(n - 1)
    }
}
"#;

pub const FIBONACCI: &str = r#"
fn fibonacci(n: Int) -> Int {
    match n {
        0 => 0,
        1 => 1,
        n => fibonacci(n - 1) + fibonacci(n - 2)
    }
}
"#;

pub const QUICKSORT: &str = r#"
fn quicksort(list: List<Int>) -> List<Int> {
    match list {
        [] => [],
        [pivot, ...rest] => {
            let smaller = rest.filter(|x| x < pivot);
            let larger = rest.filter(|x| x >= pivot);
            quicksort(smaller) ++ [pivot] ++ quicksort(larger)
        }
    }
}
"#;

// ============================================================================
// Pattern Matching
// ============================================================================

pub const PATTERN_MATCHING_LITERALS: &str = r#"
fn classify_number(n: Int) -> Text {
    match n {
        0 => "zero",
        1 => "one",
        2 => "two",
        _ => "many"
    }
}
"#;

pub const PATTERN_MATCHING_TUPLES: &str = r#"
fn describe_pair(pair: (Int, Int)) -> Text {
    match pair {
        (0, 0) => "origin",
        (0, _) => "on y-axis",
        (_, 0) => "on x-axis",
        (x, y) if x == y => "diagonal",
        _ => "other"
    }
}
"#;

pub const PATTERN_MATCHING_LISTS: &str = r#"
fn sum_list(list: List<Int>) -> Int {
    match list {
        [] => 0,
        [head, ...tail] => head + sum_list(tail)
    }
}
"#;

// ============================================================================
// Type System Features
// ============================================================================

pub const REFINEMENT_TYPES: &str = r#"
type PositiveInt = { x: Int | x > 0 }
type NonEmptyList<T> = { l: List<T> | l.length > 0 }

fn safe_head<T>(list: NonEmptyList<T>) -> T {
    list[0]
}
"#;

pub const POLYMORPHIC_FUNCTIONS: &str = r#"
fn identity<T>(x: T) -> T {
    x
}

fn map<A, B>(list: List<A>, f: A -> B) -> List<B> {
    match list {
        [] => [],
        [head, ...tail] => [f(head), ...map(tail, f)]
    }
}
"#;

pub const TYPE_ALIASES: &str = r#"
type UserId = Int
type UserName = Text
type User = { id: UserId, name: UserName }

fn create_user(id: UserId, name: UserName) -> User {
    User { id, name }
}
"#;

// ============================================================================
// Context System
// ============================================================================

pub const CONTEXT_BASIC: &str = r#"
using [Database, Logger]

fn fetch_user(id: Int) -> User {
    Logger.info("Fetching user");
    Database.query("SELECT * FROM users WHERE id = ?", [id])
}
"#;

pub const CONTEXT_COMPOSITION: &str = r#"
using [Database, Cache, Logger]

fn get_user_cached(id: Int) -> User {
    match Cache.get(id) {
        Some(user) => {
            Logger.debug("Cache hit");
            user
        },
        None => {
            Logger.debug("Cache miss");
            let user = Database.query("SELECT * FROM users WHERE id = ?", [id]);
            Cache.set(id, user);
            user
        }
    }
}
"#;

// ============================================================================
// Async/Concurrent Programs
// ============================================================================

pub const ASYNC_FUNCTION: &str = r#"
async fn fetch_data(url: Text) -> Result<Text, Error> {
    let response = await Http.get(url);
    Ok(response.body)
}
"#;

pub const CONCURRENT_PROCESSING: &str = r#"
async fn process_batch(items: List<Item>) -> List<Result> {
    let tasks = items.map(|item| async { process_item(item) });
    await Promise.all(tasks)
}
"#;

// ============================================================================
// Error Handling
// ============================================================================

pub const ERROR_HANDLING_RESULT: &str = r#"
fn divide(x: Int, y: Int) -> Result<Int, Text> {
    if y == 0 {
        Err("Division by zero")
    } else {
        Ok(x / y)
    }
}
"#;

pub const ERROR_HANDLING_MAYBE: &str = r#"
fn safe_head<T>(list: List<T>) -> Maybe<T> {
    match list {
        [] => None,
        [head, ...] => Some(head)
    }
}
"#;

pub const ERROR_PROPAGATION: &str = r#"
fn read_config(path: Text) -> Result<Config, Error> {
    let content = try File.read(path);
    let json = try Json.parse(content);
    let config = try Config.from_json(json);
    Ok(config)
}
"#;

// ============================================================================
// I/O and File System
// ============================================================================

pub const FILE_IO: &str = r#"
using [FileSystem]

fn read_and_process(path: Text) -> Result<Text, Error> {
    let content = try FileSystem.read(path);
    let processed = content.trim().to_upper();
    Ok(processed)
}
"#;

pub const FILE_IO_ASYNC: &str = r#"
using [FileSystem]

async fn copy_file(source: Text, dest: Text) -> Result<(), Error> {
    let content = await FileSystem.read_async(source);
    await FileSystem.write_async(dest, content);
    Ok(())
}
"#;

// ============================================================================
// JSON Processing
// ============================================================================

pub const JSON_PARSING: &str = r#"
using [Json]

type Config = {
    host: Text,
    port: Int,
    debug: Bool
}

fn load_config(path: Text) -> Result<Config, Error> {
    let content = try File.read(path);
    let json = try Json.parse(content);
    let config = try Config.from_json(json);
    Ok(config)
}
"#;

// ============================================================================
// Regex Processing
// ============================================================================

pub const REGEX_MATCHING: &str = r#"
using [Regex]

fn validate_email(email: Text) -> Bool {
    let pattern = Regex.compile(r"^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$");
    pattern.is_match(email)
}
"#;

pub const REGEX_EXTRACTION: &str = r#"
using [Regex]

fn extract_numbers(text: Text) -> List<Int> {
    let pattern = Regex.compile(r"\d+");
    pattern.find_all(text).map(|m| Int.parse(m))
}
"#;

// ============================================================================
// CBGR Memory Management
// ============================================================================

pub const CBGR_BASIC: &str = r#"
fn use_reference(x: &Int) -> Int {
    *x + 1
}
"#;

pub const CBGR_CHECKED: &str = r#"
fn use_checked_reference(x: &checked Int) -> Int {
    *x + 1
}
"#;

pub const CBGR_UNSAFE: &str = r#"
fn use_unsafe_reference(x: &unsafe Int) -> Int {
    *x + 1
}
"#;

// ============================================================================
// Standard Library Usage
// ============================================================================

pub const STDLIB_COLLECTIONS: &str = r#"
fn process_data() -> Int {
    let list = List.from([1, 2, 3, 4, 5]);
    let doubled = list.map(|x| x * 2);
    let filtered = doubled.filter(|x| x > 5);
    filtered.sum()
}
"#;

pub const STDLIB_MAP: &str = r#"
fn word_count(text: Text) -> Map<Text, Int> {
    let words = text.split(" ");
    let counts = Map.new();

    for word in words {
        let count = counts.get(word).unwrap_or(0);
        counts.set(word, count + 1);
    }

    counts
}
"#;

// ============================================================================
// Complex Programs
// ============================================================================

pub const WEB_SERVER: &str = r#"
using [Http, Database, Logger]

async fn handle_request(request: Request) -> Response {
    Logger.info("Handling request");

    match request.path {
        "/users" => {
            let users = await Database.query("SELECT * FROM users");
            Response.json(users)
        },
        "/users/:id" => {
            let id = request.params.get("id");
            let user = await Database.query("SELECT * FROM users WHERE id = ?", [id]);
            Response.json(user)
        },
        _ => Response.not_found()
    }
}
"#;

pub const DATA_PIPELINE: &str = r#"
using [FileSystem, Json, Database]

async fn import_data(path: Text) -> Result<(), Error> {
    Logger.info("Starting data import");

    // Read CSV file
    let content = try await FileSystem.read_async(path);
    let rows = Csv.parse(content);

    // Transform data
    let records = rows.map(|row| {
        Record {
            id: row[0],
            name: row[1],
            value: row[2]
        }
    });

    // Insert into database
    for record in records {
        try await Database.insert("records", record);
    }

    Logger.info("Import complete");
    Ok(())
}
"#;

// ============================================================================
// Test Data Generators
// ============================================================================

pub fn generate_function_program(num_functions: usize) -> String {
    let mut program = String::new();
    for i in 0..num_functions {
        program.push_str(&format!(
            "fn func{}(x: Int) -> Int {{ x + {} }}\n",
            i, i
        ));
    }
    program
}

pub fn generate_nested_expression(depth: usize) -> String {
    let mut expr = "1".to_string();
    for _ in 0..depth {
        expr = format!("({} + 1)", expr);
    }
    expr
}

pub fn generate_list_literal(size: usize) -> String {
    let elements: Vec<String> = (0..size).map(|i| i.to_string()).collect();
    format!("[{}]", elements.join(", "))
}

// ============================================================================
// Expected Results
// ============================================================================

pub struct ExpectedResult {
    pub should_compile: bool,
    pub should_type_check: bool,
    pub expected_output: Option<String>,
    pub expected_type: Option<String>,
}

pub fn get_expected_result(fixture_name: &str) -> ExpectedResult {
    match fixture_name {
        "simple_arithmetic" => ExpectedResult {
            should_compile: true,
            should_type_check: true,
            expected_output: Some("14".to_string()),
            expected_type: Some("Int".to_string()),
        },
        "factorial" => ExpectedResult {
            should_compile: true,
            should_type_check: true,
            expected_output: None,
            expected_type: Some("Int -> Int".to_string()),
        },
        _ => ExpectedResult {
            should_compile: true,
            should_type_check: true,
            expected_output: None,
            expected_type: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fixtures_are_valid() {
        // Verify all fixture strings are non-empty
        assert!(!SIMPLE_ARITHMETIC.is_empty());
        assert!(!SIMPLE_FUNCTION.is_empty());
        assert!(!FACTORIAL.is_empty());
        assert!(!FIBONACCI.is_empty());
    }

    #[test]
    fn test_generators() {
        let program = generate_function_program(5);
        assert!(program.contains("func0"));
        assert!(program.contains("func4"));

        let expr = generate_nested_expression(3);
        assert!(expr.starts_with("(((1"));

        let list = generate_list_literal(5);
        assert_eq!(list, "[0, 1, 2, 3, 4]");
    }
}
