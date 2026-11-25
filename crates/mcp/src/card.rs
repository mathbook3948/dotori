//! The cards dotori's browser shows the owner in the chat window: one that asks them to fill a
//! page's fields, and one that asks whether to log in with an account the vault keeps. The model
//! names the page's elements by their numbers; what the owner types goes from the window to the
//! page and the vault, never back to the model.

use std::fmt;

use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use serde_json::json;

const MAX_FIELDS: usize = 8;
const MAX_CHECKS: usize = 4;
const MAX_LINKS: usize = 6;
const MAX_LABEL: usize = 40;
const MAX_TITLE: usize = 60;
const MAX_HINT: usize = 80;
const MAX_SITE: usize = 30;
const MAX_VALUE: usize = 1024;
const HIDDEN_KEPT: usize = 32;
const SHORTEST_HIDDEN: usize = 4;
const SHOWN_ID: usize = 16;
const HIDDEN_MARK: &str = "[hidden]";

/// What a field of a card takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Text,
    Secret,
    Code,
}

/// A page element a card shows, by its number on the page and the label the card gives it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct Named {
    #[serde(rename = "ref")]
    pub(crate) at: u64,
    pub(crate) label: String,
}

/// A field the owner types into.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct Field {
    #[serde(rename = "ref")]
    pub(crate) at: u64,
    pub(crate) label: String,
    pub(crate) kind: Kind,
}

/// What a page element must be for the card to use it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Role {
    Text,
    Secret,
    Code,
    Check,
    Press,
}

impl From<Kind> for Role {
    fn from(kind: Kind) -> Self {
        match kind {
            Kind::Text => Self::Text,
            Kind::Secret => Self::Secret,
            Kind::Code => Self::Code,
        }
    }
}

/// A card that asks the owner to fill a page's fields, as the model asked for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FillSpec {
    pub(crate) title: String,
    pub(crate) hint: Option<String>,
    pub(crate) site: Option<String>,
    pub(crate) fields: Vec<Field>,
    pub(crate) checks: Vec<Named>,
    pub(crate) submit: Named,
    pub(crate) links: Vec<Named>,
    pub(crate) keep: bool,
}

impl FillSpec {
    /// Reads what the model asked for, refusing a card dotori would not show.
    pub(crate) fn parse(input: &Value) -> Result<Self, String> {
        let title = text(input.get("title"), "title", MAX_TITLE)?;
        let hint = optional_text(input.get("hint"), "hint", MAX_HINT)?;
        let site = optional_text(input.get("site"), "site", MAX_SITE)?;
        let fields = list(input.get("fields"), "fields", MAX_FIELDS)?
            .iter()
            .map(|field| {
                let named = named(field, "fields")?;
                let kind = match field.get("kind") {
                    None => Kind::Text,
                    Some(kind) => serde_json::from_value(kind.clone()).map_err(|_| {
                        format!("`kind` of field [{}] is text, secret or code", named.at)
                    })?,
                };
                Ok(Field {
                    at: named.at,
                    label: named.label,
                    kind,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let checks = list(input.get("checks"), "checks", MAX_CHECKS)?
            .iter()
            .map(|check| named(check, "checks"))
            .collect::<Result<Vec<_>, String>>()?;
        let submit = named(input.get("submit").ok_or("`submit` is required")?, "submit")?;
        let links = list(input.get("links"), "links", MAX_LINKS)?
            .iter()
            .map(|link| named(link, "links"))
            .collect::<Result<Vec<_>, String>>()?;
        let keep = match input.get("keep") {
            None => false,
            Some(keep) => keep.as_bool().ok_or("`keep` is true or false")?,
        };
        let spec = Self {
            title,
            hint,
            site,
            fields,
            checks,
            submit,
            links,
            keep,
        };
        let mut seen: Vec<u64> = Vec::new();
        for (at, _) in spec.roles() {
            if seen.contains(&at) {
                return Err(format!("[{at}] is named twice"));
            }
            seen.push(at);
        }
        if spec.keep {
            if spec.site.is_none() {
                return Err(String::from(
                    "`site` is required with `keep`: the name the owner knows the site by",
                ));
            }
            if spec.of(Kind::Secret).count() != 1 || spec.of(Kind::Text).count() > 1 {
                return Err(String::from(
                    "a card that keeps an account has one secret field for the password and at most one text field for the id",
                ));
            }
        }
        Ok(spec)
    }

    fn of(&self, kind: Kind) -> impl Iterator<Item = &Field> {
        self.fields.iter().filter(move |field| field.kind == kind)
    }

    /// Every element the card uses and what it must be.
    pub(crate) fn roles(&self) -> Vec<(u64, Role)> {
        self.fields
            .iter()
            .map(|field| (field.at, Role::from(field.kind)))
            .chain(self.checks.iter().map(|check| (check.at, Role::Check)))
            .chain(std::iter::once((self.submit.at, Role::Press)))
            .chain(self.links.iter().map(|link| (link.at, Role::Press)))
            .collect()
    }

    /// What the card tells the owner about where their words go.
    pub(crate) fn note(&self) -> &'static str {
        if self.keep {
            "금고에 안전하게 보관돼요. 대화하는 AI는 보지 못해요."
        } else if self.fields.iter().any(|field| field.kind == Kind::Code) {
            "인증번호는 보관하지 않아요."
        } else {
            "대화하는 AI는 적은 내용을 보지 못해요."
        }
    }

    /// The card as the chat window draws it, with the host the browser is on now and whether
    /// each checkbox is ticked on the page.
    pub(crate) fn shown(&self, id: u64, host: &str, ticked: &[(u64, bool)]) -> Value {
        let checks: Vec<Value> = self
            .checks
            .iter()
            .map(|check| {
                let on = ticked.iter().any(|(at, on)| *at == check.at && *on);
                json!({ "ref": check.at, "label": check.label, "on": on })
            })
            .collect();
        json!({
            "id": id,
            "kind": "fill",
            "title": self.title,
            "hint": self.hint,
            "host": host,
            "fields": self.fields,
            "checks": checks,
            "submit": self.submit,
            "links": self.links,
            "note": self.note(),
        })
    }

    fn label_of(&self, at: u64) -> Option<&str> {
        self.fields
            .iter()
            .find(|field| field.at == at)
            .map(|field| field.label.as_str())
            .or_else(|| {
                self.checks
                    .iter()
                    .chain(&self.links)
                    .chain(std::iter::once(&self.submit))
                    .find(|named| named.at == at)
                    .map(|named| named.label.as_str())
            })
    }
}

/// Which elements of the page now take a kept account: the id field if the page shows one, the
/// password field, and the button that logs in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LogInSpec {
    pub(crate) id: Option<u64>,
    pub(crate) password: u64,
    pub(crate) submit: u64,
}

impl LogInSpec {
    pub(crate) fn parse(input: &Value) -> Result<Self, String> {
        let number = |key: &str| -> Result<Option<u64>, String> {
            match input.get(key) {
                None | Some(Value::Null) => Ok(None),
                Some(at) => at
                    .as_u64()
                    .filter(|at| *at > 0)
                    .map(Some)
                    .ok_or_else(|| format!("`{key}` is an element number from read_page")),
            }
        };
        let spec = Self {
            id: number("id")?,
            password: number("password")?.ok_or("`password` is required")?,
            submit: number("submit")?.ok_or("`submit` is required")?,
        };
        let roles = spec.roles();
        if roles
            .iter()
            .enumerate()
            .any(|(n, (at, _))| roles[..n].iter().any(|(seen, _)| seen == at))
        {
            return Err(String::from("each element is named once"));
        }
        Ok(spec)
    }

    pub(crate) fn roles(&self) -> Vec<(u64, Role)> {
        self.id
            .map(|id| (id, Role::Text))
            .into_iter()
            .chain([(self.password, Role::Secret), (self.submit, Role::Press)])
            .collect()
    }
}

/// What the owner did with a card in the chat window.
#[derive(Clone, PartialEq, Eq)]
pub(crate) enum Answer {
    Later,
    Press {
        at: u64,
        values: Vec<(u64, String)>,
        checks: Vec<(u64, bool)>,
    },
    Allow {
        trust: bool,
    },
    Decline,
}

impl fmt::Debug for Answer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Later => f.write_str("Later"),
            Self::Press { at, values, checks } => f
                .debug_struct("Press")
                .field("at", at)
                .field(
                    "filled",
                    &values.iter().map(|(at, _)| at).collect::<Vec<_>>(),
                )
                .field("checks", checks)
                .finish(),
            Self::Allow { trust } => f.debug_struct("Allow").field("trust", trust).finish(),
            Self::Decline => f.write_str("Decline"),
        }
    }
}

/// The card a window's line answers, and how, if it answers one.
pub(crate) fn answer_of(input: &Value) -> Option<(u64, Answer)> {
    let id = input.get("id").and_then(Value::as_u64)?;
    let flag = |key: &str| input.get(key).and_then(Value::as_bool);
    if flag("later") == Some(true) {
        return Some((id, Answer::Later));
    }
    if let Some(allow) = flag("allow") {
        return Some((
            id,
            if allow {
                Answer::Allow {
                    trust: flag("trust") == Some(true),
                }
            } else {
                Answer::Decline
            },
        ));
    }
    let at = input.get("press").and_then(Value::as_u64)?;
    let pairs = |key: &str| {
        input
            .get(key)
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .filter_map(|pair| Some((pair.get("ref")?.as_u64()?, pair)))
            .collect::<Vec<_>>()
    };
    let values = pairs("values")
        .into_iter()
        .filter_map(|(at, pair)| {
            let value = pair.get("value")?.as_str()?;
            (value.len() <= MAX_VALUE).then(|| (at, value.to_owned()))
        })
        .collect();
    let checks = pairs("checks")
        .into_iter()
        .filter_map(|(at, pair)| Some((at, pair.get("on")?.as_bool()?)))
        .collect();
    Some((id, Answer::Press { at, values, checks }))
}

/// What a card came to, as the chat window draws it. It heads what the model is told, as JSON on
/// the first line, so every window that shows the conversation can draw the same thing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "card", rename_all = "snake_case")]
pub enum Shown {
    /// The owner pressed the card's button or one of its links; dotori filled `filled` and kept
    /// the account for the site named `kept`.
    Pressed {
        title: String,
        pressed: String,
        link: bool,
        filled: Vec<String>,
        codes: bool,
        kept: Option<String>,
    },
    /// The owner put the card off, or it waited too long.
    Closed { title: String, lapsed: bool },
    /// dotori logged in with the account the vault keeps for `site`.
    LoggedIn { site: String },
    /// The owner did not let dotori log in with that account.
    Declined { site: String },
}

impl Shown {
    /// What a text from a card says came of it, and the rest of the text.
    pub fn read(text: &str) -> Option<(Self, &str)> {
        let (head, rest) = text.split_once('\n').unwrap_or((text, ""));
        Some((serde_json::from_str(head).ok()?, rest))
    }

    fn head(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

/// What the model is told after the owner pressed `at` on the card `spec`, with the fields
/// dotori filled, the checkboxes it set, and the site it kept the account for.
pub(crate) fn pressed(
    spec: &FillSpec,
    at: u64,
    filled: &[u64],
    checks: &[(u64, bool)],
    kept: Result<Option<&str>, &str>,
) -> String {
    let label = spec.label_of(at).unwrap_or_default().to_owned();
    let link = at != spec.submit.at;
    let filled_labels: Vec<String> = filled
        .iter()
        .filter_map(|at| spec.label_of(*at))
        .map(str::to_owned)
        .collect();
    let codes = filled.iter().all(|at| {
        spec.fields
            .iter()
            .any(|field| field.at == *at && field.kind == Kind::Code)
    }) && !filled.is_empty();
    let shown = Shown::Pressed {
        title: spec.title.clone(),
        pressed: label.clone(),
        link,
        filled: filled_labels.clone(),
        codes,
        kept: kept.ok().flatten().map(str::to_owned),
    };
    let mut said = format!("주인이 '{label}'{} 눌렀어요.", object(&label));
    if link {
        said.push_str(" 칸은 채우지 않고 그 링크만 눌렀어요.");
    } else if !filled_labels.is_empty() {
        said.push_str(&format!(
            " 채운 칸: {} (값은 보이지 않아요).",
            filled_labels.join(", ")
        ));
    }
    let set: Vec<String> = checks
        .iter()
        .filter_map(|(at, on)| {
            let label = spec.label_of(*at)?;
            Some(format!("{label} {}", if *on { "켬" } else { "끔" }))
        })
        .collect();
    if !set.is_empty() {
        said.push_str(&format!(" 체크: {}.", set.join(", ")));
    }
    match kept {
        Ok(Some(site)) => said.push_str(&format!(
            " 금고에 {site} 계정을 넣었어요. 다음부터는 fill_login으로 로그인해요."
        )),
        Ok(None) => {}
        Err(error) => said.push_str(&format!(" 계정은 금고에 넣지 못했어요: {error}")),
    }
    format!("{}\n{said}", shown.head())
}

/// What the model is told when a card closed without the owner pressing anything.
pub(crate) fn closed(title: &str, lapsed: bool) -> String {
    let shown = Shown::Closed {
        title: title.to_owned(),
        lapsed,
    };
    let said = if lapsed {
        "주인이 카드에 답하지 않아서 카드를 닫았어요. 아무것도 채우지 않았어요."
    } else {
        "주인이 '나중에 할게요'를 눌렀어요. 아무것도 채우지 않았어요. 다시 묻지 말고 주인이 부탁할 때 해요."
    };
    format!("{}\n{said}", shown.head())
}

/// What the model is told after dotori logged in with the account the vault keeps for `site`.
pub(crate) fn logged_in(site: &str, asked: bool) -> String {
    let shown = Shown::LoggedIn {
        site: site.to_owned(),
    };
    let said = if asked {
        format!(
            "주인이 허락해서 금고의 {site} 계정으로 로그인 칸을 채우고 눌렀어요 (값은 보이지 않아요)."
        )
    } else {
        format!(
            "주인이 묻지 않기로 해 둔 {site} 계정이라, 금고의 값으로 로그인 칸을 채우고 눌렀어요 (값은 보이지 않아요)."
        )
    };
    format!("{}\n{said}", shown.head())
}

/// What the model is told when the owner did not let dotori log in with the account for `site`.
pub(crate) fn declined(site: &str) -> String {
    let shown = Shown::Declined {
        site: site.to_owned(),
    };
    format!(
        "{}\n주인이 금고의 {site} 계정으로 로그인하는 것을 거절했어요. 아무것도 채우지 않았어요.",
        shown.head()
    )
}

fn object(word: &str) -> &'static str {
    match word.chars().last() {
        Some(last @ '가'..='힣') => {
            if (u32::from(last) - u32::from('가')) % 28 == 0 {
                "를"
            } else {
                "을"
            }
        }
        _ => "을(를)",
    }
}

/// An account the owner linked: the site and its origin, the id and password the page takes, and
/// the labels the card gave them. `trusted` is set once the owner said not to ask again.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub site: String,
    pub origin: String,
    pub id_label: Option<String>,
    pub id: Option<String>,
    pub password_label: String,
    pub password: String,
    #[serde(default)]
    pub trusted: bool,
}

impl fmt::Debug for Account {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Account")
            .field("site", &self.site)
            .field("origin", &self.origin)
            .field("trusted", &self.trusted)
            .finish_non_exhaustive()
    }
}

/// Where the browser keeps the accounts the owner linked, one per origin.
pub trait Accounts: Send + Sync + fmt::Debug {
    fn find(&self, origin: &str) -> Result<Option<Account>, String>;

    fn keep(&self, account: &Account) -> Result<(), String>;
}

/// Whether an account may be kept for, or filled into, a page at `origin`: only a page sent over
/// HTTPS has an origin dotori can hold a site to.
pub(crate) fn keepable(origin: &str) -> bool {
    origin
        .strip_prefix("https://")
        .is_some_and(|host| !host.is_empty() && !host.contains('/'))
}

/// An id as a card shows it, short enough to read at a glance: the part of an email address
/// before its `@`, cut to a few characters.
pub fn shortened(id: &str) -> String {
    let local = id.split_once('@').map_or(id, |(local, _)| local);
    if local.chars().count() <= SHOWN_ID {
        return local.to_owned();
    }
    let cut: String = local.chars().take(SHOWN_ID - 1).collect();
    format!("{cut}…")
}

/// The values dotori typed into pages, hidden from everything its browser tools return.
#[derive(Default)]
pub(crate) struct Hidden {
    values: Vec<String>,
}

impl fmt::Debug for Hidden {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Hidden")
            .field("count", &self.values.len())
            .finish()
    }
}

impl Hidden {
    /// Hides `value` from now on, and the way it reads inside an address. Values too short to
    /// hide without hiding ordinary words are left alone.
    pub(crate) fn add(&mut self, value: &str) {
        let value = value.trim();
        if value.chars().count() < SHORTEST_HIDDEN {
            return;
        }
        for form in [value.to_owned(), escaped(value)] {
            if !self.values.contains(&form) {
                if self.values.len() == HIDDEN_KEPT {
                    self.values.remove(0);
                }
                self.values.push(form);
            }
        }
    }

    pub(crate) fn scrub(&self, text: &str) -> String {
        let mut longest_first: Vec<&String> = self.values.iter().collect();
        longest_first.sort_by_key(|value| std::cmp::Reverse(value.len()));
        let mut text = text.to_owned();
        for value in longest_first {
            if text.contains(value.as_str()) {
                text = text.replace(value.as_str(), HIDDEN_MARK);
            }
        }
        text
    }
}

fn escaped(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn text(value: Option<&Value>, key: &str, longest: usize) -> Result<String, String> {
    optional_text(value, key, longest)?.ok_or_else(|| format!("`{key}` is required"))
}

fn optional_text(
    value: Option<&Value>,
    key: &str,
    longest: usize,
) -> Result<Option<String>, String> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let text: String = value
        .as_str()
        .ok_or_else(|| format!("`{key}` is text"))?
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if text.is_empty() {
        return Ok(None);
    }
    if text.chars().count() > longest {
        return Err(format!("`{key}` is at most {longest} characters"));
    }
    Ok(Some(text))
}

fn list<'a>(value: Option<&'a Value>, key: &str, most: usize) -> Result<&'a [Value], String> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(&[]);
    };
    let items = value
        .as_array()
        .ok_or_else(|| format!("`{key}` is a list"))?;
    if items.len() > most {
        return Err(format!("`{key}` holds at most {most}"));
    }
    Ok(items)
}

fn named(value: &Value, key: &str) -> Result<Named, String> {
    let at = value
        .get("ref")
        .and_then(Value::as_u64)
        .filter(|at| *at > 0)
        .ok_or_else(|| format!("each of `{key}` has a `ref`, an element number from read_page"))?;
    let label = text(value.get("label"), "label", MAX_LABEL)
        .map_err(|error| format!("[{at}] in `{key}`: {error}"))?;
    Ok(Named { at, label })
}

#[cfg(test)]
#[path = "card.test.rs"]
mod tests;
