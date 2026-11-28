//! Embedding models that run on this PC, one for each language dotori speaks. A model is
//! downloaded on first use, loaded when a text needs a vector, and unloaded after a while unused.

use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::time::Duration;
use std::time::Instant;

use sha2::Digest;
use sha2::Sha256;
use tokenizers::Tokenizer;
use tokenizers::TruncationParams;
use tract_onnx::prelude::*;

use super::embedding::Refused;
use super::embedding::Use;
use super::embedding::unit;

const IDLE: Duration = Duration::from_secs(300);
const WAIT: Duration = Duration::from_secs(1800);
const LONGEST_TOKENS: usize = 512;

/// The model for the language dotori speaks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalModel {
    Korean,
    English,
}

#[derive(Clone, Copy)]
enum Pooling {
    Mean,
    First,
}

struct Source {
    path: &'static str,
    digest: &'static str,
    size: u64,
}

struct Model {
    name: &'static str,
    folder: &'static str,
    repository: &'static str,
    revision: &'static str,
    weights: Source,
    tokenizer: Source,
    query: &'static str,
    record: &'static str,
    pooling: Pooling,
}

const KOREAN: Model = Model {
    name: "local KoEn-E5-Tiny int8 292c09c",
    folder: "koen-e5-tiny-292c09c",
    repository: "exp-models/dragonkue-KoEn-E5-Tiny",
    revision: "292c09c78c71a3f00ed56ee0d1ed9f0d39182fc9",
    weights: Source {
        path: "onnx/model_qint8_arm64.onnx",
        digest: "d463c9b1c29f3202d510e2836265d60f9e6e6dd518c5930ba865700646489bd4",
        size: 38_275_821,
    },
    tokenizer: Source {
        path: "tokenizer.json",
        digest: "a6dd38d692ac1caa6d5dbc195d92f1f978b5c74ec60e02ed15fdf04404742fe3",
        size: 2_931_715,
    },
    query: "query: ",
    record: "passage: ",
    pooling: Pooling::Mean,
};

const ENGLISH: Model = Model {
    name: "local snowflake-arctic-embed-s int8 e596f50",
    folder: "arctic-embed-s-e596f50",
    repository: "Snowflake/snowflake-arctic-embed-s",
    revision: "e596f507467533e48a2e17c007f0e1dacc837b33",
    weights: Source {
        path: "onnx/model_int8.onnx",
        digest: "f93ff225320628d2e88baf2a395cae791b0e3b27edf5c70bf7b312a4d3260c14",
        size: 34_015_111,
    },
    tokenizer: Source {
        path: "tokenizer.json",
        digest: "91f1def9b9391fdabe028cd3f3fcc4efd34e5d1f08c3bf2de513ebb5911a1854",
        size: 711_649,
    },
    query: "Represent this sentence for searching relevant passages: ",
    record: "",
    pooling: Pooling::First,
};

impl LocalModel {
    fn model(self) -> &'static Model {
        match self {
            Self::Korean => &KOREAN,
            Self::English => &ENGLISH,
        }
    }
}

/// A model on this PC, kept in its own folder under the folder of models.
pub(crate) struct Local {
    model: &'static Model,
    folder: PathBuf,
    fetching: tokio::sync::Mutex<()>,
    held: Arc<Mutex<Held>>,
}

#[derive(Default)]
struct Held {
    loaded: Option<Loaded>,
    used: Option<Instant>,
    sweeping: bool,
}

struct Loaded {
    tokenizer: Tokenizer,
    plan: Arc<TypedRunnableModel>,
    inputs: usize,
}

impl Local {
    pub(crate) fn new(model: LocalModel, models: &Path) -> Self {
        let model = model.model();
        Self {
            model,
            folder: models.join(model.folder),
            fetching: tokio::sync::Mutex::new(()),
            held: Arc::default(),
        }
    }

    pub(crate) fn name(&self) -> String {
        self.model.name.to_owned()
    }

    /// Downloads the model's files unless they are here, and checks their digests.
    pub(crate) async fn prepare(&self) -> Result<(), String> {
        let _only = self.fetching.lock().await;
        for source in [&self.model.weights, &self.model.tokenizer] {
            let path = self.folder.join(source.path);
            if size(&path) == Some(source.size) {
                continue;
            }
            let url = format!(
                "https://huggingface.co/{}/resolve/{}/{}",
                self.model.repository, self.model.revision, source.path
            );
            download(&url, source, &path).await?;
        }
        Ok(())
    }

    fn ready(&self) -> bool {
        [&self.model.weights, &self.model.tokenizer]
            .iter()
            .all(|source| size(&self.folder.join(source.path)) == Some(source.size))
    }

    /// One unit-length vector per text, in the same order. Each text is run alone, so memory
    /// stays the same however many there are.
    pub(crate) async fn embed(
        &self,
        texts: &[String],
        wanted: Use,
    ) -> Result<Vec<Vec<f32>>, Refused> {
        if !self.ready() {
            return Err(Refused::from("the embedding model is not downloaded yet"));
        }
        let prefix = match wanted {
            Use::Query => self.model.query,
            Use::Record => self.model.record,
        };
        let texts: Vec<String> = texts.iter().map(|text| format!("{prefix}{text}")).collect();
        let held = Arc::clone(&self.held);
        let folder = self.folder.clone();
        let model = self.model;
        let runtime = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            let made = run(&held, model, &folder, &texts);
            dotori_native::return_unused_memory();
            let mut state = lock(&held);
            if !state.sweeping && state.loaded.is_some() {
                state.sweeping = true;
                runtime.spawn(unload_when_idle(Arc::clone(&held)));
            }
            made
        })
        .await
        .map_err(|error| Refused::from(error.to_string()))?
    }
}

fn run(
    held: &Mutex<Held>,
    model: &Model,
    folder: &Path,
    texts: &[String],
) -> Result<Vec<Vec<f32>>, Refused> {
    let mut made = Vec::with_capacity(texts.len());
    for text in texts {
        let mut held = lock(held);
        let loaded = match &mut held.loaded {
            Some(loaded) => loaded,
            slot @ None => slot.insert(load(model, folder)?),
        };
        made.push(loaded.vector(text, model.pooling)?);
        held.used = Some(Instant::now());
    }
    Ok(made)
}

impl Loaded {
    fn vector(&self, text: &str, pooling: Pooling) -> Result<Vec<f32>, Refused> {
        let encoding = self.tokenizer.encode(text, true).map_err(|error| Refused {
            detail: format!("the text cannot be split into tokens: {error}"),
            by_text: true,
        })?;
        let ids: Vec<i64> = encoding.get_ids().iter().map(|id| i64::from(*id)).collect();
        let count = ids.len();
        if count == 0 {
            return Err(Refused {
                detail: String::from("the text has no tokens"),
                by_text: true,
            });
        }
        let shape = [1, count];
        let mut inputs: TVec<TValue> = tvec![
            Tensor::from_shape(&shape, &ids).map_err(fail)?.into(),
            Tensor::from_shape(&shape, &vec![1_i64; count])
                .map_err(fail)?
                .into(),
        ];
        if self.inputs > 2 {
            inputs.push(
                Tensor::from_shape(&shape, &vec![0_i64; count])
                    .map_err(fail)?
                    .into(),
            );
        }
        let outputs = self.plan.run(inputs).map_err(fail)?;
        let hidden = outputs
            .first()
            .ok_or("the model gave no output")?
            .to_plain_array_view::<f32>()
            .map_err(fail)?;
        let &[1, tokens, width] = hidden.shape() else {
            return Err(Refused::from("the model gave an output of another shape"));
        };
        let vector = match pooling {
            Pooling::First => (0..width).map(|at| hidden[[0, 0, at]]).collect(),
            Pooling::Mean => (0..width)
                .map(|at| {
                    (0..tokens).map(|token| hidden[[0, token, at]]).sum::<f32>() / tokens as f32
                })
                .collect(),
        };
        Ok(unit(vector)?)
    }
}

fn load(model: &Model, folder: &Path) -> Result<Loaded, String> {
    let graph = tract_onnx::onnx()
        .model_for_path(folder.join(model.weights.path))
        .map_err(|error| format!("the embedding model cannot be read: {error}"))?;
    let inputs = graph.input_outlets().map_err(fail)?.len();
    let plan = graph
        .into_optimized()
        .and_then(|graph| graph.into_runnable())
        .map_err(|error| format!("the embedding model cannot be prepared: {error}"))?;
    dotori_native::return_unused_memory();
    let mut tokenizer = Tokenizer::from_file(folder.join(model.tokenizer.path))
        .map_err(|error| format!("the tokenizer cannot be read: {error}"))?;
    tokenizer
        .with_truncation(Some(TruncationParams {
            max_length: LONGEST_TOKENS,
            ..TruncationParams::default()
        }))
        .map_err(|error| error.to_string())?
        .with_padding(None);
    dotori_native::return_unused_memory();
    Ok(Loaded {
        tokenizer,
        plan,
        inputs,
    })
}

async fn unload_when_idle(held: Arc<Mutex<Held>>) {
    let mut wait = IDLE;
    loop {
        tokio::time::sleep(wait).await;
        let held = Arc::clone(&held);
        let left = tokio::task::spawn_blocking(move || {
            let mut held = lock(&held);
            let idle = held.used.map_or(IDLE, |used| used.elapsed());
            if idle < IDLE {
                return Some(IDLE - idle);
            }
            held.loaded = None;
            held.sweeping = false;
            drop(held);
            dotori_native::return_unused_memory();
            None
        })
        .await
        .unwrap_or(Some(IDLE));
        match left {
            Some(left) => wait = left,
            None => return,
        }
    }
}

fn lock(held: &Mutex<Held>) -> MutexGuard<'_, Held> {
    held.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn size(path: &Path) -> Option<u64> {
    std::fs::metadata(path).ok().map(|metadata| metadata.len())
}

fn fail(error: impl std::fmt::Display) -> String {
    format!("the embedding model failed: {error}")
}

async fn download(url: &str, source: &Source, path: &Path) -> Result<(), String> {
    let folder = path.parent().ok_or("a model file has no folder")?;
    std::fs::create_dir_all(folder).map_err(|error| error.to_string())?;
    let draft = path.with_extension(format!("part.{}", std::process::id()));
    drop_abandoned_drafts(path);
    let fetched = fetch(url, source, &draft).await;
    let placed =
        fetched.and_then(|()| std::fs::rename(&draft, path).map_err(|error| error.to_string()));
    if placed.is_err() {
        let _ = std::fs::remove_file(&draft);
    }
    placed
}

fn drop_abandoned_drafts(path: &Path) {
    let (Some(folder), Some(stem)) = (path.parent(), path.file_stem()) else {
        return;
    };
    let prefix = format!("{}.part.", stem.to_string_lossy());
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        let abandoned = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age > WAIT);
        if abandoned && entry.file_name().to_string_lossy().starts_with(&prefix) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

async fn fetch(url: &str, source: &Source, draft: &Path) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(WAIT)
        .build()
        .map_err(|error| error.to_string())?;
    let mut response = client
        .get(url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| {
            format!(
                "cannot download the embedding model: {}",
                error.without_url()
            )
        })?;
    let mut file = std::fs::File::create(draft).map_err(|error| error.to_string())?;
    let mut hasher = Sha256::new();
    let mut length = 0_u64;
    while let Some(chunk) = response.chunk().await.map_err(|error| {
        format!(
            "cannot download the embedding model: {}",
            error.without_url()
        )
    })? {
        length += chunk.len() as u64;
        if length > source.size {
            return Err(String::from("the embedding model download is too large"));
        }
        hasher.update(&chunk);
        file.write_all(&chunk).map_err(|error| error.to_string())?;
    }
    file.sync_all().map_err(|error| error.to_string())?;
    checked(source, length, &hasher.finalize())
}

fn checked(source: &Source, length: u64, digest: &[u8]) -> Result<(), String> {
    let digest: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    if length != source.size || digest != source.digest {
        return Err(format!(
            "the embedding model download is damaged ({length} bytes, sha-256 {digest})"
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "local.test.rs"]
mod tests;
