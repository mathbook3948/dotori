use crate::window::identity;
use crate::window::identity::Particle;
use crate::window::identity::called;

/// Strings of the chat window.
#[derive(Debug)]
pub(crate) struct Chat {
    pub(crate) copy: &'static str,
    pub(crate) copied: &'static str,
    pub(crate) allowed: &'static str,
    pub(crate) declined: &'static str,
    pub(crate) expired: &'static str,
    pub(crate) deciding: &'static str,
    pub(crate) erase_hint: &'static str,
    /// What allowing a shell command does, naming the assistant, given with its particle, where
    /// the language needs it.
    pub(crate) run_hint: fn(&str) -> String,
    /// What allowing a file change does, naming the assistant where the language needs it.
    pub(crate) write_hint: fn(&str) -> String,
    pub(crate) allow_hint: &'static str,
    pub(crate) decline: &'static str,
    pub(crate) allow: &'static str,
    /// The title of a card that waits for the owner's OK, naming the assistant where the language
    /// needs it.
    pub(crate) asking: fn(&str) -> String,
    pub(crate) show_whole: &'static str,
    pub(crate) in_browser: &'static str,
    /// What the owner does when the browser window is open, and that the assistant goes on.
    pub(crate) in_browser_hint: fn() -> String,
    pub(crate) show_browser: &'static str,
    pub(crate) browser_done: &'static str,
    /// The title of a login card for the given site.
    pub(crate) log_in_to: fn(&str) -> String,
    pub(crate) login_name: &'static str,
    pub(crate) login_secret: &'static str,
    pub(crate) login_wrong: &'static str,
    pub(crate) login_kept: &'static str,
    pub(crate) login_in_browser: &'static str,
    pub(crate) cancel: &'static str,
    pub(crate) log_in: &'static str,
    pub(crate) finish_in_browser: &'static str,
    pub(crate) preview_hint: &'static str,
    pub(crate) later: &'static str,
    pub(crate) open_browser: &'static str,
    pub(crate) starting: &'static str,
    pub(crate) could_not_start: &'static str,
    pub(crate) closed: &'static str,
    pub(crate) unreachable: &'static str,
    pub(crate) not_connected: &'static str,
    /// The header line while the given PC answers instead of this one.
    pub(crate) keys_hint: &'static str,
    pub(crate) thinking: &'static str,
    /// The composer's placeholder, naming the assistant.
    pub(crate) say_to: fn() -> String,
    pub(crate) welcome: &'static str,
    /// The lines under the welcome, one per line.
    pub(crate) one_chat: &'static [&'static str],
    pub(crate) ran: &'static str,
    pub(crate) ran_failed: &'static str,
    pub(crate) timed_out: &'static str,
    pub(crate) declined_not_run: &'static str,
    pub(crate) already_settled: &'static str,
    pub(crate) command_done: &'static str,
    /// How a shell command that ended with the given code reads.
    pub(crate) command_exited: fn(i64) -> String,
    pub(crate) command_signalled: &'static str,
    /// How a shell command stopped after the given number of seconds reads.
    pub(crate) command_too_slow: fn(&str) -> String,
    pub(crate) stdout: &'static str,
    pub(crate) stderr: &'static str,
    pub(crate) only_the_end: &'static str,
    pub(crate) left_running: &'static str,
    pub(crate) run_shell: &'static str,
    pub(crate) write_file: &'static str,
    pub(crate) edit_file: &'static str,
    pub(crate) erase_records: &'static str,
    pub(crate) command_field: &'static str,
    pub(crate) folder_field: &'static str,
    pub(crate) seconds_field: &'static str,
    pub(crate) path_field: &'static str,
    pub(crate) content_field: &'static str,
    pub(crate) old_field: &'static str,
    pub(crate) new_field: &'static str,
    pub(crate) all_field: &'static str,
    /// The note under a value cut short, given how many characters the whole has.
    pub(crate) cut_note: fn(usize) -> String,
    pub(crate) routine_channel: &'static str,
    pub(crate) decision_channel: &'static str,
    pub(crate) owner_only: &'static str,
    /// A thread in the given room.
    pub(crate) thread: fn(&str) -> String,
    pub(crate) tool_row: &'static str,
    /// What a card in a chat room says once the given owner allowed it (`true`) or declined it.
    pub(crate) decided_by: fn(&str, bool) -> String,
    pub(crate) sent_by_dm: &'static str,
    /// What a chat room hears when the window shows a card that takes typed values, given the
    /// host and whether the card is a login.
    pub(crate) card_on_pc: fn(&str, bool) -> String,
    pub(crate) input_card_on_pc: &'static str,
    pub(crate) sandboxed: &'static str,
    /// The title of a card that asks to log in with a kept account, given the host and the site.
    pub(crate) log_in_with_kept: fn(&str, &str) -> String,
    /// Where a log-in card's account goes, given the assistant with its particle, the host and
    /// the path, and that the model never sees its password.
    pub(crate) log_in_note: fn(&str, &str, &str) -> String,
    pub(crate) choose_account: &'static str,
    pub(crate) account: &'static str,
    pub(crate) stop_asking: &'static str,
    pub(crate) log_in_and_stop_asking: &'static str,
    /// The button that logs in with the account of the given username.
    pub(crate) log_in_as: fn(&str) -> String,
    pub(crate) log_in_without_id: &'static str,
    pub(crate) without_id: &'static str,
    pub(crate) rest_on_pc: &'static str,
}

pub(crate) const KOREAN: Chat = Chat {
    copy: "복사",
    copied: "복사했어요",
    allowed: "허락했어요",
    declined: "거절했어요",
    expired: "이제 할 수 없어요. 다시 부탁해 주세요",
    deciding: "처리하는 중이에요",
    erase_hint: "허락하면 모든 PC에서 지워요. 되돌릴 수 없어요",
    run_hint: |subject| format!("허락하면 {subject} 이 명령을 바로 실행해요"),
    write_hint: |subject| format!("허락하면 {subject} 파일을 바로 바꿔요"),
    allow_hint: "허락하면 바로 실행해요",
    decline: "거절",
    allow: "허락",
    asking: |subject| format!("{subject} 이 일을 해도 될지 물어요"),
    show_whole: "전체 보기",
    in_browser: "브라우저 창에서 하는 중이에요",
    in_browser_hint: || {
        format!(
            "다 하면 \"다 했어요\"를 누르거나 브라우저 창을 닫아 주세요. {} 이어서 할게요.",
            called(Particle::Subject)
        )
    },
    show_browser: "창 다시 보기",
    browser_done: "다 했어요",
    log_in_to: |site| format!("{site}에 로그인해 주세요"),
    login_name: "아이디",
    login_secret: "비밀번호",
    login_wrong: "아이디나 비밀번호가 맞지 않았어요. 다시 넣어 주세요.",
    login_kept: "이 페이지에만 넘기고 대화 기록에는 남기지 않아요.",
    login_in_browser: "브라우저 창에서 할게요",
    cancel: "취소",
    log_in: "로그인",
    finish_in_browser: "브라우저에서 마저 해 주세요",
    preview_hint: "화면을 눌러도 크게 열려요",
    later: "나중에",
    open_browser: "브라우저 열기",
    starting: "dotori를 켜고 있어요",
    could_not_start: "dotori를 켜지 못했어요. 앱을 다시 열어 주세요",
    closed: "dotori가 연결을 끊었어요. 다시 연결하고 있어요",
    unreachable: "dotori에 닿지 않아요. 다시 연결하고 있어요",
    not_connected: "dotori에 연결하지 못했어요. 다시 해 보고 있어요",
    keys_hint: "Enter로 보내고, Shift+Enter로 줄을 바꿔요",
    thinking: "생각하는 중",
    say_to: || format!("{} 말하기", called(Particle::To)),
    welcome: "무엇을 도와드릴까요?",
    one_chat: &["여기서 말해도, Slack에서 말해도", "같은 대화로 이어져요."],
    ran: "실행했어요",
    ran_failed: "실행했지만 실패했어요",
    timed_out: "시간 안에 끝나지 않았어요",
    declined_not_run: "거절해서 실행하지 않았어요",
    already_settled: "이미 끝난 요청이라 실행하지 않았어요",
    command_done: "명령이 끝났어요",
    command_exited: |code| {
        let after = match code.unsigned_abs() % 10 {
            0 | 3 | 6 => "으로",
            _ => "로",
        };
        format!("명령이 코드 {code}{after} 끝났어요")
    },
    command_signalled: "명령이 신호를 받고 멈췄어요",
    command_too_slow: |seconds| format!("{seconds}초 안에 끝나지 않아 멈췄어요"),
    stdout: "출력",
    stderr: "오류 출력",
    only_the_end: "(끝부분만 보여요)",
    left_running: "(명령이 남겨 둔 프로세스가 아직 쓰고 있어요)",
    run_shell: "명령 실행",
    write_file: "파일 쓰기",
    edit_file: "파일 고치기",
    erase_records: "기록 지우기",
    command_field: "명령",
    folder_field: "폴더",
    seconds_field: "시간 제한(초)",
    path_field: "파일",
    content_field: "내용",
    old_field: "바꿀 글",
    new_field: "새 글",
    all_field: "모두 바꾸기",
    cut_note: |total| format!("(전체 {total}자 중 앞부분)"),
    routine_channel: "루틴",
    decision_channel: "허락한 일",
    owner_only: "주인에게만",
    thread: |room| format!("{room} 스레드"),
    tool_row: "도구",
    decided_by: |owner, allowed| {
        if allowed {
            format!("{owner} 님이 허락했어요")
        } else {
            format!("{owner} 님이 거절했어요")
        }
    },
    sent_by_dm: "확인할 일을 DM으로 보냈어요.",
    card_on_pc: |host, log_in| {
        let kind = if log_in { "로그인" } else { "입력" };
        format!("PC의 dotori 창에 {host} {kind} 카드를 띄웠어요.")
    },
    input_card_on_pc: "PC의 dotori 창에 입력 카드를 띄웠어요.",
    sandboxed: "샌드박스 안에서 실행해요. dotori의 금고와 기록은 보이지 않아요.",
    log_in_with_kept: |host, site| format!("금고의 {host} ({site}) 계정으로 로그인해도 될까요?"),
    log_in_note: |subject, host, path| {
        format!(
            "{subject} {host}{path} 로그인 칸에 직접 채워 넣어요. 대화하는 AI는 비밀번호를 보지 못해요."
        )
    },
    choose_account: "계정 고르기",
    account: "계정",
    stop_asking: "이 계정은 다음부터 묻지 않기",
    log_in_and_stop_asking: "로그인, 다음부터 묻지 않기",
    log_in_as: |id| format!("{id} 계정으로 로그인"),
    log_in_without_id: "아이디 없는 계정으로 로그인",
    without_id: "아이디 없이",
    rest_on_pc: "나머지는 PC의 dotori 창에서 골라 주세요",
};

pub(crate) const ENGLISH: Chat = Chat {
    copy: "Copy",
    copied: "Copied",
    allowed: "Allowed",
    declined: "Declined",
    expired: "I can't do this anymore. Ask me again.",
    deciding: "Working on it",
    erase_hint: "Allow and I'll erase it on every PC. You can't undo this.",
    run_hint: |_| String::from("Allow and I'll run this command right away."),
    write_hint: |_| String::from("Allow and I'll change the file right away."),
    allow_hint: "Allow and I'll run it right away.",
    decline: "Decline",
    allow: "Allow",
    asking: |_| String::from("Can I do this?"),
    show_whole: "Show all",
    in_browser: "Working in the browser window",
    in_browser_hint: || {
        String::from(
            "When you're done, select \"I'm done\" or close the browser window. I'll take it from there.",
        )
    },
    show_browser: "Show window",
    browser_done: "I'm done",
    log_in_to: |site| format!("Log in to {site}"),
    login_name: "Username",
    login_secret: "Password",
    login_wrong: "That username or password didn't work. Try again.",
    login_kept: "I pass this only to the page and keep it out of the chat history.",
    login_in_browser: "Use the browser window",
    cancel: "Cancel",
    log_in: "Log in",
    finish_in_browser: "Finish this in the browser",
    preview_hint: "Or click the preview.",
    later: "Hide",
    open_browser: "Open browser",
    starting: "dotori is starting.",
    could_not_start: "Couldn't start dotori. Please open the app again.",
    closed: "dotori closed the connection. Reconnecting.",
    unreachable: "Can't reach dotori. Reconnecting.",
    not_connected: "Couldn't connect to dotori. Trying again.",
    keys_hint: "Enter sends, Shift+Enter adds a line.",
    thinking: "Thinking",
    say_to: || format!("Message {}", identity::name()),
    welcome: "What can I help with?",
    one_chat: &["Talk here or in Slack,", "it's the same conversation."],
    ran: "Done",
    ran_failed: "It ran but failed",
    timed_out: "Didn't finish in time",
    declined_not_run: "Declined, so I didn't run it",
    already_settled: "Already settled, so I didn't run it",
    command_done: "Command finished",
    command_exited: |code| format!("Command exited with code {code}"),
    command_signalled: "Command stopped by a signal",
    command_too_slow: |seconds| format!("Didn't finish in {seconds} seconds, so I stopped it"),
    stdout: "Output",
    stderr: "Error output",
    only_the_end: "(showing only the end)",
    left_running: "(a process the command left behind still writes here)",
    run_shell: "Run command",
    write_file: "Write file",
    edit_file: "Edit file",
    erase_records: "Erase records",
    command_field: "Command",
    folder_field: "Folder",
    seconds_field: "Time limit (s)",
    path_field: "File",
    content_field: "Content",
    old_field: "Old text",
    new_field: "New text",
    all_field: "Replace all",
    cut_note: |total| format!("(first part of {total} characters)"),
    routine_channel: "Routine",
    decision_channel: "Approvals",
    owner_only: "Only you",
    thread: |room| format!("{room} thread"),
    tool_row: "Tool",
    decided_by: |owner, allowed| {
        if allowed {
            format!("{owner} allowed it")
        } else {
            format!("{owner} declined it")
        }
    },
    sent_by_dm: "I sent you a DM to check.",
    card_on_pc: |host, log_in| {
        let kind = if log_in { "login" } else { "input" };
        format!("I opened a {kind} card for {host} in the dotori window on your PC.")
    },
    input_card_on_pc: "I opened an input card in the dotori window on your PC.",
    sandboxed: "It runs in a sandbox, where dotori's vault and records stay out of sight.",
    log_in_with_kept: |host, site| {
        format!("Can I log in with the {host} ({site}) account from your vault?")
    },
    log_in_note: |_, host, path| {
        format!("I fill in the login on {host}{path} myself. The AI model never sees the password.")
    },
    choose_account: "Choose an account",
    account: "Account",
    stop_asking: "Don't ask again for this account",
    log_in_and_stop_asking: "Log in and don't ask again",
    log_in_as: |id| format!("Log in as {id}"),
    log_in_without_id: "Log in with the account without a username",
    without_id: "no username",
    rest_on_pc: "Pick from the rest in the dotori window on your PC.",
};
