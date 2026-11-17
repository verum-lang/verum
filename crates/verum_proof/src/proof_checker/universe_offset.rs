use super::Level;

/// Encode an exact shift using one wire node per full u32 chunk on the input path.
pub(super) fn level_with_offset(base: Level, amount: u128) -> Level {
    match base {
        Level::Concrete(n) => {
            let capacity = (u32::MAX - n) as u128;
            if amount <= capacity {
                Level::Concrete(n + amount as u32)
            } else {
                append_wire_chunks(Level::Concrete(u32::MAX), amount - capacity)
            }
        }
        other => append_wire_chunks(other, amount),
    }
}

fn append_wire_chunks(mut base: Level, mut amount: u128) -> Level {
    const CHUNK: u128 = u32::MAX as u128;
    while amount > CHUNK {
        base = Level::Offset(Box::new(base), u32::MAX);
        amount -= CHUNK;
    }
    match amount as u32 {
        0 => base,
        1 => Level::Succ(Box::new(base)),
        remainder => Level::Offset(Box::new(base), remainder),
    }
}

/// Resulting level of the T-Universe rule in both native checkers.
pub(crate) fn universe_successor(base: Level) -> Level {
    level_with_offset(base, 1)
}
