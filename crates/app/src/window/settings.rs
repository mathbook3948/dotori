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
use gpui::Window;
use gpui::div;
use gpui::prelude::*;
use gpui::px;

use crate::home::Home;
use crate::settings::Form;
use crate::settings::SLACK_APP_KEY;
use crate::settings::SLACK_BOT_KEY;
use crate::settings::Settings;
use crate::settings::SlackForm;
use crate::settings::environment;
use crate::window::input::TextInput;
use crate::window::notes::NotesView;
use crate::window::parts::header;
use crate::window::parts::prose;
use crate::window::pcs::PcsView;
use crate::window::routines::RoutinesView;
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
    note: Option<Note>,
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
            api_key_env: field("기본값", form.api_key_env, cx),
            effort: field("기본값", form.effort, cx),
            other,
            slack_on: slack.on,
            slack_owner: field("U012AB3CD", slack.owner, cx),
            slack_app_env,
            slack_bot_env,
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

    fn save(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        let form = self.form(cx);
        let slack = self.slack_form(cx);
        let saved = form.settings(&self.earlier.model).and_then(|model| {
            let slack = slack.settings()?;
            let settings = Settings {
                model,
                slack,
                ..Settings::load(&self.home.settings())?
            };
            settings.save_parts(&self.home.settings(), &["model", "slack"])?;
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
            .child(field(
                &theme,
                window,
                cx,
                "API 키가 든 환경 변수",
                "키는 파일에 저장하지 않아요. 키가 든 환경 변수 이름만 적어요.",
                &self.api_key_env,
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
                            .bg(theme.accent)
                            .hover(|save| save.bg(theme.accent_hover))
                            .text_color(theme.accent_text)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("저장")
                            .on_click(cx.listener(Self::save)),
                    ),
            )
    }
}

impl SettingsView {
    fn slack_card(
        &self,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let on = self.slack_on;
        let switch = div()
            .id("slack-switch")
            .flex()
            .flex_none()
            .items_center()
            .w(px(40.))
            .h(px(22.))
            .p(px(2.))
            .rounded_full()
            .cursor_pointer()
            .bg(if on { theme.accent } else { theme.line })
            .when(on, |switch| switch.justify_end())
            .child(
                div()
                    .size(px(18.))
                    .rounded_full()
                    .bg(theme.surface)
                    .shadow_sm(),
            )
            .on_click(cx.listener(|view, _, _, cx| {
                view.slack_on = !view.slack_on;
                view.note = None;
                cx.notify();
            }));
        let head = div()
            .flex()
            .items_center()
            .gap_3()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .gap_1()
                    .child(
                        div()
                            .text_base()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Slack"),
                    )
                    .child(div().text_xs().text_color(theme.muted).child(
                        "Slack에서도 dotori와 말할 수 있어요. DM이나 멘션으로 부르면 같은 대화로 이어져요.",
                    )),
            )
            .child(switch);
        let mut card = div()
            .flex()
            .flex_col()
            .gap_4()
            .p_5()
            .rounded_2xl()
            .bg(theme.surface)
            .border_1()
            .border_color(theme.line)
            .shadow_xs()
            .child(head);
        if !on {
            return card;
        }
        let step = |n: &'static str, words: &'static str| {
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
                        .text_color(theme.accent)
                        .text_size(px(11.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(n),
                )
                .child(div().flex_1().min_w_0().child(prose(words)))
        };
        let guide = div()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded_xl()
            .bg(theme.canvas)
            .text_xs()
            .text_color(theme.muted)
            .child(step(
                "1",
                "api.slack.com/apps에서 앱을 만들고 Socket Mode를 켜요. connections:write 권한으로 앱 토큰(xapp-)을 만들어요.",
            ))
            .child(step(
                "2",
                "봇 권한 app_mentions:read, chat:write, im:history, im:read, im:write, users:read, channels:read, groups:read를 주고, app_mention과 message.im 이벤트를 구독한 뒤 설치해서 봇 토큰(xoxb-)을 받아요.",
            ))
            .child(step(
                "3",
                "두 토큰을 아래 이름의 환경 변수에 넣고 dotori를 다시 시작해요. 토큰은 파일에 저장하지 않아요.",
            ));
        let found = |input: &Entity<TextInput>, default: &str, cx: &Context<Self>| {
            let typed = input.read(cx).text().trim().to_owned();
            let name = if typed.is_empty() {
                default.to_owned()
            } else {
                typed
            };
            let there = environment(&name).is_some();
            let (ink, soft, words) = if there {
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
        };
        let app_found = found(&self.slack_app_env, SLACK_APP_KEY, cx);
        let bot_found = found(&self.slack_bot_env, SLACK_BOT_KEY, cx);
        card = card
            .child(guide)
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
            );
        card
    }
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
                .border_color(if focused { theme.accent } else { theme.line })
                .cursor_text()
                .on_mouse_down(MouseButton::Left, move |_, window, _| window.focus(&focus))
                .child(input.clone()),
        )
        .child(div().text_xs().text_color(theme.muted).child(hint))
}
