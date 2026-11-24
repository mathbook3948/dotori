//! What the windows call the assistant and the picture they show it as, from the settings. Every
//! window of this process reads the same choice, and the windows follow the settings file when it
//! changes.

use std::path::Path;
use std::path::PathBuf;
use std::sync::RwLock;

use crate::icons;
use crate::settings::AssistantSettings;
use crate::settings::Settings;

/// The assistant as the windows show it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Identity {
    pub(crate) name: Option<String>,
    pub(crate) icon: Option<PathBuf>,
}

/// A Korean particle after the assistant's name, which changes with the sound the name ends on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Particle {
    Subject,
    With,
    To,
    Of,
}

static CHOSEN: RwLock<Option<Identity>> = RwLock::new(None);

impl Identity {
    /// The assistant as the settings at `settings` name it, with its picture from `icons`.
    pub(crate) fn read(settings: &Path, icon_folder: &Path) -> Self {
        let assistant = Settings::load(settings)
            .ok()
            .and_then(|settings| settings.assistant);
        Self {
            name: Some(AssistantSettings::name_of(assistant.as_ref())),
            icon: assistant
                .and_then(|assistant| assistant.icon)
                .and_then(|name| icons::path(icon_folder, &name)),
        }
    }
}

/// Makes `identity` the one every window shows. Says whether it changed.
pub(crate) fn choose(identity: Identity) -> bool {
    let mut chosen = CHOSEN.write().expect("unpoisoned");
    if chosen.as_ref() == Some(&identity) {
        return false;
    }
    *chosen = Some(identity);
    true
}

/// What the windows call the assistant.
pub(crate) fn name() -> String {
    CHOSEN
        .read()
        .expect("unpoisoned")
        .as_ref()
        .and_then(|chosen| chosen.name.clone())
        .unwrap_or_else(|| AssistantSettings::name_of(None))
}

/// The picture the owner gave the assistant, if any.
pub(crate) fn icon() -> Option<PathBuf> {
    CHOSEN
        .read()
        .expect("unpoisoned")
        .as_ref()
        .and_then(|chosen| chosen.icon.clone())
}

/// The assistant's name followed by `particle`, such as "dotori가" or "별이".
pub(crate) fn called(particle: Particle) -> String {
    let name = name();
    let closed = ends_closed(&name);
    let after = match (particle, closed) {
        (Particle::Subject, true) => "이",
        (Particle::Subject, false) => "가",
        (Particle::With, true) => "과",
        (Particle::With, false) => "와",
        (Particle::To, _) => "에게",
        (Particle::Of, _) => "의",
    };
    format!("{name}{after}")
}

/// Whether `name`, read aloud in Korean, ends on a final consonant.
pub(crate) fn ends_closed(name: &str) -> bool {
    let lower = name
        .trim_end_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase();
    let Some(last) = lower.chars().last() else {
        return false;
    };
    if lower.chars().count() == 1 && last.is_ascii_alphabetic() {
        return matches!(last, 'l' | 'm' | 'n' | 'r');
    }
    match last {
        '가'..='힣' => (u32::from(last) - u32::from('가')) % 28 != 0,
        '0' | '1' | '3' | '6' | '7' | '8' => true,
        'l' | 'm' | 'n' | 'b' | 'p' | 't' | 'k' => true,
        'g' => lower.ends_with("ng"),
        'e' => lower.ends_with("ne") || lower.ends_with("me"),
        _ => false,
    }
}

#[cfg(test)]
#[path = "identity.test.rs"]
mod tests;
