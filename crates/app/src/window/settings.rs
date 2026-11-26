use std::collections::HashMap;
use std::mem;
use std::sync::Arc;
use std::time::Duration;

use std::thread;

use dotori_core::llm::CheckError;
use gpui::AnimationExt;
use gpui::AnyElement;
use gpui::App;
use gpui::ClickEvent;
use gpui::Context;
use gpui::Div;
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
use gpui::relative;
use gpui::svg;

use tokio::sync::oneshot;

use crate::accounts;
use crate::accounts::Site;
use crate::first_run;
use crate::first_run::FirstRun;
use crate::first_run::Step;
use crate::home::Home;
use crate::icons;
use crate::language;
use crate::language::Choice;
use crate::settings::ANTHROPIC_KEY;
use crate::settings::ASSISTANT_NAME;
use crate::settings::ASSISTANT_NAME_LONGEST;
use crate::settings::AssistantSettings;
use crate::settings::DISCORD_KEY;
use crate::settings::DiscordForm;
use crate::settings::Form;
use crate::settings::MODEL_KEY;
use crate::settings::ModelSettings;
use crate::settings::PC_NAME_LONGEST;
use crate::settings::PcSettings;
use crate::settings::SLACK_APP_KEY;
use crate::settings::SLACK_BOT_KEY;
use crate::settings::Settings;
use crate::settings::SlackForm;
use crate::settings::TELEGRAM_KEY;
use crate::settings::TelegramForm;
use crate::settings::WindowSettings;
use crate::settings::environment;
use crate::vault;
use crate::vault::Vault;
use crate::vault::now_ms;
use crate::window::chrome::Frame;
use crate::window::identity;
use crate::window::identity::Identity;
use crate::window::input::TextInput;
use crate::window::motion::SWITCH;
use crate::window::notes::NotesView;
use crate::window::parts::CHECK_ICON;
use crate::window::parts::TRASH_ICON;
use crate::window::parts::bolt;
use crate::window::parts::lock;
use crate::window::parts::logo;
use crate::window::parts::prose;
use crate::window::parts::typeface;
use crate::window::pcs::Joiner;
use crate::window::pcs::PcsView;
use crate::window::pcs::join_in_background;
use crate::window::routines::RoutinesView;
use crate::window::sandbox::SandboxView;
use crate::window::surface::SurfaceExt;
use crate::window::surface::framed;
use crate::window::surface::hairline;
use crate::window::theme::Look;
use crate::window::theme::Theme;
use crate::window::tokens::PICTURE;
use crate::words::words;

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
    sites: Vec<Site>,
    _watch_vault: Task<()>,
    name: Entity<TextInput>,
    picking: Option<Task<()>>,
    note: Option<Note>,
    look_saving: Option<Task<()>>,
    pcs: Entity<PcsView>,
    routines: Entity<RoutinesView>,
    notes: Entity<NotesView>,
    sandbox: Entity<SandboxView>,
    walk: Option<Step>,
    moves: usize,
    slides: HashMap<&'static str, (usize, usize)>,
    forward: bool,
    checking: Option<Task<()>>,
    check: Check,
    open_chat: fn(),
    join: Option<onboarding::JoinScreen>,
    joiner: Joiner,
}

const ALL_PARTS: [&str; 6] = ["model", "slack", "telegram", "discord", "assistant", "pc"];

impl Step {
    fn parts(self) -> &'static [&'static str] {
        match self {
            Step::Welcome => &[],
            Step::Key => &["model"],
            Step::Look => &["assistant"],
            Step::Elsewhere => &["slack", "telegram", "discord"],
        }
    }

    fn label(self) -> Option<&'static str> {
        let words = &words().onboarding;
        match self {
            Step::Welcome => None,
            Step::Key => Some(words.key_step),
            Step::Look => Some(words.look_step),
            Step::Elsewhere => Some(words.elsewhere_step),
        }
    }

    fn action(self) -> &'static str {
        let words = &words().onboarding;
        match self {
            Step::Welcome => words.start,
            Step::Elsewhere => words.begin_chat,
            _ => words.next,
        }
    }
}

#[derive(Clone)]
enum Note {
    Saved,
    Failed(SharedString),
}

impl SettingsView {
    pub(crate) fn new(home: Home, settings: Settings, cx: &mut Context<Self>) -> Self {
        Self::opened(home, settings, None, cx)
    }

    /// The settings walked one step at a time, from the step the owner had not finished, or
    /// else from the welcome.
    pub(crate) fn onboarding(home: Home, settings: Settings, cx: &mut Context<Self>) -> Self {
        let step = match first_run::read(&home.first_run()) {
            FirstRun::At(step) => step,
            FirstRun::Unbegun | FirstRun::Done => Step::Welcome,
        };
        Self::opened(home, settings, Some(step), cx)
    }

    fn opened(home: Home, settings: Settings, walk: Option<Step>, cx: &mut Context<Self>) -> Self {
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
            let input = cx.new(|cx| TextInput::masked(words().settings.paste, cx));
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
        let sandbox = {
            let home = home.clone();
            cx.new(|cx| SandboxView::new(home, cx))
        };
        let slack = SlackForm::of(settings.slack.as_ref());
        let telegram = TelegramForm::of(settings.telegram.as_ref());
        let telegram_token = secret(cx);
        let discord = DiscordForm::of(settings.discord.as_ref());
        let discord_token = secret(cx);
        let slack_app_token = secret(cx);
        let slack_bot_token = secret(cx);
        let vault = Vault::new(&home.root);
        let kept = vault.names().unwrap_or_default();
        let sites = accounts::sites(&vault).unwrap_or_default();
        let watch_vault = {
            let watched = Arc::new(Vault::new(&home.root));
            cx.spawn(async move |view, cx| {
                let mut seen = None;
                loop {
                    let vault = Arc::clone(&watched);
                    let changes = cx
                        .background_spawn(async move { vault.changes().ok() })
                        .await;
                    let changed = seen.is_some() && changes != seen;
                    seen = changes;
                    if changed {
                        let vault = Arc::clone(&watched);
                        let (kept, sites) = cx
                            .background_spawn(async move {
                                (
                                    vault.names().unwrap_or_default(),
                                    accounts::sites(&vault).unwrap_or_default(),
                                )
                            })
                            .await;
                        let shown = view.update(cx, |view, cx| {
                            view.kept = kept;
                            view.sites = sites;
                            cx.notify();
                        });
                        if shown.is_err() {
                            return;
                        }
                    }
                    cx.background_executor().timer(VAULT_LOOK_EVERY).await;
                }
            })
        };
        let name = field(
            ASSISTANT_NAME,
            settings
                .assistant
                .as_ref()
                .and_then(|assistant| assistant.name.clone()),
            cx,
        );
        let mut view = Self {
            pcs,
            routines,
            notes,
            sandbox,
            home,
            earlier: settings,
            openai_compatible: form.openai_compatible,
            model: field(words().settings.default_value, form.model, cx),
            base_url: field(words().settings.default_value, form.base_url, cx),
            api_key_env: form.api_key_env,
            api_key,
            effort: field(words().settings.default_value, form.effort, cx),
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
            sites,
            _watch_vault: watch_vault,
            name,
            picking: None,
            note: None,
            look_saving: None,
            walk,
            moves: 0,
            slides: HashMap::new(),
            forward: true,
            checking: None,
            check: check_model,
            open_chat: || crate::tray::open("window"),
            join: None,
            joiner: join_in_background,
        };
        view.show_saved_key(cx);
        view
    }

    fn show_saved_key(&mut self, cx: &mut Context<Self>) {
        if self.walk.is_none() || !self.api_key.read(cx).text().is_empty() {
            return;
        }
        let saved = self
            .api_key_name()
            .and_then(|name| self.vault.get(&name).ok().flatten());
        if let Some(saved) = saved {
            self.api_key
                .update(cx, |input, cx| input.set_text(&saved, cx));
        }
    }

    fn choose(&mut self, openai_compatible: bool, cx: &mut Context<Self>) {
        if self.openai_compatible == openai_compatible || self.checking() {
            return;
        }
        self.slid(PROVIDER, usize::from(self.openai_compatible));
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
        self.show_saved_key(cx);
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
        self.note = Some(match self.persist(cx) {
            Ok(()) => Note::Saved,
            Err(error) => Note::Failed(SharedString::from(error)),
        });
        cx.notify();
    }

    fn persist(&mut self, cx: &mut Context<Self>) -> Result<(), String> {
        let model = self.model_form(cx);
        self.persist_with(&model, cx)
    }

    fn model_form(&self, cx: &Context<Self>) -> ModelForm {
        let mut form = self.form(cx);
        let key = self.api_key.read(cx).text().trim().to_owned();
        if form.openai_compatible && form.api_key_env.is_none() && !key.is_empty() {
            form.api_key_env = Some(MODEL_KEY.to_owned());
        }
        ModelForm { form, key }
    }

    fn persist_with(&mut self, model: &ModelForm, cx: &mut Context<Self>) -> Result<(), String> {
        let slack = self.slack_form(cx);
        let telegram = self.telegram_form(cx);
        let discord = self.discord_form(cx);
        let typed_name = self.name.read(cx).text().trim().to_owned();
        let typed_pc = self.pcs.read(cx).typed_name(cx);
        let parts = self.walk.map_or(&ALL_PARTS[..], Step::parts);
        let owns = |part: &str| parts.contains(&part);
        let earlier_model = &self.earlier.model;
        let wanted = Settings::load(&self.home.settings()).and_then(|on_disk| {
            Ok(Settings {
                model: if owns("model") {
                    model.form.clone().settings(earlier_model)?
                } else {
                    on_disk.model.clone()
                },
                slack: if owns("slack") {
                    slack.settings()?
                } else {
                    on_disk.slack.clone()
                },
                telegram: if owns("telegram") {
                    telegram.settings()?
                } else {
                    on_disk.telegram.clone()
                },
                discord: if owns("discord") {
                    discord.settings()?
                } else {
                    on_disk.discord.clone()
                },
                assistant: if owns("assistant") {
                    assistant_named(on_disk.assistant.clone(), &typed_name)?
                } else {
                    on_disk.assistant.clone()
                },
                pc: if owns("pc") {
                    pc_named(&typed_pc)?
                } else {
                    on_disk.pc.clone()
                },
                ..on_disk
            })
        });
        let saved = wanted.and_then(|settings| {
            let typed = self.typed_secrets(model, parts, cx);
            self.tokens_given(&typed, parts)?;
            let before = if owns("model") {
                mem::replace(&mut self.api_key_env, model.form.api_key_env.clone())
            } else {
                self.api_key_env.clone()
            };
            if let Err(error) = self.keep_secrets(&typed, cx) {
                let kept_model_key = self
                    .api_key_name()
                    .is_some_and(|name| self.vault.names().unwrap_or_default().contains(&name));
                if !kept_model_key {
                    self.api_key_env = before;
                }
                return Err(error);
            }
            settings.save_parts(&self.home.settings(), parts)?;
            self.earlier = settings;
            identity::choose(Identity::read(&self.home.settings(), &self.home.icons()));
            Ok(())
        });
        self.kept = self.vault.names().unwrap_or_default();
        self.sites = accounts::sites(&self.vault).unwrap_or_default();
        saved
    }

    fn model_key_missing(&self) -> bool {
        self.api_key_name()
            .is_some_and(|name| environment(&name).is_none() && !self.kept.contains(&name))
    }

    fn leave(
        &mut self,
        step: Step,
        model: Option<&ModelForm>,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if step != Step::Welcome {
            match model {
                Some(model) => self.persist_with(model, cx)?,
                None => self.persist(cx)?,
            }
        }
        if step == Step::Key && self.model_key_missing() {
            return Err(String::from(words().settings.key_needed));
        }
        Ok(())
    }

    fn advance(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(step) = self.walk else {
            return;
        };
        if self.checking.is_some() {
            return;
        }
        if self.join.is_some() {
            self.join_advance(window, cx);
            return;
        }
        if step == Step::Key {
            let model = self.model_form(cx);
            let checked = self
                .model_to_check(&model)
                .map(|(settings, typed)| (self.check)(self.home.clone(), settings, typed));
            match checked {
                Err(error) => self.note = Some(Note::Failed(SharedString::from(error))),
                Ok(checked) => {
                    self.note = None;
                    self.lock_model_fields(true, cx);
                    let anthropic = !model.form.openai_compatible;
                    let moves = self.moves;
                    self.checking = Some(cx.spawn_in(window, async move |view, cx| {
                        let checked = checked.await.unwrap_or(Err(CheckError::NoAnswer));
                        let _ = view.update_in(cx, |view, window, cx| {
                            view.checking = None;
                            view.lock_model_fields(false, cx);
                            if view.walk != Some(step) || view.moves != moves {
                                return;
                            }
                            let passed = checked
                                .map_err(|error| check_failed(&error, anthropic))
                                .and_then(|()| view.leave(step, Some(&model), cx));
                            match passed {
                                Ok(()) => view.step_on(step, window, cx),
                                Err(error) => {
                                    view.note = Some(Note::Failed(SharedString::from(error)));
                                }
                            }
                            cx.notify();
                        });
                    }));
                }
            }
            cx.notify();
            return;
        }
        match self.leave(step, None, cx) {
            Err(error) => self.note = Some(Note::Failed(SharedString::from(error))),
            Ok(()) => self.step_on(step, window, cx),
        }
        cx.notify();
    }

    fn lock_model_fields(&self, locked: bool, cx: &mut Context<Self>) {
        for input in [&self.api_key, &self.base_url, &self.model, &self.effort] {
            input.update(cx, |input, cx| input.set_locked(locked, cx));
        }
    }

    fn model_to_check(
        &self,
        model: &ModelForm,
    ) -> Result<(ModelSettings, Option<(String, String)>), String> {
        let settings = model.form.clone().settings(&self.earlier.model)?;
        if model.key.is_empty() && self.model_key_missing() {
            return Err(String::from(words().settings.key_needed));
        }
        Ok((
            settings,
            model
                .key_name()
                .filter(|_| !model.key.is_empty())
                .map(|name| (name, model.key.clone())),
        ))
    }

    fn checking(&self) -> bool {
        self.checking.is_some()
    }

    fn step_on(&mut self, step: Step, window: &mut Window, cx: &mut Context<Self>) {
        let reached = step.next().map_or(FirstRun::Done, FirstRun::At);
        let path = self.home.first_run();
        let recorded = reached == FirstRun::Done || first_run::read(&path) != FirstRun::Done;
        if recorded && let Err(error) = first_run::write(&path, reached) {
            self.note = Some(Note::Failed(SharedString::from(error)));
            return;
        }
        match step.next() {
            Some(next) => {
                self.walk = Some(next);
                self.moves += 1;
                self.forward = true;
                self.note = None;
                if next == Step::Key {
                    window.focus(&self.api_key.focus_handle(cx));
                }
            }
            None => {
                (self.open_chat)();
                window.remove_window();
            }
        }
    }

    fn go_back(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.checking() {
            return;
        }
        if self.join.is_some() {
            self.leave_join(cx);
            return;
        }
        if let Some(previous) = self.walk.and_then(Step::previous) {
            self.walk = Some(previous);
            self.moves += 1;
            self.forward = false;
            self.note = None;
            cx.notify();
        }
    }
}

struct Secret {
    input: Entity<TextInput>,
    part: &'static str,
    name: String,
    value: String,
}

#[derive(Clone, Debug)]
struct ModelForm {
    form: Form,
    key: String,
}

impl ModelForm {
    fn key_name(&self) -> Option<String> {
        match (&self.form.api_key_env, self.form.openai_compatible) {
            (Some(name), _) => Some(name.clone()),
            (None, false) => Some(ANTHROPIC_KEY.to_owned()),
            (None, true) => None,
        }
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
        if let Some(step) = self.walk {
            return self
                .walk_view(step, &theme, window, cx)
                .on_mouse_down(MouseButton::Left, |_, window, _| window.blur())
                .into_any_element();
        }
        let frame = Frame::of(window);
        let note = self.note.clone().map(|note| {
            let (color, soft, words) = match note {
                Note::Saved => (
                    theme.ok,
                    theme.ok_soft,
                    SharedString::new_static(words().settings.saved),
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
        let spaced = self
            .settings_cards(&theme, window, cx)
            .into_iter()
            .enumerate()
            .flat_map(|(n, card)| [(n > 0).then(|| div().h_4().into_any_element()), Some(card)])
            .flatten();
        let footer = self.settings_footer(&theme, note, cx);
        frame
            .resizable(div())
            .on_mouse_down(MouseButton::Left, |_, window, _| window.blur())
            .flex()
            .flex_col()
            .size_full()
            .font(typeface())
            .bg(theme.canvas)
            .text_color(theme.text)
            .text_sm()
            .line_height(px(21.))
            .child(
                frame.strip(
                    &theme,
                    div()
                        .text_base()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(words().common.settings),
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
                    .children(spaced),
            )
            .child(footer)
            .into_any_element()
    }
}

impl SettingsView {
    fn settings_cards(
        &self,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        vec![
            self.identity_card(theme, window, cx).into_any_element(),
            self.look_card(theme, cx).into_any_element(),
            self.language_card(theme, cx).into_any_element(),
            self.sandbox.clone().into_any_element(),
            self.vault_card(theme, cx).into_any_element(),
            self.model_card(theme, window, cx).into_any_element(),
            self.slack_card(theme, window, cx).into_any_element(),
            self.telegram_card(theme, window, cx).into_any_element(),
            self.discord_card(theme, window, cx).into_any_element(),
            self.notes.clone().into_any_element(),
            self.routines.clone().into_any_element(),
            self.pcs.clone().into_any_element(),
        ]
    }

    fn settings_footer(
        &self,
        theme: &Theme,
        note: Option<AnyElement>,
        cx: &mut Context<Self>,
    ) -> Div {
        let note = note.unwrap_or_else(|| {
            div()
                .text_xs()
                .text_color(theme.muted)
                .child(prose(words().settings.applies_on_restart))
                .into_any_element()
        });
        footer().child(div().flex_1().min_w_0().child(note)).child(
            primary_button(theme, "save", words().settings.save)
                .flex_none()
                .on_click(cx.listener(Self::save)),
        )
    }

    fn model_card(&self, theme: &Theme, window: &Window, cx: &mut Context<Self>) -> Div {
        let provider =
            |id: &'static str, label: &'static str, openai: bool, cx: &mut Context<Self>| {
                segment(
                    theme,
                    id.to_owned(),
                    label,
                    self.openai_compatible == openai,
                )
                .on_click(cx.listener(move |view, _, _, cx| view.choose(openai, cx)))
            };
        let words = &words().settings;
        let mut fields = div()
            .flex()
            .flex_col()
            .gap_4()
            .child(field(
                theme,
                window,
                cx,
                words.model,
                words.model_hint,
                &self.model,
            ))
            .child(field(
                theme,
                window,
                cx,
                words.base_url,
                words.base_url_hint,
                &self.base_url,
            ))
            .child(self.secret_field(
                theme,
                window,
                cx,
                words.api_key,
                &self.api_key,
                self.api_key_name(),
            ));
        if !self.openai_compatible {
            fields = fields.child(field(
                theme,
                window,
                cx,
                words.effort,
                words.effort_hint,
                &self.effort,
            ));
        }
        div()
            .flex()
            .flex_col()
            .gap_4()
            .p_5()
            .rounded_2xl()
            .card(theme)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_base()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(words.model_title),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted)
                            .child((words.model_about)()),
                    ),
            )
            .child(self.track(
                theme,
                PROVIDER,
                usize::from(self.openai_compatible),
                vec![
                    provider("anthropic", "Anthropic", false, cx),
                    provider("openai", words.openai_compatible, true, cx),
                ],
            ))
            .child(fields)
    }
}

fn footer() -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap_3()
        .px_5()
        .py_3()
}

fn primary_button(theme: &Theme, id: &'static str, label: &'static str) -> gpui::Stateful<Div> {
    div()
        .id(id)
        .debug_selector(move || id.to_owned())
        .flex_none()
        .px_5()
        .py_2()
        .rounded_lg()
        .cursor_pointer()
        .main_button(theme)
        .hover(|button| button.opacity(0.9))
        .font_weight(FontWeight::SEMIBOLD)
        .child(label)
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

    fn typed_secrets(&self, model: &ModelForm, parts: &[&str], cx: &Context<Self>) -> Vec<Secret> {
        let [(model_input, _), slack_app, slack_bot, telegram, discord] = self.secrets();
        let model_key = model
            .key_name()
            .filter(|_| parts.contains(&"model"))
            .map(|name| Secret {
                input: model_input.clone(),
                part: "model",
                name,
                value: model.key.clone(),
            });
        let channels = [
            ("slack", self.slack_on, slack_app),
            ("slack", self.slack_on, slack_bot),
            ("telegram", self.telegram_on, telegram),
            ("discord", self.discord_on, discord),
        ]
        .into_iter()
        .filter(|(part, on, _)| *on && parts.contains(part))
        .filter_map(|(part, _, (input, name))| {
            Some(Secret {
                input: input.clone(),
                part,
                name: name?,
                value: input.read(cx).text().trim().to_owned(),
            })
        });
        model_key.into_iter().chain(channels).collect()
    }

    fn tokens_given(&self, typed: &[Secret], parts: &[&str]) -> Result<(), String> {
        let words = &words().settings;
        let given = |part: &str| {
            typed
                .iter()
                .filter(|secret| secret.part == part)
                .all(|secret| {
                    !secret.value.is_empty()
                        || environment(&secret.name).is_some()
                        || self.kept.contains(&secret.name)
                })
        };
        for (part, on, missing) in [
            ("slack", self.slack_on, words.slack_tokens_needed),
            ("telegram", self.telegram_on, words.telegram_token_needed),
            ("discord", self.discord_on, words.discord_token_needed),
        ] {
            if on && parts.contains(&part) && !given(part) {
                return Err(String::from(missing));
            }
        }
        Ok(())
    }

    fn keep_secrets(&mut self, typed: &[Secret], cx: &mut Context<Self>) -> Result<(), String> {
        let typed: Vec<&Secret> = typed
            .iter()
            .filter(|secret| !secret.value.is_empty())
            .collect();
        if let Some(secret) = typed
            .iter()
            .find(|secret| accounts::is_account(&secret.name))
        {
            return Err((words().settings.name_is_an_account)(&secret.name));
        }
        for secret in &typed {
            self.vault
                .set(&secret.name, &secret.value, now_ms())
                .map_err(|error| vault_failed(true, &error))?;
        }
        if self.walk.is_none() {
            for secret in typed {
                secret.input.update(cx, |input, cx| input.set_text("", cx));
            }
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
            .map_err(|error| vault_failed(false, &error))
            .and_then(|()| self.forget_model_key_name(name));
        self.kept = self.vault.names().unwrap_or_default();
        self.note = forgot
            .err()
            .map(|error| Note::Failed(SharedString::from(error)));
        cx.notify();
    }

    fn kept_keys(&self) -> Vec<KeyRow> {
        let slack = self.earlier.slack.as_ref();
        let named = |named: Option<&String>, default: &str| {
            named.cloned().unwrap_or_else(|| default.to_owned())
        };
        let mut slots: Vec<(Option<String>, &'static str, &'static str)> = vec![
            (
                self.api_key_name(),
                if self.openai_compatible {
                    words().settings.api_key
                } else {
                    words().settings.anthropic_key
                },
                words().settings.model_title,
            ),
            (
                Some(named(
                    slack.and_then(|slack| slack.app_token_env.as_ref()),
                    SLACK_APP_KEY,
                )),
                words().settings.slack_app_key,
                "Slack",
            ),
            (
                Some(named(
                    slack.and_then(|slack| slack.bot_token_env.as_ref()),
                    SLACK_BOT_KEY,
                )),
                words().settings.slack_bot_key,
                "Slack",
            ),
            (
                Some(named(
                    self.earlier
                        .telegram
                        .as_ref()
                        .and_then(|telegram| telegram.token_env.as_ref()),
                    TELEGRAM_KEY,
                )),
                words().settings.telegram_key,
                "Telegram",
            ),
            (
                Some(named(
                    self.earlier
                        .discord
                        .as_ref()
                        .and_then(|discord| discord.token_env.as_ref()),
                    DISCORD_KEY,
                )),
                words().settings.discord_key,
                "Discord",
            ),
        ];
        let known: Vec<String> = slots
            .iter()
            .filter_map(|(name, _, _)| name.clone())
            .collect();
        let others: Vec<String> = self
            .kept
            .iter()
            .filter(|name| !accounts::is_account(name) && !known.contains(name))
            .cloned()
            .collect();
        slots.extend(
            others
                .into_iter()
                .map(|name| (Some(name), "", words().settings.other_use)),
        );
        slots
            .into_iter()
            .filter_map(|(name, title, what)| {
                let name = name?;
                let kept = self.kept.contains(&name);
                let from_environment = environment(&name).is_some();
                (kept || from_environment).then(|| KeyRow {
                    title: if title.is_empty() {
                        name.clone()
                    } else {
                        title.to_owned()
                    },
                    what,
                    name,
                    kept,
                    from_environment,
                })
            })
            .collect()
    }

    fn unlink(&mut self, name: &str, cx: &mut Context<Self>) {
        let unlinked =
            accounts::forget(&self.vault, name).map_err(|error| vault_failed(false, &error));
        self.kept = self.vault.names().unwrap_or_default();
        self.sites = accounts::sites(&self.vault).unwrap_or_default();
        self.note = unlinked
            .err()
            .map(|error| Note::Failed(SharedString::from(error)));
        cx.notify();
    }

    fn vault_card(&self, theme: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let action = |id: SharedString, icon: &'static str| {
            let selector = id.to_string();
            div()
                .id(id)
                .debug_selector(move || selector.clone())
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(30.))
                .rounded_md()
                .cursor_pointer()
                .hover(|action| action.bg(theme.bubble))
                .child(svg().path(icon).size(px(16.)).text_color(theme.muted))
        };
        let row = |title: AnyElement, under: AnyElement| {
            div()
                .flex()
                .items_center()
                .gap_3()
                .px_3()
                .py_1p5()
                .child(lock(theme, 16.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(title)
                        .child(under),
                )
        };
        let muted = |words: String| {
            div()
                .text_xs()
                .text_color(theme.muted)
                .child(words)
                .into_any_element()
        };
        let group = |label: &'static str| {
            div()
                .px_3()
                .pt_1()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.muted)
                .child(label)
        };
        let empty = |words: &'static str| {
            div()
                .px_3()
                .py_1p5()
                .text_xs()
                .text_color(theme.muted)
                .child(words)
        };
        let keys = self.kept_keys();
        let key_rows: Vec<AnyElement> = if keys.is_empty() {
            vec![empty(words().settings.no_keys).into_any_element()]
        } else {
            keys.into_iter()
                .map(|key| {
                    let name = key.name.clone();
                    let under = if key.from_environment {
                        (words().settings.read_from_environment)(key.what)
                    } else {
                        key.what.to_owned()
                    };
                    row(
                        div().truncate().child(key.title).into_any_element(),
                        muted(under),
                    )
                    .when(key.kept && !key.from_environment, |row| {
                        row.child(
                            action(
                                SharedString::from(format!("forget-kept-{name}")),
                                TRASH_ICON,
                            )
                            .on_click(cx.listener(
                                move |view, _, _, cx| {
                                    view.forget(&name, cx);
                                },
                            )),
                        )
                    })
                    .into_any_element()
                })
                .collect()
        };
        let site_rows: Vec<AnyElement> = if self.sites.is_empty() {
            vec![empty(words().settings.no_sites).into_any_element()]
        } else {
            self.sites
                .iter()
                .map(|site| {
                    let name = site.name.clone();
                    let title = div()
                        .flex()
                        .min_w_0()
                        .gap_1()
                        .child(div().truncate().child(accounts::host(&site.origin)))
                        .child(
                            div()
                                .flex_none()
                                .text_color(theme.muted)
                                .child(format!("({})", site.site)),
                        )
                        .into_any_element();
                    let under = muted(
                        site.id
                            .clone()
                            .unwrap_or_else(|| String::from(words().settings.no_id)),
                    );
                    row(title, under)
                        .when(site.trusted, |row| {
                            row.child(
                                div()
                                    .flex()
                                    .flex_none()
                                    .items_center()
                                    .gap_1()
                                    .px_2p5()
                                    .py_0p5()
                                    .rounded_full()
                                    .bg(theme.selection)
                                    .text_color(theme.primary)
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(bolt(theme, 12.))
                                    .child(words().settings.trusted),
                            )
                        })
                        .child(
                            action(SharedString::from(format!("unlink-{name}")), TRASH_ICON)
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.unlink(&name, cx);
                                })),
                        )
                        .into_any_element()
                })
                .collect()
        };
        div()
            .flex()
            .flex_col()
            .gap_3()
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
                            .child(words().settings.vault_title),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted)
                            .child(prose(words().settings.vault_about)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .child(group(words().settings.keys_group))
                    .children(key_rows),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .child(group(words().settings.sites_group))
                    .children(site_rows),
            )
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
        let said = &words().settings;
        let (ink, checked, words) = if typed {
            (theme.muted, false, said.key_on_save)
        } else if from_environment {
            (theme.text, true, said.key_from_environment)
        } else if kept {
            (theme.text, true, said.key_kept)
        } else if name.is_none() {
            (theme.muted, false, said.key_not_needed)
        } else {
            (theme.warn, false, said.key_missing)
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
                .child(said.remove)
                .on_click(cx.listener(move |view, _, _, cx| view.forget(&name, cx)))
        });
        let hint = (said.key_hint)(name.as_deref());
        div()
            .flex()
            .flex_col()
            .gap_1p5()
            .child(field(theme, window, cx, label, "", input))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .when(checked, |row| {
                        row.child(
                            svg()
                                .path(CHECK_ICON)
                                .flex_none()
                                .size(px(13.))
                                .text_color(theme.ok),
                        )
                    })
                    .child(div().flex_1().text_xs().text_color(ink).child(words))
                    .children(forget),
            )
            .child(div().text_xs().text_color(theme.muted).child(prose(&hint)))
    }

    fn set_look(&mut self, name: &str, look: Look, cx: &mut Context<Self>) {
        let was = crate::window::theme::saved().1;
        if was != look {
            let looks = [Look::System, Look::Light, Look::Dark];
            self.slid(
                LOOK,
                looks.iter().position(|option| *option == was).unwrap_or(0),
            );
        }
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
            prompt: Some(SharedString::new_static(words().settings.use_picture)),
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
                            words().settings.no_picker,
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
                let words = &words().settings;
                String::from(if error == TOO_LARGE {
                    words.picture_too_large
                } else if error.contains("does not read as an image") {
                    words.picture_unreadable
                } else {
                    words.picture_not_saved
                })
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
        let words = &words().settings;
        div()
            .flex()
            .flex_col()
            .gap_4()
            .p_5()
            .rounded_2xl()
            .card(theme)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_base()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(words.identity_title),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted)
                            .child(words.identity_about),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_4()
                    .child(logo(theme, PICTURE))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_2()
                            .child(
                                button("icon-pick", words.choose_picture)
                                    .on_click(cx.listener(|view, _, _, cx| view.pick_icon(cx))),
                            )
                            .when(custom, |row| {
                                row.child(button("icon-default", words.default_picture).on_click(
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
                words.name,
                words.name_hint,
                &self.name,
            ))
    }

    fn look_card(&self, theme: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        div()
            .flex()
            .flex_col()
            .gap_4()
            .p_5()
            .rounded_2xl()
            .card(theme)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_base()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(words().settings.theme),
                    )
                    .child(div().text_xs().text_color(theme.muted).child(
                        if crate::window::theme::overridden() {
                            words().settings.theme_overridden
                        } else {
                            words().settings.theme_about
                        },
                    )),
            )
            .child(self.theme_list(theme, cx))
            .child(self.look_switch(theme, cx))
    }

    fn theme_list(&self, theme: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let (index, look) = crate::window::theme::saved();
        let current = crate::window::theme::names().nth(index).unwrap_or_default();
        let themes = crate::window::theme::names().map(|name| {
            let chosen = name == current;
            let (light, dark) = crate::window::theme::named(name).unwrap_or((*theme, *theme));
            let swatch = |color| {
                div()
                    .size(px(10.))
                    .rounded_full()
                    .set_apart(theme)
                    .bg(color)
            };
            div()
                .id(SharedString::from(format!("theme-{name}")))
                .debug_selector(move || format!("theme-{name}"))
                .flex()
                .items_center()
                .gap_2p5()
                .h(px(34.))
                .px_3p5()
                .rounded(px(10.))
                .cursor_pointer()
                .hover(|row| row.bg(theme.bubble))
                .when(chosen, |row| row.font_weight(FontWeight::SEMIBOLD))
                .child(div().flex_1().child(theme_label(name)))
                .child(
                    div()
                        .flex()
                        .gap(px(3.))
                        .child(swatch(light.canvas))
                        .child(swatch(light.primary))
                        .child(swatch(dark.canvas))
                        .child(swatch(dark.primary)),
                )
                .child(div().w(px(16.)).flex().justify_end().when(chosen, |mark| {
                    mark.child(
                        svg()
                            .path(CHECK_ICON)
                            .size(px(14.))
                            .text_color(theme.primary),
                    )
                }))
                .on_click(cx.listener(move |view, _, _, cx| view.set_look(name, look, cx)))
        });
        div().flex().flex_col().children(themes)
    }

    fn look_switch(&self, theme: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let (index, look) = crate::window::theme::saved();
        let current = crate::window::theme::names().nth(index).unwrap_or_default();
        let looks = [Look::System, Look::Light, Look::Dark].map(|option| {
            let chosen = option == look;
            let common = &words().common;
            let label = match option {
                Look::System => common.follow_system,
                Look::Light => common.light,
                Look::Dark => common.dark,
            };
            segment(theme, format!("look-{}", option.name()), label, chosen)
                .on_click(cx.listener(move |view, _, _, cx| view.set_look(current, option, cx)))
        });
        let at = [Look::System, Look::Light, Look::Dark]
            .iter()
            .position(|option| *option == look)
            .unwrap_or(0);
        self.track(theme, LOOK, at, looks.into_iter().collect())
    }

    fn flipped(&self, switch: &'static str) -> Option<usize> {
        self.slides.get(switch).map(|(_, moved)| *moved)
    }

    fn slid(&mut self, switch: &'static str, from: usize) {
        let (_, moved) = self.slides.get(switch).copied().unwrap_or_default();
        self.slides.insert(switch, (from, moved + 1));
    }

    fn track(
        &self,
        theme: &Theme,
        switch: &'static str,
        chosen: usize,
        options: Vec<gpui::Stateful<Div>>,
    ) -> Div {
        let count = options.len().max(1) as f32;
        let to = chosen as f32;
        let thumb = div()
            .absolute()
            .top_0()
            .bottom_0()
            .w(relative(1. / count))
            .left(relative(to / count))
            .rounded(px(8.))
            .raised(theme);
        let thumb = match self.slides.get(switch).copied() {
            Some((from, moved)) => {
                let from = from as f32;
                thumb
                    .with_animation((switch, moved), SWITCH.animation(), move |thumb, t| {
                        thumb.left(relative((from + (to - from) * t) / count))
                    })
                    .into_any_element()
            }
            None => thumb.into_any_element(),
        };
        div()
            .p(px(3.))
            .rounded(px(11.))
            .bg(theme.bubble)
            .child(div().relative().flex().child(thumb).children(options))
    }

    fn language_card(&self, theme: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let chosen = language::chosen();
        let options = Choice::ALL.map(|option| {
            let label = match option {
                Choice::System => words().common.follow_system,
                Choice::Only(language) => language.name(),
            };
            segment(
                theme,
                format!("language-{}", option.code()),
                label,
                option == chosen,
            )
            .on_click(cx.listener(move |view, _, _, cx| view.set_language(option, cx)))
        });
        div()
            .flex()
            .flex_col()
            .gap_4()
            .p_5()
            .rounded_2xl()
            .card(theme)
            .child(
                div()
                    .text_base()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(words().common.language),
            )
            .child(
                self.track(
                    theme,
                    LANGUAGE,
                    Choice::ALL
                        .iter()
                        .position(|option| *option == chosen)
                        .unwrap_or(0),
                    options.into_iter().collect(),
                ),
            )
    }

    fn set_language(&mut self, choice: Choice, cx: &mut Context<Self>) {
        let was = language::chosen();
        if was != choice {
            self.slid(
                LANGUAGE,
                Choice::ALL
                    .iter()
                    .position(|option| *option == was)
                    .unwrap_or(0),
            );
        }
        language::choose(choice);
        let path = self.home.settings();
        let earlier = self.look_saving.take();
        self.look_saving = Some(cx.spawn(async move |view, cx| {
            if let Some(earlier) = earlier {
                earlier.await;
            }
            let saved = cx
                .background_spawn(async move {
                    Settings::load(&path).and_then(|mut settings| {
                        settings.language = Some(choice.code().to_owned());
                        settings.save_parts(&path, &["language"])
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

    fn slack_card(
        &self,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let on = self.slack_on;
        let words = &words().settings;
        let card = channel_card(
            theme,
            "Slack",
            (words.slack_about)(),
            switch(theme, "slack-switch", on, self.flipped("slack-switch")).on_click(cx.listener(
                |view, _, _, cx| {
                    view.slid("slack-switch", 0);
                    view.slack_on = !view.slack_on;
                    view.note = None;
                    cx.notify();
                },
            )),
        );
        if !on {
            return card;
        }
        let [_, (_, app_name), (_, bot_name), _, _] = self.secrets();
        let app_token = self.secret_field(
            theme,
            window,
            cx,
            words.app_token,
            &self.slack_app_token,
            app_name,
        );
        let bot_token = self.secret_field(
            theme,
            window,
            cx,
            words.bot_token,
            &self.slack_bot_token,
            bot_name,
        );
        card.child(guide(theme, &words.slack_steps))
            .child(field(
                theme,
                window,
                cx,
                words.slack_owner,
                words.slack_owner_hint,
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
        let words = &words().settings;
        let card = channel_card(
            theme,
            "Telegram",
            words.telegram_about,
            switch(
                theme,
                "telegram-switch",
                on,
                self.flipped("telegram-switch"),
            )
            .on_click(cx.listener(|view, _, _, cx| {
                view.slid("telegram-switch", 0);
                view.telegram_on = !view.telegram_on;
                view.note = None;
                cx.notify();
            })),
        );
        if !on {
            return card;
        }
        let [_, _, _, (_, name), _] = self.secrets();
        let token = self.secret_field(
            theme,
            window,
            cx,
            words.bot_token,
            &self.telegram_token,
            name,
        );
        card.child(guide(theme, &words.telegram_steps))
            .child(field(
                theme,
                window,
                cx,
                words.telegram_owner,
                words.telegram_owner_hint,
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
        let words = &words().settings;
        let card = channel_card(
            theme,
            "Discord",
            words.discord_about,
            switch(theme, "discord-switch", on, self.flipped("discord-switch")).on_click(
                cx.listener(|view, _, _, cx| {
                    view.slid("discord-switch", 0);
                    view.discord_on = !view.discord_on;
                    view.note = None;
                    cx.notify();
                }),
            ),
        );
        if !on {
            return card;
        }
        let [_, _, _, _, (_, name)] = self.secrets();
        let token = self.secret_field(
            theme,
            window,
            cx,
            words.bot_token,
            &self.discord_token,
            name,
        );
        card.child(guide(theme, &words.discord_steps))
            .child(field(
                theme,
                window,
                cx,
                words.discord_owner,
                words.discord_owner_hint,
                &self.discord_owner,
            ))
            .child(token)
    }
}

struct KeyRow {
    title: String,
    what: &'static str,
    name: String,
    kept: bool,
    from_environment: bool,
}

fn vault_failed(keeping: bool, error: &str) -> String {
    eprintln!("dotori: the vault: {error}");
    let words = &words().settings;
    String::from(match (error.contains("another key"), keeping) {
        (true, true) => words.vault_mismatch_keep,
        (true, false) => words.vault_mismatch_remove,
        (false, true) => words.vault_failed_keep,
        (false, false) => words.vault_failed_remove,
    })
}

const VAULT_LOOK_EVERY: Duration = Duration::from_secs(3);
const TOO_LARGE: &str = "the picture is larger than 20 MB";

fn a_name(typed: &str, longest: usize) -> Result<(), String> {
    if typed.chars().count() > longest {
        return Err((words().settings.name_too_long)(longest));
    }
    if typed.chars().any(char::is_control) {
        return Err(String::from(words().settings.name_control));
    }
    Ok(())
}

fn pc_named(typed: &str) -> Result<Option<PcSettings>, String> {
    a_name(typed, PC_NAME_LONGEST)?;
    Ok((!typed.is_empty()).then(|| PcSettings {
        name: Some(typed.to_owned()),
    }))
}

fn assistant_named(
    assistant: Option<AssistantSettings>,
    typed: &str,
) -> Result<Option<AssistantSettings>, String> {
    a_name(typed, ASSISTANT_NAME_LONGEST)?;
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
        .card(theme)
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

fn switch(
    theme: &Theme,
    id: &'static str,
    on: bool,
    flipped: Option<usize>,
) -> gpui::Stateful<gpui::Div> {
    let to = if on { 1. } else { 0. };
    let (lit, surface) = (theme.primary, theme.surface);
    let drawn = move |inner: Div, at: f32| {
        inner
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .rounded_full()
                    .bg(lit)
                    .opacity(at),
            )
            .child(
                div()
                    .absolute()
                    .top(px(2.))
                    .left(px(2. + KNOB_TRAVEL * at))
                    .size(px(18.))
                    .rounded_full()
                    .bg(surface)
                    .shadow_sm(),
            )
    };
    let inner = div().relative().size_full();
    let inner = match flipped {
        Some(moved) => inner
            .with_animation(("flip", moved), SWITCH.animation(), move |inner, t| {
                drawn(inner, (1. - to) + (2. * to - 1.) * t)
            })
            .into_any_element(),
        None => drawn(inner, to).into_any_element(),
    };
    div()
        .id(id)
        .debug_selector(move || id.to_owned())
        .flex_none()
        .w(px(40.))
        .h(px(22.))
        .rounded_full()
        .cursor_pointer()
        .bg(theme.line)
        .child(inner)
}

const KNOB_TRAVEL: f32 = 18.;

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
    let ring =
        (focus.is_focused(window) && window.is_window_active()).then(|| theme.primary.into());
    let width = hairline(window);
    div()
        .flex()
        .flex_col()
        .gap_1p5()
        .child(div().font_weight(FontWeight::MEDIUM).child(label))
        .child(framed(
            ring,
            FIELD_RADIUS - width,
            width,
            div()
                .id(label)
                .px_3()
                .py_2()
                .rounded(px(FIELD_RADIUS - width))
                .card(theme)
                .cursor_text()
                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    window.focus(&focus);
                    cx.stop_propagation();
                })
                .child(input.clone()),
        ))
        .when(!hint.is_empty(), |field| {
            field.child(div().text_xs().text_color(theme.muted).child(hint))
        })
}

type Check =
    fn(Home, ModelSettings, Option<(String, String)>) -> oneshot::Receiver<Result<(), CheckError>>;

fn check_model(
    home: Home,
    model: ModelSettings,
    typed: Option<(String, String)>,
) -> oneshot::Receiver<Result<(), CheckError>> {
    let (tell, told) = oneshot::channel();
    thread::spawn(move || {
        let unbuilt = |error: String| {
            eprintln!("dotori: the model to check could not be made: {error}");
            CheckError::NoAnswer
        };
        let checked = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| unbuilt(error.to_string()))
            .and_then(|runtime| {
                let vault = Vault::new(&home.root);
                let saved = vault::lookup(&vault);
                let key = |name: &str| match &typed {
                    Some((typed_name, value)) if typed_name == name => Some(value.clone()),
                    _ => saved(name),
                };
                let model = Settings {
                    model,
                    ..Settings::default()
                }
                .model(key)
                .map_err(|error| unbuilt(error.to_string()))?;
                runtime.block_on(model.check())
            });
        let _ = tell.send(checked);
    });
    told
}

fn check_failed(error: &CheckError, anthropic: bool) -> String {
    let words = &words().onboarding;
    match error {
        CheckError::KeyRefused => words.key_refused.to_owned(),
        CheckError::NoAnswer if anthropic => words.no_anthropic.to_owned(),
        CheckError::NoAnswer => words.no_server.to_owned(),
    }
}

const PROVIDER: &str = "provider";
const LOOK: &str = "look";
const LANGUAGE: &str = "language";

fn segment(theme: &Theme, id: String, label: &'static str, chosen: bool) -> gpui::Stateful<Div> {
    div()
        .id(SharedString::from(id.clone()))
        .debug_selector(move || id.clone())
        .flex_1()
        .py_1()
        .rounded(px(8.))
        .flex()
        .justify_center()
        .text_size(px(13.))
        .cursor_pointer()
        .when(chosen, |segment| {
            segment
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.text)
        })
        .when(!chosen, |segment| segment.text_color(theme.muted))
        .child(label)
}

fn theme_label(name: &str) -> &'static str {
    (words().common.theme)(name)
}

const FIELD_RADIUS: f32 = 14.;

#[path = "onboarding.rs"]
mod onboarding;

#[cfg(test)]
#[path = "settings.test.rs"]
mod tests;
