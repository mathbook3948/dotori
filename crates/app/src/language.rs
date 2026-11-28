//! The language dotori speaks to its owner in: the one the settings keep, which every PC shares.
//! Until one is saved, the system's. Every window and the tray read the same one.

use std::path::Path;
use std::sync::OnceLock;
use std::sync::RwLock;

use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde::Serializer;

use crate::settings::Settings;

/// A language the windows and the tray are written in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Language {
    Korean,
    English,
}

impl Serialize for Language {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.code())
    }
}

impl<'de> Deserialize<'de> for Language {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let code = String::deserialize(deserializer)?;
        Self::ALL
            .into_iter()
            .find(|language| language.code() == code)
            .ok_or_else(|| serde::de::Error::custom(format!("no language `{code}`")))
    }
}

/// Reads the language a settings file keeps. A value an older dotori wrote, such as `system` or
/// `ko-KR`, reads as the language its tag names, or as none, and the next start saves one.
pub(crate) fn lenient<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Language>, D::Error> {
    let value = Option::<String>::deserialize(deserializer)?;
    Ok(value.and_then(|value| Language::of_tag(value.trim())))
}

const VARIABLE: &str = "DOTORI_LANGUAGE";

static CHOSEN: RwLock<Option<(Language, Language)>> = RwLock::new(None);

impl Language {
    /// Every language, in the order the settings window shows them.
    pub(crate) const ALL: [Self; 2] = [Self::Korean, Self::English];

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

fn of_system() -> Language {
    sys_locale::get_locale()
        .map(|tag| Language::of_locale(&tag))
        .unwrap_or(Language::English)
}

/// The language the settings at `path` keep, or else the system's. Settings that cannot be read
/// have none.
pub(crate) fn saved(path: &Path) -> Language {
    Settings::load(path)
        .ok()
        .and_then(|settings| settings.language)
        .unwrap_or_else(of_system)
}

/// Saves the system's language into the settings at `path` when they keep none, and says which.
/// Settings that keep a language are left as they are.
pub(crate) fn settle(path: &Path) -> Result<Option<Language>, String> {
    let mut settings = Settings::load(path)?;
    if settings.language.is_some() {
        return Ok(None);
    }
    let language = of_system();
    settings.language = Some(language);
    settings.save_parts(path, &["language"])?;
    Ok(Some(language))
}

fn overridden() -> Option<Language> {
    static OVERRIDDEN: OnceLock<Option<Language>> = OnceLock::new();
    *OVERRIDDEN.get_or_init(|| {
        let value = std::env::var(VARIABLE).ok()?;
        let language = Language::of_tag(value.trim());
        if language.is_none() {
            eprintln!("dotori: no language `{value}` in {VARIABLE}, so it is ignored");
        }
        language
    })
}

/// Makes `language` the one saved for this process, and the one it speaks from now on.
/// `DOTORI_LANGUAGE`, set to `ko` or `en`, changes only what this process speaks.
pub(crate) fn choose(language: Language) {
    let speaking = overridden().unwrap_or(language);
    *CHOSEN
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some((language, speaking));
}

fn stored() -> (Language, Language) {
    let chosen = *CHOSEN
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    chosen.unwrap_or_else(|| {
        choose(of_system());
        stored()
    })
}

/// The language the settings keep, which the settings window shows.
pub(crate) fn chosen() -> Language {
    stored().0
}

/// The language in use.
pub(crate) fn current() -> Language {
    stored().1
}

#[cfg(test)]
#[path = "language.test.rs"]
mod tests;
