//! `verum arch query` — the machine-facing surface of inference-first
//! ATS-V-2 (T0848/T0849 seed).
//!
//! One question, one stable answer shape: *"what may the code at this
//! path do?"* — answered from the SAME row inference the compiler
//! phase runs (no second derivation), with the pinned declaration and
//! the two-direction judgment included. The JSON form is the
//! agent-facing contract: coding agents run it before and after a
//! patch (the ask → patch → diff cycle of the accepted design), so
//! its fields are append-only from the first release.

use anyhow::{anyhow, Result};
use serde::Serialize;

use verum_ast::Module;
use verum_common::FileId;
use verum_kernel::arch_rows::Row;

use crate::pipeline::{infer_summaries_for_query, pinned_capabilities_of_module};

/// One capability atom with its provenance, as reported.
#[derive(Debug, Serialize)]
pub struct ReportedAtom {
    /// Canonical rendering of the capability.
    pub atom: String,
    /// `computed` — this inference derived it; or the citation source.
    pub evidence: String,
}

/// One function's solved summary.
#[derive(Debug, Serialize)]
pub struct ReportedSummary {
    /// Function name (bare, as callable within the module).
    pub function: String,
    /// Ground atoms of the summary.
    pub atoms: Vec<ReportedAtom>,
    /// Row variables — the named capability-bearing parameters this
    /// summary is polymorphic over (empty ⟺ closed row).
    pub open_over: Vec<String>,
}

/// The full answer to `verum arch query --at <file>`.
///
/// Field discipline: APPEND-ONLY. Agents parse this; a removed or
/// renamed field is a broken agent. (The same discipline that keeps
/// AP-codes learnable.)
#[derive(Debug, Serialize)]
pub struct ArchQueryReport {
    /// Module path as declared (`module a.b.c;`), if any.
    pub module: Option<String>,
    /// The inferred module surface (transitive, row-solved).
    pub inferred: Vec<ReportedAtom>,
    /// Capabilities the `@arch_module` pin declares (requires ∪
    /// exposes), if the module carries a pin.
    pub pinned: Option<Vec<String>>,
    /// Judgment, inferred vs pinned: atoms the code exercises beyond
    /// the pin. `None` when there is no pin to judge against.
    pub escalations: Option<Vec<ReportedAtom>>,
    /// Judgment, pinned vs inferred: declared rights nothing
    /// exercises (dead-right candidates feeding the rot law).
    pub dead_rights: Option<Vec<String>>,
    /// Per-function summaries (the audit detail behind `inferred`).
    pub functions: Vec<ReportedSummary>,
    /// Local call edges the solver could not resolve — surfaced,
    /// never guessed (the no-silent-⊤ law made visible).
    pub unresolved_calls: Vec<String>,
}

fn report_row_atoms(row: &Row) -> Vec<ReportedAtom> {
    row.facts()
        .map(|f| ReportedAtom {
            atom: format!("{:?}", f.atom),
            evidence: match &f.evidence {
                verum_kernel::intrinsic_dispatch::Evidence::Computed => "computed".to_string(),
                verum_kernel::intrinsic_dispatch::Evidence::Cited { source } => {
                    format!("cited: {source}")
                }
            },
        })
        .collect()
}

/// Answer the query for one parsed module.
pub fn arch_query_module(module: &Module) -> ArchQueryReport {
    let solved = infer_summaries_for_query(module);
    let surface = solved.module_surface();

    let pinned = pinned_capabilities_of_module(module);
    let (escalations, dead_rights) = match &pinned {
        Some(pin_row) => {
            let (esc, dead) = surface.judge_against(pin_row);
            (
                Some(
                    esc.iter()
                        .map(|f| ReportedAtom {
                            atom: format!("{:?}", f.atom),
                            evidence: match &f.evidence {
                                verum_kernel::intrinsic_dispatch::Evidence::Computed => {
                                    "computed".to_string()
                                }
                                verum_kernel::intrinsic_dispatch::Evidence::Cited { source } => {
                                    format!("cited: {source}")
                                }
                            },
                        })
                        .collect(),
                ),
                Some(dead.iter().map(|f| format!("{:?}", f.atom)).collect()),
            )
        }
        None => (None, None),
    };

    ArchQueryReport {
        module: module_declared_path(module),
        inferred: report_row_atoms(&surface),
        pinned: pinned
            .as_ref()
            .map(|row| row.facts().map(|f| format!("{:?}", f.atom)).collect()),
        escalations,
        dead_rights,
        functions: solved
            .summaries
            .iter()
            .map(|(name, row)| ReportedSummary {
                function: name.clone(),
                atoms: report_row_atoms(row),
                open_over: row
                    .variables()
                    .iter()
                    .map(|v| format!("{}::{}", v.owner, v.param))
                    .collect(),
            })
            .collect(),
        unresolved_calls: solved
            .unresolved
            .iter()
            .map(|e| format!("{} -> {}", e.caller, e.callee))
            .collect(),
    }
}

fn module_declared_path(module: &Module) -> Option<String> {
    use verum_ast::decl::ItemKind;
    module.items.iter().find_map(|item| match &item.kind {
        ItemKind::Module(m) => Some(m.name.name.to_string()),
        _ => None,
    })
}

/// Parse a `.vr` source and answer the query. The parse is the fast
/// parser only — the question is architectural, not typechecking, so
/// an agent gets its answer in milliseconds.
pub fn arch_query_source(source: &str) -> Result<ArchQueryReport> {
    let parser = verum_fast_parser::FastParser::new();
    let module = parser
        .parse_module_str(source, FileId::new(0))
        .map_err(|e| anyhow!("parse failed: {e:?}"))?;
    Ok(arch_query_module(&module))
}
