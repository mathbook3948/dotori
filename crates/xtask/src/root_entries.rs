use std::collections::BTreeSet;
use std::error::Error;
use std::path::Path;
use std::process::Command;

const ALLOWED_ROOT_ENTRIES: &str = "guards/root-entries.txt";

#[derive(Debug, PartialEq, Eq)]
pub struct ProjectFile<'a> {
    pub path: &'a str,
    pub is_symlink: bool,
}

pub fn stray_files(files: &[ProjectFile], allowed_root_entries: &str) -> Vec<String> {
    let allowed: BTreeSet<&str> = allowed_root_entries
        .lines()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .collect();
    let mut unlisted = BTreeSet::new();
    let mut violations = Vec::new();
    for file in files {
        let root_entry = file.path.split('/').next().unwrap_or_default();
        if !allowed.contains(root_entry) && unlisted.insert(root_entry) {
            violations.push(format!(
                "{root_entry} is not listed in {ALLOWED_ROOT_ENTRIES}"
            ));
        }
        if file.is_symlink {
            violations.push(format!("{} is a symbolic link", file.path));
        }
    }
    violations
}

pub fn run(workspace_root: &Path) -> Result<Vec<String>, Box<dyn Error>> {
    let listing = Command::new("git")
        .current_dir(workspace_root)
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .output()?;
    if !listing.status.success() {
        return Err("git ls-files failed".into());
    }
    let paths = String::from_utf8(listing.stdout)?;
    let files: Vec<ProjectFile> = paths
        .split('\0')
        .filter(|path| !path.is_empty())
        .map(|path| ProjectFile {
            path,
            is_symlink: workspace_root
                .join(path)
                .symlink_metadata()
                .is_ok_and(|metadata| metadata.file_type().is_symlink()),
        })
        .collect();
    let allowed = std::fs::read_to_string(workspace_root.join(ALLOWED_ROOT_ENTRIES))?;
    Ok(stray_files(&files, &allowed))
}

#[cfg(test)]
#[path = "root_entries.test.rs"]
mod tests;
