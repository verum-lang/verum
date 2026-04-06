// Doc command - generate HTML documentation from doc comments (/// and //!).
// Extracts documentation from source files and renders as browsable HTML.

use crate::config::Config;
use crate::error::{CliError, Result};
use crate::ui;
use colored::Colorize;
use std::fs;
use std::path::{Path, PathBuf};
use verum_common::{List, Text};
use walkdir::WalkDir;

/// Documentation output format
#[derive(Debug, Clone, Copy)]
enum DocFormat {
    Html,
    Markdown,
    Json,
}

/// Documentation metadata
struct DocMetadata {
    title: Text,
    version: Text,
    description: Option<Text>,
    authors: List<Text>,
}

/// Function documentation with cost annotations.
#[derive(Clone)]
struct FunctionDoc {
    name: Text,
    signature: Text,
    description: Text,
    /// CBGR cost: "~15ns per check"
    cbgr_cost: Option<Text>,
    /// Verification status: "Proven" | "Runtime" | "Unverified"
    verification_status: Text,
    /// Time complexity: "O(n)", "O(log n)", etc.
    time_complexity: Option<Text>,
    /// Space complexity: "O(1)", "O(n)", etc.
    space_complexity: Option<Text>,
    /// Performance characteristics
    performance_notes: List<Text>,
    examples: List<Text>,
}

pub fn execute(open: bool, document_private_items: bool, _no_deps: bool, format: &str) -> Result<()> {
    let _doc_format = match format {
        "markdown" | "md" => DocFormat::Markdown,
        "json" => DocFormat::Json,
        _ => DocFormat::Html,
    };

    ui::step(&format!("Generating Verum documentation ({})", format));

    let config = Config::load(".")?;
    let output_dir = PathBuf::from("target/doc");

    // Create output directory
    fs::create_dir_all(&output_dir)?;

    let metadata = DocMetadata {
        title: config.cog.name.clone(),
        version: config.cog.version.clone(),
        description: config.cog.description.clone(),
        authors: config.cog.authors.clone(),
    };

    ui::step(&format!(
        "Documenting {} v{}",
        metadata.title, metadata.version
    ));

    // Find all source files
    let src_path = PathBuf::from("src");
    if !src_path.exists() {
        return Err(CliError::Custom("src/ directory not found".into()));
    }

    let mut documented_files = 0;
    let mut total_functions = 0;

    for entry in WalkDir::new(&src_path)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.is_file() && is_verum_file(path) {
            match document_file(path, &output_dir, document_private_items) {
                Ok(func_count) => {
                    documented_files += 1;
                    total_functions += func_count;
                }
                Err(e) => {
                    ui::warn(&format!("Failed to document {}: {}", path.display(), e));
                }
            }
        }
    }

    // Generate index page
    generate_index(&output_dir, &metadata, documented_files, total_functions)?;

    // Generate CSS
    generate_styles(&output_dir)?;

    println!();
    ui::success(&format!(
        "Documented {} files ({} functions)",
        documented_files, total_functions
    ));

    let index_path = output_dir.join("index.html");
    println!();
    println!("Documentation: {}", index_path.display().to_string().cyan());

    if open {
        ui::step("Opening documentation in browser");
        open_in_browser(&index_path)?;
    }

    Ok(())
}

/// Check if file is a Verum source file (.vr).
fn is_verum_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext == "vr")
        .unwrap_or(false)
}

/// Document a single file
fn document_file(path: &Path, output_dir: &Path, document_private: bool) -> Result<usize> {
    let content = fs::read_to_string(path)?;

    // Use verum_parser to extract documentation with full AST analysis
    let functions =
        extract_functions_from_ast(&content, document_private, path).unwrap_or_else(|_| {
            // Fallback to pattern matching if parsing fails
            extract_functions(&content, document_private)
        });

    if functions.is_empty() {
        return Ok(0);
    }

    // Generate HTML documentation
    let relative_path = path.strip_prefix("src").unwrap_or(path);
    let doc_path = output_dir.join(relative_path).with_extension("html");

    if let Some(parent) = doc_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let html = generate_file_doc(path, &functions)?;
    fs::write(&doc_path, html)?;

    Ok(functions.len())
}

/// Extract function documentation from AST using verum_parser
fn extract_functions_from_ast(
    content: &str,
    include_private: bool,
    _path: &Path,
) -> Result<List<FunctionDoc>> {
    use verum_ast::{FileId, ItemKind, Visibility, decl::FunctionParamKind};
    use verum_lexer::Lexer;
    use verum_parser::VerumParser;

    let file_id = FileId::new(0);
    let lexer = Lexer::new(content, file_id);
    let parser = VerumParser::new();

    let module = parser
        .parse_module(lexer, file_id)
        .map_err(|errors| CliError::Custom(format!("Parse error: {:?}", errors.first())))?;

    let mut functions = List::new();

    for item in &module.items {
        if let ItemKind::Function(ref func) = item.kind {
            // Check visibility on function (not item)
            let is_public = matches!(func.visibility, Visibility::Public);
            if !is_public && !include_private {
                continue;
            }

            // Extract documentation from attributes
            let mut description = Text::new();
            let mut cbgr_cost = None;
            let mut time_complexity = None;
            let mut space_complexity = None;
            let mut performance_notes = List::new();
            let mut examples = List::new();
            let mut verification_status = Text::from("Unverified");

            // Process doc attributes
            for attr in &item.attributes {
                let attr_name = attr.name.as_str();

                if attr_name == "doc" {
                    if let Some(ref args) = attr.args {
                        description.push_str(&format!("{:?}", args));
                    }
                } else if attr_name == "verify" || attr_name == "proven" {
                    verification_status = Text::from("Proven");
                } else if attr_name == "cost" {
                    cbgr_cost = Some(Text::from("~15ns per CBGR check"));
                } else if attr_name == "time" {
                    if let Some(ref args) = attr.args {
                        time_complexity = Some(format!("{:?}", args).into());
                    }
                } else if attr_name == "space" {
                    if let Some(ref args) = attr.args {
                        space_complexity = Some(format!("{:?}", args).into());
                    }
                } else if attr_name == "perf" || attr_name == "performance" {
                    if let Some(ref args) = attr.args {
                        performance_notes.push(format!("{:?}", args).into());
                    }
                } else if attr_name == "example"
                    && let Some(ref args) = attr.args
                {
                    examples.push(format!("{:?}", args).into());
                }
            }

            // Build function signature from AST
            let mut signature = Text::new();
            if is_public {
                signature.push_str("pub ");
            }
            signature.push_str("fn ");
            signature.push_str(func.name.as_str());
            signature.push_str("(");

            // Add parameters - handle FunctionParamKind enum
            let params: Vec<String> = func
                .params
                .iter()
                .map(|p| match &p.kind {
                    FunctionParamKind::Regular { pattern, ty, .. } => {
                        format!("{:?}: {:?}", pattern, ty)
                    }
                    FunctionParamKind::SelfValue => "self".to_string(),
                    FunctionParamKind::SelfValueMut => "mut self".to_string(),
                    FunctionParamKind::SelfRef => "&self".to_string(),
                    FunctionParamKind::SelfRefMut => "&mut self".to_string(),
                    FunctionParamKind::SelfOwn => "%self".to_string(),
                    FunctionParamKind::SelfOwnMut => "%mut self".to_string(),
                    FunctionParamKind::SelfRefChecked => "&checked self".to_string(),
                    FunctionParamKind::SelfRefCheckedMut => "&checked mut self".to_string(),
                    FunctionParamKind::SelfRefUnsafe => "&unsafe self".to_string(),
                    FunctionParamKind::SelfRefUnsafeMut => "&unsafe mut self".to_string(),
                })
                .collect();
            signature.push_str(&params.join(", "));

            signature.push_str(")");

            // Add return type if present (Option type in AST)
            if let Some(ref ret_ty) = func.return_type {
                signature.push_str(&format!(" -> {:?}", ret_ty));
            }

            // Check for CBGR-related annotations
            let has_cbgr = func.params.iter().any(|p| match &p.kind {
                FunctionParamKind::SelfRef | FunctionParamKind::SelfRefMut => true,
                FunctionParamKind::Regular { ty, .. } => {
                    let type_str = format!("{:?}", ty);
                    type_str.contains("Ref") || type_str.contains("&")
                }
                _ => false,
            });

            if has_cbgr && cbgr_cost.is_none() {
                cbgr_cost = Some(Text::from("~15ns per CBGR check"));
            }

            // Check for pure/no_escape annotations
            let is_pure = item
                .attributes
                .iter()
                .any(|attr| attr.name.as_str() == "pure" || attr.name.as_str() == "no_escape");

            if is_pure && cbgr_cost.is_some() {
                cbgr_cost = Some(Text::from("0ns (optimized)"));
            }

            functions.push(FunctionDoc {
                name: Text::from(func.name.as_str()),
                signature,
                description,
                cbgr_cost,
                verification_status,
                time_complexity,
                space_complexity,
                performance_notes,
                examples,
            });
        }
    }

    Ok(functions)
}

/// Extract function documentation from source (fallback pattern matching)
fn extract_functions(content: &str, include_private: bool) -> List<FunctionDoc> {
    let mut functions = List::new();
    let lines: List<&str> = content.lines().collect();

    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim();

        // Match function declarations
        if trimmed.starts_with("pub fn ") || (include_private && trimmed.starts_with("fn ")) {
            // Extract function signature
            let signature = Text::from(trimmed);
            let name = extract_function_name(trimmed);

            // Look back for documentation comments
            let mut description = Text::new();
            let mut cbgr_cost = None;
            let mut time_complexity = None;
            let mut space_complexity = None;
            let mut performance_notes = List::new();
            let mut examples = List::new();
            let mut verification_status = Text::from("Unverified");

            for i in (0..idx).rev().take(20) {
                let doc_line = lines[i].trim();

                if doc_line.starts_with("///") {
                    let content = doc_line.trim_start_matches("///").trim();

                    if content.starts_with("Cost:") || content.starts_with("CBGR:") {
                        cbgr_cost = Some(Text::from(content));
                    } else if content.starts_with("Time:") {
                        time_complexity = Some(Text::from(content));
                    } else if content.starts_with("Space:") {
                        space_complexity = Some(Text::from(content));
                    } else if content.starts_with("Performance:") {
                        performance_notes.push(Text::from(content));
                    } else if content.starts_with("Example:") || content.starts_with("```") {
                        examples.push(Text::from(content));
                    } else if content.starts_with("Verified:") || content.starts_with("@verify") {
                        verification_status = Text::from("Proven");
                    } else if !content.is_empty() {
                        if !description.is_empty() {
                            description.push_str(" ");
                        }
                        description.push_str(content);
                    }
                } else if doc_line.starts_with("//") {
                    // Skip regular comments
                } else if !doc_line.is_empty() {
                    break;
                }
            }

            functions.push(FunctionDoc {
                name,
                signature,
                description,
                cbgr_cost,
                verification_status,
                time_complexity,
                space_complexity,
                performance_notes,
                examples,
            });
        }
    }

    functions
}

/// Extract function name from signature
fn extract_function_name(signature: &str) -> Text {
    signature
        .trim_start_matches("pub ")
        .trim_start_matches("fn ")
        .split('(')
        .next()
        .unwrap_or("unknown")
        .trim()
        .into()
}

/// Generate HTML documentation for a file
fn generate_file_doc(path: &Path, functions: &[FunctionDoc]) -> Result<Text> {
    let mut html = Text::new();

    html.push_str("<!DOCTYPE html>\n");
    html.push_str("<html>\n");
    html.push_str("<head>\n");
    html.push_str("  <meta charset=\"utf-8\">\n");
    html.push_str(&format!("  <title>{}</title>\n", path.display()));
    html.push_str("  <link rel=\"stylesheet\" href=\"../styles.css\">\n");
    html.push_str("</head>\n");
    html.push_str("<body>\n");

    html.push_str(&format!("<h1>Module: {}</h1>\n", path.display()));

    for func in functions {
        html.push_str("<div class=\"function-doc\">\n");

        // Function signature
        html.push_str(&format!("<h2 id=\"{}\">{}</h2>\n", func.name, func.name));
        html.push_str(&format!(
            "<pre class=\"signature\">{}</pre>\n",
            func.signature
        ));

        // Verification badge
        let badge_class = match func.verification_status.as_str() {
            "Proven" => "badge-proven",
            "Runtime" => "badge-runtime",
            _ => "badge-unverified",
        };
        html.push_str(&format!(
            "<span class=\"badge {}\">{}</span>\n",
            badge_class, func.verification_status
        ));

        // Description
        if !func.description.is_empty() {
            html.push_str(&format!("<p>{}</p>\n", func.description));
        }

        // Cost information
        html.push_str("<div class=\"cost-info\">\n");

        if let Some(ref cbgr) = func.cbgr_cost {
            html.push_str(&format!(
                "<div class=\"cost-item\"><strong>CBGR Cost:</strong> {}</div>\n",
                cbgr
            ));
        }

        if let Some(ref time) = func.time_complexity {
            html.push_str(&format!(
                "<div class=\"cost-item\"><strong>Time Complexity:</strong> {}</div>\n",
                time
            ));
        }

        if let Some(ref space) = func.space_complexity {
            html.push_str(&format!(
                "<div class=\"cost-item\"><strong>Space Complexity:</strong> {}</div>\n",
                space
            ));
        }

        html.push_str("</div>\n");

        // Performance notes
        if !func.performance_notes.is_empty() {
            html.push_str("<div class=\"performance-notes\">\n");
            html.push_str("<h4>Performance Characteristics</h4>\n");
            html.push_str("<ul>\n");
            for note in &func.performance_notes {
                html.push_str(&format!("<li>{}</li>\n", note));
            }
            html.push_str("</ul>\n");
            html.push_str("</div>\n");
        }

        // Examples
        if !func.examples.is_empty() {
            html.push_str("<div class=\"examples\">\n");
            html.push_str("<h4>Examples</h4>\n");
            for example in &func.examples {
                html.push_str(&format!("<pre>{}</pre>\n", example));
            }
            html.push_str("</div>\n");
        }

        html.push_str("</div>\n");
    }

    html.push_str("</body>\n");
    html.push_str("</html>\n");

    Ok(html)
}

/// Generate index page
fn generate_index(
    output_dir: &Path,
    metadata: &DocMetadata,
    file_count: usize,
    func_count: usize,
) -> Result<()> {
    let mut html = Text::new();

    html.push_str("<!DOCTYPE html>\n");
    html.push_str("<html>\n");
    html.push_str("<head>\n");
    html.push_str("  <meta charset=\"utf-8\">\n");
    html.push_str(&format!(
        "  <title>{} - Documentation</title>\n",
        metadata.title
    ));
    html.push_str("  <link rel=\"stylesheet\" href=\"styles.css\">\n");
    html.push_str("</head>\n");
    html.push_str("<body>\n");

    html.push_str(&format!(
        "<h1>{} v{}</h1>\n",
        metadata.title, metadata.version
    ));

    if let Some(ref desc) = metadata.description {
        html.push_str(&format!("<p class=\"description\">{}</p>\n", desc));
    }

    html.push_str("<div class=\"stats\">\n");
    html.push_str(&format!(
        "<div class=\"stat-item\"><strong>Files:</strong> {}</div>\n",
        file_count
    ));
    html.push_str(&format!(
        "<div class=\"stat-item\"><strong>Functions:</strong> {}</div>\n",
        func_count
    ));
    html.push_str("</div>\n");

    html.push_str("<h2>About Verum Documentation</h2>\n");
    html.push_str("<p>This documentation includes comprehensive cost annotations:</p>\n");
    html.push_str("<ul>\n");
    html.push_str("<li><strong>CBGR Cost:</strong> Runtime overhead for reference checks (~15ns per check)</li>\n");
    html.push_str("<li><strong>Verification Status:</strong> Proven (0ns), Runtime checked, or Unverified</li>\n");
    html.push_str("<li><strong>Complexity:</strong> Time and space complexity analysis</li>\n");
    html.push_str("<li><strong>Performance:</strong> Detailed performance characteristics</li>\n");
    html.push_str("</ul>\n");

    html.push_str("</body>\n");
    html.push_str("</html>\n");

    fs::write(output_dir.join("index.html"), html)?;
    Ok(())
}

/// Generate CSS styles
fn generate_styles(output_dir: &Path) -> Result<()> {
    let css = r#"
body {
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif;
    line-height: 1.6;
    max-width: 1200px;
    margin: 0 auto;
    padding: 20px;
    color: #333;
}

h1 {
    color: #2c3e50;
    border-bottom: 2px solid #3498db;
    padding-bottom: 10px;
}

h2 {
    color: #34495e;
    margin-top: 30px;
}

.function-doc {
    background: #f8f9fa;
    border: 1px solid #dee2e6;
    border-radius: 8px;
    padding: 20px;
    margin: 20px 0;
}

.signature {
    background: #2c3e50;
    color: #ecf0f1;
    padding: 15px;
    border-radius: 5px;
    overflow-x: auto;
}

.badge {
    display: inline-block;
    padding: 4px 12px;
    border-radius: 12px;
    font-size: 12px;
    font-weight: bold;
    margin: 10px 0;
}

.badge-proven {
    background: #27ae60;
    color: white;
}

.badge-runtime {
    background: #f39c12;
    color: white;
}

.badge-unverified {
    background: #95a5a6;
    color: white;
}

.cost-info {
    background: #fff3cd;
    border-left: 4px solid #ffc107;
    padding: 15px;
    margin: 15px 0;
}

.cost-item {
    margin: 5px 0;
}

.performance-notes {
    background: #d1ecf1;
    border-left: 4px solid #17a2b8;
    padding: 15px;
    margin: 15px 0;
}

.examples {
    background: #d4edda;
    border-left: 4px solid #28a745;
    padding: 15px;
    margin: 15px 0;
}

.stats {
    background: #e7f3ff;
    border: 1px solid #b3d9ff;
    border-radius: 5px;
    padding: 15px;
    margin: 20px 0;
}

.stat-item {
    display: inline-block;
    margin-right: 30px;
}

.description {
    font-size: 18px;
    color: #555;
}
"#;

    fs::write(output_dir.join("styles.css"), css)?;
    Ok(())
}

/// Open documentation in browser
fn open_in_browser(path: &Path) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(path)
            .spawn()
            .map_err(|e| CliError::Custom(format!("Failed to open browser: {}", e)))?;
    }

    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map_err(|e| CliError::Custom(format!("Failed to open browser: {}", e)))?;
    }

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(&["/C", "start", path.to_str().unwrap_or("")])
            .spawn()
            .map_err(|e| CliError::Custom(format!("Failed to open browser: {}", e)))?;
    }

    Ok(())
}
