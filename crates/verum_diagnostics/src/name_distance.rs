//! Unicode edit distance with reusable linear storage and a bounded search band.
use verum_common::List;

/// One query compared against many diagnostic candidates. Case folding and
/// suggestion ranking remain the caller's policy.
pub struct EditDistance {
    query: List<char>,
    previous: List<usize>,
    current: List<usize>,
}

impl EditDistance {
    pub fn new(query: &str) -> Self {
        let query: List<char> = query.chars().collect();
        let mut previous = List::new();
        previous.resize(query.len() + 1, 0);
        let current = previous.clone();
        Self {
            query,
            previous,
            current,
        }
    }

    /// Returns the exact distance if it is at most `limit`, otherwise `None`.
    /// An edit changes character count by at most one, so paths outside the
    /// diagonal band cannot satisfy the bound. No approximate scores escape.
    pub fn within(&mut self, candidate: &str, limit: usize) -> Option<usize> {
        let m = self.query.len();
        let n = candidate.chars().count();
        if m.abs_diff(n) > limit {
            return None;
        }
        if m == 0 || n == 0 {
            return Some(m.max(n));
        }
        let limit = limit.min(m.max(n));
        let outside = limit + 1;
        for (j, value) in self.previous.iter_mut().enumerate() {
            *value = j.min(outside);
        }
        for (index, character) in candidate.chars().enumerate() {
            let i = index + 1;
            let first = i.saturating_sub(limit).max(1);
            let last = i.saturating_add(limit).min(m);
            self.current[0] = i.min(outside);
            if first > 1 {
                self.current[first - 1] = outside;
            }
            let mut row_min = self.current[0];
            for j in first..=last {
                let substitution =
                    self.previous[j - 1] + usize::from(self.query[j - 1] != character);
                let value = substitution
                    .min(self.previous[j] + 1)
                    .min(self.current[j - 1] + 1);
                self.current[j] = value;
                row_min = row_min.min(value);
            }
            if last < m {
                self.current[last + 1] = outside;
            }
            if row_min > limit {
                return None;
            }
            std::mem::swap(&mut self.previous, &mut self.current);
        }
        (self.previous[m] <= limit).then_some(self.previous[m])
    }
}

/// Exact, unrestricted distance, using the same algorithm and linear storage.
pub fn levenshtein_distance(a: &str, b: &str) -> usize {
    EditDistance::new(a)
        .within(b, usize::MAX)
        .expect("every finite string pair has a finite edit distance")
}
