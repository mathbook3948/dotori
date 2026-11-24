use std::sync::Arc;
use std::time::Duration;

use gpui::Context;
use gpui::Entity;
use gpui::Focusable;
use gpui::FontWeight;
use gpui::MouseButton;
use gpui::SharedString;
use gpui::Task;
use gpui::Window;
use gpui::div;
use gpui::prelude::*;
use gpui::px;

use crate::home::Home;
use crate::notes::MAX_LENGTH;
use crate::notes::MAX_NOTES;
use crate::notes::Notes;
use crate::window::input::Submit;
use crate::window::input::TextInput;
use crate::window::parts::prose;
use crate::window::theme::Theme;

const REFRESH: Duration = Duration::from_secs(5);

/// The notes card in the settings window: what dotori keeps in mind for every answer, and a way
/// to add or drop a note by hand.
pub(crate) struct NotesView {
    notes: Arc<Notes>,
    shown: Vec<(i64, String)>,
    adding: Entity<TextInput>,
    failed: Option<String>,
    _refresh: Task<()>,
}

impl NotesView {
    pub(crate) fn new(home: &Home, cx: &mut Context<Self>) -> Self {
        let refresh = cx.spawn(async move |view, cx| {
            loop {
                cx.background_executor().timer(REFRESH).await;
                if view.update(cx, |view, cx| view.reload(cx)).is_err() {
                    return;
                }
            }
        });
        let adding = cx.new(|cx| TextInput::new("기억해 둘 말을 적어요", cx));
        cx.observe(&adding, |_, _, cx| cx.notify()).detach();
        let notes = Notes::new(home.notes());
        let shown = notes.list().unwrap_or_default();
        Self {
            notes,
            shown,
            adding,
            failed: None,
            _refresh: refresh,
        }
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let shown = self.notes.list().unwrap_or_default();
        if shown != self.shown {
            self.shown = shown;
            cx.notify();
        }
    }

    fn add(&mut self, cx: &mut Context<Self>) {
        let text = self.adding.read(cx).text().trim().to_owned();
        if text.is_empty() || self.adding.read(cx).composing() {
            return;
        }
        match self.notes.keep(&text) {
            Ok(_) => {
                self.adding.update(cx, |input, cx| {
                    input.take(cx);
                });
                self.failed = None;
            }
            Err(error) => {
                self.failed = Some(if error.contains("already") {
                    format!("메모는 {MAX_NOTES}개까지예요. 하나를 지우고 더해 주세요.")
                } else if error.contains("at most") {
                    format!("메모 하나는 {MAX_LENGTH}자까지예요.")
                } else {
                    error
                });
            }
        }
        self.reload(cx);
        cx.notify();
    }

    fn remove(&mut self, id: i64, cx: &mut Context<Self>) {
        self.failed = self.notes.drop_note(id).err();
        self.reload(cx);
        cx.notify();
    }
}

impl Render for NotesView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(window);
        let mut list = div().flex().flex_col().gap_1();
        if self.shown.is_empty() {
            list = list.child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_1()
                    .py_3()
                    .text_xs()
                    .text_color(theme.faint)
                    .child("아직 메모가 없어요")
                    .child("채팅창에서 \"앞으로 존댓말 써 줘\"처럼 부탁해도 돼요"),
            );
        }
        for (id, text) in &self.shown {
            let id = *id;
            list = list.child(
                div()
                    .id(SharedString::from(format!("note-{id}")))
                    .flex()
                    .items_start()
                    .gap_3()
                    .px_3()
                    .py_2()
                    .rounded_xl()
                    .hover(|row| row.bg(theme.canvas))
                    .child(
                        div()
                            .flex_none()
                            .mt(px(7.))
                            .size(px(6.))
                            .rounded_full()
                            .bg(theme.accent),
                    )
                    .child(div().flex_1().min_w_0().child(prose(text)))
                    .child(
                        div()
                            .id(SharedString::from(format!("note-remove-{id}")))
                            .flex_none()
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .text_xs()
                            .text_color(theme.muted)
                            .cursor_pointer()
                            .hover(|remove| remove.bg(theme.danger_soft).text_color(theme.danger))
                            .child("지우기")
                            .on_click(cx.listener(move |view, _, _, cx| view.remove(id, cx))),
                    ),
            );
        }
        let focus = self.adding.focus_handle(cx);
        let focused = focus.is_focused(window);
        let ready = !self.adding.read(cx).text().trim().is_empty();
        let adder = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .id("note-input")
                    .flex_1()
                    .min_w_0()
                    .px_3()
                    .py_2()
                    .rounded_lg()
                    .bg(theme.canvas)
                    .border_1()
                    .border_color(if focused { theme.accent } else { theme.line })
                    .cursor_text()
                    .on_mouse_down(MouseButton::Left, move |_, window, _| window.focus(&focus))
                    .on_action(cx.listener(|view, _: &Submit, _, cx| view.add(cx)))
                    .child(self.adding.clone()),
            )
            .child(
                div()
                    .id("note-add")
                    .flex_none()
                    .px_4()
                    .py_2()
                    .rounded_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .when(ready, |add| {
                        add.cursor_pointer()
                            .bg(theme.accent)
                            .text_color(theme.accent_text)
                            .hover(|add| add.bg(theme.accent_hover))
                    })
                    .when(!ready, |add| add.bg(theme.line).text_color(theme.faint))
                    .child("더하기")
                    .on_click(cx.listener(|view, _, _, cx| view.add(cx))),
            );
        let failed = self.failed.clone().map(|failed| {
            div()
                .px_3()
                .py_1p5()
                .rounded_lg()
                .bg(theme.danger_soft)
                .text_color(theme.danger)
                .text_xs()
                .child(prose(&failed))
        });
        div()
            .flex()
            .flex_col()
            .gap_4()
            .p_5()
            .rounded_2xl()
            .bg(theme.surface)
            .border_1()
            .border_color(theme.line)
            .shadow_xs()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_base()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("메모"),
                    )
                    .child(div().text_xs().text_color(theme.muted).child(format!(
                        "dotori가 답할 때마다 기억하는 말이에요. 사실도 말투 같은 규칙도 적어 둘 수 있어요. ({}/{MAX_NOTES})",
                        self.shown.len()
                    ))),
            )
            .child(list)
            .child(adder)
            .children(failed)
    }
}
