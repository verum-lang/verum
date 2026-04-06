//! Example snippets for interactive exploration

use std::fmt;

/// Category of example
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExampleCategory {
    /// Basic language features
    Basics,
    /// Collections and data structures
    Collections,
    /// Text processing
    Text,
    /// Input/Output operations
    IO,
    /// Asynchronous programming
    Async,
    /// Concurrency and parallelism
    Concurrency,
    /// Mathematical operations
    Math,
    /// Tensor operations (Layer 4+)
    Tensor,
    /// GPU computing (Layer 5)
    Gpu,
    /// Automatic differentiation (Layer 6)
    AutoDiff,
    /// Neural networks (Layer 7)
    NeuralNet,
    /// Meta-programming
    Meta,
    /// Error handling
    ErrorHandling,
    /// Context system (DI)
    Context,
    /// Pattern matching
    Patterns,
    /// Generators and iterators
    Generators,
}

impl fmt::Display for ExampleCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExampleCategory::Basics => write!(f, "Basics"),
            ExampleCategory::Collections => write!(f, "Collections"),
            ExampleCategory::Text => write!(f, "Text"),
            ExampleCategory::IO => write!(f, "I/O"),
            ExampleCategory::Async => write!(f, "Async"),
            ExampleCategory::Concurrency => write!(f, "Concurrency"),
            ExampleCategory::Math => write!(f, "Math"),
            ExampleCategory::Tensor => write!(f, "Tensor"),
            ExampleCategory::Gpu => write!(f, "GPU"),
            ExampleCategory::AutoDiff => write!(f, "AutoDiff"),
            ExampleCategory::NeuralNet => write!(f, "Neural Net"),
            ExampleCategory::Meta => write!(f, "Meta"),
            ExampleCategory::ErrorHandling => write!(f, "Errors"),
            ExampleCategory::Context => write!(f, "Context"),
            ExampleCategory::Patterns => write!(f, "Patterns"),
            ExampleCategory::Generators => write!(f, "Generators"),
        }
    }
}

/// An executable example snippet
#[derive(Debug, Clone)]
pub struct Example {
    /// Title of the example
    pub title: String,
    /// Brief description
    pub description: String,
    /// The Verum source code
    pub source: String,
    /// Expected output (for verification)
    pub expected_output: Option<String>,
    /// Category for organization
    pub category: ExampleCategory,
    /// Tags for search
    pub tags: Vec<String>,
    /// Difficulty level (1-5)
    pub difficulty: u8,
    /// Related examples
    pub related: Vec<String>,
}

impl Example {
    /// Create a new example
    pub fn new(
        title: impl Into<String>,
        description: impl Into<String>,
        source: impl Into<String>,
        category: ExampleCategory,
    ) -> Self {
        Self {
            title: title.into(),
            description: description.into(),
            source: source.into(),
            expected_output: None,
            category,
            tags: Vec::new(),
            difficulty: 1,
            related: Vec::new(),
        }
    }

    /// Set expected output
    pub fn with_expected(mut self, output: impl Into<String>) -> Self {
        self.expected_output = Some(output.into());
        self
    }

    /// Add tags
    pub fn with_tags(mut self, tags: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.tags.extend(tags.into_iter().map(|t| t.into()));
        self
    }

    /// Set difficulty (1-5)
    pub fn with_difficulty(mut self, level: u8) -> Self {
        self.difficulty = level.clamp(1, 5);
        self
    }

    /// Add related examples
    pub fn with_related(mut self, related: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.related.extend(related.into_iter().map(|r| r.into()));
        self
    }

    /// Format for display in a list
    pub fn list_display(&self) -> String {
        let stars = "★".repeat(self.difficulty as usize);
        format!(
            "[{}] {} {} - {}",
            self.category, stars, self.title, self.description
        )
    }

    /// Format full display with source
    pub fn full_display(&self) -> String {
        let mut output = String::new();

        output.push_str(&format!("# {}\n\n", self.title));
        output.push_str(&format!("Category: {} | Difficulty: {}/5\n\n", self.category, self.difficulty));
        output.push_str(&self.description);
        output.push_str("\n\n");
        output.push_str("```verum\n");
        output.push_str(&self.source);
        output.push_str("\n```\n");

        if let Some(expected) = &self.expected_output {
            output.push_str("\n## Expected Output\n\n");
            output.push_str("```\n");
            output.push_str(expected);
            output.push_str("\n```\n");
        }

        if !self.tags.is_empty() {
            output.push_str("\nTags: ");
            output.push_str(&self.tags.join(", "));
            output.push('\n');
        }

        if !self.related.is_empty() {
            output.push_str("\n## Related Examples\n\n");
            for r in &self.related {
                output.push_str(&format!("- {}\n", r));
            }
        }

        output
    }
}

/// Built-in examples for the playground
pub fn builtin_examples() -> Vec<Example> {
    vec![
        // Basics
        Example::new(
            "Hello World",
            "Your first Verum program",
            r#"// Welcome to Verum!
let message = "Hello, Verum!"
print(message)"#,
            ExampleCategory::Basics,
        )
        .with_expected("Hello, Verum!")
        .with_difficulty(1)
        .with_tags(["beginner", "print"]),

        Example::new(
            "Variables and Types",
            "Working with bindings and type inference",
            r#"let x = 42              // Int (inferred)
let y: Float = 3.14     // explicit type
let mut counter = 0     // mutable binding

counter = counter + 1
print(f"x={x}, y={y}, counter={counter}")"#,
            ExampleCategory::Basics,
        )
        .with_difficulty(1)
        .with_tags(["variables", "types", "inference"]),

        Example::new(
            "Functions",
            "Defining and calling functions",
            r#"fn greet(name: Text) -> Text {
    f"Hello, {name}!"
}

fn add(a: Int, b: Int) -> Int {
    a + b
}

print(greet("Verum"))
print(f"2 + 3 = {add(2, 3)}")"#,
            ExampleCategory::Basics,
        )
        .with_difficulty(1)
        .with_tags(["functions", "parameters", "return"]),

        // Collections
        Example::new(
            "List Operations",
            "Working with dynamic arrays",
            r#"let nums = List::from([1, 2, 3, 4, 5])

// Map: transform each element
let doubled = nums.map(fn(x) x * 2)
print(f"Doubled: {doubled}")

// Filter: keep elements matching predicate
let evens = nums.filter(fn(x) x % 2 == 0)
print(f"Evens: {evens}")

// Reduce: combine all elements
let sum = nums.reduce(0, fn(acc, x) acc + x)
print(f"Sum: {sum}")"#,
            ExampleCategory::Collections,
        )
        .with_expected("Doubled: [2, 4, 6, 8, 10]\nEvens: [2, 4]\nSum: 15")
        .with_difficulty(2)
        .with_tags(["list", "map", "filter", "reduce"]),

        Example::new(
            "Map and Set",
            "Using associative containers",
            r#"// Map: key-value pairs
let mut scores = Map::new()
scores.insert("Alice", 95)
scores.insert("Bob", 87)
scores.insert("Charlie", 92)

print(f"Alice's score: {scores.get("Alice")}")

// Set: unique values
let unique = Set::from([1, 2, 2, 3, 3, 3])
print(f"Unique values: {unique}")  // {1, 2, 3}"#,
            ExampleCategory::Collections,
        )
        .with_difficulty(2)
        .with_tags(["map", "set", "insert", "get"]),

        // Pattern Matching
        Example::new(
            "Pattern Matching",
            "Destructuring and matching values",
            r#"type Shape is
    | Circle(Float)
    | Rectangle { width: Float, height: Float }
    | Triangle { base: Float, height: Float };

fn area(shape: Shape) -> Float {
    match shape {
        Circle(r) => 3.14159 * r * r,
        Rectangle { width, height } => width * height,
        Triangle { base, height } => 0.5 * base * height,
    }
}

let shapes = [
    Circle(5.0),
    Rectangle { width: 4.0, height: 3.0 },
    Triangle { base: 6.0, height: 4.0 },
]

for shape in shapes {
    print(f"Area: {area(shape)}")
}"#,
            ExampleCategory::Patterns,
        )
        .with_difficulty(2)
        .with_tags(["pattern", "match", "enum", "destructure"]),

        // Generators
        Example::new(
            "Generators",
            "Lazy sequences with yield",
            r#"// Infinite Fibonacci generator
fn* fibonacci() -> Int {
    let mut a = 0
    let mut b = 1
    loop {
        yield a
        let next = a + b
        a = b
        b = next
    }
}

// Take first 10 Fibonacci numbers
let fibs = fibonacci().take(10).collect()
print(f"Fibonacci: {fibs}")"#,
            ExampleCategory::Generators,
        )
        .with_expected("Fibonacci: [0, 1, 1, 2, 3, 5, 8, 13, 21, 34]")
        .with_difficulty(3)
        .with_tags(["generator", "yield", "lazy", "infinite"]),

        // Async
        Example::new(
            "Async/Await",
            "Asynchronous programming basics",
            r#"async fn fetch_data(url: Text) -> Result<Text, Error> {
    // Simulated async operation
    await sleep(100ms)
    Ok(f"Data from {url}")
}

async fn main() {
    let result = await fetch_data("https://api.example.com")
    match result {
        Ok(data) => print(f"Received: {data}"),
        Err(e) => print(f"Error: {e}"),
    }
}"#,
            ExampleCategory::Async,
        )
        .with_difficulty(3)
        .with_tags(["async", "await", "future"]),

        // Context System
        Example::new(
            "Context System",
            "Dependency injection with contexts",
            r#"// Define a logger context
type Logger is protocol {
    fn info(&self, msg: Text);
    fn error(&self, msg: Text);
};

// Function using Logger context
fn process_data(data: List<Int>) using [Logger] {
    Logger.info(f"Processing {data.len()} items")

    let sum = data.reduce(0, fn(a, b) a + b)
    Logger.info(f"Sum: {sum}")
}

// Provide context and call
provide ConsoleLogger as Logger {
    process_data(List::from([1, 2, 3, 4, 5]))
}"#,
            ExampleCategory::Context,
        )
        .with_difficulty(3)
        .with_tags(["context", "provide", "using", "di"]),

        // Math Layer 4: Tensors
        Example::new(
            "Tensor Basics",
            "Working with multi-dimensional arrays",
            r#"// Create tensors
let a = Tensor::from([[1.0, 2.0], [3.0, 4.0]])
let b = Tensor::randn([2, 2])  // Random normal

print(f"Shape: {a.shape()}")  // [2, 2]
print(f"Sum: {a.sum()}")      // 10.0
print(f"Mean: {a.mean()}")    // 2.5

// Matrix multiplication
let c = a.matmul(b)
print(f"Result shape: {c.shape()}")"#,
            ExampleCategory::Tensor,
        )
        .with_difficulty(3)
        .with_tags(["tensor", "matrix", "shape"]),

        // AutoDiff
        Example::new(
            "Automatic Differentiation",
            "Computing gradients automatically",
            r#"// Define a function to differentiate
fn f(x: Tensor) -> Tensor {
    x.pow(2).sum()  // f(x) = sum(x^2)
}

// The gradient: df/dx = 2x
let x = Tensor::from([1.0, 2.0, 3.0])
let grad_f = grad(f)
let gradient = grad_f(x)

print(f"x = {x}")
print(f"gradient = {gradient}")  // [2.0, 4.0, 6.0]"#,
            ExampleCategory::AutoDiff,
        )
        .with_difficulty(4)
        .with_tags(["autodiff", "gradient", "differentiation"]),

        // Neural Network
        Example::new(
            "Simple Neural Network",
            "Building a basic MLP",
            r#"// Define a simple MLP
type MLP is {
    layer1: Linear,
    layer2: Linear,
};

implement MLP {
    fn new(input: Int, hidden: Int, output: Int) -> Self {
        MLP {
            layer1: Linear::new(input, hidden),
            layer2: Linear::new(hidden, output),
        }
    }

    fn forward(&self, x: Tensor) -> Tensor {
        let h = self.layer1.forward(x).relu()
        self.layer2.forward(h)
    }
}

let model = MLP::new(784, 128, 10)
let input = Tensor::randn([1, 784])
let output = model.forward(input)
print(f"Output shape: {output.shape()}")"#,
            ExampleCategory::NeuralNet,
        )
        .with_difficulty(4)
        .with_tags(["neural", "mlp", "layer", "forward"]),

        // Error Handling
        Example::new(
            "Error Handling",
            "Working with Result and Maybe",
            r#"fn divide(a: Int, b: Int) -> Result<Int, Text> {
    if b == 0 {
        Err("Division by zero")
    } else {
        Ok(a / b)
    }
}

// Using match
match divide(10, 2) {
    Ok(result) => print(f"Result: {result}"),
    Err(msg) => print(f"Error: {msg}"),
}

// Using ? operator
fn calculate() -> Result<Int, Text> {
    let x = divide(100, 5)?
    let y = divide(x, 2)?
    Ok(y)
}

print(f"Calculate: {calculate()}")"#,
            ExampleCategory::ErrorHandling,
        )
        .with_difficulty(2)
        .with_tags(["result", "error", "maybe", "propagation"]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_example_creation() {
        let ex = Example::new(
            "Test",
            "A test example",
            "let x = 1",
            ExampleCategory::Basics,
        )
        .with_difficulty(2)
        .with_tags(["test"]);

        assert_eq!(ex.title, "Test");
        assert_eq!(ex.difficulty, 2);
        assert!(!ex.tags.is_empty());
    }

    #[test]
    fn test_builtin_examples() {
        let examples = builtin_examples();
        assert!(!examples.is_empty());

        // Check all categories are represented
        let categories: std::collections::HashSet<_> =
            examples.iter().map(|e| e.category).collect();
        assert!(categories.contains(&ExampleCategory::Basics));
        assert!(categories.contains(&ExampleCategory::Collections));
    }
}
