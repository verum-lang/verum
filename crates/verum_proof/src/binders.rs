/// Convert between de Bruijn indices (innermost first) and environment slots
/// (outermost first). At a given depth the same operation is its own inverse.
pub(crate) fn binding_slot(depth: usize, index: usize) -> Option<usize> {
    if index < depth {
        Some(depth - 1 - index)
    } else {
        None
    }
}

/// Look up an innermost-first de Bruijn index in an outermost-first context.
pub(crate) fn lookup_binding<T>(entries: &[T], index: usize) -> Option<&T> {
    match binding_slot(entries.len(), index) {
        Some(slot) => Some(&entries[slot]),
        None => None,
    }
}
