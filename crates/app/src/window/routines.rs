use std::sync::Arc;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

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
use crate::routines::Routine;
use crate::routines::Routines;
use crate::window::input::Submit;
use crate::window::input::TextInput;
use crate::window::parts::prose;
use crate::window::theme::Theme;

const REFRESH: Duration = Duration::from_secs(5);

/// The routines card in the settings window: the time zone every routine is read in, what
/// dotori does at set times, and a way to stop each.
pub(crate) struct RoutinesView {
    routines: Arc<Routines>,
    shown: Vec<Routine>,
    zone: Entity<TextInput>,
    stored: Option<String>,
    zone_saved: bool,
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
        let stored = routines.zone_name().ok().flatten();
        let zone = cx.new(|cx| {
            let mut input = TextInput::new("이 PC의 시간대", cx);
            if let Some(stored) = &stored {
                input.set_text(stored, cx);
            }
            input
        });
        cx.observe(&zone, |_, _, cx| cx.notify()).detach();
        Self {
            shown: routines.list().unwrap_or_default(),
            routines,
            zone,
            stored,
            zone_saved: false,
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
        let stored = self.routines.zone_name().ok().flatten();
        if stored != self.stored {
            if !self.zone_changed(cx) {
                let text = stored.clone().unwrap_or_default();
                self.zone.update(cx, |input, cx| input.set_text(&text, cx));
            }
            self.stored = stored;
            self.zone_saved = false;
            cx.notify();
        }
    }

    fn zone_changed(&self, cx: &Context<Self>) -> bool {
        self.zone.read(cx).text().trim() != self.stored.as_deref().unwrap_or_default()
    }

    fn remove(&mut self, id: i64, cx: &mut Context<Self>) {
        self.failed = self.routines.remove(id, now_ms()).err();
        self.reload(cx);
        cx.notify();
    }

    fn save_zone(&mut self, cx: &mut Context<Self>) {
        let typed = self.zone.read(cx).text().trim().to_owned();
        let name = (!typed.is_empty()).then_some(typed);
        match self.routines.set_zone(name.as_deref(), now_ms()) {
            Ok(()) => {
                self.failed = None;
                self.zone_saved = true;
                self.stored = self.routines.zone_name().ok().flatten();
                let text = self.stored.clone().unwrap_or_default();
                self.zone.update(cx, |input, cx| input.set_text(&text, cx));
            }
            Err(failed) => {
                self.failed = Some(failed);
                self.zone_saved = false;
            }
        }
        cx.notify();
    }

    fn zone_row(&self, theme: &Theme, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focus = self.zone.focus_handle(cx);
        let focused = focus.is_focused(window);
        let changed = self.zone_changed(cx);
        let here = jiff::tz::TimeZone::system()
            .iana_name()
            .map_or_else(|| "이 PC의 시간대".to_owned(), str::to_owned);
        let hint = match (&self.stored, self.zone_saved && !changed) {
            (Some(_), true) => "바꿨어요. 다음 루틴부터 모든 PC가 이 시간대로 읽어요.".to_owned(),
            (None, true) => "바꿨어요. 다음 루틴부터 각 PC가 자기 시간대로 읽어요.".to_owned(),
            (Some(_), false) => format!(
                "모든 PC가 이 시간대로 루틴을 읽어요. 비우면 각 PC의 시간대를 써요. 이 PC의 시간대: {here}"
            ),
            (None, false) => format!(
                "지금은 각 PC의 시간대로 읽어요. Asia/Seoul처럼 적으면 모든 PC가 같은 시간대를 써요. 이 PC의 시간대: {here}"
            ),
        };
        let save = div()
            .id("zone-save")
            .debug_selector(|| "zone-save".to_owned())
            .flex_none()
            .px_4()
            .py_2()
            .rounded_lg()
            .font_weight(FontWeight::SEMIBOLD);
        let save = if changed {
            save.cursor_pointer()
                .bg(theme.primary)
                .hover(|save| save.bg(theme.primary_hover))
                .text_color(theme.on_primary)
                .on_click(cx.listener(|view, _, _, cx| view.save_zone(cx)))
        } else {
            save.bg(theme.bubble).text_color(theme.muted)
        };
        div()
            .flex()
            .flex_col()
            .gap_1p5()
            .child(div().font_weight(FontWeight::MEDIUM).child("시간대"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .id("zone")
                            .debug_selector(|| "zone".to_owned())
                            .flex_1()
                            .min_w_0()
                            .px_3()
                            .py_2()
                            .rounded_lg()
                            .bg(theme.canvas)
                            .border_1()
                            .border_color(if focused { theme.primary } else { theme.line })
                            .cursor_text()
                            .on_mouse_down(MouseButton::Left, move |_, window, _| {
                                window.focus(&focus);
                            })
                            .on_action(cx.listener(|view, _: &Submit, _, cx| {
                                if view.zone_changed(cx) {
                                    view.save_zone(cx);
                                }
                            }))
                            .child(self.zone.clone()),
                    )
                    .child(save.child("바꾸기")),
            )
            .child(div().text_xs().text_color(theme.muted).child(prose(&hint)))
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
                    .child("아직 루틴이 없어요")
                    .child("채팅창에서 \"평일 아침 8시에 오늘 일정 알려줘\"처럼 부탁해 보세요"),
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
                            .child("지우기")
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
                            .child("루틴"),
                    )
                    .child(div().text_xs().text_color(theme.muted).child(
                        "정해 둔 시간에 dotori가 알아서 하는 일이에요. 결과는 채팅창에 보여요.",
                    )),
            )
            .child(self.zone_row(&theme, window, cx))
            .child(list)
            .children(failed)
    }
}

#[cfg(test)]
#[path = "routines.test.rs"]
mod tests;
