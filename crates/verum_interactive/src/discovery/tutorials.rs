//! Interactive tutorials and challenge system for learning Verum

use super::examples::ExampleCategory;

/// A step in an interactive tutorial
#[derive(Debug, Clone)]
pub struct TutorialStep {
    /// Title of this step
    pub title: String,
    /// Explanation text (can include markdown)
    pub explanation: String,
    /// Code example shown to the user
    pub example_code: Option<String>,
    /// Exercise prompt for the user
    pub exercise_prompt: Option<String>,
    /// Expected user input (for validation)
    pub expected_input: Option<String>,
    /// Hint to show if user is stuck
    pub hint: Option<String>,
    /// Whether user can skip this step
    pub skippable: bool,
}

impl TutorialStep {
    /// Create a new tutorial step
    pub fn new(title: impl Into<String>, explanation: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            explanation: explanation.into(),
            example_code: None,
            exercise_prompt: None,
            expected_input: None,
            hint: None,
            skippable: true,
        }
    }

    /// Add example code
    pub fn with_example(mut self, code: impl Into<String>) -> Self {
        self.example_code = Some(code.into());
        self
    }

    /// Add an exercise prompt
    pub fn with_exercise(mut self, prompt: impl Into<String>, expected: impl Into<String>) -> Self {
        self.exercise_prompt = Some(prompt.into());
        self.expected_input = Some(expected.into());
        self
    }

    /// Add a hint
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// Mark as required (not skippable)
    pub fn required(mut self) -> Self {
        self.skippable = false;
        self
    }
}

/// A complete interactive tutorial
#[derive(Debug, Clone)]
pub struct Tutorial {
    /// Tutorial title
    pub title: String,
    /// Brief description
    pub description: String,
    /// Difficulty level (1-5)
    pub difficulty: u8,
    /// Estimated time in minutes
    pub estimated_minutes: u32,
    /// Category
    pub category: ExampleCategory,
    /// Tutorial steps
    pub steps: Vec<TutorialStep>,
    /// Tags for search
    pub tags: Vec<String>,
}

impl Tutorial {
    /// Create a new tutorial
    pub fn new(title: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            description: description.into(),
            difficulty: 1,
            estimated_minutes: 10,
            category: ExampleCategory::Basics,
            steps: Vec::new(),
            tags: Vec::new(),
        }
    }

    /// Set difficulty
    pub fn with_difficulty(mut self, level: u8) -> Self {
        self.difficulty = level.clamp(1, 5);
        self
    }

    /// Set estimated time
    pub fn with_time(mut self, minutes: u32) -> Self {
        self.estimated_minutes = minutes;
        self
    }

    /// Set category
    pub fn with_category(mut self, category: ExampleCategory) -> Self {
        self.category = category;
        self
    }

    /// Add a step
    pub fn add_step(mut self, step: TutorialStep) -> Self {
        self.steps.push(step);
        self
    }

    /// Add tags
    pub fn with_tags(mut self, tags: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.tags.extend(tags.into_iter().map(|t| t.into()));
        self
    }

    /// Get progress percentage based on current step
    pub fn progress(&self, current_step: usize) -> f32 {
        if self.steps.is_empty() {
            1.0
        } else {
            current_step as f32 / self.steps.len() as f32
        }
    }
}

/// A coding challenge with validation
#[derive(Debug, Clone)]
pub struct Challenge {
    /// Challenge title
    pub title: String,
    /// Problem description
    pub description: String,
    /// Difficulty level (1-5)
    pub difficulty: u8,
    /// Category
    pub category: ExampleCategory,
    /// Starting code template
    pub template: String,
    /// Test cases to validate solution
    pub test_cases: Vec<TestCase>,
    /// Hints (revealed one at a time)
    pub hints: Vec<String>,
    /// Solution code (for showing after completion)
    pub solution: String,
    /// Tags
    pub tags: Vec<String>,
}

/// A test case for validating challenge solutions
#[derive(Debug, Clone)]
pub struct TestCase {
    /// Test case name
    pub name: String,
    /// Input code to prepend to user's solution
    pub setup: Option<String>,
    /// Expression to evaluate
    pub expression: String,
    /// Expected result (as string for comparison)
    pub expected: String,
}

impl Challenge {
    /// Create a new challenge
    pub fn new(title: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            description: description.into(),
            difficulty: 2,
            category: ExampleCategory::Basics,
            template: String::new(),
            test_cases: Vec::new(),
            hints: Vec::new(),
            solution: String::new(),
            tags: Vec::new(),
        }
    }

    /// Set difficulty
    pub fn with_difficulty(mut self, level: u8) -> Self {
        self.difficulty = level.clamp(1, 5);
        self
    }

    /// Set category
    pub fn with_category(mut self, category: ExampleCategory) -> Self {
        self.category = category;
        self
    }

    /// Set template code
    pub fn with_template(mut self, template: impl Into<String>) -> Self {
        self.template = template.into();
        self
    }

    /// Add a test case
    pub fn add_test(
        mut self,
        name: impl Into<String>,
        expr: impl Into<String>,
        expected: impl Into<String>,
    ) -> Self {
        self.test_cases.push(TestCase {
            name: name.into(),
            setup: None,
            expression: expr.into(),
            expected: expected.into(),
        });
        self
    }

    /// Add a hint
    pub fn add_hint(mut self, hint: impl Into<String>) -> Self {
        self.hints.push(hint.into());
        self
    }

    /// Set solution
    pub fn with_solution(mut self, solution: impl Into<String>) -> Self {
        self.solution = solution.into();
        self
    }

    /// Add tags
    pub fn with_tags(mut self, tags: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.tags.extend(tags.into_iter().map(|t| t.into()));
        self
    }
}

/// Template for starting a new playbook
#[derive(Debug, Clone)]
pub struct PlaybookTemplate {
    /// Template name
    pub name: String,
    /// Description
    pub description: String,
    /// Category
    pub category: ExampleCategory,
    /// Initial cells
    pub cells: Vec<TemplateCell>,
}

/// A cell in a playbook template
#[derive(Debug, Clone)]
pub struct TemplateCell {
    /// Cell kind (code or markdown)
    pub is_code: bool,
    /// Cell content
    pub content: String,
}

impl PlaybookTemplate {
    /// Create a new template
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            category: ExampleCategory::Basics,
            cells: Vec::new(),
        }
    }

    /// Set category
    pub fn with_category(mut self, category: ExampleCategory) -> Self {
        self.category = category;
        self
    }

    /// Add a code cell
    pub fn add_code(mut self, content: impl Into<String>) -> Self {
        self.cells.push(TemplateCell {
            is_code: true,
            content: content.into(),
        });
        self
    }

    /// Add a markdown cell
    pub fn add_markdown(mut self, content: impl Into<String>) -> Self {
        self.cells.push(TemplateCell {
            is_code: false,
            content: content.into(),
        });
        self
    }
}

// ============================================================================
// Built-in tours — embedded from docs/by-example (T0858 slice 6)
// ============================================================================

// (slug, notes.md, main.vr) — generated by build.rs from the
// directory listing; a chapter is embedded by EXISTING. The tours
// below are curated groupings over the chapters; builtin copies of
// teaching content are forbidden (one truth of the examples).
include!(concat!(env!("OUT_DIR"), "/by_example_chapters.rs"));

/// A chapter's display title: the first markdown heading of its
/// notes, else the slug.
fn chapter_title(slug: &str, notes: &str) -> String {
    notes
        .lines()
        .find_map(|l| l.strip_prefix("# ").map(str::to_string))
        .unwrap_or_else(|| slug.to_string())
}

/// The chapter's explanation: its notes WITHOUT the `## The code`
/// section — the tour puts the code in a runnable cell right below,
/// and fenced duplication doubles the scroll.
fn chapter_explanation(notes: &str) -> String {
    let mut out = String::new();
    let mut skipping = false;
    for line in notes.lines() {
        if line.starts_with("## ") {
            skipping = line.trim() == "## The code";
        }
        if !skipping {
            out.push_str(line);
            out.push('\n');
        }
    }
    out.trim().to_string()
}

fn tour_from_chapters(
    title: &str,
    description: &str,
    difficulty: u8,
    minutes: u32,
    prefixes: &[&str],
) -> Tutorial {
    let mut tour = Tutorial::new(title, description)
        .with_difficulty(difficulty)
        .with_time(minutes);
    for (slug, notes, code) in BY_EXAMPLE_CHAPTERS {
        if prefixes.iter().any(|p| slug.starts_with(p)) {
            tour = tour.add_step(
                TutorialStep::new(chapter_title(slug, notes), chapter_explanation(notes))
                    .with_example(code.trim()),
            );
        }
    }
    tour
}

/// The guided tours: curated groupings over docs/by-example, plus a
/// closing tour that collects every chapter no group claimed — a new
/// chapter always ships in SOME tour, never silently nowhere.
pub fn builtin_tutorials() -> Vec<Tutorial> {
    const GROUPS: &[(&str, &str, u8, u32, &[&str])] = &[
        (
            "First steps",
            "Hello world to honest errors — the first half hour",
            1,
            25,
            &["01-", "02-", "03-", "04-"],
        ),
        (
            "Collections & functions",
            "Data, the functions over it, and mutation with intent",
            1,
            25,
            &["05-", "06-", "07-", "11-"],
        ),
        (
            "Abstraction",
            "Protocols, generics, and the mount system",
            2,
            30,
            &["08-", "09-", "10-"],
        ),
        (
            "Researcher: memory, proofs, contexts",
            "CBGR references, refinement types, and the context system",
            3,
            35,
            &["14-", "15-", "16-"],
        ),
    ];
    let mut tours: Vec<Tutorial> = GROUPS
        .iter()
        .map(|(t, d, diff, min, prefixes)| {
            tour_from_chapters(t, d, *diff, *min, prefixes)
        })
        .collect();

    let claimed: Vec<&str> = GROUPS
        .iter()
        .flat_map(|(_, _, _, _, prefixes)| prefixes.iter().copied())
        .collect();
    let rest: Vec<&str> = BY_EXAMPLE_CHAPTERS
        .iter()
        .map(|(slug, _, _)| *slug)
        .filter(|slug| !claimed.iter().any(|p| slug.starts_with(p)))
        .collect();
    if !rest.is_empty() {
        tours.push(tour_from_chapters(
            "More examples",
            "Every remaining by-example chapter, in order",
            2,
            40,
            &rest,
        ));
    }
    tours
}

// ============================================================================
// Built-in challenges
// ============================================================================

/// Get all built-in challenges
pub fn builtin_challenges() -> Vec<Challenge> {
    vec![
        challenge_fizzbuzz(),
        challenge_fibonacci(),
        challenge_palindrome(),
        challenge_binary_search(),
        challenge_matrix_transpose(),
    ]
}

fn challenge_fizzbuzz() -> Challenge {
    Challenge::new(
        "FizzBuzz",
        "Print numbers 1-100, but replace multiples of 3 with \"Fizz\", \
         multiples of 5 with \"Buzz\", and multiples of both with \"FizzBuzz\".",
    )
    .with_difficulty(1)
    .with_category(ExampleCategory::Basics)
    .with_template(
        r#"fn fizzbuzz(n: Int) -> Text {
    // Your code here
}"#,
    )
    .add_test("fizzbuzz(3)", "fizzbuzz(3)", "\"Fizz\"")
    .add_test("fizzbuzz(5)", "fizzbuzz(5)", "\"Buzz\"")
    .add_test("fizzbuzz(15)", "fizzbuzz(15)", "\"FizzBuzz\"")
    .add_test("fizzbuzz(7)", "fizzbuzz(7)", "\"7\"")
    .add_hint("Check divisibility with the % operator")
    .add_hint("Check for 15 (both 3 and 5) first!")
    .with_solution(
        r#"fn fizzbuzz(n: Int) -> Text {
    if n % 15 == 0 { "FizzBuzz" }
    else if n % 3 == 0 { "Fizz" }
    else if n % 5 == 0 { "Buzz" }
    else { f"{n}" }
}"#,
    )
    .with_tags(["beginner", "classic"])
}

fn challenge_fibonacci() -> Challenge {
    Challenge::new(
        "Fibonacci Generator",
        "Create a generator that yields the Fibonacci sequence: 0, 1, 1, 2, 3, 5, 8, ...",
    )
    .with_difficulty(2)
    .with_category(ExampleCategory::Generators)
    .with_template(
        r#"fn* fibonacci() -> Int {
    // Your code here
}"#,
    )
    .add_test(
        "first 8",
        "fibonacci().take(8).collect()",
        "[0, 1, 1, 2, 3, 5, 8, 13]",
    )
    .add_hint("You need two variables to track the previous two numbers")
    .add_hint("Use `yield` to produce each number")
    .with_solution(
        r#"fn* fibonacci() -> Int {
    let mut a = 0
    let mut b = 1
    loop {
        yield a
        let next = a + b
        a = b
        b = next
    }
}"#,
    )
    .with_tags(["generator", "classic", "recursion"])
}

fn challenge_palindrome() -> Challenge {
    Challenge::new(
        "Palindrome Checker",
        "Write a function that checks if a string is a palindrome (reads the same forwards and backwards).",
    )
    .with_difficulty(1)
    .with_category(ExampleCategory::Text)
    .with_template(
        r#"fn is_palindrome(s: Text) -> Bool {
    // Your code here
}"#,
    )
    .add_test("racecar", r#"is_palindrome("racecar")"#, "true")
    .add_test("hello", r#"is_palindrome("hello")"#, "false")
    .add_test("a", r#"is_palindrome("a")"#, "true")
    .add_test("empty", r#"is_palindrome("")"#, "true")
    .add_hint("Compare the string with its reverse")
    .add_hint("Use .chars().rev().collect() to reverse")
    .with_solution(
        r#"fn is_palindrome(s: Text) -> Bool {
    let chars: List<Char> = s.chars().collect()
    let reversed: List<Char> = s.chars().rev().collect()
    chars == reversed
}"#,
    )
    .with_tags(["text", "string", "beginner"])
}

fn challenge_binary_search() -> Challenge {
    Challenge::new(
        "Binary Search",
        "Implement binary search to find an element in a sorted list. Return the index or None.",
    )
    .with_difficulty(2)
    .with_category(ExampleCategory::Collections)
    .with_template(
        r#"fn binary_search(list: List<Int>, target: Int) -> Maybe<Int> {
    // Your code here
}"#,
    )
    .add_test(
        "found",
        "binary_search(List.from([1,2,3,4,5]), 3)",
        "Some(2)",
    )
    .add_test(
        "not found",
        "binary_search(List.from([1,2,3,4,5]), 6)",
        "None",
    )
    .add_test(
        "first",
        "binary_search(List.from([1,2,3,4,5]), 1)",
        "Some(0)",
    )
    .add_hint("Use low and high pointers")
    .add_hint("Calculate mid as (low + high) / 2")
    .add_hint("Narrow the search range based on comparison")
    .with_solution(
        r#"fn binary_search(list: List<Int>, target: Int) -> Maybe<Int> {
    let mut low = 0
    let mut high = list.len() - 1

    while low <= high {
        let mid = (low + high) / 2
        let value = list.get(mid).unwrap()

        if value == target {
            return Some(mid)
        } else if value < target {
            low = mid + 1
        } else {
            high = mid - 1
        }
    }

    None
}"#,
    )
    .with_tags(["algorithm", "search", "classic"])
}

fn challenge_matrix_transpose() -> Challenge {
    Challenge::new(
        "Matrix Transpose",
        "Write a function that transposes a matrix (swaps rows and columns).",
    )
    .with_difficulty(2)
    .with_category(ExampleCategory::Tensor)
    .with_template(
        r#"fn transpose(m: Tensor) -> Tensor {
    // Your code here
}"#,
    )
    .add_test(
        "2x3",
        "transpose(Tensor.from([[1,2,3],[4,5,6]])).shape()",
        "[3, 2]",
    )
    .add_hint("The result has shape [cols, rows] where the input has [rows, cols]")
    .add_hint("Use Tensor.t() method!")
    .with_solution(
        r#"fn transpose(m: Tensor) -> Tensor {
    m.t()  // Built-in transpose!
}"#,
    )
    .with_tags(["tensor", "matrix", "linear-algebra"])
}

// ============================================================================
// Built-in templates
// ============================================================================

/// Get all built-in playbook templates
pub fn builtin_templates() -> Vec<PlaybookTemplate> {
    vec![
        template_blank(),
        template_data_analysis(),
        template_machine_learning(),
        template_web_scraping(),
    ]
}

fn template_blank() -> PlaybookTemplate {
    PlaybookTemplate::new("Blank Playbook", "Start with an empty playbook")
        .with_category(ExampleCategory::Basics)
        .add_markdown("# My Playbook\n\nStart typing Verum code below!")
        .add_code("// Your code here")
}

fn template_data_analysis() -> PlaybookTemplate {
    PlaybookTemplate::new(
        "Data Analysis",
        "Template for data exploration and analysis",
    )
    .with_category(ExampleCategory::Collections)
    .add_markdown("# Data Analysis Playbook\n\nThis template helps you explore and analyze data.")
    .add_code(
        r#"// Load your data
let data = List.from([
    { name: "Alice", age: 30, score: 85 },
    { name: "Bob", age: 25, score: 92 },
    { name: "Charlie", age: 35, score: 78 },
])"#,
    )
    .add_code(
        r#"// Basic statistics
let ages = data.map(fn(x) x.age)
print(f"Average age: {ages.mean()}")"#,
    )
    .add_code(
        r#"// Filter and transform
let high_scorers = data.filter(fn(x) x.score > 80)
print(f"High scorers: {high_scorers.map(fn(x) x.name)}")"#,
    )
}

fn template_machine_learning() -> PlaybookTemplate {
    PlaybookTemplate::new("Machine Learning", "Template for ML experimentation")
        .with_category(ExampleCategory::NeuralNet)
        .add_markdown(
            "# Machine Learning Playbook\n\nExperiment with neural networks and training.",
        )
        .add_code(
            r#"// Define a simple neural network
type MLP is {
    layer1: Linear,
    layer2: Linear,
}

implement MLP {
    fn new(input: Int, hidden: Int, output: Int) -> Self {
        MLP {
            layer1: Linear.new(input, hidden),
            layer2: Linear.new(hidden, output),
        }
    }

    fn forward(&self, x: Tensor) -> Tensor {
        let h = self.layer1.forward(x).relu()
        self.layer2.forward(h)
    }
}"#,
        )
        .add_code(
            r#"// Create model and sample input
let model = MLP.new(10, 32, 2)
let x = Tensor.randn([1, 10])
let y = model.forward(x)
print(f"Output shape: {y.shape()}")"#,
        )
}

fn template_web_scraping() -> PlaybookTemplate {
    PlaybookTemplate::new(
        "Web/API Client",
        "Template for HTTP requests and API interactions",
    )
    .with_category(ExampleCategory::Async)
    .add_markdown("# Web/API Client Playbook\n\nMake HTTP requests and process responses.")
    .add_code(
        r#"// Define an async fetch function
async fn fetch_json(url: Text) -> Result<Value, Error> {
    let response = await http.get(url)
    response.json()
}"#,
    )
    .add_code(
        r#"// Example usage (uncomment to run)
// async fn main() {
//  let data = await fetch_json("https://api.example.com/data")
//  print(data)
// }"#,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tutorial_creation() {
        let tutorial = Tutorial::new("Test", "A test tutorial")
            .with_difficulty(2)
            .add_step(TutorialStep::new("Step 1", "Explanation"));

        assert_eq!(tutorial.title, "Test");
        assert_eq!(tutorial.difficulty, 2);
        assert_eq!(tutorial.steps.len(), 1);
    }

    #[test]
    fn test_builtin_tutorials() {
        let tutorials = builtin_tutorials();
        assert!(!tutorials.is_empty());

        // Check all tutorials have at least one step
        for tutorial in &tutorials {
            assert!(
                !tutorial.steps.is_empty(),
                "Tutorial '{}' has no steps",
                tutorial.title
            );
        }
    }

    #[test]
    fn test_challenge_creation() {
        let challenge = Challenge::new("Test", "A test challenge")
            .with_difficulty(3)
            .add_test("basic", "1 + 1", "2");

        assert_eq!(challenge.title, "Test");
        assert_eq!(challenge.difficulty, 3);
        assert_eq!(challenge.test_cases.len(), 1);
    }

    #[test]
    fn test_builtin_challenges() {
        let challenges = builtin_challenges();
        assert!(!challenges.is_empty());

        // Check all challenges have test cases
        for challenge in &challenges {
            assert!(
                !challenge.test_cases.is_empty(),
                "Challenge '{}' has no tests",
                challenge.title
            );
        }
    }

    #[test]
    fn test_builtin_templates() {
        let templates = builtin_templates();
        assert!(!templates.is_empty());

        // Check all templates have cells
        for template in &templates {
            assert!(
                !template.cells.is_empty(),
                "Template '{}' has no cells",
                template.name
            );
        }
    }
}
