//! Every string the windows and the tray show, in each language. Each area keeps one table per
//! language of the same type, so a string missing from a language does not build. The rules the
//! strings follow are in Voice & Content Strategy of `DESIGN.md`.

mod chat;
mod common;
mod onboarding;
mod pcs;
mod routines;
mod screen;
mod settings;

use crate::language;
use crate::language::Language;

/// The strings of every area in one language.
#[derive(Debug)]
pub(crate) struct Words {
    pub(crate) common: common::Common,
    pub(crate) onboarding: onboarding::Onboarding,
    pub(crate) settings: settings::Settings,
    pub(crate) chat: chat::Chat,
    pub(crate) pcs: pcs::Pcs,
    pub(crate) screen: screen::Screen,
    pub(crate) routines: routines::Routines,
}

static KOREAN: Words = Words {
    common: common::KOREAN,
    onboarding: onboarding::KOREAN,
    settings: settings::KOREAN,
    chat: chat::KOREAN,
    pcs: pcs::KOREAN,
    screen: screen::KOREAN,
    routines: routines::KOREAN,
};

static ENGLISH: Words = Words {
    common: common::ENGLISH,
    onboarding: onboarding::ENGLISH,
    settings: settings::ENGLISH,
    chat: chat::ENGLISH,
    pcs: pcs::ENGLISH,
    screen: screen::ENGLISH,
    routines: routines::ENGLISH,
};

/// The strings in `language`.
pub(crate) fn of(language: Language) -> &'static Words {
    match language {
        Language::Korean => &KOREAN,
        Language::English => &ENGLISH,
    }
}

/// The strings in the language in use.
pub(crate) fn words() -> &'static Words {
    of(language::current())
}

#[cfg(test)]
#[path = "words.test.rs"]
mod tests;
