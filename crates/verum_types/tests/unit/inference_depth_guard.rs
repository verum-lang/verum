use super::GlobalDepthGuard;
use verum_common::List;

#[test]
fn refused_entry_preserves_the_budget_owned_by_active_frames() {
    let mut frames = List::new();
    for _ in 0..100 {
        frames.push(GlobalDepthGuard::enter().unwrap());
    }
    for _ in 0..3 {
        assert!(GlobalDepthGuard::enter().is_err());
    }
    drop(frames.pop());
    let replacement = GlobalDepthGuard::enter().unwrap();
    assert!(GlobalDepthGuard::enter().is_err());
    drop(replacement);
    drop(frames);
    let _fresh = GlobalDepthGuard::enter().unwrap();
}
