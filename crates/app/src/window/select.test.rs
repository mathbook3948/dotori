use gpui::black;

use super::*;

fn conversation() -> Selection {
    let selection = Selection::default();
    selection.begin(black());
    let owner = selection.pen(0);
    owner.word("오늘", Joint::Newline, true);
    owner.word("날짜", Joint::Space, true);
    let reply = selection.pen(1);
    reply.word("PC", Joint::Newline, true);
    reply.word("날짜는", Joint::Space, true);
    reply.word("**2026**", Joint::Joined, true);
    reply.word("이에요.", Joint::Newline, true);
    selection
}

fn select(selection: &Selection, from: (usize, usize, usize), to: (usize, usize, usize)) {
    let spot = |(line, piece, byte)| Spot {
        key: Key { line, piece },
        byte,
    };
    let mut state = selection.0.borrow_mut();
    state.anchor = Some(spot(from));
    state.head = Some(spot(to));
}

#[test]
fn copying_joins_the_selected_words_the_way_they_were_written() {
    let selection = conversation();

    select(&selection, (0, 0, 3), (1, 3, 3));

    assert_eq!(
        selection.copied().as_deref(),
        Some("늘 날짜\n\nPC 날짜는**2026**\n이"),
        "a space between words, a line between paragraphs and a blank line between messages"
    );
}

#[test]
fn a_selection_dragged_backwards_copies_the_same_words() {
    let selection = conversation();

    select(&selection, (1, 1, 6), (0, 1, 0));

    assert_eq!(selection.copied().as_deref(), Some("날짜\n\nPC 날짜"));
}

#[test]
fn a_press_without_a_drag_selects_nothing() {
    let selection = conversation();

    select(&selection, (1, 1, 3), (1, 1, 3));

    assert_eq!(selection.copied(), None);
    assert!(!selection.clear(), "there was nothing to let go");
}

#[test]
fn only_the_selected_bytes_of_each_word_are_marked() {
    let selection = conversation();
    select(&selection, (0, 1, 3), (1, 1, 3));
    let state = selection.0.borrow();
    let marked = |line, piece, len| state.marked_in(Key { line, piece }, len);

    assert_eq!(marked(0, 0, 6), None, "before the start");
    assert_eq!(marked(0, 1, 6), Some(3..6), "from where it starts");
    assert_eq!(marked(1, 0, 2), Some(0..2), "all of a word in between");
    assert_eq!(marked(1, 1, 9), Some(0..3), "up to where it ends");
    assert_eq!(marked(1, 2, 8), None, "after the end");
}

fn around(
    selection: &Selection,
    at: (usize, usize, usize),
    pick: fn(&State, Spot) -> (Spot, Spot),
) -> Option<String> {
    let (line, piece, byte) = at;
    let ends = pick(
        &selection.0.borrow(),
        Spot {
            key: Key { line, piece },
            byte,
        },
    );
    let mut state = selection.0.borrow_mut();
    state.anchor = Some(ends.0);
    state.head = Some(ends.1);
    drop(state);
    selection.copied()
}

#[test]
fn two_presses_take_the_word_without_the_marks_around_it() {
    let selection = conversation();
    let word = |at| around(&selection, at, State::word_around);

    assert_eq!(word((1, 1, 3)).as_deref(), Some("날짜는"));
    assert_eq!(
        word((1, 3, 3)).as_deref(),
        Some("이에요"),
        "without the period"
    );
    assert_eq!(
        word((1, 2, 3)).as_deref(),
        Some("2026"),
        "without the bold marks"
    );
    assert_eq!(
        word((1, 3, 9)).as_deref(),
        Some("."),
        "a press on the mark takes the mark"
    );
}

#[test]
fn a_word_runs_on_through_a_bold_part_next_to_it() {
    let selection = Selection::default();
    selection.begin(black());
    let pen = selection.pen(0);
    pen.word("굵은", Joint::Newline, false);
    pen.word("글자로", Joint::Joined, true);
    pen.word("써요", Joint::Space, true);

    assert_eq!(
        around(&selection, (0, 1, 3), State::word_around).as_deref(),
        Some("굵은글자로")
    );
}

#[test]
fn three_presses_take_the_line_up_to_where_it_breaks() {
    let selection = conversation();
    let line = |at| around(&selection, at, State::line_around);

    assert_eq!(line((1, 1, 3)).as_deref(), Some("PC 날짜는**2026**"));
    assert_eq!(line((1, 0, 0)).as_deref(), Some("PC 날짜는**2026**"));
    assert_eq!(
        line((1, 3, 3)).as_deref(),
        Some("이에요."),
        "the line after the break"
    );
    assert_eq!(
        line((0, 0, 0)).as_deref(),
        Some("오늘 날짜"),
        "not into the next message"
    );
}
