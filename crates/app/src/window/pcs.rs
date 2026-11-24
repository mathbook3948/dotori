//! The settings window's card for the owner's other PCs: an invite code to hand out, a code to
//! join with, and the PCs linked so far.

use std::thread;
use std::time::Duration;

use dotori_connectors::peer::Book;
use dotori_connectors::peer::Code;
use dotori_connectors::peer::Identity;
use dotori_connectors::peer::Invite;
use dotori_connectors::peer::hub::Seen;
use dotori_connectors::peer::hub::join;
use dotori_connectors::peer::now;
use gpui::App;
use gpui::ClipboardItem;
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
use tokio::sync::oneshot;

use crate::home::Home;
use crate::peers;
use crate::settings::Settings;
use crate::window::input::TextInput;
use crate::window::parts::MONO;
use crate::window::parts::prose;
use crate::window::theme::Theme;

const INVITE_OPEN: u64 = 10 * 60;
const REFRESH: Duration = Duration::from_secs(2);

pub(crate) struct PcsView {
    home: Home,
    settings: Settings,
    name: String,
    book: Book,
    seen: Seen,
    invite: Option<(String, u64)>,
    copied: bool,
    code: Entity<TextInput>,
    address: Entity<TextInput>,
    joining: bool,
    note: Option<Result<SharedString, SharedString>>,
    _refresh: Task<()>,
}

impl PcsView {
    pub(crate) fn new(home: Home, settings: Settings, cx: &mut Context<Self>) -> Self {
        let refresh = cx.spawn(async move |view, cx| {
            loop {
                cx.background_executor().timer(REFRESH).await;
                if view.update(cx, |view, cx| view.reload(cx)).is_err() {
                    return;
                }
            }
        });
        let mut view = Self {
            name: peers::name(&settings),
            home,
            settings,
            book: Book::default(),
            seen: Seen::default(),
            invite: None,
            copied: false,
            code: cx.new(|cx| TextInput::new("다른 PC에서 만든 연결 코드", cx)),
            address: cx.new(|cx| TextInput::new("그 PC의 주소:포트 (코드에 있으면 비워 둬요)", cx)),
            joining: false,
            note: None,
            _refresh: refresh,
        };
        view.reload(cx);
        view
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let book = Book::load(&self.home.peers()).unwrap_or_default();
        let seen = std::fs::read_to_string(self.home.peers_seen())
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        if let Some((_, until)) = &self.invite
            && (*until <= now() || !self.home.invite().exists())
        {
            self.invite = None;
        }
        if book != self.book || seen != self.seen {
            self.book = book;
            self.seen = seen;
        }
        cx.notify();
    }

    fn make_invite(&mut self, cx: &mut Context<Self>) {
        let made = Identity::load_or_make(&self.home.peer_key()).and_then(|identity| {
            let until = now() + INVITE_OPEN;
            let (_, secret) = Invite::make(&self.home.invite(), until)?;
            let code = Code {
                key: identity.public,
                secret,
                address: self
                    .settings
                    .peers
                    .as_ref()
                    .and_then(|peers| peers.address.clone()),
            };
            Ok((code.write(), until))
        });
        match made {
            Ok(invite) => {
                self.invite = Some(invite);
                self.copied = false;
                self.note = None;
            }
            Err(error) => self.note = Some(Err(SharedString::from(error))),
        }
        cx.notify();
    }

    fn copy(&mut self, cx: &mut Context<Self>) {
        if let Some((code, _)) = &self.invite {
            cx.write_to_clipboard(ClipboardItem::new_string(code.clone()));
            self.copied = true;
            cx.notify();
        }
    }

    fn join(&mut self, cx: &mut Context<Self>) {
        if self.joining {
            return;
        }
        let typed = self.code.read(cx).text().trim().to_owned();
        let Some(code) = Code::read(&typed) else {
            self.note = Some(Err(SharedString::new_static(
                "연결 코드를 읽지 못했어요. 다른 PC에서 복사한 코드를 그대로 붙여 넣어요.",
            )));
            cx.notify();
            return;
        };
        let typed_address = self.address.read(cx).text().trim().to_owned();
        let Some(address) = (!typed_address.is_empty())
            .then_some(typed_address)
            .or_else(|| code.address.clone())
        else {
            self.note = Some(Err(SharedString::new_static(
                "코드에 주소가 없어요. 그 PC의 주소와 포트를 적어요.",
            )));
            cx.notify();
            return;
        };
        let prepared = Identity::load_or_make(&self.home.peer_key())
            .and_then(|identity| Ok((identity, peers::hello(&self.home, &self.settings)?)));
        let (identity, hello) = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                self.note = Some(Err(SharedString::from(error)));
                cx.notify();
                return;
            }
        };
        let book = self.home.peers();
        let (done, result) = oneshot::channel();
        thread::spawn(move || {
            let joined = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| error.to_string())
                .and_then(|runtime| {
                    runtime.block_on(join(&identity, &hello, &code, &address, &book))
                });
            let _ = done.send(joined);
        });
        self.joining = true;
        self.note = None;
        cx.notify();
        cx.spawn(async move |view, cx| {
            let joined = result
                .await
                .unwrap_or_else(|_| Err(String::from("묶는 중에 멈췄어요")));
            let _ = view.update(cx, |view, cx| {
                view.joining = false;
                view.note = Some(match joined {
                    Ok(peer) => {
                        view.code.update(cx, |input, cx| input.set_text("", cx));
                        view.address.update(cx, |input, cx| input.set_text("", cx));
                        Ok(SharedString::from(format!(
                            "{}와 묶었어요. dotori가 돌고 있으면 곧 연결돼요.",
                            shown_name(&peer.name, &peer.device)
                        )))
                    }
                    Err(error) => Err(SharedString::from(error)),
                });
                view.reload(cx);
            });
        })
        .detach();
    }

    fn remove(&mut self, key: String, cx: &mut Context<Self>) {
        let mut book = match Book::load(&self.home.peers()) {
            Ok(book) => book,
            Err(error) => {
                self.note = Some(Err(SharedString::from(error)));
                cx.notify();
                return;
            }
        };
        book.remove(&key, now());
        if let Err(error) = book.save(&self.home.peers()) {
            self.note = Some(Err(SharedString::from(error)));
        }
        self.reload(cx);
    }
}

fn shown_name(name: &str, device: &str) -> String {
    if name.trim().is_empty() {
        format!("PC {}", device.get(..6).unwrap_or(device))
    } else {
        name.to_owned()
    }
}

fn ago(at: u64) -> String {
    let passed = now().saturating_sub(at);
    match passed {
        0..60 => String::from("방금"),
        60..3600 => format!("{}분 전", passed / 60),
        3600..86400 => format!("{}시간 전", passed / 3600),
        _ => format!("{}일 전", passed / 86400),
    }
}

impl Render for PcsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(window);
        let listening = self.settings.peers.is_some();
        let this_pc = div()
            .flex()
            .items_center()
            .gap_3()
            .px_3()
            .py_2p5()
            .rounded_xl()
            .bg(theme.canvas)
            .child(badge(&theme, &self.name))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .child(format!("{} (이 PC)", self.name)),
                    )
                    .child(div().text_xs().text_color(theme.muted).child(if listening {
                        match self
                            .settings
                            .peers
                            .as_ref()
                            .and_then(|peers| peers.address.clone())
                        {
                            Some(address) => format!("다른 PC가 {address}로 찾아와요"),
                            None => String::from("다른 PC가 찾아올 수 있어요"),
                        }
                    } else {
                        String::from("다른 PC에 연결만 해요. 찾아오게 하려면 [peers]를 켜요")
                    })),
            );
        let invite = match &self.invite {
            None => div().flex().child(button(
                &theme,
                "invite",
                "연결 코드 만들기",
                false,
                cx.listener(|view, _, _, cx| view.make_invite(cx)),
            )),
            Some((code, until)) => div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .px_3()
                                .py_2()
                                .rounded_lg()
                                .bg(theme.canvas)
                                .font_family(MONO)
                                .text_xs()
                                .child(prose(code)),
                        )
                        .child(button(
                            &theme,
                            "copy",
                            if self.copied {
                                "복사했어요"
                            } else {
                                "복사"
                            },
                            true,
                            cx.listener(|view, _, _, cx| view.copy(cx)),
                        )),
                )
                .child(div().text_xs().text_color(theme.muted).child(format!(
                    "다른 PC의 설정에 붙여 넣어요. {}분 동안 한 번만 쓸 수 있어요.",
                    until.saturating_sub(now()).div_ceil(60)
                ))),
        };
        let join_form = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(input_box(&theme, window, cx, "code", &self.code))
            .child(input_box(&theme, window, cx, "address", &self.address))
            .child(div().flex().justify_end().child(button(
                &theme,
                "join",
                if self.joining {
                    "묶는 중…"
                } else {
                    "이 코드로 묶기"
                },
                true,
                cx.listener(|view, _, _, cx| view.join(cx)),
            )));
        let mut list = div().flex().flex_col().gap_1();
        if self.book.peers.is_empty() {
            list = list.child(
                div()
                    .py_3()
                    .text_center()
                    .text_xs()
                    .text_color(theme.muted)
                    .child("아직 묶은 PC가 없어요"),
            );
        }
        for peer in &self.book.peers {
            let seen = self.seen.peers.get(&peer.key);
            let linked = seen.is_some_and(|seen| seen.linked);
            let state = match seen {
                Some(seen) if seen.linked => String::from("연결됨"),
                Some(seen) => format!("{} 끊김", ago(seen.at)),
                None => String::from("아직 연결되지 않음"),
            };
            let name = shown_name(&peer.name, &peer.device);
            let key = peer.key.clone();
            list = list.child(
                div()
                    .id(SharedString::from(format!("pc-{}", peer.key)))
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_3()
                    .py_2()
                    .rounded_xl()
                    .hover(|row| row.bg(theme.canvas))
                    .child(badge(&theme, &name))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .child(div().font_weight(FontWeight::MEDIUM).child(name))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1p5()
                                    .text_xs()
                                    .text_color(theme.muted)
                                    .child(
                                        div()
                                            .text_color(if linked { theme.ok } else { theme.muted })
                                            .child(state),
                                    )
                                    .children(peer.address.clone().map(|address| {
                                        div().text_color(theme.muted).child(address)
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("remove-{}", peer.key)))
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .text_xs()
                            .text_color(theme.muted)
                            .cursor_pointer()
                            .hover(|remove| remove.bg(theme.danger_soft).text_color(theme.danger))
                            .child("빼기")
                            .on_click(
                                cx.listener(move |view, _, _, cx| view.remove(key.clone(), cx)),
                            ),
                    ),
            );
        }
        let note = self.note.clone().map(|note| {
            let (color, soft, words) = match note {
                Ok(words) => (theme.ok, theme.ok_soft, words),
                Err(words) => (theme.danger, theme.danger_soft, words),
            };
            div()
                .px_3()
                .py_1p5()
                .rounded_lg()
                .bg(soft)
                .text_color(color)
                .text_xs()
                .child(prose(&words))
        });
        div()
            .flex()
            .flex_col()
            .gap_4()
            .p_5()
            .rounded_2xl()
            .bg(theme.surface)
            .border_t_1()
            .border_color(theme.rim)
            .shadow_sm()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_base()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("여러 PC"),
                    )
                    .child(div().text_xs().text_color(theme.muted).child(
                        "다른 PC의 dotori와 묶으면 어디서 말을 걸어도 같은 비서와 같은 기억으로 이어져요. 주고받는 내용은 모두 암호화해요.",
                    )),
            )
            .child(this_pc)
            .child(section(&theme, "이 PC로 초대하기", invite))
            .child(section(&theme, "다른 PC에 합류하기", join_form))
            .child(section(&theme, "묶은 PC", list))
            .children(note)
    }
}

fn section(theme: &Theme, title: &'static str, body: impl IntoElement) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .pt_2()
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.muted)
                .child(title),
        )
        .child(body)
}

fn badge(theme: &Theme, name: &str) -> impl IntoElement {
    let first = name
        .chars()
        .next()
        .unwrap_or('P')
        .to_uppercase()
        .to_string();
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(30.))
        .rounded_lg()
        .bg(theme.selection)
        .text_color(theme.text)
        .font_weight(FontWeight::SEMIBOLD)
        .child(first)
}

fn button(
    theme: &Theme,
    id: &'static str,
    label: &'static str,
    quiet: bool,
    click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .flex_none()
        .px_4()
        .py_1p5()
        .rounded_lg()
        .cursor_pointer()
        .font_weight(FontWeight::MEDIUM)
        .when(quiet, |button| {
            button
                .bg(theme.bubble)
                .text_color(theme.text)
                .hover(|button| button.bg(theme.selection))
        })
        .when(!quiet, |button| {
            button
                .bg(theme.primary)
                .text_color(theme.on_primary)
                .hover(|button| button.bg(theme.primary_hover))
        })
        .child(label)
        .on_click(click)
}

fn input_box(
    theme: &Theme,
    window: &Window,
    cx: &App,
    id: &'static str,
    input: &Entity<TextInput>,
) -> impl IntoElement {
    let focus = input.focus_handle(cx);
    let focused = focus.is_focused(window);
    div()
        .id(id)
        .px_3()
        .py_2()
        .rounded_lg()
        .bg(theme.canvas)
        .border_1()
        .border_color(if focused {
            theme.primary.into()
        } else {
            gpui::transparent_black()
        })
        .cursor_text()
        .on_mouse_down(MouseButton::Left, move |_, window, _| window.focus(&focus))
        .child(input.clone())
}
