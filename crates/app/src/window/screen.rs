//! The browser tab of the chat window: dotori's browser as it is now, where dotori clicks, and the
//! owner's own mouse and keys once they take it over. Typed text arrives as the OS input method
//! composed it, so every language is handled the same way.

use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use gpui::AnyElement;
use gpui::App;
use gpui::Bounds;
use gpui::ClickEvent;
use gpui::Context;
use gpui::ElementInputHandler;
use gpui::Entity;
use gpui::EntityInputHandler;
use gpui::FocusHandle;
use gpui::Focusable;
use gpui::FontWeight;
use gpui::ImageSource;
use gpui::KeyDownEvent;
use gpui::Modifiers;
use gpui::MouseButton;
use gpui::MouseDownEvent;
use gpui::MouseMoveEvent;
use gpui::MouseUpEvent;
use gpui::PathPromptOptions;
use gpui::Pixels;
use gpui::Point;
use gpui::RenderImage;
use gpui::ScrollDelta;
use gpui::ScrollWheelEvent;
use gpui::SharedString;
use gpui::Task;
use gpui::UTF16Selection;
use gpui::Window;
use gpui::canvas;
use gpui::div;
use gpui::img;
use gpui::point;
use gpui::prelude::*;
use gpui::px;
use serde_json::Value;
use serde_json::json;
use tokio::sync::mpsc;

use crate::screen_link::Picture;
use crate::screen_link::ScreenLink;
use crate::screen_link::Seen;
use crate::window::input::Submit;
use crate::window::input::TextInput;
use crate::window::parts::prose;
use crate::window::theme::Theme;

const POINTER_SHOWN: Duration = Duration::from_millis(900);

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Idle,
    Agent,
    Waiting,
    Owner,
}

pub(crate) struct ScreenView {
    picture: Option<Arc<RenderImage>>,
    painted: bool,
    page: (f64, f64),
    mode: Mode,
    url: Option<SharedString>,
    asked: Option<SharedString>,
    error: Option<SharedString>,
    popup: bool,
    picker: Option<Value>,
    prompted: bool,
    typed: Option<Entity<TextInput>>,
    login: Option<(Entity<TextInput>, Entity<TextInput>)>,
    focus_login: bool,
    down: bool,
    pointer: Option<(f64, f64, Instant)>,
    composing: bool,
    area: Rc<Cell<Bounds<Pixels>>>,
    send: mpsc::UnboundedSender<Value>,
    focus: FocusHandle,
    stale: Vec<Arc<RenderImage>>,
    _pump: Task<()>,
}

impl ScreenView {
    pub(crate) fn new(link: ScreenLink, cx: &mut Context<Self>) -> Self {
        let ScreenLink { mut seen, send } = link;
        let pump = cx.spawn(async move |view, cx| {
            while let Some(item) = seen.recv().await {
                if view.update(cx, |view, cx| view.apply(item, cx)).is_err() {
                    return;
                }
            }
        });
        Self {
            picture: None,
            painted: false,
            page: (1280.0, 800.0),
            mode: Mode::Idle,
            url: None,
            asked: None,
            error: None,
            popup: false,
            picker: None,
            prompted: false,
            typed: None,
            login: None,
            focus_login: false,
            down: true,
            pointer: None,
            composing: false,
            area: Rc::new(Cell::new(Bounds::default())),
            send,
            focus: cx.focus_handle(),
            stale: Vec::new(),
            _pump: pump,
        }
    }

    /// Whether dotori asked the owner to take the browser over, for a mark on the tab.
    pub(crate) fn asking(&self) -> bool {
        self.asked.is_some()
    }

    fn apply(&mut self, item: Seen, cx: &mut Context<Self>) {
        match item {
            Seen::Down => {
                self.down = true;
                self.mode = Mode::Idle;
            }
            Seen::State(state) => {
                self.down = false;
                let text = |name: &str| {
                    state
                        .get(name)
                        .and_then(Value::as_str)
                        .filter(|text| !text.is_empty())
                        .map(|text| SharedString::from(text.to_owned()))
                };
                self.mode = match state.get("mode").and_then(Value::as_str) {
                    Some("agent") => Mode::Agent,
                    Some("waiting") => Mode::Waiting,
                    Some("owner") => Mode::Owner,
                    _ => Mode::Idle,
                };
                self.url = text("url").or(self.url.take());
                self.asked = text("asked");
                self.error = text("error");
                self.popup = state.get("popup").and_then(Value::as_bool).unwrap_or(false);
                let picker = state
                    .get("picker")
                    .filter(|picker| picker.is_object())
                    .cloned();
                let files = picker
                    .as_ref()
                    .and_then(|picker| picker.get("kind"))
                    .and_then(Value::as_str)
                    == Some("files");
                if picker != self.picker || (files && self.mode == Mode::Owner && !self.prompted) {
                    self.prompted = false;
                    self.open_picker(picker.as_ref(), cx);
                }
                self.picker = picker;
            }
            Seen::Frame(Picture {
                width,
                height,
                bgra,
                page_width,
                page_height,
            }) => {
                self.down = false;
                let Some(buffer) = image::RgbaImage::from_raw(width, height, bgra) else {
                    return;
                };
                let frame = image::Frame::new(buffer);
                let old = self
                    .picture
                    .replace(Arc::new(RenderImage::new(vec![frame])));
                if let Some(old) = old
                    && self.painted
                {
                    self.stale.push(old);
                }
                self.painted = false;
                self.page = (page_width, page_height);
            }
            Seen::Pointer { x, y } => {
                self.pointer = Some((x, y, Instant::now()));
                cx.spawn(async move |view, cx| {
                    cx.background_executor().timer(POINTER_SHOWN).await;
                    let _ = view.update(cx, |_, cx| cx.notify());
                })
                .detach();
            }
        }
        cx.notify();
    }

    fn open_picker(&mut self, picker: Option<&Value>, cx: &mut Context<Self>) {
        self.typed = None;
        self.login = None;
        let Some(picker) = picker else {
            return;
        };
        match picker.get("kind").and_then(Value::as_str) {
            Some("auth") => {
                self.login = Some((
                    cx.new(|cx| TextInput::new("", cx)),
                    cx.new(|cx| TextInput::masked("", cx)),
                ));
                self.focus_login = true;
            }
            Some("value") => {
                let now = picker
                    .get("value")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let hint = match picker.get("type").and_then(Value::as_str) {
                    Some("date") => "2026-11-23",
                    Some("time") => "14:30",
                    Some("datetime-local") => "2026-11-23T14:30",
                    Some("month") => "2026-11",
                    Some("week") => "2026-W48",
                    _ => "#8b5a2b",
                };
                self.typed = Some(cx.new(|cx| {
                    let mut input = TextInput::new(hint, cx);
                    input.set_text(&now, cx);
                    input
                }));
            }
            Some("files") if self.mode == Mode::Owner => {
                self.prompted = true;
                let multiple = picker
                    .get("multiple")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let chosen = cx.prompt_for_paths(PathPromptOptions {
                    files: true,
                    directories: false,
                    multiple,
                    prompt: Some(SharedString::new_static("브라우저에 올릴 파일")),
                });
                cx.spawn(async move |view, cx| {
                    let files: Vec<String> = match chosen.await {
                        Ok(Ok(Some(paths))) => paths
                            .iter()
                            .map(|path| path.display().to_string())
                            .collect(),
                        _ => Vec::new(),
                    };
                    let _ = view.update(cx, |view, _| {
                        if files.is_empty() {
                            view.say(json!({ "kind": "pick", "cancel": true }));
                        } else {
                            view.say(json!({ "kind": "pick", "files": files }));
                        }
                    });
                })
                .detach();
            }
            _ => {}
        }
    }

    fn pick(&mut self, chosen: Value, cx: &mut Context<Self>) {
        self.say(json!({ "kind": "pick", "value": chosen }));
        self.picker = None;
        self.typed = None;
        cx.notify();
    }

    fn picker_card(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let picker = self.picker.as_ref()?;
        let kind = picker.get("kind").and_then(Value::as_str)?;
        if kind == "auth" {
            return Some(self.login_card(picker, theme, cx));
        }
        let title = match kind {
            "select" => "고르기",
            "value" => "값 넣기",
            _ => "파일 고르는 중",
        };
        let body = match kind {
            "select" => {
                let selected = picker.get("selected").and_then(Value::as_i64).unwrap_or(-1);
                let options = picker
                    .get("options")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                div()
                    .id("options")
                    .flex()
                    .flex_col()
                    .max_h(px(260.))
                    .overflow_y_scroll()
                    .children(options.into_iter().enumerate().map(|(index, option)| {
                        let label = option
                            .get("label")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned();
                        let disabled = option
                            .get("disabled")
                            .and_then(Value::as_bool)
                            .unwrap_or(false);
                        let chosen = index as i64 == selected;
                        div()
                            .id(("option", index))
                            .px_3()
                            .py_1p5()
                            .rounded_md()
                            .when(chosen, |row| {
                                row.bg(theme.canvas).font_weight(FontWeight::SEMIBOLD)
                            })
                            .when(disabled, |row| row.text_color(theme.faint))
                            .when(!disabled, |row| {
                                row.cursor_pointer()
                                    .hover(|row| row.bg(theme.canvas))
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        view.pick(json!(index), cx)
                                    }))
                            })
                            .child(SharedString::from(label))
                    }))
                    .into_any_element()
            }
            "value" => div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .id("typed")
                        .flex_1()
                        .px_3()
                        .py_1p5()
                        .rounded_lg()
                        .bg(theme.canvas)
                        .border_1()
                        .border_color(theme.accent)
                        .cursor_text()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|view, _, window, cx| {
                                if let Some(typed) = &view.typed {
                                    window.focus(&typed.focus_handle(cx));
                                }
                                cx.stop_propagation();
                            }),
                        )
                        .children(self.typed.clone()),
                )
                .child(
                    div()
                        .id("put")
                        .px_3()
                        .py_1p5()
                        .rounded_lg()
                        .cursor_pointer()
                        .bg(theme.accent)
                        .text_color(theme.accent_text)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("넣기")
                        .on_click(cx.listener(|view, _, _, cx| {
                            let text = view
                                .typed
                                .as_ref()
                                .map(|typed| typed.read(cx).text().to_owned())
                                .unwrap_or_default();
                            view.pick(json!(text), cx);
                        })),
                )
                .into_any_element(),
            _ => div()
                .text_color(theme.muted)
                .child(if self.mode == Mode::Owner {
                    "파일 대화상자에서 고르면 브라우저에 올라가요."
                } else {
                    "페이지가 파일을 달라고 해요. 넘겨받으면 파일을 고를 수 있어요."
                })
                .into_any_element(),
        };
        Some(
            div()
                .id("picker")
                .occlude()
                .absolute()
                .top(px(16.))
                .left(px(16.))
                .right(px(16.))
                .flex()
                .flex_col()
                .gap_2()
                .p_3()
                .rounded_xl()
                .bg(theme.surface)
                .border_1()
                .border_color(theme.line)
                .shadow_lg()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .child(
                            div()
                                .flex_1()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(title),
                        )
                        .child(
                            div()
                                .id("cancel-pick")
                                .px_2()
                                .py_0p5()
                                .rounded_md()
                                .cursor_pointer()
                                .text_xs()
                                .text_color(theme.muted)
                                .hover(|cancel| cancel.bg(theme.canvas))
                                .child("닫기")
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.say(json!({ "kind": "pick", "cancel": true }));
                                    view.picker = None;
                                    view.typed = None;
                                    cx.notify();
                                })),
                        ),
                )
                .child(body)
                .into_any_element(),
        )
    }

    fn login_card(&self, picker: &Value, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let origin = picker
            .get("origin")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let site = origin
            .split_once("://")
            .map_or(origin, |(_, host)| host)
            .to_owned();
        let realm = picker
            .get("realm")
            .and_then(Value::as_str)
            .filter(|realm| !realm.is_empty())
            .map(|realm| {
                div()
                    .px_1p5()
                    .rounded_sm()
                    .bg(theme.other)
                    .text_xs()
                    .text_color(theme.other_text)
                    .child(SharedString::from(realm.to_owned()))
            });
        let (name, secret) = match &self.login {
            Some((name, secret)) => (Some(name.clone()), Some(secret.clone())),
            None => (None, None),
        };
        let field = |id: &'static str, label: &'static str, input: Option<Entity<TextInput>>| {
            let target = input.clone();
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.muted)
                        .child(label),
                )
                .child(
                    div()
                        .id(id)
                        .px_3()
                        .py_2()
                        .rounded_lg()
                        .bg(theme.canvas)
                        .border_1()
                        .border_color(theme.line)
                        .cursor_text()
                        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                            if let Some(input) = &target {
                                window.focus(&input.focus_handle(cx));
                            }
                            cx.stop_propagation();
                        })
                        .children(input),
                )
        };
        let next = secret.clone();
        div()
            .id("login")
            .occlude()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui::rgba(0x0000_0033))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .w(px(340.))
                    .p_5()
                    .rounded_2xl()
                    .bg(theme.surface)
                    .border_1()
                    .border_color(theme.line)
                    .shadow_lg()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .flex()
                                    .flex_none()
                                    .items_center()
                                    .justify_center()
                                    .size(px(36.))
                                    .rounded_full()
                                    .bg(theme.selection)
                                    .child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .items_center()
                                            .child(
                                                div()
                                                    .w(px(12.))
                                                    .h(px(10.))
                                                    .border_2()
                                                    .border_b_0()
                                                    .border_color(theme.accent)
                                                    .rounded_t_full(),
                                            )
                                            .child(
                                                div()
                                                    .w(px(18.))
                                                    .h(px(13.))
                                                    .rounded_sm()
                                                    .bg(theme.accent),
                                            ),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .min_w_0()
                                    .gap_0p5()
                                    .child(
                                        div()
                                            .text_base()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child("로그인이 필요해요"),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_1p5()
                                            .min_w_0()
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(theme.muted)
                                                    .truncate()
                                                    .child(SharedString::from(site)),
                                            )
                                            .children(realm),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(field("login-name", "아이디", name).on_action(
                                move |_: &Submit, window, cx| {
                                    if let Some(next) = &next {
                                        window.focus(&next.focus_handle(cx));
                                    }
                                },
                            ))
                            .child(
                                field("login-secret", "비밀번호", secret).on_action(
                                    cx.listener(|view, _: &Submit, _, cx| view.log_in(cx)),
                                ),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.faint)
                            .child("이 페이지에만 넘기고 대화 기록에는 남기지 않아요."),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .child(
                                div()
                                    .id("login-cancel")
                                    .px_3()
                                    .py_1p5()
                                    .rounded_lg()
                                    .cursor_pointer()
                                    .text_color(theme.muted)
                                    .hover(|cancel| cancel.bg(theme.canvas))
                                    .child("취소")
                                    .on_click(cx.listener(|view, _, _, cx| {
                                        view.say(json!({ "kind": "auth", "cancel": true }));
                                        view.close_login(cx);
                                    })),
                            )
                            .child(
                                div()
                                    .id("login-go")
                                    .px_4()
                                    .py_1p5()
                                    .rounded_lg()
                                    .cursor_pointer()
                                    .bg(theme.accent)
                                    .hover(|go| go.bg(theme.accent_hover))
                                    .text_color(theme.accent_text)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("로그인")
                                    .on_click(cx.listener(|view, _, _, cx| view.log_in(cx))),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn log_in(&mut self, cx: &mut Context<Self>) {
        let Some((name, secret)) = &self.login else {
            return;
        };
        let username = name.read(cx).text().to_owned();
        let password = secret.read(cx).text().to_owned();
        self.say(json!({ "kind": "auth", "username": username, "password": password }));
        self.close_login(cx);
    }

    fn close_login(&mut self, cx: &mut Context<Self>) {
        self.picker = None;
        self.login = None;
        cx.notify();
    }

    fn say(&self, value: Value) {
        let _ = self.send.send(value);
    }

    fn take(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.say(json!({ "kind": "take" }));
        self.mode = Mode::Waiting;
        window.focus(&self.focus);
        cx.notify();
    }

    fn give(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.say(json!({ "kind": "give" }));
        cx.notify();
    }

    fn on_page(&self, at: Point<Pixels>) -> Option<(f64, f64)> {
        let shown = fitted(self.area.get(), self.page);
        let x =
            f64::from(f32::from(at.x - shown.origin.x)) / f64::from(f32::from(shown.size.width));
        let y =
            f64::from(f32::from(at.y - shown.origin.y)) / f64::from(f32::from(shown.size.height));
        ((0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y))
            .then_some((x * self.page.0, y * self.page.1))
    }

    fn mouse(
        &self,
        event: &str,
        at: Point<Pixels>,
        button: MouseButton,
        clicks: usize,
        modifiers: Modifiers,
    ) {
        if self.mode != Mode::Owner {
            return;
        }
        let Some((x, y)) = self.on_page(at) else {
            return;
        };
        let button = match button {
            MouseButton::Right => "right",
            MouseButton::Middle => "middle",
            _ => "left",
        };
        self.say(json!({
            "kind": "mouse", "event": event, "x": x, "y": y, "button": button,
            "clicks": clicks, "modifiers": bits(modifiers)
        }));
    }

    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode != Mode::Owner || self.composing || !self.focus.is_focused(window) {
            return;
        }
        let stroke = &event.keystroke;
        let held = stroke.modifiers.control || stroke.modifiers.alt || stroke.modifiers.platform;
        if held && stroke.key == "v" {
            if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                self.say(json!({ "kind": "text", "text": text }));
            }
            cx.stop_propagation();
            return;
        }
        let Some((key, code, typed)) = dom_key(&stroke.key) else {
            return;
        };
        if typed.is_some() && !held {
            return;
        }
        let modifiers = bits(stroke.modifiers);
        let text = if held { None } else { typed };
        self.say(json!({ "kind": "key", "event": "down", "key": key, "code": code, "text": text.unwrap_or_default(), "modifiers": modifiers }));
        self.say(json!({ "kind": "key", "event": "up", "key": key, "code": code, "modifiers": modifiers }));
        cx.stop_propagation();
    }

    fn wheel(&mut self, event: &ScrollWheelEvent, _: &mut Window, _: &mut Context<Self>) {
        if self.mode != Mode::Owner {
            return;
        }
        let Some((x, y)) = self.on_page(event.position) else {
            return;
        };
        let (dx, dy) = match event.delta {
            ScrollDelta::Pixels(delta) => (f32::from(delta.x), f32::from(delta.y)),
            ScrollDelta::Lines(delta) => (delta.x * 40.0, delta.y * 40.0),
        };
        self.say(json!({ "kind": "wheel", "x": x, "y": y, "dx": -dx, "dy": -dy }));
    }

    fn bar(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let (color, words) = match (self.down, self.mode) {
            (true, _) => (theme.faint, "연결하는 중"),
            (false, Mode::Agent) => (theme.accent, "dotori가 쓰는 중"),
            (false, Mode::Waiting) => (theme.warn, "넘겨받는 중"),
            (false, Mode::Owner) => (theme.ok, "내가 조작 중"),
            (false, Mode::Idle) => (theme.muted, "쉬는 중"),
        };
        let button = |id: &'static str, label: &'static str, primary: bool| {
            div()
                .id(id)
                .flex_none()
                .px_3()
                .py_1()
                .rounded_lg()
                .cursor_pointer()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .when(primary, |button| {
                    button
                        .bg(theme.accent)
                        .text_color(theme.accent_text)
                        .hover(|button| button.bg(theme.accent_hover))
                })
                .when(!primary, |button| {
                    button
                        .border_1()
                        .border_color(theme.line)
                        .bg(theme.surface)
                        .hover(|button| button.border_color(theme.accent))
                })
                .child(label)
        };
        let action = match self.mode {
            Mode::Owner => button("give", "돌려주기", false)
                .on_click(cx.listener(Self::give))
                .into_any_element(),
            Mode::Waiting => div().into_any_element(),
            _ if self.down => div().into_any_element(),
            _ => button("take", "넘겨받기", true)
                .on_click(cx.listener(Self::take))
                .into_any_element(),
        };
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(theme.line)
            .bg(theme.raised)
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_1p5()
                    .px_2()
                    .py_0p5()
                    .rounded_full()
                    .bg(theme.canvas)
                    .text_xs()
                    .text_color(color)
                    .child(div().size(px(7.)).rounded_full().bg(color))
                    .child(words),
            )
            .when(self.popup, |bar| {
                bar.child(
                    div()
                        .flex_none()
                        .px_2()
                        .py_0p5()
                        .rounded_full()
                        .bg(theme.warn)
                        .text_color(theme.accent_text)
                        .text_xs()
                        .child("새 창"),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .px_2()
                    .py_0p5()
                    .rounded_md()
                    .bg(theme.canvas)
                    .text_xs()
                    .text_color(theme.muted)
                    .truncate()
                    .child(
                        self.url
                            .clone()
                            .unwrap_or_else(|| SharedString::new_static("about:blank")),
                    ),
            )
            .child(action)
    }
}

fn fitted(area: Bounds<Pixels>, page: (f64, f64)) -> Bounds<Pixels> {
    let (wide, tall) = (f32::from(area.size.width), f32::from(area.size.height));
    if page.0 <= 0.0 || page.1 <= 0.0 || wide <= 0.0 || tall <= 0.0 {
        return area;
    }
    let scale = (wide / page.0 as f32).min(tall / page.1 as f32);
    let (w, h) = (page.0 as f32 * scale, page.1 as f32 * scale);
    Bounds::new(
        point(
            area.origin.x + px((wide - w) / 2.),
            area.origin.y + px((tall - h) / 2.),
        ),
        gpui::size(px(w), px(h)),
    )
}

fn bits(modifiers: Modifiers) -> i64 {
    i64::from(modifiers.alt)
        | i64::from(modifiers.control) << 1
        | i64::from(modifiers.platform) << 2
        | i64::from(modifiers.shift) << 3
}

/// The DOM key, its Windows key code, and the text it types, for a GPUI key name.
pub(crate) fn dom_key(key: &str) -> Option<(String, i64, Option<String>)> {
    let named = match key {
        "enter" => ("Enter", 13, Some("\r")),
        "tab" => ("Tab", 9, None),
        "escape" => ("Escape", 27, None),
        "backspace" => ("Backspace", 8, None),
        "delete" => ("Delete", 46, None),
        "left" => ("ArrowLeft", 37, None),
        "right" => ("ArrowRight", 39, None),
        "up" => ("ArrowUp", 38, None),
        "down" => ("ArrowDown", 40, None),
        "home" => ("Home", 36, None),
        "end" => ("End", 35, None),
        "pageup" => ("PageUp", 33, None),
        "pagedown" => ("PageDown", 34, None),
        "space" => (" ", 32, Some(" ")),
        _ => {
            let mut chars = key.chars();
            let (Some(c), None) = (chars.next(), chars.next()) else {
                return None;
            };
            let code = i64::from(u32::from(c.to_ascii_uppercase()));
            return Some((c.to_string(), code, Some(c.to_string())));
        }
    };
    Some((named.0.to_owned(), named.1, named.2.map(str::to_owned)))
}

impl EntityInputHandler for ScreenView {
    fn text_for_range(
        &mut self,
        _: Range<usize>,
        _: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        None
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: 0..0,
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.composing.then_some(0..0)
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.composing = false;
    }

    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        _: &mut Context<Self>,
    ) {
        if self.mode != Mode::Owner {
            return;
        }
        self.composing = false;
        if text == "\r" || text == "\n" {
            return;
        }
        self.say(json!({ "kind": "text", "text": text }));
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) {
        if self.mode != Mode::Owner {
            return;
        }
        self.composing = !text.is_empty();
        self.say(json!({ "kind": "compose", "text": text }));
    }

    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        Some(Bounds::new(bounds.origin, gpui::size(px(1.), px(20.))))
    }

    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

impl Focusable for ScreenView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ScreenView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        for old in self.stale.drain(..) {
            let _ = window.drop_image(old);
        }
        let theme = Theme::of(window);
        let area = Rc::clone(&self.area);
        let entity: Entity<Self> = cx.entity();
        let focus = self.focus.clone();
        let owner = self.mode == Mode::Owner;
        if self.focus_login
            && let Some((name, _)) = &self.login
        {
            self.focus_login = false;
            window.focus(&name.focus_handle(cx));
        }
        let picker = self.picker_card(&theme, cx);
        self.painted |= self.picture.is_some();
        let screen = match &self.picture {
            Some(picture) => img(ImageSource::Render(Arc::clone(picture)))
                .size_full()
                .into_any_element(),
            None => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme.muted)
                .child(prose(if self.down {
                    "dotori의 브라우저에 연결하는 중이에요."
                } else {
                    "dotori가 브라우저를 쓰면 여기에 보여요."
                }))
                .into_any_element(),
        };
        let marker = self
            .pointer
            .filter(|(_, _, at)| at.elapsed() < POINTER_SHOWN)
            .map(|(x, y, _)| {
                let shown = fitted(self.area.get(), self.page);
                let local = self.area.get().origin;
                let left = shown.origin.x - local.x
                    + px((x / self.page.0) as f32 * f32::from(shown.size.width))
                    - px(9.);
                let top = shown.origin.y - local.y
                    + px((y / self.page.1) as f32 * f32::from(shown.size.height))
                    - px(9.);
                div()
                    .absolute()
                    .left(left)
                    .top(top)
                    .size(px(18.))
                    .rounded_full()
                    .border_2()
                    .border_color(theme.accent)
                    .bg(gpui::Rgba {
                        a: 0.25,
                        ..theme.accent
                    })
            });
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(self.bar(&theme, cx))
            .children(self.asked.clone().filter(|_| !owner).map(|asked| {
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_4()
                    .py_2()
                    .bg(theme.ok_soft)
                    .text_sm()
                    .child(
                        div()
                            .flex_1()
                            .child(prose(&format!("dotori가 부탁해요: {asked}"))),
                    )
            }))
            .children(self.error.clone().map(|error| {
                div()
                    .px_4()
                    .py_1p5()
                    .bg(theme.danger_soft)
                    .text_color(theme.danger)
                    .text_xs()
                    .child(prose(&error))
            }))
            .child(
                div()
                    .id("screen")
                    .relative()
                    .flex_1()
                    .m_3()
                    .rounded_xl()
                    .overflow_hidden()
                    .bg(theme.canvas)
                    .border_1()
                    .border_color(if owner { theme.ok } else { theme.line })
                    .track_focus(&self.focus)
                    .when(owner, |screen| screen.cursor_pointer())
                    .on_key_down(cx.listener(Self::key))
                    .on_scroll_wheel(cx.listener(Self::wheel))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |view, event: &MouseDownEvent, window, _| {
                            window.focus(&focus);
                            view.mouse(
                                "down",
                                event.position,
                                event.button,
                                event.click_count,
                                event.modifiers,
                            );
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|view, event: &MouseDownEvent, _, _| {
                            view.mouse(
                                "down",
                                event.position,
                                event.button,
                                event.click_count,
                                event.modifiers,
                            );
                        }),
                    )
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|view, event: &MouseUpEvent, _, _| {
                            view.mouse(
                                "up",
                                event.position,
                                event.button,
                                event.click_count,
                                event.modifiers,
                            );
                        }),
                    )
                    .on_mouse_up(
                        MouseButton::Right,
                        cx.listener(|view, event: &MouseUpEvent, _, _| {
                            view.mouse(
                                "up",
                                event.position,
                                event.button,
                                event.click_count,
                                event.modifiers,
                            );
                        }),
                    )
                    .on_mouse_move(cx.listener(|view, event: &MouseMoveEvent, _, _| {
                        view.mouse(
                            "move",
                            event.position,
                            MouseButton::Left,
                            0,
                            event.modifiers,
                        );
                    }))
                    .child(
                        canvas(
                            move |bounds, _, _| {
                                area.set(bounds);
                            },
                            move |bounds, (), window, cx| {
                                let focus = entity.read(cx).focus.clone();
                                window.handle_input(
                                    &focus,
                                    ElementInputHandler::new(bounds, entity),
                                    cx,
                                );
                            },
                        )
                        .absolute()
                        .size_full(),
                    )
                    .child(screen)
                    .children(marker)
                    .children(picker),
            )
    }
}

#[cfg(test)]
#[path = "screen.test.rs"]
mod tests;
