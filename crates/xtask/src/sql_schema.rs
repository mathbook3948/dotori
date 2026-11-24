use std::collections::BTreeMap;
use std::error::Error;
use std::path::Path;

const SOURCES: &str = "crates";
const SKIPPED_CRATE: &str = "crates/xtask/";
const META: &str = "meta";
const SHARED_TABLES: [&str; 1] = [META];
const TIME_WORDS: [&str; 4] = ["timestamp", "created", "updated", "date_time"];
const TIME_SUFFIXES: [&str; 2] = ["_at", "_time"];
const SPLIT_NAMES: [(&str, &str); 1] = [("sender", "sender_id or sender_name")];
const TYPED_NAMES: [(&str, &str); 3] = [("record", "TEXT"), ("seq", "INTEGER"), ("n", "INTEGER")];

#[derive(Debug, PartialEq, Eq)]
pub struct Table {
    pub name: String,
    pub columns: Vec<Column>,
    pub foreign: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Column {
    pub name: String,
    pub kind: String,
    pub primary: bool,
}

pub fn tables_in(source: &str) -> Vec<Table> {
    let sql = source.replace("\\\n", " ");
    let mut tables = Vec::new();
    let mut rest = sql.as_str();
    while let Some(at) = rest.find("CREATE ") {
        rest = &rest[at + "CREATE ".len()..];
        let head = rest.trim_start();
        let virtual_table = head.starts_with("VIRTUAL TABLE ");
        let Some(after) = head
            .strip_prefix("VIRTUAL TABLE ")
            .or_else(|| head.strip_prefix("TABLE "))
        else {
            continue;
        };
        let after = after
            .trim_start()
            .strip_prefix("IF NOT EXISTS ")
            .unwrap_or(after)
            .trim_start();
        let name: String = after
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        let Some(open) = after.find('(') else {
            continue;
        };
        let body = &after[open + 1..];
        let mut depth = 1usize;
        let mut end = body.len();
        for (index, c) in body.char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = index;
                        break;
                    }
                }
                _ => {}
            }
        }
        let mut table = Table {
            name,
            columns: Vec::new(),
            foreign: false,
        };
        for part in split_top_level(&body[..end]) {
            let part = part.split_whitespace().collect::<Vec<_>>().join(" ");
            let upper = part.to_ascii_uppercase();
            if upper.contains("REFERENCES") || upper.starts_with("FOREIGN KEY") {
                table.foreign = true;
            }
            if part.is_empty()
                || part.contains('=')
                || [
                    "PRIMARY KEY",
                    "UNIQUE",
                    "CHECK",
                    "FOREIGN KEY",
                    "CONSTRAINT",
                ]
                .iter()
                .any(|constraint| upper.starts_with(constraint))
            {
                continue;
            }
            let mut words = part.split(' ');
            let name = words.next().unwrap_or_default().to_owned();
            let kind = if virtual_table {
                String::new()
            } else {
                words.next().unwrap_or_default().to_ascii_uppercase()
            };
            table.columns.push(Column {
                name,
                kind,
                primary: upper.contains("PRIMARY KEY"),
            });
        }
        tables.push(table);
        rest = &after[open..];
    }
    tables
}

fn split_top_level(body: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    for (index, c) in body.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.push(&body[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    parts.push(&body[start..]);
    parts
}

fn is_name(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

pub fn violations(files: &[(String, String)]) -> Vec<String> {
    let mut found = Vec::new();
    let mut defined: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (path, source) in files {
        let tables = tables_in(source);
        if tables.is_empty() {
            continue;
        }
        if !source.contains("const VERSION: i64") {
            found.push(format!(
                "{path}: makes tables but has no `const VERSION: i64` for PRAGMA user_version"
            ));
        }
        for table in tables {
            defined
                .entry(table.name.clone())
                .or_default()
                .push(path.clone());
            found.extend(
                table_violations(&table)
                    .into_iter()
                    .map(|problem| format!("{path}: table {}: {problem}", table.name)),
            );
        }
    }
    for (table, paths) in defined {
        let mut paths = paths;
        paths.dedup();
        if paths.len() > 1 && !SHARED_TABLES.contains(&table.as_str()) {
            found.push(format!(
                "table {table} is made in more than one file: {}",
                paths.join(", ")
            ));
        }
    }
    found
}

fn table_violations(table: &Table) -> Vec<String> {
    let mut found = Vec::new();
    if !is_name(&table.name) {
        found.push(String::from("name is not lowercase snake_case"));
    }
    if table.foreign {
        found.push(String::from(
            "uses a foreign key; refer to records by their id",
        ));
    }
    if table.name == META {
        let shape: Vec<(&str, bool)> = table
            .columns
            .iter()
            .map(|column| (column.name.as_str(), column.primary))
            .collect();
        if shape != [("key", true), ("value", false)]
            || table.columns.first().map(|column| column.kind.as_str()) != Some("TEXT")
        {
            found.push(String::from("must be (key TEXT PRIMARY KEY, value ...)"));
        }
    }
    for column in &table.columns {
        let name = column.name.as_str();
        if !is_name(name) {
            found.push(format!("column {name} is not lowercase snake_case"));
        }
        if name.ends_with("_ms") && column.kind != "INTEGER" && !column.kind.is_empty() {
            found.push(format!(
                "column {name} holds milliseconds and must be INTEGER"
            ));
        }
        if TIME_WORDS.contains(&name) || TIME_SUFFIXES.iter().any(|suffix| name.ends_with(suffix)) {
            found.push(format!(
                "column {name} holds a time; name it with `_ms` and keep Unix milliseconds"
            ));
        }
        if let Some((_, instead)) = SPLIT_NAMES.iter().find(|(split, _)| *split == name) {
            found.push(format!("column {name} is ambiguous; use {instead}"));
        }
        if let Some((_, kind)) = TYPED_NAMES.iter().find(|(typed, _)| *typed == name)
            && !column.kind.is_empty()
            && column.kind != *kind
        {
            found.push(format!("column {name} must be {kind}"));
        }
    }
    found
}

pub fn run(workspace_root: &Path) -> Result<Vec<String>, Box<dyn Error>> {
    let mut files = Vec::new();
    collect(&workspace_root.join(SOURCES), workspace_root, &mut files)?;
    Ok(violations(&files))
}

fn collect(
    folder: &Path,
    workspace_root: &Path,
    files: &mut Vec<(String, String)>,
) -> Result<(), Box<dyn Error>> {
    let mut entries = std::fs::read_dir(folder)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(std::fs::DirEntry::path);
    for entry in entries {
        let path = entry.path();
        let relative = path
            .strip_prefix(workspace_root)?
            .to_string_lossy()
            .replace('\\', "/");
        if entry.file_type()?.is_dir() {
            if relative != SKIPPED_CRATE.trim_end_matches('/') && !relative.ends_with("/target") {
                collect(&path, workspace_root, files)?;
            }
            continue;
        }
        if relative.ends_with(".rs") && !relative.ends_with(".test.rs") {
            files.push((relative, std::fs::read_to_string(&path)?));
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "sql_schema.test.rs"]
mod tests;
