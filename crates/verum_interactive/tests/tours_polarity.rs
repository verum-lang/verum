//! The one-truth-of-examples law (T0858 slice 6): the guided tours
//! are BUILT from docs/by-example at compile time, every complete
//! chapter ships in some tour, and the explanation cells do not
//! duplicate the runnable code.

use verum_interactive::discovery::tutorials::{
    BY_EXAMPLE_CHAPTERS, builtin_tutorials,
};

#[test]
fn every_embedded_chapter_ships_in_some_tour() {
    let tours = builtin_tutorials();
    let steps_total: usize = tours.iter().map(|t| t.steps.len()).sum();
    assert_eq!(
        steps_total,
        BY_EXAMPLE_CHAPTERS.len(),
        "each chapter appears exactly once across the tours — a new \
         chapter must never ship silently nowhere (tours: {:?})",
        tours
            .iter()
            .map(|t| (t.title.clone(), t.steps.len()))
            .collect::<Vec<_>>()
    );
}

#[test]
fn tours_carry_real_teaching_content() {
    let tours = builtin_tutorials();
    assert!(tours.len() >= 4, "the four curated tours exist");
    for tour in &tours {
        assert!(!tour.steps.is_empty(), "empty tour: {}", tour.title);
        for step in &tour.steps {
            assert!(
                step.example_code.as_deref().is_some_and(|c| !c.is_empty()),
                "step without runnable code in {}: {}",
                tour.title,
                step.title
            );
            assert!(
                !step.explanation.contains("## The code"),
                "the explanation duplicates the runnable cell in {}: {}",
                tour.title,
                step.title
            );
        }
    }
}

#[test]
fn the_first_tour_starts_at_hello_world() {
    let tours = builtin_tutorials();
    let first = &tours[0];
    assert!(
        first.steps[0].title.to_lowercase().contains("hello"),
        "the newcomer's first step is hello-world, got {}",
        first.steps[0].title
    );
}
