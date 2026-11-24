use std::mem;

use gpui::App;
use gpui::ClickEvent;
use gpui::Context;
use gpui::Entity;
use gpui::FocusHandle;
use gpui::Focusable;
use gpui::FontWeight;
use gpui::MouseButton;
use gpui::PathPromptOptions;
use gpui::SharedString;
use gpui::Task;
use gpui::Window;
use gpui::div;
use gpui::prelude::*;
use gpui::px;

use crate::home::Home;
use crate::icons;
use crate::settings::ANTHROPIC_KEY;
use crate::settings::ASSISTANT_NAME;
use crate::settings::ASSISTANT_NAME_LONGEST;
use crate::settings::AssistantSettings;
use crate::settings::DISCORD_KEY;
use crate::settings::DiscordForm;
use crate::settings::Form;
use crate::settings::MODEL_KEY;
use crate::settings::ModelSettings;
use crate::settings::SLACK_APP_KEY;
use crate::settings::SLACK_BOT_KEY;
use crate::settings::Settings;
use crate::settings::SlackForm;
use crate::settings::TELEGRAM_KEY;
use crate::settings::TelegramForm;
use crate::settings::WindowSettings;
use crate::settings::environment;
use crate::vault::Vault;
use crate::vault::now_ms;
use crate::window::chrome::Frame;
use crate::window::identity;
use crate::window::identity::Identity;
use crate::window::identity::Particle;
use crate::window::identity::called;
use crate::window::input::TextInput;
use crate::window::notes::NotesView;
use crate::window::parts::logo;
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
    api_key_env: Option<String>,
    api_key: Entity<TextInput>,
    effort: Entity<TextInput>,
    other: Form,
    slack_on: bool,
    slack_owner: Entity<TextInput>,
    slack_app_token: Entity<TextInput>,
    slack_bot_token: Entity<TextInput>,
    telegram_on: bool,
    telegram_owner: Entity<TextInput>,
    telegram_token: Entity<TextInput>,
    discord_on: bool,
    discord_owner: Entity<TextInput>,
    discord_token: Entity<TextInput>,
    vault: Vault,
    kept: Vec<String>,
    name: Entity<TextInput>,
    picking: Option<Task<()>>,
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
        let secret = |cx: &mut Context<Self>| {
            let input = cx.new(|cx| TextInput::masked("붙여 넣기", cx));
            cx.observe(&input, |_, _, cx| cx.notify()).detach();
            input
        };
        let api_key = secret(cx);
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
        let telegram_token = secret(cx);
        let discord = DiscordForm::of(settings.discord.as_ref());
        let discord_token = secret(cx);
        let slack_app_token = secret(cx);
        let slack_bot_token = secret(cx);
        let vault = Vault::new(&home.root);
        let kept = vault.names().unwrap_or_default();
        let name = field(
            ASSISTANT_NAME,
            settings
                .assistant
                .as_ref()
                .and_then(|assistant| assistant.name.clone()),
            cx,
        );
        Self {
            pcs,
            routines,
            notes,
            home,
            earlier: settings,
            openai_compatible: form.openai_compatible,
            model: field("기본값", form.model, cx),
            base_url: field("기본값", form.base_url, cx),
            api_key_env: form.api_key_env,
            api_key,
            effort: field("기본값", form.effort, cx),
            other,
            slack_on: slack.on,
            slack_owner: field("U012AB3CD", slack.owner, cx),
            slack_app_token,
            slack_bot_token,
            telegram_on: telegram.on,
            telegram_owner: field("123456789", telegram.owner, cx),
            telegram_token,
            discord_on: discord.on,
            discord_owner: field("123456789012345678", discord.owner, cx),
            discord_token,
            vault,
            kept,
            name,
            picking: None,
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
        self.api_key_env = next.api_key_env;
        self.api_key.update(cx, |input, cx| input.set_text("", cx));
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
            api_key_env: self.api_key_env.clone(),
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
            app_token_env: self
                .earlier
                .slack
                .as_ref()
                .and_then(|slack| slack.app_token_env.clone()),
            bot_token_env: self
                .earlier
                .slack
                .as_ref()
                .and_then(|slack| slack.bot_token_env.clone()),
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
            token_env: self
                .earlier
                .telegram
                .as_ref()
                .and_then(|telegram| telegram.token_env.clone()),
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
            token_env: self
                .earlier
                .discord
                .as_ref()
                .and_then(|discord| discord.token_env.clone()),
        }
    }

    fn save(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let mut form = self.form(cx);
        if form.openai_compatible
            && form.api_key_env.is_none()
            && !self.api_key.read(cx).text().trim().is_empty()
        {
            form.api_key_env = Some(MODEL_KEY.to_owned());
        }
        let api_key_env = form.api_key_env.clone();
        let slack = self.slack_form(cx);
        let telegram = self.telegram_form(cx);
        let discord = self.discord_form(cx);
        let typed_name = self.name.read(cx).text().trim().to_owned();
        let wanted = form.settings(&self.earlier.model).and_then(|model| {
            let on_disk = Settings::load(&self.home.settings())?;
            let assistant = assistant_named(on_disk.assistant.clone(), &typed_name)?;
            Ok(Settings {
                model,
                slack: slack.settings()?,
                telegram: telegram.settings()?,
                discord: discord.settings()?,
                assistant,
                ..on_disk
            })
        });
        let saved = wanted.and_then(|settings| {
            let before = mem::replace(&mut self.api_key_env, api_key_env);
            if let Err(error) = self.keep_secrets(cx) {
                let kept_model_key = self
                    .api_key_name()
                    .is_some_and(|name| self.vault.names().unwrap_or_default().contains(&name));
                if !kept_model_key {
                    self.api_key_env = before;
                }
                return Err(error);
            }
            settings.save_parts(
                &self.home.settings(),
                &["model", "slack", "telegram", "discord", "assistant"],
            )?;
            self.earlier = settings;
            identity::choose(Identity::read(&self.home.settings(), &self.home.icons()));
            Ok(())
        });
        self.kept = self.vault.names().unwrap_or_default();
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
        let frame = Frame::of(window);
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
            .child(self.secret_field(
                &theme,
                window,
                cx,
                "API 키",
                &self.api_key,
                self.api_key_name(),
            ));
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
                .into_any_element()
        });
        let note = note.unwrap_or_else(|| {
            div()
                .text_xs()
                .text_color(theme.muted)
                .child("바뀐 설정은 dotori를 다시 시작하면 적용돼요")
                .into_any_element()
        });
        frame
            .resizable(div())
            .flex()
            .flex_col()
            .size_full()
            .bg(theme.glass().canvas)
            .text_color(theme.text)
            .text_sm()
            .line_height(px(21.))
            .child(
                frame.strip(
                    &theme,
                    div()
                        .text_base()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("설정"),
                    div(),
                ),
            )
            .child(
                div()
                    .id("settings")
                    .flex_1()
                    .overflow_y_scroll()
                    .px_5()
                    .pb_5()
                    .child(self.identity_card(&theme, window, cx))
                    .child(div().h_4())
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
                                            .child("대화 모델"),
                                    )
                                    .child(div().text_xs().text_color(theme.muted).child(format!(
                                        "{} 생각하고 답할 때 쓰는 모델이에요.",
                                        called(Particle::Subject)
                                    ))),
                            )
                            .child(
                                div()
                                    .flex()
                                    .p_1()
                                    .gap_1()
                                    .rounded_xl()
                                    .bg(theme.canvas)
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
                    .child(div().flex_1().child(note))
                    .child(
                        div()
                            .id("save")
                            .debug_selector(|| "save".to_owned())
                            .flex_none()
                            .px_5()
                            .py_2()
                            .rounded_lg()
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
    fn secrets(&self) -> [(&Entity<TextInput>, Option<String>); 5] {
        let named = |named: Option<&String>, default: &str| {
            named.cloned().unwrap_or_else(|| default.to_owned())
        };
        let slack = self.earlier.slack.as_ref();
        [
            (&self.api_key, self.api_key_name()),
            (
                &self.slack_app_token,
                Some(named(
                    slack.and_then(|slack| slack.app_token_env.as_ref()),
                    SLACK_APP_KEY,
                )),
            ),
            (
                &self.slack_bot_token,
                Some(named(
                    slack.and_then(|slack| slack.bot_token_env.as_ref()),
                    SLACK_BOT_KEY,
                )),
            ),
            (
                &self.telegram_token,
                Some(named(
                    self.earlier
                        .telegram
                        .as_ref()
                        .and_then(|telegram| telegram.token_env.as_ref()),
                    TELEGRAM_KEY,
                )),
            ),
            (
                &self.discord_token,
                Some(named(
                    self.earlier
                        .discord
                        .as_ref()
                        .and_then(|discord| discord.token_env.as_ref()),
                    DISCORD_KEY,
                )),
            ),
        ]
    }

    fn api_key_name(&self) -> Option<String> {
        match (&self.api_key_env, self.openai_compatible) {
            (Some(name), _) => Some(name.clone()),
            (None, false) => Some(ANTHROPIC_KEY.to_owned()),
            (None, true) => None,
        }
    }

    fn keep_secrets(&mut self, cx: &mut Context<Self>) -> Result<(), String> {
        let typed: Vec<(Entity<TextInput>, String, String)> = self
            .secrets()
            .into_iter()
            .filter_map(|(input, name)| {
                let value = input.read(cx).text().trim().to_owned();
                Some((input.clone(), name?, value)).filter(|(_, _, value)| !value.is_empty())
            })
            .collect();
        for (_, name, value) in &typed {
            self.vault
                .set(name, value, now_ms())
                .map_err(|error| vault_failed("넣지", &error))?;
        }
        for (input, _, _) in typed {
            input.update(cx, |input, cx| input.set_text("", cx));
        }
        Ok(())
    }

    fn forget_model_key_name(&mut self, name: &str) -> Result<(), String> {
        if name != MODEL_KEY || self.api_key_env.as_deref() != Some(MODEL_KEY) {
            return Ok(());
        }
        self.api_key_env = None;
        let mut settings = Settings::load(&self.home.settings())?;
        if let ModelSettings::OpenaiCompatible { api_key_env, .. } = &mut settings.model
            && api_key_env.as_deref() == Some(MODEL_KEY)
        {
            *api_key_env = None;
            settings.save_parts(&self.home.settings(), &["model"])?;
            self.earlier.model = settings.model;
        }
        Ok(())
    }

    fn forget(&mut self, name: &str, cx: &mut Context<Self>) {
        let forgot = self
            .vault
            .remove(name, now_ms())
            .map_err(|error| vault_failed("지우지", &error))
            .and_then(|()| self.forget_model_key_name(name));
        self.kept = self.vault.names().unwrap_or_default();
        self.note = forgot
            .err()
            .map(|error| Note::Failed(SharedString::from(error)));
        cx.notify();
    }

    fn secret_field(
        &self,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
        label: &'static str,
        input: &Entity<TextInput>,
        name: Option<String>,
    ) -> impl IntoElement + use<> {
        let typed = !input.read(cx).text().trim().is_empty();
        let from_environment = name.as_deref().and_then(environment).is_some();
        let kept = name
            .as_ref()
            .is_some_and(|name| self.kept.iter().any(|kept| kept == name));
        let (ink, soft, words) = if typed {
            (
                theme.muted,
                theme.bubble,
                "저장하면 금고에 안전하게 보관돼요",
            )
        } else if from_environment {
            (
                theme.ok,
                theme.ok_soft,
                "이 PC의 환경 변수 값을 쓰고 있어요",
            )
        } else if kept {
            (theme.ok, theme.ok_soft, "금고에 안전하게 보관돼 있어요")
        } else if name.is_none() {
            (theme.muted, theme.bubble, "키가 필요 없으면 비워 둬요")
        } else {
            (theme.warn, theme.warn_soft, "아직 넣지 않았어요")
        };
        let forget = (kept && !typed).then(|| {
            let name = name.clone().unwrap_or_default();
            div()
                .id(SharedString::from(format!("forget-{name}")))
                .debug_selector({
                    let name = name.clone();
                    move || format!("forget-{name}")
                })
                .px_2()
                .py_0p5()
                .rounded_md()
                .text_xs()
                .text_color(theme.muted)
                .cursor_pointer()
                .hover(|forget| forget.bg(theme.danger_soft).text_color(theme.danger))
                .child("지우기")
                .on_click(cx.listener(move |view, _, _, cx| view.forget(&name, cx)))
        });
        let hint = match &name {
            Some(name) => format!(
                "이 PC에서만 열리게 잠가 금고에 보관하고, 묶인 PC에는 암호화해서 보내요. dotori는 이 값을 대화하는 AI에게 보내지 않아요. 같은 이름({name})의 환경 변수가 있으면 그 값을 먼저 써요."
            ),
            None => String::from(
                "이 PC에서만 열리게 잠가 금고에 보관하고, 묶인 PC에는 암호화해서 보내요. dotori는 이 값을 대화하는 AI에게 보내지 않아요.",
            ),
        };
        div()
            .flex()
            .flex_col()
            .gap_1p5()
            .child(field(theme, window, cx, label, "", input))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .px_2()
                            .py_0p5()
                            .rounded_full()
                            .bg(soft)
                            .text_color(ink)
                            .text_xs()
                            .child(words),
                    )
                    .children(forget),
            )
            .child(div().text_xs().text_color(theme.muted).child(prose(&hint)))
    }

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

    fn pick_icon(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(SharedString::new_static("이 그림 쓰기")),
        });
        let folder = self.home.icons();
        self.picking = Some(cx.spawn(async move |view, cx| {
            let path = match paths.await {
                Ok(Ok(Some(paths))) => paths.into_iter().next(),
                Ok(Ok(None)) | Err(_) => None,
                Ok(Err(error)) => {
                    eprintln!("dotori: no file picker: {error}");
                    let _ = view.update(cx, |view, cx| {
                        view.note = Some(Note::Failed(SharedString::new_static(
                            "그림을 고르는 창을 열지 못했어요",
                        )));
                        cx.notify();
                    });
                    None
                }
            };
            let Some(path) = path else {
                return;
            };
            let kept = cx
                .background_executor()
                .spawn(async move {
                    let fail = |error: std::io::Error| format!("{}: {error}", path.display());
                    if std::fs::metadata(&path).map_err(fail)?.len() > icons::given_longest() {
                        return Err(String::from(TOO_LARGE));
                    }
                    std::fs::read(&path)
                        .map_err(fail)
                        .and_then(|picture| icons::keep(&folder, &picture))
                })
                .await;
            let _ = view.update(cx, |view, cx| view.use_icon(kept, cx));
        }));
    }

    fn use_icon(&mut self, kept: Result<String, String>, cx: &mut Context<Self>) {
        let set = kept
            .map_err(|error| {
                eprintln!("dotori: the picture was not kept: {error}");
                if error == TOO_LARGE {
                    String::from("20MB보다 작은 그림을 골라 주세요")
                } else if error.contains("does not read as an image") {
                    String::from("그림으로 읽지 못했어요. PNG, JPEG, GIF, WebP 그림을 골라 주세요")
                } else {
                    String::from("그림을 저장하지 못했어요. 잠시 뒤에 다시 해 보세요")
                }
            })
            .and_then(|name| self.set_icon(Some(name)));
        self.note = set
            .err()
            .map(|error| Note::Failed(SharedString::from(error)));
        cx.refresh_windows();
    }

    fn set_icon(&mut self, icon: Option<String>) -> Result<(), String> {
        let mut settings = Settings::load(&self.home.settings())?;
        let mut assistant = settings.assistant.take().unwrap_or_default();
        assistant.icon = icon;
        settings.assistant = (assistant != AssistantSettings::default()).then_some(assistant);
        settings.save_parts(&self.home.settings(), &["assistant"])?;
        let in_use = settings
            .assistant
            .as_ref()
            .and_then(|assistant| assistant.icon.clone());
        icons::prune(
            &self.home.icons(),
            &in_use.as_deref().into_iter().collect::<Vec<_>>(),
        );
        self.earlier.assistant = settings.assistant;
        identity::choose(Identity::read(&self.home.settings(), &self.home.icons()));
        Ok(())
    }

    fn identity_card(
        &self,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let custom = self
            .earlier
            .assistant
            .as_ref()
            .is_some_and(|assistant| assistant.icon.is_some());
        let button = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .debug_selector(move || id.to_owned())
                .px_3()
                .py_1p5()
                .rounded_lg()
                .bg(theme.bubble)
                .font_weight(FontWeight::MEDIUM)
                .cursor_pointer()
                .hover(|button| button.bg(theme.selection))
                .child(label)
        };
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
                            .child("이름과 아이콘"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted)
                            .child("채팅창과 답에서 이 이름과 그림으로 보여요. 묶인 PC에도 같이 보내요."),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_4()
                    .child(logo(theme, 56.))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_2()
                            .child(
                                button("icon-pick", "그림 올리기")
                                    .on_click(cx.listener(|view, _, _, cx| view.pick_icon(cx))),
                            )
                            .when(custom, |row| {
                                row.child(button("icon-default", "도토리로 되돌리기").on_click(
                                    cx.listener(|view, _, _, cx| {
                                        let set = view.set_icon(None);
                                        view.note = set
                                            .err()
                                            .map(|error| Note::Failed(SharedString::from(error)));
                                        cx.refresh_windows();
                                    }),
                                ))
                            }),
                    ),
            )
            .child(field(
                theme,
                window,
                cx,
                "이름",
                "비워 두면 dotori예요. 창에는 저장하면 바로, 대답에는 dotori를 다시 시작하면 적용돼요.",
                &self.name,
            ))
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
            format!(
                "Slack에서도 {} 말할 수 있어요. DM이나 멘션으로 부르면 같은 대화로 이어져요.",
                called(Particle::With)
            ),
            switch(theme, "slack-switch", on).on_click(cx.listener(|view, _, _, cx| {
                view.slack_on = !view.slack_on;
                view.note = None;
                cx.notify();
            })),
        );
        if !on {
            return card;
        }
        let [_, (_, app_name), (_, bot_name), _, _] = self.secrets();
        let app_token = self.secret_field(
            theme,
            window,
            cx,
            "앱 토큰",
            &self.slack_app_token,
            app_name,
        );
        let bot_token = self.secret_field(
            theme,
            window,
            cx,
            "봇 토큰",
            &self.slack_bot_token,
            bot_name,
        );
        card.child(guide(
            theme,
            &[
                "api.slack.com/apps에서 앱을 만들고 Socket Mode를 켜요. connections:write 권한으로 앱 토큰(xapp-)을 만들어요.",
                "봇 권한 app_mentions:read, chat:write, im:history, im:read, im:write, users:read, channels:read, groups:read를 주고, app_mention과 message.im 이벤트를 구독한 뒤 설치해서 봇 토큰(xoxb-)을 받아요.",
                "두 토큰을 아래에 붙여 넣고 저장한 뒤 dotori를 다시 시작해요.",
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
        .child(app_token)
        .child(bot_token)
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
        let [_, _, _, (_, name), _] = self.secrets();
        let token = self.secret_field(theme, window, cx, "봇 토큰", &self.telegram_token, name);
        card.child(guide(
            theme,
            &[
                "Telegram에서 @BotFather에게 /newbot을 보내 봇을 만들고 토큰을 받아요.",
                "토큰을 아래에 붙여 넣고 저장한 뒤 dotori를 다시 시작해요.",
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
        .child(token)
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
        let [_, _, _, _, (_, name)] = self.secrets();
        let token = self.secret_field(theme, window, cx, "봇 토큰", &self.discord_token, name);
        card.child(guide(
            theme,
            &[
                "discord.com/developers/applications에서 앱을 만들고 Bot 메뉴에서 토큰을 받아요. 같은 화면에서 Message Content Intent를 켜요.",
                "OAuth2 URL Generator에서 bot 범위와 View Channels, Send Messages, Read Message History 권한을 골라 내 서버에 봇을 초대해요.",
                "토큰을 아래에 붙여 넣고 저장한 뒤 dotori를 다시 시작해요.",
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
        .child(token)
    }
}

fn vault_failed(what: &str, error: &str) -> String {
    eprintln!("dotori: the vault: {error}");
    if error.contains("another key") {
        format!("이 PC의 금고 키가 금고와 맞지 않아서 키를 {what} 못했어요")
    } else {
        format!("키를 금고에 {what} 못했어요. 잠시 뒤에 다시 해 보세요")
    }
}

const TOO_LARGE: &str = "the picture is larger than 20 MB";

fn assistant_named(
    assistant: Option<AssistantSettings>,
    typed: &str,
) -> Result<Option<AssistantSettings>, String> {
    if typed.chars().count() > ASSISTANT_NAME_LONGEST {
        return Err(format!(
            "이름은 {ASSISTANT_NAME_LONGEST}자까지 쓸 수 있어요"
        ));
    }
    if typed.chars().any(char::is_control) {
        return Err(String::from("이름에는 글자만 써 주세요"));
    }
    let mut assistant = assistant.unwrap_or_default();
    assistant.name = (!typed.is_empty() && typed != ASSISTANT_NAME).then(|| typed.to_owned());
    Ok((assistant != AssistantSettings::default()).then_some(assistant))
}

fn channel_card(
    theme: &Theme,
    title: &'static str,
    about: impl Into<SharedString>,
    switch: impl IntoElement,
) -> gpui::Div {
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
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme.muted)
                                .child(prose(&about.into())),
                        ),
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
                .border_color(if focused {
                    theme.primary.into()
                } else {
                    gpui::transparent_black()
                })
                .cursor_text()
                .on_mouse_down(MouseButton::Left, move |_, window, _| window.focus(&focus))
                .child(input.clone()),
        )
        .when(!hint.is_empty(), |field| {
            field.child(div().text_xs().text_color(theme.muted).child(hint))
        })
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
