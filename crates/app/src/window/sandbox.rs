//! The settings window's card for the sandbox the shell commands of dotori's model run in. macOS
//! has it on from the start. Linux turns it on once bubblewrap runs, and the card can set that
//! up with the admin's OK. Windows has no sandbox and shows no card.

use std::time::Duration;

use dotori_connectors::sandbox::Blocked;
use dotori_connectors::sandbox::Sandbox;
use dotori_connectors::tool::basic::Private;
use gpui::ClipboardItem;
use gpui::Context;
use gpui::FontWeight;
use gpui::SharedString;
use gpui::Task;
use gpui::Window;
use gpui::div;
use gpui::prelude::*;
use gpui::px;
use gpui::svg;

use crate::home::Home;
use crate::sandbox;
use crate::sandbox::Failure;
use crate::window::parts::MONO;
use crate::window::parts::SHIELD_ICON;
use crate::window::parts::SHIELD_ON_ICON;
use crate::window::parts::prose;
use crate::window::theme::Theme;

const REFRESH: Duration = Duration::from_secs(5);

/// Where the sandbox stands on this PC, as the card shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum State {
    Checking,
    On,
    Off,
    Blocked(Blocked),
    Installing,
    Failed(Failure),
}

/// How the card finds out whether commands are sandboxed: yes, no and they run as they are, or
/// they are refused. It also sets the sandbox up. Tests stand in for both.
#[derive(Clone, Copy)]
pub(crate) struct Work {
    pub(crate) works: fn(&Home) -> Result<bool, Blocked>,
    pub(crate) set_up: fn() -> Result<(), Failure>,
}

impl State {
    fn waits_for_the_sandbox(self) -> bool {
        !matches!(self, Self::On | Self::Installing)
    }

    fn after(self, found: Result<bool, Blocked>) -> Self {
        match (found, self) {
            (Ok(true), _) => Self::On,
            (Err(blocked), _) => Self::Blocked(blocked),
            (Ok(false), Self::Failed(failure)) => Self::Failed(failure),
            (Ok(false), _) if cfg!(target_os = "macos") => Self::Blocked(Blocked::Failed),
            (Ok(false), _) => Self::Off,
        }
    }
}

impl Work {
    fn real() -> Self {
        Self {
            works,
            set_up: sandbox::set_up,
        }
    }
}

pub(crate) struct SandboxView {
    home: Home,
    work: Work,
    state: State,
    admin: bool,
    copied: bool,
    _refresh: Task<()>,
}

impl SandboxView {
    pub(crate) fn new(home: Home, cx: &mut Context<Self>) -> Self {
        Self::with(home, Work::real(), cx)
    }

    pub(crate) fn with(home: Home, work: Work, cx: &mut Context<Self>) -> Self {
        let refresh = cx.spawn(async move |view, cx| {
            if cfg!(windows) {
                return;
            }
            loop {
                let Ok(home) = view.update(cx, |view, _| {
                    view.state
                        .waits_for_the_sandbox()
                        .then(|| view.home.clone())
                }) else {
                    return;
                };
                if let Some(home) = home {
                    let found = cx
                        .background_executor()
                        .spawn(async move { (work.works)(&home) })
                        .await;
                    let shown = view.update(cx, |view, cx| {
                        if view.state.waits_for_the_sandbox() {
                            view.state = view.state.after(found);
                            cx.notify();
                        }
                    });
                    if shown.is_err() {
                        return;
                    }
                }
                cx.background_executor().timer(REFRESH).await;
            }
        });
        Self {
            home,
            work,
            state: State::Checking,
            admin: sandbox::needs_admin(),
            copied: false,
            _refresh: refresh,
        }
    }

    fn install(&mut self, cx: &mut Context<Self>) {
        if self.state == State::Installing {
            return;
        }
        self.state = State::Installing;
        self.copied = false;
        cx.notify();
        let (work, home) = (self.work, self.home.clone());
        cx.spawn(async move |view, cx| {
            let ended = cx
                .background_executor()
                .spawn(async move {
                    (work.set_up)()?;
                    match (work.works)(&home) {
                        Ok(true) => Ok(()),
                        _ => Err(Failure::StillOff),
                    }
                })
                .await;
            let _ = view.update(cx, |view, cx| {
                view.state = match ended {
                    Ok(()) => State::On,
                    Err(failure) => State::Failed(failure),
                };
                cx.notify();
            });
        })
        .detach();
    }

    fn copy(&mut self, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(sandbox::by_hand()));
        self.copied = true;
        cx.notify();
    }

    fn shown(&self) -> bool {
        !cfg!(windows)
    }
}

fn works(home: &Home) -> Result<bool, Blocked> {
    let private = Private {
        folder: home.root.clone(),
        open: home.work(),
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| Blocked::Failed)?;
    runtime
        .block_on(Sandbox::new(&private).ready())
        .map(|found| found.is_some())
}

impl Render for SandboxView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(window);
        if !self.shown() {
            return div();
        }
        let mut card = div()
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
                            .child("안전하게 실행"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted)
                            .child("AI가 실행하는 명령을 어디까지 볼 수 있게 할지예요."),
                    ),
            );
        match self.state {
            State::Checking => {
                return div().pb_4().child(
                    card.child(
                        div()
                            .text_xs()
                            .text_color(theme.muted)
                            .child("샌드박스가 도는지 확인하는 중이에요."),
                    ),
                );
            }
            State::On => return div().pb_4().child(card.child(on_box(&theme))),
            State::Blocked(blocked) => {
                return div().pb_4().child(card.child(blocked_box(&theme, blocked)));
            }
            State::Off | State::Installing | State::Failed(_) => {}
        }
        card = card.child(
            div()
                .flex()
                .items_start()
                .gap_3()
                .child(
                    svg()
                        .path(SHIELD_ICON)
                        .flex_none()
                        .mt(px(1.))
                        .size(px(20.))
                        .text_color(theme.muted),
                )
                .child(div().flex_1().min_w_0().child(prose(
                    "샌드박스 환경을 설치하면 AI가 실행하는 명령이 dotori의 금고와 대화 기록을 볼 수 없어서 더 안전하게 쓸 수 있어요.",
                ))),
        );
        card = card.child(match self.state {
            State::Failed(failure) => self.failed_box(&theme, failure, cx),
            State::Installing => action_row(
                &theme,
                self.admin.then_some("관리자 확인 창에서 허락해 주세요."),
                div()
                    .px_4()
                    .py_1p5()
                    .rounded_lg()
                    .bg(theme.bubble)
                    .text_color(theme.muted)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("설치하는 중이에요"),
            ),
            _ => action_row(
                &theme,
                self.admin.then_some("설치할 때 관리자 확인이 한 번 떠요."),
                button(&theme, "sandbox-install", "샌드박스 환경 설치", true)
                    .on_click(cx.listener(|view, _, _, cx| view.install(cx))),
            ),
        });
        div().pb_4().child(card)
    }
}

impl SandboxView {
    fn failed_box(&self, theme: &Theme, failure: Failure, cx: &mut Context<Self>) -> gpui::Div {
        div()
            .flex()
            .flex_col()
            .gap_2()
            .px_4()
            .py_3()
            .rounded_xl()
            .bg(theme.danger_soft)
            .child(
                div()
                    .text_color(theme.danger)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("설치하지 못했어요"),
            )
            .child(div().text_xs().child(prose(&format!(
                "{} 다시 해 보거나, 터미널에서 아래 명령을 직접 실행해 주세요.",
                failure.sentence()
            ))))
            .child(
                div()
                    .px_3()
                    .py_2()
                    .rounded_lg()
                    .bg(theme.surface)
                    .font_family(MONO)
                    .text_size(px(12.5))
                    .line_height(px(19.))
                    .child(SharedString::from(sandbox::by_hand())),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        button(
                            theme,
                            "sandbox-copy",
                            if self.copied {
                                "복사했어요"
                            } else {
                                "명령 복사"
                            },
                            false,
                        )
                        .on_click(cx.listener(|view, _, _, cx| view.copy(cx))),
                    )
                    .child(
                        button(theme, "sandbox-retry", "다시 해 보기", true)
                            .on_click(cx.listener(|view, _, _, cx| view.install(cx))),
                    ),
            )
    }
}

fn blocked_box(theme: &Theme, blocked: Blocked) -> gpui::Div {
    let why = match blocked {
        Blocked::WorkNotPlain => {
            "dotori 폴더 안의 work 폴더가 링크로 바뀌어 있어서, AI가 실행하는 명령을 막아 두었어요. work를 보통 폴더로 되돌리면 다시 실행해요."
        }
        Blocked::Failed => {
            "샌드박스가 이 PC에서 돌지 않아서, AI가 실행하는 명령을 막아 두었어요. dotori를 다시 시작해도 그대로면 알려 주세요."
        }
    };
    div()
        .flex()
        .items_start()
        .gap_3()
        .px_4()
        .py_3()
        .rounded_xl()
        .bg(theme.danger_soft)
        .child(
            svg()
                .path(SHIELD_ICON)
                .flex_none()
                .mt(px(1.))
                .size(px(20.))
                .text_color(theme.danger),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap_0p5()
                .child(
                    div()
                        .text_color(theme.danger)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("명령을 실행하지 않고 있어요"),
                )
                .child(div().text_xs().text_color(theme.text).child(prose(why))),
        )
}

fn on_box(theme: &Theme) -> gpui::Div {
    div()
        .flex()
        .items_start()
        .gap_3()
        .px_4()
        .py_3()
        .rounded_xl()
        .bg(theme.ok_soft)
        .child(
            svg()
                .path(SHIELD_ON_ICON)
                .flex_none()
                .mt(px(1.))
                .size(px(20.))
                .text_color(theme.ok),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap_0p5()
                .child(
                    div()
                        .text_color(theme.ok)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("샌드박스가 켜져 있어요"),
                )
                .child(div().text_xs().text_color(theme.text).child(prose(
                    "AI가 실행하는 명령은 dotori의 금고, 대화 기록, 브라우저 로그인을 볼 수 없어요. 그 밖의 폴더와 인터넷은 그대로 써요.",
                ))),
        )
}

fn action_row(theme: &Theme, note: Option<&'static str>, action: impl IntoElement) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap_3()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_xs()
                .text_color(theme.muted)
                .children(note.map(prose)),
        )
        .child(action)
}

fn button(
    theme: &Theme,
    id: &'static str,
    label: &'static str,
    next: bool,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .debug_selector(move || id.to_owned())
        .flex_none()
        .px_4()
        .py_1p5()
        .rounded_lg()
        .cursor_pointer()
        .when(next, |button| {
            button
                .bg(theme.primary)
                .text_color(theme.on_primary)
                .font_weight(FontWeight::SEMIBOLD)
                .hover(|button| button.bg(theme.primary_hover))
        })
        .when(!next, |button| {
            button
                .bg(theme.bubble)
                .text_color(theme.text)
                .font_weight(FontWeight::MEDIUM)
                .hover(|button| button.bg(theme.selection))
        })
        .child(label)
}

#[cfg(test)]
#[path = "sandbox.test.rs"]
mod tests;
