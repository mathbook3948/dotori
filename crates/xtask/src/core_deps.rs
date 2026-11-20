use std::collections::BTreeSet;
use std::error::Error;
use std::path::Path;
use std::process::Command;

const ALLOWED_DEPS: &str = "guards/core-allowed-deps.txt";

pub fn unlisted_deps(cargo_tree: &str, allowed_deps: &str) -> Vec<String> {
    let allowed: BTreeSet<&str> = allowed_deps
        .lines()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .collect();
    let direct_deps: BTreeSet<&str> = cargo_tree
        .lines()
        .skip(1)
        .filter(|line| !line.starts_with('['))
        .filter_map(|line| line.split_whitespace().next())
        .collect();

    direct_deps
        .into_iter()
        .filter(|name| !allowed.contains(name))
        .map(String::from)
        .collect()
}

pub fn run(workspace_root: &Path) -> Result<Vec<String>, Box<dyn Error>> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let tree = Command::new(cargo)
        .current_dir(workspace_root)
        .args(["tree", "--package", "lightbot-core", "--depth", "1"])
        .args(["--prefix", "none"])
        .output()?;
    if !tree.status.success() {
        return Err(format!(
            "cargo tree failed: {}",
            String::from_utf8_lossy(&tree.stderr)
        )
        .into());
    }

    let allowed_deps = std::fs::read_to_string(workspace_root.join(ALLOWED_DEPS))?;
    Ok(
        unlisted_deps(&String::from_utf8(tree.stdout)?, &allowed_deps)
            .into_iter()
            .map(|name| {
                format!("lightbot-core depends on {name}, which is not listed in {ALLOWED_DEPS}")
            })
            .collect(),
    )
}

#[cfg(test)]
#[path = "core_deps.test.rs"]
mod tests;
