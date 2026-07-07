//! Fixture classification and the run-everything loop.

use crate::compare::{compare_outcomes, EngineOutcome, Verdict};
use crate::engines::{build_meta_function, run_tree_walk, run_vbc};
use crate::fixtures::{Expectation, Fixture};

/// How one fixture landed against its expectation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Expected agreement, engines agree.
    Agree,
    /// Known divergence: the pin's shape matched.
    KnownDiverge,
    /// Anything unexpected: agreement fixture diverged, pin drifted to a
    /// different shape, pin disappeared (engines converged), or the fixture
    /// itself failed to build. Always a human-attention signal.
    NewDiverge,
}

impl Status {
    /// Report label.
    pub fn label(&self) -> &'static str {
        match self {
            Status::Agree => "AGREE",
            Status::KnownDiverge => "KNOWN-DIVERGE",
            Status::NewDiverge => "NEW-DIVERGE",
        }
    }
}

/// Result of one fixture run.
#[derive(Debug)]
pub struct FixtureReport {
    /// Fixture name.
    pub name: &'static str,
    /// Fixture description.
    pub description: &'static str,
    /// Classified status.
    pub status: Status,
    /// The verdict (None when the fixture failed to build).
    pub verdict: Option<Verdict>,
    /// Engine A (VBC) outcome, when the fixture ran.
    pub vbc: Option<EngineOutcome>,
    /// Engine B (tree-walk) outcome, when the fixture ran.
    pub tree: Option<EngineOutcome>,
    /// Human explanation (pin note, drift/disappearance message, build error).
    pub explanation: String,
}

/// Honest totals over one full run.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Totals {
    /// Fixtures where agreement was expected and observed.
    pub agree: usize,
    /// Pinned divergences whose shape still matches.
    pub known_diverge: usize,
    /// Unexpected results (incl. pin drift/disappearance and build failures).
    pub new_diverge: usize,
}

impl Totals {
    /// Total number of fixtures.
    pub fn total(&self) -> usize {
        self.agree + self.known_diverge + self.new_diverge
    }
}

/// Classify a fixture's verdict against its expectation.
pub fn classify(fixture: &Fixture, verdict: &Verdict) -> (Status, String) {
    match &fixture.expect {
        Expectation::Agree => {
            if verdict.is_agree() {
                (Status::Agree, String::new())
            } else {
                (
                    Status::NewDiverge,
                    format!("expected agreement, got {verdict}"),
                )
            }
        }
        Expectation::Pinned { shape, note } => {
            if shape.matches(verdict) {
                (Status::KnownDiverge, (*note).to_string())
            } else if verdict.is_agree() {
                (
                    Status::NewDiverge,
                    format!(
                        "PIN DISAPPEARED: engines now converge (pinned shape was {shape}) — \
                         verify the convergence and retire the pin"
                    ),
                )
            } else {
                (
                    Status::NewDiverge,
                    format!("PIN DRIFTED: pinned shape {shape}, observed {verdict}"),
                )
            }
        }
    }
}

/// Run one fixture through both engines and classify.
pub fn run_fixture(fixture: &Fixture) -> FixtureReport {
    let meta_fn = match build_meta_function(fixture.source, fixture.fn_name) {
        Ok(f) => f,
        Err(e) => {
            return FixtureReport {
                name: fixture.name,
                description: fixture.description,
                status: Status::NewDiverge,
                verdict: None,
                vbc: None,
                tree: None,
                explanation: format!("fixture failed to build: {e}"),
            };
        }
    };

    let vbc = run_vbc(&meta_fn, &fixture.args);
    let tree = run_tree_walk(&meta_fn, &fixture.args);
    let verdict = compare_outcomes(&vbc, &tree);
    let (status, explanation) = classify(fixture, &verdict);

    FixtureReport {
        name: fixture.name,
        description: fixture.description,
        status,
        verdict: Some(verdict),
        vbc: Some(vbc),
        tree: Some(tree),
        explanation,
    }
}

/// Run every fixture; returns per-fixture reports and honest totals.
///

/// Callers that want panic-noise suppression (the i128-overflow pin panics
/// by design) should wrap this in [`crate::engines::with_quiet_panics`].
pub fn run_all(fixtures: &[Fixture]) -> (Vec<FixtureReport>, Totals) {
    let mut reports = Vec::with_capacity(fixtures.len());
    let mut totals = Totals::default();
    for fixture in fixtures {
        let report = run_fixture(fixture);
        match report.status {
            Status::Agree => totals.agree += 1,
            Status::KnownDiverge => totals.known_diverge += 1,
            Status::NewDiverge => totals.new_diverge += 1,
        }
        reports.push(report);
    }
    (reports, totals)
}
