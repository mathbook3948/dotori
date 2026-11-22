use dotori_connectors::channel::app::ToWindow;
use gpui::App;
use gpui::Context;
use gpui::Entity;
use gpui::FocusHandle;
use gpui::Focusable;
use gpui::Hsla;
use gpui::ScrollHandle;
use gpui::SharedString;
use gpui::Task;
use gpui::Window;
use gpui::div;
use gpui::prelude::*;
use gpui::px;
use gpui::rgb;
use tokio::sync::mpsc;

use crate::link::Link;
use crate::link::Update;
use crate::window::input::Submit;
use crate::window::input::TextInput;

/// The chat window: the conversation with dotori and the traffic of every other channel.
pub(crate) struct ChatView {
    lines: Vec<ToWindow>,
    status: Option<SharedString>,
    down: bool,
    input: Entity<TextInput>,
    say: mpsc::UnboundedSender<String>,
    scroll: ScrollHandle,
    _pump: Task<()>,
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
            status: Some(SharedString::new_static("연결하는 중")),
            down: false,
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
                self.status = None;
                self.down = false;
            }
            Update::Down(reason) => {
                self.down = true;
                self.status = Some(SharedString::from(format!("{reason}. 다시 연결하는 중")))
            }
            Update::Event(event) => self.lines.push(event),
        }
        self.scroll.scroll_to_bottom();
    }

    fn submit(&mut self, _: &Submit, _: &mut Window, cx: &mut Context<Self>) {
        if self.down {
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
}

impl Focusable for ChatView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl Render for ChatView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let lines = self.lines.iter().map(line);
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0xfaf8f5))
            .text_color(rgb(0x2b2622))
            .on_action(cx.listener(Self::submit))
            .children(self.status.clone().map(|status| {
                div()
                    .px_3()
                    .py_1()
                    .bg(rgb(0xf3e3c3))
                    .text_sm()
                    .child(status)
            }))
            .child(
                div()
                    .id("conversation")
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_3()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .children(lines),
            )
            .child(
                div()
                    .p_2()
                    .border_t_1()
                    .border_color(rgb(0xe5ded4))
                    .child(self.input.clone()),
            )
    }
}

fn line(event: &ToWindow) -> impl IntoElement {
    let (who, text, color): (String, &str, Hsla) = match event {
        ToWindow::Owner { text } => (String::from("나"), text, rgb(0x6b5f55).into()),
        ToWindow::Reply { text } => (String::from("dotori"), text, rgb(0x8a5a1c).into()),
        ToWindow::Traffic {
            channel,
            place,
            who,
            text,
        } => (
            format!("{who} ({channel} {place})"),
            text,
            rgb(0x7d8a99).into(),
        ),
    };
    div()
        .flex()
        .flex_col()
        .child(div().text_xs().text_color(color).child(who))
        .child(
            div()
                .max_w(px(480.))
                .child(SharedString::from(text.to_owned())),
        )
}
