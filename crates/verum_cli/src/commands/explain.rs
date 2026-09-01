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
    // Normalize the code, handling it with or without its letter.
    //
    // `W` belongs here as much as `E` does. Until T1035 the registry held
    // no warning at all, so this branch had nothing to look up and the
    // omission was invisible; with warnings registered, `verum explain
    // W0319` still failed — the bare-number arm turned it into "EW0319",
    // a code no table can hold. A warning the compiler shows and the user
    // cannot look up is exactly as unhelpful as an error they cannot look
    // up, which is the whole reason this command exists.
    let first = code.chars().next().map(|c| c.to_ascii_uppercase());
    let normalized_code = if matches!(first, Some('E') | Some('W')) {
        code.to_uppercase()
    } else {
        format!("E{}", code)
    };

    // Try to get the explanation
    if let Some(explanation) = get_explanation(&normalized_code) {
        let rendered = render_explanation(explanation, !no_color);
        println!("{}", rendered);
        Ok(())
    } else if let Some(entry) = verum_error::registry::lookup(&normalized_code) {
        // The explanation TABLE and the codes the compiler actually PRINTS
        // were, until this fallback existed, disjoint sets. The table holds
        // twenty-seven four-digit codes; the compiler emits sixty-nine, most
        // of them three-digit. So `verum explain E400` — a type mismatch,
        // about the most common diagnostic there is — answered "not found",
        // and every code a user was likely to have in front of them did the
        // same. The registry knows all of them. A one-line description is
        // less than the hand-written explanations offer, but it is what the
        // compiler can honestly say about every code it can produce, and it
        // cannot drift: the same table the diagnostic was checked against
        // answers here.
        println!("{}", normalized_code.bold());
        println!();
        println!("  {}", entry.description);
        println!();
        println!(
            "  {} {}",
            "category:".bright_black(),
            entry.category.label()
        );
        println!();
        println!(
            "{}",
            "This code has no long-form explanation yet — the line above is its\n\
             registry entry. `verum explain` shows a worked example, causes and\n\
             fixes for codes that have one."
                .bright_black()
        );
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

        // Show available codes — from BOTH tables. Listing only the
        // explanation table told a user who mistyped that twenty-seven
        // codes exist, while omitting every code the compiler had just
        // printed at them.
        eprintln!("{}", "Available error codes:".cyan());
        let mut codes = list_error_codes();
        for registered in verum_error::registry::REGISTRY.keys() {
            let registered = verum_common::Text::from(*registered);
            if !codes.contains(&registered) {
                codes.push(registered);
            }
        }
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
        let usage_msg: &str = "Usage: verum explain E400";
        eprintln!("{}", usage_msg.bright_black());

        // Return the failure rather than calling `std::process::exit(1)`
        // here. `main` already maps an Err to a non-zero exit
        // (`process::exit(e.exit_code())`), so the exit behaviour is
        // unchanged — but exiting from inside made this branch impossible
        // to test: `process::exit` in a unit test kills the whole test
        // binary, not just the case. That is why the module's tests only
        // ever exercised codes that RESOLVE, and why nobody noticed that
        // `verum explain E400` — a code the compiler prints constantly —
        // answered "not found" for as long as it did.
        Err(crate::error::CliError::Custom(format!(
            "unknown error code '{normalized_code}'"
        )))
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

    /// The codes a user actually sees must be explainable.
    ///
    /// Each of these is emitted by the compiler and NONE of them is in the
    /// hand-written explanation table — they resolve through the registry.
    /// Before that fallback existed every one of them printed "Error code
    /// not found", which is the worst possible answer: the diagnostic told
    /// the user to look the code up, and the lookup denied the code exists.
    #[test]
    fn codes_the_compiler_actually_prints_are_explainable() {
        for code in ["E100", "E400", "E404", "E102", "E305", "E600"] {
            assert!(
                execute(code, true).is_ok(),
                "`verum explain {code}` failed, but the compiler can print {code}"
            );
        }
    }

    /// The cases the six-code list above did not reach.
    ///
    /// That list is hand-written, and being hand-written it missed every
    /// code that was actually broken. `E0319` is printed 77 times across
    /// 24 `core/` files and answered "not found"; no `W` code could be
    /// looked up at all, because the registry held none AND the
    /// normaliser above turned `W0319` into `EW0319` (T1035).
    ///
    /// Exhaustive coverage is enforced elsewhere — `verum_error`'s
    /// `registry_covers_every_emitted_code` scans every code-shaped
    /// literal in the workspace — so what these add is the END-TO-END
    /// path: registry entry present AND reachable through this command.
    #[test]
    fn warnings_and_four_digit_codes_are_explainable_too() {
        for code in ["E0319", "E0601", "E0000", "E430"] {
            assert!(
                execute(code, true).is_ok(),
                "`verum explain {code}` failed, but the compiler prints {code}"
            );
        }
        for code in ["W0319", "W0500", "W0601", "W1003", "W502", "W001"] {
            assert!(
                execute(code, true).is_ok(),
                "`verum explain {code}` failed — a warning a user was just \
                 shown must be lookupable"
            );
        }
    }

    /// Lower case reaches the same entry, for both letters.
    #[test]
    fn the_letter_is_case_insensitive_for_warnings_as_well_as_errors() {
        assert!(execute("e400", true).is_ok());
        assert!(execute("w0319", true).is_ok());
    }

    /// And a code that exists in NEITHER table is still an error — the
    /// fallback must not turn every string into a plausible answer.
    #[test]
    fn a_code_in_no_table_is_reported_as_unknown() {
        assert!(
            execute("E999", true).is_err(),
            "E999 is in neither table; reporting it as explainable would make \
             the command useless as a check"
        );
        assert!(execute("E12345", true).is_err(), "not a well-formed code");
    }
}
