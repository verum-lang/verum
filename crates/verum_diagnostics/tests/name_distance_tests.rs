use verum_common::{List, Text};
use verum_diagnostics::name_distance::{EditDistance, levenshtein_distance};

// Independent full-matrix oracle: exhaustive tiny inputs check all band edges,
// Unicode character semantics and workspace reuse after a failed search.
fn oracle(a: &str, b: &str) -> usize {
    let a: List<char> = a.chars().collect();
    let b: List<char> = b.chars().collect();
    let mut matrix = vec![vec![0; b.len() + 1]; a.len() + 1];
    for i in 0..=a.len() {
        matrix[i][0] = i;
    }
    for j in 0..=b.len() {
        matrix[0][j] = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            matrix[i][j] = (matrix[i - 1][j] + 1)
                .min(matrix[i][j - 1] + 1)
                .min(matrix[i - 1][j - 1] + usize::from(a[i - 1] != b[j - 1]));
        }
    }
    matrix[a.len()][b.len()]
}

#[test]
fn bounded_unicode_distance_matches_exhaustive_oracle() {
    let mut inputs = vec![Text::new()];
    let mut frontier = inputs.clone();
    for _ in 0..4 {
        frontier = frontier
            .iter()
            .flat_map(|prefix| ['a', 'ж', '🦀'].map(|c| Text::from(format!("{prefix}{c}"))))
            .collect();
        inputs.extend(frontier.clone());
    }
    for a in &inputs {
        let mut query = EditDistance::new(a);
        for b in &inputs {
            let exact = oracle(a, b);
            assert_eq!(levenshtein_distance(a, b), exact);
            for limit in [0, 1, 2, 3, 4, usize::MAX, 0] {
                assert_eq!(
                    query.within(b, limit),
                    (exact <= limit).then_some(exact),
                    "{a:?} {b:?} {limit}"
                );
            }
        }
    }
}

#[test]
fn long_distant_names_are_rejected_and_workspace_remains_reusable() {
    let a = "a".repeat(20_000);
    let b = "б".repeat(20_000);
    let mut query = EditDistance::new(&a);
    assert_eq!(query.within(&b, 2), None);
    assert_eq!(query.within("", 2), None);
    assert_eq!(query.within(&a, 2), Some(0));
    assert_eq!(query.within(&(a + "ж"), 2), Some(1));
}

#[test]
fn public_suggestions_preserve_ranking_thresholds_and_ties() {
    let candidates: List<Text> = ["cart", "care", "cat", "cut", "cattle", "unrelated"]
        .into_iter()
        .map(Text::from)
        .collect();
    let expected: List<Text> = ["cat", "cart", "cut", "care", "cattle"]
        .into_iter()
        .map(Text::from)
        .collect();
    assert_eq!(
        verum_diagnostics::context_error::find_similar_contexts("CAT", &candidates),
        expected
    );
    let recovery = verum_diagnostics::recovery::ErrorRecovery::with_config(1, 0, false);
    let expected: List<Text> = ["cat", "cart", "cut"].into_iter().map(Text::from).collect();
    assert_eq!(recovery.suggest_similar_names("cat", &candidates), expected);
    assert_eq!(
        verum_diagnostics::recovery::find_closest_name("cat", ["cart", "cut"], 1),
        Some("cart")
    );
}
