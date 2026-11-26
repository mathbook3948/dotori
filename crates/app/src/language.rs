//! The language dotori speaks to its owner in: the one the settings name, or else the system's.
//! Every window and the tray read the same choice.

use std::path::Path;
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

static CHOSEN: RwLock<Choice> = RwLock::new(Choice::System);

impl Language {
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

    /// The language a locale tag such as `ko-KR` or `en_US.UTF-8` names. Anything but Korean is
    /// English.
    pub(crate) fn of_locale(tag: &str) -> Self {
        if tag.to_ascii_lowercase().starts_with("ko") {
            Self::Korean
        } else {
            Self::English
        }
    }
}

impl Choice {
    /// Every choice, in the order the settings window shows them.
    pub(crate) const ALL: [Self; 3] = [
        Self::System,
        Self::Only(Language::Korean),
        Self::Only(Language::English),
    ];

    /// The choice the settings value names. A missing or unknown value follows the system.
    pub(crate) fn named(value: Option<&str>) -> Self {
        match value {
            Some("ko") => Self::Only(Language::Korean),
            Some("en") => Self::Only(Language::English),
            _ => Self::System,
        }
    }

    /// The value the settings file keeps for this choice.
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::System => "system",
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

/// Makes `choice` the one this process speaks from now on.
pub(crate) fn choose(choice: Choice) {
    *CHOSEN
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = choice;
}

/// What the owner chose in the settings, which the settings window shows.
pub(crate) fn chosen() -> Choice {
    *CHOSEN
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The language in use. `DOTORI_LANGUAGE`, set to `ko` or `en`, overrides the settings.
pub(crate) fn current() -> Language {
    let overridden = std::env::var(VARIABLE).ok();
    match Choice::named(overridden.as_deref()) {
        Choice::Only(language) => language,
        Choice::System => chosen().language(),
    }
}

#[cfg(test)]
#[path = "language.test.rs"]
mod tests;
