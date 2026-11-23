use dotori_connectors::channel::app::ToWindow;
use gpui::App;
use gpui::ClickEvent;
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
use tokio::sync::mpsc;

use crate::link::Link;
use crate::link::Update;
use crate::window::input::Submit;
use crate::window::input::TextInput;
use crate::window::parts::header;
use crate::window::parts::logo;
use crate::window::parts::prose;
use crate::window::parts::status;
use crate::window::theme::Theme;

/// The chat window: the conversation with dotori and the traffic of every other channel.
pub(crate) struct ChatView {
    lines: Vec<ToWindow>,
    link: LinkState,
    input: Entity<TextInput>,
    say: mpsc::UnboundedSender<String>,
    scroll: ScrollHandle,
    _pump: Task<()>,
}

enum LinkState {
    Connecting,
    Connected,
    Down(String),
}

impl ChatView {
    pub(crate) fn new(link: Link, cx: &mut Context<Self>) -> Self {
        let Link { mut updates, say } = link;
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
            link: LinkState::Connecting,
            input: cx.new(|cx| TextInput::new("dotori에게 말하기", cx)),
            say,
            scroll: ScrollHandle::new(),
            _pump: pump,
        }
    }

    fn apply(&mut self, update: Update) {
        match update {
            Update::Connected => {
                self.lines.clear();
                self.link = LinkState::Connected;
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

    fn status_line(&self, theme: &Theme) -> impl IntoElement {
        let (color, words) = match &self.link {
            LinkState::Connecting => (theme.warn, "연결하는 중"),
            LinkState::Connected => (theme.ok, "연결됨"),
            LinkState::Down(reason) if reason.contains("not running") => {
                (theme.danger, "dotori가 꺼져 있어요")
            }
            LinkState::Down(_) => (theme.danger, "연결이 끊겼어요"),
        };
        status(color, theme.muted, words)
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
                    .items_center()
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
                        _ => SharedString::new_static("Enter로 보내요"),
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
        let conversation = if self.lines.is_empty() {
            empty(&theme).into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .gap_3()
                .children(self.lines.iter().map(|event| line(&theme, event)))
                .into_any_element()
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(theme.canvas)
            .text_color(theme.text)
            .text_sm()
            .line_height(px(21.))
            .on_action(cx.listener(Self::submit))
            .child(header(&theme, "dotori", self.status_line(&theme)))
            .child(
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
    }
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

fn line(theme: &Theme, event: &ToWindow) -> impl IntoElement {
    match event {
        ToWindow::Owner { text } => div().flex().justify_end().child(
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
        ToWindow::Reply { text } => div()
            .flex()
            .items_start()
            .gap_2()
            .child(logo(theme, 26.))
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
            ),
        ToWindow::Traffic {
            channel,
            place,
            who,
            text,
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
                                .font_weight(FontWeight::MEDIUM)
                                .child(SharedString::from(who.clone())),
                        ),
                )
                .child(prose(text)),
        ),
    }
}

fn channel_name(channel: &str) -> String {
    match channel {
        "slack" => String::from("Slack"),
        "telegram" => String::from("Telegram"),
        "discord" => String::from("Discord"),
        other => other.to_owned(),
    }
}

fn room_name(place: &str) -> String {
    match place.split_once('/') {
        Some((room, _)) => format!("{room} 스레드"),
        None if place.starts_with('@') => String::from("주인에게만"),
        None => place.to_owned(),
    }
}
