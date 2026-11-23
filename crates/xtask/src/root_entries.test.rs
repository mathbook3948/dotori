use super::*;

fn file(path: &str) -> ProjectFile<'_> {
    ProjectFile {
        path,
        is_symlink: false,
    }
}

#[test]
fn allows_files_under_listed_root_entries() {
    let files = [file("crates/core/src/lib.rs"), file("Cargo.toml")];

    assert!(stray_files(&files, "crates\nCargo.toml\n").is_empty());
}

#[test]
fn reports_each_unlisted_root_entry_once() {
    let files = [file("last.json"), file("out/a.txt"), file("out/b.txt")];

    assert_eq!(
        stray_files(&files, "crates\n"),
        vec![
            "last.json is not listed in guards/root-entries.txt",
            "out is not listed in guards/root-entries.txt",
        ]
    );
}

#[test]
fn reports_symbolic_links_even_under_listed_entries() {
    let files = [ProjectFile {
        path: "crates/chromium",
        is_symlink: true,
    }];

    assert_eq!(
        stray_files(&files, "crates\n"),
        vec!["crates/chromium is a symbolic link"]
    );
}
