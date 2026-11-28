use std::env;
use std::fmt;
use std::fs;
use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use dotori_connectors::channel::card::Cards;
use dotori_connectors::channel::discord::DiscordChannel;
use dotori_connectors::channel::discord::DiscordConfig;
use dotori_connectors::channel::slack::SlackChannel;
use dotori_connectors::channel::slack::SlackConfig;
use dotori_connectors::channel::telegram::TelegramChannel;
use dotori_connectors::channel::telegram::TelegramConfig;
use dotori_connectors::llm::DEFAULT_SYSTEM;
use dotori_connectors::llm::FALLBACK_LANGUAGE;
use dotori_connectors::llm::anthropic::Anthropic;
use dotori_connectors::llm::anthropic::AnthropicConfig;
use dotori_connectors::llm::openai::OpenAi;
use dotori_connectors::llm::openai::OpenAiConfig;
use dotori_connectors::sub_session::SUB_SESSION_SYSTEM;
use dotori_connectors::sub_session::llm::LLM_SUB_SESSION_SYSTEM;
use dotori_core::llm::Model;
use dotori_mcp::memory::EmbeddingConfig;
use dotori_mcp::relay::Reach;
use dotori_mcp::relay::RelayConfig;
use serde::Deserialize;
use serde::Serialize;

use crate::replace::replace;
use crate::words::words;

/// Where the Anthropic API key is when the settings do not say.
pub(crate) const ANTHROPIC_KEY: &str = "ANTHROPIC_API_KEY";
/// Where the Slack app token is when the settings do not say.
pub(crate) const SLACK_APP_KEY: &str = "SLACK_APP_TOKEN";
/// Where the Telegram bot token is when the settings do not say.
pub(crate) const TELEGRAM_KEY: &str = "TELEGRAM_BOT_TOKEN";
/// Where the Discord bot token is when the settings do not say.
pub(crate) const DISCORD_KEY: &str = "DISCORD_BOT_TOKEN";
/// Where the Slack bot token is when the settings do not say.
pub(crate) const SLACK_BOT_KEY: &str = "SLACK_BOT_TOKEN";
/// The name an OpenAI-compatible API key is kept under once the owner gives one in the settings
/// window, when the settings named none. It is dotori's own, so no key set for another tool is
/// sent in its place.
pub(crate) const MODEL_KEY: &str = "DOTORI_MODEL_API_KEY";

/// Why a model could not be built.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ModelError {
    /// The key named `name` is neither in the environment nor in the vault. The owner can still
    /// give it while dotori runs.
    MissingKey {
        name: String,
        what: &'static str,
    },
    Other(String),
}

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingKey { name, what } => write!(f, "set {name} to your {what}"),
            Self::Other(message) => f.write_str(message),
        }
    }
}

/// What the user sets in `settings.toml`. A missing file means every default.
/// Keys never live in the file. It names each key, which is read from the environment variable
/// of that name or else from the vault.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Settings {
    /// The language the windows speak: `ko`, `en`, or `system` to follow the system.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) language: Option<String>,
    #[serde(default)]
    pub(crate) model: ModelSettings,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) slack: Option<SlackSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) telegram: Option<TelegramSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) discord: Option<DiscordSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) browser: Option<BrowserSettings>,
    #[serde(default, alias = "sub_tasks", skip_serializing_if = "Option::is_none")]
    pub(crate) sub_sessions: Option<SubSessionSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) memory: Option<MemorySettings>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) mcp: Vec<RelaySettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) peers: Option<PeerSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) pc: Option<PcSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) window: Option<WindowSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) assistant: Option<AssistantSettings>,
}

/// What the owner calls the assistant and the picture it shows as. `icon` names an image the
/// settings window kept in the `icons` folder; without one the assistant shows as the acorn.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AssistantSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) icon: Option<String>,
}

/// The name the assistant goes by when the owner gives none.
pub(crate) const ASSISTANT_NAME: &str = "Dotori";
/// The longest name the assistant may be given, in characters.
pub(crate) const ASSISTANT_NAME_LONGEST: usize = 20;

/// The most characters this PC's name takes.
pub(crate) const PC_NAME_LONGEST: usize = 20;

/// What the windows of every PC call this one. It stays with this PC and is not shared.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PcSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) name: Option<String>,
}

impl AssistantSettings {
    /// The name the owner gave, or `dotori`.
    pub(crate) fn name_of(assistant: Option<&Self>) -> String {
        assistant
            .and_then(|assistant| assistant.name.as_deref())
            .map(|name| {
                name.chars()
                    .filter(|c| !c.is_control())
                    .take(ASSISTANT_NAME_LONGEST)
                    .collect::<String>()
                    .trim()
                    .to_owned()
            })
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| ASSISTANT_NAME.to_owned())
    }
}

/// How the windows look: a theme from DESIGN.md by name, and `light`, `dark` or `system` to follow
/// the system's look. Missing or unknown values keep the defaults.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WindowSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) theme: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) look: Option<String>,
}

/// How the owner's other PCs reach this one. With the section, this PC listens on `listen`, or
/// on every address at the usual port. `address` is what the others should dial, such as its
/// Tailscale name. `name` is where this PC's name was kept before `[pc]`, and is read only when
/// `[pc]` names none.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PeerSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) listen: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) name: Option<String>,
}

/// An MCP server made elsewhere that dotori relays, reached at `url` or started as `command`.
/// Its token goes by the name `token_env` gives, read from the environment or the vault, never
/// from the file.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RelaySettings {
    pub(crate) name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) token_env: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) command: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) tools: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) confirm: Vec<String>,
}

/// The Telegram channel: the bot token's variable and the owner's numeric user id.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TelegramSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) token_env: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) owner: Option<i64>,
}

/// The Discord channel: the bot token's variable and the owner's Discord user id.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DiscordSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) token_env: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) owner: Option<String>,
}

/// Which tools wait for the owner's OK.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Asked {
    tools: Vec<String>,
    servers: Vec<String>,
}

impl Asked {
    /// The listed names that none of `offered` matches, as a typo would leave them.
    pub(crate) fn unmatched<'a>(&self, offered: impl Iterator<Item = &'a str>) -> Vec<String> {
        let offered: Vec<&str> = offered.collect();
        self.tools
            .iter()
            .filter(|tool| !offered.contains(&tool.as_str()))
            .chain(
                self.servers
                    .iter()
                    .filter(|server| !offered.iter().any(|tool| tool.starts_with(server.as_str()))),
            )
            .cloned()
            .collect()
    }

    /// Makes every tool of the server `listed` names wait, for a name that matched no tool.
    pub(crate) fn close_server_of(&mut self, listed: &str) {
        if let Some((server, _)) = listed.split_once("__") {
            let prefix = format!("{server}__");
            if !self.servers.contains(&prefix) {
                self.servers.push(prefix);
            }
        }
    }

    /// Makes `tool` always wait for the owner, whatever the settings say.
    pub(crate) fn always(&mut self, tool: &str) {
        if !self.tools.iter().any(|asked| asked == tool) {
            self.tools.push(tool.to_owned());
        }
    }

    pub(crate) fn needs(&self, tool: &str) -> bool {
        self.tools.iter().any(|asked| asked == tool)
            || self
                .servers
                .iter()
                .any(|server| tool.starts_with(server.as_str()))
    }
}

/// How memory is searched. Without embeddings it matches words only.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MemorySettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) embeddings: Option<EmbeddingSettings>,
}

/// An embedding API in the OpenAI `/embeddings` shape, such as OpenAI's or a local Ollama.
/// Every recorded message is sent to it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EmbeddingSettings {
    pub(crate) base_url: String,
    pub(crate) model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) api_key_env: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) dimensions: Option<u32>,
}

/// What runs sub-sessions. Without a runner the main session's model runs them in dotori's
/// loop. Without a path a coding agent is found through `PATH`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SubSessionSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) runner: Option<RunnerChoice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) path: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum RunnerChoice {
    #[default]
    Llm,
    ClaudeCode,
    Codex,
}

/// Which browser the browser tools drive. Without a path dotori downloads and uses its own
/// Chrome for Testing, never the browser the owner uses.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BrowserSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) path: Option<PathBuf>,
    #[serde(default, rename = "headless", skip_serializing)]
    pub(crate) old_headless: Option<bool>,
}

/// Slack is on when this section is in the file and names the owner. The owner is a Slack user
/// id, such as `U012AB3CD`, and nobody else is heard.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SlackSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) app_token_env: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) bot_token_env: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) owner: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "provider", rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) enum ModelSettings {
    Anthropic {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        api_key_env: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        base_url: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_tokens: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        context_window: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        effort: Option<String>,
    },
    OpenaiCompatible {
        base_url: String,
        model: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        api_key_env: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_tokens: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        context_window: Option<u32>,
    },
}

impl Default for ModelSettings {
    fn default() -> Self {
        Self::Anthropic {
            api_key_env: None,
            model: None,
            base_url: None,
            max_tokens: None,
            context_window: None,
            effort: None,
        }
    }
}

/// The Slack part of the settings window, each field already trimmed. A blank variable name
/// means the default one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SlackForm {
    pub(crate) on: bool,
    pub(crate) owner: Option<String>,
    pub(crate) app_token_env: Option<String>,
    pub(crate) bot_token_env: Option<String>,
}

impl SlackForm {
    pub(crate) fn of(slack: Option<&SlackSettings>) -> Self {
        match slack {
            Some(slack) => Self {
                on: true,
                owner: slack.owner.clone(),
                app_token_env: slack.app_token_env.clone(),
                bot_token_env: slack.bot_token_env.clone(),
            },
            None => Self::default(),
        }
    }

    /// The `[slack]` section this form asks for, or `None` when Slack is off.
    pub(crate) fn settings(&self) -> Result<Option<SlackSettings>, String> {
        if !self.on {
            return Ok(None);
        }
        let owner = self
            .owner
            .as_deref()
            .map(str::to_ascii_uppercase)
            .ok_or_else(|| {
                let said = &words().settings;
                (said.channel_missing)("Slack", &[said.missing_member_id])
            })?;
        if !(owner.starts_with('U') || owner.starts_with('W'))
            || owner.len() < 3
            || !owner
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        {
            return Err(String::from(words().settings.slack_owner_shape));
        }
        for name in [&self.app_token_env, &self.bot_token_env]
            .into_iter()
            .flatten()
        {
            if !variable_name(name) {
                return Err((words().settings.not_a_key_name)(name));
            }
        }
        Ok(Some(SlackSettings {
            app_token_env: self.app_token_env.clone(),
            bot_token_env: self.bot_token_env.clone(),
            owner: Some(owner),
        }))
    }
}

/// The Telegram part of the settings window, each field already trimmed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TelegramForm {
    pub(crate) on: bool,
    pub(crate) owner: Option<String>,
    pub(crate) token_env: Option<String>,
}

impl TelegramForm {
    pub(crate) fn of(telegram: Option<&TelegramSettings>) -> Self {
        match telegram {
            Some(telegram) => Self {
                on: true,
                owner: telegram.owner.map(|owner| owner.to_string()),
                token_env: telegram.token_env.clone(),
            },
            None => Self::default(),
        }
    }

    /// The `[telegram]` section this form asks for, or `None` when Telegram is off.
    pub(crate) fn settings(&self) -> Result<Option<TelegramSettings>, String> {
        if !self.on {
            return Ok(None);
        }
        let owner = self
            .owner
            .as_deref()
            .ok_or_else(|| {
                let said = &words().settings;
                (said.channel_missing)("Telegram", &[said.missing_user_id])
            })?
            .parse::<i64>()
            .ok()
            .filter(|owner| *owner > 0)
            .ok_or(words().settings.telegram_owner_shape)?;
        if let Some(name) = &self.token_env
            && !variable_name(name)
        {
            return Err((words().settings.not_a_key_name)(name));
        }
        Ok(Some(TelegramSettings {
            token_env: self.token_env.clone(),
            owner: Some(owner),
        }))
    }
}

/// The Discord part of the settings window, each field already trimmed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct DiscordForm {
    pub(crate) on: bool,
    pub(crate) owner: Option<String>,
    pub(crate) token_env: Option<String>,
}

impl DiscordForm {
    pub(crate) fn of(discord: Option<&DiscordSettings>) -> Self {
        match discord {
            Some(discord) => Self {
                on: true,
                owner: discord.owner.clone(),
                token_env: discord.token_env.clone(),
            },
            None => Self::default(),
        }
    }

    /// The `[discord]` section this form asks for, or `None` when Discord is off.
    pub(crate) fn settings(&self) -> Result<Option<DiscordSettings>, String> {
        if !self.on {
            return Ok(None);
        }
        let owner = self.owner.as_deref().ok_or_else(|| {
            let said = &words().settings;
            (said.channel_missing)("Discord", &[said.missing_user_id])
        })?;
        if !discord_id(owner) {
            return Err(String::from(words().settings.discord_owner_shape));
        }
        if let Some(name) = &self.token_env
            && !variable_name(name)
        {
            return Err((words().settings.not_a_key_name)(name));
        }
        Ok(Some(DiscordSettings {
            token_env: self.token_env.clone(),
            owner: Some(owner.to_owned()),
        }))
    }
}

fn discord_id(id: &str) -> bool {
    (15..=21).contains(&id.len()) && id.bytes().all(|byte| byte.is_ascii_digit())
}

fn variable_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// What the settings window holds, each field already trimmed. A blank field means the default.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Form {
    pub(crate) openai_compatible: bool,
    pub(crate) model: Option<String>,
    pub(crate) base_url: Option<String>,
    pub(crate) api_key_env: Option<String>,
    pub(crate) effort: Option<String>,
}

impl Form {
    pub(crate) fn of(settings: &ModelSettings) -> Self {
        match settings.clone() {
            ModelSettings::Anthropic {
                api_key_env,
                model,
                base_url,
                effort,
                ..
            } => Self {
                openai_compatible: false,
                model,
                base_url,
                api_key_env,
                effort,
            },
            ModelSettings::OpenaiCompatible {
                base_url,
                model,
                api_key_env,
                ..
            } => Self {
                openai_compatible: true,
                model: Some(model),
                base_url: Some(base_url),
                api_key_env,
                effort: None,
            },
        }
    }

    /// The settings this form describes. `earlier` keeps what the form does not show.
    pub(crate) fn settings(self, earlier: &ModelSettings) -> Result<ModelSettings, String> {
        let (kept_max_tokens, kept_context_window) = match earlier {
            ModelSettings::Anthropic {
                max_tokens,
                context_window,
                ..
            }
            | ModelSettings::OpenaiCompatible {
                max_tokens,
                context_window,
                ..
            } => (*max_tokens, *context_window),
        };
        if !self.openai_compatible {
            return Ok(ModelSettings::Anthropic {
                api_key_env: self.api_key_env,
                model: self.model,
                base_url: self.base_url,
                max_tokens: kept_max_tokens,
                context_window: kept_context_window,
                effort: self.effort,
            });
        }
        match (self.base_url, self.model) {
            (Some(base_url), Some(model)) => Ok(ModelSettings::OpenaiCompatible {
                base_url,
                model,
                api_key_env: self.api_key_env,
                max_tokens: kept_max_tokens,
                context_window: kept_context_window,
            }),
            _ => Err(String::from(words().settings.openai_needs_address)),
        }
    }
}

impl Settings {
    pub(crate) fn load(path: &Path) -> Result<Self, String> {
        match fs::read_to_string(path) {
            Ok(text) => {
                toml::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))
            }
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(format!("{}: {error}", path.display())),
        }
    }

    /// Writes the whole file anew. A file left half written by a crash is never read.
    pub(crate) fn save(&self, path: &Path) -> Result<(), String> {
        let text = toml::to_string(self).map_err(|error| error.to_string())?;
        replace(path, text.as_bytes(), OpenOptions::new())
    }

    /// Writes only the sections named in `parts` from these settings, leaving the rest of the
    /// file as the owner wrote it, comments included.
    pub(crate) fn save_parts(&self, path: &Path, parts: &[&str]) -> Result<(), String> {
        let written = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == ErrorKind::NotFound => return self.save(path),
            Err(error) => return Err(format!("{}: {error}", path.display())),
        };
        let mut document = written
            .parse::<toml_edit::DocumentMut>()
            .map_err(|error| format!("{}: {error}", path.display()))?;
        let fresh = toml::to_string(self)
            .map_err(|error| error.to_string())?
            .parse::<toml_edit::DocumentMut>()
            .map_err(|error| error.to_string())?;
        for part in parts {
            match fresh.get(part) {
                Some(item) => {
                    let decor = document
                        .get(part)
                        .and_then(toml_edit::Item::as_table)
                        .map(|table| table.decor().clone());
                    document.insert(part, item.clone());
                    if let (Some(decor), Some(table)) = (
                        decor,
                        document
                            .get_mut(part)
                            .and_then(toml_edit::Item::as_table_mut),
                    ) {
                        *table.decor_mut() = decor;
                    }
                }
                None => {
                    document.remove(part);
                }
            }
        }
        replace(path, document.to_string().as_bytes(), OpenOptions::new())
    }

    /// Builds the main session's model, reading keys through `key`.
    pub(crate) fn model(
        &self,
        key: impl Fn(&str) -> Option<String>,
    ) -> Result<Arc<dyn Model>, ModelError> {
        self.model_told(Some(self.named(DEFAULT_SYSTEM)), true, key)
    }

    fn named(&self, system: &str) -> String {
        let name = AssistantSettings::name_of(self.assistant.as_ref());
        let language = crate::language::current().english_name();
        system
            .replacen("You are dotori,", &format!("You are {name},"), 1)
            .replacen(
                FALLBACK_LANGUAGE,
                &format!("When nothing shows it, write in {language}."),
                1,
            )
            .replacen(
                "sub-session of dotori,",
                &format!("sub-session of {name},"),
                1,
            )
    }

    /// Builds the model that runs sub-sessions in dotori's loop, told about the sub-session.
    pub(crate) fn sub_session_model(
        &self,
        key: impl Fn(&str) -> Option<String>,
    ) -> Result<Arc<dyn Model>, ModelError> {
        self.model_told(
            Some(self.named(&format!("{SUB_SESSION_SYSTEM}\n{LLM_SUB_SESSION_SYSTEM}"))),
            false,
            key,
        )
    }

    fn model_told(
        &self,
        system: Option<String>,
        delegates: bool,
        key: impl Fn(&str) -> Option<String>,
    ) -> Result<Arc<dyn Model>, ModelError> {
        match &self.model {
            ModelSettings::Anthropic {
                api_key_env,
                model,
                base_url,
                max_tokens,
                context_window,
                effort,
            } => {
                let variable = api_key_env.as_deref().unwrap_or(ANTHROPIC_KEY);
                let api_key = key(variable).ok_or_else(|| ModelError::MissingKey {
                    name: variable.to_owned(),
                    what: "Anthropic API key",
                })?;
                let mut config = AnthropicConfig::new(api_key);
                if let Some(model) = model {
                    config.model.clone_from(model);
                }
                if let Some(base_url) = base_url {
                    config.base_url.clone_from(base_url);
                }
                if let Some(max_tokens) = max_tokens {
                    config.max_tokens = *max_tokens;
                }
                config.context_window = *context_window;
                config.effort.clone_from(effort);
                config.delegates = delegates;
                if let Some(system) = system {
                    config.system = system;
                }
                Ok(Arc::new(
                    Anthropic::new(config).map_err(|error| ModelError::Other(error.to_string()))?,
                ))
            }
            ModelSettings::OpenaiCompatible {
                base_url,
                model,
                api_key_env,
                max_tokens,
                context_window,
            } => {
                let mut config = OpenAiConfig::new(base_url.clone(), model.clone());
                if let Some(variable) = api_key_env {
                    config.api_key = Some(key(variable).ok_or_else(|| ModelError::MissingKey {
                        name: variable.clone(),
                        what: "API key",
                    })?);
                }
                config.max_tokens = *max_tokens;
                config.context_window = *context_window;
                config.delegates = delegates;
                if let Some(system) = system {
                    config.system = system;
                }
                Ok(Arc::new(
                    OpenAi::new(config).map_err(|error| ModelError::Other(error.to_string()))?,
                ))
            }
        }
    }
}

impl Settings {
    /// The embedding API memory search uses, reading its key through `key`.
    pub(crate) fn embedding(
        &self,
        key: impl Fn(&str) -> Option<String>,
    ) -> Result<Option<EmbeddingConfig>, String> {
        let Some(chosen) = self
            .memory
            .as_ref()
            .and_then(|memory| memory.embeddings.as_ref())
        else {
            return Ok(None);
        };
        let api_key = match &chosen.api_key_env {
            Some(variable) => Some(
                key(variable).ok_or_else(|| format!("set {variable} to your embedding API key"))?,
            ),
            None => None,
        };
        Ok(Some(EmbeddingConfig {
            base_url: chosen.base_url.clone(),
            model: chosen.model.clone(),
            api_key,
            dimensions: chosen.dimensions,
        }))
    }

    /// The MCP servers to relay, reading tokens through `key`. A server set up wrong is named
    /// in the second list and left out.
    /// The variables that hold the tokens of the servers only the main session reaches, which
    /// coding agents must not see.
    pub(crate) fn guarded_secrets(&self) -> Vec<String> {
        self.mcp
            .iter()
            .filter(|server| !server.confirm.is_empty())
            .filter_map(|server| server.token_env.clone())
            .collect()
    }

    /// Every name the settings read a secret under, given or not, with dotori's own model key and
    /// each default name.
    pub(crate) fn secret_names(&self) -> Vec<String> {
        let mut names = [
            MODEL_KEY,
            ANTHROPIC_KEY,
            SLACK_APP_KEY,
            SLACK_BOT_KEY,
            TELEGRAM_KEY,
            DISCORD_KEY,
        ]
        .map(str::to_owned)
        .to_vec();
        let model = match &self.model {
            ModelSettings::Anthropic { api_key_env, .. }
            | ModelSettings::OpenaiCompatible { api_key_env, .. } => api_key_env,
        };
        let slack = self.slack.as_ref();
        names.extend(
            [
                model.as_ref(),
                self.memory
                    .as_ref()
                    .and_then(|memory| memory.embeddings.as_ref())
                    .and_then(|embeddings| embeddings.api_key_env.as_ref()),
                slack.and_then(|slack| slack.app_token_env.as_ref()),
                slack.and_then(|slack| slack.bot_token_env.as_ref()),
                self.telegram
                    .as_ref()
                    .and_then(|telegram| telegram.token_env.as_ref()),
                self.discord
                    .as_ref()
                    .and_then(|discord| discord.token_env.as_ref()),
            ]
            .into_iter()
            .flatten()
            .chain(
                self.mcp
                    .iter()
                    .filter_map(|server| server.token_env.as_ref()),
            )
            .cloned(),
        );
        names.retain(|name| !reserved(name));
        names.sort();
        names.dedup();
        names
    }

    /// The relayed tools the owner must allow before each call, by the name the model sees.
    /// `"*"` stands for every tool of its server.
    pub(crate) fn asked(&self) -> Asked {
        let mut asked = Asked::default();
        for server in &self.mcp {
            for tool in &server.confirm {
                if tool == "*" {
                    asked.servers.push(format!("{}__", server.name));
                } else {
                    asked.tools.push(format!("{}__{tool}", server.name));
                }
            }
        }
        asked
    }

    /// The external MCP servers to relay, reading tokens through `key`. `guarded` picks the
    /// servers with tools that wait for the owner, which only the daemon relays, so only the
    /// main session reaches them; otherwise the rest, which the shared MCP server relays.
    /// A server started as a command inherits none of the variables in `hidden` and gets only
    /// its own token.
    pub(crate) fn relays(
        &self,
        key: impl Fn(&str) -> Option<String>,
        guarded: bool,
        hidden: &[String],
    ) -> (Vec<RelayConfig>, Vec<String>) {
        let mut chosen = Vec::new();
        let mut wrong = Vec::new();
        let mut named = Vec::new();
        for server in &self.mcp {
            if named.contains(&server.name.as_str()) {
                if server.confirm.is_empty() != guarded {
                    wrong.push(format!(
                        "{}: another MCP server already has this name",
                        server.name
                    ));
                }
                continue;
            }
            named.push(server.name.as_str());
            if server.confirm.is_empty() == guarded {
                continue;
            }
            let token = match &server.token_env {
                Some(variable) if reserved(variable) => {
                    wrong.push(format!(
                        "{}: {variable} holds a setting of the system, not a token",
                        server.name
                    ));
                    continue;
                }
                None => None,
                Some(variable) => match key(variable) {
                    Some(token) => Some((variable.clone(), token)),
                    None => {
                        wrong.push(format!("{}: set {variable} to its token", server.name));
                        continue;
                    }
                },
            };
            let reach = match (&server.url, &server.command) {
                (Some(url), None) => Reach::Http {
                    url: url.clone(),
                    token: token.map(|(_, token)| token),
                },
                (None, Some(command)) => Reach::Command {
                    program: command.clone(),
                    args: server.args.clone(),
                    token,
                    hidden_env: hidden.to_vec(),
                },
                _ => {
                    wrong.push(format!("{}: give either `url` or `command`", server.name));
                    continue;
                }
            };
            chosen.push(RelayConfig {
                name: server.name.clone(),
                reach,
                tools: server.tools.clone(),
            });
        }
        (chosen, wrong)
    }

    /// The channels besides the app, reading tokens through `key`.
    pub(crate) fn channels(
        &self,
        key: impl Fn(&str) -> Option<String>,
    ) -> (Vec<Arc<dyn Cards>>, Vec<String>) {
        let mut channels = Vec::new();
        let mut off = Vec::new();
        for (name, built) in [
            ("Slack", self.slack_channel(&key)),
            ("Telegram", self.telegram_channel(&key)),
            ("Discord", self.discord_channel(&key)),
        ] {
            match built {
                Ok(built) => channels.extend(built),
                Err(error) => off.push(format!("{name} is off: {error}")),
            }
        }
        (channels, off)
    }

    fn telegram_channel(
        &self,
        key: &impl Fn(&str) -> Option<String>,
    ) -> Result<Vec<Arc<dyn Cards>>, String> {
        let Some(telegram) = &self.telegram else {
            return Ok(Vec::new());
        };
        let owner = telegram
            .owner
            .ok_or("set `owner` under [telegram] to your Telegram user id, such as 123456789")?;
        let variable = telegram.token_env.as_deref().unwrap_or(TELEGRAM_KEY);
        let token =
            key(variable).ok_or_else(|| format!("set {variable} to your Telegram bot token"))?;
        let channel = TelegramChannel::new(TelegramConfig::new(token, owner))
            .map_err(|error| error.detail)?;
        Ok(vec![Arc::new(channel)])
    }

    fn discord_channel(
        &self,
        key: &impl Fn(&str) -> Option<String>,
    ) -> Result<Vec<Arc<dyn Cards>>, String> {
        let Some(discord) = &self.discord else {
            return Ok(Vec::new());
        };
        let owner = discord
            .owner
            .as_deref()
            .map(str::trim)
            .filter(|owner| discord_id(owner))
            .ok_or("set `owner` under [discord] to your Discord user id, such as \"123456789012345678\"")?;
        let variable = discord.token_env.as_deref().unwrap_or(DISCORD_KEY);
        let token =
            key(variable).ok_or_else(|| format!("set {variable} to your Discord bot token"))?;
        let channel =
            DiscordChannel::new(DiscordConfig::new(token, owner)).map_err(|error| error.detail)?;
        Ok(vec![Arc::new(channel)])
    }

    fn slack_channel(
        &self,
        key: &impl Fn(&str) -> Option<String>,
    ) -> Result<Vec<Arc<dyn Cards>>, String> {
        let Some(slack) = &self.slack else {
            return Ok(Vec::new());
        };
        let owner = slack
            .owner
            .clone()
            .filter(|owner| !owner.trim().is_empty())
            .ok_or("set `owner` under [slack] to your Slack user id, such as U012AB3CD")?;
        let token = |named: &Option<String>, default: &str, what: &str| {
            let variable = named.as_deref().unwrap_or(default);
            key(variable).ok_or_else(|| format!("set {variable} to your Slack {what}"))
        };
        let config = SlackConfig::new(
            token(&slack.app_token_env, SLACK_APP_KEY, "app token")?,
            token(&slack.bot_token_env, SLACK_BOT_KEY, "bot token")?,
            owner.trim(),
        );
        let channel = SlackChannel::new(config).map_err(|error| error.detail)?;
        Ok(vec![Arc::new(channel)])
    }
}

/// Whether `name` is a variable the system and programs set for themselves, such as `PATH`, which
/// never holds a token and is never kept from a command.
pub(crate) fn reserved(name: &str) -> bool {
    const RESERVED: [&str; 24] = [
        "PATH",
        "HOME",
        "USER",
        "LOGNAME",
        "SHELL",
        "TMPDIR",
        "TMP",
        "TEMP",
        "LANG",
        "LANGUAGE",
        "TERM",
        "PWD",
        "DISPLAY",
        "WAYLAND_DISPLAY",
        "DBUS_SESSION_BUS_ADDRESS",
        "SSH_AUTH_SOCK",
        "USERPROFILE",
        "APPDATA",
        "LOCALAPPDATA",
        "SYSTEMROOT",
        "WINDIR",
        "COMSPEC",
        "PATHEXT",
        "DOTORI_HOME",
    ];
    let name = name.to_ascii_uppercase();
    RESERVED.contains(&name.as_str()) || name.starts_with("LC_") || name.starts_with("XDG_")
}

pub(crate) fn environment(variable: &str) -> Option<String> {
    if crate::accounts::is_account(variable) {
        return None;
    }
    env::var(variable)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

#[cfg(test)]
#[path = "settings.test.rs"]
mod tests;
