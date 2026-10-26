//! Shared canonicalization for finite distributive-lattice normal forms.
use verum_common::List;

pub(crate) fn minimal<C: Ord>(
    clauses: impl IntoIterator<Item = C>,
    implies: impl Fn(&C, &C) -> bool,
) -> List<C> {
    let mut result: List<C> = List::new();
    for clause in clauses {
        if result.iter().any(|existing| implies(&clause, existing)) {
            continue;
        }
        result.retain(|existing| !implies(existing, &clause));
        result.push(clause);
    }
    result.sort_unstable();
    result
}
