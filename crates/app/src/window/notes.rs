use std::sync::Arc;
use std::time::Duration;

use gpui::Context;
use gpui::FontWeight;
use gpui::SharedString;
use gpui::Task;
use gpui::Window;
use gpui::div;
use gpui::prelude::*;

use crate::home::Home;
use crate::notes::MAX_NOTES;
use crate::notes::Notes;
use crate::window::parts::prose;
use crate::window::surface::SurfaceExt;
use crate::window::theme::Theme;
use crate::words::words;

const REFRESH: Duration = Duration::from_secs(5);

/// The notes card in the settings window: what dotori keeps in mind for every answer, and a way
/// to drop a note. Notes are added only by asking dotori in the chat.
pub(crate) struct NotesView {
    notes: Arc<Notes>,
    shown: Vec<(i64, String)>,
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
        let notes = Notes::new(home.notes());
        let shown = notes.list().unwrap_or_default();
        Self {
            notes,
            shown,
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
                    .text_color(theme.muted)
                    .child(words().settings.no_notes)
                    .child(words().settings.notes_example),
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
                    .child(div().flex_none().text_color(theme.primary).child("•"))
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
                            .child(words().settings.remove)
                            .on_click(cx.listener(move |view, _, _, cx| view.remove(id, cx))),
                    ),
            );
        }
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
            .card(&theme)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_base()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(words().settings.notes_title),
                    )
                    .child(
                        div().text_xs().text_color(theme.muted).child((words()
                            .settings
                            .notes_about)(
                            self.shown.len(),
                            MAX_NOTES,
                        )),
                    ),
            )
            .child(list)
            .children(failed)
    }
}
