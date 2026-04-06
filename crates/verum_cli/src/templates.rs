// Project templates for Verum
// Uses semantic types from verum_common (List, Text, Map, Set, Maybe)
// Provides scaffolding for different project types

use crate::error::Result;
use std::fs;
use std::path::Path;

pub mod binary {
    use super::*;

    pub fn create(dir: &Path, _name: &str) -> Result<()> {
        let main_content = r#"// Verum application
// Spec: v6.0-BALANCED - Use semantic types from verum_std

// Imports use dots (.) not double colons (::)
import std.io.stdio.println;
import core.base.list.List;
import core.base.text.Text;

fn main() {
    print(f"Hello, Verum!");
    print(f"Semantic honesty: CBGR overhead ~15ns per reference check");
    print(f"Tier 0: Instant compilation, interpreted execution");
}
"#;

        fs::write(dir.join("src/main.vr"), main_content)?;

        let test_content = r#"// Tests for main
import std.test.*;

#[test]
fn test_example() {
    assert(true, "Example test always passes");
}
"#;

        fs::write(dir.join("tests/main_test.vr"), test_content)?;
        Ok(())
    }
}

pub mod library {
    use super::*;

    pub fn create(dir: &Path, name: &str) -> Result<()> {
        let lib_content = format!(
            r#"// {name} library
// Spec: v6.0-BALANCED - Semantic types: List, Text, Map, Set, Maybe

// Imports use dots (.) not double colons (::)
import core.base.list.List;
import core.base.text.Text;
import core.base.maybe.Maybe;

/// Example function that adds two numbers
/// Time complexity: O(1)
/// CBGR overhead: 0ns (no references)
pub fn add(a: Int, b: Int) -> Int {{
    a + b
}}

/// Example type demonstrating semantic types
/// Use 'type X is {{ ... }}' syntax (not 'struct')
pub type Point is {{
    x: Float,
    y: Float,
}};

/// Implementation block uses 'implement' keyword (not 'impl')
implement Point {{
    /// Create a new point
    /// CBGR overhead: ~15ns (managed reference return)
    pub fn new(x: Float, y: Float) -> Point {{
        Point {{ x, y }}
    }}

    /// Calculate distance between two points
    /// CBGR overhead: ~30ns (2 managed references)
    pub fn distance(self: &Self, other: &Point) -> Float {{
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        (dx * dx + dy * dy).sqrt()
    }}
}}

/// Example using semantic List type (NOT Vec!)
pub type Path is {{
    points: List<Point>,
}};

implement Path {{
    pub fn new() -> Path {{
        Path {{ points: List.new() }}
    }}

    pub fn add_point(self: &mut Self, point: Point) {{
        self.points.push(point);
    }}

    /// Calculate total path length
    pub fn total_length(self: &Self) -> Float {{
        let mut total = 0.0;
        for i in 0..self.points.len() - 1 {{
            let p1 = &self.points[i];
            let p2 = &self.points[i + 1];
            total += p1.distance(p2);
        }}
        total
    }}
}}

/// Example using Maybe type (NOT Option!)
pub fn find_point_by_x(points: &List<Point>, x: Float) -> Maybe<&Point> {{
    for point in points {{
        if point.x == x {{
            return Maybe.Some(point);
        }}
    }}
    Maybe.None
}}
"#,
            name = name
        );

        fs::write(dir.join("src/lib.vr"), lib_content)?;

        let test_content = r#"// Library tests
import std.test.*;
import core.base.list.List;
import core.base.maybe.Maybe;

#[test]
fn test_add() {
    let result = add(2, 3);
    assert_eq(result, 5, "2 + 3 should equal 5");
}

#[test]
fn test_point_distance() {
    let p1 = Point.new(0.0, 0.0);
    let p2 = Point.new(3.0, 4.0);
    let dist = p1.distance(&p2);
    assert_eq(dist, 5.0, "Distance should be 5.0");
}

#[test]
fn test_path_length() {
    let mut path = Path.new();
    path.add_point(Point.new(0.0, 0.0));
    path.add_point(Point.new(3.0, 4.0));
    path.add_point(Point.new(6.0, 8.0));

    let length = path.total_length();
    assert_eq(length, 10.0, "Total path length should be 10.0");
}

#[test]
fn test_maybe_type() {
    let mut points = List.new();
    points.push(Point.new(1.0, 2.0));
    points.push(Point.new(3.0, 4.0));

    let found = find_point_by_x(&points, 1.0);
    match found {
        Maybe.Some(p) => assert_eq(p.x, 1.0, "Found point should have x=1.0"),
        Maybe.None => panic("Point should be found"),
    }
}
"#;

        fs::write(dir.join("tests/lib_test.vr"), test_content)?;
        Ok(())
    }
}

pub mod web_api {
    use super::*;

    pub fn create(dir: &Path, _name: &str) -> Result<()> {
        fs::create_dir_all(dir.join("src/routes"))?;

        let main_content = r#"// Web API server
// Spec: v6.0-BALANCED - Use semantic types

// Imports use dots (.) not double colons (::)
import std.io.stdio.println;
import core.base.text.Text;
import core.base.map.Map;
import std.network.http2.*;

// Module declaration (not 'mod')
module routes;

fn main() {
    let server = Server.new("127.0.0.1:8080");

    server.route("/", routes.index);
    server.route("/api/hello", routes.hello);

    print(f"Server running on http://127.0.0.1:8080");
    print(f"Performance: Tier 2 AOT, 85-95% native speed");
    print(f"CBGR overhead: ~15ns per request reference");

    server.run();
}
"#;

        fs::write(dir.join("src/main.vr"), main_content)?;

        let routes_content = r#"// API routes
// Spec: v6.0-BALANCED - Use Text and Map (NOT String and HashMap!)

// Imports use dots (.) not double colons (::)
import std.network.http2.*;
import core.base.text.Text;
import core.base.map.Map;

pub fn index(req: Request) -> Response {
    Response.ok("Welcome to Verum API")
}

pub fn hello(req: Request) -> Response {
    let name = req.query("name").unwrap_or("World");

    // Use Map, not HashMap!
    let mut data = Map.new();
    data.insert("message", f"Hello, {name}!");
    data.insert("cbgr_overhead", "~15ns per check");
    data.insert("tier", "2 (AOT)");

    Response.json(data)
}
"#;

        fs::write(dir.join("src/routes/mod.vr"), routes_content)?;

        let test_content = r#"// API tests
import std.test.*;
import std.network.http2.*;

#[test]
fn test_index_route() {
    let req = Request.get("/");
    let res = routes.index(req);
    assert_eq(res.status(), 200, "Index should return 200 OK");
}

#[test]
fn test_hello_route() {
    let req = Request.get("/api/hello?name=Verum");
    let res = routes.hello(req);
    assert(res.body().contains("Hello, Verum!"), "Response should contain greeting");
}
"#;

        fs::write(dir.join("tests/api_test.vr"), test_content)?;
        Ok(())
    }
}

pub mod cli_app {
    use super::*;

    pub fn create(dir: &Path, name: &str) -> Result<()> {
        let main_content = format!(
            r#"// Command-line application
// Spec: v6.0-BALANCED - Use semantic types

// Imports use dots (.) not double colons (::)
import core.base.text.Text;
import core.base.list.List;
import core.base.maybe.Maybe;
import std.io.*;
import std.env;

// Type definition uses 'type X is {{ ... }}'
type Cli is {{
    verbose: Bool,
    input: Maybe<Text>,
}};

// Implementation uses 'implement' keyword
implement Cli {{
    fn parse_args() -> Cli {{
        let args: List<Text> = env.args();
        let mut verbose = false;
        let mut input = Maybe.None;

        let mut i = 0;
        while i < args.len() {{
            match args[i].as_str() {{
                "--verbose" | "-v" => verbose = true,
                "--input" => {{
                    if i + 1 < args.len() {{
                        input = Maybe.Some(args[i + 1].clone());
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
    let cli = Cli.parse_args();

    if cli.verbose {{
        print(f"{name} v1.0.0");
        print(f"Language: Verum v6.0-BALANCED");
        print(f"CBGR overhead: ~15ns per check");
    }}

    match cli.input {{
        Maybe.Some(file) => process_file(&file),
        Maybe.None => {{
            print(f"Usage: {name} [OPTIONS]");
            print(f"  --verbose, -v    Enable verbose output");
            print(f"  --input FILE     Input file to process");
        }}
    }}
}}

fn process_file(path: &Text) {{
    print(f"Processing: {{path}}");
    // Implementation here
}}
"#,
            name = name
        );

        fs::write(dir.join("src/main.vr"), main_content)?;

        let test_content = r#"// CLI tests
import std.test.*;
import core.base.list.List;
import core.base.text.Text;

#[test]
fn test_cli_parsing() {
    // Test CLI argument parsing
    assert(true, "CLI parsing works");
}
"#;

        fs::write(dir.join("tests/cli_test.vr"), test_content)?;
        Ok(())
    }
}
