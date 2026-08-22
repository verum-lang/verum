//! The row-algebra laws of `docs/architecture/ats-v2-capability-rows.md`,
//! pinned executable (T0848). Section references are to that document.
//!
//! Both-polarity discipline (design §8): every law that FORBIDS a
//! behaviour is tested with a violating input that must be caught,
//! next to the innocent twin that must pass — a checker satisfied by
//! screaming at everything fails the twin.

use std::collections::BTreeMap;

use verum_kernel::arch::{Capability, NetDirection, NetProtocol, ResourceTag};
use verum_kernel::arch_rows::{atoms_dying_at_boundary, Row, RowVar, Summary};
use verum_kernel::intrinsic_dispatch::Evidence;

fn read_file() -> Capability {
    Capability::Read {
        resource: ResourceTag::File {
            path_pattern: "config".into(),
        },
    }
}

fn net_out() -> Capability {
    Capability::Network {
        protocol: NetProtocol::Tcp,
        direction: NetDirection::Outbound,
    }
}

/// §1: there is no ⊤. The widest row is an explicit enumeration —
/// structurally, the API simply has no "everything" constructor, so
/// this test pins the SHAPE of the API: a row's atom count equals
/// what was listed, nothing appears from nowhere.
#[test]
fn no_top_exists_only_listed_atoms() {
    let row = Row::computed([read_file(), net_out()]);
    assert_eq!(row.atom_count(), 2);
    assert!(row.is_closed());
}

/// §2: join is union with per-atom provenance MEET — one atom derived
/// both computed and cited holds `Cited`, never two entries and never
/// a silently-kept `Computed`.
#[test]
fn join_meets_provenance_per_atom() {
    let mut a = Row::computed([read_file()]);
    let b = Row::cited([read_file()], "protocol P max-shape");
    a.join(&b);
    assert_eq!(a.atom_count(), 1, "no duplicate labels by construction");
    let fact = a.facts().next().unwrap();
    match &fact.evidence {
        Evidence::Cited { source } => assert!(source.contains("max-shape")),
        Evidence::Computed => panic!(
            "provenance must MEET: one cited deriving path makes the atom \
             cited (§6), keeping Computed would launder authority"
        ),
    }
}

/// §2: substitution is monotone — instantiating never removes atoms.
#[test]
fn instantiation_is_monotone() {
    let var = RowVar {
        owner: "map".into(),
        param: "f".into(),
    };
    let mut open = Row::computed([read_file()]);
    open.open_over(var.clone());
    let before = open.atom_count();
    open.instantiate(&var, &Row::computed([net_out()]));
    assert!(open.atom_count() >= before);
    assert!(open.is_closed());
    assert_eq!(open.atom_count(), 2);
}

/// §3: combinators are transparent — a summary with no own atoms and
/// one mixed variable instantiates to exactly the argument's row.
#[test]
fn combinator_passes_rows_through() {
    let var = RowVar {
        owner: "map".into(),
        param: "f".into(),
    };
    let mut body = Row::empty();
    body.open_over(var);
    let map = Summary::install("map", body);

    let mut args = BTreeMap::new();
    args.insert("f".to_string(), Row::computed([net_out()]));
    let at_site = map.instantiate(&args);
    assert_eq!(at_site.atom_count(), 1, "map contributes nothing of its own");

    // Innocent twin: instantiated with a pure closure, the surface is ∅.
    let mut pure_args = BTreeMap::new();
    pure_args.insert("f".to_string(), Row::empty());
    let pure_site = map.instantiate(&pure_args);
    assert_eq!(pure_site.atom_count(), 0, "pure argument ⟹ empty surface");
}

/// §4 no-silent-⊤ law, clause (b): an open row's variables each trace
/// to a NAMED parameter — the audit can print its sources.
#[test]
fn open_rows_name_their_sources() {
    let var = RowVar {
        owner: "spawn".into(),
        param: "task".into(),
    };
    let mut row = Row::computed([Capability::Spawn {
        lifetime: verum_kernel::arch::TaskLifetime::ScopedToParent,
    }]);
    row.open_over(var);
    assert_eq!(row.variables().len(), 1);
    assert_eq!(row.variables()[0].param, "task");
}

/// Design §2 both-direction judgment: escalation AND dead right are
/// distinct findings, each carrying facts.
#[test]
fn judgment_runs_both_directions() {
    let inferred = Row::computed([read_file(), net_out()]);
    let pinned = Row::computed([read_file(), Capability::Persist {
        medium: verum_kernel::arch::PersistenceMedium::Database {
            connection_tag: "ledger".into(),
        },
    }]);
    let (escalations, dead) = inferred.judge_against(&pinned);
    assert_eq!(escalations.len(), 1, "Network exceeds the pin");
    assert_eq!(dead.len(), 1, "Persist is a dead right");

    // Innocent twin: a pin that matches the inference judges clean.
    let (e2, d2) = inferred.judge_against(&inferred);
    assert!(e2.is_empty() && d2.is_empty());
}

/// §5a domain-flow law (duel strike 1): an enforced payload atom
/// outside allow(D_dst) is caught at the edge; the innocent twin —
/// a payload whose atoms the destination allows — passes.
#[test]
fn rights_dying_at_boundary_are_caught_and_innocent_payloads_pass() {
    let payload = Row::computed([net_out(), read_file()]);
    let enforced = |c: &Capability| matches!(c, Capability::Network { .. });
    let narrow_dst = Row::computed([read_file()]);
    let dying = atoms_dying_at_boundary(&payload, &enforced, &narrow_dst);
    assert_eq!(dying.len(), 1, "Network dies at the narrow boundary");

    let wide_dst = Row::computed([read_file(), net_out()]);
    let ok = atoms_dying_at_boundary(&payload, &enforced, &wide_dst);
    assert!(ok.is_empty(), "innocent twin: destination allows the payload");
}

/// §4/§5b: a cited row REFUSES an empty authority — the stamped-
/// verdict defect class (T0841) must not re-enter through rows.
#[test]
#[should_panic(expected = "must name its authority")]
fn cited_row_refuses_empty_source() {
    let _ = Row::cited([net_out()], "   ");
}

/// §6 product lattice: joining N rows in any order yields the same
/// atom set (union is commutative/associative/idempotent) — the
/// fixpoint's convergence does not depend on visit order.
#[test]
fn join_is_order_insensitive() {
    let rows = [
        Row::computed([read_file()]),
        Row::cited([net_out()], "extern pin"),
        Row::computed([read_file(), net_out()]),
    ];
    let mut fwd = Row::empty();
    for r in &rows {
        fwd.join(r);
    }
    let mut rev = Row::empty();
    for r in rows.iter().rev() {
        rev.join(r);
    }
    assert_eq!(fwd.atom_count(), rev.atom_count());
    assert!(fwd.subsumed_by(&rev) && rev.subsumed_by(&fwd));
}

/// Idempotence: X ∪ X = X — joining a row with itself changes nothing
/// (the fixpoint's stabilisation test is exactly this).
#[test]
fn join_is_idempotent() {
    let mut row = Row::computed([read_file(), net_out()]);
    let before = row.atom_count();
    let snapshot = row.clone();
    row.join(&snapshot);
    assert_eq!(row.atom_count(), before);
}
