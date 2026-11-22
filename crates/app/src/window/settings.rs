use std::mem;

use gpui::App;
use gpui::ClickEvent;
use gpui::Context;
use gpui::Entity;
use gpui::FocusHandle;
use gpui::Focusable;
use gpui::SharedString;
use gpui::Window;
use gpui::div;
use gpui::prelude::*;
use gpui::rgb;

use crate::home::Home;
use crate::settings::Form;
use crate::settings::ModelSettings;
use crate::settings::Settings;
use crate::window::input::TextInput;

/// The settings window. It writes `settings.toml`, which the daemon reads when it starts.
pub(crate) struct SettingsView {
    home: Home,
    earlier: ModelSettings,
    openai_compatible: bool,
    model: Entity<TextInput>,
    base_url: Entity<TextInput>,
    api_key_env: Entity<TextInput>,
    effort: Entity<TextInput>,
    other: Form,
    note: Option<SharedString>,
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
        let other = Form {
            openai_compatible: !form.openai_compatible,
            ..Form::default()
        };
        Self {
            home,
            earlier: settings.model,
            openai_compatible: form.openai_compatible,
            model: field("기본값 사용", form.model, cx),
            base_url: field("기본값 사용", form.base_url, cx),
            api_key_env: field("기본값 사용", form.api_key_env, cx),
            effort: field("기본값 사용 (low, medium, high)", form.effort, cx),
            other,
            note: None,
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

    fn save(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let form = self.form(cx);
        let saved = form.settings(&self.earlier).and_then(|model| {
            Settings {
                model: model.clone(),
            }
            .save(&self.home.settings())?;
            self.earlier = model;
            Ok(())
        });
        self.note = Some(SharedString::from(match saved {
            Ok(()) => String::from("저장했습니다. dotori를 다시 시작하면 적용됩니다."),
            Err(error) => error,
        }));
        cx.notify();
    }
}

impl Focusable for SettingsView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.model.focus_handle(cx)
    }
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tab = |id: &'static str,
                   label: &'static str,
                   openai_compatible: bool,
                   cx: &mut Context<Self>| {
            let chosen = self.openai_compatible == openai_compatible;
            div()
                .id(id)
                .px_3()
                .py_1()
                .rounded_md()
                .cursor_pointer()
                .bg(if chosen { rgb(0x8a5a1c) } else { rgb(0xece6dd) })
                .text_color(if chosen { rgb(0xffffff) } else { rgb(0x2b2622) })
                .child(label)
                .on_click(cx.listener(move |view, _, _, cx| view.choose(openai_compatible, cx)))
        };
        let row = |label: &'static str, field: &Entity<TextInput>| {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_sm().child(label))
                .child(field.clone())
        };
        let mut form = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(row("모델", &self.model))
            .child(row("주소", &self.base_url))
            .child(row("API 키가 든 환경 변수", &self.api_key_env));
        if !self.openai_compatible {
            form = form.child(row("생각하는 정도", &self.effort));
        }
        div()
            .flex()
            .flex_col()
            .gap_4()
            .p_4()
            .size_full()
            .bg(rgb(0xfaf8f5))
            .text_color(rgb(0x2b2622))
            .child(div().text_xl().child("dotori 설정"))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(tab("anthropic", "Anthropic", false, cx))
                    .child(tab("openai", "OpenAI 호환", true, cx)),
            )
            .child(form)
            .child(
                div()
                    .id("save")
                    .px_4()
                    .py_2()
                    .rounded_md()
                    .cursor_pointer()
                    .bg(rgb(0x8a5a1c))
                    .text_color(rgb(0xffffff))
                    .child("저장")
                    .on_click(cx.listener(Self::save)),
            )
            .children(self.note.clone().map(|note| div().text_sm().child(note)))
    }
}
