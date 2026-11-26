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
use gpui::AnyElement;
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
use gpui::img;
use gpui::prelude::*;
use gpui::px;
use gpui::relative;
use gpui::svg;

use super::Note;
use super::SettingsView;
use super::Step;
use super::switch;
use crate::first_run;
use crate::first_run::FirstRun;
use crate::home::Home;
use crate::settings::Settings;
use crate::settings::environment;
use crate::vault;
use crate::vault::Vault;
use crate::window::chrome::Frame;
use crate::window::identity;
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
use crate::window::parts::logo;
use crate::window::parts::picture;
use crate::window::parts::prose;
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
use crate::window::tokens::TILE;
use crate::words::words;

const FIELD: f32 = 12.;
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
        let frame = Frame::of(window).fixed();
        let body = match step {
            Step::Welcome if self.join.is_some() => self.join_step(theme, window, cx),
            Step::Welcome => self.greeting(theme),
            Step::Key => self.key_step(theme, window, cx),
            Step::Look => self.look_step(theme, window, cx),
            Step::Elsewhere => self.elsewhere_step(theme, window, cx),
        };
        let from = if self.forward { SLIDE } else { -SLIDE };
        div()
            .flex()
            .flex_col()
            .size_full()
            .font(typeface())
            .bg(theme.canvas)
            .text_color(theme.text)
            .text_sm()
            .line_height(px(21.))
            .child(frame.strip(theme, div(), div()))
            .children(progress(step, self.moves, self.forward, theme))
            .child(
                div()
                    .id(("onboarding", step.position()))
                    .flex_1()
                    .overflow_y_scroll()
                    .px_6()
                    .pt_5()
                    .child(div().child(body).with_animation(
                        ("step", self.moves),
                        STEP.animation(),
                        move |step, shown| {
                            step.opacity(shown).relative().left(px(from * (1. - shown)))
                        },
                    )),
            )
            .child(self.walk_footer(step, theme, cx))
    }

    fn name_the_fields(&mut self, cx: &mut gpui::Context<Self>) {
        let words = &words().onboarding;
        let key = if self.openai_compatible {
            words.key_optional
        } else {
            "sk-ant-…"
        };
        for (input, placeholder) in [
            (&self.api_key, key),
            (&self.base_url, "http://localhost:11434/v1"),
            (&self.model, "gemma4"),
            (&self.slack_app_token, words.slack_app_token),
            (&self.slack_bot_token, words.slack_bot_token),
            (&self.slack_owner, words.slack_member),
            (&self.telegram_token, words.bot_token),
            (&self.telegram_owner, words.user_id),
            (&self.discord_token, words.bot_token),
            (&self.discord_owner, words.user_id),
        ] {
            input.update(cx, |input, cx| {
                input.set_placeholder(SharedString::new_static(placeholder), cx)
            });
        }
    }

    fn greeting(&self, theme: &Theme) -> Div {
        let words = &words().onboarding;
        let features = [
            (CLOCK_ICON, FEATURE.time, words.schedule, words.schedule_ask),
            (GLOBE_ICON, FEATURE.web, words.web, words.web_ask),
            (FOLDER_ICON, FEATURE.files, words.pc, words.pc_ask),
        ];
        div()
            .flex()
            .flex_col()
            .items_center()
            .child(picture_tile(theme, TILE, 24.).with_animation(
                "greet-picture",
                GREET.animation(),
                |tile, shown| tile.opacity(shown).relative().top(px(-14. * (1. - shown))),
            ))
            .child(arriving(
                "greet-hello",
                140,
                div()
                    .mt(px(22.))
                    .text_size(px(24.))
                    .line_height(px(32.))
                    .font_weight(FontWeight::BOLD)
                    .child((words.hello)()),
            ))
            .child(arriving(
                "greet-what",
                220,
                div()
                    .mt_2()
                    .flex()
                    .flex_col()
                    .items_center()
                    .line_height(px(22.))
                    .text_color(theme.muted)
                    .child(words.ask_anything)
                    .child(words.on_it),
            ))
            .child(
                div()
                    .mt(px(30.))
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap(px(18.))
                    .children(
                        features
                            .iter()
                            .enumerate()
                            .map(|(n, (icon, color, title, ask))| {
                                arriving(
                                    ("greet-feature", n),
                                    340 + 80 * n as u64,
                                    div()
                                        .flex()
                                        .items_start()
                                        .gap_3p5()
                                        .child(
                                            div()
                                                .flex()
                                                .flex_none()
                                                .items_center()
                                                .justify_center()
                                                .size(px(36.))
                                                .rounded(px(10.))
                                                .bg(*color)
                                                .child(
                                                    svg()
                                                        .path(*icon)
                                                        .size(px(18.))
                                                        .text_color(FEATURE.icon),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .flex()
                                                .flex_col()
                                                .flex_1()
                                                .min_w_0()
                                                .child(
                                                    div()
                                                        .font_weight(FontWeight::SEMIBOLD)
                                                        .child(*title),
                                                )
                                                .child(
                                                    div()
                                                        .text_size(px(13.))
                                                        .line_height(px(19.))
                                                        .text_color(theme.muted)
                                                        .child(prose(ask)),
                                                ),
                                        ),
                                )
                            }),
                    ),
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
                .child(field(theme, window, cx, "model", &self.model, no_model))
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

    fn look_step(&self, theme: &Theme, window: &Window, cx: &mut gpui::Context<Self>) -> Div {
        let words = &words().onboarding;
        let avatar = div()
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
            .on_click(cx.listener(|view, _, _, cx| view.pick_icon(cx)));
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
                    .card(theme)
                    .child(self.theme_list(theme, cx)),
            )
            .child(div().mt_2p5().child(self.look_switch(theme, cx)))
    }

    fn elsewhere_step(&self, theme: &Theme, window: &Window, cx: &mut gpui::Context<Self>) -> Div {
        let words = &words().onboarding;
        let channel = |id: &'static str,
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
                    let opened = div().flex().flex_col().gap_2().children(
                        inputs
                            .into_iter()
                            .map(|(id, input)| field(theme, window, cx, id, input, false)),
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
            .card(theme)
            .child(channel(
                "slack-switch",
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
            None if self.checking() => (words.checking, false),
            None => (step.action(), true),
            Some(Joining::Typing(_)) => (words.join, true),
            Some(Joining::Waiting) => (words.joining, false),
            Some(Joining::Joined { ready: false, .. }) => (words.receiving, false),
            Some(Joining::Joined { ready: true, .. }) => (words.begin_chat, true),
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
                            .child(action)
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

fn picture_tile(theme: &Theme, size: f32, radius: f32) -> Div {
    let picture: AnyElement = match identity::icon() {
        Some(icon) => img(picture(&icon, size))
            .size_full()
            .rounded(px(radius))
            .into_any_element(),
        None => logo(theme, size * 0.62).into_any_element(),
    };
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(size))
        .rounded(px(radius))
        .bg(theme.tile())
        .lifted(theme)
        .child(picture)
}

fn progress(step: Step, moves: usize, forward: bool, theme: &Theme) -> Option<Div> {
    step.label()?;
    let now = step.position();
    let filled = move |at: usize| -> AnyElement {
        let fill = div().h_full().rounded_full().bg(theme.primary);
        let (filling, emptying) = (forward && at == now, !forward && at == now + 1);
        if filling || emptying {
            return fill
                .with_animation(("bar", moves * 8 + at), STEP.animation(), move |fill, t| {
                    fill.w(relative(if filling { t } else { 1. - t }))
                })
                .into_any_element();
        }
        fill.w(relative(if at <= now { 1. } else { 0. }))
            .into_any_element()
    };
    Some(
        div().flex().flex_none().gap_1p5().px_6().children(
            Step::ALL
                .iter()
                .filter_map(|other| other.label().map(|label| (other.position(), label)))
                .map(|(at, label)| {
                    let current = at == now;
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .gap(px(7.))
                        .child(
                            div()
                                .h(px(3.))
                                .rounded_full()
                                .bg(theme.bubble)
                                .child(filled(at)),
                        )
                        .child(
                            div()
                                .text_size(px(11.))
                                .line_height(px(14.))
                                .text_color(if current { theme.text } else { theme.muted })
                                .when(current, |label| label.font_weight(FontWeight::SEMIBOLD))
                                .child(label),
                        )
                }),
        ),
    )
}

fn heading(words: &'static str) -> Div {
    div()
        .mb_1()
        .text_size(px(22.))
        .line_height(px(30.))
        .font_weight(FontWeight::BOLD)
        .child(words)
}

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
            .card(theme)
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
