//! The settings window's card for the owner's other PCs: an invite code to hand out, a code to
//! join with, and the PCs linked so far.

use std::thread;
use std::time::Duration;

use dotori_connectors::peer::Book;
use dotori_connectors::peer::Code;
use dotori_connectors::peer::Identity;
use dotori_connectors::peer::Invite;
use dotori_connectors::peer::Peer;
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
use crate::settings::PcSettings;
use crate::settings::Settings;
use crate::window::input::TextInput;
use crate::window::parts::MONO;
use crate::window::parts::prose;
use crate::window::surface::SurfaceExt;
use crate::window::surface::framed;
use crate::window::surface::hairline;
use crate::window::theme::Theme;
use crate::words::words;

const INVITE_OPEN: u64 = 10 * 60;
const REFRESH: Duration = Duration::from_secs(2);

pub(crate) struct PcsView {
    home: Home,
    settings: Settings,
    pub(super) name: Entity<TextInput>,
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
        let name = cx.new(|cx| {
            let mut input = TextInput::new(peers::unnamed(&settings), cx);
            if let Some(name) = settings.pc.as_ref().and_then(|pc| pc.name.clone()) {
                input.set_text(&name, cx);
            }
            input
        });
        cx.observe(&name, |_, _, cx| cx.notify()).detach();
        let mut view = Self {
            name,
            home,
            settings,
            book: Book::default(),
            seen: Seen::default(),
            invite: None,
            copied: false,
            code: cx.new(|cx| TextInput::new(words().pcs.code_placeholder, cx)),
            address: cx.new(|cx| TextInput::new(words().pcs.address_placeholder, cx)),
            joining: false,
            note: None,
            _refresh: refresh,
        };
        view.reload(cx);
        view
    }

    /// The name typed for this PC, or nothing when the field is empty.
    pub(crate) fn typed_name(&self, cx: &App) -> String {
        self.name.read(cx).text().trim().to_owned()
    }

    fn settings_named(&self, cx: &App) -> Settings {
        let typed = self.typed_name(cx);
        Settings {
            pc: (!typed.is_empty()).then_some(PcSettings { name: Some(typed) }),
            ..self.settings.clone()
        }
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
        let typed = (
            self.code.read(cx).text().to_owned(),
            self.address.read(cx).text().to_owned(),
        );
        let (code, address) = match join_target(&typed.0, &typed.1) {
            Ok(target) => target,
            Err(error) => {
                self.note = Some(Err(SharedString::new_static(match error {
                    Unjoinable::UnreadableCode => words().pcs.unreadable_code,
                    Unjoinable::NoAddress => words().pcs.no_address,
                })));
                cx.notify();
                return;
            }
        };
        let result = join_in_background(&self.home, &self.settings_named(cx), code, address);
        self.joining = true;
        self.note = None;
        cx.notify();
        cx.spawn(async move |view, cx| {
            let joined = result
                .await
                .unwrap_or_else(|_| Err(String::from(words().pcs.join_stopped)));
            let _ = view.update(cx, |view, cx| {
                view.joining = false;
                view.note = Some(match joined {
                    Ok(peer) => {
                        view.code.update(cx, |input, cx| input.set_text("", cx));
                        view.address.update(cx, |input, cx| input.set_text("", cx));
                        Ok(SharedString::from((words().pcs.joined)(&shown_name(
                            &peer.name,
                            &peer.device,
                        ))))
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

/// Joins another PC from a window.
pub(crate) type Joiner =
    fn(&Home, &Settings, Code, String) -> oneshot::Receiver<Result<Peer, String>>;

/// Why typed words cannot join another PC.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Unjoinable {
    UnreadableCode,
    NoAddress,
}

/// The code and address typed to join another PC, or what is wrong with them. The address typed
/// wins over the one in the code.
pub(crate) fn join_target(code: &str, address: &str) -> Result<(Code, String), Unjoinable> {
    let code = Code::read(code.trim()).ok_or(Unjoinable::UnreadableCode)?;
    let address = Some(address.trim())
        .filter(|address| !address.is_empty())
        .map(str::to_owned)
        .or_else(|| code.address.clone())
        .ok_or(Unjoinable::NoAddress)?;
    Ok((code, address))
}

/// Joins the PC that handed out `code` on a thread of its own, since a window runs no async
/// runtime, and approves it in this PC's book.
pub(crate) fn join_in_background(
    home: &Home,
    settings: &Settings,
    code: Code,
    address: String,
) -> oneshot::Receiver<Result<Peer, String>> {
    let (done, result) = oneshot::channel();
    let prepared = Identity::load_or_make(&home.peer_key())
        .and_then(|identity| Ok((identity, peers::hello(home, settings)?)));
    let book = home.peers();
    thread::spawn(move || {
        let joined = prepared.and_then(|(identity, hello)| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| error.to_string())
                .and_then(|runtime| {
                    runtime.block_on(join(&identity, &hello, &code, &address, &book))
                })
        });
        let _ = done.send(joined);
    });
    result
}

/// What the windows call a PC: its name, or the start of its device id when it has none.
pub(crate) fn shown_name(name: &str, device: &str) -> String {
    if name.trim().is_empty() {
        format!("PC {}", device.get(..6).unwrap_or(device))
    } else {
        name.to_owned()
    }
}

impl Render for PcsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(window);
        let words = &words().pcs;
        let listening = self.settings.peers.is_some();
        let shown = peers::name(&self.settings_named(cx));
        let naming = div()
            .flex()
            .flex_col()
            .gap_1p5()
            .child(input_box(&theme, window, cx, "pc-name", &self.name))
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted)
                    .child(prose(words.name_hint)),
            );
        let this_pc =
            div()
                .flex()
                .items_center()
                .gap_3()
                .px_3()
                .py_2p5()
                .rounded_xl()
                .bg(theme.canvas)
                .child(badge(&theme, &shown))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .child((words.this_pc)(&shown)),
                        )
                        .child(div().text_xs().text_color(theme.muted).child(prose(
                            &if listening {
                                match self
                                    .settings
                                    .peers
                                    .as_ref()
                                    .and_then(|peers| peers.address.clone())
                                {
                                    Some(address) => (words.reachable_at)(&address),
                                    None => String::from(words.reachable),
                                }
                            } else {
                                String::from(words.not_listening)
                            },
                        ))),
                );
        let invite = match &self.invite {
            None => div().flex().child(button(
                &theme,
                "invite",
                words.make_invite,
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
                                words.copied
                            } else {
                                words.copy
                            },
                            true,
                            cx.listener(|view, _, _, cx| view.copy(cx)),
                        )),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted)
                        .child((words.invite_hint)(
                            until.saturating_sub(now()).div_ceil(60),
                        )),
                ),
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
                    words.joining
                } else {
                    words.join
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
                    .child(words.no_pcs),
            );
        }
        for peer in &self.book.peers {
            let seen = self.seen.peers.get(&peer.key);
            let linked = seen.is_some_and(|seen| seen.linked);
            let state = match seen {
                Some(seen) if seen.linked => String::from(words.connected),
                Some(seen) => (words.dropped)(now().saturating_sub(seen.at)),
                None => String::from(words.never_connected),
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
                            .child(words.remove)
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
                            .child(words.title),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted)
                            .child((words.about)()),
                    ),
            )
            .child(this_pc)
            .child(section(&theme, words.name_section, naming))
            .child(section(&theme, words.invite_section, invite))
            .child(section(&theme, words.join_section, join_form))
            .child(section(&theme, words.linked_section, list))
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
                .bg(theme.canvas)
                .text_color(theme.text)
                .hover(|button| button.bg(theme.bubble))
        })
        .when(!quiet, |button| {
            button
                .main_button(theme)
                .hover(|button| button.opacity(0.9))
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
    let ring =
        (focus.is_focused(window) && window.is_window_active()).then(|| theme.primary.into());
    let width = hairline(window);
    framed(
        ring,
        8. - width,
        width,
        div()
            .id(id)
            .debug_selector(move || id.to_owned())
            .px_3()
            .py_2()
            .rounded(px(8. - width))
            .bg(theme.canvas)
            .cursor_text()
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                window.focus(&focus);
                cx.stop_propagation();
            })
            .child(input.clone()),
    )
}
