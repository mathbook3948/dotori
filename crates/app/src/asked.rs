//! What a card that waits for the owner says, the same in the chat window and in a chat room.

pub(crate) const SHOWN_VALUE: usize = 600;
/// What a card says under a command that would run in the sandbox.
pub(crate) const SANDBOXED: &str =
    "샌드박스 안에서 실행해요. dotori의 금고와 기록은 보이지 않아요.";
/// What a card says once its call no longer waits.
pub(crate) const LAPSED: &str = "이제 할 수 없어요. 다시 부탁해 주세요";
const HIDDEN_FIELDS: [&str; 2] = ["records", "carriers"];

/// What the owner calls one of dotori's own tools that waits for their OK.
pub(crate) fn own_tool(tool: &str) -> Option<&'static str> {
    match tool {
        "run_shell" => Some("명령 실행"),
        "write_file" => Some("파일 쓰기"),
        "edit_file" => Some("파일 고치기"),
        "erase_records" => Some("기록 지우기"),
        _ => None,
    }
}

/// The label a field of a waiting call shows under, in the owner's words where dotori knows the
/// field.
pub(crate) fn field_label(key: &str) -> &str {
    match key {
        "command" => "명령",
        "folder" => "폴더",
        "seconds" => "시간 제한(초)",
        "path" => "파일",
        "content" => "내용",
        "old" => "바꿀 글",
        "new" => "새 글",
        "all" => "모두 바꾸기",
        other => other,
    }
}

/// The fields of a waiting call as the card shows them, and whether any was shortened. A forget
/// shows what the records say, not their numbers.
pub(crate) fn asked_fields(tool: &str, input: &str, whole: bool) -> (Vec<(String, String)>, bool) {
    let hidden: &[&str] = if tool == "erase_records" {
        &HIDDEN_FIELDS
    } else {
        &[]
    };
    let fields: Vec<(String, String)> = match serde_json::from_str::<serde_json::Value>(input) {
        Ok(serde_json::Value::Object(fields)) if !fields.is_empty() => fields
            .into_iter()
            .filter(|(key, _)| !hidden.contains(&key.as_str()))
            .map(|(key, value)| {
                let shown = match value {
                    serde_json::Value::String(text) => text,
                    other => other.to_string(),
                };
                (key, shown)
            })
            .collect(),
        _ => vec![(String::from("input"), input.to_owned())],
    };
    let mut cut = false;
    let fields = fields
        .into_iter()
        .map(|(key, value)| {
            let (shown, shortened) = shown_value(&value, whole);
            cut |= shortened;
            (key, shown)
        })
        .collect();
    (fields, cut)
}

fn shown_value(value: &str, whole: bool) -> (String, bool) {
    let mut lines: Vec<&str> = Vec::new();
    for line in value.lines() {
        if line.trim().is_empty() && lines.last().is_some_and(|last| last.trim().is_empty()) {
            continue;
        }
        lines.push(line);
    }
    let joined = lines.join("\n");
    let total = joined.chars().count();
    if whole || total <= SHOWN_VALUE {
        return (joined, false);
    }
    let cut: String = joined.chars().take(SHOWN_VALUE).collect();
    (format!("{cut}\n(전체 {total}자 중 앞부분)"), true)
}

/// The assistant's `name` followed by the subject particle that fits its last sound, such as
/// "dotori가" or "별이".
pub(crate) fn subject(name: &str) -> String {
    let particle = if crate::window::identity::ends_closed(name) {
        "이"
    } else {
        "가"
    };
    format!("{name}{particle}")
}

/// The title of a card for a call that waits, where `subject` names the assistant with its
/// particle, such as "dotori가".
pub(crate) fn title(subject: &str) -> String {
    format!("{subject} 이 일을 해도 될지 물어요")
}

/// What allowing the call `tool` does, said beside the buttons.
pub(crate) fn hint(tool: &str, subject: &str) -> String {
    match tool {
        "erase_records" => String::from("허락하면 모든 PC에서 지워요. 되돌릴 수 없어요"),
        "run_shell" => format!("허락하면 {subject} 이 명령을 바로 실행해요"),
        "write_file" | "edit_file" => format!("허락하면 {subject} 파일을 바로 바꿔요"),
        _ => String::from("허락하면 바로 실행해요"),
    }
}

/// The title of a card that asks to log in with a kept account.
pub(crate) fn log_in_title(host: &str, site: &str) -> String {
    format!("금고의 {host} ({site}) 계정으로 로그인해도 될까요?")
}

/// An account as a log-in card lists it.
pub(crate) fn log_in_account(host: &str, site: &str, id: &str) -> String {
    format!("{host} ({site}), {id}")
}

/// Where a log-in card's account goes, and that the model never sees its password.
pub(crate) fn log_in_note(subject: &str, host: &str, path: &str) -> String {
    format!(
        "{subject} {host}{path} 로그인 칸에 직접 채워 넣어요. 대화하는 AI는 비밀번호를 보지 못해요."
    )
}

#[cfg(test)]
#[path = "asked.test.rs"]
mod tests;
