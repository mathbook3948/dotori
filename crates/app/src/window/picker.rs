//! The pickers of the model: a field that opens a list of choices below it, for the model and
//! how hard it thinks. A Claude API key offers the Claude models; an OpenAI compatible server
//! offers the models it lists.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::thread;

use dotori_connectors::llm::anthropic::Anthropic;
use dotori_connectors::llm::anthropic::AnthropicConfig;
use dotori_connectors::llm::anthropic::DEFAULT_MODEL;
use dotori_connectors::llm::openai::OpenAi;
use dotori_connectors::llm::openai::OpenAiConfig;
use dotori_core::llm::CheckError;
use gpui::App;
use gpui::Bounds;
use gpui::Corner;
use gpui::Div;
use gpui::Entity;
use gpui::Focusable;
use gpui::FontWeight;
use gpui::MouseDownEvent;
use gpui::Pixels;
use gpui::SharedString;
use gpui::Window;
use gpui::anchored;
use gpui::canvas;
use gpui::deferred;
use gpui::div;
use gpui::point;
use gpui::prelude::*;
use gpui::px;
use gpui::svg;
use tokio::sync::oneshot;

use super::SettingsView;
use crate::language::Choice;
use crate::window::input::TextInput;
use crate::window::parts::CHECK_ICON;
use crate::window::parts::CHEVRON_ICON;
use crate::window::parts::SEARCH_ICON;
use crate::window::surface::SurfaceExt;
use crate::window::surface::framed;
use crate::window::surface::hairline;
use crate::window::theme::Theme;
use crate::words::words;

const CLAUDE_MODELS: [(&str, &str); 4] = [
    ("claude-fable-5-2", "Claude Fable 5.2"),
    ("claude-opus-5-5", "Claude Opus 5.5"),
    (DEFAULT_MODEL, "Claude Sonnet 5.5"),
    ("claude-haiku-5-5", "Claude Haiku 5.5"),
];

const CLAUDE_ADDRESS: &str = "https://api.anthropic.com";

const EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

fn default_effort(model: &str) -> &'static str {
    match model {
        "claude-opus-5-5" | "claude-haiku-5-5" => "medium",
        _ => "high",
    }
}

const ROW: f32 = 38.;
const ROWS_SHOWN: usize = 8;
const MENU_EDGE: f32 = 4.;
const GAP: f32 = 4.;
const WINDOW_EDGE: f32 = 8.;

pub(super) const MODEL_PICKER: &str = "model-picker";
pub(super) const EFFORT_PICKER: &str = "effort-picker";

/// What the provider answered when asked for its models, as ids and the names to show.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) enum Served {
    #[default]
    Unasked,
    /// A Claude key is not there yet, so the models offered are the ones dotori knows.
    NoKey,
    Asking,
    Found(Vec<(String, String)>),
    Failed(CheckError),
}

/// Where to ask for models: the provider, its address and the key, if any.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Asked {
    pub(super) openai_compatible: bool,
    pub(super) address: String,
    pub(super) key: Option<String>,
}

type Listed = Result<Vec<(String, String)>, CheckError>;

/// Asks a provider for the models it lists.
pub(super) type List = fn(Asked) -> oneshot::Receiver<Listed>;

/// How the window asks for models: over the network, or never in tests, which set their own.
pub(super) const LIST: List = if cfg!(test) { unasked } else { list_models };

fn unasked(_: Asked) -> oneshot::Receiver<Listed> {
    oneshot::channel().1
}

fn list_models(asked: Asked) -> oneshot::Receiver<Listed> {
    let (tell, told) = oneshot::channel();
    thread::spawn(move || {
        let listed = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| error.to_string())
            .and_then(|runtime| {
                if asked.openai_compatible {
                    let mut config = OpenAiConfig::new(asked.address, "");
                    config.api_key = asked.key;
                    let server = OpenAi::new(config).map_err(|error| error.to_string())?;
                    Ok(runtime
                        .block_on(server.models())
                        .map(|names| names.into_iter().map(|name| (name.clone(), name)).collect()))
                } else {
                    let mut config = AnthropicConfig::new(asked.key.unwrap_or_default());
                    config.base_url = asked.address;
                    let claude = Anthropic::new(config).map_err(|error| error.to_string())?;
                    Ok(runtime.block_on(claude.models()))
                }
            });
        let _ = tell.send(listed.unwrap_or_else(|error| {
            eprintln!("dotori: the models could not be asked for: {error}");
            Err(CheckError::NoAnswer)
        }));
    });
    told
}

/// Where each picker was drawn last, so its list opens next to it.
#[derive(Clone, Default)]
pub(super) struct Drawn(Rc<RefCell<HashMap<&'static str, Bounds<Pixels>>>>);

impl Drawn {
    pub(super) fn at(&self, id: &'static str) -> Option<Bounds<Pixels>> {
        self.0.borrow().get(id).copied()
    }

    /// A canvas over the element it sits in, which notes where that element was drawn.
    pub(super) fn note(&self, id: &'static str) -> impl IntoElement {
        let drawn = self.clone();
        canvas(
            move |bounds, _, _| {
                drawn.0.borrow_mut().insert(id, bounds);
            },
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0()
    }
}

/// One thing to do in a menu of actions: its id, its words and what it does.
pub(super) type Action = (
    &'static str,
    &'static str,
    fn(&mut SettingsView, &mut Context<SettingsView>),
);

/// The id an open menu of actions keeps in `open_picker`, as the menu under the picture.
pub(super) const AVATAR_MENU: &str = "avatar-menu";

struct Pick {
    value: String,
    label: SharedString,
    default: bool,
}

/// How a picker sits: on the surface of the settings, or raised on the canvas of the onboarding.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Sits {
    Settings,
    Onboarding { alarm: bool },
}

struct Picker {
    id: &'static str,
    sits: Sits,
    shown: SharedString,
    chosen: String,
    picks: Vec<Pick>,
    searchable: bool,
}

pub(super) const LANGUAGE_PICKER: &str = "language-picker";
pub(super) const ZONE_PICKER: &str = "zone-picker";

fn zone_names() -> &'static [String] {
    static NAMES: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    NAMES.get_or_init(|| {
        let mut names: Vec<String> = jiff::tz::db()
            .available()
            .map(|name| name.as_str().to_owned())
            .filter(|name| name.contains('/'))
            .collect();
        names.sort();
        names
    })
}

impl SettingsView {
    fn model_picks(&self, cx: &App) -> Picker {
        let typed = self.model.read(cx).text().trim().to_owned();
        let said = &words().settings;
        let claude = !self.openai_compatible;
        let pick = |id: &str, name: &str| {
            let default = claude && id == DEFAULT_MODEL;
            Pick {
                value: if default {
                    String::new()
                } else {
                    id.to_owned()
                },
                label: SharedString::from(name.to_owned()),
                default,
            }
        };
        let mut picks: Vec<Pick> = match &self.served {
            Served::Found(names) if !names.is_empty() || !claude => {
                names.iter().map(|(id, name)| pick(id, name)).collect()
            }
            _ if claude => CLAUDE_MODELS
                .iter()
                .map(|(id, name)| pick(id, name))
                .collect(),
            _ => Vec::new(),
        };
        let chosen = if !self.openai_compatible && typed == DEFAULT_MODEL {
            String::new()
        } else {
            typed.clone()
        };
        if !chosen.is_empty() && !picks.iter().any(|pick| pick.value == chosen) {
            picks.insert(
                0,
                Pick {
                    value: chosen.clone(),
                    label: SharedString::from(chosen.clone()),
                    default: false,
                },
            );
        }
        let shown = picks.iter().find(|pick| pick.value == chosen).map_or_else(
            || SharedString::new_static(said.choose_model),
            |pick| pick.label.clone(),
        );
        Picker {
            id: MODEL_PICKER,
            sits: Sits::Settings,
            shown,
            chosen,
            picks,
            searchable: false,
        }
    }

    fn effort_picks(&self, cx: &App) -> Picker {
        let said = &words().settings;
        let model = self.model.read(cx).text().trim().to_owned();
        let model = if model.is_empty() {
            DEFAULT_MODEL
        } else {
            model.as_str()
        };
        let usual = default_effort(model);
        let typed = self.effort.read(cx).text().trim().to_owned();
        let chosen = if typed == usual { String::new() } else { typed };
        let mut picks: Vec<Pick> = EFFORTS
            .iter()
            .zip(said.efforts)
            .map(|(level, label)| Pick {
                value: if *level == usual {
                    String::new()
                } else {
                    (*level).to_owned()
                },
                label: SharedString::new_static(label),
                default: *level == usual,
            })
            .collect();
        if !chosen.is_empty() && !picks.iter().any(|pick| pick.value == chosen) {
            picks.insert(
                0,
                Pick {
                    value: chosen.clone(),
                    label: SharedString::from(chosen.clone()),
                    default: false,
                },
            );
        }
        let shown = picks
            .iter()
            .find(|pick| pick.value == chosen)
            .map_or_else(SharedString::default, |pick| pick.label.clone());
        Picker {
            id: EFFORT_PICKER,
            sits: Sits::Settings,
            shown,
            chosen,
            picks,
            searchable: false,
        }
    }

    fn language_picks(&self) -> Picker {
        let chosen = crate::language::chosen();
        let picks: Vec<Pick> = Choice::ALL
            .iter()
            .map(|choice| Pick {
                value: match choice {
                    Choice::System => String::new(),
                    Choice::Only(_) => choice.code().to_owned(),
                },
                label: SharedString::new_static(match choice {
                    Choice::System => words().common.follow_system,
                    Choice::Only(language) => language.name(),
                }),
                default: *choice == Choice::System,
            })
            .collect();
        let chosen = match chosen {
            Choice::System => String::new(),
            Choice::Only(_) => chosen.code().to_owned(),
        };
        let shown = picks
            .iter()
            .find(|pick| pick.value == chosen)
            .map_or_else(SharedString::default, |pick| pick.label.clone());
        Picker {
            id: LANGUAGE_PICKER,
            sits: Sits::Settings,
            shown,
            chosen,
            picks,
            searchable: false,
        }
    }

    fn zone_picks(&self) -> Picker {
        let system = jiff::tz::TimeZone::system().iana_name().map(str::to_owned);
        let follow = match &system {
            Some(here) => format!("{} ({here})", words().common.follow_system),
            None => words().common.follow_system.to_owned(),
        };
        let mut picks = vec![Pick {
            value: String::new(),
            label: SharedString::from(follow),
            default: true,
        }];
        picks.extend(zone_names().iter().map(|name| Pick {
            value: name.clone(),
            label: SharedString::from(name.clone()),
            default: false,
        }));
        let chosen = self.zone.clone().unwrap_or_default();
        let shown = picks.iter().find(|pick| pick.value == chosen).map_or_else(
            || SharedString::from(chosen.clone()),
            |pick| pick.label.clone(),
        );
        Picker {
            id: ZONE_PICKER,
            sits: Sits::Settings,
            shown,
            chosen,
            picks,
            searchable: true,
        }
    }

    pub(super) fn language_picker(
        &self,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let picker = self.language_picks();
        self.picker(theme, window, cx, picker, Sits::Settings)
    }

    pub(super) fn zone_picker(
        &self,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let picker = self.zone_picks();
        self.picker(theme, window, cx, picker, Sits::Settings)
    }

    pub(super) fn model_picker(
        &self,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
        sits: Sits,
    ) -> Div {
        let picker = self.model_picks(cx);
        self.picker(theme, window, cx, picker, sits)
    }

    pub(super) fn effort_picker(
        &self,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
        sits: Sits,
    ) -> Div {
        let picker = self.effort_picks(cx);
        self.picker(theme, window, cx, picker, sits)
    }

    /// The line under the model picker of an OpenAI compatible server, while its models are not
    /// there to pick.
    pub(super) fn served_line(&self, cx: &App) -> Option<(&'static str, bool)> {
        if !self.openai_compatible {
            return None;
        }
        let said = &words().settings;
        let no_address = self.base_url.read(cx).text().trim().is_empty();
        match &self.served {
            Served::Unasked if no_address => Some((said.models_need_address, false)),
            Served::Unasked | Served::NoKey | Served::Asking => Some((said.models_asking, false)),
            Served::Found(names) if names.is_empty() => Some((said.models_none, true)),
            Served::Found(_) => None,
            Served::Failed(CheckError::KeyRefused) => Some((words().onboarding.key_refused, true)),
            Served::Failed(CheckError::NoAnswer) => Some((said.models_failed, true)),
        }
    }

    fn picker(
        &self,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
        mut picker: Picker,
        sits: Sits,
    ) -> Div {
        let id = picker.id;
        let open = self.open_picker == Some(id);
        let width = hairline(window);
        let (radius, alarm) = match sits {
            Sits::Settings => (super::FIELD_RADIUS, false),
            Sits::Onboarding { alarm } => (super::onboarding::FIELD, alarm),
        };
        picker.sits = sits;
        let ring = if alarm {
            Some(theme.danger.into())
        } else {
            open.then(|| theme.primary.into())
        };
        let empty = picker.picks.is_empty();
        let searchable = picker.searchable;
        let unchosen = picker.chosen.is_empty() && !picker.picks.iter().any(|pick| pick.default);
        let drawn = self.drawn.clone();
        let trigger = div()
            .id(id)
            .debug_selector(move || id.to_owned())
            .relative()
            .flex()
            .items_center()
            .gap_2()
            .rounded(px(radius - width))
            .when(sits == Sits::Settings, |trigger| {
                trigger.px_3().py_2().bg(theme.canvas)
            })
            .when(sits != Sits::Settings, |trigger| {
                trigger.card(theme).h(px(42. - 2. * width)).px_3p5()
            })
            .when(!empty, |trigger| trigger.cursor_pointer())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .when(unchosen, |text| text.text_color(theme.muted))
                    .child(picker.shown.clone()),
            )
            .child(
                svg()
                    .path(CHEVRON_ICON)
                    .flex_none()
                    .size(px(14.))
                    .text_color(theme.muted),
            )
            .on_click(cx.listener(move |view, _, window, cx| {
                if empty {
                    return;
                }
                view.open_picker = if view.open_picker == Some(id) {
                    None
                } else {
                    Some(id)
                };
                if searchable && view.open_picker == Some(id) {
                    view.search.update(cx, |input, cx| input.set_text("", cx));
                    window.focus(&view.search.focus_handle(cx));
                }
                cx.notify();
            }));
        let holder = div()
            .relative()
            .child(framed(ring, radius - width, width, trigger))
            .child(drawn.note(id));
        match (open, self.drawn.at(id)) {
            (true, Some(bounds)) => holder.child(self.menu(theme, window, cx, picker, bounds)),
            _ => holder,
        }
    }

    fn menu(
        &self,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
        picker: Picker,
        bounds: Bounds<Pixels>,
    ) -> impl IntoElement {
        let Picker {
            id,
            sits,
            chosen,
            picks,
            searchable,
            ..
        } = picker;
        let query = self.search.read(cx).text().trim().to_lowercase();
        let picks: Vec<Pick> = if searchable && !query.is_empty() {
            let query = query.replace(' ', "_");
            picks
                .into_iter()
                .filter(|pick| {
                    pick.label.to_lowercase().replace(' ', "_").contains(&query)
                        || pick.value.to_lowercase().contains(&query)
                })
                .collect()
        } else {
            picks
        };
        let radius = match sits {
            Sits::Settings => super::FIELD_RADIUS,
            Sits::Onboarding { .. } => super::onboarding::FIELD,
        };
        let rows_tall = px(ROW * picks.len().clamp(1, ROWS_SHOWN) as f32 + 2. * MENU_EDGE);
        let tall = if searchable {
            rows_tall + px(ROW + 1.)
        } else {
            rows_tall
        };
        let below =
            bounds.bottom() + px(GAP) + tall <= window.viewport_size().height - px(WINDOW_EDGE);
        let (corner, at) = if below {
            (
                Corner::TopLeft,
                point(bounds.left(), bounds.bottom() + px(GAP)),
            )
        } else {
            (
                Corner::BottomLeft,
                point(bounds.left(), bounds.top() - px(GAP)),
            )
        };
        let drawn = self.drawn.clone();
        let rows =
            picks.into_iter().map(|pick| {
                let is_chosen = pick.value == *chosen;
                let name = if pick.value.is_empty() {
                    String::from("default")
                } else {
                    pick.value.clone()
                };
                let value = pick.value.clone();
                div()
                    .id(SharedString::from(format!("{id}-{name}")))
                    .debug_selector(move || format!("{id}-{name}"))
                    .flex_none()
                    .h(px(ROW))
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_md()
                    .cursor_pointer()
                    .hover(|row| row.bg(theme.bubble))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .when(is_chosen, |text| text.font_weight(FontWeight::SEMIBOLD))
                            .child(pick.label),
                    )
                    .when(pick.default, |row| {
                        row.child(
                            div()
                                .flex_none()
                                .text_xs()
                                .text_color(theme.muted)
                                .child(words().settings.default_pick),
                        )
                    })
                    .child(svg().path(CHECK_ICON).flex_none().size(px(13.)).text_color(
                        if is_chosen {
                            theme.text
                        } else {
                            gpui::transparent_black().into()
                        },
                    ))
                    .on_click(cx.listener(move |view, _, _, cx| view.pick(id, value.clone(), cx)))
            });
        deferred(
            anchored()
                .anchor(corner)
                .position(at)
                .snap_to_window_with_margin(px(WINDOW_EDGE))
                .child(
                    div()
                        .occlude()
                        .w(bounds.size.width)
                        .flex()
                        .flex_col()
                        .rounded(px(radius))
                        .when(sits == Sits::Settings, |list| {
                            list.bg(theme.canvas).lifted(theme)
                        })
                        .when(sits != Sits::Settings, |list| list.card(theme))
                        .text_color(theme.text)
                        .when(searchable, |list| {
                            list.child(
                                div()
                                    .id(SharedString::from(format!("{id}-search")))
                                    .debug_selector(move || format!("{id}-search"))
                                    .flex_none()
                                    .h(px(ROW))
                                    .px(px(MENU_EDGE + 12.))
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .border_b_1()
                                    .border_color(theme.line)
                                    .child(
                                        svg()
                                            .path(SEARCH_ICON)
                                            .flex_none()
                                            .size(px(14.))
                                            .text_color(theme.muted),
                                    )
                                    .child(div().flex_1().min_w_0().child(self.search.clone())),
                            )
                        })
                        .child(
                            div()
                                .id(SharedString::from(format!("{id}-list")))
                                .max_h(rows_tall)
                                .overflow_y_scroll()
                                .p(px(MENU_EDGE))
                                .flex()
                                .flex_col()
                                .children(rows),
                        )
                        .on_mouse_down_out(cx.listener(
                            move |view, event: &MouseDownEvent, _, cx| {
                                let on_trigger = drawn
                                    .at(id)
                                    .is_some_and(|bounds| bounds.contains(&event.position));
                                if !on_trigger && view.open_picker == Some(id) {
                                    view.open_picker = None;
                                    cx.notify();
                                }
                            },
                        )),
                ),
        )
        .with_priority(1)
    }

    /// A menu of actions under the element drawn as `id`, shown while `open_picker` is `id`. It
    /// closes on a click outside it or on one of its actions.
    pub(super) fn actions_menu(
        &self,
        theme: &Theme,
        window: &Window,
        cx: &mut Context<Self>,
        id: &'static str,
        actions: Vec<Action>,
    ) -> Option<impl IntoElement + use<>> {
        let bounds = self.drawn.at(id)?;
        let tall = px(ROW * actions.len() as f32 + 2. * MENU_EDGE);
        let below =
            bounds.bottom() + px(GAP) + tall <= window.viewport_size().height - px(WINDOW_EDGE);
        let (corner, at) = if below {
            (
                Corner::TopLeft,
                point(bounds.left(), bounds.bottom() + px(GAP)),
            )
        } else {
            (
                Corner::BottomLeft,
                point(bounds.left(), bounds.top() - px(GAP)),
            )
        };
        let radius = if self.walk.is_some() {
            super::onboarding::FIELD
        } else {
            super::FIELD_RADIUS
        };
        let walk = self.walk.is_some();
        let drawn = self.drawn.clone();
        let rows = actions.into_iter().map(|(name, label, act)| {
            div()
                .id(name)
                .debug_selector(move || name.to_owned())
                .flex_none()
                .h(px(ROW))
                .px_3()
                .flex()
                .items_center()
                .rounded_md()
                .whitespace_nowrap()
                .cursor_pointer()
                .hover(|row| row.bg(theme.bubble))
                .child(label)
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.open_picker = None;
                    act(view, cx);
                    cx.notify();
                }))
        });
        Some(
            deferred(
                anchored()
                    .anchor(corner)
                    .position(at)
                    .snap_to_window_with_margin(px(WINDOW_EDGE))
                    .child(
                        div()
                            .occlude()
                            .flex()
                            .flex_col()
                            .p(px(MENU_EDGE))
                            .rounded(px(radius))
                            .when(!walk, |list| list.bg(theme.canvas).lifted(theme))
                            .when(walk, |list| list.card(theme))
                            .text_color(theme.text)
                            .children(rows)
                            .on_mouse_down_out(cx.listener(
                                move |view, event: &MouseDownEvent, _, cx| {
                                    let on_trigger = drawn
                                        .at(id)
                                        .is_some_and(|bounds| bounds.contains(&event.position));
                                    if !on_trigger && view.open_picker == Some(id) {
                                        view.open_picker = None;
                                        cx.notify();
                                    }
                                },
                            )),
                    ),
            )
            .with_priority(1),
        )
    }

    fn pick(&mut self, id: &'static str, value: String, cx: &mut Context<Self>) {
        self.open_picker = None;
        match id {
            LANGUAGE_PICKER => {
                let choice = Choice::ALL
                    .into_iter()
                    .find(|choice| match choice {
                        Choice::System => value.is_empty(),
                        Choice::Only(_) => choice.code() == value,
                    })
                    .unwrap_or(Choice::System);
                self.set_language(choice, cx);
            }
            ZONE_PICKER => self.set_zone((!value.is_empty()).then_some(value), cx),
            _ => {
                let input: &Entity<TextInput> = if id == MODEL_PICKER {
                    &self.model
                } else {
                    &self.effort
                };
                input.update(cx, |input, cx| input.set_text(&value, cx));
                if self.walk.is_none() {
                    self.save_part("model", false, cx);
                }
            }
        }
        cx.notify();
    }

    /// Asks the provider for its models, unless it was already asked with the same address and
    /// key. A Claude key that is not there yet leaves the models dotori knows.
    pub(super) fn ask_served(&mut self, cx: &mut Context<Self>) {
        let openai_compatible = self.openai_compatible;
        let typed = self.base_url.read(cx).text().trim().to_owned();
        let address = match (typed.is_empty(), openai_compatible) {
            (true, true) => {
                self.served = Served::Unasked;
                self.served_for = None;
                cx.notify();
                return;
            }
            (true, false) => String::from(CLAUDE_ADDRESS),
            (false, _) => typed,
        };
        let typed_key = self.api_key.read(cx).text().trim().to_owned();
        let key = if typed_key.is_empty() {
            self.api_key_name().and_then(|name| {
                crate::settings::environment(&name)
                    .or_else(|| crate::vault::lookup(&self.vault)(&name))
            })
        } else {
            Some(typed_key)
        };
        if !openai_compatible && key.is_none() {
            self.served = Served::NoKey;
            self.served_for = None;
            cx.notify();
            return;
        }
        let asked = Asked {
            openai_compatible,
            address,
            key,
        };
        let settled = !matches!(self.served, Served::Unasked | Served::NoKey);
        if self.served_for.as_ref() == Some(&asked) && settled {
            return;
        }
        self.served_for = Some(asked.clone());
        self.served = Served::Asking;
        let told = (self.list)(asked.clone());
        self.asking = Some(cx.spawn(async move |view, cx| {
            let found = told.await.unwrap_or(Err(CheckError::NoAnswer));
            let _ = view.update(cx, |view, cx| {
                if view.served_for.as_ref() == Some(&asked) {
                    view.served = match found {
                        Ok(names) => Served::Found(names),
                        Err(error) => Served::Failed(error),
                    };
                    cx.notify();
                }
            });
        }));
        cx.notify();
    }

    /// Asks for the server's models once the model fields are shown and nothing was asked yet.
    pub(super) fn ask_served_once(&mut self, cx: &mut Context<Self>) {
        let no_address = self.base_url.read(cx).text().trim().is_empty();
        if self.served == Served::Unasked && !(self.openai_compatible && no_address) {
            self.ask_served(cx);
        }
    }
}
