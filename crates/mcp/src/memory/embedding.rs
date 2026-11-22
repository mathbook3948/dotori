use std::time::Duration;

use reqwest::Client;
use serde_json::Value;
use serde_json::json;

const TIMEOUT: Duration = Duration::from_secs(60);

/// An embedding API in the OpenAI `/embeddings` shape. Turning it on sends recorded text there.
#[derive(Clone, PartialEq, Eq)]
pub struct EmbeddingConfig {
    pub base_url: String,
    pub model: String,
    pub api_key: Option<String>,
    pub dimensions: Option<u32>,
}

impl std::fmt::Debug for EmbeddingConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmbeddingConfig")
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("api_key", &self.api_key.as_ref().map(|_| "set"))
            .field("dimensions", &self.dimensions)
            .finish()
    }
}

/// Why texts got no vectors. `by_text` means the API refused what was sent, so another text can
/// still work.
#[derive(Debug)]
pub(crate) struct Refused {
    pub(crate) detail: String,
    pub(crate) by_text: bool,
}

impl From<String> for Refused {
    fn from(detail: String) -> Self {
        Self {
            detail,
            by_text: false,
        }
    }
}

impl From<&str> for Refused {
    fn from(detail: &str) -> Self {
        Self::from(detail.to_owned())
    }
}

impl From<Refused> for String {
    fn from(refused: Refused) -> Self {
        refused.detail
    }
}

pub(crate) struct Embedder {
    config: EmbeddingConfig,
    client: Client,
}

impl Embedder {
    pub(crate) fn new(config: EmbeddingConfig) -> Result<Self, String> {
        let client = Client::builder()
            .timeout(TIMEOUT)
            .build()
            .map_err(|error| error.to_string())?;
        Ok(Self { config, client })
    }

    /// What each vector records as the model that made it. A change here makes every vector again.
    pub(crate) fn name(&self) -> String {
        let base = self.config.base_url.trim_end_matches('/');
        match self.config.dimensions {
            Some(dimensions) => format!("{base} {} {dimensions}", self.config.model),
            None => format!("{base} {}", self.config.model),
        }
    }

    /// One unit-length vector per text, in the same order.
    pub(crate) async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, Refused> {
        let mut body = json!({
            "model": self.config.model,
            "input": texts,
            "encoding_format": "float",
        });
        if let Some(dimensions) = self.config.dimensions {
            body["dimensions"] = json!(dimensions);
        }
        let url = format!("{}/embeddings", self.config.base_url.trim_end_matches('/'));
        let mut request = self
            .client
            .post(url)
            .header("content-type", "application/json")
            .body(body.to_string());
        if let Some(key) = &self.config.api_key {
            request = request.bearer_auth(key);
        }
        let response = request.send().await.map_err(|error| {
            format!("the embedding API is unreachable: {}", error.without_url())
        })?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|error| error.without_url().to_string())?;
        if !status.is_success() {
            let shown: String = text.chars().take(300).collect();
            return Err(Refused {
                detail: format!("the embedding API answered {status}: {shown}"),
                by_text: matches!(status.as_u16(), 400 | 413 | 422),
            });
        }
        let answer: Value = serde_json::from_str(&text)
            .map_err(|error| format!("the embedding API answered something else: {error}"))?;
        Ok(vectors(&answer, texts.len())?)
    }
}

fn vectors(answer: &Value, expected: usize) -> Result<Vec<Vec<f32>>, String> {
    let data = answer
        .get("data")
        .and_then(Value::as_array)
        .ok_or("the embedding API answered without `data`")?;
    let mut found: Vec<Option<Vec<f32>>> = vec![None; expected];
    for (position, item) in data.iter().enumerate() {
        let index = item
            .get("index")
            .and_then(Value::as_u64)
            .and_then(|index| usize::try_from(index).ok())
            .unwrap_or(position);
        let vector = item
            .get("embedding")
            .and_then(Value::as_array)
            .ok_or("an embedding is not a list of numbers")?
            .iter()
            .map(|number| number.as_f64().map(|number| number as f32))
            .collect::<Option<Vec<f32>>>()
            .ok_or("an embedding is not a list of numbers")?;
        let slot = found
            .get_mut(index)
            .ok_or("the embedding API answered for a text it was not sent")?;
        *slot = Some(unit(vector)?);
    }
    let vectors: Vec<Vec<f32>> = found
        .into_iter()
        .collect::<Option<_>>()
        .ok_or("the embedding API left a text out")?;
    let Some(first) = vectors.first() else {
        return Ok(vectors);
    };
    if vectors.iter().any(|vector| vector.len() != first.len()) {
        return Err(String::from(
            "the embedding API answered vectors of different lengths",
        ));
    }
    Ok(vectors)
}

fn unit(mut vector: Vec<f32>) -> Result<Vec<f32>, String> {
    let length = vector
        .iter()
        .map(|value| f64::from(*value) * f64::from(*value))
        .sum::<f64>()
        .sqrt();
    if vector.is_empty() || !length.is_finite() || length == 0.0 {
        return Err(String::from("the embedding API answered an empty vector"));
    }
    for value in &mut vector {
        *value = (f64::from(*value) / length) as f32;
    }
    Ok(vector)
}

pub(crate) fn to_bytes(vector: &[f32]) -> Vec<u8> {
    vector
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

pub(crate) fn from_bytes(bytes: &[u8]) -> Vec<f32> {
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|chunk| f32::from_le_bytes(*chunk))
        .collect()
}

#[cfg(test)]
#[path = "embedding.test.rs"]
mod tests;
