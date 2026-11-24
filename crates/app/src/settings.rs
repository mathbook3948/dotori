use std::env;
use std::fs;
use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use dotori_connectors::channel::slack::SlackChannel;
use dotori_connectors::channel::slack::SlackConfig;
use dotori_connectors::llm::anthropic::Anthropic;
use dotori_connectors::llm::anthropic::AnthropicConfig;
use dotori_connectors::llm::openai::OpenAi;
use dotori_connectors::llm::openai::OpenAiConfig;
use dotori_connectors::sub_session::SUB_SESSION_SYSTEM;
use dotori_connectors::sub_session::llm::LLM_SUB_SESSION_SYSTEM;
use dotori_core::channel::Channel;
use dotori_core::llm::Model;
use dotori_mcp::memory::EmbeddingConfig;
use dotori_mcp::relay::Reach;
use dotori_mcp::relay::RelayConfig;
use serde::Deserialize;
use serde::Serialize;

use crate::replace::replace;

const ANTHROPIC_KEY: &str = "ANTHROPIC_API_KEY";
/// Where the Slack app token is when the settings do not say.
pub(crate) const SLACK_APP_KEY: &str = "SLACK_APP_TOKEN";
/// Where the Slack bot token is when the settings do not say.
pub(crate) const SLACK_BOT_KEY: &str = "SLACK_BOT_TOKEN";

/// What the user sets in `settings.toml`. A missing file means every default.
/// Keys never live in the file. It names the environment variable that holds each key.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Settings {
    #[serde(default)]
    pub(crate) model: ModelSettings,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) slack: Option<SlackSettings>,
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
}

/// How the owner's other PCs reach this one. With the section, this PC listens on `listen`, or
/// on every address at the usual port. `address` is what the others should dial, such as its
/// Tailscale name, and `name` is how the windows show this PC.
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
/// A token goes in the environment variable `token_env` names, never in the file.
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
        effort: Option<String>,
    },
    OpenaiCompatible {
        base_url: String,
        model: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        api_key_env: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_tokens: Option<u32>,
    },
}

impl Default for ModelSettings {
    fn default() -> Self {
        Self::Anthropic {
            api_key_env: None,
            model: None,
            base_url: None,
            max_tokens: None,
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
            .ok_or("Slack을 쓰려면 내 Slack 멤버 ID를 적어 주세요")?;
        if !(owner.starts_with('U') || owner.starts_with('W'))
            || owner.len() < 3
            || !owner
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        {
            return Err(String::from(
                "Slack 멤버 ID는 U012AB3CD처럼 U나 W로 시작하는 대문자와 숫자예요",
            ));
        }
        for name in [&self.app_token_env, &self.bot_token_env]
            .into_iter()
            .flatten()
        {
            let mut chars = name.chars();
            let first = chars
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
            if !first || !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return Err(format!(
                    "`{name}`은 환경 변수 이름이 아니에요. 토큰은 여기 말고 환경 변수에 넣어 주세요"
                ));
            }
        }
        Ok(Some(SlackSettings {
            app_token_env: self.app_token_env.clone(),
            bot_token_env: self.bot_token_env.clone(),
            owner: Some(owner),
        }))
    }
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
        let kept_max_tokens = match earlier {
            ModelSettings::Anthropic { max_tokens, .. }
            | ModelSettings::OpenaiCompatible { max_tokens, .. } => *max_tokens,
        };
        if !self.openai_compatible {
            return Ok(ModelSettings::Anthropic {
                api_key_env: self.api_key_env,
                model: self.model,
                base_url: self.base_url,
                max_tokens: kept_max_tokens,
                effort: self.effort,
            });
        }
        match (self.base_url, self.model) {
            (Some(base_url), Some(model)) => Ok(ModelSettings::OpenaiCompatible {
                base_url,
                model,
                api_key_env: self.api_key_env,
                max_tokens: kept_max_tokens,
            }),
            _ => Err(String::from("OpenAI 호환 API는 주소와 모델이 필요합니다.")),
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
    ) -> Result<Arc<dyn Model>, String> {
        self.model_told(None, key)
    }

    /// Builds the model that runs sub-sessions in dotori's loop, told about the sub-session.
    pub(crate) fn sub_session_model(
        &self,
        key: impl Fn(&str) -> Option<String>,
    ) -> Result<Arc<dyn Model>, String> {
        self.model_told(
            Some(format!("{SUB_SESSION_SYSTEM}\n{LLM_SUB_SESSION_SYSTEM}")),
            key,
        )
    }

    fn model_told(
        &self,
        system: Option<String>,
        key: impl Fn(&str) -> Option<String>,
    ) -> Result<Arc<dyn Model>, String> {
        match &self.model {
            ModelSettings::Anthropic {
                api_key_env,
                model,
                base_url,
                max_tokens,
                effort,
            } => {
                let variable = api_key_env.as_deref().unwrap_or(ANTHROPIC_KEY);
                let api_key = key(variable)
                    .ok_or_else(|| format!("set {variable} to your Anthropic API key"))?;
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
                config.effort.clone_from(effort);
                if let Some(system) = system {
                    config.system = system;
                }
                Ok(Arc::new(
                    Anthropic::new(config).map_err(|error| error.to_string())?,
                ))
            }
            ModelSettings::OpenaiCompatible {
                base_url,
                model,
                api_key_env,
                max_tokens,
            } => {
                let mut config = OpenAiConfig::new(base_url.clone(), model.clone());
                if let Some(variable) = api_key_env {
                    config.api_key = Some(
                        key(variable).ok_or_else(|| format!("set {variable} to your API key"))?,
                    );
                }
                config.max_tokens = *max_tokens;
                if let Some(system) = system {
                    config.system = system;
                }
                Ok(Arc::new(
                    OpenAi::new(config).map_err(|error| error.to_string())?,
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
    pub(crate) fn relays(
        &self,
        key: impl Fn(&str) -> Option<String>,
    ) -> (Vec<RelayConfig>, Vec<String>) {
        let mut chosen = Vec::new();
        let mut wrong = Vec::new();
        for server in &self.mcp {
            let reach = match (&server.url, &server.command) {
                (Some(url), None) => {
                    let token = match &server.token_env {
                        None => None,
                        Some(variable) => match key(variable) {
                            Some(token) => Some(token),
                            None => {
                                wrong.push(format!("{}: set {variable} to its token", server.name));
                                continue;
                            }
                        },
                    };
                    Reach::Http {
                        url: url.clone(),
                        token,
                    }
                }
                (None, Some(command)) => Reach::Command {
                    program: command.clone(),
                    args: server.args.clone(),
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
    ) -> Result<Vec<Arc<dyn Channel>>, String> {
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

pub(crate) fn environment(variable: &str) -> Option<String> {
    env::var(variable)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

#[cfg(test)]
#[path = "settings.test.rs"]
mod tests;
