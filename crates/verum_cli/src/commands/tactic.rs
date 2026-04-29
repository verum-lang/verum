//! `verum tactic` subcommand — surfaces
//! `verum_verification::tactic_combinator::DefaultTacticCatalog` so
//! IDE / REPL / docs-generator consumers can ask the canonical
//! combinator catalogue what its 15 entries are, what their algebraic
//! laws look like, and what a single combinator's full doc record is.
//!
//! ## Why this is the integration that #76 was missing
//!
//! Pre-this-module the canonical combinator set lived as prose
//! comments in `core/proof/tactics/combinators.vr` and as ad-hoc
//! pattern-matches in `verum_smt::tactic_laws`. There was no
//! programmatic surface that could answer "what combinators ship?",
//! "what's `solve`'s signature?", "what algebraic laws does the
//! simplifier exploit?".
//!
//! This command is the **transport-layer integration**: it wires
//! [`DefaultTacticCatalog`](verum_verification::tactic_combinator::DefaultTacticCatalog)
//! to a typed CLI. LSP can shell out for completion metadata; the
//! docs generator can ingest the JSON output verbatim; CI can pin
//! the catalogue's shape via golden tests.
//!
//! Same architectural pattern as proof-draft / verify-ladder /
//! proof-repair: single trait boundary + reference V0 impl + future
//! domain-specific catalogues plug in via [`CompositeTacticCatalog`]
//! without touching this command handler.
//!
//! ## Subcommands
//!
//!   * `verum tactic list [--format=plain|json] [--category=…]`
//!     Lists every combinator with a one-line summary.
//!   * `verum tactic explain <name> [--format=plain|json]`
//!     Full structured doc for a single combinator.
//!   * `verum tactic laws [--format=plain|json]`
//!     The canonical algebraic-law inventory.

use crate::error::{CliError, Result};
use verum_verification::tactic_combinator::{
    AlgebraicLaw, CombinatorCategory, DefaultTacticCatalog, TacticCatalog, TacticEntry,
};

/// Run `verum tactic list`.  Optional category filter.
pub fn run_list(format: &str, category: Option<&str>) -> Result<()> {
    validate_format(format)?;
    let cat_filter = category
        .map(|c| {
            parse_category(c).ok_or_else(|| {
                CliError::InvalidArgument(format!(
                    "unknown --category '{}' (valid: identity, composition, control, focus, forward)",
                    c
                ))
            })
        })
        .transpose()?;

    let catalog = DefaultTacticCatalog::new();
    let entries: Vec<TacticEntry> = catalog
        .entries()
        .into_iter()
        .filter(|e| match cat_filter {
            Some(c) => e.combinator.category() == c,
            None => true,
        })
        .collect();

    match format {
        "plain" => emit_list_plain(&entries, cat_filter),
        "json" => emit_list_json(&entries),
        _ => unreachable!(),
    }
    Ok(())
}

/// Run `verum tactic explain <name>`.
pub fn run_explain(name: &str, format: &str) -> Result<()> {
    validate_format(format)?;
    let catalog = DefaultTacticCatalog::new();
    let entry = catalog.lookup(name).ok_or_else(|| {
        CliError::InvalidArgument(format!(
            "unknown tactic '{}' — run `verum tactic list` for the full catalogue",
            name
        ))
    })?;

    match format {
        "plain" => emit_explain_plain(&entry, &catalog),
        "json" => emit_explain_json(&entry, &catalog),
        _ => unreachable!(),
    }
    Ok(())
}

/// Run `verum tactic laws`.
pub fn run_laws(format: &str) -> Result<()> {
    validate_format(format)?;
    let catalog = DefaultTacticCatalog::new();
    let laws = catalog.laws();
    match format {
        "plain" => emit_laws_plain(&laws),
        "json" => emit_laws_json(&laws),
        _ => unreachable!(),
    }
    Ok(())
}

// =============================================================================
// Validation helpers
// =============================================================================

fn validate_format(format: &str) -> Result<()> {
    if format != "plain" && format != "json" {
        return Err(CliError::InvalidArgument(format!(
            "--format must be 'plain' or 'json', got '{}'",
            format
        )));
    }
    Ok(())
}

fn parse_category(c: &str) -> Option<CombinatorCategory> {
    match c {
        "identity" => Some(CombinatorCategory::Identity),
        "composition" => Some(CombinatorCategory::Composition),
        "control" => Some(CombinatorCategory::Control),
        "focus" => Some(CombinatorCategory::Focus),
        "forward" => Some(CombinatorCategory::Forward),
        _ => None,
    }
}

// =============================================================================
// Plain emitters
// =============================================================================

fn emit_list_plain(entries: &[TacticEntry], cat_filter: Option<CombinatorCategory>) {
    let header = match cat_filter {
        Some(c) => format!("Tactic combinator catalogue (category: {})", c.name()),
        None => "Tactic combinator catalogue (V0 reference)".to_string(),
    };
    println!("{}", header);
    println!("{}", "─".repeat(header.len()));
    println!();
    println!(
        "  {:<18}  {:<14}  {}",
        "Name", "Category", "Semantics"
    );
    println!(
        "  {}  {}  {}",
        "─".repeat(18),
        "─".repeat(14),
        "─".repeat(50)
    );
    for e in entries {
        println!(
            "  {:<18}  {:<14}  {}",
            e.combinator.name(),
            e.combinator.category().name(),
            e.semantics.as_str()
        );
    }
    println!();
    println!("Total: {} combinator(s)", entries.len());
    println!();
    println!("Inspect a single combinator with `verum tactic explain <name>`.");
    println!("Print algebraic laws with `verum tactic laws`.");
}

fn emit_explain_plain(entry: &TacticEntry, catalog: &DefaultTacticCatalog) {
    let header = format!(
        "{} — {} category",
        entry.combinator.name(),
        entry.combinator.category().name()
    );
    println!("{}", header);
    println!("{}", "─".repeat(header.len()));
    println!();
    println!("Signature : {}", entry.signature.as_str());
    println!("Semantics : {}", entry.semantics.as_str());
    println!();
    println!("Example:");
    for line in entry.example.as_str().lines() {
        println!("    {}", line);
    }
    println!();
    if !entry.laws.is_empty() {
        println!("Algebraic laws:");
        let all_laws = catalog.laws();
        for law_name in &entry.laws {
            let law = all_laws.iter().find(|l| l.name.as_str() == law_name.as_str());
            match law {
                Some(l) => {
                    println!("  • {}", l.name.as_str());
                    println!("      {} ≡ {}", l.lhs.as_str(), l.rhs.as_str());
                }
                None => println!("  • {} (rationale missing)", law_name.as_str()),
            }
        }
        println!();
    }
    println!("Doc anchor: #{}", entry.doc_anchor.as_str());
}

fn emit_laws_plain(laws: &[AlgebraicLaw]) {
    println!("Algebraic laws — canonical normalisation set");
    println!("{}", "─".repeat(45));
    println!();
    for l in laws {
        println!("  {}", l.name.as_str());
        println!("    {} ≡ {}", l.lhs.as_str(), l.rhs.as_str());
        println!("    ↪ {}", l.rationale.as_str());
        println!();
    }
    println!("Total: {} law(s)", laws.len());
}

// =============================================================================
// JSON emitters
// =============================================================================

fn emit_list_json(entries: &[TacticEntry]) {
    let mut out = String::from("{\n");
    out.push_str("  \"schema_version\": 1,\n");
    out.push_str(&format!("  \"count\": {},\n", entries.len()));
    out.push_str("  \"entries\": [\n");
    for (i, e) in entries.iter().enumerate() {
        out.push_str(&format_entry_json(e, "    "));
        out.push_str(if i + 1 < entries.len() { ",\n" } else { "\n" });
    }
    out.push_str("  ]\n}");
    println!("{}", out);
}

fn emit_explain_json(entry: &TacticEntry, catalog: &DefaultTacticCatalog) {
    let all_laws = catalog.laws();
    let mut out = String::from("{\n");
    out.push_str("  \"schema_version\": 1,\n");
    out.push_str(&format!(
        "  \"name\": \"{}\",\n",
        json_escape(entry.combinator.name())
    ));
    out.push_str(&format!(
        "  \"category\": \"{}\",\n",
        entry.combinator.category().name()
    ));
    out.push_str(&format!(
        "  \"signature\": \"{}\",\n",
        json_escape(entry.signature.as_str())
    ));
    out.push_str(&format!(
        "  \"semantics\": \"{}\",\n",
        json_escape(entry.semantics.as_str())
    ));
    out.push_str(&format!(
        "  \"example\": \"{}\",\n",
        json_escape(entry.example.as_str())
    ));
    out.push_str(&format!(
        "  \"doc_anchor\": \"{}\",\n",
        json_escape(entry.doc_anchor.as_str())
    ));
    out.push_str("  \"laws\": [\n");
    for (i, law_name) in entry.laws.iter().enumerate() {
        let law = all_laws.iter().find(|l| l.name.as_str() == law_name.as_str());
        match law {
            Some(l) => {
                out.push_str(&format_law_json(l, "    "));
            }
            None => {
                out.push_str(&format!(
                    "    {{ \"name\": \"{}\", \"resolved\": false }}",
                    json_escape(law_name.as_str())
                ));
            }
        }
        out.push_str(if i + 1 < entry.laws.len() { ",\n" } else { "\n" });
    }
    out.push_str("  ]\n}");
    println!("{}", out);
}

fn emit_laws_json(laws: &[AlgebraicLaw]) {
    let mut out = String::from("{\n");
    out.push_str("  \"schema_version\": 1,\n");
    out.push_str(&format!("  \"count\": {},\n", laws.len()));
    out.push_str("  \"laws\": [\n");
    for (i, l) in laws.iter().enumerate() {
        out.push_str(&format_law_json(l, "    "));
        out.push_str(if i + 1 < laws.len() { ",\n" } else { "\n" });
    }
    out.push_str("  ]\n}");
    println!("{}", out);
}

fn format_entry_json(e: &TacticEntry, indent: &str) -> String {
    format!(
        "{indent}{{ \"name\": \"{}\", \"category\": \"{}\", \"signature\": \"{}\", \"semantics\": \"{}\", \"example\": \"{}\", \"doc_anchor\": \"{}\", \"laws\": [{}] }}",
        json_escape(e.combinator.name()),
        e.combinator.category().name(),
        json_escape(e.signature.as_str()),
        json_escape(e.semantics.as_str()),
        json_escape(e.example.as_str()),
        json_escape(e.doc_anchor.as_str()),
        e.laws
            .iter()
            .map(|l| format!("\"{}\"", json_escape(l.as_str())))
            .collect::<Vec<_>>()
            .join(", "),
    )
}

fn format_law_json(l: &AlgebraicLaw, indent: &str) -> String {
    let participants = l
        .participants
        .iter()
        .map(|c| format!("\"{}\"", c.name()))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "{indent}{{ \"name\": \"{}\", \"lhs\": \"{}\", \"rhs\": \"{}\", \"rationale\": \"{}\", \"participants\": [{}] }}",
        json_escape(l.name.as_str()),
        json_escape(l.lhs.as_str()),
        json_escape(l.rhs.as_str()),
        json_escape(l.rationale.as_str()),
        participants
    )
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

    // ----- format / category validation -----

    #[test]
    fn validate_format_accepts_plain_and_json() {
        assert!(validate_format("plain").is_ok());
        assert!(validate_format("json").is_ok());
    }

    #[test]
    fn validate_format_rejects_other() {
        assert!(matches!(
            validate_format("yaml"),
            Err(CliError::InvalidArgument(_))
        ));
    }

    #[test]
    fn parse_category_round_trip() {
        for (s, expected) in [
            ("identity", CombinatorCategory::Identity),
            ("composition", CombinatorCategory::Composition),
            ("control", CombinatorCategory::Control),
            ("focus", CombinatorCategory::Focus),
            ("forward", CombinatorCategory::Forward),
        ] {
            assert_eq!(parse_category(s), Some(expected));
        }
        assert_eq!(parse_category("nonsense"), None);
    }

    // ----- json_escape -----

    #[test]
    fn json_escape_quotes_and_backslashes() {
        assert_eq!(json_escape("a\"b\\c"), "a\\\"b\\\\c");
    }

    #[test]
    fn json_escape_newlines() {
        assert_eq!(json_escape("a\nb"), "a\\nb");
    }

    // ----- run_list -----

    #[test]
    fn run_list_plain_smoke() {
        // Should run without error and not panic.
        assert!(run_list("plain", None).is_ok());
    }

    #[test]
    fn run_list_json_smoke() {
        assert!(run_list("json", None).is_ok());
    }

    #[test]
    fn run_list_rejects_unknown_format() {
        assert!(matches!(
            run_list("yaml", None),
            Err(CliError::InvalidArgument(_))
        ));
    }

    #[test]
    fn run_list_rejects_unknown_category() {
        assert!(matches!(
            run_list("plain", Some("nonsense")),
            Err(CliError::InvalidArgument(_))
        ));
    }

    #[test]
    fn run_list_with_valid_category() {
        for cat in ["identity", "composition", "control", "focus", "forward"] {
            assert!(run_list("plain", Some(cat)).is_ok(), "category {}", cat);
        }
    }

    // ----- run_explain -----

    #[test]
    fn run_explain_resolves_every_canonical_combinator() {
        for name in [
            "skip",
            "fail",
            "seq",
            "orelse",
            "repeat",
            "repeat_n",
            "try",
            "solve",
            "first_of",
            "all_goals",
            "index_focus",
            "named_focus",
            "per_goal_split",
            "have",
            "apply_with",
        ] {
            assert!(run_explain(name, "plain").is_ok(), "name {}", name);
        }
    }

    #[test]
    fn run_explain_rejects_unknown_name() {
        assert!(matches!(
            run_explain("nonsense", "plain"),
            Err(CliError::InvalidArgument(_))
        ));
    }

    #[test]
    fn run_explain_rejects_unknown_format() {
        assert!(matches!(
            run_explain("solve", "yaml"),
            Err(CliError::InvalidArgument(_))
        ));
    }

    // ----- run_laws -----

    #[test]
    fn run_laws_plain_smoke() {
        assert!(run_laws("plain").is_ok());
    }

    #[test]
    fn run_laws_json_smoke() {
        assert!(run_laws("json").is_ok());
    }

    // ----- format_entry_json + format_law_json -----

    #[test]
    fn format_entry_json_produces_well_formed_object() {
        let catalog = DefaultTacticCatalog::new();
        let entry = catalog.lookup("solve").unwrap();
        let s = format_entry_json(&entry, "");
        // Must be valid JSON for one record.
        let parsed: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed["name"], "solve");
        assert_eq!(parsed["category"], "control");
        assert!(parsed["semantics"].is_string());
    }

    #[test]
    fn format_law_json_produces_well_formed_object() {
        let catalog = DefaultTacticCatalog::new();
        let laws = catalog.laws();
        let l = &laws[0];
        let s = format_law_json(l, "");
        let parsed: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert!(parsed["name"].is_string());
        assert!(parsed["lhs"].is_string());
        assert!(parsed["rhs"].is_string());
        assert!(parsed["participants"].is_array());
    }
}
