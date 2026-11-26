//! The language dotori speaks to its owner in: the one the settings name, or else the system's.
//! Every window and the tray read the same choice, worked out once when it is made.

use std::path::Path;
use std::sync::OnceLock;
use std::sync::RwLock;

use crate::settings::Settings;

/// A language the windows and the tray are written in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Language {
    Korean,
    English,
}

/// What the settings ask for: a language, or the system's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Choice {
    System,
    Only(Language),
}

const VARIABLE: &str = "DOTORI_LANGUAGE";
const SYSTEM: &str = "system";

static CHOSEN: RwLock<Option<(Choice, Language)>> = RwLock::new(None);

impl Language {
    /// The language's name in English, as the model is told it.
    pub(crate) fn english_name(self) -> &'static str {
        match self {
            Self::Korean => "Korean",
            Self::English => "English",
        }
    }

    /// The value the settings file keeps for this language.
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::Korean => "ko",
            Self::English => "en",
        }
    }

    /// The language's name in itself, as a list of languages shows it.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Korean => "한국어",
            Self::English => "English",
        }
    }

    /// The language a tag such as `ko`, `KO`, `ko-KR`, `ko_KR.UTF-8` or `en-US` names, read by
    /// its primary subtag in any case. A tag of any other language names none.
    pub(crate) fn of_tag(tag: &str) -> Option<Self> {
        let primary = tag.split(['-', '_', '.', '@']).next().unwrap_or_default();
        if primary.eq_ignore_ascii_case("ko") {
            Some(Self::Korean)
        } else if primary.eq_ignore_ascii_case("en") {
            Some(Self::English)
        } else {
            None
        }
    }

    /// The language the system's locale tag names. Anything but Korean is English.
    pub(crate) fn of_locale(tag: &str) -> Self {
        Self::of_tag(tag).unwrap_or(Self::English)
    }
}

impl Choice {
    /// Every choice, in the order the settings window shows them.
    pub(crate) const ALL: [Self; 3] = [
        Self::System,
        Self::Only(Language::Korean),
        Self::Only(Language::English),
    ];

    /// The choice the settings value names. A missing value follows the system, and so does an
    /// unknown one, with a warning.
    pub(crate) fn named(value: Option<&str>) -> Self {
        let Some(value) = value.map(str::trim) else {
            return Self::System;
        };
        if value.eq_ignore_ascii_case(SYSTEM) {
            return Self::System;
        }
        match Language::of_tag(value) {
            Some(language) => Self::Only(language),
            None => {
                eprintln!("dotori: no language `{value}`, so it follows the system's");
                Self::System
            }
        }
    }

    /// The value the settings file keeps for this choice.
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::System => SYSTEM,
            Self::Only(language) => language.code(),
        }
    }

    fn language(self) -> Language {
        match self {
            Self::Only(language) => language,
            Self::System => sys_locale::get_locale()
                .map(|tag| Language::of_locale(&tag))
                .unwrap_or(Language::English),
        }
    }
}

/// What the settings at `path` ask for. Settings that cannot be read follow the system.
pub(crate) fn saved(path: &Path) -> Choice {
    let settings = Settings::load(path).ok();
    Choice::named(
        settings
            .as_ref()
            .and_then(|settings| settings.language.as_deref()),
    )
}

fn overridden() -> Option<Choice> {
    static OVERRIDDEN: OnceLock<Option<Choice>> = OnceLock::new();
    *OVERRIDDEN.get_or_init(|| {
        std::env::var(VARIABLE)
            .ok()
            .map(|value| Choice::named(Some(&value)))
    })
}

/// Makes `choice` the one this process speaks from now on, and works out its language once.
/// `DOTORI_LANGUAGE`, set to `ko` or `en`, overrides the settings.
pub(crate) fn choose(choice: Choice) {
    let language = overridden().unwrap_or(choice).language();
    *CHOSEN
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some((choice, language));
}

fn stored() -> (Choice, Language) {
    let chosen = *CHOSEN
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    chosen.unwrap_or_else(|| {
        choose(Choice::System);
        stored()
    })
}

/// What the owner chose in the settings, which the settings window shows.
pub(crate) fn chosen() -> Choice {
    stored().0
}

/// The language in use, as the last choice worked it out.
pub(crate) fn current() -> Language {
    stored().1
}

#[cfg(test)]
#[path = "language.test.rs"]
mod tests;
