use verum_proof::proof_checker::{Certificate, Level, Term};
use verum_source_proof::scoped::*;
type Result<T> = std::result::Result<T, Error>;
use verum_ast::decl::{ItemKind, TheoremDecl};
fn parse(src: &str) -> TheoremDecl {
    let m = verum_fast_parser::FastParser::new()
        .parse_module_str(src, verum_common::span::FileId::new(0))
        .expect("real Verum parser");
    let mut ts = m.items.iter().filter_map(|i| match &i.kind {
        ItemKind::Theorem(t) => Some(t.clone()),
        _ => None,
    });
    let t = ts.next().expect("one theorem");
    assert!(ts.next().is_none());
    t
}
fn run(s: &str) -> Result<Artifact> {
    elaborate(&parse(s), PROFILE)
}
fn accepts(s: &str) -> Artifact {
    run(s).unwrap()
}
#[test]
fn actual_source_generic_identity_and_composition() {
    let a = accepts(include_str!("fixtures/identity.vr"));
    assert_eq!(a.level_bindings, vec![("u".into(), "level_0".into())]);
    assert_eq!(
        a.telescope.iter().map(|b| b.role).collect::<Vec<_>>(),
        [Role::Type, Role::Value]
    );
    assert_eq!(
        a.certificate.term,
        Term::lam(
            Term::Universe(Level::Var("level_0".into())),
            Term::lam(Term::var(0), Term::var(0))
        )
    );
    let c = accepts(include_str!("fixtures/composition.vr"));
    let domains = [
        Term::Universe(Level::Var("level_0".into())),
        Term::Universe(Level::Var("level_1".into())),
        Term::Universe(Level::Var("level_2".into())),
        Term::pi(Term::var(2), Term::var(2)),
        Term::pi(Term::var(2), Term::var(2)),
        Term::var(4),
    ];
    let proof = domains.iter().rev().fold(
        Term::app(Term::var(1), Term::app(Term::var(2), Term::var(0))),
        |b, d| Term::lam(d.clone(), b),
    );
    let ty = domains
        .iter()
        .rev()
        .fold(Term::var(3), |b, d| Term::pi(d.clone(), b));
    assert_eq!(c.certificate.term, proof);
    assert_eq!(c.certificate.claimed_type, ty);
}
#[test]
fn source_identity_equality_checks_actual_value_type() {
    let a = accepts("theorem refl<universe u>(A:Type(u), x:A): x == x proof by reflexivity;");
    assert_eq!(
        a.certificate.claimed_type,
        Term::pi(
            Term::Universe(Level::Var("level_0".into())),
            Term::pi(
                Term::var(0),
                Term::Id {
                    ty: Box::new(Term::var(1)),
                    lhs: Box::new(Term::var(0)),
                    rhs: Box::new(Term::var(0))
                }
            )
        )
    );
    assert!(matches!(
        run("theorem unequal<universe u>(A:Type(u), x:A, y:A): x == y proof by reflexivity;"),
        Err(Error::Rejected(_))
    ));
}
#[test]
fn explicit_profile_is_required_and_unannotated_generics_not_demoted() {
    let t = parse(include_str!("fixtures/identity.vr"));
    assert!(matches!(
        elaborate(&t, "default-runtime"),
        Err(Error::Unsupported(_))
    ));
    assert!(matches!(
        run("theorem id<A>(x:A): A proof = x;"),
        Err(Error::Unsupported(_))
    ));
}
#[test]
fn wrong_composition_domain_is_not_repaired() {
    let s = include_str!("fixtures/composition.vr").replace("g: fn(B)", "g: fn(A)");
    assert!(matches!(run(&s), Err(Error::Rejected(_))));
}
#[test]
fn scope_types_values_universes_are_distinct() {
    assert!(matches!(
        run("theorem wrong(A:Type, x:A, y:x): A proof = x;"),
        Err(Error::Rejected(_))
    ));
    assert!(matches!(
        run("theorem unknown<universe u>(A:Type(v), x:A): A proof = x;"),
        Err(Error::Scope(_))
    ));
    assert!(
        run("theorem correct_universe(A:Type(0), B:Type(0), k:fn(Type(0))->B): B proof = k(A);")
            .is_ok()
    );
    assert!(matches!(
        run("theorem wrong_universe(A:Type(1), B:Type(0), k:fn(Type(0))->B): B proof = k(A);"),
        Err(Error::Rejected(_))
    ));
}
#[test]
fn premises_are_sequential_scoped_telescope_not_unlifted_append() {
    let a = accepts(
        "theorem implication<universe u>(P:Type(u)) requires P ensures P proof by assumption;",
    );
    assert_eq!(
        a.certificate.claimed_type,
        Term::pi(
            Term::Universe(Level::Var("level_0".into())),
            Term::pi(Term::var(0), Term::var(1))
        )
    );
    assert_eq!(a.telescope.last().unwrap().role, Role::Premise);
}
#[test]
fn actual_nested_shadowing_preserves_innermost_bindings() {
    let a = accepts(
        "theorem shadow<universe u>(a:Type(u), x:a): forall (a:Type(u)). forall x:a. a proof = |a:Type(u)| |x:a| x;",
    );
    let b = accepts(
        "theorem renamed<universe v>(outer:Type(v), old:outer): forall (inner:Type(v)). forall fresh:inner. inner proof = |inner:Type(v)| |fresh:inner| fresh;",
    );
    assert_eq!(a.certificate.term, b.certificate.term);
    assert_eq!(a.certificate.claimed_type, b.certificate.claimed_type);
    assert!(matches!(
        run(
            "theorem capture<universe u>(A:Type(u), B:Type(u), x:A): forall x:B. A proof = |x:B| x;"
        ),
        Err(Error::Rejected(_))
    ));
}
#[test]
fn dependent_source_types_do_not_capture_same_spelled_values() {
    let a = accepts("theorem keep<universe u>(A:Type(u), x:A): forall y:A. A proof = |y:A| x;");
    assert_eq!(
        a.certificate.term,
        Term::lam(
            Term::Universe(Level::Var("level_0".into())),
            Term::lam(Term::var(0), Term::lam(Term::var(1), Term::var(1)))
        )
    );
}
#[test]
fn unsupported_semantics_are_not_erased() {
    for s in [
        "theorem truth(): true proof by trivial;",
        "theorem default(A:Type, x:A=x): A proof = x;",
        "theorem guarded(A:Type, x:A): forall y:A where true. A proof = |y:A| x;",
    ] {
        assert!(matches!(run(s), Err(Error::Unsupported(_))), "{s}");
    }
}
#[test]
fn stable_ids_lower_against_independent_named_stack_oracle() {
    // Independently choose every bound reference at depths 1..8, with arbitrary IDs.
    // Expected indices come solely from name-stack position, not the production IDs.
    let mut cases = 0;
    for depth in 1..=8 {
        for chosen in 0..depth {
            let names = (0..depth).map(|i| format!("n{i}")).collect::<Vec<_>>();
            let selected = names[chosen].clone();
            let expected_index = names.iter().rev().position(|s| s == &selected).unwrap();
            let mut ir = Ir::Var(BinderId(101 + 17 * chosen as u64));
            let mut expected = Term::var(expected_index);
            for i in (0..depth).rev() {
                ir = Ir::Lam {
                    id: BinderId(101 + 17 * i as u64),
                    domain: Box::new(Ir::Universe(Level::Concrete(0))),
                    body: Box::new(ir),
                };
                expected = Term::lam(Term::universe(0), expected);
            }
            assert_eq!(lower(&ir, &mut vec![]).unwrap(), expected);
            cases += 1;
        }
    }
    assert_eq!(cases, 36);
    assert!(lower(&Ir::Var(BinderId(999)), &mut vec![]).is_err());
    let bad = Ir::Lam {
        id: BinderId(1),
        domain: Box::new(Ir::Universe(Level::Concrete(0))),
        body: Box::new(Ir::Lam {
            id: BinderId(1),
            domain: Box::new(Ir::Universe(Level::Concrete(0))),
            body: Box::new(Ir::Var(BinderId(1))),
        }),
    };
    assert!(lower(&bad, &mut vec![]).is_err());
}
#[test]
fn distinct_universe_schema_replays_under_independent_finite_valuations() {
    fn substitute(t: &Term, vs: &[u32; 3]) -> Term {
        fn level(l: &Level, vs: &[u32; 3]) -> Level {
            match l {
                Level::Concrete(n) => Level::Concrete(*n),
                Level::Var(n) => {
                    Level::Concrete(vs[n.strip_prefix("level_").unwrap().parse::<usize>().unwrap()])
                }
                Level::Succ(x) => Level::Succ(Box::new(level(x, vs))),
                Level::Max(a, b) => Level::Max(Box::new(level(a, vs)), Box::new(level(b, vs))),
                Level::Offset(a, n) => Level::Offset(Box::new(level(a, vs)), *n),
            }
        }
        match t {
            Term::Universe(l) => Term::Universe(level(l, vs)),
            Term::Var(i) => Term::Var(*i),
            Term::Pi(a, b) => Term::pi(substitute(a, vs), substitute(b, vs)),
            Term::Lam(a, b) => Term::lam(substitute(a, vs), substitute(b, vs)),
            Term::App(a, b) => Term::app(substitute(a, vs), substitute(b, vs)),
            _ => panic!("composition oracle fragment"),
        }
    }
    let a = accepts(include_str!("fixtures/composition.vr"));
    let mut count = 0;
    for u in [0, 1, 17] {
        for v in [0, 3, 15] {
            for w in [0, 7, 31] {
                let vs = [u, v, w];
                let c = Certificate {
                    term: substitute(&a.certificate.term, &vs),
                    claimed_type: substitute(&a.certificate.claimed_type, &vs),
                    metadata: Default::default(),
                };
                c.verify().unwrap();
                verum_proof::proof_checker_nbe::verify_certificate(&c).unwrap();
                count += 1;
            }
        }
    }
    assert_eq!(count, 27);
    accepts("theorem max_identity<universe u,universe v>(a:Type(max(u,v)),x:a): a proof=x;");
}
