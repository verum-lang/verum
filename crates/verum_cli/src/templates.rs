// Project templates for Verum.
// Uses correct Verum syntax: `mount` (not import), `@test` (not #[test]),
// `type X is { ... }` (not struct), `implement` (not impl).
// Templates are profile-aware: application, systems, research.

use crate::config::LanguageProfile;
use crate::error::Result;
use std::fs;
use std::path::Path;

pub mod binary {
    use super::*;

    pub fn create(dir: &Path, name: &str, profile: LanguageProfile) -> Result<()> {
        let main_content = match profile {
            LanguageProfile::Application => format!(
                r#"// {name} — Verum application
// Profile: application (safe by default, no @unsafe)

mount core.base.{{List, Text, Maybe}};

fn main() {{
    let greeting: Text = "Hello from {name}!";
    print(greeting);

    // Semantic types: List (not Vec), Text (not String), Maybe (not Option)
    let items: List<Int> = [1, 2, 3, 4, 5];
    let sum = items.fold(0, |acc, x| acc + x);
    print(f"Sum: {{sum}}");
}}
"#,
                name = name
            ),
            LanguageProfile::Systems => format!(
                r#"// {name} — Verum systems application
// Profile: systems (full language, @unsafe allowed)

mount core.base.{{List, Text, Maybe}};
mount core.mem.{{Heap, allocator}};

fn main() {{
    let greeting: Text = "Hello from {name}!";
    print(greeting);

    // Systems profile: manual memory control available
    let data = Heap(List.from([1, 2, 3, 4, 5]));
    let sum = data.fold(0, |acc, x| acc + x);
    print(f"Sum: {{sum}}");

    // Three-tier references:
    //   &T         — managed (~15ns CBGR check)
    //   &checked T — compiler-proven (0ns)
    //   &unsafe T  — manual proof (0ns, requires @unsafe)
}}
"#,
                name = name
            ),
            LanguageProfile::Research => format!(
                r#"// {name} — Verum research application
// Profile: research (dependent types, formal proofs)

mount core.base.{{List, Text, Maybe}};

// Refinement type: Nat is Int where value >= 0
type Nat is Int where self >= 0;

// Function with refinement-typed parameter
fn factorial(n: Nat) -> Nat {{
    if n == 0 {{
        1
    }} else {{
        n * factorial(n - 1)
    }}
}}

fn main() {{
    let result = factorial(10);
    print(f"10! = {{result}}");
}}
"#,
                name = name
            ),
        };

        fs::write(dir.join("src/main.vr"), main_content)?;

        let test_content = format!(
            r#"// Tests for {name}

mount core.base.{{List, Text}};

@test
fn test_example() {{
    assert(true, "Example test");
}}

@test
fn test_semantic_types() {{
    let items: List<Int> = [1, 2, 3];
    assert_eq(items.len(), 3, "List should have 3 elements");
}}
"#,
            name = name
        );

        fs::write(dir.join("tests/main_test.vr"), test_content)?;
        Ok(())
    }
}

pub mod library {
    use super::*;

    pub fn create(dir: &Path, name: &str, profile: LanguageProfile) -> Result<()> {
        let lib_content = match profile {
            LanguageProfile::Application | LanguageProfile::Systems => format!(
                r#"// {name} library
// Semantic types: List (not Vec), Text (not String), Maybe (not Option)

mount core.base.{{List, Text, Maybe}};

/// Add two integers.
pub fn add(a: Int, b: Int) -> Int {{
    a + b
}}

/// A 2D point using record syntax (`type X is {{ ... }}`).
pub type Point is {{
    x: Float,
    y: Float,
}};

/// Implementation block uses `implement` keyword (not `impl`).
implement Point {{
    pub fn new(x: Float, y: Float) -> Point {{
        Point {{ x, y }}
    }}

    pub fn distance(&self, other: &Point) -> Float {{
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        (dx * dx + dy * dy).sqrt()
    }}
}}

/// A path made of points, using semantic `List` type.
pub type Path is {{
    points: List<Point>,
}};

implement Path {{
    pub fn new() -> Path {{
        Path {{ points: List.new() }}
    }}

    pub fn add_point(&mut self, point: Point) {{
        self.points.push(point);
    }}

    pub fn total_length(&self) -> Float {{
        let mut total = 0.0;
        for i in 0..self.points.len() - 1 {{
            total += self.points[i].distance(&self.points[i + 1]);
        }}
        total
    }}
}}

/// Find a point by x-coordinate, returning `Maybe` (not `Option`).
pub fn find_point_by_x(points: &List<Point>, x: Float) -> Maybe<&Point> {{
    for point in points {{
        if point.x == x {{
            return Some(point);
        }}
    }}
    None
}}
"#,
                name = name
            ),
            LanguageProfile::Research => format!(
                r#"// {name} library — with refinement types and verification
// Profile: research

mount core.base.{{List, Text, Maybe}};

/// A non-empty list — compile-time guarantee via refinement type.
pub type NonEmptyList<T> is List<T> where self.len() > 0;

/// Safe head function — guaranteed non-empty input.
pub fn head<T>(list: &NonEmptyList<T>) -> &T {{
    &list[0]
}}

/// A positive integer.
pub type Positive is Int where self > 0;

/// Division that cannot fail — denominator is always > 0.
pub fn safe_div(a: Int, b: Positive) -> Int {{
    a / b
}}

/// Standard point type.
pub type Point is {{
    x: Float,
    y: Float,
}};

implement Point {{
    pub fn new(x: Float, y: Float) -> Point {{
        Point {{ x, y }}
    }}

    pub fn distance(&self, other: &Point) -> Float {{
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        (dx * dx + dy * dy).sqrt()
    }}
}}
"#,
                name = name
            ),
        };

        fs::write(dir.join("src/lib.vr"), lib_content)?;

        let test_content = format!(
            r#"// Tests for {name}

mount core.base.{{List, Maybe}};

@test
fn test_add() {{
    let result = add(2, 3);
    assert_eq(result, 5, "2 + 3 should equal 5");
}}

@test
fn test_point_distance() {{
    let p1 = Point.new(0.0, 0.0);
    let p2 = Point.new(3.0, 4.0);
    assert_eq(p1.distance(&p2), 5.0, "Distance should be 5.0");
}}

@test
fn test_find_point() {{
    let mut points = List.new();
    points.push(Point.new(1.0, 2.0));
    points.push(Point.new(3.0, 4.0));

    match find_point_by_x(&points, 1.0) {{
        Some(p) => assert_eq(p.x, 1.0, "Found point should have x=1.0"),
        None => panic("Point should be found"),
    }}
}}
"#,
            name = name
        );

        fs::write(dir.join("tests/lib_test.vr"), test_content)?;
        Ok(())
    }
}

pub mod web_api {
    use super::*;

    pub fn create(dir: &Path, _name: &str, _profile: LanguageProfile) -> Result<()> {
        fs::create_dir_all(dir.join("src/routes"))?;

        let main_content = r#"// Web API server

mount core.base.{Text, Map};
mount core.net.http.{Server, Request, Response};

module routes;

fn main() {
    let server = Server.bind("127.0.0.1:8080");

    server.route("/", routes.index);
    server.route("/api/hello", routes.hello);

    print("Listening on http://127.0.0.1:8080");
    server.run();
}
"#;

        fs::write(dir.join("src/main.vr"), main_content)?;

        let routes_content = r#"// API route handlers

mount core.base.{Text, Map};
mount core.net.http.{Request, Response};

pub fn index(req: &Request) -> Response {
    Response.ok("Welcome to Verum API")
}

pub fn hello(req: &Request) -> Response {
    let name = req.query("name").unwrap_or("World");

    let mut data: Map<Text, Text> = Map.new();
    data.insert("message", f"Hello, {name}!");

    Response.json(data)
}
"#;

        fs::write(dir.join("src/routes/mod.vr"), routes_content)?;

        let test_content = r#"// API tests

mount core.net.http.{Request, Response};

@test
fn test_index_route() {
    let req = Request.get("/");
    let res = routes.index(&req);
    assert_eq(res.status(), 200, "Index should return 200");
}

@test
fn test_hello_route() {
    let req = Request.get("/api/hello?name=Verum");
    let res = routes.hello(&req);
    assert(res.body().contains("Hello, Verum!"), "Should contain greeting");
}
"#;

        fs::write(dir.join("tests/api_test.vr"), test_content)?;
        Ok(())
    }
}

pub mod cli_app {
    use super::*;

    pub fn create(dir: &Path, name: &str, _profile: LanguageProfile) -> Result<()> {
        let main_content = format!(
            r#"// {name} — command-line application

mount core.base.{{Text, List, Maybe}};
mount core.sys.env;

type Cli is {{
    verbose: Bool,
    input: Maybe<Text>,
}};

implement Cli {{
    fn parse(args: &List<Text>) -> Cli {{
        let mut verbose = false;
        let mut input: Maybe<Text> = None;
        let mut i = 1;  // skip program name

        while i < args.len() {{
            match args[i].as_str() {{
                "--verbose" | "-v" => verbose = true,
                "--input" | "-i" => {{
                    if i + 1 < args.len() {{
                        input = Some(args[i + 1].clone());
                        i += 1;
                    }}
                }}
                _ => (),
            }}
            i += 1;
        }}

        Cli {{ verbose, input }}
    }}
}}

fn main() {{
    let cli = Cli.parse(&env.args());

    if cli.verbose {{
        print("{name} v0.1.0");
    }}

    match cli.input {{
        Some(file) => {{
            print(f"Processing: {{file}}");
        }}
        None => {{
            print("Usage: {name} [OPTIONS]");
            print("  -v, --verbose    Verbose output");
            print("  -i, --input FILE Input file");
        }}
    }}
}}
"#,
            name = name
        );

        fs::write(dir.join("src/main.vr"), main_content)?;

        let test_content = format!(
            r#"// Tests for {name} CLI

mount core.base.{{List, Text}};

@test
fn test_cli_parse_verbose() {{
    let args: List<Text> = ["{name}", "--verbose"];
    let cli = Cli.parse(&args);
    assert(cli.verbose, "Should parse --verbose flag");
}}

@test
fn test_cli_parse_input() {{
    let args: List<Text> = ["{name}", "--input", "file.txt"];
    let cli = Cli.parse(&args);
    assert_eq(cli.input, Some("file.txt"), "Should parse --input");
}}
"#,
            name = name
        );

        fs::write(dir.join("tests/cli_test.vr"), test_content)?;
        Ok(())
    }
}
