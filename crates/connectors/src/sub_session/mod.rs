//! Sub-session runners. Each one takes a brief and returns one outcome.

use std::path::Component;
use std::path::Path;
use std::path::PathBuf;

pub mod cli;
pub mod llm;

const PRIVATE: [&str; 2] = ["AppData", "Library"];

/// What a sub-session is told about itself, on top of its brief.
pub const SUB_SESSION_SYSTEM: &str = "\
You are a sub-session of dotori, the owner's personal assistant, which handed you a task.
The task's description is all you know, and nobody will answer questions. Finish the task with your tools.
When you are done, give your whole result in one final answer. The main session reads only that.
When several tools do the same thing, use fetch_url, the browser and search_memory first.";

/// The folder a sub-session works in: the one its job is about, or dotori's work folder `work`
/// when the job names none. A folder no sub-session may work in is refused with the reason.
pub fn working_folder(work: &Path, asked: Option<&str>) -> Result<PathBuf, String> {
    let Some(asked) = asked else {
        std::fs::create_dir_all(work)
            .map_err(|error| format!("cannot make {}: {error}", work.display()))?;
        return Ok(work.to_path_buf());
    };
    let asked = Path::new(asked);
    let shown = asked.display();
    if !asked.is_absolute() {
        return Err(format!(
            "{shown} is not a folder on this PC: give the whole path"
        ));
    }
    let folder = asked
        .canonicalize()
        .ok()
        .filter(|folder| folder.is_dir())
        .ok_or_else(|| format!("{shown} is not a folder on this PC"))?;
    if work.canonicalize().is_ok_and(|own| folder.starts_with(own)) {
        return Ok(plain(folder));
    }
    let hidden = folder.components().any(|part| match part {
        Component::Normal(name) => name.to_string_lossy().starts_with('.'),
        _ => false,
    });
    let private = ["HOME", "USERPROFILE"]
        .into_iter()
        .filter_map(std::env::var_os)
        .filter_map(|home| PathBuf::from(home).canonicalize().ok())
        .any(|home| {
            home.starts_with(&folder)
                || PRIVATE
                    .iter()
                    .any(|name| folder.starts_with(home.join(name)))
        });
    let dotori = work
        .parent()
        .and_then(|root| root.canonicalize().ok())
        .is_some_and(|root| folder.starts_with(&root));
    if hidden || private || dotori || folder.parent().is_none() {
        return Err(format!(
            "{shown} is not a folder a sub-session may work in: pick the project folder itself"
        ));
    }
    Ok(plain(folder))
}

fn plain(folder: PathBuf) -> PathBuf {
    let stripped = folder
        .to_string_lossy()
        .strip_prefix(r"\\?\")
        .filter(|rest| !rest.starts_with("UNC\\"))
        .map(PathBuf::from);
    stripped.unwrap_or(folder)
}

#[cfg(test)]
#[path = "mod.test.rs"]
mod tests;
