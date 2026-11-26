/// Strings more than one window or the tray shows.
#[derive(Debug)]
pub(crate) struct Common {
    pub(crate) settings_title: &'static str,
    pub(crate) onboarding_title: &'static str,
    pub(crate) browser_title: &'static str,
    pub(crate) open_chat: &'static str,
    pub(crate) settings: &'static str,
    pub(crate) quit: &'static str,
    pub(crate) follow_system: &'static str,
    pub(crate) light: &'static str,
    pub(crate) dark: &'static str,
    pub(crate) language: &'static str,
    /// The name of the theme DESIGN.md calls by the given name.
    pub(crate) theme: fn(&str) -> &'static str,
}

pub(crate) const KOREAN: Common = Common {
    settings_title: "dotori 설정",
    onboarding_title: "dotori 시작하기",
    browser_title: "dotori 브라우저",
    open_chat: "대화창 열기",
    settings: "설정",
    quit: "끝내기",
    follow_system: "시스템 따라",
    light: "밝게",
    dark: "어둡게",
    language: "언어",
    theme: |name| match name {
        "acorn" => "도토리",
        "graphite" => "그래파이트",
        "paper" => "페이퍼",
        "forest" => "포레스트",
        "indigo" => "인디고",
        _ => "테마",
    },
};

pub(crate) const ENGLISH: Common = Common {
    settings_title: "dotori settings",
    onboarding_title: "Welcome to dotori",
    browser_title: "dotori browser",
    open_chat: "Open chat",
    settings: "Settings",
    quit: "Quit",
    follow_system: "Match system",
    light: "Light",
    dark: "Dark",
    language: "Language",
    theme: |name| match name {
        "acorn" => "Acorn",
        "graphite" => "Graphite",
        "paper" => "Paper",
        "forest" => "Forest",
        "indigo" => "Indigo",
        _ => "Theme",
    },
};
