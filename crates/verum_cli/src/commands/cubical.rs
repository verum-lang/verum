//! `verum cubical` subcommand — typed cubical/HoTT primitive
//! catalogue surface + face-formula validator.

use crate::error::{CliError, Result};
use verum_verification::cubical::{
    CubicalCatalog, CubicalCategory, DefaultCubicalCatalog, FaceFormula,
};

fn validate_format(s: &str) -> Result<()> {
    if s != "plain" && s != "json" && s != "markdown" {
        return Err(CliError::InvalidArgument(format!(
            "--output must be 'plain', 'json', or 'markdown', got '{}'",
            s
        )));
    }
    Ok(())
}

fn parse_category(s: &str) -> Result<CubicalCategory> {
    CubicalCategory::from_name(s).ok_or_else(|| {
        CliError::InvalidArgument(format!(
            "--category must be one of identity / path_ops / induction / transport / composition / glue / universe, got '{}'",
            s
        ))
    })
}

// =============================================================================
// run_primitives — list every catalogue entry
// =============================================================================

pub fn run_primitives(category: Option<&str>, output: &str) -> Result<()> {
    validate_format(output)?;
    let cat_filter = category.map(parse_category).transpose()?;
    let catalog = DefaultCubicalCatalog::new();
    let entries: Vec<_> = catalog
        .entries()
        .into_iter()
        .filter(|e| match cat_filter {
            Some(c) => e.category == c,
            None => true,
        })
        .collect();
    match output {
        "plain" => {
            println!(
                "Cubical primitive catalogue ({}{}):",
                entries.len(),
                if let Some(c) = cat_filter {
                    format!(" in category `{}`", c.name())
                } else {
                    String::new()
                }
            );
            println!();
            println!(
                "  {:<14} {:<14} {}",
                "Name", "Category", "Semantics"
            );
            println!(
                "  {:<14} {:<14} {}",
                "─".repeat(14),
                "─".repeat(14),
                "─".repeat(60)
            );
            for e in &entries {
                println!(
                    "  {:<14} {:<14} {}",
                    e.primitive.name(),
                    e.category.name(),
                    e.semantics.as_str()
                );
            }
            println!();
            println!(
                "Total: {} primitive(s).  Inspect a single entry with `verum cubical explain <name>`.",
                entries.len()
            );
        }
        "json" => {
            let mut out = String::from("{\n");
            out.push_str("  \"schema_version\": 1,\n");
            out.push_str(&format!("  \"count\": {},\n", entries.len()));
            out.push_str("  \"entries\": [\n");
            for (i, e) in entries.iter().enumerate() {
                let body = serde_json::to_string(e).unwrap_or_default();
                out.push_str(&format!(
                    "    {}{}",
                    body,
                    if i + 1 < entries.len() { ",\n" } else { "\n" }
                ));
            }
            out.push_str("  ]\n}");
            println!("{}", out);
        }
        "markdown" => {
            println!("# Cubical primitive catalogue\n");
            println!("| Name | Category | Semantics |");
            println!("|---|---|---|");
            for e in &entries {
                println!(
                    "| `{}` | `{}` | {} |",
                    e.primitive.name(),
                    e.category.name(),
                    e.semantics.as_str()
                );
            }
        }
        _ => unreachable!(),
    }
    Ok(())
}

// =============================================================================
// run_explain — full doc for a single primitive
// =============================================================================

pub fn run_explain(name: &str, output: &str) -> Result<()> {
    validate_format(output)?;
    let catalog = DefaultCubicalCatalog::new();
    let entry = catalog.lookup(name).ok_or_else(|| {
        CliError::InvalidArgument(format!(
            "unknown cubical primitive '{}' — run `verum cubical primitives` for the full inventory",
            name
        ))
    })?;
    match output {
        "plain" => {
            println!("{} — {} category", entry.primitive.name(), entry.category.name());
            println!("{}", "─".repeat(40));
            println!();
            println!("Signature : {}", entry.signature.as_str());
            println!("Semantics : {}", entry.semantics.as_str());
            println!();
            println!("Example:");
            for line in entry.example.as_str().lines() {
                println!("    {}", line);
            }
            if !entry.computation_rules.is_empty() {
                println!();
                println!("Computation rules:");
                let all_rules = catalog.computation_rules();
                for rule_name in &entry.computation_rules {
                    let rule = all_rules.iter().find(|r| r.name.as_str() == rule_name.as_str());
                    match rule {
                        Some(r) => {
                            println!("  • {}", r.name.as_str());
                            println!("      {} ↪ {}", r.lhs.as_str(), r.rhs.as_str());
                        }
                        None => {
                            println!("  • {} (rationale missing)", rule_name.as_str())
                        }
                    }
                }
            }
            println!();
            println!("Doc anchor: #{}", entry.doc_anchor.as_str());
        }
        "json" => {
            let body = serde_json::to_string_pretty(&entry).unwrap_or_default();
            println!("{}", body);
        }
        "markdown" => {
            println!(
                "# `{}` — {} category\n",
                entry.primitive.name(),
                entry.category.name()
            );
            println!("**Signature:** `{}`\n", entry.signature.as_str());
            println!("**Semantics:** {}\n", entry.semantics.as_str());
            println!("**Example:**\n");
            println!("```verum");
            println!("{}", entry.example.as_str());
            println!("```\n");
            if !entry.computation_rules.is_empty() {
                println!("**Computation rules:**\n");
                let all_rules = catalog.computation_rules();
                for rule_name in &entry.computation_rules {
                    if let Some(r) =
                        all_rules.iter().find(|r| r.name.as_str() == rule_name.as_str())
                    {
                        println!(
                            "- `{}` — `{}` ↪ `{}`",
                            r.name.as_str(),
                            r.lhs.as_str(),
                            r.rhs.as_str()
                        );
                    }
                }
            }
        }
        _ => unreachable!(),
    }
    Ok(())
}

// =============================================================================
// run_rules — list every computation rule
// =============================================================================

pub fn run_rules(output: &str) -> Result<()> {
    validate_format(output)?;
    let catalog = DefaultCubicalCatalog::new();
    let rules = catalog.computation_rules();
    match output {
        "plain" => {
            println!("Cubical computation rules ({}):", rules.len());
            println!();
            for r in &rules {
                println!("  {}", r.name.as_str());
                println!("      {} ↪ {}", r.lhs.as_str(), r.rhs.as_str());
                println!("      {}", r.rationale.as_str());
            }
        }
        "json" => {
            let mut out = String::from("{\n");
            out.push_str("  \"schema_version\": 1,\n");
            out.push_str(&format!("  \"count\": {},\n", rules.len()));
            out.push_str("  \"rules\": [\n");
            for (i, r) in rules.iter().enumerate() {
                let body = serde_json::to_string(r).unwrap_or_default();
                out.push_str(&format!(
                    "    {}{}",
                    body,
                    if i + 1 < rules.len() { ",\n" } else { "\n" }
                ));
            }
            out.push_str("  ]\n}");
            println!("{}", out);
        }
        "markdown" => {
            println!("# Cubical computation rules\n");
            println!("| Name | LHS ↪ RHS | Rationale |");
            println!("|---|---|---|");
            for r in &rules {
                println!(
                    "| `{}` | `{}` ↪ `{}` | {} |",
                    r.name.as_str(),
                    r.lhs.as_str(),
                    r.rhs.as_str(),
                    r.rationale.as_str()
                );
            }
        }
        _ => unreachable!(),
    }
    Ok(())
}

// =============================================================================
// run_face — parse + validate a face formula
// =============================================================================

pub fn run_face(formula: &str, output: &str) -> Result<()> {
    validate_format(output)?;
    if formula.trim().is_empty() {
        return Err(CliError::InvalidArgument(
            "face formula must be non-empty".into(),
        ));
    }
    let parsed = FaceFormula::parse(formula).map_err(|e| {
        CliError::InvalidArgument(format!(
            "face formula parse error: {}",
            e.as_str()
        ))
    })?;
    let canonical = parsed.render();
    let vars: Vec<String> = parsed
        .free_variables()
        .iter()
        .map(|v| v.as_str().to_string())
        .collect();
    match output {
        "plain" => {
            println!("Face formula");
            println!("  input         : {}", formula);
            println!("  canonical     : {}", canonical.as_str());
            println!(
                "  free vars     : {}",
                if vars.is_empty() {
                    "(none)".to_string()
                } else {
                    vars.join(", ")
                }
            );
        }
        "json" => {
            let mut out = String::from("{\n");
            out.push_str("  \"schema_version\": 1,\n");
            out.push_str(&format!(
                "  \"input\": \"{}\",\n",
                json_escape(formula)
            ));
            out.push_str(&format!(
                "  \"canonical\": \"{}\",\n",
                json_escape(canonical.as_str())
            ));
            out.push_str("  \"free_variables\": [");
            let parts: Vec<String> = vars.iter().map(|v| format!("\"{}\"", v)).collect();
            out.push_str(&parts.join(", "));
            out.push_str("],\n");
            let parsed_json = serde_json::to_string(&parsed).unwrap_or_default();
            out.push_str(&format!("  \"ast\": {}\n", parsed_json));
            out.push('}');
            println!("{}", out);
        }
        "markdown" => {
            println!("# Face formula\n");
            println!("- **input** — `{}`", formula);
            println!("- **canonical** — `{}`", canonical.as_str());
            println!(
                "- **free variables** — {}",
                if vars.is_empty() {
                    "(none)".to_string()
                } else {
                    vars.iter()
                        .map(|v| format!("`{}`", v))
                        .collect::<Vec<_>>()
                        .join(", ")
                }
            );
        }
        _ => unreachable!(),
    }
    Ok(())
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_format_round_trip() {
        for f in ["plain", "json", "markdown"] {
            assert!(validate_format(f).is_ok());
        }
        assert!(validate_format("yaml").is_err());
    }

    #[test]
    fn parse_category_canonical() {
        for c in [
            "identity",
            "path_ops",
            "induction",
            "transport",
            "composition",
            "glue",
            "universe",
        ] {
            assert!(parse_category(c).is_ok());
        }
        assert!(parse_category("garbage").is_err());
    }

    #[test]
    fn run_primitives_smoke_every_format() {
        for f in ["plain", "json", "markdown"] {
            assert!(run_primitives(None, f).is_ok());
        }
    }

    #[test]
    fn run_primitives_with_category_filter() {
        for c in [
            "identity",
            "path_ops",
            "induction",
            "transport",
            "composition",
            "glue",
            "universe",
        ] {
            assert!(run_primitives(Some(c), "json").is_ok());
        }
    }

    #[test]
    fn run_primitives_rejects_unknown_category() {
        assert!(matches!(
            run_primitives(Some("garbage"), "plain"),
            Err(CliError::InvalidArgument(_))
        ));
    }

    #[test]
    fn run_explain_every_canonical_primitive() {
        let names = [
            "path",
            "path_over",
            "refl",
            "sym",
            "trans",
            "ap",
            "apd",
            "j_rule",
            "transp",
            "coe",
            "subst",
            "hcomp",
            "comp",
            "glue",
            "unglue",
            "equiv",
            "univalence",
        ];
        for n in names {
            assert!(run_explain(n, "plain").is_ok(), "explain {} failed", n);
        }
    }

    #[test]
    fn run_explain_rejects_unknown() {
        assert!(matches!(
            run_explain("garbage", "plain"),
            Err(CliError::InvalidArgument(_))
        ));
    }

    #[test]
    fn run_rules_every_format() {
        for f in ["plain", "json", "markdown"] {
            assert!(run_rules(f).is_ok());
        }
    }

    #[test]
    fn run_face_canonical_inputs() {
        for s in [
            "1",
            "0",
            "i = 0",
            "i = 0 ∧ j = 1",
            "i = 0 ∨ j = 1",
            "(i = 0 ∨ j = 1) ∧ k = 0",
        ] {
            assert!(run_face(s, "json").is_ok(), "face {} failed", s);
        }
    }

    #[test]
    fn run_face_rejects_empty() {
        assert!(matches!(
            run_face("", "plain"),
            Err(CliError::InvalidArgument(_))
        ));
        assert!(matches!(
            run_face("   ", "plain"),
            Err(CliError::InvalidArgument(_))
        ));
    }

    #[test]
    fn run_face_rejects_malformed() {
        for s in ["i =", "i = 2", "(i = 0", "garbage @ 0"] {
            assert!(
                matches!(run_face(s, "plain"), Err(CliError::InvalidArgument(_))),
                "should reject {}",
                s
            );
        }
    }

    #[test]
    fn run_face_rejects_unknown_format() {
        assert!(matches!(
            run_face("i = 0", "yaml"),
            Err(CliError::InvalidArgument(_))
        ));
    }

    #[test]
    fn json_escape_handles_control_chars() {
        assert_eq!(json_escape("a\"b\nc"), "a\\\"b\\nc");
    }
}
