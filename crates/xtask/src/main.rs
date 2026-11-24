mod core_deps;
mod design_tokens;
mod docs;
mod forbidden_chars;
mod root_entries;
mod sql_schema;

use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let guard = std::env::args().nth(1).unwrap_or_default();

    let violations = match guard.as_str() {
        "forbidden-chars" => forbidden_chars::run(&workspace_root),
        "core-deps" => core_deps::run(&workspace_root),
        "root-entries" => root_entries::run(&workspace_root),
        "docs" => docs::check(&workspace_root),
        "sql-schema" => sql_schema::run(&workspace_root),
        "design-tokens" => design_tokens::check(&workspace_root),
        "design-write" => design_tokens::write(&workspace_root),
        "docs-write" => docs::write(&workspace_root),
        _ => {
            eprintln!(
                "usage: xtask <forbidden-chars|core-deps|root-entries|docs|docs-write|sql-schema|design-tokens|design-write>"
            );
            return ExitCode::from(2);
        }
    };

    match violations {
        Ok(violations) if violations.is_empty() => ExitCode::SUCCESS,
        Ok(violations) => {
            for violation in violations {
                eprintln!("{guard}: {violation}");
            }
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("{guard}: {error}");
            ExitCode::FAILURE
        }
    }
}
