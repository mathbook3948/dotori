use std::env;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::sync::Arc;

use dotori_connectors::llm::anthropic::Anthropic;
use dotori_connectors::llm::anthropic::AnthropicConfig;
use dotori_connectors::llm::openai::OpenAi;
use dotori_connectors::llm::openai::OpenAiConfig;
use dotori_core::llm::Model;
use serde::Deserialize;
use serde::Serialize;

const ANTHROPIC_KEY: &str = "ANTHROPIC_API_KEY";

/// What the user sets in `settings.toml`. A missing file means every default.
/// Keys never live in the file. It names the environment variable that holds each key.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Settings {
    #[serde(default)]
    pub(crate) model: ModelSettings,
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

pub(crate) fn environment(variable: &str) -> Option<String> {
    env::var(variable)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

#[cfg(test)]
#[path = "settings.test.rs"]
mod tests;
