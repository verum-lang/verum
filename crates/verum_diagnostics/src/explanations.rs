//! Error explanation system for Verum compiler errors.
//!
//! Provides detailed explanations, examples, and solutions for error codes.
//! Accessible via `verum --explain E0312` command.

use crate::colors::{Color, ColorScheme};
use once_cell::sync::Lazy;
use verum_common::{List, Map, Text};

/// Detailed explanation for an error code
#[derive(Debug, Clone)]
pub struct ErrorExplanation {
    /// Error code (e.g., "E0312")
    pub code: Text,
    /// Short title
    pub title: Text,
    /// Detailed description
    pub description: Text,
    /// Example scenarios
    pub examples: List<Example>,
    /// Suggested solutions
    pub solutions: List<Solution>,
    /// Related documentation links
    pub see_also: List<Text>,
}

/// An example that demonstrates when an error occurs
#[derive(Debug, Clone)]
pub struct Example {
    /// Description of the scenario
    pub description: Text,
    /// Example code that triggers the error
    pub code: Text,
    /// Optional: the correct version
    pub correct: Option<Text>,
}

/// A suggested solution for fixing an error
#[derive(Debug, Clone)]
pub struct Solution {
    /// Solution title
    pub title: Text,
    /// Detailed explanation
    pub description: Text,
    /// Optional code example
    pub code: Option<Text>,
}

/// Global registry of error explanations
pub static ERROR_EXPLANATIONS: Lazy<Map<Text, ErrorExplanation>> = Lazy::new(|| {
    let mut map = Map::new();

    // E0312: Refinement constraint not satisfied
    map.insert(
        "E0312".into(),
        ErrorExplanation {
            code: "E0312".into(),
            title: "Refinement constraint not satisfied".into(),
            description: r#"A value does not satisfy the refinement predicate required by its type.

Refinement types in Verum allow you to express constraints on values using
logical predicates. The compiler uses SMT solvers to verify these constraints
at compile-time whenever possible.

This error occurs when:
1. A literal value obviously violates the constraint (e.g., -5 for Positive)
2. The SMT solver proves a value cannot satisfy the constraint
3. A function argument doesn't meet the required refinement"#
                .into(),
            examples: vec![
                Example {
                    description: "Negative value for Positive type".into(),
                    code: r#"type Positive is Int{> 0};

fn example() {
    let x: Positive = -5;  // Error: -5 does not satisfy > 0
}"#
                    .into(),
                    correct: Some(
                        r#"type Positive is Int{> 0};

fn example() {
    let x: Positive = 5;  // OK: 5 > 0
}"#
                        .into(),
                    ),
                },
                Example {
                    description: "Out-of-bounds array index".into(),
                    code: r#"fn access(arr: List<Int>, idx: usize{< arr.len()}) -> Int {
    arr[idx]
}

fn caller() {
    let list = [1, 2, 3];
    access(list, 5);  // Error: 5 >= 3 (array length)
}"#
                    .into(),
                    correct: Some(
                        r#"fn caller() {
    let list = [1, 2, 3];
    access(list, 2);  // OK: 2 < 3
}"#
                        .into(),
                    ),
                },
                Example {
                    description: "Division by zero prevention".into(),
                    code: r#"type NonZero is Int{!= 0};

fn divide(a: Int, b: NonZero) -> Int {
    a / b
}

fn caller() {
    divide(10, 0);  // Error: 0 violates != 0
}"#
                    .into(),
                    correct: None,
                },
            ]
            .into(),
            solutions: vec![
                Solution {
                    title: "Use runtime validation with try_from".into(),
                    description: "Convert the value using try_from() which returns a Result".into(),
                    code: Some(
                        r#"let x = Positive.try_from(-5)?;  // Returns Result<Positive, Error>
// Handle the error case appropriately"#
                            .into(),
                    ),
                },
                Solution {
                    title: "Use a checked constructor".into(),
                    description: "Use a constructor that validates at runtime".into(),
                    code: Some(
                        r#"let x = Positive.new(-5).expect("Must be positive");
// Or with proper error handling:
let x = Positive.new(-5).unwrap_or(Positive.new(1).unwrap());"#
                            .into(),
                    ),
                },
                Solution {
                    title: "Fix the value to satisfy the constraint".into(),
                    description: "Ensure the value meets the refinement predicate".into(),
                    code: Some(r#"let x: Positive = 5;  // OK: 5 > 0"#.into()),
                },
                Solution {
                    title: "Relax the refinement type".into(),
                    description:
                        "If the constraint is too strict, consider using a less restrictive type"
                            .into(),
                    code: Some(
                        r#"// Instead of Positive (> 0), use NonNegative (>= 0)
type NonNegative is Int{>= 0};
let x: NonNegative = 0;  // OK"#
                            .into(),
                    ),
                },
            ]
            .into(),
            see_also: vec![
                "Refinement types constrain values at compile-time via predicates (e.g., Int{> 0}); the SMT solver (Z3) verifies constraints are satisfiable".into(),
                "CBGR (Counting-Based Generational References) provides memory safety with ~15ns overhead per reference check".into(),
                "Runtime validation: use @verify for compile-time proofs or explicit if/match guards for runtime checks".into(),
            ]
            .into(),
        },
    );

    // E0306: Capability violation
    map.insert(
        "E0306".into(),
        ErrorExplanation {
            code: "E0306".into(),
            title: "Capability violation".into(),
            description: r#"Function uses a capability not declared in its using clause.

Capability attenuation is a security mechanism that restricts what capabilities
a function can use. Each function must explicitly declare which capabilities it
needs in its 'using' clause. This error occurs when:
- A function attempts to use a capability it didn't declare
- The using clause is missing entirely
- The wrong sub-context is specified"#
                .into(),
            examples: vec![Example {
                description: "Using Execute capability when only Query is declared".into(),
                code: r#"fn attempt_delete(id: Int) -> Result<()>
    using [Database::Query]
{
    db.execute(f"DELETE FROM users WHERE id = {id}")?;
    // Error: Database::Execute not in using clause
}"#
                .into(),
                correct: Some(
                    r#"fn attempt_delete(id: Int) -> Result<()>
    using [Database::Execute]  // Declare the correct capability
{
    db.execute(f"DELETE FROM users WHERE id = {id}")?;
}"#
                    .into(),
                ),
            }].into(),
            solutions: vec![
                Solution {
                    title: "Add the capability to the using clause".into(),
                    description: "Declare the capability your function needs".into(),
                    code: Some(
                        r#"fn my_function() -> Result<()>
    using [Database::Execute]  // Add missing capability
{
    // Now you can use Database::Execute
}"#
                        .into(),
                    ),
                },
                Solution {
                    title: "Use a different approach that requires fewer capabilities".into(),
                    description: "Refactor to use only the capabilities you have".into(),
                    code: Some(
                        r#"// Instead of modifying the database directly,
// return the data and let the caller handle modifications
fn get_user_data(id: Int) -> Result<UserData>
    using [Database::Query]  // Only needs Query
{
    db.query(f"SELECT * FROM users WHERE id = {id}")
}"#
                        .into(),
                    ),
                },
            ].into(),
            see_also: vec![
                "Context system: functions declare dependencies via 'using [Context]'; capability attenuation restricts sub-context access for least-privilege security".into(),
                "Capability attenuation: sub-contexts (e.g., Database::Query vs Database::Execute) enable fine-grained capability control; functions can only use declared sub-contexts".into(),
            ].into(),
        },
    );

    // E0307: Sub-context not found
    map.insert(
        "E0307".into(),
        ErrorExplanation {
            code: "E0307".into(),
            title: "Sub-context not found".into(),
            description: r#"Referenced a sub-context that doesn't exist in the context hierarchy.

Contexts can be subdivided into sub-contexts for fine-grained capability control.
This error occurs when you reference a sub-context name that the parent context
doesn't define."#
                .into(),
            examples: vec![Example {
                description: "Referencing a non-existent sub-context".into(),
                code: r#"using [Database.Read]
// Error: Database only defines Query and Execute"#
                    .into(),
                correct: Some(r#"using [Database::Query]  // Use a valid sub-context"#.into()),
            }].into(),
            solutions: vec![
                Solution {
                    title: "Use a valid sub-context name".into(),
                    description: "Check the context definition for available sub-contexts".into(),
                    code: Some(
                        r#"// Example context definition:
context Database {
    Query    // Available
    Execute  // Available
    // Read is not defined
}

// Use the correct sub-context:
using [Database::Query]"#
                            .into(),
                    ),
                },
                Solution {
                    title: "Define the sub-context if needed".into(),
                    description: "Add the sub-context to the context definition".into(),
                    code: Some(
                        r#"context Database {
    Query
    Execute
    Read     // Add new sub-context
}"#
                        .into(),
                    ),
                },
            ].into(),
            see_also: vec!["Context system: sub-contexts subdivide contexts for fine-grained capability control; each sub-context name must match a defined entry in the parent context's definition".into()].into(),
        },
    );

    // E0308: Capability not provided
    map.insert(
        "E0308".into(),
        ErrorExplanation {
            code: "E0308".into(),
            title: "Capability not provided".into(),
            description: r#"A required capability was not provided in the environment.

While a function may declare the capabilities it needs in its 'using' clause,
those capabilities must be provided before the function is called. This error
occurs when calling a function that requires a capability that hasn't been
installed via a 'provide' statement."#
                .into(),
            examples: vec![Example {
                description: "Calling function without providing required capability".into(),
                code: r#"fn main() {
    let data = read_file("data.txt")?;
    // Error: FileSystem::Read required but not provided
}"#
                .into(),
                correct: Some(
                    r#"fn main() {
    provide FileSystem = RealFileSystem.new();
    let data = read_file("data.txt")?;
}"#
                    .into(),
                ),
            }].into(),
            solutions: vec![
                Solution {
                    title: "Provide the capability before calling".into(),
                    description: "Install the required context provider".into(),
                    code: Some(
                        r#"provide FileSystem = RealFileSystem.new();
// Now functions requiring FileSystem can be called"#
                            .into(),
                    ),
                },
                Solution {
                    title: "Pass the capability as a parameter".into(),
                    description: "Instead of using the global context, pass it explicitly".into(),
                    code: Some(
                        r#"fn my_function(fs: &FileSystem) -> Result<Data> {
    // Use fs parameter instead of global context
}"#
                        .into(),
                    ),
                },
            ].into(),
            see_also: vec![
                "Context system: all context dependencies must be provided before use via 'provide ContextName = ProviderImpl.new()'".into(),
                "Provider installation: providers implement context protocols and are resolved at runtime; use 'provide' to install before calling dependent functions".into(),
            ].into(),
        },
    );

    // E0309: Partial implementation warning
    map.insert(
        "E0309".into(),
        ErrorExplanation {
            code: "E0309".into(),
            title: "Partial implementation of context".into(),
            description: r#"A context implementation only provides some sub-contexts, not all.

This is a warning, not an error, because partial implementations are allowed.
However, they should be clearly documented so users know which capabilities
are available and which are not."#
                .into(),
            examples: vec![Example {
                description: "Read-only filesystem implementation".into(),
                code: r#"implement FileSystem for ReadOnlyFS {
    // Only implements Read, not Write or Admin
    // Warning: partial implementation
}"#
                .into(),
                correct: Some(
                    r#"/// ReadOnlyFS provides read-only access to the filesystem.
/// Note: Does not implement Write or Admin sub-contexts.
implement FileSystem for ReadOnlyFS {
    // Implemented: Read
    // Not implemented: Write, Admin (documented above)
}"#
                    .into(),
                ),
            }].into(),
            solutions: vec![
                Solution {
                    title: "Document the partial implementation".into(),
                    description: "Add clear documentation about which capabilities are missing"
                        .into(),
                    code: Some(
                        r#"/// ReadOnlyFS provides limited filesystem access.
/// Implements: Read
/// Does not implement: Write, Admin
implement FileSystem for ReadOnlyFS { ... }"#
                            .into(),
                    ),
                },
                Solution {
                    title: "Implement all sub-contexts".into(),
                    description: "Complete the implementation if all capabilities are needed"
                        .into(),
                    code: Some(
                        r#"implement FileSystem for CompleteFS {
    // Implement all sub-contexts: Read, Write, Admin
}"#
                        .into(),
                    ),
                },
            ].into(),
            see_also: vec![
                "Context system: providers can implement a subset of sub-contexts (partial implementation); the capability subset rule allows Provider<C.S1> without requiring all sub-contexts".into(),
                "Partial implementations: document which sub-contexts are missing; callers using unimplemented sub-contexts will get E0308 at runtime".into(),
            ].into(),
        },
    );

    // E0310: Unsafe array access
    map.insert(
        "E0310".into(),
        ErrorExplanation {
            code: "E0310".into(),
            title: "Unsafe array access".into(),
            description: r#"Array index is not proven to be within bounds.

Verum requires array accesses to be provably safe at compile-time, or checked
at runtime. This prevents out-of-bounds access errors.

The compiler uses SMT solving to verify that indices are within bounds based on:
- Literal values
- Refinement types on indices
- Control flow analysis"#
                .into(),
            examples: vec![
                Example {
                    description: "Unchecked index".into(),
                    code: r#"fn get_item(arr: List<Int>, idx: usize) -> Int {
    arr[idx]  // Error: idx might be out of bounds
}"#
                    .into(),
                    correct: Some(
                        r#"fn get_item(arr: List<Int>, idx: usize{< arr.len()}) -> Int {
    arr[idx]  // OK: refinement ensures idx < arr.len()
}"#
                        .into(),
                    ),
                },
                Example {
                    description: "Dynamic index without checks".into(),
                    code: r#"let arr = [1, 2, 3];
let idx = get_user_input();  // Could be anything
let value = arr[idx];  // Error!"#
                        .into(),
                    correct: Some(
                        r#"let arr = [1, 2, 3];
let idx = get_user_input();
let value = arr.get(idx).unwrap_or(&0);  // Safe access with fallback"#
                            .into(),
                    ),
                },
            ].into(),
            solutions: vec![
                Solution {
                    title: "Use refinement types on indices".into(),
                    description: "Add a refinement constraint to the index parameter".into(),
                    code: Some(
                        r#"fn access(arr: List<T>, idx: usize{< arr.len()}) -> T {
    arr[idx]  // Proven safe
}"#
                        .into(),
                    ),
                },
                Solution {
                    title: "Use safe accessor methods".into(),
                    description: "Use get() which returns Option<T>".into(),
                    code: Some(
                        r#"let value = arr.get(idx).unwrap_or(&default_value);
// Or with error handling:
let value = arr.get(idx).ok_or("Index out of bounds")?;"#
                            .into(),
                    ),
                },
                Solution {
                    title: "Add runtime bounds check".into(),
                    description: "Explicitly check bounds before access".into(),
                    code: Some(
                        r#"if idx < arr.len() {
    let value = arr[idx];  // Proven safe within this branch
    // ...
}"#
                        .into(),
                    ),
                },
            ].into(),
            see_also: vec![
                "Array safety: Verum requires compile-time proof or runtime check that indices are within bounds; use refinement types, .get(), or explicit bounds checks".into(),
                "Refinement types: constrain index parameters with predicates like idx: usize{< arr.len()} to prove bounds at compile-time via SMT".into(),
            ].into(),
        },
    );

    // E0301: Context not declared
    map.insert(
        "E0301".into(),
        ErrorExplanation {
            code: "E0301".into(),
            title: "Context used but not declared".into(),
            description:
                r#"A context is used within a function but not declared in the function signature.

Verum's context system requires explicit declaration of all dependencies.
This ensures:
- Clear dependency tracking
- No hidden global state
- Testability and modularity"#
                    .into(),
            examples: vec![Example {
                description: "Using undeclared context".into(),
                code: r#"fn process_user(id: Int) -> User {
    let db = use Database;  // Error: Database not declared
    db.get_user(id)
}"#
                .into(),
                correct: Some(
                    r#"fn process_user(id: Int) using [Database] -> User {
    let db = use Database;  // OK: declared in signature
    db.get_user(id)
}"#
                    .into(),
                ),
            }].into(),
            solutions: vec![
                Solution {
                    title: "Declare the context in function signature".into(),
                    description: "Add 'using [Context]' to the function signature".into(),
                    code: Some(
                        r#"fn my_function() using [Database, Logger] -> Result {
    // Now can use Database and Logger
}"#
                        .into(),
                    ),
                },
                Solution {
                    title: "Pass the dependency as a parameter".into(),
                    description: "If you don't want to use contexts, pass explicitly".into(),
                    code: Some(
                        r#"fn my_function(db: &Database) -> Result {
    // Use db parameter
}"#
                        .into(),
                    ),
                },
            ].into(),
            see_also: vec![
                "Context system: all context dependencies must be explicitly declared via 'using [ContextName]' in function signatures for clear dependency tracking and testability".into(),
                "Dependency injection: Verum's context system provides runtime DI (~5-30ns lookup); alternatives include explicit parameter passing for simpler cases".into(),
            ].into(),
        },
    );

    // E0313: Integer overflow
    map.insert(
        "E0313".into(),
        ErrorExplanation {
            code: "E0313".into(),
            title: "Integer overflow detected".into(),
            description: r#"An arithmetic operation may overflow the integer bounds.

Verum tracks integer ranges through refinement types and warns about potential
overflows at compile-time. This prevents undefined behavior and security issues."#
                .into(),
            examples: vec![Example {
                description: "Potential overflow in addition".into(),
                code: r#"fn add_large(x: Int{> 1000000000}) -> Int {
    x + x  // Error: may overflow Int32
}"#
                .into(),
                correct: Some(
                    r#"fn add_large(x: Int{> 1000000000}) -> Int64 {
    (x as Int64) + (x as Int64)  // OK: use larger type
}"#
                    .into(),
                ),
            }].into(),
            solutions: vec![
                Solution {
                    title: "Use a larger integer type".into(),
                    description: "Switch to Int64 or Int128".into(),
                    code: Some(r#"let result: Int64 = (x as Int64) + (y as Int64);"#.into()),
                },
                Solution {
                    title: "Use checked arithmetic".into(),
                    description: "Use checked_add() which returns Option".into(),
                    code: Some(r#"let result = x.checked_add(y).ok_or("Overflow")?;"#.into()),
                },
                Solution {
                    title: "Add refinement constraints".into(),
                    description: "Narrow the input range to prevent overflow".into(),
                    code: Some(
                        r#"fn add_safe(x: Int{< 1000}, y: Int{< 1000}) -> Int {
    x + y  // Proven safe: max value is 1999
}"#
                        .into(),
                    ),
                },
            ].into(),
            see_also: vec![
                "Integer types: Verum supports Int (default), Int8/16/32/64/128, UInt variants; use wider types to prevent overflow".into(),
                "Arithmetic safety: Verum detects potential overflows at compile-time via refinement type range tracking; use checked_add/checked_mul for runtime safety".into(),
            ].into(),
        },
    );

    // E0314: Division by zero
    map.insert(
        "E0314".into(),
        ErrorExplanation {
            code: "E0314".into(),
            title: "Division by zero".into(),
            description: r#"A division or modulo operation may divide by zero.

This error occurs when the compiler cannot prove that the divisor is non-zero."#
                .into(),
            examples: vec![Example {
                description: "Unchecked division".into(),
                code: r#"fn divide(a: Int, b: Int) -> Int {
    a / b  // Error: b might be 0
}"#
                .into(),
                correct: Some(
                    r#"type NonZero is Int{!= 0};

fn divide(a: Int, b: NonZero) -> Int {
    a / b  // OK: b is proven non-zero
}"#
                    .into(),
                ),
            }].into(),
            solutions: vec![
                Solution {
                    title: "Use NonZero refinement type".into(),
                    description: "Require divisor to be non-zero via refinement".into(),
                    code: Some(
                        r#"type NonZero is Int{!= 0};
fn divide(a: Int, b: NonZero) -> Int { a / b }"#
                            .into(),
                    ),
                },
                Solution {
                    title: "Add runtime check".into(),
                    description: "Check for zero before division".into(),
                    code: Some(
                        r#"if b != 0 {
    let result = a / b;  // Safe
} else {
    // Handle division by zero
}"#
                        .into(),
                    ),
                },
                Solution {
                    title: "Use checked division".into(),
                    description: "Use checked_div() which returns Option".into(),
                    code: Some(
                        r#"let result = a.checked_div(b).ok_or("Division by zero")?;"#.into(),
                    ),
                },
            ].into(),
            see_also: vec!["Arithmetic safety: division by zero is prevented via refinement types (NonZero = Int{!= 0}), runtime checks, or checked_div() returning Maybe<Int>".into()].into(),
        },
    );

    // E0317: Unused Result that must be handled
    map.insert(
        "E0317".into(),
        ErrorExplanation {
            code: "E0317".into(),
            title: "Unused Result that must be handled".into(),
            description:
                r#"A function marked with @must_handle returned a Result that was not used.

Functions annotated with @must_handle require their return values to be
explicitly handled. This prevents accidentally ignoring errors."#
                    .into(),
            examples: vec![Example {
                description: "Ignoring a Result that must be handled".into(),
                code: r#"@must_handle
fn save_file(path: Text, data: Text) -> Result<(), Error> {
    // ...
}

fn caller() {
    save_file("file.txt", "data");  // Error: Result not handled
}"#
                .into(),
                correct: Some(
                    r#"fn caller() -> Result<(), Error> {
    save_file("file.txt", "data")?;  // OK: propagate error
    Ok(())
}"#
                    .into(),
                ),
            }].into(),
            solutions: vec![
                Solution {
                    title: "Use the ? operator to propagate".into(),
                    description: "Propagate the error to the caller".into(),
                    code: Some(r#"save_file("file.txt", "data")?;"#.into()),
                },
                Solution {
                    title: "Handle the Result explicitly".into(),
                    description: "Use match or if-let to handle success and error cases".into(),
                    code: Some(
                        r#"match save_file("file.txt", "data") {
    Ok(()) => println!("Saved"),
    Err(e) => eprintln!("Error: {}", e),
}"#
                        .into(),
                    ),
                },
                Solution {
                    title: "Store the Result for later handling".into(),
                    description: "Assign to a variable if you'll handle it later".into(),
                    code: Some(
                        r#"let result = save_file("file.txt", "data");
// ... later ...
result?;"#
                            .into(),
                    ),
                },
            ].into(),
            see_also: vec![
                "@must_handle annotation: applied to error TYPES (not functions); all Result<T, MarkedType> values must be handled via ?, unwrap, expect, match, or if-let before drop".into(),
                "Error handling: Verum enforces explicit error handling; @must_handle is a compile ERROR (stronger than @must_use warning); allowed operations: ?, unwrap(), expect(), match, if-let, .is_err() check".into(),
            ].into(),
        },
    );

    // E0203: Result type mismatch in try operator
    map.insert(
        "E0203".into(),
        ErrorExplanation {
            code: "E0203".into(),
            title: "Result type mismatch in '?' operator".into(),
            description:
                r#"The error type in a Result doesn't match the function's return error type.

The '?' operator can only be used when the error types are compatible.
This requires either:
1. The same error type
2. A From impl to convert between error types"#
                    .into(),
            examples: vec![Example {
                description: "Incompatible error types".into(),
                code: r#"fn process() -> Result<Int, IoError> {
    let value = parse_int()?;  // Error: ParseError != IoError
    Ok(value)
}"#
                .into(),
                correct: Some(
                    r#"fn process() -> Result<Int, Error> {
    let value = parse_int()?;  // OK: both convert to Error
    Ok(value)
}"#
                    .into(),
                ),
            }].into(),
            solutions: vec![
                Solution {
                    title: "Use a common error type".into(),
                    description: "Define an enum that wraps both error types".into(),
                    code: Some(
                        r#"enum MyError {
    Io(IoError),
    Parse(ParseError),
}

impl From<IoError> for MyError { ... }
impl From<ParseError> for MyError { ... }"#
                            .into(),
                    ),
                },
                Solution {
                    title: "Use map_err to convert".into(),
                    description: "Explicitly convert the error type".into(),
                    code: Some(r#"let value = parse_int().map_err(|e| IoError.from(e))?;"#.into()),
                },
            ].into(),
            see_also: vec![
                "Try operator: '?' desugars to match expr { Ok(v) => v, Err(e) => return Err(e.into()) }; requires From<SourceError> for TargetError".into(),
                "Error conversion: implement From<ErrorA> for ErrorB to enable automatic conversion; use .map_err() for explicit conversion when From is not available".into(),
            ].into(),
        },
    );

    // E0309: Branch verification failure
    map.insert("E0309".into(), ErrorExplanation {
        code: "E0309".into(),
        title: "Branch verification failed".into(),
        description: r#"The compiler could not verify that all branches satisfy the required postcondition.

This typically occurs when:
- Different branches produce incompatible refinements
- Not all code paths return a value
- A refinement constraint is violated in some branch"#.into(),
        examples: vec![
            Example {
                description: "Inconsistent refinements across branches".into(),
                code: r#"fn get_positive(flag: Bool) -> Int{> 0} {
    if flag {
        5      // OK: 5 > 0
    } else {
        -1     // Error: -1 not > 0
    }
}"#.into(),
                correct: Some(r#"fn get_positive(flag: Bool) -> Int{> 0} {
    if flag {
        5      // OK: 5 > 0
    } else {
        1      // OK: 1 > 0
    }
}"#.into()),
            },
        ].into(),
        solutions: vec![
            Solution {
                title: "Ensure all branches satisfy the constraint".into(),
                description: "Check that each branch produces a valid value".into(),
                code: Some(r#"// Each branch must satisfy the refinement
fn get_positive(flag: Bool) -> Int{> 0} {
    if flag { 5 } else { 1 }
}"#.into()),
            },
            Solution {
                title: "Use runtime validation".into(),
                description: "Validate the result at runtime".into(),
                code: Some(r#"fn get_value(flag: Bool) -> Result<Positive, Error> {
    let raw = if flag { 5 } else { -1 };
    Positive.try_from(raw)
}"#.into()),
            },
        ].into(),
        see_also: vec![
            "Branch verification: all branches in a function must satisfy the declared postcondition (ensures clause); the SMT solver checks each path independently".into(),
        ].into(),
    });

    // E0316: Resource already consumed
    map.insert(
        "E0316".into(),
        ErrorExplanation {
            code: "E0316".into(),
            title: "Resource already consumed".into(),
            description: r#"Attempted to use a linear resource that was already consumed.

Linear types (resources) can only be used once. This prevents:
- Use-after-free errors
- Double-close errors
- Resource leaks"#
                .into(),
            examples: vec![Example {
                description: "Using a file handle twice".into(),
                code: r#"fn process() {
    let file = File::open("data.txt")?;
    file.close();  // Consumes file
    file.read();   // Error: file already consumed
}"#
                .into(),
                correct: Some(
                    r#"fn process() {
    let file = File::open("data.txt")?;
    file.read();   // Use file
    file.close();  // Then close
}"#
                    .into(),
                ),
            }].into(),
            solutions: vec![
                Solution {
                    title: "Reorder operations".into(),
                    description: "Use the resource before consuming it".into(),
                    code: Some(
                        r#"// Perform all operations before final consumption
resource.operation1();
resource.operation2();
resource.consume();"#
                            .into(),
                    ),
                },
                Solution {
                    title: "Use borrowing instead".into(),
                    description: "Borrow the resource rather than consuming".into(),
                    code: Some(
                        r#"fn use_resource(resource: &Resource) {
    // Can use multiple times via borrow
}"#
                        .into(),
                    ),
                },
            ].into(),
            see_also: vec![
                "Linear types: resources marked as linear can be used at most once; prevents use-after-free, double-close, and resource leaks at compile-time".into(),
                "Resource management: use borrowing (&T) for non-consuming access; linear resources are automatically freed when they go out of scope".into(),
            ].into(),
        },
    );

    // E0101: Use-after-free error
    map.insert(
        "E0101".into(),
        ErrorExplanation {
            code: "E0101".into(),
            title: "Use-after-free detected".into(),
            description: r#"An attempt was made to access memory that has been freed.

Verum's CBGR (Counting-Based Generational References) system detected a use-after-free
by comparing the generation counter in the reference with the generation counter in
the allocation header. When these don't match, it indicates the memory was freed
and potentially reallocated for something else.

This is a critical memory safety error that can lead to:
- Reading garbage data
- Corrupting unrelated memory
- Security vulnerabilities
- Undefined behavior"#
                .into(),
            examples: vec![Example {
                description: "Accessing freed resource".into(),
                code: r#"fn example() {
    let ptr = Box.new(42);
    drop(ptr);       // Frees the memory
    let x = *ptr;    // Error: use-after-free!
}"#
                .into(),
                correct: Some(
                    r#"fn example() {
    let ptr = Box.new(42);
    let x = *ptr;    // Access before freeing
    drop(ptr);       // Free after done using
}"#
                    .into(),
                ),
            }].into(),
            solutions: vec![
                Solution {
                    title: "Ensure proper lifetime management".into(),
                    description: "Access the data before it's freed".into(),
                    code: Some(
                        r#"// Access data within its lifetime
let data = create_data();
process(&data);  // Borrow, don't move
// data is still valid here"#
                            .into(),
                    ),
                },
                Solution {
                    title: "Use references instead of raw pointers".into(),
                    description: "Verum's reference system prevents use-after-free".into(),
                    code: Some(
                        r#"// Use &T references with CBGR protection
fn process(data: &Data) {
    // CBGR ensures data is still valid
}"#
                        .into(),
                    ),
                },
            ].into(),
            see_also: vec![
                "CBGR system: Counting-Based Generational References detect use-after-free by comparing generation counters in references vs allocation headers (~15ns check overhead)".into(),
                "Memory safety: Verum's three-tier reference model (&T default CBGR, &checked T zero-cost compiler-proven, &unsafe T manual proof) prevents dangling references".into(),
            ].into(),
        },
    );

    // E0102: Double-free error
    map.insert(
        "E0102".into(),
        ErrorExplanation {
            code: "E0102".into(),
            title: "Double-free detected".into(),
            description: r#"An attempt was made to free memory that has already been freed.

Double-free errors can corrupt memory allocator metadata and lead to:
- Heap corruption
- Security vulnerabilities (exploitation via double-free)
- Unpredictable program behavior"#
                .into(),
            examples: vec![Example {
                description: "Freeing a resource twice".into(),
                code: r#"fn example() {
    let ptr = Box.new(42);
    drop(ptr);    // First free - OK
    drop(ptr);    // Second free - Error!
}"#
                .into(),
                correct: Some(
                    r#"fn example() {
    let ptr = Box.new(42);
    drop(ptr);    // Free only once
}"#
                    .into(),
                ),
            }].into(),
            solutions: vec![
                Solution {
                    title: "Track ownership carefully".into(),
                    description: "Ensure each resource is owned by exactly one variable".into(),
                    code: Some(
                        r#"// Use Verum's ownership system
let owned = Resource.new();
// owned is automatically freed when it goes out of scope"#
                            .into(),
                    ),
                },
                Solution {
                    title: "Use RAII patterns".into(),
                    description: "Let Verum handle cleanup automatically".into(),
                    code: Some(
                        r#"fn example() {
    let resource = Resource.new();
    // Use resource
}  // Automatically freed here - exactly once"#
                            .into(),
                    ),
                },
            ].into(),
            see_also: vec![
                "CBGR system: generational references detect double-free by checking allocation state; ThinRef is 16 bytes (ptr + generation + epoch_caps), FatRef is 24 bytes".into(),
                "Ownership: Verum uses single-owner semantics; each resource has exactly one owner; use RAII patterns for automatic cleanup at scope exit".into(),
            ].into(),
        },
    );

    // E0103: Null pointer dereference
    map.insert(
        "E0103".into(),
        ErrorExplanation {
            code: "E0103".into(),
            title: "Null pointer dereference".into(),
            description: r#"An attempt was made to dereference a null or unallocated pointer.

This error occurs when trying to access memory through a reference that points
to no valid data. This is typically caught by Verum's CBGR system which tracks
allocation state."#
                .into(),
            examples: vec![Example {
                description: "Dereferencing null".into(),
                code: r#"fn example() {
    let ptr: &Int = null;  // Not valid in Verum!
    let x = *ptr;          // Null pointer dereference
}"#
                .into(),
                correct: Some(
                    r#"fn example() {
    let value: Maybe<Int> = None;
    match value {
        Some(x) => println("{}", x),
        None => println("No value"),
    }
}"#
                    .into(),
                ),
            }].into(),
            solutions: vec![Solution {
                title: "Use Maybe<T> for optional values".into(),
                description: "Verum uses Maybe instead of null".into(),
                code: Some(
                    r#"let optional: Maybe<Int> = Some(42);
if let Some(value) = optional {
    // value is guaranteed non-null here
}"#
                    .into(),
                ),
            }].into(),
            see_also: vec!["Maybe type: Verum uses Maybe<T> (variants: Some(T) | None) instead of null pointers; pattern match with 'match' or 'if let Some(v)' to safely access values".into()].into(),
        },
    );

    // E0201: Type inference failed
    map.insert(
        "E0201".into(),
        ErrorExplanation {
            code: "E0201".into(),
            title: "Type inference failed".into(),
            description: r#"The compiler could not infer the type of an expression.

Verum uses Hindley-Milner type inference, but sometimes explicit type annotations
are needed when the compiler cannot determine the type from context."#
                .into(),
            examples: vec![Example {
                description: "Ambiguous type".into(),
                code: r#"let x = [];  // What type is the list?"#.into(),
                correct: Some(r#"let x: List<Int> = [];  // Now the type is clear"#.into()),
            }].into(),
            solutions: vec![
                Solution {
                    title: "Add explicit type annotation".into(),
                    description: "Specify the type explicitly".into(),
                    code: Some(r#"let x: List<Int> = [];"#.into()),
                },
                Solution {
                    title: "Provide more context".into(),
                    description: "Use the value in a way that reveals its type".into(),
                    code: Some(
                        r#"let x = [];
x.push(42);  // Now compiler knows it's List<Int>"#
                            .into(),
                    ),
                },
            ].into(),
            see_also: vec!["Type system: Verum uses Hindley-Milner type inference; add explicit annotations (let x: Type = ...) when the compiler cannot infer from context".into()].into(),
        },
    );

    // E0202: Method not found
    map.insert(
        "E0202".into(),
        ErrorExplanation {
            code: "E0202".into(),
            title: "Method not found".into(),
            description: r#"The type does not have the requested method.

This can occur because:
- The method name is misspelled
- The type doesn't implement the required protocol
- The method exists but with different visibility"#
                .into(),
            examples: vec![Example {
                description: "Calling undefined method".into(),
                code: r#"let x: Int = 42;
x.reverse();  // Int doesn't have reverse"#
                    .into(),
                correct: Some(
                    r#"let x: Text = "hello";
x.reverse();  // Text has reverse"#
                        .into(),
                ),
            }].into(),
            solutions: vec![
                Solution {
                    title: "Check method spelling".into(),
                    description: "Ensure the method name is correct".into(),
                    code: None,
                },
                Solution {
                    title: "Implement the required protocol".into(),
                    description: "Add protocol implementation if needed".into(),
                    code: Some(
                        r#"implement Display for MyType {
    fn display(&self) -> Text { ... }
}"#
                        .into(),
                    ),
                },
            ].into(),
            see_also: vec![
                "Protocols: defined via 'type Name is protocol { ... }'; types must 'implement ProtocolName for Type' to gain methods".into(),
                "Methods: check spelling, visibility, and whether the type implements the required protocol; use 'implement Protocol for Type' to add methods".into(),
            ].into(),
        },
    );

    // E0302: Context not declared
    map.insert(
        "E0302".into(),
        ErrorExplanation {
            code: "E0302".into(),
            title: "Context dependency not declared".into(),
            description: r#"A function uses a context that wasn't declared in its signature.

Verum's context system requires explicit declaration of all context dependencies
in the function signature using the 'using' clause. This ensures:
- Clear dependency tracking
- No hidden global state
- Testability and modularity"#
                .into(),
            examples: vec![Example {
                description: "Using undeclared context".into(),
                code: r#"fn query_user(id: Int) -> User {
    let db = use Database;  // Error: Database not declared
    db.get_user(id)
}"#
                .into(),
                correct: Some(
                    r#"fn query_user(id: Int) using [Database] -> User {
    let db = use Database;  // OK: declared in signature
    db.get_user(id)
}"#
                    .into(),
                ),
            }].into(),
            solutions: vec![Solution {
                title: "Add context to function signature".into(),
                description: "Declare the context dependency".into(),
                code: Some(
                    r#"fn my_function() using [Database, Logger] -> Result<()> {
    // Now can use Database and Logger contexts
}"#
                    .into(),
                ),
            }].into(),
            see_also: vec![
                "Context system: all context dependencies must be declared via 'using [ContextName]' in the function signature; undeclared usage is E0302".into(),
                "Dependency injection: use 'using' clause for context DI or pass dependencies explicitly as parameters for simpler cases".into(),
            ].into(),
        },
    );

    // E0303: Context not provided
    map.insert(
        "E0303".into(),
        ErrorExplanation {
            code: "E0303".into(),
            title: "Required context not provided".into(),
            description: r#"A function requires a context that hasn't been provided.

Before calling a function that uses a context, you must provide that context
using the 'provide' statement."#
                .into(),
            examples: vec![Example {
                description: "Missing context provider".into(),
                code: r#"fn main() {
    let user = get_user(42)?;  // Error: Database not provided
}"#
                .into(),
                correct: Some(
                    r#"fn main() {
    provide Database = PostgresDB.connect("postgres://...")?;
    let user = get_user(42)?;  // OK: Database is provided
}"#
                    .into(),
                ),
            }].into(),
            solutions: vec![Solution {
                title: "Provide the required context".into(),
                description: "Use 'provide' before calling the function".into(),
                code: Some(
                    r#"provide Database = PostgresDB.connect(connection_string)?;
// Now functions using [Database] can be called"#
                        .into(),
                ),
            }].into(),
            see_also: vec!["Context system: use 'provide ContextName = ProviderImpl.new()' to install a context provider before calling functions that require it".into()].into(),
        },
    );

    // E0304: Circular context dependency
    map.insert(
        "E0304".into(),
        ErrorExplanation {
            code: "E0304".into(),
            title: "Circular context dependency".into(),
            description: r#"A circular dependency was detected in the context graph.

Contexts cannot have circular dependencies because they must be initialized
in order. If A depends on B and B depends on A, neither can be initialized first."#
                .into(),
            examples: vec![Example {
                description: "Circular dependency".into(),
                code: r#"context A using [B] { ... }
context B using [A] { ... }  // Circular!"#
                    .into(),
                correct: Some(
                    r#"context A using [C] { ... }
context B using [C] { ... }  // Both depend on C, no cycle"#
                        .into(),
                ),
            }].into(),
            solutions: vec![Solution {
                title: "Break the cycle".into(),
                description: "Refactor to remove circular dependency".into(),
                code: Some(
                    r#"// Extract shared functionality to a third context
context Shared { ... }
context A using [Shared] { ... }
context B using [Shared] { ... }"#
                        .into(),
                ),
            }].into(),
            see_also: vec![
                "Context system: contexts must form a DAG (directed acyclic graph); circular dependencies are detected at compile-time and must be broken by extracting shared functionality".into(),
                "Dependency graphs: refactor circular dependencies by introducing a shared base context that both sides depend on".into(),
            ].into(),
        },
    );

    // E0305: Affine type used multiple times
    map.insert(
        "E0305".into(),
        ErrorExplanation {
            code: "E0305".into(),
            title: "Affine type used multiple times".into(),
            description: r#"A value with an affine (linear) type was used more than once.

Affine types ensure resources are used at most once, preventing:
- Use-after-free errors
- Double-close errors
- Resource leaks

Once an affine value is used (moved), it cannot be used again."#
                .into(),
            examples: vec![Example {
                description: "Using resource twice".into(),
                code: r#"fn example() {
    let file = File.open("data.txt")?;
    process(file);     // file moved here
    process(file);     // Error: file already used
}"#
                .into(),
                correct: Some(
                    r#"fn example() {
    let file = File.open("data.txt")?;
    process(&file);    // Borrow instead of move
    process(&file);    // Can borrow again
}"#
                    .into(),
                ),
            }].into(),
            solutions: vec![
                Solution {
                    title: "Use borrowing instead".into(),
                    description: "Borrow the value with & instead of moving".into(),
                    code: Some(r#"process(&resource);  // Borrow, don't move"#.into()),
                },
                Solution {
                    title: "Clone if needed".into(),
                    description: "Clone the value if multiple ownership is needed".into(),
                    code: Some(
                        r#"let copy = resource.clone();
process(resource);
process(copy);"#
                            .into(),
                    ),
                },
            ].into(),
            see_also: vec![
                "Affine types: values with affine (linear) type can be used at most once; moving transfers ownership; use &T borrowing for non-consuming access".into(),
                "Ownership: single-owner semantics; clone explicitly if multiple ownership is needed; borrows (&T, &mut T) allow temporary access without consuming".into(),
            ].into(),
        },
    );

    // E0311: SMT verification timeout
    map.insert(
        "E0311".into(),
        ErrorExplanation {
            code: "E0311".into(),
            title: "SMT verification timeout".into(),
            description:
                r#"The SMT solver exceeded the time limit while trying to verify a constraint.

This can happen when:
- The constraint is very complex
- The verification involves unbounded recursion
- The predicates are too intricate for the solver

The default timeout can be adjusted in project settings."#
                    .into(),
            examples: vec![Example {
                description: "Complex constraint causing timeout".into(),
                code: r#"fn complex(n: Int{> 0 && is_prime(n) && n < 1000000}) {
    // SMT may timeout on complex predicates
}"#
                .into(),
                correct: Some(
                    r#"// Simplify the predicate
type PositiveUnderMillion is Int{> 0 && < 1000000};
fn complex(n: PositiveUnderMillion) {
    // Check primality at runtime if needed
}"#
                    .into(),
                ),
            }].into(),
            solutions: vec![
                Solution {
                    title: "Simplify the constraint".into(),
                    description: "Break complex constraints into simpler parts".into(),
                    code: None,
                },
                Solution {
                    title: "Add verification hints".into(),
                    description: "Help the SMT solver with assertions".into(),
                    code: Some(
                        r#"@verify(hint: "use_induction")
fn recursive_proof(n: Int{>= 0}) { ... }"#
                            .into(),
                    ),
                },
                Solution {
                    title: "Increase timeout".into(),
                    description: "Adjust SMT timeout in project settings".into(),
                    code: Some(
                        r#"// In Verum.toml:
[verification]
smt_timeout_ms = 60000"#
                            .into(),
                    ),
                },
            ].into(),
            see_also: vec![
                "Verification system: SMT solver (Z3) verifies refinement types and contracts; default timeout configurable via [verification] smt_timeout_ms in Verum.toml".into(),
                "SMT integration: simplify predicates, add @verify hints (e.g., use_induction), or increase timeout for complex proofs".into(),
            ].into(),
        },
    );

    // E0315: Postcondition not satisfied
    map.insert(
        "E0315".into(),
        ErrorExplanation {
            code: "E0315".into(),
            title: "Postcondition not satisfied".into(),
            description:
                r#"The function's return value does not satisfy the declared postcondition.

Postconditions (ensures clauses) declare what must be true about the function's
return value. The compiler verifies these at compile-time using SMT solving."#
                    .into(),
            examples: vec![Example {
                description: "Postcondition violation".into(),
                code: r#"fn abs(x: Int) -> Int
    ensures result >= 0
{
    x  // Error: x might be negative
}"#
                .into(),
                correct: Some(
                    r#"fn abs(x: Int) -> Int
    ensures result >= 0
{
    if x >= 0 { x } else { -x }  // OK: always non-negative
}"#
                    .into(),
                ),
            }].into(),
            solutions: vec![
                Solution {
                    title: "Fix the implementation".into(),
                    description: "Ensure all return paths satisfy the postcondition".into(),
                    code: None,
                },
                Solution {
                    title: "Weaken the postcondition".into(),
                    description: "If the postcondition is too strong, adjust it".into(),
                    code: Some(
                        r#"fn my_func(x: Int) -> Int
    ensures result == x || result == -x  // Weaker but achievable
{ ... }"#
                            .into(),
                    ),
                },
            ].into(),
            see_also: vec![
                "Verification system: postconditions (ensures clauses) are checked at compile-time via SMT; all return paths must satisfy the declared constraint".into(),
                "Contracts: 'ensures result >= 0' declares postconditions; 'requires x > 0' declares preconditions; both verified by Z3 SMT solver".into(),
            ].into(),
        },
    );

    // E0318: Precondition not established
    map.insert(
        "E0318".into(),
        ErrorExplanation {
            code: "E0318".into(),
            title: "Precondition not established".into(),
            description: r#"The caller does not establish the function's required precondition.

Preconditions (requires clauses) declare what must be true when calling a function.
The caller must prove these conditions are met."#
                .into(),
            examples: vec![Example {
                description: "Calling without precondition".into(),
                code: r#"fn sqrt(x: Float) -> Float
    requires x >= 0
{ ... }

fn caller() {
    sqrt(-1.0);  // Error: precondition not established
}"#
                .into(),
                correct: Some(
                    r#"fn caller() {
    sqrt(4.0);  // OK: 4.0 >= 0
}"#
                    .into(),
                ),
            }].into(),
            solutions: vec![
                Solution {
                    title: "Ensure precondition is met".into(),
                    description: "Pass values that satisfy the precondition".into(),
                    code: None,
                },
                Solution {
                    title: "Add runtime check".into(),
                    description: "Check the condition before calling".into(),
                    code: Some(
                        r#"if x >= 0 {
    sqrt(x)  // OK: condition proven in this branch
} else {
    handle_negative(x)
}"#
                        .into(),
                    ),
                },
            ].into(),
            see_also: vec![
                "Verification system: preconditions (requires clauses) must be proven by the caller; the SMT solver checks that arguments satisfy the constraint at the call site".into(),
                "Contracts: use 'requires condition' to declare preconditions; callers must prove the condition via refinement types or control flow guards".into(),
            ].into(),
        },
    );

    // E0401: I/O error
    map.insert(
        "E0401".into(),
        ErrorExplanation {
            code: "E0401".into(),
            title: "I/O operation failed".into(),
            description: r#"An input/output operation failed at runtime.

This is a recoverable runtime error that should be handled appropriately.
Common causes include:
- File not found
- Permission denied
- Network unavailable
- Disk full"#
                .into(),
            examples: vec![Example {
                description: "File not found".into(),
                code: r#"let content = File.read("missing.txt")?;  // May fail"#.into(),
                correct: Some(
                    r#"match File.read("config.txt") {
    Ok(content) => process(content),
    Err(e) => {
        log.error("Failed to read config: {}", e);
        use_default_config()
    }
}"#
                    .into(),
                ),
            }].into(),
            solutions: vec![
                Solution {
                    title: "Handle the error gracefully".into(),
                    description: "Use match or ? operator".into(),
                    code: Some(
                        r#"let content = File.read(path)?;  // Propagate error
// Or handle locally:
let content = File.read(path).unwrap_or_default();"#
                            .into(),
                    ),
                },
                Solution {
                    title: "Provide fallback".into(),
                    description: "Use default value on error".into(),
                    code: Some(
                        r#"let config = File.read("config.toml")
    .unwrap_or_else(|_| DEFAULT_CONFIG.to_string());"#
                            .into(),
                    ),
                },
            ].into(),
            see_also: vec!["Error handling: I/O errors are recoverable; use '?' for propagation, match for explicit handling, or .unwrap_or_default() for fallback values".into()].into(),
        },
    );

    // E0402: Network error
    map.insert(
        "E0402".into(),
        ErrorExplanation {
            code: "E0402".into(),
            title: "Network operation failed".into(),
            description: r#"A network operation failed at runtime.

This is a recoverable error that should be handled with appropriate retry
logic and fallback strategies."#
                .into(),
            examples: vec![Example {
                description: "HTTP request failure".into(),
                code: r#"let response = Http.get("https://api.example.com/data")?;"#.into(),
                correct: Some(
                    r#"let response = retry(3, || {
    Http.get("https://api.example.com/data")
}).await?;"#
                        .into(),
                ),
            }].into(),
            solutions: vec![
                Solution {
                    title: "Implement retry logic".into(),
                    description: "Retry transient failures".into(),
                    code: Some(
                        r#"let result = retry_with_backoff(
    max_attempts: 3,
    || Http.get(url)
).await;"#
                            .into(),
                    ),
                },
                Solution {
                    title: "Use circuit breaker".into(),
                    description: "Prevent cascade failures".into(),
                    code: Some(
                        r#"let cb = CircuitBreaker.new(threshold: 5);
let result = cb.call(|| Http.get(url)).await;"#
                            .into(),
                    ),
                },
            ].into(),
            see_also: vec![
                "Error handling (Level 3 - resilience): network errors are transient and should use retry logic with exponential backoff; Verum provides retry_with_backoff utility".into(),
                "Circuit breakers: prevent cascade failures by tracking error rates; CircuitBreaker.new(threshold: N) opens the circuit after N consecutive failures".into(),
            ].into(),
        },
    );

    // E0501: Security violation
    map.insert(
        "E0501".into(),
        ErrorExplanation {
            code: "E0501".into(),
            title: "Security violation detected".into(),
            description: r#"A security constraint was violated.

This includes capability violations, unauthorized access attempts,
and sandbox escapes. These are serious errors that may indicate
malicious activity."#
                .into(),
            examples: vec![Example {
                description: "Unauthorized capability use".into(),
                code: r#"fn read_sensitive() using [FileIO::Read] {
    // Only has Read capability
    File.write("/etc/passwd", "malicious");  // Error!
}"#
                .into(),
                correct: Some(
                    r#"fn read_sensitive() using [FileIO::Read] {
    // Only use capabilities we have
    let content = File.read("/etc/passwd")?;
    process(content)
}"#
                    .into(),
                ),
            }].into(),
            solutions: vec![
                Solution {
                    title: "Request proper capabilities".into(),
                    description: "Declare the capabilities you need".into(),
                    code: Some(
                        r#"fn write_data() using [FileIO::Write] {
    // Now we have write capability
    File.write(path, data)?;
}"#
                        .into(),
                    ),
                },
                Solution {
                    title: "Use capability attenuation".into(),
                    description: "Provide only necessary capabilities to callees".into(),
                    code: Some(
                        r#"// Caller has full FileIO, but attenuates for callee
attenuate FileIO to [FileIO::Read] {
    read_only_function();  // Cannot write
}"#
                        .into(),
                    ),
                },
            ].into(),
            see_also: vec![
                "Security (Level 4): capability violations indicate unauthorized access; request proper capabilities via 'using' clause or use capability attenuation to restrict callees".into(),
                "Capabilities: sub-contexts enable fine-grained control (e.g., FileIO::Read vs FileIO::Write); use 'attenuate Context to [SubContext]' to restrict callee capabilities".into(),
            ].into(),
        },
    );

    // E0E0: Rust keyword used instead of Verum equivalent
    map.insert(
        "E0E0".into(),
        ErrorExplanation {
            code: "E0E0".into(),
            title: "Rust keyword used instead of Verum equivalent".into(),
            description: r#"Verum uses different keywords than Rust for many constructs.
If you are migrating from Rust, the following mappings will help:

  struct Name { ... }    -->  type Name is { ... };
  enum Name { A, B }     -->  type Name is A | B;
  trait Name { ... }     -->  type Name is protocol { ... };
  impl Name { ... }      -->  implement Name { ... }
  impl Trait for T       -->  implement Trait for T
  use foo::bar           -->  mount foo.bar
  mod name               -->  module name
  crate                  -->  cog

Verum's syntax emphasizes readability and semantic clarity."#
                .into(),
            examples: vec![
                Example {
                    description: "Defining a record type (like Rust struct)".into(),
                    code: r#"struct Point { x: f64, y: f64 }"#.into(),
                    correct: Some(r#"type Point is { x: Float, y: Float };"#.into()),
                },
                Example {
                    description: "Defining a sum type (like Rust enum)".into(),
                    code: r#"enum Shape { Circle(f64), Rect(f64, f64) }"#.into(),
                    correct: Some(r#"type Shape is Circle(Float) | Rect(Float, Float);"#.into()),
                },
                Example {
                    description: "Defining a protocol (like Rust trait)".into(),
                    code: r#"trait Display { fn fmt(&self) -> String; }"#.into(),
                    correct: Some(r#"type Display is protocol { fn fmt(&self) -> Text; };"#.into()),
                },
            ]
            .into(),
            solutions: vec![
                Solution {
                    title: "Replace Rust keywords with Verum equivalents".into(),
                    description: "Use the keyword mapping table above to convert your code".into(),
                    code: None,
                },
            ]
            .into(),
            see_also: vec![
                "Grammar: grammar/verum.ebnf".into(),
                "Syntax: docs/detailed/05-syntax-grammar.md".into(),
            ]
            .into(),
        },
    );

    // E0E1: Rust type name used instead of Verum semantic type
    map.insert(
        "E0E1".into(),
        ErrorExplanation {
            code: "E0E1".into(),
            title: "Rust type name used instead of Verum semantic type".into(),
            description: r#"Verum uses semantic type names that describe meaning, not implementation.
If you are migrating from Rust, the following type mappings will help:

  String / &str  -->  Text
  Vec<T>         -->  List<T>
  HashMap<K,V>   -->  Map<K,V>
  HashSet<T>     -->  Set<T>
  Box<T>         -->  Heap<T>
  Rc<T> / Arc<T> -->  Shared<T>
  Option<T>      -->  Maybe<T>

Verum's semantic types make code more readable and intent-revealing."#
                .into(),
            examples: vec![
                Example {
                    description: "Using List instead of Vec".into(),
                    code: r#"let items: Vec<Int> = vec![1, 2, 3];"#.into(),
                    correct: Some(r#"let items: List<Int> = [1, 2, 3];"#.into()),
                },
                Example {
                    description: "Using Text instead of String".into(),
                    code: r#"let name: String = "Alice".to_string();"#.into(),
                    correct: Some(r#"let name: Text = "Alice";"#.into()),
                },
            ]
            .into(),
            solutions: vec![
                Solution {
                    title: "Replace Rust type names with Verum semantic types".into(),
                    description: "Use the type mapping table above to convert your code".into(),
                    code: None,
                },
            ]
            .into(),
            see_also: vec![
                "Type System: docs/detailed/03-type-system.md".into(),
            ]
            .into(),
        },
    );

    // E0E2: Rust macro syntax used
    map.insert(
        "E0E2".into(),
        ErrorExplanation {
            code: "E0E2".into(),
            title: "Rust macro syntax used instead of Verum syntax".into(),
            description: r#"Verum does not use the '!' suffix for macros or built-in functions.
If you are migrating from Rust, the following mappings will help:

  println!("...")      -->  print("...")
  format!("x={}", x)  -->  f"x={x}"
  panic!("error")      -->  panic("error")
  assert!(cond)        -->  assert(cond)
  assert_eq!(a, b)     -->  assert_eq(a, b)
  unreachable!()       -->  unreachable()
  vec![1, 2, 3]        -->  [1, 2, 3] or List(1, 2, 3)
  matches!(x, P)       -->  x is P
  dbg!(val)            -->  debug(val)

In Verum, compile-time constructs use the '@' prefix: @derive, @cfg, @const."#
                .into(),
            examples: vec![
                Example {
                    description: "Printing output".into(),
                    code: r#"println!("Hello, {}!", name);"#.into(),
                    correct: Some(r#"print(f"Hello, {name}!");"#.into()),
                },
                Example {
                    description: "Assertions".into(),
                    code: r#"assert_eq!(result, 42);"#.into(),
                    correct: Some(r#"assert_eq(result, 42);"#.into()),
                },
            ]
            .into(),
            solutions: vec![
                Solution {
                    title: "Remove the '!' and use Verum function syntax".into(),
                    description: "Built-in functions in Verum are called like regular functions".into(),
                    code: None,
                },
            ]
            .into(),
            see_also: vec![
                "Grammar: grammar/verum.ebnf".into(),
            ]
            .into(),
        },
    );

    map
});

/// Get explanation for an error code
pub fn get_explanation(code: &str) -> Option<&'static ErrorExplanation> {
    let code_text: Text = code.into();
    ERROR_EXPLANATIONS.get(&code_text)
}

/// Render an error explanation as formatted text
pub fn render_explanation(explanation: &ErrorExplanation, use_color: bool) -> String {
    let mut output = String::new();
    let colors = if use_color {
        ColorScheme::auto()
    } else {
        ColorScheme::no_color()
    };

    // Header
    let code_colored = colors.error_code.wrap(&explanation.code);
    output.push_str(&format!("\n{}: {}\n", code_colored, explanation.title));
    output.push_str(&"=".repeat(60));
    output.push_str("\n\n");

    // Description
    output.push_str(&explanation.description);
    output.push_str("\n\n");

    // Examples
    if !explanation.examples.is_empty() {
        output.push_str(&colors.severity_note.wrap("Examples"));
        output.push('\n');
        output.push_str(&"-".repeat(60));
        output.push_str("\n\n");

        for (i, example) in explanation.examples.iter().enumerate() {
            output.push_str(&format!("{}. {}\n\n", i + 1, example.description));

            // Incorrect code
            output.push_str("   ");
            output.push_str(&colors.severity_error.wrap("✗ Incorrect:"));
            output.push_str("\n   ```verum\n");
            for line in example.code.lines() {
                output.push_str(&format!("   {}\n", line));
            }
            output.push_str("   ```\n\n");

            // Correct code (if provided)
            if let Some(correct) = &example.correct {
                output.push_str("   ");
                output.push_str(&colors.severity_help.wrap("✓ Correct:"));
                output.push_str("\n   ```verum\n");
                for line in correct.lines() {
                    output.push_str(&format!("   {}\n", line));
                }
                output.push_str("   ```\n\n");
            }
        }
    }

    // Solutions
    if !explanation.solutions.is_empty() {
        output.push_str(&colors.severity_help.wrap("Solutions"));
        output.push('\n');
        output.push_str(&"-".repeat(60));
        output.push_str("\n\n");

        for (i, solution) in explanation.solutions.iter().enumerate() {
            output.push_str(&format!("{}. ", i + 1));
            output.push_str(&colors.severity_help.wrap(&solution.title));
            output.push('\n');
            output.push_str(&format!("   {}\n", solution.description));

            if let Some(code) = &solution.code {
                output.push_str("\n   ```verum\n");
                for line in code.lines() {
                    output.push_str(&format!("   {}\n", line));
                }
                output.push_str("   ```\n");
            }
            output.push('\n');
        }
    }

    // See Also
    if !explanation.see_also.is_empty() {
        output.push_str(&colors.severity_note.wrap("See Also"));
        output.push('\n');
        output.push_str(&"-".repeat(60));
        output.push('\n');
        for item in &explanation.see_also {
            output.push_str(&format!("  • {}\n", item));
        }
        output.push('\n');
    }

    output
}

/// List all available error codes
pub fn list_error_codes() -> List<Text> {
    ERROR_EXPLANATIONS.keys().cloned().collect()
}

/// Search for error codes by keyword
pub fn search_errors(keyword: &str) -> List<Text> {
    let keyword_lower = keyword.to_lowercase();
    ERROR_EXPLANATIONS
        .iter()
        .filter(|(_, explanation)| {
            explanation.title.to_lowercase().contains(&keyword_lower)
                || explanation
                    .description
                    .to_lowercase()
                    .contains(&keyword_lower)
        })
        .map(|(code, _)| code.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_explanation_e0312() {
        let explanation = get_explanation("E0312").unwrap();
        assert_eq!(explanation.code, "E0312");
        assert!(explanation.title.contains("Refinement"));
        assert!(!explanation.examples.is_empty());
        assert!(!explanation.solutions.is_empty());
    }

    #[test]
    fn test_get_explanation_not_found() {
        let result = get_explanation("E9999");
        assert!(result.is_none());
    }

    #[test]
    fn test_render_explanation_no_color() {
        let explanation = get_explanation("E0312").unwrap();
        let rendered = render_explanation(explanation, false);

        assert!(rendered.contains("E0312"));
        assert!(rendered.contains("Refinement"));
        assert!(rendered.contains("Examples"));
        assert!(rendered.contains("Solutions"));
        assert!(!rendered.contains("\x1b[")); // No ANSI codes
    }

    #[test]
    fn test_render_explanation_with_color() {
        let explanation = get_explanation("E0312").unwrap();
        let rendered = render_explanation(explanation, true);

        assert!(rendered.contains("E0312"));
        // May contain ANSI codes depending on environment
    }

    #[test]
    fn test_list_error_codes() {
        let codes = list_error_codes();
        assert!(codes.len() >= 10); // Should have at least 10 error codes
        assert!(codes.contains(&Text::from("E0312")));
        assert!(codes.contains(&Text::from("E0308")));
    }

    #[test]
    fn test_search_errors() {
        let results = search_errors("refinement");
        assert!(!results.is_empty());
        assert!(results.contains(&Text::from("E0312")));
    }

    #[test]
    fn test_search_errors_case_insensitive() {
        let results = search_errors("REFINEMENT");
        assert!(!results.is_empty());
        assert!(results.contains(&Text::from("E0312")));
    }

    #[test]
    fn test_all_error_codes_have_examples() {
        for (code, explanation) in ERROR_EXPLANATIONS.iter() {
            assert!(
                !explanation.examples.is_empty(),
                "Error code {} has no examples",
                code
            );
        }
    }

    #[test]
    fn test_all_error_codes_have_solutions() {
        for (code, explanation) in ERROR_EXPLANATIONS.iter() {
            assert!(
                !explanation.solutions.is_empty(),
                "Error code {} has no solutions",
                code
            );
        }
    }
}
