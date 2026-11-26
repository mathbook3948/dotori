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
