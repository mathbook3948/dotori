use std::cell::Cell;
use std::rc::Rc;

use gpui::DispatchPhase;
use gpui::Modifiers;
use gpui::TestAppContext;
use gpui::canvas;
use gpui::point;

use super::*;
use crate::window::theme::Theme;

struct Strip {
    heard: Rc<Cell<usize>>,
}

impl Render for Strip {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let heard = self.heard.clone();
        div()
            .size_full()
            .child(
                canvas(
                    |_, _, _| {},
                    move |_, _, window, _| {
                        let heard = heard.clone();
                        window.on_mouse_event(move |_: &MouseDownEvent, phase, _, _| {
                            if phase == DispatchPhase::Bubble {
                                heard.set(heard.get() + 1);
                            }
                        });
                    },
                )
                .absolute()
                .size_full(),
            )
            .child(Frame::of(window).strip(&Theme::of(window), div(), div()))
    }
}

#[gpui::test]
fn on_windows_every_press_on_the_strip_is_left_to_the_system(cx: &mut TestAppContext) {
    let heard = Rc::new(Cell::new(0));
    let (_, cx) = cx.add_window_view(|_, _| Strip {
        heard: heard.clone(),
    });
    let on_strip = point(px(200.), px(TOP_HEIGHT / 2.));

    for click_count in [1, 2] {
        cx.simulate_event(MouseDownEvent {
            button: MouseButton::Left,
            position: on_strip,
            modifiers: Modifiers::none(),
            click_count,
            first_mouse: false,
        });
        cx.simulate_mouse_up(on_strip, MouseButton::Left, Modifiers::none());
    }

    assert_eq!(
        heard.get(),
        2,
        "the system moves the window on one press and fills the screen on two only if they pass"
    );
}
