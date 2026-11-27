use std::sync::Arc;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use gpui::Context;
use gpui::FontWeight;
use gpui::SharedString;
use gpui::Task;
use gpui::Window;
use gpui::div;
use gpui::prelude::*;
use gpui::px;

use crate::home::Home;
use crate::routines::Routine;
use crate::routines::Routines;
use crate::window::parts::link_line;
use crate::window::parts::prose;
use crate::window::settings::GoTo;
use crate::window::settings::Part;
use crate::window::theme::Theme;
use crate::words::words;

const REFRESH: Duration = Duration::from_secs(5);

impl gpui::EventEmitter<GoTo> for RoutinesView {}

/// The routines card in the settings window: what dotori does at set times, and a way to stop
/// each.
pub(crate) struct RoutinesView {
    routines: Arc<Routines>,
    shown: Vec<Routine>,
    failed: Option<String>,
    _refresh: Task<()>,
}

impl RoutinesView {
    pub(crate) fn new(home: &Home, cx: &mut Context<Self>) -> Self {
        let refresh = cx.spawn(async move |view, cx| {
            loop {
                cx.background_executor().timer(REFRESH).await;
                if view.update(cx, |view, cx| view.reload(cx)).is_err() {
                    return;
                }
            }
        });
        let routines = Routines::new(home.routines());
        Self {
            shown: routines.list().unwrap_or_default(),
            routines,
            failed: None,
            _refresh: refresh,
        }
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let shown = self.routines.list().unwrap_or_default();
        if shown != self.shown {
            self.shown = shown;
            cx.notify();
        }
    }

    fn remove(&mut self, id: i64, cx: &mut Context<Self>) {
        self.failed = self.routines.remove(id, now_ms()).err();
        self.reload(cx);
        cx.notify();
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| i64::try_from(since.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

impl Render for RoutinesView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(window);
        let strings = &words().routines;
        let mut list = div().flex().flex_col().gap_1();
        if self.shown.is_empty() {
            list = list.child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_1()
                    .py_4()
                    .text_xs()
                    .text_color(theme.muted)
                    .child(strings.empty)
                    .child(strings.empty_ask),
            );
        }
        for routine in &self.shown {
            let id = routine.id;
            list = list.child(
                div()
                    .id(SharedString::from(format!("routine-{id}")))
                    .flex()
                    .items_start()
                    .gap_3()
                    .px_3()
                    .py_2p5()
                    .rounded_xl()
                    .hover(|row| row.bg(theme.canvas))
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .w(px(56.))
                            .py_1()
                            .rounded_lg()
                            .bg(theme.selection)
                            .text_color(theme.text)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(routine.time.clone()),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap_0p5()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(routine.name.clone()),
                                    )
                                    .child(
                                        div()
                                            .px_1p5()
                                            .rounded_sm()
                                            .bg(theme.bubble)
                                            .text_xs()
                                            .text_color(theme.muted)
                                            .child(routine.when()),
                                    ),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted)
                                    .child(prose(&routine.task)),
                            ),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("routine-remove-{id}")))
                            .flex_none()
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .text_xs()
                            .text_color(theme.muted)
                            .cursor_pointer()
                            .hover(|remove| remove.bg(theme.danger_soft).text_color(theme.danger))
                            .child(strings.remove)
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
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_base()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(strings.title),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted)
                            .child((strings.caption)()),
                    )
                    .child(link_line(
                        &theme,
                        "zone-general",
                        strings.zone_moved,
                        cx.listener(|_, _, _, cx| cx.emit(GoTo(Part::Zone))),
                    )),
            )
            .child(list)
            .children(failed)
    }
}
