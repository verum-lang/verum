//! Both-polarity checks for the cell editor's power features (owner
//! request 2026-08-23: "мощный редактор" + >1 MB buffers).  Every
//! feature is exercised through the PUBLIC EditorState surface with a
//! positive and a negative/inverse assertion.
#![cfg(feature = "playbook")]

use verum_interactive::playbook::ui::EditorState;

fn ed(content: &str) -> EditorState {
    let mut e = EditorState::new();
    e.set_content(content);
    e
}

#[test]
fn auto_indent_inherits_and_opens_blocks() {
    let mut e = ed("fn main() {");
    e.cursor = (0, 11);
    e.insert_newline_auto_indent();
    assert_eq!(e.lines[1], "    ", "one level added after an opener");
    // Plain continuation inherits the indent without adding a level.
    e.insert_str("let x = 1;");
    e.insert_newline_auto_indent();
    assert_eq!(e.lines[2], "    ", "continuation keeps the indent");
}

#[test]
fn auto_indent_splits_a_brace_pair_into_a_block() {
    let mut e = ed("fn f() {}");
    e.cursor = (0, 8); // between { and }
    e.insert_newline_auto_indent();
    assert_eq!(e.lines.len(), 3);
    assert_eq!(e.lines[1], "    ", "cursor line is the indented body");
    assert_eq!(e.lines[2], "}", "closer moved to its own line");
    assert_eq!(e.cursor, (1, 4));
}

#[test]
fn smart_insert_closes_pairs_and_skips_over_closers() {
    let mut e = ed("");
    e.insert_char_smart('(');
    assert_eq!(e.lines[0], "()", "opener auto-closes");
    assert_eq!(e.cursor.1, 1, "cursor sits inside the pair");
    e.insert_char_smart(')');
    assert_eq!(e.lines[0], "()", "typing the closer skips over it");
    assert_eq!(e.cursor.1, 2);
    // Negative: an opener typed right before a word stays literal.
    let mut e2 = ed("word");
    e2.cursor = (0, 0);
    e2.insert_char_smart('(');
    assert_eq!(e2.lines[0], "(word", "no auto-close before a word");
}

#[test]
fn smart_insert_wraps_a_selection() {
    let mut e = ed("abc");
    e.select_all();
    e.insert_char_smart('(');
    assert_eq!(e.lines[0], "(abc)", "selection wrapped, not replaced");
}

#[test]
fn backspace_removes_an_empty_pair_whole() {
    let mut e = ed("");
    e.insert_char_smart('[');
    e.backspace();
    assert_eq!(e.lines[0], "", "both halves of the empty pair died");
    // Negative: a non-empty pair loses only one char.
    let mut e2 = ed("(x)");
    e2.cursor = (0, 2);
    e2.backspace();
    assert_eq!(e2.lines[0], "()");
}

#[test]
fn duplicate_delete_and_move_lines() {
    let mut e = ed("aaa\nbbb\nccc");
    e.cursor = (1, 0);
    e.duplicate_lines();
    assert_eq!(e.lines, vec!["aaa", "bbb", "bbb", "ccc"]);
    assert_eq!(e.cursor.0, 2, "cursor follows the copy");
    e.delete_lines();
    assert_eq!(e.lines, vec!["aaa", "bbb", "ccc"]);
    e.cursor = (2, 0);
    e.move_lines_up();
    assert_eq!(e.lines, vec!["aaa", "ccc", "bbb"]);
    e.move_lines_down();
    assert_eq!(e.lines, vec!["aaa", "bbb", "ccc"]);
    // Negative: moving the first line up is a no-op.
    e.cursor = (0, 0);
    e.move_lines_up();
    assert_eq!(e.lines, vec!["aaa", "bbb", "ccc"]);
}

#[test]
fn toggle_comment_both_polarities() {
    let mut e = ed("    let x = 1;\n    let y = 2;");
    e.select_all();
    e.toggle_comment();
    assert_eq!(e.lines[0], "    // let x = 1;");
    assert_eq!(e.lines[1], "    // let y = 2;");
    e.select_all();
    e.toggle_comment();
    assert_eq!(e.lines[0], "    let x = 1;", "second toggle removes");
    assert_eq!(e.lines[1], "    let y = 2;");
}

#[test]
fn indent_and_dedent_lines() {
    let mut e = ed("a\nb");
    e.select_all();
    e.indent_lines();
    assert_eq!(e.lines, vec!["    a", "    b"]);
    e.select_all();
    e.dedent_lines();
    assert_eq!(e.lines, vec!["a", "b"]);
    // Negative: dedent at column zero stays put.
    e.dedent_lines();
    assert_eq!(e.lines, vec!["a", "b"]);
}

#[test]
fn smart_home_toggles_between_indent_and_zero() {
    let mut e = ed("    body");
    e.cursor = (0, 8);
    e.move_home_smart(false);
    assert_eq!(e.cursor.1, 4, "first Home → first non-whitespace");
    e.move_home_smart(false);
    assert_eq!(e.cursor.1, 0, "second Home → column zero");
    e.move_home_smart(false);
    assert_eq!(e.cursor.1, 4, "third Home → back to the indent");
}

#[test]
fn kill_to_eol_and_join_lines() {
    let mut e = ed("hello world\nnext");
    e.cursor = (0, 5);
    e.kill_to_eol();
    assert_eq!(e.lines[0], "hello");
    e.kill_to_eol(); // at EOL: joins the next line
    assert_eq!(e.lines[0], "hellonext");

    let mut e2 = ed("first\n    second");
    e2.cursor = (0, 0);
    e2.join_lines();
    assert_eq!(e2.lines[0], "first second", "join trims and spaces");
}

#[test]
fn word_deletion_left_and_right() {
    let mut e = ed("alpha beta gamma");
    e.cursor = (0, 10); // after "beta"
    e.delete_word_left();
    assert_eq!(e.lines[0], "alpha  gamma");
    let mut e2 = ed("alpha beta");
    e2.cursor = (0, 0);
    e2.delete_word_right();
    assert_eq!(e2.lines[0], "beta");
}

#[test]
fn undo_coalesces_typed_words_but_not_structure() {
    let mut e = ed("");
    for c in "hello".chars() {
        e.insert_char(c);
    }
    assert!(e.undo(), "undo undoes something");
    assert_eq!(e.lines[0], "", "the WHOLE word is one undo entry");
    assert!(e.redo());
    assert_eq!(e.lines[0], "hello", "redo restores the word");
    // Structural edits are separate entries.
    e.duplicate_lines();
    e.toggle_comment();
    assert!(e.undo());
    assert!(!e.lines[e.cursor.0].contains("//"), "comment undone alone");
}

#[test]
fn undo_is_a_line_span_delta_not_a_buffer_clone() {
    // 1 MB-class buffer: 40k lines.  Type a char and undo — both must
    // be effectively instant and correct.  (The old implementation
    // cloned all 40k lines per keystroke.)
    let big: String = (0..40_000)
        .map(|i| format!("let v{i} = {i};\n"))
        .collect();
    let mut e = ed(&big);
    e.cursor = (20_000, 0);
    let t0 = std::time::Instant::now();
    for c in "changed".chars() {
        e.insert_char(c);
    }
    assert!(e.undo());
    let elapsed = t0.elapsed();
    assert_eq!(e.lines[20_000], "let v20000 = 20000;");
    assert_eq!(e.lines.len(), 40_000);
    assert!(
        elapsed.as_millis() < 200,
        "typing+undo on a 40k-line buffer took {elapsed:?} — snapshots \
         must be line-span deltas, not full clones"
    );
}

#[test]
fn undo_restores_multiline_operations_exactly() {
    let original = "one\ntwo\nthree";
    let mut e = ed(original);
    e.cursor = (1, 0);
    e.insert_newline_auto_indent(); // splits line 1
    assert_eq!(e.lines.len(), 4);
    assert!(e.undo());
    assert_eq!(e.content(), original, "split undone to the exact buffer");

    let mut e2 = ed(original);
    e2.selection = Some(verum_interactive::playbook::ui::Selection {
        start: (0, 1),
        end: (2, 2),
    });
    e2.backspace(); // deletes the multi-line selection
    assert_eq!(e2.content(), "oree");
    assert!(e2.undo());
    assert_eq!(e2.content(), original, "selection delete fully undone");
}

#[test]
fn bracket_matching_finds_the_pair_and_rejects_unbalanced() {
    let e = {
        let mut e = ed("fn f(a: Int) {\n    (a)\n}");
        e.cursor = (0, 13); // on '{'
        e
    };
    let m = e.matching_bracket().expect("brace pair found");
    assert_eq!(m.0, (0, 13));
    assert_eq!(m.1, (2, 0), "match crosses lines");
    let mut e2 = ed("((x)");
    e2.cursor = (0, 0);
    assert!(
        e2.matching_bracket().is_none(),
        "unbalanced opener has no match"
    );
}

#[test]
fn horizontal_scroll_follows_the_cursor() {
    let long = "x".repeat(500);
    let mut e = ed(&long);
    e.cursor = (0, 400);
    e.ensure_cursor_visible_h(80);
    assert!(e.h_scroll > 300, "window panned to the cursor");
    assert!(e.h_scroll <= 400, "cursor stays inside the window");
    e.cursor = (0, 0);
    e.ensure_cursor_visible_h(80);
    assert_eq!(e.h_scroll, 0, "returning home pans back");
}

#[test]
fn page_move_walks_a_page_and_clamps() {
    let mut e = ed(&(0..100).map(|i| format!("l{i}\n")).collect::<String>());
    e.page_move(true, 30, false);
    assert_eq!(e.cursor.0, 30);
    e.page_move(false, 300, false);
    assert_eq!(e.cursor.0, 0, "clamped at the top");
    e.page_move(true, 10_000, false);
    assert_eq!(e.cursor.0, e.lines.len() - 1, "clamped at the bottom");
}
