//! Error code explanation command.
//!
//! Provides detailed explanations for Verum compiler error codes.
//! Usage: verum explain E0312

use crate::error::Result;
use crate::ui;
use colored::Colorize;
use verum_diagnostics::{get_explanation, list_error_codes, render_explanation, search_errors};

/// Execute the explain command
pub fn execute(code: &str, no_color: bool) -> Result<()> {
    // Normalize error code (handle with or without 'E' prefix)
    let normalized_code = if code.starts_with('E') || code.starts_with('e') {
        code.to_uppercase()
    } else {
        format!("E{}", code)
    };

    // Try to get the explanation
    if let Some(explanation) = get_explanation(&normalized_code) {
        let rendered = render_explanation(explanation, !no_color);
        println!("{}", rendered);
        Ok(())
    } else {
        // Error code not found - show helpful message
        eprintln!(
            "{} Error code '{}' not found",
            "Error:".red().bold(),
            normalized_code
        );
        eprintln!();

        // Try to search for similar codes
        let search_results = search_errors(&code.to_lowercase());
        if !search_results.is_empty() {
            eprintln!("{}", "Did you mean one of these?".yellow());
            for result_code in search_results.iter().take(5) {
                eprintln!("  • {}", result_code.as_str().green());
            }
            eprintln!();
        }

        // Show available codes
        eprintln!("{}", "Available error codes:".cyan());
        let mut codes = list_error_codes();
        codes.sort();

        // Group by category (first 2 digits after E)
        let mut current_category = String::new();
        for error_code in codes {
            // Extract category (e.g., "03" from "E0312")
            let error_code_str: &str = error_code.as_str();
            let category: String = if error_code_str.len() >= 4 {
                error_code_str[1..3].to_string()
            } else {
                String::new()
            };

            if category != current_category {
                if !current_category.is_empty() {
                    eprintln!();
                }
                current_category = category.clone();

                let category_name = match category.as_str() {
                    "02" => "Try Operator Errors",
                    "03" => "Context & Type Errors",
                    _ => "Other Errors",
                };
                eprintln!("  {}", category_name.bold());
            }

            let error_code_str: &str = error_code.as_str();
            eprintln!("    {}", error_code_str.green());
        }

        eprintln!();
        let usage_msg: &str = "Usage: verum explain E0312";
        eprintln!("{}", usage_msg.bright_black());

        std::process::exit(1);
    }
}

/// List all error codes with brief descriptions
pub fn list_all() -> Result<()> {
    ui::info("Available Verum Error Codes");
    println!();

    let mut codes = list_error_codes();
    codes.sort();

    let mut current_category = String::new();

    for code in codes {
        if let Some(explanation) = get_explanation(&code) {
            // Extract category
            let code_str: &str = code.as_str();
            let category: String = if code_str.len() >= 4 {
                code_str[1..3].to_string()
            } else {
                String::new()
            };

            if category != current_category {
                if !current_category.is_empty() {
                    println!();
                }
                current_category = category.clone();

                let category_name = match category.as_str() {
                    "02" => "Try Operator Errors (E0203-E0205)",
                    "03" => "Context & Type Errors (E0301-E0320)",
                    _ => "Other Errors",
                };
                println!("{}", category_name.bold().cyan());
                println!("{}", "─".repeat(50).cyan());
            }

            let code_str: &str = code.as_str();
            println!("  {} - {}", code_str.green().bold(), explanation.title);
        }
    }

    println!();
    let info_msg: &str = "Use 'verum explain <CODE>' for detailed information";
    println!("{}", info_msg.bright_black());

    Ok(())
}

/// Search for error codes by keyword
pub fn search(keyword: &str) -> Result<()> {
    let results = search_errors(keyword);

    if results.is_empty() {
        eprintln!(
            "{} No error codes found matching '{}'",
            "Info:".yellow(),
            keyword
        );
        eprintln!();
        let try_msg: &str = "Try searching for:";
        eprintln!("{}", try_msg.bright_black());
        eprintln!("  • refinement");
        eprintln!("  • context");
        eprintln!("  • array");
        eprintln!("  • overflow");
        eprintln!("  • division");
        return Ok(());
    }

    ui::info(&format!("Error codes matching '{}':", keyword));
    println!();

    for code in results {
        if let Some(explanation) = get_explanation(&code) {
            let code_str: &str = code.as_str();
            println!("  {} - {}", code_str.green().bold(), explanation.title);
        }
    }

    println!();
    let info_msg: &str = "Use 'verum explain <CODE>' for detailed information";
    println!("{}", info_msg.bright_black());

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_execute_with_valid_code() {
        let result = execute("E0312", true);
        assert!(result.is_ok());
    }

    #[test]
    fn test_execute_with_code_without_e() {
        let result = execute("0312", true);
        assert!(result.is_ok());
    }

    #[test]
    fn test_list_all() {
        let result = list_all();
        assert!(result.is_ok());
    }

    #[test]
    fn test_search_refinement() {
        let result = search("refinement");
        assert!(result.is_ok());
    }
}
