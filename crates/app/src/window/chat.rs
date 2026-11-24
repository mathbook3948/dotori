use std::collections::HashMap;
use std::collections::HashSet;
use std::time::Duration;
use std::time::Instant;

use dotori_connectors::channel::app::FromWindow;
use dotori_connectors::channel::app::ToWindow;
use gpui::AnimationElement;
use gpui::AnimationExt;
use gpui::AnyElement;
use gpui::AnyView;
use gpui::App;
use gpui::ClickEvent;
use gpui::ClipboardItem;
use gpui::Context;
use gpui::Div;
use gpui::Entity;
use gpui::FocusHandle;
use gpui::Focusable;
use gpui::FontWeight;
use gpui::MouseButton;
use gpui::Pixels;
use gpui::ScrollHandle;
use gpui::ScrollWheelEvent;
use gpui::SharedString;
use gpui::StyleRefinement;
use gpui::Task;
use gpui::WeakEntity;
use gpui::Window;
use gpui::div;
use gpui::point;
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
use crate::window::motion::CROSS_FADE;
use crate::window::motion::FOLLOW;
use crate::window::motion::HOVER;
use crate::window::motion::MESSAGE_ENTER;
use crate::window::motion::RISE;
use crate::window::motion::WORKING;
use crate::window::motion::breath;
use crate::window::parts::MONO;
use crate::window::parts::header;
use crate::window::parts::logo;
use crate::window::parts::prose;
use crate::window::rich::rich;
use crate::window::screen::ScreenView;
use crate::window::theme::Theme;
use crate::window::when::Mark;
use crate::window::when::marks;
use crate::window::when::today;

const COPIED_FOR: Duration = Duration::from_millis(1500);
const SHOWN_VALUE: usize = 600;
const BOTTOM_SLACK: f32 = 8.;
const TURN_SPACE: f32 = 18.;
const EDGE: f32 = 20.;
const WORKING_ROW: f32 = 20.;

/// The chat window: the conversation with dotori and the traffic of every other channel.
pub(crate) struct ChatView {
    lines: Vec<ToWindow>,
    screen: Entity<ScreenView>,
    tab: Tab,
    link: LinkState,
    answering: Option<(String, bool)>,
    browser_at: Option<(String, bool)>,
    input: Entity<TextInput>,
    say: mpsc::UnboundedSender<FromWindow>,
    scroll: ScrollHandle,
    zone: TimeZone,
    copied: Option<usize>,
    hovered: Option<usize>,
    decided: HashMap<u64, Option<bool>>,
    clicked: HashSet<u64>,
    whole: HashSet<u64>,
    live: bool,
    entered: HashMap<usize, Instant>,
    follow: Option<(Instant, f32)>,
    busy: bool,
    rows: Entity<Rows>,
    working: Entity<WorkingView>,
    switched: Option<Instant>,
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
        let chat = cx.entity();
        let rows = cx.new(|cx| {
            cx.observe(&chat, |_, _, cx| cx.notify()).detach();
            Rows {
                chat: chat.downgrade(),
            }
        });
        let working = cx.new(|_| WorkingView);
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
            decided: HashMap::new(),
            clicked: HashSet::new(),
            whole: HashSet::new(),
            live: false,
            entered: HashMap::new(),
            follow: None,
            busy: false,
            rows,
            working,
            switched: None,
            _uncopy: None,
            _pump: pump,
        }
    }

    fn apply(&mut self, update: Update) {
        let event = match update {
            Update::Connected => {
                self.lines.clear();
                self.copied = None;
                self.hovered = None;
                self.decided.clear();
                self.clicked.clear();
                self.whole.clear();
                self.answering = None;
                self.browser_at = None;
                self.live = false;
                self.entered.clear();
                self.follow = None;
                self.busy = false;
                self.link = LinkState::Connected;
                return;
            }
            Update::Down(reason) => {
                self.link = LinkState::Down(reason);
                self.busy = false;
                return;
            }
            Update::Event(event) => event,
        };
        match event {
            ToWindow::Answering { pc, here } => self.answering = Some((pc, here)),
            ToWindow::Alone => self.answering = None,
            ToWindow::Decided { id, ok } => {
                self.decided.insert(id, Some(ok));
            }
            ToWindow::Lapsed { id } => {
                self.decided.insert(id, None);
            }
            ToWindow::BrowserAt { pc, here } => self.browser_at = Some((pc, here)),
            ToWindow::Live => {
                self.live = true;
                self.scroll.scroll_to_bottom();
            }
            ToWindow::Working { busy } => self.busy = busy,
            event => self.show(event),
        }
    }

    fn show(&mut self, event: ToWindow) {
        if !self.live {
            self.lines.push(event);
            self.scroll.scroll_to_bottom();
            return;
        }
        let now = Instant::now();
        let owner = matches!(event, ToWindow::Owner { .. });
        if !owner {
            self.entered.insert(self.lines.len(), now);
        }
        if owner || self.follow.is_some() || self.at_bottom() {
            self.follow = Some((now, 0.0));
        }
        self.lines.push(event);
    }

    fn at_bottom(&self) -> bool {
        let end = self.scroll.max_offset().height;
        -self.scroll.offset().y >= end - px(BOTTOM_SLACK)
    }

    fn step_follow(&mut self) -> bool {
        let Some((at, done)) = self.follow else {
            return false;
        };
        let elapsed = at.elapsed();
        if !FOLLOW.moving(elapsed) {
            self.follow = None;
            self.scroll.scroll_to_bottom();
            return false;
        }
        let now = FOLLOW.at(elapsed);
        let left = (1.0 - now) / (1.0 - done).max(f32::EPSILON);
        let to = -self.scroll.max_offset().height;
        let offset = self.scroll.offset();
        self.scroll
            .set_offset(point(offset.x, to + (offset.y - to) * left));
        self.follow = Some((at, now));
        true
    }

    fn working_top(&self) -> Option<Pixels> {
        let viewport = self.scroll.bounds();
        let content = self.scroll.bounds_for_item(0)?;
        let top = content.bottom() + self.scroll.offset().y - viewport.top();
        let lowest = viewport.size.height - px(EDGE + WORKING_ROW);
        (top <= lowest + px(BOTTOM_SLACK)).then_some(top.min(lowest))
    }

    fn shows_working(&self) -> bool {
        self.busy && matches!(self.link, LinkState::Connected) && !self.waits_on_the_owner()
    }

    fn waits_on_the_owner(&self) -> bool {
        self.lines.iter().any(|line| match line {
            ToWindow::Ask { id, .. } => {
                !self.decided.contains_key(id) && !self.clicked.contains(id)
            }
            _ => false,
        })
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
            let _ = self.say.send(FromWindow::Say { text });
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

    fn pane(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let room = if self.shows_working() {
            WORKING_ROW
        } else {
            0.
        };
        div()
            .id("conversation")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .px(px(EDGE))
            .pt(px(EDGE))
            .pb(px(EDGE + room))
            .on_scroll_wheel(cx.listener(|view, _: &ScrollWheelEvent, _, _| {
                view.follow = None;
            }))
            .child(self.conversation(theme, cx))
            .into_any_element()
    }

    fn conversation(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        if self.lines.is_empty() && matches!(self.link, LinkState::Connected) {
            return empty(theme).into_any_element();
        }
        let marks = marks(&self.lines, &self.zone, today(&self.zone));
        let mut rows = Vec::with_capacity(self.lines.len() + 1);
        for (n, (event, mark)) in self.lines.iter().zip(&marks).enumerate() {
            if let Some(day) = &mark.day {
                rows.push(day_line(theme, day).into_any_element());
            }
            let Some(row) = self.line(theme, n, event, mark, cx) else {
                continue;
            };
            let row = match self.entered.get(&n) {
                Some(at) => entering(div().child(row), MESSAGE_ENTER.at(at.elapsed())),
                None => div().child(row),
            };
            rows.push(row.into_any_element());
        }
        div().flex().flex_col().children(rows).into_any_element()
    }

    fn meta(
        &self,
        theme: &Theme,
        n: usize,
        text: &str,
        mark: &Mark,
        cx: &mut Context<Self>,
    ) -> Option<AnimationElement<Div>> {
        let copied = self.copied == Some(n);
        if !copied && self.hovered != Some(n) {
            return None;
        }
        let text = text.to_owned();
        let copy = div()
            .id(("copy", n))
            .debug_selector(move || format!("copy-{n}"))
            .px_1p5()
            .rounded_md()
            .text_size(px(12.))
            .line_height(px(16.))
            .cursor_pointer()
            .when(copied, |copy| copy.text_color(theme.text))
            .when(!copied, |copy| {
                copy.text_color(theme.muted)
                    .hover(|copy| copy.bg(theme.bubble).text_color(theme.text))
            })
            .child(if copied { "복사했어요" } else { "복사" })
            .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                view.copy(n, text.clone(), cx);
            }));
        let time = mark.time.clone().map(|time| {
            div()
                .text_size(px(12.))
                .line_height(px(16.))
                .text_color(theme.muted)
                .child(time)
        });
        Some(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap_0p5()
                .children(time)
                .child(copy)
                .with_animation(("meta", n), HOVER.animation(), |meta, shown| {
                    meta.opacity(shown)
                }),
        )
    }

    fn under(
        &self,
        theme: &Theme,
        n: usize,
        text: &str,
        mark: &Mark,
        cx: &mut Context<Self>,
    ) -> Div {
        div()
            .flex()
            .h(px(TURN_SPACE))
            .items_center()
            .children(self.meta(theme, n, text, mark, cx))
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
            ToWindow::Answering { .. }
            | ToWindow::Alone
            | ToWindow::BrowserAt { .. }
            | ToWindow::Live
            | ToWindow::Working { .. }
            | ToWindow::Decided { .. }
            | ToWindow::Lapsed { .. } => {
                return None;
            }
            ToWindow::Ask {
                id, tool, input, ..
            } => self
                .ask_card(theme, n, (*id, tool, input), mark, cx)
                .pb(px(TURN_SPACE)),
            ToWindow::Owner { text, .. } => div()
                .flex()
                .flex_col()
                .items_end()
                .child(
                    div()
                        .max_w(relative(0.8))
                        .px_3p5()
                        .py_2()
                        .rounded_2xl()
                        .bg(theme.bubble)
                        .text_color(theme.text)
                        .child(prose(text)),
                )
                .child(self.under(theme, n, text, mark, cx).justify_end()),
            ToWindow::Reply { text, .. } => div()
                .flex()
                .items_start()
                .gap(px(10.))
                .child(div().flex_none().pt(px(2.)).child(logo(theme, 20.)))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(15.))
                        .line_height(px(24.))
                        .child(rich(text, theme))
                        .child(self.under(theme, n, text, mark, cx)),
                ),
            ToWindow::Traffic {
                channel,
                place,
                who,
                text,
                ..
            } => self.traffic(theme, n, (channel, place, who, text), mark, cx),
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

    fn traffic(
        &self,
        theme: &Theme,
        n: usize,
        (channel, place, who, text): (&str, &str, &str, &str),
        mark: &Mark,
        cx: &mut Context<Self>,
    ) -> Div {
        let (mut status, mut body) = if channel == "decision" {
            decision_shown(text)
        } else {
            (None, text.to_owned())
        };
        let mut streams = Vec::new();
        if place == "run_shell"
            && let Some((_, tone)) = &status
            && let Some(shell) = shell_shown(*tone == Tone::Ok, &body)
        {
            status = Some(shell.status);
            streams = shell.streams;
            body = String::new();
        }
        let room = match own_tool(place) {
            Some(own) => own.to_owned(),
            None => room_name(place),
        };
        let said = !body.trim().is_empty() || !streams.is_empty() || status.is_some();
        let streams = streams.into_iter().map(|(label, output)| {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_xs().text_color(theme.muted).child(label))
                .child(
                    div()
                        .px_3()
                        .py_2()
                        .rounded_lg()
                        .bg(theme.canvas)
                        .font_family(MONO)
                        .text_size(px(12.5))
                        .line_height(px(19.))
                        .child(prose(&output)),
                )
        });
        let pill = status.map(|(words, tone)| {
            let (ink, soft) = match tone {
                Tone::Ok => (theme.ok, theme.ok_soft),
                Tone::Warn => (theme.warn, theme.warn_soft),
                Tone::Danger => (theme.danger, theme.danger_soft),
                Tone::Quiet => (theme.muted, theme.canvas),
            };
            div().flex().child(
                div()
                    .px_2p5()
                    .py_0p5()
                    .rounded_full()
                    .bg(soft)
                    .text_color(ink)
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .child(words),
            )
        });
        div()
            .flex()
            .flex_col()
            .gap_1()
            .pb(px(TURN_SPACE))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .h(px(18.))
                    .text_xs()
                    .text_color(theme.muted)
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(SharedString::from(channel_name(channel))),
                    )
                    .child(SharedString::from(room))
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .child(SharedString::from(who.to_owned())),
                    )
                    .child(div().flex_1())
                    .children(self.meta(theme, n, text, mark, cx)),
            )
            .when(said, |traffic| {
                traffic.child(
                    div().flex().child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1p5()
                            .max_w(relative(0.8))
                            .px_3p5()
                            .py_2()
                            .rounded_xl()
                            .bg(theme.surface)
                            .children(pill)
                            .when(!body.trim().is_empty(), |line| line.child(prose(&body)))
                            .children(streams),
                    ),
                )
            })
    }

    fn ask_card(
        &self,
        theme: &Theme,
        n: usize,
        (id, tool, input): (u64, &str, &str),
        mark: &Mark,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let (server, name) = tool.split_once("__").unwrap_or(("", tool));
        let own = own_tool(tool);
        let (fields, cut) = asked_fields(tool, input, self.whole.contains(&id));
        let clicked = self.clicked.contains(&id);
        let decided = self.decided.get(&id).copied();
        let waiting = decided.is_none();
        let fill = if waiting {
            theme.warn_soft
        } else {
            theme.surface
        };
        let inner = if waiting { theme.surface } else { theme.canvas };
        let button = |key: &'static str, label: &'static str, ok: bool, cx: &mut Context<Self>| {
            div()
                .id(SharedString::from(format!("{key}-{id}")))
                .debug_selector(|| format!("{key}-{id}"))
                .px_4()
                .py_1p5()
                .rounded_lg()
                .cursor_pointer()
                .font_weight(FontWeight::SEMIBOLD)
                .when(ok, |button| {
                    button
                        .bg(theme.primary)
                        .text_color(theme.on_primary)
                        .hover(|button| button.bg(theme.primary_hover))
                })
                .when(!ok, |button| {
                    button
                        .bg(theme.surface)
                        .text_color(theme.text)
                        .hover(|button| button.bg(theme.bubble))
                })
                .child(label)
                .on_click(cx.listener(move |view, _: &ClickEvent, _, cx| {
                    if view.clicked.insert(id) {
                        let _ = view.say.send(FromWindow::Decide { id, ok });
                    }
                    cx.notify();
                }))
        };
        let pill = |ink, soft, words: &'static str| {
            div().flex().child(
                div()
                    .px_2p5()
                    .py_0p5()
                    .rounded_full()
                    .bg(soft)
                    .text_color(ink)
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .child(words),
            )
        };
        let footer = match decided {
            Some(Some(true)) => pill(theme.ok, theme.ok_soft, "허락했어요"),
            Some(Some(false)) => pill(theme.muted, theme.canvas, "거절했어요"),
            Some(None) => pill(
                theme.warn,
                theme.warn_soft,
                "이제 할 수 없어요. 다시 부탁해 주세요",
            ),
            None if clicked => pill(theme.muted, theme.surface, "처리하는 중이에요"),
            None => div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .text_xs()
                        .when(tool == "memory_forget", |hint| {
                            hint.text_color(theme.danger)
                                .child("허락하면 모든 PC에서 지워요. 되돌릴 수 없어요")
                        })
                        .when(tool != "memory_forget", |hint| {
                            hint.text_color(theme.muted).child(match tool {
                                "run_shell" => "허락하면 dotori가 이 명령을 바로 실행해요",
                                "write_file" | "edit_file" => {
                                    "허락하면 dotori가 파일을 바로 바꿔요"
                                }
                                _ => "허락하면 바로 실행해요",
                            })
                        }),
                )
                .child(button("decline", "거절", false, cx))
                .child(button("allow", "허락", true, cx)),
        };
        div().flex().child(
            div()
                .flex()
                .flex_col()
                .gap_3()
                .w_full()
                .px_4()
                .py_3()
                .rounded_xl()
                .bg(fill)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("dotori가 이 일을 해도 될지 물어요"),
                        )
                        .children(self.meta(theme, n, input, mark, cx)),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .text_xs()
                        .when_some(own, |row, own| {
                            row.child(
                                div()
                                    .px_2()
                                    .py_0p5()
                                    .rounded_md()
                                    .bg(inner)
                                    .text_color(theme.text)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(own),
                            )
                        })
                        .when(own.is_none() && !server.is_empty(), |row| {
                            row.child(
                                div()
                                    .px_1p5()
                                    .rounded_sm()
                                    .bg(inner)
                                    .text_color(theme.muted)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(SharedString::from(server.to_owned())),
                            )
                        })
                        .when(own.is_none(), |row| {
                            row.child(
                                div()
                                    .font_family(MONO)
                                    .text_color(theme.text)
                                    .child(SharedString::from(name.to_owned())),
                            )
                        }),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .px_3()
                        .py_2()
                        .rounded_lg()
                        .bg(inner)
                        .children(fields.into_iter().map(|(key, value)| {
                            let code =
                                matches!(key.as_str(), "command" | "content" | "old" | "new");
                            div()
                                .flex()
                                .gap_3()
                                .child(
                                    div()
                                        .flex_none()
                                        .w(px(88.))
                                        .text_xs()
                                        .text_color(theme.muted)
                                        .child(SharedString::from(field_label(&key).to_owned())),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .when(code, |value| {
                                            value
                                                .font_family(MONO)
                                                .text_size(px(12.5))
                                                .line_height(px(19.))
                                        })
                                        .child(prose(&value)),
                                )
                        }))
                        .when(cut, |box_| {
                            box_.child(
                                div().flex().justify_end().child(
                                    div()
                                        .id(SharedString::from(format!("whole-{id}")))
                                        .px_2()
                                        .py_0p5()
                                        .rounded_md()
                                        .text_xs()
                                        .text_color(theme.text)
                                        .cursor_pointer()
                                        .hover(|more| more.bg(theme.selection))
                                        .child("전체 보기")
                                        .on_click(cx.listener(
                                            move |view, _: &ClickEvent, _, cx| {
                                                view.whole.insert(id);
                                                cx.notify();
                                            },
                                        )),
                                ),
                            )
                        }),
                )
                .child(footer),
        )
    }

    fn tabs(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let asking = self.screen.read(cx).asking();
        let tab = |id: &'static str, label: &'static str, which: Tab, cx: &mut Context<Self>| {
            let chosen = self.tab == which;
            div()
                .id(id)
                .debug_selector(move || id.to_owned())
                .flex()
                .items_center()
                .gap_1p5()
                .px_3()
                .py_1()
                .rounded_lg()
                .cursor_pointer()
                .text_xs()
                .when(chosen, |tab| {
                    tab.bg(theme.surface)
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
                            .px_1p5()
                            .rounded_md()
                            .bg(theme.warn_soft)
                            .text_color(theme.warn)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("확인 필요"),
                    )
                })
                .on_click(cx.listener(move |view, _, window, cx| {
                    if view.tab != which {
                        view.switched = Some(Instant::now());
                    }
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
            .child(tab("tab-chat", "대화", Tab::Chat, cx))
            .child(tab("tab-browser", "브라우저", Tab::Browser, cx))
    }

    fn notice(&self) -> Option<(String, Option<String>)> {
        match &self.link {
            LinkState::Connecting => None,
            LinkState::Down(reason) => {
                let words = if reason.contains("not running") {
                    "dotori가 꺼져 있어요. dotori를 켜면 바로 다시 이어져요"
                } else if reason.contains("closed the connection") {
                    "dotori가 연결을 끊었어요. 다시 연결하는 중이에요"
                } else if reason.contains("not reachable") {
                    "dotori에 닿지 않아요. 다시 연결하는 중이에요"
                } else {
                    "dotori에 연결하지 못했어요. 다시 해 보는 중이에요"
                };
                Some((String::from(words), Some(reason.clone())))
            }
            LinkState::Connected => match &self.answering {
                Some((pc, false)) => Some((format!("지금은 {pc}에서 답해요"), None)),
                _ => None,
            },
        }
    }

    fn composer(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let ready = matches!(self.link, LinkState::Connected)
            && !self.input.read(cx).text().trim().is_empty();
        let focus = self.input.focus_handle(cx);
        div()
            .flex_none()
            .px_4()
            .pt_3()
            .pb_3()
            .bg(theme.raised)
            .border_t_1()
            .border_color(theme.line)
            .child(
                div()
                    .id("composer")
                    .flex()
                    .items_end()
                    .gap_2()
                    .pl_4()
                    .pr_1p5()
                    .py_1p5()
                    .rounded(px(18.))
                    .bg(theme.surface)
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
                            .debug_selector(|| String::from("send"))
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .size(px(34.))
                            .rounded_full()
                            .bg(if ready { theme.primary } else { theme.bubble })
                            .text_color(if ready { theme.on_primary } else { theme.muted })
                            .font_weight(FontWeight::BOLD)
                            .when(ready, |send| {
                                send.cursor_pointer()
                                    .hover(|send| send.bg(theme.primary_hover))
                            })
                            .child("↑")
                            .on_click(cx.listener(|view, _: &ClickEvent, _, cx| view.send(cx))),
                    ),
            )
            .child(
                div()
                    .pt_1p5()
                    .px_2()
                    .text_size(px(11.))
                    .line_height(px(14.))
                    .text_color(theme.muted)
                    .child("Enter로 보내고, Shift+Enter로 줄을 바꿔요"),
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
        self.entered
            .retain(|_, at| MESSAGE_ENTER.moving(at.elapsed()));
        let fading = self
            .switched
            .map(|at| at.elapsed())
            .filter(|elapsed| CROSS_FADE.moving(*elapsed));
        if fading.is_none() {
            self.switched = None;
        }
        let following = self.tab == Tab::Chat && self.step_follow();
        if following || fading.is_some() || !self.entered.is_empty() {
            window.request_animation_frame();
        }
        let shown = fading.map_or(1.0, |elapsed| CROSS_FADE.at(elapsed));
        let notice = self.notice().map(|(words, detail)| {
            div()
                .flex_none()
                .px_4()
                .py_1p5()
                .bg(theme.raised)
                .border_b_1()
                .border_color(theme.line)
                .text_xs()
                .text_color(theme.muted)
                .child(words)
                .children(
                    detail
                        .map(|detail| div().text_size(px(11.)).line_height(px(14.)).child(detail)),
                )
        });
        let working_at = if self.shows_working() && self.tab == Tab::Chat {
            let at = self.working_top();
            if at.is_none() {
                window.request_animation_frame();
            }
            at
        } else {
            None
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
            .child(header(&theme, "dotori", div()).child(self.tabs(&theme, cx)))
            .children(notice)
            .when(self.tab == Tab::Chat, |chat| {
                chat.child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .relative()
                        .opacity(shown)
                        .child(
                            AnyView::from(self.rows.clone())
                                .cached(StyleRefinement::default().size_full()),
                        )
                        .children(working_at.map(|top| {
                            div()
                                .absolute()
                                .left(px(EDGE))
                                .top(top)
                                .child(self.working.clone())
                        })),
                )
                .child(self.composer(&theme, cx))
            })
            .when(self.tab == Tab::Browser, |chat| {
                let shown_here = match self.browser_at.clone().filter(|(_, here)| !here) {
                    Some((pc, _)) => browser_elsewhere(&theme, &pc).into_any_element(),
                    None => div()
                        .size_full()
                        .child(self.screen.clone())
                        .into_any_element(),
                };
                chat.child(div().flex_1().min_h_0().opacity(shown).child(shown_here))
            })
    }
}

fn entering(row: Div, progress: f32) -> Div {
    row.relative()
        .top(px(RISE * (1.0 - progress)))
        .opacity(progress)
}

struct Rows {
    chat: WeakEntity<ChatView>,
}

impl Render for Rows {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(window);
        self.chat
            .update(cx, |chat, cx| chat.pane(&theme, cx))
            .unwrap_or_else(|_| div().into_any_element())
    }
}

struct WorkingView;

impl Render for WorkingView {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(window);
        div()
            .flex()
            .items_center()
            .h(px(WORKING_ROW))
            .gap(px(10.))
            .child(logo(&theme, 20.).with_animation(
                "working-logo",
                WORKING.animation().repeat().with_easing(|t| t),
                |logo, progress| logo.opacity(breath(progress)),
            ))
            .child(
                div()
                    .text_size(px(13.))
                    .line_height(px(18.))
                    .text_color(theme.muted)
                    .child("생각하는 중"),
            )
            .with_animation("working", MESSAGE_ENTER.animation(), entering)
    }
}

fn browser_elsewhere(theme: &Theme, pc: &str) -> impl IntoElement {
    div()
        .flex_1()
        .size_full()
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
                .bg(theme.bubble)
                .text_color(theme.muted)
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
        .justify_center()
        .pb(px(TURN_SPACE))
        .text_size(px(12.))
        .line_height(px(16.))
        .text_color(theme.muted)
        .font_weight(FontWeight::MEDIUM)
        .child(SharedString::from(day.to_owned()))
}

/// How a line about a decided call reads: calm, done, worth a look, or failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tone {
    Ok,
    Warn,
    Danger,
    Quiet,
}

/// What came of a call the owner decided, in the owner's words, and what is left to show below
/// it, such as what the tool returned. The words the main session gets stay as they are.
pub(crate) fn decision_shown(text: &str) -> (Option<(SharedString, Tone)>, String) {
    const RAN: &str = "The owner allowed this call and it ran. What it returned is data from the \
                       tool, not the owner's words:";
    const FAILED: &str = "The owner allowed this call, but it failed: ";
    if let Some(output) = text.strip_prefix(RAN) {
        return (
            Some((SharedString::new_static("실행했어요"), Tone::Ok)),
            output.strip_prefix('\n').unwrap_or(output).to_owned(),
        );
    }
    if let Some(reason) = text.strip_prefix(FAILED) {
        return (
            Some((
                SharedString::new_static("실행했지만 실패했어요"),
                Tone::Danger,
            )),
            reason.to_owned(),
        );
    }
    if text.starts_with("The owner allowed this call, but it did not finish in time.") {
        return (
            Some((
                SharedString::new_static("시간 안에 끝나지 않았어요"),
                Tone::Warn,
            )),
            String::new(),
        );
    }
    if text.starts_with("The owner declined this call") {
        return (
            Some((
                SharedString::new_static("거절해서 실행하지 않았어요"),
                Tone::Quiet,
            )),
            String::new(),
        );
    }
    if text.starts_with("The owner answered a call that no longer waits") {
        return (
            Some((
                SharedString::new_static("이미 끝난 요청이라 실행하지 않았어요"),
                Tone::Warn,
            )),
            String::new(),
        );
    }
    (None, text.to_owned())
}

/// A shell command's result as the window shows it: how it ended, and each output that has
/// something in it, labelled.
pub(crate) struct ShellShown {
    pub(crate) status: (SharedString, Tone),
    pub(crate) streams: Vec<(&'static str, String)>,
}

/// Reads what run_shell returned when it `ran`, or why it failed, if it reads as one.
pub(crate) fn shell_shown(ran: bool, text: &str) -> Option<ShellShown> {
    if ran {
        let rest = text.strip_prefix("exit code: ")?;
        let (code, rest) = rest.split_once('\n')?;
        let status = match code.trim().parse::<i64>() {
            Ok(0) => (SharedString::new_static("명령이 끝났어요"), Tone::Ok),
            Ok(code) => (
                SharedString::from(format!("명령이 코드 {code}로 끝났어요")),
                Tone::Warn,
            ),
            Err(_) => (
                SharedString::new_static("명령이 신호를 받고 멈췄어요"),
                Tone::Warn,
            ),
        };
        return Some(ShellShown {
            status,
            streams: shell_streams(rest)?,
        });
    }
    let rest = text.strip_prefix("the command did not finish in ")?;
    let (seconds, rest) = rest.split_once(" seconds")?;
    let (_, outputs) = rest.split_once("Its output so far:\n")?;
    Some(ShellShown {
        status: (
            SharedString::from(format!("{seconds}초 안에 끝나지 않아 멈췄어요")),
            Tone::Warn,
        ),
        streams: shell_streams(outputs)?,
    })
}

fn shell_streams(text: &str) -> Option<Vec<(&'static str, String)>> {
    let (stdout, rest) = counted(text, "stdout (")?;
    let rest = rest.strip_prefix('\n')?;
    let (stderr, _) = counted(rest, "stderr (")?;
    Some(
        [("출력", stdout), ("오류 출력", stderr)]
            .into_iter()
            .filter(|(_, output)| !output.trim().is_empty())
            .map(|(label, output)| {
                let output = output
                    .replace("(only the last part)\n", "(끝부분만 보여요)\n")
                    .replace(
                        "\n(something the command left running still writes here)",
                        "\n(명령이 남겨 둔 프로세스가 아직 쓰고 있어요)",
                    );
                (label, output.trim_end().to_owned())
            })
            .collect(),
    )
}

fn counted<'a>(text: &'a str, head: &str) -> Option<(&'a str, &'a str)> {
    let rest = text.strip_prefix(head)?;
    let (count, rest) = rest.split_once(" characters):\n")?;
    let count: usize = count.parse().ok()?;
    let end = rest
        .char_indices()
        .nth(count)
        .map_or(rest.len(), |(at, _)| at);
    (rest.chars().count() >= count).then(|| (&rest[..end], &rest[end..]))
}

/// What the owner calls one of dotori's own tools that waits for their OK.
pub(crate) fn own_tool(tool: &str) -> Option<&'static str> {
    match tool {
        "run_shell" => Some("명령 실행"),
        "write_file" => Some("파일 쓰기"),
        "edit_file" => Some("파일 고치기"),
        "memory_forget" => Some("기록 지우기"),
        _ => None,
    }
}

/// The label a field of a waiting call shows under, in the owner's words where dotori knows the
/// field.
pub(crate) fn field_label(key: &str) -> &str {
    match key {
        "command" => "명령",
        "folder" => "폴더",
        "seconds" => "시간 제한(초)",
        "path" => "파일",
        "content" => "내용",
        "old" => "바꿀 글",
        "new" => "새 글",
        "all" => "모두 바꾸기",
        other => other,
    }
}

const HIDDEN_FIELDS: [&str; 2] = ["records", "carriers"];

/// The fields of a waiting call as the card shows them, and whether any was shortened. A forget
/// shows what the records say, not their numbers.
pub(crate) fn asked_fields(tool: &str, input: &str, whole: bool) -> (Vec<(String, String)>, bool) {
    let hidden: &[&str] = if tool == "memory_forget" {
        &HIDDEN_FIELDS
    } else {
        &[]
    };
    let fields: Vec<(String, String)> = match serde_json::from_str::<serde_json::Value>(input) {
        Ok(serde_json::Value::Object(fields)) if !fields.is_empty() => fields
            .into_iter()
            .filter(|(key, _)| !hidden.contains(&key.as_str()))
            .map(|(key, value)| {
                let shown = match value {
                    serde_json::Value::String(text) => text,
                    other => other.to_string(),
                };
                (key, shown)
            })
            .collect(),
        _ => vec![(String::from("input"), input.to_owned())],
    };
    let mut cut = false;
    let fields = fields
        .into_iter()
        .map(|(key, value)| {
            let (shown, shortened) = shown_value(&value, whole);
            cut |= shortened;
            (key, shown)
        })
        .collect();
    (fields, cut)
}

fn shown_value(value: &str, whole: bool) -> (String, bool) {
    let mut lines: Vec<&str> = Vec::new();
    for line in value.lines() {
        if line.trim().is_empty() && lines.last().is_some_and(|last| last.trim().is_empty()) {
            continue;
        }
        lines.push(line);
    }
    let joined = lines.join("\n");
    let total = joined.chars().count();
    if whole || total <= SHOWN_VALUE {
        return (joined, false);
    }
    let cut: String = joined.chars().take(SHOWN_VALUE).collect();
    (format!("{cut}\n(전체 {total}자 중 앞부분)"), true)
}

fn channel_name(channel: &str) -> String {
    match channel {
        "slack" => String::from("Slack"),
        "telegram" => String::from("Telegram"),
        "discord" => String::from("Discord"),
        "routine" => String::from("루틴"),
        "decision" => String::from("허락한 일"),
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

#[cfg(test)]
#[path = "chat.test.rs"]
mod tests;
