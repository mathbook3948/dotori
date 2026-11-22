use std::env;
use std::fs;
use std::io::ErrorKind;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;

use dotori_connectors::channel::slack::SlackChannel;
use dotori_connectors::channel::slack::SlackConfig;
use dotori_connectors::llm::anthropic::Anthropic;
use dotori_connectors::llm::anthropic::AnthropicConfig;
use dotori_connectors::llm::openai::OpenAi;
use dotori_connectors::llm::openai::OpenAiConfig;
use dotori_core::channel::Channel;
use dotori_core::id::ChannelUser;
use dotori_core::id::ExternalUserId;
use dotori_core::llm::Model;
use serde::Deserialize;
use serde::Serialize;

const ANTHROPIC_KEY: &str = "ANTHROPIC_API_KEY";
const SLACK_APP_KEY: &str = "SLACK_APP_TOKEN";
const SLACK_BOT_KEY: &str = "SLACK_BOT_TOKEN";

/// What the user sets in `settings.toml`. A missing file means every default.
/// Keys never live in the file. It names the environment variable that holds each key.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Settings {
    #[serde(default)]
    pub(crate) model: ModelSettings,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) slack: Option<SlackSettings>,
}

/// Slack is on when this section is in the file. The owner is a Slack user id, such as `U012AB3CD`.
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

/// A channel to connect, and the owner's account on it if one is set.
pub(crate) type Connected = (Arc<dyn Channel>, Option<ChannelUser>);

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

/// What the settings window holds, each field already trimmed. A blank field means the default.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(
    all(not(feature = "window"), not(test)),
    expect(dead_code, reason = "only the settings window reads a form")
)]
pub(crate) struct Form {
    pub(crate) openai_compatible: bool,
    pub(crate) model: Option<String>,
    pub(crate) base_url: Option<String>,
    pub(crate) api_key_env: Option<String>,
    pub(crate) effort: Option<String>,
}

#[cfg_attr(
    all(not(feature = "window"), not(test)),
    expect(dead_code, reason = "only the settings window reads a form")
)]
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

    #[cfg_attr(
        all(not(feature = "window"), not(test)),
        expect(dead_code, reason = "only the settings window writes settings")
    )]
    /// Writes the whole file anew. A file left half written by a crash is never read.
    pub(crate) fn save(&self, path: &Path) -> Result<(), String> {
        let fail = |error: std::io::Error| format!("{}: {error}", path.display());
        let text = toml::to_string(self).map_err(|error| error.to_string())?;
        if let Some(folder) = path.parent() {
            fs::create_dir_all(folder).map_err(fail)?;
        }
        let draft = path.with_extension("toml.new");
        let mut file = fs::File::create(&draft).map_err(fail)?;
        file.write_all(text.as_bytes()).map_err(fail)?;
        file.sync_all().map_err(fail)?;
        fs::rename(&draft, path).map_err(fail)
    }

    /// Builds the main conversation's model, reading keys through `key`.
    pub(crate) fn model(
        &self,
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
                Ok(Arc::new(
                    OpenAi::new(config).map_err(|error| error.to_string())?,
                ))
            }
        }
    }
}

impl Settings {
    /// The channels besides the app, reading tokens through `key`.
    pub(crate) fn channels(
        &self,
        key: impl Fn(&str) -> Option<String>,
    ) -> Result<Vec<Connected>, String> {
        let Some(slack) = &self.slack else {
            return Ok(Vec::new());
        };
        let token = |named: &Option<String>, default: &str, what: &str| {
            let variable = named.as_deref().unwrap_or(default);
            key(variable).ok_or_else(|| format!("set {variable} to your Slack {what}"))
        };
        let config = SlackConfig::new(
            token(&slack.app_token_env, SLACK_APP_KEY, "app token")?,
            token(&slack.bot_token_env, SLACK_BOT_KEY, "bot token")?,
        );
        let channel = SlackChannel::new(config).map_err(|error| error.detail)?;
        let owner = slack.owner.as_ref().map(|user| ChannelUser {
            channel: channel.id().clone(),
            user: ExternalUserId::new(user.as_str()),
        });
        Ok(vec![(Arc::new(channel), owner)])
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
