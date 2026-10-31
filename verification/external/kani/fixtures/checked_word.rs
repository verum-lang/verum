fn successor(value: u64) -> Option<u64> {
    value.checked_add(1)
}

#[kani::proof]
fn exact_word_successor() {
    let value: u64 = kani::any();
    kani::cover!(value == 0, "zero");
    kani::cover!(value == u64::MAX, "limit");
    kani::cover!(value == u64::MAX - 1, "last_successor");
    let observed = successor(value);
    if value == u64::MAX {
        assert_eq!(observed, None);
    } else {
        assert_eq!(observed, Some(value + 1));
    }
}
