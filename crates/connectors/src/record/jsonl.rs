use std::fs::File;
use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;

use dotori_core::BoxFuture;
use dotori_core::record::Record;
use dotori_core::record::RecordError;
use dotori_core::record::RecordStore;

use crate::record::form::Line;

/// The raw record as one JSON object per line in a single file that only grows.
/// A line cut short by a crash is dropped on load and cut away before the next append.
#[derive(Clone, Debug)]
pub struct JsonlStore {
    path: Arc<PathBuf>,
    writing: Arc<Mutex<()>>,
}

impl JsonlStore {
    /// The file and its folder are created on the first append.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: Arc::new(path.into()),
            writing: Arc::new(Mutex::new(())),
        }
    }
}

impl RecordStore for JsonlStore {
    fn load(&self) -> BoxFuture<'_, Result<Vec<Record>, RecordError>> {
        let path = Arc::clone(&self.path);
        Box::pin(blocking(move || {
            let mut text = String::new();
            match File::open(path.as_path()) {
                Ok(mut file) => file.read_to_string(&mut text).map_err(fail)?,
                Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
                Err(error) => return Err(fail(error)),
            };
            let whole = text.rfind('\n').map_or(0, |end| end + 1);
            let mut records = Vec::new();
            for (number, line) in text[..whole].lines().enumerate() {
                if line.trim().is_empty() {
                    continue;
                }
                let record = serde_json::from_str::<Line>(line)
                    .map_err(|error| error.to_string())
                    .and_then(Line::into_record)
                    .map_err(|detail| RecordError {
                        detail: format!("line {} of {}: {detail}", number + 1, path.display()),
                    })?;
                records.push(record);
            }
            Ok(records)
        }))
    }

    fn append(&self, records: Vec<Record>) -> BoxFuture<'_, Result<(), RecordError>> {
        let path = Arc::clone(&self.path);
        let writing = Arc::clone(&self.writing);
        Box::pin(blocking(move || {
            let mut text = String::new();
            for record in &records {
                let line = serde_json::to_string(&Line::of(record)).map_err(fail)?;
                text.push_str(&line);
                text.push('\n');
            }
            let _guard = writing
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(folder) = path.parent() {
                std::fs::create_dir_all(folder).map_err(fail)?;
            }
            let mut file = OpenOptions::new()
                .read(true)
                .append(true)
                .create(true)
                .open(path.as_path())
                .map_err(fail)?;
            let kept = whole_lines(&mut file).map_err(fail)?;
            file.set_len(kept).map_err(fail)?;
            let written = file
                .write_all(text.as_bytes())
                .and_then(|()| file.sync_data());
            if let Err(error) = written {
                let _ = file.set_len(kept);
                return Err(fail(error));
            }
            Ok(())
        }))
    }
}

fn whole_lines(file: &mut File) -> std::io::Result<u64> {
    let length = file.metadata()?.len();
    let mut end = length;
    let mut byte = [0u8; 1];
    while end > 0 {
        file.seek(SeekFrom::Start(end - 1))?;
        file.read_exact(&mut byte)?;
        if byte[0] == b'\n' {
            break;
        }
        end -= 1;
    }
    Ok(end)
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, RecordError> + Send + 'static,
) -> Result<T, RecordError> {
    tokio::task::spawn_blocking(work)
        .await
        .unwrap_or_else(|error| {
            Err(RecordError {
                detail: format!("record task failed: {error}"),
            })
        })
}

fn fail(error: impl std::fmt::Display) -> RecordError {
    RecordError {
        detail: error.to_string(),
    }
}

#[cfg(test)]
#[path = "jsonl.test.rs"]
mod tests;
