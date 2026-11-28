//! The window that greets the owner on the first run: a greeting, then the model's key, the
//! assistant's name and theme, and the other places to talk, one screen each. It is the settings
//! view walked a step at a time, so it saves the same way the settings window does. From the
//! greeting the owner can instead join a PC that already runs dotori, which sends the model and
//! its key over.

use std::mem;
use std::time::Duration;

use dotori_connectors::peer::hub::JoinError;
use gpui::AnimationElement;
use gpui::AnimationExt;
use gpui::App;
use gpui::Div;
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
use gpui::relative;
use gpui::svg;

use super::Note;
use super::SettingsView;
use super::Step;
use super::picker::AVATAR_MENU;
use super::switch;
use crate::first_run;
use crate::first_run::FirstRun;
use crate::home::Home;
use crate::settings::Settings;
use crate::settings::environment;
use crate::vault;
use crate::vault::Vault;
use crate::window::chrome::Frame;
use crate::window::input::TextInput;
use crate::window::motion::GREET;
use crate::window::motion::MESSAGE_ENTER;
use crate::window::motion::RISE;
use crate::window::motion::SLIDE;
use crate::window::motion::STEP;
use crate::window::parts::CHECK_ICON;
use crate::window::parts::CLOCK_ICON;
use crate::window::parts::FOLDER_ICON;
use crate::window::parts::GLOBE_ICON;
use crate::window::parts::PENCIL_ICON;
use crate::window::parts::picture_tile;
use crate::window::parts::prose;
use crate::window::parts::thinking;
use crate::window::parts::typeface;
use crate::window::pcs::Unjoinable;
use crate::window::pcs::join_target;
use crate::window::pcs::shown_name;
use crate::window::surface::SurfaceExt;
use crate::window::surface::framed;
use crate::window::surface::hairline;
use crate::window::theme::Theme;
use crate::window::tokens::AVATAR;
use crate::window::tokens::FEATURE;
use crate::window::tokens::GREET_TEXT;
use crate::window::tokens::SIDEBAR;
use crate::window::tokens::TILE;
use crate::window::tokens::WALK;
use crate::words::words;

pub(super) const FIELD: f32 = 12.;
const CARD: f32 = 14.;
const KEYS_PAGE: &str = "https://console.anthropic.com/settings/keys";
const LOOK_FOR_MODEL: Duration = Duration::from_secs(2);

/// The screen behind the greeting's link, which joins this PC to one the owner already uses.
pub(super) struct JoinScreen {
    pub(super) code: Entity<TextInput>,
    pub(super) address: Entity<TextInput>,
    pub(super) state: Joining,
    _work: Option<Task<()>>,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Joining {
    /// The owner types the code, with what went wrong last time.
    Typing(Option<Failure>),
    Waiting,
    /// Joined the PC of this name; `ready` once its model and key are here.
    Joined {
        name: String,
        ready: bool,
    },
}

/// What went wrong joining, shown under the field it is about.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Failure {
    Typed(Unjoinable),
    /// The other PC was not reached or refused the code, or first run could not be kept.
    Joining(SharedString),
}

impl Failure {
    fn words(&self) -> SharedString {
        let words = &words().onboarding;
        match self {
            Failure::Typed(Unjoinable::UnreadableCode) => {
                SharedString::new_static(words.code_unreadable)
            }
            Failure::Typed(Unjoinable::NoAddress) => SharedString::new_static(words.address_needed),
            Failure::Joining(words) => words.clone(),
        }
    }
}

fn model_ready(home: &Home) -> bool {
    Settings::load(&home.settings()).is_ok_and(|settings| {
        settings
            .model(vault::lookup(&Vault::new(&home.root)))
            .is_ok()
    })
}

impl SettingsView {
    pub(super) fn walk_view(
        &mut self,
        step: Step,
        theme: &Theme,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Div {
        self.name_the_fields(cx);
        if step == Step::Key {
            self.ask_served_once(cx);
        }
        let frame = Frame::of(window);
        let body = match step {
            Step::Welcome if self.join.is_some() => self.join_step(theme, window, cx),
            Step::Welcome => div(),
            Step::Key => self.key_step(theme, window, cx),
            Step::Look => self.look_step(theme, window, cx),
            Step::Elsewhere => self.elsewhere_step(theme, window, cx),
        };
        let from = if self.forward { SLIDE } else { -SLIDE };
        let moving = div().child(body).with_animation(
            ("step", self.moves),
            STEP.animation(),
            move |step, shown| step.opacity(shown).relative().left(px(from * (1. - shown))),
        );
        let shell = div()
            .flex()
            .flex_col()
            .size_full()
            .font(typeface())
            .bg(theme.canvas)
            .text_color(theme.text)
            .text_sm()
            .line_height(px(21.))
            .child(frame.strip(theme, div(), div()));
        if step == Step::Welcome && self.join.is_none() {
            return shell.child(self.greeting(theme, cx));
        }
        if step == Step::Welcome {
            return shell.child(
                div().flex_1().min_h_0().flex().justify_center().child(
                    div()
                        .h_full()
                        .w(px(WALK))
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .id(("onboarding", step.position()))
                                .flex_1()
                                .overflow_y_scroll()
                                .px_6()
                                .pt_5()
                                .child(moving),
                        )
                        .child(self.walk_footer(step, theme, cx)),
                ),
            );
        }
        shell.child(self.walk_beside(step, theme, moving, cx))
    }

    fn walk_beside(
        &mut self,
        step: Step,
        theme: &Theme,
        body: impl IntoElement,
        cx: &mut gpui::Context<Self>,
    ) -> Div {
        let now = step.position();
        let steps = Step::ALL
            .iter()
            .filter_map(|other| other.label().map(|label| (other.position(), label)))
            .map(|(at, label)| {
                let current = at == now;
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .h(px(38.))
                    .px_3()
                    .rounded_md()
                    .when(current, |item| {
                        item.bg(theme.bubble).font_weight(FontWeight::SEMIBOLD)
                    })
                    .when(!current, |item| item.text_color(theme.muted))
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .size(px(20.))
                            .rounded_full()
                            .text_size(px(11.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .when(at < now, |mark| {
                                mark.bg(theme.primary).text_color(theme.canvas).child("✓")
                            })
                            .when(at == now, |mark| {
                                mark.main_button(theme).child(at.to_string())
                            })
                            .when(at > now, |mark| mark.bg(theme.bubble).child(at.to_string())),
                    )
                    .child(label)
            });
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .child(
                div()
                    .flex_none()
                    .w(px(SIDEBAR))
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .px_3()
                    .pt_2()
                    .children(steps),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .rounded_tl(px(16.))
                    .bg(theme.surface)
                    .child(
                        div()
                            .id(("onboarding", step.position()))
                            .flex_1()
                            .overflow_y_scroll()
                            .px(px(40.))
                            .pt(px(28.))
                            .child(div().max_w(px(WALK)).child(body)),
                    )
                    .child(
                        div().px(px(16.)).child(
                            div()
                                .max_w(px(WALK + 48.))
                                .child(self.walk_footer(step, theme, cx)),
                        ),
                    ),
            )
    }

    pub(super) fn name_the_fields(&mut self, cx: &mut gpui::Context<Self>) {
        let words = &words().onboarding;
        let key = if self.openai_compatible {
            words.key_optional
        } else {
            "sk-ant-…"
        };
        let address = if self.openai_compatible {
            "http://localhost:11434/v1"
        } else {
            "https://api.anthropic.com"
        };
        let walking = self.walk.is_some();
        let named = |onboarding: &'static str, example: &'static str| {
            if walking { onboarding } else { example }
        };
        for (input, placeholder) in [
            (&self.api_key, key),
            (&self.base_url, address),
            (
                &self.slack_app_token,
                named(words.slack_app_token, "xapp-…"),
            ),
            (
                &self.slack_bot_token,
                named(words.slack_bot_token, "xoxb-…"),
            ),
            (&self.slack_owner, named(words.slack_member, "U012AB3CD")),
            (&self.telegram_token, named(words.bot_token, "")),
            (&self.telegram_owner, named(words.user_id, "123456789")),
            (&self.discord_token, named(words.bot_token, "")),
            (
                &self.discord_owner,
                named(words.user_id, "123456789012345678"),
            ),
        ] {
            input.update(cx, |input, cx| {
                input.set_placeholder(SharedString::new_static(placeholder), cx)
            });
        }
    }

    fn greeting(&self, theme: &Theme, cx: &mut gpui::Context<Self>) -> Div {
        let words = &words().onboarding;
        let talk = [
            (
                FEATURE.time,
                CLOCK_ICON,
                words.schedule_ask,
                words.greet_replies[0],
            ),
            (
                FEATURE.web,
                GLOBE_ICON,
                words.web_ask,
                words.greet_replies[1],
            ),
            (
                FEATURE.files,
                FOLDER_ICON,
                words.pc_ask,
                words.greet_replies[2],
            ),
        ];
        let mut said = Vec::new();
        for (n, (color, icon, ask, reply)) in talk.into_iter().enumerate() {
            let at = GREET_FIRST + GREET_EVERY * n as u64;
            let thinks = at + GREET_HEARD;
            let answers = thinks + GREET_THINKS;
            said.push(
                arriving(
                    ("greet-ask", n),
                    at,
                    div().flex().justify_end().child(
                        div()
                            .max_w(relative(0.8))
                            .px_3p5()
                            .py_2()
                            .rounded_2xl()
                            .bg(theme.bubble)
                            .child(prose(ask.trim_matches('"'))),
                    ),
                )
                .into_any_element(),
            );
            let dots = thinking(theme.muted, ("greet-breath", n));
            let mark = div()
                .absolute()
                .top_0()
                .left_0()
                .px_3p5()
                .py_2()
                .rounded_2xl()
                .bg(theme.canvas)
                .child(dots)
                .with_animation(
                    ("greet-think", n),
                    MESSAGE_ENTER.between(
                        Duration::from_millis(thinks),
                        Duration::from_millis(answers),
                    ),
                    |thinking, shown| thinking.opacity(shown),
                );
            said.push(
                div()
                    .flex()
                    .items_start()
                    .gap(px(10.))
                    .child(arriving(
                        ("greet-face", n),
                        thinks,
                        div()
                            .mt(px(4.5))
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .size(px(28.))
                            .rounded(px(8.))
                            .bg(color)
                            .child(svg().path(icon).size(px(15.)).text_color(FEATURE.icon)),
                    ))
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .items_start()
                            .child(arriving(
                                ("greet-reply", n),
                                answers,
                                div()
                                    .max_w(relative(0.85))
                                    .px_3p5()
                                    .py_2()
                                    .rounded_2xl()
                                    .bg(theme.canvas)
                                    .child(prose(reply)),
                            ))
                            .child(mark),
                    )
                    .into_any_element(),
            );
        }
        let start = div()
            .id("next")
            .debug_selector(|| "next".to_owned())
            .flex()
            .items_center()
            .justify_center()
            .w(px(220.))
            .h(px(48.))
            .rounded(px(CARD))
            .main_button(theme)
            .cursor_pointer()
            .hover(|start| start.opacity(0.9))
            .text_size(px(15.))
            .font_weight(FontWeight::SEMIBOLD)
            .child(Step::Welcome.action())
            .on_click(cx.listener(Self::advance));
        let join = div()
            .id("join-pc")
            .debug_selector(|| "join-pc".to_owned())
            .text_size(px(13.))
            .line_height(px(18.))
            .text_color(theme.muted)
            .cursor_pointer()
            .child(words.join_existing)
            .on_click(cx.listener(|view, _, window, cx| view.open_join(window, cx)));
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .child(
                div()
                    .flex_none()
                    .w(px(GREET_TEXT))
                    .flex()
                    .flex_col()
                    .justify_center()
                    .pl(px(64.))
                    .pr(px(24.))
                    .child(picture_tile(theme, TILE, 20.).with_animation(
                        "greet-picture",
                        GREET.animation(),
                        |tile, shown| tile.opacity(shown).relative().top(px(-14. * (1. - shown))),
                    ))
                    .child(arriving(
                        "greet-hello",
                        140,
                        div()
                            .mt(px(28.))
                            .text_size(px(32.))
                            .line_height(px(42.))
                            .font_weight(FontWeight::BOLD)
                            .child(prose(&(words.hello)())),
                    ))
                    .child(arriving(
                        "greet-what",
                        220,
                        div()
                            .mt_3()
                            .flex()
                            .flex_col()
                            .text_size(px(15.))
                            .line_height(px(24.))
                            .text_color(theme.muted)
                            .child(words.ask_anything)
                            .child(words.on_it),
                    ))
                    .child(
                        div()
                            .mt(px(36.))
                            .flex()
                            .flex_col()
                            .items_start()
                            .gap_4()
                            .child(start)
                            .child(join),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .justify_center()
                    .my(px(56.))
                    .mr(px(40.))
                    .ml(px(24.))
                    .px_6()
                    .py(px(28.))
                    .gap(px(14.))
                    .rounded(px(20.))
                    .card(theme)
                    .children(said),
            )
    }

    pub(super) fn open_join(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) {
        let words = &words().onboarding;
        let code = cx.new(|cx| TextInput::new("", cx));
        let address = cx.new(|cx| TextInput::new(words.address_optional, cx));
        window.focus(&code.focus_handle(cx));
        self.join = Some(JoinScreen {
            code,
            address,
            state: Joining::Typing(None),
            _work: None,
        });
        self.moves += 1;
        self.slides.clear();
        self.forward = true;
        self.note = None;
        cx.notify();
    }

    pub(super) fn leave_join(&mut self, cx: &mut gpui::Context<Self>) {
        if !matches!(
            self.join.as_ref().map(|join| &join.state),
            Some(Joining::Typing(_))
        ) {
            return;
        }
        self.join = None;
        self.moves += 1;
        self.slides.clear();
        self.forward = false;
        cx.notify();
    }

    pub(super) fn join_advance(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) {
        let Some(join) = self.join.as_mut() else {
            return;
        };
        match &join.state {
            Joining::Typing(_) => {}
            Joining::Joined { ready: true, .. } => {
                if let Err(error) = first_run::write(&self.home.first_run(), FirstRun::Done) {
                    self.note = Some(Note::Failed(SharedString::from(error)));
                    cx.notify();
                    return;
                }
                (self.open_chat)();
                window.remove_window();
                return;
            }
            Joining::Waiting | Joining::Joined { .. } => return,
        }
        let typed = (
            join.code.read(cx).text().to_owned(),
            join.address.read(cx).text().to_owned(),
        );
        let (code, address) = match join_target(&typed.0, &typed.1) {
            Ok(target) => target,
            Err(error) => {
                join.state = Joining::Typing(Some(Failure::Typed(error)));
                cx.notify();
                return;
            }
        };
        let joined = (self.joiner)(&self.home, &self.earlier, code, address);
        for input in [&join.code, &join.address] {
            input.update(cx, |input, cx| input.set_locked(true, cx));
        }
        join.state = Joining::Waiting;
        self.note = None;
        let home = self.home.clone();
        join._work = Some(cx.spawn(async move |view, cx| {
            let joined = joined
                .await
                .unwrap_or_else(|_| Err(JoinError::Here(String::from(words().pcs.join_stopped))));
            let name = match joined {
                Ok(peer) => shown_name(&peer.name, &peer.device),
                Err(error) => {
                    eprintln!("dotori: joining another PC failed: {error}");
                    let failure = match error {
                        JoinError::There(_) => {
                            SharedString::from((words().onboarding.join_failed)())
                        }
                        JoinError::Here(error) => {
                            SharedString::from((words().onboarding.join_here_failed)(&error))
                        }
                    };
                    let _ = view.update(cx, |view, cx| {
                        if let Some(join) = view.join.as_mut() {
                            join.state = Joining::Typing(Some(Failure::Joining(failure)));
                            for input in [&join.code, &join.address] {
                                input.update(cx, |input, cx| input.set_locked(false, cx));
                            }
                        }
                        cx.notify();
                    });
                    return;
                }
            };
            loop {
                let home = home.clone();
                let ready = cx.background_spawn(async move { model_ready(&home) }).await;
                let shown = view.update(cx, |view, cx| {
                    if let Some(join) = view.join.as_mut() {
                        join.state = Joining::Joined {
                            name: name.clone(),
                            ready,
                        };
                    }
                    cx.notify();
                });
                if ready || shown.is_err() {
                    return;
                }
                cx.background_executor().timer(LOOK_FOR_MODEL).await;
            }
        }));
        cx.notify();
    }

    fn set_up_here(
        &mut self,
        _: &gpui::ClickEvent,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let settings = match Settings::load(&self.home.settings()) {
            Ok(settings) => settings,
            Err(error) => {
                self.note = Some(Note::Failed(SharedString::from(error)));
                cx.notify();
                return;
            }
        };
        let mut fresh = Self::opened(self.home.clone(), settings, self.walk, cx);
        fresh.moves = self.moves;
        fresh.forward = self.forward;
        fresh.slides = mem::take(&mut self.slides);
        fresh.check = self.check;
        fresh.open_chat = self.open_chat;
        fresh.joiner = self.joiner;
        *self = fresh;
        self.step_on(Step::Welcome, window, cx);
        cx.notify();
    }

    fn join_step(&self, theme: &Theme, window: &Window, cx: &mut gpui::Context<Self>) -> Div {
        let words = &words().onboarding;
        let Some(join) = &self.join else {
            return div();
        };
        let joined = match &join.state {
            Joining::Joined { name, ready } => Some((name, *ready)),
            _ => None,
        };
        let failed = match &join.state {
            Joining::Typing(Some(failure)) => Some(failure),
            _ => None,
        };
        let under = |about: bool| {
            failed.filter(|_| about).map(|failure| {
                div()
                    .mt_2()
                    .text_size(px(13.))
                    .text_color(theme.danger)
                    .child(prose(&failure.words()))
            })
        };
        let code_wrong = failed == Some(&Failure::Typed(Unjoinable::UnreadableCode));
        let address_wrong = failed == Some(&Failure::Typed(Unjoinable::NoAddress));
        div()
            .flex()
            .flex_col()
            .child(heading(words.join_title))
            .child(
                div()
                    .mb_4()
                    .text_size(px(13.))
                    .line_height(px(20.))
                    .text_color(theme.muted)
                    .child(prose(words.join_hint)),
            )
            .child(label(theme, words.code_label))
            .child(field(
                theme,
                window,
                cx,
                "join-code",
                &join.code,
                code_wrong,
            ))
            .children(under(code_wrong))
            .child(div().h_3p5())
            .child(label(theme, words.address_label))
            .child(field(
                theme,
                window,
                cx,
                "join-address",
                &join.address,
                address_wrong,
            ))
            .children(under(!code_wrong))
            .children(joined.map(|(name, ready)| {
                arriving(
                    ("joined", usize::from(ready)),
                    0,
                    div()
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .mt_3()
                        .child(
                            svg()
                                .path(CHECK_ICON)
                                .flex_none()
                                .size(px(14.))
                                .text_color(theme.ok),
                        )
                        .child(div().flex_1().text_size(px(13.)).child(if ready {
                            (words.received)(name)
                        } else {
                            (words.joined)(name)
                        })),
                )
            }))
    }

    fn key_step(&self, theme: &Theme, window: &Window, cx: &mut gpui::Context<Self>) -> Div {
        let words = &words().onboarding;
        let anthropic = !self.openai_compatible;
        let failed = match &self.note {
            Some(Note::Failed(error)) => Some(error.clone()),
            _ => None,
        };
        let name = self.api_key_name();
        let typed = !self.api_key.read(cx).text().trim().is_empty();
        let from_environment = name.as_deref().and_then(environment).is_some();
        let missing = |input: &Entity<TextInput>| input.read(cx).text().trim().is_empty();
        let no_address = failed.is_some() && missing(&self.base_url);
        let no_model = failed.is_some() && missing(&self.model);
        let provider =
            |id: &'static str, label: &'static str, openai: bool, cx: &mut gpui::Context<Self>| {
                super::segment(
                    theme,
                    id.to_owned(),
                    label,
                    self.openai_compatible == openai,
                )
                .on_click(cx.listener(move |view, _, _, cx| view.choose(openai, cx)))
            };
        let head = div()
            .flex()
            .flex_col()
            .child(heading(words.connect_title))
            .child(
                self.track(
                    theme,
                    super::PROVIDER,
                    usize::from(self.openai_compatible),
                    vec![
                        provider("anthropic", "Anthropic", false, cx),
                        provider("openai", words.compatible, true, cx),
                    ],
                )
                .mt_4()
                .mb_3(),
            );
        let mut body = div().flex().flex_col().child(if anthropic {
            div()
                .mb_4()
                .flex()
                .flex_wrap()
                .gap_x_1()
                .text_size(px(13.))
                .line_height(px(20.))
                .text_color(theme.muted)
                .children(
                    (words.key_use)()
                        .split(' ')
                        .map(|word| div().child(word.to_owned()))
                        .collect::<Vec<_>>(),
                )
                .child(
                    div()
                        .id("make-key")
                        .text_color(theme.text)
                        .font_weight(FontWeight::SEMIBOLD)
                        .cursor_pointer()
                        .child(words.make_key)
                        .on_click(|_, _, cx| cx.open_url(KEYS_PAGE)),
                )
        } else {
            div()
                .mb_4()
                .text_size(px(13.))
                .line_height(px(20.))
                .text_color(theme.muted)
                .child(prose(words.server_hint))
        });
        if !anthropic {
            body = body
                .child(label(theme, words.address_label))
                .child(field(
                    theme,
                    window,
                    cx,
                    "base-url",
                    &self.base_url,
                    no_address,
                ))
                .child(div().h_3p5())
                .child(label(theme, words.model_label))
                .child(self.model_picker(
                    theme,
                    window,
                    cx,
                    super::picker::Sits::Onboarding { alarm: no_model },
                ))
                .children(self.served_line(cx).map(|(line, warn)| {
                    div()
                        .mt_2()
                        .text_size(px(13.))
                        .line_height(px(20.))
                        .text_color(if warn { theme.warn } else { theme.muted })
                        .child(prose(line))
                }))
                .child(div().h_3p5())
                .child(label(theme, words.key_label));
        } else {
            body = body.child(label(theme, words.key_label));
        }
        body = body.child(field(
            theme,
            window,
            cx,
            "api-key",
            &self.api_key,
            failed.is_some() && anthropic,
        ));
        if let Some(error) = failed {
            return self.under_tabs(
                head,
                body.child(
                    div()
                        .mt_2()
                        .text_size(px(13.))
                        .text_color(theme.danger)
                        .child(prose(&error)),
                ),
            );
        }
        let from_environment = !typed && from_environment;
        let body = body.child(if from_environment {
            div()
                .flex()
                .items_center()
                .gap_1p5()
                .mt_2p5()
                .child(
                    svg()
                        .path(CHECK_ICON)
                        .flex_none()
                        .size(px(14.))
                        .text_color(theme.ok),
                )
                .child(
                    div()
                        .flex_1()
                        .text_size(px(13.))
                        .text_color(theme.text)
                        .child(words.key_from_environment),
                )
        } else {
            div()
                .mt_2()
                .text_size(px(13.))
                .line_height(px(20.))
                .text_color(theme.muted)
                .child(prose(words.key_safe))
        });
        self.under_tabs(head, body)
    }

    fn under_tabs(&self, head: Div, body: Div) -> Div {
        match self.slides.get(super::PROVIDER) {
            Some((_, moved)) => head.child(arriving(("under-tabs", *moved), 0, body)),
            None => head.child(body),
        }
    }

    /// The assistant's picture with a pencil, which picks another when pressed. The settings
    /// window and the onboarding window show the same one.
    /// The picture with its pencil. Without a picture of the owner's it opens the file picker;
    /// with one it opens a menu to choose another or go back to the default.
    pub(super) fn avatar(
        &self,
        theme: &Theme,
        window: &Window,
        cx: &mut gpui::Context<Self>,
    ) -> Div {
        let custom = self
            .earlier
            .assistant
            .as_ref()
            .is_some_and(|assistant| assistant.icon.is_some());
        let open = custom && self.open_picker == Some(AVATAR_MENU);
        let said = &words().settings;
        let menu = if open {
            self.actions_menu(
                theme,
                window,
                cx,
                AVATAR_MENU,
                vec![
                    ("avatar-other", said.other_picture, |view, cx| {
                        view.pick_icon(cx);
                    }),
                    ("avatar-default", said.default_picture, |view, cx| {
                        view.drop_icon(cx);
                    }),
                ],
            )
        } else {
            None
        };
        let tile = div()
            .id("avatar")
            .debug_selector(|| "avatar".to_owned())
            .relative()
            .flex_none()
            .cursor_pointer()
            .child(picture_tile(theme, AVATAR, 17.))
            .child(
                div()
                    .absolute()
                    .right(px(-5.))
                    .bottom(px(-5.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(24.))
                    .rounded_full()
                    .raised(theme)
                    .main_button(theme)
                    .child(
                        svg()
                            .path(PENCIL_ICON)
                            .size(px(11.))
                            .text_color(theme.canvas),
                    ),
            )
            .on_click(cx.listener(move |view, _, _, cx| {
                if custom {
                    view.open_picker = if view.open_picker == Some(AVATAR_MENU) {
                        None
                    } else {
                        Some(AVATAR_MENU)
                    };
                    cx.notify();
                } else {
                    view.pick_icon(cx);
                }
            }));
        div()
            .relative()
            .flex_none()
            .child(tile)
            .child(self.drawn.note(AVATAR_MENU))
            .children(menu)
    }

    fn look_step(&self, theme: &Theme, window: &Window, cx: &mut gpui::Context<Self>) -> Div {
        let words = &words().onboarding;
        let avatar = self.avatar(theme, window, cx);
        div()
            .flex()
            .flex_col()
            .child(heading(words.look_title))
            .child(
                div()
                    .text_size(px(13.))
                    .line_height(px(20.))
                    .text_color(theme.muted)
                    .child(prose(words.look_hint)),
            )
            .child(
                div()
                    .flex()
                    .items_end()
                    .gap_3p5()
                    .mt_4()
                    .child(avatar)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .child(label(theme, words.name_label))
                            .child(field(theme, window, cx, "name", &self.name, false)),
                    ),
            )
            .child(div().mt_4().child(label(theme, words.theme_label)))
            .child(
                div()
                    .py_1()
                    .rounded(px(CARD))
                    .bg(theme.canvas)
                    .child(self.theme_list(theme, cx)),
            )
            .child(div().mt_2p5().child(self.look_switch(theme, cx)))
    }

    fn elsewhere_step(&self, theme: &Theme, window: &Window, cx: &mut gpui::Context<Self>) -> Div {
        let words = &words().onboarding;
        let channel = |id: &'static str,
                       part: &'static str,
                       name: &'static str,
                       on: bool,
                       inputs: Vec<(&'static str, &Entity<TextInput>)>,
                       flip: fn(&mut SettingsView),
                       cx: &mut gpui::Context<Self>| {
            div()
                .flex()
                .flex_col()
                .gap_2()
                .px_3p5()
                .py_2p5()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .child(div().flex_1().child(name))
                        .child(
                            switch(theme, id, on, self.flipped(id)).on_click(cx.listener(
                                move |view, _, _, cx| {
                                    view.slid(id, 0);
                                    flip(view);
                                    view.note = None;
                                    cx.notify();
                                },
                            )),
                        ),
                )
                .when(on, |row| {
                    let opened = div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .children(inputs.into_iter().map(|(id, input)| {
                            field(theme, window, cx, id, input, self.alarmed(input))
                        }))
                        .children(
                            self.missing_line(theme, part)
                                .map(|line| line.text_size(px(13.)).line_height(px(20.))),
                        );
                    match self.flipped(id) {
                        Some(moved) => row.child(arriving((id, moved), 0, opened)),
                        None => row.child(opened),
                    }
                })
        };
        let channels = div()
            .py_1()
            .rounded(px(CARD))
            .bg(theme.canvas)
            .child(channel(
                "slack-switch",
                "slack",
                "Slack",
                self.slack_on,
                vec![
                    ("slack-app-token", &self.slack_app_token),
                    ("slack-bot-token", &self.slack_bot_token),
                    ("slack-owner", &self.slack_owner),
                ],
                |view| view.slack_on = !view.slack_on,
                cx,
            ))
            .child(channel(
                "telegram-switch",
                "telegram",
                "Telegram",
                self.telegram_on,
                vec![
                    ("telegram-token", &self.telegram_token),
                    ("telegram-owner", &self.telegram_owner),
                ],
                |view| view.telegram_on = !view.telegram_on,
                cx,
            ))
            .child(channel(
                "discord-switch",
                "discord",
                "Discord",
                self.discord_on,
                vec![
                    ("discord-token", &self.discord_token),
                    ("discord-owner", &self.discord_owner),
                ],
                |view| view.discord_on = !view.discord_on,
                cx,
            ));
        div()
            .flex()
            .flex_col()
            .child(heading(words.elsewhere_title))
            .child(
                div()
                    .text_size(px(13.))
                    .line_height(px(20.))
                    .text_color(theme.muted)
                    .child(prose(&(words.elsewhere_hint)())),
            )
            .child(div().mt_5().child(channels))
    }

    fn walk_footer(&self, step: Step, theme: &Theme, cx: &mut gpui::Context<Self>) -> Div {
        let words = &words().onboarding;
        let joining = self.join.as_ref().map(|join| &join.state);
        let failed = match (&self.note, step, joining) {
            (Some(Note::Failed(error)), step, None) if step != Step::Key => Some(error.clone()),
            (Some(Note::Failed(error)), _, Some(Joining::Joined { .. })) => Some(error.clone()),
            _ => None,
        };
        let backs = match joining {
            Some(state) => matches!(state, Joining::Typing(_)),
            None => step.previous().is_some(),
        };
        let (action, moves_on) = match joining {
            None if self.checking() => (None, false),
            None => (Some(step.action()), true),
            Some(Joining::Typing(_)) => (Some(words.join), true),
            Some(Joining::Waiting) => (Some(words.joining), false),
            Some(Joining::Joined { ready: false, .. }) => (Some(words.receiving), false),
            Some(Joining::Joined { ready: true, .. }) => (Some(words.begin_chat), true),
        };
        let under: Option<(&'static str, &'static str, bool)> = match joining {
            None if step == Step::Welcome => Some(("join-pc", words.join_existing, true)),
            Some(Joining::Joined { ready: false, .. }) => {
                Some(("set-up-here", words.set_up_here, false))
            }
            _ => None,
        };
        div()
            .flex()
            .flex_col()
            .flex_none()
            .gap_2()
            .px_6()
            .pt_4()
            .pb_6()
            .when(under.is_some(), |footer| footer.gap_1().pb_4())
            .children(failed.map(|error| {
                div()
                    .text_size(px(13.))
                    .line_height(px(19.))
                    .text_color(theme.danger)
                    .child(prose(&error))
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .when(backs, |footer| {
                        footer.child(
                            div()
                                .id("back")
                                .debug_selector(|| "back".to_owned())
                                .px_3()
                                .py_3()
                                .rounded(px(14.))
                                .text_color(theme.muted)
                                .when(!self.checking(), |back| {
                                    back.cursor_pointer().hover(|back| back.bg(theme.bubble))
                                })
                                .child(words.back)
                                .on_click(cx.listener(Self::go_back)),
                        )
                    })
                    .child(
                        div()
                            .id("next")
                            .debug_selector(|| "next".to_owned())
                            .flex_1()
                            .flex()
                            .items_center()
                            .justify_center()
                            .h(px(if step == Step::Welcome && joining.is_none() {
                                50.
                            } else {
                                48.
                            }))
                            .rounded(px(CARD))
                            .main_button(theme)
                            .when(moves_on, |next| {
                                next.cursor_pointer().hover(|next| next.opacity(0.9))
                            })
                            .text_size(px(15.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(match action {
                                Some(action) => action.into_any_element(),
                                None => thinking(theme.canvas, "checking").into_any_element(),
                            })
                            .on_click(cx.listener(Self::advance)),
                    ),
            )
            .children(under.map(|(id, words, opens)| {
                div()
                    .id(id)
                    .debug_selector(move || id.to_owned())
                    .flex()
                    .justify_center()
                    .py_1()
                    .text_size(px(13.))
                    .line_height(px(18.))
                    .text_color(theme.muted)
                    .cursor_pointer()
                    .child(words)
                    .on_click(cx.listener(move |view, event, window, cx| {
                        if opens {
                            view.open_join(window, cx);
                        } else {
                            view.set_up_here(event, window, cx);
                        }
                    }))
            }))
    }
}

fn heading(words: &'static str) -> Div {
    div()
        .mb_1()
        .text_size(px(22.))
        .line_height(px(30.))
        .font_weight(FontWeight::BOLD)
        .child(words)
}

const GREET_FIRST: u64 = 400;
const GREET_EVERY: u64 = 1200;
const GREET_HEARD: u64 = 250;
const GREET_THINKS: u64 = 600;

fn arriving(id: impl Into<gpui::ElementId>, delay: u64, element: Div) -> AnimationElement<Div> {
    element.with_animation(
        id,
        MESSAGE_ENTER.after(Duration::from_millis(delay)),
        |element, shown| {
            element
                .opacity(shown)
                .relative()
                .top(px(RISE * (1. - shown)))
        },
    )
}

fn label(theme: &Theme, words: &'static str) -> Div {
    div()
        .mb_1p5()
        .text_xs()
        .line_height(px(16.))
        .text_color(theme.muted)
        .child(words)
}

fn ring(
    theme: &Theme,
    window: &Window,
    input: &Entity<TextInput>,
    cx: &App,
    alarm: bool,
) -> Option<gpui::Hsla> {
    if alarm {
        Some(theme.danger.into())
    } else if input.focus_handle(cx).is_focused(window) && window.is_window_active() {
        Some(theme.primary.into())
    } else {
        None
    }
}

fn field(
    theme: &Theme,
    window: &Window,
    cx: &App,
    id: &'static str,
    input: &Entity<TextInput>,
    alarm: bool,
) -> Div {
    let focus = input.focus_handle(cx);
    let width = hairline(window);
    framed(
        ring(theme, window, input, cx, alarm),
        FIELD - width,
        width,
        div()
            .bg(theme.canvas)
            .id(id)
            .flex()
            .items_center()
            .h(px(42. - 2. * width))
            .px_3p5()
            .rounded(px(FIELD - width))
            .cursor_text()
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                window.focus(&focus);
                cx.stop_propagation();
            })
            .child(input.clone()),
    )
}
