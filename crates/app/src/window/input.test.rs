use gpui::TestAppContext;

use super::*;

struct Narrow {
    input: Entity<TextInput>,
}

impl Render for Narrow {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(80.)).child(self.input.clone())
    }
}

#[gpui::test]
fn the_input_method_box_sits_at_the_caret_of_a_scrolled_field(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| Narrow {
        input: cx.new(|cx| TextInput::new("", cx)),
    });
    let input = view.read_with(cx, |view, _| view.input.clone());
    input.update(cx, |input, cx| {
        input.set_text(
            "a long line that runs well past the edge of a narrow field",
            cx,
        )
    });
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();

    let (shift, field, caret) = input.update_in(cx, |input, window, cx| {
        let field = input.last_bounds.expect("drawn");
        let end = input.range_to_utf16(&(input.content.len()..input.content.len()));
        let caret = input
            .bounds_for_range(end, field, window, cx)
            .expect("a box");
        (input.shift, field, caret)
    });

    assert!(shift > Pixels::ZERO, "the field scrolled to the caret");
    assert!(
        caret.left() <= field.right() && caret.left() >= field.left(),
        "{caret:?} lies in {field:?}"
    );
}

fn focused_field(cx: &mut TestAppContext) -> (Entity<TextInput>, &mut gpui::VisualTestContext) {
    cx.update(|cx| cx.bind_keys(bindings()));
    let (view, cx) = cx.add_window_view(|_, cx| Narrow {
        input: cx.new(|cx| TextInput::multiline("", cx)),
    });
    let input = view.read_with(cx, |view, _| view.input.clone());
    cx.update(|window, cx| window.focus(&input.focus_handle(cx)));
    cx.run_until_parked();
    (input, cx)
}

fn text(input: &Entity<TextInput>, cx: &mut gpui::VisualTestContext) -> String {
    input.read_with(cx, |input, _| input.text().to_owned())
}

#[gpui::test]
fn undo_takes_back_what_was_typed_a_word_at_a_time_and_redo_puts_it_back(cx: &mut TestAppContext) {
    let (input, cx) = focused_field(cx);
    cx.simulate_input("hello big world");

    cx.simulate_keystrokes("secondary-z");
    assert_eq!(text(&input, cx), "hello big");
    cx.simulate_keystrokes("secondary-z");
    assert_eq!(text(&input, cx), "hello");
    cx.simulate_keystrokes("secondary-z");
    assert_eq!(text(&input, cx), "");

    cx.simulate_keystrokes("secondary-y");
    assert_eq!(text(&input, cx), "hello");
    cx.simulate_keystrokes("secondary-shift-z");
    assert_eq!(text(&input, cx), "hello big");
}

#[gpui::test]
fn deleting_in_a_row_is_taken_back_at_once_and_a_new_edit_drops_what_was_undone(
    cx: &mut TestAppContext,
) {
    let (input, cx) = focused_field(cx);
    cx.simulate_input("hello");
    cx.simulate_keystrokes("backspace backspace backspace");
    assert_eq!(text(&input, cx), "he");

    cx.simulate_keystrokes("secondary-z");
    assert_eq!(text(&input, cx), "hello");

    cx.simulate_keystrokes("secondary-z");
    cx.simulate_input("x");
    cx.simulate_keystrokes("secondary-y");
    assert_eq!(
        text(&input, cx),
        "x",
        "nothing is left to redo after typing"
    );
}

#[gpui::test]
fn undo_puts_back_a_selection_that_was_typed_over(cx: &mut TestAppContext) {
    let (input, cx) = focused_field(cx);
    cx.simulate_input("hello");
    cx.simulate_keystrokes("secondary-a");
    cx.simulate_input("bye");

    cx.simulate_keystrokes("secondary-z");

    input.read_with(cx, |input, _| {
        assert_eq!(input.text(), "hello");
        assert_eq!(
            input.selected_range,
            0..5,
            "the selection comes back with it"
        );
    });
}

#[gpui::test]
fn a_sent_message_is_not_brought_back(cx: &mut TestAppContext) {
    let (input, cx) = focused_field(cx);
    cx.simulate_input("sent");
    input.update(cx, |input, cx| input.take(cx));

    cx.simulate_keystrokes("secondary-z");

    assert_eq!(text(&input, cx), "");
}

#[gpui::test]
fn korean_put_together_by_an_input_method_is_taken_back_a_word_at_a_time(cx: &mut TestAppContext) {
    let (input, cx) = focused_field(cx);
    input.update_in(cx, |input, window, cx| {
        for word in [["ㅇ", "아", "안"], ["ㄴ", "녀", "녕"]] {
            for composing in word {
                input.replace_and_mark_text_in_range(None, composing, None, window, cx);
            }
            input.replace_text_in_range(None, word[2], window, cx);
        }
        input.replace_text_in_range(None, " ", window, cx);
        input.replace_and_mark_text_in_range(None, "ㅎ", None, window, cx);
        input.replace_text_in_range(None, "하", window, cx);
    });
    assert_eq!(text(&input, cx), "안녕 하");

    cx.simulate_keystrokes("secondary-z");
    assert_eq!(text(&input, cx), "안녕");
    cx.simulate_keystrokes("secondary-z");
    assert_eq!(text(&input, cx), "");
}
