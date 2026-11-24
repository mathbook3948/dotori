use std::time::Duration;

use dotori_connectors::channel::app::ToWindow;
use gpui::AnyElement;
use gpui::App;
use gpui::ClickEvent;
use gpui::ClipboardItem;
use gpui::Context;
use gpui::Entity;
use gpui::FocusHandle;
use gpui::Focusable;
use gpui::FontWeight;
use gpui::MouseButton;
use gpui::ScrollHandle;
use gpui::SharedString;
use gpui::Task;
use gpui::Window;
use gpui::div;
use gpui::prelude::*;
use gpui::px;
use gpui::relative;
use jiff::tz::TimeZone;
use tokio::sync::mpsc;

use crate::link::Link;
use crate::link::Update;
use crate::screen_link::ScreenLink;
use crate::window::input::Submit;
use crate::window::input::TextInput;
use crate::window::parts::header;
use crate::window::parts::logo;
use crate::window::parts::prose;
use crate::window::parts::status;
use crate::window::screen::ScreenView;
use crate::window::theme::Theme;
use crate::window::when::Mark;
use crate::window::when::marks;
use crate::window::when::today;

const COPIED_FOR: Duration = Duration::from_millis(1500);

/// The chat window: the conversation with dotori and the traffic of every other channel.
pub(crate) struct ChatView {
    lines: Vec<ToWindow>,
    screen: Entity<ScreenView>,
    tab: Tab,
    link: LinkState,
    answering: Option<(String, bool)>,
    browser_at: Option<(String, bool)>,
    input: Entity<TextInput>,
    say: mpsc::UnboundedSender<String>,
    scroll: ScrollHandle,
    zone: TimeZone,
    copied: Option<usize>,
    hovered: Option<usize>,
    _uncopy: Option<Task<()>>,
    _pump: Task<()>,
}

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Chat,
    Browser,
}

enum LinkState {
    Connecting,
    Connected,
    Down(String),
}

impl ChatView {
    pub(crate) fn new(link: Link, screen: ScreenLink, cx: &mut Context<Self>) -> Self {
        let Link { mut updates, say } = link;
        let screen = cx.new(|cx| ScreenView::new(screen, cx));
        cx.observe(&screen, |_, _, cx| cx.notify()).detach();
        let pump = cx.spawn(async move |view, cx| {
            while let Some(update) = updates.recv().await {
                let applied = view.update(cx, |view, cx| {
                    view.apply(update);
                    cx.notify();
                });
                if applied.is_err() {
                    return;
                }
            }
        });
        Self {
            lines: Vec::new(),
            screen,
            tab: Tab::Chat,
            link: LinkState::Connecting,
            answering: None,
            browser_at: None,
            input: cx.new(|cx| TextInput::multiline("dotori에게 말하기", cx)),
            say,
            scroll: ScrollHandle::new(),
            zone: TimeZone::system(),
            copied: None,
            hovered: None,
            _uncopy: None,
            _pump: pump,
        }
    }

    fn apply(&mut self, update: Update) {
        match update {
            Update::Connected => {
                self.lines.clear();
                self.copied = None;
                self.hovered = None;
                self.answering = None;
                self.browser_at = None;
                self.link = LinkState::Connected;
            }
            Update::Event(ToWindow::Answering { pc, here }) => {
                self.answering = Some((pc, here));
                return;
            }
            Update::Event(ToWindow::Alone) => {
                self.answering = None;
                return;
            }
            Update::Event(ToWindow::BrowserAt { pc, here }) => {
                self.browser_at = Some((pc, here));
                return;
            }
            Update::Down(reason) => self.link = LinkState::Down(reason),
            Update::Event(event) => self.lines.push(event),
        }
        self.scroll.scroll_to_bottom();
    }

    fn submit(&mut self, _: &Submit, _: &mut Window, cx: &mut Context<Self>) {
        self.send(cx);
    }

    fn send(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.link, LinkState::Connected) {
            return;
        }
        let text = self.input.update(cx, |input, cx| {
            if input.composing() || input.text().trim().is_empty() {
                None
            } else {
                Some(input.take(cx))
            }
        });
        if let Some(text) = text {
            let _ = self.say.send(text);
        }
    }

    fn copy(&mut self, n: usize, text: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.copied = Some(n);
        self._uncopy = Some(cx.spawn(async move |view, cx| {
            cx.background_executor().timer(COPIED_FOR).await;
            let _ = view.update(cx, |view, cx| {
                if view.copied == Some(n) {
                    view.copied = None;
                    cx.notify();
                }
            });
        }));
        cx.notify();
    }

    fn conversation(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        if self.lines.is_empty() && matches!(self.link, LinkState::Connected) {
            return empty(theme).into_any_element();
        }
        let marks = marks(&self.lines, &self.zone, today(&self.zone));
        let mut rows = Vec::with_capacity(self.lines.len());
        let mut spaced = false;
        for (n, (event, mark)) in self.lines.iter().zip(&marks).enumerate() {
            if let Some(day) = &mark.day {
                rows.push(day_line(theme, day).into_any_element());
                spaced = false;
            }
            let Some(row) = self.line(theme, n, event, mark, cx) else {
                continue;
            };
            rows.push(
                div()
                    .when(spaced, |row| row.pt_3())
                    .when(!spaced && n > 0, |row| row.pt_1())
                    .child(row)
                    .into_any_element(),
            );
            spaced = mark.time.is_some();
        }
        div().flex().flex_col().children(rows).into_any_element()
    }

    fn meta(
        &self,
        theme: &Theme,
        n: usize,
        text: &str,
        mark: &Mark,
        copy_first: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let copied = self.copied == Some(n);
        let text = text.to_owned();
        let copy = (copied || self.hovered == Some(n)).then(|| {
            div()
                .id(("copy", n))
                .px_1p5()
                .rounded_md()
                .text_size(px(11.))
                .line_height(px(16.))
                .cursor_pointer()
                .when(copied, |copy| copy.text_color(theme.accent))
                .when(!copied, |copy| {
                    copy.text_color(theme.muted)
                        .hover(|copy| copy.bg(theme.line).text_color(theme.text))
                })
                .child(if copied { "복사했어요" } else { "복사" })
                .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                    view.copy(n, text.clone(), cx);
                }))
        });
        let time = mark.time.clone().map(|time| {
            div()
                .text_size(px(11.))
                .line_height(px(16.))
                .text_color(theme.faint)
                .child(time)
        });
        let (before, after) = if copy_first {
            (copy, None)
        } else {
            (None, copy)
        };
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_0p5()
            .children(before)
            .children(time)
            .children(after)
    }

    fn line(
        &self,
        theme: &Theme,
        n: usize,
        event: &ToWindow,
        mark: &Mark,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let row = match event {
            ToWindow::Answering { .. } | ToWindow::Alone | ToWindow::BrowserAt { .. } => {
                return None;
            }
            ToWindow::Owner { text, .. } => div()
                .flex()
                .justify_end()
                .items_end()
                .gap_1()
                .child(self.meta(theme, n, text, mark, true, cx))
                .child(
                    div()
                        .max_w(relative(0.8))
                        .px_3p5()
                        .py_2()
                        .rounded_2xl()
                        .rounded_br_md()
                        .bg(theme.accent)
                        .text_color(theme.accent_text)
                        .child(prose(text)),
                ),
            ToWindow::Reply { text, .. } => div()
                .flex()
                .items_start()
                .gap_2()
                .child(logo(theme, 26.))
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .min_w_0()
                        .items_end()
                        .gap_1()
                        .child(
                            div()
                                .max_w(relative(0.8))
                                .px_3p5()
                                .py_2()
                                .rounded_2xl()
                                .rounded_tl_md()
                                .bg(theme.surface)
                                .border_1()
                                .border_color(theme.line)
                                .shadow_xs()
                                .child(prose(text)),
                        )
                        .child(self.meta(theme, n, text, mark, false, cx)),
                ),
            ToWindow::Traffic {
                channel,
                place,
                who,
                text,
                ..
            } => div().flex().child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .w_full()
                    .px_3()
                    .py_2()
                    .rounded_xl()
                    .bg(theme.other)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_xs()
                            .text_color(theme.other_text)
                            .child(
                                div()
                                    .px_1p5()
                                    .rounded_sm()
                                    .border_1()
                                    .border_color(theme.other_text)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(SharedString::from(channel_name(channel))),
                            )
                            .child(SharedString::from(room_name(place)))
                            .child(div().size(px(3.)).rounded_full().bg(theme.other_text))
                            .child(
                                div()
                                    .flex_1()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(SharedString::from(who.clone())),
                            )
                            .child(self.meta(theme, n, text, mark, true, cx)),
                    )
                    .child(prose(text)),
            ),
        };
        Some(
            row.id(("line", n))
                .on_hover(cx.listener(move |view, hovered: &bool, _, cx| {
                    if *hovered {
                        view.hovered = Some(n);
                    } else if view.hovered == Some(n) {
                        view.hovered = None;
                    }
                    cx.notify();
                }))
                .into_any_element(),
        )
    }

    fn tabs(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let asking = self.screen.read(cx).asking();
        let tab = |id: &'static str, label: &'static str, which: Tab, cx: &mut Context<Self>| {
            let chosen = self.tab == which;
            div()
                .id(id)
                .relative()
                .px_3()
                .py_1()
                .rounded_lg()
                .cursor_pointer()
                .text_xs()
                .when(chosen, |tab| {
                    tab.bg(theme.surface)
                        .shadow_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.text)
                })
                .when(!chosen, |tab| {
                    tab.text_color(theme.muted)
                        .hover(|tab| tab.text_color(theme.text))
                })
                .child(label)
                .when(which == Tab::Browser && asking, |tab| {
                    tab.child(
                        div()
                            .absolute()
                            .top(px(2.))
                            .right(px(2.))
                            .size(px(7.))
                            .rounded_full()
                            .bg(theme.warn),
                    )
                })
                .on_click(cx.listener(move |view, _, window, cx| {
                    view.tab = which;
                    if which == Tab::Browser {
                        window.focus(&view.screen.focus_handle(cx));
                    } else {
                        window.focus(&view.input.focus_handle(cx));
                    }
                    cx.notify();
                }))
        };
        div()
            .flex()
            .flex_none()
            .gap_1()
            .p_1()
            .rounded_xl()
            .bg(theme.canvas)
            .border_1()
            .border_color(theme.line)
            .child(tab("tab-chat", "대화", Tab::Chat, cx))
            .child(tab("tab-browser", "브라우저", Tab::Browser, cx))
    }

    fn status_line(&self, theme: &Theme) -> impl IntoElement {
        let (color, words) = match &self.link {
            LinkState::Connecting => (theme.warn, "연결하는 중"),
            LinkState::Connected => (theme.ok, "연결됨"),
            LinkState::Down(reason) if reason.contains("not running") => {
                (theme.danger, "dotori가 꺼져 있어요")
            }
            LinkState::Down(_) => (theme.danger, "연결이 끊겼어요"),
        };
        let answering = self
            .answering
            .clone()
            .filter(|_| matches!(self.link, LinkState::Connected))
            .map(|(pc, here)| {
                let ink = if here { theme.ok } else { theme.other_text };
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .py_0p5()
                    .rounded_full()
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .bg(if here { theme.ok_soft } else { theme.other })
                    .text_color(ink)
                    .child(div().flex_none().size(px(5.)).rounded_full().bg(ink))
                    .child(if here {
                        format!("이 PC({pc})에서 답해요")
                    } else {
                        format!("{pc}에서 답해요")
                    })
            });
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(status(color, theme.muted, words))
            .children(answering)
    }

    fn composer(&self, theme: &Theme, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.input.focus_handle(cx).is_focused(window);
        let ready = matches!(self.link, LinkState::Connected)
            && !self.input.read(cx).text().trim().is_empty();
        let focus = self.input.focus_handle(cx);
        div()
            .flex_none()
            .px_4()
            .pt_2()
            .pb_4()
            .child(
                div()
                    .id("composer")
                    .flex()
                    .items_end()
                    .gap_2()
                    .pl_4()
                    .pr_1p5()
                    .py_1p5()
                    .rounded_2xl()
                    .bg(theme.surface)
                    .border_1()
                    .border_color(if focused { theme.accent } else { theme.line })
                    .shadow_sm()
                    .cursor_text()
                    .on_mouse_down(MouseButton::Left, move |_, window, _| {
                        window.focus(&focus);
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .py(px(6.))
                            .text_size(px(15.))
                            .line_height(px(22.))
                            .child(self.input.clone()),
                    )
                    .child(
                        div()
                            .id("send")
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .size(px(34.))
                            .rounded_full()
                            .bg(if ready { theme.accent } else { theme.line })
                            .text_color(if ready {
                                theme.accent_text
                            } else {
                                theme.faint
                            })
                            .font_weight(FontWeight::BOLD)
                            .when(ready, |send| {
                                send.cursor_pointer()
                                    .hover(|send| send.bg(theme.accent_hover))
                            })
                            .child("↑")
                            .on_click(cx.listener(|view, _: &ClickEvent, _, cx| view.send(cx))),
                    ),
            )
            .child(
                div()
                    .pt_1p5()
                    .px_1()
                    .text_xs()
                    .text_color(theme.faint)
                    .child(match &self.link {
                        LinkState::Down(reason) => {
                            SharedString::from(format!("다시 연결하는 중이에요. {reason}"))
                        }
                        _ => SharedString::new_static("Enter로 보내고, Shift+Enter로 줄을 바꿔요"),
                    }),
            )
    }
}

impl Focusable for ChatView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl Render for ChatView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(window);
        let conversation = self.conversation(&theme, cx);
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(theme.canvas)
            .text_color(theme.text)
            .text_sm()
            .line_height(px(21.))
            .on_action(cx.listener(Self::submit))
            .child(header(&theme, "dotori", self.status_line(&theme)).child(self.tabs(&theme, cx)))
            .when(self.tab == Tab::Chat, |chat| {
                chat.child(
                    div()
                        .id("conversation")
                        .flex_1()
                        .overflow_y_scroll()
                        .track_scroll(&self.scroll)
                        .px_4()
                        .py_5()
                        .child(conversation),
                )
                .child(self.composer(&theme, window, cx))
            })
            .when(self.tab == Tab::Browser, |chat| {
                match self.browser_at.clone().filter(|(_, here)| !here) {
                    Some((pc, _)) => chat.child(browser_elsewhere(&theme, &pc)),
                    None => chat.child(div().flex_1().min_h_0().child(self.screen.clone())),
                }
            })
    }
}

fn browser_elsewhere(theme: &Theme, pc: &str) -> impl IntoElement {
    div()
        .flex_1()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_3()
        .px_8()
        .bg(theme.canvas)
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(56.))
                .rounded_2xl()
                .bg(theme.other)
                .text_color(theme.other_text)
                .text_xl()
                .font_weight(FontWeight::SEMIBOLD)
                .child(pc.chars().next().unwrap_or('P').to_uppercase().to_string()),
        )
        .child(
            div()
                .text_lg()
                .font_weight(FontWeight::SEMIBOLD)
                .child(format!("브라우저는 {pc}에 있어요")),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .text_color(theme.muted)
                .text_center()
                .child("dotori는 로그인을 한곳에 모으려고 브라우저를 한 PC에서만 열어요.")
                .child(format!("보거나 넘겨받으려면 {pc}의 채팅창을 열어 주세요.")),
        )
}

fn empty(theme: &Theme) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap_3()
        .pt(px(96.))
        .child(logo(theme, 56.))
        .child(
            div()
                .text_lg()
                .font_weight(FontWeight::SEMIBOLD)
                .child("무엇을 도와드릴까요?"),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .text_color(theme.muted)
                .child("여기서 말해도, Slack에서 말해도")
                .child("같은 대화로 이어져요."),
        )
}

fn day_line(theme: &Theme, day: &str) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_3()
        .py_3()
        .child(div().flex_1().h(px(1.)).bg(theme.line))
        .child(
            div()
                .px_2p5()
                .py_0p5()
                .rounded_full()
                .bg(theme.raised)
                .border_1()
                .border_color(theme.line)
                .text_size(px(11.))
                .text_color(theme.muted)
                .font_weight(FontWeight::MEDIUM)
                .child(SharedString::from(day.to_owned())),
        )
        .child(div().flex_1().h(px(1.)).bg(theme.line))
}

fn channel_name(channel: &str) -> String {
    match channel {
        "slack" => String::from("Slack"),
        "telegram" => String::from("Telegram"),
        "discord" => String::from("Discord"),
        "routine" => String::from("루틴"),
        other => other.to_owned(),
    }
}

fn room_name(place: &str) -> String {
    let (room, thread) = match place.split_once('/') {
        Some((room, _)) => (room, true),
        None => (place, false),
    };
    let named = match room {
        "@owner" => String::from("주인에게만"),
        direct if direct.starts_with('D') => String::from("DM"),
        other => other.to_owned(),
    };
    if thread {
        format!("{named} 스레드")
    } else {
        named
    }
}
