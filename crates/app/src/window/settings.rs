use std::mem;

use gpui::App;
use gpui::ClickEvent;
use gpui::Context;
use gpui::Entity;
use gpui::FocusHandle;
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
use crate::settings::ANTHROPIC_KEY;
use crate::settings::DISCORD_KEY;
use crate::settings::DiscordForm;
use crate::settings::Form;
use crate::settings::SLACK_APP_KEY;
use crate::settings::SLACK_BOT_KEY;
use crate::settings::Settings;
use crate::settings::SlackForm;
use crate::settings::TELEGRAM_KEY;
use crate::settings::TelegramForm;
use crate::settings::WindowSettings;
use crate::settings::environment;
use crate::window::input::TextInput;
use crate::window::notes::NotesView;
use crate::window::parts::header;
use crate::window::parts::prose;
use crate::window::pcs::PcsView;
use crate::window::routines::RoutinesView;
use crate::window::theme::Look;
use crate::window::theme::Theme;

/// The settings window. It writes `settings.toml`, which the daemon reads when it starts.
pub(crate) struct SettingsView {
    home: Home,
    earlier: Settings,
    openai_compatible: bool,
    model: Entity<TextInput>,
    base_url: Entity<TextInput>,
    api_key_env: Entity<TextInput>,
    effort: Entity<TextInput>,
    other: Form,
    slack_on: bool,
    slack_owner: Entity<TextInput>,
    slack_app_env: Entity<TextInput>,
    slack_bot_env: Entity<TextInput>,
    telegram_on: bool,
    telegram_owner: Entity<TextInput>,
    telegram_token_env: Entity<TextInput>,
    discord_on: bool,
    discord_owner: Entity<TextInput>,
    discord_token_env: Entity<TextInput>,
    note: Option<Note>,
    look_saving: Option<Task<()>>,
    pcs: Entity<PcsView>,
    routines: Entity<RoutinesView>,
    notes: Entity<NotesView>,
}

#[derive(Clone)]
enum Note {
    Saved,
    Failed(SharedString),
}

impl SettingsView {
    pub(crate) fn new(home: Home, settings: Settings, cx: &mut Context<Self>) -> Self {
        let form = Form::of(&settings.model);
        let field = |placeholder: &'static str, text: Option<String>, cx: &mut Context<Self>| {
            cx.new(|cx| {
                let mut input = TextInput::new(placeholder, cx);
                if let Some(text) = text {
                    input.set_text(&text, cx);
                }
                input
            })
        };
        let api_key_env = field("기본값", form.api_key_env.clone(), cx);
        cx.observe(&api_key_env, |_, _, cx| cx.notify()).detach();
        let other = Form {
            openai_compatible: !form.openai_compatible,
            ..Form::default()
        };
        let pcs = {
            let home = home.clone();
            let settings = settings.clone();
            cx.new(|cx| PcsView::new(home, settings, cx))
        };
        let routines = cx.new(|cx| RoutinesView::new(&home, cx));
        let notes = cx.new(|cx| NotesView::new(&home, cx));
        let slack = SlackForm::of(settings.slack.as_ref());
        let telegram = TelegramForm::of(settings.telegram.as_ref());
        let telegram_token_env = field(TELEGRAM_KEY, telegram.token_env.clone(), cx);
        cx.observe(&telegram_token_env, |_, _, cx| cx.notify())
            .detach();
        let discord = DiscordForm::of(settings.discord.as_ref());
        let discord_token_env = field(DISCORD_KEY, discord.token_env.clone(), cx);
        cx.observe(&discord_token_env, |_, _, cx| cx.notify())
            .detach();
        let slack_app_env = field(SLACK_APP_KEY, slack.app_token_env.clone(), cx);
        let slack_bot_env = field(SLACK_BOT_KEY, slack.bot_token_env.clone(), cx);
        cx.observe(&slack_app_env, |_, _, cx| cx.notify()).detach();
        cx.observe(&slack_bot_env, |_, _, cx| cx.notify()).detach();
        Self {
            pcs,
            routines,
            notes,
            home,
            earlier: settings,
            openai_compatible: form.openai_compatible,
            model: field("기본값", form.model, cx),
            base_url: field("기본값", form.base_url, cx),
            api_key_env,
            effort: field("기본값", form.effort, cx),
            other,
            slack_on: slack.on,
            slack_owner: field("U012AB3CD", slack.owner, cx),
            slack_app_env,
            slack_bot_env,
            telegram_on: telegram.on,
            telegram_owner: field("123456789", telegram.owner, cx),
            telegram_token_env,
            discord_on: discord.on,
            discord_owner: field("123456789012345678", discord.owner, cx),
            discord_token_env,
            note: None,
            look_saving: None,
        }
    }

    fn choose(&mut self, openai_compatible: bool, cx: &mut Context<Self>) {
        if self.openai_compatible == openai_compatible {
            return;
        }
        let shown = self.form(cx);
        let next = mem::replace(&mut self.other, shown);
        let fill = |field: &Entity<TextInput>, text: Option<String>, cx: &mut Context<Self>| {
            field.update(cx, |input, cx| {
                input.set_text(text.as_deref().unwrap_or(""), cx)
            });
        };
        fill(&self.model, next.model, cx);
        fill(&self.base_url, next.base_url, cx);
        fill(&self.api_key_env, next.api_key_env, cx);
        fill(&self.effort, next.effort, cx);
        self.openai_compatible = openai_compatible;
        self.note = None;
        cx.notify();
    }

    fn form(&self, cx: &Context<Self>) -> Form {
        let read = |field: &Entity<TextInput>| {
            let text = field.read(cx).text().trim().to_owned();
            (!text.is_empty()).then_some(text)
        };
        Form {
            openai_compatible: self.openai_compatible,
            model: read(&self.model),
            base_url: read(&self.base_url),
            api_key_env: read(&self.api_key_env),
            effort: (!self.openai_compatible)
                .then(|| read(&self.effort))
                .flatten(),
        }
    }

    fn slack_form(&self, cx: &Context<Self>) -> SlackForm {
        let read = |field: &Entity<TextInput>| {
            let text = field.read(cx).text().trim().to_owned();
            (!text.is_empty()).then_some(text)
        };
        SlackForm {
            on: self.slack_on,
            owner: read(&self.slack_owner),
            app_token_env: read(&self.slack_app_env),
            bot_token_env: read(&self.slack_bot_env),
        }
    }

    fn telegram_form(&self, cx: &Context<Self>) -> TelegramForm {
        let read = |field: &Entity<TextInput>| {
            let text = field.read(cx).text().trim().to_owned();
            (!text.is_empty()).then_some(text)
        };
        TelegramForm {
            on: self.telegram_on,
            owner: read(&self.telegram_owner),
            token_env: read(&self.telegram_token_env),
        }
    }

    fn discord_form(&self, cx: &Context<Self>) -> DiscordForm {
        let read = |field: &Entity<TextInput>| {
            let text = field.read(cx).text().trim().to_owned();
            (!text.is_empty()).then_some(text)
        };
        DiscordForm {
            on: self.discord_on,
            owner: read(&self.discord_owner),
            token_env: read(&self.discord_token_env),
        }
    }

    fn save(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let form = self.form(cx);
        let slack = self.slack_form(cx);
        let telegram = self.telegram_form(cx);
        let discord = self.discord_form(cx);
        let saved = form.settings(&self.earlier.model).and_then(|model| {
            let slack = slack.settings()?;
            let telegram = telegram.settings()?;
            let discord = discord.settings()?;
            let settings = Settings {
                model,
                slack,
                telegram,
                discord,
                ..Settings::load(&self.home.settings())?
            };
            settings.save_parts(
                &self.home.settings(),
                &["model", "slack", "telegram", "discord"],
            )?;
            self.earlier = settings;
            Ok(())
        });
        self.note = Some(match saved {
            Ok(()) => Note::Saved,
            Err(error) => Note::Failed(SharedString::from(error)),
        });
        cx.notify();
    }
}

impl Focusable for SettingsView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.model.focus_handle(cx)
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(window);
        let segment = |id: &'static str,
                       label: &'static str,
                       openai_compatible: bool,
                       cx: &mut Context<Self>| {
            let chosen = self.openai_compatible == openai_compatible;
            div()
                .id(id)
                .flex_1()
                .py_1p5()
                .rounded_lg()
                .text_center()
                .cursor_pointer()
                .when(chosen, |segment| {
                    segment
                        .bg(theme.surface)
                        .shadow_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.text)
                })
                .when(!chosen, |segment| {
                    segment
                        .text_color(theme.muted)
                        .hover(|segment| segment.text_color(theme.text))
                })
                .child(label)
                .on_click(cx.listener(move |view, _, _, cx| view.choose(openai_compatible, cx)))
        };
        let mut fields = div()
            .flex()
            .flex_col()
            .gap_4()
            .child(field(
                &theme,
                window,
                cx,
                "모델",
                "비워 두면 기본 모델을 써요.",
                &self.model,
            ))
            .child(field(
                &theme,
                window,
                cx,
                "API 주소",
                "비워 두면 공식 주소를 써요.",
                &self.base_url,
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1p5()
                    .child(field(
                        &theme,
                        window,
                        cx,
                        "API 키가 든 환경 변수",
                        if self.openai_compatible {
                            "키는 파일에 저장하지 않아요. 키가 필요 없으면 비워 둬요."
                        } else {
                            "키는 파일에 저장하지 않아요. 비워 두면 ANTHROPIC_API_KEY예요."
                        },
                        &self.api_key_env,
                    ))
                    .when(
                        !self.openai_compatible
                            || !self.api_key_env.read(cx).text().trim().is_empty(),
                        |key| key.child(found(&theme, &self.api_key_env, ANTHROPIC_KEY, cx)),
                    ),
            );
        if !self.openai_compatible {
            fields = fields.child(field(
                &theme,
                window,
                cx,
                "생각하는 정도",
                "low, medium, high 중 하나예요.",
                &self.effort,
            ));
        }
        let note = self.note.clone().map(|note| {
            let (color, soft, words) = match note {
                Note::Saved => (
                    theme.ok,
                    theme.ok_soft,
                    SharedString::new_static("저장했어요. dotori를 다시 시작하면 적용돼요."),
                ),
                Note::Failed(error) => (theme.danger, theme.danger_soft, error),
            };
            div()
                .flex_1()
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
            .size_full()
            .bg(theme.canvas)
            .text_color(theme.text)
            .text_sm()
            .line_height(px(21.))
            .child(header(
                &theme,
                "설정",
                div()
                    .text_xs()
                    .text_color(theme.muted)
                    .child("바뀐 설정은 dotori를 다시 시작하면 적용돼요"),
            ))
            .child(
                div()
                    .id("settings")
                    .flex_1()
                    .overflow_y_scroll()
                    .p_5()
                    .child(self.look_card(&theme, cx))
                    .child(div().h_4())
                    .child(
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
                                            .child("대화 모델"),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(theme.muted)
                                            .child("dotori가 생각하고 답할 때 쓰는 모델이에요."),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .p_1()
                                    .gap_1()
                                    .rounded_xl()
                                    .bg(theme.canvas)
                                    .border_1()
                                    .border_color(theme.line)
                                    .child(segment("anthropic", "Anthropic", false, cx))
                                    .child(segment("openai", "OpenAI 호환", true, cx)),
                            )
                            .child(fields),
                    )
                    .child(div().h_4())
                    .child(self.slack_card(&theme, window, cx))
                    .child(div().h_4())
                    .child(self.telegram_card(&theme, window, cx))
                    .child(div().h_4())
                    .child(self.discord_card(&theme, window, cx))
                    .child(div().h_4())
                    .child(self.notes.clone())
                    .child(div().h_4())
                    .child(self.routines.clone())
                    .child(div().h_4())
                    .child(self.pcs.clone()),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_3()
                    .px_5()
                    .py_3()
                    .bg(theme.raised)
                    .border_t_1()
                    .border_color(theme.line)
                    .child(div().flex_1().children(note))
                    .child(
                        div()
                            .id("save")
                            .flex_none()
                            .px_5()
                            .py_2()
                            .rounded_xl()
                            .cursor_pointer()
                            .bg(theme.primary)
                            .hover(|save| save.bg(theme.primary_hover))
                            .text_color(theme.on_primary)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("저장")
                            .on_click(cx.listener(Self::save)),
                    ),
            )
    }
}

impl SettingsView {
    fn set_look(&mut self, name: &str, look: Look, cx: &mut Context<Self>) {
        crate::window::theme::choose(name, look);
        let path = self.home.settings();
        let window = WindowSettings {
            theme: Some(name.to_owned()),
            look: Some(look.name().to_owned()),
        };
        let earlier = self.look_saving.take();
        self.look_saving = Some(cx.spawn(async move |view, cx| {
            if let Some(earlier) = earlier {
                earlier.await;
            }
            let saved = cx
                .background_spawn(async move {
                    Settings::load(&path).and_then(|mut settings| {
                        settings.window = Some(window);
                        settings.save_parts(&path, &["window"])
                    })
                })
                .await;
            if let Err(error) = saved {
                let _ = view.update(cx, |view, cx| {
                    view.note = Some(Note::Failed(SharedString::from(error)));
                    cx.notify();
                });
            }
        }));
        cx.refresh_windows();
    }

    fn look_card(&self, theme: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let (index, look) = crate::window::theme::saved();
        let current = crate::window::theme::names().nth(index).unwrap_or_default();
        let themes = crate::window::theme::names().map(|name| {
            let chosen = name == current;
            let (light, dark) = crate::window::theme::named(name).unwrap_or((*theme, *theme));
            let swatch = |color| div().size(px(14.)).rounded_full().bg(color);
            div()
                .id(SharedString::from(format!("theme-{name}")))
                .debug_selector(move || format!("theme-{name}"))
                .flex()
                .items_center()
                .gap_3()
                .px_3()
                .py_2()
                .rounded_lg()
                .cursor_pointer()
                .when(chosen, |row| {
                    row.bg(theme.bubble).font_weight(FontWeight::SEMIBOLD)
                })
                .when(!chosen, |row| row.hover(|row| row.bg(theme.canvas)))
                .child(div().flex_1().child(theme_label(name)))
                .child(
                    div()
                        .flex()
                        .gap_1()
                        .child(swatch(light.canvas))
                        .child(swatch(light.primary))
                        .child(swatch(dark.canvas))
                        .child(swatch(dark.primary)),
                )
                .on_click(cx.listener(move |view, _, _, cx| view.set_look(name, look, cx)))
        });
        let looks = [Look::System, Look::Light, Look::Dark].map(|option| {
            let chosen = option == look;
            let label = match option {
                Look::System => "시스템 따라",
                Look::Light => "밝게",
                Look::Dark => "어둡게",
            };
            div()
                .id(SharedString::from(format!("look-{}", option.name())))
                .debug_selector(move || format!("look-{}", option.name()))
                .flex_1()
                .py_1p5()
                .rounded_lg()
                .text_center()
                .cursor_pointer()
                .when(chosen, |segment| {
                    segment
                        .bg(theme.surface)
                        .shadow_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.text)
                })
                .when(!chosen, |segment| {
                    segment
                        .text_color(theme.muted)
                        .hover(|segment| segment.text_color(theme.text))
                })
                .child(label)
                .on_click(cx.listener(move |view, _, _, cx| view.set_look(current, option, cx)))
        });
        div()
            .flex()
            .flex_col()
            .gap_4()
            .p_5()
            .rounded_2xl()
            .bg(theme.surface)
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
                            .child("모양"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted)
                            .child(if crate::window::theme::overridden() {
                                "DOTORI_THEME이 정해져 있어서 창은 그 모양으로 그려요. 고른 것은 저장돼요."
                            } else {
                                "고르면 모든 창이 바로 바뀌어요."
                            }),
                    ),
            )
            .child(div().flex().flex_col().gap_1().children(themes))
            .child(
                div()
                    .flex()
                    .p_1()
                    .gap_1()
                    .rounded_xl()
                    .bg(theme.canvas)
                    .children(looks),
            )
    }

    fn slack_card(
        &self,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let on = self.slack_on;
        let card = channel_card(
            theme,
            "Slack",
            "Slack에서도 dotori와 말할 수 있어요. DM이나 멘션으로 부르면 같은 대화로 이어져요.",
            switch(theme, "slack-switch", on).on_click(cx.listener(|view, _, _, cx| {
                view.slack_on = !view.slack_on;
                view.note = None;
                cx.notify();
            })),
        );
        if !on {
            return card;
        }
        let app_found = found(theme, &self.slack_app_env, SLACK_APP_KEY, cx);
        let bot_found = found(theme, &self.slack_bot_env, SLACK_BOT_KEY, cx);
        card.child(guide(
            theme,
            &[
                "api.slack.com/apps에서 앱을 만들고 Socket Mode를 켜요. connections:write 권한으로 앱 토큰(xapp-)을 만들어요.",
                "봇 권한 app_mentions:read, chat:write, im:history, im:read, im:write, users:read, channels:read, groups:read를 주고, app_mention과 message.im 이벤트를 구독한 뒤 설치해서 봇 토큰(xoxb-)을 받아요.",
                "두 토큰을 아래 이름의 환경 변수에 넣고 dotori를 다시 시작해요. 토큰은 파일에 저장하지 않아요.",
            ],
        ))
        .child(field(
            theme,
            window,
            cx,
            "내 Slack 멤버 ID",
            "Slack에서 내 프로필을 열고 더보기 메뉴의 \"멤버 ID 복사\"로 얻어요. 이 사람의 말만 들어요.",
            &self.slack_owner,
        ))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1p5()
                .child(field(
                    theme,
                    window,
                    cx,
                    "앱 토큰이 든 환경 변수",
                    "비워 두면 SLACK_APP_TOKEN이에요.",
                    &self.slack_app_env,
                ))
                .child(app_found),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1p5()
                .child(field(
                    theme,
                    window,
                    cx,
                    "봇 토큰이 든 환경 변수",
                    "비워 두면 SLACK_BOT_TOKEN이에요.",
                    &self.slack_bot_env,
                ))
                .child(bot_found),
        )
    }

    fn telegram_card(
        &self,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let on = self.telegram_on;
        let card = channel_card(
            theme,
            "Telegram",
            "Telegram 봇으로도 말할 수 있어요. 봇에게 바로 말하거나 그룹에서 부르면 같은 대화로 이어져요.",
            switch(theme, "telegram-switch", on).on_click(cx.listener(|view, _, _, cx| {
                view.telegram_on = !view.telegram_on;
                view.note = None;
                cx.notify();
            })),
        );
        if !on {
            return card;
        }
        let token_found = found(theme, &self.telegram_token_env, TELEGRAM_KEY, cx);
        card.child(guide(
            theme,
            &[
                "Telegram에서 @BotFather에게 /newbot을 보내 봇을 만들고 토큰을 받아요.",
                "토큰을 아래 이름의 환경 변수에 넣고 dotori를 다시 시작해요. 토큰은 파일에 저장하지 않아요.",
                "그룹에서도 쓰려면 봇을 그룹에 넣고 @봇이름으로 불러요.",
            ],
        ))
        .child(field(
            theme,
            window,
            cx,
            "내 Telegram 사용자 ID",
            "숫자예요. @userinfobot에게 아무 말이나 보내면 알려 줘요. 이 사람의 말만 들어요.",
            &self.telegram_owner,
        ))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1p5()
                .child(field(
                    theme,
                    window,
                    cx,
                    "봇 토큰이 든 환경 변수",
                    "비워 두면 TELEGRAM_BOT_TOKEN이에요.",
                    &self.telegram_token_env,
                ))
                .child(token_found),
        )
    }
}

impl SettingsView {
    fn discord_card(
        &self,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let on = self.discord_on;
        let card = channel_card(
            theme,
            "Discord",
            "Discord 봇으로도 말할 수 있어요. 봇에게 DM을 보내거나 서버에서 부르면 같은 대화로 이어져요.",
            switch(theme, "discord-switch", on).on_click(cx.listener(|view, _, _, cx| {
                view.discord_on = !view.discord_on;
                view.note = None;
                cx.notify();
            })),
        );
        if !on {
            return card;
        }
        let token_found = found(theme, &self.discord_token_env, DISCORD_KEY, cx);
        card.child(guide(
            theme,
            &[
                "discord.com/developers/applications에서 앱을 만들고 Bot 메뉴에서 토큰을 받아요. 같은 화면에서 Message Content Intent를 켜요.",
                "OAuth2 URL Generator에서 bot 범위와 View Channels, Send Messages, Read Message History 권한을 골라 내 서버에 봇을 초대해요.",
                "토큰을 아래 이름의 환경 변수에 넣고 dotori를 다시 시작해요. 토큰은 파일에 저장하지 않아요.",
            ],
        ))
        .child(field(
            theme,
            window,
            cx,
            "내 Discord 사용자 ID",
            "설정의 고급에서 개발자 모드를 켜고, 내 프로필을 눌러 \"사용자 ID 복사\"로 얻어요. 이 사람의 말만 들어요.",
            &self.discord_owner,
        ))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1p5()
                .child(field(
                    theme,
                    window,
                    cx,
                    "봇 토큰이 든 환경 변수",
                    "비워 두면 DISCORD_BOT_TOKEN이에요.",
                    &self.discord_token_env,
                ))
                .child(token_found),
        )
    }
}

fn channel_card(
    theme: &Theme,
    title: &'static str,
    about: &'static str,
    switch: impl IntoElement,
) -> gpui::Div {
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
                .items_center()
                .gap_3()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .gap_1()
                        .child(
                            div()
                                .text_base()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(title),
                        )
                        .child(div().text_xs().text_color(theme.muted).child(prose(about))),
                )
                .child(switch),
        )
}

fn switch(theme: &Theme, id: &'static str, on: bool) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .w(px(40.))
        .h(px(22.))
        .p(px(2.))
        .rounded_full()
        .cursor_pointer()
        .bg(if on { theme.primary } else { theme.line })
        .when(on, |switch| switch.justify_end())
        .child(
            div()
                .size(px(18.))
                .rounded_full()
                .bg(theme.surface)
                .shadow_sm(),
        )
}

fn guide(theme: &Theme, steps: &[&'static str]) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .p_3()
        .rounded_xl()
        .bg(theme.canvas)
        .text_xs()
        .text_color(theme.muted)
        .children(steps.iter().enumerate().map(|(n, words)| {
            div()
                .flex()
                .items_start()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .size(px(18.))
                        .mt(px(1.5))
                        .rounded_full()
                        .bg(theme.selection)
                        .text_color(theme.text)
                        .text_size(px(11.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(SharedString::from((n + 1).to_string())),
                )
                .child(div().flex_1().min_w_0().child(prose(words)))
        }))
}

fn found(theme: &Theme, input: &Entity<TextInput>, default: &str, cx: &App) -> gpui::Div {
    let typed = input.read(cx).text().trim().to_owned();
    let name = if typed.is_empty() {
        default.to_owned()
    } else {
        typed
    };
    let (ink, soft, words) = if environment(&name).is_some() {
        (theme.ok, theme.ok_soft, format!("{name}이 이 PC에 있어요"))
    } else {
        (
            theme.warn,
            theme.warn_soft,
            format!("{name}이 이 PC에 아직 없어요"),
        )
    };
    div().flex().child(
        div()
            .flex()
            .items_center()
            .gap_1p5()
            .px_2()
            .py_0p5()
            .rounded_full()
            .bg(soft)
            .text_color(ink)
            .text_xs()
            .child(div().flex_none().size(px(6.)).rounded_full().bg(ink))
            .child(words),
    )
}

fn field(
    theme: &Theme,
    window: &Window,
    cx: &App,
    label: &'static str,
    hint: &'static str,
    input: &Entity<TextInput>,
) -> impl IntoElement {
    let focus = input.focus_handle(cx);
    let focused = focus.is_focused(window);
    div()
        .flex()
        .flex_col()
        .gap_1p5()
        .child(div().font_weight(FontWeight::MEDIUM).child(label))
        .child(
            div()
                .id(label)
                .px_3()
                .py_2()
                .rounded_lg()
                .bg(theme.canvas)
                .border_1()
                .border_color(if focused { theme.primary } else { theme.line })
                .cursor_text()
                .on_mouse_down(MouseButton::Left, move |_, window, _| window.focus(&focus))
                .child(input.clone()),
        )
        .child(div().text_xs().text_color(theme.muted).child(hint))
}

fn theme_label(name: &str) -> &'static str {
    match name {
        "graphite" => "그래파이트",
        "paper" => "페이퍼",
        "forest" => "포레스트",
        "indigo" => "인디고",
        "acorn" => "도토리",
        _ => "테마",
    }
}

#[cfg(test)]
#[path = "settings.test.rs"]
mod tests;
