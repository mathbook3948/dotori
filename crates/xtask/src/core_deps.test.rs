use super::*;

const CARGO_TREE: &str = "dotori-core v0.1.0 (/repo/crates/core)\n\
serde v1.0.0\n\
gpui v0.2.0\n\
[dev-dependencies]\n\
tempfile v3.0.0\n";

#[test]
fn reports_deps_missing_from_the_allowed_list() {
    assert_eq!(
        unlisted_deps(CARGO_TREE, "serde\n"),
        vec!["gpui", "tempfile"]
    );
}

#[test]
fn passes_when_every_dep_is_listed() {
    assert!(unlisted_deps(CARGO_TREE, "gpui\nserde\n\ntempfile\n").is_empty());
}

#[test]
fn passes_when_core_has_no_deps() {
    assert!(unlisted_deps("dotori-core v0.1.0 (/repo/crates/core)\n", "").is_empty());
}
