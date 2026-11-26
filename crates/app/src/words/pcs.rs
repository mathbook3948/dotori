use crate::window::identity;
use crate::window::identity::Particle;

/// Strings of the other PCs card and the PC list.
#[derive(Debug)]
pub(crate) struct Pcs {
    pub(crate) title: &'static str,
    /// The caption under the title, which calls the assistant by its name.
    pub(crate) about: fn() -> String,
    /// This PC's row, given its name.
    pub(crate) this_pc: fn(&str) -> String,
    /// How the other PCs reach this one, given the address they use.
    pub(crate) reachable_at: fn(&str) -> String,
    pub(crate) reachable: &'static str,
    pub(crate) not_listening: &'static str,
    pub(crate) name_section: &'static str,
    pub(crate) name_hint: &'static str,
    pub(crate) invite_section: &'static str,
    pub(crate) make_invite: &'static str,
    pub(crate) copy: &'static str,
    pub(crate) copied: &'static str,
    /// The hint under an invite code, given the minutes it stays open.
    pub(crate) invite_hint: fn(u64) -> String,
    pub(crate) join_section: &'static str,
    pub(crate) code_placeholder: &'static str,
    pub(crate) address_placeholder: &'static str,
    pub(crate) join: &'static str,
    pub(crate) joining: &'static str,
    pub(crate) unreadable_code: &'static str,
    pub(crate) no_address: &'static str,
    pub(crate) join_stopped: &'static str,
    /// The note after linking with the PC of the given name.
    pub(crate) joined: fn(&str) -> String,
    pub(crate) linked_section: &'static str,
    pub(crate) no_pcs: &'static str,
    pub(crate) connected: &'static str,
    /// A PC's state when it dropped the given number of seconds ago.
    pub(crate) dropped: fn(u64) -> String,
    pub(crate) never_connected: &'static str,
    pub(crate) remove: &'static str,
}

pub(crate) const KOREAN: Pcs = Pcs {
    title: "여러 PC",
    about: || {
        format!(
            "다른 PC와 묶으면 어디서 말을 걸어도 같은 {} 같은 기억으로 이어져요. 주고받는 내용은 모두 암호화해요.",
            identity::called(Particle::With)
        )
    },
    this_pc: |name| format!("{name} (이 PC)"),
    reachable_at: |address| {
        let open = !identity::ends_closed(address)
            || matches!(address.chars().last(), Some('1' | '7' | '8' | 'l' | 'L'));
        let to = if open { "로" } else { "으로" };
        format!("다른 PC가 {address}{to} 찾아와요")
    },
    reachable: "다른 PC가 찾아올 수 있어요",
    not_listening: "다른 PC에 연결만 해요. 찾아오게 하려면 [peers]를 켜 주세요",
    name_section: "이 PC의 이름",
    name_hint: "다른 PC에는 이 이름으로 보여요.",
    invite_section: "이 PC로 초대하기",
    make_invite: "연결 코드 만들기",
    copy: "복사",
    copied: "복사했어요",
    invite_hint: |minutes| {
        format!("다른 PC의 설정에 붙여 넣어 주세요. {minutes}분 동안 한 번만 쓸 수 있어요.")
    },
    join_section: "다른 PC에 합류하기",
    code_placeholder: "다른 PC에서 만든 연결 코드",
    address_placeholder: "주소:포트 (코드에 있으면 비워 두세요)",
    join: "이 코드로 묶기",
    joining: "묶는 중…",
    unreadable_code: "연결 코드를 읽지 못했어요. 다시 붙여 넣어 주세요.",
    no_address: "코드에 주소가 없어요. 그 PC의 주소와 포트를 적어 주세요.",
    join_stopped: "묶는 중에 멈췄어요. 다시 묶어 주세요.",
    joined: |name| {
        let with = if identity::ends_closed(name) {
            "과"
        } else {
            "와"
        };
        format!(
            "{name}{with} 묶었어요. 그 PC에서 {} 켜져 있으면 곧 연결돼요.",
            identity::called(Particle::Subject)
        )
    },
    linked_section: "묶은 PC",
    no_pcs: "아직 묶은 PC가 없어요",
    connected: "연결됨",
    dropped: |passed| match passed {
        0..60 => String::from("방금 끊김"),
        60..3600 => format!("{}분 전 끊김", passed / 60),
        3600..86400 => format!("{}시간 전 끊김", passed / 3600),
        _ => format!("{}일 전 끊김", passed / 86400),
    },
    never_connected: "아직 연결되지 않음",
    remove: "빼기",
};

pub(crate) const ENGLISH: Pcs = Pcs {
    title: "Your PCs",
    about: || {
        format!(
            "Link your other PCs, and {} keeps the same memory wherever you talk. Everything sent between them is encrypted.",
            identity::name()
        )
    },
    this_pc: |name| format!("{name} (this PC)"),
    reachable_at: |address| format!("Your other PCs reach this one at {address}."),
    reachable: "Your other PCs can reach this one.",
    not_listening: "This PC only connects out. Turn on [peers] to let others reach it.",
    name_section: "This PC's name",
    name_hint: "Your other PCs see this name.",
    invite_section: "Invite to this PC",
    make_invite: "Create link code",
    copy: "Copy",
    copied: "Copied",
    invite_hint: |minutes| {
        let unit = if minutes == 1 { "minute" } else { "minutes" };
        format!(
            "Paste it into the other PC's settings. It works once, for the next {minutes} {unit}."
        )
    },
    join_section: "Join another PC",
    code_placeholder: "Link code from the other PC",
    address_placeholder: "Address:port, if needed",
    join: "Link with this code",
    joining: "Linking…",
    unreadable_code: "I couldn't read that link code. Paste it again.",
    no_address: "The code has no address. Enter that PC's address and port.",
    join_stopped: "Linking stopped partway. Try again.",
    joined: |name| {
        format!(
            "Linked with {name}. It connects as soon as {} runs there.",
            identity::name()
        )
    },
    linked_section: "Linked PCs",
    no_pcs: "No PCs linked yet",
    connected: "Connected",
    dropped: |passed| {
        let (count, unit) = match passed {
            0..60 => return String::from("Disconnected just now"),
            60..3600 => (passed / 60, "minute"),
            3600..86400 => (passed / 3600, "hour"),
            _ => (passed / 86400, "day"),
        };
        let plural = if count == 1 { "" } else { "s" };
        format!("Disconnected {count} {unit}{plural} ago")
    },
    never_connected: "Not connected yet",
    remove: "Remove",
};
