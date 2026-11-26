use crate::window::identity::Particle;
use crate::window::identity::called;
use crate::window::identity::name;

/// Strings of the browser window.
#[derive(Debug)]
pub(crate) struct Screen {
    pub(crate) files_asked: &'static str,
    pub(crate) upload_prompt: &'static str,
    pub(crate) select_title: &'static str,
    pub(crate) value_title: &'static str,
    pub(crate) files_title: &'static str,
    pub(crate) put: &'static str,
    pub(crate) files_owner: &'static str,
    pub(crate) files_agent: &'static str,
    pub(crate) close: &'static str,
    pub(crate) login_title: &'static str,
    pub(crate) login_name: &'static str,
    pub(crate) login_secret: &'static str,
    pub(crate) login_failed: &'static str,
    pub(crate) login_kept: &'static str,
    pub(crate) cancel: &'static str,
    pub(crate) log_in: &'static str,
    pub(crate) connecting: &'static str,
    /// The bar's state while the assistant drives the browser.
    pub(crate) agent: fn() -> String,
    pub(crate) waiting: &'static str,
    /// The bar's state while the owner drives the browser.
    pub(crate) owner: fn() -> String,
    /// The bar's state while nobody drives the browser.
    pub(crate) idle: fn() -> String,
    pub(crate) done: &'static str,
    pub(crate) take: &'static str,
    pub(crate) popup: &'static str,
    /// The empty page while the assistant's browser connects.
    pub(crate) page_connecting: fn() -> String,
    /// The empty page before the assistant opens anything.
    pub(crate) page_empty: fn() -> String,
    /// What the assistant asked the owner to do, given that ask.
    pub(crate) asks: fn(&str) -> String,
}

pub(crate) const KOREAN: Screen = Screen {
    files_asked: "페이지가 파일을 골라 달라고 해요",
    upload_prompt: "브라우저에 올릴 파일",
    select_title: "고르기",
    value_title: "값 넣기",
    files_title: "파일 고르는 중",
    put: "넣기",
    files_owner: "파일 대화상자에서 고르면 브라우저에 올라가요.",
    files_agent: "페이지가 파일을 달라고 해요. 넘겨받으면 파일을 고를 수 있어요.",
    close: "닫기",
    login_title: "로그인이 필요해요",
    login_name: "아이디",
    login_secret: "비밀번호",
    login_failed: "아이디나 비밀번호가 맞지 않았어요. 다시 넣어 주세요.",
    login_kept: "이 페이지에만 넘기고 대화 기록에는 남기지 않아요.",
    cancel: "취소",
    log_in: "로그인",
    connecting: "연결하는 중이에요",
    agent: || format!("{} 쓰고 있어요", called(Particle::Subject)),
    waiting: "넘겨받는 중이에요",
    owner: || format!("다 하면 {} 이어서 해요", called(Particle::Subject)),
    idle: || format!("{} 쉬고 있어요", called(Particle::Subject)),
    done: "다 했어요",
    take: "넘겨받기",
    popup: "새 창",
    page_connecting: || format!("{} 브라우저에 연결하는 중이에요.", called(Particle::Of)),
    page_empty: || {
        format!(
            "{} 브라우저를 쓰면 여기에 보여요.",
            called(Particle::Subject)
        )
    },
    asks: |asked| format!("{} 부탁해요: {asked}", called(Particle::Subject)),
};

pub(crate) const ENGLISH: Screen = Screen {
    files_asked: "The page wants you to pick a file",
    upload_prompt: "Files to upload",
    select_title: "Choose an option",
    value_title: "Enter a value",
    files_title: "Picking a file",
    put: "Fill in",
    files_owner: "Pick a file in the dialog to upload it to the page.",
    files_agent: "The page wants a file. Take over to pick one.",
    close: "Close",
    login_title: "Log in to continue",
    login_name: "Username",
    login_secret: "Password",
    login_failed: "That username or password didn't work. Try again.",
    login_kept: "These go to this page only and stay out of the chat history.",
    cancel: "Cancel",
    log_in: "Log in",
    connecting: "Connecting",
    agent: || format!("{} is browsing", name()),
    waiting: "Taking over",
    owner: || format!("{} picks up when you're done", name()),
    idle: || format!("{} is idle", name()),
    done: "I'm done",
    take: "Take over",
    popup: "New window",
    page_connecting: || format!("Connecting to {}'s browser.", name()),
    page_empty: || format!("When {} uses the browser, it shows up here.", name()),
    asks: |asked| format!("{} asks: {asked}", name()),
};
